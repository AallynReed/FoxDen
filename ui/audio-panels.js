// Apps and Devices panels on the Audio tab: Windows' own volume, mute and level per app and device.
(function () {
  if (!window.__TAURI__) return;
  const { invoke } = window.__TAURI__.core;
  const root = document.getElementById("panel-audio");
  const POLL_MS = 250;
  const HOLD_MS = 1500;
  const LABELS = { game: "Game", chatRender: "Chat", media: "Media", aux: "Aux" };

  const SPEAKER = "M2 6h3l4-3v10l-4-3H2z";
  const MIC = "M6.5 2h3v7h-3zM3.5 8a4.5 4.5 0 0 0 9 0M8 12.5V15M5.5 15h5";
  // Sound waves when on, a cross or slash when muted.
  const MUTE_ICONS = {
    out: [icon(SPEAKER + "M11.5 5.5a3.5 3.5 0 0 1 0 5M13.5 3.5a6.5 6.5 0 0 1 0 9", "on"), SPEAKER + "M11 6l4 4M15 6l-4 4"],
    in: [icon(MIC, "on"), MIC + "M2 2l12 12"],
  };
  const STAR = "M8 1.8l1.9 3.9 4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6z";

  const shortName = (exe) => exe.replace(/\.exe$/i, "");
  const touched = new Map();
  const held = (key) => Date.now() - (touched.get(key) || 0) < HOLD_MS;
  const timers = new Map();

  // At most one call per key in flight; the newest value wins.
  function later(key, fn) {
    touched.set(key, Date.now());
    const t = timers.get(key);
    if (t) return void (t.fn = fn);
    const entry = { fn: null };
    timers.set(key, entry);
    fn().finally(function next() {
      if (entry.fn) {
        const f = entry.fn;
        entry.fn = null;
        f().finally(next);
      } else timers.delete(key);
    });
  }

  function build(getMix, getPriority) {
    const st = { view: null, rows: new Map(), keys: "", devKeys: "" };

    const panel = (id, title) => {
      const d = el("details", { className: "aud-more aud-vpanel", id });
      try {
        d.open = localStorage.getItem("fox-" + id) === "1";
      } catch {}
      d.addEventListener("toggle", () => {
        try {
          localStorage.setItem("fox-" + id, d.open ? "1" : "0");
        } catch {}
        if (d.open) poll();
      });
      const count = el("span", { className: "aud-tag" });
      d.append(el("summary", {}, title, count));
      d.count = count;
      return d;
    };

    const apps = panel("aud-apps", "Apps");
    const appList = el("ul", { className: "aud-vlist" });
    appList.setAttribute("aria-label", "Apps playing sound");
    const appEmpty = el("p", { className: "aud-empty aud-vnote", textContent: "No app is playing sound." });
    apps.append(el("div", { className: "aud-vbody" }, el("p", { className: "hint aud-vnote", textContent: "Windows' own volume for each app, as in its Volume mixer. Nothing is moved." }), appList, appEmpty));

    const devs = panel("aud-devs", "Devices");
    const outList = el("ul", { className: "aud-vlist" });
    outList.setAttribute("aria-labelledby", "aud-devs-out");
    const inList = el("ul", { className: "aud-vlist" });
    inList.setAttribute("aria-labelledby", "aud-devs-in");
    const problem = el("p", { className: "problem", role: "alert", hidden: true });
    devs.append(
      el(
        "div",
        { className: "aud-vbody" },
        el("h3", { className: "aud-vhead", id: "aud-devs-out", textContent: "Playback" }),
        outList,
        el("h3", { className: "aud-vhead", id: "aud-devs-in", textContent: "Recording" }),
        inList,
        el("p", { className: "hint aud-vnote", textContent: "Recording levels move only while an app records from the device." }),
      ),
    );

    function row(key, name, onVolume, onMute, extra, capture) {
      const r = { key };
      r.name = el("span", { className: "aud-chipname", textContent: name, title: name });
      r.sub = el("span", { className: "aud-vsub" });
      r.fill = el("span");
      const meter = el("span", { className: "aud-meter" }, r.fill);
      meter.setAttribute("aria-hidden", "true");
      r.vol = el("input", { type: "range", min: 0, max: 100, step: 1 });
      r.vol.setAttribute("aria-label", name + " volume");
      r.num = el("span", { className: "aud-num" });
      r.vol.oninput = () => {
        setText(r.num, r.vol.value + "%");
        later("v" + key, () => onVolume(r.vol.value / 100));
      };
      const [on, off] = MUTE_ICONS[capture ? "in" : "out"];
      r.mute = el("button", { type: "button", className: "aud-mute aud-icon-btn", title: "Mute" }, on.cloneNode(true), icon(off, "off"));
      r.mute.setAttribute("aria-label", "Mute " + name);
      r.mute.onclick = () => {
        const on = r.mute.getAttribute("aria-pressed") !== "true";
        r.mute.setAttribute("aria-pressed", String(on));
        later("m" + key, () => onMute(on));
      };
      r.node = el(
        "li",
        { className: "aud-vrow2" },
        el("span", { className: "aud-vname" }, r.name, r.sub),
        meter,
        el("span", { className: "aud-vol" }, r.vol, r.num),
        r.mute,
        ...(extra ? [extra] : []),
      );
      return r;
    }

    function show(r, v) {
      if (!held("v" + r.key)) {
        r.vol.value = Math.round(v.volume * 100);
        setText(r.num, r.vol.value + "%");
      }
      if (!held("m" + r.key)) r.mute.setAttribute("aria-pressed", String(!!v.muted));
      r.mute.title = r.mute.getAttribute("aria-pressed") === "true" ? "Unmute" : "Mute";
      r.peak = Math.max(v.peak || 0, (r.peak || 0) * 0.7);
      r.fill.style.transform = "scaleX(" + Math.min(1, r.peak).toFixed(3) + ")";
    }

    async function call(cmd, args) {
      try {
        take(await invoke(cmd, args));
        problem.hidden = true;
      } catch (e) {
        problem.hidden = false;
        setText(problem, errText(e));
      }
    }

    function channelOf(exe) {
      const mix = getMix();
      const c = mix && mix.config.channels.find((x) => x.channel !== "chatCapture" && (x.apps || []).some((a) => a.toLowerCase() === exe.toLowerCase()));
      return c ? LABELS[c.channel] : "";
    }

    function renderApps(list) {
      const keys = list.map((a) => a.exe + "|" + a.endpoint).join("\n");
      if (keys !== st.keys) {
        st.keys = keys;
        const next = new Map();
        for (const a of list) {
          const key = "a" + a.exe + "|" + a.endpoint;
          next.set(key, st.rows.get(key) || row(key, shortName(a.exe), (v) => call("set_app_volume", { exe: a.exe, endpoint: a.endpoint, volume: v, muted: null }), (m) => call("set_app_volume", { exe: a.exe, endpoint: a.endpoint, volume: null, muted: m })));
        }
        for (const [k, r] of st.rows) if (k[0] === "a" && !next.has(k)) st.rows.delete(k);
        for (const [k, r] of next) st.rows.set(k, r);
        appList.replaceChildren(...list.map((a) => st.rows.get("a" + a.exe + "|" + a.endpoint).node));
      }
      for (const a of list) {
        const r = st.rows.get("a" + a.exe + "|" + a.endpoint);
        const ch = channelOf(a.exe);
        setText(r.sub, (a.device || "Unknown output") + (ch ? " · FoxDen " + ch : "") + (a.active ? "" : " · idle"));
        r.node.classList.toggle("idle", !a.active);
        show(r, a);
      }
      appEmpty.hidden = list.length > 0;
      setText(apps.count, String(list.filter((a) => a.active).length));
    }

    function renderDevices(list) {
      const pv = getPriority();
      const picked = new Set(pv && pv.config.on ? [...pv.outputs, ...pv.inputs].filter((r) => r.chosen).map((r) => r.id) : []);
      const keys = list.map((d) => d.id + d.capture + d.default + (d.note || "")).join("\n");
      if (keys !== st.devKeys) {
        st.devKeys = keys;
        for (const k of [...st.rows.keys()]) if (k[0] === "d") st.rows.delete(k);
        for (const d of list) {
          const key = "d" + d.id;
          const make = el("button", { type: "button", className: "aud-make aud-icon-btn", title: "Make default" }, icon(STAR));
          make.setAttribute("aria-label", d.capture ? "Make " + d.name + " the default recording device" : "Make " + d.name + " the default playback device");
          make.hidden = d.default || !!d.note;
          make.onclick = () => call("make_default_device", { id: d.id });
          st.rows.set(key, row(key, d.name, (v) => call("set_device_volume", { id: d.id, volume: v, muted: null }), (m) => call("set_device_volume", { id: d.id, volume: null, muted: m }), make, d.capture));
        }
        outList.replaceChildren(...list.filter((d) => !d.capture).map((d) => st.rows.get("d" + d.id).node));
        inList.replaceChildren(...list.filter((d) => d.capture).map((d) => st.rows.get("d" + d.id).node));
      }
      for (const d of list) {
        const r = st.rows.get("d" + d.id);
        const tags = [d.default ? "Default" : "", picked.has(d.id) ? "Priority pick" : "", d.note || ""].filter(Boolean);
        setText(r.sub, tags.join(" · "));
        r.node.classList.toggle("special", !!d.note);
        r.node.classList.toggle("chosen", d.default);
        show(r, d);
      }
      setText(devs.count, String(list.length));
    }

    function take(view) {
      st.view = view;
      renderApps(view.apps || []);
      renderDevices(view.devices || []);
    }

    let busy = false;
    async function poll() {
      if (busy || root.hidden || document.hidden || !(apps.open || devs.open)) return;
      busy = true;
      try {
        take(await invoke("get_volumes"));
      } catch (e) {
        problem.hidden = false;
        setText(problem, errText(e));
      } finally {
        busy = false;
      }
    }
    setInterval(poll, POLL_MS);

    return [el("div", { className: "aud-vols" }, apps, el("div", {}, devs, problem))];
  }

  window.__foxAudioPanels = { build };
})();
