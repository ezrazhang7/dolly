// Dolly editor frontend. No framework, no bundler: the backend does the math
// (dolly-core via Tauri commands), this file animates the result and edits the
// non-destructive project data (segments + settings).

const { invoke, convertFileSrc } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const $ = (id) => document.getElementById(id);
const els = {
  canvas: $("canvas"), frame: $("frame"), screen: $("screen"), vid: $("vid"),
  mockscreen: $("mockscreen"), cursor: $("cursor"), ripples: $("ripples"),
  play: $("btn-play"), timecode: $("timecode"), mockbadge: $("mockbadge"), recbadge: $("recbadge"),
  ticks: $("ticks"), clickdots: $("clickdots"), blocks: $("blocks"), playhead: $("playhead"),
  ruler: $("ruler"), timeline: $("timeline"), stage: $("stage"),
  sessionInfo: $("session-info"), segInfo: $("seg-info"), secSegment: $("sec-segment"),
  toast: $("toast"), empty: $("empty"),
};

const GRADIENTS = {
  dusk: "linear-gradient(135deg, #667eea 0%, #764ba2 100%)",
  ember: "linear-gradient(135deg, #f5576c 0%, #f093fb 100%)",
  ocean: "linear-gradient(135deg, #4facfe 0%, #00f2fe 100%)",
  forest: "linear-gradient(135deg, #43e97b 0%, #38f9d7 100%)",
  graphite: "linear-gradient(135deg, #45474f 0%, #101014 100%)",
};

const S = {
  session: null,   // SessionDto from Rust
  plan: null,      // PlanDto from Rust
  segments: [],    // working copy (the source of truth for edits)
  selected: -1,
  playing: false,
  recording: false,
  t: 0,            // preview clock in ms (mock sessions)
  rippled: new Set(),
};

const meta = () => S.session.project.meta;
const dur = () => (S.session ? meta().duration_ms : 0);
const cam = () => S.session.project.camera;
const cur = () => S.session.project.cursor;
const sty = () => S.session.project.style;
const usingVideo = () => S.session && !!S.session.video_path;

// ---------------------------------------------------------------------------
// Session loading
// ---------------------------------------------------------------------------

async function loadMock() {
  setSession(await invoke("load_mock"));
  toast("Mock session loaded — synthetic screen, real camera math.");
}

function setSession(dto) {
  S.session = dto;
  S.segments = (dto.project.segments || []).map((s) => ({ ...s }));
  S.selected = -1;
  S.rippled.clear();
  S.t = 0;

  if (usingVideo()) {
    els.vid.src = convertFileSrc(dto.video_path);
    els.vid.loop = true;
    els.vid.classList.remove("hidden");
    els.mockscreen.classList.add("hidden");
  } else {
    els.vid.pause();
    els.vid.removeAttribute("src");
    els.vid.classList.add("hidden");
    els.mockscreen.classList.remove("hidden");
  }
  els.mockbadge.classList.toggle("hidden", !dto.mock);

  const m = meta();
  els.sessionInfo.textContent =
    `${m.width}×${m.height} @ ${m.fps} fps\n` +
    `${(m.duration_ms / 1000).toFixed(1)} s · scale ${m.scale_factor}` +
    (dto.video_path ? `\n${dto.video_path}` : "\nmock (no video file)");

  syncSidebar();
  fitCanvas();
  applyStyle();
  renderRuler();
  els.empty.classList.add("hidden");
  solvePlan().then(() => {
    seek(0);
    setPlaying(true);
  });
}

async function solvePlan() {
  if (!S.session) return;
  try {
    S.plan = await invoke("solve", { segments: S.segments, camera: cam(), cursor: cur() });
  } catch (e) {
    toast("solve failed: " + e);
    return;
  }
  renderBlocks();
  renderSegPanel();
}
const debouncedSolve = debounce(solvePlan, 160);

// ---------------------------------------------------------------------------
// Preview: fit, style, per-frame transform
// ---------------------------------------------------------------------------

function fitCanvas() {
  if (!S.session) return;
  const m = meta();
  const box = els.stage.getBoundingClientRect();
  const availW = box.width - 40;
  const availH = box.height - 40 - 44; // transport row
  const ratio = m.width / m.height;
  let w = Math.min(availW, availH * ratio);
  let h = w / ratio;
  els.canvas.style.width = w + "px";
  els.canvas.style.height = h + "px";
  const pad = sty().inset * Math.min(w, h);
  els.frame.style.left = els.frame.style.right = pad + "px";
  els.frame.style.top = els.frame.style.bottom = pad + "px";
}

function applyStyle() {
  if (!S.session) return;
  const st = sty();
  const bg = st.background || "gradient:dusk";
  if (bg.startsWith("gradient:")) {
    els.canvas.style.background = GRADIENTS[bg.slice(9)] || GRADIENTS.dusk;
  } else if (bg.startsWith("color:")) {
    els.canvas.style.background = bg.slice(6);
  }
  els.frame.style.borderRadius = st.corner_radius + "px";
  els.frame.style.boxShadow = st.shadow
    ? "0 24px 70px -12px rgba(0,0,0,0.6), 0 6px 22px rgba(0,0,0,0.35)"
    : "none";
  els.cursor.style.width = ((28 * cur().scale) / meta().width) * 100 + "%";
  fitCanvas();
  document.querySelectorAll(".swatch").forEach((el) =>
    el.classList.toggle("on", el.dataset.bg === bg));
}

function frameIndex(t) {
  const p = S.plan;
  return Math.max(0, Math.min(p.frame_times_ms.length - 1, Math.round((t / 1000) * p.fps)));
}

function applyFrame(t) {
  if (!S.plan) return;
  const m = meta();
  const i = frameIndex(t);
  const c = S.plan.camera[i];
  const k = S.plan.cursor[i];

  const s = m.width / c.w;
  els.screen.style.transform =
    `scale(${s}) translate(${(-c.x / m.width) * 100}%, ${(-c.y / m.height) * 100}%)`;

  els.cursor.style.left = (k.x / m.width) * 100 + "%";
  els.cursor.style.top = (k.y / m.height) * 100 + "%";
  els.cursor.style.opacity = cur().auto_hide && k.speed < 30 ? 0.25 : 1;

  // Click ripples: fire each click once as the playhead passes it.
  for (let ci = 0; ci < S.session.clicks.length; ci++) {
    const [ct, cx, cy] = S.session.clicks[ci];
    if (ct <= t && t - ct < 400 && !S.rippled.has(ci)) {
      S.rippled.add(ci);
      spawnRipple(cx / m.width, cy / m.height);
    }
  }

  const x = (t / dur()) * 100;
  els.playhead.style.left = `calc(16px + ${x}% - ${x / 100 * 32}px)`;
  els.timecode.textContent = `${fmt(t)} / ${fmt(dur())}`;
}

function spawnRipple(fx, fy) {
  const el = document.createElement("div");
  el.className = "ripple";
  el.style.left = fx * 100 + "%";
  el.style.top = fy * 100 + "%";
  els.ripples.appendChild(el);
  setTimeout(() => el.remove(), 600);
}

// ---------------------------------------------------------------------------
// Playback clock
// ---------------------------------------------------------------------------

let lastNow = performance.now();
function tick(now) {
  const dt = now - lastNow;
  lastNow = now;
  if (S.session && S.plan) {
    if (usingVideo()) {
      S.t = els.vid.currentTime * 1000;
    } else if (S.playing) {
      S.t += dt;
      if (S.t >= dur()) { S.t = 0; S.rippled.clear(); }
    }
    applyFrame(S.t);
  }
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);

function setPlaying(on) {
  S.playing = on;
  els.play.innerHTML = on ? "&#10074;&#10074;" : "&#9654;";
  if (usingVideo()) on ? els.vid.play() : els.vid.pause();
}

function seek(t) {
  S.t = Math.max(0, Math.min(dur(), t));
  S.rippled.clear();
  // Pre-mark clicks already behind the playhead so seeking doesn't replay them.
  S.session.clicks.forEach(([ct], i) => { if (ct < S.t - 50) S.rippled.add(i); });
  if (usingVideo()) els.vid.currentTime = S.t / 1000;
  if (S.plan) applyFrame(S.t);
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

function trackRect() { return els.blocks.getBoundingClientRect(); }
function pxToMs(clientX) {
  const r = trackRect();
  return Math.max(0, Math.min(dur(), ((clientX - r.left) / r.width) * dur()));
}

function renderRuler() {
  els.ticks.innerHTML = "";
  els.clickdots.innerHTML = "";
  const d = dur();
  if (!d) return;
  const stepMs = d > 45000 ? 5000 : 1000;
  for (let t = 0; t <= d; t += stepMs) {
    const x = (t / d) * 100;
    const tk = document.createElement("div");
    tk.className = "tick" + (t % (stepMs * 5) === 0 ? " major" : "");
    tk.style.left = x + "%";
    els.ticks.appendChild(tk);
    if (t % (stepMs * (d > 45000 ? 2 : 1)) === 0) {
      const lb = document.createElement("div");
      lb.className = "tick-label";
      lb.style.left = x + "%";
      lb.textContent = t / 1000 + "s";
      els.ticks.appendChild(lb);
    }
  }
  for (const [ct] of S.session.clicks) {
    const dot = document.createElement("div");
    dot.className = "clickdot";
    dot.style.left = (ct / d) * 100 + "%";
    els.clickdots.appendChild(dot);
  }
}

function renderBlocks() {
  els.blocks.innerHTML = "";
  const d = dur();
  S.segments.forEach((seg, i) => {
    const el = document.createElement("div");
    el.className = "block" + (i === S.selected ? " sel" : "");
    el.style.left = (seg.start_ms / d) * 100 + "%";
    el.style.width = (Math.max(150, seg.end_ms - seg.start_ms) / d) * 100 + "%";
    el.textContent = seg.zoom.toFixed(1) + "×";
    el.dataset.i = i;
    for (const side of ["l", "r"]) {
      const h = document.createElement("div");
      h.className = "edge " + side;
      h.dataset.side = side;
      el.appendChild(h);
    }
    el.addEventListener("pointerdown", (e) => beginBlockDrag(e, i));
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      removeSegment(i);
    });
    els.blocks.appendChild(el);
  });
}

function beginBlockDrag(e, i) {
  if (e.button !== 0) return;
  e.stopPropagation();
  const seg = S.segments[i];
  const mode = e.target.dataset?.side || "move";
  const startX = e.clientX;
  const orig = { start: seg.start_ms, end: seg.end_ms };
  const msPerPx = dur() / trackRect().width;
  let moved = false;
  const MIN = 250;

  const onMove = (ev) => {
    const dms = (ev.clientX - startX) * msPerPx;
    if (Math.abs(ev.clientX - startX) > 2) moved = true;
    if (mode === "move") {
      const len = orig.end - orig.start;
      seg.start_ms = Math.max(0, Math.min(dur() - len, orig.start + dms));
      seg.end_ms = seg.start_ms + len;
    } else if (mode === "l") {
      seg.start_ms = Math.max(0, Math.min(orig.end - MIN, orig.start + dms));
    } else {
      seg.end_ms = Math.min(dur(), Math.max(orig.start + MIN, orig.end + dms));
    }
    renderBlocks();
  };
  const onUp = () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    selectSegment(i);
    if (moved) debouncedSolve();
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
}

function selectSegment(i) {
  S.selected = i;
  renderBlocks();
  renderSegPanel();
}

function renderSegPanel() {
  const seg = S.segments[S.selected];
  els.secSegment.classList.toggle("hidden", !seg);
  if (!seg) return;
  els.segInfo.textContent =
    `${fmt(seg.start_ms)} → ${fmt(seg.end_ms)}  ·  center (${Math.round(seg.cx)}, ${Math.round(seg.cy)})`;
  setRange("in-segzoom", seg.zoom, (v) => (+v).toFixed(2) + "×");
}

function addSegmentAt(t) {
  const m = meta();
  let cx = m.width / 2, cy = m.height / 2;
  if (S.plan) {
    const k = S.plan.cursor[frameIndex(t)];
    cx = k.x; cy = k.y;
  }
  S.segments.push({
    start_ms: Math.max(0, t - 400),
    end_ms: Math.min(dur(), t + 1100),
    cx, cy,
    zoom: cam().click_zoom,
  });
  selectSegment(S.segments.length - 1);
  debouncedSolve();
}

function removeSegment(i) {
  S.segments.splice(i, 1);
  if (S.selected === i) S.selected = -1;
  else if (S.selected > i) S.selected--;
  renderBlocks();
  renderSegPanel();
  debouncedSolve();
}

els.blocks.addEventListener("pointerdown", (e) => {
  if (e.target === els.blocks && e.button === 0) addSegmentAt(pxToMs(e.clientX));
});

// Ruler scrubbing.
els.ruler.addEventListener("pointerdown", (e) => {
  const scrub = (ev) => seek(pxToMs(ev.clientX));
  scrub(e);
  const onUp = () => {
    window.removeEventListener("pointermove", scrub);
    window.removeEventListener("pointerup", onUp);
  };
  window.addEventListener("pointermove", scrub);
  window.addEventListener("pointerup", onUp);
});

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

function buildSwatches() {
  const host = $("bg-swatches");
  host.innerHTML = "";
  for (const name of Object.keys(GRADIENTS)) {
    const el = document.createElement("div");
    el.className = "swatch";
    el.style.background = GRADIENTS[name];
    el.dataset.bg = "gradient:" + name;
    el.title = name;
    el.addEventListener("click", () => setBackground("gradient:" + name));
    host.appendChild(el);
  }
}

function setBackground(bg) {
  sty().background = bg;
  applyStyle();
  invoke("set_style", { style: sty() }).catch(() => {});
}

function syncSidebar() {
  const st = sty(), c = cam(), k = cur();
  setRange("in-inset", st.inset, (v) => `${Math.round(v * 100)}%`);
  setRange("in-radius", st.corner_radius, (v) => `${v}px`);
  $("in-shadow").checked = st.shadow;
  $("in-cam-on").checked = c.enabled;
  setRange("in-clickzoom", c.click_zoom, (v) => (+v).toFixed(2) + "×");
  $("in-typing").checked = c.zoom_on_typing;
  setRange("in-panomega", c.pan_omega, (v) => "ω" + v);
  setRange("in-zoomomega", c.zoom_omega, (v) => "ω" + v);
  setRange("in-cscale", k.scale, (v) => (+v).toFixed(1) + "×");
  setRange("in-comega", k.omega, (v) => "ω" + v);
  $("in-chide").checked = k.auto_hide;
}

function setRange(id, value, fmtFn) {
  const el = $(id);
  el.value = value;
  const val = $(id.replace("in-", "val-"));
  if (val) val.textContent = fmtFn(value);
  el._fmt = fmtFn;
}

function bindRange(id, apply) {
  const el = $(id);
  el.addEventListener("input", () => {
    const v = parseFloat(el.value);
    const val = $(id.replace("in-", "val-"));
    if (val && el._fmt) val.textContent = el._fmt(v);
    apply(v);
  });
}

function bindSidebar() {
  buildSwatches();
  $("in-bgcolor").addEventListener("input", (e) => setBackground("color:" + e.target.value));
  bindRange("in-inset", (v) => { sty().inset = v; applyStyle(); pushStyle(); });
  bindRange("in-radius", (v) => { sty().corner_radius = v; applyStyle(); pushStyle(); });
  $("in-shadow").addEventListener("change", (e) => { sty().shadow = e.target.checked; applyStyle(); pushStyle(); });

  $("in-cam-on").addEventListener("change", (e) => { cam().enabled = e.target.checked; debouncedSolve(); });
  bindRange("in-clickzoom", (v) => { cam().click_zoom = v; debouncedSolve(); });
  $("in-typing").addEventListener("change", (e) => { cam().zoom_on_typing = e.target.checked; });
  bindRange("in-panomega", (v) => { cam().pan_omega = v; debouncedSolve(); });
  bindRange("in-zoomomega", (v) => { cam().zoom_omega = v; debouncedSolve(); });
  $("btn-redetect").addEventListener("click", async () => {
    try {
      S.segments = await invoke("regenerate_segments", { camera: cam() });
      S.selected = -1;
      await solvePlan();
      toast(`Re-detected ${S.segments.length} zoom block(s) from the input log.`);
    } catch (e) { toast("" + e); }
  });

  bindRange("in-cscale", (v) => { cur().scale = v; applyStyle(); });
  bindRange("in-comega", (v) => { cur().omega = v; debouncedSolve(); });
  $("in-chide").addEventListener("change", (e) => { cur().auto_hide = e.target.checked; });

  bindRange("in-segzoom", (v) => {
    const seg = S.segments[S.selected];
    if (!seg) return;
    seg.zoom = v;
    renderBlocks();
    debouncedSolve();
  });
  $("btn-segdelete").addEventListener("click", () => {
    if (S.selected >= 0) removeSegment(S.selected);
  });
}

const pushStyle = debounce(() => invoke("set_style", { style: sty() }).catch(() => {}), 300);

// ---------------------------------------------------------------------------
// Top bar: record / save / export
// ---------------------------------------------------------------------------

async function toggleRecord() {
  const btn = $("btn-record");
  if (!S.recording) {
    try {
      await appWindow.minimize();
      await sleep(500); // let the minimize animation clear the screen
      await invoke("start_recording");
      S.recording = true;
      btn.classList.add("on");
      btn.lastChild.textContent = "Stop";
      els.recbadge.classList.remove("hidden");
    } catch (e) {
      await appWindow.unminimize();
      toast("" + e);
    }
  } else {
    try {
      const dto = await invoke("stop_recording");
      S.recording = false;
      btn.classList.remove("on");
      btn.lastChild.textContent = "Record";
      els.recbadge.classList.add("hidden");
      setSession(dto);
      toast(`Recorded ${(dto.project.meta.duration_ms / 1000).toFixed(1)} s → ${dto.video_path}`);
    } catch (e) { toast("" + e); }
  }
}

function bindTopbar() {
  $("btn-mock").addEventListener("click", loadMock);
  $("btn-record").addEventListener("click", toggleRecord);
  $("btn-save").addEventListener("click", async () => {
    try { toast("Saved " + (await invoke("save_project"))); }
    catch (e) { toast("" + e); }
  });
  $("btn-export").addEventListener("click", async () => {
    try { await invoke("export_video"); }
    catch (e) { toast("" + e); }
  });
  els.play.addEventListener("click", () => setPlaying(!S.playing));
}

// ---------------------------------------------------------------------------
// Utilities & init
// ---------------------------------------------------------------------------

function fmt(ms) {
  const s = ms / 1000;
  return `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, "0")}`;
}
function debounce(fn, wait) {
  let h;
  return (...a) => { clearTimeout(h); h = setTimeout(() => fn(...a), wait); };
}
function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
function toast(msg) {
  els.toast.textContent = msg;
  els.toast.classList.remove("hidden");
  clearTimeout(toast._h);
  toast._h = setTimeout(() => els.toast.classList.add("hidden"), 4200);
}

window.addEventListener("keydown", (e) => {
  if (e.code === "Space" && S.session) { e.preventDefault(); setPlaying(!S.playing); }
  if ((e.code === "Delete" || e.code === "Backspace") && S.selected >= 0) removeSegment(S.selected);
});
window.addEventListener("resize", fitCanvas);
new ResizeObserver(fitCanvas).observe(els.stage);

bindTopbar();
bindSidebar();
loadMock().catch((e) => {
  els.empty.querySelector("p").textContent = "Failed to load a session: " + e;
});
