const BACKENDS = { openrgb: "OpenRGB", hid: "Native HID", board: "Boards, memory and cards", net: "Network lights", audio: "Audio capture", screen: "Screen capture" };
const KINDS = { gpu: "Graphics card", strip: "Light strip" };
const BUSES = { usb: "USB", network: "Network" };
// How a detected device could be driven: the name of the route and the setting that switches it on.
const ROUTES = { native: ["Native driver", "set-native"], openrgb: ["OpenRGB", "set-openrgb"] };
const STATES = { active: ["on", "Active"], available: ["off", "Available"], unsupported: ["none", "Unsupported"] };
const PRESETS = [
  ["Red", "#FF0000"],
  ["Orange", "#FF6A00"],
  ["Yellow", "#FFD000"],
  ["Green", "#00C850"],
  ["Cyan", "#00C8FF"],
  ["Blue", "#0050FF"],
  ["Violet", "#963CFF"],
  ["Pink", "#FF3CA0"],
  ["White", "#FFFFFF"],
  ["Black", "#000000"],
];

const lastUsed = {};
const recent = [];
try {
  recent.push(...JSON.parse(localStorage.getItem("foxrgb.colours")).filter((c) => /^#[0-9A-F]{6}$/.test(c)).slice(0, 8));
} catch {}
let picker = null;
let held = null;
let pressed = 0;
const pending = { effect: null, layers: null };
const lastLayer = {};
// The device whose background is being edited, or "" for the main one.
let editDevice = "";
// The selected row of the layer list: an index into the layers, or -1 for the background.
let layerSel = -1;
let dragFrom = -1;
let layerFocus = "";
let rowCount = 0;
let asking = null;
let refocus = "";

document.addEventListener(
  "pointerdown",
  (e) => {
    held = e.target;
    pressed = performance.now();
  },
  true,
);
document.addEventListener("pointerup", () => (held = null), true);
document.addEventListener("pointercancel", () => (held = null), true);
document.addEventListener("focusout", () => scheduleRender());

const toHex = (c) => ("#" + c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")).toUpperCase();
const plural = (n, word) => n.toLocaleString("en-GB") + " " + word + (n === 1 ? "" : "s");
const sentence = (s) => s.charAt(0).toUpperCase() + s.slice(1);
const words = (s) => sentence(s).replace(/_/g, " ");

function el(tag, props, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

// A state is shown as a shape with its name beside it.
function setState(node, mark, text) {
  if (node.dataset.state === mark + text) return;
  node.dataset.state = mark + text;
  node.replaceChildren(el("span", { className: "mark " + mark }), text);
}

function read(input) {
  if (input.type === "checkbox") return input.checked;
  if (input.type === "range" || input.type === "number") return Number(input.value);
  return input.value;
}

// A control the user is dragging or typing in keeps its own value.
function busy(input) {
  return input === held || (input === document.activeElement && /^(text|number)$/.test(input.type));
}

function put(input, value) {
  if (value === undefined || busy(input) || input.hasAttribute("aria-invalid")) return;
  if (input.type === "checkbox") input.checked = Boolean(value);
  else if (input.type !== "button") input.value = value;
  if (input.show) input.show(value);
}

// Runs fn once per frame with the newest arguments, read when the control changed.
function throttled(fn) {
  let args = null;
  return (...latest) => {
    const idle = !args;
    args = latest;
    if (!idle) return;
    requestAnimationFrame(() => {
      const now = args;
      args = null;
      fn(...now);
    });
  };
}

// Marks a field as wrong and says why on its line: its own, or one it shares with its neighbours.
function flag(input, problem) {
  if (problem) input.setAttribute("aria-invalid", "true");
  else input.removeAttribute("aria-invalid");
  input.problem = problem;
  const line = input.line || $(input.id + "-problem");
  if (!line) {
    if (problem) note(problem);
    return;
  }
  // Fields of an effect that is no longer shown drop out here.
  line.fields = new Set([...(line.fields || []), input].filter((field) => field.isConnected));
  const first = [...line.fields].find((field) => field.problem);
  setText(line, first ? first.problem : "");
  line.hidden = !first;
}

// Gives a field the line that explains a value it cannot use.
function explain(input, line) {
  input.line = line;
  input.setAttribute("aria-describedby", line.id);
}

// Such a value stays in the field until it is put right. Escape puts the last good one back.
function revert(input, restore) {
  input.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !input.hasAttribute("aria-invalid")) return;
    e.preventDefault();
    e.stopPropagation();
    flag(input, "");
    restore();
  });
}

// Ties a slider to the number field beside it. A scale of 100 shows 0 to 1 as a percentage.
function pair(range, box, scale, send, line) {
  const digits = (String(range.step * scale).split(".")[1] || "").length;
  const what = box.getAttribute("aria-label").replace(" in per cent", "");
  const shown = (value) => (value * scale).toFixed(digits);
  Object.assign(box, { min: range.min * scale, max: range.max * scale, step: range.step * scale });
  if (line) explain(box, line);
  range.show = (value = Number(range.value)) => {
    if (box !== document.activeElement && !box.hasAttribute("aria-invalid")) box.value = shown(value);
  };
  range.oninput = () => {
    flag(box, "");
    range.show();
    send(Number(range.value));
  };
  box.oninput = () => {
    const value = Number(Number(box.value).toFixed(digits));
    const ok = box.value !== "" && value >= box.min && value <= box.max;
    flag(box, ok ? "" : `${what} must be from ${box.min} to ${box.max}${scale === 100 ? "%" : ""}.`);
    if (!ok) return;
    range.value = value / scale;
    send(value / scale);
  };
  revert(box, () => (box.value = shown(Number(range.value))));
}

function toHsv([r, g, b]) {
  const max = Math.max(r, g, b);
  const span = max - Math.min(r, g, b);
  let h = 0;
  if (span) h = max === r ? (g - b) / span : max === g ? (b - r) / span + 2 : (r - g) / span + 4;
  return [(h * 60 + 360) % 360, max ? (span / max) * 100 : 0, max / 2.55];
}

function fromHsv([h, s, v]) {
  const part = (n) => {
    const k = (n + h / 60) % 6;
    return Math.round(v * 2.55 * (1 - (s / 100) * Math.max(0, Math.min(k, 4 - k, 1))));
  };
  return [part(5), part(3), part(1)];
}

// Three or six hex digits as a colour, or null.
function fromHex(text) {
  const digits = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text.trim());
  if (!digits) return null;
  const six = digits[1].length === 3 ? digits[1].replace(/./g, "$&$&") : digits[1];
  return [0, 2, 4].map((i) => parseInt(six.slice(i, i + 2), 16));
}

// A slider track of flat blocks, one for each colour.
function blocks(colours) {
  const stop = (c, i) => `rgb(${c}) ${(i / colours.length) * 100}% ${((i + 1) / colours.length) * 100}%`;
  return "linear-gradient(to right, " + colours.map(stop).join(", ") + ")";
}

// Colours picked earlier, newest first. Kept by the web view like the view options.
function remember(rgb) {
  const hex = toHex(rgb);
  if (PRESETS.some(([, preset]) => preset === hex)) return;
  recent.splice(0, recent.length, hex, ...recent.filter((c) => c !== hex).slice(0, 7));
  try {
    localStorage.setItem("foxrgb.colours", JSON.stringify(recent));
  } catch {}
}

// A colour field: a swatch that opens sliders, a hex field and ready-made colours below its row.
function colourField(param, id, target) {
  const chip = el("span", { className: "chip" });
  const code = el("span", { id: id + "-code" });
  const button = el("button", { type: "button", className: "swatch", id }, chip, code);
  const problem = el("p", { className: "problem", id: id + "-problem", hidden: true });
  const hex = el("input", { type: "text", className: "hex", id: id + "-hex", maxLength: 7, autocomplete: "off", spellcheck: false });
  const picks = el("div", { className: "swatches" });
  const panel = el("div", { className: "picker", id: id + "-picker", hidden: true });
  let rgb = null;
  let hsv = [0, 0, 0];
  let before = null;
  let sentAt = 0;
  let timer = 0;

  // A colour chosen here. The app is told, and its echo is not taken as news for a moment.
  const choose = (next, keep) => {
    rgb = next;
    if (!keep) hsv = toHsv(rgb);
    sentAt = performance.now();
    paint();
    queueEffect(param.name, rgb, target);
    clearTimeout(timer);
    timer = setTimeout(scheduleRender, 350);
  };

  const sliders = [["Hue", 360], ["Saturation", 100], ["Brightness", 100]].map(([name, max], n) => {
    const range = el("input", { type: "range", min: 0, max, step: 1, id: id + "-" + n });
    const box = el("input", { type: "number", className: "exact" });
    box.setAttribute("aria-label", name);
    const set = (value) => {
      hsv[n] = value;
      choose(fromHsv(hsv), true);
    };
    pair(range, box, 1, set, problem);
    panel.append(el("div", { className: "field range" }, el("label", { htmlFor: range.id, textContent: name }), range, box));
    return range;
  });

  function paint() {
    const text = toHex(rgb);
    const [h, s, v] = hsv;
    const steps = (count, at) => blocks(Array.from({ length: count }, (_, i) => fromHsv(at(((i + 0.5) / count) * 100))));
    chip.style.background = text;
    setText(code, text);
    if (hex !== document.activeElement && !hex.hasAttribute("aria-invalid")) hex.value = text;
    sliders.forEach((range, n) => {
      range.value = Math.round(hsv[n]);
      range.show();
    });
    sliders[0].style.setProperty("--track", steps(12, (k) => [k * 3.6, 100, 100]));
    sliders[1].style.setProperty("--track", steps(8, (k) => [h, k, v]));
    sliders[2].style.setProperty("--track", steps(8, (k) => [h, s, k]));
    for (const radio of picks.querySelectorAll("input")) radio.checked = radio.value === text;
  }

  function fill() {
    const pick = ([name, colour]) => {
      const radio = el("input", { type: "radio", name: id + "-pick", value: colour });
      const square = el("span", { className: "chip" });
      radio.setAttribute("aria-label", name);
      radio.onchange = () => choose(fromHex(colour));
      square.style.background = colour;
      return el("label", { className: "tile", title: name }, radio, square);
    };
    const row = (title, colours) => [el("span", { className: "meta", textContent: title }), el("div", { className: "tiles" }, ...colours.map(pick))];
    picks.replaceChildren(...row("Presets", PRESETS), ...(recent.length ? row("Recent", recent.map((c) => [c, c])) : []));
  }

  function open(on) {
    if (on === !panel.hidden || !rgb) return;
    if (on && picker) picker.close();
    picker = on ? { close: () => open(false) } : null;
    if (on) fill();
    else if (!same(before, rgb)) remember(rgb);
    before = rgb;
    for (const field of panel.querySelectorAll("[aria-invalid]")) flag(field, "");
    panel.hidden = !on;
    button.setAttribute("aria-expanded", on);
    paint();
  }

  // Follows the app, except while a colour is being chosen here.
  button.show = (value) => {
    if (performance.now() - sentAt < 300 || panel.contains(held) || same(value, rgb)) return;
    rgb = value;
    if (!same(fromHsv(hsv), rgb)) hsv = toHsv(rgb);
    paint();
  };
  button.onclick = () => open(panel.hidden);
  button.dataset.param = param.name;
  button.setAttribute("aria-expanded", "false");
  button.setAttribute("aria-controls", panel.id);
  button.setAttribute("aria-labelledby", id + "-label " + code.id);
  hex.oninput = () => {
    const typed = fromHex(hex.value);
    flag(hex, typed ? "" : "Enter a colour as six hex digits, such as #FF6A00.");
    if (typed) choose(typed);
  };
  hex.onblur = () => {
    if (!hex.hasAttribute("aria-invalid")) hex.value = toHex(rgb);
  };
  explain(hex, problem);
  revert(hex, () => (hex.value = toHex(rgb)));
  picks.setAttribute("role", "radiogroup");
  picks.setAttribute("aria-label", "Ready-made colours");
  panel.append(el("div", { className: "field" }, el("label", { htmlFor: hex.id, textContent: "Hex" }), hex), problem, picks);
  panel.setAttribute("role", "group");
  panel.setAttribute("aria-labelledby", id + "-label");
  const close = (e) => {
    if (e.key !== "Escape" || panel.hidden) return;
    e.preventDefault();
    open(false);
    button.focus();
  };
  button.addEventListener("keydown", close);
  panel.addEventListener("keydown", close);
  return el("div", { className: "field colour" }, el("label", { htmlFor: id, id: id + "-label", textContent: param.label }), button, panel);
}

// The background being edited: the main one, or the device's own when it has one.
function edited(snap = state.snap) {
  return (editDevice && snap.overrides[editDevice]) || snap.effect;
}

function setEdited(effect) {
  return editDevice ? call("set_override", { id: editDevice, effect }) : call("set_effect", { effect });
}

// `target` is "effect" for the background or "layer" for the selected layer.
function queueEffect(name, value, target = "effect") {
  if (!state.snap) return;
  if (target === "layer") return queueLayers((layers) => layers[layerSel] && (layers[layerSel].effect[name] = value));
  if (!pending.effect) {
    pending.effect = { ...edited() };
    requestAnimationFrame(() => {
      const effect = pending.effect;
      pending.effect = null;
      setEdited(effect);
    });
  }
  pending.effect[name] = value;
}

// Changes the layer list. Changes made in the same frame go out together.
function queueLayers(change) {
  if (!state.snap) return;
  if (!pending.layers) {
    pending.layers = JSON.parse(JSON.stringify(state.snap.layers));
    requestAnimationFrame(() => {
      const layers = pending.layers;
      pending.layers = null;
      call("set_layers", { layers });
    });
  }
  change(pending.layers);
}

function currentInfo() {
  const snap = state.snap;
  return snap ? snap.catalog.find((c) => c.default.kind === edited(snap).kind) : undefined;
}

function chooseEffect(kind) {
  const info = state.snap.catalog.find((c) => c.default.kind === kind);
  if (info) setEdited(lastUsed[kind] || info.default);
}

function randomise() {
  const info = currentInfo();
  if (!info) return;
  const effect = { ...edited() };
  for (const p of info.params) {
    const c = p.control;
    if (c.type === "range") {
      const steps = Math.round((c.max - c.min) / c.step);
      effect[p.name] = Number((c.min + Math.round(Math.random() * steps) * c.step).toFixed(4));
    } else if (c.type === "color") effect[p.name] = [0, 1, 2].map(() => Math.floor(Math.random() * 256));
    else if (c.type === "toggle") effect[p.name] = Math.random() < 0.5;
    else if (c.options && c.options.length) effect[p.name] = c.options[Math.floor(Math.random() * c.options.length)];
  }
  setEdited(effect);
}

// The effect in use stays listed and says so, so that the list always has a row to tab to.
function filterEffects() {
  const q = $("effect-search").value.trim().toLowerCase();
  const kind = state.snap && edited().kind;
  let shown = 0;
  for (const pick of $("effect-list").children) {
    const { radio, label, tag } = pick.parts;
    const match = label.textContent.toLowerCase().includes(q);
    pick.hidden = !match && radio.value !== kind;
    tag.hidden = match;
    if (match) shown++;
  }
  $("effect-none").hidden = shown > 0;
}

// Arrow down goes from the search into the list, Enter takes its first match.
function searchKey(e) {
  const search = $("effect-search");
  if (e.key === "Escape" && search.value) {
    search.value = "";
    filterEffects();
  } else if (e.key === "ArrowDown" || (e.key === "Enter" && search.value.trim())) {
    const rows = [...$("effect-list").children].filter((pick) => !pick.hidden);
    const found = rows.filter((pick) => pick.parts.tag.hidden);
    const used = e.key === "ArrowDown" && found.find((pick) => pick.parts.radio.checked);
    const row = used || found[0] || rows[0];
    if (!row) return;
    e.preventDefault();
    if (e.key === "ArrowDown") row.parts.radio.focus();
    // As with the arrow keys inside the list, going to an effect chooses it.
    row.parts.radio.click();
  }
}

const KEY_NAMES = {
  0x1b: "Esc", 0x2c: "PrtSc", 0x91: "ScrLk", 0x13: "Pause", 0xc0: "`", 0xbd: "-", 0xbb: "=", 0x08: "Backspace",
  0x2d: "Ins", 0x24: "Home", 0x21: "PgUp", 0x90: "Num", 0x6f: "/", 0x6a: "*", 0x6d: "-", 0x09: "Tab",
  0xdb: "[", 0xdd: "]", 0xdc: "\\", 0x2e: "Del", 0x23: "End", 0x22: "PgDn", 0x6b: "+", 0x14: "Caps",
  0xba: ";", 0xde: "'", 0x0d: "Enter", 0xa0: "Shift", 0xbc: ",", 0xbe: ".", 0xbf: "/", 0xa1: "Shift",
  0x26: "\u2191", 0xa2: "Ctrl", 0x5b: "Win", 0xa4: "Alt", 0x20: "Space", 0xa5: "Alt", 0x5c: "Win",
  0x5d: "Menu", 0xa3: "Ctrl", 0x25: "\u2190", 0x28: "\u2193", 0x27: "\u2192", 0x6e: ".",
};

const KEY_SHORT = {
  0x2c: "Prt", 0x91: "Scr", 0x13: "Brk", 0x08: "Bksp", 0x2d: "Ins", 0x24: "Hm", 0x21: "PgU", 0x22: "PgD",
  0x90: "Num", 0x2e: "Del", 0xa0: "Shift", 0xa1: "Shift", 0x5b: "Win", 0x5c: "Win", 0x5d: "Mnu", 0xa2: "Ctl", 0xa3: "Ctl",
};
const onPad = (k) => k.x > 18.25;

function keyName(vk) {
  if (KEY_NAMES[vk]) return KEY_NAMES[vk];
  if (vk >= 0x70 && vk <= 0x7b) return "F" + (vk - 0x6f);
  if (vk >= 0x60 && vk <= 0x69) return String(vk - 0x60);
  return String.fromCharCode(vk);
}

// Shift, Ctrl and Alt arrive without their side; the event's location tells it.
function keyCode(e) {
  const sided = { 16: 0xa0, 17: 0xa2, 18: 0xa4 }[e.keyCode];
  return sided ? sided + (e.location === 2 ? 1 : 0) : e.keyCode;
}

let showPad = false;

function keysField(param, id) {
  const board = el("div", { className: "keyboard", id, tabIndex: 0 });
  board.setAttribute("role", "group");
  board.setAttribute("aria-label", param.label);
  const toggle = (vk) =>
    queueLayers((layers) => {
      const keys = layers[layerSel].effect.keys;
      const at = keys.indexOf(vk);
      if (at < 0) keys.push(vk);
      else keys.splice(at, 1);
    });
  for (const k of state.snap.key_layout) {
    const key = el("button", { type: "button", textContent: KEY_SHORT[k.vk] || keyName(k.vk), tabIndex: -1 });
    key.dataset.vk = k.vk;
    key.title = (onPad(k) ? "Number pad " : "") + keyName(k.vk);
    if (onPad(k)) key.classList.add("pad");
    key.style.left = "calc(" + (k.x - k.w / 2) + " * var(--unit))";
    key.style.width = "calc(" + k.w + " * var(--unit))";
    key.style.top = (k.row / 6) * 100 + "%";
    key.onclick = () => toggle(k.vk);
    board.append(key);
  }
  // Tab still moves on, so the board never traps the keyboard.
  board.onkeydown = (e) => {
    if (e.key === "Tab" || e.repeat) return;
    if (e.key === "Escape") return board.blur();
    const vk = keyCode(e);
    if (!state.snap.key_layout.some((k) => k.vk === vk)) return;
    e.preventDefault();
    toggle(vk);
  };
  const pad = el("input", { type: "checkbox", className: "switch", id: id + "-pad", checked: showPad });
  pad.oninput = () => {
    showPad = pad.checked;
    board.classList.toggle("full", showPad);
  };
  board.classList.toggle("full", showPad);
  const clear = el("button", { type: "button", textContent: "Clear keys" });
  clear.onclick = () => queueLayers((layers) => (layers[layerSel].effect.keys = []));
  const count = el("span", { className: "meta", id: id + "-count" });
  return el("div", { className: "field keys" },
    el("label", { htmlFor: id, textContent: param.label }),
    board,
    el("p", { className: "hint", textContent: "Click keys, or click the board and press them. Tab moves on." }),
    el("div", { className: "field" }, el("label", { htmlFor: pad.id, textContent: "Show number pad" }), pad),
    el("div", { className: "row" }, count, clear));
}

function paramRow(param, target = "effect") {
  const control = param.control;
  const id = (target === "layer" ? "layer-param-" : "param-") + param.name;
  const problem = $(target === "layer" ? "layer-problem" : "params-problem");
  if (control.type === "color") return colourField(param, id, target);
  if (control.type === "keys") return keysField(param, id);
  const row = el("div", { className: "field" }, el("label", { htmlFor: id, textContent: param.label }));
  let input;
  if (control.type === "range") {
    input = el("input", { type: "range", min: control.min, max: control.max, step: control.step });
    const box = el("input", { type: "number", className: "exact" });
    box.setAttribute("aria-label", param.label);
    pair(input, box, 1, (value) => queueEffect(param.name, value, target), problem);
    row.classList.add("range");
    row.append(input, box);
  } else {
    if (control.type === "toggle") input = el("input", { type: "checkbox", className: "switch" });
    else input = el("select", {}, ...(control.options || []).map((o) => el("option", { value: o, textContent: words(o) })));
    input.oninput = () => queueEffect(param.name, read(input), target);
    row.append(input);
  }
  input.id = id;
  input.dataset.param = param.name;
  return row;
}

function renderEffects(snap) {
  if (editDevice && !snap.devices.some((d) => d.id === editDevice)) editDevice = "";
  if (layerSel >= snap.layers.length) layerSel = snap.layers.length - 1;
  renderLayers(snap);
  $("base-edit").hidden = layerSel >= 0;
  $("layer-edit").hidden = layerSel < 0;
  if (layerSel >= 0) return renderLayer(snap, snap.layers[layerSel]);

  const effect = edited(snap);
  const pick = $("edit-device");
  const devices = snap.devices.filter((d) => d.enabled);
  const pickKey = devices.map((d) => d.id + (snap.overrides[d.id] ? "*" : "")).join("|");
  if (pick.dataset.key !== pickKey) {
    pick.dataset.key = pickKey;
    pick.replaceChildren(
      el("option", { value: "", textContent: "All devices" }),
      ...devices.map((d) => el("option", { value: d.id, textContent: d.name + (snap.overrides[d.id] ? " (own)" : "") })),
    );
  }
  pick.value = editDevice;
  const own = Boolean(editDevice && snap.overrides[editDevice]);
  $("device-follow").hidden = !editDevice || own;
  $("device-reset").hidden = !own;

  const list = $("effect-list");
  const listKey = snap.catalog.map((c) => c.default.kind + "=" + c.label).join("|");
  if (list.dataset.key !== listKey) {
    list.dataset.key = listKey;
    list.replaceChildren(
      ...snap.catalog.map((info) => {
        const radio = el("input", { type: "radio", name: "effect", value: info.default.kind });
        const label = el("span", { className: "name", textContent: info.label });
        const tag = el("span", { className: "tag", textContent: "In use", hidden: true });
        radio.onchange = () => chooseEffect(radio.value);
        return Object.assign(el("label", { className: "pick" }, radio, label, tag), { parts: { radio, label, tag } });
      }),
    );
  }
  for (const radio of list.querySelectorAll("input")) radio.checked = radio.value === effect.kind;
  filterEffects();

  const box = $("params");
  const info = currentInfo();
  const params = info ? info.params : [];
  const paramKey = effect.kind + JSON.stringify(params);
  if (box.dataset.key !== paramKey) {
    box.dataset.key = paramKey;
    if (picker) picker.close();
    box.replaceChildren(...params.map((p) => paramRow(p)));
    $("params-problem").hidden = true;
  }
  for (const input of box.querySelectorAll("[data-param]")) put(input, effect[input.dataset.param]);
  $("params-empty").hidden = params.length > 0;

  // The list scrolls on its own, so keep the chosen effect in view.
  if (!$("panel-effects").hidden && list.dataset.kind !== effect.kind) {
    list.dataset.kind = effect.kind;
    const checked = list.querySelector("input:checked");
    if (checked) checked.parentNode.scrollIntoView({ block: "nearest" });
  }
}

// What a layer draws: its catalogue entry and whether it is a key or overlay effect.
function layerInfo(snap, effect) {
  const post = snap.post_catalog.find((c) => c.default.kind === effect.kind);
  return post ? { info: post, post: true } : { info: snap.catalog.find((c) => c.default.kind === effect.kind), post: false };
}

const SCOPES = { all: "All devices", keyboards: "Keyboards", chosen: "Chosen devices" };

function scopeWords(snap, layer) {
  if (layer.scope !== "chosen") return SCOPES[layer.scope];
  const n = layer.devices.filter((id) => snap.devices.some((d) => d.id === id)).length;
  return n === 1 ? snap.devices.find((d) => layer.devices.includes(d.id)).name : plural(n, "device");
}

const appName = (name) => name.trim().toLowerCase().replace(/\.exe$/, "");

// Whether a layer tied to apps shows right now.
function live(layer) {
  const named = (n) => layer.apps.some((a) => appName(a) === n);
  if (layer.when === "running") return state.apps.running.some(named);
  if (layer.when === "front") return !!state.apps.front && named(state.apps.front);
  return true;
}

function whenWords(layer) {
  if (layer.when === "always") return "";
  if (!layer.apps.length) return "no app yet";
  const names = layer.apps[0] + (layer.apps.length > 1 ? " +" + (layer.apps.length - 1) : "");
  return layer.when === "front" ? names + " in front" : "while " + names + " is open";
}

function eyeIcon(open) {
  const path = open
    ? "M1 8s2.5-5 7-5 7 5 7 5-2.5 5-7 5-7-5-7-5zm7 2.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z"
    : "M2 2l12 12M6.5 3.2A7 7 0 0 1 8 3c4.5 0 7 5 7 5a12 12 0 0 1-2 2.6M10 12.6A7 7 0 0 1 8 13c-4.5 0-7-5-7-5a12 12 0 0 1 2.4-3";
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  const shape = document.createElementNS("http://www.w3.org/2000/svg", "path");
  shape.setAttribute("d", path);
  shape.setAttribute("fill", open ? "currentColor" : "none");
  shape.setAttribute("fill-rule", "evenodd");
  if (!open) {
    shape.setAttribute("stroke", "currentColor");
    shape.setAttribute("stroke-width", "1.5");
  }
  svg.append(shape);
  return svg;
}

// Moves layer `from` to `to` (array indices, 0 at the bottom) and keeps it selected.
function moveLayer(from, to) {
  const n = state.snap.layers.length;
  to = Math.max(0, Math.min(n - 1, to));
  if (from === to || from < 0 || from >= n) return;
  queueLayers((layers) => layers.splice(to, 0, ...layers.splice(from, 1)));
  layerSel = to;
  layerFocus = "layer-" + to;
}

function selectLayer(i) {
  layerSel = i;
  layerFocus = i < 0 ? "layer-base" : "layer-" + i;
  scheduleRender();
}

// The top layer is listed first, the background last, as in an image editor.
function renderLayers(snap) {
  const add = $("layer-add");
  if (!add.options.length) add.replaceChildren(el("option", { value: "", textContent: "Add a layer" }), ...layerKinds(snap));
  const list = $("layer-list");
  const key = JSON.stringify([snap.layers.map((l) => [l.enabled, l.effect.kind, l.scope, l.devices, l.blend, Math.round(l.opacity * 100), l.when, l.apps, live(l)]), edited(snap).kind, snap.effect.kind, Object.keys(snap.overrides), layerSel, snap.devices.length]);
  if (list.dataset.key === key) return;
  list.dataset.key = key;
  const focused = list.contains(document.activeElement) ? document.activeElement.id : "";
  const rows = snap.layers.map((layer, i) => {
    const { info } = layerInfo(snap, layer.effect);
    const name = info ? info.label : words(layer.effect.kind);
    const eye = el("button", { type: "button", className: "eye", id: "layer-eye-" + i }, eyeIcon(layer.enabled));
    eye.setAttribute("aria-pressed", String(layer.enabled));
    eye.setAttribute("aria-label", (layer.enabled ? "Hide " : "Show ") + name);
    eye.onclick = () => queueLayers((layers) => (layers[i].enabled = !layers[i].enabled));
    const pick = el("button", { type: "button", className: "layer-name", id: "layer-" + i },
      el("span", { className: "name", textContent: name }),
      el("span", { className: "tag", textContent: [scopeWords(snap, layer), whenWords(layer), layer.opacity < 1 ? Math.round(layer.opacity * 100) + "%" : "", layer.blend !== "normal" ? words(layer.blend) : ""].filter(Boolean).join(" · ") }));
    pick.setAttribute("aria-pressed", String(i === layerSel));
    pick.title = "Alt+Up and Alt+Down move the layer";
    pick.onclick = () => selectLayer(i);
    pick.onkeydown = (e) => {
      if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
      e.preventDefault();
      moveLayer(i, i + (e.key === "ArrowUp" ? 1 : -1));
    };
    const grip = el("span", { className: "grip" });
    grip.setAttribute("aria-hidden", "true");
    const row = el("li", { className: "layer" + (layer.enabled ? "" : " off") + (layer.enabled && !live(layer) ? " waiting" : "") + (i === layerSel ? " selected" : "") }, grip, eye, pick);
    row.draggable = true;
    row.ondragstart = (e) => {
      dragFrom = i;
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(i));
      row.classList.add("dragging");
    };
    row.ondragend = () => {
      dragFrom = -1;
      for (const r of list.children) r.classList.remove("dragging", "drop-above", "drop-below");
    };
    row.ondragover = (e) => {
      if (dragFrom < 0) return;
      e.preventDefault();
      const above = e.offsetY < row.offsetHeight / 2;
      row.classList.toggle("drop-above", above);
      row.classList.toggle("drop-below", !above);
    };
    row.ondragleave = () => row.classList.remove("drop-above", "drop-below");
    row.ondrop = (e) => {
      e.preventDefault();
      const above = row.classList.contains("drop-above");
      row.classList.remove("drop-above", "drop-below");
      if (dragFrom < 0) return;
      // Above a row in the list means higher in the stack.
      let to = above ? i + 1 : i;
      if (dragFrom < to) to--;
      moveLayer(dragFrom, to);
    };
    return row;
  });
  const own = Object.keys(snap.overrides).length;
  const baseInfo = snap.catalog.find((c) => c.default.kind === snap.effect.kind);
  const base = el("button", { type: "button", className: "layer-name", id: "layer-base" },
    el("span", { className: "name", textContent: "Background" }),
    el("span", { className: "tag", textContent: (baseInfo ? baseInfo.label : words(snap.effect.kind)) + (own ? " · " + own + " own" : "") }));
  base.setAttribute("aria-pressed", String(layerSel < 0));
  base.onclick = () => selectLayer(-1);
  const lock = el("span", { className: "grip none" });
  lock.setAttribute("aria-hidden", "true");
  const baseRow = el("li", { className: "layer base" + (layerSel < 0 ? " selected" : "") }, lock, base);
  baseRow.ondragover = (e) => dragFrom >= 0 && (e.preventDefault(), baseRow.classList.add("drop-above"));
  baseRow.ondragleave = () => baseRow.classList.remove("drop-above");
  baseRow.ondrop = (e) => {
    e.preventDefault();
    baseRow.classList.remove("drop-above");
    if (dragFrom >= 0) moveLayer(dragFrom, 0);
  };
  list.replaceChildren(...rows.reverse(), baseRow);
  const again = layerFocus || focused;
  if (again && $(again)) {
    $(again).focus();
    layerFocus = "";
  }
}

function layerKinds(snap) {
  const group = (label, entries, tag) =>
    el("optgroup", { label }, ...entries.filter((c) => c.default.kind !== "none").map((c) => el("option", { value: tag + c.default.kind, textContent: c.label })));
  return [group("Key and overlay effects", snap.post_catalog, "post:"), group("Effects", snap.catalog, "fill:")];
}

function entryFor(snap, value) {
  const [tag, kind] = value.split(":");
  return (tag === "post" ? snap.post_catalog : snap.catalog).find((c) => c.default.kind === kind);
}

function renderLayer(snap, layer) {
  const { info } = layerInfo(snap, layer.effect);
  lastLayer[layer.effect.kind] = layer.effect;
  setText($("layer-title"), info ? info.label : "Layer");
  const kind = $("layer-kind");
  const kinds = JSON.stringify([snap.post_catalog.map((c) => c.label), snap.catalog.map((c) => c.label)]);
  if (kind.dataset.key !== kinds) {
    kind.dataset.key = kinds;
    kind.replaceChildren(...layerKinds(snap));
  }
  if (!busy(kind)) kind.value = (snap.post_catalog.some((c) => c.default.kind === layer.effect.kind) ? "post:" : "fill:") + layer.effect.kind;
  put($("layer-blend"), layer.blend);
  put($("layer-opacity"), Math.round(layer.opacity * 100));
  put($("layer-scope"), layer.scope);

  const checks = $("layer-devices");
  checks.hidden = layer.scope !== "chosen";
  const checksKey = snap.devices.map((d) => d.id).join("|") + layerSel;
  if (checks.dataset.key !== checksKey) {
    checks.dataset.key = checksKey;
    checks.replaceChildren(
      ...snap.devices.map((d) => {
        const box = el("input", { type: "checkbox", id: "layer-dev-" + d.id });
        box.dataset.device = d.id;
        box.oninput = () => queueLayers((layers) => {
          const l = layers[layerSel];
          l.devices = l.devices.filter((id) => id !== d.id);
          if (box.checked) l.devices.push(d.id);
        });
        return el("label", { className: "check" }, box, el("span", { textContent: d.name }));
      }),
    );
  }
  for (const box of checks.querySelectorAll("input")) box.checked = layer.devices.includes(box.dataset.device);

  put($("layer-when"), layer.when);
  $("layer-apps").hidden = layer.when === "always";
  const apps = $("layer-app-list");
  const appsKey = layerSel + JSON.stringify(layer.apps);
  if (apps.dataset.key !== appsKey) {
    apps.dataset.key = appsKey;
    apps.replaceChildren(
      ...layer.apps.map((name, n) => {
        const remove = el("button", { type: "button", className: "remove", textContent: "Remove", id: "layer-app-remove-" + n });
        remove.setAttribute("aria-label", "Remove " + name);
        remove.onclick = () => {
          queueLayers((layers) => layers[layerSel].apps.splice(n, 1));
          layerFocus = "layer-app";
        };
        return el("li", { className: "app" }, el("span", { className: "name", textContent: name }), remove);
      }),
    );
  }
  setText($("layer-app-hint"), layer.apps.length ? "Pick an app with a window open, or type its .exe name." : "Add an app. The layer stays hidden until one is " + (layer.when === "front" ? "in front." : "open."));

  const box = $("layer-params");
  const params = info ? info.params : [];
  const key = layerSel + layer.effect.kind + JSON.stringify(params);
  if (box.dataset.key !== key) {
    box.dataset.key = key;
    if (picker) picker.close();
    box.replaceChildren(...params.map((p) => paramRow(p, "layer")));
    $("layer-problem").hidden = true;
  }
  for (const input of box.querySelectorAll("[data-param]")) put(input, layer.effect[input.dataset.param]);
  const board = box.querySelector(".keyboard");
  if (board) {
    const on = layer.effect.keys;
    const [r, g, b] = layer.effect.color;
    board.style.setProperty("--on", `rgb(${r}, ${g}, ${b})`);
    board.style.setProperty("--on-text", 0.299 * r + 0.587 * g + 0.114 * b > 140 ? "#000" : "#fff");
    for (const key of board.children) key.setAttribute("aria-pressed", String(on.includes(Number(key.dataset.vk))));
    setText($(board.id + "-count"), on.length ? plural(on.length, "key") + ": " + on.map(keyName).join(" ") : "No keys chosen");
    const padKey = (vk) => state.snap.key_layout.some((k) => k.vk === vk && onPad(k));
    if (!showPad && on.some(padKey)) {
      showPad = true;
      board.classList.add("full");
      $(board.id + "-pad").checked = true;
    }
  }
  // The second colour only matters when the colours fade to it.
  const second = box.querySelector('[data-param="second"]');
  if (second) second.closest(".field").hidden = layer.effect.colors !== "fade_to_second";
  const main = box.querySelector('[data-param="color"]');
  if (main && "colors" in layer.effect) main.closest(".field").hidden = !["single", "fade_to_second"].includes(layer.effect.colors);
}

function addLayer(value) {
  const entry = entryFor(state.snap, value);
  if (!entry) return;
  const effect = JSON.parse(JSON.stringify(entry.default));
  queueLayers((layers) => layers.push({ enabled: true, opacity: 1, blend: "normal", scope: entry.default.kind.startsWith("press_") || entry.default.kind === "key_lights" ? "keyboards" : "all", devices: [], when: "always", apps: [], effect }));
  layerSel = state.snap.layers.length;
  layerFocus = "layer-" + layerSel;
}

function wireLayers() {
  $("layer-add").oninput = () => {
    const value = $("layer-add").value;
    $("layer-add").value = "";
    addLayer(value);
  };
  $("layer-kind").oninput = () => {
    const entry = entryFor(state.snap, $("layer-kind").value);
    if (entry) queueLayers((layers) => (layers[layerSel].effect = JSON.parse(JSON.stringify(lastLayer[entry.default.kind] || entry.default))));
  };
  $("layer-blend").oninput = () => queueLayers((layers) => (layers[layerSel].blend = $("layer-blend").value));
  pair($("layer-opacity"), $("layer-opacity-exact"), 1, (v) => queueLayers((layers) => (layers[layerSel].opacity = v / 100)), $("layer-problem"));
  $("layer-scope").oninput = () => queueLayers((layers) => (layers[layerSel].scope = $("layer-scope").value));
  $("layer-when").oninput = () => queueLayers((layers) => (layers[layerSel].when = $("layer-when").value));
  // The list of open apps is read when the field is used, not kept up to date.
  $("layer-app").onfocus = async () => {
    const names = (await call("list_apps")) || [];
    $("layer-app-choices").replaceChildren(...names.map((n) => el("option", { value: n })));
  };
  $("layer-app-form").onsubmit = (e) => {
    e.preventDefault();
    const input = $("layer-app");
    const name = input.value.trim();
    if (!name) return;
    input.value = "";
    queueLayers((layers) => {
      const l = layers[layerSel];
      if (!l.apps.some((a) => appName(a) === appName(name))) l.apps.push(name);
    });
  };
  $("layer-reset").onclick = () => {
    const entry = layerInfo(state.snap, state.snap.layers[layerSel].effect).info;
    if (entry) queueLayers((layers) => (layers[layerSel].effect = JSON.parse(JSON.stringify(entry.default))));
  };
  $("layer-copy").onclick = () => {
    const i = layerSel;
    queueLayers((layers) => layers.splice(i + 1, 0, JSON.parse(JSON.stringify(layers[i]))));
    layerSel = i + 1;
    layerFocus = "layer-" + layerSel;
  };
  $("layer-remove").onclick = () => {
    const i = layerSel;
    queueLayers((layers) => layers.splice(i, 1));
    layerSel = Math.min(i, state.snap.layers.length - 2);
    layerFocus = layerSel < 0 ? "layer-base" : "layer-" + layerSel;
  };
  $("edit-device").oninput = () => {
    editDevice = $("edit-device").value;
    scheduleRender();
  };
  $("device-reset").onclick = () => call("set_override", { id: editDevice, effect: null });
}

// Only reset_layout knows the defaults, so reset everything and put the other devices back.
function resetPosition(id) {
  const others = state.snap.devices.filter((d) => d.id !== id);
  return quietly(async () => {
    if ((await call("reset_layout")) === undefined) return;
    for (const d of others) await sendRect(d, d.rect);
    note("Position reset");
  });
}

function deviceRow(device) {
  const name = el("button", { type: "button", className: "name" });
  const status = el("span", { className: "meta" });
  const on = el("input", { type: "checkbox", className: "switch" });
  const meta = el("p", { className: "meta" });
  const level = el("input", { type: "range", min: 0, max: 1, step: 0.01, id: "device-level-" + ++rowCount });
  const box = el("input", { type: "number", className: "exact" });
  const reset = el("button", { type: "button", textContent: "Reset position" });
  const problem = el("p", { className: "problem", id: "device-problem-" + rowCount, hidden: true });
  const send = throttled((change) => {
    const now = findDevice(device.id);
    if (now) call("set_device", { id: now.id, enabled: now.enabled, brightness: now.brightness, rect: now.rect, ...change });
  });
  const fields = ["X", "Y", "Width", "Height"].map((label, n) => {
    const input = el("input", { type: "number", className: "exact", min: 0, max: 100, step: 0.5 });
    const typed = () => {
      const value = input.value === "" ? NaN : input.value / 100;
      const ok = value >= 0 && value <= 1;
      flag(input, ok ? "" : label + " must be from 0 to 100.");
      return ok ? value : null;
    };
    input.oninput = typed;
    input.onchange = () => {
      const now = findDevice(device.id);
      const value = typed();
      if (!now || value === null) return;
      // Only the field that was edited changes; the others show rounded values.
      const rect = now.rect.slice();
      rect[n] = value;
      const w = clamp(rect[2], MIN_SIZE, 1);
      const h = clamp(rect[3], MIN_SIZE, 1);
      sendRect(now, [clamp(rect[0], 0, 1 - w), clamp(rect[1], 0, 1 - h), w, h]);
    };
    explain(input, problem);
    revert(input, () => {
      const now = findDevice(device.id);
      if (now) input.value = (rectOf(now)[n] * 100).toFixed(1);
    });
    return el("label", { textContent: label }, input);
  });
  const place = fields.map((field) => field.lastChild);
  const sizes = (device.zones || []).map((zone) => {
    const input = el("input", { type: "number", className: "exact", min: zone.min, max: zone.max, step: 1 });
    const typed = () => {
      const n = input.value === "" ? NaN : Number(input.value);
      const ok = Number.isInteger(n) && n >= zone.min && n <= zone.max;
      flag(input, ok ? "" : `${zone.name} takes ${zone.min} to ${zone.max} LEDs.`);
      return ok ? n : null;
    };
    input.oninput = typed;
    input.onchange = () => {
      const n = typed();
      if (n === null || !state.snap) return;
      const settings = structuredClone(state.snap.settings);
      settings.zone_leds = { ...settings.zone_leds, [zone.key]: n };
      call("set_settings", { settings });
    };
    explain(input, problem);
    revert(input, () => (input.value = zone.leds));
    return el("label", { textContent: zone.name }, input);
  });
  const counts = sizes.map((field) => field.lastChild);

  name.onclick = () => selectDevice(preview.selected === device.id ? null : device.id);
  on.onchange = () => send({ enabled: on.checked });
  on.setAttribute("aria-label", "Enable " + device.name);
  box.setAttribute("aria-label", "Brightness in per cent");
  pair(level, box, 100, (brightness) => send({ brightness }), problem);
  reset.onclick = () => resetPosition(device.id);

  const details = el(
    "div",
    { className: "details" },
    meta,
    el(
      "div",
      { className: "field range" },
      el("label", { htmlFor: level.id, textContent: "Brightness" }),
      level,
      el("span", { className: "percent" }, box, "%"),
    ),
    el("p", { className: "meta", textContent: "Position and size, in % of the canvas" }),
    el("div", { className: "place" }, ...fields),
    ...(sizes.length ? [el("p", { className: "meta", textContent: "LEDs" }), el("div", { className: "place" }, ...sizes)] : []),
    problem,
    el("div", { className: "actions" }, reset),
  );
  const row = el("li", { className: "device" }, el("div", { className: "device-head" }, name, status, on), details);
  row.parts = { name, status, on, level, meta, place, counts, details };
  return row;
}

function renderDevices(snap) {
  const list = $("device-list");
  const zoneKey = (d) => (d.zones || []).map((z) => [z.key, z.min, z.max].join(",")).join(";");
  const key = snap.devices.map((d) => d.id + ":" + zoneKey(d)).join("|");
  if (list.dataset.key !== key) {
    list.dataset.key = key;
    list.replaceChildren(...snap.devices.map(deviceRow));
  }
  snap.devices.forEach((device, i) => {
    const row = list.children[i];
    const { name, status, on, level, meta, place, counts, details } = row.parts;
    const selected = device.id === preview.selected;
    const about = [BACKENDS[device.backend] || device.backend, KINDS[device.kind] || words(device.kind), plural(device.leds.length, "LED")];
    row.classList.toggle("selected", selected);
    details.hidden = !selected;
    name.setAttribute("aria-expanded", selected);
    name.title = device.name;
    setText(name, device.name);
    setText(status, device.enabled ? "On" : "Off");
    setText(meta, about.join(" · "));
    put(on, device.enabled);
    put(level, device.brightness);
    const rect = rectOf(device);
    place.forEach((input, n) => put(input, (rect[n] * 100).toFixed(1)));
    counts.forEach((input, n) => put(input, device.zones[n].leds));
  });
  $("devices-empty").hidden = snap.devices.length > 0;

  const selected = preview.selected || "";
  if (!$("panel-devices").hidden && list.dataset.selected !== selected) {
    list.dataset.selected = selected;
    const row = list.querySelector(".selected");
    if (row) row.scrollIntoView({ block: "nearest" });
  }
}

// Takes the user to a control on the Settings tab and points it out.
function showSetting(id) {
  const input = $(id);
  const row = input.closest(".field") || input;
  showTab($("tab-settings"));
  row.scrollIntoView({ block: "center" });
  input.focus();
  row.classList.add("target");
  setTimeout(() => row.classList.remove("target"), 2500);
}

function detectedRow(item) {
  const [mark, label] = STATES[item.state] || ["none", words(item.state)];
  const route = ROUTES[item.route];
  const tag = el("span", { className: "state" });
  const about = [KINDS[item.kind] || words(item.kind), BUSES[item.bus], route && route[0]];
  setState(tag, mark, label);
  const row = el(
    "li",
    { className: "found" },
    el("div", { className: "split" }, el("span", { className: "name", textContent: item.name, title: item.name }), tag),
    el("span", { className: "meta", textContent: about.filter(Boolean).join(" · ") }),
    el("span", { className: "hint", textContent: item.note }),
  );
  if (item.state === "available" && route) {
    const go = el("button", { type: "button", textContent: route[0] + " setting" });
    go.onclick = () => showSetting(route[1]);
    row.append(go);
  }
  if (item.bus === "usb" && item.state !== "active") {
    const info = el("button", { type: "button", textContent: "Send device info" });
    info.onclick = () => showInfo(item);
    row.append(info);
  }
  return row;
}

async function showInfo(item) {
  const text = await call("device_info", { id: item.id, name: item.name });
  if (text === undefined) return;
  $("info-text").value = text;
  setText($("info-status"), "");
  $("info-send").disabled = false;
  $("info-sheet").showModal();
}

function wireInfo() {
  const status = $("info-status");
  $("info-close").onclick = () => $("info-sheet").close();
  $("info-copy").onclick = async () => {
    await navigator.clipboard.writeText($("info-text").value);
    setText(status, "Copied. Paste it into a device issue on GitHub.");
  };
  $("info-send").onclick = async () => {
    const send = $("info-send");
    send.disabled = true;
    setText(status, "Sending…");
    try {
      await invoke("send_device_info", { text: $("info-text").value });
      setText(status, "Sent. Thank you.");
    } catch (e) {
      send.disabled = false;
      setText(status, typeof e === "string" ? e : String(e));
    }
  };
}

function renderDetected(snap) {
  const found = snap.detected || [];
  const list = $("detected-list");
  const key = JSON.stringify(found);
  if (list.dataset.key !== key) {
    list.dataset.key = key;
    list.replaceChildren(...found.map(detectedRow));
  }
  $("detected-empty").hidden = found.length > 0;
}

// A profile can only be saved from the live state, so the source is loaded, saved under the
// new name, and the live state is then put back.
function copyProfile(from, to, remove) {
  const { effect, brightness, active_profile: active } = state.snap;
  if (remove) refocus = "Rename " + to;
  return quietly(async () => {
    const loaded = active === from || (await call("load_profile", { name: from })) !== undefined;
    const saved = loaded && (await call("save_profile", { name: to })) !== undefined;
    const gone = saved && remove && (await call("delete_profile", { name: from })) !== undefined;
    if (active === from) {
      if (saved && !gone) await call("load_profile", { name: from });
    } else if (active) await call("load_profile", { name: active });
    else if (loaded) {
      await call("set_effect", { effect });
      await call("set_brightness", { value: brightness });
    }
    if (gone && lastProfile === from) lastProfile = to;
    if (saved) note(gone ? "Renamed to " + to : "Duplicated as " + to);
  });
}

// Asks for a profile name. `own` is a name that may stay as it is.
function askName(title, value, own, done) {
  const input = $("name-input");
  asking = { own, done };
  setText($("name-title"), title);
  input.value = value;
  flag(input, "");
  $("name-sheet").showModal();
  input.select();
}

function profileRow(name) {
  const button = (label, run) => {
    const b = el("button", { type: "button", textContent: label });
    b.setAttribute("aria-label", label + " " + name);
    b.onclick = run;
    return b;
  };
  const remove = button("Delete", () => {
    if (remove.dataset.armed) {
      const names = state.snap.profiles;
      const next = names[names.indexOf(name) + 1] || names[names.indexOf(name) - 1];
      refocus = "Load " + next;
      return call("delete_profile", { name }).then((out) => out !== undefined && note("Deleted profile " + name));
    }
    remove.dataset.armed = "1";
    remove.textContent = "Confirm";
    remove.setAttribute("aria-label", "Confirm deleting " + name);
    setTimeout(() => {
      delete remove.dataset.armed;
      remove.textContent = "Delete";
      remove.setAttribute("aria-label", "Delete " + name);
    }, 4000);
  });
  return el(
    "li",
    { className: "profile" },
    el("div", { className: "split" }, el("span", { className: "name", textContent: name, title: name }), el("span", { className: "tag", textContent: "Active" })),
    el(
      "div",
      { className: "actions" },
      button("Load", () => call("load_profile", { name })),
      button("Rename", () => askName("Rename profile", name, name, (to) => to !== name && copyProfile(name, to, true))),
      button("Duplicate", () => askName("Duplicate profile", name + " copy", null, (to) => copyProfile(name, to, false))),
      remove,
    ),
  );
}

function renderProfiles(snap) {
  const list = $("profile-list");
  const key = JSON.stringify(snap.profiles);
  if (list.dataset.key !== key) {
    // New rows drop the focus. It goes back to the same button, or to the one a rename or a delete asked for.
    const lost = list.contains(document.activeElement) && document.activeElement.getAttribute("aria-label");
    const find = (label) => [...list.querySelectorAll("button")].find((b) => b.getAttribute("aria-label") === label);
    list.dataset.key = key;
    list.replaceChildren(...snap.profiles.map(profileRow));
    if (lost) (find(lost) || find(refocus) || $("profile-name")).focus();
  }
  snap.profiles.forEach((name, i) => list.children[i].classList.toggle("active", name === snap.active_profile));
  $("profiles-empty").hidden = snap.profiles.length > 0;
}

function paramProblem(control, value) {
  if (control.type === "range") {
    const ok = typeof value === "number" && value >= control.min && value <= control.max;
    return ok ? "" : `a number from ${control.min} to ${control.max}`;
  }
  if (control.type === "color") {
    const ok = Array.isArray(value) && value.length === 3 && value.every((v) => Number.isInteger(v) && v >= 0 && v <= 255);
    return ok ? "" : "three whole numbers from 0 to 255";
  }
  if (control.type === "toggle") return typeof value === "boolean" ? "" : "true or false";
  return (control.options || []).includes(value) ? "" : "one of " + (control.options || []).join(", ");
}

// Checks effect JSON against the catalogue. Gives the effect, or a sentence saying what is wrong.
function readEffect(text) {
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    return { problem: "This is not valid JSON." };
  }
  const kind = data && data.kind;
  const info = state.snap.catalog.find((c) => c.default.kind === kind);
  if (typeof kind !== "string") return { problem: 'The effect needs a "kind", such as "rainbow".' };
  if (!info) return { problem: `There is no effect of kind "${kind}".` };
  const extra = Object.keys(data).find((k) => k !== "kind" && !info.params.some((p) => p.name === k));
  if (extra) return { problem: `${info.label} has no parameter "${extra}".` };
  const effect = { ...info.default, ...data };
  for (const p of info.params) {
    const problem = paramProblem(p.control, effect[p.name]);
    if (problem) return { problem: `"${p.name}" must be ${problem}.` };
  }
  return { effect };
}

function openJson(hint) {
  const text = $("json-text");
  setText($("json-hint"), hint);
  text.value = JSON.stringify(edited(), null, 2);
  flag(text, "");
  $("json-sheet").showModal();
  text.select();
}

async function copyEffect() {
  try {
    await navigator.clipboard.writeText(JSON.stringify(edited(), null, 2));
    note("Effect copied as JSON");
  } catch {
    openJson("Copying was blocked. Press Ctrl+C to copy the selected text.");
  }
}

const IP = ["ip", "Address", "ip"];
// Each kind of network light: its name, a blank entry as OpenRGB defaults it, and the fields the user sets.
const NET_KINDS = {
  hue: {
    name: "Philips Hue bridge",
    pair: "Press the link button on the bridge. FoxRGB waits 35 seconds for it.",
    blank: { ip: "", username: "", clientkey: "" },
    fields: [IP],
  },
  nanoleaf: {
    name: "Nanoleaf",
    pair: "Hold the power button on the controller for 5 to 7 seconds, until its lights flash. FoxRGB waits 30 seconds.",
    blank: { ip: "", port: 16021, auth_token: "" },
    fields: [IP, ["port", "Port", 1, 65535]],
  },
  wiz: {
    name: "Philips Wiz",
    blank: { ip: "", use_cool_white: false, use_warm_white: false, white_strategy: "Average" },
    fields: [IP, ["white_strategy", "White from", ["Average", "Minimum", "None"]], ["use_cool_white", "Cool white", "check"], ["use_warm_white", "Warm white", "check"]],
  },
  lifx: {
    name: "LIFX",
    blank: { ip: "", name: "", multizone: false, extended_multizone: false },
    fields: [IP, ["name", "Name", "text"], ["multizone", "Multizone", "check"], ["extended_multizone", "Extended multizone", "check"]],
  },
  govee: { name: "Govee", blank: { ip: "" }, fields: [IP] },
  yeelight: {
    name: "Yeelight",
    blank: { ip: "", host_ip: "", music_mode: false },
    fields: [IP, ["host_ip", "This PC's address", "optip"], ["music_mode", "Music mode", "check"]],
  },
  kasa: { name: "Kasa Smart", blank: { ip: "", name: "" }, fields: [IP, ["name", "Name", "text"]] },
  elgato_key_light: { name: "Elgato Key Light", blank: { ip: "" }, fields: [IP] },
  elgato_light_strip: { name: "Elgato Light Strip", blank: { ip: "" }, fields: [IP] },
  e131: {
    name: "E1.31 (sACN)",
    blank: { name: "", ip: "", num_leds: 1, start_universe: 1, start_channel: 1, keepalive_time: 0, universe_size: 512 },
    fields: [
      ["name", "Name", "text"],
      ["ip", "Address, empty for multicast", "optip"],
      ["num_leds", "LEDs", 1, 8192],
      ["start_universe", "First universe", 1, 63999],
      ["start_channel", "First channel", 1, 512],
      ["universe_size", "Universe size", 1, 512],
      ["keepalive_time", "Keepalive (ms)", 0, 60000],
    ],
  },
  ddp: {
    name: "DDP",
    blank: { name: "", ip: "", port: 4048, num_leds: 1, keepalive_time: 1000 },
    fields: [["name", "Name", "text"], IP, ["port", "Port", 1, 65535], ["num_leds", "LEDs", 1, 8192], ["keepalive_time", "Keepalive (ms)", 0, 60000]],
  },
  espurna: { name: "Espurna", blank: { ip: "", port: 80, apikey: "" }, fields: [IP, ["port", "Port", 1, 65535], ["apikey", "API key", "text"]] },
  wled: { name: "WLED / LeetDesk / SRGBmods WLC", blank: { ip: "" }, fields: [IP] },
  twinkly: { name: "Twinkly", blank: { ip: "" }, fields: [IP] },
  cololight: { name: "Cololight", blank: { ip: "" }, fields: [IP] },
  dmx: {
    name: "DMX (serial)",
    blank: { name: "", port: "COM3", keepalive_time: 0, red_channel: 1, green_channel: 2, blue_channel: 3, brightness_channel: 0 },
    fields: [
      ["name", "Name", "text"],
      ["port", "Serial port", "serial"],
      ["red_channel", "Red channel", 0, 512],
      ["green_channel", "Green channel", 0, 512],
      ["blue_channel", "Blue channel", 0, 512],
      ["brightness_channel", "Brightness channel", 0, 512],
      ["keepalive_time", "Keepalive (ms)", 0, 60000],
    ],
  },
};
let netFound = [];
let netPairing = "";

// Private, link-local or loopback, as the drivers accept.
function localIp(text) {
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(text);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    if (v4.slice(1).some((n) => Number(n) > 255)) return false;
    return a === 10 || a === 127 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
  }
  return /^(::1|f[cd][0-9a-f]{2}:[0-9a-f:]*|fe[89ab][0-9a-f]:[0-9a-f:]*)$/i.test(text);
}

function netCheck(type, min, max, input) {
  if (type === "check") return { value: input.checked };
  const text = input.value.trim();
  if (Array.isArray(type) || type === "text") return { value: text };
  if (type === "ip" || type === "optip") {
    if (!text && type === "optip") return { value: "" };
    return localIp(text) ? { value: text } : { problem: "Enter an address on this network, such as 192.168.1.20." };
  }
  if (type === "serial") return /^COM\d{1,3}$/i.test(text) ? { value: text.toUpperCase() } : { problem: "Enter a serial port, such as COM3." };
  const n = Number(text);
  return text !== "" && Number.isInteger(n) && n >= min && n <= max ? { value: n } : { problem: `Enter a whole number from ${min} to ${max}.` };
}

function saveNet(change) {
  if (!state.snap) return;
  const settings = structuredClone(state.snap.settings);
  change(settings.network);
  return call("set_settings", { settings });
}

async function pairNet(kind, item) {
  if (netPairing) return;
  netPairing = kind + item.ip;
  const spec = NET_KINDS[kind];
  setText($("net-status"), spec.pair);
  scheduleRender();
  const out = kind === "hue" ? await call("pair_hue", { ip: item.ip }) : await call("pair_nanoleaf", { ip: item.ip, port: item.port || 16021 });
  netPairing = "";
  setText($("net-status"), out === undefined ? "" : "Paired " + spec.name + " at " + item.ip);
  scheduleRender();
}

function pairButton(kind, item, paired) {
  const waiting = netPairing === kind + item.ip;
  const button = el("button", { type: "button", textContent: waiting ? "Waiting" : paired ? "Pair again" : "Pair" });
  if (netPairing) button.setAttribute("aria-disabled", "true");
  button.onclick = () => localIp(item.ip) && pairNet(kind, item);
  return button;
}

function netRow(kind, index, item) {
  const spec = NET_KINDS[kind];
  const id = `net-${kind}-${index}`;
  const problem = el("p", { className: "problem", id: id + "-problem", hidden: true });
  const fields = spec.fields.map(([key, label, type, max]) => {
    const kindOf = typeof type === "number" ? "number" : type;
    let input;
    if (kindOf === "check") input = el("input", { type: "checkbox", checked: Boolean(item[key]) });
    else if (Array.isArray(type)) input = el("select", {}, ...type.map((o) => el("option", { value: o, textContent: o })));
    else input = el("input", { type: kindOf === "number" ? "number" : "text", autocomplete: "off", spellcheck: false });
    if (kindOf === "number") Object.assign(input, { min: type, max, step: 1 });
    if (kindOf !== "check") input.value = item[key] ?? "";
    input.id = `${id}-${key}`;
    const check = () => {
      const out = netCheck(kindOf, type, max, input);
      flag(input, out.problem || "");
      return out;
    };
    explain(input, problem);
    input.oninput = check;
    input.onchange = () => {
      const out = check();
      if (!("problem" in out)) saveNet((n) => (n[kind][index][key] = out.value));
    };
    revert(input, () => (input.value = item[key] ?? ""));
    if (kindOf === "check") return el("label", { className: "check" }, input, label);
    return el("label", {}, label, input);
  });
  const remove = el("button", { type: "button", textContent: "Remove", id: id + "-remove" });
  remove.onclick = () => saveNet((n) => n[kind].splice(index, 1));
  const status = el("span", { className: "state" });
  const actions = el("div", { className: "actions" });
  if (spec.pair) {
    const paired = Boolean(item.username || item.auth_token);
    setState(status, paired ? "on" : "off", paired ? "Paired" : "Not paired");
    actions.append(Object.assign(pairButton(kind, item, paired), { id: id + "-pair" }));
  }
  actions.append(remove);
  const title = item.name || item.ip || item.port || "New";
  return el(
    "li",
    { className: "net-item" },
    el("div", { className: "split" }, el("span", { className: "name", textContent: spec.name + ": " + title }), status),
    el("div", { className: "net-fields" }, ...fields),
    problem,
    actions,
  );
}

function foundRow(item) {
  const spec = NET_KINDS[item.family];
  const added = (state.snap.settings.network[item.family] || []).find((d) => d.ip === item.ip);
  const row = el("li", { className: "net-item" }, el("div", { className: "split" }, el("span", { className: "name", textContent: item.name || spec.name }), el("span", { className: "meta", textContent: item.ip })));
  if (spec.pair) row.append(el("div", { className: "actions" }, pairButton(item.family, { ip: item.ip, port: item.port }, Boolean(added && (added.username || added.auth_token)))));
  else if (added) row.append(el("p", { className: "meta", textContent: "Added" }));
  else {
    const add = el("button", { type: "button", textContent: "Add" });
    add.onclick = () => saveNet((n) => n[item.family].push({ ...spec.blank, ip: item.ip, ...("name" in spec.blank ? { name: item.name } : {}) }));
    row.append(el("div", { className: "actions" }, add));
  }
  return row;
}

function renderNetwork(snap) {
  const net = snap.settings.network;
  $("net-body").hidden = !snap.settings.network_enabled;
  put($("set-net-discover"), net.discover);
  const select = $("net-family");
  if (!select.options.length) select.append(...Object.entries(NET_KINDS).map(([k, s]) => el("option", { value: k, textContent: s.name })));
  const list = $("net-list");
  const key = JSON.stringify([net, netPairing, netFound]);
  const focused = list.contains(document.activeElement) ? document.activeElement : null;
  // A field being edited keeps its text; the list is rebuilt once focus leaves it.
  if (list.dataset.key === key || (focused && /^(text|number)$/.test(focused.type)) || list.querySelector("[aria-invalid]")) return;
  list.dataset.key = key;
  const rows = Object.keys(NET_KINDS).flatMap((kind) => (net[kind] || []).map((item, i) => netRow(kind, i, item)));
  list.replaceChildren(...rows);
  if (focused && focused.id && $(focused.id)) $(focused.id).focus();
  $("net-empty").hidden = rows.length > 0;
  $("net-found").replaceChildren(...netFound.map(foundRow));
}

function wireNetwork() {
  $("set-net-discover").onchange = () => saveNet((n) => (n.discover = $("set-net-discover").checked));
  $("net-add").onclick = () => {
    const kind = $("net-family").value;
    saveNet((n) => n[kind].push({ ...NET_KINDS[kind].blank }));
  };
  $("net-search").onclick = async () => {
    const button = $("net-search");
    if (button.hasAttribute("aria-disabled")) return;
    button.setAttribute("aria-disabled", "true");
    setText($("net-status"), "Searching");
    const found = await call("scan_network");
    button.removeAttribute("aria-disabled");
    netFound = found || [];
    setText($("net-status"), found === undefined ? "" : found.length ? "Found " + plural(found.length, "light") : "Nothing found");
    scheduleRender();
  };
}

const PROTOCOLS = { adalight: "Adalight", tpm2: "TPM2", keyboard_visualizer: "Keyboard Visualizer" };
const STRIP_BLANK = { name: "LED strip", port: "", baud: 115200, leds: 60, protocol: "adalight" };
let stripPorts = [];

function saveStrips(change) {
  if (!state.snap) return;
  const settings = structuredClone(state.snap.settings);
  change(settings.led_strips);
  return call("set_settings", { settings });
}

async function loadPorts() {
  const ports = await invoke("serial_ports").catch(() => null);
  if (!ports) return;
  stripPorts = ports;
  $("strip-ports").replaceChildren(...ports.map((p) => el("option", { value: p })));
}

function stripCheck(key, input, protocol) {
  const text = input.value.trim();
  if (key === "name" || key === "protocol") return { value: text };
  if (key === "port") {
    if (/^COM\d{1,3}$/i.test(text)) return { value: text.toUpperCase() };
    if (/^udp:\S+$/i.test(text) && protocol === "keyboard_visualizer") return { value: "udp:" + text.slice(4) };
    return { problem: protocol === "keyboard_visualizer" ? "Enter a serial port such as COM3, or udp:address." : "Enter a serial port, such as COM3." };
  }
  const [min, max] = key === "leds" ? [1, 8192] : [1, protocol === "keyboard_visualizer" && /^udp:/i.test(input.port.value) ? 65535 : 4000000];
  const n = Number(text);
  return text !== "" && Number.isInteger(n) && n >= min && n <= max ? { value: n } : { problem: `Enter a whole number from ${min} to ${max}.` };
}

function stripRow(index, item) {
  const id = "strip-" + index;
  const problem = el("p", { className: "problem", id: id + "-problem", hidden: true });
  const inputs = {
    name: el("input", { type: "text", autocomplete: "off", spellcheck: false }),
    port: el("input", { type: "text", autocomplete: "off", spellcheck: false, placeholder: "COM3" }),
    protocol: el("select", {}, ...Object.entries(PROTOCOLS).map(([value, textContent]) => el("option", { value, textContent }))),
    baud: el("input", { type: "number", min: 1, step: 1 }),
    leds: el("input", { type: "number", min: 1, max: 8192, step: 1 }),
  };
  inputs.port.setAttribute("list", "strip-ports");
  inputs.port.onfocus = loadPorts;
  const labels = { name: "Name", port: "Port", protocol: "Protocol", baud: "Baud rate", leds: "LEDs" };
  const fields = Object.entries(inputs).map(([key, input]) => {
    input.id = `${id}-${key}`;
    input.port = inputs.port;
    input.value = item[key] ?? "";
    const check = () => {
      const out = stripCheck(key, input, inputs.protocol.value);
      flag(input, out.problem || "");
      return out;
    };
    explain(input, problem);
    input.oninput = check;
    input.onchange = () => {
      check();
      if (key === "protocol") ["port", "baud"].forEach((k) => inputs[k].oninput());
      // The card is saved whole, once every field is right.
      const all = Object.entries(inputs).map(([k, i]) => [k, stripCheck(k, i, inputs.protocol.value)]);
      if (all.some(([, out]) => "problem" in out)) return;
      saveStrips((s) => (s[index] = Object.fromEntries(all.map(([k, out]) => [k, out.value]))));
    };
    revert(input, () => (input.value = item[key] ?? ""));
    return el("label", {}, labels[key], input);
  });
  const remove = el("button", { type: "button", textContent: "Remove", id: id + "-remove" });
  remove.onclick = () => saveStrips((s) => s.splice(index, 1));
  return el(
    "li",
    { className: "net-item" },
    el("div", { className: "split" }, el("span", { className: "name", textContent: item.name || "LED strip" }), el("span", { className: "meta", textContent: item.port || "No port" })),
    el("div", { className: "net-fields" }, ...fields),
    problem,
    el("div", { className: "actions" }, remove),
  );
}

function renderStrips(snap) {
  const strips = snap.settings.led_strips || [];
  const list = $("strip-list");
  const key = JSON.stringify(strips);
  const focused = list.contains(document.activeElement) ? document.activeElement : null;
  if (list.dataset.key === key || (focused && /^(text|number)$/.test(focused.type)) || list.querySelector("[aria-invalid]")) return;
  list.dataset.key = key;
  list.replaceChildren(...strips.map((item, i) => stripRow(i, item)));
  if (focused && focused.id && $(focused.id)) $(focused.id).focus();
  $("strip-empty").hidden = strips.length > 0;
}

function wireStrips() {
  loadPorts();
  $("strip-add").onclick = async () => {
    await loadPorts();
    const used = (state.snap.settings.led_strips || []).map((s) => s.port);
    const port = stripPorts.find((p) => !used.includes(p)) || "";
    saveStrips((s) => s.push({ ...STRIP_BLANK, port }));
  };
}

function renderSettings(snap) {
  for (const input of document.querySelectorAll("[data-setting]")) put(input, snap.settings[input.dataset.setting]);
  $("openrgb-server").hidden = !snap.settings.openrgb_enabled;
  renderNetwork(snap);
  renderStrips(snap);

  const list = $("backend-list");
  const key = JSON.stringify(snap.backends);
  if (list.dataset.key === key) return;
  list.dataset.key = key;
  list.replaceChildren(
    ...snap.backends.map((b) => {
      const status = el("span", { className: "state" });
      if (b.error) setState(status, "warn", "Error");
      else if (!b.active) setState(status, "off", "Off");
      else setState(status, "on", b.devices ? plural(b.devices, "device") : "On");
      const row = el("li", { className: "backend" }, el("span", { textContent: BACKENDS[b.name] || b.name }), status);
      if (b.error) row.append(el("span", { className: "problem", textContent: sentence(b.error) }));
      return row;
    }),
  );
}

function renderBar(snap) {
  put($("brightness"), snap.brightness);
  setText($("pause"), snap.paused ? "Resume" : "Pause");
  put($("view-snap"), view.snap);
  put($("view-grid"), view.grid);
  put($("view-labels"), view.labels);
  put($("view-shapes"), view.shapes);
  put($("view-keys"), view.keys);
  put($("view-zoom"), view.zoom);
  const device = findDevice(preview.selected);
  const rect = device ? rectOf(device).map((v, i) => "XYWH"[i] + " " + (v * 100).toFixed(1)) : [];
  setText($("readout"), device ? device.name + "  " + rect.join("  ") : "");
}

function settingValue(input) {
  if (input.type === "number") {
    const n = Number(input.value);
    return input.value !== "" && Number.isInteger(n) && n >= Number(input.min) && n <= Number(input.max) ? n : null;
  }
  if (input.type === "text") return input.value.trim() || null;
  return read(input);
}

function wirePanels() {
  const setting = (name, value) => state.snap && call("set_settings", { settings: { ...state.snap.settings, [name]: value } });
  pair($("brightness"), $("brightness-box"), 100, throttled((value) => call("set_brightness", { value })));
  pair($("set-fps"), $("set-fps-box"), 1, throttled((fps) => setting("fps", fps)));

  $("pause").onclick = () => state.snap && call("set_paused", { paused: !state.snap.paused });
  $("undo").onclick = () => step(-1);
  $("redo").onclick = () => step(1);
  $("view-snap").onchange = () => setView({ snap: $("view-snap").checked });
  $("view-grid").onchange = () => setView({ grid: Number($("view-grid").value), snap: true });
  $("view-labels").onchange = () => setView({ labels: $("view-labels").checked });
  $("view-shapes").onchange = () => setView({ shapes: $("view-shapes").checked });
  $("view-keys").onchange = () => setView({ keys: $("view-keys").checked });
  $("view-zoom").onchange = () => setView({ zoom: Number($("view-zoom").value) });
  $("reset-layout").onclick = () => call("reset_layout").then((out) => out !== undefined && note("Layout reset"));
  $("rescan").onclick = () => call("rescan").then((out) => out !== undefined && note("Rescanning devices"));
  $("detect").onclick = async () => {
    const button = $("detect");
    if (button.hasAttribute("aria-disabled")) return;
    button.setAttribute("aria-disabled", "true");
    setText(button, "Detecting");
    note("Detecting hardware");
    // The command only starts the pass. What it found comes with the snapshot that follows.
    const result = nextSnapshot(5000);
    const found = (await call("detect")) !== undefined && (await result);
    button.removeAttribute("aria-disabled");
    setText(button, "Detect now");
    // A device that came or went has been reported by now.
    if ($("st-note").textContent === "Detecting hardware") note(found ? "Nothing has changed" : "");
  };

  const search = $("effect-search");
  search.oninput = filterEffects;
  search.onkeydown = searchKey;
  wireLayers();
  $("effect-reset").onclick = () => currentInfo() && setEdited(currentInfo().default);
  $("effect-random").onclick = randomise;
  $("effect-copy").onclick = copyEffect;
  $("effect-paste").onclick = () => openJson("Paste an effect over this one, or edit it, then choose Apply.");

  const profile = $("profile-name");
  profile.oninput = () => {
    delete profile.dataset.replace;
    flag(profile, "");
  };
  $("profile-form").onsubmit = async (e) => {
    e.preventDefault();
    const name = profile.value.trim();
    if (!name) return flag(profile, "Enter a name for the profile.");
    if (state.snap.profiles.includes(name) && profile.dataset.replace !== name) {
      profile.dataset.replace = name;
      return flag(profile, `A profile named "${name}" already exists. Save again to replace it.`);
    }
    if ((await call("save_profile", { name })) === undefined) return;
    profile.value = "";
    profile.oninput();
    note("Saved profile " + name);
  };

  const name = $("name-input");
  name.oninput = () => flag(name, "");
  wireInfo();
  $("name-cancel").onclick = () => $("name-sheet").close();
  $("name-form").onsubmit = (e) => {
    const to = name.value.trim();
    const taken = to !== asking.own && state.snap.profiles.includes(to);
    flag(name, !to ? "Enter a name." : taken ? `A profile named "${to}" already exists.` : "");
    if (name.hasAttribute("aria-invalid")) return e.preventDefault();
    asking.done(to);
  };

  const json = $("json-text");
  json.oninput = () => flag(json, readEffect(json.value).problem || "");
  $("json-cancel").onclick = () => $("json-sheet").close();
  $("json-form").onsubmit = (e) => {
    const { effect, problem } = readEffect(json.value);
    flag(json, problem || "");
    if (problem) return e.preventDefault();
    setEdited(effect).then((out) => out !== undefined && note("Effect applied"));
  };

  wireNetwork();
  wireStrips();
  $("conflicts-refresh").onclick = refreshConflicts;
  $("conflicts-close").onclick = async () => {
    const report = await call("close_conflicts_now");
    if (report) note(report.text);
    if (report && report.closed.length + report.left.length) showClosed(report);
    refreshConflicts();
  };

  for (const input of document.querySelectorAll("[data-setting]")) {
    if (input.type === "range") continue;
    const check = () => {
      const value = settingValue(input);
      flag(input, value === null ? input.dataset.invalid : "");
      return value;
    };
    input.oninput = check;
    input.onchange = () => {
      const value = check();
      if (value !== null) setting(input.dataset.setting, value);
    };
  }
}

// The outcome of installing or removing what the modules need.
function showModules(outcome) {
  const line = $("modules-outcome");
  setText(line, outcome.text);
  line.className = outcome.ok ? "hint" : "problem";
  line.hidden = false;
  note(outcome.text);
}

// The last pass that closed or left anything, kept in view.
function showClosed(report) {
  const line = $("conflicts-last");
  if (!report || !report.text) return;
  setText(line, report.text + (report.at ? " at " + report.at : ""));
  line.hidden = false;
}

let conflictsAsked = 0;

async function refreshConflicts() {
  const asked = ++conflictsAsked;
  const found = await call("get_conflicts");
  if (!found || asked !== conflictsAsked) return;
  $("conflicts-empty").hidden = found.length > 0;
  $("conflict-list").replaceChildren(
    ...found.flatMap((app) => {
      const status = el("span", { className: "state" });
      if (app.running === false) setState(status, "off", "Not running");
      else if (app.closes) setState(status, "warn", "Running");
      else setState(status, "off", "Left alone");
      const row = el("li", { className: "backend" }, el("span", { textContent: app.name }), status);
      return [row, ...(app.exes || []).map(exeRow)];
    }),
  );
}

// One executable of a conflicting app, running or seen before, with whether FoxRGB closes it.
function exeRow(exe) {
  const id = "close-" + exe.name.toLowerCase().replace(/[^a-z0-9]/g, "-");
  const box = el("input", { type: "checkbox", className: "switch", id, checked: exe.closes });
  box.onchange = async () => {
    const keep = (state.snap.settings.keep_running || []).filter((n) => n.toLowerCase() !== exe.name.toLowerCase());
    if (!box.checked) keep.push(exe.name);
    await call("set_settings", { settings: { ...state.snap.settings, keep_running: keep } });
    refreshConflicts();
  };
  const label = el("label", { htmlFor: id, textContent: "Close " + exe.name, title: exe.path || "" });
  return el("li", { className: "backend exe" }, label, box);
}

const update = { info: null };

function showUpdate(info) {
  update.info = info;
  const button = $("update-open");
  setText(button, "Update to " + info.version);
  button.hidden = false;
  setText($("update-status"), "Version " + info.version + " available");
  setText($("update-title"), "FoxRGB " + info.version);
  setText($("update-notes"), info.notes || "No release notes.");
  if (info.auto) note("Updating to " + info.version + " soon. Open the update to put it off.");
}

async function wireUpdates() {
  const sheet = $("update-sheet");
  $("update-open").onclick = () => update.info && !sheet.open && sheet.showModal();
  $("update-later").onclick = () => {
    invoke("later_update");
    sheet.close();
    note("Update put off");
  };
  $("update-install").onclick = async () => {
    const button = $("update-install");
    if (button.hasAttribute("aria-disabled")) return;
    button.setAttribute("aria-disabled", "true");
    $("update-progress").hidden = false;
    setText($("update-progress"), "Downloading");
    try {
      await invoke("install_update");
    } catch (e) {
      setText($("update-progress"), sentence(String(e)));
      button.removeAttribute("aria-disabled");
    }
  };
  $("update-check").onclick = async () => {
    const status = $("update-status");
    setText(status, "Checking");
    try {
      const info = await invoke("check_update");
      if (info) showUpdate(info);
      else setText(status, "Up to date");
    } catch (e) {
      setText(status, sentence(String(e)));
    }
  };
  await listen("update", (e) => showUpdate(e.payload));
  await listen("apps", (e) => {
    state.apps = e.payload;
    scheduleRender();
  });
  await listen("update-progress", (e) => {
    const { done, total } = e.payload;
    const mb = (n) => (n / 1048576).toFixed(1) + " MB";
    $("update-progress").hidden = false;
    setText($("update-progress"), total ? Math.round((done / total) * 100) + "% of " + mb(total) : mb(done));
  });
  const known = await call("get_update");
  if (known) showUpdate(known);
}
