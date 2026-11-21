// Audio tab: channel strips with their apps, the chat mix and a per-channel EQ. Fills #panel-audio.
(function () {
  const root = document.getElementById("panel-audio");
  if (!root || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  // Channel id and label, in the order the mixer reports levels. All but the mic take apps.
  const CHANNELS = [
    ["game", "Game"],
    ["chatRender", "Chat"],
    ["media", "Media"],
    ["aux", "Aux"],
    ["chatCapture", "Mic"],
  ];
  const APP_CHANNELS = CHANNELS.filter(([id]) => id !== "chatCapture");
  const LABEL = Object.fromEntries(CHANNELS);
  const FREQS = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
  const PRESETS = [
    ["flat", "Flat", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
    ["bass", "Bass boost", [6, 5, 4, 2, 0, 0, 0, 0, 0, 0]],
    ["voice", "Voice clarity", [-4, -3, -2, -1, 0, 2, 4, 4, 2, 0]],
    ["treble", "Treble", [0, 0, 0, 0, 0, 0, 2, 4, 5, 6]],
  ];
  const HOLD_MS = 2500;
  const SEND_MS = 90;
  const DB_RANGE = 15;

  const touched = new Map();
  const timers = new Map();
  let mix = null;
  let eqChannel = null;
  let watching = false;
  let shown = [0, 0, 0, 0, 0];

  const message = (e) => (typeof e === "string" ? e : (e && e.message) || String(e));
  const pct = (v) => Math.round(v * 100);
  const visible = () => !root.hidden && !document.hidden;
  const lower = (s) => s.toLowerCase();
  const shortName = (exe) => exe.replace(/\.exe$/i, "");

  function el(tag, props, ...children) {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  }

  function svg(tag, attrs, ...children) {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const name in attrs) node.setAttribute(name, attrs[name]);
    node.append(...children);
    return node;
  }

  const icon = (d) => svg("svg", { viewBox: "0 0 16 16", width: 16, height: 16, "aria-hidden": "true", class: "aud-icon" }, svg("path", { d }));

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

  function seg(options, onPick, label) {
    const box = el("div", { className: "aud-seg", role: "group" });
    box.setAttribute("aria-label", label);
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
      row: el("div", { className: "aud-vol" }, input, out),
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

  function chatMix(key, send) {
    const input = el("input", { type: "range", min: -100, max: 100, step: 1, id: "aud-chatmix" });
    const out = el("span", { className: "aud-num aud-wide" });
    const word = (v) => (v === 0 ? "Even" : v < 0 ? "Game +" + -v : "Chat +" + v);
    input.oninput = () => {
      touch(key);
      setText(out, word(+input.value));
      later(key, () => send(input.value / 100));
    };
    const note = el("span", { className: "aud-dial", textContent: "Set by the headset's ChatMix dial", hidden: true });
    const row = el(
      "div",
      { className: "aud-chat" },
      el("label", { className: "aud-name", htmlFor: "aud-chatmix", textContent: "Chat mix" }),
      el("span", { className: "aud-end", textContent: "Game" }),
      input,
      el("span", { className: "aud-end", textContent: "Chat" }),
      out,
      note,
    );
    let dial = null;
    return {
      row,
      show(b) {
        if (dial || held(key) || b == null) return;
        input.value = Math.round(b * 100);
        setText(out, word(+input.value));
      },
      // While a headset's dial is on, it sets the mix and the slider only shows it.
      dial(d) {
        dial = d;
        input.disabled = !!d;
        note.hidden = !d;
        row.classList.toggle("dial", !!d);
        if (d) {
          input.value = d.chat - d.game;
          setText(out, word(+input.value));
        }
      },
    };
  }

  function chan(id) {
    return mix && mix.config.channels.find((c) => c.channel === id);
  }

  // A channel with something to play but switched off stays silent.
  const idle = (c) => !c.on && (c.apps.length > 0 || !!c.source);
  const switchOn = (c) => sendMix({ kind: "route", channel: c.channel, source: c.source, loopback: c.loopback, on: true });

  // ---- Top bar ----

  const mx = { strips: {} };

  function buildBar() {
    mx.output = el("select", { className: "aud-device", id: "aud-output" });
    mx.output.onchange = () => {
      touch("output");
      sendMix({ kind: "output", name: mx.output.value });
    };
    mx.rescan = el("button", { type: "button", className: "aud-icon-btn", title: "Rescan devices and apps" }, icon("M13 3v4H9M3 13V9h4M12.6 7A5 5 0 0 0 3.6 5.5M3.4 9a5 5 0 0 0 9 1.5"));
    mx.rescan.setAttribute("aria-label", "Rescan devices and apps");
    mx.rescan.onclick = () => loadMixer(true);
    mx.mark = el("span", { className: "mark" });
    mx.stateText = el("span");
    mx.state = el("span", { className: "state aud-state" }, mx.mark, mx.stateText);
    mx.state.setAttribute("role", "status");
    mx.run = el("button", { type: "button", className: "aud-run" });
    mx.run.onclick = () => {
      if (mx.run.getAttribute("aria-disabled") === "true") return;
      mx.run.setAttribute("aria-disabled", "true");
      sendMix({ kind: "run", on: !(mix && mix.running) }).finally(() => mx.run.removeAttribute("aria-disabled"));
    };
    mx.problem = el("p", { className: "problem", role: "alert", hidden: true });
    return el(
      "div",
      { className: "aud-top" },
      el("label", { className: "aud-label", htmlFor: "aud-output", textContent: "Output" }),
      mx.output,
      mx.rescan,
      el("span", { className: "aud-run-box" }, mx.state, mx.run),
    );
  }

  // ---- Strips ----

  function stripHead(id, label) {
    const s = mx.strips[id];
    s.offTag = el("span", { className: "aud-offtag", textContent: "Off", hidden: true });
    s.turnOn = el("button", { type: "button", className: "aud-small", textContent: "Turn on", hidden: true });
    s.turnOn.setAttribute("aria-label", "Turn on " + label);
    s.turnOn.onclick = () => switchOn(chan(id));
    return el("div", { className: "aud-head" }, el("h3", { id: "aud-h-" + id, textContent: label }), s.offTag, s.turnOn);
  }

  function stripFoot(id, label) {
    const s = mx.strips[id];
    s.vol = volume("mv" + id, label, (v) => sendMix({ kind: "volume", channel: id, volume: v }));
    s.mute = muteButton("mm" + id, label, (m) => sendMix({ kind: "mute", channel: id, muted: m }));
    s.fill = el("span");
    const meter = el("span", { className: "aud-meter" }, s.fill);
    meter.setAttribute("aria-hidden", "true");
    s.eqBtn = el("button", { type: "button", className: "aud-eqbtn", textContent: "EQ" });
    s.eqBtn.setAttribute("aria-controls", "aud-eq");
    s.eqBtn.setAttribute("aria-expanded", "false");
    s.eqBtn.setAttribute("aria-label", "Edit " + label + " EQ");
    s.eqBtn.onclick = () => openEq(eqChannel === id ? null : id);
    s.eqOn = el("input", { type: "checkbox", className: "switch" });
    s.eqOn.setAttribute("aria-label", label + " EQ on");
    s.eqOn.title = "EQ on or off";
    s.eqOn.onchange = () => sendEq(id, s.eqOn.checked);
    return el("div", { className: "aud-foot" }, meter, s.vol.row, el("div", { className: "aud-ctl" }, s.mute, s.eqBtn, s.eqOn));
  }

  function appStrip(id, label) {
    const s = (mx.strips[id] = {});
    s.chips = el("ul", { className: "aud-chips" });
    s.chips.setAttribute("aria-label", label + " apps");
    s.empty = el("p", { className: "aud-empty", textContent: "No apps yet. Add an app to hear it here." });
    s.add = el("select", { className: "aud-add" });
    s.add.setAttribute("aria-label", "Add an app to " + label);
    s.add.onchange = () => {
      const exe = s.add.value;
      s.add.value = "";
      if (!exe) return;
      const c = chan(id);
      sendMix({ kind: "apps", channel: id, apps: [...c.apps, exe] }).then(() => {
        const now = chan(id);
        if (now && !now.on) switchOn(now);
      });
    };
    return (s.node = el("section", { className: "aud-strip" }, stripHead(id, label), s.chips, s.empty, s.add, stripFoot(id, label)));
  }

  function micStrip() {
    const id = "chatCapture";
    const s = (mx.strips[id] = {});
    s.source = el("select", { className: "aud-device", id: "aud-mic" });
    s.source.onchange = () => {
      touch("r" + id);
      const [kind, ...rest] = s.source.value.split(":");
      sendMix({ kind: "route", channel: id, source: rest.join(":"), loopback: kind === "out", on: !!s.source.value });
    };
    s.empty = el("p", { className: "aud-empty", textContent: "Pick a microphone to mix it in." });
    return (s.node = el(
      "section",
      { className: "aud-strip" },
      stripHead(id, "Mic"),
      el("label", { className: "aud-label", htmlFor: "aud-mic", textContent: "Microphone" }),
      s.source,
      s.empty,
      buildVmic(),
      stripFoot(id, "Mic"),
    ));
  }

  // ---- Virtual mic ----

  const vm = {};

  function buildVmic() {
    vm.sw = el("input", { type: "checkbox", className: "switch", id: "aud-vmic" });
    vm.sw.onchange = () => wantVmic(vm.sw.checked);
    vm.note = el("p", { className: "aud-empty", role: "status", hidden: true });
    vm.gate = el("input", { type: "checkbox", id: "aud-gate" });
    vm.gate.setAttribute("aria-label", "Noise gate on");
    vm.level = el("input", { type: "range", min: -90, max: 0, step: 1 });
    vm.level.setAttribute("aria-label", "Noise gate threshold in dB");
    vm.levelOut = el("span", { className: "aud-num" });
    const sendGate = () => {
      touch("gate");
      setText(vm.levelOut, vm.level.value + " dB");
      later("gate", () => sendMix({ kind: "gate", on: vm.gate.checked, threshold_db: +vm.level.value }));
    };
    vm.gate.onchange = sendGate;
    vm.level.oninput = sendGate;
    return el(
      "div",
      { className: "aud-vmic" },
      el("div", { className: "aud-vrow" }, el("label", { className: "aud-name", htmlFor: "aud-vmic", textContent: "Virtual mic" }), vm.sw),
      vm.note,
      el("div", { className: "aud-gate" }, el("label", { className: "check" }, vm.gate, "Gate"), vm.level, vm.levelOut),
    );
  }

  // Turning it on needs VB-CABLE; without it the install sheet asks first.
  function wantVmic(on) {
    touch("vmic");
    if (!on) return sendMix({ kind: "virtual_mic", on: false });
    if (mix && mix.vmic && mix.vmic.installed) return sendMix({ kind: "virtual_mic", on: true });
    say(vm.progress, "");
    if (!vm.sheet.open) vm.sheet.showModal();
  }

  function buildSheet() {
    vm.sheet = document.getElementById("vmic-sheet");
    vm.progress = document.getElementById("vmic-progress");
    vm.install = document.getElementById("vmic-install");
    vm.sheet.addEventListener("close", () => {
      touched.delete("vmic");
      renderVmic();
    });
    document.getElementById("vmic-cancel").onclick = () => vm.sheet.close();
    for (const id of ["vmic-donate", "vmic-donate-settings"]) {
      document.getElementById(id).onclick = () => invoke("open_link", { which: "vbaudio" }).catch(() => {});
    }
    vm.install.onclick = async () => {
      vm.install.disabled = true;
      say(vm.progress, "Installing VB-CABLE. Windows asks for administrator rights.");
      try {
        take(await invoke("virtual_mic_install"));
        vm.sheet.close();
      } catch (e) {
        say(vm.progress, message(e));
      } finally {
        vm.install.disabled = false;
      }
    };
    vm.setSw = document.getElementById("set-vmic");
    vm.setSw.onchange = () => wantVmic(vm.setSw.checked);
    vm.state = document.getElementById("vmic-state");
    vm.outcome = document.getElementById("vmic-outcome");
    vm.remove = document.getElementById("vmic-remove");
    vm.remove.onclick = async () => {
      vm.remove.disabled = true;
      say(vm.outcome, "Removing VB-CABLE. Windows asks for administrator rights.");
      try {
        take(await invoke("virtual_mic_remove"));
        say(vm.outcome, "Removed. Restart Windows to finish.");
      } catch (e) {
        say(vm.outcome, message(e));
      } finally {
        vm.remove.disabled = false;
      }
    };
  }

  function vmicNote(cfg, v, mic) {
    if (v.error) return v.error;
    if (v.installed && !v.ready) return "Restart Windows to finish installing VB-CABLE.";
    if (!cfg.virtual_mic) return "";
    if (!v.installed) return "VB-CABLE is not installed.";
    if (!mic.source) return "Pick a microphone above.";
    return "Apps can pick " + (v.name || "FoxDen VB-Cable (Microphone)") + ".";
  }

  function renderVmic() {
    if (!mix) return;
    const cfg = mix.config;
    const v = mix.vmic || {};
    const on = !!cfg.virtual_mic;
    if (!held("vmic")) {
      vm.sw.checked = on;
      vm.setSw.checked = on;
    }
    say(vm.note, vmicNote(cfg, v, chan("chatCapture") || {}));
    vm.note.classList.toggle("warn", !!v.error);
    if (!held("gate")) {
      vm.gate.checked = !!cfg.gate_on;
      vm.level.value = cfg.gate_db;
      setText(vm.levelOut, Math.round(cfg.gate_db) + " dB");
    }
    vm.level.disabled = !cfg.gate_on;
    setText(vm.state, !v.installed ? "Not installed" : !v.ready ? "Restart Windows to finish" : v.ours ? "Installed by FoxDen" : "Installed");
    vm.remove.hidden = !v.ours;
  }

  function masterStrip() {
    mx.master = volume("mvmaster", "Master", (v) => sendMix({ kind: "master", volume: v }));
    mx.masterMute = muteButton("mmmaster", "Master", (m) => sendMix({ kind: "master_mute", muted: m }));
    mx.masterOut = el("p", { className: "aud-empty" });
    return el(
      "section",
      { className: "aud-strip aud-master" },
      el("div", { className: "aud-head" }, el("h3", { id: "aud-h-master", textContent: "Master" })),
      mx.masterOut,
      el("div", { className: "aud-foot" }, el("span", { className: "aud-meter aud-blank" }), mx.master.row, el("div", { className: "aud-ctl" }, mx.masterMute)),
    );
  }

  // Where an assigned app stands while the mixer runs.
  function appState(exe) {
    if (!mix.running) return ["", "", ""];
    const st = (mix.app_states || {})[lower(exe)];
    if (!st || st.state === "idle") return ["", "Not playing", ""];
    if (st.state === "playing") return ["on", "Mixed", ""];
    if (st.state === "elsewhere") return ["warn", "Restart it", "Still playing on " + st.output + ". Restart it to move it here."];
    if (st.state === "contains") return ["warn", "Not mixed", "It started " + shortName(st.app) + ", which is in another channel."];
    return ["warn", "Failed", st.message];
  }

  function removeButton(text, onClick) {
    const b = el("button", { type: "button", className: "aud-x" }, icon("M4 4l8 8M12 4l-8 8"));
    b.setAttribute("aria-label", text);
    b.title = text;
    b.onclick = onClick;
    return b;
  }

  function chip(id, label, exe, apps) {
    const [mark, word, note] = appState(exe);
    const name = shortName(exe);
    const item = el(
      "li",
      { className: "aud-chip" + (mark === "warn" ? " warn" : "") },
      el("span", { className: "mark" + (mark ? " " + mark : " none") }),
      el("span", { className: "aud-chipname", textContent: name, title: exe + (word ? ": " + word : "") }),
    );
    if (mark === "warn") item.append(el("span", { className: "aud-tag", textContent: word }));
    item.append(removeButton("Remove " + name + " from " + label, () => sendMix({ kind: "apps", channel: id, apps: apps.filter((a) => a !== exe) })));
    if (note) item.append(el("span", { className: "aud-chipnote", textContent: note }));
    return item;
  }

  function deviceChip(c, label) {
    const what = (c.loopback ? "Everything on " : "") + c.source;
    return el(
      "li",
      { className: "aud-chip aud-devchip" },
      el("span", { className: "aud-tag", textContent: "Device" }),
      el("span", { className: "aud-chipname", textContent: what, title: what }),
      removeButton("Stop recording " + c.source + " into " + label, () => sendMix({ kind: "route", channel: c.channel, source: "", loopback: c.loopback, on: c.apps.length > 0 })),
    );
  }

  function renderStrips() {
    const cfg = mix.config;
    const assigned = new Set(cfg.channels.flatMap((c) => c.apps || []).map(lower));
    const free = (mix.apps || []).filter((a) => !assigned.has(lower(a.exe)));
    for (const [id, label] of CHANNELS) {
      const s = mx.strips[id];
      const c = chan(id);
      if (!c) continue;
      const off = idle(c);
      s.offTag.hidden = !off;
      s.turnOn.hidden = !off;
      s.node.classList.toggle("off", off);
      s.vol.show(c.volume);
      s.mute.show(c.muted);
      s.eqOn.checked = c.eq_on;
      s.eqBtn.setAttribute("aria-expanded", String(eqChannel === id));
      if (id === "chatCapture") {
        if (!held("r" + id)) {
          const list = [["", "None"], ...mix.inputs.map((i) => ["in:" + i, i])];
          if (c.source && c.loopback) list.push(["out:" + c.source, "Everything on " + c.source]);
          options(s.source, list, c.source ? (c.loopback ? "out:" : "in:") + c.source : "", c.source + " (missing)");
        }
        s.empty.hidden = !!c.source;
        continue;
      }
      const apps = c.apps || [];
      const want = JSON.stringify([apps, c.source, c.loopback, mix.running, apps.map(appState)]);
      if (s.chips.dataset.list !== want) {
        s.chips.dataset.list = want;
        const items = apps.map((exe) => chip(id, label, exe, apps));
        if (c.source) items.push(deviceChip(c, label));
        s.chips.replaceChildren(...items);
      }
      s.chips.hidden = !s.chips.children.length;
      s.empty.hidden = !!s.chips.children.length;
      options(
        s.add,
        [["", free.length ? "+ Add app" : "No other apps playing sound"], ...free.map((a) => [a.exe, shortName(a.exe) + (a.active ? " (playing)" : "")])],
        "",
        "",
      );
    }
    mx.master.show(cfg.master);
    mx.masterMute.show(cfg.master_muted);
    setText(mx.masterOut, cfg.output ? "Plays on " + cfg.output : "Pick an output above.");
  }

  // ---- EQ ----

  function buildEq() {
    mx.eqTitle = el("h2", { id: "aud-eq-title" });
    mx.eqSwitch = el("input", { type: "checkbox", className: "switch", id: "aud-eq-on" });
    mx.eqSwitch.onchange = () => sendEq(eqChannel, mx.eqSwitch.checked);
    mx.eqClose = el("button", { type: "button", className: "aud-small", textContent: "Close" });
    mx.eqClose.onclick = () => openEq(null);
    mx.presets = seg(PRESETS.map(([id, text]) => [id, text]), applyPreset, "Presets");
    mx.adv = el("input", { type: "checkbox", id: "aud-eq-adv" });
    mx.adv.onchange = () => mx.eqBox.classList.toggle("adv", mx.adv.checked);
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
      const type = el("select", { className: "aud-adv" });
      const freq = el("input", { type: "number", min: 20, max: 20000, step: 1, className: "aud-adv" });
      const q = el("input", { type: "number", min: 0.1, max: 10, step: 0.01, className: "aud-adv" });
      const name = el("span", { className: "aud-num aud-hz" });
      const edit = (fn) => () => {
        const b = bandsOf(eqChannel)[i];
        if (!b) return;
        fn(b);
        sendEq(eqChannel);
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
    mx.eqBox = el(
      "section",
      { className: "box aud-eqbox", id: "aud-eq", hidden: true },
      el("div", { className: "aud-eqhead" }, mx.eqTitle, el("label", { className: "check" }, mx.eqSwitch, "On"), mx.eqClose),
      el(
        "div",
        { className: "aud-eqbody" },
        el(
          "div",
          { className: "aud-eqside" },
          el("span", { className: "aud-label", textContent: "Preset" }),
          mx.presets,
          mx.plot,
          el("label", { className: "check aud-advtoggle" }, mx.adv, "Advanced: filter types, frequency and Q"),
        ),
        mx.bands,
      ),
    );
    mx.eqBox.setAttribute("aria-labelledby", "aud-eq-title");
  }

  const signed = (v) => (v > 0 ? "+" : v < 0 ? "−" : "") + Math.abs(v).toFixed(1);
  const hz = (f) => (f >= 1000 ? +(f / 1000).toFixed(1) + "k" : Math.round(f) + "");

  function clampNum(input, lo, hi, was) {
    const v = parseFloat(input.value);
    const out = Number.isFinite(v) ? Math.min(Math.max(v, lo), hi) : was;
    input.value = out;
    return out;
  }

  function bandsOf(id) {
    const c = chan(id);
    return c ? c.eq : [];
  }

  function openEq(id) {
    eqChannel = id;
    mx.eqBox.hidden = !id;
    renderMixer();
    if (id) mx.eqBox.scrollIntoView({ block: "nearest" });
  }

  // The preset whose layout and gains the bands have now, if any.
  function presetOf(bands) {
    if (bands.length !== FREQS.length) return null;
    const plain = bands.every((b, i) => b.filter === "PeakingEQ" && Math.round(b.frequency) === FREQS[i] && Math.abs(b.q - 1.414) < 0.01);
    if (!plain) return null;
    const p = PRESETS.find(([, , gains]) => gains.every((g, i) => Math.abs(bands[i].gain_db - g) < 0.01));
    return p ? p[0] : null;
  }

  function applyPreset(name) {
    const c = chan(eqChannel);
    if (!c) return;
    const gains = PRESETS.find(([id]) => id === name)[2];
    c.eq = FREQS.map((frequency, i) => ({ enabled: true, filter: "PeakingEQ", frequency, gain_db: gains[i], q: 1.414 }));
    sendEq(eqChannel, name !== "flat" || c.eq_on);
  }

  function sendEq(id, on) {
    const c = chan(id);
    if (!c) return;
    if (on != null) c.eq_on = on;
    const key = "eq" + id;
    touch(key);
    const change = { kind: "eq", channel: id, enabled: c.eq_on, bands: c.eq };
    later(key, () => sendMix(JSON.parse(JSON.stringify(change))));
    renderMixer();
  }

  function renderEq() {
    const c = chan(eqChannel);
    if (!c) return;
    setText(mx.eqTitle, LABEL[eqChannel] + " EQ");
    mx.eqSwitch.setAttribute("aria-label", LABEL[eqChannel] + " EQ on");
    mx.eqSwitch.checked = c.eq_on;
    mx.presets.pick(presetOf(c.eq));
    const filters = mix.filters || [];
    mx.bandUi.forEach((ui, n) => {
      const b = c.eq[n];
      ui.gain.parentNode.hidden = !b;
      if (!b) return;
      const f = filters.find((x) => x.id === b.filter);
      const at = hz(b.frequency) + " Hz";
      ui.gain.setAttribute("aria-label", at + " gain");
      ui.type.setAttribute("aria-label", at + " filter");
      ui.freq.setAttribute("aria-label", "Band " + (n + 1) + " frequency in Hz");
      ui.q.setAttribute("aria-label", at + " Q");
      options(ui.type, filters.map((x) => [x.id, x.label]), b.filter, b.filter);
      setText(ui.name, hz(b.frequency));
      if (document.activeElement !== ui.gain) ui.gain.value = b.gain_db;
      ui.gain.disabled = !!f && !f.gain;
      setText(ui.db, signed(b.gain_db));
      if (document.activeElement !== ui.freq) ui.freq.value = Math.round(b.frequency);
      if (document.activeElement !== ui.q) ui.q.value = +b.q.toFixed(2);
    });
    const i = CHANNELS.findIndex(([id]) => id === eqChannel);
    const curve = (mix.curves || [])[i] || [];
    const pts = curve.map((db, n) => (n / (curve.length - 1)) * 100 + "," + (30 - (Math.max(-DB_RANGE, Math.min(DB_RANGE, db)) / DB_RANGE) * 30));
    mx.line.setAttribute("points", pts.join(" "));
    mx.plot.classList.toggle("off", !c.eq_on);
  }

  // ---- Advanced ----

  function buildAdvanced() {
    mx.park = el("select", { className: "aud-device", id: "aud-park" });
    mx.park.onchange = () => {
      touch("park");
      sendMix({ kind: "park", name: mx.park.value });
    };
    mx.parked = el("p", { className: "hint", hidden: true });
    mx.sources = {};
    const rows = APP_CHANNELS.map(([id, label]) => {
      const select = el("select", { className: "aud-device", id: "aud-src-" + id });
      select.onchange = () => {
        touch("r" + id);
        const [kind, ...rest] = select.value.split(":");
        const c = chan(id);
        sendMix({ kind: "route", channel: id, source: rest.join(":"), loopback: kind === "out", on: !!select.value || c.apps.length > 0 });
      };
      mx.sources[id] = select;
      return [el("label", { htmlFor: select.id, textContent: label }), select];
    });
    return el(
      "details",
      { className: "aud-more" },
      el("summary", { textContent: "Advanced" }),
      el(
        "div",
        { className: "aud-moregrid" },
        el(
          "div",
          { className: "aud-moreitem" },
          el("label", { className: "aud-name", htmlFor: "aud-park", textContent: "Spare output for apps" }),
          el("p", { className: "hint", textContent: "Apps you add are moved to this output, which FoxDen keeps muted while the mixer runs, so you hear each app once." }),
          mx.park,
          mx.parked,
        ),
        el(
          "div",
          { className: "aud-moreitem" },
          el("span", { className: "aud-name", textContent: "Record from a device" }),
          el("p", { className: "hint", textContent: "Mixes a device into a channel as well as its apps, for sound that does not come from an app, such as a line in." }),
          el("div", { className: "aud-srcgrid" }, ...rows.flat()),
        ),
      ),
    );
  }

  function renderAdvanced() {
    const cfg = mix.config;
    if (!held("park")) {
      const spare = mix.outputs.filter((o) => o !== cfg.output).map((o) => [o, o]);
      options(mx.park, [["", "Pick one for me"], ...spare], cfg.park || "", (cfg.park || "") + " (missing)");
    }
    say(mx.parked, mix.running && mix.parked_on ? "Now using " + mix.parked_on + "." : "");
    for (const [id] of APP_CHANNELS) {
      const c = chan(id);
      if (!c || held("r" + id)) continue;
      const list = [
        ["", "Nothing, apps only"],
        ...mix.outputs.map((o) => ["out:" + o, "Everything on " + o, o === cfg.output]),
        ...mix.inputs.map((i) => ["in:" + i, i]),
      ];
      const current = c.source ? (c.loopback ? "out:" : "in:") + c.source : "";
      options(mx.sources[id], list, current, c.source + " (missing)");
    }
  }

  // ---- Data ----

  async function sendMix(change) {
    try {
      take(await invoke("mixer_set", { change }));
      say(mx.problem, problemText());
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

  const problemText = () => mix.error || (mix.running && mix.park_error) || "";

  async function loadMixer(rescan) {
    try {
      take(await invoke("get_mixer", { rescan }));
      say(mx.problem, problemText());
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
    mx.run.setAttribute("aria-label", mix.running ? "Stop the mixer" : "Start the mixer");
    if (!held("output")) options(mx.output, mix.outputs.map((o) => [o, o]), cfg.output, cfg.output + " (missing)");
    mx.chat.show(cfg.chat_mix);
    renderStrips();
    renderVmic();
    renderAdvanced();
    renderEq();
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
    CHANNELS.forEach(([id], n) => (mx.strips[id].fill.style.transform = "scaleX(" + Math.min(1, shown[n]).toFixed(3) + ")"));
  }

  // ---- Page ----

  const settings = document.getElementById("panel-settings");

  function watch() {
    const on = visible();
    if (on && !watching) loadMixer(false);
    watching = on;
    if (settings && !settings.hidden && !document.hidden) loadMixer(false);
  }

  const top = buildBar();
  mx.chat = chatMix("mcm", (b) => sendMix({ kind: "chat_mix", balance: b }));
  listen("chatmix", (e) => mx.chat.dial(e.payload));
  invoke("headset_dial").then((d) => mx.chat.dial(d)).catch(() => {});
  const pair = el("div", { className: "aud-pair" }, appStrip("game", "Game"), appStrip("chatRender", "Chat"), mx.chat.row);
  const strips = el("div", { className: "aud-strips" }, pair, appStrip("media", "Media"), appStrip("aux", "Aux"), micStrip(), masterStrip());
  for (const node of strips.querySelectorAll(".aud-strip")) node.setAttribute("aria-labelledby", node.querySelector("h3").id);
  buildEq();
  const mixer = el("section", { className: "box aud-mixer" }, el("h2", { textContent: "Mixer" }), top, mx.problem, strips);
  root.replaceChildren(el("div", { className: "aud-page" }, mixer, mx.eqBox, buildAdvanced()));

  buildSheet();
  new MutationObserver(watch).observe(root, { attributes: true, attributeFilter: ["hidden"] });
  if (settings) new MutationObserver(watch).observe(settings, { attributes: true, attributeFilter: ["hidden"] });
  document.addEventListener("visibilitychange", watch);
  setInterval(levels, 100);
  setInterval(() => watching && mix && mix.running && loadMixer(false), 2000);
  watch();
})();
