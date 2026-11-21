//! Video Builder: a template of scenes (title cards, the person's own clips, an end card) joined into one finished video.
use crate::edit;
use crate::ffmpeg;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Scene {
    /// title | clip | end
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub dur: f64,
    #[serde(default)]
    pub path: String,
    /// remove silences and fast-forward still moments in this clip first
    #[serde(default)]
    pub tidy: bool,
    /// a line shown near the bottom at the start of a clip
    #[serde(default)]
    pub caption: String,
    /// use only this part of the clip (seconds in the file); 0 means no limit
    #[serde(default)]
    pub trim_start: f64,
    #[serde(default)]
    pub trim_end: f64,
    /// playback speed of the clip, 0 or 1 means normal
    #[serde(default)]
    pub speed: f64,
    /// drop the clip's own sound
    #[serde(default)]
    pub mute: bool,
    /// bottom | top | middle
    #[serde(default)]
    pub caption_pos: String,
    /// seconds the caption stays, 0 means automatic
    #[serde(default)]
    pub caption_dur: f64,
    /// card text size multiplier, 0 means normal
    #[serde(default)]
    pub size: f64,
    /// colours for this card only, empty means the brand colours
    #[serde(default)]
    pub bg: String,
    #[serde(default)]
    pub accent: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Brand {
    #[serde(default)]
    pub bg: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub logo: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Project {
    pub title: String,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub brand: Brand,
    /// 16:9 | 9:16 | 1:1
    #[serde(default)]
    pub aspect: String,
    #[serde(default)]
    pub transition: edit::Transition,
    /// bars | blur: what fills the space around a clip that does not match the format
    #[serde(default)]
    pub fit: String,
}

/// dark lettering on a light background, white otherwise
fn text_on(bg: &str) -> &'static str {
    let h = bg.trim_start_matches("0x");
    let v = |i: usize| u32::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    if h.len() == 6 && 0.299 * v(0) + 0.587 * v(2) + 0.114 * v(4) > 160.0 { "0x111111" } else { "white" }
}

fn hex(c: &str, fallback: &str) -> String {
    let h: String = c.trim_start_matches('#').chars().filter(|x| x.is_ascii_hexdigit()).collect();
    format!("0x{}", if h.len() == 6 { h } else { fallback.to_string() })
}

fn size(aspect: &str) -> (u32, u32) {
    match aspect {
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        _ => (1920, 1080),
    }
}

fn f(v: f64) -> String {
    format!("{:.3}", v)
}

fn txt(dir: &Path, name: &str, t: &str) -> Result<String> {
    let p = dir.join(name);
    std::fs::write(&p, t)?;
    Ok(edit::esc_path(&p.to_string_lossy()))
}

/// A clip scene after the optional tidy-up: the file to use and its length.
fn prepare(scene: &Scene, tmp: &Path, i: usize) -> Result<(PathBuf, f64)> {
    let mut src = PathBuf::from(&scene.path);
    let mut dur = ffmpeg::duration(&src).ok_or_else(|| anyhow!("Scene {} has no video yet", i + 1))?;
    let speed = if scene.speed > 0.0 { scene.speed.clamp(0.25, 8.0) } else { 1.0 };
    let a = scene.trim_start.clamp(0.0, dur);
    let b = if scene.trim_end > a + 0.2 { scene.trim_end.min(dur) } else { dur };
    if a > 0.01 || b < dur - 0.01 || (speed - 1.0).abs() > 0.01 {
        let out = tmp.join(format!("cut{i}.mp4"));
        let p = edit::Project { input: scene.path.clone(), clips: vec![edit::Clip { start: a, end: b, speed }], transition: edit::Transition::default(), overlays: vec![], preset: String::new() };
        edit::export_to(&p, &out)?;
        dur = ffmpeg::duration(&out).unwrap_or((b - a) / speed);
        src = out;
    }
    let base = src.to_string_lossy().to_string();
    if !scene.tidy || dur < 8.0 {
        return Ok((src, dur));
    }
    let a = edit::analyze_quick(&src)?;
    let sil: Vec<(f64, f64)> = a.silences.iter().filter(|(s, e)| e - s >= 0.8).map(|(s, e)| (if *s < 0.3 { 0.0 } else { s + 0.15 }, if *e >= dur - 0.3 { dur } else { e - 0.15 })).filter(|(s, e)| e > s).collect();
    let idle: Vec<(f64, f64)> = a.idle.iter().filter(|(s, e)| e - s >= 2.0).map(|(s, e)| (s + 0.4, e - 0.4)).filter(|(s, e)| e > s).collect();
    if sil.is_empty() && idle.is_empty() {
        return Ok((src, dur));
    }
    // cut the silences, then slow-down-free fast-forward the still parts: split at every boundary and classify each piece
    let mut cuts = vec![0.0, dur];
    for (s, e) in sil.iter().chain(idle.iter()) {
        cuts.push(*s);
        cuts.push(*e);
    }
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cuts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    let mut clips = vec![];
    for w in cuts.windows(2) {
        let (s, e) = (w[0], w[1]);
        let m = (s + e) / 2.0;
        if e - s < 0.2 || sil.iter().any(|(a, b)| m >= *a && m <= *b) {
            continue;
        }
        let speed = if idle.iter().any(|(a, b)| m >= *a && m <= *b) { 4.0 } else { 1.0 };
        clips.push(edit::Clip { start: s, end: e, speed });
    }
    if clips.is_empty() {
        return Ok((src, dur));
    }
    let out = tmp.join(format!("tidy{i}.mp4"));
    let p = edit::Project { input: base.clone(), clips, transition: edit::Transition::default(), overlays: vec![], preset: String::new() };
    edit::export_to(&p, &out)?;
    let d = ffmpeg::duration(&out).unwrap_or(dur);
    Ok((out, d))
}

pub fn render(p: &Project, out: &Path) -> Result<()> {
    if p.scenes.is_empty() {
        return Err(anyhow!("Add at least one scene"));
    }
    let tmp = crate::edit::unique_tmp("recora-studio");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let r = render_inner(p, out, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn render_inner(p: &Project, out: &Path, tmp: &Path) -> Result<()> {
    let (w, h) = size(&p.aspect);
    let bg = hex(&p.brand.bg, "101828");
    let accent = hex(&p.brand.accent, "0a84ff");
    let font = edit::font_file().map(|x| format!("fontfile='{x}':")).unwrap_or_default();
    let logo = if !p.brand.logo.is_empty() && Path::new(&p.brand.logo).exists() { Some(p.brand.logo.clone()) } else { None };
    let mut inputs: Vec<String> = vec![];
    let mut g: Vec<String> = vec![];
    let mut lens: Vec<f64> = vec![];
    let mut n_in = 0usize;
    for (i, s) in p.scenes.iter().enumerate() {
        if s.kind == "clip" {
            let (path, dur) = prepare(s, tmp, i)?;
            let has_a = ffmpeg::has_audio(&path) && !s.mute;
            let idx = n_in;
            n_in += 1;
            inputs.extend(["-i".into(), path.to_string_lossy().into()]);
            let mut chain = if p.fit == "blur" {
                format!("[{idx}:v]split[fa{i}][fb{i}];[fa{i}]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},boxblur=24:3,eq=brightness=-0.08[fbg{i}];[fb{i}]scale={w}:{h}:force_original_aspect_ratio=decrease[ffg{i}];[fbg{i}][ffg{i}]overlay=(W-w)/2:(H-h)/2,setsar=1,fps=30,format=yuv420p")
            } else {
                format!("[{idx}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={bg},setsar=1,fps=30,format=yuv420p")
            };
            if !s.caption.trim().is_empty() {
                let tf = txt(tmp, &format!("c{i}.txt"), s.caption.trim())?;
                let end = if s.caption_dur > 0.0 { (0.4 + s.caption_dur).min(dur - 0.1) } else { (dur - 0.3).min(5.0).max(1.0) };
                let y = match s.caption_pos.as_str() { "top" => "h*0.08", "middle" => "(h-text_h)/2", _ => "h*0.84" };
                chain += &format!(",drawtext={font}textfile='{tf}':fontsize=h*{}:fontcolor={}:box=1:boxcolor={accent}@0.9:boxborderw=18:x=(w-text_w)/2:y={y}:enable='between(t,0.4,{})'", if h > w { "0.03" } else { "0.045" }, text_on(&accent), f(end));
            }
            g.push(format!("{chain}[v{i}]"));
            if has_a {
                g.push(format!("[{idx}:a]aresample=48000,aformat=channel_layouts=stereo[a{i}]"));
            } else {
                g.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={}[a{i}]", f(dur)));
            }
            lens.push(dur);
        } else {
            let dur = if s.dur >= 1.0 { s.dur.min(15.0) } else { 3.0 };
            let bg = if s.bg.is_empty() { bg.clone() } else { hex(&s.bg, "101828") };
            let accent = if s.accent.is_empty() { accent.clone() } else { hex(&s.accent, "0a84ff") };
            let ink = text_on(&bg);
            let k = if s.size > 0.0 { s.size.clamp(0.5, 2.0) } else { 1.0 };
            let mut chain = format!("color=c={bg}:s={w}x{h}:r=30:d={},format=yuv420p,setsar=1", f(dur));
            // an accent bar under the title gives the card its own look
            chain += &format!(",drawbox=x=iw*0.42:y=ih*0.57:w=iw*0.16:h=6:color={accent}:t=fill");
            let big = f(if h > w { 0.05 } else { 0.075 } * k);
            if !s.text.trim().is_empty() {
                let tf = txt(tmp, &format!("t{i}.txt"), s.text.trim())?;
                chain += &format!(",drawtext={font}textfile='{tf}':fontsize=h*{big}:fontcolor={ink}:x=(w-text_w)/2:y=h*0.44-text_h/2");
            }
            if !s.sub.trim().is_empty() {
                let tf = txt(tmp, &format!("s{i}.txt"), s.sub.trim())?;
                chain += &format!(",drawtext={font}textfile='{tf}':fontsize=h*{}:fontcolor={ink}@0.75:x=(w-text_w)/2:y=h*0.66", f(0.034 * k));
            }
            if let Some(l) = &logo {
                let idx = n_in;
                n_in += 1;
                inputs.extend(["-loop".into(), "1".into(), "-t".into(), f(dur), "-i".into(), l.clone()]);
                g.push(format!("{chain}[bg{i}]"));
                g.push(format!("[{idx}:v]scale=-2:h*0.13:flags=lanczos,format=rgba[lg{i}]").replace("h*0.13", &format!("{}", (h as f64 * 0.13) as u32)));
                g.push(format!("[bg{i}][lg{i}]overlay=(W-w)/2:H*0.12:shortest=1,format=yuv420p[v{i}]"));
            } else {
                g.push(format!("{chain}[v{i}]"));
            }
            g.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={}[a{i}]", f(dur)));
            lens.push(dur);
        }
    }
    // join the scenes
    let n = p.scenes.len();
    let td = if n > 1 && !p.transition.kind.is_empty() && p.transition.kind != "none" {
        p.transition.dur.clamp(0.2, 1.5).min(lens.iter().cloned().fold(f64::MAX, f64::min) / 2.0)
    } else {
        0.0
    };
    let (vout, aout);
    if n == 1 {
        vout = "v0".to_string();
        aout = "a0".to_string();
    } else if td > 0.0 {
        let kind = match p.transition.kind.as_str() { "slide" => "slideleft", "wipe" => "wipeleft", "zoom" => "zoomin", "circle" => "circleopen", "dissolve" => "dissolve", _ => "fade" };
        let (mut cv, mut ca, mut len) = ("v0".to_string(), "a0".to_string(), lens[0]);
        for k in 1..n {
            g.push(format!("[{cv}][v{k}]xfade=transition={kind}:duration={}:offset={}[x{k}]", f(td), f((len - td).max(0.0))));
            g.push(format!("[{ca}][a{k}]acrossfade=d={}[y{k}]", f(td)));
            cv = format!("x{k}");
            ca = format!("y{k}");
            len += lens[k] - td;
        }
        vout = cv;
        aout = ca;
    } else {
        let ins: String = (0..n).map(|i| format!("[v{i}][a{i}]")).collect();
        g.push(format!("{ins}concat=n={n}:v=1:a=1[cv][ca]"));
        vout = "cv".into();
        aout = "ca".into();
    }
    let mut args: Vec<String> = inputs;
    args.extend(["-filter_complex".to_string(), g.join(";"), "-map".into(), format!("[{vout}]"), "-map".into(), format!("[{aout}]")]);
    args.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"].map(String::from));
    args.push(out.to_string_lossy().into());
    ffmpeg::run(&args)?;
    Ok(())
}

pub fn output_path(dir: &Path, title: &str) -> PathBuf {
    let clean: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { ' ' }).collect::<String>().trim().to_string();
    let name = if clean.is_empty() { "Video".to_string() } else { clean.chars().take(60).collect() };
    let mut p = dir.join(format!("{name}.mp4"));
    let mut i = 2;
    while p.exists() && i < 1000 {
        p = dir.join(format!("{name} ({i}).mp4"));
        i += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dir: &Path, name: &str, secs: u32, audio: bool) -> String {
        let p = dir.join(name);
        let mut a: Vec<String> = ["-f", "lavfi", "-i"].map(String::from).to_vec();
        a.push(format!("testsrc2=size=640x360:rate=30:duration={secs}"));
        if audio {
            a.extend(["-f", "lavfi", "-i"].map(String::from));
            a.push(format!("sine=frequency=440:duration={secs}"));
        }
        a.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p"].map(String::from));
        if audio {
            a.extend(["-c:a", "aac", "-shortest"].map(String::from));
        }
        a.push(p.to_string_lossy().into());
        ffmpeg::run(&a).unwrap();
        p.to_string_lossy().into()
    }
    fn dir(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("recora-studio-test-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn proj(a: String, b: String) -> Project {
        Project {
            title: "Demo".into(),
            aspect: "16:9".into(),
            brand: Brand { bg: "#101828".into(), accent: "#0a84ff".into(), logo: String::new() },
            transition: edit::Transition { kind: "fade".into(), dur: 0.5 },
            fit: String::new(),
            scenes: vec![
                Scene { kind: "title".into(), text: "Product demo: it's 100% new".into(), sub: "LocMasters".into(), dur: 3.0, path: String::new(), tidy: false, caption: String::new(), ..Default::default() },
                Scene { kind: "clip".into(), text: String::new(), sub: String::new(), dur: 0.0, path: a, tidy: false, caption: "Step one".into(), ..Default::default() },
                Scene { kind: "clip".into(), text: String::new(), sub: String::new(), dur: 0.0, path: b, tidy: true, caption: String::new(), ..Default::default() },
                Scene { kind: "end".into(), text: "Thanks".into(), sub: "locmasters.com".into(), dur: 2.0, path: String::new(), tidy: false, caption: String::new(), ..Default::default() },
            ],
        }
    }

    #[test]
    fn builds_a_video_from_scenes_with_and_without_audio_and_logo() {
        if !ffmpeg::available() { return; }
        let d = dir("a");
        let a = make(&d, "a.mp4", 4, true);
        let b = make(&d, "b.mp4", 3, false); // a silent screen recording
        let mut p = proj(a, b);
        let logo = d.join("logo.png");
        ffmpeg::run(&["-f", "lavfi", "-i", "color=c=red:s=200x100:d=1", "-frames:v", "1", logo.to_str().unwrap()]).unwrap();
        p.brand.logo = logo.to_string_lossy().into();
        let out = d.join("out.mp4");
        render(&p, &out).unwrap();
        let dur = ffmpeg::duration(&out).unwrap();
        // 3 + 4 + 3 + 2 minus three transitions of 0.5 s
        assert!((dur - 10.5).abs() < 0.6, "duration {dur}");
        assert!(ffmpeg::has_audio(&out));
    }

    #[test]
    fn vertical_without_transition_and_name_collisions() {
        if !ffmpeg::available() { return; }
        let d = dir("b");
        let a = make(&d, "a.mp4", 3, true);
        let mut p = proj(a.clone(), a);
        p.aspect = "9:16".into();
        p.transition.kind = "none".into();
        let out = output_path(&d, "My: demo?");
        assert!(out.file_name().unwrap().to_string_lossy().starts_with("My  demo"));
        render(&p, &out).unwrap();
        let dur = ffmpeg::duration(&out).unwrap();
        assert!((dur - 11.0).abs() < 0.6, "duration {dur}");
        assert_ne!(output_path(&d, "My: demo?"), out);
    }

    #[test]
    fn a_clip_scene_without_a_video_is_a_clear_error() {
        if !ffmpeg::available() { return; }
        let d = dir("c");
        let mut p = proj(String::new(), String::new());
        p.scenes.retain(|s| s.kind == "clip");
        let e = render(&p, &d.join("x.mp4")).unwrap_err().to_string();
        assert!(e.contains("Scene 1"), "{e}");
    }

    #[test]
    fn trims_speeds_mutes_and_styles_scenes() {
        if !ffmpeg::available() { return; }
        let d = dir("d");
        let a = make(&d, "a.mp4", 6, true);
        let mut p = proj(a.clone(), a);
        p.transition.kind = "none".into();
        p.fit = "blur".into();
        p.aspect = "9:16".into();
        // first clip: seconds 1 to 5 at double speed (2 s), muted, caption on top
        p.scenes[1].trim_start = 1.0; p.scenes[1].trim_end = 5.0; p.scenes[1].speed = 2.0; p.scenes[1].mute = true;
        p.scenes[1].caption_pos = "top".into(); p.scenes[1].caption_dur = 1.0;
        // second clip: only the first 2 seconds, not tidied
        p.scenes[2].tidy = false; p.scenes[2].trim_end = 2.0;
        // light card with bigger text
        p.scenes[0].bg = "#f2f2f7".into(); p.scenes[0].accent = "#ff375f".into(); p.scenes[0].size = 1.3; p.scenes[0].dur = 2.0;
        let out = d.join("o.mp4");
        render(&p, &out).unwrap();
        let dur = ffmpeg::duration(&out).unwrap();
        // 2 + 2 + 2 + 2
        assert!((dur - 8.0).abs() < 0.6, "duration {dur}");
    }
}
