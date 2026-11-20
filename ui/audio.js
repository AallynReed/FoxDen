// Audio tab: Sonar's mixer through SteelSeries GG, and the standalone mixer. Fills #panel-audio.
(function () {
  const root = document.getElementById("panel-audio");
  if (!root || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  // Sonar's api id, label and redirection id, in the order the mixer reports levels.
  const CHANNELS = [
    ["game", "Game", "game"],
    ["chatRender", "Chat", "chat"],
    ["media", "Media", "media"],
    ["aux", "Aux", "aux"],
    ["chatCapture", "Mic", "mic"],
  ];
  const MIXES = [
    ["streaming", "Stream"],
    ["monitoring", "Monitor"],
  ];
  const CHAT_STATES = {
    finiteWheel: "Set by the dial on the headset.",
    differentDeviceSelected: "Game and Chat play on different devices, so there is nothing to mix.",
    noDeviceSelected: "Pick an output for Game and Chat first.",
  };
  const HOLD_MS = 2500;
  const SEND_MS = 90;
  const DB_RANGE = 15;

  const touched = new Map();
  const timers = new Map();
  let snap = null;
  let sonar = { state: "off" };
  let mix = null;
  let eqChannel = "game";
  let watching = false;
  let shown = [0, 0, 0, 0, 0];

  const message = (e) => (typeof e === "string" ? e : (e && e.message) || String(e));
  const pct = (v) => Math.round(v * 100);
  const visible = () => !root.hidden && !document.hidden;

  function el(tag, props, ...children) {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  }

  function svg(tag, attrs) {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const name in attrs) node.setAttribute(name, attrs[name]);
    return node;
  }

  function setText(node, text) {
    if (node.textContent !== text) node.textContent = text;
  }

  function say(node, text) {
    node.hidden = !text;
    setText(node, text || "");
  }

  // A value the user changed a moment ago is not overwritten by an older reading.
  const touch = (key) => touched.set(key, Date.now());
  const held = (key) => Date.now() - (touched.get(key) || 0) < HOLD_MS;

  // Sends at most once per SEND_MS while a slider moves; the last value always goes.
  function later(key, fn) {
    if (timers.has(key)) {
      timers.get(key).fn = fn;
      return;
    }
    const entry = { fn };
    timers.set(key, entry);
    entry.fn();
    entry.fn = null;
    setTimeout(function tick() {
      if (entry.fn) {
        entry.fn();
        entry.fn = null;
        setTimeout(tick, SEND_MS);
      } else timers.delete(key);
    }, SEND_MS);
  }

  function seg(options, onPick) {
    const box = el("div", { className: "aud-seg", role: "group" });
    const buttons = options.map(([id, text]) => {
      const b = el("button", { type: "button", textContent: text });
      b.dataset.id = id;
      b.onclick = () => onPick(id);
      return b;
    });
    box.append(...buttons);
    box.pick = (id) => buttons.forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.id === id)));
    return box;
  }

  function muteButton(key, label, onToggle) {
    const b = el("button", { type: "button", className: "aud-mute", textContent: "Mute" });
    b.setAttribute("aria-label", "Mute " + label);
    b.setAttribute("aria-pressed", "false");
    b.onclick = () => {
      const on = b.getAttribute("aria-pressed") !== "true";
      b.setAttribute("aria-pressed", String(on));
      touch(key);
      onToggle(on);
    };
    b.show = (on) => held(key) || b.setAttribute("aria-pressed", String(!!on));
    return b;
  }

  // A volume slider with its readout; `send` gets 0..1.
  function volume(key, label, send) {
    const input = el("input", { type: "range", min: 0, max: 100, step: 1 });
    input.setAttribute("aria-label", label + " volume");
    const out = el("span", { className: "aud-num" });
    input.oninput = () => {
      touch(key);
      setText(out, input.value + "%");
      later(key, () => send(input.value / 100));
    };
    return {
      input,
      out,
      show(v) {
        if (held(key) || v == null) return;
        input.value = pct(v);
        setText(out, pct(v) + "%");
      },
    };
  }

  function options(select, list, current, missing) {
    const want = JSON.stringify([list, current]);
    if (select.dataset.list === want) return;
    select.dataset.list = want;
    const items = list.map(([value, text, off]) => el("option", { value, textContent: text, disabled: !!off }));
    if (current && !list.some(([v]) => v === current)) items.unshift(el("option", { value: current, textContent: missing }));
    select.replaceChildren(...items);
    select.value = current || "";
  }

  // ---- Sonar ----

  const so = {};

  function buildSonar() {
    so.mark = el("span", { className: "mark" });
    so.state = el("span", { className: "state" }, so.mark, (so.stateText = el("span")));
    so.head = el("h2", {}, "Sonar", so.state);
    so.offText = el("p", { className: "hint", textContent: "Sonar's mixer needs the SteelSeries GG setting." });
    so.turnOn = el("button", { type: "button", className: "primary", textContent: "Turn on SteelSeries GG" });
    so.turnOn.onclick = turnOn;
    so.off = el("div", { className: "aud-off" }, so.offText, so.turnOn);
    so.note = el("p", { className: "aud-note" });
    so.problem = el("p", { className: "problem", role: "alert", hidden: true });

    so.mode = seg(
      [
        ["classic", "Classic"],
        ["stream", "Streamer"],
      ],
      (mode) => {
        touch("mode");
        so.mode.pick(mode);
        showMode(mode);
        sendSonar({ kind: "mode", mode });
      },
    );
    so.modeRow = el("div", { className: "aud-bar" }, el("span", { className: "aud-label", textContent: "Mode" }), so.mode);

    so.classic = el("div", { className: "aud-grid aud-classic" });
    so.classic.append(...["", "Device", "Volume", "", ""].map((t) => el("span", { className: "aud-col", textContent: t })));
    so.rows = {};
    for (const [id, label, redirect] of [["master", "Master", null], ...CHANNELS]) {
      const row = (so.rows[id] = {});
      const name = el("span", { className: "aud-name", textContent: label });
      row.device = el("select", { className: "aud-device" });
      row.device.setAttribute("aria-label", label + " device");
      row.device.hidden = !redirect;
      row.device.onchange = () => {
        touch("d" + id);
        sendSonar({ kind: "device", channel: id, id: row.device.value });
      };
      const vkey = id === "master" ? "v" : "v" + id;
      const mkey = id === "master" ? "m" : "m" + id;
      row.vol = volume(vkey, label, (v) => sendSonar(id === "master" ? { kind: "master", volume: v } : { kind: "volume", channel: id, volume: v }));
      row.mute = muteButton(mkey, label, (m) =>
        sendSonar(id === "master" ? { kind: "master_mute", muted: m } : { kind: "mute", channel: id, muted: m }),
      );
      so.classic.append(name, redirect ? row.device : el("span"), row.vol.input, row.vol.out, row.mute);
      row.redirect = redirect;
    }

    so.stream = el("div", { className: "aud-grid aud-stream" });
    so.stream.append(el("span"), ...MIXES.map(([, t]) => el("span", { className: "aud-col aud-span3", textContent: t })));
    so.srows = {};
    for (const [id, label] of [["master", "Master"], ...CHANNELS]) {
      so.stream.append(el("span", { className: "aud-name", textContent: label }));
      so.srows[id] = {};
      for (const [mixId, mixLabel] of MIXES) {
        const key = mixId + id;
        const vol = volume("sv" + key, label + " " + mixLabel, (v) =>
          sendSonar(id === "master" ? { kind: "stream_master", mix: mixId, volume: v } : { kind: "stream_volume", mix: mixId, channel: id, volume: v }),
        );
        const mute = muteButton("sm" + key, label + " " + mixLabel, (m) =>
          sendSonar(
            id === "master" ? { kind: "stream_master_mute", mix: mixId, muted: m } : { kind: "stream_mute", mix: mixId, channel: id, muted: m },
          ),
        );
        so.srows[id][mixId] = { vol, mute };
        so.stream.append(vol.input, vol.out, mute);
      }
    }

    so.chat = chatMix("cm", (b) => sendSonar({ kind: "chat_mix", balance: b }));
    so.chatHint = el("p", { className: "hint", hidden: true });
    so.body = el("div", { className: "aud-body" }, so.modeRow, so.classic, so.stream, so.chat.row, so.chatHint);
    so.box = el("section", { className: "box aud-sonar" }, so.head, so.off, so.note, so.problem, so.body);
  }

  function chatMix(key, send) {
    const input = el("input", { type: "range", min: -100, max: 100, step: 1 });
    input.setAttribute("aria-label", "Chat mix");
    const out = el("span", { className: "aud-num aud-wide" });
    const word = (v) => (v === 0 ? "Even" : v < 0 ? "Game " + -v : "Chat " + v);
    input.oninput = () => {
      touch(key);
      setText(out, word(+input.value));
      later(key, () => send(input.value / 100));
    };
    const row = el(
      "div",
      { className: "aud-chat" },
      el("span", { className: "aud-name", textContent: "Chat mix" }),
      el("span", { className: "aud-end", textContent: "Game" }),
      input,
      el("span", { className: "aud-end", textContent: "Chat" }),
      out,
    );
    return {
      row,
      input,
      show(b) {
        if (held(key) || b == null) return;
        input.value = Math.round(b * 100);
        setText(out, word(+input.value));
      },
    };
  }

  function sendSonar(change) {
    invoke("sonar_set", { change }).catch((e) => say(so.problem, message(e)));
  }

  function turnOn() {
    if (!snap) return;
    so.turnOn.setAttribute("aria-disabled", "true");
    invoke("set_settings", { settings: { ...snap.settings, steelseries_gg: true } })
      .catch((e) => say(so.problem, message(e)))
      .finally(() => so.turnOn.removeAttribute("aria-disabled"));
  }

  function showMode(mode) {
    so.classic.hidden = mode === "stream";
    so.stream.hidden = mode !== "stream";
  }

  function renderSonar() {
    const v = sonar;
    const off = v.state === "off";
    const online = v.state === "online";
    so.mark.className = "mark" + (online ? " on" : v.state === "offline" ? " warn" : "");
    setText(so.stateText, { off: "Off", searching: "Looking for GG", offline: "Not connected", online: "Connected" }[v.state] || v.state);
    so.off.hidden = !off;
    say(so.note, off || online ? "" : v.state === "searching" ? "Looking for SteelSeries GG..." : (v.problem || "SteelSeries GG is not running") + ". The mixer beside this one works without it.");
    say(so.problem, online ? v.error : "");
    so.body.hidden = !online;
    if (!online) return;

    const mode = v.mode || "classic";
    if (!held("mode")) {
      so.mode.pick(mode);
      showMode(mode);
    }
    const classic = v.classic || { masters: {}, devices: {} };
    const vm = (slot) => (slot && slot.classic) || {};
    const outputs = (v.outputs || []).map((d) => [d.id, d.name]);
    const inputs = (v.inputs || []).map((d) => [d.id, d.name]);
    for (const [id, row] of Object.entries(so.rows)) {
      const slot = id === "master" ? classic.masters : (classic.devices || {})[id];
      row.vol.show(vm(slot).volume);
      row.mute.show(vm(slot).muted);
      if (row.redirect && !held("d" + id)) {
        const r = (v.redirections || []).find((x) => x.id === row.redirect);
        options(row.device, id === "chatCapture" ? inputs : outputs, r ? r.deviceId : "", "Unknown device");
      }
    }
    const streamer = v.streamer || { masters: {}, devices: {} };
    for (const [id, mixes] of Object.entries(so.srows)) {
      const slot = (id === "master" ? streamer.masters : (streamer.devices || {})[id]) || {};
      for (const [mixId, c] of Object.entries(mixes)) {
        const s = (slot.stream || {})[mixId] || {};
        c.vol.show(s.volume);
        c.mute.show(s.muted);
      }
    }
    const cm = v.chat_mix;
    so.chat.row.hidden = !cm;
    if (cm) {
      so.chat.show(cm.balance);
      const writable = cm.state === "enabled";
      so.chat.input.disabled = !writable;
      say(so.chatHint, writable ? "" : CHAT_STATES[cm.state] || "Read-only right now.");
    } else so.chatHint.hidden = true;
  }

  // ---- Standalone mixer ----

  const mx = { rows: {} };

  function buildMixer() {
    mx.mark = el("span", { className: "mark" });
    mx.stateText = el("span");
    mx.head = el("h2", {}, "Mixer", el("span", { className: "state" }, mx.mark, mx.stateText));
    mx.hint = el("p", { className: "hint", textContent: "Sonar's volumes, chat mix and EQ without GG. Records an input, or what plays on an output, and plays it on one output." });
    mx.output = el("select", { className: "aud-device" });
    mx.output.setAttribute("aria-label", "Play on");
    mx.output.onchange = () => {
      touch("output");
      sendMix({ kind: "output", name: mx.output.value });
    };
    mx.rescan = el("button", { type: "button", textContent: "Rescan" });
    mx.rescan.onclick = () => loadMixer(true);
    mx.run = el("button", { type: "button", className: "primary aud-run" });
    mx.run.onclick = () => {
      mx.run.setAttribute("aria-disabled", "true");
      sendMix({ kind: "run", on: !(mix && mix.running) }).finally(() => mx.run.removeAttribute("aria-disabled"));
    };
    mx.bar = el("div", { className: "aud-bar" }, el("span", { className: "aud-label", textContent: "Play on" }), mx.output, mx.rescan, mx.run);
    mx.problem = el("p", { className: "problem", role: "alert", hidden: true });

    mx.grid = el("div", { className: "aud-grid aud-mix" });
    mx.grid.append(...["", "Source", "Volume", "", "", "Level"].map((t) => el("span", { className: "aud-col", textContent: t })));
    for (const [id, label] of CHANNELS) {
      const row = (mx.rows[id] = {});
      row.on = el("input", { type: "checkbox" });
      row.on.setAttribute("aria-label", "Use " + label);
      row.source = el("select", { className: "aud-device" });
      row.source.setAttribute("aria-label", label + " source");
      const route = () => {
        touch("r" + id);
        const [kind, ...rest] = row.source.value.split(":");
        sendMix({ kind: "route", channel: id, source: rest.join(":"), loopback: kind === "out", on: row.on.checked && !!row.source.value });
      };
      row.on.onchange = route;
      row.source.onchange = () => {
        row.on.checked = !!row.source.value;
        route();
      };
      row.vol = volume("mv" + id, label, (v) => sendMix({ kind: "volume", channel: id, volume: v }));
      row.mute = muteButton("mm" + id, label, (m) => sendMix({ kind: "mute", channel: id, muted: m }));
      row.fill = el("span");
      const meter = el("span", { className: "aud-meter" }, row.fill);
      meter.setAttribute("aria-hidden", "true");
      mx.grid.append(el("label", { className: "check aud-name" }, row.on, label), row.source, row.vol.input, row.vol.out, row.mute, meter);
    }
    mx.master = volume("mvmaster", "Master", (v) => sendMix({ kind: "master", volume: v }));
    mx.masterMute = muteButton("mmmaster", "Master", (m) => sendMix({ kind: "master_mute", muted: m }));
    mx.grid.append(el("span", { className: "aud-name", textContent: "Master" }), el("span"), mx.master.input, mx.master.out, mx.masterMute, el("span"));
    mx.chat = chatMix("mcm", (b) => sendMix({ kind: "chat_mix", balance: b }));

    buildEq();
    mx.box = el("section", { className: "box aud-mixer" }, mx.head, mx.hint, mx.bar, mx.problem, mx.grid, mx.chat.row, mx.eq);
  }

  function buildEq() {
    mx.eqSeg = seg(
      CHANNELS.map(([id, label]) => [id, label]),
      (id) => {
        eqChannel = id;
        renderMixer();
      },
    );
    mx.eqOn = el("input", { type: "checkbox", className: "switch" });
    mx.eqOn.setAttribute("aria-label", "EQ on");
    mx.eqOn.onchange = () => sendEq();
    mx.flat = el("button", { type: "button", textContent: "Flat" });
    mx.flat.onclick = () => {
      for (const b of bandsOf(eqChannel)) b.gain_db = 0;
      sendEq();
    };
    mx.plot = svg("svg", { class: "aud-plot", viewBox: "0 0 100 60", preserveAspectRatio: "none", "aria-hidden": "true" });
    for (const db of [-12, -6, 0, 6, 12]) {
      const y = 30 - (db / DB_RANGE) * 30;
      mx.plot.append(svg("line", { x1: 0, x2: 100, y1: y, y2: y, class: db ? "grid" : "zero" }));
    }
    mx.line = svg("polyline", { class: "curve" });
    mx.plot.append(mx.line);
    mx.bands = el("div", { className: "aud-bands" });
    mx.bandUi = [];
    for (let i = 0; i < 10; i++) {
      const gain = el("input", { type: "range", min: -12, max: 12, step: 0.5, className: "aud-vert" });
      const db = el("span", { className: "aud-num" });
      const type = el("select");
      const freq = el("input", { type: "number", min: 20, max: 20000, step: 1 });
      const q = el("input", { type: "number", min: 0.1, max: 10, step: 0.01 });
      const name = el("span", { className: "aud-num aud-hz" });
      gain.setAttribute("aria-label", "Band " + (i + 1) + " gain");
      type.setAttribute("aria-label", "Band " + (i + 1) + " filter");
      freq.setAttribute("aria-label", "Band " + (i + 1) + " frequency in Hz");
      q.setAttribute("aria-label", "Band " + (i + 1) + " Q");
      const edit = (fn) => () => {
        const b = bandsOf(eqChannel)[i];
        if (!b) return;
        fn(b);
        sendEq();
      };
      gain.oninput = edit((b) => {
        b.gain_db = +gain.value;
        setText(db, signed(b.gain_db));
      });
      type.onchange = edit((b) => (b.filter = type.value));
      freq.onchange = edit((b) => (b.frequency = clampNum(freq, 20, 20000, b.frequency)));
      q.onchange = edit((b) => (b.q = clampNum(q, 0.1, 10, b.q)));
      mx.bandUi.push({ gain, db, type, freq, q, name });
      mx.bands.append(el("div", { className: "aud-band" }, name, gain, db, type, freq, q));
    }
    mx.eq = el(
      "div",
      { className: "aud-eq" },
      el("h3", { textContent: "Parametric EQ" }),
      el("div", { className: "aud-bar" }, mx.eqSeg, el("label", { className: "check" }, mx.eqOn, "On"), mx.flat),
      mx.plot,
      mx.bands,
    );
  }

  const signed = (v) => (v > 0 ? "+" : v < 0 ? "−" : "") + Math.abs(v).toFixed(1);
  const hz = (f) => (f >= 1000 ? +(f / 1000).toFixed(1) + "k" : Math.round(f) + "");

  function clampNum(input, lo, hi, was) {
    const v = parseFloat(input.value);
    const out = Number.isFinite(v) ? Math.min(Math.max(v, lo), hi) : was;
    input.value = out;
    return out;
  }

  function chan(id) {
    return mix && mix.config.channels.find((c) => c.channel === id);
  }

  function bandsOf(id) {
    const c = chan(id);
    return c ? c.eq : [];
  }

  function sendEq() {
    const c = chan(eqChannel);
    if (!c) return;
    c.eq_on = mx.eqOn.checked;
    const key = "eq" + eqChannel;
    touch(key);
    const change = { kind: "eq", channel: eqChannel, enabled: c.eq_on, bands: c.eq };
    later(key, () => sendMix(JSON.parse(JSON.stringify(change))));
    renderMixer();
  }

  async function sendMix(change) {
    try {
      take(await invoke("mixer_set", { change }));
      say(mx.problem, mix.error);
    } catch (e) {
      say(mx.problem, message(e));
      loadMixer(false);
    }
  }

  // Keeps local edits the user is still making.
  function take(view) {
    if (mix) {
      for (const c of view.config.channels) {
        const old = mix.config.channels.find((x) => x.channel === c.channel);
        if (old && held("eq" + c.channel)) {
          c.eq = old.eq;
          c.eq_on = old.eq_on;
        }
      }
    }
    mix = view;
    renderMixer();
  }

  async function loadMixer(rescan) {
    try {
      take(await invoke("get_mixer", { rescan }));
      say(mx.problem, mix.error);
    } catch (e) {
      say(mx.problem, message(e));
    }
  }

  function renderMixer() {
    if (!mix) return;
    const cfg = mix.config;
    mx.mark.className = "mark" + (mix.running ? " on" : "");
    setText(mx.stateText, mix.running ? "Running" : "Stopped");
    setText(mx.run, mix.running ? "Stop" : "Start");
    mx.run.classList.toggle("primary", !mix.running);
    if (!held("output")) options(mx.output, mix.outputs.map((o) => [o, o]), cfg.output, cfg.output + " (missing)");
    for (const c of cfg.channels) {
      const row = mx.rows[c.channel];
      if (!row) continue;
      if (!held("r" + c.channel)) {
        row.on.checked = c.on;
        const list = [
          ["", "None"],
          ...mix.outputs.map((o) => ["out:" + o, "Playing on " + o, o === cfg.output]),
          ...mix.inputs.map((i) => ["in:" + i, i]),
        ];
        const current = c.source ? (c.loopback ? "out:" : "in:") + c.source : "";
        options(row.source, list, current, c.source + " (missing)");
      }
      row.vol.show(c.volume);
      row.mute.show(c.muted);
    }
    mx.master.show(cfg.master);
    mx.masterMute.show(cfg.master_muted);
    mx.chat.show(cfg.chat_mix);
    renderEq();
  }

  function renderEq() {
    const i = CHANNELS.findIndex(([id]) => id === eqChannel);
    const c = chan(eqChannel);
    mx.eqSeg.pick(eqChannel);
    if (!c) return;
    mx.eqOn.checked = c.eq_on;
    const filters = mix.filters || [];
    mx.bandUi.forEach((ui, n) => {
      const b = c.eq[n];
      ui.gain.parentNode.hidden = !b;
      if (!b) return;
      const f = filters.find((x) => x.id === b.filter);
      options(ui.type, filters.map((x) => [x.id, x.label]), b.filter, b.filter);
      setText(ui.name, hz(b.frequency));
      if (document.activeElement !== ui.gain) ui.gain.value = b.gain_db;
      ui.gain.disabled = !!f && !f.gain;
      setText(ui.db, signed(b.gain_db));
      if (document.activeElement !== ui.freq) ui.freq.value = Math.round(b.frequency);
      if (document.activeElement !== ui.q) ui.q.value = +b.q.toFixed(2);
    });
    const curve = (mix.curves || [])[i] || [];
    const pts = curve.map((db, n) => (n / (curve.length - 1)) * 100 + "," + (30 - (Math.max(-DB_RANGE, Math.min(DB_RANGE, db)) / DB_RANGE) * 30));
    mx.line.setAttribute("points", pts.join(" "));
    mx.plot.classList.toggle("off", !c.eq_on);
  }

  // Level meters, while the routes play and the tab is shown.
  async function levels() {
    if (!mix || !mix.running || !visible()) {
      if (shown.some((v) => v > 0)) {
        shown = shown.map(() => 0);
        paintLevels();
      }
      return;
    }
    try {
      const peaks = await invoke("get_mixer_levels");
      shown = shown.map((v, n) => Math.max(peaks[n] || 0, v * 0.8));
      paintLevels();
    } catch {
      // the next tick tries again
    }
  }

  function paintLevels() {
    CHANNELS.forEach(([id], n) => (mx.rows[id].fill.style.transform = "scaleX(" + Math.min(1, shown[n]).toFixed(3) + ")"));
  }

  // ---- Page ----

  function watch() {
    const on = visible();
    if (on === watching) return;
    watching = on;
    invoke("sonar_watch", { on }).catch(() => {});
    if (on) {
      invoke("get_sonar").then((v) => {
        sonar = v;
        renderSonar();
      });
      loadMixer(false);
    }
  }

  buildSonar();
  buildMixer();
  root.replaceChildren(el("div", { className: "aud-page" }, so.box, mx.box));
  renderSonar();

  new MutationObserver(watch).observe(root, { attributes: true, attributeFilter: ["hidden"] });
  document.addEventListener("visibilitychange", watch);
  listen("sonar", (e) => {
    sonar = e.payload;
    renderSonar();
  });
  listen("snapshot", (e) => (snap = e.payload));
  invoke("get_snapshot").then((s) => (snap = snap || s));
  setInterval(levels, 100);
  watch();
})();
