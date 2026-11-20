//! A small log file next to the recordings (recora-log.txt) so a failed capture can be diagnosed.
use parking_lot::Mutex;
use std::io::Write;
use std::path::PathBuf;

static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn init(dir: &std::path::Path) {
    let p = dir.join("recora-log.txt");
    if std::fs::metadata(&p).map(|m| m.len() > 512_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&p);
    }
    *PATH.lock() = Some(p);
    log(&format!("---- Recora {} ----", env!("CARGO_PKG_VERSION")));
}

pub fn path() -> Option<PathBuf> {
    PATH.lock().clone()
}

pub fn log(msg: &str) {
    let Some(p) = path() else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "[{}] {msg}", chrono::Local::now().format("%H:%M:%S%.3f"));
    }
}
