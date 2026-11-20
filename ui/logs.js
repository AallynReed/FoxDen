const RANK = { info: 0, warn: 1, error: 2 };
const SHORT = { info: "info", warn: "warn", error: "err" };
const $ = (id) => document.getElementById(id);
const tauri = window.__TAURI__;
let entries = [];
let least = "info";

function sample() {
  const now = Date.now();
  return [
    { at: now - 9000, level: "info", source: "app", text: "FoxDen 0.3.0 started" },
    { at: now - 8000, level: "info", source: "devices", text: "Connected Logitech G515 (hid)" },
    { at: now - 4000, level: "warn", source: "devices", text: "Unknown board: no driver yet" },
    { at: now - 1000, level: "error", source: "hid", text: "MSI Mystic Light: write failed" },
  ];
}

const call = (name) => (tauri ? tauri.core.invoke(name) : Promise.resolve(name === "get_logs" ? sample() : null));

function time(ms) {
  return new Date(ms).toLocaleTimeString([], { hour12: false });
}

function shown() {
  const text = $("filter").value.trim().toLowerCase();
  return entries.filter(
    (e) => RANK[e.level] >= RANK[least] && (!text || (e.source + " " + e.text).toLowerCase().includes(text)),
  );
}

function row(e) {
  const li = document.createElement("li");
  li.className = e.level;
  for (const [cls, text] of [["at", time(e.at)], ["lvl", SHORT[e.level]], ["src", e.source], ["msg", e.text]]) {
    const span = document.createElement("span");
    span.className = cls;
    span.textContent = text;
    li.append(span);
  }
  return li;
}

function render() {
  const list = $("log");
  const atEnd = list.scrollHeight - list.scrollTop - list.clientHeight < 24;
  const rows = shown();
  list.replaceChildren(...rows.map(row));
  $("empty").hidden = rows.length > 0;
  $("count").textContent = rows.length === entries.length ? `${entries.length} entries` : `${rows.length} of ${entries.length}`;
  if (atEnd) list.scrollTop = list.scrollHeight;
}

function plain(e) {
  return `${time(e.at)} ${SHORT[e.level]} ${e.source}: ${e.text}`;
}

async function init() {
  for (const b of document.querySelectorAll(".seg button")) {
    b.onclick = () => {
      least = b.dataset.level;
      for (const o of document.querySelectorAll(".seg button")) o.setAttribute("aria-checked", String(o === b));
      render();
    };
  }
  $("filter").oninput = render;
  $("copy").onclick = async () => {
    await navigator.clipboard.writeText(shown().map(plain).join("\n"));
    $("count").textContent = "Copied";
  };
  $("clear").onclick = async () => {
    await call("clear_logs");
    entries = [];
    render();
  };
  entries = (await call("get_logs")) || [];
  render();
  $("log").scrollTop = $("log").scrollHeight;
  if (tauri) {
    await tauri.event.listen("log", (e) => {
      entries.push(e.payload);
      if (entries.length > 1000) entries.shift();
      render();
    });
  }
}

init();
