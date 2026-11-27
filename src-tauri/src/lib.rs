mod apps;
mod captions;
mod shapes;
mod diag;
mod ai;
mod edit;
mod ffmpeg;
mod library;
mod recorder;
mod settings;

use parking_lot::Mutex;
use recorder::mux::Report;
use recorder::{audio, CaptureSpec, Session, Source};
use serde_json::json;
use settings::{Region, Settings};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};


pub struct AppState {
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    session: Mutex<Option<Session>>,
    phase: Mutex<String>, // idle | countdown | recording | paused | saving
    voice: Mutex<Option<(recorder::audio::AudioRec, recorder::audio::Levels)>>,
}

type R<T> = Result<T, String>;
fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn phase(app: &AppHandle) -> String {
    app.state::<AppState>().phase.lock().clone()
}
fn set_phase(app: &AppHandle, p: &str) {
    *app.state::<AppState>().phase.lock() = p.to_string();
    let _ = app.emit("rec:phase", p);
}

// ---------------------------------------------------------------------------------------------------------------------
// settings & sources
// ---------------------------------------------------------------------------------------------------------------------
#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, state: State<AppState>, new: Settings) -> R<()> {
    let hotkeys_changed = {
        let old = state.settings.lock();
        old.hotkey_toggle != new.hotkey_toggle || old.hotkey_pause != new.hotkey_pause || old.hotkey_marker != new.hotkey_marker
    };
    recorder::blur::set_level(new.blur_level);
    new.save(&state.settings_path);
    *state.settings.lock() = new;
    if hotkeys_changed {
        register_hotkeys(&app);
    }
    Ok(())
}

#[tauri::command]
fn get_phase(state: State<AppState>) -> String {
    state.phase.lock().clone()
}

#[tauri::command]
async fn get_sources() -> serde_json::Value {
    json!({
        "monitors": recorder::list_monitors(),
        "windows": recorder::list_windows(),
        "mics": audio::list_inputs(),
        "platform": if cfg!(windows) { "windows" } else { "other" },
    })
}

#[tauri::command]
async fn list_apps() -> Vec<apps::AppEntry> {
    tauri::async_runtime::spawn_blocking(apps::list_installed).await.unwrap_or_default()
}

#[tauri::command]
async fn launch_app(name: String, id: String) -> R<serde_json::Value> {
    let entry = apps::AppEntry { name, id };
    let r = tauri::async_runtime::spawn_blocking(move || apps::launch(&entry, Duration::from_secs(20)))
        .await
        .map_err(|e| e.to_string())??;
    Ok(json!({ "window": r.window, "reused": r.reused }))
}

#[tauri::command]
async fn preflight(state: State<'_, AppState>) -> R<serde_json::Value> {
    let s = state.settings.lock().clone();
    let mut checks = vec![];
    let ff = ffmpeg::available();
    checks.push(json!({"id":"engine","label":"Capture engine","ok":ff,
        "detail": if !ff { "Video tools are missing. Reinstall Recora." } else if cfg!(windows) { "Windows Graphics Capture, hardware encoder" } else { "Test mode (no real screen)" }}));
    let _ = std::fs::create_dir_all(&s.out_dir);
    let free = fs2::available_space(&s.out_dir).unwrap_or(0);
    let gb = free as f64 / 1_073_741_824.0;
    checks.push(json!({"id":"disk","label":"Disk space","ok": gb > 2.0,"detail": format!("{gb:.0} GB free in your recordings folder")}));
    if s.mic {
        let ok = audio::has_default_input();
        checks.push(json!({"id":"mic","label":"Microphone","ok":ok,"detail": if ok { "Ready" } else { "No microphone found. Recording will have no voice." }}));
    }
    Ok(json!({ "checks": checks }))
}

// ---------------------------------------------------------------------------------------------------------------------
// recording
// ---------------------------------------------------------------------------------------------------------------------
fn build_spec(s: &Settings) -> Result<CaptureSpec, String> {
    let source = match s.source_kind.as_str() {
        "window" => {
            let h: isize = s.window_hwnd.parse().map_err(|_| "Choose a window to record first.".to_string())?;
            Source::Window { hwnd: h }
        }
        "region" => {
            let r = s.region.clone().ok_or("Draw the area you want to record first.")?;
            Source::Region(recorder::RegionSel { monitor: r.monitor, x: r.x, y: r.y, w: r.w, h: r.h })
        }
        _ => Source::Screen { monitor: s.monitor },
    };
    Ok(CaptureSpec { source, fps: s.fps, quality: s.quality.clone(), cursor: s.show_cursor, zones: Vec::new(), blur_apps: Vec::new(), origin: (0, 0), size: (0, 0) })
}

/// The Tauri monitor that matches recorder monitor `idx` (matched by device name, then by position in the list).
fn tauri_monitor(app: &AppHandle, idx: usize) -> Option<tauri::Monitor> {
    let infos = recorder::list_monitors();
    let want = infos.get(idx).map(|m| m.name.clone()).unwrap_or_default();
    let mons = app.available_monitors().ok()?;
    mons.iter().find(|m| m.name().map(|n| *n == want).unwrap_or(false)).cloned().or_else(|| mons.get(idx).cloned()).or_else(|| mons.first().cloned())
}

/// "C:\\Apps\\Zoom.exe" -> "zoom"
fn norm_proc(p: &str) -> String {
    let name = p.rsplit(['\\', '/']).next().unwrap_or(p).to_lowercase();
    name.trim_end_matches(".exe").trim().to_string()
}

/// Adds the blur zones of the settings to a capture spec.
fn add_zones(app: &AppHandle, s: &Settings, spec: &mut CaptureSpec) {
    let mon = match &spec.source {
        Source::Screen { monitor } => *monitor,
        Source::Region(r) => r.monitor,
        Source::Window { .. } => s.monitor,
    };
    if let Some(m) = tauri_monitor(app, mon) {
        spec.origin = (m.position().x, m.position().y);
        spec.size = (m.size().width, m.size().height);
    }
    recorder::blur::set_level(s.blur_level);
    if s.blur_apps_on {
        let mut skip = String::new(); // the app being recorded is never blurred
        if let Source::Window { hwnd } = &spec.source {
            if let Some(w) = recorder::list_windows().iter().find(|w| w.hwnd == hwnd.to_string()) {
                skip = norm_proc(&w.process);
            }
        }
        spec.blur_apps = s.blur_apps.iter().map(|a| norm_proc(a)).filter(|a| !a.is_empty() && *a != skip).collect();
    }
    if !s.hide_zones_on {
        return;
    }
    for z in s.hide_zones.iter().filter(|z| z.on && (z.process.is_some() || z.monitor == mon)) {
        spec.zones.push(recorder::ZoneSpec { process: z.process.clone(), x: z.x, y: z.y, w: z.w, h: z.h });
    }
}

fn show_capsule(app: &AppHandle, labels: bool) {
    if app.get_webview_window("capsule").is_some() {
        return;
    }
    let (mut x, mut y) = (400.0, 24.0);
    if let Ok(Some(m)) = app.primary_monitor() {
        let sc = m.scale_factor();
        x = m.position().x as f64 / sc + (m.size().width as f64 / sc - 230.0) / 2.0;
        y = m.position().y as f64 / sc + 10.0;
    }
    let _ = WebviewWindowBuilder::new(app, "capsule", WebviewUrl::App(format!("capsule.html?labels={}", labels as u8).into()))
        .title("Recora")
        .inner_size(230.0, 64.0)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        .content_protected(true) // keeps the bar itself out of the recording
        .build();
}

fn close_capsule(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("capsule") {
        let _ = w.close();
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn do_start(app: &AppHandle) -> R<()> {
    let st = app.state::<AppState>();
    {
        let mut p = st.phase.lock();
        if *p != "idle" {
            return Err("A recording is already in progress.".into());
        }
        *p = "countdown".into();
    }
    let _ = app.emit("rec:phase", "countdown");
    let s = st.settings.lock().clone();
    let fail = |app: &AppHandle, msg: String| -> R<()> {
        close_capsule(app);
        show_main(app);
        set_phase(app, "idle");
        let _ = app.emit("rec:error", &msg);
        Err(msg)
    };
    let mut spec = match build_spec(&s) {
        Ok(x) => x,
        Err(m) => return fail(app, m),
    };
    add_zones(app, &s, &mut spec);
    if !ffmpeg::available() {
        return fail(app, "Video tools are missing. Please reinstall Recora.".into());
    }
    let out_dir = PathBuf::from(&s.out_dir);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        return fail(app, format!("Cannot use the recordings folder: {e}"));
    }
    diag::init(&out_dir);
    diag::log(&format!("start: source={:?} fps={} quality={} cursor={} mic={} system={}", spec.source, spec.fps, spec.quality, spec.cursor, s.mic, s.system_audio));
    show_capsule(app, s.labels_left > 0);
    if s.labels_left > 0 {
        let st = app.state::<AppState>();
        let mut cur = st.settings.lock();
        cur.labels_left = cur.labels_left.saturating_sub(1);
        cur.save(&st.settings_path);
    }
    if s.hide_on_record {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    }
    for n in (1..=s.countdown).rev() {
        let _ = app.emit("rec:countdown", n);
        std::thread::sleep(Duration::from_secs(1));
        if phase(app) != "countdown" {
            return Ok(()); // cancelled
        }
    }
    let tmp = out_dir.join(".recora").join("tmp").join(chrono::Local::now().format("%H%M%S").to_string());
    let aspec = if s.mic || s.system_audio {
        Some(audio::AudioSpec { mic: s.mic, mic_device: s.mic_device.clone(), system: s.system_audio })
    } else {
        None
    };
    let mut session = match Session::new(tmp, spec, aspec) {
        Ok(x) => x,
        Err(e) => return fail(app, e2s(e)),
    };
    if let Err(e) = session.start_segment() {
        session.discard();
        return fail(app, format!("Could not start capturing: {e}"));
    }
    *st.session.lock() = Some(session);
    set_phase(app, "recording");
    let _ = app.emit("rec:countdown", 0);
    let a2 = app.clone();
    std::thread::spawn(move || ticker(a2));
    Ok(())
}

fn ticker(app: AppHandle) {
    let mut n: u32 = 0;
    let mut misses = 0;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        n += 1;
        // recording a window: when that window is closed, save what we have instead of recording an empty frame
        if n % 10 == 0 {
            let (is_win, hwnd) = {
                let st = app.state::<AppState>();
                let s = st.settings.lock();
                (s.source_kind == "window", s.window_hwnd.clone())
            };
            if is_win && !cfg!(not(windows)) && std::env::var_os("RECORA_MOCK").is_none() {
                if recorder::list_windows().iter().any(|w| w.hwnd == hwnd) {
                    misses = 0;
                } else {
                    misses += 1;
                }
                if misses >= 3 {
                    let _ = app.emit("rec:note", "The recorded window was closed, so the recording was saved.");
                    let a = app.clone();
                    std::thread::spawn(move || { let _ = do_stop(&a); });
                    break;
                }
            }
        }
        let st = app.state::<AppState>();
        let g = st.session.lock();
        let Some(s) = g.as_ref() else { break };
        use std::sync::atomic::Ordering::Relaxed;
        let _ = app.emit(
            "rec:tick",
            json!({
                "phase": st.phase.lock().clone(),
                "secs": s.recorded(),
                "mic": audio::Levels::get(&s.levels.mic),
                "sys": audio::Levels::get(&s.levels.sys),
                "micMuted": s.mic_muted.load(Relaxed),
                "sysMuted": s.sys_muted.load(Relaxed),
                "markers": s.markers.len(),
            }),
        );
    }
}

fn timestamp_name() -> String {
    chrono::Local::now().format("Recording %Y-%m-%d %H.%M.%S").to_string()
}

fn do_stop(app: &AppHandle) -> R<()> {
    let st = app.state::<AppState>();
    let ph = phase(app);
    if ph == "countdown" {
        set_phase(app, "idle");
        close_capsule(app);
        show_main(app);
        return Ok(());
    }
    if ph != "recording" && ph != "paused" {
        return Ok(());
    }
    set_phase(app, "saving");
    let session = st.session.lock().take();
    let out_dir = PathBuf::from(st.settings.lock().out_dir.clone());
    let open_after = st.settings.lock().open_after;
    let app2 = app.clone();
    std::thread::spawn(move || {
        let _ = app2.emit("rec:saving", true);
        let out = unique_path(&out_dir, &timestamp_name(), "mp4");
        let result: Result<Report, String> = match session {
            Some(s) => s.finish(&out).map_err(e2s),
            None => Err("No active recording".into()),
        };
        close_capsule(&app2);
        match result {
            Ok(rep) => {
                library::write_meta(Path::new(&rep.path), &rep.markers, &rep.cursor);
                let _ = app2.emit("rec:done", &rep);
            }
            Err(e) => {
                let _ = app2.emit("rec:error", &e);
            }
        }
        set_phase(&app2, "idle");
        if open_after {
            show_main(&app2);
        }
    });
    Ok(())
}

/// Stops and throws the recording away (nothing is saved).
fn do_cancel(app: &AppHandle) -> R<()> {
    let st = app.state::<AppState>();
    let ph = phase(app);
    if ph == "countdown" {
        return do_stop(app);
    }
    if ph != "recording" && ph != "paused" {
        return Ok(());
    }
    set_phase(app, "saving");
    let session = st.session.lock().take();
    let app2 = app.clone();
    std::thread::spawn(move || {
        if let Some(s) = session {
            s.discard();
        }
        close_capsule(&app2);
        set_phase(&app2, "idle");
        show_main(&app2);
        let _ = app2.emit("rec:cancelled", true);
    });
    Ok(())
}

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({i}).{ext}"));
        i += 1;
    }
    p
}

#[tauri::command]
async fn start_recording(app: AppHandle) -> R<()> {
    do_start(&app)
}
#[tauri::command]
async fn stop_recording(app: AppHandle) -> R<()> {
    do_stop(&app)
}
#[tauri::command]
async fn cancel_recording(app: AppHandle) -> R<()> {
    do_cancel(&app)
}

fn do_toggle_pause(app: &AppHandle) -> R<()> {
    let st = app.state::<AppState>();
    let ph = phase(app);
    let mut g = st.session.lock();
    let Some(s) = g.as_mut() else { return Ok(()) };
    if ph == "recording" {
        s.end_segment().map_err(e2s)?;
        drop(g);
        set_phase(app, "paused");
    } else if ph == "paused" {
        s.start_segment().map_err(e2s)?;
        drop(g);
        set_phase(app, "recording");
    }
    Ok(())
}

#[tauri::command]
async fn toggle_pause(app: AppHandle) -> R<()> {
    do_toggle_pause(&app)
}

#[tauri::command]
fn toggle_mute(state: State<AppState>, kind: String) {
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(s) = state.session.lock().as_ref() {
        let f = if kind == "mic" { &s.mic_muted } else { &s.sys_muted };
        f.store(!f.load(Relaxed), Relaxed);
    }
}

#[tauri::command]
fn add_marker(state: State<AppState>) {
    if let Some(s) = state.session.lock().as_mut() {
        s.add_marker();
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// region picker
// ---------------------------------------------------------------------------------------------------------------------
#[tauri::command]
async fn pick_region(app: AppHandle, state: State<'_, AppState>) -> R<()> {
    if app.get_webview_window("region").is_some() {
        return Ok(());
    }
    let mon_idx = state.settings.lock().monitor;
    let infos = recorder::list_monitors();
    let want = infos.get(mon_idx).map(|m| m.name.clone()).unwrap_or_default();
    let mons = app.available_monitors().map_err(e2s)?;
    let m = mons
        .iter()
        .find(|m| m.name().map(|n| *n == want).unwrap_or(false))
        .or_else(|| mons.get(mon_idx))
        .or_else(|| mons.first())
        .ok_or("No screen found")?;
    let sc = m.scale_factor();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    std::thread::sleep(Duration::from_millis(180));
    WebviewWindowBuilder::new(&app, "region", WebviewUrl::App(format!("region.html?mon={mon_idx}&scale={sc}").into()))
        .title("Choose area")
        .position(m.position().x as f64 / sc, m.position().y as f64 / sc)
        .inner_size(m.size().width as f64 / sc, m.size().height as f64 / sc)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()
        .map_err(e2s)?;
    Ok(())
}

#[tauri::command]
async fn region_done(app: AppHandle, state: State<'_, AppState>, x: u32, y: u32, w: u32, h: u32) -> R<()> {
    {
        let mut s = state.settings.lock();
        let mon = s.monitor;
        s.region = Some(Region { monitor: mon, x, y, w, h });
        s.source_kind = "region".into();
        s.save(&state.settings_path);
    }
    if let Some(r) = app.get_webview_window("region") {
        let _ = r.close();
    }
    show_main(&app);
    let _ = app.emit("settings:changed", true);
    Ok(())
}

/// The recording pill grows when you point at it; this keeps it centred on the same spot.
#[tauri::command]
fn island_size(app: AppHandle, w: f64, h: f64) -> R<()> {
    let Some(win) = app.get_webview_window("capsule") else { return Ok(()) };
    let sc = win.scale_factor().map_err(e2s)?;
    let pos = win.outer_position().map_err(e2s)?;
    let size = win.outer_size().map_err(e2s)?;
    let (nw, nh) = ((w * sc).round() as i32, (h * sc).round() as i32);
    let cx = pos.x + size.width as i32 / 2;
    let _ = win.set_size(tauri::PhysicalSize::new(nw as u32, nh as u32));
    let _ = win.set_position(tauri::PhysicalPosition::new(cx - nw / 2, pos.y));
    Ok(())
}

#[tauri::command]
async fn source_preview(state: State<'_, AppState>) -> R<Option<String>> {
    let s = state.settings.lock().clone();
    let Ok(spec) = build_spec(&s) else { return Ok(None) };
    tauri::async_runtime::spawn_blocking(move || recorder::preview(&spec.source)).await.map_err(e2s)
}

#[tauri::command]
async fn pick_zone(app: AppHandle, state: State<'_, AppState>) -> R<()> {
    if app.get_webview_window("region").is_some() {
        return Ok(());
    }
    let mon_idx = state.settings.lock().monitor;
    let m = tauri_monitor(&app, mon_idx).ok_or("No screen found")?;
    let sc = m.scale_factor();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    std::thread::sleep(Duration::from_millis(180));
    WebviewWindowBuilder::new(&app, "region", WebviewUrl::App(format!("region.html?mon={mon_idx}&scale={sc}&zone=1").into()))
        .title("Choose area")
        .position(m.position().x as f64 / sc, m.position().y as f64 / sc)
        .inner_size(m.size().width as f64 / sc, m.size().height as f64 / sc)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()
        .map_err(e2s)?;
    Ok(())
}

/// The picker closed with a rectangle (pixels of the monitor). If an app window is under it, the zone follows that window.
#[tauri::command]
async fn zone_done(app: AppHandle, state: State<'_, AppState>, x: u32, y: u32, w: u32, h: u32) -> R<()> {
    if let Some(r) = app.get_webview_window("region") {
        let _ = r.close();
    }
    let mon_idx = state.settings.lock().monitor;
    let origin = tauri_monitor(&app, mon_idx).map(|m| (m.position().x, m.position().y)).unwrap_or((0, 0));
    let (sx, sy) = (origin.0 + x as i32, origin.1 + y as i32);
    std::thread::sleep(Duration::from_millis(250)); // let the picker disappear before looking at what is underneath
    let under = recorder::window_at(sx + w as i32 / 2, sy + h as i32 / 2);
    let id = chrono::Local::now().timestamp_millis() as u64;
    let zone = match under {
        Some((proc, _title, (l, t, r, b))) if r > l && b > t => {
            let (ww, wh) = ((r - l) as f64, (b - t) as f64);
            let fx = ((sx - l) as f64 / ww).clamp(0.0, 1.0);
            let fy = ((sy - t) as f64 / wh).clamp(0.0, 1.0);
            let fw = (w as f64 / ww).clamp(0.0, 1.0 - fx);
            let fh = (h as f64 / wh).clamp(0.0, 1.0 - fy);
            let stem = proc.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
            let mut name = stem.clone();
            if let Some(c) = name.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            settings::HideZone { id, name, process: Some(stem + ".exe"), monitor: mon_idx, x: fx, y: fy, w: fw, h: fh, on: true }
        }
        _ => settings::HideZone { id, name: "Screen area".into(), process: None, monitor: mon_idx, x: x as f64, y: y as f64, w: w as f64, h: h as f64, on: true },
    };
    {
        let mut st = state.settings.lock();
        st.hide_zones.push(zone);
        st.hide_zones_on = true;
        st.save(&state.settings_path);
    }
    show_main(&app);
    let _ = app.emit("settings:changed", true);
    Ok(())
}

#[tauri::command]
async fn region_cancel(app: AppHandle) -> R<()> {
    if let Some(r) = app.get_webview_window("region") {
        let _ = r.close();
    }
    show_main(&app);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// library
// ---------------------------------------------------------------------------------------------------------------------
#[tauri::command]
async fn lib_list(state: State<'_, AppState>) -> R<Vec<library::Item>> {
    let dir = PathBuf::from(state.settings.lock().out_dir.clone());
    tauri::async_runtime::spawn_blocking(move || library::list(&dir)).await.map_err(e2s)
}
#[tauri::command]
async fn lib_rename(path: String, name: String) -> R<String> {
    library::rename(Path::new(&path), &name).map(|p| p.to_string_lossy().to_string()).map_err(e2s)
}
#[tauri::command]
async fn lib_delete(path: String) -> R<()> {
    library::delete(Path::new(&path)).map_err(e2s)
}
#[tauri::command]
async fn lib_trim(path: String, start: f64, end: f64, precise: bool) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || library::trim(Path::new(&path), start, end, precise).map(|p| p.to_string_lossy().to_string()).map_err(e2s))
        .await
        .map_err(e2s)?
}
#[tauri::command]
async fn lib_export(path: String, kind: String) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || library::export(Path::new(&path), &kind).map(|p| p.to_string_lossy().to_string()).map_err(e2s))
        .await
        .map_err(e2s)?
}
#[tauri::command]
async fn edit_export(project: edit::Project) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || edit::export(&project).map(|p| p.to_string_lossy().to_string()).map_err(e2s)).await.map_err(e2s)?
}
#[tauri::command]
async fn edit_strip(state: State<'_, AppState>, path: String, n: u32) -> R<String> {
    let work = PathBuf::from(state.settings.lock().out_dir.clone()).join(".recora").join("strips");
    tauri::async_runtime::spawn_blocking(move || edit::strip(Path::new(&path), &work, n.clamp(4, 40)).map(|p| p.to_string_lossy().to_string()).map_err(e2s)).await.map_err(e2s)?
}
/// Starts recording the microphone for a voice-over in the editor.
#[tauri::command]
fn voice_start(state: State<'_, AppState>) -> R<()> {
    let mut g = state.voice.lock();
    if g.is_some() {
        return Err("Already recording".into());
    }
    let s = state.settings.lock().clone();
    let dir = PathBuf::from(&s.out_dir).join(".recora").join("voice");
    std::fs::create_dir_all(&dir).map_err(e2s)?;
    let seg = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as usize).unwrap_or(0);
    let levels = recorder::audio::Levels::new();
    let rec = recorder::audio::AudioRec::start(recorder::audio::AudioSpec { mic: true, mic_device: s.mic_device.clone(), system: false }, dir, seg, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), levels.clone());
    *g = Some((rec, levels));
    Ok(())
}
/// How loud the microphone is right now (0 to 1), so the person can see it is working.
#[tauri::command]
fn voice_level(state: State<'_, AppState>) -> f32 {
    state.voice.lock().as_ref().map(|(_, l)| recorder::audio::Levels::get(&l.mic)).unwrap_or(0.0)
}
/// Stops the voice-over and returns the saved file.
#[tauri::command]
async fn voice_stop(state: State<'_, AppState>) -> R<String> {
    let taken = state.voice.lock().take();
    let Some((rec, _)) = taken else { return Err("Not recording".into()) };
    tauri::async_runtime::spawn_blocking(move || {
        let res = rec.stop();
        match res.mic {
            Some(w) if w.path.exists() => Ok(w.path.to_string_lossy().to_string()),
            _ => Err(res.errors.first().cloned().unwrap_or_else(|| "The microphone recorded nothing".into())),
        }
    })
    .await
    .map_err(e2s)?
}
#[tauri::command]
async fn edit_wave(state: State<'_, AppState>, path: String) -> R<String> {
    let work = PathBuf::from(state.settings.lock().out_dir.clone()).join(".recora").join("strips");
    tauri::async_runtime::spawn_blocking(move || edit::wave(Path::new(&path), &work).map(|p| p.map(|x| x.to_string_lossy().to_string()).unwrap_or_default()).map_err(e2s)).await.map_err(e2s)?
}
#[tauri::command]
async fn edit_analyze(path: String) -> R<edit::Analysis> {
    tauri::async_runtime::spawn_blocking(move || edit::analyze(Path::new(&path)).map_err(e2s)).await.map_err(e2s)?
}
/// Basic facts about any video the person picks, so the editor can open it without the library.
#[tauri::command]
async fn edit_analyze_quick(path: String) -> R<edit::Analysis> {
    tauri::async_runtime::spawn_blocking(move || edit::analyze_quick(Path::new(&path)).map_err(e2s)).await.map_err(e2s)?
}
#[tauri::command]
fn cap_status(state: State<AppState>) -> captions::Status {
    captions::status(&state.settings_path.parent().map(|p| p.join("models")).unwrap_or_default())
}

fn models_dir(state: &State<AppState>) -> PathBuf {
    state.settings_path.parent().map(|p| p.join("models")).unwrap_or_default()
}

#[tauri::command]
async fn cap_download(app: AppHandle, state: State<'_, AppState>, id: String) -> R<()> {
    let dir = models_dir(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let a = app.clone();
        let idc = id.clone();
        captions::download(&dir, &id, None, &move |done, total| {
            let _ = a.emit("cap:dl", serde_json::json!({ "id": idc, "done": done, "total": total }));
        })
        .map_err(e2s)
    })
    .await
    .map_err(e2s)?
}

#[tauri::command]
fn cap_cancel() {
    captions::CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
async fn cap_run(app: AppHandle, state: State<'_, AppState>, path: String, model: String, lang: String) -> R<Vec<captions::Word>> {
    let mp = captions::model_path(&models_dir(&state), &model);
    tauri::async_runtime::spawn_blocking(move || {
        let a = app.clone();
        let words = captions::transcribe(Path::new(&path), &mp, &lang, &move |p| {
            let _ = a.emit("cap:prog", p);
        })
        .map_err(e2s)?;
        library::write_words(Path::new(&path), &words, &lang, &model);
        Ok(words)
    })
    .await
    .map_err(e2s)?
}

#[tauri::command]
fn cap_words(path: String) -> Option<serde_json::Value> {
    library::read_words(Path::new(&path))
}

#[tauri::command]
fn cap_save(path: String, words: Vec<captions::Word>, lang: String, model: String) {
    library::write_words(Path::new(&path), &words, &lang, &model);
}

#[tauri::command]
fn edit_cursor(path: String) -> Vec<[f32; 4]> {
    library::read_cursor(Path::new(&path))
}

#[tauri::command]
fn edit_plan(analysis: edit::Analysis, dur: f64, level: Option<u8>) -> edit::Plan {
    edit::plan(&analysis, dur, level.unwrap_or(2))
}
#[tauri::command]
async fn edit_probe(path: String) -> R<library::Item> {
    tauri::async_runtime::spawn_blocking(move || library::probe(Path::new(&path)).map_err(e2s)).await.map_err(e2s)?
}
#[tauri::command]
async fn ai_edit(state: State<'_, AppState>, instruction: String, facts: ai::Facts) -> R<ai::Reply> {
    let (p, k, m, b) = {
        let s = state.settings.lock();
        let p = s.ai_provider.clone();
        let k = s.ai_keys.get(&p).cloned().unwrap_or_else(|| s.ai_key.clone());
        let m = s.ai_models.get(&p).cloned().unwrap_or_else(|| s.ai_model.clone());
        (p, k, m, s.ai_base.clone())
    };
    tauri::async_runtime::spawn_blocking(move || ai::edit(&ai::Cfg { provider: &p, key: &k, model: &m, base: &b }, &instruction, &facts).map_err(e2s)).await.map_err(e2s)?
}
#[derive(serde::Serialize)]
struct ModelList {
    models: Vec<String>,
    pick: Option<String>,
}
#[tauri::command]
async fn ai_models(provider: String, key: String, base: Option<String>) -> R<ModelList> {
    let base = base.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        ai::list_models(&ai::Cfg { provider: &provider, key: &key, model: "", base: &base }).map(|(models, pick)| ModelList { models, pick }).map_err(e2s)
    })
    .await
    .map_err(e2s)?
}
#[tauri::command]
async fn ai_test(provider: String, key: String, model: String, base: Option<String>) -> R<()> {
    let base = base.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || ai::test(&ai::Cfg { provider: &provider, key: &key, model: &model, base: &base }).map_err(e2s)).await.map_err(e2s)?
}
#[tauri::command]
fn ai_estimate(instruction: String, facts: ai::Facts) -> u64 {
    ai::estimate_tokens(&instruction, &facts)
}
/// Opens a help page in the browser. Only https links are allowed.
#[tauri::command]
fn open_url(url: String) -> R<()> {
    if !url.starts_with("https://") {
        return Err("Only https links can be opened".into());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(e2s)
}
#[tauri::command]
fn reveal(path: String) -> R<()> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(e2s)
}
#[tauri::command]
fn open_file(path: String) -> R<()> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(e2s)
}
#[tauri::command]
fn open_folder(state: State<AppState>) -> R<()> {
    let d = state.settings.lock().out_dir.clone();
    let _ = std::fs::create_dir_all(&d);
    tauri_plugin_opener::open_path(d, None::<&str>).map_err(e2s)
}

// ---------------------------------------------------------------------------------------------------------------------
// hotkeys, tray, boot
// ---------------------------------------------------------------------------------------------------------------------
fn register_hotkeys(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let s = app.state::<AppState>().settings.lock().clone();
    for hk in [&s.hotkey_toggle, &s.hotkey_pause, &s.hotkey_marker] {
        if let Ok(sc) = hk.parse::<Shortcut>() {
            let _ = gs.register(sc);
        }
    }
}

fn on_hotkey(app: &AppHandle, sc: &Shortcut) {
    let s = app.state::<AppState>().settings.lock().clone();
    let is = |t: &str| t.parse::<Shortcut>().map(|x| x == *sc).unwrap_or(false);
    let a = app.clone();
    if is(&s.hotkey_toggle) {
        if phase(app) == "idle" {
            std::thread::spawn(move || {
                let _ = do_start(&a);
            });
        } else {
            let _ = do_stop(app);
        }
    } else if is(&s.hotkey_pause) {
        std::thread::spawn(move || {
            let _ = do_toggle_pause(&a);
        });
    } else if is(&s.hotkey_marker) {
        if let Some(sess) = app.state::<AppState>().session.lock().as_mut() {
            sess.add_marker();
        }
        let _ = app.emit("rec:marker", true);
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        on_hotkey(app, shortcut);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from("."));
            let path = dir.join("settings.json");
            let s = Settings::load(&path);
            let _ = std::fs::create_dir_all(&s.out_dir);
            app.manage(AppState { settings: Mutex::new(s), settings_path: path, session: Mutex::new(None), phase: Mutex::new("idle".into()), voice: Mutex::new(None) });
            register_hotkeys(app.handle());
            recorder::warm_up();
            let open = MenuItem::with_id(app, "open", "Open Recora", true, None::<&str>)?;
            let rec = MenuItem::with_id(app, "rec", "Start / stop recording", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &rec, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("Recora").show_menu_on_left_click(false);
            if let Some(i) = app.default_window_icon() {
                tray = tray.icon(i.clone());
            }
            tray.on_menu_event(|app, ev| match ev.id.as_ref() {
                "open" => show_main(app),
                "rec" => {
                    let a = app.clone();
                    if phase(app) == "idle" {
                        std::thread::spawn(move || {
                            let _ = do_start(&a);
                        });
                    } else {
                        let _ = do_stop(app);
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, ev| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                    show_main(tray.app_handle());
                }
            })
            .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings, set_settings, get_phase, get_sources, source_preview, island_size, pick_zone, zone_done, list_apps, launch_app, preflight, start_recording, stop_recording, cancel_recording, toggle_pause, toggle_mute,
            add_marker, pick_region, region_done, region_cancel, lib_list, lib_rename, lib_delete, lib_trim, lib_export, edit_export, edit_strip, edit_wave, voice_start, voice_stop, voice_level, edit_analyze, edit_analyze_quick, edit_plan, edit_cursor, cap_status, cap_download, cap_cancel, cap_run, cap_words, cap_save, edit_probe, ai_edit, ai_test, ai_models, ai_estimate, open_url, reveal,
            open_file, open_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Recora");
}
