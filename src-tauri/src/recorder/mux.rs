//! Turns segments (video + wav files) into one finished MP4 and checks the result against the wall clock.
use super::Seg;
use crate::ffmpeg;
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Default, Clone)]
pub struct Report {
    pub path: String,
    pub duration: f64,
    pub wall: f64,
    pub notes: Vec<String>,
    pub markers: Vec<f64>,
    /// mouse path and clicks: [seconds, x, y, buttons], x and y as shares of the recorded area
    #[serde(skip)]
    pub cursor: Vec<[f32; 4]>,
}

fn mux_segment(seg: &Seg, idx: usize, dir: &Path, notes: &mut Vec<String>) -> Result<PathBuf> {
    let vdur = ffmpeg::duration(&seg.video).unwrap_or(0.0);
    let target = seg.wall.max(0.2);
    let gap = (target - vdur).max(0.0);
    let pad = gap > 0.5; // the picture stopped changing: hold the last frame for the missing time
    let audios: Vec<(&PathBuf, f64)> = [(seg.mic.as_ref(), seg.mic_offset), (seg.sys.as_ref(), seg.sys_offset)]
        .into_iter()
        .filter_map(|(p, o)| p.filter(|p| p.exists()).map(|p| (p, o)))
        .collect();
    if audios.is_empty() && !pad {
        return Ok(seg.video.clone());
    }
    let out = dir.join(format!("m{idx}.mp4"));
    let mut a: Vec<String> = vec!["-i".into(), seg.video.to_string_lossy().into()];
    let mut filters: Vec<String> = vec![];
    for (j, (p, off)) in audios.iter().enumerate() {
        if *off > 0.02 {
            a.extend(["-itsoffset".into(), format!("{off:.3}")]);
        } else if *off < -0.02 {
            a.extend(["-ss".into(), format!("{:.3}", -off)]);
        }
        a.extend(["-i".into(), p.to_string_lossy().into()]);
        filters.push(format!("[{}:a]aresample=48000,apad[a{j}]", j + 1));
    }
    match audios.len() {
        0 => {}
        1 => filters.push("[a0]anull[aout]".into()),
        _ => filters.push("[a0][a1]amix=inputs=2:duration=longest:normalize=0[aout]".into()),
    }
    if pad {
        filters.push(format!("[0:v]tpad=stop_mode=clone:stop_duration={gap:.3}[v]"));
        notes.push(format!("The screen was still for the last {gap:.1} s, so the final picture was held to keep the right length."));
    }
    a.extend(["-filter_complex".into(), filters.join(";")]);
    a.extend(["-map".into(), if pad { "[v]".into() } else { "0:v".into() }]);
    if !audios.is_empty() {
        a.extend(["-map".into(), "[aout]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }
    if pad {
        a.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"].map(String::from));
    } else {
        a.extend(["-c:v".into(), "copy".into()]);
    }
    a.extend(["-t".into(), format!("{target:.3}"), "-movflags".into(), "+faststart".into(), out.to_string_lossy().into()]);
    ffmpeg::run(&a)?;
    Ok(out)
}

pub fn finalize(segs: &[Seg], dir: &Path, out: &Path) -> Result<Report> {
    let mut notes = vec![];
    let mut parts = vec![];
    for (i, s) in segs.iter().enumerate() {
        parts.push(mux_segment(s, i, dir, &mut notes)?);
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    if parts.len() == 1 {
        if std::fs::rename(&parts[0], out).is_err() {
            std::fs::copy(&parts[0], out)?;
        }
    } else {
        let list = dir.join("list.txt");
        let text: String = parts.iter().map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''"))).collect();
        std::fs::write(&list, text)?;
        ffmpeg::run(&["-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), list.to_string_lossy().to_string(), "-c".into(), "copy".into(), "-movflags".into(), "+faststart".into(), out.to_string_lossy().to_string()])?;
    }
    let wall: f64 = segs.iter().map(|s| s.wall).sum();
    let duration = ffmpeg::duration(out).unwrap_or(0.0);
    if duration < wall * 0.96 - 0.5 {
        notes.push(format!("The saved video is {duration:.0} s but {wall:.0} s were recorded. Please press Details and send the log so this can be fixed."));
    }
    Ok(Report { path: out.to_string_lossy().into(), duration, wall, notes, markers: vec![], cursor: vec![] })
}
