//! Offline captions: speech to words with timings, using whisper.cpp on this computer.
//! The program (whisper-cli) travels inside the installer; the language model is downloaded once, on request, and kept in
//! the app's data folder. Nothing about the recording is ever sent anywhere.
use crate::ffmpeg;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Word {
    /// start and end in seconds of the recording
    pub s: f64,
    pub e: f64,
    pub w: String,
}

pub struct Model {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    /// approximate size in megabytes, shown before downloading
    pub mb: u32,
    /// a file smaller than this is a broken download
    pub min_bytes: u64,
}

pub const MODELS: &[Model] = &[
    Model { id: "fast", file: "ggml-base-q5_1.bin", label: "Fast", note: "Small and quick. Good for clear English; weaker for Arabic.", mb: 60, min_bytes: 40_000_000 },
    Model { id: "balanced", file: "ggml-small-q5_1.bin", label: "Balanced", note: "A good middle: decent Arabic on most computers.", mb: 190, min_bytes: 120_000_000 },
    Model { id: "best", file: "ggml-large-v3-turbo-q5_0.bin", label: "Best", note: "Very accurate and reasonably quick. Needs about 2 GB of memory.", mb: 580, min_bytes: 400_000_000 },
    Model { id: "max", file: "ggml-large-v3-q5_0.bin", label: "Maximum (recommended for Arabic)", note: "The most accurate for Arabic and mixed Arabic/English. About 3 times slower than Best; needs about 3 GB of memory.", mb: 1080, min_bytes: 900_000_000 },
];

fn model(id: &str) -> &'static Model {
    MODELS.iter().find(|m| m.id == id).unwrap_or(&MODELS[1])
}

pub fn model_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(model(id).file)
}

/// whisper-cli next to the installed app, in any of the places the installer may put resources.
pub fn engine() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RECORA_WHISPER") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    let name = if cfg!(windows) { "whisper-cli.exe" } else { "whisper-cli" };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    ["whisper", "resources/whisper", "_up_/whisper", "."]
        .iter()
        .map(|d| dir.join(d).join(name))
        .find(|p| p.metadata().map(|m| m.len() > 1000).unwrap_or(false))
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub mb: u32,
    pub have: bool,
}

#[derive(Serialize)]
pub struct Status {
    pub engine: bool,
    pub dir: String,
    pub custom: bool,
    pub models: Vec<ModelInfo>,
}

pub fn status(dir: &Path) -> Status {
    Status {
        engine: engine().is_some(),
        dir: dir.to_string_lossy().to_string(),
        custom: false,
        models: MODELS.iter().map(|m| ModelInfo { id: m.id, label: m.label, note: m.note, mb: m.mb, have: dir.join(m.file).metadata().map(|x| x.len() >= m.min_bytes).unwrap_or(false) }).collect(),
    }
}

/// Downloads a model into `dir`, reporting (bytes so far, total). Written to a temporary name and renamed when complete.
pub fn download(dir: &Path, id: &str, base: Option<&str>, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let m = model(id);
    std::fs::create_dir_all(dir)?;
    let url = format!("{}/{}", base.unwrap_or("https://huggingface.co/ggerganov/whisper.cpp/resolve/main"), m.file);
    CANCEL.store(false, Ordering::Relaxed);
    let resp = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(20)).build().get(&url).call().map_err(|e| match e {
        ureq::Error::Status(c, _) => anyhow!("The download server answered {c}. Try again later."),
        e => anyhow!("Could not download the model. Check the internet connection. ({e})"),
    })?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let part = dir.join(format!("{}.part", m.file));
    let mut out = std::fs::File::create(&part)?;
    let mut r = resp.into_reader();
    let (mut done, mut buf, mut last) = (0u64, vec![0u8; 256 * 1024], 0u64);
    loop {
        if CANCEL.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err(anyhow!("Download cancelled"));
        }
        let n = r.read(&mut buf).map_err(|e| anyhow!("The download was interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last > 512 * 1024 {
            progress(done, total.max(done));
            last = done;
        }
    }
    out.flush()?;
    drop(out);
    if done < m.min_bytes || (total > 0 && done != total) {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!("The download did not finish. Try again."));
    }
    std::fs::rename(&part, dir.join(m.file))?;
    progress(done, done);
    Ok(())
}

fn is_noise(t: &str) -> bool {
    let t = t.trim();
    t.is_empty() || (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) || t.contains('♪') || t.contains('♫')
}

fn is_punct(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty() && t.chars().all(|c| !c.is_alphanumeric())
}

/// Words from whisper-cli's JSON (one entry per word). Tolerates broken bytes, drops sound tags such as [MUSIC],
/// and glues punctuation to the word before it.
pub fn parse_words(raw: &[u8]) -> Result<Vec<Word>> {
    let text = String::from_utf8_lossy(raw);
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| anyhow!("Could not read the transcript: {e}"))?;
    let mut out: Vec<Word> = Vec::new();
    for seg in v["transcription"].as_array().cloned().unwrap_or_default() {
        let t = seg["text"].as_str().unwrap_or("");
        let (s, e) = (seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0, seg["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0);
        if is_noise(t) {
            continue;
        }
        let w = t.trim().replace('\u{FFFD}', "");
        if w.is_empty() {
            continue;
        }
        // a piece that starts without a space continues the word before it (some languages split inside words)
        let glued = !t.starts_with(' ') && !out.is_empty() && out.last().map(|l| (s - l.e).abs() < 0.35).unwrap_or(false);
        if is_punct(&w) || glued {
            if let Some(l) = out.last_mut() {
                l.w.push_str(&w);
                l.e = l.e.max(e);
                continue;
            }
        }
        out.push(Word { s, e: e.max(s + 0.05), w });
    }
    // each word ends before the next begins
    for i in 1..out.len() {
        if out[i - 1].e > out[i].s {
            out[i - 1].e = out[i].s.max(out[i - 1].s + 0.02);
        }
    }
    Ok(out)
}

fn progress_pct(line: &str) -> Option<u32> {
    let i = line.find("progress =")? + 10;
    line[i..].trim().trim_end_matches('%').trim().parse().ok()
}

/// Speech to words. `lang` is a code such as "ar" or "en", or "auto".
pub fn transcribe(src: &Path, model_file: &Path, lang: &str, progress: &dyn Fn(u32)) -> Result<Vec<Word>> {
    let exe = engine().ok_or_else(|| anyhow!("The captions engine is missing from this installation."))?;
    if !model_file.exists() {
        return Err(anyhow!("Download the language model first."));
    }
    CANCEL.store(false, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("recora_cap_{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let wav = tmp.join("speech.wav");
    // 16 kHz mono; only the speech band is kept and quiet speech is levelled up. No noise remover: it damages speech more than it helps.
    ffmpeg::run(&[std::ffi::OsString::from("-i"), src.into(), "-vn".into(), "-ac".into(), "1".into(), "-ar".into(), "16000".into(), "-af".into(), "highpass=f=80,lowpass=f=7600,dynaudnorm=f=150:g=15".into(), "-c:a".into(), "pcm_s16le".into(), wav.clone().into()])
        .map_err(|_| anyhow!("This recording has no sound to turn into captions."))?;
    let base = tmp.join("out");
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).clamp(2, 8);
    let mut c = Command::new(&exe);
    c.arg("-m").arg(model_file).arg("-f").arg(&wav).arg("-l").arg(lang).args(["-ml", "1", "-sow", "-oj", "-pp", "-nth", "0.6", "-sns", "-t"]).arg(threads.to_string()).arg("-of").arg(&base);
    // a short hint in the language helps punctuation and mixed-in English terms
    match lang {
        "ar" => { c.arg("--prompt").arg("مرحباً بكم، في هذا الفيديو نشرح خطوة بخطوة، مع بعض المصطلحات التقنية مثل Windows وGitHub."); }
        "en" => { c.arg("--prompt").arg("Hello, and welcome. In this video we go through it step by step."); }
        _ => {}
    }
    if let Some(d) = exe.parent() {
        c.current_dir(d);
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    c.creation_flags(0x0800_0000 | 0x0000_4000); // no console window, below normal priority
    let mut child = c.spawn().map_err(|e| anyhow!("Could not start the captions engine: {e}"))?;
    let err = child.stderr.take().unwrap();
    let mut tail: Vec<String> = Vec::new();
    // stderr is read line by line; the progress lines drive the bar
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for l in BufReader::new(err).split(b'\n').flatten() {
            let _ = tx.send(String::from_utf8_lossy(&l).to_string());
        }
    });
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(l) => {
                if let Some(p) = progress_pct(&l) {
                    progress(p.min(100));
                } else if !l.trim().is_empty() {
                    tail.push(l);
                    if tail.len() > 6 {
                        tail.remove(0);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if CANCEL.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(anyhow!("Cancelled"));
        }
    }
    let status = child.wait()?;
    let _ = reader.join();
    let json = std::fs::read(base.with_extension("json"));
    let _ = std::fs::remove_dir_all(&tmp);
    if !status.success() {
        return Err(anyhow!("The captions engine stopped: {}", tail.last().cloned().unwrap_or_default()));
    }
    let words = parse_words(&json.map_err(|_| anyhow!("The captions engine produced no result."))?)?;
    progress(100);
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_words_drops_sound_tags_and_glues_punctuation() {
        let j = r#"{"transcription":[
          {"offsets":{"from":0,"to":400},"text":" [_BEG_]"},
          {"offsets":{"from":500,"to":900},"text":" مرحبا"},
          {"offsets":{"from":900,"to":1200},"text":"،"},
          {"offsets":{"from":1300,"to":1800},"text":" بكم"},
          {"offsets":{"from":1800,"to":2000},"text":" (music)"},
          {"offsets":{"from":2100,"to":2600},"text":" في"},
          {"offsets":{"from":2600,"to":3000},"text":"ديو"},
          {"offsets":{"from":3100,"to":3500},"text":" Hello"},
          {"offsets":{"from":3500,"to":3600},"text":"."}
        ]}"#;
        let w = parse_words(j.as_bytes()).unwrap();
        let t: Vec<&str> = w.iter().map(|x| x.w.as_str()).collect();
        assert_eq!(t, vec!["مرحبا،", "بكم", "فيديو", "Hello."]);
        assert_eq!((w[0].s, w[0].e), (0.5, 1.2));
        assert!(w.windows(2).all(|p| p[0].e <= p[1].s + 1e-9));
    }

    #[test]
    fn survives_broken_bytes_inside_a_word() {
        let mut j = br#"{"transcription":[{"offsets":{"from":0,"to":500},"text":" ab"#.to_vec();
        j.extend_from_slice(&[0xD8]);
        j.extend_from_slice(br#"c"},{"offsets":{"from":600,"to":900},"text":" ok"}]}"#);
        let w = parse_words(&j).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].w, "abc");
        assert!(parse_words(b"not json").is_err());
    }

    #[test]
    fn reads_progress_lines() {
        assert_eq!(progress_pct("whisper_print_progress_callback: progress =  42%"), Some(42));
        assert_eq!(progress_pct("whisper_init: loading model"), None);
    }

    #[test]
    fn lists_models_and_notices_the_ones_present() {
        let d = std::env::temp_dir().join(format!("recora_models_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(status(&d).models.iter().all(|m| !m.have));
        std::fs::write(d.join("ggml-small-q5_1.bin"), vec![0u8; 5]).unwrap();
        assert!(status(&d).models.iter().all(|m| !m.have), "a tiny file is a broken download");
        assert!(model("unknown").id == "balanced");
        assert!(MODELS.iter().all(|m| m.file.ends_with(".bin")));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A stand-in for whisper-cli that behaves like the real one (same arguments, progress lines, JSON file),
    /// so the whole path through ffmpeg, the process and the parser is exercised.
    #[cfg(unix)]
    #[test]
    fn runs_the_engine_end_to_end_with_a_stand_in() {
        use std::os::unix::fs::PermissionsExt;
        if !ffmpeg::available() {
            return;
        }
        let d = std::env::temp_dir().join(format!("recora_cap_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join("whisper-cli");
        std::fs::write(
            &fake,
            r#"#!/bin/bash
while [ $# -gt 0 ]; do case "$1" in -f) f="$2"; shift;; -of) o="$2"; shift;; -l) l="$2"; shift;; esac; shift; done
# the extracted audio must be 16 kHz mono
info=$(ffprobe -v error -show_entries stream=sample_rate,channels -of csv=p=0 "$f" 2>/dev/null || ffmpeg -hide_banner -i "$f" 2>&1 | grep -o "16000 Hz, mono")
echo "$info $l" > "$o.args"
echo "whisper_print_progress_callback: progress =  10%" >&2
echo "whisper_print_progress_callback: progress =  70%" >&2
printf '{"transcription":[{"offsets":{"from":200,"to":700},"text":" hello"},{"offsets":{"from":800,"to":1200},"text":" world"}]}' > "$o.json"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("RECORA_WHISPER", &fake);
        let src = d.join("a.wav");
        ffmpeg::run(&["-f", "lavfi", "-i", "sine=frequency=300:duration=2", src.to_str().unwrap()]).unwrap();
        let model = d.join("m.bin");
        std::fs::write(&model, b"x").unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let w = transcribe(&src, &model, "ar", &|p| seen.lock().unwrap().push(p)).unwrap();
        assert_eq!(w.iter().map(|x| x.w.as_str()).collect::<Vec<_>>(), vec!["hello", "world"]);
        let seen = seen.lock().unwrap();
        assert!(seen.contains(&10) && seen.contains(&70) && *seen.last().unwrap() == 100, "{seen:?}");
        assert!(transcribe(&src, &d.join("missing.bin"), "ar", &|_| {}).is_err());
        std::env::remove_var("RECORA_WHISPER");
        let _ = std::fs::remove_dir_all(&d);
    }
}
