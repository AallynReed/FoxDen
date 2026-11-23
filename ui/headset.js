// Headset settings on the Devices tab, for supported SteelSeries headsets.
(function () {
  const box = document.getElementById("hs-box");
  if (!box || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const $ = (id) => document.getElementById(id);
  const panel = $("panel-devices");
  const list = $("hs-list");
  const status = $("hs-status");
  const EVERY = 20000;
  const LABELS = {
    sidetone: "Sidetone",
    mic_volume: "Mic volume",
    volume_limiter: "Volume limiter",
    auto_off: "Auto-off",
    mic_led: "Mute light",
    bt_power_on: "Bluetooth at power-on",
    bt_call: "Bluetooth call",
    anc: "Noise cancelling",
    eq_preset: "EQ preset",
    ambient_level: "Ambient sound level",
    voice_focus: "Voice focus",
    voice_prompts: "Voice prompts",
  };
  const BANDS = ["32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
  let views = [];
  let busy = 0;
  let timer = null;

  const minutes = (m) => (m === 0 ? "Never" : m + " min");
  const db = (h) => (h > 0 ? "+" : h < 0 ? "−" : "") + Math.abs(h / 2).toFixed(1);

  async function send(cmd, args, what) {
    busy++;
    setText(status, "");
    try {
      await invoke(cmd, args);
    } catch (e) {
      setText(status, sentence(String(e)) + ". " + what + " was not changed.");
    } finally {
      busy--;
    }
    refresh();
  }

  function setControl(h, c, value) {
    c.value = value;
    send("headset_set", { key: h.key, control: c.id, value }, LABELS[c.id] || c.id);
  }

  function readout(c, text) {
    const num = el("span", { textContent: text });
    const out = el("span", { className: "hs-val" }, num);
    out.num = num;
    if (c.value != null && !c.read) out.append(el("span", { className: "hs-src", textContent: "last set", title: "The headset does not report this. FoxDen shows what it last sent." }));
    return out;
  }

  function steps(h, c, off) {
    const { min, max } = c.shape;
    const input = el("input", { type: "range", min, max, step: 1, value: c.value ?? min, disabled: off, className: c.value == null ? "unset" : "" });
    input.setAttribute("aria-label", LABELS[c.id]);
    const val = readout(c, c.value == null ? "—" : c.value + "/" + max);
    input.oninput = () => {
      input.classList.remove("unset");
      setText(val.num, input.value + "/" + max);
    };
    input.onchange = () => setControl(h, c, Number(input.value));
    return [input, val];
  }

  function toggle(h, c, off) {
    const input = el("input", { type: "checkbox", className: "switch", checked: c.value === 1, disabled: off });
    input.setAttribute("aria-label", LABELS[c.id]);
    input.onchange = () => setControl(h, c, input.checked ? 1 : 0);
    return [input, readout(c, c.value == null ? "Not set" : "")];
  }

  function segments(h, c, off, options) {
    const seg = el("div", { className: "hs-seg", role: "group" });
    seg.setAttribute("aria-label", LABELS[c.id]);
    for (const [v, name] of options) {
      const b = el("button", { type: "button", textContent: name, disabled: off });
      b.setAttribute("aria-pressed", String(c.value === v));
      b.onclick = () => c.value !== v && setControl(h, c, v);
      seg.append(b);
    }
    const custom = c.id === "eq_preset" && c.value === 4;
    return [seg, readout(c, c.value == null ? "—" : custom ? "Custom" : "")];
  }

  function dropdown(h, c, off, options) {
    const sel = el("select", { disabled: off });
    sel.setAttribute("aria-label", LABELS[c.id]);
    if (c.value == null) sel.append(el("option", { value: "", textContent: "Not set", disabled: true, selected: true }));
    else if (!options.some(([v]) => v === c.value)) options = [...options, [c.value, minutes(c.value)]].sort((a, b) => a[0] - b[0]);
    for (const [v, name] of options) sel.append(el("option", { value: v, textContent: name, selected: c.value === v }));
    sel.onchange = () => setControl(h, c, Number(sel.value));
    return [sel, readout(c, "")];
  }

  function control(h, c, off) {
    const s = c.shape;
    let parts;
    if (s.type === "steps") parts = steps(h, c, off);
    else if (s.type === "switch") parts = toggle(h, c, off);
    else if (s.type === "minutes") parts = dropdown(h, c, off, s.presets.map((m) => [m, minutes(m)]));
    else if (s.options.length > 4) parts = dropdown(h, c, off, s.options);
    else parts = segments(h, c, off, s.options);
    const inline = s.type === "switch" || parts[0].tagName === "SELECT";
    return el("div", { className: "hs-row" + (inline ? " hs-inline" : "") }, el("span", { className: "hs-label", textContent: LABELS[c.id] || c.id }), ...parts);
  }

  function eq(h, off) {
    const bands = (h.eq.bands || Array(10).fill(0)).slice();
    const out = el("div", { className: "hs-eq" });
    const sliders = el("div", { className: "hs-bands" });
    BANDS.forEach((f, i) => {
      const val = el("span", { className: "hs-db", textContent: db(bands[i]) });
      const input = el("input", { type: "range", min: h.eq.min, max: h.eq.max, step: 1, value: bands[i], disabled: off });
      input.setAttribute("aria-label", f + " Hz");
      input.oninput = () => {
        bands[i] = Number(input.value);
        setText(val, db(bands[i]));
      };
      sliders.append(el("label", { className: "hs-band" }, val, input, el("span", { className: "hs-hz", textContent: f })));
    });
    const apply = el("button", { type: "button", className: "primary", textContent: "Apply EQ", disabled: off });
    apply.onclick = () => send("headset_eq", { key: h.key, bands }, "The EQ");
    const head = el("div", { className: "hs-row hs-inline" }, el("span", { className: "hs-label", textContent: "Custom EQ, dB" }), el("span", { className: "hs-val", textContent: h.eq.bands ? "Last set by FoxDen" : "Not set yet" }));
    out.append(head, sliders, el("div", { className: "actions" }, apply));
    return out;
  }

  function facts(h) {
    const out = [];
    if (h.battery != null) out.push("Battery " + h.battery + "%" + (h.charging ? ", charging" : ""));
    if (h.chatmix) out.push("ChatMix game " + h.chatmix[0] + " / chat " + h.chatmix[1]);
    return out.join(" · ");
  }

  function factsLine(h) {
    const span = el("span", { className: "meta hs-facts", textContent: facts(h) });
    span.dataset.key = h.key;
    return span;
  }

  function card(h) {
    const off = !h.connected;
    const tag = el("span", { className: "state" });
    if (!h.answered) setState(tag, "warn", "Not answering");
    else if (off) setState(tag, "off", "Headset off");
    else setState(tag, "on", "Connected");
    const grid = el("div", { className: "hs-grid" }, ...h.controls.map((c) => control(h, c, off)));
    const out = el(
      "li",
      { className: "hs-card" },
      el("div", { className: "split" }, el("span", { className: "name", textContent: h.name, title: h.name }), tag),
      factsLine(h),
      grid,
    );
    if (h.eq) out.append(eq(h, off));
    return out;
  }

  function render() {
    box.hidden = !views.length;
    const key = JSON.stringify(views);
    if (list.dataset.key === key) return;
    list.dataset.key = key;
    list.replaceChildren(...views.map(card));
  }

  async function refresh() {
    if (busy) return;
    try {
      views = await invoke("headset_list");
    } catch (e) {
      views = [];
    }
    if (!busy) render();
  }

  function tick() {
    clearTimeout(timer);
    timer = setTimeout(tick, EVERY);
    if (!panel.hidden && !document.hidden && !list.contains(document.activeElement)) refresh();
  }

  window.__TAURI__.event.listen("chatmix", (e) => {
    const d = e.payload;
    for (const h of views) {
      if (!h.chatmix || !d) continue;
      h.chatmix = [d.game, d.chat];
      const span = list.querySelector('.hs-facts[data-key="' + h.key + '"]');
      if (span) span.textContent = facts(h);
    }
  });

  $("tab-devices").addEventListener("click", () => refresh());
  refresh();
  tick();
})();
