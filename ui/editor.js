/* Recora editor: cut, trim, speed, transitions, text, highlight boxes and blur on a timeline. */
(() => {
  const { icon: I, $, $$, esc, invoke, fileUrl, fmt, toast, ICONS } = R;
  Object.assign(ICONS, {
    text: '<path d="M5 6.5V5h14v1.5M12 5v14M9 19h6"/>',
    box: '<rect x="4" y="6" width="16" height="12" rx="2.5"/>',
    blur: '<rect x="4" y="5" width="16" height="14" rx="2.5"/><path d="M8 10h8M8 14h5" opacity=".55"/>',
    undo: '<path d="M9 7L4.5 11.5 9 16M5 11.5h9a5 5 0 0 1 0 10h-2"/>',
  });
  const SPEEDS = [0.5, 1, 1.5, 2, 4];
  const TRANS = [["none", "None"], ["fade", "Fade"], ["dissolve", "Dissolve"], ["slide", "Slide"], ["wipe", "Wipe"], ["zoom", "Zoom"], ["circle", "Circle"]];
  const PRESETS = [["", "Original"], ["youtube", "YouTube"], ["small", "Small"], ["vertical", "Vertical"], ["gif", "GIF"]];
  const COLORS = ["#ffffff", "#ffcc00", "#ff3b30", "#34c759", "#0a84ff", "#000000"];
  let uid = 1;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const r2 = (v) => Math.round(v * 100) / 100;

  window.openEditor = function (it, onDone) {
    closeEditor();
    const dur = it.duration || 1;
    const st = { clips: [{ id: uid++, start: 0, end: dur, speed: 1 }], ov: [], sel: 0, selOv: null, trans: { kind: "none", dur: 0.5 }, preset: "", t: 0, playing: false, zoom: 1, strip: "", ar: 16 / 9, busy: false };
    const hist = [];
    const snap = () => JSON.stringify({ c: st.clips, o: st.ov, t: st.trans });
    const push = () => { hist.push(snap()); if (hist.length > 60) hist.shift(); };
    const undo = () => { const h = hist.pop(); if (!h) return; const d = JSON.parse(h); st.clips = d.c; st.ov = d.o; st.trans = d.t; st.sel = Math.min(st.sel, st.clips.length - 1); st.selOv = null; seek(st.t); draw(); };

    const len = (c) => (c.end - c.start) / c.speed;
    const total = () => st.clips.reduce((s, c) => s + len(c), 0);
    const starts = () => { let a = 0; return st.clips.map((c) => { const s = a; a += len(c); return s; }); };
    const where = (t) => { const s = starts(); let i = st.clips.length - 1; for (let k = 0; k < s.length; k++) if (t < s[k] + len(st.clips[k]) - 1e-6) { i = k; break; } return i; };

    const el = document.createElement("div");
    el.className = "ed"; el.id = "editor";
    el.innerHTML = `<div class="ed-top"><button class="btn sm" id="ed-x">${I("x")}Close</button><div class="ed-title" id="ed-title"></div><span class="grow"></span><div class="seg fit" id="ed-pre">${PRESETS.map(([k, n]) => `<button data-k="${k}">${n}</button>`).join("")}</div><button class="btn primary" id="ed-go">Export</button></div>
      <div class="ed-main">
        <div class="ed-stage"><div class="ed-frame" id="frame"><video id="ev" src="${fileUrl(it.path)}" preload="auto" playsinline></video><div class="ed-ovl" id="ovl"></div></div></div>
        <aside class="ed-side" id="side"></aside>
      </div>
      <div class="ed-bar"><button class="btn sm" id="b-play"></button><span class="ed-time" id="b-time">00:00 / 00:00</span><span class="grow"></span>
        <button class="btn sm" id="b-split" title="Split at the playhead (S)">${I("scissors")}Split</button><button class="btn sm" id="b-del" title="Delete the selected piece (Delete)">${I("trash")}Delete</button><button class="btn sm" id="b-undo" title="Undo (Ctrl+Z)">${I("undo")}Undo</button>
        <label class="ed-zoom">Zoom<input type="range" id="b-zoom" min="1" max="8" step="0.1" value="1"></label></div>
      <div class="ed-tl" id="tl"></div>`;
    document.body.appendChild(el);
    const vid = $("#ev", el);
    const tlEl = $("#tl", el), side = $("#side", el), ovl = $("#ovl", el), frame = $("#frame", el);
    invoke("edit_strip", { path: it.path, n: 24 }).then((p) => { if (p) { st.strip = fileUrl(p); draw(); } }).catch(() => {});

    // ---------------------------------------------------------------- playback
    let cur = 0, raf = 0;
    function seek(t) {
      st.t = clamp(t, 0, total());
      cur = where(st.t); const c = st.clips[cur], s = starts()[cur];
      vid.currentTime = c.start + (st.t - s) * c.speed; vid.playbackRate = c.speed;
      paint();
    }
    function tick() {
      if (!st.playing) return;
      const c = st.clips[cur];
      if (vid.currentTime >= c.end - 0.03) {
        if (cur + 1 < st.clips.length) { cur++; vid.currentTime = st.clips[cur].start; vid.playbackRate = st.clips[cur].speed; }
        else { setPlay(false); st.t = total(); paint(); return; }
      }
      const c2 = st.clips[cur];
      st.t = starts()[cur] + Math.max(0, vid.currentTime - c2.start) / c2.speed;
      paint(); raf = requestAnimationFrame(tick);
    }
    function setPlay(on) {
      st.playing = on; $("#b-play", el).innerHTML = on ? I("pause") + "Pause" : I("play") + "Play";
      cancelAnimationFrame(raf);
      if (on) { if (st.t >= total() - 0.05) seek(0); vid.playbackRate = st.clips[cur].speed; vid.play().catch(() => {}); raf = requestAnimationFrame(tick); } else vid.pause();
    }

    // ---------------------------------------------------------------- stage overlays
    function paintOverlays() {
      const H = frame.clientHeight || 360;
      ovl.innerHTML = st.ov.filter((o) => st.t >= o.start && st.t <= o.end + 0.001).map((o) => {
        const sel = o.id === st.selOv ? " sel" : "";
        if (o.kind === "text") return `<div class="ov t${sel}" data-id="${o.id}" dir="auto" style="left:${o.x * 100}%;top:${o.y * 100}%;font-size:${(o.size || 0.06) * H}px;color:${o.color}">${esc(o.text)}</div>`;
        return `<div class="ov ${o.kind}${sel}" data-id="${o.id}" style="left:${o.x * 100}%;top:${o.y * 100}%;width:${o.w * 100}%;height:${o.h * 100}%;${o.kind === "box" ? `border-color:${o.color}` : ""}"><i class="rz"></i></div>`;
      }).join("");
      $$(".ov", ovl).forEach((n) => (n.onpointerdown = (e) => dragOverlay(e, n)));
    }
    function dragOverlay(e, n) {
      e.preventDefault(); e.stopPropagation();
      const o = st.ov.find((x) => x.id == n.dataset.id); if (!o) return;
      if (st.selOv !== o.id) { st.selOv = o.id; st.sel = -1; drawSide(); drawTl(); }
      push();
      const fr = frame.getBoundingClientRect(), resize = e.target.classList.contains("rz");
      const sx = e.clientX, sy = e.clientY, ox = o.x, oy = o.y, ow = o.w, oh = o.h;
      n.setPointerCapture(e.pointerId);
      const mv = (ev) => {
        const dx = (ev.clientX - sx) / fr.width, dy = (ev.clientY - sy) / fr.height;
        if (resize) { o.w = clamp(ow + dx, 0.03, 1 - o.x); o.h = clamp(oh + dy, 0.03, 1 - o.y); }
        else { o.x = clamp(ox + dx, 0, 0.98); o.y = clamp(oy + dy, 0, 0.98); }
        paintOverlays();
      };
      const up = () => { n.removeEventListener("pointermove", mv); n.removeEventListener("pointerup", up); };
      n.addEventListener("pointermove", mv); n.addEventListener("pointerup", up);
    }

    // ---------------------------------------------------------------- timeline
    const pps = () => Math.max(20, ((tlEl.clientWidth - 24) / Math.max(total(), 1)) * st.zoom);
    function drawTl() {
      const k = pps(), T = total(), W = T * k, S = starts();
      let ticks = ""; const step = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600].find((x) => x * k >= 70) || 600;
      for (let t = 0; t <= T; t += step) ticks += `<span class="tk" style="left:${t * k}px">${fmt(t)}</span>`;
      const clips = st.clips.map((c, i) => {
        const w = len(c) * k, full = (dur / (c.end - c.start)) * w;
        const bg = st.strip ? `background-image:url('${st.strip}');background-size:${full}px 100%;background-position:${-(c.start / (c.end - c.start)) * w}px 0;` : "";
        const mk = it.markers.filter((m) => m >= c.start && m <= c.end).map((m) => `<i class="mkr" style="left:${((m - c.start) / c.speed) * k}px"></i>`).join("");
        return `<div class="cl${i === st.sel ? " sel" : ""}" data-i="${i}" style="left:${S[i] * k}px;width:${w}px;${bg}"><i class="eh l"></i><i class="eh r"></i>${c.speed !== 1 ? `<b class="sp">${c.speed}×</b>` : ""}${mk}</div>`;
      }).join("");
      const joins = st.trans.kind !== "none" ? S.slice(1).map((s) => `<i class="jn" style="left:${s * k}px" title="${st.trans.kind}"></i>`).join("") : "";
      const bars = st.ov.map((o, i) => `<div class="ob ${o.kind}${o.id === st.selOv ? " sel" : ""}" data-id="${o.id}" style="left:${o.start * k}px;width:${Math.max(8, (o.end - o.start) * k)}px;top:${(i % 3) * 22}px"><i class="eh l"></i><span>${o.kind === "text" ? esc(o.text || "Text") : o.kind === "box" ? "Highlight" : "Blur"}</span><i class="eh r"></i></div>`).join("");
      tlEl.innerHTML = `<div class="tl-in" style="width:${W + 24}px"><div class="tl-ruler" id="ruler">${ticks}</div><div class="tl-clips" id="tcl">${clips}${joins}</div><div class="tl-ovs" id="tov">${bars || `<em>Add text, a highlight or a blur from the side panel</em>`}</div><i class="ph2" id="ph2"></i></div>`;
      tlEl.querySelector(".tl-in").style.setProperty("--k", k);
      wireTl(k); paintHead();
    }
    function paintHead() { const k = pps(); const p = $("#ph2", el); if (p) p.style.left = st.t * k + "px"; }
    function timeAt(ev) { const r = $("#ruler", el).getBoundingClientRect(); return clamp((ev.clientX - r.left) / pps(), 0, total()); }
    function wireTl() {
      const scrub = (ev) => { setPlay(false); seek(timeAt(ev)); };
      const drag = (fn) => (e) => { e.preventDefault(); const mv = (ev) => fn(ev); const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); }; window.addEventListener("pointermove", mv); window.addEventListener("pointerup", up); };
      $("#ruler", el).onpointerdown = (e) => { scrub(e); drag(scrub)(e); };
      $$(".cl", el).forEach((n) => {
        const i = +n.dataset.i;
        n.onpointerdown = (e) => {
          if (e.target.classList.contains("eh")) {
            e.stopPropagation(); push(); const c = st.clips[i], left = e.target.classList.contains("l"), x0 = e.clientX, s0 = c.start, e0 = c.end, k = pps();
            drag((ev) => { const d = ((ev.clientX - x0) / k) * c.speed; if (left) c.start = clamp(s0 + d, 0, c.end - 0.2); else c.end = clamp(e0 + d, c.start + 0.2, dur); st.sel = i; seek(Math.min(st.t, total())); drawTl(); })(e); return;
          }
          st.sel = i; st.selOv = null; setPlay(false); seek(timeAt(e)); drawSide(); drawTl(); drag(scrub)(e);
        };
      });
      $$(".ob", el).forEach((n) => {
        n.onpointerdown = (e) => {
          const o = st.ov.find((x) => x.id == n.dataset.id); if (!o) return; e.stopPropagation(); st.selOv = o.id; st.sel = -1; push();
          const k = pps(), x0 = e.clientX, s0 = o.start, e0 = o.end, mode = e.target.classList.contains("l") ? "l" : e.target.classList.contains("r") ? "r" : "m";
          seek(o.start + 0.01); drawSide();
          drag((ev) => { const d = (ev.clientX - x0) / k, T = total();
            if (mode === "m") { const L = e0 - s0; o.start = clamp(s0 + d, 0, T - L); o.end = o.start + L; } else if (mode === "l") o.start = clamp(s0 + d, 0, o.end - 0.2); else o.end = clamp(e0 + d, o.start + 0.2, T);
            drawTl(); paintOverlays(); })(e);
          drawTl();
        };
      });
    }

    // ---------------------------------------------------------------- side panel
    function drawSide() {
      const o = st.ov.find((x) => x.id === st.selOv), c = st.clips[st.sel];
      const swatches = (cur) => COLORS.map((x) => `<button class="sw${x === cur ? " on" : ""}" data-c="${x}" style="background:${x}"></button>`).join("");
      side.innerHTML = `
        <div class="label sm">Add to the video</div>
        <div class="add"><button class="btn" data-add="text">${I("text")}Text</button><button class="btn" data-add="box">${I("box")}Highlight</button><button class="btn" data-add="blur">${I("blur")}Blur</button></div>
        ${o ? `<div class="label sm">${o.kind === "text" ? "Text" : o.kind === "box" ? "Highlight" : "Blur"}</div><div class="card">
          ${o.kind === "text" ? `<textarea id="o-text" dir="auto" rows="2" placeholder="Type here">${esc(o.text)}</textarea><div class="row2"><span>Size</span><input type="range" id="o-size" min="0.03" max="0.14" step="0.005" value="${o.size || 0.06}"></div>` : ""}
          ${o.kind !== "blur" ? `<div class="row2"><span>Colour</span><div class="sws">${swatches(o.color)}</div></div>` : `<p class="muted">Drag it on the picture, and pull the corner to resize. The area is pixelated so the text under it cannot be read.</p>`}
          <div class="row2"><span>Shown</span><span class="muted" id="o-when">${fmt(o.start)} – ${fmt(o.end)}</span></div><div class="row2"><button class="btn sm" id="o-here">Start here</button><button class="btn sm" id="o-del">${I("trash")}Remove</button></div></div>` : ""}
        ${c ? `<div class="label sm">Selected piece</div><div class="card"><div class="row2"><span>Speed</span><div class="seg fit" id="sp">${SPEEDS.map((s) => `<button data-s="${s}" class="${c.speed === s ? "on" : ""}">${s}×</button>`).join("")}</div></div><div class="row2"><span>Length</span><span class="muted">${fmt(len(c))}</span></div></div>` : ""}
        <div class="label sm">Transition between pieces</div>
        <div class="card"><div class="tgrid" id="tr">${TRANS.map(([k, n]) => `<button data-k="${k}" class="${st.trans.kind === k ? "on" : ""}">${n}</button>`).join("")}</div>
          ${st.trans.kind !== "none" ? `<div class="row2"><span>Length</span><input type="range" id="tr-d" min="0.2" max="1.5" step="0.1" value="${st.trans.dur}"><span class="muted">${st.trans.dur}s</span></div>${st.clips.length < 2 ? `<p class="muted">Split the video to see it work.</p>` : ""}` : ""}</div>`;
      $$("[data-add]", side).forEach((b) => (b.onclick = () => addOverlay(b.dataset.add)));
      const t = $("#o-text", side); if (t) { t.oninput = () => { o.text = t.value; paintOverlays(); drawTl(); }; t.onfocus = push; t.onkeydown = (e) => e.stopPropagation(); }
      const sz = $("#o-size", side); if (sz) { sz.onpointerdown = push; sz.oninput = () => { o.size = +sz.value; paintOverlays(); }; }
      $$(".sw", side).forEach((b) => (b.onclick = () => { push(); o.color = b.dataset.c; paintOverlays(); drawSide(); }));
      const dl = $("#o-del", side); if (dl) dl.onclick = () => { push(); st.ov = st.ov.filter((x) => x !== o); st.selOv = null; draw(); };
      const oh = $("#o-here", side); if (oh) oh.onclick = () => { push(); const L = o.end - o.start; o.start = st.t; o.end = Math.min(total(), o.start + L); draw(); };
      $$("#sp button", side).forEach((b) => (b.onclick = () => { push(); c.speed = +b.dataset.s; seek(Math.min(st.t, total())); draw(); }));
      $$("#tr button", side).forEach((b) => (b.onclick = () => { push(); st.trans.kind = b.dataset.k; draw(); }));
      const td = $("#tr-d", side); if (td) { td.onpointerdown = push; td.oninput = () => { st.trans.dur = +(+td.value).toFixed(1); td.nextElementSibling.textContent = st.trans.dur + "s"; }; }
    }
    function addOverlay(kind) {
      push(); const T = total(), s = clamp(st.t, 0, Math.max(0, T - 0.5));
      const base = { id: uid++, kind, start: r2(s), end: r2(Math.min(T, s + 3)), x: 0.1, y: 0.1, w: 0.3, h: 0.2, text: "", size: 0.06, color: kind === "box" ? "#ffcc00" : "#ffffff" };
      if (kind === "text") { base.text = "Your text"; base.y = 0.78; base.x = 0.08; }
      if (kind === "blur") { base.x = 0.35; base.y = 0.35; }
      st.ov.push(base); st.selOv = base.id; st.sel = -1; draw();
      const t = $("#o-text", side); if (t) { t.focus(); t.select(); }
    }

    // ---------------------------------------------------------------- actions
    function split() {
      const i = where(st.t), c = st.clips[i], s = starts()[i], src = c.start + (st.t - s) * c.speed;
      if (src - c.start < 0.2 || c.end - src < 0.2) { toast("Move the playhead inside a piece first"); return; }
      push(); st.clips.splice(i, 1, { ...c, id: uid++, end: src }, { ...c, id: uid++, start: src }); st.sel = i + 1; draw();
    }
    function del() {
      if (st.selOv) { push(); st.ov = st.ov.filter((o) => o.id !== st.selOv); st.selOv = null; draw(); return; }
      if (st.sel < 0) return;
      if (st.clips.length < 2) { toast("The video needs at least one piece"); return; }
      push(); st.clips.splice(st.sel, 1); st.sel = Math.min(st.sel, st.clips.length - 1);
      st.ov.forEach((o) => { o.end = Math.min(o.end, total()); o.start = Math.min(o.start, o.end - 0.2); });
      seek(Math.min(st.t, total())); draw();
    }
    async function go() {
      if (st.busy) return;
      const L = st.clips.map(len), minL = Math.min(...L), td = st.trans.kind !== "none" && st.clips.length > 1 ? Math.min(clamp(st.trans.dur, 0.1, 2), minL / 2) : 0, S = starts();
      // the picture overlaps while a transition plays, so overlay times move earlier by the overlap before them
      const shift = (t) => t - td * where(t - 1e-6 < 0 ? 0 : t - 1e-6);
      const project = {
        input: it.path, preset: st.preset, transition: { kind: st.trans.kind === "none" ? "" : st.trans.kind, dur: st.trans.dur },
        clips: st.clips.map((c) => ({ start: r2(c.start), end: r2(c.end), speed: c.speed })),
        overlays: st.ov.map((o) => ({ kind: o.kind, start: Math.max(0, shift(o.start)), end: Math.max(0.2, shift(o.end)), x: o.x, y: o.y, w: o.w, h: o.h, text: o.text, size: o.size, color: o.color })),
      };
      void S;
      st.busy = true; setPlay(false); const b = $("#ed-go", el); b.disabled = true; b.innerHTML = `<span class="spin"></span>Exporting…`;
      try { const out = await invoke("edit_export", { project }); toast("Saved: " + String(out).split(/[\\/]/).pop()); if (onDone) onDone(out); }
      catch (e) { toast(e, true); }
      st.busy = false; b.disabled = false; b.textContent = "Export";
    }

    // ---------------------------------------------------------------- wiring
    function paint() { $("#b-time", el).textContent = `${fmt(st.t)} / ${fmt(total())}`; paintHead(); paintOverlays(); }
    function draw() { $("#ed-title", el).textContent = it.name; $$("#ed-pre button", el).forEach((b) => b.classList.toggle("on", b.dataset.k === st.preset)); $("#b-play", el).innerHTML = I(st.playing ? "pause" : "play") + (st.playing ? "Pause" : "Play"); $("#b-undo", el).disabled = !hist.length; drawSide(); drawTl(); paint(); }
    $$("#ed-pre button", el).forEach((b) => (b.onclick = () => { st.preset = b.dataset.k; draw(); }));
    $("#ed-x", el).onclick = closeEditor; $("#ed-go", el).onclick = go;
    $("#b-play", el).onclick = () => setPlay(!st.playing);
    $("#b-split", el).onclick = split; $("#b-del", el).onclick = del; $("#b-undo", el).onclick = undo;
    $("#b-zoom", el).oninput = (e) => { st.zoom = +e.target.value; drawTl(); };
    vid.addEventListener("loadedmetadata", () => { if (vid.videoWidth) { st.ar = vid.videoWidth / vid.videoHeight; frame.style.aspectRatio = st.ar; } paint(); });
    vid.addEventListener("click", () => setPlay(!st.playing));
    const key = (e) => {
      if (!$("#editor")) return document.removeEventListener("keydown", key, true);
      if (/INPUT|TEXTAREA/.test(document.activeElement.tagName)) return;
      if (e.code === "Space") { e.preventDefault(); setPlay(!st.playing); }
      else if (e.key === "s" || e.key === "S") split();
      else if (e.key === "Delete" || e.key === "Backspace") del();
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); }
      else if (e.key === "Escape") closeEditor();
      else if (e.key === "ArrowLeft") { setPlay(false); seek(st.t - (e.shiftKey ? 5 : 1 / 30)); }
      else if (e.key === "ArrowRight") { setPlay(false); seek(st.t + (e.shiftKey ? 5 : 1 / 30)); }
      e.stopPropagation();
    };
    document.addEventListener("keydown", key, true);
    new ResizeObserver(() => { drawTl(); paintOverlays(); }).observe(tlEl);
    draw(); seek(0);
    window.__editor = st;
  };
  function closeEditor() { const e = $("#editor"); if (e) { const v = $("video", e); if (v) v.pause(); e.remove(); } }
  window.closeEditor = closeEditor;
})();
