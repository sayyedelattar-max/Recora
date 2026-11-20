use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Region {
    pub monitor: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// A rectangle that is blurred while recording. With `process` set it belongs to that app's window
/// (x, y, w, h are fractions of the window and it follows the window); otherwise x, y, w, h are pixels of `monitor`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HideZone {
    pub id: u64,
    pub name: String,
    pub process: Option<String>,
    pub monitor: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub on: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub out_dir: String,
    pub source_kind: String, // screen | window | region
    pub monitor: usize,
    pub window_hwnd: String,
    pub window_title: String,
    pub region: Option<Region>,
    pub fps: u32,
    pub quality: String, // standard | high | max
    pub countdown: u32,
    pub hide_on_record: bool,
    pub show_cursor: bool,
    pub mic: bool,
    pub mic_device: String,
    pub system_audio: bool,
    pub theme: String, // system | light | dark
    pub hotkey_toggle: String,
    pub hotkey_pause: String,
    pub hotkey_marker: String,
    pub open_after: bool,
    pub hide_zones: Vec<HideZone>,
    pub hide_zones_on: bool,
    /// apps whose windows are blurred while recording (only where they are visible); e.g. chat apps
    pub blur_apps: Vec<String>,
    pub blur_apps_on: bool,
    pub labels_left: u32, // recordings left in which the recording pill shows all labels
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            out_dir: default_out_dir().to_string_lossy().to_string(),
            source_kind: "screen".into(),
            monitor: 0,
            window_hwnd: String::new(),
            window_title: String::new(),
            region: None,
            fps: 30,
            quality: "high".into(),
            countdown: 3,
            hide_on_record: true,
            show_cursor: true,
            mic: true,
            mic_device: String::new(),
            system_audio: true,
            theme: "system".into(),
            hotkey_toggle: "F9".into(),
            hotkey_pause: "F10".into(),
            hotkey_marker: "F8".into(),
            open_after: true,
            hide_zones: Vec::new(),
            hide_zones_on: true,
            blur_apps: ["Zoom", "ms-teams", "Teams", "WhatsApp", "Telegram", "Slack", "Discord", "Messenger", "Signal", "Skype", "Viber", "WeChat", "Element"].iter().map(|s| s.to_string()).collect(),
            blur_apps_on: true,
            labels_left: 3,
        }
    }
}

pub fn default_out_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let videos = base.join("Videos");
    (if videos.exists() { videos } else { base }).join("Recora")
}

impl Settings {
    pub fn load(path: &std::path::Path) -> Settings {
        let mut s: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // the first version used Ctrl+Shift combinations that were hard to remember
        for (cur, old, new) in [(&mut s.hotkey_toggle, "Ctrl+Shift+R", "F9"), (&mut s.hotkey_pause, "Ctrl+Shift+P", "F10"), (&mut s.hotkey_marker, "Ctrl+Shift+M", "F8")] {
            if cur.eq_ignore_ascii_case(old) {
                *cur = new.into();
            }
        }
        s
    }
    pub fn save(&self, path: &std::path::Path) {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, t);
        }
    }
    /// Target video bitrate (bits/s) for the hardware encoder.
    pub fn bitrate(&self, w: u32, h: u32) -> u32 {
        let px = (w as f64) * (h as f64);
        let factor = match self.quality.as_str() {
            "standard" => 0.07,
            "max" => 0.22,
            _ => 0.12,
        };
        let fps_scale = if self.fps >= 60 { 1.6 } else { 1.0 };
        ((px * 30.0 * factor * fps_scale) as u32).clamp(2_000_000, 120_000_000)
    }
}
