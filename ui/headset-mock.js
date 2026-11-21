// Stand-in for the headset commands when the page runs in a plain browser.
(function () {
  const mock = window.__foxMock;
  if (!mock) return;
  const levels = { type: "choice", options: [[0, "Off"], [1, "Low"], [2, "Medium"], [3, "High"]] };
  const c = (id, shape, value, read = true) => ({ id, shape, value, read });
  const headsets = [
    {
      key: "1038:22A1",
      name: "SteelSeries Arctis Nova 7",
      answered: true,
      connected: true,
      battery: 46,
      charging: false,
      chatmix: [100, 70],
      controls: [
        c("sidetone", levels, 0),
        c("mic_volume", { type: "steps", min: 0, max: 7 }, 7),
        c("volume_limiter", { type: "switch" }, 1),
        c("auto_off", { type: "minutes", max: 255, presets: [0, 1, 5, 10, 15, 30, 60, 90] }, 0),
        c("mic_led", levels, 1),
        c("bt_power_on", { type: "switch" }, 0),
        c("bt_call", { type: "choice", options: [[0, "No change"], [1, "-12 dB"], [2, "Mute game"]] }, 0),
      ],
      eq: null,
    },
    {
      key: "1038:12E0",
      name: "SteelSeries Arctis Nova Pro Wireless",
      answered: true,
      connected: true,
      battery: 75,
      charging: true,
      chatmix: null,
      controls: [
        c("sidetone", levels, 2, false),
        c("auto_off", { type: "choice", options: [[0, "Never"], [1, "1 min"], [2, "5 min"], [3, "10 min"], [4, "15 min"], [5, "30 min"], [6, "60 min"]] }, null, false),
        c("eq_preset", { type: "choice", options: [[0, "Flat"], [1, "Bass"], [2, "Focus"], [3, "Smiley"]] }, null, false),
      ],
      eq: { bands: null, min: -20, max: 20 },
    },
  ];
  // ?headset=none shows the page without a headset.
  const none = new URLSearchParams(location.search).get("headset") === "none";
  const find = (key) => {
    const h = headsets.find((x) => x.key === key);
    if (!h) throw "the headset's receiver is not plugged in";
    if (!h.connected) throw "the headset is switched off";
    return h;
  };
  Object.assign(mock.commands, {
    headset_list: () => (none ? [] : JSON.parse(JSON.stringify(headsets))),
    headset_set({ key, control, value }) {
      const ctl = find(key).controls.find((x) => x.id === control);
      if (!ctl) throw "no such setting";
      ctl.value = value;
    },
    headset_eq({ key, bands }) {
      const h = find(key);
      h.eq.bands = bands.slice();
      h.controls.find((x) => x.id === "eq_preset").value = 4;
    },
  });
})();
