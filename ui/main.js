const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Range params are [min, max, step]; string params are colours.
const EFFECTS = {
  rainbow: { label: "Rainbow wave", params: { speed: [0, 5, 0.05], scale: [0.2, 4, 0.05], angle: [0, 360, 1] } },
  gradient: { label: "Gradient", params: { from: "color", to: "color", speed: [0, 5, 0.05], scale: [0.2, 4, 0.05] } },
  cycle: { label: "Spectrum cycle", params: { speed: [0, 5, 0.05] } },
  breathing: { label: "Breathing", params: { color: "color", speed: [0, 5, 0.05] } },
  static: { label: "Static", params: { color: "color" } },
};

const DEFAULTS = {
  rainbow: { speed: 1, scale: 1, angle: 0 },
  gradient: { from: [255, 106, 0], to: [150, 60, 255], speed: 1, scale: 1 },
  cycle: { speed: 1 },
  breathing: { color: [255, 106, 0], speed: 1 },
  static: { color: [255, 106, 0] },
};

const $ = (id) => document.getElementById(id);
const toHex = (c) => "#" + c.map((v) => v.toString(16).padStart(2, "0")).join("");
const fromHex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));

const settings = structuredClone(DEFAULTS);
let kind = "rainbow";
let devices = [];
let frames = new Map();

function pushEffect() {
  invoke("set_effect", { effect: { kind, ...settings[kind] } });
}

function buildParams() {
  const root = $("params");
  root.replaceChildren();
  for (const [name, spec] of Object.entries(EFFECTS[kind].params)) {
    const label = document.createElement("label");
    const input = document.createElement("input");
    label.textContent = name;
    if (spec === "color") {
      input.type = "color";
      input.value = toHex(settings[kind][name]);
      input.oninput = () => {
        settings[kind][name] = fromHex(input.value);
        pushEffect();
      };
    } else {
      const out = document.createElement("output");
      [input.type, input.min, input.max, input.step] = ["range", ...spec];
      input.value = settings[kind][name];
      out.textContent = input.value;
      input.oninput = () => {
        settings[kind][name] = Number(input.value);
        out.textContent = input.value;
        pushEffect();
      };
      label.append(out);
    }
    root.append(label, input);
  }
}

const canvas = $("canvas");
const ctx = canvas.getContext("2d");

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const { width, height } = canvas.getBoundingClientRect();
  if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  for (const device of devices) {
    const [rx, ry, rw, rh] = device.rect;
    const [x, y, w, h] = [rx * width, ry * height, rw * width, rh * height];
    const [cols, rows] = device.grid;
    const [cw, ch] = [w / cols, h / rows];
    const gap = Math.min(cw, ch) * 0.12;

    ctx.shadowBlur = 0;
    ctx.fillStyle = "#151922";
    ctx.strokeStyle = "#262c3a";
    ctx.beginPath();
    ctx.roundRect(x - 6, y - 6, w + 12, h + 12, 8);
    ctx.fill();
    ctx.stroke();

    ctx.fillStyle = "#8a93a6";
    ctx.font = "11px 'Segoe UI', system-ui, sans-serif";
    ctx.fillText(device.name, x - 6, y - 12);

    const colors = frames.get(device.id);
    if (!colors) continue;
    device.leds.forEach(([lx, ly], i) => {
      const css = `rgb(${colors[i].join(",")})`;
      ctx.fillStyle = css;
      ctx.shadowColor = css;
      ctx.shadowBlur = 10;
      ctx.beginPath();
      ctx.roundRect(x + lx * w - cw / 2 + gap, y + ly * h - ch / 2 + gap, cw - gap * 2, ch - gap * 2, 3);
      ctx.fill();
    });
  }
  requestAnimationFrame(draw);
}

async function init() {
  const select = $("effect");
  for (const [id, { label }] of Object.entries(EFFECTS)) select.add(new Option(label, id));

  const [effect, brightness] = await invoke("get_state");
  ({ kind } = effect);
  Object.assign(settings[kind], effect);
  delete settings[kind].kind;
  select.value = kind;
  buildParams();

  select.onchange = () => {
    kind = select.value;
    buildParams();
    pushEffect();
  };

  const slider = $("brightness");
  const showBrightness = () => ($("brightness-out").textContent = Math.round(slider.value * 100) + "%");
  slider.value = brightness;
  showBrightness();
  slider.oninput = () => {
    showBrightness();
    invoke("set_brightness", { value: Number(slider.value) });
  };

  devices = await invoke("devices");
  await listen("frame", ({ payload }) => {
    frames = new Map(payload.map((f) => [f.device, f.colors]));
  });
  requestAnimationFrame(draw);
}

init();
