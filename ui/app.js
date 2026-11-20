(() => {
  const { T, icon: I, $, $$, esc, invoke, listen, fileUrl, fmt, size, applyTheme, toast } = R;
  let S = null;
  let SRC = { monitors: [], windows: [], mics: [], platform: "" };
  let phase = "idle";
  let lib = [];
  let libQ = "";
  let checks = [];
  let newClip = false;
  let page = "record";

  const debounce = (f, ms) => { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => f(...a), ms); }; };
  const persist = debounce(() => invoke("set_settings", { new: S }), 200);
  const set = (k, v, rerender = true) => { S[k] = v; persist(); if (k === "theme") applyTheme(v); if (rerender) render(); if (k === "mic" || k === "out_dir") preflight(); };

  // ---------------------------------------------------------------- shell
  function buildShell() {
    const win = T.window.getCurrentWindow();
    $("#winbtns").innerHTML = `<button id="wmin" title="Minimize">${I("min")}</button><button id="wmax" title="Maximize">${I("max")}</button><button id="wclose" class="close" title="Close">${I("x")}</button>`;
    $("#wmin").onclick = () => win.minimize();
    $("#wmax").onclick = () => win.toggleMaximize();
    $("#wclose").onclick = () => (phase === "idle" ? win.close() : win.hide());
    const nav = [["record", "Record", "record"], ["library", "Library", "library"], ["settings", "Settings", "settings"]];
    $("#rail").innerHTML = `<div class="brand"><img src="logo.png" alt="">Recora</div>` + nav.map(([id, label, ic]) => `<button class="nav" data-go="${id}">${I(ic)}<span>${label}</span><i class="dot" hidden></i></button>`).join("") + `<div class="spacer"></div><button class="nav" id="open-folder">${I("folder")}<span>Open folder</span></button>`;
    $$("[data-go]").forEach((b) => (b.onclick = () => go(b.dataset.go)));
    $("#open-folder").onclick = () => invoke("open_folder").catch((e) => toast(e, true));
  }
  function go(p) {
    page = p;
    $$(".page").forEach((el) => el.classList.toggle("on", el.id === "p-" + p));
    $$("[data-go]").forEach((b) => b.classList.toggle("on", b.dataset.go === p));
    if (p === "library") { newClip = false; $$(".dot")[1].hidden = true; refreshLib(); }
    if (p === "record") preflight();
  }

  // ---------------------------------------------------------------- record page
  const DRM = ["apple tv", "appletv", "netflix", "disney", "prime video", "primevideo", "hulu", "peacock", "paramount", "hbo", "crunchyroll"];
  const isDrm = (t) => { t = String(t || "").toLowerCase(); return DRM.some((d) => t.includes(d)); };
  function row(ic, title, sub, key, extra = "", color = "") {
    const on = S[key];
    return `<div class="row">${ic ? `<div class="ic" style="--c:${color || "#8e8e93"}">${I(ic)}</div>` : ""}<div class="t"><b>${title}</b>${sub ? `<span>${sub}</span>` : ""}${extra}</div><button class="switch ${on ? "on" : ""}" data-sw="${key}" role="switch" aria-checked="${!!on}" aria-label="${title}"></button></div>`;
  }
  function segHtml(key, opts, fit) {
    return `<div class="seg ${fit ? "fit" : ""}" data-seg="${key}">${opts.map(([v, l]) => `<button data-v="${v}" class="${String(S[key]) === String(v) ? "on" : ""}">${l}</button>`).join("")}</div>`;
  }
  function segRow(title, key, opts) {
    return `<div class="row"><div class="t"><b>${title}</b></div>${segHtml(key, opts, true)}</div>`;
  }
  function sourceRow() {
    const m = SRC.monitors[Math.min(S.monitor, SRC.monitors.length - 1)] || { width: 1920, height: 1080 };
    if (S.source_kind === "window") {
      const cur = SRC.windows.find((w) => w.hwnd === S.window_hwnd);
      const sub = !S.window_hwnd ? "No app chosen" : cur ? esc((cur.process || "").replace(/\.exe$/i, "") || "Open now") : "Not open any more. Choose it again.";
      return `<div class="row"><div class="ic" style="--c:#0a84ff">${I("window")}</div><div class="t"><b>${S.window_hwnd ? esc(S.window_title) : "Choose an app"}</b><span>${sub}</span></div><button class="btn sm ${S.window_hwnd ? "" : "primary"}" id="pick-app">${S.window_hwnd ? "Change…" : "Choose…"}</button></div>`;
    }
    if (S.source_kind === "region") {
      return `<div class="row"><div class="ic" style="--c:#0a84ff">${I("region")}</div><div class="t"><b>${S.region ? `${S.region.w} × ${S.region.h} px` : "Choose an area"}</b><span>${S.region ? "Saved area" : "Drag on your screen to choose it"}</span></div><button class="btn sm ${S.region ? "" : "primary"}" id="draw-region">${S.region ? "Change…" : "Choose…"}</button></div>`;
    }
    const multi = SRC.monitors.length > 1;
    return `<div class="row"><div class="ic" style="--c:#0a84ff">${I("screen")}</div><div class="t"><b>${multi ? "Display" : "Entire screen"}</b><span>${m.width} × ${m.height}</span></div>${multi ? `<select id="mon">${SRC.monitors.map((x, i) => `<option value="${i}" ${i === S.monitor ? "selected" : ""}>Display ${i + 1}</option>`).join("")}</select>` : ""}</div>`;
  }
  function zonesLine() {
    const z = (S.hide_zones || []).filter((x) => x.on);
    if (!(S.hide_zones || []).length) return "Nothing marked yet";
    return z.length ? z.map((x) => esc(x.name)).join(", ") : "All marked areas are turned off";
  }
  // ---- live picture of the chosen source
  const pv = { key: "", url: null, busy: false };
  const srcKey = () => [S.source_kind, S.monitor, S.window_hwnd, JSON.stringify(S.region)].join("|");
  function heroHtml() {
    const m = SRC.monitors[Math.min(S.monitor, SRC.monitors.length - 1)] || { width: 1920, height: 1080 };
    let cap = `Display ${S.monitor + 1} · ${m.width} × ${m.height}`;
    if (S.source_kind === "window") cap = S.window_hwnd ? S.window_title : "No app chosen";
    if (S.source_kind === "region") cap = S.region ? `Area · ${S.region.w} × ${S.region.h}` : "No area chosen";
    let zones = "";
    if (S.hide_zones_on && S.source_kind === "screen") {
      zones = (S.hide_zones || []).filter((z) => z.on && !z.process && z.monitor === S.monitor).map((z) => `<i class="zbox" style="left:${(z.x / m.width) * 100}%;top:${(z.y / m.height) * 100}%;width:${(z.w / m.width) * 100}%;height:${(z.h / m.height) * 100}%"><span>Blurred</span></i>`).join("");
    }
    const ready = (S.source_kind === "screen") || (S.source_kind === "window" && S.window_hwnd) || (S.source_kind === "region" && S.region);
    const body = pv.url ? `<img src="${pv.url}" alt="">${zones}` : `<div class="ph">${I(S.source_kind === "window" ? "window" : S.source_kind === "region" ? "region" : "screen")}<span>${ready ? (pv.busy ? "Loading preview…" : "Preview unavailable") : "Choose what to record"}</span></div>`;
    return `<div class="hero ${pv.url ? "has" : ""}" id="hero">${body}<div class="cap"><span>${esc(cap)}</span></div><button class="rf" id="pv-refresh" title="Refresh preview" aria-label="Refresh preview">${I("refresh")}</button></div>`;
  }
  async function loadPreview(force) {
    const k = srcKey();
    if (pv.busy || (!force && k === pv.key && pv.url)) return;
    pv.busy = true; pv.key = k;
    try { pv.url = (await invoke("source_preview")) || null; } catch (_) { pv.url = null; }
    pv.busy = false;
    if (page === "record" && k === srcKey()) { const h = $("#hero"); if (h) h.outerHTML = heroHtml(), wireHero(); }
  }
  function wireHero() { const b = $("#pv-refresh"); if (b) b.onclick = () => { pv.url = null; const h = $("#hero"); if (h) h.outerHTML = heroHtml(), wireHero(); loadPreview(true); }; }
  function sentence() {
    const aud = [S.mic && "your voice", S.system_audio && "system sound"].filter(Boolean);
    return aud.length ? "With " + aud.join(" and ") + "." : "Without sound.";
  }
  function renderRecord() {
    const mics = SRC.mics.length > 1 ? `<select id="mic-dev" style="margin-top:6px;max-width:300px"><option value="">Default microphone</option>${SRC.mics.map((m) => `<option ${S.mic_device === m ? "selected" : ""}>${esc(m)}</option>`).join("")}</select>` : "";
    const drmApp = S.source_kind === "window" && isDrm(S.window_title);
    const bad = checks.filter((c) => !c.ok);
    $("#p-record").innerHTML = `<div class="col">
      <div class="head"><div><h1>Record</h1><p class="sub">${sentence()}</p></div></div>
      ${segHtml("source_kind", [["screen", "Screen"], ["window", "App"], ["region", "Area"]])}
      ${heroHtml()}
      <div class="group" style="margin-top:12px">${sourceRow()}</div>
      ${drmApp ? `<div class="notice">${I("warn")}<span>${esc(S.window_title)} protects its video, so Windows may record the picture as black. You can still record it. The sound is not affected.</span></div>` : ""}
      <div class="label">Sound</div>
      <div class="group">
        ${row("mic", "Microphone", "", "mic", mics, "#ff9500")}
        ${SRC.platform === "windows" ? row("speaker", "System sound", "", "system_audio", "", "#ff2d55") : ""}
      </div>
      <div class="label">Picture</div>
      <div class="group">
        ${row("cursor", "Show the cursor", "", "show_cursor", "", "#8e8e93")}
        ${segRow("Quality", "quality", [["standard", "Standard"], ["high", "High"], ["max", "Max"]])}
        ${segRow("Frame rate", "fps", [[30, "30"], [60, "60"]])}
      </div>
      <div class="label">Privacy</div>
      <div class="group">
        <div class="row"><div class="ic" style="--c:#5856d6">${I("shield")}</div><div class="t"><b>Blur chat apps</b><span>${blurLine()}</span></div><button class="btn sm" id="pick-blur">Choose apps…</button><button class="switch ${S.blur_apps_on ? "on" : ""}" data-sw="blur_apps_on" role="switch" aria-checked="${!!S.blur_apps_on}" aria-label="Blur chat apps"></button></div>
      </div>
      <div class="label">Start</div>
      <div class="group">
        ${segRow("Countdown", "countdown", [[0, "None"], [3, "3 s"], [5, "5 s"]])}
      </div>
      ${bad.length ? `<div class="issues">${bad.map((c) => `<div class="notice">${I("warn")}<span>${esc(c.label)}${c.detail ? ". " + esc(c.detail) : ""}</span></div>`).join("")}</div>` : ""}
      ${S.source_kind !== "window" ? `<p class="foot" style="text-align:center;margin-top:22px">Video from apps like Apple TV and Netflix is protected by Windows and may record as black.</p>` : ""}
    </div>
    <div class="start"><button class="startbtn ${phase !== "idle" ? "busy" : ""}" id="go-rec">Start recording</button><small>or press <kbd>${esc(S.hotkey_toggle)}</kbd> from any app</small></div>`;
    wireRecord(); wireHero();
    if (pv.key !== srcKey() || !pv.url) loadPreview();
  }
  function wireRecord() {
    const root = $("#p-record");
    $$("[data-sw]", root).forEach((b) => (b.onclick = () => set(b.dataset.sw, !S[b.dataset.sw])));
    $$("[data-seg]", root).forEach((g) => $$("button", g).forEach((b) => (b.onclick = () => {
      const k = g.dataset.seg; let v = b.dataset.v; if (["fps", "countdown"].includes(k)) v = +v;
      set(k, v);
    })));
    const pa = $("#pick-app", root); if (pa) pa.onclick = openAppPicker;
    const mon = $("#mon", root); if (mon) mon.onchange = () => set("monitor", +mon.value);
    const md = $("#mic-dev", root); if (md) md.onchange = () => set("mic_device", md.value, false);
    const dr = $("#draw-region", root); if (dr) dr.onclick = () => invoke("pick_region").catch((e) => toast(e, true));
    $("#pick-blur", root).onclick = openBlurPicker;
    $("#go-rec", root).onclick = async () => {
      try { await invoke("start_recording"); } catch (e) { toast(e, true); }
    };
  }
  async function preflight() {
    try { checks = (await invoke("preflight")).checks; } catch (_) { checks = []; }
    if (page === "record") renderRecord();
  }

  // ---------------------------------------------------------------- library
  async function refreshLib() {
    try { lib = await invoke("lib_list"); } catch (e) { lib = []; }
    renderLib();
  }
  function renderLib() {
    const q = libQ.trim().toLowerCase();
    const items = lib.filter((x) => !q || x.name.toLowerCase().includes(q));
    const body = !lib.length
      ? `<div class="empty"><div class="big">${I("film")}</div><h2>No recordings</h2><p>Recordings you make appear here.</p><button class="btn primary" data-go-rec>Record</button></div>`
      : !items.length ? `<div class="empty"><h2>No match</h2><p>Nothing is called “${esc(libQ)}”.</p></div>`
      : `<div class="grid">${items.map((x) => `<article class="clip" data-path="${esc(x.path)}"><div class="th" ${x.thumb ? `data-th="${esc(fileUrl(x.thumb))}"` : ""}>${x.thumb ? "" : I(x.ext === "mp3" ? "music" : "film")}<span class="kind">${esc(x.ext.toUpperCase())}</span>${x.markers.length ? `<span class="mk">${I("flag")}${x.markers.length}</span>` : ""}${x.duration ? `<span class="dur">${fmt(x.duration)}</span>` : ""}</div><div class="meta"><b>${esc(x.name)}</b><span>${when(x.modified)} · ${size(x.size)}</span></div></article>`).join("")}</div>`;
    $("#p-library").innerHTML = `<div class="head"><div><h1>Library</h1></div>
      <div class="tools"><input type="search" id="lq" placeholder="Search recordings" value="${esc(libQ)}"><button class="btn" id="lf">${I("folder")}Open folder</button></div></div>${body}`;
    $$("[data-th]").forEach((el) => (el.style.backgroundImage = `url("${el.dataset.th}")`));
    const lq = $("#lq"); lq.oninput = () => { libQ = lq.value; const pos = lq.selectionStart; renderLib(); const n = $("#lq"); n.focus(); n.setSelectionRange(pos, pos); };
    $("#lf").onclick = () => invoke("open_folder");
    const g = $("[data-go-rec]"); if (g) g.onclick = () => go("record");
    $$(".clip").forEach((c) => (c.onclick = () => { const it = lib.find((x) => x.path === c.dataset.path); if (it) openClip(it); }));
  }
  function when(sec) {
    const d = new Date(sec * 1000), now = new Date();
    const same = d.toDateString() === now.toDateString();
    const y = new Date(now - 864e5).toDateString() === d.toDateString();
    const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return same ? "Today " + t : y ? "Yesterday " + t : d.toLocaleDateString([], { day: "numeric", month: "short" }) + " " + t;
  }



  // ---------------------------------------------------------------- blur apps
  const normP = (p) => String(p || "").split(/[\\/]/).pop().replace(/\.exe$/i, "").trim().toLowerCase();
  const blurSet = () => new Set((S.blur_apps || []).map(normP));
  function openNow() {
    const m = new Map();
    for (const w of SRC.windows) { const k = normP(w.process); if (k && !m.has(k)) m.set(k, { key: k, name: (w.process || "").replace(/\.exe$/i, ""), title: w.title }); }
    return [...m.values()];
  }
  function blurLine() {
    if (!S.blur_apps_on) return "Off. Everything on the screen is recorded as it is.";
    const set = blurSet(), open = openNow().filter((a) => set.has(a.key));
    return open.length ? esc(open.map((a) => a.name).join(", ")) + (open.length === 1 ? " is open and will be blurred" : " are open and will be blurred") : "No chat app is open right now";
  }
  async function openBlurPicker() {
    SRC = await invoke("get_sources");
    let q = "";
    const el = document.createElement("div"); el.className = "veil"; el.id = "blurpick"; document.body.appendChild(el);
    const close = () => { el.remove(); document.removeEventListener("keydown", onKey); renderRecord(); if (page === "settings") renderSettings(); };
    const onKey = (e) => { if (e.key === "Escape") close(); };
    document.addEventListener("keydown", onKey);
    el.addEventListener("mousedown", (e) => { if (e.target === el) close(); });
    const toggle = (name, key) => {
      const set = blurSet();
      S.blur_apps = set.has(key) ? S.blur_apps.filter((a) => normP(a) !== key) : [...(S.blur_apps || []), name];
      persist(); draw();
    };
    function draw() {
      const set = blurSet(), open = openNow().filter((a) => !q || a.name.toLowerCase().includes(q.toLowerCase()) || a.title.toLowerCase().includes(q.toLowerCase()));
      const openKeys = new Set(openNow().map((a) => a.key));
      const saved = (S.blur_apps || []).filter((a) => !openKeys.has(normP(a)));
      const tick = (on) => `<span class="tick ${on ? "on" : ""}">${on ? I("check") : ""}</span>`;
      el.innerHTML = `<div class="modal apppick"><div class="ap-head"><div><h3>Apps to blur</h3><p class="muted">Their windows are blurred in the recording, only where you can see them. Windows in front of them stay clear.</p></div><button class="close-x flat" id="bx">${I("x")}</button></div>
        <div class="ap-tools"><label class="search">${I("search")}<input id="bq" placeholder="Search open apps" value="${esc(q)}" autocomplete="off"></label><button class="btn" id="brf">Refresh</button></div>
        <div class="ap-body">
          <div class="label sm">Open now</div>
          ${open.length ? `<div class="apps">${open.map((a) => `<button class="app" data-k="${esc(a.key)}" data-n="${esc(a.name)}"><div class="av">${esc(a.name.charAt(0))}</div><div class="t"><b>${esc(a.name)}</b><span>${esc(a.title)}</span></div>${tick(set.has(a.key))}</button>`).join("")}</div>` : `<div class="empty">No open app matches.</div>`}
          ${saved.length && !q ? `<div class="label sm">Blurred whenever they are open</div><div class="apps">${saved.map((a) => `<button class="app" data-k="${esc(normP(a))}" data-n="${esc(a)}"><div class="av">${esc(a.charAt(0))}</div><div class="t"><b>${esc(a)}</b><span>Not open now</span></div>${tick(true)}</button>`).join("")}</div>` : ""}
        </div>
        <div class="ap-foot muted">Apps in a web browser (WhatsApp Web, for example) are part of the browser window, so choosing the browser would blur every tab.</div></div>`;
      $("#bx", el).onclick = close;
      const inp = $("#bq", el); inp.oninput = () => { q = inp.value; const p = inp.selectionStart; draw(); const n = $("#bq", el); n.focus(); n.setSelectionRange(p, p); };
      $("#brf", el).onclick = async () => { SRC = await invoke("get_sources"); draw(); };
      $$(".app", el).forEach((b) => (b.onclick = () => toggle(b.dataset.n, b.dataset.k)));
    }
    draw();
  }

  // ---------------------------------------------------------------- app picker (open apps + apps installed on the PC)
  const RK = "recora.recentApps";
  const recents = () => { try { return JSON.parse(localStorage.getItem(RK) || "[]"); } catch { return []; } };
  const remember = (a) => { try { localStorage.setItem(RK, JSON.stringify([a, ...recents().filter((r) => r.id !== a.id)].slice(0, 6))); } catch {} };
  let INSTALLED = null;
  async function openAppPicker() {
    closePicker();
    SRC = await invoke("get_sources");
    let tab = "open", q = "", busy = null;
    const el = document.createElement("div");
    el.className = "veil"; el.id = "apppick";
    document.body.appendChild(el);
    const close = () => el.remove();
    el.addEventListener("mousedown", (e) => { if (e.target === el && !busy) close(); });
    const onKey = (e) => { if (e.key === "Escape" && !busy) { close(); document.removeEventListener("keydown", onKey); } };
    document.addEventListener("keydown", onKey);
    const match = (t) => !q || t.toLowerCase().includes(q.toLowerCase());
    async function choose(w, note) {
      S.window_hwnd = w.hwnd; S.window_title = w.title; persist(); close(); render();
      if (note) toast(note);
    }
    async function start(a) {
      busy = a; draw();
      try {
        const r = await invoke("launch_app", { name: a.name, id: a.id });
        remember(a);
        SRC = await invoke("get_sources");
        await choose(r.window, r.reused ? `${a.name} was already open. Its window is selected.` : `${a.name} is open and selected. Press Start when you are ready.`);
      } catch (e) { busy = null; draw(); toast(e, true); }
    }
    function draw() {
      let list = "";
      if (tab === "open") {
        const ws = SRC.windows.filter((w) => match(w.title) || match(w.process || ""));
        list = ws.length ? `<div class="apps">${ws.map((w) => `<button class="app ${S.window_hwnd === w.hwnd ? "on" : ""}" data-h="${esc(w.hwnd)}"><div class="av">${esc((w.process || w.title).charAt(0))}</div><div class="t"><b>${esc(w.title)}</b><span>${esc((w.process || "").replace(/\.exe$/i, ""))}</span></div></button>`).join("")}</div>`
          : `<div class="empty">${q ? "No open window matches your search." : "No open windows found. Open the app, then press Refresh."}</div>`;
      } else if (!INSTALLED) {
        list = `<div class="empty"><span class="spin"></span>Reading the apps on your PC…</div>`;
      } else {
        const rec = q ? [] : recents();
        const all = INSTALLED.filter((a) => match(a.name));
        const card = (a) => `<button class="app ${busy && busy.id === a.id ? "busy" : ""}" data-id="${esc(a.id)}" data-n="${esc(a.name)}" ${busy ? "disabled" : ""}><div class="av">${busy && busy.id === a.id ? '<span class="spin"></span>' : esc(a.name.charAt(0))}</div><div class="t"><b>${esc(a.name)}</b><span>${busy && busy.id === a.id ? "Opening… this can take a few seconds" : "Open and record"}</span></div></button>`;
        list = (rec.length ? `<div class="label sm">Recent</div><div class="apps">${rec.map(card).join("")}</div><div class="label sm">All apps</div>` : "") +
          (all.length ? `<div class="apps">${all.map(card).join("")}</div>` : `<div class="empty">No app matches your search.</div>`);
      }
      el.innerHTML = `<div class="modal apppick"><div class="ap-head"><div><h3>Choose an app to record</h3><p class="muted">Only that app's window is recorded.</p></div><button class="close-x flat" id="apx">${I("x")}</button></div>
        <div class="ap-tools"><div class="seg" id="aptabs"><button data-t="open" class="${tab === "open" ? "on" : ""}">Open now</button><button data-t="launch" class="${tab === "launch" ? "on" : ""}">Start an app</button></div>
        <label class="search">${I("search")}<input id="apq" placeholder="${tab === "open" ? "Search open windows" : "Search apps on this PC"}" value="${esc(q)}" autocomplete="off"></label>
        ${tab === "open" ? `<button class="btn" id="aprf">Refresh</button>` : ""}</div>
        <div class="ap-body">${list}</div>
        <div class="ap-foot muted">${tab === "open" ? "Apps that are running right now." : "Recora starts the app, waits for its window, then selects it. Apps that run as administrator may need Recora to run as administrator too."}</div></div>`;
      $("#apx", el).onclick = () => !busy && close();
      $$("#aptabs button", el).forEach((b) => (b.onclick = async () => {
        if (busy) return; tab = b.dataset.t; q = ""; draw();
        if (tab === "launch" && !INSTALLED) { INSTALLED = await invoke("list_apps"); draw(); }
      }));
      const inp = $("#apq", el); inp.oninput = () => { q = inp.value; const pos = inp.selectionStart; draw(); const n = $("#apq", el); n.focus(); n.setSelectionRange(pos, pos); };
      if (!busy) inp.focus();
      const rf = $("#aprf", el); if (rf) rf.onclick = async () => { SRC = await invoke("get_sources"); draw(); };
      $$(".app[data-h]", el).forEach((b) => (b.onclick = () => { const w = SRC.windows.find((x) => x.hwnd === b.dataset.h); if (w) choose(w); }));
      $$(".app[data-id]", el).forEach((b) => (b.onclick = () => start({ id: b.dataset.id, name: b.dataset.n })));
    }
    draw();
  }
  function closePicker() { const e = $("#apppick"); if (e) e.remove(); }

  // ---------------------------------------------------------------- clip modal (result + player + trim + export)
  function openClip(it, opts = {}) {
    closeClip();
    const isVid = ["mp4", "mkv", "webm", "mov"].includes(it.ext);
    const isAud = ["mp3", "m4a"].includes(it.ext);
    const dur = it.duration || 0;
    let a = 0, b = dur;
    const el = document.createElement("div");
    el.className = "veil"; el.id = "clip";
    const media = isVid ? `<video id="vid" src="${fileUrl(it.path)}" controls preload="metadata"></video>` : isAud ? `<audio id="vid" src="${fileUrl(it.path)}" controls style="width:90%;margin:40px auto"></audio>` : `<img src="${fileUrl(it.path)}" style="width:100%">`;
    el.innerHTML = `<div class="modal"><div class="vid"><button class="close-x" id="cx">${I("x")}</button>${media}</div>
      <div class="info">
        ${opts.fresh ? `<div class="chip" style="align-self:flex-start;color:var(--ok)">${I("check")} Saved</div>` : ""}
        <input class="name" id="nm" value="${esc(it.name)}" spellcheck="false" title="Click to rename">
        <div class="stats"><span class="chip">${fmt(dur)}</span><span class="chip">${size(it.size)}</span><span class="chip">${esc(it.ext.toUpperCase())}</span>${it.markers.length ? `<span class="chip">${I("flag")}${it.markers.length} ${it.markers.length === 1 ? "marker" : "markers"}</span>` : ""}</div>
        ${(opts.notes || []).map((n) => `<div class="note warn">${esc(n)}</div>`).join("")}
        ${isVid && dur > 1 ? `<button class="btn primary" id="ed-open" style="width:100%;height:38px">${I("film")}Edit video</button>` : ""}
        ${isVid && dur > 1 ? `<div class="trim"><div class="label">Trim</div><div class="range" id="rg"><div class="track"></div><div class="sel" id="sel"></div>${it.markers.map((m) => `<i class="mkr" style="left:${(m / dur) * 100}%" title="Marker ${fmt(m)}"></i>`).join("")}<i class="ph" id="ph"></i><div class="h" id="ha"></div><div class="h" id="hb"></div></div>
          <div class="times"><span id="ta">00:00</span><span id="tl"></span><span id="tb"></span></div>
          <div style="display:flex;gap:8px;margin-top:10px"><button class="btn sm" id="trim-fast" title="Instant, cuts at the nearest keyframe">${I("scissors")}Trim fast</button><button class="btn sm" id="trim-pre" title="Frame-accurate, takes longer">Trim exact</button></div></div>` : ""}
        <div><div class="label">Make a copy as</div><div class="exports">
          ${isVid ? `<button class="btn sm" data-x="gif">${I("gif")}GIF</button><button class="btn sm" data-x="mp3">${I("music")}Audio</button><button class="btn sm" data-x="vertical">${I("phone")}Vertical</button><button class="btn sm" data-x="small">${I("shrink")}Smaller</button><button class="btn sm" data-x="webm">WebM</button>` : ""}</div></div>
        <div class="spacer" style="flex:1"></div>
        <div class="actions"><button class="btn" id="op">${I("play")}Open</button><button class="btn" id="rv">${I("reveal")}Show in folder</button><button class="btn danger" id="dl" style="grid-column:1/3">${I("trash")}Move to recycle bin</button></div>
      </div></div>`;
    document.body.appendChild(el);
    el.addEventListener("mousedown", (e) => { if (e.target === el) closeClip(); });
    $("#cx").onclick = closeClip;
    const eo = $("#ed-open"); if (eo) eo.onclick = () => { closeClip(); openEditor(it, () => refreshLib()); };
    $("#op").onclick = () => invoke("open_file", { path: it.path });
    $("#rv").onclick = () => invoke("reveal", { path: it.path });
    $("#dl").onclick = async () => { if (confirm("Move this recording to the recycle bin?")) { try { await invoke("lib_delete", { path: it.path }); closeClip(); refreshLib(); } catch (e) { toast(e, true); } } };
    const nm = $("#nm");
    const rename = async () => { const v = nm.value.trim(); if (v && v !== it.name) { try { const p = await invoke("lib_rename", { path: it.path, name: v }); it.path = p; it.name = v; refreshLib(); toast("Renamed"); } catch (e) { toast(e, true); nm.value = it.name; } } };
    nm.onblur = rename; nm.onkeydown = (e) => { if (e.key === "Enter") nm.blur(); e.stopPropagation(); };
    $$("[data-x]", el).forEach((bt) => (bt.onclick = async () => {
      const old = bt.innerHTML; bt.disabled = true; bt.innerHTML = `<span class="spin"></span>Working…`;
      try { const out = await invoke("lib_export", { path: it.path, kind: bt.dataset.x }); toast("Saved: " + out.split(/[\\/]/).pop()); refreshLib(); } catch (e) { toast(e, true); }
      bt.disabled = false; bt.innerHTML = old;
    }));
    if (isVid && dur > 1) wireTrim(it, dur, (x, y) => { a = x; b = y; });
    const doTrim = (precise) => async (ev) => {
      const bt = ev.currentTarget; const old = bt.innerHTML; bt.disabled = true; bt.innerHTML = `<span class="spin"></span>Cutting…`;
      try { const out = await invoke("lib_trim", { path: it.path, start: a, end: b, precise }); toast("Saved: " + out.split(/[\\/]/).pop()); refreshLib(); } catch (e) { toast(e, true); }
      bt.disabled = false; bt.innerHTML = old;
    };
    const tf = $("#trim-fast"); if (tf) { tf.onclick = doTrim(false); $("#trim-pre").onclick = doTrim(true); }
  }
  function wireTrim(it, dur, onChange) {
    const rg = $("#rg"), ha = $("#ha"), hb = $("#hb"), sel = $("#sel"), ph = $("#ph"), vid = $("#vid");
    let a = 0, b = dur;
    const draw = () => {
      ha.style.left = (a / dur) * 100 + "%"; hb.style.left = (b / dur) * 100 + "%";
      sel.style.left = (a / dur) * 100 + "%"; sel.style.width = ((b - a) / dur) * 100 + "%";
      $("#ta").textContent = fmt(a); $("#tb").textContent = fmt(b); $("#tl").textContent = "Length " + fmt(b - a);
      onChange(a, b);
    };
    const drag = (h, which) => h.addEventListener("pointerdown", (e) => {
      h.setPointerCapture(e.pointerId);
      const mv = (ev) => {
        const r = rg.getBoundingClientRect();
        let t = Math.min(dur, Math.max(0, ((ev.clientX - r.left) / r.width) * dur));
        if (which === "a") a = Math.min(t, b - 0.3); else b = Math.max(t, a + 0.3);
        draw(); if (vid) vid.currentTime = which === "a" ? a : Math.max(a, b - 0.05);
      };
      const up = () => { h.removeEventListener("pointermove", mv); h.removeEventListener("pointerup", up); };
      h.addEventListener("pointermove", mv); h.addEventListener("pointerup", up);
    });
    drag(ha, "a"); drag(hb, "b"); draw();
    if (vid) vid.addEventListener("timeupdate", () => { ph.style.left = (vid.currentTime / dur) * 100 + "%"; });
  }
  function closeClip() { const c = $("#clip"); if (c) { const v = $("video", c); if (v) v.pause(); c.remove(); } }

  // ---------------------------------------------------------------- settings
  function renderSettings() {
    const hk = (k, label) => `<div class="row"><div class="t"><b>${label}</b></div><button class="btn hk" data-hk="${k}">${esc(S[k])}</button></div>`;
    const zones = (S.hide_zones || []).map((z) => `<div class="row zone" data-z="${z.id}"><div class="t"><b>${esc(z.name)}</b><span>${z.process ? "Follows the app window" : "Fixed place on the screen"}</span></div><button class="switch ${z.on ? "on" : ""}" data-zon="${z.id}" role="switch" aria-checked="${z.on}"></button><button class="x" data-zdel="${z.id}" title="Remove">${I("x")}</button></div>`).join("");
    $("#p-settings").innerHTML = `<div class="col"><div class="head"><h1>Settings</h1></div>
        <div class="label" style="margin-top:0">General</div>
        <div class="group">
          <div class="row"><div class="t"><b>Recordings folder</b></div><div class="path"><code title="${esc(S.out_dir)}">${esc(S.out_dir)}</code><button class="btn sm" id="chg">Change…</button></div></div>
          <div class="row"><div class="t"><b>Appearance</b></div>${segHtml("theme", [["system", "Auto"], ["light", "Light"], ["dark", "Dark"]], true)}</div>
          ${row("", "Show the recording when it's saved", "", "open_after")}
        </div>
        <div class="label">Shortcuts</div>
        <div class="group">
          ${hk("hotkey_toggle", "Start or stop")}
          ${hk("hotkey_pause", "Pause or resume")}
          ${hk("hotkey_marker", "Add a marker")}
        </div>
        <p class="foot">They work in any app. Click one, then press the new key. <button class="btn link" id="hk-reset">Reset to F9, F10, F8</button></p>
        <div class="label">Privacy</div>
        <div class="group">
          <div class="row"><div class="t"><b>Blur chat apps</b><span>${blurLine()}</span></div><button class="btn sm" id="pick-blur2">Choose apps…</button><button class="switch ${S.blur_apps_on ? "on" : ""}" data-sw="blur_apps_on" role="switch" aria-checked="${!!S.blur_apps_on}"></button></div>
        </div>
        <p class="foot">Text becomes unreadable and pictures unclear. The windows keep their look.</p>
        <div class="label">Marked areas (advanced)</div>
        <div class="group">
          ${row("", "Blur marked areas while recording", "A fixed part of the screen, whatever is shown there", "hide_zones_on")}
          ${zones}
          <div class="row"><button class="btn sm" id="add-zone">Mark an area…</button></div>
        </div>
        <p class="foot">Use this only for a part of a window. For whole chat apps, use "Blur chat apps" above.</p>
        <p class="foot" style="margin-top:28px">Recora 2.0 · LocMasters</p>
      </div>`;
    $("#pick-blur2").onclick = openBlurPicker;
    $("#add-zone").onclick = () => invoke("pick_zone").catch((e) => toast(e, true));
    $("#hk-reset").onclick = () => { S.hotkey_toggle = "F9"; S.hotkey_pause = "F10"; S.hotkey_marker = "F8"; persist(); renderSettings(); renderRecord(); };
    $$("[data-zon]").forEach((b) => (b.onclick = () => { const z = S.hide_zones.find((x) => String(x.id) === b.dataset.zon); if (z) { z.on = !z.on; persist(); renderSettings(); renderRecord(); } }));
    $$("[data-zdel]").forEach((b) => (b.onclick = () => { S.hide_zones = S.hide_zones.filter((x) => String(x.id) !== b.dataset.zdel); persist(); renderSettings(); renderRecord(); }));
    $$("[data-seg]", $("#p-settings")).forEach((g) => $$("button", g).forEach((b) => (b.onclick = () => set(g.dataset.seg, b.dataset.v))));
    $$("[data-sw]", $("#p-settings")).forEach((b) => (b.onclick = () => set(b.dataset.sw, !S[b.dataset.sw])));
    $("#chg").onclick = async () => {
      try { const d = await T.dialog.open({ directory: true, defaultPath: S.out_dir }); if (d) set("out_dir", d); } catch (e) { toast("Could not open the folder picker", true); }
    };
    $$("[data-hk]").forEach((b) => (b.onclick = () => {
      b.classList.add("listening"); b.textContent = "Press the keys…";
      const kd = (e) => {
        e.preventDefault();
        if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
        if (e.key === "Escape") { cleanup(); return; }
        const m = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
        let k = e.key.length === 1 ? e.key.toUpperCase() : e.key;
        if (!/^([A-Z]|F([1-9]|1[0-2]))$/.test(k) || (!m.length && !/^F/.test(k))) { toast("Use an F-key, or Ctrl/Alt/Shift with a letter", true); return; }
        set(b.dataset.hk, [...m, k].join("+"), false); cleanup();
      };
      const cleanup = () => { document.removeEventListener("keydown", kd, true); renderSettings(); };
      document.addEventListener("keydown", kd, true);
    }));
  }

  // ---------------------------------------------------------------- boot
  function render() { renderRecord(); renderSettings(); }
  async function boot() {
    buildShell();
    S = await invoke("get_settings");
    applyTheme(S.theme);
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => S.theme === "system" && applyTheme("system"));
    SRC = await invoke("get_sources");
    phase = await invoke("get_phase");
    render(); go("record");
    listen("rec:phase", (p) => { phase = p; if (p === "idle") renderRecord(); else $("#go-rec") && $("#go-rec").classList.add("busy"); });
    listen("rec:error", (m) => { toast(m, true); phase = "idle"; renderRecord(); });
    listen("rec:done", async (rep) => {
      newClip = true; $$(".dot")[1].hidden = false;
      await refreshLib();
      const it = lib.find((x) => x.path === rep.path);
      if (it) { it.markers = rep.markers && rep.markers.length ? rep.markers : it.markers; openClip(it, { fresh: true, notes: rep.notes }); }
      else if (rep.notes && rep.notes.length) toast(rep.notes[0], true);
    });
    listen("rec:note", (m) => toast(m));
    listen("rec:cancelled", () => { toast("Recording discarded"); });
    listen("settings:changed", async () => { S = await invoke("get_settings"); SRC = await invoke("get_sources"); render(); });
    document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeClip(); });
    preflight(); refreshLib();
  }
  boot().catch((e) => { document.body.insertAdjacentHTML("beforeend", `<pre style="padding:20px;color:#e33">${esc(e && e.stack || e)}</pre>`); });
})();
