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
  const OUTPUTS = ["Speakers (Realtek(R) Audio)", "Headphones (Arctis Nova 7)", "SteelSeries Sonar - Gaming", "SteelSeries Sonar - Chat", "SteelSeries Sonar - Media"];
  const INPUTS = ["Microphone (Arctis Nova 7)", "SteelSeries Sonar - Microphone", "Line In (Realtek(R) Audio)"];
  const copy = (v) => JSON.parse(JSON.stringify(v));
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  const bands = () => [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map((frequency) => ({ enabled: true, filter: "PeakingEQ", frequency, gain_db: 0, q: 1.414 }));

  // ---- Sonar ----

  const vm = (volume, muted = false) => ({ volume, muted });
  const sonar = {
    mode: "classic",
    classic: { masters: { classic: vm(0.9), stream: {} }, devices: {} },
    streamer: { masters: { classic: null, stream: { streaming: vm(1), monitoring: vm(0.7) } }, devices: {} },
    chat_mix: { balance: -0.2, state: "enabled" },
    redirections: [
      { id: "game", deviceId: "out-1", isRunning: true },
      { id: "chat", deviceId: "out-1", isRunning: true },
      { id: "media", deviceId: "out-0", isRunning: true },
      { id: "aux", deviceId: "out-1", isRunning: false },
      { id: "mic", deviceId: "in-0", isRunning: true },
    ],
    outputs: [
      { id: "out-0", name: "Speakers (Realtek(R) Audio)" },
      { id: "out-1", name: "Headphones (Arctis Nova 7)" },
    ],
    inputs: [
      { id: "in-0", name: "Microphone (Arctis Nova 7)" },
      { id: "in-1", name: "Line In (Realtek(R) Audio)" },
    ],
  };
  CHANNELS.forEach((c, i) => {
    sonar.classic.devices[c] = { classic: vm(1 - i * 0.12, c === "aux"), stream: {} };
    sonar.streamer.devices[c] = { classic: null, stream: { streaming: vm(0.8), monitoring: vm(0.6, c === "chatCapture") } };
  });
  let watching = false;

  function sonarView() {
    if (!mock.commands.get_snapshot().settings.steelseries_gg) return { state: "off" };
    return { state: "online", problem: null, error: null, ...copy(sonar) };
  }

  const tellSonar = () => mock.emit("sonar", sonarView());
  const redirect = { chatRender: "chat", chatCapture: "mic" };
  const slot = (channel) => (channel === "master" ? null : sonar.classic.devices[channel]);

  // ---- Standalone mixer ----

  const mix = {
    output: OUTPUTS[1],
    channels: CHANNELS.map((channel) => ({ channel, source: "", loopback: channel !== "chatCapture", on: false, volume: 1, muted: false, eq_on: false, eq: bands() })),
    master: 1,
    master_muted: false,
    chat_mix: 0,
    running: false,
  };
  mix.channels[0] = { ...mix.channels[0], source: OUTPUTS[2], on: true };
  mix.channels[1] = { ...mix.channels[1], source: OUTPUTS[3], on: true };
  let running = false;
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

  function mixView() {
    return {
      config: copy(mix),
      running,
      error,
      outputs: OUTPUTS,
      inputs: INPUTS,
      filters: FILTERS.map(([id, label, gain]) => ({ id, label, gain })),
      curves: mix.channels.map((c) => Array.from({ length: 96 }, (_, i) => response(c, 20 * Math.pow(1000, i / 95)))),
    };
  }

  const ch = (id) => mix.channels.find((c) => c.channel === id);

  function specsError() {
    if (!mix.output) return "Pick an output to play on";
    const on = mix.channels.filter((c) => c.on && c.source);
    const loop = on.find((c) => c.loopback && c.source === mix.output);
    if (loop) return loop.channel + " would play back into itself; pick another source or output";
    if (!on.length) return "Turn on at least one channel with a source";
    return null;
  }

  function play() {
    error = specsError();
    running = !error;
    mix.running = running;
    if (error) throw error;
  }

  Object.assign(mock.commands, {
    get_sonar: () => sonarView(),
    sonar_watch({ on }) {
      watching = on;
      if (on) setTimeout(tellSonar, 200);
    },
    sonar_set({ change: c }) {
      if (!mock.commands.get_snapshot().settings.steelseries_gg) return;
      const s = c.channel && slot(c.channel);
      if (c.kind === "mode") sonar.mode = c.mode;
      else if (c.kind === "volume") s.classic.volume = c.volume;
      else if (c.kind === "mute") s.classic.muted = c.muted;
      else if (c.kind === "master") sonar.classic.masters.classic.volume = c.volume;
      else if (c.kind === "master_mute") sonar.classic.masters.classic.muted = c.muted;
      else if (c.kind === "stream_volume") sonar.streamer.devices[c.channel].stream[c.mix].volume = c.volume;
      else if (c.kind === "stream_mute") sonar.streamer.devices[c.channel].stream[c.mix].muted = c.muted;
      else if (c.kind === "stream_master") sonar.streamer.masters.stream[c.mix].volume = c.volume;
      else if (c.kind === "stream_master_mute") sonar.streamer.masters.stream[c.mix].muted = c.muted;
      else if (c.kind === "chat_mix") sonar.chat_mix.balance = c.balance;
      else if (c.kind === "device") {
        const id = redirect[c.channel] || c.channel;
        sonar.redirections.find((r) => r.id === id).deviceId = c.id;
      }
      setTimeout(tellSonar, 150);
    },
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
      return mix.channels.map((c, i) => (running && c.on && c.source && !c.muted ? c.volume * (0.35 + 0.3 * Math.abs(Math.sin(t * (2 + i)))) : 0));
    },
  });

  setInterval(() => watching && tellSonar(), 2000);
})();
