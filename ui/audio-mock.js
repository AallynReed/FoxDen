// Stand-in for the audio commands when the page runs in a plain browser.
(function () {
  const mock = window.__foxMock;
  if (!mock) return;

  const CHANNELS = ["game", "chatRender", "media", "aux", "chatCapture"];
  const FILTERS = [
    ["ByPass", "Bypass", false],
    ["LowPass", "Low pass", false],
    ["HighPass", "High pass", false],
    ["BandPassPeakQ", "Band pass (Q)", false],
    ["BandPassPeak0dB", "Band pass (0 dB)", false],
    ["NotchFilter", "Notch", false],
    ["AllPass", "All pass", false],
    ["PeakingEQ", "Peaking", true],
    ["LowShelving", "Low shelf", true],
    ["HighShelving", "High shelf", true],
  ];
  const OUTPUTS = ["Speakers (Realtek(R) Audio)", "Headphones (USB Audio)", "Q27 (AMD High Definition Audio Device)", "CABLE Input (Virtual Cable)"];
  const INPUTS = ["Microphone (USB Audio)", "Line In (Realtek(R) Audio)"];
  const APPS = [
    { exe: "Discord.exe", active: true },
    { exe: "cs2.exe", active: true },
    { exe: "Spotify.exe", active: true },
    { exe: "chrome.exe", active: false },
    { exe: "obs64.exe", active: false },
  ];
  const copy = (v) => JSON.parse(JSON.stringify(v));
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  const proc = (over = {}) => {
    const base = {
      denoise: { on: false, strength: 1 },
      agc: { on: false, target_db: -20, max_gain_db: 12, max_cut_db: 12, speed_db: 6, floor_db: -50 },
      comp: { on: false, threshold_db: -18, ratio: 3, attack_ms: 5, release_ms: 120, knee_db: 6, makeup_db: 0, auto_makeup: false },
      deesser: { on: false, frequency: 6500, amount_db: 6 },
      limiter: { on: false, ceiling_db: -1, release_ms: 80 },
    };
    for (const k in over) Object.assign(base[k], over[k]);
    return base;
  };
  const bands = () => [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map((frequency) => ({ enabled: true, filter: "PeakingEQ", frequency, gain_db: 0, q: 1.414 }));

  // ---- Mixer ----

  const mix = {
    output: OUTPUTS[1],
    park: "",
    channels: CHANNELS.map((channel) => ({ channel, source: "", loopback: channel !== "chatCapture", apps: [], on: false, volume: 1, muted: false, eq_on: false, eq: bands(), processing: proc() })),
    master: 1,
    master_muted: false,
    chat_mix: 0,
    running: true,
    virtual_mic: false,
    gate_on: false,
    gate_db: -50,
    auto_assign: true,
    auto: ["spotify.exe"],
    no_auto: ["chrome.exe"],
    profiles: [],
    monitor: false,
    monitor_volume: 0.5,
    master_processing: proc(),
    game_side: ["game"],
  };
  mix.channels[0] = { ...mix.channels[0], apps: ["cs2.exe", "RocketLeague.exe"], on: true };
  mix.channels[1] = { ...mix.channels[1], apps: ["Discord.exe"], on: true };
  mix.channels[2] = { ...mix.channels[2], apps: ["Spotify.exe", "chrome.exe"], on: true };
  mix.channels[4] = { ...mix.channels[4], source: INPUTS[0], on: true };
  let running = true;
  let error = null;
  // ?vmic=1 starts with VB-CABLE installed by FoxDen.
  const vmic = { installed: false, ours: false, ready: false, name: null, playing: false, error: null };
  if (new URLSearchParams(location.search).get("vmic") === "1") Object.assign(vmic, { installed: true, ours: true, ready: true, name: "FoxDen VB-Cable (Microphone)" });

  // Rough shape of each band, enough to draw.
  function response(c, f) {
    if (!c.eq_on) return 0;
    let db = 0;
    for (const b of c.eq) {
      if (!b.enabled) continue;
      const x = Math.log2(f / b.frequency);
      if (b.filter === "PeakingEQ") db += b.gain_db * Math.exp(-x * x * b.q * b.q * 1.5);
      else if (b.filter === "LowShelving") db += b.gain_db / (1 + Math.exp(4 * x));
      else if (b.filter === "HighShelving") db += b.gain_db / (1 + Math.exp(-4 * x));
      else if (b.filter === "LowPass") db += x > 0 ? -12 * x : 0;
      else if (b.filter === "HighPass") db += x < 0 ? 12 * x : 0;
      else if (b.filter === "NotchFilter") db -= 24 * Math.exp(-x * x * 40);
    }
    return db;
  }

  const lower = (s) => s.toLowerCase();
  const gains = (g) => [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map((frequency, i) => ({ enabled: true, filter: "PeakingEQ", frequency, gain_db: g[i], q: 1.414 }));
  const band = (filter, frequency, gain_db, q) => ({ enabled: true, filter, frequency, gain_db, q });
  const BUILTINS = [
    ["Flat", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
    ["Bass boost", [6, 5, 4, 2, 0, 0, 0, 0, 0, 0]],
    ["Voice clarity", [-4, -3, -2, -1, 0, 2, 4, 4, 2, 0]],
    ["Treble", [0, 0, 0, 0, 0, 0, 2, 4, 5, 6]],
  ]
    .map(([name, g]) => ({ name, kind: "output", builtin: true, eq: { on: true, bands: gains(g) }, volume: null, gate: null, processing: null }))
    .concat([
      { name: "Clean voice", kind: "mic", builtin: true, eq: { on: true, bands: [band("HighPass", 80, 0, 0.707), band("PeakingEQ", 250, -2, 1), band("PeakingEQ", 3000, 2, 1)] }, volume: null, gate: { on: true, db: -50 }, processing: proc({ denoise: { on: true, strength: 0.8 }, comp: { on: true, threshold_db: -22, ratio: 2.5, attack_ms: 8, release_ms: 150, auto_makeup: true }, limiter: { on: true } }) },
      { name: "Broadcast", kind: "mic", builtin: true, eq: { on: true, bands: [band("HighPass", 70, 0, 0.707), band("LowShelving", 120, 3, 0.707), band("PeakingEQ", 400, -2, 1.2), band("PeakingEQ", 3500, 3, 0.9), band("HighShelving", 10000, 2, 0.707)] }, volume: null, gate: { on: true, db: -45 }, processing: proc({ denoise: { on: true, strength: 0.6 }, comp: { on: true, threshold_db: -26, ratio: 4, attack_ms: 3, release_ms: 100, auto_makeup: true }, deesser: { on: true }, limiter: { on: true, release_ms: 60 } }) },
      { name: "Streamer", kind: "mic", builtin: true, eq: { on: true, bands: [band("HighPass", 90, 0, 0.707), band("PeakingEQ", 300, -2, 1), band("PeakingEQ", 4000, 2, 0.9)] }, volume: null, gate: { on: true, db: -48 }, processing: proc({ denoise: { on: true }, agc: { on: true, target_db: -18 }, comp: { on: true, threshold_db: -20, auto_makeup: true }, deesser: { on: true, frequency: 7000, amount_db: 5 }, limiter: { on: true } }) },
      { name: "Noisy room", kind: "mic", builtin: true, eq: { on: true, bands: [band("HighPass", 120, 0, 0.707), band("PeakingEQ", 2500, 2, 1)] }, volume: null, gate: { on: true, db: -40 }, processing: proc({ denoise: { on: true }, agc: { on: true, max_gain_db: 9 }, comp: { on: true, threshold_db: -22, release_ms: 150, auto_makeup: true }, limiter: { on: true } }) },
      { name: "Noise gate", kind: "mic", builtin: true, eq: null, volume: null, gate: { on: true, db: -40 }, processing: null },
      { name: "Night mode", kind: "output", builtin: true, eq: null, volume: null, gate: null, processing: proc({ comp: { on: true, threshold_db: -32, ratio: 4, attack_ms: 10, release_ms: 250, knee_db: 10, auto_makeup: true }, limiter: { on: true, release_ms: 100 } }) },
      { name: "Clip guard", kind: "output", builtin: true, eq: null, volume: null, gate: null, processing: proc({ limiter: { on: true, release_ms: 60 } }) },
    ]);
  const sink = () => mix.park || OUTPUTS.find((o) => o !== mix.output && o !== OUTPUTS[0]);

  function appStates() {
    const out = {};
    for (const c of mix.channels) {
      for (const exe of c.apps) {
        const app = APPS.find((a) => lower(a.exe) === lower(exe));
        out[lower(exe)] = !app ? { state: "idle" } : exe === "chrome.exe" ? { state: "elsewhere", output: OUTPUTS[1] } : exe === "Discord.exe" ? { state: "own_output", output: OUTPUTS[1] } : { state: "playing" };
      }
    }
    return out;
  }

  function mixView() {
    const follow = prio.on && prio.mixer ? pick(prio.outputs) : null;
    return {
      config: copy(mix),
      playing_on: running ? (follow ? follow.name : mix.output) : null,
      following: !!follow,
      profiles: [...BUILTINS, ...mix.profiles],
      running,
      error,
      outputs: OUTPUTS,
      inputs: INPUTS,
      apps: APPS,
      app_states: running ? appStates() : {},
      parked_on: running ? sink() : null,
      park_error: null,
      filters: FILTERS.map(([id, label, gain]) => ({ id, label, gain })),
      vmic: { ...vmic, playing: vmic.ready && mix.virtual_mic && !!ch("chatCapture").source },
      curves: mix.channels.map((c) => Array.from({ length: 96 }, (_, i) => response(c, 20 * Math.pow(1000, i / 95)))),
    };
  }

  const ch = (id) => mix.channels.find((c) => c.channel === id);

  function specsError() {
    if (!mix.output) return "Pick an output to play on";
    const on = mix.channels.filter((c) => c.on && (c.source || c.apps.length));
    const loop = on.find((c) => c.loopback && c.source === mix.output);
    if (loop) return loop.channel + " would play back into itself; pick another source or output";
    if (!on.length) return "Turn on at least one channel with a source or an app";
    return null;
  }

  function play() {
    error = specsError();
    running = !error;
    mix.running = running;
    if (error) throw error;
  }

  function store(p) {
    if (BUILTINS.some((b) => b.name === p.name)) throw p.name + " is a built-in profile; pick another name";
    const i = mix.profiles.findIndex((x) => x.name === p.name);
    if (i >= 0) mix.profiles[i] = { ...p, builtin: false };
    else mix.profiles.push({ ...p, builtin: false });
  }

  Object.assign(mock.commands, {
    get_mixer: () => mixView(),
    get_priority: () => prioView(),
    set_priority({ priority }) {
      Object.assign(prio, copy(priority));
      return prioView();
    },
    sound_code: ({ profile }) => "foxden-sound:" + btoa(JSON.stringify({ v: 2, ...profile })).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
    sound_decode({ code }) {
      if (!code.startsWith("foxden-sound:")) throw "That is not a FoxDen sound code";
      let data;
      try {
        data = JSON.parse(atob(code.slice(13).replace(/-/g, "+").replace(/_/g, "/")));
      } catch {
        throw "The share code is damaged";
      }
      if (data.v !== 1 && data.v !== 2) throw "This code comes from a newer FoxDen; update to use it";
      return { name: String(data.name || "Shared profile").slice(0, 40), kind: data.kind === "mic" ? "mic" : "output", builtin: false, eq: data.eq || null, volume: data.volume ?? null, gate: data.gate || null, processing: data.processing || null };
    },
    mixer_set({ change: c }) {
      if (c.kind === "output") {
        if (mix.channels.some((x) => x.on && x.loopback && x.source === c.name)) throw "That output is a source; it would play back into itself";
        mix.output = c.name;
        if (running) play();
      } else if (c.kind === "route") {
        if (c.on && c.loopback && c.source === mix.output) throw c.channel + " would play back into itself; pick another source or output";
        Object.assign(ch(c.channel), { source: c.source, loopback: c.loopback, on: c.on });
        if (running) play();
      } else if (c.kind === "park") mix.park = c.name;
      else if (c.kind === "apps") {
        const taken = c.apps.map(lower);
        for (const x of mix.channels) if (x.channel !== c.channel) x.apps = x.apps.filter((a) => !taken.includes(lower(a)));
        const target = ch(c.channel);
        mix.no_auto.push(...target.apps.map(lower).filter((a) => !taken.includes(a)));
        mix.auto = mix.auto.filter((a) => !taken.includes(a) || target.apps.map(lower).includes(a));
        if (c.apps.length && !target.apps.length && !target.source) target.on = true;
        target.apps = c.apps;
      } else if (c.kind === "volume") ch(c.channel).volume = clamp(c.volume, 0, 1);
      else if (c.kind === "mute") ch(c.channel).muted = c.muted;
      else if (c.kind === "master") mix.master = clamp(c.volume, 0, 1);
      else if (c.kind === "master_mute") mix.master_muted = c.muted;
      else if (c.kind === "chat_mix") mix.chat_mix = clamp(c.balance, -1, 1);
      else if (c.kind === "virtual_mic") mix.virtual_mic = c.on && vmic.installed;
      else if (c.kind === "gate") Object.assign(mix, { gate_on: c.on, gate_db: clamp(c.threshold_db, -90, 0) });
      else if (c.kind === "eq") Object.assign(ch(c.channel), { eq_on: c.enabled, eq: c.bands.slice(0, 10) });
      else if (c.kind === "processing") {
        const p = copy(c.processing);
        if (c.channel !== "chatCapture") Object.assign(p, { denoise: proc().denoise, agc: proc().agc, deesser: proc().deesser });
        if (c.channel) ch(c.channel).processing = p;
        else mix.master_processing = p;
      } else if (c.kind === "monitor") Object.assign(mix, { monitor: c.on, monitor_volume: clamp(c.volume, 0, 1) });
      else if (c.kind === "mix_sides") mix.game_side = ["game", "media", "aux"].filter((x) => c.channels.includes(x));
      else if (c.kind === "auto_assign") mix.auto_assign = c.on;
      else if (c.kind === "allow_auto") mix.no_auto = mix.no_auto.filter((a) => a !== lower(c.exe));
      else if (c.kind === "apply_profile") {
        const mic = c.channel === "chatCapture";
        if ((c.profile.kind === "mic") !== mic) throw mic ? "That profile is for an output channel, not the mic" : "That is a mic profile; apply it to the mic";
        const x = ch(c.channel);
        if (c.profile.eq) Object.assign(x, { eq_on: c.profile.eq.on, eq: copy(c.profile.eq.bands) });
        if (c.profile.volume != null) x.volume = c.profile.volume;
        if (c.profile.gate) Object.assign(mix, { gate_on: c.profile.gate.on, gate_db: c.profile.gate.db });
        if (c.profile.processing) x.processing = copy(c.profile.processing);
      } else if (c.kind === "save_profile") {
        const x = ch(c.channel);
        const kind = c.channel === "chatCapture" ? "mic" : "output";
        store({ name: c.name, kind, eq: { on: x.eq_on, bands: copy(x.eq) }, volume: x.volume, gate: kind === "mic" ? { on: mix.gate_on, db: mix.gate_db } : null, processing: copy(x.processing) });
      } else if (c.kind === "add_profile") store(c.profile);
      else if (c.kind === "rename_profile") {
        const p = mix.profiles.find((x) => x.name === c.from);
        if (p) p.name = c.to;
      } else if (c.kind === "delete_profile") mix.profiles = mix.profiles.filter((x) => x.name !== c.name);
      else if (c.kind === "run") {
        if (c.on) play();
        else {
          running = mix.running = false;
          error = null;
        }
      }
      return mixView();
    },
    async virtual_mic_install() {
      await new Promise((r) => setTimeout(r, 900));
      Object.assign(vmic, { installed: true, ours: true, ready: true, name: "FoxDen VB-Cable (Microphone)" });
      mix.virtual_mic = true;
      return mixView();
    },
    virtual_mic_remove() {
      mix.virtual_mic = false;
      Object.assign(vmic, { installed: false, ours: false, ready: false, name: null });
      return mixView();
    },
    get_mixer_levels() {
      const t = performance.now() / 1000;
      const wave = (i) => Math.abs(Math.sin(t * (2 + i)));
      const peaks = mix.channels.map((c, i) => (running && c.on && (c.source || c.apps.length) && !c.muted ? c.volume * (0.35 + 0.3 * wave(i)) : 0));
      const gr = (p, i) => ({
        agc: p.agc.on ? 4 * Math.sin(t / 3 + i) : 0,
        comp: p.comp.on ? 6 * wave(i) : 0,
        deess: p.deesser.on ? 3 * wave(i + 1) * wave(i + 2) : 0,
        limit: p.limiter.on ? Math.max(0, 3 * Math.sin(t * 1.7 + i)) : 0,
      });
      return { peaks, reduction: [...mix.channels.map((c, i) => gr(c.processing, i)), gr(mix.master_processing, 5)] };
    },
    get_volumes: () => volumesView(),
    set_app_volume({ exe, endpoint, volume, muted }) {
      const a = SESSIONS.find((x) => x.exe === exe && x.endpoint === endpoint);
      if (!a) throw exe + " is no longer playing";
      if (volume != null) a.volume = clamp(volume, 0, 1);
      if (muted != null) a.muted = muted;
      return volumesView();
    },
    set_device_volume({ id, volume, muted }) {
      const d = ENDPOINTS.find((x) => x.id === id);
      if (!d) throw "That device is gone";
      if (volume != null) d.volume = clamp(volume, 0, 1);
      if (muted != null) d.muted = muted;
      return volumesView();
    },
    make_default_device({ id }) {
      const d = ENDPOINTS.find((x) => x.id === id);
      if (!d || d.note) throw "FoxDen's virtual mic endpoints are not made default here";
      for (const x of ENDPOINTS) if (x.capture === d.capture) x.default = x === d;
      return volumesView();
    },
  });

  // ---- Apps and Devices panels ----

  const ENDPOINTS = [
    { id: "hp", name: OUTPUTS[1], capture: false, default: true, volume: 0.62, muted: false },
    { id: "spk", name: OUTPUTS[0], capture: false, default: false, volume: 0.4, muted: false },
    { id: "q27", name: OUTPUTS[2], capture: false, default: false, volume: 1, muted: true, note: "Parking output, muted while the mixer runs" },
    { id: "feed", name: "FoxDen Mic feed (VB-Audio Virtual Cable)", capture: false, default: false, volume: 1, muted: false, note: "FoxDen virtual mic feed" },
    { id: "solo", name: "Microphone (4- HyperX SoloCast)", capture: true, default: true, volume: 0.8, muted: false },
    { id: "hpmic", name: INPUTS[0], capture: true, default: false, volume: 0.7, muted: false },
  ];
  const SESSIONS = [
    { exe: "Discord.exe", endpoint: "hp", volume: 1, muted: false, active: true },
    { exe: "cs2.exe", endpoint: "q27", volume: 0.8, muted: false, active: true },
    { exe: "Spotify.exe", endpoint: "q27", volume: 0.55, muted: false, active: true },
    { exe: "chrome.exe", endpoint: "hp", volume: 1, muted: true, active: false },
    { exe: "explorer.exe", endpoint: "spk", volume: 1, muted: false, active: false },
  ];

  function volumesView() {
    const t = performance.now() / 1000;
    const name = (id) => (ENDPOINTS.find((d) => d.id === id) || {}).name || "";
    return {
      apps: SESSIONS.map((a, i) => ({ ...a, device: name(a.endpoint), peak: a.active && !a.muted ? a.volume * (0.3 + 0.4 * Math.abs(Math.sin(t * (1.5 + i)))) : 0 })),
      devices: ENDPOINTS.map((d, i) => ({ ...d, note: d.note || null, peak: d.muted || d.note ? 0 : d.volume * (d.capture ? 0.2 : 0.5) * Math.abs(Math.sin(t * (1 + i))) })),
    };
  }

  // ---- Device priority: ?headset=cycle switches the headset off and on every 6 s ----

  const DEVICES = {
    out: [
      { id: "hp", name: OUTPUTS[1], usb: true },
      { id: "spk", name: OUTPUTS[0] },
      { id: "q27", name: OUTPUTS[2] },
    ],
    in: [
      { id: "solo", name: "Microphone (4- HyperX SoloCast)" },
      { id: "hpmic", name: INPUTS[0], usb: true },
    ],
  };
  const prio = {
    on: true,
    mixer: true,
    windows: false,
    outputs: [DEVICES.out[0], DEVICES.out[1]].map(({ id, name }) => ({ id, name })),
    inputs: [DEVICES.in[0]].map(({ id, name }) => ({ id, name })),
  };
  let headsetOn = true;
  const statusOf = (d) => (!d ? "unplugged" : d.usb && !headsetOn ? "off" : "available");
  const all = () => [...DEVICES.out, ...DEVICES.in];
  const pick = (list) => list.map((e) => all().find((d) => d.id === e.id)).find((d) => statusOf(d) === "available") || null;

  function rows(list) {
    const chosen = pick(list);
    return list.map((e) => {
      const d = all().find((x) => x.id === e.id);
      return { id: e.id, name: e.name, status: statusOf(d), chosen: !!chosen && chosen.id === e.id };
    });
  }

  function prioView() {
    const entry = ({ id, name }) => ({ id, name });
    return { config: copy(prio), outputs: rows(prio.outputs), inputs: rows(prio.inputs), choices_out: DEVICES.out.map(entry), choices_in: DEVICES.in.map(entry) };
  }

  if (new URLSearchParams(location.search).get("headset") === "cycle") {
    setInterval(() => {
      headsetOn = !headsetOn;
      mock.emit("audio-priority", prioView());
    }, 6000);
  }

  // ?tab=audio opens that tab; ?dial=1 pretends a headset's ChatMix dial is turning.
  const query = new URLSearchParams(location.search);
  let dial = null;
  mock.commands.headset_dial = () => dial;
  if (query.get("dial") === "1") {
    dial = { game: 100, chat: 100 };
    setInterval(() => {
      const t = performance.now() / 4000;
      const v = Math.round(Math.sin(t) * 100);
      dial = { game: v > 0 ? 100 - v : 100, chat: v < 0 ? 100 + v : 100 };
      mock.emit("chatmix", dial);
    }, 250);
  }
  const tab = query.get("tab");
  if (tab) addEventListener("load", () => document.getElementById("tab-" + tab)?.click());
  // ?eq=Game opens that channel's EQ, ?preset=Treble picks one and ?more=1 opens Advanced, for screenshots.
  addEventListener("load", () =>
    setTimeout(() => {
      if (query.get("eq")) document.querySelector('[aria-label="Edit ' + query.get("eq") + ' EQ"]')?.click();
      if (query.get("preset")) [...document.querySelectorAll(".aud-seg button")].find((b) => b.textContent === query.get("preset"))?.click();
      if (query.get("more")) document.querySelector(".aud-more")?.setAttribute("open", "");
      if (query.get("prio")) document.querySelector(".aud-priobtn")?.click();
      if (query.get("fx")) document.querySelector('[aria-label="Edit ' + query.get("fx") + ' processing"]')?.click();
    }, 300),
  );
})();
