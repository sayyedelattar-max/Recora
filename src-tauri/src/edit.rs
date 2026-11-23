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

/// `kind`: text, box, blur, image. Times are on the edited timeline, positions are fractions of the frame.
#[derive(Deserialize, Clone, Debug, Default)]
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
    /// rect | round | ellipse | arrow | line | spot
    #[serde(default)]
    pub shape: String,
    /// how much the inside of a highlight is filled, 0 to 1
    #[serde(default)]
    pub fill: f64,
    /// outline thickness as a fraction of the frame height
    #[serde(default)]
    pub thick: f64,
    #[serde(default)]
    pub flipx: bool,
    #[serde(default)]
    pub flipy: bool,
    /// text: plain | box | outline | banner
    #[serde(default)]
    pub style: String,
    /// colour of the box or banner behind text
    #[serde(default)]
    pub color2: String,
    /// blur: pixel | soft | solid
    #[serde(default)]
    pub mode: String,
    /// image file
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub opacity: f64,
    /// none | fade | slide
    #[serde(default)]
    pub anim: String,
}

/// Sound laid over the video. Times are on the edited timeline.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Sound {
    /// file | fx
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub fx: String,
    pub start: f64,
    /// 0 plays it to its natural end
    #[serde(default)]
    pub end: f64,
    /// 0 means normal
    #[serde(default)]
    pub vol: f64,
    #[serde(default)]
    pub fade: f64,
    /// repeat to fill the time (music)
    #[serde(default)]
    pub repeat: bool,
    /// background music: it is lowered while someone speaks when ducking is on
    #[serde(default)]
    pub music: bool,
}

/// A title card added before or after the video.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Card {
    pub text: String,
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub dur: f64,
    #[serde(default)]
    pub bg: String,
    #[serde(default)]
    pub accent: String,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Project {
    pub input: String,
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub transition: Transition,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub sounds: Vec<Sound>,
    #[serde(default)]
    pub intro: Option<Card>,
    #[serde(default)]
    pub outro: Option<Card>,
    /// clean steady background noise from the recording's own sound
    #[serde(default)]
    pub denoise: bool,
    /// lower the music while the recording's own sound has speech
    #[serde(default)]
    pub duck: bool,
    /// seconds of fade from and to black at the start and end (sound fades too)
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    /// size of the source picture; filled in on export
    #[serde(default)]
    pub vw: u32,
    #[serde(default)]
    pub vh: u32,
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

fn rgb(c: &str) -> [u8; 3] {
    let h = colour(c);
    let h = h.trim_start_matches("0x");
    let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(255);
    [v(0), v(2), v(4)]
}

fn luminance(c: &str) -> f64 {
    let [r, g, b] = rgb(c);
    0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
}

/// A title card as a video segment (and a silent sound segment when the video has sound). Returns its length.
fn card_seg(g: &mut Vec<String>, c: &Card, tag: &str, tmp: &Path, size: (u32, u32), rate: f64, with_audio: bool) -> Result<f64> {
    let d = if c.dur >= 1.0 { c.dur.min(10.0) } else { 3.0 };
    let bg = if c.bg.is_empty() { "#101828" } else { &c.bg };
    let ac = if c.accent.is_empty() { "#0a84ff" } else { &c.accent };
    let ink = if luminance(bg) > 160.0 { "0x111111" } else { "white" };
    let ff = font_file().map(|x| format!("fontfile='{x}':")).unwrap_or_default();
    let (w, h) = size;
    let big = if h > w { 0.05 } else { 0.075 };
    let mut chain = format!("color=c={}:s={w}x{h}:r={}:d={},format=yuv420p,setsar=1,drawbox=x=iw*0.42:y=ih*0.57:w=iw*0.16:h=6:color={}:t=fill", colour(bg), f(rate), f(d), colour(ac));
    if !c.text.trim().is_empty() {
        let tf = text_file(tmp, 900 + tag.len() * 7 + tag.as_bytes()[0] as usize, &c.text)?;
        chain += &format!(",drawtext={ff}textfile='{tf}':fontsize=h*{}:fontcolor={ink}:x=(w-text_w)/2:y=h*0.44-text_h/2", f(big));
    }
    if !c.sub.trim().is_empty() {
        let tf = text_file(tmp, 950 + tag.len() * 7 + tag.as_bytes()[0] as usize, &c.sub)?;
        chain += &format!(",drawtext={ff}textfile='{tf}':fontsize=h*0.034:fontcolor={ink}@0.75:x=(w-text_w)/2:y=h*0.66");
    }
    g.push(format!("{chain}[v{tag}]"));
    if with_audio {
        g.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={},asetpts=PTS-STARTPTS[a{tag}]", f(d)));
    }
    Ok(d)
}

/// Sound effects and music made on the spot, so nothing needs a licence. Returns the ffmpeg source and its length.
pub fn fx_source(name: &str) -> Option<(String, f64)> {
    let e = |expr: &str, d: f64| (format!("aevalsrc={expr}:s=48000:d={d}"), d);
    Some(match name {
        "click" => e("0.9*sin(2*PI*1800*t)*exp(-t*90)", 0.12),
        "pop" => e("0.9*sin(2*PI*600*t)*exp(-t*28)", 0.3),
        "ding" => e("0.5*(sin(2*PI*1319*t)+0.45*sin(2*PI*2637*t))*exp(-t*4.5)", 1.4),
        "success" => e("0.5*sin(2*PI*if(lt(t\\,0.14)\\,659\\,if(lt(t\\,0.28)\\,880\\,1175))*t)*exp(-mod(t\\,0.14)*7)", 0.6),
        "notify" => e("0.5*sin(2*PI*if(lt(t\\,0.12)\\,988\\,1319)*t)*exp(-mod(t\\,0.12)*9)", 0.5),
        "whoosh" => ("anoisesrc=d=0.8:c=pink:r=48000:a=0.7,highpass=f=250,lowpass=f=5000,afade=t=in:d=0.4,afade=t=out:st=0.4:d=0.4".into(), 0.8),
        "pad" => e("0.12*(sin(2*PI*130.81*t)+sin(2*PI*196*t)+sin(2*PI*261.63*t)+0.7*sin(2*PI*329.63*t)+0.5*sin(2*PI*392*t))*(0.75+0.25*sin(2*PI*0.1*t))", 20.0),
        "pulse" => e("0.3*sin(2*PI*if(lt(mod(t\\,2)\\,0.5)\\,261.63\\,if(lt(mod(t\\,2)\\,1)\\,329.63\\,if(lt(mod(t\\,2)\\,1.5)\\,392\\,329.63)))*t)*exp(-mod(t\\,0.5)*3)", 16.0),
        _ => return None,
    })
}

/// Lays a PNG over the picture for a time range, with an optional fade or slide in and out.
fn png_layer(g: &mut Vec<String>, extra: &mut Vec<String>, n_in: &mut usize, png_path: &str, rate: f64, (t0, t1): (f64, f64), at: (String, String), o: &Overlay, vh: u32, cur: &str, next: &str, i: usize, pre: &str) {
    let idx = *n_in;
    *n_in += 1;
    extra.extend(["-loop".into(), "1".into(), "-framerate".into(), f(rate), "-t".into(), f(t1 + 0.1), "-i".into(), png_path.to_string()]);
    let mut ch = format!("[{idx}:v]{pre}format=rgba");
    if o.opacity > 0.0 && o.opacity < 1.0 {
        ch += &format!(",colorchannelmixer=aa={}", f(o.opacity));
    }
    let animated = o.anim == "fade" || o.anim == "slide";
    if animated {
        let d = 0.3f64.min((t1 - t0) / 3.0);
        ch += &format!(",fade=t=in:st={}:d={}:alpha=1,fade=t=out:st={}:d={}:alpha=1", f(t0), f(d), f((t1 - d).max(t0)), f(d));
    }
    g.push(format!("{ch}[ly{i}]"));
    let y = if o.anim == "slide" { format!("{}+(1-min(1\\,max(0\\,(t-{})/0.35)))*{}", at.1, f(t0), f(vh as f64 * 0.05)) } else { at.1.clone() };
    g.push(format!("[{cur}][ly{i}]overlay=x={}:y='{}':enable='between(t,{},{})':eof_action=pass[{next}]", at.0, y, f(t0), f(t1)));
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
    let gif = p.preset == "gif";
    let has_audio = has_audio && !gif;
    let (vw, vh) = if p.vw > 1 && p.vh > 1 { (p.vw & !1, p.vh & !1) } else { (1920, 1080) };
    let n = p.clips.len();
    let l = lengths(p);
    let rate = fps.clamp(10.0, 60.0);
    let mut g: Vec<String> = vec![];
    let mut extra: Vec<String> = vec![];
    let mut n_in = 1usize;
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
            g.push(format!("[0:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{},aresample=48000,aformat=channel_layouts=stereo[a{i}]", f(c.start), f(c.end), t));
        }
    }
    // the pieces, with an optional title card before and after
    let (mut nv, mut na, mut sl): (Vec<String>, Vec<String>, Vec<f64>) = (vec![], vec![], vec![]);
    let mut intro_len = 0.0;
    if let Some(c) = &p.intro {
        intro_len = card_seg(&mut g, c, "ci", tmp, (vw, vh), rate, has_audio)?;
        nv.push("vci".into()); na.push("aci".into()); sl.push(intro_len);
    }
    for i in 0..n { nv.push(format!("v{i}")); na.push(format!("a{i}")); sl.push(l[i]); }
    if let Some(c) = &p.outro {
        let d = card_seg(&mut g, c, "co", tmp, (vw, vh), rate, has_audio)?;
        nv.push("vco".into()); na.push("aco".into()); sl.push(d);
    }
    let m = nv.len();
    let td = trans_dur(p, &sl);
    let total_len = sl.iter().sum::<f64>() - td * (m.saturating_sub(1)) as f64;
    // overlays and sounds are placed on the edited video, so they move later by the title card
    let off = if p.intro.is_some() { intro_len - td } else { 0.0 };
    let mut cur_v = nv[0].clone();
    let mut cur_a = na[0].clone();
    if m == 1 {
        // nothing to join
    } else if td > 0.0 {
        let kind = xfade_name(&p.transition.kind).unwrap();
        let mut len = sl[0];
        for k in 1..m {
            let (xv, xa) = (format!("x{k}"), format!("y{k}"));
            g.push(format!("[{cur_v}][{}]xfade=transition={kind}:duration={}:offset={}[{xv}]", nv[k], f(td), f((len - td).max(0.0))));
            if has_audio {
                g.push(format!("[{cur_a}][{}]acrossfade=d={}[{xa}]", na[k], f(td)));
                cur_a = xa;
            }
            cur_v = xv;
            len += sl[k] - td;
        }
    } else {
        let ins: String = (0..m).map(|i| if has_audio { format!("[{}][{}]", nv[i], na[i]) } else { format!("[{}]", nv[i]) }).collect();
        g.push(format!("{ins}concat=n={m}:v=1:a={}[cv]{}", if has_audio { 1 } else { 0 }, if has_audio { "[ca]" } else { "" }));
        cur_v = "cv".into();
        cur_a = "ca".into();
    }
    if has_audio && p.denoise {
        g.push(format!("[{cur_a}]highpass=f=70,afftdn=nr=14:nf=-28:tn=1[adn]"));
        cur_a = "adn".into();
    }
    // overlays on the edited timeline
    let font = font_file();
    for (i, o) in p.overlays.iter().enumerate() {
        let (t0, t1) = (o.start + off, (o.end + off).max(o.start + off + 0.1));
        let en = format!("enable='between(t,{},{})'", f(t0), f(t1));
        let next = format!("o{i}");
        let (vwf, vhf) = (vw as f64, vh as f64);
        let anim = o.anim == "fade" || o.anim == "slide" || o.anim == "pop";
        let fade_a = format!("alpha='min(1\\,max(0\\,(t-{})/0.3))*min(1\\,max(0\\,({}-t)/0.3))'", f(t0), f(t1));
        match o.kind.as_str() {
            "text" if !o.text.trim().is_empty() => {
                let tf = text_file(tmp, i, &o.text)?;
                let size = if o.size > 0.0 { o.size } else { 0.06 };
                let ff = font.as_ref().map(|x| format!("fontfile='{x}':")).unwrap_or_default();
                let style = if !o.style.is_empty() { o.style.as_str() } else if o.bg { "box" } else { "plain" };
                let boxc = if o.color2.is_empty() { "0x000000".to_string() } else { colour(&o.color2) };
                let mut chain = format!("[{cur_v}]");
                let mut ty = format!("h*{}", f(o.y));
                if o.anim == "slide" {
                    ty = format!("h*{}+(1-min(1\\,max(0\\,(t-{})/0.35)))*h*0.04", f(o.y), f(t0));
                }
                let fs = if o.anim == "pop" { format!("'h*{}*(0.6+0.4*min(1\\,max(0\\,(t-{})/0.25)))'", f(size), f(t0)) } else { format!("h*{}", f(size)) };
                let mut dt = format!("drawtext={ff}textfile='{tf}':fontsize={fs}:fontcolor={}:x=w*{}:y='{ty}'", colour(&o.color), f(o.x));
                match style {
                    "box" => dt += &format!(":box=1:boxcolor={boxc}@0.6:boxborderw=14"),
                    "outline" => dt += &format!(":borderw={}:bordercolor=black", ((vhf * 0.004).round()).max(2.0)),
                    "banner" => {
                        let pad = vhf * size * 0.45;
                        let bc = if o.color2.is_empty() { "0x0a84ff".to_string() } else { colour(&o.color2) };
                        chain += &format!("drawbox=x=0:y=ih*{}-{}:w=iw:h=ih*{}+{}:color={bc}@0.92:t=fill:{en},", f(o.y), f(pad), f(size * 1.25), f(pad * 2.0 - vhf * size * 0.25));
                    }
                    _ => dt += ":shadowcolor=black@0.7:shadowx=2:shadowy=2",
                }
                if anim { dt += &format!(":{fade_a}"); }
                g.push(format!("{chain}{dt}:{en}[{next}]"));
            }
            "box" => {
                let shape = if o.shape.is_empty() { "rect" } else { o.shape.as_str() };
                let col = if o.color.is_empty() { "#ffcc00" } else { &o.color };
                let path = tmp.join(format!("sh{i}.png"));
                let thick = (if o.thick > 0.0 { o.thick } else { 0.006 } * vhf).max(2.0);
                if shape == "spot" {
                    let dim = if o.fill > 0.0 { o.fill.min(0.9) } else { 0.65 };
                    std::fs::write(&path, crate::shapes::spotlight(vw, vh, o.x * vwf, o.y * vhf, (o.w * vwf).max(8.0), (o.h * vhf).max(8.0), if o.shape == "spot" { if o.mode == "rect" { "rect" } else { "ellipse" } } else { "ellipse" }, dim)?)?;
                    png_layer(&mut g, &mut extra, &mut n_in, &esc_in(&path), rate, (t0, t1), ("0".into(), "0".into()), o, vh, &cur_v, &next, i, "");
                } else {
                    let (wpx, hpx) = (((o.w * vwf / 2.0).round() as u32 * 2).max(4), ((o.h * vhf / 2.0).round() as u32 * 2).max(4));
                    std::fs::write(&path, crate::shapes::png(&crate::shapes::Spec { shape, w: wpx, h: hpx, rgb: rgb(col), fill: o.fill.clamp(0.0, 1.0), outline: true, thick, flipx: o.flipx, flipy: o.flipy, mask: false })?)?;
                    png_layer(&mut g, &mut extra, &mut n_in, &esc_in(&path), rate, (t0, t1), (format!("W*{}", f(o.x)), format!("H*{}", f(o.y))), o, vh, &cur_v, &next, i, "");
                }
            }
            "blur" => {
                let shape = if o.shape.is_empty() { "rect" } else { o.shape.as_str() };
                let (wpx, hpx) = (((o.w * vwf / 2.0).round() as u32 * 2).max(4), ((o.h * vhf / 2.0).round() as u32 * 2).max(4));
                let (xpx, ypx) = ((o.x * vwf).round() as u32, (o.y * vhf).round() as u32);
                let mode = if o.mode.is_empty() { "pixel" } else { o.mode.as_str() };
                if mode == "solid" {
                    let path = tmp.join(format!("sh{i}.png"));
                    let col = if o.color.is_empty() { "#000000" } else { &o.color };
                    std::fs::write(&path, crate::shapes::png(&crate::shapes::Spec { shape, w: wpx, h: hpx, rgb: rgb(col), fill: 1.0, outline: false, thick: 2.0, flipx: false, flipy: false, mask: false })?)?;
                    png_layer(&mut g, &mut extra, &mut n_in, &esc_in(&path), rate, (t0, t1), (xpx.to_string(), ypx.to_string()), &Overlay { anim: String::new(), opacity: 0.0, ..o.clone() }, vh, &cur_v, &next, i, "");
                } else {
                    let block = match o.level { 1 => 8, 3 => 24, _ => 14 };
                    let sigma = match o.level { 1 => 6, 3 => 22, _ => 12 };
                    let (a, b) = (format!("ba{i}"), format!("bb{i}"));
                    g.push(format!("[{cur_v}]split[{a}][{b}]"));
                    let fx = if mode == "soft" { format!("gblur=sigma={sigma}") } else { format!("scale=iw/{block}:ih/{block},scale=iw*{block}:ih*{block}:flags=neighbor,scale={wpx}:{hpx}:flags=neighbor,boxblur={}:2", block * 2 / 3) };
                    if shape == "rect" {
                        g.push(format!("[{b}]crop={wpx}:{hpx}:{xpx}:{ypx},{fx}[bl{i}]"));
                    } else {
                        let path = tmp.join(format!("mk{i}.png"));
                        std::fs::write(&path, crate::shapes::png(&crate::shapes::Spec { shape, w: wpx, h: hpx, rgb: [255; 3], fill: 1.0, outline: false, thick: 2.0, flipx: false, flipy: false, mask: true })?)?;
                        let idx = n_in;
                        n_in += 1;
                        extra.extend(["-loop".into(), "1".into(), "-framerate".into(), f(rate), "-t".into(), f(t1 + 0.1), "-i".into(), esc_in(&path)]);
                        g.push(format!("[{b}]crop={wpx}:{hpx}:{xpx}:{ypx},{fx},format=yuv420p[bq{i}]"));
                        g.push(format!("[{idx}:v]format=gray[mk{i}]"));
                        g.push(format!("[bq{i}][mk{i}]alphamerge[bl{i}]"));
                    }
                    g.push(format!("[{a}][bl{i}]overlay=x={xpx}:y={ypx}:{en}:eof_action=pass[{next}]"));
                }
            }
            "zoom" => {
                let (wpx, hpx) = (((o.w * vwf / 2.0).round() as u32 * 2).max(16), ((o.h * vhf / 2.0).round() as u32 * 2).max(16));
                let (xpx, ypx) = (((o.x * vwf).round() as u32).min(vw.saturating_sub(wpx)), ((o.y * vhf).round() as u32).min(vh.saturating_sub(hpx)));
                let (a, b) = (format!("za{i}"), format!("zb{i}"));
                let d = 0.3f64.min((t1 - t0) / 3.0);
                g.push(format!("[{cur_v}]split[{a}][{b}]"));
                g.push(format!("[{b}]crop={wpx}:{hpx}:{xpx}:{ypx},scale={vw}:{vh}:flags=lanczos,setsar=1,format=rgba,fade=t=in:st={}:d={}:alpha=1,fade=t=out:st={}:d={}:alpha=1[zz{i}]", f(t0), f(d), f((t1 - d).max(t0)), f(d)));
                g.push(format!("[{a}][zz{i}]overlay=0:0:{en}:eof_action=pass[{next}]"));
            }
            "image" if !o.path.is_empty() && Path::new(&o.path).exists() => {
                let wpx = ((if o.w > 0.0 { o.w } else { 0.2 }) * vwf / 2.0).round().max(2.0) as u32 * 2;
                png_layer(&mut g, &mut extra, &mut n_in, &o.path, rate, (t0, t1), (format!("W*{}", f(o.x)), format!("H*{}", f(o.y))), o, vh, &cur_v, &next, i, &format!("scale={wpx}:-2:flags=lanczos,"));
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
    let fi = p.fade_in.clamp(0.0, 3.0);
    let fo = p.fade_out.clamp(0.0, 3.0);
    let mut post = post;
    if ext != "gif" {
        if fi > 0.0 { post += &format!(",fade=t=in:st=0:d={}", f(fi)); }
        if fo > 0.0 { post += &format!(",fade=t=out:st={}:d={}", f((total_len - fo).max(0.0)), f(fo)); }
    }
    g.push(format!("[{cur_v}]{post}[vout]"));
    // sounds: effects and music are mixed under or over the video's own sound
    let mut audio: Option<String> = if has_audio { Some(cur_a.clone()) } else { None };
    let mut music_ins: Vec<String> = vec![];
    if !gif && !p.sounds.is_empty() {
        let base = if let Some(a) = &audio { a.clone() } else {
            g.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={},asetpts=PTS-STARTPTS[abase]", f(total_len)));
            "abase".to_string()
        };
        let mut ins = vec![format!("[{base}]")];
        for (k, sn) in p.sounds.iter().enumerate() {
            let (natural, idx) = if sn.kind == "fx" {
                let (src, d) = fx_source(&sn.fx).ok_or_else(|| anyhow!("Unknown sound effect"))?;
                extra.extend(["-f".into(), "lavfi".into(), "-i".into(), src]);
                (d, n_in)
            } else {
                if !Path::new(&sn.path).exists() { continue; }
                extra.extend(["-i".into(), sn.path.clone()]);
                (ffmpeg::duration(Path::new(&sn.path)).unwrap_or(5.0), n_in)
            };
            n_in += 1;
            let start = (sn.start + off).max(0.0);
            let want = if sn.end > sn.start { Some(sn.end - sn.start) } else if sn.repeat { Some((total_len - start).max(0.5)) } else { None };
            let mut ch = format!("[{idx}:a]aresample=48000,aformat=channel_layouts=stereo");
            if sn.repeat { ch += ",aloop=loop=-1:size=2000000000"; }
            let len = want.unwrap_or(natural).min(if sn.repeat { 1e9 } else { natural });
            ch += &format!(",atrim=duration={},asetpts=PTS-STARTPTS", f(len));
            let fd = sn.fade.clamp(0.0, 5.0).min(len / 2.0);
            if fd > 0.0 { ch += &format!(",afade=t=in:d={},afade=t=out:st={}:d={}", f(fd), f(len - fd), f(fd)); }
            ch += &format!(",volume={}", f(if sn.vol > 0.0 { sn.vol.min(3.0) } else { 1.0 }));
            let ms = (start * 1000.0).round() as i64;
            ch += &format!(",adelay={ms}|{ms}[sd{k}]");
            g.push(ch);
            if sn.music && p.duck && has_audio { music_ins.push(format!("[sd{k}]")); } else { ins.push(format!("[sd{k}]")); }
        }
        if !music_ins.is_empty() {
            // the voice drives the compressor that lowers the music, and is also heard on its own
            let mixed = if music_ins.len() > 1 { g.push(format!("{}amix=inputs={}:duration=longest:normalize=0[mus]", music_ins.concat(), music_ins.len())); "mus" } else { g.push(format!("{}anull[mus]", music_ins[0])); "mus" };
            g.push(format!("[{base}]asplit[vo1][vo2]"));
            g.push(format!("[{mixed}][vo2]sidechaincompress=threshold=0.02:ratio=10:attack=30:release=600:makeup=1[duck]"));
            ins[0] = "[vo1]".into();
            ins.push("[duck]".into());
        }
        if ins.len() > 1 {
            g.push(format!("{}amix=inputs={}:duration=first:normalize=0:dropout_transition=0[amix]", ins.concat(), ins.len()));
            audio = Some("amix".into());
        } else if audio.is_none() {
            audio = None;
        }
    }
    if let Some(au) = audio.clone() {
        if ext != "gif" && (fi > 0.0 || fo > 0.0) {
            let mut ch = format!("[{au}]anull");
            if fi > 0.0 { ch += &format!(",afade=t=in:st=0:d={}", f(fi)); }
            if fo > 0.0 { ch += &format!(",afade=t=out:st={}:d={}", f((total_len - fo).max(0.0)), f(fo)); }
            g.push(format!("{ch}[afin]"));
            audio = Some("afin".into());
        }
    }
    let mut a: Vec<String> = vec!["-i".into(), p.input.clone()];
    a.extend(extra);
    a.extend(["-filter_complex".to_string(), g.join(";"), "-map".into(), "[vout]".into()]);
    if let Some(au) = &audio {
        if ext != "gif" {
            a.extend(["-map".to_string(), format!("[{au}]")]);
            a.extend(["-c:a", "aac", "-b:a", "192k"].map(String::from));
        }
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

fn esc_in(p: &Path) -> String {
    p.to_string_lossy().to_string()
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
    let mut p = p.clone();
    if let Some((w, h)) = ffmpeg::size(input) { p.vw = w; p.vh = h; }
    let (mut args, _ext) = build(&p, ffmpeg::has_audio(input), fps, &tmp)?;
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
#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
pub struct Analysis {
    pub silences: Vec<(f64, f64)>,
    pub idle: Vec<(f64, f64)>,
    pub scenes: Vec<f64>,
    /// false when the sound is missing or so quiet that pauses cannot be told from speech
    pub voice: bool,
}

/// What polishing would do, worked out from an analysis, in seconds of the source.
#[derive(serde::Serialize, Default, Debug, Clone)]
pub struct Plan {
    pub cuts: Vec<(f64, f64)>,
    pub fast: Vec<(f64, f64)>,
    pub speed: f64,
    pub saved: f64,
    pub voice: bool,
}

fn overlap(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for (s1, e1) in a {
        for (s2, e2) in b {
            let (s, e) = (s1.max(*s2), e1.min(*e2));
            if e > s {
                out.push((s, e));
            }
        }
    }
    out.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    out
}

/// A careful plan: only dead air goes (nothing said AND nothing changing on screen), still moments with no
/// speech are fast-forwarded, and the total is capped so a quiet recording is never emptied.
pub fn plan(a: &Analysis, dur: f64) -> Plan {
    let mut p = Plan { speed: 4.0, voice: a.voice, ..Default::default() };
    if dur < 10.0 {
        return p;
    }
    let idle: Vec<(f64, f64)> = a.idle.iter().filter(|(s, e)| e - s >= 2.0).cloned().collect();
    // dead air: still screen and silent at once
    let mut cuts: Vec<(f64, f64)> = if a.voice { overlap(&a.silences, &idle).into_iter().filter(|(s, e)| e - s >= 1.5).map(|(s, e)| (if s < 0.3 { 0.0 } else { s + 0.25 }, if e > dur - 0.3 { dur } else { e - 0.25 })).filter(|(s, e)| e - s >= 0.6).collect() } else { vec![] };
    // still moments that are not dead air: speed them up, but only when nobody is speaking over them
    let quiet: Vec<(f64, f64)> = if a.voice { a.silences.clone() } else { vec![(0.0, dur)] };
    let mut fast: Vec<(f64, f64)> = overlap(&idle, &quiet).into_iter().filter(|(s, e)| e - s >= 3.0).map(|(s, e)| (s + 0.5, e - 0.5)).filter(|(s, e)| e - s >= 1.5).collect();
    // keep fast ranges out of the cuts
    fast = fast.into_iter().filter(|(s, e)| !cuts.iter().any(|(a, b)| s < b && e > a)).collect();
    // never take more than half of the recording: keep the biggest wins first
    let gain = |r: &(f64, f64), cut: bool| if cut { r.1 - r.0 } else { (r.1 - r.0) * 0.75 };
    let mut all: Vec<(f64, bool, (f64, f64))> = cuts.iter().map(|r| (gain(r, true), true, *r)).chain(fast.iter().map(|r| (gain(r, false), false, *r))).collect();
    all.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let (mut saved, cap) = (0.0, dur * 0.5);
    cuts.clear();
    fast.clear();
    for (g, is_cut, r) in all {
        if saved + g > cap {
            continue;
        }
        saved += g;
        if is_cut { cuts.push(r) } else { fast.push(r) }
    }
    cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    fast.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    p.cuts = cuts;
    p.fast = fast;
    p.saved = saved;
    p
}

/// Loudest and average level of the sound track in dB, when there is one.
fn levels(path: &Path, low: bool) -> Option<(f64, f64)> {
    if !ffmpeg::has_audio(path) {
        return None;
    }
    let out = if low { ffmpeg::cmd_low() } else { ffmpeg::cmd() }.args(["-hide_banner", "-nostats", "-i", &path.to_string_lossy(), "-vn", "-af", "volumedetect", "-f", "null", "-"]).output().ok()?;
    let t = String::from_utf8_lossy(&out.stderr).to_string();
    Some((num_after(&t, "max_volume:")?, num_after(&t, "mean_volume:")?))
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
    // the pause threshold follows how loud this recording is, so a quiet microphone is not mistaken for silence
    let lv = levels(path, !scenes_too);
    let voice = matches!(lv, Some((mx, _)) if mx > -42.0);
    if voice {
        let thr = (lv.unwrap().0 - 26.0).clamp(-55.0, -30.0);
        a.extend(["-af".to_string(), format!("silencedetect=n={thr:.0}dB:d=0.8")]);
    }
    a.extend(["-f", "null", "-"].map(String::from));
    let out = if scenes_too { ffmpeg::cmd() } else { ffmpeg::cmd_low() }.args(&a).output()?;
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(anyhow!("analysis failed: {}", text.lines().last().unwrap_or("")));
    }
    if !scenes_too {
        return Ok(Analysis { silences: pairs(&text, "silence_start:", "silence_end:", total), idle: pairs(&text, "freeze_start:", "freeze_end:", total), scenes: vec![], voice });
    }
    let out2 = ffmpeg::cmd().args(["-hide_banner", "-nostats", "-i", &p, "-an", "-vf", "select=gt(scene\\,0.3),showinfo", "-f", "null", "-"]).output()?;
    let mut scenes = scene_times(&String::from_utf8_lossy(&out2.stderr));
    // keep scene changes at least 2 s apart
    let mut last = -10.0;
    scenes.retain(|t| { let keep = *t - last >= 2.0 && *t > 0.5; if keep { last = *t; } keep });
    Ok(Analysis { silences: pairs(&text, "silence_start:", "silence_end:", total), idle: pairs(&text, "freeze_start:", "freeze_end:", total), scenes, voice })
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
        Project { input, clips: vec![Clip { start: 0.0, end: 2.0, speed: 1.0 }, Clip { start: 3.0, end: 6.0, speed: 2.0 }], transition: Transition { kind: "fade".into(), dur: 0.5 }, overlays: vec![], preset: String::new(), ..Default::default() }
    }

    fn clip_with(dir: &Path, name: &str, video: &str, audio: Option<&str>, secs: u32) -> String {
        let p = dir.join(name);
        let mut a: Vec<String> = ["-f", "lavfi", "-i"].map(String::from).to_vec();
        a.push(format!("{video}:d={secs}"));
        if let Some(au) = audio {
            a.extend(["-f", "lavfi", "-i"].map(String::from));
            a.push(format!("{au}:d={secs}"));
        }
        a.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p"].map(String::from));
        if audio.is_some() {
            a.extend(["-c:a", "aac", "-shortest"].map(String::from));
        }
        a.push(p.to_string_lossy().into());
        ffmpeg::run(&a).unwrap();
        p.to_string_lossy().into()
    }

    #[test]
    fn polish_never_empties_a_quiet_recording() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("plan1");
        // a still screen with a silent sound track: nothing is said, so nothing may be cut
        let p = clip_with(&d, "still.mp4", "color=c=navy:s=320x180:r=15", Some("anullsrc=r=44100:cl=mono"), 40);
        let a = analyze_quick(Path::new(&p)).unwrap();
        assert!(!a.voice, "silent track must not count as voice");
        let pl = plan(&a, 40.0);
        assert!(pl.cuts.is_empty(), "cuts {:?}", pl.cuts);
        assert!(pl.saved <= 20.0 + 1e-6, "saved {}", pl.saved);
        // very quiet but real speech-like tone: still not treated as silence
        let q = clip_with(&d, "quiet.mp4", "testsrc2=size=320x180:rate=15", Some("aevalsrc=0.02*sin(2*PI*300*t):s=44100"), 20);
        let a2 = analyze_quick(Path::new(&q)).unwrap();
        assert!(a2.silences.is_empty(), "silences {:?}", a2.silences);
    }

    #[test]
    fn polish_removes_dead_air_but_keeps_narration_over_a_still_screen() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("plan2");
        // 0-10 s talking, 10-24 s nothing at all, 24-34 s talking, the screen never changes
        let p = clip_with(&d, "dead.mp4", "color=c=navy:s=320x180:r=15", Some("aevalsrc=if(lt(t\\,10)+gt(t\\,24)\\,0.5*sin(2*PI*440*t)\\,0):s=44100"), 34);
        let a = analyze_quick(Path::new(&p)).unwrap();
        assert!(a.voice);
        let pl = plan(&a, 34.0);
        assert_eq!(pl.cuts.len(), 1, "{:?} {:?}", a, pl);
        let (s, e) = pl.cuts[0];
        assert!(s > 9.5 && s < 11.5 && e > 22.5 && e < 24.5, "cut {s}-{e}");
        assert!(pl.fast.is_empty(), "narration must not be sped up: {:?}", pl.fast);
    }

    #[test]
    fn plan_is_capped() {
        let a = Analysis { silences: vec![(0.0, 100.0)], idle: vec![(0.0, 100.0)], scenes: vec![], voice: true };
        let pl = plan(&a, 100.0);
        assert!(pl.saved <= 50.0 + 1e-6);
        assert!(plan(&a, 8.0).cuts.is_empty());
    }

    #[test]
    fn exports_shapes_images_sounds_and_title_cards() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("rich");
        let src = sample(&d);
        let img = d.join("pic.png");
        ffmpeg::run(&["-f", "lavfi", "-i", "color=c=green:s=120x80:d=1", "-frames:v", "1", img.to_str().unwrap()]).unwrap();
        let mut p = proj(src);
        p.transition = Transition { kind: "fade".into(), dur: 0.4 };
        let o = |kind: &str, shape: &str| Overlay { kind: kind.into(), shape: shape.into(), start: 0.5, end: 3.0, x: 0.2, y: 0.2, w: 0.3, h: 0.3, color: "#ff3b30".into(), level: 2, bg: true, ..Default::default() };
        p.overlays = vec![
            Overlay { fill: 0.25, thick: 0.01, anim: "fade".into(), ..o("box", "ellipse") },
            Overlay { shape: "arrow".into(), flipx: true, anim: "slide".into(), ..o("box", "arrow") },
            o("box", "round"),
            Overlay { fill: 0.7, mode: "ellipse".into(), ..o("box", "spot") },
            Overlay { mode: "pixel".into(), ..o("blur", "ellipse") },
            Overlay { mode: "soft".into(), x: 0.6, ..o("blur", "round") },
            Overlay { mode: "solid".into(), color: "#000000".into(), x: 0.1, y: 0.6, w: 0.2, h: 0.1, ..o("blur", "rect") },
            Overlay { path: img.to_string_lossy().into(), w: 0.15, x: 0.7, y: 0.05, opacity: 0.8, anim: "fade".into(), ..o("image", "") },
            Overlay { kind: "text".into(), text: "Banner".into(), style: "banner".into(), size: 0.07, y: 0.7, x: 0.1, color: "#ffffff".into(), anim: "slide".into(), ..o("text", "") },
            Overlay { kind: "text".into(), text: "Outlined".into(), style: "outline".into(), size: 0.07, y: 0.4, x: 0.1, color: "#ffffff".into(), ..o("text", "") },
        ];
        p.sounds = vec![
            Sound { kind: "fx".into(), fx: "ding".into(), start: 1.0, vol: 0.8, ..Default::default() },
            Sound { kind: "fx".into(), fx: "pad".into(), start: 0.0, end: 0.0, repeat: true, vol: 0.3, fade: 0.5, ..Default::default() },
            Sound { kind: "fx".into(), fx: "whoosh".into(), start: 2.0, ..Default::default() },
        ];
        p.intro = Some(Card { text: "Title".into(), sub: "Sub".into(), dur: 2.0, bg: "#f2f2f7".into(), accent: "#0a84ff".into() });
        p.outro = Some(Card { text: "Thanks".into(), dur: 2.0, ..Default::default() });
        let out = d.join("rich.mp4");
        export_to(&p, &out).unwrap();
        if let Ok(k) = std::env::var("RECORA_KEEP") { let _ = std::fs::copy(&out, k); }
        let dur = ffmpeg::duration(&out).unwrap();
        // 2 + (2 + 1.5 - 0.4) + 2 minus two more joins of 0.4
        assert!((dur - 6.3).abs() < 0.7, "duration {dur}");
        assert!(ffmpeg::has_audio(&out));
        // a sound added to a video that has none still produces sound
        let silent = clip_with(&d, "mute.mp4", "testsrc2=size=320x180:rate=15", None, 6);
        let mut q = proj(silent);
        q.sounds = vec![Sound { kind: "fx".into(), fx: "success".into(), start: 1.0, ..Default::default() }];
        let out2 = d.join("snd.mp4");
        export_to(&q, &out2).unwrap();
        assert!(ffmpeg::has_audio(&out2));
        assert!(fx_source("nope").is_none());
    }

    #[test]
    fn exports_denoise_ducking_fades_zoom_and_pop() {
        if !ffmpeg::available() { return; }
        let d = tempfile_dir("duck");
        let src = sample(&d);
        let mut p = proj(src);
        p.denoise = true; p.duck = true; p.fade_in = 0.5; p.fade_out = 0.5;
        p.sounds = vec![Sound { kind: "fx".into(), fx: "pad".into(), start: 0.0, repeat: true, music: true, vol: 0.3, fade: 1.0, ..Default::default() }];
        p.overlays = vec![
            Overlay { kind: "zoom".into(), start: 1.0, end: 3.0, x: 0.1, y: 0.1, w: 0.4, h: 0.4, ..Default::default() },
            Overlay { kind: "text".into(), text: "Pop".into(), anim: "pop".into(), start: 0.5, end: 3.0, x: 0.1, y: 0.5, size: 0.08, color: "#ffffff".into(), bg: true, ..Default::default() },
        ];
        let out = d.join("duck.mp4");
        export_to(&p, &out).unwrap();
        assert!(ffmpeg::has_audio(&out));
        assert!(ffmpeg::duration(&out).unwrap() > 1.0);
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
            Overlay { kind: "text".into(), start: 0.2, end: 2.0, x: 0.1, y: 0.8, w: 0.0, h: 0.0, text: "Hello: it's 100% fine".into(), size: 0.06, color: "#ffffff".into(), level: 0, bg: false, ..Default::default() },
            Overlay { kind: "box".into(), start: 0.0, end: 3.0, x: 0.1, y: 0.1, w: 0.3, h: 0.2, text: String::new(), size: 0.0, color: "#ff3b30".into(), level: 0, bg: true, ..Default::default() },
            Overlay { kind: "blur".into(), start: 0.0, end: 3.0, x: 0.5, y: 0.5, w: 0.3, h: 0.3, text: String::new(), size: 0.0, color: String::new(), level: 3, bg: true, ..Default::default() },
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
