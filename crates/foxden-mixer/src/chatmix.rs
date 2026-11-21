//! The chat mix curve: one balance from -1 (all game) to 1 (all chat).

/// s = sign(b)·2|b| / (|b| + 1).
fn to_scaled(balance: f32) -> f32 {
    if balance == 0.0 {
        return 0.0;
    }
    let s = balance.signum();
    (s * 2.0 * balance) / (balance + s)
}

/// Linear gains `(game, chat)` for a balance.
pub fn gains(balance: f32) -> (f32, f32) {
    let s = to_scaled(balance.clamp(-1.0, 1.0));
    ((1.0 - s).clamp(0.0, 1.0), (1.0 + s).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_ends_and_centre() {
        assert_eq!(gains(0.0), (1.0, 1.0));
        let (g, c) = gains(1.0);
        assert!(g.abs() < 1e-6 && (c - 1.0).abs() < 1e-6);
        let (g, c) = gains(-1.0);
        assert!((g - 1.0).abs() < 1e-6 && c.abs() < 1e-6);
        let (g, _) = gains(0.5);
        assert!((g - (1.0 - 2.0 / 3.0)).abs() < 1e-6);
    }
}
