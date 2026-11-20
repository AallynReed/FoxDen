use std::collections::HashSet;

use crate::Battery;
use crate::hid::Coll;

/// What one poll knows: the HID collections, which products a driver already handled, and the results.
/// Vendor drivers run first and claim their products, so the generic fallbacks skip them.
#[derive(Default)]
pub struct Ctx {
    pub colls: Vec<Coll>,
    pub claimed: HashSet<(u16, u16)>,
    /// Bluetooth container ids already reported.
    pub bt_containers: HashSet<String>,
    pub out: Vec<Battery>,
}

impl Ctx {
    pub fn new(colls: Vec<Coll>) -> Self {
        Self {
            colls,
            ..Self::default()
        }
    }

    pub fn is_claimed(&self, vid: u16, pid: u16) -> bool {
        self.claimed.contains(&(vid, pid))
    }

    pub fn claim(&mut self, vid: u16, pid: u16) {
        self.claimed.insert((vid, pid));
    }

    pub fn add(&mut self, mut b: Battery, vid: u16, pid: u16) {
        if b.vid == 0 {
            b.vid = vid;
        }
        self.out.push(b);
        self.claim(vid, pid);
    }

    /// `base`, or `base#2`, `base#3`... when identical devices are already listed. No HID paths,
    /// so the key survives moving a receiver to another port.
    pub fn stable_key(&self, base: &str) -> String {
        let taken = |k: &str| self.out.iter().any(|b| b.key == k && b.connected);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}#{n}"))
            .find(|k| !taken(k))
            .unwrap()
    }

    /// Distinct unclaimed products of a vendor.
    pub fn products(&self, vid: u16) -> Vec<(u16, u16)> {
        let mut v: Vec<_> = self
            .colls
            .iter()
            .filter(|c| c.vid == vid && !self.is_claimed(c.vid, c.pid))
            .map(|c| (c.vid, c.pid))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Collections of one product that pass `pred`.
    pub fn find(&self, vid: u16, pid: u16, pred: impl Fn(&Coll) -> bool) -> Vec<Coll> {
        self.colls
            .iter()
            .filter(|c| c.vid == vid && c.pid == pid && pred(c))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_devices_get_numbered_keys() {
        let mut ctx = Ctx::default();
        assert_eq!(ctx.stable_key("mouse"), "mouse");
        ctx.add(
            Battery {
                key: "mouse".into(),
                ..Default::default()
            },
            1,
            2,
        );
        assert_eq!(ctx.stable_key("mouse"), "mouse#2");
        assert!(ctx.is_claimed(1, 2));
    }
}
