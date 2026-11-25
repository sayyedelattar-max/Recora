pub mod audio;
pub mod cursor;
pub mod mock;
pub mod blur;
pub mod preview;
pub mod mux;
#[cfg(windows)]
pub mod windows;

use crate::ffmpeg;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub hwnd: String,
    pub title: String,
    pub process: String,
}

#[derive(Clone, Debug)]
pub struct RegionSel {
    pub monitor: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug)]
pub enum Source {
    Screen { monitor: usize },
    Window { hwnd: isize },
    Region(RegionSel),
}

#[derive(Clone, Debug)]
pub struct CaptureSpec {
    pub source: Source,
    pub fps: u32,
    pub quality: String,
    pub cursor: bool,
    /// areas to blur while recording
    pub zones: Vec<ZoneSpec>,
    /// process names (lower case, no .exe) whose windows are blurred where visible
    pub blur_apps: Vec<String>,
    /// top-left corner of the recorded monitor on the desktop (physical pixels)
    pub origin: (i32, i32),
    /// size of the recorded monitor in physical pixels (0 when unknown)
    pub size: (u32, u32),
}

/// `process` set: fractions of that app's window. Otherwise pixels of the recorded monitor.
#[derive(Clone, Debug)]
pub struct ZoneSpec {
    pub process: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl CaptureSpec {
    pub fn bitrate_for(&self, w: u32, h: u32) -> u32 {
        let s = crate::settings::Settings { fps: self.fps, quality: self.quality.clone(), ..Default::default() };
        s.bitrate(w, h)
    }
}

pub struct VideoStats {
    pub frames: u64,
}

pub trait ActiveCapture: Send {
    fn first_frame(&self) -> Option<Instant>;
    fn last_error(&self) -> Option<String> {
        None
    }
    fn stop(self: Box<Self>) -> Result<VideoStats>;
}

/// The app window under a desktop point (physical pixels): process name, title and its rectangle.
pub fn window_at(x: i32, y: i32) -> Option<(String, String, (i32, i32, i32, i32))> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            return windows::window_at(x, y);
        }
    }
    let _ = (x, y);
    None
}

/// A picture of the chosen source as a data URL, or None when it cannot be captured right now.
pub fn preview(source: &Source) -> Option<String> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            let (w, h, bgra) = windows::grab(source).ok()?;
            let (tw, th, bgr) = preview::downscale_bgra(&bgra, w, h, 520);
            return Some(preview::bmp_data_url(tw, th, &bgr));
        }
    }
    let _ = source;
    Some(preview::sample(520, 325))
}

pub fn warm_up() {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            windows::warm_up();
        }
    }
}

pub fn start_capture(spec: &CaptureSpec, out: &Path) -> Result<Box<dyn ActiveCapture>> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            return windows::start(spec, out);
        }
    }
    mock::start(spec, out)
}

pub fn list_monitors() -> Vec<MonitorInfo> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            return windows::list_monitors();
        }
    }
    vec![MonitorInfo { index: 0, name: "DISPLAY1".into(), width: 2560, height: 1600, primary: true }]
}

pub fn list_windows() -> Vec<WindowInfo> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            return windows::list_windows();
        }
    }
    vec![WindowInfo { hwnd: "1".into(), title: "Sample window".into(), process: "sample.exe".into() }]
}

// ---------------------------------------------------------------------------------------------------------------------
// A recording session: one or more segments (a pause ends a segment), then one muxing pass.
// ---------------------------------------------------------------------------------------------------------------------

pub struct Seg {
    pub video: PathBuf,
    pub mic: Option<PathBuf>,
    pub sys: Option<PathBuf>,
    pub mic_offset: f64, // seconds the audio starts after the video (negative: before)
    pub sys_offset: f64,
    pub wall: f64,
    pub frames: u64,
}

struct Active {
    cap: Box<dyn ActiveCapture>,
    audio: Option<audio::AudioRec>,
    t_start: Instant,
    video: PathBuf,
}

pub struct Session {
    pub dir: PathBuf,
    spec: CaptureSpec,
    audio: Option<audio::AudioSpec>,
    segs: Vec<Seg>,
    active: Option<Active>,
    pub markers: Vec<f64>,
    pub mic_muted: Arc<AtomicBool>,
    pub sys_muted: Arc<AtomicBool>,
    pub levels: audio::Levels,
    pub notes: Vec<String>,
    cursor: Option<cursor::Tracker>,
    cursor_log: Vec<[f32; 4]>,
}

impl Session {
    pub fn new(dir: PathBuf, spec: CaptureSpec, audio: Option<audio::AudioSpec>) -> Result<Session> {
        std::fs::create_dir_all(&dir)?;
        Ok(Session {
            dir,
            spec,
            audio,
            segs: vec![],
            active: None,
            markers: vec![],
            mic_muted: Arc::new(AtomicBool::new(false)),
            sys_muted: Arc::new(AtomicBool::new(false)),
            levels: audio::Levels::new(),
            notes: vec![],
            cursor: None,
            cursor_log: vec![],
        })
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn start_segment(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let n = self.segs.len();
        let video = self.dir.join(format!("seg{n}.mp4"));
        let t_start = Instant::now();
        let audio = self.audio.as_ref().map(|a| {
            audio::AudioRec::start(
                audio::AudioSpec { mic: a.mic, mic_device: a.mic_device.clone(), system: a.system },
                self.dir.clone(),
                n,
                self.mic_muted.clone(),
                self.sys_muted.clone(),
                self.levels.clone(),
            )
        });
        if self.cursor.is_none() {
            let (o, sz) = (self.spec.origin, self.spec.size);
            let area: cursor::Area = match &self.spec.source {
                Source::Screen { .. } if sz.0 > 0 => Box::new(move || Some((o.0, o.1, sz.0 as i32, sz.1 as i32))),
                Source::Region(r) if sz.0 > 0 => {
                    let (x, y, w, h) = (o.0 + r.x as i32, o.1 + r.y as i32, r.w as i32, r.h as i32);
                    Box::new(move || Some((x, y, w, h)))
                }
                Source::Window { hwnd } => cursor::window_area(*hwnd),
                _ => Box::new(|| None),
            };
            self.cursor = Some(cursor::Tracker::start(cursor::desktop_reader(), area));
        }
        let cap = match start_capture(&self.spec, &video) {
            Ok(c) => c,
            Err(e) => {
                if let Some(a) = audio {
                    let _ = a.stop();
                }
                return Err(e);
            }
        };
        self.active = Some(Active { cap, audio, t_start, video });
        Ok(())
    }

    /// Seconds recorded so far (pauses excluded).
    pub fn recorded(&self) -> f64 {
        let done: f64 = self.segs.iter().map(|s| s.wall).sum();
        done + self.active.as_ref().map(|a| a.t_start.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    pub fn frames_live(&self) -> bool {
        self.active.as_ref().map(|a| a.cap.first_frame().is_some()).unwrap_or(false)
    }

    pub fn add_marker(&mut self) {
        let t = self.recorded();
        self.markers.push(t);
    }

    pub fn end_segment(&mut self) -> Result<()> {
        let Some(a) = self.active.take() else { return Ok(()) };
        let wall = a.t_start.elapsed().as_secs_f64();
        let first_video = a.cap.first_frame();
        let cap_err = a.cap.last_error();
        if let Some(t) = &self.cursor {
            let base: f64 = self.segs.iter().map(|s| s.wall).sum();
            let zero = first_video.unwrap_or(a.t_start);
            self.cursor_log.extend(t.take(a.t_start, zero, base));
        }
        let stats = a.cap.stop();
        crate::diag::log(&format!("segment ended: wall={wall:.1}s first_frame={} stats={:?} error={:?}", first_video.is_some(), stats.as_ref().map(|s| s.frames).ok(), cap_err));
        let ares = a.audio.map(|x| x.stop());
        let stats = stats?;
        let off = |w: &Option<audio::WriterOut>| -> f64 {
            match (w.as_ref().and_then(|w| w.first_at), first_video) {
                (Some(at), Some(v)) => at.duration_since(v).as_secs_f64() * if at >= v { 1.0 } else { -1.0 },
                (Some(at), None) => at.duration_since(a.t_start).as_secs_f64(),
                _ => 0.0,
            }
        };
        let mut seg = Seg { video: a.video, mic: None, sys: None, mic_offset: 0.0, sys_offset: 0.0, wall, frames: stats.frames };
        if let Some(r) = ares {
            for e in r.errors {
                let msg = if e.starts_with("mic") {
                    "The microphone could not be opened, so this recording has no voice."
                } else {
                    "System sound could not be captured, so it is missing from this recording."
                };
                if !self.notes.iter().any(|n| n == msg) {
                    self.notes.push(msg.to_string());
                }
            }
            seg.mic_offset = off(&r.mic);
            seg.sys_offset = off(&r.sys);
            seg.mic = r.mic.map(|w| w.path);
            seg.sys = r.sys.map(|w| w.path);
        }
        if !seg.video.exists() || std::fs::metadata(&seg.video).map(|m| m.len() < 1000).unwrap_or(true) {
            // a pause that lasted a blink; skip silently unless it is the only segment
            if self.segs.is_empty() && wall > 0.5 {
                let size = std::fs::metadata(&seg.video).map(|m| m.len()).unwrap_or(0);
                crate::diag::log(&format!("no usable video: file exists={} size={size}", seg.video.exists()));
                let why = cap_err.map(|e| format!(" Reason: {e}")).unwrap_or_default();
                let log = crate::diag::path().map(|p| format!(" Details were saved in {}", p.display())).unwrap_or_default();
                return Err(anyhow!("Recora did not receive any video from the screen.{why}{log}"));
            }
            return Ok(());
        }
        self.segs.push(seg);
        Ok(())
    }

    pub fn finish(mut self, out: &Path) -> Result<mux::Report> {
        self.end_segment()?;
        if self.segs.is_empty() {
            return Err(anyhow!("Nothing was recorded. Capture produced no video; try recording for a little longer."));
        }
        let mut report = mux::finalize(&self.segs, &self.dir, out)?;
        report.notes.extend(self.notes.clone());
        report.markers = self.markers.clone();
        report.cursor = std::mem::take(&mut self.cursor_log);
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(report)
    }

    pub fn discard(mut self) {
        let _ = self.end_segment();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn ffmpeg_ready() -> bool {
    ffmpeg::available()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec() -> CaptureSpec {
        CaptureSpec { source: Source::Screen { monitor: 0 }, fps: 30, quality: "high".into(), cursor: true, zones: vec![], blur_apps: vec![], origin: (0, 0), size: (0, 0) }
    }

    #[test]
    fn pause_resume_keeps_wall_clock_length() {
        let dir = std::env::temp_dir().join("recora_t1");
        let _ = std::fs::remove_dir_all(&dir);
        let out = std::env::temp_dir().join("recora_t1_out.mp4");
        let mut s = Session::new(dir, spec(), None).unwrap();
        s.start_segment().unwrap();
        std::thread::sleep(Duration::from_secs(3));
        s.add_marker();
        s.end_segment().unwrap(); // pause
        std::thread::sleep(Duration::from_secs(1));
        s.start_segment().unwrap();
        std::thread::sleep(Duration::from_secs(3));
        let rep = s.finish(&out).unwrap();
        println!("report: {rep:?}");
        assert!((rep.wall - 6.0).abs() < 0.8, "wall {}", rep.wall);
        // the synthetic source starts with a short burst of frames, so it can run slightly long; it must never be short
        assert!(rep.duration >= rep.wall - 0.5 && rep.duration < rep.wall + 1.8, "duration {} vs wall {}", rep.duration, rep.wall);
        assert_eq!(rep.markers.len(), 1);
    }

    #[test]
    fn audio_is_mixed_offset_and_padded_to_video_length() {
        let dir = std::env::temp_dir().join("recora_t2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // video: 5 s, "mic": 2 s tone that starts 1 s late, "system": 1 s tone (loopback stops when silent)
        ffmpeg::run(&["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=5", "-c:v", "libx264", "-pix_fmt", "yuv420p", dir.join("v.mp4").to_str().unwrap()]).unwrap();
        ffmpeg::run(&["-f", "lavfi", "-i", "sine=frequency=440:duration=2", dir.join("mic.wav").to_str().unwrap()]).unwrap();
        ffmpeg::run(&["-f", "lavfi", "-i", "sine=frequency=660:duration=1", dir.join("sys.wav").to_str().unwrap()]).unwrap();
        let seg = Seg { video: dir.join("v.mp4"), mic: Some(dir.join("mic.wav")), sys: Some(dir.join("sys.wav")), mic_offset: 1.0, sys_offset: -0.0, wall: 5.0, frames: 150 };
        let out = dir.join("final.mp4");
        let rep = mux::finalize(&[seg], &dir, &out).unwrap();
        assert!(ffmpeg::has_audio(&out));
        assert!((rep.duration - 5.0).abs() < 0.3, "duration {}", rep.duration);
    }

    #[test]
    fn static_tail_is_padded_to_the_wall_clock() {
        // the picture stopped changing after 3 s, but 6 s were recorded: the clip must still be 6 s long
        let dir = std::env::temp_dir().join("recora_t3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ffmpeg::run(&["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", dir.join("v.mp4").to_str().unwrap()]).unwrap();
        let seg = Seg { video: dir.join("v.mp4"), mic: None, sys: None, mic_offset: 0.0, sys_offset: 0.0, wall: 6.0, frames: 90 };
        let rep = mux::finalize(&[seg], &dir, &dir.join("final.mp4")).unwrap();
        assert!((rep.duration - 6.0).abs() < 0.3, "duration {}", rep.duration);
        assert!(rep.notes.iter().any(|n| n.contains("held")));
    }
}
