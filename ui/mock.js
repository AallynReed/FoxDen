// In-memory stand-in for the Tauri side, so index.html runs in a plain browser.
(function () {
  if (window.__TAURI__) return;

  const copy = (v) => JSON.parse(JSON.stringify(v));
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  const sat = (v) => clamp(v, 0, 1);
  const frac = (v) => v - Math.floor(v);
  const smooth = (k) => k * k * (3 - 2 * k);
  const lerp = (a, b, k) => a + (b - a) * k;
  const mul = (c, k) => c.map((v) => v * sat(k));
  const mix = (a, b, k) => a.map((v, i) => lerp(v, b[i], sat(k)));

  function grid(id, name, kind, backend, rect, cols, rows) {
    const leds = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) leds.push([(c + 0.5) / cols, (r + 0.5) / rows]);
    }
    return { id, name, kind, backend, rect, grid: [cols, rows], leds };
  }

  // What the native drivers and an OpenRGB server would show.
  const hardware = [
    grid("hid:board", "Motherboard ARGB header", "motherboard", "hid", [0.06, 0.12, 0.52, 0.05], 24, 1),
    grid("hid:gpu", "Graphics card", "gpu", "hid", [0.06, 0.3, 0.4, 0.05], 12, 1),
    grid("hid:ram0", "RAM 1", "memory", "hid", [0.68, 0.1, 0.03, 0.36], 1, 10),
    grid("hid:ram1", "RAM 2", "memory", "hid", [0.76, 0.1, 0.03, 0.36], 1, 10),
    grid("hid:keyboard", "TKL keyboard", "keyboard", "hid", [0.06, 0.58, 0.54, 0.32], 17, 6),
    grid("hid:mouse", "Mouse", "mouse", "hid", [0.7, 0.62, 0.06, 0.22], 1, 2),
    grid("openrgb:0", "Corsair Vengeance RGB DDR5", "memory", "openrgb", [0.84, 0.1, 0.03, 0.36], 1, 10),
    grid("openrgb:1", "Corsair Vengeance RGB DDR5", "memory", "openrgb", [0.91, 0.1, 0.03, 0.36], 1, 10),
    grid("openrgb:2", "ASUS Radeon RX 9070 XT", "gpu", "openrgb", [0.5, 0.3, 0.14, 0.05], 5, 1),
    grid("hid:1462:7d99", "MSI B760M Gaming Plus WiFi", "motherboard", "hid", [0.06, 0.2, 0.3, 0.05], 10, 1),
    grid("hid:046d:c539", "Logitech G502 Lightspeed", "mouse", "hid", [0.8, 0.62, 0.06, 0.22], 1, 2),
    grid("hid:046d:c547", "Logitech G515 Lightspeed TKL", "keyboard", "hid", [0.06, 0.4, 0.4, 0.13], 17, 6),
  ];

  const ORANGE = [255, 106, 0];
  const VIOLET = [150, 60, 255];
  const CYAN = [0, 200, 255];
  const NAVY = [0, 12, 40];
  const range = (name, label, min, max, step) => ({ name, label, control: { type: "range", min, max, step } });
  const colour = (name, label) => ({ name, label, control: { type: "color" } });
  const toggle = (name, label) => ({ name, label, control: { type: "toggle" } });
  const entry = (label, value, ...params) => ({ label, default: value, params });
  const speed = range("speed", "Speed", 0, 5, 0.05);
  const scale = range("scale", "Scale", 0.2, 4, 0.05);
  const angle = range("angle", "Angle", 0, 360, 1);
  const width = range("width", "Width", 0.05, 1, 0.05);
  const sensitivity = range("sensitivity", "Sensitivity", 0.2, 5, 0.05);
  const main = colour("color", "Colour");
  const background = colour("background", "Background");

  // Same entries, in the same order, as Effect::catalog in crates/foxrgb-core/src/effect.rs.
  const catalog = [
    entry("Rainbow wave", { kind: "rainbow", speed: 1, scale: 1, angle: 0 }, speed, scale, angle),
    entry("Gradient", { kind: "gradient", from: ORANGE, to: VIOLET, speed: 1, scale: 1 }, colour("from", "From"), colour("to", "To"), speed, scale),
    entry("Spectrum cycle", { kind: "cycle", speed: 1 }, speed),
    entry("Breathing", { kind: "breathing", color: ORANGE, speed: 1 }, main, speed),
    entry("Static", { kind: "static", color: ORANGE }, main),
    entry("Wave", { kind: "wave", color: CYAN, background: NAVY, speed: 1, width: 0.4, angle: 0 }, main, background, speed, width, angle),
    entry(
      "Comet",
      { kind: "comet", color: ORANGE, background: [0, 0, 0], speed: 1, length: 0.5, angle: 0 },
      main,
      background,
      speed,
      range("length", "Tail length", 0.05, 1, 0.05),
      angle,
    ),
    entry(
      "Sparkle",
      { kind: "sparkle", color: [255, 255, 255], background: NAVY, speed: 1, density: 0.3 },
      main,
      background,
      speed,
      range("density", "Density", 0, 1, 0.05),
    ),
    entry("Plasma", { kind: "plasma", speed: 1, scale: 1 }, speed, scale),
    entry("Fire", { kind: "fire", speed: 1, intensity: 1 }, speed, range("intensity", "Intensity", 0.2, 2, 0.05)),
    entry(
      "Ripple",
      { kind: "ripple", color: CYAN, background: NAVY, speed: 1, width: 0.4, rings: 3 },
      main,
      background,
      speed,
      width,
      range("rings", "Rings", 1, 8, 0.5),
    ),
    entry(
      "Radial rainbow",
      { kind: "radial_rainbow", speed: 1, repeats: 1, twist: 0 },
      speed,
      range("repeats", "Repeats", 1, 6, 1),
      range("twist", "Twist", 0, 4, 0.1),
    ),
    entry(
      "Colour shift",
      { kind: "color_shift", first: ORANGE, second: VIOLET, third: CYAN, use_third: true, speed: 1 },
      colour("first", "First colour"),
      colour("second", "Second colour"),
      colour("third", "Third colour"),
      toggle("use_third", "Use third colour"),
      speed,
    ),
    entry(
      "Audio spectrum",
      { kind: "audio_spectrum", color: ORANGE, rainbow: true, sensitivity: 1 },
      toggle("rainbow", "Colour by band"),
      colour("color", "Bar colour"),
      sensitivity,
    ),
    entry(
      "Audio pulse",
      { kind: "audio_pulse", color: ORANGE, sensitivity: 1, floor: 0.1 },
      main,
      sensitivity,
      range("floor", "Idle brightness", 0, 1, 0.05),
    ),
    entry(
      "Screen ambience",
      { kind: "screen_ambience", saturation: 1, brightness: 1 },
      range("saturation", "Saturation", 0, 3, 0.05),
      range("brightness", "Brightness", 0.2, 3, 0.05),
    ),
    entry(
      "Key ripple",
      { kind: "key_ripple", color: ORANGE, background: NAVY, speed: 1.5, width: 0.05, fade: 1 },
      main,
      background,
      speed,
      range("width", "Width", 0.01, 0.3, 0.01),
      range("fade", "Fade", 0.2, 4, 0.1),
    ),
  ];

  const state = {
    // Per device: enabled, brightness and, once moved, rect.
    tweaks: {},
    effect: copy(catalog[0].default),
    brightness: 1,
    paused: false,
    profiles: new Map([
      ["Desk", { effect: copy(catalog[1].default), brightness: 0.8 }],
      ["Night", { effect: { kind: "static", color: [255, 60, 0] }, brightness: 0.25 }],
    ]),
    active: null,
    keyboard: true,
    settings: {
      fps: 30,
      openrgb_enabled: false,
      openrgb_host: "127.0.0.1",
      openrgb_port: 6742,
      native_drivers: true,
      close_to_tray: true,
      start_minimized: false,
      autostart: false,
      auto_detect: true,
      auto_update: true,
      lighting_module: true,
      fans_module: true,
      close_conflicts: true,
      send_reports: false,
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

  // Distance along the direction `angle`, in degrees.
  function along(x, y, angle) {
    const a = (angle * Math.PI) / 180;
    return x * Math.cos(a) + y * Math.sin(a);
  }

  // 1 in the middle of a repeating band, 0 outside it.
  const band = (phase, width) => smooth(sat(1 - Math.abs(frac(phase) - 0.5) / Math.max(width * 0.5, 1e-3)));

  // Repeatable pseudo-random value in 0..1, the same arithmetic as the Rust side.
  function hash(x, y, n) {
    let h = Math.imul(x, 0x9e3779b1) ^ Math.imul(y, 0x85ebca6b) ^ Math.imul(n, 0xc2b2ae35);
    h ^= h >>> 15;
    h = Math.imul(h, 0x2c1b3c6d);
    h ^= h >>> 12;
    h = Math.imul(h, 0x297a2d39);
    h ^= h >>> 15;
    return (h >>> 8) / 16777216;
  }

  function noise(x, y) {
    const x0 = Math.floor(x);
    const y0 = Math.floor(y);
    const row = (yi) => lerp(hash(x0, yi, 0), hash(x0 + 1, yi, 0), smooth(x - x0));
    return lerp(row(y0), row(y0 + 1), smooth(y - y0));
  }

  // Stands in for music: sixteen bands that rise and fall on a beat.
  function audio(t) {
    const beat = 0.55 + 0.45 * Math.abs(Math.sin(t * 3.2));
    const bands = Array.from({ length: 16 }, (_, i) => sat((0.3 + 0.7 * Math.abs(Math.sin(t * (0.9 + i * 0.23) + i * 1.7))) * beat * (1 - i / 28)));
    return { bands, level: bands.reduce((a, b) => a + b) / 16 };
  }

  // Stands in for the desktop: slow bands of colour, brighter towards the bottom.
  const screen = (x, y, t) => mul(hsv(0.6 + 0.15 * Math.sin(t * 0.4 + x * 3) + y * 0.2), 0.3 + 0.7 * y);

  function sample(e, x, y, t, sound) {
    switch (e.kind) {
      case "static":
        return e.color;
      case "breathing":
        return mul(e.color, 0.5 - 0.5 * Math.cos(t * e.speed * 2));
      case "cycle":
        return hsv(t * e.speed * 0.1);
      case "rainbow":
        return hsv(along(x, y, e.angle) * e.scale - t * e.speed * 0.2);
      case "gradient":
        return mix(e.from, e.to, 1 - Math.abs(frac(x * e.scale - t * e.speed * 0.2) * 2 - 1));
      case "wave":
        return mix(e.background, e.color, band(along(x, y, e.angle) - t * e.speed * 0.2, e.width));
      case "comet": {
        const k = sat(1 - frac(t * e.speed * 0.3 - along(x, y, e.angle)) / Math.max(e.length, 1e-3));
        return mix(e.background, e.color, k * k);
      }
      case "sparkle": {
        const cx = Math.floor(x * 256);
        const cy = Math.floor(y * 256);
        const time = t * e.speed * 2 + hash(cx, cy, -1);
        if (hash(cx, cy, Math.floor(time)) >= e.density) return e.background;
        return mix(e.background, e.color, Math.sin(Math.PI * frac(time)));
      }
      case "plasma": {
        const px = x * e.scale * 6;
        const py = y * e.scale * 6;
        const tt = t * e.speed;
        const v =
          Math.sin(px + tt) +
          Math.sin(py * 1.3 - tt * 0.8) +
          Math.sin((px + py) * 0.7 + tt * 0.6) +
          Math.sin(Math.hypot(px - 3, py - 3) * 1.5 - tt);
        return hsv(v * 0.125 + tt * 0.02);
      }
      case "fire": {
        const rise = t * e.speed * 2;
        const n = 0.65 * noise(x * 6, y * 4 + rise) + 0.35 * noise(x * 13, y * 9 + rise * 1.8);
        const heat = 0.7 * n + 0.45 - ((1 - y) * 1.1) / Math.max(e.intensity, 0.05);
        return [sat(heat * 3) * 255, sat(heat * 3 - 1) * 255, sat(heat * 3 - 2) * 255];
      }
      case "ripple":
        return mix(e.background, e.color, band(Math.hypot(x - 0.5, y - 0.5) * e.rings - t * e.speed * 0.3, e.width));
      case "radial_rainbow": {
        const turn = Math.atan2(y - 0.5, x - 0.5) / (Math.PI * 2);
        return hsv(turn * Math.max(Math.round(e.repeats), 1) + Math.hypot(x - 0.5, y - 0.5) * e.twist - t * e.speed * 0.2);
      }
      case "color_shift": {
        const colors = e.use_third ? [e.first, e.second, e.third] : [e.first, e.second];
        const p = frac(t * e.speed * 0.1) * colors.length;
        const i = Math.floor(p);
        return mix(colors[i % colors.length], colors[(i + 1) % colors.length], smooth(p - i));
      }
      case "audio_spectrum": {
        const i = Math.min(Math.floor(sat(x) * 16), 15);
        const height = sat(sound.bands[i] * e.sensitivity);
        return mul(e.rainbow ? hsv((i / 16) * 0.85) : e.color, height * 17 - (1 - y) * 16);
      }
      case "audio_pulse":
        return mul(e.color, lerp(e.floor, 1, sat(sound.level * e.sensitivity)));
      case "screen_ambience": {
        const c = screen(x, y, t);
        const grey = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        return c.map((v) => (grey + (v - grey) * e.saturation) * e.brightness);
      }
      case "key_ripple": {
        // Pretend presses: one a second, walking across the keyboard.
        const n = Math.floor(t);
        const age = t - n;
        const px = 0.3 + ((n * 0.37) % 0.4);
        const off = Math.hypot(x - px, y - 0.75) - age * e.speed * 0.3;
        const ring = smooth(sat(1 - Math.abs(off) / Math.max(e.width * 0.5, 1e-3)));
        return mix(e.background, e.color, ring * sat(1 - age / Math.max(e.fade, 0.05)));
      }
      default:
        return [0, 0, 0];
    }
  }

  // The pretend OpenRGB server listens on the default address only.
  function openrgb() {
    const s = state.settings;
    return s.openrgb_enabled && /^(127\.0\.0\.1|localhost)$/.test(s.openrgb_host) && s.openrgb_port === 6742;
  }

  function devices() {
    const on = { openrgb: openrgb(), hid: state.settings.native_drivers };
    return hardware
      .filter((d) => on[d.backend] && (state.keyboard || d.id !== "hid:046d:c547"))
      .map((d) => ({ ...d, enabled: true, brightness: 1, ...state.tweaks[d.id] }));
  }

  function detected() {
    const native = (id, name, kind, note) => {
      const on = state.settings.native_drivers;
      return { id, name, kind, bus: "usb", route: "native", state: on ? "active" : "available", note: on ? "Driven by the native driver." : note };
    };
    const served = (id, name, kind, bus) => {
      const on = openrgb();
      const note = on ? "Driven through the OpenRGB server." : "No native driver yet. The OpenRGB server can drive it.";
      return { id, name, kind, bus, route: "openrgb", state: on ? "active" : "available", note };
    };
    const plain = (id, name, kind, note) => ({ id, name, kind, bus: "usb", route: "none", state: "unsupported", note });
    const found = [
      native("usb:1462:7d99", "MSI B760M Gaming Plus WiFi", "motherboard", "Native drivers are off."),
      native("usb:046d:c539", "Logitech G502 Lightspeed", "mouse", "Native drivers are off."),
      native("usb:046d:c547", "Logitech G515 Lightspeed TKL", "keyboard", "The native driver is unfinished: the keyboard can take its lighting back."),
      served("memory:CMH32GX5M2B6000Z30", "Corsair Vengeance RGB DDR5, 2 modules", "memory", "memory"),
      served("gpu:1002:7550", "ASUS Radeon RX 9070 XT", "gpu", "gpu"),
      plain("usb:1038:2202", "SteelSeries Arctis Nova 7", "headset", "Has no lighting that software can control."),
      plain("usb:03f0:0b8b", "HyperX SoloCast", "microphone", "Its status light cannot be controlled."),
      {
        id: "network:openrgb",
        name: "OpenRGB server",
        kind: "server",
        bus: "network",
        route: "openrgb",
        state: openrgb() ? "active" : "available",
        note: openrgb() ? "Connected on 127.0.0.1:6742, 3 devices." : "Answering on 127.0.0.1:6742.",
      },
    ];
    return found.filter((d) => state.keyboard || d.id !== "usb:046d:c547");
  }

  function snapshot() {
    const s = state.settings;
    const list = devices();
    const count = (backend) => list.filter((d) => d.backend === backend).length;
    const refused = `cannot connect to ${s.openrgb_host}:${s.openrgb_port}: connection refused`;
    const backends = [
      { name: "openrgb", active: openrgb(), devices: count("openrgb"), error: s.openrgb_enabled && !openrgb() ? refused : null },
      { name: "hid", active: s.native_drivers, devices: count("hid"), error: null },
    ];
    // The captures only show up while an effect uses them.
    if (state.effect.kind.startsWith("audio")) backends.push({ name: "audio", active: true, devices: 0, error: null });
    if (state.effect.kind === "screen_ambience") backends.push({ name: "screen", active: true, devices: 0, error: null });
    return copy({
      devices: list,
      effect: state.effect,
      brightness: state.brightness,
      paused: state.paused,
      catalog,
      profiles: [...state.profiles.keys()].sort(),
      active_profile: state.active,
      settings: s,
      backends,
      detected: detected(),
    });
  }

  // An edit that no longer matches the active profile clears it.
  function checkProfile() {
    const live = { effect: state.effect, brightness: state.brightness };
    if (state.active && !same(state.profiles.get(state.active), live)) state.active = null;
  }

  const commands = {
    get_snapshot: () => snapshot(),
    set_effect({ effect }) {
      if (!catalog.some((c) => c.default.kind === effect.kind)) throw `unknown effect ${effect.kind}`;
      state.effect = effect;
      checkProfile();
    },
    set_brightness({ value }) {
      state.brightness = clamp(value, 0, 1);
      checkProfile();
    },
    set_paused({ paused }) {
      state.paused = paused;
    },
    set_device({ id, enabled, brightness, rect }) {
      if (!devices().some((d) => d.id === id)) throw `unknown device ${id}`;
      if (rect.some((v) => !Number.isFinite(v)) || rect[2] <= 0 || rect[3] <= 0) throw "invalid device rectangle";
      state.tweaks[id] = { enabled, brightness: clamp(brightness, 0, 1), rect };
    },
    reset_layout() {
      for (const tweak of Object.values(state.tweaks)) delete tweak.rect;
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
      const on = (s) => s.lighting_module || s.fans_module;
      const changed = on(settings) !== on(state.settings);
      state.settings = { ...settings, fps: clamp(settings.fps, 10, 60) };
      start();
      const text = on(settings) ? "install PawnIO, then install the FoxRGB service" : "remove the FoxRGB service, then remove PawnIO";
      if (changed) setTimeout(() => emit("modules", { ok: true, text: "Development build, so nothing was changed. Would: " + text }), 300);
    },
    get_conflicts() {
      return [{ name: "SignalRGB", area: "both", closes: state.settings.close_conflicts }, { name: "Fan Control", area: "fans", closes: state.settings.fans_module }];
    },
    close_conflicts_now() {
      return { closed: ["SignalRGB"], left: [], text: "Closed SignalRGB", at: "09:14" };
    },
    last_conflicts() {
      return { closed: ["SignalRGB"], left: [], text: "Closed SignalRGB", at: "09:14" };
    },
    check_update() {
      const info = { version: "0.4.0", notes: "Mock release notes.", auto: state.settings.auto_update };
      setTimeout(() => emit("update", info));
      return info;
    },
    install_update() {
      throw "development builds do not install updates";
    },
    later_update() {},
    get_update() {
      return null;
    },
    open_logs() {
      window.open("logs.html", "logs", "width=760,height=480");
    },
    rescan() {},
    // Returns at once, as the app does. The pass then finds the Logitech keyboard unplugged or plugged back in.
    detect() {
      setTimeout(() => {
        state.keyboard = !state.keyboard;
        emit("snapshot", snapshot());
      }, 400);
    },
  };

  const listeners = { frame: new Set(), snapshot: new Set() };
  const emit = (event, payload) => (listeners[event] || []).forEach((handler) => handler({ event, payload }));

  function invoke(name, args) {
    return new Promise((resolve, reject) => {
      if (!commands[name]) return reject(`unknown command ${name}`);
      try {
        const out = commands[name](copy(args || {}));
        if (!name.startsWith("get_") && name !== "detect") emit("snapshot", snapshot());
        resolve(out === undefined ? null : out);
      } catch (e) {
        reject(String(e));
      }
    });
  }

  function listen(event, handler) {
    (listeners[event] ||= new Set()).add(handler);
    return Promise.resolve(() => listeners[event].delete(handler));
  }

  const began = performance.now();
  let timer = 0;

  function tick() {
    const t = (performance.now() - began) / 1000;
    const sound = audio(t);
    const frames = devices().map((d) => {
      const [x, y, w, h] = d.rect;
      const k = state.paused || !d.enabled ? 0 : state.brightness * d.brightness;
      const colors = d.leds.map(([lx, ly]) => sample(state.effect, x + lx * w, y + ly * h, t, sound).map((v) => Math.round(clamp(v, 0, 255) * k)));
      return { device: d.id, colors };
    });
    emit("frame", frames);
  }

  function start() {
    clearInterval(timer);
    timer = setInterval(tick, 1000 / state.settings.fps);
  }

  start();
  window.__TAURI__ = { core: { invoke }, event: { listen } };
  // Other mock files add their commands here.
  window.__foxMock = { commands, emit };
})();
