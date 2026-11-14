// Fans tab. Fills #panel-fans.
(function () {
  const root = document.getElementById("panel-fans");
  if (!root || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  // The status bar's passing line, from main.js.
  const tell = window.note;

  const SVG_NS = "http://www.w3.org/2000/svg";
  const T_MIN = 20;
  const T_MAX = 100;
  const EMPTY = '{"curves":[],"rules":[]}';
  const WORDS = { running: "Running", stopped: "Stopped", error: "Error", start: "Starting", stop: "Stopping" };
  const MARKS = { running: "mark on", stopped: "mark", error: "mark warn" };
  const NOTICE =
    "FoxRGB cannot drive this PC's fan headers yet, because the hardware helper is not built. " +
    "Temperatures are real where the service can read them. Simulated fans let you try curves until then.";
  const OFFLINE = "The service is not running. Start it under Status to see the fans and edit curves.";

  const ui = {};
  const plot = { w: 0, h: 0, x0: 38, x1: 0, y0: 12, y1: 0 };
  let live = { service: "", detail: null, version: null, sim: false, sensors: [], fans: [] };
  // The service's config, as text and parsed.
  let reported = EMPTY;
  let saved = JSON.parse(EMPTY);
  // The config being edited and the text it started from.
  let draft = JSON.parse(EMPTY);
  let base = EMPTY;
  let chosen = 0;
  let view = "status";
  let past = [];
  let pending = "";
  let problem = null;
  let revealed = null;
  let note = "";
  let drag = null;
  let asking = false;
  let serial = 0;
  let queued = false;

  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  const trim = (v) => String(Math.round(v * 10) / 10);
  const percent = (v) => Math.round(v * 100) + "%";
  const reading = (v) => (v == null ? "no reading" : v.toFixed(1) + " °C");
  const speed = (v) => (v == null ? "no reading" : v.toLocaleString("en-GB") + " rpm");
  // What a fan is doing now: duty when FoxRGB sets it, then speed.
  const output = (fan) => (fan.duty == null ? "" : percent(fan.duty) + " · ") + speed(fan.rpm);
  const message = (e) => (typeof e === "string" ? e : (e && e.message) || String(e));
  const sentence = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  const quoted = (name) => '"' + name + '"';
  const list = new Intl.ListFormat("en-GB", { type: "conjunction" });

  function el(tag, props, ...children) {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  }

  function svg(tag, attrs, ...children) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const name in attrs) node.setAttribute(name, attrs[name]);
    node.append(...children);
    return node;
  }

  function setText(node, text) {
    if (node.textContent !== text) node.textContent = text;
  }

  const button = (text, className = "") => el("button", { type: "button", className, textContent: text });
  const number = (min, max, step) => el("input", { type: "number", min, max, step });

  // A place for a message about the control above it.
  function slot(...inputs) {
    const p = el("p", { className: "problem", id: "fan-problem-" + ++serial, hidden: true });
    p.setAttribute("role", "alert");
    for (const input of inputs) input.setAttribute("aria-describedby", p.id);
    return p;
  }

  function say(place, text) {
    place.hidden = !text;
    setText(place, text);
  }

  // Buttons stay focusable while they are unavailable.
  function able(control, on) {
    control.setAttribute("aria-disabled", String(!on));
  }

  function press(control, action) {
    control.addEventListener("click", () => control.getAttribute("aria-disabled") !== "true" && action());
  }

  function tidy(config) {
    return {
      curves: ((config && config.curves) || []).map((c) => ({
        name: c.name,
        sensors: [...c.sensors],
        points: c.points.map((p) => [p[0], p[1]]),
        hysteresis: c.hysteresis,
      })),
      // In a fixed order, so that taking a rule away and putting it back is not a change.
      rules: ((config && config.rules) || [])
        .map((r) => ({
          fan: r.fan,
          curve: r.curve,
          min_duty: r.min_duty,
          max_duty: r.max_duty,
          ramp: r.ramp,
        }))
        .sort((a, b) => (a.fan < b.fan ? -1 : a.fan > b.fan ? 1 : 0)),
    };
  }

  const pack = (config) => JSON.stringify(tidy(config));
  const edited = () => pack(draft) !== base;
  const told = (where, key) => (problem && problem.where === where && problem.key === key ? problem.text : "");

  function typingIn(box) {
    const a = document.activeElement;
    return /^(text|number)$/.test(a && a.type) && box.contains(a);
  }

  // True while the user is typing in an editor field or dragging a point.
  const locked = () => Boolean(drag) || typingIn(ui.curves) || typingIn(ui.rules);

  // The draft follows the service until the user changes something.
  function sync() {
    const mine = pack(draft);
    if (mine === reported) {
      base = reported;
    } else if (mine === base && !locked()) {
      const name = draft.curves[chosen] && draft.curves[chosen].name;
      draft = JSON.parse(reported);
      base = reported;
      past = [];
      asking = false;
      const again = draft.curves.findIndex((c) => c.name === name);
      chosen = again >= 0 ? again : clamp(chosen, 0, Math.max(0, draft.curves.length - 1));
    }
  }

  function take(state) {
    live = state;
    // Without a running service the config in the state is only a placeholder.
    if (state.service !== "running") {
      note = "";
      return;
    }
    reported = pack(state.config);
    saved = JSON.parse(reported);
    sync();
  }

  async function refresh() {
    take(await invoke("get_fans"));
  }

  // Every change to the draft goes through here so that it can be undone.
  function edit(mutate) {
    const before = pack(draft);
    mutate();
    const changed = pack(draft) !== before;
    if (changed) {
      past.push(before);
      if (past.length > 100) past.shift();
      note = "";
    }
    // What the service objected to stays on screen until the draft changes.
    if (changed || !(problem && problem.rejected)) problem = null;
    render();
  }

  function undo() {
    if (!past.length || pending) return;
    draft = JSON.parse(past.pop());
    chosen = clamp(chosen, 0, Math.max(0, draft.curves.length - 1));
    asking = false;
    problem = null;
    note = "Undone";
    render();
    hold();
  }

  // Keeps focus in the tab when the control that had it has gone.
  function hold() {
    const active = document.activeElement;
    if (!root.contains(active) || !active.checkVisibility()) (ui.bar.hidden ? ui.views.querySelector(":checked") : ui.undo).focus();
  }

  function revert() {
    if (!edited() || pending) return;
    past.push(pack(draft));
    draft = JSON.parse(reported);
    base = reported;
    chosen = clamp(chosen, 0, Math.max(0, draft.curves.length - 1));
    asking = false;
    problem = null;
    note = "Reverted to what the service has";
    render();
  }

  // Takes a typed number into the draft, within its range, and says so when it was outside it.
  function commit(input, lo, hi, what, at, put, shown) {
    const v = input.value.trim() === "" ? NaN : Number(input.value);
    if (Number.isFinite(v)) edit(() => put(clamp(v, lo, hi)));
    if (!(v >= lo && v <= hi)) {
      problem = { ...at, text: what + " must be a number from " + lo + " to " + hi + "." };
      render();
    }
    input.value = shown();
  }

  function dutyAt(points, temp) {
    if (temp <= points[0][0]) return points[0][1];
    for (let i = 1; i < points.length; i++) {
      const [t0, d0] = points[i - 1];
      const [t1, d1] = points[i];
      if (temp <= t1) return d0 + ((d1 - d0) * (temp - t0)) / (t1 - t0);
    }
    return points[points.length - 1][1];
  }

  function hottest(curve) {
    let best = null;
    for (const s of live.sensors) {
      if (curve.sensors.includes(s.id) && s.value != null && (!best || s.value > best.value)) best = s;
    }
    return best;
  }

  // A point stays between its neighbours, so the order never changes.
  function setPoint(curve, i, temp, duty) {
    const points = curve.points;
    const lo = i > 0 ? points[i - 1][0] + 1 : T_MIN;
    const hi = i < points.length - 1 ? points[i + 1][0] - 1 : T_MAX;
    if (lo <= hi) points[i][0] = clamp(Math.round(temp * 10) / 10, lo, hi);
    points[i][1] = clamp(Math.round(duty * 100) / 100, 0, 1);
  }

  // The middle of the widest gap, or null when the curve is full.
  function roomFor(points) {
    const temps = [T_MIN - 1, ...points.map((p) => p[0]), T_MAX + 1];
    let at = null;
    let widest = 1.99;
    for (let i = 1; i < temps.length; i++) {
      if (temps[i] - temps[i - 1] > widest) {
        widest = temps[i] - temps[i - 1];
        at = Math.round((temps[i] + temps[i - 1]) / 2);
      }
    }
    return at;
  }

  function addPoint(temp, from) {
    const points = draft.curves[chosen].points;
    const t = clamp(Math.round(temp === undefined ? roomFor(points) : temp), T_MIN, T_MAX);
    if (points.some((p) => Math.abs(p[0] - t) < 1)) {
      problem = { where: "points", key: chosen, text: "There is already a point at " + t + " °C." };
      return render();
    }
    const duty = Math.round(dutyAt(points, t) * 100) / 100;
    edit(() => {
      points.push([t, duty]);
      points.sort((a, b) => a[0] - b[0]);
    });
    const i = points.findIndex((p) => p[0] === t);
    (from === "graph" ? ui.handles.children[i] : ui.points.children[i].parts.temp).focus();
  }

  function removePoint(i, from) {
    const points = draft.curves[chosen].points;
    if (points.length < 2) {
      problem = { where: "points", key: chosen, text: "A curve needs at least one point." };
      return render();
    }
    edit(() => points.splice(i, 1));
    const next = Math.min(i, points.length - 1);
    (from === "graph" ? ui.handles.children[next] : ui.points.children[next].parts.remove).focus();
  }

  function freeName(stem, bare) {
    const taken = (name) => draft.curves.some((c) => c.name === name);
    if (bare && !taken(stem)) return stem;
    for (let n = bare ? 2 : 1; ; n++) {
      if (!taken(stem + " " + n)) return stem + " " + n;
    }
  }

  function addCurve(from) {
    const sensor = live.sensors.find((s) => s.value != null);
    edit(() => {
      draft.curves.push(
        from
          ? { ...tidy({ curves: [from] }).curves[0], name: freeName(from.name.slice(0, 32) + " copy", true) }
          : {
              name: freeName("Curve", false),
              sensors: sensor ? [sensor.id] : [],
              points: [[30, 0.25], [50, 0.4], [70, 0.75], [85, 1]],
              hysteresis: 2,
            },
      );
      chosen = draft.curves.length - 1;
    });
    ui.name.focus();
    ui.name.select();
  }

  function deleteCurve() {
    asking = false;
    edit(() => {
      const [gone] = draft.curves.splice(chosen, 1);
      draft.rules = draft.rules.filter((r) => r.curve !== gone.name);
      chosen = Math.max(0, Math.min(chosen, draft.curves.length - 1));
    });
    (draft.curves.length ? ui.remove : ui.add).focus();
  }

  function renameCurve() {
    const curve = draft.curves[chosen];
    const name = ui.name.value.trim();
    let wrong = "";
    if (!name) wrong = "A curve needs a name.";
    else if (draft.curves.some((c) => c !== curve && c.name === name)) wrong = "Another curve is already called " + name + ".";
    if (wrong) {
      problem = { where: "name", key: chosen, text: wrong };
      render();
    } else {
      edit(() => {
        for (const rule of draft.rules) if (rule.curve === curve.name) rule.curve = name;
        curve.name = name;
      });
    }
    ui.name.value = curve.name;
  }

  function fanName(id) {
    const fan = live.fans.find((f) => f.id === id);
    return fan ? fan.name : id;
  }

  // The sensors of a curve that give no reading, by name.
  function lost(curve) {
    const sensor = (id) => live.sensors.find((s) => s.id === id);
    return curve.sensors.filter((id) => !sensor(id) || sensor(id).value == null).map((id) => (sensor(id) ? sensor(id).name : id));
  }

  const silent = (names) => list.format(names) + (names.length === 1 ? " gives" : " give") + " no reading";

  // What is wrong with a fan right now, in words, or nothing.
  function trouble(fan) {
    if (fan.error) return "Error: " + fan.error;
    if (!fan.failsafe) return "";
    const curve = saved.curves.find((c) => c.name === fan.curve);
    const gone = curve ? lost(curve) : [];
    return "Fail-safe: full speed, because " + (gone.length ? silent(gone) : "a sensor of " + fan.curve + " gives no reading");
  }

  // Fans the service reports, then fans that only a rule still names.
  function ruleFans() {
    const fans = live.fans.map((f) => ({ id: f.id, name: f.name, live: f }));
    for (const rule of draft.rules) {
      if (!fans.some((f) => f.id === rule.fan)) fans.push({ id: rule.fan, name: rule.fan, live: null });
    }
    return fans;
  }

  function go(name) {
    view = name;
    ui.views.querySelector('[value="' + name + '"]').checked = true;
  }

  async function service(action) {
    const from = action === "start" ? ui.start : ui.stop;
    pending = action;
    problem = null;
    render();
    try {
      await invoke(action + "_service");
      await refresh();
    } catch (e) {
      problem = { where: "service", text: sentence(message(e)) };
    }
    pending = "";
    render();
    if (from.hidden) (from === ui.start ? ui.stop : ui.start).focus();
  }

  async function simulate() {
    pending = "sim";
    problem = null;
    try {
      await invoke("set_fan_sim", { on: ui.sim.checked });
      await refresh();
    } catch (e) {
      problem = { where: "sim", text: sentence(message(e)) };
    }
    pending = "";
    render();
  }

  // What the service would refuse in the draft, with the control that puts it right.
  function fault() {
    const bare = draft.curves.findIndex((c) => !c.sensors.length);
    if (bare >= 0) {
      return { view: "curves", curve: bare, text: "Curve " + quoted(draft.curves[bare].name) + " has no sensor. Choose at least one.", target: () => ui.choices.querySelector("input") || ui.name };
    }
    const rule = draft.rules.find((r) => r.min_duty > r.max_duty);
    if (!rule) return null;
    const card = () => [...ui.ruleList.children].find((c) => c.fan === rule.fan);
    return { view: "rules", text: fanName(rule.fan) + ": the minimum must not be above the maximum.", target: () => card() && card().parts.min };
  }

  async function apply() {
    if (!edited() || pending || live.service !== "running") return;
    // Nothing is sent while the draft has a fault this tab can see. The key or button leads to it instead.
    const wrong = fault();
    if (wrong) {
      if (wrong.curve !== undefined) chosen = wrong.curve;
      go(wrong.view);
      problem = { where: "apply", text: "Not applied. " + wrong.text, rejected: true };
      render();
      const target = wrong.target();
      if (target) target.focus();
      return;
    }
    const sent = pack(draft);
    pending = "apply";
    problem = null;
    render();
    try {
      await invoke("set_fan_config", { config: JSON.parse(sent) });
    } catch (e) {
      pending = "";
      return reject(message(e));
    }
    if (pack(draft) === sent) base = sent;
    note = "Applied";
    await refresh().catch(() => {});
    pending = "";
    render();
  }

  // The service's reasons for refusing a config, in the words and units of this tab.
  const REFUSALS = [
    [/^curve name '(.*)' is used more than once$/, (c) => "Two curves are called " + quoted(c) + ". Give one of them another name."],
    [/^curve '(.*)' has no sensors$/, (c) => "Curve " + quoted(c) + " has no sensor. Choose at least one."],
    [/^curve '(.*)' has no points$/, (c) => "Curve " + quoted(c) + " has no points. Add at least one."],
    [/^curve '(.*)' has a point that is not a finite number$/, (c) => "A point of curve " + quoted(c) + " is not a number."],
    [/^curve '(.*)' has a duty outside 0 to 1$/, (c) => "A duty of curve " + quoted(c) + " is outside 0 to 100%."],
    [/^curve '(.*)' points must be sorted by rising temperature$/, (c) => "Each point of curve " + quoted(c) + " must be hotter than the one before it."],
    [/^curve '(.*)' hysteresis must be from 0 to 30$/, (c) => "The hysteresis of curve " + quoted(c) + " must be from 0 to 30 °C."],
    [/^fan '(.*)' has more than one rule$/, (f) => fanName(f) + " has more than one rule."],
    [/^fan '(.*)' uses unknown curve '(.*)'$/, (f, c) => fanName(f) + " follows " + quoted(c) + ", which is not among the curves."],
    [/^fan '(.*)' needs duties from 0 to 1, /, (f) => fanName(f) + ": the minimum must not be above the maximum, and both must be from 0 to 100%."],
    [/^fan '(.*)' ramp must be /, (f) => fanName(f) + ": the ramp must be at least 1% per second."],
  ];
  const UNSAVED = "The changes are in use, but the service could not save them. They will be gone when it starts again.";

  function plain(text) {
    // The one refusal that comes after the config has taken effect.
    if (text.startsWith("config applied")) return UNSAVED;
    for (const [pattern, say] of REFUSALS) {
      const found = pattern.exec(text);
      if (found) return "Not applied. " + say(...found.slice(1));
    }
    return "Not applied. " + sentence(text) + ".";
  }

  // Puts the service's objection beside the control it is about and goes there.
  function reject(text) {
    const said = plain(text);
    const longest = (a, b) => b.length - a.length;
    const fan = ruleFans()
      .map((f) => f.id)
      .filter((id) => text.includes("fan '" + id + "'"))
      .sort(longest)[0];
    const name = draft.curves
      .map((c) => c.name)
      .filter((n) => text.includes("curve '" + n + "'") || text.includes("curve name '" + n + "'"))
      .sort(longest)[0];
    // Read without the name, so that a curve called "CPU sensor" is not taken for a sensor problem.
    const rest = text.replace("'" + (fan === undefined ? name : fan) + "'", "");
    let target = ui.apply;
    if (fan !== undefined) {
      problem = { where: "rule", key: fan, text: said, rejected: true };
      go("rules");
    } else if (name !== undefined) {
      chosen = draft.curves.findLastIndex((c) => c.name === name);
      const where = /hysteresis/.test(rest) ? "hysteresis" : /sensor/.test(rest) ? "sensors" : /curve name/.test(rest) ? "name" : "points";
      problem = { where, key: chosen, text: said, rejected: true };
      go("curves");
    } else {
      problem = { where: "apply", text: said, rejected: true };
    }
    render();
    if (problem.where === "rule") {
      const card = [...ui.ruleList.children].find((c) => c.fan === fan);
      if (card) target = /ramp/.test(rest) ? card.parts.ramp : /duty/.test(rest) ? card.parts.min : card.parts.curve;
    } else if (problem.where === "name") target = ui.name;
    else if (problem.where === "hysteresis") target = ui.hyst;
    else if (problem.where === "sensors") target = ui.choices.querySelector("input") || ui.name;
    else if (problem.where === "points") target = ui.points.querySelector("input");
    target.focus();
  }

  function buildViews() {
    ui.views = el("div", { className: "fan-views" });
    ui.views.setAttribute("role", "radiogroup");
    ui.views.setAttribute("aria-label", "Fan sections");
    ui.marks = {};
    for (const [value, label] of [["status", "Status"], ["curves", "Curves"], ["rules", "Rules"]]) {
      const radio = el("input", { type: "radio", name: "fan-view", value, checked: value === view });
      radio.onchange = () => {
        view = value;
        render();
      };
      ui.marks[value] = el("span", { className: "fan-dot", hidden: true, textContent: "unsaved changes" });
      ui.views.append(el("label", { className: "pick" }, radio, el("span", { textContent: label }), ui.marks[value]));
    }
  }

  function buildStatus() {
    ui.shape = el("span", { className: "mark none" });
    ui.word = el("strong");
    ui.version = el("span", { className: "fan-num" });
    const state = el("p", { className: "fan-state" }, ui.shape, ui.word, ui.version);
    state.setAttribute("role", "status");
    ui.detail = el("p", { className: "hint" });
    ui.start = button("Start for this session", "primary");
    ui.stop = button("Stop");
    ui.serviceProblem = slot();
    ui.notice = el("p", { className: "fan-note", textContent: NOTICE });
    ui.sim = el("input", { type: "checkbox", className: "switch", id: "fan-sim" });
    ui.simRow = el("div", { className: "field" }, el("label", { htmlFor: "fan-sim", textContent: "Simulated fans" }), ui.sim);
    ui.simProblem = slot(ui.sim);
    ui.sensors = el("ul", { className: "list fan-rows" });
    ui.sensors.setAttribute("aria-labelledby", "fan-sensors-head");
    ui.noSensors = el("p", { className: "hint", textContent: "The service reports no temperature sensors." });
    ui.fans = el("ul", { className: "list fan-rows" });
    ui.fans.setAttribute("aria-labelledby", "fan-fans-head");
    ui.noFans = el("p", { className: "hint", textContent: "No fans to show. Switch on simulated fans to try curves." });
    ui.readings = el(
      "div",
      { className: "fan-group" },
      el("h2", { id: "fan-sensors-head", textContent: "Sensors" }),
      ui.sensors,
      ui.noSensors,
      el("h2", { id: "fan-fans-head", textContent: "Fans" }),
      ui.fans,
      ui.noFans,
    );
    ui.status = el(
      "div",
      { className: "fan-view" },
      el("h2", { textContent: "Service" }),
      state,
      ui.detail,
      el("div", { className: "actions" }, ui.start, ui.stop),
      ui.serviceProblem,
      ui.notice,
      ui.simRow,
      ui.simProblem,
      ui.readings,
    );
    press(ui.start, () => service("start"));
    press(ui.stop, () => service("stop"));
    ui.sim.onchange = simulate;
  }

  function buildGraph() {
    ui.svg = svg("svg", { role: "group", "aria-label": "Curve graph, duty against temperature. The table below lists the same points." });
    ui.grid = svg("g", { class: "fan-grid", "aria-hidden": "true" });
    ui.area = svg("path", { class: "fan-area" });
    ui.nowLine = svg("line", { class: "fan-now-line" });
    ui.line = svg("path", { class: "fan-line" });
    ui.hit = svg("path", { class: "fan-add" }, svg("title", {}, "Add a point here"));
    ui.nowMark = svg("path", { class: "fan-now-mark", d: "M0,-6L6,0L0,6L-6,0Z" });
    ui.handles = svg("g", {});
    ui.svg.append(ui.grid, ui.area, ui.nowLine, ui.line, ui.hit, ui.nowMark, ui.handles);
    ui.graph = el("div", { className: "fan-graph" }, ui.svg);
    ui.hit.addEventListener("click", (e) => addPoint(pointer(e)[0], "graph"));
    new ResizeObserver(() => sizeGraph() && drawGraph()).observe(ui.graph);
  }

  function buildCurves() {
    ui.curvesOff = el("p", { className: "hint", textContent: OFFLINE });
    ui.curveList = el("div", { className: "picks fan-curves" });
    ui.curveList.setAttribute("role", "radiogroup");
    ui.curveList.setAttribute("aria-labelledby", "fan-curves-head");
    ui.noCurves = el("p", { className: "hint", textContent: "No curves yet. A curve sets a fan's duty from a temperature." });
    ui.add = button("Add");
    ui.copy = button("Duplicate");
    ui.remove = button("Delete");
    ui.curveActions = el("div", { className: "actions" }, ui.add, ui.copy, ui.remove);
    ui.askText = el("p");
    ui.askYes = button("Delete curve");
    ui.askNo = button("Cancel");
    ui.ask = el("div", { className: "fan-ask", hidden: true }, ui.askText, el("div", { className: "actions" }, ui.askYes, ui.askNo));
    ui.ask.setAttribute("role", "group");
    ui.ask.setAttribute("aria-label", "Confirm deleting the curve");

    ui.name = el("input", { type: "text", id: "fan-curve-name", maxLength: 40, autocomplete: "off", spellcheck: false });
    ui.nameProblem = slot(ui.name);
    buildGraph();
    ui.now = el("p", { className: "fan-now" });
    ui.points = el("tbody");
    const head = el(
      "tr",
      {},
      el("th", { scope: "col" }, el("span", { className: "sr-only", textContent: "Point" })),
      el("th", { scope: "col", textContent: "Temperature" }),
      el("th", { scope: "col", textContent: "Duty" }),
      el("th", { scope: "col" }, el("span", { className: "sr-only", textContent: "Remove" })),
    );
    const table = el("table", { className: "fan-points" }, el("thead", {}, head), ui.points);
    table.setAttribute("aria-labelledby", "fan-points-head");
    ui.pointsProblem = slot();
    ui.addPoint = button("Add point");
    ui.choices = el("div", { className: "fan-checks" });
    ui.choices.setAttribute("role", "group");
    ui.choices.setAttribute("aria-labelledby", "fan-choices-head");
    ui.noChoices = el("p", { className: "hint", textContent: "The service reports no sensors to choose from." });
    ui.sensorsProblem = slot(ui.choices);
    ui.hyst = number(0, 30, 0.5);
    ui.hyst.id = "fan-hysteresis";
    ui.hystProblem = slot(ui.hyst);

    ui.editor = el(
      "div",
      { className: "fan-group" },
      el("div", { className: "fields" }, el("label", { htmlFor: "fan-curve-name", textContent: "Name" }), ui.name),
      ui.nameProblem,
      el("h2", { id: "fan-points-head", textContent: "Points" }),
      ui.graph,
      ui.now,
      el("p", {
        className: "hint",
        textContent: "Drag a point to move it, or click the line to add one. With a point focused, the arrow keys move it and Delete removes it.",
      }),
      table,
      ui.pointsProblem,
      el("div", { className: "actions" }, ui.addPoint),
      el("h2", { id: "fan-choices-head", textContent: "Sensors" }),
      ui.choices,
      ui.noChoices,
      el("p", {
        className: "hint",
        textContent: "The hottest of the chosen sensors drives the curve. If one of them stops reading, the fans on this curve run at full speed.",
      }),
      ui.sensorsProblem,
      el("div", { className: "field" }, el("label", { htmlFor: "fan-hysteresis", textContent: "Hysteresis (°C)" }), ui.hyst),
      el("p", { className: "hint", textContent: "How far the temperature must fall before the duty drops." }),
      ui.hystProblem,
    );
    ui.curvesBody = el(
      "div",
      { className: "fan-group" },
      el("h2", { id: "fan-curves-head", textContent: "Curves" }),
      ui.curveList,
      ui.noCurves,
      ui.curveActions,
      ui.ask,
      ui.editor,
    );
    ui.curves = el("div", { className: "fan-view", hidden: true }, ui.curvesOff, ui.curvesBody);

    press(ui.add, () => addCurve());
    press(ui.copy, () => addCurve(draft.curves[chosen]));
    press(ui.remove, () => {
      asking = true;
      render();
      ui.askNo.focus();
    });
    press(ui.askYes, deleteCurve);
    press(ui.askNo, () => {
      asking = false;
      render();
      ui.remove.focus();
    });
    ui.ask.addEventListener("keydown", (e) => {
      if (e.key !== "Escape") return;
      // Handled here, so that it does not also deselect a device on the canvas.
      e.preventDefault();
      ui.askNo.click();
    });
    press(ui.addPoint, () => addPoint(undefined, "table"));
    ui.name.onchange = renameCurve;
    ui.hyst.onchange = () => {
      const curve = draft.curves[chosen];
      const at = { where: "hysteresis", key: chosen };
      commit(ui.hyst, 0, 30, "Hysteresis", at, (v) => (curve.hysteresis = Math.round(v * 10) / 10), () => curve.hysteresis);
    };
  }

  function buildRules() {
    ui.rulesOff = el("p", { className: "hint", textContent: OFFLINE });
    ui.ruleList = el("ul", { className: "list" });
    ui.ruleList.setAttribute("aria-labelledby", "fan-rules-head");
    ui.noRules = el("p", { className: "hint", textContent: "No fans to control. Switch on simulated fans under Status to try curves." });
    ui.rulesBody = el(
      "div",
      { className: "fan-group" },
      el("h2", { id: "fan-rules-head", textContent: "Rules" }),
      el("p", {
        className: "hint",
        textContent: "A fan follows one curve. Its duty stays between the minimum and the maximum and changes no faster than the ramp.",
      }),
      ui.ruleList,
      ui.noRules,
    );
    ui.rules = el("div", { className: "fan-view", hidden: true }, ui.rulesOff, ui.rulesBody);
  }

  function buildBar() {
    ui.saveDot = el("span", { className: "fan-dot", hidden: true });
    ui.saveText = el("span");
    ui.save = el("p", { className: "fan-save" }, ui.saveDot, ui.saveText);
    ui.save.setAttribute("role", "status");
    ui.apply = button("Apply", "primary");
    ui.revert = button("Revert");
    ui.undo = button("Undo edit");
    ui.apply.title = "Ctrl+S";
    ui.apply.setAttribute("aria-keyshortcuts", "Control+S");
    ui.undo.title = "Ctrl+Z";
    ui.undo.setAttribute("aria-keyshortcuts", "Control+Z");
    ui.bar = el("div", { className: "fan-apply", hidden: true }, ui.save, el("div", { className: "actions" }, ui.apply, ui.revert, ui.undo));
    press(ui.apply, apply);
    press(ui.revert, revert);
    press(ui.undo, undo);
  }

  function sensorRow() {
    const name = el("span", { className: "name" });
    const value = el("span", { className: "fan-num" });
    const row = el("li", {}, name, value);
    row.parts = { name, value };
    return row;
  }

  function fanRow() {
    const name = el("span", { className: "name" });
    const now = el("span", { className: "fan-num" });
    const fill = el("i");
    const bar = el("span", { className: "fan-bar" }, fill);
    const words = el("span", { className: "fan-words hint" });
    const row = el("li", { className: "fan-item" }, name, now, bar, words);
    row.parts = { name, now, fill, bar, words };
    return row;
  }

  function curvePick(i) {
    const radio = el("input", { type: "radio", name: "fan-curve", value: i });
    const name = el("span", { className: "name" });
    const tag = el("span", { className: "fan-tag" });
    radio.onchange = () => {
      chosen = i;
      asking = false;
      problem = null;
      render();
    };
    const row = el("label", { className: "pick" }, radio, name, tag);
    row.parts = { radio, name, tag };
    return row;
  }

  // Position of a pointer event in the graph, as temperature and duty.
  function pointer(e) {
    const box = ui.svg.getBoundingClientRect();
    const k = plot.w / box.width;
    const temp = T_MIN + (((e.clientX - box.left) * k - plot.x0) / (plot.x1 - plot.x0)) * (T_MAX - T_MIN);
    return [temp, (plot.y1 - (e.clientY - box.top) * k) / (plot.y1 - plot.y0)];
  }

  function highlight(i) {
    [...ui.handles.children].forEach((g, n) => g.classList.toggle("on", n === i));
    [...ui.points.children].forEach((row, n) => row.classList.toggle("on", n === i));
  }

  function handle(i) {
    const g = svg(
      "g",
      { class: "fan-handle", tabindex: 0, role: "slider", "aria-orientation": "vertical", "aria-valuemin": 0, "aria-valuemax": 100 },
      svg("rect", { class: "hit", x: -12, y: -12, width: 24, height: 24 }),
      svg("rect", { class: "ring", x: -9, y: -9, width: 18, height: 18 }),
      svg("rect", { class: "dot", x: -5, y: -5, width: 10, height: 10 }),
    );
    const drop = () => {
      if (!drag) return;
      const before = drag.before;
      drag = null;
      if (pack(draft) !== before) {
        past.push(before);
        problem = null;
        note = "";
      }
      sync();
      render();
    };
    g.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      g.setPointerCapture(e.pointerId);
      drag = { i, before: pack(draft), x: e.clientX, y: e.clientY, moved: false };
    });
    g.addEventListener("pointermove", (e) => {
      if (!drag || drag.i !== i) return;
      if (!drag.moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 3) return;
      drag.moved = true;
      const [temp, duty] = pointer(e);
      setPoint(draft.curves[chosen], i, Math.round(temp), duty);
      queue();
    });
    g.addEventListener("pointerup", drop);
    g.addEventListener("pointercancel", drop);
    g.addEventListener("keydown", (e) => {
      const step = e.shiftKey ? 5 : 1;
      const move = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
      if (e.key === "Escape" && drag) {
        e.preventDefault();
        draft = JSON.parse(drag.before);
        drag = null;
        render();
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        removePoint(i, "graph");
      } else if (move) {
        e.preventDefault();
        const curve = draft.curves[chosen];
        edit(() => setPoint(curve, i, curve.points[i][0] + move[0], curve.points[i][1] + move[1] / 100));
      }
    });
    g.addEventListener("focus", () => highlight(i));
    g.addEventListener("blur", () => highlight(-1));
    return g;
  }

  function pointRow(i) {
    const temp = number(T_MIN, T_MAX, 1);
    const level = number(0, 100, 1);
    const remove = button("Remove", "fan-small");
    temp.setAttribute("aria-label", "Point " + (i + 1) + " temperature in degrees Celsius");
    level.setAttribute("aria-label", "Point " + (i + 1) + " duty in per cent");
    remove.setAttribute("aria-label", "Remove point " + (i + 1));
    temp.onchange = () => {
      const curve = draft.curves[chosen];
      const at = { where: "points", key: chosen };
      const typed = Number(temp.value);
      commit(temp, T_MIN, T_MAX, "Temperature", at, (v) => setPoint(curve, i, v, curve.points[i][1]), () => trim(curve.points[i][0]));
      if (typed >= T_MIN && typed <= T_MAX && Math.abs(curve.points[i][0] - typed) > 0.05) {
        problem = { ...at, text: "Point " + (i + 1) + " stays between its neighbours, so it is at " + trim(curve.points[i][0]) + " °C." };
        render();
      }
    };
    level.onchange = () => {
      const curve = draft.curves[chosen];
      const at = { where: "points", key: chosen };
      commit(level, 0, 100, "Duty", at, (v) => setPoint(curve, i, curve.points[i][0], v / 100), () => Math.round(curve.points[i][1] * 100));
    };
    press(remove, () => removePoint(i, "table"));
    const row = el(
      "tr",
      {},
      el("th", { scope: "row", textContent: i + 1 }),
      el("td", {}, temp, el("span", { className: "fan-unit", textContent: "°C" })),
      el("td", {}, level, el("span", { className: "fan-unit", textContent: "%" })),
      el("td", {}, remove),
    );
    row.parts = { temp, level, remove };
    row.addEventListener("focusin", () => highlight(i));
    row.addEventListener("focusout", () => highlight(-1));
    return row;
  }

  function sensorChoice(id) {
    const box = el("input", { type: "checkbox", value: id });
    const name = el("span", { className: "name" });
    const value = el("span", { className: "fan-num" });
    box.onchange = () =>
      edit(() => {
        const curve = draft.curves[chosen];
        const others = curve.sensors.filter((s) => s !== id);
        curve.sensors = box.checked ? [...others, id] : others;
        // Back in the saved order when the same sensors are chosen again.
        const was = saved.curves.find((c) => c.name === curve.name);
        if (was && was.sensors.length === curve.sensors.length && was.sensors.every((s) => curve.sensors.includes(s))) {
          curve.sensors = [...was.sensors];
        }
      });
    const row = el("label", { className: "fan-check" }, box, name, value);
    row.parts = { box, name, value };
    return row;
  }

  function ruleCard(fan) {
    const n = ++serial;
    const curve = el("select", { id: "fan-rule-curve-" + n });
    const min = number(0, 100, 1);
    const max = number(0, 100, 1);
    const ramp = number(1, 100, 1);
    const name = el("span", { className: "name", id: "fan-rule-name-" + n });
    const tag = el("span", { className: "fan-tag" });
    const now = el("span", { className: "fan-num" });
    const wrong = el("p", { className: "problem", hidden: true });
    const warn = slot(min, max);
    const said = slot(curve, ramp);
    const rule = () => draft.rules.find((r) => r.fan === fan.id);
    const at = { where: "rule", key: fan.id };
    const field = (label, input, key, lo, what) => {
      input.onchange = () => {
        const r = rule();
        if (r) commit(input, lo, 100, what, at, (v) => (r[key] = Math.round(v * 10) / 1000), () => trim(r[key] * 100));
      };
      return el("label", {}, el("span", { textContent: label }), input);
    };
    curve.onchange = () => {
      edit(() => {
        const r = rule();
        if (r && curve.value) r.curve = curve.value;
        else if (r) draft.rules.splice(draft.rules.indexOf(r), 1);
        else if (curve.value) draft.rules.push({ fan: fan.id, curve: curve.value, min_duty: 0, max_duty: 1, ramp: 0.2 });
      });
      hold();
    };
    const body = el(
      "div",
      { className: "fan-card" },
      el("div", { className: "fan-card-head" }, name, tag, now),
      wrong,
      el("div", { className: "fields" }, el("label", { htmlFor: curve.id, textContent: "Curve" }), curve),
      el(
        "div",
        { className: "fan-trio" },
        field("Minimum (%)", min, "min_duty", 0, "Minimum duty"),
        field("Maximum (%)", max, "max_duty", 0, "Maximum duty"),
        field("Ramp (%/s)", ramp, "ramp", 1, "Ramp"),
      ),
      warn,
      said,
    );
    body.setAttribute("role", "group");
    body.setAttribute("aria-labelledby", name.id);
    ramp.setAttribute("aria-label", "Ramp, per cent per second");
    const card = el("li", { fan: fan.id }, body);
    card.parts = { curve, min, max, ramp, name, tag, now, wrong, warn, said };
    return card;
  }

  function sizeGraph() {
    const w = ui.graph.clientWidth;
    if (!w || w === plot.w) return false;
    plot.w = w;
    plot.h = clamp(Math.round(w * 0.8), 200, 320);
    plot.x1 = w - 14;
    plot.y1 = plot.h - 24;
    ui.svg.setAttribute("viewBox", "0 0 " + w + " " + plot.h);
    ui.svg.setAttribute("height", plot.h);
    const marks = [];
    for (let t = T_MIN; t <= T_MAX; t += 10) {
      const x = Math.round(px(t)) + 0.5;
      marks.push(svg("line", { x1: x, x2: x, y1: plot.y0, y2: plot.y1, class: t % 20 ? "minor" : "major" }));
      if (t % 20 === 0 && t < T_MAX) marks.push(svg("text", { x, y: plot.h - 8, "text-anchor": "middle" }, String(t)));
    }
    marks.push(svg("text", { x: w - 2, y: plot.h - 8, "text-anchor": "end" }, T_MAX + " °C"));
    for (let d = 0; d <= 100; d += 25) {
      const y = Math.round(py(d / 100)) + 0.5;
      marks.push(svg("line", { x1: plot.x0, x2: plot.x1, y1: y, y2: y, class: "major" }));
      marks.push(svg("text", { x: plot.x0 - 6, y: y + 4, "text-anchor": "end" }, d === 100 ? "100%" : String(d)));
    }
    ui.grid.replaceChildren(...marks);
    return true;
  }

  const px = (temp) => plot.x0 + ((clamp(temp, T_MIN, T_MAX) - T_MIN) / (T_MAX - T_MIN)) * (plot.x1 - plot.x0);
  const py = (duty) => plot.y1 - clamp(duty, 0, 1) * (plot.y1 - plot.y0);

  function drawGraph() {
    const curve = draft.curves[chosen];
    if (!curve || !plot.w || !curve.points.length) return;
    const points = curve.points;
    let d = "M" + plot.x0 + "," + py(points[0][1]);
    for (const [temp, duty] of points) d += "L" + px(temp) + "," + py(duty);
    d += "L" + plot.x1 + "," + py(points[points.length - 1][1]);
    ui.line.setAttribute("d", d);
    ui.hit.setAttribute("d", d);
    ui.area.setAttribute("d", d + "L" + plot.x1 + "," + plot.y1 + "L" + plot.x0 + "," + plot.y1 + "Z");

    if (ui.handles.childElementCount !== points.length) ui.handles.replaceChildren(...points.map((_, i) => handle(i)));
    points.forEach(([temp, duty], i) => {
      const g = ui.handles.children[i];
      g.setAttribute("transform", "translate(" + px(temp) + "," + py(duty) + ")");
      g.setAttribute("aria-label", "Point " + (i + 1));
      g.setAttribute("aria-valuenow", Math.round(duty * 100));
      g.setAttribute("aria-valuetext", trim(temp) + " °C, " + percent(duty));
    });

    // One lost sensor is enough for full speed, as in the service.
    const gone = lost(curve);
    const hot = gone.length ? null : hottest(curve);
    ui.nowLine.style.display = ui.nowMark.style.display = hot ? "" : "none";
    if (hot) {
      const x = Math.round(px(hot.value)) + 0.5;
      const asked = dutyAt(points, hot.value);
      for (const [name, value] of [["x1", x], ["x2", x], ["y1", plot.y0], ["y2", plot.y1]]) ui.nowLine.setAttribute(name, value);
      ui.nowMark.setAttribute("transform", "translate(" + x + "," + py(asked) + ")");
      setText(ui.now, hot.name + " is at " + reading(hot.value) + ", so the curve asks for " + percent(asked) + ".");
    } else {
      setText(ui.now, gone.length ? sentence(silent(gone)) + ", so fans on this curve run at full speed." : "No sensor chosen.");
    }
  }

  function renderService(running) {
    const state = live.service;
    ui.shape.className = MARKS[state] || "mark none";
    setText(ui.word, WORDS[pending] || WORDS[state] || "Checking");
    setText(ui.version, live.version ? "version " + live.version : "");
    let detail = "";
    if (state === "stopped") detail = "Fan control runs in the FoxRGB background service, which is not running.";
    else if (state === "error") detail = live.detail ? sentence(live.detail) : "The service did not answer as expected.";
    say(ui.detail, detail);
    ui.start.hidden = !state || running;
    ui.stop.hidden = !state || state === "stopped";
    able(ui.start, !pending);
    able(ui.stop, !pending);
    say(ui.serviceProblem, told("service"));

    ui.notice.hidden = !state || (running && !live.sim && live.fans.length > 0);
    ui.simRow.hidden = !state;
    ui.sim.disabled = !running;
    if (pending !== "sim") ui.sim.checked = live.sim;
    say(ui.simProblem, told("sim"));
    ui.readings.hidden = !running;
  }

  function renderReadings() {
    const sensors = live.sensors;
    if (ui.sensors.childElementCount !== sensors.length) ui.sensors.replaceChildren(...sensors.map(sensorRow));
    sensors.forEach((sensor, i) => {
      const { name, value } = ui.sensors.children[i].parts;
      setText(name, sensor.name);
      setText(value, reading(sensor.value));
      value.classList.toggle("none", sensor.value == null);
    });
    ui.noSensors.hidden = sensors.length > 0;

    const fans = live.fans;
    if (ui.fans.childElementCount !== fans.length) ui.fans.replaceChildren(...fans.map(fanRow));
    fans.forEach((fan, i) => {
      const { name, now, fill, bar, words } = ui.fans.children[i].parts;
      setText(name, fan.name);
      setText(now, output(fan));
      bar.hidden = fan.duty == null;
      if (fan.duty != null) fill.style.width = clamp(fan.duty, 0, 1) * 100 + "%";
      const wrong = trouble(fan);
      setText(words, wrong || (fan.curve ? "Follows " + fan.curve : "Not controlled"));
      words.className = "fan-words " + (wrong ? "problem" : "hint");
    });
    ui.noFans.hidden = fans.length > 0;
  }

  function renderCurves(open) {
    ui.curvesOff.hidden = open;
    ui.curvesBody.hidden = !open;
    if (!open) return;
    const curves = draft.curves;
    if (ui.curveList.childElementCount !== curves.length) ui.curveList.replaceChildren(...curves.map((_, i) => curvePick(i)));
    curves.forEach((curve, i) => {
      const { radio, name, tag } = ui.curveList.children[i].parts;
      radio.checked = i === chosen;
      setText(name, curve.name);
      name.title = curve.name;
      const was = saved.curves.find((c) => c.name === curve.name);
      setText(tag, !was ? "New" : JSON.stringify(was) !== JSON.stringify(curve) ? "Edited" : "");
    });
    ui.curveList.hidden = !curves.length;
    ui.noCurves.hidden = curves.length > 0;
    ui.curveActions.hidden = asking;
    ui.ask.hidden = !asking;
    able(ui.copy, curves.length > 0);
    able(ui.remove, curves.length > 0);

    const curve = curves[chosen];
    ui.editor.hidden = !curve;
    if (!curve) return;
    if (asking) {
      const users = draft.rules.filter((r) => r.curve === curve.name).map((r) => fanName(r.fan));
      const follow = users.length === 1 ? " follows it and" : " follow it and";
      setText(ui.askText, "Delete " + curve.name + "?" + (users.length ? " " + list.format(users) + follow + " will no longer be controlled." : ""));
    }
    if (document.activeElement !== ui.name) ui.name.value = curve.name;
    say(ui.nameProblem, told("name", chosen));

    const points = curve.points;
    if (ui.points.childElementCount !== points.length) ui.points.replaceChildren(...points.map((_, i) => pointRow(i)));
    points.forEach(([temp, duty], i) => {
      const parts = ui.points.children[i].parts;
      if (document.activeElement !== parts.temp) parts.temp.value = trim(temp);
      if (document.activeElement !== parts.level) parts.level.value = Math.round(duty * 100);
      able(parts.remove, points.length > 1);
    });
    able(ui.addPoint, roomFor(points) !== null);
    say(ui.pointsProblem, told("points", chosen));
    sizeGraph();
    drawGraph();

    const ids = [...live.sensors.map((s) => s.id), ...curve.sensors.filter((id) => !live.sensors.some((s) => s.id === id))];
    if (ui.choices.dataset.key !== ids.join("|")) {
      const focused = ui.choices.contains(document.activeElement) ? document.activeElement.value : null;
      ui.choices.dataset.key = ids.join("|");
      ui.choices.replaceChildren(...ids.map(sensorChoice));
      const again = [...ui.choices.querySelectorAll("input")].find((box) => box.value === focused);
      if (again) again.focus();
    }
    ids.forEach((id, i) => {
      const { box, name, value } = ui.choices.children[i].parts;
      const sensor = live.sensors.find((s) => s.id === id);
      box.checked = curve.sensors.includes(id);
      setText(name, sensor ? sensor.name : id);
      setText(value, sensor ? reading(sensor.value) : "not available");
      value.classList.toggle("none", !sensor || sensor.value == null);
    });
    ui.noChoices.hidden = ids.length > 0;
    say(ui.sensorsProblem, told("sensors", chosen) || (curve.sensors.length ? "" : "Choose at least one sensor. The service does not accept a curve without one."));
    if (document.activeElement !== ui.hyst) ui.hyst.value = curve.hysteresis;
    say(ui.hystProblem, told("hysteresis", chosen));
  }

  function renderRules(open) {
    ui.rulesOff.hidden = open;
    ui.rulesBody.hidden = !open;
    if (!open) return;
    const fans = ruleFans();
    const names = draft.curves.map((c) => c.name);
    const key = JSON.stringify([fans.map((f) => f.id), names]);
    if (ui.ruleList.dataset.key !== key) {
      if (typingIn(ui.rules)) return;
      ui.ruleList.dataset.key = key;
      ui.ruleList.replaceChildren(...fans.map(ruleCard));
      for (const card of ui.ruleList.children) {
        card.parts.curve.append(
          el("option", { value: "", textContent: "Not controlled" }),
          ...names.map((name) => el("option", { value: name, textContent: name })),
        );
      }
    }
    fans.forEach((fan, i) => {
      const parts = ui.ruleList.children[i].parts;
      const rule = draft.rules.find((r) => r.fan === fan.id);
      const was = saved.rules.find((r) => r.fan === fan.id);
      setText(parts.name, fan.name);
      setText(parts.tag, JSON.stringify(was || null) === JSON.stringify(rule || null) ? "" : "Edited");
      setText(parts.now, fan.live ? output(fan.live) : "not connected");
      say(parts.wrong, fan.live ? trouble(fan.live) : "");
      if (document.activeElement !== parts.curve) parts.curve.value = rule ? rule.curve : "";
      // A long curve name is cut short in the list, so it is also the tooltip.
      parts.curve.title = rule ? rule.curve : "";
      for (const [input, value] of [[parts.min, rule ? rule.min_duty : 0], [parts.max, rule ? rule.max_duty : 1], [parts.ramp, rule ? rule.ramp : 0.2]]) {
        input.disabled = !rule;
        if (document.activeElement !== input) input.value = trim(value * 100);
      }
      const above = Boolean(rule) && rule.min_duty > rule.max_duty;
      say(parts.warn, above ? "The minimum is above the maximum, which the service does not accept." : "");
      parts.min.setAttribute("aria-invalid", above);
      parts.max.setAttribute("aria-invalid", above);
      say(parts.said, told("rule", fan.id));
    });
    ui.noRules.hidden = fans.length > 0;
  }

  function renderBar(running, open) {
    const dirty = edited();
    // Kept while there is something to undo, even when there is nothing to edit.
    ui.bar.hidden = !open && !past.length;
    let text = note || "No unsaved changes";
    if (pending === "apply") text = "Applying";
    else if (problem && problem.where === "apply") text = problem.text;
    else if (problem && problem.rejected) text = "Not applied";
    else if (dirty && !running) text = "Unsaved changes. Start the service to apply them.";
    else if (dirty) text = "Unsaved changes";
    setText(ui.saveText, text);
    ui.save.classList.toggle("changed", dirty);
    ui.saveDot.hidden = !dirty;
    able(ui.apply, dirty && running && !pending);
    able(ui.revert, dirty && !pending);
    able(ui.undo, past.length > 0 && !pending);
  }

  function render() {
    queued = false;
    const running = live.service === "running";
    const open = running || edited();
    const mine = tidy(draft);
    ui.status.hidden = view !== "status";
    ui.curves.hidden = view !== "curves";
    ui.rules.hidden = view !== "rules";
    ui.marks.curves.hidden = JSON.stringify(mine.curves) === JSON.stringify(saved.curves);
    ui.marks.rules.hidden = JSON.stringify(mine.rules) === JSON.stringify(saved.rules);
    renderService(running);
    renderReadings();
    renderCurves(open);
    renderRules(open);
    renderBar(running, open);
    // A new message is scrolled into view once, clear of the bar that could cover it.
    if (problem !== revealed) {
      revealed = problem;
      const place = problem && [...root.querySelectorAll(".problem")].find((p) => p.textContent === problem.text);
      if (place) place.scrollIntoView({ block: "nearest" });
    }
  }

  function queue() {
    if (queued) return;
    queued = true;
    requestAnimationFrame(render);
  }

  buildViews();
  buildStatus();
  buildCurves();
  buildRules();
  buildBar();
  root.replaceChildren(ui.views, ui.status, ui.curves, ui.rules, ui.bar);

  // Ctrl+S and Ctrl+Z while this tab is shown. main.js sends them here, wherever the focus is.
  window.fanKeys = {
    apply(target) {
      // What is being typed goes in first. When it could not be taken as typed, nothing is sent.
      if (root.contains(target) && /^(text|number)$/.test(target.type)) {
        problem = null;
        target.dispatchEvent(new Event("change"));
        if (problem) return;
      }
      if (live.service !== "running") tell(edited() ? "Start the service to apply the fan changes" : "No fan changes to apply");
      else if (!edited()) tell("No fan changes to apply");
      apply();
    },
    undo() {
      if (!past.length) tell("No fan edit to undo");
      undo();
    },
  };
  // A config that arrived while a field had focus is taken up once the field is left.
  root.addEventListener("focusout", () =>
    setTimeout(() => {
      sync();
      render();
    }),
  );

  listen("fans", (e) => {
    take(e.payload);
    render();
  });
  refresh()
    .catch((e) => (live = { ...live, service: "error", detail: message(e) }))
    .then(render);
  render();
})();
