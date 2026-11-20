//! App picker helpers: the list of apps installed on the PC (Start menu), launching one, and finding its window.
use crate::recorder::{self, WindowInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppEntry {
    pub name: String,
    pub id: String, // AppUserModelID (works for classic and Store apps)
}

/// Parses the JSON that `Get-StartApps | ConvertTo-Json` prints. One app comes back as an object, several as an array.
pub fn parse_start_apps(json: &str) -> Vec<AppEntry> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim().trim_start_matches('\u{feff}')) else { return vec![] };
    let items: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a,
        o @ serde_json::Value::Object(_) => vec![o],
        _ => vec![],
    };
    let mut seen = HashSet::new();
    let mut out: Vec<AppEntry> = items
        .iter()
        .filter_map(|i| {
            let name = i.get("Name")?.as_str()?.trim().to_string();
            let id = i.get("AppID")?.as_str()?.trim().to_string();
            let low = name.to_lowercase();
            if name.is_empty() || id.is_empty() || low.contains("uninstall") || low.starts_with("recora") {
                return None;
            }
            seen.insert(id.to_lowercase()).then_some(AppEntry { name, id })
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

pub fn list_installed() -> Vec<AppEntry> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | ConvertTo-Json -Compress"])
                .creation_flags(0x0800_0000)
                .output();
            if let Ok(o) = out {
                return parse_start_apps(&String::from_utf8_lossy(&o.stdout));
            }
            return vec![];
        }
    }
    ["Calculator", "Notepad", "Google Chrome", "Microsoft Word", "Zoom", "memoQ", "Visual Studio Code", "Paint"]
        .iter()
        .map(|n| AppEntry { name: n.to_string(), id: format!("sample.{n}") })
        .collect()
}

fn stem(p: &str) -> String {
    p.trim_end_matches(".exe").trim_end_matches(".EXE").to_lowercase()
}

/// Is a window of this app already open? Compares the app name with the process name and the window title.
pub fn find_open(name: &str, windows: &[WindowInfo]) -> Option<WindowInfo> {
    let n = name.to_lowercase();
    let squash = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let ns = squash(&n);
    windows
        .iter()
        .find(|w| {
            let p = squash(&stem(&w.process));
            let t = squash(&w.title.to_lowercase());
            (p.len() >= 3 && (ns.contains(&p) || p.contains(&ns))) || (ns.len() >= 4 && t.contains(&ns))
        })
        .cloned()
}

/// The first window that was not there before the launch; a window that matches the app name wins.
pub fn pick_new(before: &HashSet<String>, now: &[WindowInfo], name: &str) -> Option<WindowInfo> {
    let fresh: Vec<&WindowInfo> = now.iter().filter(|w| !before.contains(&w.hwnd)).collect();
    if fresh.is_empty() {
        return None;
    }
    let only: Vec<WindowInfo> = fresh.iter().map(|w| (*w).clone()).collect();
    find_open(name, &only).or_else(|| Some(only[0].clone()))
}

#[derive(Serialize)]
pub struct LaunchResult {
    pub window: WindowInfo,
    pub reused: bool,
}

pub fn launch(entry: &AppEntry, wait: Duration) -> Result<LaunchResult, String> {
    let before_list = recorder::list_windows();
    if let Some(w) = find_open(&entry.name, &before_list) {
        return Ok(LaunchResult { window: w, reused: true });
    }
    let before: HashSet<String> = before_list.iter().map(|w| w.hwnd.clone()).collect();
    start_process(entry)?;
    let t0 = Instant::now();
    while t0.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(400));
        let now = recorder::list_windows();
        if let Some(w) = pick_new(&before, &now, &entry.name) {
            std::thread::sleep(Duration::from_millis(600)); // let the splash screen turn into the real window
            let again = recorder::list_windows();
            let w = pick_new(&before, &again, &entry.name).unwrap_or(w);
            return Ok(LaunchResult { window: w, reused: false });
        }
    }
    Err(format!("{} did not open a window in {} seconds. Open it yourself, then choose it from the \"Open now\" tab.", entry.name, wait.as_secs()))
}

fn start_process(entry: &AppEntry) -> Result<(), String> {
    #[cfg(windows)]
    {
        if std::env::var_os("RECORA_MOCK").is_none() {
            use std::os::windows::process::CommandExt;
            return std::process::Command::new("explorer.exe")
                .arg(format!("shell:AppsFolder\\{}", entry.id))
                .creation_flags(0x0800_0000)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Could not open {}: {e}", entry.name));
        }
    }
    let _ = entry;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(h: &str, t: &str, p: &str) -> WindowInfo {
        WindowInfo { hwnd: h.into(), title: t.into(), process: p.into() }
    }
    #[test]
    fn parses_array_and_single_object() {
        let a = parse_start_apps(r#"[{"Name":"Zoom","AppID":"Zoom.Z"},{"Name":"Calculator","AppID":"calc!App"},{"Name":"Uninstall Zoom","AppID":"u"}]"#);
        assert_eq!(a.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Calculator", "Zoom"]);
        assert_eq!(parse_start_apps("\u{feff}{\"Name\":\"Paint\",\"AppID\":\"p\"}").len(), 1);
        assert!(parse_start_apps("garbage").is_empty());
    }
    #[test]
    fn finds_already_open_app() {
        let ws = [w("1", "Inbox - Mail", "olk.exe"), w("2", "Untitled - Notepad", "Notepad.exe"), w("3", "memoQ - LQA", "memoQ.exe")];
        assert_eq!(find_open("Notepad", &ws).unwrap().hwnd, "2");
        assert_eq!(find_open("memoQ 10", &ws).unwrap().hwnd, "3");
        assert!(find_open("Zoom", &ws).is_none());
    }
    #[test]
    fn picks_the_new_window_not_the_old_ones() {
        let before: HashSet<String> = ["1".to_string()].into();
        let now = [w("1", "Old", "a.exe"), w("5", "Splash", "x.exe"), w("6", "Calculator", "CalculatorApp.exe")];
        assert_eq!(pick_new(&before, &now, "Calculator").unwrap().hwnd, "6");
        assert_eq!(pick_new(&before, &[w("1", "Old", "a.exe")], "Calculator"), None);
    }
}
