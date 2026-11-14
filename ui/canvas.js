const PAD = 20;
const MIN_SIZE = 0.02;
const ARROWS = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
const PAINT = {};

const preview = {
  canvas: null,
  ctx: null,
  w: 0,
  h: 0,
  frames: new Map(),
  widths: new Map(),
  selected: null,
  hover: null,
  drag: null,
  dirty: true,
};

// View options. Kept in the browser's storage, not in the app's settings.
const view = { snap: false, grid: 0.05, labels: true, keys: true, zoom: 1 };
const ZOOMS = [0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), Math.max(lo, hi));
const tidy = (v) => Math.round(v * 1e4) / 1e4;
const snapped = (v) => Math.round(v / view.grid) * view.grid;

function setView(change) {
  Object.assign(view, change);
  try {
    localStorage.setItem("foxrgb.view", JSON.stringify(view));
  } catch {}
  if ("zoom" in change) applyZoom();
  preview.dirty = true;
  scheduleRender();
}

// The web view scales everything. In a plain browser the browser's own zoom does that.
function applyZoom() {
  const api = window.__TAURI__.webview;
  if (api) api.getCurrentWebview().setZoom(view.zoom).catch((e) => showError(String(e)));
}

// One step larger or smaller, or back to 100% with 0.
function zoomBy(steps) {
  const at = ZOOMS.indexOf(view.zoom);
  const zoom = steps ? ZOOMS[clamp(at + steps, 0, ZOOMS.length - 1)] : 1;
  setView({ zoom });
  note("Zoom " + Math.round(zoom * 100) + "%");
}

// The canvas takes its colours from the stylesheet, so it follows the theme.
function loadPaint() {
  const css = getComputedStyle(document.documentElement);
  for (const name of ["line", "stroke", "text", "muted", "accent", "plate", "unlit"]) {
    PAINT[name] = css.getPropertyValue("--" + name).trim();
  }
  preview.dirty = true;
}

function findDevice(id) {
  return state.snap ? state.snap.devices.find((d) => d.id === id) : undefined;
}

// Devices from the bottom up. The selected one is on top, so its outline and handle stay in view.
function stack() {
  const devices = state.snap ? state.snap.devices : [];
  return [...devices].sort((a, b) => (a.id === preview.selected) - (b.id === preview.selected));
}

// A device being dragged is shown at the drag rect until the next snapshot.
function rectOf(device) {
  const drag = preview.drag;
  return drag && drag.id === device.id ? drag.rect : device.rect;
}

function toPx([x, y, w, h]) {
  const cw = preview.w - PAD * 2;
  const ch = preview.h - PAD * 2;
  return [Math.round(PAD + x * cw), Math.round(PAD + y * ch), Math.max(2, Math.round(w * cw)), Math.max(2, Math.round(h * ch))];
}

function labelWidth(ctx, text) {
  let width = preview.widths.get(text);
  if (width === undefined) preview.widths.set(text, (width = ctx.measureText(text).width));
  return width;
}

function drawGrid(ctx) {
  const cw = preview.w - PAD * 2;
  const ch = preview.h - PAD * 2;
  ctx.beginPath();
  for (let i = 1; i * view.grid < 0.9999; i++) {
    const x = Math.round(PAD + i * view.grid * cw) - 0.5;
    const y = Math.round(PAD + i * view.grid * ch) - 0.5;
    ctx.moveTo(x, PAD);
    ctx.lineTo(x, PAD + ch);
    ctx.moveTo(PAD, y);
    ctx.lineTo(PAD + cw, y);
  }
  ctx.stroke();
}

function drawDevice(ctx, device) {
  const [x, y, w, h] = toPx(rectOf(device));
  const selected = device.id === preview.selected;
  const hovered = device.id === preview.hover;
  const colors = preview.frames.get(device.id);
  const cw = w / Math.max(1, device.grid[0]);
  const ch = h / Math.max(1, device.grid[1]);
  const gap = Math.min(1.5, Math.min(cw, ch) * 0.12);
  const lw = Math.max(1, cw - gap * 2);
  const lh = Math.max(1, ch - gap * 2);

  ctx.globalAlpha = device.enabled ? 1 : 0.35;
  ctx.fillStyle = PAINT.plate;
  ctx.fillRect(x, y, w, h);
  let last = "";
  for (let i = 0; i < device.leds.length; i++) {
    const c = colors && colors[i];
    const css = c && c[0] + c[1] + c[2] > 24 ? "rgb(" + c[0] + "," + c[1] + "," + c[2] + ")" : PAINT.unlit;
    if (css !== last) ctx.fillStyle = last = css;
    const led = device.leds[i];
    ctx.fillRect(x + led[0] * w - lw / 2, y + led[1] * h - lh / 2, lw, lh);
  }
  ctx.globalAlpha = 1;

  // A simulated device has a broken outline, and says so in its label where that fits.
  ctx.strokeStyle = selected ? PAINT.accent : hovered ? PAINT.text : PAINT.stroke;
  ctx.setLineDash(simulated(device) ? [4, 3] : []);
  ctx.strokeRect(x - 0.5, y - 0.5, w + 1, h + 1);
  ctx.setLineDash([]);
  const off = device.enabled ? "" : " (off)";
  const full = simulated(device) ? device.name + (device.enabled ? " (simulated)" : " (simulated, off)") : device.name + off;
  const label = selected || hovered || labelWidth(ctx, full) <= w ? full : device.name + off;
  if (selected || hovered || (view.labels && labelWidth(ctx, label) <= w)) {
    ctx.fillStyle = selected || hovered ? PAINT.text : PAINT.muted;
    ctx.fillText(label, x, y - 5);
  }
  if (selected) {
    ctx.fillStyle = PAINT.accent;
    ctx.fillRect(x + w - 4, y + h - 4, 9, 9);
  }
}

function drawPreview() {
  requestAnimationFrame(drawPreview);
  const snap = state.snap;
  if (!preview.dirty || !snap || !preview.w) return;
  preview.dirty = false;
  const { ctx, w, h } = preview;
  ctx.clearRect(0, 0, w, h);
  ctx.lineWidth = 1;
  ctx.font = "12px 'Segoe UI', system-ui, sans-serif";
  ctx.strokeStyle = PAINT.line;
  ctx.strokeRect(PAD - 0.5, PAD - 0.5, w - PAD * 2 + 1, h - PAD * 2 + 1);
  if (view.snap) drawGrid(ctx);
  for (const device of stack()) drawDevice(ctx, device);
}

function pointer(e) {
  const box = preview.canvas.getBoundingClientRect();
  return [e.clientX - box.left, e.clientY - box.top];
}

function hitTest(px, py) {
  const devices = stack();
  const selected = findDevice(preview.selected);
  if (selected) {
    const [x, y, w, h] = toPx(rectOf(selected));
    if (Math.abs(px - x - w) <= 8 && Math.abs(py - y - h) <= 8) return { device: selected, mode: "resize" };
  }
  for (let i = devices.length - 1; i >= 0; i--) {
    const [x, y, w, h] = toPx(rectOf(devices[i]));
    if (px >= x - 3 && px <= x + w + 3 && py >= y - 3 && py <= y + h + 3) return { device: devices[i], mode: "move" };
  }
  return null;
}

function selectDevice(id) {
  if (id === preview.selected) return;
  preview.selected = id;
  preview.dirty = true;
  scheduleRender();
}

function sendRect(device, rect) {
  return call("set_device", { id: device.id, enabled: device.enabled, brightness: device.brightness, rect: rect.map(tidy) });
}

function onPointerDown(e) {
  if (e.button !== 0) return;
  const [px, py] = pointer(e);
  const hit = hitTest(px, py);
  selectDevice(hit ? hit.device.id : null);
  if (!hit) return;
  preview.canvas.setPointerCapture(e.pointerId);
  const rect = rectOf(hit.device).slice();
  preview.drag = { id: hit.device.id, mode: hit.mode, from: [px, py], origin: rect, rect, moved: false, done: false };
}

function onPointerMove(e) {
  const [px, py] = pointer(e);
  const drag = preview.drag;
  if (!drag || drag.done) {
    const hit = hitTest(px, py);
    const hover = hit ? hit.device.id : null;
    if (hover !== preview.hover) {
      preview.hover = hover;
      preview.dirty = true;
    }
    preview.canvas.style.cursor = !hit ? "default" : hit.mode === "resize" ? "nwse-resize" : "move";
    return;
  }
  if (!drag.moved && Math.hypot(px - drag.from[0], py - drag.from[1]) < 3) return;
  const dx = (px - drag.from[0]) / (preview.w - PAD * 2);
  const dy = (py - drag.from[1]) / (preview.h - PAD * 2);
  const [x, y, w, h] = drag.origin;
  const fit = view.snap && !e.altKey ? snapped : (v) => v;
  drag.rect =
    drag.mode === "move"
      ? [clamp(fit(x + dx), 0, 1 - w), clamp(fit(y + dy), 0, 1 - h), w, h]
      : [x, y, clamp(fit(x + w + dx) - x, MIN_SIZE, 1 - x), clamp(fit(y + h + dy) - y, MIN_SIZE, 1 - y)];
  drag.moved = true;
  preview.dirty = true;
  scheduleRender();
}

function onPointerUp() {
  const drag = preview.drag;
  if (!drag || drag.done) return;
  const device = findDevice(drag.id);
  if (!drag.moved || !device) {
    preview.drag = null;
    preview.dirty = true;
    return;
  }
  drag.done = true;
  sendRect(device, drag.rect).then((out) => {
    if (out === undefined && preview.drag === drag) {
      preview.drag = null;
      preview.dirty = true;
    }
  });
}

// Arrows move by 1% of the canvas, with Ctrl by 5%. Shift resizes instead.
function onKeyDown(e) {
  const device = findDevice(preview.selected);
  const dir = ARROWS[e.key];
  if (!device || !dir || e.altKey || e.metaKey) return;
  e.preventDefault();
  const [x, y, w, h] = device.rect;
  const step = e.ctrlKey ? 0.05 : 0.01;
  const dx = dir[0] * step;
  const dy = dir[1] * step;
  sendRect(
    device,
    e.shiftKey
      ? [x, y, clamp(w + dx, MIN_SIZE, 1 - x), clamp(h + dy, MIN_SIZE, 1 - y)]
      : [clamp(x + dx, 0, 1 - w), clamp(y + dy, 0, 1 - h), w, h],
  );
}

function previewFrames(frames) {
  preview.frames = new Map(frames.map((f) => [f.device, f.colors]));
  preview.dirty = true;
}

function previewSnapshot() {
  if (preview.drag && preview.drag.done) preview.drag = null;
  if (preview.selected && !findDevice(preview.selected)) preview.selected = null;
  preview.dirty = true;
}

function previewInit(canvas) {
  preview.canvas = canvas;
  preview.ctx = canvas.getContext("2d");
  try {
    const saved = JSON.parse(localStorage.getItem("foxrgb.view"));
    if (saved && saved.grid > 0) Object.assign(view, saved);
  } catch {}
  if (!ZOOMS.includes(view.zoom)) view.zoom = 1;
  if (view.zoom !== 1) applyZoom();
  loadPaint();
  matchMedia("(prefers-color-scheme: light)").addEventListener("change", loadPaint);
  const resized = new ResizeObserver(() => {
    const dpr = window.devicePixelRatio || 1;
    const box = canvas.getBoundingClientRect();
    preview.w = box.width;
    preview.h = box.height;
    canvas.width = Math.round(box.width * dpr);
    canvas.height = Math.round(box.height * dpr);
    preview.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    preview.dirty = true;
  });
  // The device-pixel box also changes when the window moves to a display with another scale.
  try {
    resized.observe(canvas, { box: "device-pixel-content-box" });
  } catch {
    resized.observe(canvas);
  }
  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("pointercancel", onPointerUp);
  canvas.addEventListener("pointerleave", () => {
    preview.hover = null;
    preview.dirty = true;
  });
  canvas.addEventListener("keydown", onKeyDown);
  requestAnimationFrame(drawPreview);
}
