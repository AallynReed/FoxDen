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
const view = { snap: false, grid: 0.05, labels: true, shapes: true, keys: true, zoom: 1 };
const ZOOMS = [0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), Math.max(lo, hi));
const tidy = (v) => Math.round(v * 1e4) / 1e4;
const snapped = (v) => Math.round(v / view.grid) * view.grid;

function setView(change) {
  Object.assign(view, change);
  try {
    localStorage.setItem("foxden.view", JSON.stringify(view));
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

// Device bodies by kind. Each draws inside the device's rect and says how its LEDs look.
const rings = new WeakMap();

// LEDs spread evenly round a centre, as on a fan.
function isRing(leds) {
  if (rings.has(leds)) return rings.get(leds);
  let ring = false;
  if (leds.length >= 8) {
    const cx = leds.reduce((s, l) => s + l[0], 0) / leds.length;
    const cy = leds.reduce((s, l) => s + l[1], 0) / leds.length;
    const d = leds.map((l) => Math.hypot(l[0] - cx, l[1] - cy));
    const mean = d.reduce((s, v) => s + v, 0) / d.length;
    const spread = Math.sqrt(d.reduce((s, v) => s + (v - mean) ** 2, 0) / d.length);
    ring = mean > 0.2 && spread / mean < 0.15;
  }
  rings.set(leds, ring);
  return ring;
}

function shapeOf(device, w, h) {
  if (isRing(device.leds)) return "fan";
  const long = Math.max(w, h) / Math.max(1, Math.min(w, h));
  switch (device.kind) {
    case "keyboard":
      return "keyboard";
    case "mouse":
      return "mouse";
    case "memory":
      return "memory";
    case "gpu":
      return long > 16 ? "strip" : "gpu";
    case "motherboard":
      return long > 3 ? "strip" : "board";
    case "strip":
      return "strip";
    default:
      return long > 6 ? "strip" : "plate";
  }
}

// The app's cut corner: the top right is clipped.
function cutPath(ctx, x, y, w, h, cut) {
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(x + w - cut, y);
  ctx.lineTo(x + w, y + cut);
  ctx.lineTo(x + w, y + h);
  ctx.lineTo(x, y + h);
  ctx.closePath();
}

function mousePath(ctx, x, y, w, h) {
  ctx.beginPath();
  ctx.moveTo(x, y + h * 0.42);
  ctx.bezierCurveTo(x, y + h * 0.08, x + w * 0.2, y, x + w / 2, y);
  ctx.bezierCurveTo(x + w * 0.8, y, x + w, y + h * 0.08, x + w, y + h * 0.42);
  ctx.lineTo(x + w * 0.96, y + h * 0.78);
  ctx.bezierCurveTo(x + w * 0.92, y + h, x + w * 0.08, y + h, x + w * 0.04, y + h * 0.78);
  ctx.closePath();
}

function line(ctx, x1, y1, x2, y2) {
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
}

// Fills and outlines the body, draws its details, and leaves the outline path for clipping.
function drawBody(ctx, shape, x, y, w, h, edge) {
  const s = Math.min(w, h);
  ctx.fillStyle = PAINT.plate;
  ctx.strokeStyle = PAINT.line;
  switch (shape) {
    case "mouse": {
      mousePath(ctx, x, y, w, h);
      ctx.fill();
      line(ctx, x + w / 2, y + 1, x + w / 2, y + h * 0.4);
      line(ctx, x + w * 0.02, y + h * 0.4, x + w * 0.98, y + h * 0.4);
      ctx.fillStyle = PAINT.line;
      ctx.fillRect(x + w / 2 - w * 0.07, y + h * 0.1, w * 0.14, h * 0.2);
      mousePath(ctx, x, y, w, h);
      break;
    }
    case "memory": {
      const tall = h > w;
      cutPath(ctx, x, y, w, h, Math.min(6, s * 0.3));
      ctx.fill();
      ctx.fillStyle = PAINT.stroke;
      // Gold fingers on the edge away from the light bar, broken by the key notch.
      if (tall) {
        const fw = Math.max(2, w * 0.14);
        ctx.fillRect(x + w - fw, y + h * 0.06, fw, h * 0.4);
        ctx.fillRect(x + w - fw, y + h * 0.52, fw, h * 0.42);
      } else {
        const fh = Math.max(2, h * 0.14);
        ctx.fillRect(x + w * 0.06, y + h - fh, w * 0.4, fh);
        ctx.fillRect(x + w * 0.52, y + h - fh, w * 0.42, fh);
      }
      cutPath(ctx, x, y, w, h, Math.min(6, s * 0.3));
      break;
    }
    case "gpu": {
      cutPath(ctx, x, y, w, h, Math.min(8, s * 0.25));
      ctx.fill();
      // The bracket at the left and the PCIe fingers along the bottom.
      ctx.fillStyle = PAINT.line;
      ctx.fillRect(x, y, Math.max(2, w * 0.03), h);
      ctx.fillStyle = PAINT.stroke;
      ctx.fillRect(x + w * 0.12, y + h - Math.max(2, h * 0.08), w * 0.3, Math.max(2, h * 0.08));
      const fans = w / h > 3.2 ? 3 : 2;
      const r = Math.min(h * 0.3, (w * 0.8) / fans / 2.3);
      for (let i = 0; i < fans; i++) {
        const cx = x + w * 0.1 + ((i + 0.5) * w * 0.86) / fans;
        const cy = y + h * 0.6;
        ctx.beginPath();
        ctx.arc(cx, cy, r, 0, Math.PI * 2);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(cx, cy, r * 0.25, 0, Math.PI * 2);
        ctx.stroke();
      }
      cutPath(ctx, x, y, w, h, Math.min(8, s * 0.25));
      break;
    }
    case "board": {
      cutPath(ctx, x, y, w, h, Math.min(10, s * 0.08));
      ctx.fill();
      ctx.strokeRect(x + w * 0.38, y + h * 0.12, w * 0.22, h * 0.24);
      for (let i = 0; i < 4; i++) ctx.strokeRect(x + w * (0.68 + i * 0.06), y + h * 0.08, w * 0.03, h * 0.42);
      for (let i = 0; i < 2; i++) ctx.strokeRect(x + w * 0.08, y + h * (0.56 + i * 0.18), w * 0.6, h * 0.05);
      ctx.strokeRect(x + w * 0.04, y + h * 0.06, w * 0.12, h * 0.36);
      cutPath(ctx, x, y, w, h, Math.min(10, s * 0.08));
      break;
    }
    case "fan": {
      const cx = x + w / 2;
      const cy = y + h / 2;
      const r = s / 2;
      ctx.fillRect(cx - r, cy - r, r * 2, r * 2);
      ctx.beginPath();
      ctx.arc(cx, cy, r * 0.78, 0, Math.PI * 2);
      ctx.stroke();
      for (let i = 0; i < 7; i++) {
        const a = (i / 7) * Math.PI * 2;
        ctx.beginPath();
        ctx.moveTo(cx + Math.cos(a) * r * 0.22, cy + Math.sin(a) * r * 0.22);
        ctx.quadraticCurveTo(cx + Math.cos(a + 0.5) * r * 0.55, cy + Math.sin(a + 0.5) * r * 0.55, cx + Math.cos(a + 0.9) * r * 0.72, cy + Math.sin(a + 0.9) * r * 0.72);
        ctx.stroke();
      }
      ctx.beginPath();
      ctx.arc(cx, cy, r * 0.22, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      ctx.beginPath();
      ctx.rect(cx - r, cy - r, r * 2, r * 2);
      break;
    }
    case "keyboard":
      cutPath(ctx, x, y, w, h, Math.min(10, s * 0.08));
      ctx.fill();
      cutPath(ctx, x, y, w, h, Math.min(10, s * 0.08));
      break;
    case "strip": {
      const c = Math.min(4, s * 0.4);
      cutPath(ctx, x, y, w, h, c);
      ctx.fill();
      cutPath(ctx, x, y, w, h, c);
      break;
    }
    default:
      ctx.beginPath();
      ctx.rect(x, y, w, h);
      ctx.fill();
      ctx.beginPath();
      ctx.rect(x, y, w, h);
  }
  ctx.strokeStyle = edge;
  ctx.stroke();
}

const css = (c) => (c && c[0] + c[1] + c[2] > 24 ? "rgb(" + c[0] + "," + c[1] + "," + c[2] + ")" : PAINT.unlit);

// LEDs in the look of the body: keycaps, a light bar, a ring, dots or plain cells.
function drawLeds(ctx, shape, device, colors, x, y, w, h) {
  const leds = device.leds;
  const [cols, rows] = device.grid;
  const cw = w / Math.max(1, cols);
  const ch = h / Math.max(1, rows);
  const at = (i) => [x + leds[i][0] * w, y + leds[i][1] * h];
  if (shape === "memory" || (shape === "gpu" && rows === 1)) {
    // One light bar, split into the stick's or card's LEDs.
    const tall = h > w;
    const n = leds.length;
    for (let i = 0; i < n; i++) {
      ctx.fillStyle = css(colors && colors[i]);
      if (shape === "memory" && tall) ctx.fillRect(x + w * 0.1, y + h * 0.04 + (i * h * 0.92) / n, w * 0.45, (h * 0.92) / n + 0.5);
      else if (shape === "memory") ctx.fillRect(x + w * 0.04 + (i * w * 0.92) / n, y + h * 0.1, (w * 0.92) / n + 0.5, h * 0.45);
      else ctx.fillRect(x + w * 0.06 + (i * w * 0.9) / n, y + h * 0.08, (w * 0.9) / n + 0.5, Math.max(2, h * 0.14));
    }
    return;
  }
  if (shape === "fan") {
    const cx = x + w / 2;
    const cy = y + h / 2;
    const r = Math.min(w, h) / 2;
    const n = leds.length;
    ctx.lineWidth = Math.max(2, r * 0.16);
    for (let i = 0; i < n; i++) {
      const [lx, ly] = at(i);
      const a = Math.atan2(ly - cy, lx - cx);
      ctx.strokeStyle = css(colors && colors[i]);
      ctx.beginPath();
      ctx.arc(cx, cy, r * 0.88, a - Math.PI / n - 0.01, a + Math.PI / n + 0.01);
      ctx.stroke();
    }
    ctx.lineWidth = 1;
    return;
  }
  if (shape === "mouse" || shape === "strip" || shape === "board") {
    const r = shape === "mouse" ? Math.min(w, h) * 0.16 : Math.max(1, Math.min(cw, ch, shape === "board" ? Math.min(w, h) * 0.08 : Infinity) * 0.36);
    for (let i = 0; i < leds.length; i++) {
      const [lx, ly] = at(i);
      ctx.fillStyle = css(colors && colors[i]);
      ctx.beginPath();
      ctx.arc(lx, ly, r, 0, Math.PI * 2);
      ctx.fill();
    }
    return;
  }
  // Keycaps and plain cells: a square per LED, keycaps with a darker rim.
  const gap = shape === "keyboard" ? Math.max(1, Math.min(cw, ch) * 0.12) : Math.min(1.5, Math.min(cw, ch) * 0.12);
  const lw = Math.max(1, cw - gap * 2);
  const lh = Math.max(1, ch - gap * 2);
  let last = "";
  for (let i = 0; i < leds.length; i++) {
    const c = css(colors && colors[i]);
    if (c !== last) ctx.fillStyle = last = c;
    const [lx, ly] = at(i);
    ctx.fillRect(lx - lw / 2, ly - lh / 2, lw, lh);
  }
  if (shape === "keyboard" && lw > 6) {
    ctx.strokeStyle = PAINT.plate;
    for (let i = 0; i < leds.length; i++) {
      const [lx, ly] = at(i);
      ctx.strokeRect(lx - lw / 2 + 1.5, ly - lh / 2 + 1.5, lw - 3, lh - 3);
    }
  }
}

function drawDevice(ctx, device) {
  const [x, y, w, h] = toPx(rectOf(device));
  const selected = device.id === preview.selected;
  const hovered = device.id === preview.hover;
  const colors = preview.frames.get(device.id);
  const shape = view.shapes ? shapeOf(device, w, h) : "plate";
  const edge = selected ? PAINT.accent : hovered ? PAINT.text : PAINT.stroke;

  ctx.globalAlpha = device.enabled ? 1 : 0.35;
  ctx.save();
  drawBody(ctx, shape, x + 0.5, y + 0.5, w - 1, h - 1, edge);
  if (shape === "mouse") ctx.clip();
  drawLeds(ctx, shape, device, colors, x, y, w, h);
  ctx.restore();
  ctx.globalAlpha = 1;

  if (selected) {
    ctx.strokeStyle = PAINT.accent;
    ctx.strokeRect(x - 2.5, y - 2.5, w + 5, h + 5);
  }
  const label = device.name + (device.enabled ? "" : " (off)");
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
    const saved = JSON.parse(localStorage.getItem("foxden.view"));
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
