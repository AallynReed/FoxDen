const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = { snap: null };
const $ = (id) => document.getElementById(id);
let renderQueued = false;

function showError(text) {
  $("error-text").textContent = text;
  $("error").hidden = false;
}

// Resolves to undefined when the command failed; the error is already on screen.
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

function render() {
  renderQueued = false;
  const snap = state.snap;
  if (!snap) return;
  renderBar(snap);
  renderEffects(snap);
  renderDevices(snap);
  renderProfiles(snap);
  renderSettings(snap);
  if (typeof renderFeatures === "function") renderFeatures(snap);
}

function loadFeatures() {
  document.head.append(el("link", { rel: "stylesheet", href: "features.css" }));
  const script = el("script", { src: "features.js" });
  script.onload = () => {
    try {
      featuresInit();
    } catch (e) {
      console.error(e);
    }
  };
  document.head.append(script);
}

function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

function setSnapshot(snap) {
  state.snap = snap;
  lastUsed[snap.effect.kind] = snap.effect;
  previewSnapshot();
  scheduleRender();
}

function wireTabs() {
  const tabs = [...document.querySelectorAll("[role=tab]")];
  const show = (tab) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", on);
      t.tabIndex = on ? 0 : -1;
      $(t.getAttribute("aria-controls")).hidden = !on;
    }
    scheduleRender();
  };
  tabs.forEach((tab, i) => {
    tab.onclick = () => show(tab);
    tab.onkeydown = (e) => {
      const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      const next = tabs[(i + step + tabs.length) % tabs.length];
      show(next);
      next.focus();
    };
  });
}

async function init() {
  wireTabs();
  wirePanels();
  $("error-close").onclick = () => ($("error").hidden = true);
  previewInit($("canvas"));
  await listen("snapshot", (e) => setSnapshot(e.payload));
  await listen("frame", (e) => previewFrames(e.payload));
  const snap = await call("get_snapshot");
  if (snap) setSnapshot(snap);
  loadFeatures();
}

init().catch((e) => showError(String(e)));
