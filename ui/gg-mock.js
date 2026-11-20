// Mock of SteelSeries Engine's devices for the plain-browser preview.
(function () {
  const mock = window.__foxMock;
  if (!mock) return;
  const devices = [
    { id: 358, name: "Arctis Nova 7", kind: "Headset", usb: "1038:22a1", firmware: "1.4.2", update: true, updating: false, battery: 64, charging: false },
    { id: 412, name: "Aerox 5 Wireless", kind: "Mouse", usb: "1038:1854", firmware: "", update: true, updating: false, battery: 12, charging: true },
    { id: 97, name: "Apex 7", kind: "Keyboard", usb: "1038:1612", firmware: "2.0.11", update: false, updating: false, battery: null, charging: false },
  ];
  // Starts offline, as GG is on this PC; ?gg=online shows devices.
  const online = new URLSearchParams(location.search).get("gg") === "online";
  const view = () => {
    const s = mock.commands.get_snapshot().settings;
    if (!s.steelseries_gg) return { on: false, online: false, error: null, devices: [] };
    return online ? { on: true, online: true, error: null, devices } : { on: true, online: false, error: "SteelSeries GG is not running", devices: [] };
  };
  Object.assign(mock.commands, {
    gg_devices: () => view(),
    gg_refresh() {
      setTimeout(() => mock.emit("gg-devices", view()), 200);
    },
    gg_firmware_update({ id }) {
      const d = devices.find((x) => x.id === id);
      if (!d) throw "the device is not connected";
      if (d.battery != null && d.battery <= 15) throw `battery at ${d.battery}%: charge it above 15% first`;
      d.updating = true;
      setTimeout(() => {
        d.updating = false;
        d.update = false;
        d.firmware = "1.5.0";
        mock.emit("gg-devices", view());
      }, 4000);
      return "Updating " + d.name;
    },
  });
})();
