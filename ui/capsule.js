(() => {
  const { icon: I, $, invoke, listen, fmt } = R;
  const pill = $("#pill"), tip = $("#tip");
  const first = new URLSearchParams(location.search).get("labels") === "1";
  let st = { phase: "countdown", secs: 0, mic: 0, sys: 0, micMuted: false, sysMuted: false };
  let askT = null, mode = "countdown", open = false, labels = false, closeT = null;
  let keys = { toggle: "F9", pause: "F10", marker: "F8" };
  invoke("get_settings").then((s) => { if (s) keys = { toggle: s.hotkey_toggle, pause: s.hotkey_pause, marker: s.hotkey_marker }; }).catch(() => {});

  // the window is small while the island rests and grows while you point at it
  const W = { rest: [222, 52], open: [346, 60], labels: [346, 82], tip: [346, 82] };
  const size = (k) => invoke("island_size", { w: W[k][0], h: W[k][1] }).catch(() => {});
  const setOpen = (on, withLabels) => {
    clearTimeout(closeT);
    if (mode !== "rec") on = false;
    if (on) {
      labels = !!withLabels; size(labels ? "labels" : "tip");
      requestAnimationFrame(() => { pill.classList.add("open"); pill.classList.toggle("labels", labels); });
      open = true;
    } else {
      pill.classList.remove("open", "labels"); open = false; labels = false; tip.classList.remove("on");
      closeT = setTimeout(() => size("rest"), 340);
    }
  };
  pill.addEventListener("mouseenter", () => setOpen(true));
  document.addEventListener("mouseleave", () => { if (!labels) setOpen(false); });

  const label = (k) => ({ bm: ["Mute microphone", ""], bs: ["Mute sound", ""], bk: ["Marker", keys.marker], bp: [st.phase === "paused" ? "Resume" : "Pause", keys.pause], bx: ["Stop and save", keys.toggle], bc: ["Discard recording", ""] }[k]);
  const showTip = (btn) => {
    if (labels) return;
    const [t, k] = label(btn.id); const r = btn.getBoundingClientRect();
    tip.innerHTML = `${t}${k ? `<kbd>${k}</kbd>` : ""}`; tip.style.left = Math.min(innerWidth - 60, Math.max(60, r.left + r.width / 2)) + "px"; tip.classList.add("on");
  };
  const showCount = (n) => {
    pill.className = "pill"; mode = "countdown";
    pill.innerHTML = `<div class="num" id="n">${n}</div><span class="txt">Starting</span><span class="sp" data-tauri-drag-region></span><button class="cb" id="cancel" aria-label="Cancel">${I("x")}</button>`;
    $("#cancel").onclick = () => invoke("stop_recording");
  };
  const build = () => {
    const keep = open ? " open" + (labels ? " labels" : "") : "";
    pill.className = "pill rec" + (st.phase === "paused" ? " paused" : "") + keep;
    pill.innerHTML = `<i class="dot"></i><span class="time" id="time" data-tauri-drag-region>${fmt(st.secs)}</span><span class="sp" data-tauri-drag-region></span>
      <span class="opt">
      <button class="cb ${st.micMuted ? "off" : ""}" id="bm" aria-label="Mute microphone">${I(st.micMuted ? "micoff" : "mic")}<span class="lv"><i id="lm"></i></span><small>${st.micMuted ? "Unmute" : "Mic"}</small></button>
      <button class="cb ${st.sysMuted ? "off" : ""}" id="bs" aria-label="Mute sound">${I(st.sysMuted ? "speakeroff" : "speaker")}<span class="lv"><i id="ls"></i></span><small>${st.sysMuted ? "Unmute" : "Sound"}</small></button>
      <button class="cb" id="bk" aria-label="Marker">${I("flag")}<small>Marker</small></button></span>
      <button class="cb pz" id="bp" aria-label="Pause">${I(st.phase === "paused" ? "play" : "pause")}<small>${st.phase === "paused" ? "Resume" : "Pause"}</small></button>
      <button class="cb stop" id="bx" aria-label="Stop">${I("stop")}<small>Stop</small></button>
      <button class="cb cancel" id="bc" aria-label="Discard recording">${I("x")}<small>Discard</small></button>`;
    $("#bm").onclick = () => invoke("toggle_mute", { kind: "mic" });
    $("#bs").onclick = () => invoke("toggle_mute", { kind: "sys" });
    $("#bk").onclick = () => { invoke("add_marker"); flash(); };
    $("#bp").onclick = () => invoke("toggle_pause");
    $("#bx").onclick = () => invoke("stop_recording");
    // discard needs a second click, so a recording is never thrown away by accident
    const bc = $("#bc");
    bc.onclick = () => {
      if (!pill.classList.contains("ask-on")) {
        pill.classList.add("ask-on"); bc.classList.add("ask"); bc.innerHTML = "Discard?";
        askT = setTimeout(() => { pill.classList.remove("ask-on"); bc.classList.remove("ask"); bc.innerHTML = I("x") + "<small>Discard</small>"; }, 3000);
      } else { clearTimeout(askT); invoke("cancel_recording"); }
    };
    ["bm", "bs", "bk", "bp", "bx", "bc"].forEach((id) => { const b = $("#" + id); b.onmouseenter = () => showTip(b); b.onmouseleave = () => tip.classList.remove("on"); });
  };
  const flash = () => { const b = $("#bk"); if (!b) return; b.classList.add("flash"); setTimeout(() => b.classList.remove("flash"), 350); };
  showCount(3);
  let shownLabels = false;
  listen("rec:countdown", (n) => {
    if (n > 0) { if (!$("#n")) showCount(n); else $("#n").textContent = n; }
    else { mode = "rec"; build(); if (first && !shownLabels) { shownLabels = true; setOpen(true, true); setTimeout(() => { if (labels) setOpen(false); }, 6000); } }
  });
  listen("rec:tick", (t) => {
    const prev = st; st = t;
    if (mode !== "rec") return;
    if (!$("#time") || prev.phase !== t.phase || prev.micMuted !== t.micMuted || prev.sysMuted !== t.sysMuted) build();
    $("#time").textContent = fmt(t.secs);
    $("#lm").style.width = Math.min(100, t.mic * 160) + "%";
    $("#ls").style.width = Math.min(100, t.sys * 160) + "%";
  });
  listen("rec:marker", flash);
  listen("rec:saving", () => { mode = "saving"; setOpen(false); pill.className = "pill"; pill.innerHTML = `<span class="spin"></span><span class="txt">Saving…</span>`; });
})();
