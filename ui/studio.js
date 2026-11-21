/* Studio: pick a template, record or choose a clip for each scene, and get one finished video. */
(() => {
  const { icon: I, $, $$, esc, invoke, fmt, toast, ICONS, fileUrl } = R;
  ICONS.studio = '<path d="M5 19L15.5 8.5M14 4.5l.9 1.9 1.9.9-1.9.9-.9 1.9-.9-1.9-1.9-.9 1.9-.9zM19 12l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6z"/>';
  const VIDEO = ["mp4", "mkv", "webm", "mov"];
  const BGS = ["#101828", "#0b1f3a", "#1c1c1e", "#2b1a3d", "#14342b", "#3a1f1f"], ACCENTS = ["#0a84ff", "#34c759", "#ff9f0a", "#ff375f", "#bf5af2", "#30d5c8"];
  let uid = 1;
  const title = (text, sub) => ({ id: uid++, kind: "title", text, sub, dur: 3 });
  const clip = (label, hint, caption = "") => ({ id: uid++, kind: "clip", label, hint, path: "", name: "", len: 0, tidy: true, caption });
  const end = (text, sub) => ({ id: uid++, kind: "end", text, sub, dur: 3 });
  const TEMPLATES = [
    { id: "demo", name: "Product demo", desc: "Show what your product does, from problem to result", aspect: "16:9", icon: "#0a84ff", scenes: () => [title("Your product name", "One line about what it does"), clip("The starting point", "Show the problem, or where the user begins", "The problem"), clip("The key feature", "Show the most important thing in action", "How it works"), clip("The result", "Show what the user gets", "The result"), end("Try it today", "your-website.com")] },
    { id: "tutorial", name: "Tutorial", desc: "A clear how-to in three steps", aspect: "16:9", icon: "#34c759", scenes: () => [title("How to …", "A quick guide"), clip("Step 1", "Show the first step", "Step 1"), clip("Step 2", "Show the next step", "Step 2"), clip("Step 3", "Show the last step", "Step 3"), end("Thanks for watching", "")] },
    { id: "issue", name: "Issue report", desc: "Show a problem so someone can fix it fast", aspect: "16:9", icon: "#ff9f0a", scenes: () => [title("Issue report", "What went wrong"), clip("What happens", "Record the problem as it occurs", "What happens"), clip("What you expected", "Show how it should behave", "Expected"), end("Details in the ticket", "")] },
    { id: "short", name: "Short announcement", desc: "A vertical video for phones, about 30 seconds", aspect: "9:16", icon: "#ff375f", scenes: () => [title("What's new", "A quick look"), clip("The new thing", "Show the new feature or update", "New"), end("Try it now", "")] },
  ];
  const load = () => { try { return JSON.parse(localStorage.getItem("recora.studio") || "{}"); } catch (e) { return {}; } };
  const keep = () => { try { localStorage.setItem("recora.studio", JSON.stringify({ brand: st.brand })); } catch (e) {} };
  const st = { tpl: null, scenes: [], vtitle: "", aspect: "16:9", fit: "bars", view: "build", trans: "fade", brand: Object.assign({ bg: "#101828", accent: "#0a84ff", logo: "" }, load().brand || {}), waiting: null, busy: false, result: "" };
  let root = null;

  const dialog = () => window.__TAURI__ && window.__TAURI__.dialog;
  const sw = (list, cur, key) => `<div class="swr">${list.map((c) => `<button class="sw${c === cur ? " on" : ""}" data-k="${key}" data-c="${c}" style="background:${c}"></button>`).join("")}</div>`;
  const seg = (key, opts, cur) => `<div class="seg fit">${opts.map(([v, n]) => `<button data-sg="${key}" data-v="${v}" class="${cur === v ? "on" : ""}">${n}</button>`).join("")}</div>`;

  function render(r) { if (r) root = r; if (!root) return; st.tpl && st.view !== "gallery" ? builder() : gallery(); }
  // redraw without losing the scroll position
  function keep2(fn) {
    let sc = root; while (sc && sc !== document.body && sc.scrollHeight <= sc.clientHeight + 1) sc = sc.parentElement;
    const y = sc ? sc.scrollTop : 0; fn(); if (sc) sc.scrollTop = y;
  }
  const redraw = () => keep2(builder);
  const effLen = (s) => (s.kind === "clip" ? Math.max(0.2, ((s.te > s.ts ? Math.min(s.te, s.len || s.te) : s.len || 0) - (s.ts || 0)) / (s.speed || 1)) : s.dur || 3);
  const total = () => st.scenes.reduce((a, s) => a + (s.kind === "clip" && !s.path ? 0 : effLen(s)), 0);
  const ink = (bg) => { const h = (bg || "").replace("#", ""); if (h.length !== 6) return "#fff"; const v = (i) => parseInt(h.slice(i, i + 2), 16); return 0.299 * v(0) + 0.587 * v(2) + 0.114 * v(4) > 160 ? "#111" : "#fff"; };

  function gallery() {
    root.innerHTML = `<div class="col"><div class="head"><div><h1>Studio</h1><p class="sub">Pick a template. Record each scene, and Recora puts the finished video together.</p></div></div>
      ${st.tpl ? `<button class="tc resume" id="s-resume"><b>Continue "${esc(st.vtitle)}"</b><span>${st.scenes.length} scenes, about ${fmt(total())}</span></button>` : ""}
      <div class="tgal">${TEMPLATES.map((t) => `<button class="tc" data-t="${t.id}"><span class="tcv" style="--c:${t.icon}">${I("film")}<i>${t.aspect}</i></span><b>${t.name}</b><span>${t.desc}</span></button>`).join("")}</div>
      <p class="foot">Your recordings are never changed. Everything works offline.</p></div>`;
    const rs = $("#s-resume", root); if (rs) rs.onclick = () => { st.view = "build"; builder(); };
    $$("[data-t]", root).forEach((b) => (b.onclick = () => { const t = TEMPLATES.find((x) => x.id === b.dataset.t); st.tpl = t; st.scenes = t.scenes(); st.vtitle = t.name; st.aspect = t.aspect; st.result = ""; st.view = "build"; builder(); }));
  }

  const segS = (i, k, opts, cur) => `<div class="seg fit">${opts.map(([v, n]) => `<button data-ss="${i}|${k}|${v}" class="${String(cur) === String(v) ? "on" : ""}">${n}</button>`).join("")}</div>`;
  const colorRow = (i, k, label, cur, fallback) => `<div class="row2"><span>${label}</span><div class="cpick"><input type="color" data-ci="${i}|${k}" value="${cur || fallback}"><button class="lnk" data-cr="${i}|${k}" ${cur ? "" : "disabled"}>Use brand</button></div></div>`;

  function sceneCard(s, i) {
    const n = st.scenes.length, open = s.open;
    const head = `<div class="sh"><span class="grip" data-grip="${i}" title="Drag to reorder">⋮⋮</span><span class="sn">${i + 1}</span><div class="t"><b>${s.kind === "clip" ? esc(s.label) : s.kind === "title" ? "Title card" : "Closing card"}</b><span>${s.kind === "clip" ? esc(s.hint) : "Text on a plain background with your colours"}</span></div><button class="mini" data-open="${i}" title="More options" aria-expanded="${!!open}">${open ? "▴" : "▾"}</button><button class="mini" data-up="${i}" ${i === 0 ? "disabled" : ""}>↑</button><button class="mini" data-dn="${i}" ${i === n - 1 ? "disabled" : ""}>↓</button><button class="mini" data-dup="${i}" title="Duplicate this scene">⧉</button><button class="mini" data-rm="${i}" title="Remove this scene">×</button></div>`;
    if (s.kind !== "clip") {
      const bg = s.bg || st.brand.bg, ac = s.accent || st.brand.accent, k = s.size || 1;
      return `<div class="sc" data-card="${i}">${head}<div class="sb"><div class="cprev ${st.aspect === "9:16" ? "v" : st.aspect === "1:1" ? "sq" : ""}" style="background:${bg};color:${ink(bg)}"><b style="font-size:${(st.aspect === "9:16" ? 15 : 20) * k}px" dir="auto">${esc(s.text)}</b><i style="background:${ac}"></i><span style="font-size:${11 * k}px" dir="auto">${esc(s.sub)}</span></div>
        <input class="txt wide" data-f="text" data-i="${i}" dir="auto" placeholder="Title" value="${esc(s.text)}"><input class="txt wide" data-f="sub" data-i="${i}" dir="auto" placeholder="Smaller line (optional)" value="${esc(s.sub)}">
        ${open ? `<div class="adj"><div class="row2"><span>Shown for</span><input type="range" data-dur="${i}" min="1" max="10" step="0.5" value="${s.dur || 3}"><span class="muted" id="dv${i}">${s.dur || 3}s</span></div>
        <div class="row2"><span>Text size</span>${segS(i, "size", [[0.8, "Small"], [1, "Medium"], [1.3, "Large"]], s.size || 1)}</div>
        ${colorRow(i, "bg", "Background", s.bg, st.brand.bg)}${colorRow(i, "accent", "Accent", s.accent, st.brand.accent)}</div>` : ""}</div></div>`;
    }
    const slot = s.path ? `<div class="slot has"><span class="ok">${I("check")}</span><div class="t"><b>${esc(s.name)}</b><span>${fmt(s.len)} long${(s.ts || s.te || (s.speed && s.speed !== 1)) ? `, used ${fmt(effLen(s))}` : ""}</span></div><button class="btn sm" data-rec="${i}">Record again</button><button class="btn sm" data-pick="${i}">Replace</button></div>` : `<div class="slot"><button class="btn primary sm" data-rec="${i}">${I("record")}Record this scene</button><button class="btn sm" data-pick="${i}">Choose a video…</button><button class="btn sm" data-lib="${i}">From my recordings</button></div>`;
    const adj = open && s.path ? `<div class="adj"><video class="pv" id="pv${i}" src="${fileUrl(s.path)}#t=${s.ts || 0}" controls preload="metadata" playsinline></video>
        <div class="row2"><span>Use from</span><input class="num" type="number" min="0" step="0.1" data-n="${i}|ts" value="${(s.ts || 0).toFixed(1)}"><span>to</span><input class="num" type="number" min="0" step="0.1" data-n="${i}|te" value="${(s.te || s.len || 0).toFixed(1)}"></div>
        <div class="row2"><button class="btn sm" data-hs="${i}">Start here</button><button class="btn sm" data-he="${i}">End here</button><button class="btn sm" data-hr="${i}">Whole clip</button></div>
        <div class="row2"><span>Speed</span>${segS(i, "speed", [[0.5, "0.5×"], [1, "1×"], [1.5, "1.5×"], [2, "2×"], [4, "4×"]], s.speed || 1)}</div>
        <div class="row2"><span>Clip sound</span>${segS(i, "mute", [[0, "On"], [1, "Off"]], s.mute ? 1 : 0)}</div>
        <div class="row2"><span>Line position</span>${segS(i, "pos", [["bottom", "Bottom"], ["middle", "Middle"], ["top", "Top"]], s.pos || "bottom")}</div>
        <div class="row2"><span>Line stays</span><input type="range" data-cd="${i}" min="0" max="10" step="0.5" value="${s.cdur || 0}"><span class="muted" id="cv${i}">${s.cdur ? s.cdur + "s" : "Auto"}</span></div></div>` : open ? `<p class="muted" style="margin:0">Add a video to this scene first.</p>` : "";
    return `<div class="sc" data-card="${i}">${head}<div class="sb">${slot}<input class="txt wide" data-f="caption" data-i="${i}" dir="auto" placeholder="A short line shown at the start (optional)" value="${esc(s.caption)}"><label class="tidy"><button class="switch${s.tidy ? " on" : ""}" data-tidy="${i}" role="switch" aria-checked="${s.tidy}"></button><span>Tidy this clip: remove silences and fast-forward still moments</span></label>${adj}</div></div>`;
  }

  function builder() {
    const missing = st.scenes.filter((s) => s.kind === "clip" && !s.path).length;
    root.innerHTML = `<div class="col"><div class="head"><div><button class="lnk back" id="s-back">← Templates</button><h1>${esc(st.tpl.name)}</h1></div></div>
      <div class="label">Video name</div><div class="group"><div class="row"><input class="txt wide" id="s-title" dir="auto" value="${esc(st.vtitle)}"></div></div>
      <div class="label">Scenes <span class="muted" style="text-transform:none;letter-spacing:0">· about ${fmt(total())}</span></div><div class="scenes">${st.scenes.map(sceneCard).join("")}</div>
      <div class="adds"><button class="btn sm" data-add="clip">+ Clip scene</button><button class="btn sm" data-add="title">+ Title card</button><button class="btn sm" data-add="end">+ Closing card</button></div>
      <div class="label">Look</div><div class="group">
        <div class="row"><div class="t"><b>Format</b></div>${seg("aspect", [["16:9", "Wide"], ["9:16", "Vertical"], ["1:1", "Square"]], st.aspect)}</div>
        <div class="row"><div class="t"><b>Around the clip</b><span>What fills the space when a clip does not match the format</span></div>${seg("fit", [["bars", "Plain"], ["blur", "Blurred"]], st.fit)}</div>
        <div class="row"><div class="t"><b>Transition</b></div>${seg("trans", [["none", "None"], ["fade", "Fade"], ["slide", "Slide"], ["zoom", "Zoom"]], st.trans)}</div>
        <div class="row"><div class="t"><b>Background</b></div>${sw(BGS, st.brand.bg, "bg")}<input type="color" class="cp" data-bk="bg" value="${st.brand.bg}"></div>
        <div class="row"><div class="t"><b>Accent</b></div>${sw(ACCENTS, st.brand.accent, "accent")}<input type="color" class="cp" data-bk="accent" value="${st.brand.accent}"></div>
        <div class="row"><div class="t"><b>Logo</b><span>${st.brand.logo ? esc(st.brand.logo.split(/[\\/]/).pop()) : "Shown on the title and closing cards (PNG or JPG)"}</span></div><button class="btn sm" id="s-logo">${st.brand.logo ? "Change" : "Choose…"}</button>${st.brand.logo ? `<button class="btn sm" id="s-nologo">Remove</button>` : ""}</div>
      </div>
      <div class="sfoot">${st.result ? `<div class="sdone"><b>Your video is ready</b><span>${esc(st.result.split(/[\\/]/).pop())}</span></div><button class="btn" id="s-show">Show in folder</button><button class="btn" id="s-open">Play</button>` : `<span class="muted">${st.busy ? "Building your video. This can take a few minutes, and you can keep working." : missing ? `${missing} ${missing === 1 ? "scene still needs" : "scenes still need"} a video` : "Everything is ready"}</span>`}<button class="btn primary" id="s-make" ${st.busy || missing ? "disabled" : ""}>${st.busy ? `<span class="spin"></span>Making…` : "Make my video"}</button></div></div>`;
    wire();
  }

  function wire() {
    const S = (i) => st.scenes[+i], clamp = (v, a, b) => Math.min(b, Math.max(a, v));
    $("#s-back", root).onclick = () => { st.view = "gallery"; render(); };
    $("#s-title", root).oninput = (e) => (st.vtitle = e.target.value);
    $$("[data-f]", root).forEach((c) => (c.oninput = () => { const s = S(c.dataset.i); s[c.dataset.f] = c.value; st.result = ""; const p = c.closest(".sc"); if (s.kind !== "clip" && p) { const cp = $(".cprev", p); if (cp) { $("b", cp).textContent = s.text; $("span", cp).textContent = s.sub; } } }));
    $$("[data-sg]", root).forEach((b) => (b.onclick = () => { st[b.dataset.sg] = b.dataset.v; redraw(); }));
    $$("[data-k]", root).forEach((b) => (b.onclick = () => { st.brand[b.dataset.k] = b.dataset.c; keep(); redraw(); }));
    $$("[data-bk]", root).forEach((c) => { c.oninput = () => { st.brand[c.dataset.bk] = c.value; }; c.onchange = () => { keep(); redraw(); }; });
    $$("[data-rm]", root).forEach((b) => (b.onclick = () => { st.scenes.splice(+b.dataset.rm, 1); redraw(); }));
    $$("[data-up]", root).forEach((b) => (b.onclick = () => { const i = +b.dataset.up; [st.scenes[i - 1], st.scenes[i]] = [st.scenes[i], st.scenes[i - 1]]; redraw(); }));
    $$("[data-dn]", root).forEach((b) => (b.onclick = () => { const i = +b.dataset.dn; [st.scenes[i + 1], st.scenes[i]] = [st.scenes[i], st.scenes[i + 1]]; redraw(); }));
    $$("[data-dup]", root).forEach((b) => (b.onclick = () => { const i = +b.dataset.dup; st.scenes.splice(i + 1, 0, { ...st.scenes[i], id: uid++, open: false }); redraw(); }));
    $$("[data-open]", root).forEach((b) => (b.onclick = () => { const s = S(b.dataset.open); s.open = !s.open; redraw(); }));
    $$("[data-tidy]", root).forEach((b) => (b.onclick = () => { const s = S(b.dataset.tidy); s.tidy = !s.tidy; redraw(); }));
    $$("[data-add]", root).forEach((b) => (b.onclick = () => { const k = b.dataset.add; st.scenes.splice(k === "end" ? st.scenes.length : k === "title" ? 0 : st.scenes.length - (st.scenes[st.scenes.length - 1].kind === "end" ? 1 : 0), 0, k === "clip" ? clip("Another scene", "Show the next part", "") : k === "title" ? title("Title", "") : end("Thanks", "")); redraw(); }));
    // scene options
    $$("[data-ss]", root).forEach((b) => (b.onclick = () => { const [i, k, v] = b.dataset.ss.split("|"), s = S(i); if (k === "mute") s.mute = v === "1"; else if (k === "pos") s.pos = v; else s[k] = isNaN(+v) ? v : +v; st.result = ""; redraw(); }));
    $$("[data-dur]", root).forEach((r) => (r.oninput = () => { const s = S(r.dataset.dur); s.dur = +r.value; $("#dv" + r.dataset.dur, root).textContent = s.dur + "s"; }));
    $$("[data-cd]", root).forEach((r) => (r.oninput = () => { const s = S(r.dataset.cd); s.cdur = +r.value; $("#cv" + r.dataset.cd, root).textContent = s.cdur ? s.cdur + "s" : "Auto"; }));
    $$("[data-ci]", root).forEach((c) => { const [i, k] = c.dataset.ci.split("|"); c.oninput = () => { S(i)[k] = c.value; const p = c.closest(".sc"), cp = p && $(".cprev", p); if (cp) { if (k === "bg") { cp.style.background = c.value; cp.style.color = ink(c.value); } else $("i", cp).style.background = c.value; } }; c.onchange = redraw; });
    $$("[data-cr]", root).forEach((b) => (b.onclick = () => { const [i, k] = b.dataset.cr.split("|"); S(i)[k] = ""; redraw(); }));
    $$("[data-n]", root).forEach((inp) => { inp.onkeydown = (e) => e.stopPropagation(); inp.onchange = () => { const [i, k] = inp.dataset.n.split("|"), s = S(i), v = Math.max(0, +inp.value || 0); if (k === "ts") s.ts = clamp(v, 0, (s.te || s.len) - 0.3); else s.te = clamp(v, (s.ts || 0) + 0.3, s.len || v); if (s.te >= s.len - 0.05) s.te = 0; if (s.ts < 0.05) s.ts = 0; st.result = ""; redraw(); }; });
    const pv = (i) => $("#pv" + i, root);
    $$("[data-hs]", root).forEach((b) => (b.onclick = () => { const s = S(b.dataset.hs), v = pv(b.dataset.hs); s.ts = clamp(v.currentTime, 0, (s.te || s.len) - 0.3); redraw(); }));
    $$("[data-he]", root).forEach((b) => (b.onclick = () => { const s = S(b.dataset.he), v = pv(b.dataset.he); s.te = clamp(v.currentTime, (s.ts || 0) + 0.3, s.len); if (s.te >= s.len - 0.05) s.te = 0; redraw(); }));
    $$("[data-hr]", root).forEach((b) => (b.onclick = () => { const s = S(b.dataset.hr); s.ts = 0; s.te = 0; redraw(); }));
    // drag to reorder from the grip
    $$("[data-grip]", root).forEach((g) => {
      const card = g.closest(".sc"); g.onmousedown = () => (card.draggable = true);
      card.ondragstart = (e) => { e.dataTransfer.setData("text/plain", g.dataset.grip); e.dataTransfer.effectAllowed = "move"; card.classList.add("dragging"); };
      card.ondragend = () => { card.draggable = false; card.classList.remove("dragging"); $$(".sc.over", root).forEach((x) => x.classList.remove("over")); };
      card.ondragover = (e) => { e.preventDefault(); card.classList.add("over"); }; card.ondragleave = () => card.classList.remove("over");
      card.ondrop = (e) => { e.preventDefault(); const from = +e.dataTransfer.getData("text/plain"), to = +g.dataset.grip; if (isNaN(from) || from === to) return; const [m] = st.scenes.splice(from, 1); st.scenes.splice(to, 0, m); redraw(); };
    });
    $$("[data-rec]", root).forEach((b) => (b.onclick = () => startRecording(+b.dataset.rec)));
    $$("[data-pick]", root).forEach((b) => (b.onclick = () => pickFile(+b.dataset.pick)));
    $$("[data-lib]", root).forEach((b) => (b.onclick = () => pickLib(+b.dataset.lib)));
    $("#s-logo", root).onclick = async () => { try { const f = await dialog().open({ multiple: false, filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg"] }] }); if (f) { st.brand.logo = Array.isArray(f) ? f[0] : f; keep(); redraw(); } } catch (e) { toast(e, true); } };
    const nl = $("#s-nologo", root); if (nl) nl.onclick = () => { st.brand.logo = ""; keep(); redraw(); };
    const sh = $("#s-show", root); if (sh) { sh.onclick = () => invoke("reveal", { path: st.result }).catch(() => {}); $("#s-open", root).onclick = () => invoke("open_file", { path: st.result }).catch((e) => toast(e, true)); }
    $("#s-make", root).onclick = make;
  }

  async function assign(i, it) { const s = st.scenes[i]; if (!s) return; s.path = it.path; s.name = it.name; s.len = it.duration || 0; s.ts = 0; s.te = 0; st.result = ""; st.view = "build"; render(); }
  async function pickFile(i) {
    try { const f = await dialog().open({ multiple: false, filters: [{ name: "Video", extensions: VIDEO }] }); if (!f) return; assign(i, await invoke("edit_probe", { path: Array.isArray(f) ? f[0] : f })); } catch (e) { toast(e, true); }
  }
  function pickLib(i) {
    const vids = (window.__lib ? window.__lib() : []).filter((x) => VIDEO.includes(x.ext) && x.duration > 0.5);
    const v = document.createElement("div"); v.className = "veil"; document.body.appendChild(v);
    v.innerHTML = `<div class="modal"><div class="ap-head"><div><h3>Choose a recording</h3></div><button class="close-x flat" id="lx">${I("x")}</button></div><div class="apps" style="padding:4px 14px 16px;max-height:55vh;overflow:auto">${vids.length ? vids.map((x, k) => `<button class="app" data-k="${k}"><div class="av">${I("film")}</div><div class="t"><b>${esc(x.name)}</b><span>${fmt(x.duration)}</span></div></button>`).join("") : `<div class="empty">No recordings yet</div>`}</div></div>`;
    const c = () => v.remove(); $("#lx", v).onclick = c; v.addEventListener("mousedown", (e) => { if (e.target === v) c(); });
    $$(".app", v).forEach((b) => (b.onclick = () => { assign(i, vids[+b.dataset.k]); c(); }));
  }

  // record one scene: a small note stays on top, and the next saved recording fills the slot
  function startRecording(i) {
    st.waiting = i; showWait();
    window.__goPage && window.__goPage("record");
  }
  function showWait() {
    hideWait(); if (st.waiting == null) return;
    const s = st.scenes[st.waiting], bar = document.createElement("div"); bar.id = "st-wait";
    bar.innerHTML = `<b>Scene ${st.waiting + 1} of ${st.scenes.length}: ${esc(s.label)}</b><span>${esc(s.hint)}. Press the record button or F9 and stop when this scene is done.</span><button class="btn sm" id="st-cancel">Cancel</button>`;
    document.body.appendChild(bar); $("#st-cancel", bar).onclick = () => { st.waiting = null; hideWait(); window.__goPage && window.__goPage("studio"); };
  }
  const hideWait = () => { const b = $("#st-wait"); if (b) b.remove(); };
  // called when a recording is saved; true means Studio used it
  function onSaved(it) {
    if (st.waiting == null) return false;
    const i = st.waiting; st.waiting = null; hideWait(); assign(i, it); window.__goPage && window.__goPage("studio"); toast(`Scene ${i + 1} saved`); return true;
  }

  async function make() {
    const clips = st.scenes.filter((s) => s.kind === "clip");
    if (!clips.length || clips.some((s) => !s.path)) { toast("Every clip scene needs a video", true); return; }
    st.busy = true; st.result = ""; builder();
    const project = { title: st.vtitle || st.tpl.name, aspect: st.aspect, fit: st.fit, transition: { kind: st.trans === "none" ? "" : st.trans, dur: 0.5 }, brand: st.brand, scenes: st.scenes.map((s) => ({ kind: s.kind, text: s.text || "", sub: s.sub || "", dur: s.dur || 3, path: s.path || "", tidy: !!s.tidy, caption: s.caption || "", trim_start: s.ts || 0, trim_end: s.te || 0, speed: s.speed || 1, mute: !!s.mute, caption_pos: s.pos || "bottom", caption_dur: s.cdur || 0, size: s.size || 0, bg: s.bg || "", accent: s.accent || "" })) };
    try { st.result = await invoke("studio_render", { project }); toast("Your video is ready"); window.__refreshLib && window.__refreshLib(); }
    catch (e) { toast(e, true); }
    st.busy = false; builder();
  }

  window.studio = { render, onSaved };
})();
