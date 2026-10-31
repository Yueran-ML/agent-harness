//! Half floats (IEEE 754 binary16), as a texture of `Rgba16Float` keeps them: an `f32` rounded to
//! the nearest, ties to even, and back. Written here rather than taken from a crate, for the two
//! functions that the tables need.

/// The largest finite half float.
pub const MAX: f32 = 65504.0;

/// The bits of the half float nearest `value` (ties to even). Beyond the largest half float it
/// is an infinity, and a NaN stays one.
#[must_use]
pub fn to_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = u16::try_from((bits >> 16) & 0x8000).unwrap_or(0);
    let abs = bits & 0x7fff_ffff;
    if abs >= 0x7f80_0000 {
        // An infinity, or a NaN (kept quiet).
        return sign | 0x7c00 | if abs > 0x7f80_0000 { 0x0200 } else { 0 };
    }
    // The biased exponent of a half.
    let exponent = i32::try_from(abs >> 23).unwrap_or(0) - 127 + 15;
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    if exponent <= 0 {
        // A subnormal half, or zero: the significand with its leading 1, shifted down to units
        // of 2^-24.
        let shift = u32::try_from(14 - exponent).unwrap_or(u32::MAX);
        if shift > 24 {
            return sign;
        }
        let significand = (abs & 0x007f_ffff) | 0x0080_0000;
        let mut half = significand >> shift;
        let rest = significand & ((1 << shift) - 1);
        let middle = 1 << (shift - 1);
        if rest > middle || (rest == middle && half & 1 == 1) {
            // Into the smallest normal where it carries, which is right.
            half += 1;
        }
        return sign | u16::try_from(half).unwrap_or(0);
    }
    let exponent = u32::try_from(exponent).unwrap_or(0);
    let mut half = (exponent << 10) | ((abs >> 13) & 0x03ff);
    let rest = abs & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        // Into the exponent where it carries, and to an infinity past the largest.
        half += 1;
    }
    sign | u16::try_from(half).unwrap_or(0x7c00)
}

/// The value of the half float of these bits.
#[must_use]
pub fn from_bits(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let significand = u32::from(bits & 0x03ff);
    match exponent {
        0 => {
            // 2^-24 is exact, and so is a significand of 10 bits times it.
            let magnitude = f32::from(u16::try_from(significand).unwrap_or(0)) * 2.0_f32.powi(-24);
            if sign == 0 {
                magnitude
            } else {
                -magnitude
            }
        }
        31 => f32::from_bits(sign | 0x7f80_0000 | (significand << 13)),
        _ => f32::from_bits(sign | ((exponent + 112) << 23) | (significand << 13)),
    }
}

/// `value` as a texture of half floats keeps it: clamped to the finite half floats (so that a
/// large value of a table is the largest a half holds, not an infinity), and rounded.
#[must_use]
pub fn round(value: f32) -> f32 {
    from_bits(to_bits(value.clamp(-MAX, MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_half_float_goes_to_an_f32_and_back() {
        for bits in 0..=u16::MAX {
            let value = from_bits(bits);
            if value.is_nan() {
                assert!(from_bits(to_bits(value)).is_nan());
            } else {
                assert_eq!(to_bits(value), bits, "{bits:#06x} {value}");
            }
        }
    }

    #[test]
    fn values_between_two_halves_go_to_the_nearer_and_ties_to_the_even() {
        // Each pair of neighbours among the positive finite halves, and the negatives alike.
        for bits in 0..0x7bff_u16 {
            for sign in [0, 0x8000] {
                let (low, high) = (bits | sign, (bits + 1) | sign);
                let (a, b) = (from_bits(low), from_bits(high));
                // Halves have 11 bits of significand, so their midpoint is an f32 exactly.
                let middle = f32::midpoint(a, b);
                let even = if low & 1 == 0 { low } else { high };
                assert_eq!(to_bits(middle), even, "the middle of {low:#06x}");
                let toward_a = f32::from_bits(middle.to_bits() - 1);
                let toward_b = f32::from_bits(middle.to_bits() + 1);
                let (near_a, near_b) = if a.abs() < b.abs() {
                    (toward_a, toward_b)
                } else {
                    (toward_b, toward_a)
                };
                assert_eq!(to_bits(near_a), low, "below the middle of {low:#06x}");
                assert_eq!(to_bits(near_b), high, "above the middle of {low:#06x}");
            }
        }
    }

    #[test]
    fn the_ends_are_infinities_zeros_and_the_largest() {
        assert_eq!(to_bits(65504.0), 0x7bff);
        assert_eq!(
            to_bits(65520.0),
            0x7c00,
            "the middle of the largest and 2^16"
        );
        assert_eq!(to_bits(65519.99), 0x7bff);
        assert_eq!(to_bits(1e9), 0x7c00);
        assert_eq!(to_bits(-1e9), 0xfc00);
        assert_eq!(to_bits(f32::INFINITY), 0x7c00);
        assert!(from_bits(to_bits(f32::NAN)).is_nan());
        assert_eq!(to_bits(2.0_f32.powi(-25)), 0, "a tie, to the even zero");
        assert_eq!(to_bits(2.0_f32.powi(-25) * 1.0001), 1);
        assert_eq!(to_bits(2.0_f32.powi(-26)), 0);
        assert_eq!(to_bits(-0.0), 0x8000);
        assert_eq!(to_bits(1.0), 0x3c00);
        assert_eq!(round(1e9), 65504.0);
        assert_eq!(round(-1e9), -65504.0);
        assert_eq!(round(0.1), 0.099_975_586);
    }
}
