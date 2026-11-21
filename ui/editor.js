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
  const S_AI = () => !!(window.__ai && window.__ai());
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const r2 = (v) => Math.round(v * 100) / 100;

  window.openEditor = function (it, onDone, opts = {}) {
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
    el.innerHTML = `<div class="ed-mb" id="mb">${["File", "Edit", "Clip", "Add", "Tools", "View", "Help"].map((n) => `<button data-m="${n}">${n}</button>`).join("")}</div><div class="ed-top"><button class="btn sm" id="ed-x">${I("x")}Close</button><div class="ed-title" id="ed-title"></div><span class="grow"></span><div class="seg fit" id="ed-pre">${PRESETS.map(([k, n]) => `<button data-k="${k}">${n}</button>`).join("")}</div><button class="btn primary" id="ed-go">Export</button></div>
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
    const HANDLES = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
    const LINES = [0, 1 / 3, 0.5, 2 / 3, 1];
    const BLUR_PX = { 1: 5, 2: 9, 3: 16 };
    function ovNode(id) { return $$(".ov", ovl).find((n) => n.dataset.id == id); }
    // normalised size of an overlay (text has no fixed box, so it is measured on screen)
    function ovSize(o) {
      if (o.kind !== "text") return { w: o.w, h: o.h };
      const n = ovNode(o.id), fr = frame.getBoundingClientRect();
      return n && fr.width ? { w: n.offsetWidth / fr.width, h: n.offsetHeight / fr.height } : { w: 0.3, h: (o.size || 0.06) * 1.5 };
    }
    function styleOv(n, o, H) {
      const on = st.t >= o.start && st.t <= o.end + 0.001;
      n.className = `ov ${o.kind === "text" ? "t" : o.kind}${o.id === st.selOv ? " sel" : ""}${on ? "" : " off"}`;
      n.style.left = o.x * 100 + "%"; n.style.top = o.y * 100 + "%";
      if (o.kind === "text") {
        n.style.fontSize = (o.size || 0.06) * H + "px"; n.style.color = o.color;
        n.style.background = o.bg === false ? "transparent" : "";
        n.style.textShadow = o.bg === false ? "0 1px 6px rgba(0,0,0,.7)" : "";
        const tx = n.firstChild; if (tx.textContent !== o.text) tx.textContent = o.text;
      } else {
        n.style.width = o.w * 100 + "%"; n.style.height = o.h * 100 + "%";
        if (o.kind === "box") n.style.borderColor = o.color;
        else { const px = (BLUR_PX[o.level] || 9) + "px"; n.style.backdropFilter = `blur(${px})`; n.style.webkitBackdropFilter = `blur(${px})`; }
      }
    }
    function paintOverlays() {
      const H = frame.clientHeight || 360;
      const want = st.ov.filter((o) => o.id === st.selOv || (st.t >= o.start && st.t <= o.end + 0.001));
      const have = new Map($$(".ov", ovl).map((n) => [n.dataset.id, n]));
      want.forEach((o) => {
        let n = have.get(String(o.id));
        if (!n) {
          n = document.createElement("div"); n.dataset.id = o.id; if (o.kind === "text") n.dir = "auto";
          n.innerHTML = (o.kind === "text" ? `<span class="tx"></span>` : "") + HANDLES.filter((h) => o.kind !== "text" || h.length === 2).map((h) => `<i class="hd ${h}" data-h="${h}"></i>`).join("");
          n.onpointerdown = (e) => dragOverlay(e, n);
          ovl.appendChild(n);
        }
        have.delete(String(o.id)); styleOv(n, o, H);
      });
      have.forEach((n) => n.remove());
    }
    function guides(x, y) {
      let gx = $(".gd.v", ovl), gy = $(".gd.h", ovl);
      if (!gx) { gx = document.createElement("div"); gx.className = "gd v"; gy = document.createElement("div"); gy.className = "gd h"; ovl.append(gx, gy); }
      gx.style.display = x == null ? "none" : "block"; gy.style.display = y == null ? "none" : "block";
      if (x != null) gx.style.left = x * 100 + "%"; if (y != null) gy.style.top = y * 100 + "%";
    }
    // pull a value onto the nearest guide line, within a few pixels
    function snap1(v, cands, tol) { let best = null, d = tol; for (const c of cands) { const k = Math.abs(c - v); if (k < d) { d = k; best = c; } } return best; }
    function dragOverlay(e, n) {
      if (e.button) return;
      e.preventDefault(); e.stopPropagation();
      const o = st.ov.find((x) => x.id == n.dataset.id); if (!o) return;
      if (st.selOv !== o.id) { st.selOv = o.id; st.sel = -1; drawSide(); drawTl(); paintOverlays(); }
      push();
      const fr = frame.getBoundingClientRect(), h = e.target.dataset.h || "";
      const sx = e.clientX, sy = e.clientY, o0 = { x: o.x, y: o.y, w: o.w, h: o.h, size: o.size }, sz = ovSize(o);
      const tolX = 7 / fr.width, tolY = 7 / fr.height;
      const mv = (ev) => {
        const dx = (ev.clientX - sx) / fr.width, dy = (ev.clientY - sy) / fr.height, free = ev.altKey;
        let gx = null, gy = null;
        if (!h) {                                              // move
          let x = o0.x + dx, y = o0.y + dy;
          if (!free) {
            const sxn = [x, x + sz.w / 2, x + sz.w].map((v, i) => [snap1(v, LINES, tolX), i]).filter((a) => a[0] != null)[0];
            if (sxn) { x = sxn[0] - sxn[1] * sz.w / 2; gx = sxn[0]; }
            const syn = [y, y + sz.h / 2, y + sz.h].map((v, i) => [snap1(v, LINES, tolY), i]).filter((a) => a[0] != null)[0];
            if (syn) { y = syn[0] - syn[1] * sz.h / 2; gy = syn[0]; }
          }
          o.x = clamp(x, 0, Math.max(0, 1 - Math.min(sz.w, 0.98))); o.y = clamp(y, 0, Math.max(0, 1 - Math.min(sz.h, 0.98)));
        } else if (o.kind === "text") {                        // corners scale the lettering
          const east = h.includes("e"), south = h.includes("s");
          const f = clamp(1 + ((east ? dx : -dx) / Math.max(sz.w, 0.05) + (south ? dy : -dy) / Math.max(sz.h, 0.05)) / 2, 0.3, 4);
          o.size = clamp(o0.size * f, 0.02, 0.2);
          const g = o.size / o0.size; if (!east) o.x = clamp(o0.x + sz.w * (1 - g), 0, 1); if (!south) o.y = clamp(o0.y + sz.h * (1 - g), 0, 1);
        } else {                                               // box and blur: any edge or corner
          let x1 = o0.x, y1 = o0.y, x2 = o0.x + o0.w, y2 = o0.y + o0.h;
          if (h.includes("e")) x2 += dx; if (h.includes("w")) x1 += dx; if (h.includes("s")) y2 += dy; if (h.includes("n")) y1 += dy;
          if (!free) {
            const mvx = h.includes("e") ? "x2" : h.includes("w") ? "x1" : "", mvy = h.includes("s") ? "y2" : h.includes("n") ? "y1" : "";
            if (mvx) { const v = mvx === "x2" ? x2 : x1, c = snap1(v, LINES, tolX); if (c != null) { if (mvx === "x2") x2 = c; else x1 = c; gx = c; } }
            if (mvy) { const v = mvy === "y2" ? y2 : y1, c = snap1(v, LINES, tolY); if (c != null) { if (mvy === "y2") y2 = c; else y1 = c; gy = c; } }
          }
          x1 = clamp(x1, 0, 1); x2 = clamp(x2, 0, 1); y1 = clamp(y1, 0, 1); y2 = clamp(y2, 0, 1);
          if (x2 - x1 < 0.03) { if (h.includes("w")) x1 = x2 - 0.03; else x2 = x1 + 0.03; }
          if (y2 - y1 < 0.03) { if (h.includes("n")) y1 = y2 - 0.03; else y2 = y1 + 0.03; }
          o.x = clamp(x1, 0, 0.97); o.y = clamp(y1, 0, 0.97); o.w = Math.min(x2, 1) - o.x; o.h = Math.min(y2, 1) - o.y;
        }
        guides(gx, gy); styleOv(n, o, frame.clientHeight || 360);
      };
      const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); window.removeEventListener("pointercancel", up); guides(null, null); drawSide(); };
      window.addEventListener("pointermove", mv); window.addEventListener("pointerup", up); window.addEventListener("pointercancel", up);
    }
    function nudge(dx, dy) {
      const o = st.ov.find((x) => x.id === st.selOv); if (!o) return false;
      const z = ovSize(o); o.x = clamp(o.x + dx, 0, Math.max(0, 1 - z.w)); o.y = clamp(o.y + dy, 0, Math.max(0, 1 - z.h)); paintOverlays(); return true;
    }
    function align(o, how) {
      const z = ovSize(o); push();
      if (how === "l") o.x = 0.03; else if (how === "c") o.x = (1 - z.w) / 2; else if (how === "r") o.x = 1 - z.w - 0.03;
      else if (how === "t") o.y = 0.04; else if (how === "m") o.y = (1 - z.h) / 2; else if (how === "b") o.y = 1 - z.h - 0.04;
      o.x = clamp(o.x, 0, 1); o.y = clamp(o.y, 0, 1); paintOverlays();
    }
    function dupOverlay() {
      const o = st.ov.find((x) => x.id === st.selOv); if (!o) return false;
      push(); const n2 = { ...o, id: uid++, x: Math.min(o.x + 0.03, 0.9), y: Math.min(o.y + 0.04, 0.9) }; st.ov.push(n2); st.selOv = n2.id; draw(); return true;
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
            const srcT = (() => { const w = where(st.t); return st.clips[w].start + (st.t - starts()[w]) * st.clips[w].speed; })(); drag((ev) => { const d = ((ev.clientX - x0) / k) * c.speed, tol = (8 / k) * c.speed, sn = (v) => (!ev.altKey && srcT != null && Math.abs(v - srcT) < tol ? srcT : v); if (left) c.start = clamp(sn(s0 + d), 0, c.end - 0.2); else c.end = clamp(sn(e0 + d), c.start + 0.2, dur); st.sel = i; seek(Math.min(st.t, total())); drawTl(); })(e); return;
          }
          st.sel = i; st.selOv = null; setPlay(false); seek(timeAt(e)); drawSide(); drawTl(); drag(scrub)(e);
        };
      });
      $$(".ob", el).forEach((n) => {
        n.onpointerdown = (e) => {
          const o = st.ov.find((x) => x.id == n.dataset.id); if (!o) return; e.stopPropagation(); st.selOv = o.id; st.sel = -1; push();
          const k = pps(), x0 = e.clientX, s0 = o.start, e0 = o.end, mode = e.target.classList.contains("l") ? "l" : e.target.classList.contains("r") ? "r" : "m";
          seek(o.start + 0.01); drawSide();
          const marks = [st.t, 0, total(), ...starts(), ...st.clips.map((c, i) => starts()[i] + len(c)), ...st.ov.filter((x) => x !== o).flatMap((x) => [x.start, x.end])], tol = 8 / k;
          const sn = (v) => { const c = snap1(v, marks, tol); return c == null ? v : c; };
          drag((ev) => { const d = (ev.clientX - x0) / k, T = total(), free = ev.altKey;
            if (mode === "m") { const L = e0 - s0; let a = s0 + d; if (!free) { const A = snap1(a, marks, tol), B = snap1(a + L, marks, tol); if (A != null) a = A; else if (B != null) a = B - L; } o.start = clamp(a, 0, T - L); o.end = o.start + L; }
            else if (mode === "l") o.start = clamp(free ? s0 + d : sn(s0 + d), 0, o.end - 0.2); else o.end = clamp(free ? e0 + d : sn(e0 + d), o.start + 0.2, T);
            o.start = r2(o.start); o.end = r2(o.end);
            drawTl(); paintOverlays(); })(e);
          drawTl();
        };
      });
    }

    // ---------------------------------------------------------------- smart tools
    // every helper works on the pieces in source time and keeps the order
    const rebuild = (fn) => { const out = []; for (const c of st.clips) for (const p of fn(c)) if (p.end - p.start >= 0.2) out.push({ ...p, id: uid++ }); return out; };
    const subtract = (c, ranges) => {
      let parts = [{ ...c }];
      for (const [a, b] of ranges) {
        const next = [];
        for (const p of parts) {
          if (b <= p.start || a >= p.end) { next.push(p); continue; }
          if (a > p.start) next.push({ ...p, end: a });
          if (b < p.end) next.push({ ...p, start: b });
        }
        parts = next;
      }
      return parts;
    };
    const cutRanges = (ranges) => { st.clips = rebuild((c) => subtract(c, ranges)); };
    const speedRanges = (ranges, sp) => {
      st.clips = rebuild((c) => {
        if (c.speed !== 1) return [c];
        const cuts = [c.start, c.end];
        for (const [a, b] of ranges) { if (a > c.start && a < c.end) cuts.push(a); if (b > c.start && b < c.end) cuts.push(b); }
        cuts.sort((x, y) => x - y);
        const out = [];
        for (let i = 0; i < cuts.length - 1; i++) {
          const m = (cuts[i] + cuts[i + 1]) / 2, idle = ranges.some(([a, b]) => m >= a && m <= b);
          out.push({ ...c, start: cuts[i], end: cuts[i + 1], speed: idle ? sp : 1 });
        }
        return out;
      });
    };
    const splitTimes = (times) => { st.clips = rebuild((c) => { const ts = times.filter((t) => t > c.start + 0.3 && t < c.end - 0.3).sort((x, y) => x - y); const cuts = [c.start, ...ts, c.end]; return cuts.slice(0, -1).map((s, i) => ({ ...c, start: s, end: cuts[i + 1] })); }); };
    async function smart(kind, btn) {
      const old = btn.innerHTML; btn.disabled = true; btn.innerHTML = `<span class="spin"></span>Looking…`;
      try {
        if (!st.an) st.an = await invoke("edit_analyze", { path: it.path });
        const before = total(); push();
        if (kind === "silence") {
          const r = st.an.silences.filter(([a, b]) => b - a >= 0.8).map(([a, b]) => [a + 0.15, b - 0.15]).filter(([a, b]) => b > a);
          if (!r.length) { hist.pop(); toast("No silent parts found"); } else { cutRanges(r); toast(`Removed ${r.length} silent ${r.length === 1 ? "part" : "parts"}, ${fmt(before - total())} shorter`); }
        } else if (kind === "idle") {
          const r = st.an.idle.filter(([a, b]) => b - a >= 2).map(([a, b]) => [a + 0.4, b - 0.4]).filter(([a, b]) => b > a);
          if (!r.length) { hist.pop(); toast("The screen was never still for long"); } else { speedRanges(r, 4); toast(`Sped up ${r.length} still ${r.length === 1 ? "moment" : "moments"} to 4×, ${fmt(before - total())} shorter`); }
        } else {
          if (!st.an.scenes.length) { hist.pop(); toast("No clear scene changes found"); } else { splitTimes(st.an.scenes); toast(`Split into ${st.clips.length} scenes`); }
        }
        st.sel = 0; seek(Math.min(st.t, total())); draw();
      } catch (err) { hist.pop(); toast(err, true); }
      btn.disabled = false; btn.innerHTML = old;
    }

    // ---------------------------------------------------------------- by time, quick clean-up, AI
    const AR = "٠١٢٣٤٥٦٧٨٩";
    const normDigits = (s) => s.replace(/[٠-٩]/g, (d) => AR.indexOf(d)).replace(/[،٫]/g, ".").replace(/[–—−]/g, "-");
    // "1:30", "90", "1:02:03" or "1.5" to seconds
    const toSec = (s) => { const p = s.trim().split(":").map(Number); if (!p.length || p.some((n) => !isFinite(n) || n < 0)) return NaN; return p.reduce((a, n) => a * 60 + n, 0); };
    // lines such as "0:10-0:25", "1:30 to 2:00", "من 10 الى 25" -> [[10,25], ...]
    function parseRanges(text) {
      const out = [];
      for (const line of normDigits(text).split(/\n|;/)) {
        const m = line.match(/(\d+(?::\d+){0,2}(?:\.\d+)?)\s*(?:-|to|until|الى|إلى|لـ|حتى)\s*(\d+(?::\d+){0,2}(?:\.\d+)?)/i);
        if (!m) { if (line.trim()) out.push(null); continue; }
        const a = toSec(m[1]), b = toSec(m[2]);
        if (isNaN(a) || isNaN(b) || b <= a) out.push(null); else out.push([a, b]);
      }
      return out;
    }
    // a range on the edited timeline as ranges of the source video
    function tlToSrc(a, b) {
      const S = starts(), out = [];
      st.clips.forEach((c, i) => { const s = S[i], e2 = s + len(c), lo = Math.max(a, s), hi = Math.min(b, e2); if (hi - lo > 0.01) out.push([c.start + (lo - s) * c.speed, c.start + (hi - s) * c.speed]); });
      return out;
    }
    const srcToTl = (t) => { const S = starts(); for (let i = 0; i < st.clips.length; i++) { const c = st.clips[i]; if (t >= c.start - 1e-6 && t <= c.end + 1e-6) return S[i] + (t - c.start) / c.speed; } return null; };
    function byTime(mode, text) {
      const rs = parseRanges(text);
      if (!rs.length) { toast("Write at least one range, for example 0:10-0:25"); return; }
      if (rs.some((r) => !r)) { toast("One line is not clear. Use the form 0:10-0:25", true); return; }
      const T = total(), before = T; push();
      let src = [];
      if (mode === "cut") rs.forEach(([a, b]) => src.push(...tlToSrc(a, Math.min(b, T))));
      else {
        const keep = rs.map(([a, b]) => [a, Math.min(b, T)]).sort((x, y) => x[0] - y[0]); let pos = 0;
        keep.forEach(([a, b]) => { if (a > pos) src.push(...tlToSrc(pos, a)); pos = Math.max(pos, b); });
        if (pos < T) src.push(...tlToSrc(pos, T));
      }
      if (!src.length) { hist.pop(); toast("Those times are outside the video"); return; }
      cutRanges(src); st.sel = 0; st.selOv = null; st.ov.forEach((o) => { o.end = Math.min(o.end, total()); o.start = Math.min(o.start, Math.max(0, o.end - 0.2)); });
      seek(Math.min(st.t, total())); draw(); toast(`${mode === "cut" ? "Removed" : "Kept"} ${rs.length} ${rs.length === 1 ? "range" : "ranges"}, now ${fmt(total())} (was ${fmt(before)})`);
    }
    async function cleanup(btn) {
      const old = btn.innerHTML; btn.disabled = true; btn.innerHTML = `<span class="spin"></span>Cleaning up…`;
      try {
        if (!st.an) st.an = await invoke("edit_analyze", { path: it.path });
        const A = st.an, before = total(); push();
        const sil = A.silences.filter(([a, b]) => b - a >= 0.8).map(([a, b]) => [a < 0.3 ? 0 : a + 0.15, b >= dur - 0.3 ? dur : b - 0.15]).filter(([a, b]) => b > a);
        const idl = A.idle.filter(([a, b]) => b - a >= 2).map(([a, b]) => [a + 0.4, b - 0.4]).filter(([a, b]) => b > a);
        if (!sil.length && !idl.length) { hist.pop(); toast("Nothing to clean up. This recording is already tight"); }
        else { if (idl.length) speedRanges(idl, 4); if (sil.length) cutRanges(sil); st.sel = 0; seek(Math.min(st.t, total())); draw(); toast(`Cleaned up: ${sil.length} silent ${sil.length === 1 ? "part" : "parts"} removed, ${idl.length} still ${idl.length === 1 ? "moment" : "moments"} sped up. ${fmt(before - total())} shorter`); }
      } catch (err) { hist.pop(); toast(err, true); }
      btn.disabled = false; btn.innerHTML = old;
    }
    const facts = async () => { if (!st.an) st.an = await invoke("edit_analyze", { path: it.path }); return { duration: dur, markers: it.markers || [], silences: st.an.silences.slice(0, 40), idle: st.an.idle.slice(0, 40), scenes: st.an.scenes.slice(0, 40) }; };
    const opText = (o) => o.op === "cut" ? `Remove ${fmt(o.from)} to ${fmt(o.to)}` : o.op === "keep" ? `Keep only ${fmt(o.from)} to ${fmt(o.to)}` : o.op === "speed" ? `${o.speed}× from ${fmt(o.from)} to ${fmt(o.to)}` : o.op === "split" ? `Split at ${fmt(o.from)}` : `Caption “${o.text}” at ${fmt(o.from)}`;
    function applyOps(ops) {
      push();
      for (const o of ops) {
        if (o.op === "cut") cutRanges([[o.from, o.to]]);
        else if (o.op === "keep") cutRanges([[0, o.from], [o.to, dur]].filter(([a, b]) => b > a));
        else if (o.op === "speed") speedRanges([[o.from, o.to]], o.speed);
        else if (o.op === "split") splitTimes([o.from]);
        else if (o.op === "text") { const a = srcToTl(o.from); if (a != null) { const b = srcToTl(Math.min(o.to, dur)) ?? a + 3; st.ov.push({ id: uid++, kind: "text", start: r2(a), end: r2(Math.max(b, a + 1)), x: 0.08, y: 0.78, w: 0, h: 0, text: o.text, size: 0.06, color: "#ffffff" }); } }
      }
      st.sel = 0; st.selOv = null; seek(Math.min(st.t, total())); draw();
    }
    async function askAi(btn) {
      const box = $("#ai-q", side), out = $("#ai-out", side), q = box.value.trim();
      if (!q) { box.focus(); return; }
      const old = btn.innerHTML; btn.disabled = true; btn.innerHTML = `<span class="spin"></span>Thinking…`; out.innerHTML = "";
      try {
        const f = await facts(), r = await invoke("ai_edit", { instruction: q, facts: f });
        st.aiOps = r.ops;
        out.innerHTML = `<p>${esc(r.explain || "")}</p>${r.ops.length ? `<ul class="ai-ops">${r.ops.map((o) => `<li>${esc(opText(o))}</li>`).join("")}</ul><div class="row2"><button class="btn primary sm" id="ai-apply">Apply ${r.ops.length} ${r.ops.length === 1 ? "edit" : "edits"}</button><button class="btn sm" id="ai-no">Dismiss</button></div>` : `<p class="muted">No edits were suggested.</p>`}<p class="ai-note">Used ${r.tokens_in + r.tokens_out} tokens (${r.tokens_in} in, ${r.tokens_out} out). Apply can be undone with Ctrl+Z.</p>`;
        const ap = $("#ai-apply", out); if (ap) ap.onclick = () => { applyOps(st.aiOps); out.innerHTML = `<p class="ai-note">Applied. Check the timeline, and press Undo if it is not what you wanted.</p>`; };
        const no = $("#ai-no", out); if (no) no.onclick = () => (out.innerHTML = "");
      } catch (err) { out.innerHTML = `<p style="color:#ff453a">${esc(String(err))}</p>`; }
      btn.disabled = false; btn.innerHTML = old;
    }

    // ---------------------------------------------------------------- recipes: ready-made edits you can adjust
    const N = (k, label, unit, def, min, max, step) => ({ k, label, unit, def, min, max, step });
    const STEPS = {
      silence: { label: "Remove silences", hint: "Cuts pauses and joins what is left", params: [N("min", "Pauses longer than", "s", 0.8, 0.3, 3, 0.1), N("pad", "Keep around speech", "s", 0.15, 0, 0.6, 0.05)],
        async run(p) { const A = await analysis(), r = A.silences.filter(([a, b]) => b - a >= p.min).map(([a, b]) => [a < 0.3 ? 0 : a + p.pad, b >= dur - 0.3 ? dur : b - p.pad]).filter(([a, b]) => b > a); if (!r.length) return "No silences"; cutRanges(r); return `${r.length} silent ${r.length === 1 ? "part" : "parts"} removed`; } },
      idle: { label: "Speed up still screen", hint: "Fast-forwards while nothing changes", params: [N("speed", "Speed", "×", 4, 2, 8, 1), N("min", "Still for longer than", "s", 2, 1, 10, 0.5)],
        async run(p) { const A = await analysis(), r = A.idle.filter(([a, b]) => b - a >= p.min).map(([a, b]) => [a + 0.4, b - 0.4]).filter(([a, b]) => b > a); if (!r.length) return "No still moments"; speedRanges(r, Math.min(p.speed, 4)); return `${r.length} still ${r.length === 1 ? "moment" : "moments"} sped up`; } },
      markers: { label: "Keep my marked moments", hint: "Keeps only the parts around the markers you pressed (F8)", params: [N("before", "Before each marker", "s", 5, 1, 30, 1), N("after", "After each marker", "s", 10, 1, 60, 1)],
        async run(p) {
          const ms = (it.markers || []).slice().sort((a, b) => a - b); if (!ms.length) return "No markers in this recording, skipped";
          const keep = []; ms.forEach((m) => { const a = Math.max(0, m - p.before), b = Math.min(dur, m + p.after); const l = keep[keep.length - 1]; if (l && a <= l[1]) l[1] = Math.max(l[1], b); else keep.push([a, b]); });
          const cut = []; let pos = 0; keep.forEach(([a, b]) => { if (a > pos) cut.push([pos, a]); pos = b; }); if (pos < dur) cut.push([pos, dur]);
          if (cut.length) cutRanges(cut); return `Kept ${keep.length} marked ${keep.length === 1 ? "moment" : "moments"}`; } },
      fit: { label: "Fit a target length", hint: "Speeds everything up evenly if it is longer (up to 4×)", params: [N("sec", "Target length", "s", 60, 5, 600, 5)],
        async run(p) { const T = total(); if (T <= p.sec) return "Already short enough"; const f = T / p.sec; st.clips.forEach((c) => (c.speed = r2(clamp(c.speed * f, 0.25, 4)))); return total() > p.sec + 1 ? `Sped up to the limit, now ${fmt(total())}` : `Sped up ${f.toFixed(1)}× to fit`; } },
      transition: { label: "Smooth the joins", hint: "A short transition wherever pieces meet", params: [{ k: "kind", label: "Style", select: [["fade", "Fade"], ["dissolve", "Dissolve"], ["slide", "Slide"], ["wipe", "Wipe"], ["zoom", "Zoom"], ["circle", "Circle"]], def: "fade" }, N("dur", "Length", "s", 0.4, 0.2, 1.5, 0.1)],
        async run(p) { st.trans = { kind: p.kind, dur: p.dur }; return st.clips.length > 1 ? "Transitions set" : "Set, it shows once there are cuts"; } },
      title: { label: "Opening title", hint: "Use {name} for the recording's name", params: [{ k: "text", label: "Text", text: true, def: "{name}" }, N("dur", "Shown for", "s", 3, 1, 10, 0.5)],
        async run(p) { const t = p.text.replace(/\{name\}/g, it.name).trim(); if (!t) return "No text"; st.ov.push({ id: uid++, kind: "text", start: 0, end: Math.min(p.dur, total()), x: 0.08, y: 0.42, w: 0, h: 0, text: t, size: 0.09, color: "#ffffff" }); return "Title added"; } },
      endcard: { label: "Closing text", hint: "A line at the very end", params: [{ k: "text", label: "Text", text: true, def: "Thanks for watching" }, N("dur", "Shown for", "s", 3, 1, 10, 0.5)],
        async run(p) { const t = p.text.trim(); if (!t) return "No text"; const T = total(); st.ov.push({ id: uid++, kind: "text", start: r2(Math.max(0, T - p.dur)), end: r2(T), x: 0.08, y: 0.42, w: 0, h: 0, text: t, size: 0.08, color: "#ffffff" }); return "Closing text added"; } },
      output: { label: "Save as", hint: "The format of the finished video", params: [{ k: "preset", label: "Format", select: [["", "Original quality"], ["youtube", "YouTube"], ["small", "Small file"], ["vertical", "Vertical (phone)"], ["gif", "GIF"]], def: "" }],
        async run(p) { st.preset = p.preset; return "Format: " + (PRESETS.find(([k]) => k === p.preset) || [0, "Original"])[1]; } },
    };
    const mk = (t, p) => ({ t, on: true, p: { ...Object.fromEntries(STEPS[t].params.map((q) => [q.k, q.def])), ...(p || {}) } });
    const BUILTIN = [
      { id: "tutorial", name: "Tutorial polish", desc: "Tighten a how-to: no dead air, no waiting", steps: [mk("silence"), mk("idle"), mk("transition", { dur: 0.25 })] },
      { id: "social", name: "Short for social", desc: "A vertical clip of about a minute", steps: [mk("silence"), mk("idle"), mk("fit", { sec: 60 }), mk("output", { preset: "vertical" })] },
      { id: "highlights", name: "My marked moments", desc: "Only what you marked while recording", steps: [mk("markers"), mk("transition"), mk("output", { preset: "youtube" })] },
      { id: "share", name: "Quick share", desc: "Smaller and shorter, ready to send", steps: [mk("silence"), mk("idle"), mk("output", { preset: "small" })] },
      { id: "intro", name: "Titled video", desc: "Opening title and closing line", steps: [mk("silence"), mk("title"), mk("endcard"), mk("transition")] },
    ];
    const loadMine = () => { try { return JSON.parse(localStorage.getItem("recora.recipes") || "[]"); } catch (e) { return []; } };
    const saveMine = (a) => { try { localStorage.setItem("recora.recipes", JSON.stringify(a)); } catch (e) { toast("Could not save the recipe here", true); } };
    async function analysis() { if (!st.an) st.an = await invoke("edit_analyze", { path: it.path }); return st.an; }
    async function runRecipe(rc, exportAfter) {
      const before = total(), notes = []; push();
      for (const s of rc.steps) { if (!s.on) continue; try { notes.push(`${STEPS[s.t].label}: ${await STEPS[s.t].run(s.p)}`); } catch (err) { notes.push(`${STEPS[s.t].label}: ${err}`); } }
      st.sel = 0; st.selOv = null; seek(Math.min(st.t, total())); draw();
      toast(`${rc.name}: ${fmt(before)} → ${fmt(total())}`);
      return notes;
    }
    function openRecipes(start) {
      closeRecipes();
      let cur = start ? JSON.parse(JSON.stringify(start)) : null, notes = [], mine = loadMine();
      const v = document.createElement("div"); v.className = "veil rcp"; el.appendChild(v);
      const ctl = (s, i) => STEPS[s.t].params.map((q) => q.select ? `<label class="pr"><span>${q.label}</span><select data-i="${i}" data-k="${q.k}">${q.select.map(([k, n]) => `<option value="${k}" ${String(s.p[q.k]) === k ? "selected" : ""}>${n}</option>`).join("")}</select></label>` : q.text ? `<label class="pr"><span>${q.label}</span><input data-i="${i}" data-k="${q.k}" dir="auto" value="${esc(s.p[q.k])}"></label>` : `<label class="pr"><span>${q.label}</span><input type="range" data-i="${i}" data-k="${q.k}" min="${q.min}" max="${q.max}" step="${q.step}" value="${s.p[q.k]}"><b>${s.p[q.k]}${q.unit}</b></label>`).join("");
      const draw2 = () => {
        if (!cur) {
          const card = (r, own) => `<button class="rc" data-id="${r.id}"><b>${esc(r.name)}</b><span>${esc(r.desc || "")}</span><em>${r.steps.filter((s) => s.on).map((s) => STEPS[s.t].label).join(" · ")}</em>${own ? `<i class="rm" data-rm="${r.id}" title="Delete this recipe">×</i>` : ""}</button>`;
          v.innerHTML = `<div class="modal rmodal"><div class="ap-head"><div><h3>Recipes</h3><p class="muted">Ready-made edits. Pick one, adjust anything, and run it. Everything can be undone.</p></div><button class="close-x flat" id="rx">${I("x")}</button></div><div class="rgrid">${BUILTIN.map((r) => card(r)).join("")}${mine.map((r) => card(r, true)).join("")}<button class="rc new" id="rnew"><b>+ Build your own</b><span>Choose the steps and the numbers</span></button></div></div>`;
          $("#rx", v).onclick = closeRecipes;
          $$(".rc[data-id]", v).forEach((b) => (b.onclick = (ev) => { if (ev.target.dataset.rm) return; cur = JSON.parse(JSON.stringify([...BUILTIN, ...mine].find((r) => r.id === b.dataset.id))); notes = []; draw2(); }));
          $$("[data-rm]", v).forEach((b) => (b.onclick = () => { mine = mine.filter((r) => r.id !== b.dataset.rm); saveMine(mine); draw2(); }));
          $("#rnew", v).onclick = () => { cur = { id: "u" + Date.now(), name: "My recipe", desc: "", steps: [mk("silence")] }; notes = []; draw2(); };
          return;
        }
        v.innerHTML = `<div class="modal rmodal"><div class="ap-head"><div><input class="rname" id="rn" value="${esc(cur.name)}" aria-label="Recipe name"><p class="muted">${esc(cur.desc || "Steps run from top to bottom.")}</p></div><button class="close-x flat" id="rx">${I("x")}</button></div>
          <div class="rsteps">${cur.steps.map((s, i) => `<div class="rs ${s.on ? "" : "off"}"><div class="rh"><button class="switch ${s.on ? "on" : ""}" data-on="${i}" role="switch" aria-checked="${s.on}"></button><div class="t"><b>${STEPS[s.t].label}</b><span>${STEPS[s.t].hint}</span></div><button class="mini" data-up="${i}" ${i === 0 ? "disabled" : ""}>↑</button><button class="mini" data-dn="${i}" ${i === cur.steps.length - 1 ? "disabled" : ""}>↓</button><button class="mini" data-del="${i}">×</button></div>${s.on ? `<div class="rp">${ctl(s, i)}</div>` : ""}</div>`).join("")}
          <div class="radd"><select id="radd"><option value="">Add a step…</option>${Object.entries(STEPS).map(([k, d]) => `<option value="${k}">${d.label}</option>`).join("")}</select></div></div>
          ${notes.length ? `<ul class="rnotes">${notes.map((n) => `<li>${esc(n)}</li>`).join("")}</ul>` : ""}
          <div class="rfoot"><button class="btn" id="rback">Back</button><span class="grow"></span><button class="btn" id="rsave">Save as my recipe</button><button class="btn" id="rrun">Run</button><button class="btn primary" id="rgo">Run and export</button></div></div>`;
        $("#rx", v).onclick = closeRecipes; $("#rback", v).onclick = () => { cur = null; draw2(); };
        $("#rn", v).oninput = (ev) => (cur.name = ev.target.value);
        $$("[data-on]", v).forEach((b) => (b.onclick = () => { cur.steps[+b.dataset.on].on = !cur.steps[+b.dataset.on].on; draw2(); }));
        $$("[data-del]", v).forEach((b) => (b.onclick = () => { cur.steps.splice(+b.dataset.del, 1); draw2(); }));
        $$("[data-up]", v).forEach((b) => (b.onclick = () => { const i = +b.dataset.up; [cur.steps[i - 1], cur.steps[i]] = [cur.steps[i], cur.steps[i - 1]]; draw2(); }));
        $$("[data-dn]", v).forEach((b) => (b.onclick = () => { const i = +b.dataset.dn; [cur.steps[i + 1], cur.steps[i]] = [cur.steps[i], cur.steps[i + 1]]; draw2(); }));
        $$("[data-k]", v).forEach((c) => { const upd = () => { const q = STEPS[cur.steps[+c.dataset.i].t].params.find((x) => x.k === c.dataset.k); cur.steps[+c.dataset.i].p[c.dataset.k] = q.select || q.text ? c.value : +c.value; if (c.type === "range") c.nextElementSibling.textContent = c.value + q.unit; }; c.oninput = upd; c.onchange = upd; c.onkeydown = (ev) => ev.stopPropagation(); });
        $("#radd", v).onchange = (ev) => { if (ev.target.value) { cur.steps.push(mk(ev.target.value)); draw2(); } };
        $("#rsave", v).onclick = () => { const r = JSON.parse(JSON.stringify(cur)); r.id = "u" + Date.now(); r.name = (r.name || "My recipe").trim() || "My recipe"; mine = [...mine.filter((x) => x.name !== r.name), r]; saveMine(mine); toast("Saved in your recipes"); };
        const run = async (exp) => { const b = $(exp ? "#rgo" : "#rrun", v); b.disabled = true; b.innerHTML = `<span class="spin"></span>Working…`; notes = await runRecipe(cur); if (exp) { closeRecipes(); go(); } else draw2(); };
        $("#rrun", v).onclick = () => run(false); $("#rgo", v).onclick = () => run(true);
      };
      v.addEventListener("mousedown", (ev) => { if (ev.target === v) closeRecipes(); });
      draw2();
    }
    function closeRecipes() { const v = $(".rcp", el); if (v) v.remove(); }

    // ---------------------------------------------------------------- side panel
    function drawSide() {
      const o = st.ov.find((x) => x.id === st.selOv), c = st.clips[st.sel];
      const swatches = (cur) => COLORS.map((x) => `<button class="sw${x === cur ? " on" : ""}" data-c="${x}" style="background:${x}"></button>`).join("");
      side.innerHTML = `
        <button class="sm-b" id="quick" style="background:var(--accent);margin-bottom:2px"><b style="color:#fff">Quick clean-up</b><span style="color:rgba(255,255,255,.85)">One tap: removes silences and speeds up still moments</span></button>
        <button class="sm-b" id="rcps"><b>Recipes</b><span>Ready-made edits you can adjust: tutorial, social short, marked moments…</span></button>
        <div class="label sm">Ask the AI</div>
        <div class="card">${S_AI() ? `<textarea id="ai-q" dir="auto" rows="2" placeholder="For example: remove the first 10 seconds and speed up the slow parts"></textarea><div class="row2"><span class="ai-note" id="ai-est"></span><button class="btn primary sm" id="ai-go">Suggest edits</button></div><div id="ai-out"></div>` : `<p class="muted" style="margin:0">Optional. Add your own API key in Settings and describe the edit in a sentence. Nothing is sent until you press the button.</p><div class="row2"><button class="btn sm" id="ai-set">Set it up</button></div>`}</div>
        <div class="label sm">Cut by time</div>
        <div class="card cutbox"><textarea id="cut-t" rows="3" placeholder="One range per line, for example&#10;0:10-0:25&#10;1:30-2:00"></textarea><p class="ai-note" style="margin:0">Times follow the timeline below. Arabic numbers work too.</p><div class="row2"><button class="btn sm" id="cut-del">Delete these</button><button class="btn sm" id="cut-keep">Keep only these</button></div></div>
        <div class="label sm">Smart tools</div>
        <div class="smart"><button class="sm-b" data-smart="silence"><b>Remove silences</b><span>Cuts every pause longer than a moment</span></button><button class="sm-b" data-smart="idle"><b>Speed up still screen</b><span>Fast-forwards while nothing changes</span></button><button class="sm-b" data-smart="scenes"><b>Find scenes</b><span>Splits where the picture changes</span></button></div>
        <div class="label sm">Add to the video</div>
        <div class="add"><button class="btn" data-add="text">${I("text")}Text</button><button class="btn" data-add="box">${I("box")}Highlight</button><button class="btn" data-add="blur">${I("blur")}Blur</button></div>
        ${o ? `<div class="label sm">${o.kind === "text" ? "Text" : o.kind === "box" ? "Highlight" : "Blur"}</div><div class="card">
          ${o.kind === "text" ? `<textarea id="o-text" dir="auto" rows="2" placeholder="Type here">${esc(o.text)}</textarea><div class="row2"><span>Size</span><input type="range" id="o-size" min="0.02" max="0.2" step="0.005" value="${o.size || 0.06}"></div><div class="row2"><span>Background</span><div class="seg fit" id="o-bg"><button data-v="1" class="${o.bg !== false ? "on" : ""}">On</button><button data-v="0" class="${o.bg === false ? "on" : ""}">Off</button></div></div>` : ""}
          ${o.kind === "blur" ? `<div class="row2"><span>Strength</span><div class="seg fit" id="o-lv"><button data-v="1" class="${o.level === 1 ? "on" : ""}">Light</button><button data-v="2" class="${(o.level || 2) === 2 ? "on" : ""}">Medium</button><button data-v="3" class="${o.level === 3 ? "on" : ""}">Strong</button></div></div>` : `<div class="row2"><span>Colour</span><div class="sws">${swatches(o.color)}</div></div>`}
          <div class="row2"><span>Position</span><div class="al" id="o-al">${[["l", "Left"], ["c", "Centre"], ["r", "Right"], ["t", "Top"], ["m", "Middle"], ["b", "Bottom"]].map(([k, n]) => `<button data-a="${k}" title="${n}">${n[0]}</button>`).join("")}</div></div>
          <div class="row2"><span>From</span><input class="num" type="number" id="o-s" min="0" step="0.1" value="${o.start.toFixed(1)}"><span>to</span><input class="num" type="number" id="o-e" min="0" step="0.1" value="${o.end.toFixed(1)}"></div>
          <div class="row2"><button class="btn sm" id="o-here">Start here</button><button class="btn sm" id="o-end">End here</button></div>
          <div class="row2"><button class="btn sm" id="o-dup">Duplicate</button><button class="btn sm" id="o-del">${I("trash")}Remove</button></div>
          <p class="ai-note" style="margin:0">Drag on the picture, pull any handle to resize. Arrow keys nudge it, Alt turns snapping off.</p></div>` : ""}
        ${c ? `<div class="label sm">Selected piece</div><div class="card"><div class="row2"><span>Speed</span><div class="seg fit" id="sp">${SPEEDS.map((s) => `<button data-s="${s}" class="${c.speed === s ? "on" : ""}">${s}×</button>`).join("")}</div></div><div class="row2"><span>Length</span><span class="muted">${fmt(len(c))}</span></div></div>` : ""}
        <div class="label sm">Transition between pieces</div>
        <div class="card"><div class="tgrid" id="tr">${TRANS.map(([k, n]) => `<button data-k="${k}" class="${st.trans.kind === k ? "on" : ""}">${n}</button>`).join("")}</div>
          ${st.trans.kind !== "none" ? `<div class="row2"><span>Length</span><input type="range" id="tr-d" min="0.2" max="1.5" step="0.1" value="${st.trans.dur}"><span class="muted">${st.trans.dur}s</span></div>${st.clips.length < 2 ? `<p class="muted">Split the video to see it work.</p>` : ""}` : ""}</div>`;
      $("#rcps", side).onclick = () => openRecipes();
      $("#quick", side).onclick = (ev) => cleanup(ev.currentTarget);
      const cd = $("#cut-del", side); if (cd) { cd.onclick = () => byTime("cut", $("#cut-t", side).value); $("#cut-keep", side).onclick = () => byTime("keep", $("#cut-t", side).value); const ct = $("#cut-t", side); ct.onkeydown = (ev) => ev.stopPropagation(); ct.value = st.cutText || ""; ct.oninput = () => (st.cutText = ct.value); }
      const ag = $("#ai-go", side); if (ag) { const q = $("#ai-q", side); q.onkeydown = (ev) => ev.stopPropagation(); q.value = st.aiText || ""; ag.onclick = () => askAi(ag); const est = () => { st.aiText = q.value; if (!q.value.trim()) { $("#ai-est", side).textContent = ""; return; } facts().then((f) => invoke("ai_estimate", { instruction: q.value, facts: f })).then((n) => { $("#ai-est", side).textContent = `About ${n} tokens`; }).catch(() => {}); }; let tm; q.oninput = () => { clearTimeout(tm); tm = setTimeout(est, 600); }; }
      const as = $("#ai-set", side); if (as) as.onclick = () => { closeEditor(); window.__goSettings && window.__goSettings(); };
      $$("[data-smart]", side).forEach((b) => (b.onclick = () => smart(b.dataset.smart, b)));
      $$("[data-add]", side).forEach((b) => (b.onclick = () => addOverlay(b.dataset.add)));
      const t = $("#o-text", side); if (t) { t.oninput = () => { o.text = t.value; paintOverlays(); drawTl(); }; t.onfocus = push; t.onkeydown = (e) => e.stopPropagation(); }
      const sz = $("#o-size", side); if (sz) { sz.onpointerdown = push; sz.oninput = () => { o.size = +sz.value; paintOverlays(); }; }
      $$(".sw", side).forEach((b) => (b.onclick = () => { push(); o.color = b.dataset.c; paintOverlays(); drawSide(); }));
      const dl = $("#o-del", side); if (dl) dl.onclick = () => { push(); st.ov = st.ov.filter((x) => x !== o); st.selOv = null; draw(); };
      const oh = $("#o-here", side); if (oh) oh.onclick = () => { push(); const L = o.end - o.start; o.start = r2(st.t); o.end = r2(Math.min(total(), o.start + L)); if (o.end - o.start < 0.2) o.end = r2(Math.min(total(), o.start + 0.2)); draw(); };
      const oe = $("#o-end", side); if (oe) oe.onclick = () => { push(); o.end = r2(Math.max(o.start + 0.2, Math.min(total(), st.t))); draw(); };
      const os = $("#o-s", side), oe2 = $("#o-e", side);
      if (os) { [os, oe2].forEach((i) => { i.onkeydown = (ev) => { ev.stopPropagation(); if (ev.key === "Enter") i.blur(); }; i.onfocus = push; });
        const upd = () => { os.value = o.start.toFixed(1); oe2.value = o.end.toFixed(1); drawTl(); paintOverlays(); };
        os.onchange = () => { o.start = r2(clamp(+os.value || 0, 0, o.end - 0.2)); upd(); };
        oe2.onchange = () => { o.end = r2(clamp(+oe2.value || 0, o.start + 0.2, total())); upd(); }; }
      const od = $("#o-dup", side); if (od) od.onclick = dupOverlay;
      $$("#o-al button", side).forEach((b) => (b.onclick = () => align(o, b.dataset.a)));
      $$("#o-lv button", side).forEach((b) => (b.onclick = () => { push(); o.level = +b.dataset.v; drawSide(); paintOverlays(); }));
      $$("#o-bg button", side).forEach((b) => (b.onclick = () => { push(); o.bg = b.dataset.v === "1"; drawSide(); paintOverlays(); }));
      $$("#sp button", side).forEach((b) => (b.onclick = () => { push(); c.speed = +b.dataset.s; seek(Math.min(st.t, total())); draw(); }));
      $$("#tr button", side).forEach((b) => (b.onclick = () => { push(); st.trans.kind = b.dataset.k; draw(); }));
      const td = $("#tr-d", side); if (td) { td.onpointerdown = push; td.oninput = () => { st.trans.dur = +(+td.value).toFixed(1); td.nextElementSibling.textContent = st.trans.dur + "s"; }; }
    }
    function addOverlay(kind, pos) {
      push(); const T = total(), s = clamp(st.t, 0, Math.max(0, T - 0.5));
      const base = { id: uid++, kind, start: r2(s), end: r2(Math.min(T, s + 3)), x: 0.1, y: 0.1, w: 0.3, h: 0.2, text: "", size: 0.06, color: kind === "box" ? "#ffcc00" : "#ffffff", level: 2, bg: true };
      if (kind === "text") { base.text = "Your text"; base.y = 0.78; base.x = 0.08; }
      if (kind === "blur") { base.x = 0.35; base.y = 0.35; }
      if (pos) { base.x = clamp(pos.x - (kind === "text" ? 0.02 : base.w / 2), 0, 0.9); base.y = clamp(pos.y - (kind === "text" ? 0.03 : base.h / 2), 0, 0.9); }
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
        overlays: st.ov.map((o) => ({ kind: o.kind, start: Math.max(0, shift(o.start)), end: Math.max(0.2, shift(o.end)), x: o.x, y: o.y, w: o.w, h: o.h, text: o.text, size: o.size, color: o.color, level: o.level || 2, bg: o.bg !== false })),
      };
      void S;
      st.busy = true; setPlay(false); const b = $("#ed-go", el); b.disabled = true; b.innerHTML = `<span class="spin"></span>Exporting…`;
      try { const out = await invoke("edit_export", { project }); if (!opts.auto) toast("Saved: " + String(out).split(/[\\/]/).pop()); if (onDone) onDone(out, total()); }
      catch (e) { toast(e, true); }
      st.busy = false; b.disabled = false; b.textContent = "Export";
    }

    // ---------------------------------------------------------------- right-click menu
    function closeMenu() { const m = $(".ctx", el); if (m) m.remove(); }
    function showMenu(x, y, items) {
      closeMenu();
      const m = document.createElement("div"); m.className = "ctx";
      const build = (list) => list.map((it2, i) => it2 === "-" ? `<hr>` : it2.head ? `<div class="ch">${it2.head}</div>` : `<div class="ci${it2.off ? " off" : ""}${it2.danger ? " danger" : ""}" data-i="${i}">${it2.sub ? "" : ""}<span>${it2.label}</span>${it2.key ? `<kbd>${it2.key}</kbd>` : ""}${it2.sub ? `<i class="chev">›</i>` : ""}${it2.sub ? `<div class="ctx sub">${build(it2.sub)}</div>` : ""}</div>`).join("");
      m.innerHTML = build(items); el.appendChild(m);
      const wire = (root, list) => $$(":scope > .ci", root).forEach((b) => {
        const d = list[+b.dataset.i]; if (!d || d.off) return;
        if (d.sub) { const sm = $(":scope > .sub", b); wire(sm, d.sub); b.onmouseenter = () => { sm.classList.add("on"); const r = sm.getBoundingClientRect(); if (r.right > innerWidth - 8) sm.style.left = "auto", sm.style.right = "100%"; if (r.bottom > innerHeight - 8) sm.style.top = innerHeight - 8 - r.bottom + "px"; }; b.onmouseleave = () => sm.classList.remove("on"); return; }
        b.onclick = (ev) => { ev.stopPropagation(); closeMenu(); d.fn(); };
      });
      wire(m, items);
      const r = m.getBoundingClientRect();
      m.style.left = Math.min(x, innerWidth - r.width - 8) + "px"; m.style.top = Math.min(y, innerHeight - r.height - 8) + "px";
    }
    const speedItems = (c) => SPEEDS.map((s) => ({ label: `${s}×${c.speed === s ? "  ✓" : ""}`, fn: () => { push(); c.speed = s; seek(Math.min(st.t, total())); draw(); } }));
    function dupClip(i) { push(); const c = st.clips[i]; st.clips.splice(i + 1, 0, { ...c, id: uid++ }); st.sel = i + 1; draw(); }
    function moveClip(i, d) { const j = i + d; if (j < 0 || j >= st.clips.length) return; push(); const [c] = st.clips.splice(i, 1); st.clips.splice(j, 0, c); st.sel = j; seek(Math.min(st.t, total())); draw(); }
    function trimHere(side) {
      const i = where(st.t), c = st.clips[i], src = c.start + (st.t - starts()[i]) * c.speed;
      if (side === "before") { if (src - c.start < 0.2 && i === 0) { toast("Nothing before the playhead"); return; } push(); st.clips = st.clips.slice(i); st.clips[0] = { ...st.clips[0], start: Math.min(src, c.end - 0.2) }; }
      else { if (c.end - src < 0.2 && i === st.clips.length - 1) { toast("Nothing after the playhead"); return; } push(); st.clips = st.clips.slice(0, i + 1); st.clips[i] = { ...st.clips[i], end: Math.max(src, c.start + 0.2) }; }
      st.sel = 0; st.ov.forEach((o) => { o.end = Math.min(o.end, total()); o.start = Math.min(o.start, o.end - 0.2); }); seek(Math.min(st.t, total())); draw();
    }
    const smartItems = () => [{ head: "Smart" }, { label: "Remove silences", fn: () => smart("silence", $("[data-smart=silence]", side)) }, { label: "Speed up still screen", fn: () => smart("idle", $("[data-smart=idle]", side)) }, { label: "Find scenes", fn: () => smart("scenes", $("[data-smart=scenes]", side)) }];
    const addItems = (pos) => [{ head: "Add here" }, { label: "Text", fn: () => addOverlay("text", pos) }, { label: "Highlight", fn: () => addOverlay("box", pos) }, { label: "Blur", fn: () => addOverlay("blur", pos) }];
    function overlayMenu(o, x, y) {
      st.selOv = o.id; st.sel = -1; drawSide(); drawTl();
      showMenu(x, y, [
        { label: "Duplicate", fn: () => { push(); const n2 = { ...o, id: uid++, start: Math.min(o.start + 0.5, Math.max(0, total() - 0.5)), x: Math.min(o.x + 0.03, 0.95), y: Math.min(o.y + 0.04, 0.95) }; n2.end = Math.min(total(), n2.start + (o.end - o.start)); st.ov.push(n2); st.selOv = n2.id; draw(); } },
        { label: "Start at the playhead", fn: () => { push(); const L = o.end - o.start; o.start = st.t; o.end = Math.min(total(), o.start + L); draw(); } },
        { label: "Show until the end", fn: () => { push(); o.end = total(); draw(); } },
        { label: "Show for the whole video", fn: () => { push(); o.start = 0; o.end = total(); draw(); } },
        ...(o.kind !== "blur" ? [{ label: "Colour", sub: COLORS.map((c) => ({ label: `<i class="dotc" style="background:${c}"></i>${c === "#ffffff" ? "White" : c === "#ffcc00" ? "Yellow" : c === "#ff3b30" ? "Red" : c === "#34c759" ? "Green" : c === "#0a84ff" ? "Blue" : "Black"}${o.color === c ? "  ✓" : ""}`, fn: () => { push(); o.color = c; draw(); } })) }] : []),
        "-", { label: "Delete", key: "Del", danger: true, fn: () => { push(); st.ov = st.ov.filter((x2) => x2 !== o); st.selOv = null; draw(); } },
      ]);
    }
    function clipMenu(i, ev) {
      const c = st.clips[i]; st.sel = i; st.selOv = null; setPlay(false); seek(timeAt(ev)); drawSide(); drawTl();
      showMenu(ev.clientX, ev.clientY, [
        { label: "Split here", key: "S", fn: split },
        { label: "Duplicate piece", fn: () => dupClip(i) },
        { label: "Speed", sub: speedItems(c) },
        { label: "Move left", off: i === 0, fn: () => moveClip(i, -1) },
        { label: "Move right", off: i === st.clips.length - 1, fn: () => moveClip(i, 1) },
        "-", { label: "Delete everything before here", fn: () => trimHere("before") }, { label: "Delete everything after here", fn: () => trimHere("after") },
        ...addItems(null), ...smartItems(),
        "-", { label: "Delete piece", key: "Del", danger: true, off: st.clips.length < 2, fn: del },
      ]);
    }
    el.addEventListener("contextmenu", (e) => {
      if (/INPUT|TEXTAREA/.test(e.target.tagName)) return;
      e.preventDefault(); e.stopPropagation();
      const ob = e.target.closest(".ob"), ov = e.target.closest(".ov"), cl = e.target.closest(".cl");
      const oid = ob ? ob.dataset.id : ov ? ov.dataset.id : null;
      if (oid) { const o = st.ov.find((x) => x.id == oid); if (o) return overlayMenu(o, e.clientX, e.clientY); }
      if (cl) return clipMenu(+cl.dataset.i, e);
      const fr = frame.getBoundingClientRect(), inFrame = e.clientX >= fr.left && e.clientX <= fr.right && e.clientY >= fr.top && e.clientY <= fr.bottom;
      if (inFrame) return showMenu(e.clientX, e.clientY, [{ label: st.playing ? "Pause" : "Play", key: "Space", fn: () => setPlay(!st.playing) }, { label: "Split at the playhead", key: "S", fn: split }, ...addItems({ x: (e.clientX - fr.left) / fr.width, y: (e.clientY - fr.top) / fr.height }), ...smartItems()]);
      if (e.target.closest("#tl")) return showMenu(e.clientX, e.clientY, [{ label: "Go to start", fn: () => { setPlay(false); seek(0); } }, { label: "Go to end", fn: () => { setPlay(false); seek(total()); } }, ...addItems(null), ...smartItems()]);
      showMenu(e.clientX, e.clientY, [{ label: "Undo", key: "Ctrl+Z", off: !hist.length, fn: undo }, ...smartItems()]);
    }, true);
    el.addEventListener("pointerdown", (e) => { if (!e.target.closest(".ctx")) closeMenu(); }, true);
    el.addEventListener("wheel", closeMenu, { passive: true });

    // ---------------------------------------------------------------- menu bar
    const CUT_FOCUS = () => { const t = $("#cut-t", side); if (t) { t.scrollIntoView({ block: "center" }); t.focus(); } };
    function shortcuts() {
      const v = document.createElement("div"); v.className = "veil rcp"; el.appendChild(v);
      const rows = [["Space", "Play or pause"], ["S", "Split at the playhead"], ["Delete", "Delete the selected piece or item"], ["Ctrl + Z", "Undo"], ["← →", "One frame back or forward"], ["Shift + ← →", "Five seconds"], ["Right-click", "More options for what is under the pointer"], ["Esc", "Close a menu, then the editor"]];
      v.innerHTML = `<div class="modal rmodal"><div class="ap-head"><div><h3>Keyboard shortcuts</h3></div><button class="close-x flat" id="kx">${I("x")}</button></div><div class="krows">${rows.map(([k, d]) => `<div><kbd>${k}</kbd><span>${d}</span></div>`).join("")}</div></div>`;
      const c = () => v.remove(); $("#kx", v).onclick = c; v.addEventListener("mousedown", (ev) => { if (ev.target === v) c(); });
    }
    const menuDefs = () => ({
      File: [{ label: "Open another video…", fn: () => { closeEditor(); window.__pickVideo && window.__pickVideo(); } }, { label: "Export", key: "", fn: go }, { label: "Export as", sub: PRESETS.map(([k, n]) => ({ label: n, fn: () => { st.preset = k; draw(); go(); } })) }, "-", { label: "Close editor", key: "Esc", fn: closeEditor }],
      Edit: [{ label: "Undo", key: "Ctrl+Z", off: !hist.length, fn: undo }, "-", { label: "Split at the playhead", key: "S", fn: split }, { label: "Delete selected", key: "Del", fn: del }, "-", { label: "Delete everything before the playhead", fn: () => trimHere("before") }, { label: "Delete everything after the playhead", fn: () => trimHere("after") }],
      Clip: [{ label: "Duplicate", off: st.sel < 0, fn: () => dupClip(st.sel) }, { label: "Speed", off: st.sel < 0, sub: st.sel >= 0 ? speedItems(st.clips[st.sel]) : [] }, { label: "Move left", off: st.sel <= 0, fn: () => moveClip(st.sel, -1) }, { label: "Move right", off: st.sel < 0 || st.sel >= st.clips.length - 1, fn: () => moveClip(st.sel, 1) }, "-", { label: "Transition between pieces", sub: TRANS.map(([k, n]) => ({ label: n + (st.trans.kind === k ? "  ✓" : ""), fn: () => { push(); st.trans.kind = k; draw(); } })) }],
      Add: [{ label: "Text", fn: () => addOverlay("text") }, { label: "Highlight box", fn: () => addOverlay("box") }, { label: "Blur area", fn: () => addOverlay("blur") }],
      Tools: [{ label: "Quick clean-up", fn: () => cleanup($("#quick", side)) }, { label: "Recipes…", fn: () => openRecipes() }, "-", { label: "Remove silences", fn: () => smart("silence", $("[data-smart=silence]", side)) }, { label: "Speed up still screen", fn: () => smart("idle", $("[data-smart=idle]", side)) }, { label: "Find scenes", fn: () => smart("scenes", $("[data-smart=scenes]", side)) }, "-", { label: "Cut by time…", fn: CUT_FOCUS }, { label: "Ask the AI…", fn: () => { const q = $("#ai-q", side); if (q) { q.scrollIntoView({ block: "center" }); q.focus(); } else toast("Add your API key in Settings to use this") } }],
      View: [{ label: "Fit the timeline", fn: () => { st.zoom = 1; $("#b-zoom", el).value = 1; drawTl(); } }, { label: "Zoom in", fn: () => { st.zoom = Math.min(8, st.zoom * 1.5); $("#b-zoom", el).value = st.zoom; drawTl(); } }, { label: "Zoom out", fn: () => { st.zoom = Math.max(1, st.zoom / 1.5); $("#b-zoom", el).value = st.zoom; drawTl(); } }],
      Help: [{ label: "Keyboard shortcuts", fn: shortcuts }, { label: "Right-click anywhere for more options", off: true, fn: () => {} }],
    });
    $$("#mb button", el).forEach((b) => {
      const open = () => { $$("#mb button", el).forEach((x) => x.classList.toggle("on", x === b)); const r = b.getBoundingClientRect(); showMenu(r.left, r.bottom + 2, menuDefs()[b.dataset.m]); };
      b.onclick = (ev) => { ev.stopPropagation(); if (b.classList.contains("on") && $(".ctx", el)) { closeMenu(); b.classList.remove("on"); } else open(); };
      b.onmouseenter = () => { if ($(".ctx", el) && $$("#mb button.on", el).length && !b.classList.contains("on")) open(); };
    });
    el.addEventListener("pointerdown", (ev) => { if (!ev.target.closest("#mb")) $$("#mb button.on", el).forEach((x) => x.classList.remove("on")); }, true);

    // ---------------------------------------------------------------- wiring
    function paint() { $("#b-time", el).textContent = `${fmt(st.t)} / ${fmt(total())}`; paintHead(); paintOverlays(); }
    function draw() { $("#ed-title", el).textContent = it.name; $$("#ed-pre button", el).forEach((b) => b.classList.toggle("on", b.dataset.k === st.preset)); $("#b-play", el).innerHTML = I(st.playing ? "pause" : "play") + (st.playing ? "Pause" : "Play"); $("#b-undo", el).disabled = !hist.length; drawSide(); drawTl(); paint(); }
    $$("#ed-pre button", el).forEach((b) => (b.onclick = () => { st.preset = b.dataset.k; draw(); }));
    $("#ed-x", el).onclick = closeEditor; $("#ed-go", el).onclick = go;
    $("#b-play", el).onclick = () => setPlay(!st.playing);
    $("#b-split", el).onclick = split; $("#b-del", el).onclick = del; $("#b-undo", el).onclick = undo;
    $("#b-zoom", el).oninput = (e) => { st.zoom = +e.target.value; drawTl(); };
    vid.addEventListener("loadedmetadata", () => { if (vid.videoWidth) { st.ar = vid.videoWidth / vid.videoHeight; frame.style.aspectRatio = st.ar; } paint(); });
    vid.addEventListener("click", () => { if (st.selOv) { st.selOv = null; draw(); return; } setPlay(!st.playing); });
    const key = (e) => {
      if (!$("#editor")) return document.removeEventListener("keydown", key, true);
      if (/INPUT|TEXTAREA/.test(document.activeElement.tagName)) return;
      if (e.code === "Space") { e.preventDefault(); setPlay(!st.playing); }
      else if (e.key === "s" || e.key === "S") split();
      else if (e.key === "Delete" || e.key === "Backspace") del();
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); }
      else if (e.key === "Escape") { if ($(".ctx", el)) closeMenu(); else if (st.selOv) { st.selOv = null; draw(); } else if ($(".rcp", el)) closeRecipes(); else closeEditor(); }
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") { e.preventDefault(); dupOverlay(); }
      else if (st.selOv && e.key.startsWith("Arrow")) { e.preventDefault(); const k = e.shiftKey ? 0.02 : 0.004; if (e.key === "ArrowLeft") nudge(-k, 0); else if (e.key === "ArrowRight") nudge(k, 0); else if (e.key === "ArrowUp") nudge(0, -k); else nudge(0, k); }
      else if (e.key === "ArrowLeft") { setPlay(false); seek(st.t - (e.shiftKey ? 5 : 1 / 30)); }
      else if (e.key === "ArrowRight") { setPlay(false); seek(st.t + (e.shiftKey ? 5 : 1 / 30)); }
      e.stopPropagation();
    };
    document.addEventListener("keydown", key, true);
    new ResizeObserver(() => { drawTl(); paintOverlays(); }).observe(tlEl);
    draw(); seek(0);
    window.__editor = st;
    // started by the autopilot: apply a ready-made recipe, and when asked, export without showing anything
    if (opts.recipeId) {
      const rc = BUILTIN.find((r) => r.id === opts.recipeId);
      if (opts.auto) el.style.display = "none";
      if (rc) (async () => { await runRecipe(rc); if (opts.auto) { await go(); closeEditor(); } })();
    }
  };
  function closeEditor() { const e = $("#editor"); if (e) { const v = $("video", e); if (v) v.pause(); e.remove(); } }
  window.closeEditor = closeEditor;
})();
