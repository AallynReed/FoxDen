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
  const bands = () => [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map((frequency) => ({ enabled: true, filter: "PeakingEQ", frequency, gain_db: 0, q: 1.414 }));

  // ---- Mixer ----

  const mix = {
    output: OUTPUTS[1],
    park: "",
    channels: CHANNELS.map((channel) => ({ channel, source: "", loopback: channel !== "chatCapture", apps: [], on: false, volume: 1, muted: false, eq_on: false, eq: bands() })),
    master: 1,
    master_muted: false,
    chat_mix: 0,
    running: true,
  };
  mix.channels[0] = { ...mix.channels[0], apps: ["cs2.exe", "RocketLeague.exe"], on: true };
  mix.channels[1] = { ...mix.channels[1], apps: ["Discord.exe"], on: true };
  mix.channels[2] = { ...mix.channels[2], apps: ["Spotify.exe", "chrome.exe"], on: true };
  mix.channels[4] = { ...mix.channels[4], source: INPUTS[0], on: true };
  let running = true;
  let error = null;

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
  const sink = () => mix.park || OUTPUTS.find((o) => o !== mix.output && o !== OUTPUTS[0]);

  function appStates() {
    const out = {};
    for (const c of mix.channels) {
      for (const exe of c.apps) {
        const app = APPS.find((a) => lower(a.exe) === lower(exe));
        out[lower(exe)] = !app ? { state: "idle" } : exe === "chrome.exe" ? { state: "elsewhere", output: OUTPUTS[1] } : { state: "playing" };
      }
    }
    return out;
  }

  function mixView() {
    return {
      config: copy(mix),
      running,
      error,
      outputs: OUTPUTS,
      inputs: INPUTS,
      apps: APPS,
      app_states: running ? appStates() : {},
      parked_on: running ? sink() : null,
      park_error: null,
      filters: FILTERS.map(([id, label, gain]) => ({ id, label, gain })),
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

  Object.assign(mock.commands, {
    get_mixer: () => mixView(),
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
        if (c.apps.length && !target.apps.length && !target.source) target.on = true;
        target.apps = c.apps;
      } else if (c.kind === "volume") ch(c.channel).volume = clamp(c.volume, 0, 1);
      else if (c.kind === "mute") ch(c.channel).muted = c.muted;
      else if (c.kind === "master") mix.master = clamp(c.volume, 0, 1);
      else if (c.kind === "master_mute") mix.master_muted = c.muted;
      else if (c.kind === "chat_mix") mix.chat_mix = clamp(c.balance, -1, 1);
      else if (c.kind === "eq") Object.assign(ch(c.channel), { eq_on: c.enabled, eq: c.bands.slice(0, 10) });
      else if (c.kind === "run") {
        if (c.on) play();
        else {
          running = mix.running = false;
          error = null;
        }
      }
      return mixView();
    },
    get_mixer_levels() {
      const t = performance.now() / 1000;
      return mix.channels.map((c, i) => (running && c.on && (c.source || c.apps.length) && !c.muted ? c.volume * (0.35 + 0.3 * Math.abs(Math.sin(t * (2 + i)))) : 0));
    },
  });

})();
