// Stand-in for the fan commands when the page runs in a plain browser.
(function () {
  const mock = window.__foxMock;
  if (!mock) return;

  const AMBIENT = 25;
  const FIRMWARE_DUTY = 0.4;
  const OFFLINE = "the background service is not running";
  // Id, name and degrees above ambient at full load with stopped fans.
  const SENSORS = [
    ["sim/cpu", "Simulated CPU", 70],
    ["sim/gpu", "Simulated GPU", 60],
  ];
  // Id, name and rpm at full duty.
  const FANS = [
    ["sim/cpu_fan", "Simulated CPU fan", 1800],
    ["sim/case_fan", "Simulated case fan", 1200],
    ["sim/gpu_fan", "Simulated GPU fan", 3000],
  ];

  const copy = (v) => JSON.parse(JSON.stringify(v));
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
  // Load swings between idle and busy so the curves have something to follow.
  const loadAt = (t) => 0.5 - 0.45 * Math.cos((t / 90) * 2 * Math.PI);

  const svc = { running: false, sim: false, time: 0, config: { curves: [], rules: [] }, anchors: [], sensors: [], fans: [] };
  const model = { temps: [42, 38], duties: [null, null, null], rpms: [0, 0, 0] };

  function dutyAt(points, temp) {
    if (temp <= points[0][0]) return points[0][1];
    for (let i = 1; i < points.length; i++) {
      const [t0, d0] = points[i - 1];
      const [t1, d1] = points[i];
      if (temp <= t1) return d0 + ((d1 - d0) * (temp - t0)) / (t1 - t0);
    }
    return points[points.length - 1][1];
  }

  function step(dt) {
    const duty = model.duties.map((d) => (d === null ? FIRMWARE_DUTY : d));
    // The case fan helps both; each part mostly depends on its own fan.
    const airflow = [0.7 * duty[0] + 0.3 * duty[1], 0.7 * duty[2] + 0.3 * duty[1]];
    const heat = 1 - Math.exp(-dt / 8);
    SENSORS.forEach((sensor, i) => {
      const settled = AMBIENT + loadAt(svc.time + i * 30) * sensor[2] * (1 - 0.6 * airflow[i]);
      model.temps[i] += (settled - model.temps[i]) * heat;
    });
    const spin = 1 - Math.exp(-dt);
    FANS.forEach((fan, i) => (model.rpms[i] += (duty[i] * fan[2] - model.rpms[i]) * spin));
  }

  // The service's control loop: hottest sensor, hysteresis, limits, ramp, full speed when a sensor is lost.
  function control(dt, sensors) {
    const config = svc.config;
    const wanted = config.curves.map((curve, i) => {
      const temps = curve.sensors.map((id) => (sensors.find((s) => s.id === id) || { value: null }).value);
      const hottest = temps.length && !temps.includes(null) ? Math.max(...temps) : null;
      const held = svc.anchors[i];
      const keep =
        hottest !== null &&
        held != null &&
        hottest < held &&
        hottest > held - curve.hysteresis &&
        dutyAt(curve.points, hottest) <= dutyAt(curve.points, held);
      svc.anchors[i] = keep ? held : hottest;
      return svc.anchors[i] === null ? null : dutyAt(curve.points, svc.anchors[i]);
    });

    return FANS.map((fan, i) => {
      const view = { id: fan[0], name: fan[1], rpm: Math.round(model.rpms[i]), duty: null, curve: null, failsafe: false, error: null };
      const rule = config.rules.find((r) => r.fan === fan[0]);
      if (!rule) {
        model.duties[i] = null;
        return view;
      }
      const want = wanted[config.curves.findIndex((c) => c.name === rule.curve)];
      let duty = 1;
      if (want === null) {
        view.failsafe = true;
      } else {
        const target = clamp(want, rule.min_duty, rule.max_duty);
        const now = model.duties[i];
        const most = rule.ramp * dt;
        duty = now !== null && Math.abs(target - now) > most ? now + Math.sign(target - now) * most : target;
      }
      model.duties[i] = duty;
      view.duty = duty;
      view.curve = rule.curve;
      return view;
    });
  }

  function tick(dt) {
    svc.time += dt;
    const sensors = [
      { id: "board/system", name: "Mainboard", value: 36 + 2 * Math.sin(svc.time / 50) },
      { id: "disk/0", name: "System drive", value: null },
    ];
    if (svc.sim) {
      step(dt);
      sensors.unshift(...SENSORS.map((sensor, i) => ({ id: sensor[0], name: sensor[1], value: model.temps[i] })));
    } else {
      model.duties.fill(null);
    }
    svc.sensors = sensors;
    svc.fans = svc.sim ? control(dt, sensors) : [];
  }

  function state() {
    if (!svc.running) {
      return { service: "stopped", detail: null, version: null, sim: false, sensors: [], fans: [], config: { curves: [], rules: [] } };
    }
    return copy({ service: "running", detail: null, version: "0.1.0", sim: svc.sim, sensors: svc.sensors, fans: svc.fans, config: svc.config });
  }

  function tell() {
    if (svc.running) tick(0);
    mock.emit("fans", state());
  }

  // The same rules and wording as FanConfig::validate in foxrgb-fan.
  function validate(config) {
    const names = new Set();
    for (const curve of config.curves) {
      const name = curve.name;
      if (names.has(name)) return `curve name '${name}' is used more than once`;
      names.add(name);
      if (!curve.sensors.length) return `curve '${name}' has no sensors`;
      if (!curve.points.length) return `curve '${name}' has no points`;
      for (const [temp, duty] of curve.points) {
        if (!Number.isFinite(temp) || !Number.isFinite(duty)) return `curve '${name}' has a point that is not a finite number`;
        if (duty < 0 || duty > 1) return `curve '${name}' has a duty outside 0 to 1`;
      }
      if (curve.points.some((p, i) => i > 0 && curve.points[i - 1][0] >= p[0])) {
        return `curve '${name}' points must be sorted by rising temperature`;
      }
      if (!(curve.hysteresis >= 0 && curve.hysteresis <= 30)) return `curve '${name}' hysteresis must be from 0 to 30`;
    }

    const fans = new Set();
    for (const rule of config.rules) {
      const fan = rule.fan;
      if (fans.has(fan)) return `fan '${fan}' has more than one rule`;
      fans.add(fan);
      if (!names.has(rule.curve)) return `fan '${fan}' uses unknown curve '${rule.curve}'`;
      if (!(rule.min_duty >= 0 && rule.min_duty <= rule.max_duty && rule.max_duty <= 1)) {
        return `fan '${fan}' needs duties from 0 to 1, with the minimum not above the maximum`;
      }
      if (!(rule.ramp >= 0.01 && Number.isFinite(rule.ramp))) return `fan '${fan}' ramp must be a finite number of at least 0.01`;
    }
    return null;
  }

  Object.assign(mock.commands, {
    get_fans: () => state(),
    set_fan_config({ config }) {
      if (!svc.running) throw OFFLINE;
      const error = validate(config);
      if (error) throw error;
      svc.config = config;
      svc.anchors = [];
      tell();
    },
    set_fan_sim({ on }) {
      if (!svc.running) throw OFFLINE;
      svc.sim = Boolean(on);
      tell();
    },
    // Takes a moment, as starting a process does.
    start_service() {
      if (svc.running) return null;
      return new Promise((resolve) => {
        setTimeout(() => {
          svc.running = true;
          tell();
          resolve(null);
        }, 600);
      });
    },
    stop_service() {
      svc.running = false;
      model.duties.fill(null);
      tell();
    },
  });

  setInterval(() => {
    if (!svc.running) return;
    tick(1);
    mock.emit("fans", state());
  }, 1000);
})();
