use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};

/// Highest SDK protocol version this client understands.
const PROTOCOL_VERSION: u32 = 5;
const MAX_PACKET: u32 = 8 << 20;
const NO_LED: u32 = u32::MAX;

const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const REQUEST_PROTOCOL_VERSION: u32 = 40;
const SET_CLIENT_NAME: u32 = 50;
const DEVICE_LIST_UPDATED: u32 = 100;
const UPDATE_LEDS: u32 = 1050;
const SET_CUSTOM_MODE: u32 = 1100;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
const LIST_ATTEMPTS: usize = 5;

struct Slot {
    device: Device,
    direct: bool,
    last: Vec<Color>,
}

/// Client for an OpenRGB SDK server.
pub struct OpenRgbBackend {
    stream: TcpStream,
    slots: Vec<Slot>,
    dead: bool,
}

impl OpenRgbBackend {
    /// Connects and enumerates controllers. OpenRGB listens on 127.0.0.1:6742 by default.
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let context = |e: String| format!("OpenRGB at {host}:{port}: {e}");
        let mut stream = open(host, port).map_err(context)?;
        let controllers = enumerate(&mut stream).map_err(context)?;
        let count = controllers.len();
        let slots = controllers
            .iter()
            .enumerate()
            .map(|(i, c)| Slot {
                device: device(i, count, c),
                direct: false,
                last: Vec::new(),
            })
            .collect();
        Ok(Self {
            stream,
            slots,
            dead: false,
        })
    }
}

impl Backend for OpenRgbBackend {
    fn name(&self) -> &str {
        "openrgb"
    }

    fn devices(&self) -> Vec<Device> {
        self.slots.iter().map(|s| s.device.clone()).collect()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if self.dead {
            return Err("OpenRGB connection lost".into());
        }
        if let Some(reason) = interruption(&self.stream) {
            self.dead = true;
            return Err(reason.into());
        }
        let index = device_id
            .strip_prefix("openrgb:")
            .and_then(|i| i.parse::<usize>().ok())
            .filter(|&i| i < self.slots.len())
            .ok_or_else(|| format!("unknown device {device_id}"))?;
        let slot = &mut self.slots[index];
        if colors.len() != slot.device.leds.len() {
            return Err(format!(
                "{device_id} has {} LEDs, got {} colours",
                slot.device.leds.len(),
                colors.len()
            ));
        }
        if colors.is_empty() || slot.last.as_slice() == colors {
            return Ok(());
        }

        let mut out = Vec::new();
        if !slot.direct {
            out.extend(packet(index as u32, SET_CUSTOM_MODE, &[]));
        }
        let size = 6 + 4 * colors.len();
        let mut payload = Vec::with_capacity(size);
        payload.extend_from_slice(&(size as u32).to_le_bytes());
        payload.extend_from_slice(&(colors.len() as u16).to_le_bytes());
        for c in colors {
            payload.extend_from_slice(&[c.0, c.1, c.2, 0]);
        }
        out.extend(packet(index as u32, UPDATE_LEDS, &payload));

        if let Err(e) = self.stream.write_all(&out) {
            self.dead = true;
            return Err(format!("OpenRGB connection lost: {e}"));
        }
        slot.direct = true;
        slot.last.clear();
        slot.last.extend_from_slice(colors);
        Ok(())
    }
}

fn err(e: io::Error) -> String {
    e.to_string()
}

fn open(host: &str, port: u16) -> Result<TcpStream, String> {
    let mut last = "no address found".to_string();
    for addr in (host, port).to_socket_addrs().map_err(err)? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true).map_err(err)?;
                stream.set_read_timeout(Some(READ_TIMEOUT)).map_err(err)?;
                stream.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(err)?;
                return Ok(stream);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// Up to protocol 5 the server only speaks unprompted when its device list changed.
fn interruption(stream: &TcpStream) -> Option<&'static str> {
    const LOST: Option<&str> = Some("OpenRGB connection lost");
    if stream.set_nonblocking(true).is_err() {
        return LOST;
    }
    let pending = stream.peek(&mut [0u8; 1]);
    if stream.set_nonblocking(false).is_err() {
        return LOST;
    }
    match pending {
        Err(e) if e.kind() == ErrorKind::WouldBlock => None,
        Ok(n) if n > 0 => Some("OpenRGB device list changed, reconnect to read it again"),
        _ => LOST,
    }
}

fn packet(device: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(16 + payload.len());
    p.extend_from_slice(b"ORGB");
    p.extend_from_slice(&device.to_le_bytes());
    p.extend_from_slice(&id.to_le_bytes());
    p.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    p.extend_from_slice(payload);
    p
}

/// Returns the next packet with `id`. Other packets are skipped, and `changed` is set when
/// one of them announces a new device list.
fn read_reply(stream: &mut TcpStream, id: u32, changed: &mut bool) -> io::Result<Vec<u8>> {
    for _ in 0..64 {
        let mut header = [0u8; 16];
        stream.read_exact(&mut header)?;
        let word = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
        if &header[..4] != b"ORGB" || word(12) > MAX_PACKET {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "not an OpenRGB SDK server",
            ));
        }
        let mut payload = vec![0u8; word(12) as usize];
        stream.read_exact(&mut payload)?;
        if word(8) == id {
            return Ok(payload);
        }
        *changed |= word(8) == DEVICE_LIST_UPDATED;
    }
    Err(io::Error::new(
        ErrorKind::InvalidData,
        "no reply from the server",
    ))
}

fn first_u32(payload: &[u8]) -> Result<u32, String> {
    Reader(payload).u32()
}

fn enumerate(stream: &mut TcpStream) -> Result<Vec<Controller>, String> {
    let ours = PROTOCOL_VERSION.to_le_bytes();
    stream
        .write_all(&packet(0, REQUEST_PROTOCOL_VERSION, &ours))
        .map_err(err)?;
    let version = match read_reply(stream, REQUEST_PROTOCOL_VERSION, &mut false) {
        Ok(payload) => first_u32(&payload)?.min(PROTOCOL_VERSION),
        // Servers that predate protocol 1 never answer this request.
        Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => 0,
        Err(e) => return Err(err(e)),
    };

    stream
        .write_all(&packet(0, SET_CLIENT_NAME, b"FoxRGB\0"))
        .map_err(err)?;
    let request = if version == 0 {
        Vec::new()
    } else {
        version.to_le_bytes().to_vec()
    };
    // OpenRGB announces every controller it detects, so the list can change while we read it.
    for _ in 0..LIST_ATTEMPTS {
        let mut changed = false;
        stream
            .write_all(&packet(0, REQUEST_CONTROLLER_COUNT, &[]))
            .map_err(err)?;
        let count = read_reply(stream, REQUEST_CONTROLLER_COUNT, &mut changed).map_err(err)?;
        let mut controllers = Vec::new();
        for i in 0..first_u32(&count)? {
            stream
                .write_all(&packet(i, REQUEST_CONTROLLER_DATA, &request))
                .map_err(err)?;
            let blob = read_reply(stream, REQUEST_CONTROLLER_DATA, &mut changed).map_err(err)?;
            let controller = parse_controller(&blob, version);
            controllers.push(controller.map_err(|e| format!("controller {i}: {e}"))?);
        }
        if !changed {
            return Ok(controllers);
        }
    }
    Err("the device list kept changing".into())
}

#[derive(Debug, PartialEq)]
struct Zone {
    leds: u32,
    /// Height, width and row-major zone LED indices; `NO_LED` marks an empty cell.
    matrix: Option<(u32, u32, Vec<u32>)>,
}

#[derive(Debug, PartialEq)]
struct Controller {
    kind: u32,
    name: String,
    zones: Vec<Zone>,
    leds: usize,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("the server sent truncated data".into());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// Strings carry a 16-bit length that includes the terminating NUL.
    fn string(&mut self) -> Result<String, String> {
        let len = self.u16()? as usize;
        let text = String::from_utf8_lossy(self.take(len)?);
        Ok(text.trim_end_matches('\0').to_string())
    }
}

/// Reads the controller data block for protocol versions 0 to 5.
fn parse_controller(blob: &[u8], version: u32) -> Result<Controller, String> {
    let mut r = Reader(blob);
    r.u32()?; // data size
    let kind = r.u32()?;
    let name = r.string()?;
    // vendor (from v1), description, version, serial, location
    let strings = if version >= 1 { 5 } else { 4 };
    for _ in 0..strings {
        r.string()?;
    }

    let modes = r.u16()?;
    r.u32()?; // active mode
    for _ in 0..modes {
        r.string()?;
        // nine 32-bit fields; v3 adds brightness min, max and value
        r.take(if version >= 3 { 48 } else { 36 })?;
        let colors = r.u16()? as usize;
        r.take(4 * colors)?;
    }

    let mut zones = Vec::new();
    for _ in 0..r.u16()? {
        r.string()?;
        r.take(12)?; // type, minimum and maximum size
        let leds = r.u32()?;
        let matrix = if r.u16()? > 0 {
            let (height, width) = (r.u32()?, r.u32()?);
            let bytes = (height as usize)
                .checked_mul(width as usize)
                .and_then(|cells| cells.checked_mul(4))
                .ok_or("matrix map is too large")?;
            let map = r
                .take(bytes)?
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()));
            Some((height, width, map.collect()))
        } else {
            None
        };
        if version >= 4 {
            for _ in 0..r.u16()? {
                r.string()?;
                r.take(12)?; // segment type, start, size
            }
        }
        if version >= 5 {
            r.u32()?; // zone flags
        }
        zones.push(Zone { leds, matrix });
    }

    for _ in 0..r.u16()? {
        r.string()?;
        r.u32()?;
    }
    // The server drops an update that carries more colours than this.
    let leds = r.u16()? as usize;
    r.take(4 * leds)?;
    Ok(Controller {
        kind,
        name,
        zones,
        leds,
    })
}

fn device(index: usize, count: usize, c: &Controller) -> Device {
    let (grid, leds) = layout(c);
    let name = if c.name.is_empty() {
        format!("OpenRGB device {index}")
    } else {
        c.name.clone()
    };
    let kind = match c.kind {
        0 => DeviceKind::Motherboard,
        1 => DeviceKind::Memory,
        2 => DeviceKind::Gpu,
        4 => DeviceKind::Strip,
        5 => DeviceKind::Keyboard,
        6 => DeviceKind::Mouse,
        18 => DeviceKind::Keyboard, // keypad
        _ => DeviceKind::Other,
    };
    Device {
        id: format!("openrgb:{index}"),
        name,
        kind,
        rect: flow_rect(index, count, grid),
        grid,
        leds,
    }
}

/// Stacks the zones top to bottom and returns `[columns, rows]` with one position per LED.
fn layout(c: &Controller) -> ([u32; 2], Vec<[f32; 2]>) {
    // matrix map, first LED, LED count, rows, columns
    let mut bands = Vec::new();
    let mut start = 0;
    for zone in &c.zones {
        let n = (zone.leds as usize).min(c.leds - start);
        if n > 0 {
            let matrix = zone.matrix.as_ref().filter(|m| m.0 > 0 && m.1 > 0);
            let (rows, cols) = matrix.map_or((1, n as u32), |m| (m.0, m.1));
            bands.push((matrix.map(|m| &m.2), start, n, rows, cols));
        }
        start += n;
    }
    let total = bands.iter().map(|b| b.3).sum::<u32>().max(1);
    let widest = bands.iter().map(|b| b.4).max().unwrap_or(1);

    let mut leds = vec![[0.5, 0.5]; c.leds];
    let mut top = 0;
    for (map, start, n, rows, cols) in bands {
        let at = |col: u32, row: u32| {
            [
                (col as f32 + 0.5) / cols as f32,
                ((top + row) as f32 + 0.5) / total as f32,
            ]
        };
        match map {
            Some(map) => {
                // LEDs the map leaves out sit in the middle of the zone.
                leds[start..start + n].fill([0.5, (top as f32 + rows as f32 / 2.0) / total as f32]);
                for (cell, &led) in map.iter().enumerate() {
                    if led != NO_LED && (led as usize) < n {
                        leds[start + led as usize] = at(cell as u32 % cols, cell as u32 / cols);
                    }
                }
            }
            None => {
                for i in 0..n {
                    leds[start + i] = at(i as u32, 0);
                }
            }
        }
        top += rows;
    }
    ([widest, total], leds)
}

/// One cell per device in a near-square arrangement, so default rects never overlap.
fn flow_rect(index: usize, count: usize, grid: [u32; 2]) -> [f32; 4] {
    let cols = ((count as f32).sqrt().ceil() as usize).max(1);
    let rows = count.div_ceil(cols).max(1);
    let (w, h) = (1.0 / cols as f32, 1.0 / rows as f32);
    let shape = (grid[1] as f32 / grid[0] as f32).clamp(0.15, 1.0);
    [
        ((index % cols) as f32 + 0.1) * w,
        ((index / cols) as f32 + 0.1) * h,
        0.8 * w,
        0.8 * h * shape,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    fn put16(b: &mut Vec<u8>, v: u16) {
        b.extend_from_slice(&v.to_le_bytes());
    }

    fn put32(b: &mut Vec<u8>, values: &[u32]) {
        for v in values {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn put_str(b: &mut Vec<u8>, s: &str) {
        put16(b, s.len() as u16 + 1);
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }

    fn zone_tail(b: &mut Vec<u8>, version: u32, segment: bool) {
        if version >= 4 {
            put16(b, segment as u16);
            if segment {
                put_str(b, "Half");
                put32(b, &[1, 0, 2]);
            }
        }
        if version >= 5 {
            put32(b, &[0]);
        }
    }

    /// A keyboard: 2x3 key matrix with one empty cell (5 LEDs), a 4 LED strip, a single LED.
    fn blob(version: u32) -> Vec<u8> {
        let mut b = Vec::new();
        put32(&mut b, &[0, 5]);
        put_str(&mut b, "Test Keyboard");
        if version >= 1 {
            put_str(&mut b, "Vendor");
        }
        for s in ["Description", "1.0", "SN1", "HID: test"] {
            put_str(&mut b, s);
        }

        put16(&mut b, 1);
        put32(&mut b, &[0]);
        put_str(&mut b, "Direct");
        put32(&mut b, &[0, 0x20, 0, 0]);
        if version >= 3 {
            put32(&mut b, &[0, 100]);
        }
        put32(&mut b, &[0, 2, 0]);
        if version >= 3 {
            put32(&mut b, &[100]);
        }
        put32(&mut b, &[0, 1]);
        put16(&mut b, 2);
        put32(&mut b, &[0x0000FF, 0x00FF00]);

        put16(&mut b, 3);
        put_str(&mut b, "Keys");
        put32(&mut b, &[2, 5, 5, 5]);
        put16(&mut b, 8 + 4 * 6);
        put32(&mut b, &[2, 3, 0, 1, 2, NO_LED, 3, 4]);
        zone_tail(&mut b, version, false);
        put_str(&mut b, "Strip");
        put32(&mut b, &[1, 4, 4, 4]);
        put16(&mut b, 0);
        zone_tail(&mut b, version, true);
        put_str(&mut b, "Logo");
        put32(&mut b, &[0, 1, 1, 1]);
        put16(&mut b, 0);
        zone_tail(&mut b, version, false);

        put16(&mut b, 10);
        for i in 0..10 {
            put_str(&mut b, &format!("LED {i}"));
            put32(&mut b, &[i]);
        }
        put16(&mut b, 10);
        put32(&mut b, &[0; 10]);
        if version >= 5 {
            put16(&mut b, 1);
            put_str(&mut b, "Alt name");
            put32(&mut b, &[0]);
        }
        let size = b.len() as u32;
        b[..4].copy_from_slice(&size.to_le_bytes());
        b
    }

    fn keyboard() -> Controller {
        Controller {
            kind: 5,
            name: "Test Keyboard".into(),
            zones: vec![
                Zone {
                    leds: 5,
                    matrix: Some((2, 3, vec![0, 1, 2, NO_LED, 3, 4])),
                },
                Zone {
                    leds: 4,
                    matrix: None,
                },
                Zone {
                    leds: 1,
                    matrix: None,
                },
            ],
            leds: 10,
        }
    }

    #[test]
    fn parses_every_supported_version() {
        for version in 0..=PROTOCOL_VERSION {
            assert_eq!(
                parse_controller(&blob(version), version),
                Ok(keyboard()),
                "v{version}"
            );
        }
    }

    #[test]
    fn layout_depends_on_version() {
        assert_ne!(parse_controller(&blob(5), 2), Ok(keyboard()));
        assert_ne!(parse_controller(&blob(0), 1), Ok(keyboard()));
    }

    #[test]
    fn truncated_data_is_an_error() {
        let full = blob(4);
        for len in 0..full.len() {
            assert!(parse_controller(&full[..len], 4).is_err(), "{len} bytes");
        }
    }

    #[test]
    fn odd_zones_still_give_one_position_per_led() {
        let c = Controller {
            kind: 99,
            name: String::new(),
            zones: vec![
                // the map points past the zone and leaves LEDs 0 and 2 out
                Zone {
                    leds: 3,
                    matrix: Some((1, 2, vec![7, 1])),
                },
                Zone {
                    leds: 0,
                    matrix: None,
                },
                // claims more LEDs than the controller has colours
                Zone {
                    leds: u32::MAX,
                    matrix: Some((0, 5, Vec::new())),
                },
            ],
            leds: 5,
        };
        let d = device(0, 1, &c);
        assert_eq!(
            (d.name.as_str(), d.kind),
            ("OpenRGB device 0", DeviceKind::Other)
        );
        assert_eq!(d.grid, [2, 2]);
        assert_eq!(
            d.leds,
            [
                [0.5, 0.25],
                [0.75, 0.25],
                [0.5, 0.25],
                [0.25, 0.75],
                [0.75, 0.75]
            ]
        );
    }

    #[test]
    fn maps_zones_onto_the_device() {
        let d = device(1, 2, &keyboard());
        assert_eq!(
            (d.id.as_str(), d.name.as_str(), d.kind),
            ("openrgb:1", "Test Keyboard", DeviceKind::Keyboard)
        );
        assert_eq!(d.grid, [4, 4]);
        assert_eq!(d.leds.len(), 10);
        // matrix: first key top left, LED 3 in the middle of the second row
        assert_eq!(d.leds[0], [0.5 / 3.0, 0.125]);
        assert_eq!(d.leds[3], [0.5, 0.375]);
        // strip on the third row, single LED centred on the fourth
        assert_eq!(d.leds[5], [0.125, 0.625]);
        assert_eq!(d.leds[8], [0.875, 0.625]);
        assert_eq!(d.leds[9], [0.5, 0.875]);
    }

    #[test]
    fn default_rects_do_not_overlap() {
        let rects: Vec<_> = (0..5).map(|i| flow_rect(i, 5, [4, 4])).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a[0] >= 0.0 && a[1] >= 0.0 && a[0] + a[2] <= 1.0 && a[1] + a[3] <= 1.0);
            for b in &rects[i + 1..] {
                let apart = a[0] + a[2] <= b[0]
                    || b[0] + b[2] <= a[0]
                    || a[1] + a[3] <= b[1]
                    || b[1] + b[3] <= a[1];
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    /// What the mock server does once it has sent the last controller.
    #[derive(Clone, Copy, PartialEq)]
    enum Then {
        Serve,
        HangUp,
        Notify,
        /// The list changes on every pass, not only the first.
        Churn,
    }

    /// Plays the server on a free port and returns everything the client sent.
    /// Version 0 ignores the version request, as servers that old do.
    /// The device list changes during the first pass, so the client has to read it twice.
    fn mock(version: u32, blobs: Vec<Vec<u8>>, then: Then) -> (u16, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut passes = 0;
            loop {
                let mut header = [0u8; 16];
                if s.read_exact(&mut header).is_err() {
                    return seen;
                }
                let word = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
                let mut payload = vec![0u8; word(12) as usize];
                s.read_exact(&mut payload).unwrap();
                seen.extend_from_slice(&header);
                seen.extend_from_slice(&payload);
                let reply = match word(8) {
                    REQUEST_PROTOCOL_VERSION if version > 0 => version.to_le_bytes().to_vec(),
                    REQUEST_CONTROLLER_COUNT => {
                        passes += 1;
                        if passes == 1 || then == Then::Churn {
                            s.write_all(&packet(0, 100, &[])).unwrap();
                        }
                        // the last controller only shows up after the first pass
                        let count = blobs.len() - usize::from(passes == 1);
                        (count as u32).to_le_bytes().to_vec()
                    }
                    REQUEST_CONTROLLER_DATA => blobs[word(4) as usize].clone(),
                    _ => continue,
                };
                s.write_all(&packet(word(4), word(8), &reply)).unwrap();
                let last = word(4) as usize + 1 == blobs.len();
                if word(8) == REQUEST_CONTROLLER_DATA && last && passes == 2 {
                    match then {
                        Then::Serve | Then::Churn => {}
                        Then::HangUp => return seen,
                        Then::Notify => s.write_all(&packet(0, 100, &[])).unwrap(),
                    }
                }
            }
        });
        (port, server)
    }

    #[test]
    fn connect_and_push_exact_bytes() {
        let (port, server) = mock(4, vec![blob(4), blob(4)], Then::Serve);
        let mut backend = OpenRgbBackend::connect("127.0.0.1", port).unwrap();
        let devices = backend.devices();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1].id, "openrgb:1");
        assert_eq!(devices[1].leds.len(), 10);
        assert!(devices[0].rect[0] + devices[0].rect[2] <= devices[1].rect[0]);

        let first: Vec<Color> = (1..=10).map(|i| Color(i, 0x80, 0xFF - i)).collect();
        let second = vec![Color(9, 8, 7); 10];
        backend.push("openrgb:1", &first).unwrap();
        backend.push("openrgb:1", &first).unwrap();
        backend.push("openrgb:1", &second).unwrap();
        assert!(backend.push("openrgb:1", &second[..3]).is_err());
        assert!(backend.push("openrgb:2", &second).is_err());
        assert!(backend.push("virtual:board", &second).is_err());
        drop(backend);

        let mut expected = Vec::new();
        // version request (asks for 5), client name
        expected.extend_from_slice(b"ORGB\0\0\0\0\x28\0\0\0\x04\0\0\0\x05\0\0\0");
        expected.extend_from_slice(b"ORGB\0\0\0\0\x32\0\0\0\x07\0\0\0FoxRGB\0");
        // first pass: controller count, data for 0 at the negotiated version 4, list changes
        expected.extend_from_slice(b"ORGB\0\0\0\0\0\0\0\0\0\0\0\0");
        expected.extend_from_slice(b"ORGB\0\0\0\0\x01\0\0\0\x04\0\0\0\x04\0\0\0");
        // second pass: controller count, data for 0 and 1
        expected.extend_from_slice(b"ORGB\0\0\0\0\0\0\0\0\0\0\0\0");
        expected.extend_from_slice(b"ORGB\0\0\0\0\x01\0\0\0\x04\0\0\0\x04\0\0\0");
        expected.extend_from_slice(b"ORGB\x01\0\0\0\x01\0\0\0\x04\0\0\0\x04\0\0\0");
        // custom mode once, then two frames of 46 bytes: size, LED count, r g b 0
        expected.extend_from_slice(b"ORGB\x01\0\0\0\x4c\x04\0\0\0\0\0\0");
        expected.extend_from_slice(b"ORGB\x01\0\0\0\x1a\x04\0\0\x2e\0\0\0\x2e\0\0\0\x0a\0");
        expected.extend((1..=10u8).flat_map(|i| [i, 0x80, 0xFF - i, 0]));
        expected.extend_from_slice(b"ORGB\x01\0\0\0\x1a\x04\0\0\x2e\0\0\0\x2e\0\0\0\x0a\0");
        expected.extend([9, 8, 7, 0].repeat(10));
        assert_eq!(server.join().unwrap(), expected);
    }

    #[test]
    fn connect_gives_up_when_the_list_keeps_changing() {
        let (port, server) = mock(5, vec![blob(5)], Then::Churn);
        let error = OpenRgbBackend::connect("127.0.0.1", port).err().unwrap();
        assert!(error.contains("kept changing"), "{error}");
        let seen = server.join().unwrap();
        let count_request = b"ORGB\0\0\0\0\0\0\0\0\0\0\0\0";
        let asked = seen.windows(16).filter(|w| w == count_request).count();
        assert_eq!(asked, LIST_ATTEMPTS);
    }

    #[test]
    fn caps_the_version_at_what_we_implement() {
        let (port, server) = mock(9, vec![blob(5)], Then::Serve);
        let backend = OpenRgbBackend::connect("127.0.0.1", port).unwrap();
        assert_eq!(backend.devices()[0].leds.len(), 10);
        drop(backend);
        let seen = server.join().unwrap();
        assert!(seen.ends_with(b"ORGB\0\0\0\0\x01\0\0\0\x04\0\0\0\x05\0\0\0"));
    }

    #[test]
    fn falls_back_to_version_0_when_the_server_stays_silent() {
        let (port, server) = mock(0, vec![blob(0)], Then::Serve);
        let backend = OpenRgbBackend::connect("127.0.0.1", port).unwrap();
        assert_eq!(backend.devices()[0].name, "Test Keyboard");
        drop(backend);
        assert!(
            server
                .join()
                .unwrap()
                .ends_with(b"ORGB\0\0\0\0\x01\0\0\0\0\0\0\0")
        );
    }

    #[test]
    fn push_keeps_failing_after_the_server_goes_away() {
        let (port, server) = mock(5, vec![blob(5)], Then::HangUp);
        let mut backend = OpenRgbBackend::connect("127.0.0.1", port).unwrap();
        server.join().unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut shade = 0u8;
        while backend.push("openrgb:0", &[Color(shade, 0, 0); 10]).is_ok() {
            assert!(
                Instant::now() < deadline,
                "writes to a closed socket kept succeeding"
            );
            shade = shade.wrapping_add(1);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            backend
                .push("openrgb:0", &[Color(shade, 0, 0); 10])
                .is_err()
        );
        assert!(backend.push("openrgb:0", &[Color::BLACK; 10]).is_err());
    }

    #[test]
    fn push_fails_once_the_device_list_changes() {
        let (port, server) = mock(5, vec![blob(5)], Then::Notify);
        let mut backend = OpenRgbBackend::connect("127.0.0.1", port).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            match backend.push("openrgb:0", &[Color::BLACK; 10]) {
                Err(e) => break e,
                Ok(()) => assert!(Instant::now() < deadline, "the notification never arrived"),
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(error.contains("device list"), "{error}");
        assert!(backend.push("openrgb:0", &[Color(1, 2, 3); 10]).is_err());
        drop(backend);
        server.join().unwrap();
    }

    #[test]
    fn connect_fails_fast_without_a_server() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let started = Instant::now();
        assert!(OpenRgbBackend::connect("127.0.0.1", port).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
