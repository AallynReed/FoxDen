use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x10F5;
/// (pid, on the cable); transmitters first.
const MODELS: &[(u16, bool)] = &[(0x2251, false), (0x2262, false), (0x2253, true)];
const IN_ID: u8 = 0x07;

/// RACE 0x9942: turns on the chip's CoAP client so it answers the read below.
const COAP_ON: &[u8] = &[0x05, 0x5A, 0x02, 0x00, 0x42, 0x99];
/// RACE 0x9902 carrying CoAP GET /GSI (status document).
const GET_GSI: &[u8] = &[
    0x05, 0x5A, 0x0A, 0x00, 0x02, 0x99, 0x40, 0x01, 0x00, 0x01, 0xB3, b'G', b'S', b'I',
];

/// Output report 06: `[06][len lo][len hi][RACE bytes]`.
fn frame(race: &[u8]) -> Vec<u8> {
    let mut r = vec![0x06, race.len() as u8, 0x00];
    r.extend_from_slice(race);
    r
}

/// Payload of report 07 `[07][len][00][payload]`.
fn payload(r: &[u8]) -> Option<&[u8]> {
    if r.len() < 3 || r[0] != IN_ID {
        return None;
    }
    let n = (r[1] as usize).min(r.len() - 3);
    Some(&r[3..3 + n])
}

/// The newest `"240":"<percent>"` of a GSI document.
fn gsi_level(doc: &[u8]) -> Option<u8> {
    const KEY: &[u8] = b"\"240\":\"";
    if !doc.windows(5).any(|w| w == b"\"GSI\"") {
        return None;
    }
    let mut level = None;
    for (at, _) in doc
        .windows(KEY.len())
        .enumerate()
        .filter(|(_, w)| *w == KEY)
    {
        let rest = &doc[at + KEY.len()..];
        let digits = rest
            .iter()
            .take(4)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 || rest.get(digits) != Some(&b'"') {
            continue;
        }
        let v = rest[..digits]
            .iter()
            .fold(0u32, |a, &b| a * 10 + (b - b'0') as u32);
        if v <= 100 {
            level = Some(v as u8);
        }
    }
    level
}

fn drain(dev: &Dev) {
    for _ in 0..5 {
        let _ = dev.get_input_report(IN_ID);
    }
}

fn read_level(c: &Coll) -> Option<u8> {
    let dev = Dev::open(c)?;
    drain(&dev);
    if !dev.write(&frame(COAP_ON)) {
        return None;
    }
    sleep(Duration::from_millis(300));
    drain(&dev);
    if !dev.write(&frame(GET_GSI)) {
        return None;
    }
    // The JSON reply spans several reports.
    let mut doc = Vec::new();
    for _ in 0..30 {
        sleep(Duration::from_millis(30));
        let Some(r) = dev.get_input_report(IN_ID) else {
            continue;
        };
        let Some(p) = payload(&r) else { continue };
        doc.extend_from_slice(p);
        if let Some(l) = gsi_level(&doc) {
            return Some(l);
        }
    }
    None
}

fn model_name(product: &str) -> String {
    if product.is_empty() {
        "Turtle Beach Stealth 700 Gen 3".into()
    } else if product.to_lowercase().starts_with("turtle beach") {
        product.into()
    } else {
        format!("Turtle Beach {product}")
    }
}

pub fn poll_turtle_beach(ctx: &mut Ctx) {
    let mut first: Option<(Coll, bool)> = None;
    let mut reading = None;
    for &(pid, cable) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let found = ctx.find(VID, pid, |c| {
            c.page == 0xFF13 && c.usage == 0x0001 && c.in_len > 0 && c.out_len > 0
        });
        let Some(c) = found.into_iter().next() else {
            continue;
        };
        if let Some(l) = read_level(&c) {
            reading = Some((c, cable, l));
            break;
        }
        first.get_or_insert((c, cable));
    }
    let (c, cable, level) = match (reading, first) {
        (Some((c, cable, l)), _) => (c, cable, Some(l)),
        (None, Some((c, cable))) => (c, cable, None),
        (None, None) => return,
    };
    let b = Battery {
        key: ctx.stable_key("turtlebeach-stealth700"),
        name: model_name(&c.product),
        via: if cable {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        }
        .into(),
        kind: Kind::Headset,
        percent: level,
        asleep: level.is_none(),
        grace_secs: 300,
        ..Default::default()
    };
    ctx.add(b, VID, c.pid);
    for &(pid, _) in MODELS {
        ctx.claim(VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_requests() {
        assert_eq!(
            frame(COAP_ON),
            [0x06, 0x06, 0x00, 0x05, 0x5A, 0x02, 0x00, 0x42, 0x99]
        );
        assert_eq!(frame(GET_GSI)[..4], [0x06, 0x0E, 0x00, 0x05]);
    }

    #[test]
    fn level_from_a_split_document() {
        let a = [&[0x07, 10, 0x00][..], br#"{"OR":"GSI"#].concat();
        let b = [
            &[0x07, 23, 0x00][..],
            br#"","KVP":{"240":"73","#,
            &[0xAA; 8],
        ]
        .concat();
        let mut doc = Vec::new();
        doc.extend_from_slice(payload(&a).unwrap());
        assert_eq!(gsi_level(&doc), None);
        doc.extend_from_slice(payload(&b).unwrap());
        assert_eq!(gsi_level(&doc), Some(73));
        assert_eq!(gsi_level(br#"{"OR":"GSI","KVP":{"240":"250"}}"#), None);
        assert_eq!(
            model_name("Stealth 700X Gen 3"),
            "Turtle Beach Stealth 700X Gen 3"
        );
    }
}
