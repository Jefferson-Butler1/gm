//! Deterministic integer trig (CORDIC).
//!
//! Shifts and adds only, so results are bit-identical on every machine. Public angles are `u16` turns (65536 = one turn) from
//! +x toward +y; internally one turn is 2^32 for headroom.

use crate::{Fx, FxVec2};

const HALF_TURN: i64 = 1 << 31;
const QUARTER_TURN: i64 = 1 << 30;

/// `atan(2^-i)` in 2^32-per-turn units.
const ATAN: [i64; 31] = [
    536_870_912,
    316_933_406,
    167_458_907,
    85_004_756,
    42_667_331,
    21_354_465,
    10_679_838,
    5_340_245,
    2_670_163,
    1_335_087,
    667_544,
    333_772,
    166_886,
    83_443,
    41_722,
    20_861,
    10_430,
    5_215,
    2_608,
    1_304,
    652,
    326,
    163,
    81,
    41,
    20,
    10,
    5,
    3,
    1,
    1,
];

/// CORDIC gain compensation (`prod 1/sqrt(1 + 2^-2i)`) as `Fx` bits.
const GAIN_INV: i64 = 2_608_131_496;

/// Unit vector at `angle`. Components are within ~2^-29 of exact.
#[must_use]
pub fn unit(angle: u16) -> FxVec2 {
    // Into (-half, half], then fold the back half-plane forward: CORDIC converges within
    // about ±99°.
    let mut z = i64::from(angle).wrapping_shl(16);
    if z > HALF_TURN {
        z = z.wrapping_sub(HALF_TURN.wrapping_mul(2));
    }
    let flip = !(-QUARTER_TURN..=QUARTER_TURN).contains(&z);
    if flip {
        z = if z > 0 {
            z.wrapping_sub(HALF_TURN)
        } else {
            z.wrapping_add(HALF_TURN)
        };
    }
    let (mut x, mut y) = (GAIN_INV, 0_i64);
    for (i, &a) in (0_u32..).zip(&ATAN) {
        let (dx, dy) = (y.wrapping_shr(i), x.wrapping_shr(i));
        if z >= 0 {
            (x, y, z) = (x.wrapping_sub(dx), y.wrapping_add(dy), z.wrapping_sub(a));
        } else {
            (x, y, z) = (x.wrapping_add(dx), y.wrapping_sub(dy), z.wrapping_add(a));
        }
    }
    if flip {
        (x, y) = (x.wrapping_neg(), y.wrapping_neg());
    }
    FxVec2 {
        x: Fx::from_bits(x),
        y: Fx::from_bits(y),
    }
}

/// Angle of `v`, or `None` for the zero vector.
#[must_use]
pub fn angle_of(v: FxVec2) -> Option<u16> {
    let (mut x, mut y) = (i128::from(v.x.to_bits()), i128::from(v.y.to_bits()));
    if x == 0 && y == 0 {
        return None;
    }
    // Scale up so the top bit sits at 62: precision for short vectors, and CORDIC's ~1.65x
    // growth still fits comfortably in i128.
    let top = x.unsigned_abs().max(y.unsigned_abs());
    let shift = top.leading_zeros().saturating_sub(65);
    (x, y) = (x.wrapping_shl(shift), y.wrapping_shl(shift));

    let mut z = 0_i64;
    if x < 0 {
        (x, y, z) = (x.wrapping_neg(), y.wrapping_neg(), HALF_TURN);
    }
    // Rotate toward the +x axis, accumulating the rotation.
    for (i, &a) in (0_u32..).zip(&ATAN) {
        let (dx, dy) = (y.wrapping_shr(i), x.wrapping_shr(i));
        if y > 0 {
            (x, y, z) = (x.wrapping_add(dx), y.wrapping_sub(dy), z.wrapping_add(a));
        } else {
            (x, y, z) = (x.wrapping_sub(dx), y.wrapping_add(dy), z.wrapping_sub(a));
        }
    }
    // Round to u16 turns; `& 0xFFFF` wraps negative angles into range.
    let turns = z.wrapping_add(1 << 15).wrapping_shr(16) & 0xFFFF;
    u16::try_from(turns).ok()
}

/// Signed shortest turn from `from` to `to`, in u16 turn units.
#[must_use]
pub const fn angle_diff(from: u16, to: u16) -> i16 {
    to.wrapping_sub(from).cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Within 2^-19 (~2e-6).
    fn close(a: Fx, b: i64) -> bool {
        a.to_bits().abs_diff(b.wrapping_shl(32)) < 1 << 13
    }

    #[test]
    fn unit_hits_the_axes() {
        for (angle, x, y) in [(0, 1, 0), (16384, 0, 1), (32768, -1, 0), (49152, 0, -1)] {
            let v = unit(angle);
            assert!(close(v.x, x) && close(v.y, y), "{angle}: {v:?}");
        }
    }

    #[test]
    fn angle_of_inverts_unit() {
        for angle in (0..=u16::MAX).step_by(97) {
            let back = angle_of(unit(angle)).unwrap();
            assert!(angle_diff(angle, back).abs() <= 1, "{angle} -> {back}");
        }
    }

    #[test]
    fn angle_of_handles_long_and_zero_vectors() {
        let big = Fx::from_num(1_000_000);
        assert_eq!(angle_of(FxVec2 { x: big, y: big }), Some(8192));
        assert_eq!(angle_of(FxVec2::default()), None);
    }
}
