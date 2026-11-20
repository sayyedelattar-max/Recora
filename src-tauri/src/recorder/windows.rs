//! Windows Graphics Capture backend (the same API OBS and Xbox Game Bar use). Frames are read from the GPU and
//! piped to the bundled ffmpeg, which encodes with the graphics card (NVENC / Quick Sync / AMF) or, if none works,
//! with x264. A pacer thread writes exactly `fps` frames per second by the wall clock, so a still screen still
//! produces a video as long as the recording and nothing is ever "compressed" in time.
use super::{blur, ActiveCapture, CaptureSpec, MonitorInfo, Source, VideoStats, WindowInfo, ZoneSpec};
use crate::ffmpeg;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    GraphicsCaptureItemType, ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

// ---------------------------------------------------------------------------------------------------------------------
// encoder choice
// ---------------------------------------------------------------------------------------------------------------------
fn enc_args(name: &str, quality: &str) -> Vec<String> {
    let q = match quality {
        "standard" => 0,
        "max" => 2,
        _ => 1,
    };
    let v = |a: [&str; 3]| a[q].to_string();
    match name {
        "h264_nvenc" => vec!["-c:v".into(), name.into(), "-preset".into(), "p3".into(), "-rc".into(), "vbr".into(), "-cq".into(), v(["29", "24", "20"]), "-b:v".into(), "0".into()],
        "h264_qsv" => vec!["-c:v".into(), name.into(), "-preset".into(), "veryfast".into(), "-global_quality".into(), v(["29", "24", "20"])],
        "h264_amf" => vec!["-c:v".into(), name.into(), "-quality".into(), "speed".into(), "-rc".into(), "cqp".into(), "-qp_i".into(), v(["28", "23", "19"]), "-qp_p".into(), v(["30", "25", "21"])],
        _ => vec!["-c:v".into(), "libx264".into(), "-preset".into(), "superfast".into(), "-crf".into(), v(["27", "23", "19"])],
    }
}

fn probe(name: &str) -> bool {
    let mut c = ffmpeg::cmd();
    c.args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:r=30", "-frames:v", "8", "-pix_fmt", "yuv420p"])
        .args(enc_args(name, "high"))
        .args(["-f", "null", "-"]);
    c.output().map(|o| o.status.success()).unwrap_or(false)
}

static ENCODER: OnceLock<&'static str> = OnceLock::new();

fn pick_encoder() -> &'static str {
    ENCODER.get_or_init(|| {
        for n in ["h264_nvenc", "h264_qsv", "h264_amf"] {
            if probe(n) {
                crate::diag::log(&format!("video encoder: {n} (graphics card)"));
                return n;
            }
        }
        crate::diag::log("video encoder: libx264 (processor)");
        "libx264"
    })
}

/// Called once at start-up so the first recording does not wait for the encoder test.
pub fn warm_up() {
    std::thread::spawn(|| {
        let _ = pick_encoder();
    });
}

// ---------------------------------------------------------------------------------------------------------------------
// capture
// ---------------------------------------------------------------------------------------------------------------------
#[derive(Clone)]
struct Flags {
    out: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    quality: String,
    crop: Option<(u32, u32, u32, u32)>, // x, y, w, h in source pixels
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    first: Mutex<Option<Instant>>,
    frames: Mutex<u64>,
    err: Mutex<Option<String>>,
    latest: Mutex<Option<Vec<u8>>>,
    stop: AtomicBool,
    pacer: Mutex<Option<JoinHandle<()>>>,
    /// areas to blur, in picture pixels (x0, y0, x1, y1); kept up to date by the zone tracker
    zones: Mutex<Vec<(i32, i32, i32, i32)>>,
}

fn note_err(shared: &Shared, msg: String) {
    crate::diag::log(&msg);
    *shared.err.lock() = Some(msg);
}

/// Copies a BGRA picture into a w x h canvas (cropping or padding with black when the window changed size).
fn fit(src: &[u8], sw: u32, sh: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let cw = (sw.min(w) * 4) as usize;
    for y in 0..sh.min(h) as usize {
        let s = y * (sw as usize) * 4;
        let d = y * (w as usize) * 4;
        if s + cw <= src.len() {
            out[d..d + cw].copy_from_slice(&src[s..s + cw]);
        }
    }
    out
}

/// Writes frames to ffmpeg at a steady rate by the wall clock, repeating the newest picture while the screen is still.
fn pacer(shared: Arc<Shared>, mut child: std::process::Child, fps: u32) {
    let Some(mut stdin) = child.stdin.take() else { return };
    let fpsf = fps.max(1) as f64;
    let mut last: Vec<u8> = Vec::new();
    let t0;
    loop {
        if let Some(f) = shared.latest.lock().take() {
            last = f;
            t0 = Instant::now();
            break;
        }
        if shared.stop.load(Ordering::SeqCst) {
            drop(stdin);
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    let mut written: u64 = 0;
    loop {
        let stopping = shared.stop.load(Ordering::SeqCst);
        if let Some(f) = shared.latest.lock().take() {
            last = f;
        }
        let expected = (t0.elapsed().as_secs_f64() * fpsf).floor() as u64 + 1;
        let mut burst = 0;
        while written < expected && burst < 8 {
            if let Err(e) = stdin.write_all(&last) {
                note_err(&shared, format!("the video encoder stopped: {e}"));
                drop(stdin);
                let _ = child.wait();
                return;
            }
            written += 1;
            burst += 1;
        }
        if stopping {
            break;
        }
        let due = Duration::from_secs_f64(written as f64 / fpsf);
        let now = t0.elapsed();
        std::thread::sleep(if due > now { (due - now).min(Duration::from_millis(40)) } else { Duration::from_millis(1) });
    }
    drop(stdin);
    match child.wait_with_output() {
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            crate::diag::log(&format!("encoder finished: {written} frames written, exit {:?}{}", o.status.code(), if err.is_empty() { String::new() } else { format!(", ffmpeg said: {err}") }));
            if !o.status.success() {
                note_err(&shared, format!("the video encoder failed: {}", err.lines().last().unwrap_or("unknown error")));
            }
        }
        Err(e) => note_err(&shared, format!("the video encoder could not be closed: {e}")),
    }
}

struct Handler {
    flags: Flags,
    tmp: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = Box<dyn Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let enc = pick_encoder();
        let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgra"].iter().map(|s| s.to_string()).collect();
        args.extend(["-s".into(), format!("{}x{}", f.width, f.height), "-framerate".into(), f.fps.to_string(), "-i".into(), "-".into()]);
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
        args.extend(enc_args(enc, &f.quality));
        crate::diag::log(&format!("ffmpeg: {} {}", enc, args.join(" ")));
        let child = ffmpeg::cmd()
            .args(&args)
            .arg(&f.out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let child = match child {
            Ok(c) => c,
            Err(e) => {
                note_err(&f.shared, format!("the video encoder could not start: {e}"));
                return Err(e.into());
            }
        };
        let shared = f.shared.clone();
        let fps = f.fps;
        *f.shared.pacer.lock() = Some(std::thread::spawn(move || pacer(shared, child, fps)));
        Ok(Handler { flags: f, tmp: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _ctl: InternalCaptureControl) -> Result<(), Self::Error> {
        match self.handle(frame) {
            Ok(()) => Ok(()),
            Err(e) => {
                note_err(&self.flags.shared, format!("a frame could not be read: {e}"));
                Err(e)
            }
        }
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        crate::diag::log("capture closed");
        Ok(())
    }
}

impl Handler {
    fn handle(&mut self, frame: &mut Frame) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (fw, fh) = (frame.width(), frame.height());
        let (w, h) = (self.flags.width, self.flags.height);
        let crop = self.flags.crop;
        let out = {
            let buf = match crop {
                None => frame.buffer()?,
                Some((x, y, cw, ch)) => frame.buffer_crop(x, y, x + cw, y + ch)?,
            };
            let data = buf.as_nopadding_buffer(&mut self.tmp);
            let (sw, sh) = match crop {
                None => (fw, fh),
                Some((_, _, cw, ch)) => (cw, ch),
            };
            if sw == w && sh == h && data.len() == (w * h * 4) as usize {
                data.to_vec()
            } else {
                fit(data, sw, sh, w, h)
            }
        };
        let mut out = out;
        let zones = self.flags.shared.zones.lock().clone();
        for (x0, y0, x1, y1) in zones {
            blur::soften(&mut out, w, h, x0, y0, x1, y1);
        }
        *self.flags.shared.latest.lock() = Some(out);
        {
            let mut first = self.flags.shared.first.lock();
            if first.is_none() {
                *first = Some(Instant::now());
            }
        }
        *self.flags.shared.frames.lock() += 1;
        Ok(())
    }
}

pub struct Wgc {
    control: Control,
    shared: Arc<Shared>,
}

type Control = CaptureControl<Handler, Box<dyn Error + Send + Sync>>;

/// Starts the capture; first without the yellow capture border (Windows 11), then with default settings.
/// A start only counts when the first frame really arrives, otherwise the next setting is tried.
fn launch<T: TryInto<GraphicsCaptureItemType> + Copy + Send + 'static>(item: T, cursor: CursorCaptureSettings, flags: Flags) -> Result<Control> {
    let mut last = String::new();
    for (name, border) in [("no border", DrawBorderSettings::WithoutBorder), ("default border", DrawBorderSettings::Default)] {
        *flags.shared.err.lock() = None;
        *flags.shared.first.lock() = None;
        flags.shared.stop.store(false, Ordering::SeqCst);
        let settings = Settings::new(item, cursor, border, SecondaryWindowSettings::Default, MinimumUpdateIntervalSettings::Default, DirtyRegionSettings::Default, ColorFormat::Bgra8, flags.clone());
        crate::diag::log(&format!("starting capture ({name})"));
        match Handler::start_free_threaded(settings) {
            Ok(c) => {
                let t0 = Instant::now();
                while t0.elapsed() < Duration::from_millis(3000) {
                    if flags.shared.first.lock().is_some() || flags.shared.err.lock().is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                if flags.shared.first.lock().is_some() && flags.shared.err.lock().is_none() {
                    crate::diag::log(&format!("first frame arrived after {} ms ({name})", t0.elapsed().as_millis()));
                    return Ok(c);
                }
                last = flags.shared.err.lock().clone().unwrap_or_else(|| format!("no frame arrived within 3 s ({name})"));
                crate::diag::log(&format!("capture ({name}) gave no frame: {last}"));
                flags.shared.stop.store(true, Ordering::SeqCst);
                let _ = c.stop();
                if let Some(h) = flags.shared.pacer.lock().take() {
                    let _ = h.join();
                }
            }
            Err(e) => {
                last = format!("{e}");
                crate::diag::log(&format!("capture ({name}) failed to start: {last}"));
            }
        }
    }
    Err(anyhow!("the screen capture could not start: {last}"))
}


// ---------------------------------------------------------------------------------------------------------------------
// hide zones
// ---------------------------------------------------------------------------------------------------------------------
type Rect4 = (i32, i32, i32, i32);

struct WinRec {
    proc: String, // lower case, no .exe
    title: String,
    rect: Rect4,
}

fn norm(p: &str) -> String {
    p.rsplit(['\\', '/']).next().unwrap_or(p).to_lowercase().trim_end_matches(".exe").trim().to_string()
}

/// All visible app windows, the one in front first (the order Windows lists them in).
fn all_windows(cache: &mut std::collections::HashMap<isize, String>) -> Vec<WinRec> {
    let mut v = Vec::new();
    if let Ok(list) = Window::enumerate() {
        for w in list {
            if !w.is_valid() {
                continue;
            }
            let Ok(r) = w.rect() else { continue };
            let key = w.as_raw_hwnd() as isize;
            let proc = cache.entry(key).or_insert_with(|| norm(&w.process_name().unwrap_or_default())).clone();
            v.push(WinRec { proc, title: w.title().unwrap_or_default(), rect: (r.left, r.top, r.right, r.bottom) });
        }
    }
    v
}

/// Where everything that must be blurred is inside the recorded picture right now.
/// - apps: the windows of these processes, but only the parts that no other window covers
/// - zones: marked areas (fractions of an app window, or pixels of the monitor)
fn blur_rects(zones: &[ZoneSpec], apps: &[String], monitor_origin: (i32, i32), frame_origin: (i32, i32), windows: &[WinRec]) -> Vec<Rect4> {
    let mut screen: Vec<Rect4> = Vec::new();
    // chat apps etc.
    if !apps.is_empty() {
        for (i, w) in windows.iter().enumerate() {
            if !apps.contains(&w.proc) || (w.rect.2 - w.rect.0) < 60 || (w.rect.3 - w.rect.1) < 40 {
                continue;
            }
            // windows in front of it, that are not blurred themselves. The window shadow (about 8 px at the sides and bottom) is not part of what you see.
            let above: Vec<Rect4> = windows[..i]
                .iter()
                .filter(|o| !apps.contains(&o.proc) && !o.title.trim().is_empty() && (o.rect.2 - o.rect.0) > 40 && (o.rect.3 - o.rect.1) > 30)
                .map(|o| (o.rect.0 + 8, o.rect.1, o.rect.2 - 8, o.rect.3 - 8))
                .collect();
            screen.extend(blur::visible_parts(w.rect, &above));
        }
    }
    // marked areas
    let pad = 8;
    for z in zones {
        let r: Option<Rect4> = match &z.process {
            Some(p) => {
                let p = norm(p);
                windows.iter().filter(|w| w.proc == p).map(|w| w.rect).max_by_key(|r| ((r.2 - r.0).max(0) as i64) * ((r.3 - r.1).max(0) as i64)).map(|(l, t, r, b)| {
                    let (ww, wh) = ((r - l) as f64, (b - t) as f64);
                    (l + (z.x * ww) as i32 - pad, t + (z.y * wh) as i32 - pad, l + ((z.x + z.w) * ww) as i32 + pad, t + ((z.y + z.h) * wh) as i32 + pad)
                })
            }
            None => Some((monitor_origin.0 + z.x as i32 - pad, monitor_origin.1 + z.y as i32 - pad, monitor_origin.0 + (z.x + z.w) as i32 + pad, monitor_origin.1 + (z.y + z.h) as i32 + pad)),
        };
        if let Some(r) = r {
            screen.push(r);
        }
    }
    screen.into_iter().map(|(l, t, r, b)| (l - frame_origin.0, t - frame_origin.1, r - frame_origin.0, b - frame_origin.1)).collect()
}

fn start_tracker(shared: Arc<Shared>, zones: Vec<ZoneSpec>, apps: Vec<String>, spec_source: Source, origin: (i32, i32)) {
    let mut cache = std::collections::HashMap::new();
    let compute = move |cache: &mut std::collections::HashMap<isize, String>| -> Vec<Rect4> {
        let wins = all_windows(cache);
        let frame_origin = match &spec_source {
            Source::Screen { .. } => origin,
            Source::Region(r) => (origin.0 + r.x as i32, origin.1 + r.y as i32),
            Source::Window { hwnd } => {
                let w = Window::from_raw_hwnd(*hwnd as *mut std::ffi::c_void);
                w.rect().map(|r| (r.left, r.top)).unwrap_or(origin)
            }
        };
        blur_rects(&zones, &apps, origin, frame_origin, &wins)
    };
    // the first picture must already be blurred, so the first set is computed before the capture starts
    *shared.zones.lock() = compute(&mut cache);
    crate::diag::log(&format!("blur: {} areas now ({} apps, {} marked zones)", shared.zones.lock().len(), apps.len(), zones.len()));
    std::thread::spawn(move || {
        while !shared.stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            let r = compute(&mut cache);
            *shared.zones.lock() = r;
        }
    });
}

/// The app window under a desktop point, topmost first (used when you mark an area to hide).
pub fn window_at(x: i32, y: i32) -> Option<(String, String, Rect4)> {
    let list = Window::enumerate().ok()?;
    for w in list {
        if !w.is_valid() {
            continue;
        }
        let title = w.title().unwrap_or_default();
        let proc = w.process_name().unwrap_or_default();
        if title.trim().is_empty() || title == "Program Manager" || title == "Choose area" || proc.to_lowercase().starts_with("recora") {
            continue;
        }
        let Ok(r) = w.rect() else { continue };
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom && (r.right - r.left) > 120 {
            return Some((proc, title, (r.left, r.top, r.right, r.bottom)));
        }
    }
    None
}


// ---------------------------------------------------------------------------------------------------------------------
// one picture, for the preview on the Record page
// ---------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Default)]
struct GrabFlags {
    slot: Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>,
    crop: Option<(u32, u32, u32, u32)>,
}

struct Grabber {
    flags: GrabFlags,
}

impl GraphicsCaptureApiHandler for Grabber {
    type Flags = GrabFlags;
    type Error = Box<dyn Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Grabber { flags: ctx.flags })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _ctl: InternalCaptureControl) -> Result<(), Self::Error> {
        if self.flags.slot.lock().is_some() {
            return Ok(());
        }
        let (fw, fh) = (frame.width(), frame.height());
        let crop = self.flags.crop;
        let mut tmp = Vec::new();
        let picture = {
            let buf = match crop {
                None => frame.buffer()?,
                Some((x, y, w, h)) => frame.buffer_crop(x, y, x + w, y + h)?,
            };
            let data = buf.as_nopadding_buffer(&mut tmp);
            let (w, h) = crop.map(|c| (c.2, c.3)).unwrap_or((fw, fh));
            // keep it light: shrink here, on the borrowed data, instead of copying a whole 4K frame
            let step = ((w / 640).max(1)) as usize;
            let (tw, th) = (w as usize / step, h as usize / step);
            let mut small = Vec::with_capacity(tw * th * 4);
            for y in 0..th {
                for x in 0..tw {
                    let p = ((y * step) * w as usize + x * step) * 4;
                    if p + 3 < data.len() {
                        small.extend_from_slice(&data[p..p + 4]);
                    } else {
                        small.extend_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
            (tw as u32, th as u32, small)
        };
        *self.flags.slot.lock() = Some(picture);
        Ok(())
    }
}

fn grab_item<T: TryInto<GraphicsCaptureItemType> + Copy + Send + 'static>(item: T, crop: Option<(u32, u32, u32, u32)>) -> Result<(u32, u32, Vec<u8>)> {
    let flags = GrabFlags { slot: Arc::new(Mutex::new(None)), crop };
    for border in [DrawBorderSettings::WithoutBorder, DrawBorderSettings::Default] {
        let settings = Settings::new(item, CursorCaptureSettings::WithoutCursor, border, SecondaryWindowSettings::Default, MinimumUpdateIntervalSettings::Default, DirtyRegionSettings::Default, ColorFormat::Bgra8, flags.clone());
        if let Ok(c) = Grabber::start_free_threaded(settings) {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(1500) && flags.slot.lock().is_none() {
                std::thread::sleep(Duration::from_millis(30));
            }
            let _ = c.stop();
            if let Some(p) = flags.slot.lock().take() {
                return Ok(p);
            }
        }
    }
    Err(anyhow!("no picture"))
}

pub fn grab(source: &Source) -> Result<(u32, u32, Vec<u8>)> {
    match source {
        Source::Screen { monitor } => grab_item(Monitor::from_index(*monitor + 1).map_err(|e| anyhow!("{e}"))?, None),
        Source::Region(r) => {
            let mon = Monitor::from_index(r.monitor + 1).map_err(|e| anyhow!("{e}"))?;
            let (mw, mh) = (mon.width()?, mon.height()?);
            let (w, h) = (r.w.min(mw), r.h.min(mh));
            grab_item(mon, Some((r.x.min(mw.saturating_sub(w)), r.y.min(mh.saturating_sub(h)), w, h)))
        }
        Source::Window { hwnd } => {
            let win = Window::from_raw_hwnd(*hwnd as *mut std::ffi::c_void);
            if !win.is_valid() {
                return Err(anyhow!("window closed"));
            }
            grab_item(win, None)
        }
    }
}

fn even(v: u32) -> u32 {
    (v & !1).max(2)
}

pub fn start(spec: &CaptureSpec, out: &Path) -> Result<Box<dyn ActiveCapture>> {
    let shared = Arc::new(Shared::default());
    let cursor = if spec.cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor };
    let mk = |w: u32, h: u32, crop| Flags { out: out.to_path_buf(), width: even(w), height: even(h), fps: spec.fps, quality: spec.quality.clone(), crop, shared: shared.clone() };
    if !spec.zones.is_empty() || !spec.blur_apps.is_empty() {
        start_tracker(shared.clone(), spec.zones.clone(), spec.blur_apps.clone(), spec.source.clone(), spec.origin);
    }
    let control = match &spec.source {
        Source::Screen { monitor } | Source::Region(super::RegionSel { monitor, .. }) => {
            let mon = Monitor::from_index(*monitor + 1).map_err(|e| anyhow!("monitor {}: {e}", monitor + 1))?;
            let (mw, mh) = (mon.width()?, mon.height()?);
            crate::diag::log(&format!("monitor {} is {mw}x{mh}", monitor + 1));
            let flags = match &spec.source {
                Source::Region(r) => {
                    let (rw, rh) = (even(r.w.min(mw)), even(r.h.min(mh)));
                    let (rx, ry) = (r.x.min(mw.saturating_sub(rw)), r.y.min(mh.saturating_sub(rh)));
                    mk(rw, rh, Some((rx, ry, rw, rh)))
                }
                _ => mk(mw, mh, None),
            };
            launch(mon, cursor, flags)?
        }
        Source::Window { hwnd } => {
            let win = Window::from_raw_hwnd(*hwnd as *mut std::ffi::c_void);
            if !win.is_valid() {
                return Err(anyhow!("that window is no longer open"));
            }
            let (ww, wh) = (win.width()?.max(2) as u32, win.height()?.max(2) as u32);
            crate::diag::log(&format!("window is {ww}x{wh}"));
            launch(win, cursor, mk(ww, wh, None))?
        }
    };
    Ok(Box::new(Wgc { control, shared }))
}

impl ActiveCapture for Wgc {
    fn first_frame(&self) -> Option<Instant> {
        *self.shared.first.lock()
    }
    fn last_error(&self) -> Option<String> {
        self.shared.err.lock().clone()
    }
    fn stop(self: Box<Self>) -> Result<VideoStats> {
        // close the video file first (the pacer closes ffmpeg's input and waits for it), then end the capture
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.shared.pacer.lock().take() {
            let _ = h.join();
        }
        let frames = *self.shared.frames.lock();
        let _ = self.control.stop();
        Ok(VideoStats { frames })
    }
}

pub fn list_monitors() -> Vec<MonitorInfo> {
    let mut v = Vec::new();
    if let Ok(list) = Monitor::enumerate() {
        for (i, m) in list.iter().enumerate() {
            v.push(MonitorInfo {
                index: i,
                name: m.device_name().unwrap_or_default(),
                width: m.width().unwrap_or(0),
                height: m.height().unwrap_or(0),
                primary: i == 0,
            });
        }
    }
    v
}

pub fn list_windows() -> Vec<WindowInfo> {
    let mut v = Vec::new();
    if let Ok(list) = Window::enumerate() {
        for w in list {
            let title = w.title().unwrap_or_default();
            if title.trim().is_empty() || !w.is_valid() {
                continue;
            }
            if matches!(title.as_str(), "Program Manager" | "Windows Input Experience" | "Recora") || title.starts_with("Recora") {
                continue;
            }
            let (ww, wh) = (w.width().unwrap_or(0), w.height().unwrap_or(0));
            if ww < 120 || wh < 80 {
                continue;
            }
            v.push(WindowInfo { hwnd: (w.as_raw_hwnd() as isize).to_string(), title, process: w.process_name().unwrap_or_default() });
        }
    }
    v
}
