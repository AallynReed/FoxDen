use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use foxrgb_fan::{Curve, FanRule};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use super::*;

const QUICK: Duration = Duration::from_millis(300);

const STATUS: &str = r#"{"ok":true,"version":"0.1.0","status":{
    "sensors":[{"id":"sim:cpu","name":"CPU","value":41.5},{"id":"sim:gone","name":"Gone","value":null}],
    "fans":[{"id":"sim:fan1","name":"Fan 1","rpm":900,"duty":0.4,"curve":"cpu","failsafe":false,"error":null},
            {"id":"sim:fan2","name":"Fan 2","rpm":null,"duty":null,"curve":null,"failsafe":true,"error":"stuck"}]}}"#;

/// One client connected to the fake service.
struct Peer(Conn);

impl Peer {
    /// The next request line, or `None` once the client has gone.
    fn read(&mut self) -> Option<String> {
        let mut line = String::new();
        match self.0.read_line(&mut line) {
            Ok(n) if n > 0 => Some(line.trim_end().to_string()),
            _ => None,
        }
    }

    fn write(&mut self, reply: &str) {
        let reply = reply.replace('\n', "");
        let _ = self.0.get_ref().write_all(format!("{reply}\n").as_bytes());
    }
}

fn listen(pipe: &str) -> File {
    let wide: Vec<u16> = pipe.encode_utf16().chain(Some(0)).collect();
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT;
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            mode,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(
        handle,
        INVALID_HANDLE_VALUE,
        "{pipe}: {}",
        std::io::Error::last_os_error()
    );
    unsafe { File::from_raw_handle(handle as _) }
}

/// Starts a pipe server that hands every client to `peer` on its own thread. Returns the pipe's name.
fn serve(name: &str, peer: impl Fn(Peer) + Send + Sync + 'static) -> String {
    let pipe = format!(r"\\.\pipe\foxrgb-client-test-{}-{name}", std::process::id());
    let peer = Arc::new(peer);
    let mut listener = listen(&pipe);
    let again = pipe.clone();
    thread::spawn(move || {
        loop {
            let handle = listener.as_raw_handle() as _;
            let connected = unsafe {
                ConnectNamedPipe(handle, std::ptr::null_mut()) != 0
                    || GetLastError() == ERROR_PIPE_CONNECTED
            };
            let client = std::mem::replace(&mut listener, listen(&again));
            if connected {
                let peer = peer.clone();
                thread::spawn(move || peer(Peer(BufReader::new(client))));
            }
        }
    });
    pipe
}

/// A server that sends the same line back whatever it is asked.
fn parrot(name: &str, reply: &'static str) -> Client {
    let pipe = serve(name, move |mut peer| {
        while peer.read().is_some() {
            peer.write(reply);
        }
    });
    Client::new(&pipe, QUICK)
}

fn sample() -> FanConfig {
    FanConfig {
        curves: vec![Curve {
            name: "cpu".into(),
            sensors: vec!["sim:cpu".into()],
            points: vec![(30.0, 0.25), (80.0, 1.0)],
            hysteresis: 2.0,
        }],
        rules: vec![FanRule {
            fan: "sim:fan1".into(),
            curve: "cpu".into(),
            min_duty: 0.2,
            max_duty: 1.0,
            ramp: 0.5,
        }],
    }
}

/// Answers the way the service does, with fixed data, and records what it was asked.
fn fake(name: &str) -> (Client, Arc<Mutex<Vec<Value>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let pipe = serve(name, move |mut peer| {
        while let Some(line) = peer.read() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = match request["cmd"].as_str() {
                Some("status") => STATUS.to_string(),
                Some("get_config") => json!({"ok": true, "config": sample()}).to_string(),
                Some("set_config") if request["config"]["curves"][0]["points"] == json!([]) => {
                    json!({"ok": false, "error": "curve 'cpu' has no points"}).to_string()
                }
                Some("set_config" | "shutdown") => json!({"ok": true}).to_string(),
                Some("close_conflicts") => {
                    json!({"ok": true, "closed": ["Fan Control"], "left": ["Corsair iCUE"]})
                        .to_string()
                }
                _ => json!({"ok": false, "error": "bad request"}).to_string(),
            };
            log.lock().unwrap().push(request);
            peer.write(&reply);
        }
    });
    (Client::new(&pipe, QUICK), asked)
}

fn report() -> Report {
    let sensor = |id: &str, name: &str, value| SensorStatus {
        id: id.into(),
        name: name.into(),
        value,
    };
    let fan1 = FanStatus {
        id: "sim:fan1".into(),
        name: "Fan 1".into(),
        rpm: Some(900),
        duty: Some(0.4),
        curve: Some("cpu".into()),
        failsafe: false,
        error: None,
    };
    let fan2 = FanStatus {
        id: "sim:fan2".into(),
        name: "Fan 2".into(),
        failsafe: true,
        error: Some("stuck".into()),
        ..FanStatus::default()
    };
    Report {
        version: "0.1.0".into(),
        status: Status {
            sensors: vec![
                sensor("sim:cpu", "CPU", Some(41.5)),
                sensor("sim:gone", "Gone", None),
            ],
            fans: vec![fan1, fan2],
        },
    }
}

#[test]
fn calls_send_the_protocol_requests_and_read_the_replies() {
    let (client, asked) = fake("calls");
    assert_eq!(client.status(), Ok(report()));
    assert_eq!(client.get_config(), Ok(sample()));
    assert_eq!(client.set_config(&sample()), Ok(()));
    assert_eq!(client.shutdown(), Ok(()));
    let config = json!({
        "curves": [{"name": "cpu", "sensors": ["sim:cpu"], "points": [[30.0, 0.25], [80.0, 1.0]], "hysteresis": 2.0}],
        "rules": [{"fan": "sim:fan1", "curve": "cpu", "min_duty": 0.2, "max_duty": 1.0, "ramp": 0.5}],
    });
    assert_eq!(
        *asked.lock().unwrap(),
        [
            json!({"cmd": "status"}),
            json!({"cmd": "get_config"}),
            json!({"cmd": "set_config", "config": config}),
            json!({"cmd": "shutdown"}),
        ]
    );
}

#[test]
fn close_conflicts_sends_the_scope_and_no_names() {
    let (client, asked) = fake("close");
    let scope = Scope {
        lighting: true,
        fans: false,
        keep_openrgb: true,
        keep_steelseries: false,
    };
    let closed = client.close_conflicts(scope).unwrap();
    assert_eq!(closed.closed, ["Fan Control"]);
    assert_eq!(closed.left, ["Corsair iCUE"]);
    assert_eq!(
        *asked.lock().unwrap(),
        [json!({"cmd": "close_conflicts", "lighting": true, "fans": false, "keep_openrgb": true, "keep_steelseries": false})]
    );
}

#[test]
fn a_rejected_config_returns_the_services_error() {
    let (client, _) = fake("reject");
    let mut config = sample();
    config.curves[0].points.clear();
    assert_eq!(
        client.set_config(&config),
        Err("curve 'cpu' has no points".into())
    );
    assert_eq!(client.state().service, Service::Running);
}

#[test]
fn state_is_built_from_status_and_config() {
    let (client, _) = fake("state");
    let state = client.state();
    let report = report();
    let expected = FanState {
        service: Service::Running,
        detail: None,
        version: Some("0.1.0".into()),
        sensors: report.status.sensors,
        fans: report.status.fans,
        config: sample(),
    };
    assert_eq!(state, expected);

    let json = serde_json::to_value(&state).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["config", "detail", "fans", "sensors", "service", "version"]
    );
    assert_eq!(json["service"], "running");
    assert_eq!(
        json["sensors"][1],
        json!({"id": "sim:gone", "name": "Gone", "value": null})
    );
    assert_eq!(
        json["fans"][0],
        json!({"id": "sim:fan1", "name": "Fan 1", "rpm": 900, "duty": 0.4f32, "curve": "cpu", "failsafe": false, "error": null})
    );
    assert_eq!(
        json["config"]["curves"][0]["points"],
        json!([[30.0, 0.25], [80.0, 1.0]])
    );
    assert_eq!(json["config"]["rules"][0]["ramp"], 0.5);
}

#[test]
fn a_status_with_sim_from_an_older_service_still_reads() {
    let client = parrot(
        "old",
        r#"{"ok":true,"version":"0.0.9","sim":true,"status":{"sensors":[],"fans":[]}}"#,
    );
    assert_eq!(client.status().unwrap().version, "0.0.9");
}

#[test]
fn nothing_listening_is_stopped_not_an_error() {
    let pipe = format!(r"\\.\pipe\foxrgb-client-test-{}-absent", std::process::id());
    let client = Client::new(&pipe, QUICK);
    assert_eq!(client.status(), Err(NOT_RUNNING.into()));
    assert_eq!(client.get_config(), Err(NOT_RUNNING.into()));
    let state = client.state();
    assert_eq!(state, FanState::default());
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        (&json["service"], &json["detail"], &json["version"]),
        (&json!("stopped"), &Value::Null, &Value::Null)
    );
}

#[test]
fn a_service_that_hangs_up_is_an_error() {
    let pipe = serve("hangup", |mut peer| {
        peer.read();
    });
    let client = Client::new(&pipe, QUICK);
    assert_eq!(client.status(), Err(CLOSED.into()));
    let state = client.state();
    assert_eq!(
        (state.service, state.detail.as_deref()),
        (Service::Error, Some(CLOSED))
    );
    assert_eq!(serde_json::to_value(&state).unwrap()["service"], "error");
}

#[test]
fn a_reply_that_makes_no_sense_is_an_error() {
    let cases = [
        ("text", "hello", "makes no sense"),
        ("bare", r#"{"version":"0.1.0"}"#, "makes no sense"),
        ("short", r#"{"ok":true}"#, "makes no sense"),
        ("refused", r#"{"ok":false,"error":"busy"}"#, "busy"),
        ("mute", r#"{"ok":false}"#, "refused the request"),
    ];
    for (name, reply, expected) in cases {
        let state = parrot(name, reply).state();
        assert_eq!(state.service, Service::Error, "{reply}");
        let detail = state.detail.unwrap();
        assert!(detail.contains(expected), "{reply}: {detail}");
    }
}

#[test]
fn a_silent_service_cannot_hold_a_call_past_the_timeout() {
    let held = Arc::new(AtomicBool::new(true));
    let gate = held.clone();
    let pipe = serve("silent", move |mut peer| {
        while peer.read().is_some() {
            while gate.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            peer.write(STATUS);
        }
    });
    let client = Client::new(&pipe, QUICK);

    let start = Instant::now();
    assert_eq!(client.status(), Err(SILENT.into()));
    assert!(
        (QUICK..QUICK * 6).contains(&start.elapsed()),
        "{:?}",
        start.elapsed()
    );

    // While the first answer is still owed, later calls fail at once instead of piling up.
    let start = Instant::now();
    let state = client.state();
    assert_eq!(
        (state.service, state.detail.as_deref()),
        (Service::Error, Some(SILENT))
    );
    assert!(start.elapsed() < QUICK, "{:?}", start.elapsed());

    held.store(false, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.status() != Ok(report()) {
        assert!(Instant::now() < deadline, "the client never recovered");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_shutdown_gets_through_while_another_answer_is_owed() {
    let stopped = Arc::new(AtomicBool::new(false));
    let seen = stopped.clone();
    let pipe = serve("stop", move |mut peer| {
        while let Some(line) = peer.read() {
            if line.contains("shutdown") {
                seen.store(true, Ordering::Relaxed);
                peer.write(r#"{"ok":true}"#);
            }
            // Anything else is never answered.
        }
    });
    let client = Client::new(&pipe, QUICK);
    assert_eq!(client.status(), Err(SILENT.into()));
    assert_eq!(client.get_config(), Err(SILENT.into()));
    assert_eq!(client.shutdown(), Ok(()));
    assert!(stopped.load(Ordering::Relaxed));
}

#[test]
fn a_shutdown_that_is_never_answered_gives_up_in_time() {
    let pipe = serve("deaf", |mut peer| while peer.read().is_some() {});
    let client = Client::new(&pipe, QUICK);
    let start = Instant::now();
    assert_eq!(client.shutdown(), Err(SILENT.into()));
    assert!(start.elapsed() < QUICK * 6, "{:?}", start.elapsed());
}

#[test]
fn an_answer_that_comes_late_is_not_taken_for_the_next_one() {
    let held = Arc::new(AtomicBool::new(true));
    let gate = held.clone();
    let pipe = serve("late", move |mut peer| {
        while let Some(line) = peer.read() {
            while gate.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            if line.contains("get_config") {
                peer.write(&json!({"ok": true, "config": sample()}).to_string());
            } else {
                peer.write(STATUS);
            }
        }
    });
    let client = Client::new(&pipe, QUICK);
    assert_eq!(client.status(), Err(SILENT.into()));

    held.store(false, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match client.get_config() {
            Ok(config) => {
                assert_eq!(config, sample());
                break;
            }
            Err(e) => assert_eq!(e, SILENT),
        }
        assert!(Instant::now() < deadline, "the client never recovered");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_pipe_names_the_process_that_serves_it() {
    let pipe = serve("pid", |mut peer| while peer.read().is_some() {});
    assert_eq!(server_pid(&pipe), Some(std::process::id()));
    assert_eq!(server_pid(&format!("{pipe}-absent")), None);
}

#[test]
fn reconnects_when_the_connection_is_gone() {
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let pipe = serve("restart", move |mut peer| {
        count.fetch_add(1, Ordering::Relaxed);
        // One answer, then hang up, as if the service had been restarted.
        if peer.read().is_some() {
            peer.write(STATUS);
        }
    });
    let client = Client::new(&pipe, QUICK);
    for _ in 0..3 {
        assert_eq!(client.status(), Ok(report()));
    }
    assert_eq!(connections.load(Ordering::Relaxed), 3);
}

#[test]
fn calls_from_several_threads_take_turns() {
    let (client, asked) = fake("threads");
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..20 {
                    assert_eq!(client.status(), Ok(report()));
                }
            });
        }
    });
    assert_eq!(asked.lock().unwrap().len(), 160);
}

/// Ends the service even when the test fails halfway.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Drives a real foxrgb-service on the real pipe, so it cannot run beside another instance.
#[test]
#[ignore = "needs FOXRGB_SERVICE_EXE and no other foxrgb-service running"]
fn drives_the_real_service() {
    let exe = std::env::var("FOXRGB_SERVICE_EXE")
        .expect("FOXRGB_SERVICE_EXE points at a built foxrgb-service.exe");
    let data = std::env::current_exe()
        .unwrap()
        .with_file_name(format!("foxrgb-client-real-{}", std::process::id()));
    let client = Client::new(PIPE, Duration::from_secs(5));
    assert_eq!(
        client.state(),
        FanState::default(),
        "another service is already running"
    );

    let service = Command::new(exe)
        .arg("run")
        .env("FOXRGB_DATA_DIR", &data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut service = Reaped(service);
    let wait_for = |what: &str, done: &dyn Fn(&FanState) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = client.state();
            if done(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {state:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    };

    let state = wait_for("the service to answer", &|state| {
        state.service == Service::Running
    });
    assert_eq!(state.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(state.config, FanConfig::default());
    assert_eq!(server_pid(PIPE), Some(service.0.id()));

    // A curve with no rules, so no real fan is driven.
    let config = FanConfig {
        curves: vec![Curve {
            name: "all".into(),
            sensors: vec!["none".into()],
            points: vec![(30.0, 0.3), (80.0, 1.0)],
            hysteresis: 2.0,
        }],
        rules: Vec::new(),
    };
    client.set_config(&config).unwrap();
    assert_eq!(client.get_config(), Ok(config.clone()));

    let mut broken = config.clone();
    broken.rules.push(FanRule {
        fan: "none".into(),
        curve: "missing".into(),
        min_duty: 0.3,
        max_duty: 1.0,
        ramp: 1.0,
    });
    let refused = client.set_config(&broken).unwrap_err();
    assert!(refused.contains("unknown curve"), "{refused}");
    assert_eq!(client.get_config(), Ok(config));
    client.set_config(&FanConfig::default()).unwrap();

    client.shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "the service did not exit after shutdown"
        );
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(client.state(), FanState::default());
    assert_eq!(server_pid(PIPE), None);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn an_older_service_refuses_hub_lights_and_still_answers_the_rest() {
    let (client, _) = fake("old-hubs");
    assert!(client.hub_lights(&BTreeMap::new()).is_err());
    assert!(client.set_hub_lights(&[]).is_ok());
    assert_eq!(client.status(), Ok(report()));
}

#[test]
fn hub_lights_send_the_zone_lengths_and_frames_in_batches() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let pipe = serve("hubs", move |mut peer| {
        while let Some(line) = peer.read() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = match request["cmd"].as_str() {
                Some("hub_lights") => json!({"ok": true, "devices": [], "generation": 7,
                    "zones": [{"device": "hid:hub", "index": 1, "name": "Fan 1",
                    "leds": request["lengths"]["hid:hub#1"], "min": 0, "max": 34}]}),
                Some("set_hub_lights") => {
                    json!({"ok": true, "failed": {"hid:gone": "unknown device"}})
                }
                _ => json!({"ok": true}),
            };
            log.lock().unwrap().push(request);
            peer.write(&reply.to_string());
        }
    });
    let client = Client::new(&pipe, QUICK);
    let lengths = BTreeMap::from([("hid:hub#1".to_string(), 12)]);
    let hubs = client.hub_lights(&lengths).unwrap();
    assert_eq!((hubs.zones[0].leds, hubs.generation), (12, 7));
    let frame = |device: &str| HubFrame {
        device: device.into(),
        colors: "ff0000".into(),
    };
    let sent = client
        .set_hub_lights(&[frame("hid:hub"), frame("hid:gone")])
        .unwrap();
    assert!(sent.open);
    assert_eq!(sent.failed["hid:gone"], "unknown device");
    client.release_hub_lights(hubs.generation).unwrap();
    let asked = asked.lock().unwrap();
    assert_eq!(
        asked[0],
        json!({"cmd": "hub_lights", "lengths": {"hid:hub#1": 12}})
    );
    assert_eq!(
        asked[1],
        json!({"cmd": "set_hub_lights", "frames": [
            {"device": "hid:hub", "colors": "ff0000"},
            {"device": "hid:gone", "colors": "ff0000"}]})
    );
    assert_eq!(
        asked[2],
        json!({"cmd": "release_hub_lights", "generation": 7})
    );
}

#[test]
fn hub_frames_are_split_to_fit_a_line() {
    let frame = |n: usize| HubFrame {
        device: "d".into(),
        colors: "0".repeat(n),
    };
    let frames = [frame(50), frame(50), frame(200), frame(10)];
    let sizes: Vec<usize> = batches(&frames, 180).iter().map(|b| b.len()).collect();
    assert_eq!(sizes, [2, 1, 1]);
    assert!(batches(&[], 180).is_empty());
}
