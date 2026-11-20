/* Autopilot: after a recording is saved, look at it quietly in the background and, only when it clearly helps,
   offer one calm suggestion. It never interrupts, never repeats, and learns to stay quiet. */
(() => {
  const { invoke, $, $$, esc, fmt, icon: I, toast } = R;
  const VIDEO = ["mp4", "mkv", "webm", "mov"];
  const seen = new Set();
  let pending = null, timer = 0;
  const mem = () => { try { return JSON.parse(localStorage.getItem("recora.ap") || "{}"); } catch (e) { return {}; } };
  const save = (m) => { try { localStorage.setItem("recora.ap", JSON.stringify(m)); } catch (e) {} };
  const mode = () => (window.__S && window.__S().autopilot) || "suggest";
  const sum = (v, f) => v.reduce((s, r) => s + f(r), 0);

  // what is worth offering? the numbers come from the quick analysis, no AI involved
  function decide(it, A) {
    const dur = it.duration, ms = (it.markers || []).slice().sort((a, b) => a - b);
    const sil = A.silences.filter(([a, b]) => b - a >= 0.8), idl = A.idle.filter(([a, b]) => b - a >= 2);
    const saves = sum(sil, ([a, b]) => b - a - 0.3) + sum(idl, ([a, b]) => (b - a - 0.8) * 0.75);
    if (ms.length >= 2 && dur > 60) {
      let cover = 0, last = -1e9; ms.forEach((m) => { const a = Math.max(0, m - 5), b = Math.min(dur, m + 10); cover += b - Math.max(a, last); last = Math.max(last, b); });
      if (cover < dur * 0.6) return { kind: "highlights", recipe: "highlights", title: `Keep your ${ms.length} marked moments`, text: `Saves ${fmt(dur - cover)}. Only the parts around the markers you pressed.`, saves: dur - cover };
    }
    if (saves >= Math.max(10, dur * 0.1)) {
      const bits = []; if (sil.length) bits.push(`remove ${sil.length} silent ${sil.length === 1 ? "part" : "parts"}`); if (idl.length) bits.push(`speed up ${idl.length} waiting ${idl.length === 1 ? "moment" : "moments"}`);
      return { kind: "tutorial", recipe: "tutorial", title: `Saves ${fmt(saves)}`, text: bits.join(" and ").replace(/^./, (c) => c.toUpperCase()) + ".", saves };
    }
    if (it.size > 300 * 1024 * 1024) return { kind: "share", recipe: "share", title: "This file is large", text: `${Math.round(it.size / 1048576)} MB. Make a smaller copy that is easy to send?`, saves: 0 };
    return null;
  }

  function markup(p, inline) {
    return `<div class="ap${inline ? " inline" : ""}" role="status"><div class="ap-t"><b>${esc(p.title)}</b><span>${esc(p.text)}</span></div>
      <button class="btn primary sm" data-a="polish">${p.kind === "highlights" ? "Make it" : p.kind === "share" ? "Make a copy" : "Polish it"}</button><button class="btn sm" data-a="custom">Customize</button><button class="ap-x" data-a="no" title="Not now" aria-label="Not now">×</button><a class="lnk" data-a="never">Don't suggest this</a></div>`;
  }
  function wire(root, p) {
    $$("[data-a]", root).forEach((b) => (b.onclick = () => act(b.dataset.a, p)));
    root.onmouseenter = () => clearTimeout(timer);
  }
  function hide() { const b = $("#ap-bar"); if (b) b.remove(); clearTimeout(timer); }
  const dot = (on) => { const d = $$(".dot")[2]; if (d) d.hidden = !on; };
  function show(p) {
    pending = p; hide();
    if ($("#editor")) { dot(true); return; } // never over the editor, wait quietly
    const bar = document.createElement("div"); bar.id = "ap-bar"; bar.innerHTML = markup(p); document.body.appendChild(bar); wire(bar, p);
    // fades into a dot next to Editor after a while, still there when wanted
    timer = setTimeout(() => { hide(); dot(true); }, 15000);
  }
  function busy(p, text) { const b = $("#ap-bar") || (() => { const x = document.createElement("div"); x.id = "ap-bar"; document.body.appendChild(x); return x; })(); b.innerHTML = `<div class="ap"><div class="ap-t"><b>${esc(text)}</b><span>This runs in the background. You can keep working.</span></div><span class="spin"></span></div>`; }
  async function act(a, p) {
    const m = mem(); m.skip = m.skip || {}; m.mute = m.mute || {};
    if (a === "no") { m.skip[p.kind] = (m.skip[p.kind] || 0) + 1; if (m.skip[p.kind] >= 2) m.mute[p.kind] = true; save(m); pending = null; hide(); dot(false); renderPage(); return; }
    if (a === "never") { m.mute[p.kind] = true; save(m); pending = null; hide(); dot(false); renderPage(); toast("Okay, no more suggestions like this"); return; }
    pending = null; dot(false); renderPage();
    if (a === "custom") { hide(); window.__closeClip && window.__closeClip(); openEditor(p.it, () => window.__refreshLib && window.__refreshLib(), { recipeId: p.recipe }); return; }
    busy(p, "Polishing your recording…");
    openEditor(p.it, (out, len) => {
      window.__refreshLib && window.__refreshLib();
      const bar = $("#ap-bar"); if (!bar) return;
      bar.innerHTML = `<div class="ap"><div class="ap-t"><b>Done. A polished copy is saved</b><span>${len ? `${fmt(len)} long, ${fmt(Math.max(0, p.it.duration - len))} shorter. ` : ""}Your original is untouched.</span></div><button class="btn primary sm" id="ap-show">Show</button></div>`;
      $("#ap-show", bar).onclick = () => { invoke("reveal", { path: out }).catch(() => {}); hide(); };
      timer = setTimeout(hide, 10000);
    }, { recipeId: p.recipe, auto: true });
  }

  // a quiet card at the top of the Editor page for a suggestion that faded away
  function renderPage() {
    const root = $("#p-editor"); if (!root) return;
    const old = $("#ap-inline", root); if (old) old.remove();
    dot(!!pending);
    if (!pending) return;
    const h = document.createElement("div"); h.id = "ap-inline"; h.innerHTML = `<div class="label">Suggestion for “${esc(pending.it.name)}”</div>` + markup(pending, true);
    const head = $(".head", root); if (head) head.after(h); wire(h, pending);
  }

  async function onSaved(it) {
    const m = mem(), md = mode();
    if (md === "off" || !it || !VIDEO.includes(it.ext) || it.duration < 20 || seen.has(it.path)) return;
    seen.add(it.path);
    let A; try { A = await invoke("edit_analyze_quick", { path: it.path }); } catch (e) { return; }
    const p = decide(it, A); if (!p || (m.mute && m.mute[p.kind])) return;
    p.it = it;
    if (md === "auto" && p.kind === "tutorial") { act("polish", p); return; }
    show(p);
  }
  window.autopilot = { onSaved, renderPage };
})();
