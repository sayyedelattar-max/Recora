/* Browser stand-in for the Tauri bridge, used only when the page is opened outside Recora (design preview / tests). */
(function () {
  if (window.__TAURI__) return;
  const q = new URLSearchParams(location.search);
  const handlers = {};
  const svgThumb = (a, b, t) => "data:image/svg+xml;utf8," + encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' width='480' height='270'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs><rect width='480' height='270' fill='url(#g)'/><rect x='40' y='40' width='260' height='170' rx='12' fill='rgba(255,255,255,.55)'/><rect x='60' y='62' width='120' height='10' rx='5' fill='rgba(0,0,0,.15)'/><rect x='60' y='84' width='200' height='10' rx='5' fill='rgba(0,0,0,.12)'/><rect x='250' y='120' width='200' height='120' rx='12' fill='rgba(255,255,255,.75)'/></svg>`).replace(/'/g, '%27');
  const now = Math.floor(Date.now() / 1000);
  let S = { out_dir: "C:\\Users\\Sayed\\Videos\\Recora", source_kind: "screen", monitor: 0, window_hwnd: "", window_title: "", region: null, fps: 30, quality: "high", countdown: 3, hide_on_record: true, show_cursor: true, mic: true, mic_device: "", system_audio: true, theme: "system", hotkey_toggle: "F9", hotkey_pause: "F10", hotkey_marker: "F8", open_after: true, hide_zones: q.has('zones') ? [{ id: 1, name: 'Zoom', process: 'Zoom.exe', monitor: 0, x: .7, y: .1, w: .28, h: .8, on: true }] : [], hide_zones_on: true, blur_apps_on: true, blur_apps: ['Zoom', 'WhatsApp', 'Telegram'], labels_left: 3 };
  const lib = q.has("empty") ? [] : [
    { path: "C:\\v\\Recording 2026-11-18 17.43.12.mp4", name: "Recording 2026-11-18 17.43.12", ext: "mp4", size: 184e6, modified: now - 3600, duration: 372, thumb: svgThumb("#c9d6e8", "#e9d9e6"), markers: [40, 118, 250] },
    { path: "C:\\v\\LQA walkthrough.mp4", name: "LQA walkthrough", ext: "mp4", size: 512e6, modified: now - 86400, duration: 1260, thumb: svgThumb("#d8e8d2", "#e8e2c9"), markers: [] },
    { path: "C:\\v\\Bug repro (GIF).gif", name: "Bug repro (GIF)", ext: "gif", size: 9e6, modified: now - 172800, duration: 8, thumb: svgThumb("#e8c9c9", "#e9d9e6"), markers: [] },
    { path: "C:\\v\\Client call (audio).mp3", name: "Client call (audio)", ext: "mp3", size: 31e6, modified: now - 400000, duration: 2210, thumb: "", markers: [] },
    { path: "C:\\v\\Tutorial intro.mp4", name: "Tutorial intro", ext: "mp4", size: 77e6, modified: now - 700000, duration: 95, thumb: svgThumb("#cfd3f0", "#f0d3e6"), markers: [12] },
  ];
  const cmds = {
    get_settings: () => S,
    set_settings: ({ new: n }) => { S = n; },
    get_phase: () => "idle",
    get_sources: () => ({ platform: "windows", mics: ["Microphone (Realtek Audio)", "Headset (USB)"], monitors: [{ index: 0, name: "\\\\.\\DISPLAY1", width: 2560, height: 1600, primary: true }, { index: 1, name: "\\\\.\\DISPLAY2", width: 1920, height: 1080, primary: false }], windows: [{ hwnd: "11", title: "Zoom Meeting", process: "Zoom.exe" }, { hwnd: "12", title: "Memoq - LQA Project", process: "memoQ.exe" }, { hwnd: "13", title: "Google Chrome", process: "chrome.exe" }, { hwnd: "14", title: "Visual Studio Code", process: "Code.exe" }] }),
    preflight: () => ({ checks: [{ id: "engine", label: "Capture engine", ok: true, detail: "Windows Graphics Capture" }, { id: "disk", label: "182 GB free", ok: true, detail: "" }, { id: "mic", label: "Microphone", ok: true, detail: "" }] }),
    list_apps: async () => { await new Promise((r) => setTimeout(r, 400)); return ["Calculator", "Google Chrome", "Microsoft Word", "memoQ", "Notepad", "Paint", "Visual Studio Code", "Zoom"].map((n) => ({ name: n, id: "app." + n })); },
    launch_app: async ({ name }) => { await new Promise((r) => setTimeout(r, 1500)); return { reused: name === "Zoom", window: { hwnd: "90", title: name, process: name + ".exe" } }; },
    source_preview: async () => { await new Promise((r) => setTimeout(r, 350)); return svgThumb('#9db7d8', '#cdb9d8'); },
    lib_list: () => lib,
    start_recording: () => { setTimeout(() => emit("rec:phase", "countdown"), 0); },
    pick_region: () => { location.href = "region.html?scale=1.25"; },
  };
  const emit = (ev, p) => (handlers[ev] || []).forEach((cb) => cb({ payload: p }));
  window.__emit = emit;
  const win = { minimize() {}, toggleMaximize() {}, close() {}, hide() {} };
  window.__TAURI__ = {
    core: { invoke: async (c, a) => { if (cmds[c]) return cmds[c](a || {}); console.log("invoke", c, a); return null; }, convertFileSrc: (p) => p },
    event: { listen: async (ev, cb) => { (handlers[ev] = handlers[ev] || []).push(cb); return () => {}; } },
    window: { getCurrentWindow: () => win },
    dialog: { open: async () => "D:\\Recordings" },
  };
})();
