// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Decimal numbers written straight into the text that receives them.
//!
//! `core::fmt` renders a number through `fmt::write`, a `Formatter` and `pad_integral` and
//! wants a `String` of its own to render into; the formatters in `ops` then copied that string
//! into the destination.  These writers append the digits in place.  They answer exactly what
//! `core::fmt` answers — `{v}` for an unsigned integer, `{v:.p}` for a float — so the rendered
//! text is the same whichever path wrote it; `push_fixed` declines (answers `false`) where its
//! exact arithmetic would not fit, and the caller falls back to `core::fmt` there.

/// `10^i` for every precision `push_fixed` renders.
const P10: [u128; 23] = {
    let mut t = [1u128; 23];
    let mut i = 1;
    while i < 23 {
        t[i] = t[i - 1] * 10;
        i += 1;
    }
    t
};

/// The highest precision `push_fixed` renders itself: `mant * 10^p` stays below `2^128`.
pub const MAX_FIXED_PRECISION: usize = 22;

/// Append `q` in decimal — `{q}`.
pub fn push_u64(s: &mut String, mut q: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (q % 10) as u8;
        q /= 10;
        if q == 0 {
            break;
        }
    }
    s.push_str(ascii(&buf[i..]));
}

fn push_u128(s: &mut String, mut q: u128) {
    if let Ok(small) = u64::try_from(q) {
        push_u64(s, small);
        return;
    }
    let mut buf = [0u8; 40];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (q % 10) as u8;
        q /= 10;
        if q == 0 {
            break;
        }
    }
    s.push_str(ascii(&buf[i..]));
}

fn ascii(digits: &[u8]) -> &str {
    // Every byte written above is an ASCII digit.
    std::str::from_utf8(digits).unwrap_or_default()
}

/// Append `{v:.precision$}` exactly as `core::fmt` renders it, or answer `false` and append nothing.
///
/// `core::fmt` rounds the EXACT binary value of `v` to `precision` decimals, an exact tie to even.
/// Here `v = mant * 2^exp` and the rounded value `round(v * 10^precision)` is computed in integers:
/// for `exp < 0` as `(mant * 10^precision) >> -exp` with the shifted-out bits deciding the rounding,
/// so no floating-point step can round differently.  A value whose integer part would not fit
/// 128 bits, a precision over `MAX_FIXED_PRECISION`, and a non-finite value decline.
pub fn push_fixed(s: &mut String, v: f64, precision: usize) -> bool {
    if !v.is_finite() || precision > MAX_FIXED_PRECISION {
        return false;
    }
    let bits = v.to_bits();
    let neg = bits >> 63 != 0;
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mant, exp) = if biased == 0 {
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), biased - 1075)
    };
    let rounded: u128 = if mant == 0 {
        0
    } else if exp >= 0 {
        if exp > 74 {
            return false;
        }
        match (u128::from(mant) << exp).checked_mul(P10[precision]) {
            Some(x) => x,
            None => return false,
        }
    } else {
        let shift = exp.unsigned_abs();
        let scaled = u128::from(mant) * P10[precision];
        if shift >= 128 {
            // scaled < 2^127 <= half of 2^shift: rounds to zero.
            0
        } else {
            let whole = scaled >> shift;
            let rest = scaled & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            if rest > half || (rest == half && whole & 1 == 1) {
                whole + 1
            } else {
                whole
            }
        }
    };
    if neg {
        s.push('-');
    }
    push_u128(s, rounded / P10[precision]);
    if precision > 0 {
        s.push('.');
        let mut fraction = rounded % P10[precision];
        let mut buf = [b'0'; MAX_FIXED_PRECISION];
        let mut i = precision;
        while i > 0 {
            i -= 1;
            buf[i] = b'0' + (fraction % 10) as u8;
            fraction /= 10;
        }
        s.push_str(ascii(&buf[..precision]));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// The fast writers against `core::fmt` itself, byte for byte.
    #[test]
    fn fixed_matches_core_fmt() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let check = |v: f64, p: usize| {
            let mut want = String::new();
            write!(want, "{v:.p$}").unwrap();
            let mut got = String::from("x");
            if push_fixed(&mut got, v, p) {
                assert_eq!(&got[1..], want, "{v:e} at precision {p}");
            } else {
                assert_eq!(got, "x");
            }
            // f32 renders its exact value, which the widened f64 holds unchanged.
            let narrow = v as f32;
            let mut want32 = String::new();
            write!(want32, "{narrow:.p$}").unwrap();
            let mut got32 = String::new();
            if push_fixed(&mut got32, f64::from(narrow), p) {
                assert_eq!(got32, want32, "{narrow:e}f32 at precision {p}");
            }
        };
        for v in [
            0.0,
            -0.0,
            0.25,
            0.35,
            2.5,
            3.5,
            -2.5,
            0.05,
            0.5,
            -0.5,
            -0.01,
            1.005,
            2.675,
            5e-324,
            1e-300,
            1e22,
            1.5e22,
            9.5,
            123_456.789,
        ] {
            for p in 0..=MAX_FIXED_PRECISION {
                check(v, p);
            }
        }
        for _ in 0..200_000 {
            check(f64::from_bits(next()), (next() % 23) as usize);
            let w = ((next() % 2_000_000) as f64 - 1_000_000.0) / ((next() % 1000 + 1) as f64);
            check(w, (next() % 6) as usize);
            let h = ((next() % 100_000) as f64 + 0.5)
                / [1.0, 10.0, 100.0, 2.0, 4.0, 8.0][(next() % 6) as usize];
            check(h, (next() % 4) as usize);
        }
    }

    #[test]
    fn integers_match_core_fmt() {
        for v in [0, 1, 9, 10, 99, 100, 12_345, u64::MAX, u64::MAX / 10] {
            let mut got = String::new();
            push_u64(&mut got, v);
            assert_eq!(got, v.to_string());
        }
    }
}
