//! Microphone + system-sound capture to WAV files.
//! Every stream gets its own writer thread. The writer fills gaps with silence using the wall clock, because
//! WASAPI loopback delivers *nothing* while the system is silent - without this, later audio drifts early.
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct Chunk {
    at: Instant,
    data: Vec<f32>,
}

pub struct WriterOut {
    pub path: PathBuf,
    pub first_at: Option<Instant>,
}

pub struct AudioSpec {
    pub mic: bool,
    pub mic_device: String,
    pub system: bool,
}

#[derive(Clone)]
pub struct Levels {
    pub mic: Arc<AtomicU32>,
    pub sys: Arc<AtomicU32>,
}

impl Levels {
    pub fn new() -> Self {
        Levels { mic: Arc::new(AtomicU32::new(0)), sys: Arc::new(AtomicU32::new(0)) }
    }
    pub fn get(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Ordering::Relaxed))
    }
}

pub struct AudioRec {
    stop: Sender<()>,
    ctl: Option<JoinHandle<Vec<(String, WriterOut)>>>,
}

pub struct AudioResult {
    pub mic: Option<WriterOut>,
    pub sys: Option<WriterOut>,
    pub errors: Vec<String>,
}

pub fn list_inputs() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default()
}

pub fn has_default_input() -> bool {
    cpal::default_host().default_input_device().is_some()
}

fn level_store(a: &AtomicU32, data: &[f32]) {
    let mut peak = 0f32;
    for s in data {
        let v = s.abs();
        if v > peak {
            peak = v;
        }
    }
    let old = f32::from_bits(a.load(Ordering::Relaxed));
    // fast attack, slow release
    let v = if peak > old { peak } else { old * 0.85 + peak * 0.15 };
    a.store(v.to_bits(), Ordering::Relaxed);
}

fn writer(rx: Receiver<Chunk>, path: PathBuf, channels: u16, rate: u32, muted: Arc<AtomicBool>, level: Arc<AtomicU32>) -> Option<WriterOut> {
    let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&path, spec).ok()?;
    let mut first_at: Option<Instant> = None;
    let mut written: u64 = 0; // frames
    while let Ok(ch) = rx.recv() {
        let frames = (ch.data.len() / channels as usize) as u64;
        let start = *first_at.get_or_insert_with(|| ch.at - Duration::from_secs_f64(frames as f64 / rate as f64));
        let expected = (ch.at.saturating_duration_since(start).as_secs_f64() * rate as f64) as u64;
        let expected_before = expected.saturating_sub(frames);
        if expected_before > written + (rate as u64 / 25) {
            for _ in 0..((expected_before - written) * channels as u64) {
                let _ = w.write_sample(0f32);
            }
            written = expected_before;
        }
        let m = muted.load(Ordering::Relaxed);
        level_store(&level, if m { &[] } else { &ch.data });
        if m {
            level.store(0f32.to_bits(), Ordering::Relaxed);
        }
        for s in &ch.data {
            let _ = w.write_sample(if m { 0.0 } else { *s });
        }
        written += frames;
    }
    let _ = w.finalize();
    Some(WriterOut { path, first_at })
}

fn build(device: &cpal::Device, cfg: cpal::SupportedStreamConfig, tx: Sender<Chunk>) -> Result<Stream> {
    let config: cpal::StreamConfig = cfg.clone().into();
    let err_fn = |e| eprintln!("audio stream error: {e}");
    let s = match cfg.sample_format() {
        SampleFormat::F32 => device.build_input_stream(&config, move |d: &[f32], _| { let _ = tx.send(Chunk { at: Instant::now(), data: d.to_vec() }); }, err_fn, None),
        SampleFormat::I16 => device.build_input_stream(&config, move |d: &[i16], _| { let _ = tx.send(Chunk { at: Instant::now(), data: d.iter().map(|v| *v as f32 / 32768.0).collect() }); }, err_fn, None),
        SampleFormat::U16 => device.build_input_stream(&config, move |d: &[u16], _| { let _ = tx.send(Chunk { at: Instant::now(), data: d.iter().map(|v| (*v as f32 - 32768.0) / 32768.0).collect() }); }, err_fn, None),
        SampleFormat::I32 => device.build_input_stream(&config, move |d: &[i32], _| { let _ = tx.send(Chunk { at: Instant::now(), data: d.iter().map(|v| *v as f32 / 2147483648.0).collect() }); }, err_fn, None),
        other => return Err(anyhow!("unsupported audio format {other:?}")),
    }?;
    s.play()?;
    Ok(s)
}

impl AudioRec {
    /// Opens the requested devices. Never fails as a whole: devices that cannot be opened are reported in `errors`
    /// of the final result and simply stay silent.
    pub fn start(spec: AudioSpec, dir: PathBuf, seg: usize, mic_muted: Arc<AtomicBool>, sys_muted: Arc<AtomicBool>, levels: Levels) -> AudioRec {
        let (stop_tx, stop_rx) = channel::<()>();
        let ctl = std::thread::spawn(move || {
            let host = cpal::default_host();
            let mut streams: Vec<Stream> = Vec::new();
            let mut writers: Vec<(String, JoinHandle<Option<WriterOut>>)> = Vec::new();
            let mut errors: Vec<(String, String)> = Vec::new();
            if spec.mic {
                let dev = if spec.mic_device.is_empty() {
                    host.default_input_device()
                } else {
                    host.input_devices().ok().and_then(|mut d| d.find(|x| x.name().map(|n| n == spec.mic_device).unwrap_or(false))).or_else(|| host.default_input_device())
                };
                match dev.ok_or_else(|| anyhow!("no microphone found")).and_then(|d| {
                    let cfg = d.default_input_config()?;
                    let (tx, rx) = channel();
                    let path = dir.join(format!("seg{seg}_mic.wav"));
                    let (ch, rate) = (cfg.channels(), cfg.sample_rate().0);
                    let (m, l) = (mic_muted.clone(), levels.mic.clone());
                    writers.push(("mic".into(), std::thread::spawn(move || writer(rx, path, ch, rate, m, l))));
                    build(&d, cfg, tx)
                }) {
                    Ok(s) => streams.push(s),
                    Err(e) => errors.push(("mic".into(), e.to_string())),
                }
            }
            if spec.system && cfg!(windows) {
                match host.default_output_device().ok_or_else(|| anyhow!("no speakers found")).and_then(|d| {
                    let cfg = d.default_output_config()?;
                    let (tx, rx) = channel();
                    let path = dir.join(format!("seg{seg}_sys.wav"));
                    let (ch, rate) = (cfg.channels(), cfg.sample_rate().0);
                    let (m, l) = (sys_muted.clone(), levels.sys.clone());
                    writers.push(("sys".into(), std::thread::spawn(move || writer(rx, path, ch, rate, m, l))));
                    build(&d, cfg, tx) // an input stream on an output device = WASAPI loopback
                }) {
                    Ok(s) => streams.push(s),
                    Err(e) => errors.push(("sys".into(), e.to_string())),
                }
            }
            let _ = stop_rx.recv();
            drop(streams); // closes the senders; writers drain and finish
            let mut out = Vec::new();
            for (kind, h) in writers {
                if let Ok(Some(w)) = h.join() {
                    out.push((kind, w));
                }
            }
            for (kind, e) in errors {
                out.push((format!("err:{kind}"), WriterOut { path: PathBuf::from(e), first_at: None }));
            }
            out
        });
        AudioRec { stop: stop_tx, ctl: Some(ctl) }
    }

    pub fn stop(mut self) -> AudioResult {
        let _ = self.stop.send(());
        let mut res = AudioResult { mic: None, sys: None, errors: vec![] };
        if let Some(h) = self.ctl.take() {
            for (kind, w) in h.join().unwrap_or_default() {
                match kind.as_str() {
                    "mic" => res.mic = Some(w),
                    "sys" => res.sys = Some(w),
                    k if k.starts_with("err:") => res.errors.push(format!("{}: {}", &k[4..], w.path.to_string_lossy())),
                    _ => {}
                }
            }
        }
        res
    }
}
