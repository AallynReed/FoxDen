const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = { snap: null };
// Undo history of the effect, brightness, devices and layout, read from the snapshots.
const hist = { steps: [], at: -1, quiet: 0, sent: null, stamp: 0 };
const $ = (id) => document.getElementById(id);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const typing = (t) => t.isContentEditable || t.tagName === "TEXTAREA" || (t.tagName === "INPUT" && /^(text|number)$/.test(t.type));
// With Ctrl: steps of zoom, 0 for back to 100%.
const ZOOM_KEYS = { "=": 1, "+": 1, "-": -1, 0: 0 };
let renderQueued = false;
let noteTimer = 0;
let frames = 0;
let known = null;
let listed = false;
let lastProfile = null;
let settle = null;

function showError(text) {
  setState($("error-text"), "warn", sentence(text));
  $("error").hidden = false;
}

// A short line in the status bar saying what just happened.
function note(text) {
  const line = $("st-note");
  line.textContent = line.title = text;
  clearTimeout(noteTimer);
  if (text) noteTimer = setTimeout(note, 8000, "");
}

// Resolves to undefined when the command failed; the error is already on screen.
// Windows keeps keys typed into this window from the key hook, so the key effects get them from here.
for (const type of ["keydown", "keyup"]) {
  document.addEventListener(
    type,
    (e) => {
      if (e.repeat || !e.keyCode || !(state.snap && state.snap.post.kind.startsWith("press_"))) return;
      invoke("key_event", { vk: e.keyCode, down: type === "keydown" }).catch(() => {});
    },
    true,
  );
}

async function call(name, args) {
  try {
    const out = await invoke(name, args);
    return out === undefined ? null : out;
  } catch (e) {
    showError(typeof e === "string" ? e : (e && e.message) || String(e));
    scheduleRender();
    return undefined;
  }
}

function renderStatus(snap) {
  setText($("app-version"), snap.version);
  const leds = snap.devices.reduce((n, d) => n + d.leds.length, 0);
  const broken = snap.backends.filter((b) => b.error);
  const profile = snap.active_profile || (lastProfile && lastProfile + ", edited");
  if (!snap.settings.lighting_module) setState($("st-state"), "off", "Lighting off");
  else setState($("st-state"), snap.paused ? "off" : "on", snap.paused ? "Paused" : "Running");
  setText($("st-count"), plural(snap.devices.length, "device") + ", " + plural(leds, "LED"));
  setText($("st-profile"), profile ? "Profile " + profile : "No profile");
  $("st-profile").title = profile || "";
  $("st-problem").hidden = !broken.length;
  if (broken.length === 1) setState($("st-problem"), "warn", (BACKENDS[broken[0].name] || broken[0].name) + " error");
  else if (broken.length) setState($("st-problem"), "warn", broken.length + " backend errors");
}

function render() {
  renderQueued = false;
  const snap = state.snap;
  if (!snap) return;
  renderBar(snap);
  renderEffects(snap);
  renderDevices(snap);
  renderDetected(snap);
  renderProfiles(snap);
  renderSettings(snap);
  renderStatus(snap);
  renderModules(snap);
}

const LIGHTING_TABS = ["tab-effects", "tab-devices", "tab-profiles"];

// Hides the tabs of a module that is off, and leaves a hidden tab for the first one shown.
function renderModules(snap) {
  const off = { "tab-fans": !snap.settings.fans_module };
  for (const id of LIGHTING_TABS) off[id] = !snap.settings.lighting_module;
  for (const tab of document.querySelectorAll("[role=tab]")) tab.hidden = !!off[tab.id];
  const current = document.querySelector("[role=tab][aria-selected=true]");
  if (current && current.hidden) showTab(document.querySelector("[role=tab]:not([hidden])"));
}

function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

// Says which devices came or went since the last snapshot.
function noteChanges(snap) {
  const detected = snap.detected || [];
  const now = new Map([...detected, ...snap.devices].map((d) => [d.id, d.name]));
  const names = (from, to) => [...new Set([...from].filter(([id]) => !to.has(id)).map(([, name]) => name))];
  const list = (found) => (found.length > 3 ? plural(found.length, "device") : found.join(", "));
  // The first detection pass lists all it finds at once, which is not news.
  if (known && !listed) for (const d of detected) known.set(d.id, d.name);
  listed = listed || detected.length > 0;
  if (known) {
    const added = names(now, known);
    const gone = names(known, now);
    if (added.length) note("Added: " + list(added));
    else if (gone.length) note("Removed: " + list(gone));
  }
  known = now;
}

function undoable(snap) {
  const per = (pick) => Object.fromEntries(snap.devices.map((d) => [d.id, pick(d)]));
  return { effect: snap.effect, post: snap.post, brightness: snap.brightness, rects: per((d) => d.rect), levels: per((d) => [d.enabled, d.brightness]) };
}

// What changed between two states: a label for the user, and a key that tells one kind of edit from another.
function difference(a, b) {
  const fields = Object.keys({ ...a.effect, ...b.effect }).filter((k) => !same(a.effect[k], b.effect[k]));
  const changed = (key) => Object.keys(b[key]).filter((id) => a[key][id] && !same(a[key][id], b[key][id]));
  const [moved, tuned] = [changed("rects"), changed("levels")];
  const parts = [];
  if (fields.length) parts.push(["effect", fields.includes("kind") ? "kind" : fields]);
  if (!same(a.post, b.post)) parts.push(["post effect", ""]);
  if (a.brightness !== b.brightness) parts.push(["brightness", ""]);
  if (tuned.length) parts.push(["device", tuned]);
  if (moved.length) parts.push(["layout", moved]);
  return parts.length ? { label: parts.map((p) => p[0]).join(" and "), key: String(parts) } : null;
}

function renderHistory() {
  const show = (id, verb, change) => {
    $(id).setAttribute("aria-disabled", !change);
    $(id).title = change ? verb + " " + change.label + " change" : "Nothing to " + verb.toLowerCase();
  };
  show("undo", "Undo", hist.at > 0 && hist.steps[hist.at]);
  show("redo", "Redo", hist.steps[hist.at + 1]);
}

function track(snap) {
  if (hist.quiet) return;
  const now = undoable(snap);
  const last = hist.steps[hist.at];
  const change = last && difference(last, now);
  const time = performance.now();
  const tip = hist.at > 0 && hist.at === hist.steps.length - 1;
  // One edit lasts while the pointer stays down, or while its changes follow each other closely.
  const since = held ? pressed : time - 600;
  if (!change) {
    // Nothing to undo, but a device that has just appeared joins the current step.
    hist.at = Math.max(hist.at, 0);
    hist.steps[hist.at] = { ...last, ...now, rects: { ...(last && last.rects), ...now.rects }, levels: { ...(last && last.levels), ...now.levels } };
  } else if (tip && last.key === change.key && hist.stamp > since) {
    // Still the same edit: a slider being dragged, a key held down.
    hist.steps[hist.at] = { ...last, ...now };
    if (!difference(hist.steps[hist.at - 1], now)) hist.steps.length = hist.at--;
  } else {
    hist.steps.length = hist.at + 1;
    hist.steps.push({ ...now, ...change });
    if (hist.steps.length > 100) hist.steps.shift();
    hist.at = hist.steps.length - 1;
  }
  if (change) hist.stamp = time;
  renderHistory();
}

// Runs several commands as one step: the history looks again only once they have settled.
async function quietly(run) {
  hist.quiet++;
  try {
    await run();
  } finally {
    setTimeout(() => {
      if (--hist.quiet || !state.snap) return;
      hist.sent = null;
      followProfile(state.snap);
      track(state.snap);
      scheduleRender();
    }, 150);
  }
}

// Undo with -1, redo with 1.
function step(by) {
  const to = hist.steps[hist.at + by];
  if (!to || preview.drag) return;
  const from = hist.sent || undoable(state.snap);
  const { label } = hist.steps[by < 0 ? hist.at : hist.at + 1];
  hist.at += by;
  hist.sent = to;
  renderHistory();
  note((by < 0 ? "Undid " : "Redid ") + label + " change");
  quietly(async () => {
    if (!same(from.effect, to.effect)) await call("set_effect", { effect: to.effect });
    if (!same(from.post, to.post)) await call("set_post", { post: to.post });
    if (from.brightness !== to.brightness) await call("set_brightness", { value: to.brightness });
    for (const [id, rect] of Object.entries(to.rects)) {
      if (!findDevice(id) || !from.rects[id] || (same(from.rects[id], rect) && same(from.levels[id], to.levels[id]))) continue;
      const [enabled, brightness] = to.levels[id];
      await call("set_device", { id, enabled, brightness, rect: rect.map(tidy) });
    }
  });
}

// The profile in use: the active one, or the one that was active before it was edited.
function followProfile(snap) {
  if (snap.active_profile) lastProfile = snap.active_profile;
  else if (!snap.profiles.includes(lastProfile)) lastProfile = null;
}

// Resolves true at the next snapshot, or false when none comes within `ms`.
function nextSnapshot(ms) {
  return new Promise((done) => {
    settle = done;
    setTimeout(done, ms, false);
  });
}

function setSnapshot(snap) {
  state.snap = snap;
  lastUsed[snap.effect.kind] = snap.effect;
  // Profiles loaded on the way through a rename or a duplicate are not the one in use.
  if (!hist.quiet) followProfile(snap);
  previewSnapshot();
  track(snap);
  noteChanges(snap);
  scheduleRender();
  if (settle) settle(true);
  settle = null;
}

function showTab(tab) {
  if (!tab || tab.hidden) return;
  for (const t of document.querySelectorAll("[role=tab]")) {
    const on = t === tab;
    t.setAttribute("aria-selected", on);
    t.tabIndex = on ? 0 : -1;
    $(t.getAttribute("aria-controls")).hidden = !on;
  }
  // Only the lighting tabs need the canvas.
  document.body.classList.toggle("wide", !LIGHTING_TABS.includes(tab.id));
  if (tab.id === "tab-settings") refreshConflicts();
  // In the Fans tab Ctrl+Z and Ctrl+Y belong to the fan draft; these buttons keep to the lighting.
  for (const [id, keys] of [["undo", "Control+Z"], ["redo", "Control+Y"]]) {
    if (tab.id === "tab-fans") $(id).removeAttribute("aria-keyshortcuts");
    else $(id).setAttribute("aria-keyshortcuts", keys);
  }
  scheduleRender();
}

function wireTabs() {
  for (const tab of document.querySelectorAll("[role=tab]")) {
    tab.onclick = () => showTab(tab);
    tab.onkeydown = (e) => {
      const tabs = [...document.querySelectorAll("[role=tab]:not([hidden])")];
      const i = tabs.indexOf(tab);
      const to = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
      if (to === undefined) return;
      e.preventDefault();
      const next = tabs[(to + tabs.length) % tabs.length];
      showTab(next);
      next.focus();
    };
  }
}

function openHelp() {
  if (!$("help-sheet").open) $("help-sheet").showModal();
}

// Ctrl+S saves over the profile in use, or asks for a name when there is none.
function saveShortcut() {
  const name = lastProfile;
  if (name) return call("save_profile", { name }).then((out) => out !== undefined && note("Saved profile " + name));
  showTab($("tab-profiles"));
  $("profile-name").focus();
  note("Name the profile, then press Enter");
}

function selectNext(by) {
  const devices = state.snap.devices;
  const i = devices.findIndex((d) => d.id === preview.selected);
  const next = devices[i < 0 ? (by > 0 ? 0 : devices.length - 1) : (i + by + devices.length) % devices.length];
  if (!next) return;
  selectDevice(next.id);
  note("Selected " + next.name);
}

function onShortcut(e) {
  if (e.defaultPrevented || !state.snap || e.altKey) return;
  const t = e.target;
  const key = e.key.toLowerCase();
  // The zoom keys also work over a sheet, which may be what needs to be larger.
  if ((e.ctrlKey || e.metaKey) && ZOOM_KEYS[key] !== undefined) {
    e.preventDefault();
    return zoomBy(ZOOM_KEYS[key]);
  }
  if (document.querySelector("dialog[open]")) return;
  const loose = t === document.body || t === document.documentElement;
  if (ARROWS[e.key]) {
    if (loose) onKeyDown(e);
  } else if (e.ctrlKey || e.metaKey) {
    // While the Fans tab is shown these keys act on its draft, wherever the focus is.
    const fans = !$("panel-fans").hidden && window.fanKeys;
    if ((key === "z" || key === "y") && !typing(t)) {
      e.preventDefault();
      if (!fans) step(key === "y" || e.shiftKey ? 1 : -1);
      else if (key === "z" && !e.shiftKey) fans.undo();
    } else if (key === "s") {
      e.preventDefault();
      if (fans) fans.apply(t);
      else saveShortcut();
    } else if (key === "f") {
      e.preventDefault();
      showTab($("tab-effects"));
      $("effect-search").focus();
      $("effect-search").select();
    }
  } else if (typing(t) || t.tagName === "SELECT") {
    return;
  } else if (key === "escape") {
    selectDevice(null);
  } else if (!view.keys) {
    return;
  } else if (key === "?") {
    e.preventDefault();
    openHelp();
  } else if (key === "[" || key === "]") {
    selectNext(key === "]" ? 1 : -1);
  } else if (key === "g") {
    setView({ snap: !view.snap });
    note(view.snap ? "Snap to grid on" : "Snap to grid off");
  } else if (key === "l") {
    setView({ labels: !view.labels });
    note(view.labels ? "Labels shown" : "Labels hidden");
  } else if (key === " " && (loose || t === preview.canvas)) {
    e.preventDefault();
    $("pause").click();
  }
}

async function init() {
  wireTabs();
  wirePanels();
  $("error-close").onclick = () => ($("error").hidden = true);
  $("help-open").onclick = openHelp;
  $("logs-open").onclick = $("logs-open-settings").onclick = () => call("open_logs");
  $("discord-open").onclick = () => call("open_link", { which: "discord" });
  $("support-open").onclick = () => $("support-sheet").showModal();
  $("support-kofi").onclick = () => call("open_link", { which: "kofi" });
  $("support-paypal").onclick = () => call("open_link", { which: "paypal" });
  $("support-close").onclick = () => $("support-sheet").close();
  $("st-problem").onclick = () => showSetting("backend-list");
  document.addEventListener("keydown", onShortcut);
  previewInit($("canvas"));
  await listen("snapshot", (e) => setSnapshot(e.payload));
  await listen("modules", (e) => showModules(e.payload));
  await listen("conflicts", (e) => {
    if (e.payload.text) note(e.payload.text);
    showClosed(e.payload);
    refreshConflicts();
  });
  showClosed(await call("last_conflicts"));
  await wireUpdates();
  await listen("frame", (e) => {
    frames++;
    previewFrames(e.payload);
  });
  setInterval(() => {
    setText($("st-fps"), frames + " fps");
    frames = 0;
  }, 1000);
  const snap = await call("get_snapshot");
  // A snapshot event that got here first is the newer of the two.
  if (snap && !state.snap) setSnapshot(snap);
}

init().catch((e) => showError(String(e)));
