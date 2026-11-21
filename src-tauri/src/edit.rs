//! The editor's engine: a small project description (cuts, speed, transitions, overlays, preset) is turned into one
//! ffmpeg filter graph. The picture is never re-compressed more than once.
use crate::ffmpeg;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone, Debug)]
pub struct Clip {
    pub start: f64,
    pub end: f64,
    #[serde(default = "one")]
    pub speed: f64,
}
fn one() -> f64 {
    1.0
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Transition {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub dur: f64,
}

/// `kind`: text, box, blur. Times are on the edited timeline, positions are fractions of the frame.
#[derive(Deserialize, Clone, Debug)]
pub struct Overlay {
    pub kind: String,
    pub start: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub size: f64,
    #[serde(default)]
    pub color: String,
    /// blur strength 1 to 3 (block size 8, 14, 24); 0 means the middle one
    #[serde(default)]
    pub level: u8,
    /// a dark box behind text; on unless the editor turns it off
    #[serde(default = "yes")]
    pub bg: bool,
}
fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
pub struct Project {
    pub input: String,
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub transition: Transition,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default)]
    pub preset: String,
}

fn xfade_name(k: &str) -> Option<&'static str> {
    Some(match k {
        "fade" => "fade",
        "dissolve" => "dissolve",
        "slide" => "slideleft",
        "wipe" => "wipeleft",
        "zoom" => "zoomin",
        "circle" => "circleopen",
        _ => return None,
    })
}

fn colour(c: &str) -> String {
    let h: String = c.trim_start_matches('#').chars().filter(|x| x.is_ascii_hexdigit()).collect();
    if h.len() == 6 { format!("0x{h}") } else { "0xffffff".into() }
}

fn f(v: f64) -> String {
    format!("{:.3}", v)
}

/// Length of every clip after the speed change.
pub fn lengths(p: &Project) -> Vec<f64> {
    p.clips.iter().map(|c| (c.end - c.start) / c.speed.clamp(0.25, 8.0)).collect()
}

/// Total length of the result, transitions included.
pub fn total(p: &Project) -> f64 {
    let l = lengths(p);
    let td = trans_dur(p, &l);
    l.iter().sum::<f64>() - td * (l.len().saturating_sub(1)) as f64
}

fn trans_dur(p: &Project, l: &[f64]) -> f64 {
    if xfade_name(&p.transition.kind).is_none() || l.len() < 2 {
        return 0.0;
    }
    let min = l.iter().cloned().fold(f64::MAX, f64::min);
    p.transition.dur.clamp(0.1, 2.0).min(min / 2.0)
}

/// Writes the text of an overlay to a file so no character needs escaping in the filter graph.
fn text_file(dir: &Path, i: usize, t: &str) -> Result<String> {
    let p = dir.join(format!("t{i}.txt"));
    std::fs::write(&p, t)?;
    Ok(esc_path(&p.to_string_lossy()))
}

pub(crate) fn esc_path(s: &str) -> String {
    s.replace('\\', "/").replace(':', "\\:").replace('\'', "\\'")
}

pub(crate) fn font_file() -> Option<String> {
    let cands = [
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/tahoma.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ];
    cands.iter().find(|p| Path::new(p).exists()).map(|s| esc_path(s))
}

/// Builds the ffmpeg arguments (without the output file) and returns them with the output extension.
pub fn build(p: &Project, has_audio: bool, fps: f64, tmp: &Path) -> Result<(Vec<String>, &'static str)> {
    if p.clips.is_empty() {
        return Err(anyhow!("Nothing left to export"));
    }
    for c in &p.clips {
        if c.end <= c.start + 0.05 {
            return Err(anyhow!("A piece of the video is shorter than 0.05 s"));
        }
    }
    let has_audio = has_audio && p.preset != "gif";
    let n = p.clips.len();
    let l = lengths(p);
    let td = trans_dur(p, &l);
    let rate = fps.clamp(10.0, 60.0);
    let mut g: Vec<String> = vec![];
    for (i, c) in p.clips.iter().enumerate() {
        let sp = c.speed.clamp(0.25, 8.0);
        g.push(format!("[0:v]trim=start={}:end={},setpts=(PTS-STARTPTS)/{},fps={},format=yuv420p,setsar=1[v{i}]", f(c.start), f(c.end), f(sp), f(rate)));
        if has_audio {
            // atempo takes 0.5 to 2 per stage
            let mut tempo = vec![];
            let mut s = sp;
            while s > 2.0 { tempo.push(2.0); s /= 2.0; }
            while s < 0.5 { tempo.push(0.5); s /= 0.5; }
            tempo.push(s);
            let t = tempo.iter().map(|x| format!("atempo={}", f(*x))).collect::<Vec<_>>().join(",");
            g.push(format!("[0:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{},aresample=48000[a{i}]", f(c.start), f(c.end), t));
        }
    }
    // join the pieces
    let mut cur_v = "v0".to_string();
    let mut cur_a = "a0".to_string();
    if n == 1 {
        // nothing to join
    } else if td > 0.0 {
        let kind = xfade_name(&p.transition.kind).unwrap();
        let mut len = l[0];
        for k in 1..n {
            let (nv, na) = (format!("x{k}"), format!("y{k}"));
            g.push(format!("[{cur_v}][v{k}]xfade=transition={kind}:duration={}:offset={}[{nv}]", f(td), f((len - td).max(0.0))));
            if has_audio {
                g.push(format!("[{cur_a}][a{k}]acrossfade=d={}[{na}]", f(td)));
                cur_a = na;
            }
            cur_v = nv;
            len += l[k] - td;
        }
    } else {
        let ins: String = (0..n).map(|i| if has_audio { format!("[v{i}][a{i}]") } else { format!("[v{i}]") }).collect();
        g.push(format!("{ins}concat=n={n}:v=1:a={}[cv]{}", if has_audio { 1 } else { 0 }, if has_audio { "[ca]" } else { "" }));
        cur_v = "cv".into();
        cur_a = "ca".into();
    }
    // overlays on the edited timeline
    let font = font_file();
    for (i, o) in p.overlays.iter().enumerate() {
        let en = format!("enable='between(t,{},{})'", f(o.start), f(o.end.max(o.start + 0.1)));
        let next = format!("o{i}");
        match o.kind.as_str() {
            "text" if !o.text.trim().is_empty() => {
                let tf = text_file(tmp, i, &o.text)?;
                let size = if o.size > 0.0 { o.size } else { 0.06 };
                let ff = font.as_ref().map(|x| format!("fontfile='{x}':")).unwrap_or_default();
                g.push(format!("[{cur_v}]drawtext={ff}textfile='{tf}':fontsize=h*{}:fontcolor={}:box={}:boxcolor=black@0.55:boxborderw=14:x=w*{}:y=h*{}:{en}[{next}]", f(size), colour(&o.color), if o.bg { 1 } else { 0 }, f(o.x), f(o.y)));
            }
            "box" => {
                g.push(format!("[{cur_v}]drawbox=x=iw*{}:y=ih*{}:w=iw*{}:h=ih*{}:color={}:t=4:{en}[{next}]", f(o.x), f(o.y), f(o.w), f(o.h), colour(if o.color.is_empty() { "#ffcc00" } else { &o.color })));
            }
            "blur" => {
                // pixelate first so text cannot be read, then soften the blocks
                let block = match o.level { 1 => 8, 3 => 24, _ => 14 };
                let (a, b) = (format!("ba{i}"), format!("bb{i}"));
                g.push(format!("[{cur_v}]split[{a}][{b}]"));
                g.push(format!("[{b}]crop=trunc(iw*{}/2)*2:trunc(ih*{}/2)*2:iw*{}:ih*{},scale=iw/{blk}:ih/{blk},scale=iw*{blk}:ih*{blk}:flags=neighbor,boxblur={rad}:2[bl{i}]", f(o.w), f(o.h), f(o.x), f(o.y), blk = block, rad = block * 2 / 3));
                g.push(format!("[{a}][bl{i}]overlay=x=W*{}:y=H*{}:{en}[{next}]", f(o.x), f(o.y)));
            }
            _ => continue,
        }
        cur_v = next;
    }
    // the preset decides the final size and container
    let (post, vcodec, ext): (String, Vec<&str>, &str) = match p.preset.as_str() {
        "gif" => (format!("fps=12,scale=720:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[pp];[s1][pp]paletteuse=dither=bayer:bayer_scale=4"), vec![], "gif"),
        "small" => ("scale=-2:720,format=yuv420p".into(), vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "28"], "mp4"),
        "youtube" => ("scale=-2:'min(1080,ih)',format=yuv420p".into(), vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"], "mp4"),
        "vertical" => ("split[sa][sb];[sa]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=30:5[bgv];[sb]scale=1080:-2[fgv];[bgv][fgv]overlay=(W-w)/2:(H-h)/2,format=yuv420p".into(), vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"], "mp4"),
        _ => ("scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p".into(), vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"], "mp4"),
    };
    g.push(format!("[{cur_v}]{post}[vout]"));
    let mut a: Vec<String> = vec!["-i".into(), p.input.clone(), "-filter_complex".into(), g.join(";"), "-map".into(), "[vout]".into()];
    if has_audio && ext != "gif" {
        a.extend(["-map".to_string(), format!("[{cur_a}]")]);
        a.extend(["-c:a", "aac", "-b:a", "192k"].map(String::from));
    }
    a.extend(vcodec.iter().map(|s| s.to_string()));
    if ext == "mp4" {
        a.extend(["-movflags", "+faststart"].map(String::from));
    }
    if ext == "gif" {
        a.extend(["-loop", "0"].map(String::from));
    }
    Ok((a, ext))
}

/// A scratch folder name no other job in this process shares.
pub(crate) fn unique_tmp(prefix: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
}

fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    (2..1000).map(|i| path.with_file_name(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists()).unwrap_or(path)
}

pub fn export(p: &Project) -> Result<PathBuf> {
    let input = Path::new(&p.input);
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let suffix = match p.preset.as_str() { "gif" => " (edited GIF)", "small" => " (edited, small)", "vertical" => " (edited, vertical)", "youtube" => " (edited, YouTube)", _ => " (edited)" };
    let ext = if p.preset == "gif" { "gif" } else { "mp4" };
    let out = unique(input.with_file_name(format!("{stem}{suffix}.{ext}")));
    export_to(p, &out)?;
    Ok(out)
}

/// Same, to a path of the caller's choosing.
pub fn export_to(p: &Project, out: &Path) -> Result<()> {
    let input = Path::new(&p.input);
    let tmp = unique_tmp("recora-edit");
    std::fs::create_dir_all(&tmp)?;
    let fps = ffmpeg::fps(input).unwrap_or(30.0);
    let (mut args, _ext) = build(p, ffmpeg::has_audio(input), fps, &tmp)?;
    args.push(out.to_string_lossy().into());
    let r = ffmpeg::run(&args);
    let _ = std::fs::remove_dir_all(&tmp);
    r?;
    Ok(())
}

/// One picture of `n` thumbnails side by side for the timeline.
pub fn strip(path: &Path, work: &Path, n: u32) -> Result<PathBuf> {
    let dur = ffmpeg::duration(path).unwrap_or(1.0).max(0.5);
    let key = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let stem: String = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let out = work.join(format!("{stem}_{key}_{n}.jpg"));
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(work)?;
    ffmpeg::run(&["-i".to_string(), path.to_string_lossy().into(), "-vf".into(), format!("fps={:.5},scale=-2:72,tile={n}x1", n as f64 / dur), "-frames:v".into(), "1".into(), "-q:v".into(), "5".into(), out.to_string_lossy().into()])?;
    Ok(out)
}

/// What the smart tools found in a video, all in seconds of the source.
#[derive(serde::Serialize, Default, Debug)]
pub struct Analysis {
    pub silences: Vec<(f64, f64)>,
    pub idle: Vec<(f64, f64)>,
    pub scenes: Vec<f64>,
}

fn num_after(line: &str, key: &str) -> Option<f64> {
    let i = line.find(key)? + key.len();
    line[i..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect::<String>().parse().ok()
}

fn pairs(text: &str, start_key: &str, end_key: &str, total: f64) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let mut open: Option<f64> = None;
    for line in text.lines() {
        if let Some(s) = num_after(line, start_key) {
            open = Some(s.max(0.0));
        } else if let Some(e) = num_after(line, end_key) {
            if let Some(s) = open.take() {
                if e > s {
                    out.push((s, e));
                }
            }
        }
    }
    if let Some(s) = open {
        if total > s {
            out.push((s, total));
        }
    }
    out
}

fn scene_times(text: &str) -> Vec<f64> {
    text.lines().filter(|l| l.contains("showinfo")).filter_map(|l| num_after(l, "pts_time:")).collect()
}

/// Pauses and still-screen only, on a shrunken picture and at low priority. This is what the autopilot runs in the background.
pub fn analyze_quick(path: &Path) -> Result<Analysis> {
    detect(path, false)
}

/// Looks for silent stretches, a screen that does not change, and scene changes.
pub fn analyze(path: &Path) -> Result<Analysis> {
    detect(path, true)
}

fn detect(path: &Path, scenes_too: bool) -> Result<Analysis> {
    let total = ffmpeg::duration(path).unwrap_or(0.0);
    let p = path.to_string_lossy().to_string();
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-nostats".into(), "-i".into(), p.clone(), "-vf".into(), "scale=320:-2,freezedetect=n=-55dB:d=2".into()];
    if ffmpeg::has_audio(path) {
        a.extend(["-af".to_string(), "silencedetect=n=-35dB:d=0.8".to_string()]);
    }
    a.extend(["-f", "null", "-"].map(String::from));
    let out = if scenes_too { ffmpeg::cmd() } else { ffmpeg::cmd_low() }.args(&a).output()?;
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(anyhow!("analysis failed: {}", text.lines().last().unwrap_or("")));
    }
    if !scenes_too {
        return Ok(Analysis { silences: pairs(&text, "silence_start:", "silence_end:", total), idle: pairs(&text, "freeze_start:", "freeze_end:", total), scenes: vec![] });
    }
    let out2 = ffmpeg::cmd().args(["-hide_banner", "-nostats", "-i", &p, "-an", "-vf", "select=gt(scene\\,0.3),showinfo", "-f", "null", "-"]).output()?;
    let mut scenes = scene_times(&String::from_utf8_lossy(&out2.stderr));
    // keep scene changes at least 2 s apart
    let mut last = -10.0;
    scenes.retain(|t| { let keep = *t - last >= 2.0 && *t > 0.5; if keep { last = *t; } keep });
    Ok(Analysis { silences: pairs(&text, "silence_start:", "silence_end:", total), idle: pairs(&text, "freeze_start:", "freeze_end:", total), scenes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> String {
        let p = dir.join("src.mp4");
        ffmpeg::run(&["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=6", "-f", "lavfi", "-i", "sine=frequency=440:duration=6", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", p.to_str().unwrap()]).unwrap();
        p.to_string_lossy().into()
    }
    fn proj(input: String) -> Project {
        Project { input, clips: vec![Clip { start: 0.0, end: 2.0, speed: 1.0 }, Clip { start: 3.0, end: 6.0, speed: 2.0 }], transition: Transition { kind: "fade".into(), dur: 0.5 }, overlays: vec![], preset: String::new() }
    }

    #[test]
    fn length_math() {
        let p = proj("x".into());
        assert_eq!(lengths(&p), vec![2.0, 1.5]);
        assert!((total(&p) - 3.0).abs() < 1e-9);
        let mut q = p.clone();
        q.transition.kind = "none".into();
        assert!((total(&q) - 3.5).abs() < 1e-9);
    }

    #[test]
    fn rejects_empty_and_tiny() {
        let mut p = proj("x".into());
        p.clips.clear();
        assert!(build(&p, true, 30.0, Path::new(".")).is_err());
        let mut p = proj("x".into());
        p.clips[0].end = 0.01;
        assert!(build(&p, true, 30.0, Path::new(".")).is_err());
    }

    #[test]
    fn exports_cuts_speed_transition_and_overlays() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("edit1");
        let mut p = proj(sample(&d));
        p.overlays = vec![
            Overlay { kind: "text".into(), start: 0.2, end: 2.0, x: 0.1, y: 0.8, w: 0.0, h: 0.0, text: "Hello: it's 100% fine".into(), size: 0.06, color: "#ffffff".into(), level: 0, bg: false },
            Overlay { kind: "box".into(), start: 0.0, end: 3.0, x: 0.1, y: 0.1, w: 0.3, h: 0.2, text: String::new(), size: 0.0, color: "#ff3b30".into(), level: 0, bg: true },
            Overlay { kind: "blur".into(), start: 0.0, end: 3.0, x: 0.5, y: 0.5, w: 0.3, h: 0.3, text: String::new(), size: 0.0, color: String::new(), level: 3, bg: true },
        ];
        let out = export(&p).unwrap();
        let dur = ffmpeg::duration(&out).unwrap();
        assert!((dur - 3.0).abs() < 0.3, "duration {dur}");
        assert!(ffmpeg::has_audio(&out));
    }

    #[test]
    fn exports_presets_without_audio_and_single_clip() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("edit2");
        let src = sample(&d);
        for preset in ["gif", "small", "vertical", "youtube"] {
            let mut p = proj(src.clone());
            p.clips = vec![Clip { start: 1.0, end: 3.0, speed: 0.5 }];
            p.preset = preset.into();
            let out = export(&p).unwrap();
            assert!(out.exists(), "{preset}");
            let dur = ffmpeg::duration(&out).unwrap();
            assert!((dur - 4.0).abs() < 0.6, "{preset} duration {dur}");
        }
    }

    #[test]
    fn parses_detector_output() {
        let t = "[silencedetect @ 0x1] silence_start: 3.5\n[silencedetect @ 0x1] silence_end: 7.25 | silence_duration: 3.75\n[silencedetect @ 0x1] silence_start: 9";
        assert_eq!(pairs(t, "silence_start:", "silence_end:", 12.0), vec![(3.5, 7.25), (9.0, 12.0)]);
        let f = "[freezedetect @ 0x2] lavfi.freezedetect.freeze_start: 1.5\n[freezedetect @ 0x2] lavfi.freezedetect.freeze_end: 4";
        assert_eq!(pairs(f, "freeze_start:", "freeze_end:", 5.0), vec![(1.5, 4.0)]);
        assert_eq!(scene_times("[Parsed_showinfo_1 @ 0x3] n:   0 pts:  90 pts_time:3.0 pos: 1"), vec![3.0]);
    }

    #[test]
    fn finds_silence_idle_and_scene() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("edit4");
        let p = d.join("a.mp4");
        ffmpeg::run(&["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=3", "-f", "lavfi", "-i", "color=c=black:s=320x240:r=30:d=4", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=3", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono:d=4", "-filter_complex", "[0:v]setsar=1[a];[1:v]setsar=1[b];[a][b]concat=n=2:v=1:a=0[v];[2:a][3:a]concat=n=2:v=0:a=1[o]", "-map", "[v]", "-map", "[o]", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", p.to_str().unwrap()]).unwrap();
        let q = analyze_quick(&p).unwrap();
        assert!(q.scenes.is_empty() && q.silences.iter().any(|(s, _)| *s > 2.5) && q.idle.iter().any(|(s, _)| *s > 2.5), "{:?}", q);
        let a = analyze(&p).unwrap();
        assert!(a.silences.iter().any(|(s, e)| *s > 2.5 && *s < 3.6 && *e > 6.0), "{:?}", a.silences);
        assert!(a.idle.iter().any(|(s, e)| *s > 2.5 && *s < 4.0 && *e > 6.0), "{:?}", a.idle);
        assert!(a.scenes.iter().any(|t| (*t - 3.0).abs() < 0.5), "{:?}", a.scenes);
    }

    #[test]
    fn makes_a_strip() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("edit3");
        let src = sample(&d);
        let s = strip(Path::new(&src), &d.join("strips"), 8).unwrap();
        assert!(s.metadata().unwrap().len() > 1000);
    }

    fn tempfile_dir(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("recora-test-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
