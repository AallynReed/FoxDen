//! Sonar's chat mix curve (SoundStage.Services.ChatMix.ChatMixService).

/// Balance runs from -1 (all game) to 1 (all chat).
pub const BALANCE_MIN: f32 = -1.0;
pub const BALANCE_MAX: f32 = 1.0;
/// Changes smaller than this are ignored.
pub const BALANCE_EPSILON: f32 = 0.001;

/// `ToScaled`: s = sign(b)·2|b| / (|b| + 1).
pub fn to_scaled(balance: f32) -> f32 {
    if balance == 0.0 {
        return 0.0;
    }
    let s = balance.signum();
    (s * 2.0 * balance) / (balance + s)
}

/// `ToLinear`, the inverse of [`to_scaled`].
pub fn to_linear(scaled: f32) -> f32 {
    if scaled == 0.0 {
        return 0.0;
    }
    let s = scaled.signum();
    (-s * scaled) / (scaled - 2.0 * s)
}

/// Linear gains `(game, chat)` for a balance, as `SetChatmix` hands them to the audio driver.
pub fn gains(balance: f32) -> (f32, f32) {
    let s = to_scaled(balance.clamp(BALANCE_MIN, BALANCE_MAX));
    ((1.0 - s).clamp(0.0, 1.0), (1.0 + s).clamp(0.0, 1.0))
}

/// `ToBalance`: the balance that gives these linear gains (one of them must be 1).
pub fn balance_from_gains(game: f32, chat: f32) -> f32 {
    let scaled = if (game - 1.0).abs() > f32::EPSILON { 1.0 - game } else { chat - 1.0 };
    to_linear(scaled)
}

pub fn validate(balance: f32) -> Result<f32, String> {
    if (BALANCE_MIN..=BALANCE_MAX).contains(&balance) {
        Ok(balance)
    } else {
        Err("Balance is out of range [-1.0; 1.0]".into())
    }
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

    #[test]
    fn round_trip() {
        for b in [-0.9f32, -0.3, 0.2, 0.75] {
            let (g, c) = gains(b);
            assert!((balance_from_gains(g, c) - b).abs() < 1e-4, "{b}");
        }
    }
}
