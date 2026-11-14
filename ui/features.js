const FEAT = { ready: false, frames: 0, fps: 0, known: null, note: "", auto: 0, scanning: false };
const AUTO_KEY = "foxrgb.autoDetect";
const AUTO_MS = 10000;

const typing = (t) => Boolean(t) && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
const pct = (v) => Math.round(v * 1000) / 10;

function featNote(text) {
  FEAT.note = text;
  if (FEAT.ready) setText($("st-note"), text);
}

function currentInfo() {
  const snap = state.snap;
  return snap ? snap.catalog.find((c) => c.default.kind === snap.effect.kind) : undefined;
}

function applyInspector() {
  const d = findDevice(preview.selected);
  if (!d) return;
  const v = ["x", "y", "w", "h"].map((k) => Number($("insp-" + k).value) / 100);
  if (v.some((n) => !Number.isFinite(n))) return featNote("Position and size must be numbers from 0 to 100.");
  const w = clamp(v[2], MIN_SIZE, 1);
  const h = clamp(v[3], MIN_SIZE, 1);
  sendRect(d, [clamp(v[0], 0, 1 - w), clamp(v[1], 0, 1 - h), w, h]);
}

// Only reset_layout knows the defaults, so reset everything and put the other devices back.
async function resetPosition(id) {
  if (!state.snap || !findDevice(id)) return;
  const others = state.snap.devices.filter((d) => d.id !== id).map((d) => ({ device: d, rect: d.rect }));
  if ((await call("reset_layout")) === undefined) return;
  for (const o of others) await sendRect(o.device, o.rect);
  featNote("Position reset");
}

function buildInspector() {
  const num = (key, label) => {
    const input = el("input", { type: "number", min: 0, max: 100, step: 0.5, id: "insp-" + key });
    input.onchange = applyInspector;
    return el("label", { htmlFor: input.id, textContent: label }, input);
  };
  const on = el("input", { type: "checkbox", className: "switch", id: "insp-on" });
  const level = el("input", { type: "range", min: 0, max: 1, step: 0.01, id: "insp-level" });
  const reset = el("button", { type: "button", id: "insp-reset", textContent: "Reset position" });
  const send = throttled(() => {
    const d = findDevice(preview.selected);
    if (d) call("set_device", { id: d.id, enabled: on.checked, brightness: Number(level.value), rect: d.rect });
  });
  on.onchange = send;
  level.oninput = send;
  reset.onclick = () => resetPosition(preview.selected);
  const box = el(
    "div",
    { className: "inspector", id: "inspector", hidden: true },
    el("h2", { id: "insp-name", textContent: "Selected device" }),
    el("p", { className: "hint wide", id: "insp-meta" }),
    num("x", "X (%)"),
    num("y", "Y (%)"),
    num("w", "Width (%)"),
    num("h", "Height (%)"),
    el("label", { htmlFor: "insp-on", textContent: "Enabled" }, on),
    el("label", { htmlFor: "insp-level", textContent: "Brightness" }, level),
    el("div", { className: "wide" }, reset),
  );
  const none = el("p", { className: "hint", id: "insp-none", textContent: "Select a device to edit its position and size. [ and ] step through the devices." });
  $("device-list").before(none, box);
}

function filterEffects() {
  const q = $("effect-search").value.trim().toLowerCase();
  let shown = 0;
  for (const pick of $("effect-list").children) {
    const hit = pick.textContent.toLowerCase().includes(q);
    pick.hidden = !hit;
    if (hit) shown++;
  }
  $("effect-none").hidden = shown > 0;
}

function randomise() {
  const info = currentInfo();
  if (!info) return;
  const effect = { ...state.snap.effect };
  for (const p of info.params) {
    const c = p.control;
    if (c.type === "range") {
      const steps = Math.round((c.max - c.min) / c.step);
      effect[p.name] = Number((c.min + Math.round(Math.random() * steps) * c.step).toFixed(4));
    } else if (c.type === "color") effect[p.name] = [0, 1, 2].map(() => Math.floor(Math.random() * 256));
    else if (c.type === "toggle") effect[p.name] = Math.random() < 0.5;
    else if (c.options && c.options.length) effect[p.name] = c.options[Math.floor(Math.random() * c.options.length)];
  }
  call("set_effect", { effect });
}

function buildEffectTools() {
  const search = el("input", { type: "text", id: "effect-search", className: "effect-search", placeholder: "Search effects", autocomplete: "off", spellcheck: false });
  search.setAttribute("aria-label", "Search effects");
  search.oninput = filterEffects;
  search.onkeydown = (e) => {
    if (e.key !== "Escape" || !search.value) return;
    search.value = "";
    filterEffects();
  };
  $("effect-list").before(search);
  $("effect-list").after(el("p", { className: "hint", id: "effect-none", hidden: true, textContent: "No effects match. Clear the search to see them all." }));

  const reset = el("button", { type: "button", id: "effect-reset", textContent: "Reset to defaults" });
  const random = el("button", { type: "button", id: "effect-random", textContent: "Randomise" });
  reset.onclick = () => {
    const info = currentInfo();
    if (info) call("set_effect", { effect: info.default });
  };
  random.onclick = randomise;
  $("params-empty").after(el("div", { className: "actions" }, reset, random));
}

function buildHelp() {
  const rows = [
    ["Space", "Pause or resume"],
    ["Ctrl+S", "Save profile"],
    ["Ctrl+F", "Search effects"],
    ["[ and ]", "Select the previous or next device"],
    ["Arrow keys", "Move the selected device"],
    ["Shift+Arrow keys", "Resize the selected device"],
    ["Esc", "Deselect"],
    ["?", "Show this list"],
  ];
  const sheet = el(
    "dialog",
    { className: "sheet", id: "help-sheet" },
    el("h2", { id: "help-title", textContent: "Keyboard shortcuts" }),
    el("dl", {}, ...rows.flatMap(([k, v]) => [el("dt", { textContent: k }), el("dd", { textContent: v })])),
    el("form", { method: "dialog", className: "actions" }, el("button", { type: "submit", textContent: "Close" })),
  );
  sheet.setAttribute("aria-labelledby", "help-title");
  document.body.append(sheet);
}

function openHelp() {
  const sheet = $("help-sheet");
  if (!sheet.open) sheet.showModal();
}

function buildStatus() {
  const help = el("button", { type: "button", id: "help-open", textContent: "Shortcuts" });
  help.onclick = openHelp;
  const note = el("span", { className: "grow", id: "st-note" });
  note.setAttribute("role", "status");
  const line = el(
    "div",
    { className: "statusline", id: "statusline" },
    el("span", { id: "st-backends" }),
    el("span", { id: "st-count" }),
    el("span", { id: "st-state" }),
    el("span", { id: "st-fps" }),
    note,
    help,
  );
  document.querySelector(".stage").append(line);
}

function renderStatus(snap) {
  const off = snap.backends.filter((b) => !b.active && !b.error).length;
  const parts = snap.backends
    .filter((b) => b.active || b.error)
    .map((b) => (b.error ? "▲ " : "● ") + (BACKENDS[b.name] || b.name) + (b.error ? " error" : " on"));
  if (off) parts.push("○ " + off + " off");
  setText($("st-backends"), parts.join("   "));
  const leds = snap.devices.reduce((n, d) => n + d.leds.length, 0);
  setText($("st-count"), plural(snap.devices.length, "device") + ", " + plural(leds, "LED"));
  setText($("st-state"), (snap.paused ? "Paused" : "Running") + " · " + (snap.active_profile ? "Profile " + snap.active_profile : "No profile"));
  setText($("st-fps"), FEAT.fps + " fps");
  setText($("st-note"), FEAT.note);
  $("st-note").title = FEAT.note;
}

function trackDevices(snap) {
  const now = new Map(snap.devices.map((d) => [d.id, d.name]));
  if (FEAT.known) {
    const added = [...now.keys()].filter((id) => !FEAT.known.has(id)).map((id) => now.get(id));
    const gone = [...FEAT.known.keys()].filter((id) => !now.has(id)).map((id) => FEAT.known.get(id));
    const list = (names) => (names.length > 3 ? plural(names.length, "device") : names.join(", "));
    if (added.length) FEAT.note = "Found " + list(added);
    else if (gone.length) FEAT.note = "Disconnected: " + list(gone);
  }
  FEAT.known = now;
}

function autoScan() {
  if (document.hidden || preview.drag || FEAT.scanning) return;
  FEAT.scanning = true;
  invoke("rescan")
    .catch((e) => {
      const box = $("set-auto");
      box.checked = false;
      box.onchange();
      showError("Automatic detection stopped: " + (typeof e === "string" ? e : String(e)) + ". Check the backends under Settings, then switch it on again.");
    })
    .finally(() => (FEAT.scanning = false));
}

function buildAutoDetect() {
  const box = el("input", { type: "checkbox", className: "switch", id: "set-auto" });
  try {
    box.checked = localStorage.getItem(AUTO_KEY) === "1";
  } catch {}
  box.onchange = () => {
    clearInterval(FEAT.auto);
    FEAT.auto = box.checked ? setInterval(autoScan, AUTO_MS) : 0;
    try {
      localStorage.setItem(AUTO_KEY, box.checked ? "1" : "0");
    } catch {}
    featNote(box.checked ? "Automatic detection on" : "Automatic detection off");
  };
  $("rescan").parentNode.after(
    el("div", { className: "field" }, el("label", { htmlFor: "set-auto", textContent: "Detect devices automatically" }), box),
    el("p", { className: "hint", textContent: "Rescans every 10 seconds and reports devices that appear or disappear." }),
  );
  if (box.checked) FEAT.auto = setInterval(autoScan, AUTO_MS);
}

function saveShortcut() {
  const name = state.snap.active_profile;
  if (name) {
    call("save_profile", { name }).then((out) => out !== undefined && featNote("Saved profile " + name));
    return;
  }
  $("tab-profiles").click();
  $("profile-name").focus();
  featNote("Name the profile, then press Enter to save it.");
}

function onShortcut(e) {
  if (e.defaultPrevented || !state.snap) return;
  const t = e.target;
  if ((e.ctrlKey || e.metaKey) && !e.altKey) {
    const k = e.key.toLowerCase();
    if (k === "s") {
      e.preventDefault();
      saveShortcut();
    } else if (k === "f") {
      e.preventDefault();
      $("tab-effects").click();
      $("effect-search").focus();
      $("effect-search").select();
    }
    return;
  }
  if (e.altKey || typing(t) || $("help-sheet").open) return;
  const devices = state.snap.devices;
  if (e.key === "?") {
    e.preventDefault();
    openHelp();
  } else if (e.key === "Escape" && preview.selected) {
    selectDevice(null);
  } else if ((e.key === "[" || e.key === "]") && devices.length) {
    const n = devices.length;
    const step = e.key === "]" ? 1 : -1;
    const i = devices.findIndex((d) => d.id === preview.selected);
    const next = devices[i < 0 ? (step > 0 ? 0 : n - 1) : (i + step + n) % n];
    selectDevice(next.id);
    featNote("Selected " + next.name);
  } else if (e.key === " " && (t === document.body || t === document.documentElement || t === preview.canvas)) {
    e.preventDefault();
    $("pause").click();
  } else if (e.key.startsWith("Arrow") && t === document.body) {
    onKeyDown(e);
  }
}

function renderFeatures(snap) {
  if (!FEAT.ready) return;
  const d = findDevice(preview.selected);
  $("inspector").hidden = !d;
  $("insp-none").hidden = Boolean(d) || snap.devices.length === 0;
  if (d) {
    setText($("insp-name"), d.name);
    setText($("insp-meta"), (BACKENDS[d.backend] || d.backend) + " · " + plural(d.leds.length, "LED"));
    const r = rectOf(d);
    ["x", "y", "w", "h"].forEach((k, i) => put($("insp-" + k), pct(r[i])));
    put($("insp-on"), d.enabled);
    put($("insp-level"), d.brightness);
  }
  filterEffects();
  trackDevices(snap);
  renderStatus(snap);
}

function featuresInit() {
  buildInspector();
  buildEffectTools();
  buildHelp();
  buildStatus();
  buildAutoDetect();
  document.addEventListener("keydown", onShortcut);
  listen("frame", () => FEAT.frames++);
  setInterval(() => {
    FEAT.fps = FEAT.frames;
    FEAT.frames = 0;
    setText($("st-fps"), FEAT.fps + " fps");
  }, 1000);
  FEAT.ready = true;
  scheduleRender();
}
