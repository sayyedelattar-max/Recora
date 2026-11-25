//! Mouse position and clicks while recording, written next to the video so the editor can add click rings and a
//! cursor glow afterwards. Only positions are kept, never keystrokes or what was clicked.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Desktop position in physical pixels and the buttons held (1 = left, 2 = right).
pub type Reader = Box<dyn Fn() -> Option<(i32, i32, u8)> + Send>;
/// The recorded area on the desktop right now: left, top, width, height.
pub type Area = Box<dyn Fn() -> Option<(i32, i32, i32, i32)> + Send>;

pub struct Tracker {
    stop: Arc<AtomicBool>,
    raw: Arc<Mutex<Vec<(Instant, f32, f32, u8)>>>,
    h: Option<JoinHandle<()>>,
}

impl Tracker {
    pub fn start(read: Reader, area: Area) -> Tracker {
        let stop = Arc::new(AtomicBool::new(false));
        let raw: Arc<Mutex<Vec<(Instant, f32, f32, u8)>>> = Arc::new(Mutex::new(Vec::new()));
        let (s2, r2) = (stop.clone(), raw.clone());
        let h = std::thread::spawn(move || {
            let mut last: Option<(f32, f32, u8, Instant)> = None;
            while !s2.load(Ordering::Relaxed) {
                if let (Some((x, y, b)), Some((ax, ay, aw, ah))) = (read(), area()) {
                    if aw > 0 && ah > 0 {
                        let (nx, ny) = ((x - ax) as f32 / aw as f32, (y - ay) as f32 / ah as f32);
                        let now = Instant::now();
                        let changed = last.map(|(lx, ly, lb, lt)| (lx - nx).abs() > 0.0004 || (ly - ny).abs() > 0.0004 || lb != b || now.duration_since(lt) > Duration::from_millis(400)).unwrap_or(true);
                        if changed {
                            let mut v = r2.lock().unwrap();
                            if v.len() < 600_000 {
                                v.push((now, nx, ny, b));
                            }
                            last = Some((nx, ny, b, now));
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(16));
            }
        });
        Tracker { stop, raw, h: Some(h) }
    }

    /// Takes everything seen since `from` as `[time, x, y, buttons]`. Time starts at `zero` (the first video frame) plus `base`.
    pub fn take(&self, from: Instant, zero: Instant, base: f64) -> Vec<[f32; 4]> {
        let mut v = self.raw.lock().unwrap();
        let keep: Vec<_> = v.iter().filter(|s| s.0 >= from).cloned().collect();
        v.clear();
        keep.into_iter()
            .map(|(t, x, y, b)| {
                let dt = if t >= zero { t.duration_since(zero).as_secs_f64() } else { 0.0 };
                [((base + dt) * 1000.0).round() as f32 / 1000.0, (x * 10000.0).round() / 10000.0, (y * 10000.0).round() / 10000.0, b as f32]
            })
            .collect()
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.h.take() {
            let _ = h.join();
        }
    }
}

#[cfg(windows)]
pub fn desktop_reader() -> Reader {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    Box::new(|| unsafe {
        let mut p = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut p) == 0 {
            return None;
        }
        let mut b = 0u8;
        if GetAsyncKeyState(VK_LBUTTON as i32) < 0 {
            b |= 1;
        }
        if GetAsyncKeyState(VK_RBUTTON as i32) < 0 {
            b |= 2;
        }
        Some((p.x, p.y, b))
    })
}

#[cfg(not(windows))]
pub fn desktop_reader() -> Reader {
    Box::new(|| None)
}

/// A window's rectangle on the desktop (physical pixels).
#[cfg(windows)]
pub fn window_area(hwnd: isize) -> Area {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    Box::new(move || unsafe {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetWindowRect(hwnd as _, &mut r) == 0 {
            return None;
        }
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    })
}

#[cfg(not(windows))]
pub fn window_area(_hwnd: isize) -> Area {
    Box::new(|| None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;

    #[test]
    fn follows_the_mouse_and_notes_clicks_inside_the_area() {
        let x = Arc::new(AtomicI32::new(100));
        let b = Arc::new(AtomicI32::new(0));
        let (x2, b2) = (x.clone(), b.clone());
        let t = Tracker::start(Box::new(move || Some((x2.load(Ordering::Relaxed), 200, b2.load(Ordering::Relaxed) as u8))), Box::new(|| Some((0, 0, 1000, 500))));
        let began = Instant::now();
        std::thread::sleep(Duration::from_millis(120));
        x.store(500, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(120));
        b.store(1, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(120));
        let s = t.take(began, began, 5.0);
        assert!(s.len() >= 3, "{s:?}");
        assert!(s.iter().any(|p| (p[1] - 0.5).abs() < 0.001 && (p[2] - 0.4).abs() < 0.001));
        assert!(s.iter().any(|p| p[3] == 1.0));
        assert!(s[0][0] >= 5.0 && s.windows(2).all(|w| w[1][0] >= w[0][0]));
        assert!(t.take(began, began, 0.0).is_empty(), "taken samples are removed");
    }
}
