// SteelSeries devices from GG's Engine on the Devices tab: firmware, battery and updates.
(function () {
  const box = document.getElementById("gg-box");
  if (!box || !window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  const sheet = $("gg-fw-sheet");
  let view = { on: false, online: false, error: null, devices: [] };
  let on = false;
  let asked = null;

  function row(d) {
    const tag = el("span", { className: "state" });
    if (d.updating) setState(tag, "warn", "Updating");
    else if (d.update) setState(tag, "on", "Update available");
    else setState(tag, "off", d.firmware ? "Up to date" : "Firmware not read");
    const battery = d.battery == null ? "" : "battery " + d.battery + "%" + (d.charging ? ", charging" : "");
    const about = [d.kind, d.usb, d.firmware && "firmware " + d.firmware, battery].filter(Boolean);
    const out = el(
      "li",
      { className: "found" },
      el("div", { className: "split" }, el("span", { className: "name", textContent: d.name, title: d.name }), tag),
      el("span", { className: "meta", textContent: about.join(" · ") }),
    );
    if (d.update && !d.updating) {
      const go = el("button", { type: "button", textContent: "Update firmware" });
      go.onclick = () => confirm(d);
      out.append(go);
    }
    return out;
  }

  function render() {
    box.hidden = !on;
    if (!on) return;
    const status = $("gg-status");
    if (!view.on) setText(status, "Connecting to SteelSeries GG");
    else if (!view.online) setText(status, (view.error || "SteelSeries GG is not running") + ". Start GG to see firmware and battery levels.");
    else if (!view.devices.length) setText(status, "Engine lists no connected devices.");
    else setText(status, "Engine owns these devices' firmware. Updates run only after you confirm.");
    const list = $("gg-list");
    const key = JSON.stringify(view.devices);
    if (list.dataset.key !== key) {
      list.dataset.key = key;
      list.replaceChildren(...view.devices.map(row));
    }
  }

  function confirm(d) {
    asked = d;
    const low = d.battery != null && d.battery <= 15;
    setText($("gg-fw-text"), "SteelSeries Engine downloads and installs official firmware on " + d.name + (d.firmware ? ", now on " + d.firmware : "") + ".");
    const status = $("gg-fw-status");
    status.hidden = !low;
    setText(status, low ? "Battery at " + d.battery + "%. Charge it above 15% first." : "");
    $("gg-fw-go").disabled = low;
    sheet.showModal();
  }

  $("gg-fw-cancel").onclick = () => sheet.close();
  $("gg-fw-go").onclick = async () => {
    const go = $("gg-fw-go");
    const status = $("gg-fw-status");
    if (!asked || go.disabled) return;
    go.disabled = true;
    status.hidden = false;
    setText(status, "Asking Engine");
    try {
      const text = await invoke("gg_firmware_update", { id: asked.id });
      sheet.close();
      if (window.note) window.note(text);
      invoke("gg_refresh").catch(() => {});
    } catch (e) {
      setText(status, sentence(String(e)));
      go.disabled = false;
    }
  };
  $("gg-refresh").onclick = () => invoke("gg_refresh").catch(() => {});

  listen("gg-devices", (e) => {
    view = e.payload;
    render();
  });
  listen("snapshot", (e) => {
    const now = !!e.payload.settings.steelseries_gg;
    if (now === on) return;
    on = now;
    render();
    if (on) invoke("gg_refresh").catch(() => {});
  });
  invoke("get_snapshot").then((snap) => {
    on = !!snap.settings.steelseries_gg;
    return invoke("gg_devices");
  }).then((v) => {
    view = v;
    render();
  }).catch(() => {});
})();
