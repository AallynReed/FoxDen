//! Ported from OpenRGB Controllers/PhilipsHueController (GPL-2.0-or-later), with its hueplusplus REST calls.

use std::thread;
use std::time::{Duration, Instant};

use foxden_core::{Color, DeviceKind};
use serde_json::Value;

use crate::net::{self, http};
use crate::{Driver, Found, HueBridge, Light, local, parse, quote, text};

const PORT: u16 = 80;
/// hueplusplus Config: requestUsernameDelay and requestUsernameAttemptInterval.
const PAIR_FOR: Duration = Duration::from_secs(35);
const PAIR_EVERY: Duration = Duration::from_secs(1);
const DEVICE_TYPE: &str = "FoxDen#FoxDen";

#[derive(Clone, Copy, Debug, PartialEq)]
struct Xy {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Gamut {
    red: Xy,
    green: Xy,
    blue: Xy,
}

const fn gamut(r: (f32, f32), g: (f32, f32), b: (f32, f32)) -> Gamut {
    Gamut {
        red: Xy { x: r.0, y: r.1 },
        green: Xy { x: g.0, y: g.1 },
        blue: Xy { x: b.0, y: b.1 },
    }
}

const GAMUT_A: Gamut = gamut((0.704, 0.296), (0.2151, 0.7106), (0.138, 0.08));
const GAMUT_B: Gamut = gamut((0.675, 0.322), (0.409, 0.518), (0.167, 0.04));
const GAMUT_C: Gamut = gamut((0.692, 0.308), (0.17, 0.7), (0.153, 0.048));
const MAX_GAMUT: Gamut = gamut((1.0, 0.0), (0.0, 1.0), (0.0, 0.0));

fn sign(p0: Xy, p1: Xy, p2: Xy) -> f32 {
    (p0.x - p2.x) * (p1.y - p2.y) - (p1.x - p2.x) * (p0.y - p2.y)
}

fn right_of(xy: Xy, p1: Xy, p2: Xy) -> bool {
    sign(xy, p1, p2) < 0.0
}

fn project(xy: Xy, p1: Xy, p2: Xy) -> Xy {
    let ab_x = p2.x - p1.x;
    let ab_y = p2.y - p1.y;
    let len = ab_x * ab_x + ab_y * ab_y;
    let dot = (xy.x - p1.x) * ab_x + (xy.y - p1.y) * ab_y;
    Xy {
        x: p1.x + ab_x * dot / len,
        y: p1.y + ab_y * dot / len,
    }
}

impl Gamut {
    fn contains(&self, xy: Xy) -> bool {
        !right_of(xy, self.red, self.green)
            && !right_of(xy, self.green, self.blue)
            && !right_of(xy, self.blue, self.red)
    }

    fn corrected(&self, xy: Xy) -> Xy {
        let (r, g, b) = (self.red, self.green, self.blue);
        if right_of(xy, r, g) {
            if right_of(xy, g, b) {
                g
            } else if right_of(xy, b, r) {
                r
            } else {
                project(xy, r, g)
            }
        } else if right_of(xy, g, b) {
            if right_of(xy, b, r) {
                b
            } else {
                project(xy, g, b)
            }
        } else if right_of(xy, b, r) {
            project(xy, b, r)
        } else {
            xy
        }
    }
}

fn linear(c: f32) -> f32 {
    if c > 0.04045 {
        ((c + 0.055) / (1.0 + 0.055)).powf(2.4)
    } else {
        c / 12.92
    }
}

/// hueplusplus `RGB::toXY(gamut)`: the colour point and the brightest channel.
fn to_xy(c: Color, gamut: &Gamut) -> (Xy, f32) {
    if c == Color::BLACK {
        return (
            Xy {
                x: 0.322_726_73,
                y: 0.329_022_9,
            },
            0.0,
        );
    }
    let (red, green, blue) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (rc, gc, bc) = (linear(red), linear(green), linear(blue));
    let x_ = rc * 0.664511 + gc * 0.154324 + bc * 0.162028;
    let y_ = rc * 0.283881 + gc * 0.668433 + bc * 0.047685;
    let z_ = rc * 0.000088 + gc * 0.072310 + bc * 0.986039;
    let xy = Xy {
        x: x_ / (x_ + y_ + z_),
        y: y_ / (x_ + y_ + z_),
    };
    let xy = if gamut.contains(xy) {
        xy
    } else {
        gamut.corrected(xy)
    };
    (xy, red.max(green).max(blue))
}

/// A float as nlohmann's `dump()` writes it.
fn num(v: f32) -> String {
    let v = v as f64;
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// What the bridge last reported, the way hueplusplus caches it to trim requests.
#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    on: Option<bool>,
    bri: Option<i64>,
    xy: Option<(f32, f32)>,
    colormode: String,
}

impl State {
    fn read(state: &Value) -> Self {
        State {
            on: state["on"].as_bool(),
            bri: state["bri"].as_i64(),
            xy: state["xy"].as_array().filter(|a| a.len() == 2).map(|a| {
                (
                    a[0].as_f64().unwrap_or(0.0) as f32,
                    a[1].as_f64().unwrap_or(0.0) as f32,
                )
            }),
            colormode: text(&state["colormode"]),
        }
    }
}

/// The request hueplusplus `setColorRGB(rgb, 0)` commits, after `commit()` and `trimRequest()`.
#[derive(Clone, Debug, PartialEq)]
struct Request {
    bri: Option<i64>,
    on: Option<bool>,
    xy: Option<(f32, f32)>,
}

impl Request {
    fn new(c: Color, gamut: &Gamut, state: &State) -> Self {
        let (xy, brightness) = to_xy(c, gamut);
        let bri = ((brightness * 254.0).round() as i64).clamp(0, 254);
        let xy = (xy.x.clamp(0.0, 1.0), xy.y.clamp(0.0, 1.0));
        let on = if !state.on.unwrap_or(false) && bri != 0 {
            Some(true)
        } else if bri == 0 && state.on.unwrap_or(true) {
            Some(false)
        } else {
            None
        };
        let same = |a: f32, b: f32| (a - b).abs() <= 1e-4;
        let xy_kept = !(state.colormode == "xy"
            && state
                .xy
                .is_some_and(|(x, y)| same(x, xy.0) && same(y, xy.1)));
        Request {
            bri: (state.bri != Some(bri)).then_some(bri),
            on: on.filter(|on| state.on != Some(*on)),
            xy: xy_kept.then_some(xy),
        }
    }

    /// The PUT body, or nothing when only `transitiontime` would be left.
    fn body(&self) -> Option<String> {
        if self.bri.is_none() && self.on.is_none() && self.xy.is_none() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(bri) = self.bri {
            parts.push(format!("\"bri\":{bri}"));
        }
        if let Some(on) = self.on {
            parts.push(format!("\"on\":{on}"));
        }
        parts.push("\"transitiontime\":0".into());
        if let Some((x, y)) = self.xy {
            parts.push(format!("\"xy\":[{},{}]", num(x), num(y)));
        }
        Some(format!("{{{}}}", parts.join(",")))
    }

    fn apply(&self, state: &mut State) {
        if let Some(bri) = self.bri {
            state.bri = Some(bri);
        }
        if let Some(on) = self.on {
            state.on = Some(on);
        }
        if let Some(xy) = self.xy {
            state.xy = Some(xy);
        }
    }
}

/// hueplusplus `LightFactory::getColorType` and `Light::getColorGamut`.
fn light_gamut(light: &Value) -> Gamut {
    let control = &light["capabilities"]["control"];
    let kind = match control["colorgamuttype"].as_str() {
        Some(t) => t.to_string(),
        None => {
            let model = text(&light["modelid"]);
            let a = [
                "LST001", "LLC005", "LLC006", "LLC007", "LLC010", "LLC011", "LLC012", "LLC013",
                "LLC014",
            ];
            let b = ["LCT001", "LCT002", "LCT003", "LCT007", "LLM001"];
            let c = [
                "LCT010", "LCT011", "LCT012", "LCT014", "LCT015", "LCT016", "LLC020", "LST002",
                "LCA003", "LCB001",
            ];
            if a.contains(&model.as_str()) {
                "A".into()
            } else if b.contains(&model.as_str()) {
                "B".into()
            } else if c.contains(&model.as_str()) {
                "C".into()
            } else {
                return MAX_GAMUT;
            }
        }
    };
    match kind.as_str() {
        "A" => GAMUT_A,
        "B" => GAMUT_B,
        "C" => GAMUT_C,
        _ => {
            let p = |i: usize| {
                let pt = &control["colorgamut"][i];
                Some((pt[0].as_f64()? as f32, pt[1].as_f64()? as f32))
            };
            match (p(0), p(1), p(2)) {
                (Some(r), Some(g), Some(b)) => gamut(r, g, b),
                _ => MAX_GAMUT,
            }
        }
    }
}

/// `hasColorControl()`: lights hueplusplus gives a colour strategy.
fn has_color(light: &Value) -> bool {
    matches!(
        text(&light["type"]).to_lowercase().as_str(),
        "color light" | "extended color light"
    )
}

struct HueLight {
    ip: String,
    path: String,
    gamut: Gamut,
    state: State,
    dark: bool,
}

impl Driver for HueLight {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        // PhilipsHueController::SetColor: black goes out once.
        let black = c == Color::BLACK;
        if black && self.dark {
            return Ok(());
        }
        self.dark = black;
        let request = Request::new(c, &self.gamut, &self.state);
        let Some(body) = request.body() else {
            return Ok(());
        };
        let (status, _) = http("PUT", &self.ip, PORT, &self.path, Some(&body))?;
        if status / 100 != 2 {
            return Err(format!("bridge answered {status}"));
        }
        request.apply(&mut self.state);
        Ok(())
    }
}

/// Every colour light on a paired bridge, each driven over REST.
pub fn open(bridge: &HueBridge) -> Vec<Light> {
    if bridge.username.is_empty() || local(&bridge.ip).is_err() {
        return Vec::new();
    }
    let Ok((200, body)) = http(
        "GET",
        &bridge.ip,
        PORT,
        &format!("/api/{}/lights", bridge.username),
        None,
    ) else {
        return Vec::new();
    };
    let lights = parse(&body);
    let Some(lights) = lights.as_object() else {
        return Vec::new();
    };
    let mut ids: Vec<&String> = lights.keys().collect();
    ids.sort_by_key(|id| id.parse::<u32>().unwrap_or(u32::MAX));
    ids.into_iter()
        .filter(|id| has_color(&lights[*id]))
        .map(|id| {
            let light = &lights[id];
            let unique = text(&light["uniqueid"]);
            let key = if unique.is_empty() {
                format!("{}-{id}", bridge.ip)
            } else {
                unique
            };
            let name = text(&light["name"]);
            let driver = HueLight {
                ip: bridge.ip.clone(),
                path: format!("/api/{}/lights/{id}/state", bridge.username),
                gamut: light_gamut(light),
                state: State::read(&light["state"]),
                dark: false,
            };
            Light::new(
                format!("net:hue:{key}"),
                if name.is_empty() {
                    "Philips Hue light".into()
                } else {
                    format!("Hue {name}")
                },
                DeviceKind::Other,
                1,
                Box::new(driver),
            )
        })
        .collect()
}

pub fn pair_body() -> String {
    format!(
        "{{\"devicetype\":{},\"generateclientkey\":true}}",
        quote(DEVICE_TYPE)
    )
}

#[derive(Debug, PartialEq)]
enum Pairing {
    Done { username: String, clientkey: String },
    Waiting,
    Failed(String),
}

/// `Bridge::requestUsername`: error 101 means the link button has not been pressed yet.
fn pairing(reply: &str) -> Pairing {
    let reply = parse(reply);
    let first = &reply[0];
    let user = &first["success"]["username"];
    if let Some(user) = user.as_str() {
        return Pairing::Done {
            username: user.into(),
            clientkey: text(&first["success"]["clientkey"]),
        };
    }
    match first["error"]["type"].as_i64() {
        Some(101) | None => Pairing::Waiting,
        Some(_) => Pairing::Failed(text(&first["error"]["description"])),
    }
}

/// Asks the bridge for a user while its link button is pressed, for up to 35 seconds.
pub fn pair(ip: &str) -> Result<HueBridge, String> {
    local(ip)?;
    let start = Instant::now();
    loop {
        thread::sleep(PAIR_EVERY);
        if let Ok((_, reply)) = http("POST", ip, PORT, "/api", Some(&pair_body())) {
            match pairing(&reply) {
                Pairing::Done {
                    username,
                    clientkey,
                } => {
                    return Ok(HueBridge {
                        ip: ip.into(),
                        username,
                        clientkey,
                    });
                }
                Pairing::Failed(e) => return Err(e),
                Pairing::Waiting => {}
            }
        }
        if start.elapsed() >= PAIR_FOR {
            return Err("the link button on the bridge was not pressed".into());
        }
    }
}

pub const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 5\r\nST: ssdp:all\r\n\r\n";

/// `UPnP::getDevices` entries whose SERVER names a Hue bridge: the host of each LOCATION.
fn bridges(replies: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in replies {
        let field = |name: &str| {
            let start = s.find(name)? + name.len() + 1;
            let end = s[start..].find("\r\n").map_or(s.len(), |e| start + e);
            Some(s[start..end].to_string())
        };
        let (Some(location), Some(server)) = (field("LOCATION:"), field("SERVER:")) else {
            continue;
        };
        if !server.contains("IpBridge") {
            continue;
        }
        let Some(start) = location.find("//").map(|i| i + 2) else {
            continue;
        };
        let host = &location[start..];
        let host = host
            .split([':', '/'])
            .next()
            .unwrap_or_default()
            .to_string();
        if !out.contains(&host) {
            out.push(host);
        }
    }
    out
}

/// `BridgeFinder::parseDescription`: the serial number, which is the bridge's MAC.
fn description_mac(description: &str) -> Option<String> {
    if !description.contains("<modelName>Philips hue bridge") {
        return None;
    }
    let begin = description.find("<serialNumber>")? + "<serialNumber>".len();
    let end = description[begin..].find("</serialNumber>")? + begin;
    Some(description[begin..end].to_string())
}

/// `BridgeFinder::findBridges` over SSDP. hueplusplus's cloud lookup is not used.
pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket
        .send_to(SEARCH.as_bytes(), "239.255.255.250:1900")
        .is_err()
    {
        return Vec::new();
    }
    let replies: Vec<String> = net::collect(&socket, Duration::from_secs(3))
        .into_iter()
        .map(|(_, r)| String::from_utf8_lossy(&r).to_string())
        .collect();
    bridges(&replies)
        .into_iter()
        .filter(|ip| local(ip).is_ok())
        .filter_map(|ip| {
            let (_, desc) = http("GET", &ip, PORT, "/description.xml", None).ok()?;
            description_mac(&desc)?;
            Some(Found {
                family: "hue",
                ip,
                port: PORT,
                name: "Philips Hue bridge".into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_xy() -> State {
        State {
            on: Some(true),
            bri: Some(254),
            xy: Some((0.5, 0.5)),
            colormode: "xy".into(),
        }
    }

    #[test]
    fn black_turns_the_light_off_at_hueplusplus_white_point() {
        let body = Request::new(Color::BLACK, &GAMUT_C, &on_xy())
            .body()
            .unwrap();
        // XYBrightness {0.32272673f, 0.32902291f}, 0.f widened to double
        assert_eq!(
            body,
            r#"{"bri":0,"on":false,"transitiontime":0,"xy":[0.3227267265319824,0.32902291417121887]}"#
        );
    }

    #[test]
    fn red_in_the_full_gamut() {
        // X = 0.664511, Y = 0.283881, Z = 0.000088 in float: x = X/(X+Y+Z), y = Y/(X+Y+Z)
        let off = State {
            on: Some(false),
            ..Default::default()
        };
        let body = Request::new(Color(255, 0, 0), &MAX_GAMUT, &off)
            .body()
            .unwrap();
        assert_eq!(
            body,
            r#"{"bri":254,"on":true,"transitiontime":0,"xy":[0.7006062269210815,0.29930099844932556]}"#
        );
    }

    #[test]
    fn unchanged_values_are_trimmed() {
        let mut state = on_xy();
        let request = Request::new(Color(255, 0, 0), &MAX_GAMUT, &state);
        request.apply(&mut state);
        assert_eq!(
            Request::new(Color(255, 0, 0), &MAX_GAMUT, &state).body(),
            None
        );
        // colormode "ct" keeps xy in every request
        state.colormode = "ct".into();
        assert_eq!(
            Request::new(Color(255, 0, 0), &MAX_GAMUT, &state)
                .body()
                .unwrap(),
            r#"{"transitiontime":0,"xy":[0.7006062269210815,0.29930099844932556]}"#
        );
    }

    #[test]
    fn points_outside_a_gamut_move_onto_it() {
        assert_eq!(
            MAX_GAMUT.corrected(Xy { x: 1.0, y: 1.0 }),
            Xy { x: 0.5, y: 0.5 }
        );
        assert_eq!(
            MAX_GAMUT.corrected(Xy { x: -1.0, y: -1.0 }),
            Xy { x: 0.0, y: 0.0 }
        );
        assert_eq!(
            MAX_GAMUT.corrected(Xy { x: 2.0, y: -0.5 }),
            Xy { x: 1.0, y: 0.0 }
        );
        assert!(GAMUT_B.contains(Xy { x: 0.4, y: 0.3 }));
    }

    #[test]
    fn gamut_comes_from_capabilities_or_the_model() {
        let typed: Value =
            serde_json::from_str(r#"{"capabilities":{"control":{"colorgamuttype":"A"}}}"#).unwrap();
        assert_eq!(light_gamut(&typed), GAMUT_A);
        let other: Value = serde_json::from_str(
            r#"{"capabilities":{"control":{"colorgamuttype":"other","colorgamut":[[0.6,0.3],[0.2,0.7],[0.1,0.1]]}}}"#,
        )
        .unwrap();
        assert_eq!(
            light_gamut(&other),
            gamut((0.6, 0.3), (0.2, 0.7), (0.1, 0.1))
        );
        let old: Value = serde_json::from_str(r#"{"modelid":"LCT015"}"#).unwrap();
        assert_eq!(light_gamut(&old), GAMUT_C);
        let unknown: Value = serde_json::from_str(r#"{"modelid":"XYZ"}"#).unwrap();
        assert_eq!(light_gamut(&unknown), MAX_GAMUT);
        assert!(has_color(
            &serde_json::from_str(r#"{"type":"Extended color light"}"#).unwrap()
        ));
        assert!(!has_color(
            &serde_json::from_str(r#"{"type":"Dimmable light"}"#).unwrap()
        ));
    }

    #[test]
    fn pairing_waits_for_the_link_button() {
        assert_eq!(
            pair_body(),
            r#"{"devicetype":"FoxDen#FoxDen","generateclientkey":true}"#
        );
        assert_eq!(
            pairing(
                r#"[{"error":{"type":101,"address":"","description":"link button not pressed"}}]"#
            ),
            Pairing::Waiting
        );
        assert_eq!(
            pairing(r#"[{"success":{"username":"abc","clientkey":"0F"}}]"#),
            Pairing::Done {
                username: "abc".into(),
                clientkey: "0F".into()
            }
        );
        assert_eq!(
            pairing(r#"[{"error":{"type":7,"description":"invalid value"}}]"#),
            Pairing::Failed("invalid value".into())
        );
    }

    #[test]
    fn ssdp_replies_name_the_bridges() {
        let replies = vec![
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.40:80/description.xml\r\nSERVER: Hue/1.0 UPnP/1.0 IpBridge/1.48.0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.40:80/description.xml\r\nSERVER: Hue/1.0 UPnP/1.0 IpBridge/1.48.0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.9:49152/x.xml\r\nSERVER: Linux UPnP/1.0 Router\r\n\r\n".into(),
        ];
        assert_eq!(bridges(&replies), vec!["192.168.1.40"]);
        assert_eq!(
            description_mac(
                "<modelName>Philips hue bridge 2015</modelName><serialNumber>001788abcdef</serialNumber>"
            ),
            Some("001788abcdef".into())
        );
        assert_eq!(description_mac("<modelName>Other</modelName>"), None);
        assert_eq!(
            SEARCH,
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 5\r\nST: ssdp:all\r\n\r\n"
        );
    }
}
