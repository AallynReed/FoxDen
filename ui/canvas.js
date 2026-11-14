const PAD = 20;
const MIN_SIZE = 0.02;
const PAINT = { line: "#36373c", plate: "#0f1011", off: "#232428", text: "#e7e7ea", muted: "#9d9ea6", accent: "#e8822e" };

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

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), Math.max(lo, hi));

function findDevice(id) {
  return state.snap ? state.snap.devices.find((d) => d.id === id) : undefined;
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

function drawDevice(ctx, device) {
  const [x, y, w, h] = toPx(rectOf(device));
  const selected = device.id === preview.selected;
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
    const css = c && c[0] + c[1] + c[2] > 24 ? "rgb(" + c[0] + "," + c[1] + "," + c[2] + ")" : PAINT.off;
    if (css !== last) ctx.fillStyle = last = css;
    const led = device.leds[i];
    ctx.fillRect(x + led[0] * w - lw / 2, y + led[1] * h - lh / 2, lw, lh);
  }
  ctx.globalAlpha = 1;

  ctx.strokeStyle = selected ? PAINT.accent : PAINT.line;
  ctx.strokeRect(x - 0.5, y - 0.5, w + 1, h + 1);
  const label = device.enabled ? device.name : device.name + " (off)";
  if (selected || device.id === preview.hover || labelWidth(ctx, label) <= w) {
    ctx.fillStyle = selected ? PAINT.text : PAINT.muted;
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
  for (const device of snap.devices) drawDevice(ctx, device);
}

function pointer(e) {
  const box = preview.canvas.getBoundingClientRect();
  return [e.clientX - box.left, e.clientY - box.top];
}

function hitTest(px, py) {
  const devices = state.snap ? state.snap.devices : [];
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
  return call("set_device", { id: device.id, enabled: device.enabled, brightness: device.brightness, rect });
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
  drag.rect =
    drag.mode === "move"
      ? [clamp(x + dx, 0, 1 - w), clamp(y + dy, 0, 1 - h), w, h]
      : [x, y, clamp(w + dx, MIN_SIZE, 1 - x), clamp(h + dy, MIN_SIZE, 1 - y)];
  drag.moved = true;
  preview.dirty = true;
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

function onKeyDown(e) {
  const device = findDevice(preview.selected);
  const step = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
  if (!device || !step) return;
  e.preventDefault();
  const [x, y, w, h] = device.rect;
  const dx = step[0] * 0.01;
  const dy = step[1] * 0.01;
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
