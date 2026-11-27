//! Macros kept in a Logitech mouse's onboard profiles (HID++ 0x8100), so they run from its memory.

use std::time::{Duration, Instant};

use hidapi::HidDevice;

use crate::hidpp;

const FEATURE: u16 = 0x8100;
const SW_ID: u8 = 0x0C;
const DIRECTORY: u16 = 0x0000;
const ROM: u16 = 0x0100;
const BUTTONS_AT: usize = 0x20;

pub const MACRO_FORMAT: u8 = 1;
pub const DELAY_MAX_MS: u32 = 0xFFFF;

/// What the 0x8100 descriptor says about the memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub profile_format: u8,
    pub macro_format: u8,
    pub profiles: u8,
    pub buttons: u8,
    pub sectors: u16,
    pub size: u16,
}

impl Layout {
    pub fn parse(desc: &[u8; 16]) -> Self {
        Layout {
            profile_format: desc[1],
            macro_format: desc[2],
            profiles: desc[3],
            buttons: desc[5],
            sectors: desc[6] as u16,
            size: u16::from_be_bytes([desc[7], desc[8]]),
        }
    }

    /// Only the memory model and formats this module writes.
    pub fn supported(&self, memory_model: u8) -> bool {
        memory_model == 1
            && (1..=3).contains(&self.profile_format)
            && self.macro_format == MACRO_FORMAT
            && self.buttons > 0
            && self.buttons <= 16
            && (64..=4096).contains(&self.size)
    }
}

/// CRC-16/CCITT with 0xFFFF start, over everything but a sector's last two bytes.
pub fn crc(data: &[u8]) -> u16 {
    let mut c: u16 = 0xFFFF;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

pub fn crc_ok(sector: &[u8]) -> bool {
    let n = sector.len();
    n > 2 && crc(&sector[..n - 2]).to_be_bytes() == sector[n - 2..]
}

pub fn seal(sector: &mut [u8]) {
    let n = sector.len();
    let c = crc(&sector[..n - 2]);
    sector[n - 2..].copy_from_slice(&c.to_be_bytes());
}

/// What one button of a profile does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Mouse(u16),
    Key { mods: u8, key: u8 },
    Consumer(u16),
    Special(u8),
    Macro { sector: u8, offset: u8 },
    Disabled,
    Other([u8; 4]),
}

impl Binding {
    pub fn parse(b: [u8; 4]) -> Self {
        match b {
            [0x80, 0x01, hi, lo] => Binding::Mouse(u16::from_be_bytes([hi, lo])),
            [0x80, 0x02, mods, key] => Binding::Key { mods, key },
            [0x80, 0x03, hi, lo] => Binding::Consumer(u16::from_be_bytes([hi, lo])),
            [0x90, code, _, _] => Binding::Special(code),
            [0x00, sector, 0x00, offset] => Binding::Macro { sector, offset },
            [0xFF, ..] => Binding::Disabled,
            other => Binding::Other(other),
        }
    }

    pub fn bytes(self) -> [u8; 4] {
        match self {
            Binding::Mouse(m) => {
                let [hi, lo] = m.to_be_bytes();
                [0x80, 0x01, hi, lo]
            }
            Binding::Key { mods, key } => [0x80, 0x02, mods, key],
            Binding::Consumer(c) => {
                let [hi, lo] = c.to_be_bytes();
                [0x80, 0x03, hi, lo]
            }
            Binding::Special(code) => [0x90, code, 0, 0],
            Binding::Macro { sector, offset } => [0x00, sector, 0x00, offset],
            Binding::Disabled => [0xFF; 4],
            Binding::Other(b) => b,
        }
    }

    pub fn label(self) -> String {
        match self {
            Binding::Mouse(m) => match m {
                0x01 => "Left click".into(),
                0x02 => "Right click".into(),
                0x04 => "Middle click".into(),
                0x08 => "Back".into(),
                0x10 => "Forward".into(),
                m => format!("Mouse buttons {m:#06x}"),
            },
            Binding::Key { mods, key } => format!("Key {key:#04x} (modifiers {mods:#04x})"),
            Binding::Consumer(c) => format!("Media {c:#06x}"),
            Binding::Special(code) => match code {
                0x01 => "Wheel left".into(),
                0x02 => "Wheel right".into(),
                0x03 => "DPI up".into(),
                0x04 => "DPI down".into(),
                0x05 => "DPI cycle".into(),
                0x06 => "DPI default".into(),
                0x07 => "DPI shift".into(),
                0x08 => "Next profile".into(),
                0x09 => "Previous profile".into(),
                0x0A => "Cycle profiles".into(),
                0x0B => "G-shift".into(),
                0x0C => "Battery level".into(),
                c => format!("Special {c:#04x}"),
            },
            Binding::Macro { .. } => "Macro".into(),
            Binding::Disabled => "Off".into(),
            Binding::Other(b) => format!("{b:02x?}"),
        }
    }
}

pub fn buttons(profile: &[u8], count: u8) -> Vec<Binding> {
    (0..count as usize)
        .map(|i| {
            let at = BUTTONS_AT + i * 4;
            Binding::parse([profile[at], profile[at + 1], profile[at + 2], profile[at + 3]])
        })
        .collect()
}

/// The profile with one button changed and its CRC redone.
pub fn with_button(profile: &[u8], slot: u8, binding: Binding) -> Vec<u8> {
    let mut p = profile.to_vec();
    let at = BUTTONS_AT + slot as usize * 4;
    p[at..at + 4].copy_from_slice(&binding.bytes());
    seal(&mut p);
    p
}

/// The user profiles the directory lists, as (sector, enabled).
pub fn directory(dir: &[u8], profiles: u8) -> Vec<(u16, bool)> {
    dir[..dir.len() - 2]
        .as_chunks::<4>()
        .0
        .iter()
        .take(profiles as usize)
        .take_while(|e| e[0..2] != [0xFF, 0xFF])
        .map(|e| (u16::from_be_bytes([e[0], e[1]]), e[2] != 0))
        .collect()
}

/// One step a stored macro takes. Modifier keys go in `mods` as HID modifier bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Press { mods: u8, key: u8 },
    Release { mods: u8, key: u8 },
    Delay(u16),
}

const OP_DELAY: u8 = 0x40;
const OP_PRESS: u8 = 0x43;
const OP_RELEASE: u8 = 0x44;
const OP_END: u8 = 0xFF;

/// The macro as one sector of `size` bytes, sealed; it has to fit in one.
pub fn encode(ops: &[Op], size: u16) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    for op in ops {
        match *op {
            Op::Press { mods, key } => b.extend([OP_PRESS, mods, key]),
            Op::Release { mods, key } => b.extend([OP_RELEASE, mods, key]),
            Op::Delay(ms) => {
                b.push(OP_DELAY);
                b.extend(ms.to_be_bytes());
            }
        }
    }
    b.push(OP_END);
    let room = size as usize - 2;
    if b.len() > room {
        return Err(format!("Too long for the mouse: {} bytes, it holds {room}.", b.len()));
    }
    b.resize(size as usize, 0xFF);
    seal(&mut b);
    Ok(b)
}

/// The steps of a macro sector this module wrote, `None` for anything else.
pub fn decode(sector: &[u8]) -> Option<Vec<Op>> {
    let mut ops = Vec::new();
    let mut i = 0;
    while i < sector.len().saturating_sub(2) {
        let take = |n: usize| sector.get(i + 1..i + 1 + n);
        match sector[i] {
            OP_END => return Some(ops),
            OP_PRESS => {
                let a = take(2)?;
                ops.push(Op::Press { mods: a[0], key: a[1] });
                i += 3;
            }
            OP_RELEASE => {
                let a = take(2)?;
                ops.push(Op::Release { mods: a[0], key: a[1] });
                i += 3;
            }
            OP_DELAY => {
                let a = take(2)?;
                ops.push(Op::Delay(u16::from_be_bytes([a[0], a[1]])));
                i += 3;
            }
            _ => return None,
        }
    }
    None
}

/// A sector no profile uses, after the profiles and their directory.
pub fn free_sector(layout: &Layout, used: &[u16]) -> Option<u16> {
    (layout.profiles as u16 + 1..layout.sectors).find(|s| !used.contains(s))
}

/// Sector access, so the steps below can be tested without a mouse.
pub trait Memory {
    fn layout(&self) -> Layout;
    fn read(&self, sector: u16) -> Result<Vec<u8>, String>;
    fn write(&self, sector: u16, data: &[u8]) -> Result<(), String>;
}

/// One button as the enabled profiles have it, next to its factory setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub slot: u8,
    pub now: Binding,
    pub factory: Binding,
}

fn enabled(mem: &impl Memory) -> Result<Vec<u16>, String> {
    let l = mem.layout();
    let all = directory(&mem.read(DIRECTORY)?, l.profiles);
    let on: Vec<u16> = all.iter().filter(|p| p.1).map(|p| p.0).collect();
    if on.is_empty() {
        return Err("The mouse has no profile turned on.".into());
    }
    Ok(on)
}

/// The buttons of the first enabled profile.
pub fn list(mem: &impl Memory) -> Result<Vec<Button>, String> {
    let l = mem.layout();
    let first = mem.read(enabled(mem)?[0])?;
    let rom = mem.read(ROM + 1)?;
    let now = buttons(&first, l.buttons);
    let factory = buttons(&rom, l.buttons);
    Ok(now.into_iter().zip(factory).enumerate().map(|(i, (now, factory))| Button { slot: i as u8, now, factory }).collect())
}

fn used_sectors(mem: &impl Memory) -> Result<Vec<u16>, String> {
    let l = mem.layout();
    let mut used = Vec::new();
    for (sector, _) in directory(&mem.read(DIRECTORY)?, l.profiles) {
        used.push(sector);
        let Ok(p) = mem.read(sector) else { continue };
        for b in buttons(&p, l.buttons).into_iter().chain(buttons(&p[0x40..], l.buttons)) {
            if let Binding::Macro { sector, .. } = b {
                used.push(sector as u16);
            }
        }
    }
    Ok(used)
}

fn set_everywhere(mem: &impl Memory, slot: u8, binding: Binding) -> Result<(), String> {
    for sector in enabled(mem)? {
        let p = mem.read(sector)?;
        if !crc_ok(&p) {
            return Err(format!("Profile sector {sector} fails its check; nothing was changed there."));
        }
        if buttons(&p, mem.layout().buttons)[slot as usize] != binding {
            mem.write(sector, &with_button(&p, slot, binding))?;
        }
    }
    Ok(())
}

/// Stores a macro and puts it on a button in every enabled profile. Returns the sector used.
pub fn store(mem: &impl Memory, slot: u8, ops: &[Op]) -> Result<u16, String> {
    let l = mem.layout();
    if slot >= l.buttons {
        return Err("The mouse has no such button.".into());
    }
    let data = encode(ops, l.size)?;
    let mine = match list(mem)?[slot as usize].now {
        Binding::Macro { sector, offset: 0 } if sector as u16 > l.profiles as u16 && (sector as u16) < l.sectors => {
            Some(sector as u16)
        }
        _ => None,
    };
    let sector = match mine {
        Some(s) => s,
        None => free_sector(&l, &used_sectors(mem)?).ok_or("The mouse's memory is full.")?,
    };
    mem.write(sector, &data)?;
    set_everywhere(mem, slot, Binding::Macro { sector: sector as u8, offset: 0 })?;
    Ok(sector)
}

/// Puts a button back to what it does from the factory.
pub fn reset(mem: &impl Memory, slot: u8) -> Result<(), String> {
    let b = list(mem)?.into_iter().nth(slot as usize).ok_or("The mouse has no such button.")?;
    set_everywhere(mem, slot, b.factory)
}

/// A mouse whose onboard memory this module can write.
pub struct Mouse {
    hid: HidDevice,
    index: u8,
    feature: u8,
    pub layout: Layout,
    pub name: String,
    pub id: String,
}

fn long(index: u8, feature: u8, function: u8, params: &[u8]) -> hidpp::Report {
    let mut r = hidpp::long(index, feature, function, params);
    r[3] = function | SW_ID;
    r
}

fn ask(hid: &HidDevice, r: &hidpp::Report) -> Result<[u8; 16], String> {
    ask_within(hid, r, Duration::from_millis(1000))
}

fn ask_within(hid: &HidDevice, r: &hidpp::Report, wait: Duration) -> Result<[u8; 16], String> {
    hid.write(r).map_err(|e| e.to_string())?;
    let end = Instant::now() + wait;
    let mut buf = [0u8; 64];
    while Instant::now() < end {
        let n = hid.read_timeout(&mut buf, 50).map_err(|e| e.to_string())?;
        if let Some(x) = hidpp::reply_to(r, &buf[..n]) {
            return x.map_err(|c| format!("HID++ error {c:#04x}"));
        }
    }
    Err("no reply from the mouse".into())
}

/// Feature 0x0005, the name the device gives itself.
fn device_name(hid: &HidDevice, index: u8) -> Option<String> {
    let f = ask(hid, &long(index, 0, 0, &0x0005u16.to_be_bytes())).ok()?[0];
    if f == 0 {
        return None;
    }
    let len = ask(hid, &long(index, f, 0x00, &[])).ok()?[0] as usize;
    let mut name = Vec::new();
    while name.len() < len.min(64) {
        let part = ask(hid, &long(index, f, 0x10, &[name.len() as u8])).ok()?;
        name.extend_from_slice(&part[..(len - name.len()).min(16)]);
    }
    let name = String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_string();
    (!name.is_empty()).then_some(name)
}

impl Mouse {
    fn open(hid: HidDevice, index: u8, product: &str) -> Option<Mouse> {
        let feature = ask(&hid, &long(index, 0, 0, &FEATURE.to_be_bytes())).ok()?[0];
        if feature == 0 {
            return None;
        }
        let desc = ask(&hid, &long(index, feature, 0x00, &[])).ok()?;
        let layout = Layout::parse(&desc);
        if !layout.supported(desc[0]) {
            return None;
        }
        let name = device_name(&hid, index).unwrap_or_else(|| product.trim().to_string());
        let name = name.trim_start_matches("Logitech").trim().to_string();
        let id = name.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
        Some(Mouse { hid, index, feature, layout, name: format!("Logitech {name}"), id })
    }

    fn read_sector(&self, sector: u16) -> Result<Vec<u8>, String> {
        let size = self.layout.size;
        let mut out = vec![0u8; size as usize];
        let mut off = 0u16;
        while off < size {
            let at = off.min(size - 16);
            let [s1, s2] = sector.to_be_bytes();
            let [o1, o2] = at.to_be_bytes();
            let r = ask(&self.hid, &long(self.index, self.feature, 0x50, &[s1, s2, o1, o2]))?;
            out[at as usize..at as usize + 16].copy_from_slice(&r);
            off = at + 16;
        }
        Ok(out)
    }

    /// Writes a whole sector and reads it back.
    fn write_sector(&self, sector: u16, data: &[u8]) -> Result<(), String> {
        let size = self.layout.size;
        if data.len() != size as usize || !crc_ok(data) || sector == DIRECTORY || sector >= ROM {
            return Err("refusing a bad sector write".into());
        }
        let [s1, s2] = sector.to_be_bytes();
        let [l1, l2] = size.to_be_bytes();
        let slow = Duration::from_secs(3);
        let step = |what: &str, r: hidpp::Report| {
            ask_within(&self.hid, &r, slow).map_err(|e| format!("The mouse didn't take the write ({what}: {e})."))
        };
        step("start", long(self.index, self.feature, 0x60, &[s1, s2, 0, 0, l1, l2]))?;
        for chunk in data.chunks(16) {
            let mut c = [0u8; 16];
            c[..chunk.len()].copy_from_slice(chunk);
            step("data", long(self.index, self.feature, 0x70, &c))?;
        }
        step("end", long(self.index, self.feature, 0x80, &[]))?;
        if self.read_sector(sector)? != data {
            return Err(format!("sector {sector:#06x} reads back different"));
        }
        Ok(())
    }

    /// 1 onboard, 2 host.
    pub fn mode(&self) -> Result<u8, String> {
        Ok(ask(&self.hid, &long(self.index, self.feature, 0x20, &[]))?[0])
    }

    pub fn set_mode(&self, onboard: bool) -> Result<(), String> {
        ask(&self.hid, &long(self.index, self.feature, 0x10, &[if onboard { 1 } else { 2 }])).map(|_| ())
    }

}

impl Memory for Mouse {
    fn layout(&self) -> Layout {
        self.layout
    }
    fn read(&self, sector: u16) -> Result<Vec<u8>, String> {
        self.read_sector(sector)
    }
    fn write(&self, sector: u16, data: &[u8]) -> Result<(), String> {
        self.write_sector(sector, data)
    }
}

/// The mice with onboard memory this module can write, on a cable or at a receiver's first slots.
pub fn mice() -> Vec<Mouse> {
    hidapi::HidApi::disable_device_discovery();
    let Ok(api) = hidapi::HidApi::new() else { return Vec::new() };
    let mut out = Vec::new();
    let listed = crate::scan::collections()
        .into_iter()
        .filter(|i| i.vid == hidpp::VID && hidpp::is_long_channel(i.usage_page, i.usage))
        .map(crate::scan::Dev::new);
    for d in listed {
        let pid = d.product_id();
        let product = d.product_string().unwrap_or_default().to_string();
        let slots: &[u8] = if hidpp::cable(pid) { &[0xFF] } else { &[1, 2, 3] };
        for &index in slots {
            let Ok(hid) = d.open_device(&api) else { break };
            if let Some(m) = Mouse::open(hid, index, &product) {
                out.push(m);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g502_profile() -> Vec<u8> {
        let mut p = vec![0xFF; 255];
        p[..13].copy_from_slice(&[0x01, 0, 0, 0x70, 0x17, 0, 0, 0, 0, 0, 0, 0, 0]);
        let b: [u8; 44] = [
            0x80, 1, 0, 1, 0x80, 1, 0, 2, 0x80, 1, 0, 4, 0x80, 1, 0, 8, 0x80, 1, 0, 0x10, 0x90, 7, 0, 0, 0x90, 4, 0, 0, 0x90,
            3, 0, 0, 0x90, 0x0C, 0, 0, 0x90, 2, 0, 0, 0x90, 1, 0, 0,
        ];
        p[0x20..0x20 + 44].copy_from_slice(&b);
        seal(&mut p);
        p
    }

    #[test]
    fn crc_matches_the_g502_directory() {
        let mut dir = vec![0xFF; 255];
        dir[..20].copy_from_slice(&[0, 1, 1, 0xFF, 0, 2, 0, 0xFF, 0, 3, 0, 0xFF, 0, 4, 0, 0xFF, 0, 5, 0, 0xFF]);
        assert_eq!(crc(&dir[..253]), 0x4037);
        assert_eq!(directory(&dir, 5), vec![(1, true), (2, false), (3, false), (4, false), (5, false)]);
    }

    #[test]
    fn buttons_read_and_change_one_slot() {
        let p = g502_profile();
        let b = buttons(&p, 11);
        assert_eq!(b[0], Binding::Mouse(1));
        assert_eq!(b[5].label(), "DPI shift");
        assert_eq!(b[10].label(), "Wheel left");
        let q = with_button(&p, 9, Binding::Macro { sector: 6, offset: 0 });
        assert!(crc_ok(&q));
        assert_eq!(buttons(&q, 11)[9], Binding::Macro { sector: 6, offset: 0 });
        let changed: Vec<usize> = (0..253).filter(|&i| p[i] != q[i]).collect();
        assert_eq!(changed, vec![0x20 + 36, 0x20 + 37]);
    }

    #[test]
    fn macros_round_trip_and_must_fit() {
        let ops = [Op::Press { mods: 0x02, key: 0x0B }, Op::Delay(30), Op::Release { mods: 0x02, key: 0x0B }];
        let s = encode(&ops, 255).unwrap();
        assert_eq!(&s[..10], &[0x43, 0x02, 0x0B, 0x40, 0x00, 0x1E, 0x44, 0x02, 0x0B, 0xFF]);
        assert!(crc_ok(&s));
        assert_eq!(decode(&s).unwrap(), ops);
        assert!(encode(&[Op::Delay(1); 84], 255).is_ok() && encode(&[Op::Delay(1); 85], 255).unwrap_err().contains("Too long"));
        assert!(decode(&[0x01; 20]).is_none());
    }

    struct Fake(std::cell::RefCell<std::collections::HashMap<u16, Vec<u8>>>, std::cell::Cell<usize>);

    impl Memory for Fake {
        fn layout(&self) -> Layout {
            Layout::parse(&[1, 3, 1, 5, 1, 0x0B, 0x10, 0, 0xFF, 0x0A, 4, 0, 0, 0, 0, 0])
        }
        fn read(&self, s: u16) -> Result<Vec<u8>, String> {
            Ok(self.0.borrow().get(&s).cloned().unwrap_or_else(|| vec![0xFF; 255]))
        }
        fn write(&self, s: u16, d: &[u8]) -> Result<(), String> {
            self.1.set(self.1.get() + 1);
            self.0.borrow_mut().insert(s, d.to_vec());
            Ok(())
        }
    }

    fn fake() -> Fake {
        let mut dir = vec![0xFF; 255];
        dir[..20].copy_from_slice(&[0, 1, 1, 0xFF, 0, 2, 0, 0xFF, 0, 3, 0, 0xFF, 0, 4, 0, 0xFF, 0, 5, 0, 0xFF]);
        seal(&mut dir);
        let mut rom = g502_profile();
        rom[255 - 2..].copy_from_slice(&[0xFF, 0xFF]);
        let m = [(0u16, dir), (1, g502_profile()), (2, g502_profile()), (0x101, rom)];
        Fake(std::cell::RefCell::new(m.into_iter().collect()), std::cell::Cell::new(0))
    }

    #[test]
    fn store_reuses_its_sector_and_reset_restores_the_factory_button() {
        let mem = fake();
        let before = mem.read(1).unwrap();
        let ops = [Op::Press { mods: 0, key: 0x14 }, Op::Release { mods: 0, key: 0x14 }];
        assert_eq!(store(&mem, 9, &ops).unwrap(), 6);
        assert_eq!(list(&mem).unwrap()[9].now, Binding::Macro { sector: 6, offset: 0 });
        assert_eq!(list(&mem).unwrap()[9].factory.label(), "Wheel right");
        assert_eq!(mem.read(2).unwrap(), g502_profile());
        assert_eq!(store(&mem, 9, &ops[..1]).unwrap(), 6);
        assert_eq!(store(&mem, 10, &ops).unwrap(), 7);
        assert_eq!(decode(&mem.read(6).unwrap()).unwrap(), ops[..1]);
        reset(&mem, 9).unwrap();
        reset(&mem, 10).unwrap();
        assert_eq!(mem.read(1).unwrap(), before);
        let writes = mem.1.get();
        reset(&mem, 10).unwrap();
        assert_eq!(mem.1.get(), writes);
        assert!(store(&mem, 11, &ops).is_err());
    }

    #[test]
    fn free_sectors_skip_the_profiles() {
        let l = Layout::parse(&[1, 3, 1, 5, 1, 0x0B, 0x10, 0, 0xFF, 0x0A, 4, 0, 0, 0, 0, 0]);
        assert!(l.supported(1));
        assert_eq!((l.buttons, l.sectors, l.size), (11, 16, 255));
        assert_eq!(free_sector(&l, &[]), Some(6));
        assert_eq!(free_sector(&l, &[6, 7]), Some(8));
        assert_eq!(free_sector(&l, &(6..16).collect::<Vec<_>>()), None);
    }
}
