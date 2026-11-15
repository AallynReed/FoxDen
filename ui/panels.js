const BACKENDS = { openrgb: "OpenRGB", hid: "Native HID", audio: "Audio capture", screen: "Screen capture" };
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
let pendingEffect = null;
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
function colourField(param, id) {
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
    queueEffect(param.name, rgb);
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

function queueEffect(name, value) {
  if (!state.snap) return;
  if (!pendingEffect) {
    pendingEffect = { ...state.snap.effect };
    requestAnimationFrame(() => {
      const effect = pendingEffect;
      pendingEffect = null;
      call("set_effect", { effect });
    });
  }
  pendingEffect[name] = value;
}

function currentInfo() {
  const snap = state.snap;
  return snap ? snap.catalog.find((c) => c.default.kind === snap.effect.kind) : undefined;
}

function chooseEffect(kind) {
  const info = state.snap.catalog.find((c) => c.default.kind === kind);
  if (info) call("set_effect", { effect: lastUsed[kind] || info.default });
}

function randomise() {
  const info = currentInfo();
  if (!info) return;
  const effect = { ...state.snap.effect };
  for (const p of info.params) {
    const c = p.control;
    if (c.type === "range") {
      const steps = Math.round((c.max - c.min) / c.step);
      effect[p.name] = Number((c.min + Math.round(Math.random() * steps) * c.step).toFixed(4));
    } else if (c.type === "color") effect[p.name] = [0, 1, 2].map(() => Math.floor(Math.random() * 256));
    else if (c.type === "toggle") effect[p.name] = Math.random() < 0.5;
    else if (c.options && c.options.length) effect[p.name] = c.options[Math.floor(Math.random() * c.options.length)];
  }
  call("set_effect", { effect });
}

// The effect in use stays listed and says so, so that the list always has a row to tab to.
function filterEffects() {
  const q = $("effect-search").value.trim().toLowerCase();
  const kind = state.snap && state.snap.effect.kind;
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

function paramRow(param) {
  const control = param.control;
  const id = "param-" + param.name;
  if (control.type === "color") return colourField(param, id);
  const row = el("div", { className: "field" }, el("label", { htmlFor: id, textContent: param.label }));
  let input;
  if (control.type === "range") {
    input = el("input", { type: "range", min: control.min, max: control.max, step: control.step });
    const box = el("input", { type: "number", className: "exact" });
    box.setAttribute("aria-label", param.label);
    pair(input, box, 1, (value) => queueEffect(param.name, value), $("params-problem"));
    row.classList.add("range");
    row.append(input, box);
  } else {
    if (control.type === "toggle") input = el("input", { type: "checkbox", className: "switch" });
    else input = el("select", {}, ...(control.options || []).map((o) => el("option", { value: o, textContent: words(o) })));
    input.oninput = () => queueEffect(param.name, read(input));
    row.append(input);
  }
  input.id = id;
  input.dataset.param = param.name;
  return row;
}

function renderEffects(snap) {
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
  for (const radio of list.querySelectorAll("input")) radio.checked = radio.value === snap.effect.kind;
  filterEffects();

  const box = $("params");
  const info = currentInfo();
  const params = info ? info.params : [];
  const paramKey = snap.effect.kind + JSON.stringify(params);
  if (box.dataset.key !== paramKey) {
    box.dataset.key = paramKey;
    if (picker) picker.close();
    box.replaceChildren(...params.map(paramRow));
    $("params-problem").hidden = true;
  }
  for (const input of box.querySelectorAll("[data-param]")) put(input, snap.effect[input.dataset.param]);
  $("params-empty").hidden = params.length > 0;

  // The list scrolls on its own, so keep the chosen effect in view.
  if (!$("panel-effects").hidden && list.dataset.kind !== snap.effect.kind) {
    list.dataset.kind = snap.effect.kind;
    const checked = list.querySelector("input:checked");
    if (checked) checked.parentNode.scrollIntoView({ block: "nearest" });
  }
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
    problem,
    el("div", { className: "actions" }, reset),
  );
  const row = el("li", { className: "device" }, el("div", { className: "device-head" }, name, status, on), details);
  row.parts = { name, status, on, level, meta, place, details };
  return row;
}

function renderDevices(snap) {
  const list = $("device-list");
  const key = snap.devices.map((d) => d.id).join("|");
  if (list.dataset.key !== key) {
    list.dataset.key = key;
    list.replaceChildren(...snap.devices.map(deviceRow));
  }
  snap.devices.forEach((device, i) => {
    const row = list.children[i];
    const { name, status, on, level, meta, place, details } = row.parts;
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
  return row;
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
  text.value = JSON.stringify(state.snap.effect, null, 2);
  flag(text, "");
  $("json-sheet").showModal();
  text.select();
}

async function copyEffect() {
  try {
    await navigator.clipboard.writeText(JSON.stringify(state.snap.effect, null, 2));
    note("Effect copied as JSON");
  } catch {
    openJson("Copying was blocked. Press Ctrl+C to copy the selected text.");
  }
}

function renderSettings(snap) {
  for (const input of document.querySelectorAll("[data-setting]")) put(input, snap.settings[input.dataset.setting]);
  $("openrgb-server").hidden = !snap.settings.openrgb_enabled;

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
  $("effect-reset").onclick = () => currentInfo() && call("set_effect", { effect: currentInfo().default });
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
    call("set_effect", { effect }).then((out) => out !== undefined && note("Effect applied"));
  };

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
    ...found.map((app) => {
      const status = el("span", { className: "state" });
      if (app.closes) setState(status, "warn", "Running");
      else setState(status, "off", "Left alone");
      return el("li", { className: "backend" }, el("span", { textContent: app.name }), status);
    }),
  );
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
  await listen("update-progress", (e) => {
    const { done, total } = e.payload;
    const mb = (n) => (n / 1048576).toFixed(1) + " MB";
    $("update-progress").hidden = false;
    setText($("update-progress"), total ? Math.round((done / total) * 100) + "% of " + mb(total) : mb(done));
  });
  const known = await call("get_update");
  if (known) showUpdate(known);
}
