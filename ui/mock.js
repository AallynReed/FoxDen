// In-memory stand-in for the Tauri side, so index.html runs in a plain browser.
(function () {
  if (window.__TAURI__) return;

  const copy = (v) => JSON.parse(JSON.stringify(v));
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  const frac = (v) => v - Math.floor(v);

  function grid(id, name, kind, rect, cols, rows) {
    const leds = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) leds.push([(c + 0.5) / cols, (r + 0.5) / rows]);
    }
    return { id, name, kind, backend: "virtual", rect, grid: [cols, rows], leds, enabled: true, brightness: 1 };
  }

  const layout = () => [
    grid("virtual:strip", "Light strip", "strip", [0.04, 0.06, 0.92, 0.03], 60, 1),
    grid("virtual:ram1", "Memory 1", "memory", [0.05, 0.16, 0.03, 0.34], 1, 10),
    grid("virtual:ram2", "Memory 2", "memory", [0.1, 0.16, 0.03, 0.34], 1, 10),
    grid("virtual:board", "Motherboard", "motherboard", [0.16, 0.16, 0.2, 0.12], 6, 2),
    grid("virtual:gpu", "Graphics card", "gpu", [0.16, 0.38, 0.2, 0.06], 12, 1),
    grid("virtual:panel", "Light panel", "other", [0.42, 0.16, 0.54, 0.4], 48, 30),
    grid("virtual:keyboard", "Keyboard", "keyboard", [0.04, 0.64, 0.5, 0.28], 22, 6),
    grid("virtual:mouse", "Mouse", "mouse", [0.6, 0.66, 0.07, 0.24], 3, 5),
  ];

  const ORANGE = [255, 106, 0];
  const VIOLET = [150, 60, 255];
  const speed = { name: "speed", label: "Speed", control: { type: "range", min: 0, max: 5, step: 0.05 } };
  const scale = { name: "scale", label: "Scale", control: { type: "range", min: 0.2, max: 4, step: 0.05 } };
  const colour = (name, label) => ({ name, label, control: { type: "color" } });

  const catalog = [
    {
      label: "Rainbow wave",
      default: { kind: "rainbow", speed: 1, scale: 1, angle: 0 },
      params: [speed, scale, { name: "angle", label: "Angle", control: { type: "range", min: 0, max: 360, step: 1 } }],
    },
    {
      label: "Gradient",
      default: { kind: "gradient", from: ORANGE, to: VIOLET, speed: 1, scale: 1 },
      params: [colour("from", "From"), colour("to", "To"), speed, scale],
    },
    {
      label: "Wave",
      default: { kind: "wave", color: ORANGE, speed: 1, direction: "right", mirror: false },
      params: [
        colour("color", "Colour"),
        speed,
        { name: "direction", label: "Direction", control: { type: "choice", options: ["left", "right", "up", "down"] } },
        { name: "mirror", label: "Mirror", control: { type: "toggle" } },
      ],
    },
    { label: "Breathing", default: { kind: "breathing", color: ORANGE, speed: 1 }, params: [colour("color", "Colour"), speed] },
    { label: "Static", default: { kind: "static", color: ORANGE }, params: [colour("color", "Colour")] },
  ];

  const state = {
    devices: layout(),
    effect: copy(catalog[0].default),
    brightness: 1,
    paused: false,
    profiles: new Map([
      ["Desk", { effect: copy(catalog[1].default), brightness: 0.8 }],
      ["Night", { effect: { kind: "static", color: [255, 60, 0] }, brightness: 0.25 }],
    ]),
    active: null,
    settings: {
      fps: 60,
      openrgb_enabled: false,
      openrgb_host: "127.0.0.1",
      openrgb_port: 6742,
      native_drivers: false,
      show_virtual: true,
      close_to_tray: true,
      start_minimized: false,
      autostart: false,
    },
  };

  function hsv(h) {
    const x = frac(h) * 6;
    const f = frac(x) * 255;
    switch (Math.floor(x)) {
      case 0: return [255, f, 0];
      case 1: return [255 - f, 255, 0];
      case 2: return [0, 255, f];
      case 3: return [0, 255 - f, 255];
      case 4: return [f, 0, 255];
      default: return [255, 0, 255 - f];
    }
  }

  const mul = (c, k) => [c[0] * k, c[1] * k, c[2] * k];
  const mix = (a, b, k) => [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];

  function sample(e, x, y, t) {
    switch (e.kind) {
      case "static":
        return e.color;
      case "breathing":
        return mul(e.color, 0.5 - 0.5 * Math.cos(t * e.speed * 2));
      case "rainbow": {
        const a = (e.angle * Math.PI) / 180;
        return hsv((x * Math.cos(a) + y * Math.sin(a)) * e.scale - t * e.speed * 0.2);
      }
      case "gradient":
        return mix(e.from, e.to, 1 - Math.abs(frac(x * e.scale - t * e.speed * 0.2) * 2 - 1));
      case "wave": {
        let d = e.direction === "left" || e.direction === "right" ? x : y;
        if (e.mirror) d = Math.abs(d - 0.5) * 2;
        const sign = e.direction === "right" || e.direction === "down" ? -1 : 1;
        return mul(e.color, (0.5 + 0.5 * Math.cos((d * 1.5 + sign * t * e.speed * 0.3) * Math.PI * 2)) ** 2);
      }
      default:
        return [0, 0, 0];
    }
  }

  const visible = () => (state.settings.show_virtual ? state.devices : []);

  function snapshot() {
    const s = state.settings;
    const refused = `cannot connect to ${s.openrgb_host}:${s.openrgb_port}: connection refused`;
    return copy({
      devices: visible(),
      effect: state.effect,
      brightness: state.brightness,
      paused: state.paused,
      catalog,
      profiles: [...state.profiles.keys()].sort(),
      active_profile: state.active,
      settings: s,
      backends: [
        { name: "virtual", active: s.show_virtual, devices: visible().length, error: null },
        { name: "openrgb", active: false, devices: 0, error: s.openrgb_enabled ? refused : null },
        { name: "hid", active: s.native_drivers, devices: 0, error: null },
        { name: "audio", active: false, devices: 0, error: null },
        { name: "screen", active: false, devices: 0, error: null },
      ],
    });
  }

  const commands = {
    get_snapshot: () => snapshot(),
    set_effect({ effect }) {
      if (!catalog.some((c) => c.default.kind === effect.kind)) throw `unknown effect ${effect.kind}`;
      state.effect = effect;
      state.active = null;
    },
    set_brightness({ value }) {
      state.brightness = clamp(value, 0, 1);
      state.active = null;
    },
    set_paused({ paused }) {
      state.paused = paused;
    },
    set_device({ id, enabled, brightness, rect }) {
      const device = state.devices.find((d) => d.id === id);
      if (!device) throw `no device ${id}`;
      Object.assign(device, { enabled, brightness: clamp(brightness, 0, 1), rect });
    },
    reset_layout() {
      const base = layout();
      state.devices.forEach((d, i) => (d.rect = base[i].rect));
    },
    save_profile({ name }) {
      if (!name.trim()) throw "profile name is empty";
      state.profiles.set(name.trim(), copy({ effect: state.effect, brightness: state.brightness }));
      state.active = name.trim();
    },
    load_profile({ name }) {
      const profile = state.profiles.get(name);
      if (!profile) throw `no profile named ${name}`;
      state.effect = copy(profile.effect);
      state.brightness = profile.brightness;
      state.active = name;
    },
    delete_profile({ name }) {
      if (!state.profiles.delete(name)) throw `no profile named ${name}`;
      if (state.active === name) state.active = null;
    },
    set_settings({ settings }) {
      state.settings = settings;
      start();
    },
    rescan() {},
  };

  const listeners = { frame: new Set(), snapshot: new Set() };
  const emit = (event, payload) => listeners[event].forEach((handler) => handler({ event, payload }));

  function invoke(name, args) {
    return new Promise((resolve, reject) => {
      if (!commands[name]) return reject(`unknown command ${name}`);
      try {
        const out = commands[name](copy(args || {}));
        if (name !== "get_snapshot") emit("snapshot", snapshot());
        resolve(out === undefined ? null : out);
      } catch (e) {
        reject(String(e));
      }
    });
  }

  function listen(event, handler) {
    listeners[event].add(handler);
    return Promise.resolve(() => listeners[event].delete(handler));
  }

  const began = performance.now();
  let timer = 0;

  function tick() {
    const t = (performance.now() - began) / 1000;
    const frames = visible().map((d) => {
      const [x, y, w, h] = d.rect;
      const k = state.paused || !d.enabled ? 0 : state.brightness * d.brightness;
      const colors = d.leds.map(([lx, ly]) => {
        const c = sample(state.effect, x + lx * w, y + ly * h, t);
        return [Math.round(c[0] * k), Math.round(c[1] * k), Math.round(c[2] * k)];
      });
      return { device: d.id, colors };
    });
    emit("frame", frames);
  }

  function start() {
    clearInterval(timer);
    timer = setInterval(tick, 1000 / clamp(state.settings.fps, 10, 60));
  }

  start();
  window.__TAURI__ = { core: { invoke }, event: { listen } };
})();
