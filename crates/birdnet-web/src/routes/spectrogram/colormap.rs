//! Viridis colormap for spectrogram rendering.

/// Approximate viridis colormap: maps t in [0,1] to (R,G,B).
pub fn viridis(t: f32) -> (u8, u8, u8) {
    // Control points: (t, R, G, B)
    let cps: &[(f32, f32, f32, f32)] = &[
        (0.000, 68.0, 1.0, 84.0),
        (0.125, 71.0, 44.0, 122.0),
        (0.250, 59.0, 82.0, 139.0),
        (0.375, 44.0, 113.0, 142.0),
        (0.500, 33.0, 145.0, 140.0),
        (0.625, 39.0, 173.0, 129.0),
        (0.750, 92.0, 200.0, 99.0),
        (0.875, 170.0, 220.0, 50.0),
        (1.000, 253.0, 231.0, 37.0),
    ];

    let t = t.clamp(0.0, 1.0);
    let i = cps
        .partition_point(|cp| cp.0 <= t)
        .saturating_sub(1)
        .min(cps.len() - 2);
    let (t0, r0, g0, b0) = cps[i];
    let (t1, r1, g1, b1) = cps[i + 1];
    let frac = if (t1 - t0).abs() < 1e-6 {
        0.0
    } else {
        (t - t0) / (t1 - t0)
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let lerp = |a: f32, b: f32| (a + frac * (b - a)).clamp(0.0, 255.0) as u8;
    (lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viridis_endpoints() {
        let (r, _g, b) = viridis(0.0);
        assert!(b > r, "cold end should be blue-heavy");
        let (r2, g2, _b2) = viridis(1.0);
        assert!(r2 > 200 && g2 > 200, "warm end should be yellow");
    }

    #[test]
    fn viridis_midpoint_is_greenish() {
        let (_r, g, _b) = viridis(0.5);
        assert!(g > 100, "midpoint should have significant green");
    }
}
