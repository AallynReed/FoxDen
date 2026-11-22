// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::{Duration, Instant};

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "royuan";
const VID: u16 = 0x3151;

const SET_LED_PARAMS: u8 = 0x07;
const GET_LED_PARAMS: u8 = 0x87;
const SET_DIRECT_COLOR: u8 = 0x0E;
const EFFECT_STATIC: u8 = 0x01;
const PRE_SET_MS: u64 = 500;
const POST_SET_MS: u64 = 1500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Checksum {
    NextPowerOfTwo,
    OnesComplement,
}

/// RoyuanKeyboardProfile.
#[derive(Clone, Copy, Debug)]
struct Profile {
    checksum: Checksum,
    speed_max: u8,
    speed_default: u8,
    brightness_max: u8,
    brightness_default: u8,
    option_default: u8,
    dazzle_default: u8,
    reverse_speed: bool,
    solid_option_default: u8,
    solid_overrides_static: bool,
}

const LEGACY: Profile = Profile {
    checksum: Checksum::NextPowerOfTwo,
    speed_max: 0x05,
    speed_default: 0x04,
    brightness_max: 0x04,
    brightness_default: 0x04,
    option_default: 0x00,
    dazzle_default: 0x07,
    reverse_speed: false,
    solid_option_default: 0x00,
    solid_overrides_static: false,
};

const AKKO: Profile = Profile {
    checksum: Checksum::OnesComplement,
    speed_max: 0x04,
    speed_default: 0x00,
    brightness_max: 0x04,
    brightness_default: 0x03,
    option_default: 0x08,
    dazzle_default: 0x00,
    reverse_speed: true,
    solid_option_default: 0x00,
    solid_overrides_static: false,
};

const GK68HE: Profile = Profile {
    checksum: Checksum::OnesComplement,
    speed_max: 0x04,
    speed_default: 0x02,
    brightness_max: 0x04,
    brightness_default: 0x04,
    option_default: 0x08,
    dazzle_default: 0x00,
    reverse_speed: true,
    solid_option_default: 0x07,
    solid_overrides_static: true,
};

const GK68HE_DIRECT: u8 = 0x15;
const GK68HE_USER_COLOR: u8 = 0x07;
const AKKO_COLOR_SINGLE: u8 = 0x08;

/// CalculateChecksum.
fn checksum(kind: Checksum, data: &[u8]) -> u8 {
    let sum: u32 = data.iter().map(|&b| b as u32).sum();
    match kind {
        Checksum::OnesComplement => 0xFF - (sum & 0xFF) as u8,
        Checksum::NextPowerOfTwo => {
            let mut p = 1u32;
            while p < sum {
                p <<= 1;
            }
            (p.wrapping_sub(sum).wrapping_sub(1) & 0xFF) as u8
        }
    }
}

/// The controller's current_* fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Royuan {
    profile_id: u8,
    mode: u8,
    speed: u8,
    brightness: u8,
    dazzle: u8,
    option: u8,
    rgb: [u8; 3],
}

fn profile(id: u8) -> Profile {
    match id {
        0 => LEGACY,
        1 => AKKO,
        _ => GK68HE,
    }
}

impl Royuan {
    fn new(profile_id: u8) -> Royuan {
        let p = profile(profile_id);
        Royuan {
            profile_id,
            mode: 0x01,
            speed: p.speed_default,
            brightness: p.brightness_default,
            dazzle: p.dazzle_default,
            option: p.option_default,
            rgb: [0; 3],
        }
    }

    fn p(&self) -> Profile {
        profile(self.profile_id)
    }

    /// ReadState: GET_LEDPARAM, then up to five feature reads 80 ms apart.
    fn read_state(&mut self, io: &mut dyn Io) -> Result<bool, String> {
        let p = self.p();
        let mut req = [0u8; 65];
        req[1] = GET_LED_PARAMS;
        req[8] = checksum(p.checksum, &req[1..8]);
        io.feature(&req)?;
        for _ in 0..5 {
            io.sleep(80);
            let mut r = [0u8; 65];
            let n = io.get_feature(&mut r)?;
            if n > 8 && r[1] == GET_LED_PARAMS {
                self.mode = r[2];
                self.speed = if r[3] <= p.speed_max {
                    p.speed_max - r[3]
                } else {
                    p.speed_default
                };
                self.brightness = r[4].min(p.brightness_max);
                self.option = r[5];
                self.rgb = [r[6], r[7], r[8]];
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The SET_LEDPARAM report SendUpdate builds.
    fn update(&self) -> [u8; 65] {
        let p = self.p();
        let mut b = [0u8; 65];
        b[1] = SET_LED_PARAMS;
        b[2] = self.mode;
        b[3] = if p.reverse_speed {
            p.speed_max - self.speed.min(p.speed_max)
        } else {
            self.speed.min(p.speed_max)
        };
        b[4] = self.brightness.min(p.brightness_max);
        b[5] = if p.solid_overrides_static && self.mode == EFFECT_STATIC {
            p.solid_option_default
        } else {
            self.option | self.dazzle
        };
        b[6..9].copy_from_slice(&self.rgb);
        b[9] = checksum(p.checksum, &b[1..9]);
        b
    }

    /// SendDirectColor.
    fn direct(&self) -> [u8; 65] {
        let mut b = [0u8; 65];
        b[1] = SET_DIRECT_COLOR;
        b[2..5].copy_from_slice(&self.rgb);
        b[8] = checksum(self.p().checksum, &b[1..8]);
        b
    }

    /// SetMode.
    fn set_mode(&mut self, mode: u8, speed: u8, brightness: u8) {
        let p = self.p();
        self.mode = mode;
        self.speed = speed.min(p.speed_max);
        self.brightness = brightness.min(p.brightness_max);
    }
}

/// Epomaker TH80 Pro / Attack Shark K86: "Direct" is Always On with dazzle off.
struct Legacy(Royuan);

impl Proto for Legacy {
    /// DeviceUpdateMode for Direct: SetOption(default), SetDazzle(false), SetMode.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.0.option = 0x00;
        self.0.dazzle = 0x07;
        self.0.set_mode(0x01, 0, 0x04);
        io.feature(&self.0.update())
    }

    /// DeviceUpdateLEDs: SetDazzle(false), SetColor.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.0.rgb = rgb(colors[0]);
        io.feature(&self.0.update())
    }
}

/// Akko B series: Static with a single colour, every SET paced by the write thread.
struct Akko {
    state: Royuan,
    last: Option<Instant>,
}

impl Proto for Akko {
    /// DeviceUpdateMode for Static then DeviceUpdateLEDs, merged by the write thread into one SET.
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        let brightness = if self.state.mode == EFFECT_STATIC {
            self.state.brightness
        } else {
            0x03
        };
        self.state.set_mode(EFFECT_STATIC, 0, brightness);
        self.state.option = AKKO_COLOR_SINGLE;
        Ok(())
    }

    /// WriteThread: the SET goes out 500 ms after it was asked for and 1.5 s after the last one.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.state.rgb = rgb(colors[0]);
        let since = self.last.map(|t| t.elapsed()).unwrap_or(Duration::MAX);
        let wait = Duration::from_millis(POST_SET_MS).saturating_sub(since);
        io.sleep(wait.max(Duration::from_millis(PRE_SET_MS)).as_millis() as u64);
        let r = io.feature(&self.state.update());
        self.last = Some(Instant::now());
        r
    }

    /// The destructor waits out the 1.5 s after the last SET.
    fn stop(&mut self, io: &mut dyn Io) {
        if let Some(t) = self.last {
            let left = Duration::from_millis(POST_SET_MS).saturating_sub(t.elapsed());
            io.sleep(left.as_millis() as u64);
        }
    }
}

/// Skyloong GK68HE Pro: Direct is one colour sent with SET_DIRECT_COLOR.
struct Gk68he(Royuan);

impl Proto for Gk68he {
    /// DeviceUpdateMode for Direct: SetOption(user colour), SetMode (SendUpdate and SendDirectColor).
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.0.option = GK68HE_USER_COLOR;
        self.0.set_mode(GK68HE_DIRECT, 0, 0x04);
        io.feature(&self.0.update())?;
        io.feature(&self.0.direct())
    }

    /// SetColor while in Direct.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.0.rgb = rgb(colors[0]);
        io.feature(&self.0.direct())
    }
}

fn single(info: &Info, d: &Detector) -> foxden_core::Device {
    placed(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        [1, 1],
        vec![[0.5, 0.5]],
    )
}

/// The constructor's ReadState; detection stops when it fails.
fn read_or_fail(io: &mut dyn Io, id: u8) -> Result<Royuan, String> {
    let mut state = Royuan::new(id);
    if !state.read_state(io)? {
        return Err("keyboard did not answer GET_LEDPARAM".into());
    }
    Ok(state)
}

fn open_akko(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let state = read_or_fail(io.as_mut(), 1)?;
    let proto = Akko { state, last: None };
    Ok(boxed(io, FAMILY, single(info, d), proto))
}

fn open_gk68he(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let state = read_or_fail(io.as_mut(), 2)?;
    Ok(boxed(io, FAMILY, single(info, d), Gk68he(state)))
}

fn open_legacy(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = opened(api, info)?;
    let proto = Legacy(Royuan::new(0));
    Ok(boxed(io, FAMILY, single(info, d), proto))
}

pub static DETECTORS: [Detector; 6] = [
    det(
        "Akko 3068B Plus / PC75B Plus",
        VID,
        0x4003,
        0,
        ANY,
        ANY,
        open_akko,
    ),
    det("Skyloong GK68HE Pro", VID, 0x5029, 2, ANY, ANY, open_gk68he),
    // SignalRGB enters Direct exactly as the GK68HE does (SET_LEDPARAM 0x15, then 0x0E colours).
    det("Royuan Keyboard", VID, 0x4002, 0, ANY, ANY, open_gk68he),
    det(
        "Epomaker TH80 Pro (USB Cable)",
        VID,
        0x4010,
        2,
        ANY,
        ANY,
        open_legacy,
    ),
    det(
        "Epomaker TH80 Pro (USB Dongle)",
        VID,
        0x4011,
        2,
        ANY,
        ANY,
        open_legacy,
    ),
    det(
        "Attack Shark K86 (USB Cable)",
        VID,
        0x4015,
        2,
        ANY,
        ANY,
        open_legacy,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn led_params(mode: u8, speed: u8, bright: u8, option: u8, rgb: [u8; 3]) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[1..9].copy_from_slice(&[
            GET_LED_PARAMS,
            mode,
            speed,
            bright,
            option,
            rgb[0],
            rgb[1],
            rgb[2],
        ]);
        r
    }

    // CalculateChecksum
    #[test]
    fn checksums() {
        assert_eq!(checksum(Checksum::OnesComplement, &[0x87]), 0x78);
        assert_eq!(checksum(Checksum::OnesComplement, &[0xFF, 0x02]), 0xFE);
        // sum 0x0C: next power of two 16, 16 - 12 - 1
        assert_eq!(checksum(Checksum::NextPowerOfTwo, &[0x07, 0x01, 0x04]), 3);
        assert_eq!(checksum(Checksum::NextPowerOfTwo, &[0x10]), 0xFF);
        assert_eq!(checksum(Checksum::NextPowerOfTwo, &[]), 0);
    }

    // ReadState: request, then 80 ms waits until a GET_LEDPARAM answer
    #[test]
    fn read_state_retries() {
        let mut rec = Rec::with(vec![
            vec![0, 0x00],
            led_params(0x05, 0x01, 0x09, 0x18, [1, 2, 3]),
        ]);
        let mut s = Royuan::new(2);
        assert!(s.read_state(&mut rec).unwrap());
        let mut req = pad(&[0, GET_LED_PARAMS], 65);
        req[8] = 0x78;
        assert_eq!(
            rec.take(),
            vec![
                f(req),
                Op::Sleep(80),
                Op::GetF(0, 65),
                Op::Sleep(80),
                Op::GetF(0, 65)
            ]
        );
        assert_eq!(
            (s.mode, s.speed, s.brightness, s.option),
            (0x05, 3, 4, 0x18)
        );
        assert_eq!(s.rgb, [1, 2, 3]);
        let mut none = Rec::default();
        assert!(!Royuan::new(1).read_state(&mut none).unwrap());
        assert_eq!(none.take().len(), 11);
    }

    fn set(mode: u8, speed: u8, bright: u8, opt: u8, rgb: [u8; 3], kind: Checksum) -> Op {
        let mut b = vec![0u8; 65];
        b[1..9].copy_from_slice(&[0x07, mode, speed, bright, opt, rgb[0], rgb[1], rgb[2]]);
        b[9] = checksum(kind, &b[1..9]);
        f(b)
    }

    // RGBController_EpomakerController DeviceUpdateMode(Direct) then DeviceUpdateLEDs
    #[test]
    fn legacy_frames() {
        let [first, again, next] = pushes(
            Legacy(Royuan::new(0)),
            1,
            vec![],
            &[Color(10, 20, 30)],
            &[Color(1, 1, 1)],
        );
        let k = Checksum::NextPowerOfTwo;
        assert_eq!(
            first,
            vec![
                set(1, 0, 4, 0x07, [0, 0, 0], k),
                set(1, 0, 4, 0x07, [10, 20, 30], k)
            ]
        );
        assert!(again.is_empty());
        assert_eq!(next, vec![set(1, 0, 4, 0x07, [1, 1, 1], k)]);
        let Op::F(b) = &next[0] else { panic!() };
        // sum 0x07+1+4+7+3 = 22: 32 - 22 - 1
        assert_eq!(b[9], 9);
    }

    // RGBController_SkyloongGK68HEPro DeviceUpdateMode(Direct) then SetColor in Direct
    #[test]
    fn gk68he_frames() {
        let mut state = Royuan::new(2);
        state.rgb = [9, 8, 7];
        let [first, _, next] = pushes(
            Gk68he(state),
            1,
            vec![],
            &[Color(1, 2, 3)],
            &[Color(4, 5, 6)],
        );
        let direct = |rgb: [u8; 3]| {
            let mut b = pad(&[0, 0x0E, rgb[0], rgb[1], rgb[2]], 65);
            b[8] = checksum(Checksum::OnesComplement, &b[1..8]);
            f(b)
        };
        let k = Checksum::OnesComplement;
        assert_eq!(
            first,
            vec![
                set(0x15, 4, 4, 0x07, [9, 8, 7], k),
                direct([9, 8, 7]),
                direct([1, 2, 3])
            ]
        );
        assert_eq!(next, vec![direct([4, 5, 6])]);
    }

    // Akko DeviceUpdateMode(Static) + DeviceUpdateLEDs merged into one paced SET
    #[test]
    fn akko_paced_frames() {
        let mut state = Royuan::new(1);
        state.mode = 0x04;
        let proto = Akko { state, last: None };
        let [first, _, next] = pushes(proto, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        let k = Checksum::OnesComplement;
        assert_eq!(
            first,
            vec![Op::Sleep(500), set(1, 4, 3, 0x08, [1, 2, 3], k)]
        );
        let Op::Sleep(ms) = next[0] else { panic!() };
        assert!((1400..=1500).contains(&ms), "{ms}");
        assert_eq!(next[1], set(1, 4, 3, 0x08, [4, 5, 6], k));
    }

    // Akko keeps the device's brightness when it was already in Static
    #[test]
    fn akko_static_brightness() {
        let mut state = Royuan::new(1);
        state.brightness = 0x01;
        let proto = Akko { state, last: None };
        let [first, _, _] = pushes(proto, 1, vec![], &[Color(0, 0, 0)], &[Color(0, 0, 0)]);
        assert_eq!(
            first[1],
            set(1, 4, 1, 0x08, [0, 0, 0], Checksum::OnesComplement)
        );
    }
}
