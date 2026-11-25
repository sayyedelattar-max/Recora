use crate::ffmpeg;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Serialize, Clone, Debug)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub modified: i64,
    pub duration: f64,
    pub thumb: String,
    pub markers: Vec<f64>,
}

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    durations: HashMap<String, f64>,
}

const EXTS: [&str; 7] = ["mp4", "mkv", "webm", "gif", "mp3", "m4a", "mov"];

fn work_dir(dir: &Path) -> PathBuf {
    dir.join(".recora")
}

pub fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".recora.json");
    PathBuf::from(s)
}

pub fn write_meta(path: &Path, markers: &[f64], cursor: &[[f32; 4]]) {
    if markers.is_empty() && cursor.len() < 2 {
        return;
    }
    let _ = std::fs::write(sidecar(path), serde_json::json!({ "markers": markers, "cursor": cursor }).to_string());
}

/// The mouse path saved with a recording: `[seconds, x, y, buttons]` with x and y as shares of the picture.
pub fn read_cursor(path: &Path) -> Vec<[f32; 4]> {
    std::fs::read_to_string(sidecar(path))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| serde_json::from_value(v.get("cursor").cloned()?).ok())
        .unwrap_or_default()
}

fn read_markers(path: &Path) -> Vec<f64> {
    std::fs::read_to_string(sidecar(path))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("markers").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()))
        .unwrap_or_default()
}

fn thumb_path(dir: &Path, f: &Path, size: u64, modified: i64) -> PathBuf {
    let stem = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let safe: String = stem.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    work_dir(dir).join("thumbs").join(format!("{safe}_{size}_{modified}.jpg"))
}

pub fn list(dir: &Path) -> Vec<Item> {
    let mut files: Vec<(PathBuf, u64, i64)> = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !p.is_file() || !EXTS.contains(&ext.as_str()) {
                continue;
            }
            if let Ok(m) = e.metadata() {
                let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                files.push((p, m.len(), t));
            }
        }
    }
    files.sort_by(|a, b| b.2.cmp(&a.2));
    let meta_path = work_dir(dir).join("meta.json");
    let mut meta: Meta = std::fs::read_to_string(&meta_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let _ = std::fs::create_dir_all(work_dir(dir).join("thumbs"));
    let mut dirty = false;
    let mut budget = 12; // probe/thumbnail at most this many new files per call to keep the page responsive
    let mut items = vec![];
    for (p, size, modified) in files {
        let key = format!("{}|{}|{}", p.to_string_lossy(), size, modified);
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut duration = meta.durations.get(&key).copied().unwrap_or(-1.0);
        if duration < 0.0 && budget > 0 {
            duration = ffmpeg::duration(&p).unwrap_or(0.0);
            meta.durations.insert(key, duration);
            dirty = true;
            budget -= 1;
        }
        let tp = thumb_path(dir, &p, size, modified);
        let mut thumb = String::new();
        if ext != "mp3" && ext != "m4a" {
            if !tp.exists() && budget > 0 {
                let ss = if duration > 3.0 { "1" } else { "0" };
                let _ = ffmpeg::run(&["-ss".to_string(), ss.to_string(), "-i".into(), p.to_string_lossy().to_string(), "-frames:v".into(), "1".into(), "-vf".into(), "scale=480:-2".into(), tp.to_string_lossy().to_string()]);
                budget -= 1;
            }
            if tp.exists() {
                thumb = tp.to_string_lossy().to_string();
            }
        }
        items.push(Item {
            name: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            markers: read_markers(&p),
            path: p.to_string_lossy().to_string(),
            ext,
            size,
            modified,
            duration: duration.max(0.0),
            thumb,
        });
    }
    if dirty {
        if let Ok(t) = serde_json::to_string(&meta) {
            let _ = std::fs::write(&meta_path, t);
        }
    }
    items
}

/// An item for one file anywhere on the disk (not only the recordings folder).
pub fn probe(path: &Path) -> Result<Item> {
    let m = std::fs::metadata(path).map_err(|e| anyhow!("cannot open the file: {e}"))?;
    let duration = ffmpeg::duration(path).ok_or_else(|| anyhow!("this file does not look like a video"))?;
    Ok(Item {
        name: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        markers: read_markers(path),
        path: path.to_string_lossy().to_string(),
        ext: path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default(),
        size: m.len(),
        modified: m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
        duration,
        thumb: String::new(),
    })
}

fn clean_name(n: &str) -> String {
    let s: String = n.chars().filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    s.trim().trim_end_matches('.').to_string()
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    let name = clean_name(new_name);
    if name.is_empty() {
        return Err(anyhow!("The name cannot be empty"));
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let target = path.with_file_name(format!("{name}.{ext}"));
    if target.exists() && target != path {
        return Err(anyhow!("A file with that name already exists"));
    }
    std::fs::rename(path, &target)?;
    let (a, b) = (sidecar(path), sidecar(&target));
    if a.exists() {
        let _ = std::fs::rename(a, b);
    }
    Ok(target)
}

pub fn delete(path: &Path) -> Result<()> {
    trash::delete(path).map_err(|e| anyhow!("could not move to the recycle bin: {e}"))?;
    let _ = std::fs::remove_file(sidecar(path));
    Ok(())
}

fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    for i in 2..1000 {
        let p = path.with_file_name(format!("{stem} ({i}).{ext}"));
        if !p.exists() {
            return p;
        }
    }
    path
}

pub fn trim(path: &Path, start: f64, end: f64, precise: bool) -> Result<PathBuf> {
    if end <= start + 0.05 {
        return Err(anyhow!("The end must be after the start"));
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let out = unique(path.with_file_name(format!("{stem} (trimmed).mp4")));
    let mut a: Vec<String> = vec!["-ss".into(), format!("{start:.3}"), "-to".into(), format!("{end:.3}"), "-i".into(), path.to_string_lossy().into()];
    if precise {
        a.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"].map(String::from));
    } else {
        a.extend(["-c", "copy", "-avoid_negative_ts", "make_zero"].map(String::from));
    }
    a.extend(["-movflags".into(), "+faststart".into(), out.to_string_lossy().into()]);
    ffmpeg::run(&a)?;
    Ok(out)
}

pub fn export(path: &Path, kind: &str) -> Result<PathBuf> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let inp = path.to_string_lossy().to_string();
    let (suffix, ext, args): (&str, &str, Vec<String>) = match kind {
        "gif" => (" (GIF)", "gif", vec!["-i".into(), inp, "-vf".into(), "fps=12,scale=720:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer:bayer_scale=4".into(), "-loop".into(), "0".into()]),
        "mp3" => (" (audio)", "mp3", vec!["-i".into(), inp, "-vn".into(), "-c:a".into(), "libmp3lame".into(), "-q:a".into(), "2".into()]),
        "vertical" => (" (vertical)", "mp4", vec!["-i".into(), inp, "-filter_complex".into(), "[0:v]split[a][b];[a]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=30:5[bg];[b]scale=1080:-2[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,format=yuv420p[v]".into(), "-map".into(), "[v]".into(), "-map".into(), "0:a?".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), "-crf".into(), "20".into(), "-c:a".into(), "aac".into(), "-movflags".into(), "+faststart".into()]),
        "webm" => (" (WebM)", "webm", vec!["-i".into(), inp, "-c:v".into(), "libvpx-vp9".into(), "-crf".into(), "32".into(), "-b:v".into(), "0".into(), "-deadline".into(), "realtime".into(), "-cpu-used".into(), "6".into(), "-c:a".into(), "libopus".into()]),
        "small" => (" (small)", "mp4", vec!["-i".into(), inp, "-vf".into(), "scale=-2:720".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), "-crf".into(), "28".into(), "-pix_fmt".into(), "yuv420p".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "128k".into(), "-movflags".into(), "+faststart".into()]),
        other => return Err(anyhow!("unknown export '{other}'")),
    };
    let out = unique(path.with_file_name(format!("{stem}{suffix}.{ext}")));
    let mut a = args;
    a.push(out.to_string_lossy().into());
    ffmpeg::run(&a)?;
    Ok(out)
}
