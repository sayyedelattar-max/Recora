use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub fn bin() -> PathBuf {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() && p.metadata().map(|m| m.len() > 1000).unwrap_or(false) {
                return p;
            }
        }
    }
    PathBuf::from(name)
}

pub fn cmd() -> Command {
    let mut c = Command::new(bin());
    c.stdin(Stdio::null());
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

pub fn available() -> bool {
    cmd().arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Runs ffmpeg and returns its stderr; errors carry the last lines of it.
pub fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Result<String> {
    let out = cmd().args(["-hide_banner", "-y", "-loglevel", "error"]).args(args).output()?;
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        Ok(err)
    } else {
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        Err(anyhow!("ffmpeg failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | ")))
    }
}

/// Duration in seconds, read from the container header.
pub fn duration(path: &Path) -> Option<f64> {
    let out = cmd().args(["-hide_banner", "-i"]).arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    let i = text.find("Duration:")? + 9;
    let t: String = text[i..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.').collect();
    let p: Vec<&str> = t.split(':').collect();
    if p.len() != 3 {
        return None;
    }
    Some(p[0].parse::<f64>().ok()? * 3600.0 + p[1].parse::<f64>().ok()? * 60.0 + p[2].parse::<f64>().ok()?)
}

pub fn has_audio(path: &Path) -> bool {
    cmd().args(["-hide_banner", "-i"]).arg(path).output().map(|o| String::from_utf8_lossy(&o.stderr).contains("Audio:")).unwrap_or(false)
}
