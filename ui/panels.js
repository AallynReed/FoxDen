const BACKENDS = { virtual: "Virtual", openrgb: "OpenRGB", hid: "Native HID", audio: "Audio capture", screen: "Screen capture" };

const lastUsed = {};
let held = null;
let pendingEffect = null;
let rowCount = 0;

document.addEventListener("pointerdown", (e) => (held = e.target), true);
document.addEventListener("pointerup", () => (held = null), true);
document.addEventListener("pointercancel", () => (held = null), true);
document.addEventListener("focusout", () => scheduleRender());

const toHex = (c) => "#" + c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
const fromHex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const plural = (n, word) => n.toLocaleString("en-GB") + " " + word + (n === 1 ? "" : "s");
const words = (s) => (s.charAt(0).toUpperCase() + s.slice(1)).replace(/_/g, " ");

function el(tag, props, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

function read(input) {
  if (input.type === "checkbox") return input.checked;
  if (input.type === "color") return fromHex(input.value);
  if (input.type === "range" || input.type === "number") return Number(input.value);
  return input.value;
}

// A control the user is dragging or typing in keeps its own value.
function busy(input) {
  return input === held || (input === document.activeElement && /^(text|number|color)$/.test(input.type));
}

function put(input, value) {
  if (value === undefined || busy(input)) return;
  if (input.type === "checkbox") input.checked = Boolean(value);
  else input.value = input.type === "color" ? toHex(value) : value;
  if (input.show) input.show();
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

function chooseEffect(kind) {
  const info = state.snap.catalog.find((c) => c.default.kind === kind);
  if (info) call("set_effect", { effect: lastUsed[kind] || info.default });
}

function paramRow(param) {
  const control = param.control;
  const id = "param-" + param.name;
  let input;
  if (control.type === "range") input = el("input", { type: "range", min: control.min, max: control.max, step: control.step });
  else if (control.type === "color") input = el("input", { type: "color" });
  else if (control.type === "toggle") input = el("input", { type: "checkbox", className: "switch" });
  else input = el("select", {}, ...(control.options || []).map((o) => el("option", { value: o, textContent: words(o) })));
  input.id = id;
  input.dataset.param = param.name;

  const row = el("div", { className: "field" }, el("label", { htmlFor: id, textContent: param.label }));
  if (control.type === "range") {
    const digits = Math.min(3, (String(control.step).split(".")[1] || "").length);
    const out = el("output");
    input.show = () => setText(out, Number(input.value).toFixed(digits));
    row.classList.add("range");
    row.append(out);
  }
  row.append(input);
  input.oninput = () => {
    if (input.show) input.show();
    queueEffect(param.name, read(input));
  };
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
        radio.onchange = () => chooseEffect(radio.value);
        return el("label", { className: "pick" }, radio, el("span", { textContent: info.label }));
      }),
    );
  }
  for (const radio of list.querySelectorAll("input")) radio.checked = radio.value === snap.effect.kind;

  const box = $("params");
  const info = snap.catalog.find((c) => c.default.kind === snap.effect.kind);
  const params = info ? info.params : [];
  const paramKey = snap.effect.kind + JSON.stringify(params);
  if (box.dataset.key !== paramKey) {
    box.dataset.key = paramKey;
    box.replaceChildren(...params.map(paramRow));
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

function deviceRow(device) {
  const n = ++rowCount;
  const name = el("button", { type: "button", className: "name" });
  const on = el("input", { type: "checkbox", className: "switch", id: "device-on-" + n });
  const level = el("input", { type: "range", min: 0, max: 1, step: 0.01, id: "device-level-" + n });
  const out = el("output");
  const meta = el("span", { className: "meta" });
  const send = throttled((enabled, brightness) => {
    const current = findDevice(device.id);
    if (current) call("set_device", { id: device.id, enabled, brightness, rect: current.rect });
  });
  const changed = () => send(on.checked, Number(level.value));
  name.onclick = () => selectDevice(device.id);
  on.onchange = changed;
  level.show = () => setText(out, Math.round(level.value * 100) + "%");
  level.oninput = () => {
    level.show();
    changed();
  };
  const row = el(
    "li",
    { className: "device" },
    el("div", { className: "device-head" }, name, el("label", { htmlFor: on.id, textContent: "Enabled" }), on),
    meta,
    el("div", { className: "level" }, el("label", { htmlFor: level.id, textContent: "Brightness" }), level, out),
  );
  row.parts = { name, on, level, meta };
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
    const { name, on, level, meta } = row.parts;
    row.classList.toggle("selected", device.id === preview.selected);
    setText(name, device.name);
    setText(meta, (BACKENDS[device.backend] || device.backend) + " · " + plural(device.leds.length, "LED"));
    put(on, device.enabled);
    put(level, device.brightness);
  });
  $("devices-empty").hidden = snap.devices.length > 0;

  const selected = preview.selected || "";
  if (!$("panel-devices").hidden && list.dataset.selected !== selected) {
    list.dataset.selected = selected;
    const row = list.querySelector(".selected");
    if (row) row.scrollIntoView({ block: "nearest" });
  }
}

function renderProfiles(snap) {
  const list = $("profile-list");
  const key = JSON.stringify(snap.profiles);
  if (list.dataset.key !== key) {
    list.dataset.key = key;
    list.replaceChildren(
      ...snap.profiles.map((name) => {
        const load = el("button", { type: "button", textContent: "Load" });
        const remove = el("button", { type: "button", textContent: "Delete" });
        load.setAttribute("aria-label", "Load " + name);
        remove.setAttribute("aria-label", "Delete " + name);
        load.onclick = () => call("load_profile", { name });
        remove.onclick = () => call("delete_profile", { name });
        return el(
          "li",
          { className: "profile" },
          el("span", { className: "name", textContent: name }),
          el("span", { className: "tag", textContent: "Active" }),
          load,
          remove,
        );
      }),
    );
  }
  snap.profiles.forEach((name, i) => list.children[i].classList.toggle("active", name === snap.active_profile));
  $("profiles-empty").hidden = snap.profiles.length > 0;
}

function renderSettings(snap) {
  for (const input of document.querySelectorAll("[data-setting]")) put(input, snap.settings[input.dataset.setting]);

  const list = $("backend-list");
  const key = JSON.stringify(snap.backends);
  if (list.dataset.key === key) return;
  list.dataset.key = key;
  list.replaceChildren(
    ...snap.backends.map((b) => {
      const status = b.error ? "Error" : !b.active ? "Off" : b.devices ? plural(b.devices, "device") : "On";
      const row = el(
        "li",
        { className: "backend" },
        el("span", { textContent: BACKENDS[b.name] || b.name }),
        el("span", { className: "status", textContent: status }),
      );
      if (b.error) row.append(el("span", { className: "problem", textContent: b.error }));
      return row;
    }),
  );
}

function renderBar(snap) {
  put($("brightness"), snap.brightness);
  setText($("pause"), snap.paused ? "Resume" : "Pause");
  const leds = snap.devices.reduce((n, d) => n + d.leds.length, 0);
  const parts = [plural(snap.devices.length, "device"), plural(leds, "LED")];
  if (snap.active_profile) parts.push(snap.active_profile);
  if (snap.paused) parts.push("paused");
  setText($("summary"), parts.join(" · "));
}

function settingValue(input) {
  if (input.type === "number") {
    const n = Number(input.value);
    return Number.isInteger(n) && n >= Number(input.min) && n <= Number(input.max) ? n : null;
  }
  if (input.type === "text") return input.value.trim() || null;
  return read(input);
}

function wirePanels() {
  const brightness = $("brightness");
  const sendBrightness = throttled((value) => call("set_brightness", { value }));
  brightness.show = () => setText($("brightness-out"), Math.round(brightness.value * 100) + "%");
  brightness.oninput = () => {
    brightness.show();
    sendBrightness(Number(brightness.value));
  };

  $("pause").onclick = () => state.snap && call("set_paused", { paused: !state.snap.paused });
  $("reset-layout").onclick = () => call("reset_layout");
  $("rescan").onclick = () => call("rescan");

  $("profile-form").onsubmit = async (e) => {
    e.preventDefault();
    const input = $("profile-name");
    const name = input.value.trim();
    if (!name) return showError("Enter a name for the profile.");
    if ((await call("save_profile", { name })) !== undefined) input.value = "";
  };

  const fps = $("set-fps");
  fps.show = () => setText($("set-fps-out"), fps.value + " fps");
  fps.oninput = fps.show;

  for (const input of document.querySelectorAll("[data-setting]")) {
    input.onchange = () => {
      if (!state.snap) return;
      const value = settingValue(input);
      if (value === null) return showError(input.dataset.invalid);
      call("set_settings", { settings: { ...state.snap.settings, [input.dataset.setting]: value } });
    };
  }
}
