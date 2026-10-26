// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

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

/// `@FR-F-Render`'s tie rule (@C156): append the shortest decimal that reads back as `v` — `{v}` —
/// with an exact TIE broken to the even last digit.  `core::fmt` gives the shortest length and
/// the nearest candidate of it, but breaks an exact tie upward in magnitude (flt2dec's shortest
/// mode rounds a half up); loft rounds every conversion between text and float ties-to-even,
/// IEEE 754's default, as `{v:.p}` and every parse already do.
pub fn push_shortest_f64(s: &mut String, v: f64) {
    use std::fmt::Write as _;
    let start = s.len();
    write!(s, "{v}").unwrap();
    if v.is_finite() && v != 0.0 {
        let bits = v.to_bits();
        let biased = ((bits >> 52) & 0x7ff) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        let (mant, exp) = if biased == 0 {
            (frac, -1074)
        } else {
            (frac | (1 << 52), biased - 1075)
        };
        if let Some(even) =
            tie_to_even(&s[start..], mant, exp, |t| t.parse::<f64>().ok() == Some(v))
        {
            s.truncate(start);
            s.push_str(&even);
        }
    }
}

/// [`push_shortest_f64`] for a `single`: its own shortest form, read back as an `f32`.
pub fn push_shortest_f32(s: &mut String, v: f32) {
    use std::fmt::Write as _;
    let start = s.len();
    write!(s, "{v}").unwrap();
    if v.is_finite() && v != 0.0 {
        let bits = v.to_bits();
        let biased = ((bits >> 23) & 0xff) as i32;
        let frac = u64::from(bits & ((1u32 << 23) - 1));
        let (mant, exp) = if biased == 0 {
            (frac, -149)
        } else {
            (frac | (1 << 23), biased - 150)
        };
        if let Some(even) =
            tie_to_even(&s[start..], mant, exp, |t| t.parse::<f32>().ok() == Some(v))
        {
            s.truncate(start);
            s.push_str(&even);
        }
    }
}

/// [`push_shortest_f64`] as a value.
#[must_use]
pub fn shortest_f64(v: f64) -> String {
    let mut s = String::new();
    push_shortest_f64(&mut s, v);
    s
}

/// [`push_shortest_f32`] as a value.
#[must_use]
pub fn shortest_f32(v: f32) -> String {
    let mut s = String::new();
    push_shortest_f32(&mut s, v);
    s
}

/// The even candidate where `shown` — `core::fmt`'s shortest text of `mant * 2^exp` — broke an
/// exact tie upward onto an odd digit, if it reads back; `None` otherwise (nearly always).
///
/// A tie needs the exact value to END one digit past the shortest form, in a 5.  With `mant`
/// made odd the exact value is `mant << exp` (no factor 5, so no trailing zero) or
/// `mant * 5^-exp / 10^-exp` (odd, so none either): its digits are those of one integer, and a
/// value whose integer passes 18 digits cannot end one past a form of at most 17.  The two
/// candidates are that integer's neighbours on the grid one digit up; the upper is what
/// `core::fmt` chose, and the lower is answered when it is the even one and reads back (just
/// below a power of two the lower half-gap is the narrower, and only the upper may read back).
fn tie_to_even(
    shown: &str,
    mant: u64,
    exp: i32,
    reads_back: impl Fn(&str) -> bool,
) -> Option<String> {
    let (neg, body) = shown
        .strip_prefix('-')
        .map_or((false, shown), |b| (true, b));
    if !body.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let digits: Vec<u8> = body
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let int_len = i32::try_from(body.find('.').unwrap_or(body.len())).ok()?;
    let lead = digits.iter().take_while(|&&d| d == 0).count();
    let last = digits.iter().rposition(|&d| d != 0)?;
    // An even last digit: whichever candidate this is, it is the even one already.
    if digits[last].is_multiple_of(2) {
        return None;
    }
    let mut shown_int: u128 = 0;
    for &d in &digits[lead..=last] {
        shown_int = shown_int.checked_mul(10)? + u128::from(d);
    }
    let shown_unit = int_len - 1 - i32::try_from(last).ok()?;
    // The exact value as `x * 10^x_unit`.
    let tz = mant.trailing_zeros();
    let (m, e) = (mant >> tz, exp + i32::try_from(tz).ok()?);
    let (x, x_unit): (u128, i32) = if e >= 0 {
        if e > 64 {
            return None;
        }
        (u128::from(m) << e, 0)
    } else {
        let n = e.unsigned_abs();
        if n > 27 {
            return None;
        }
        (u128::from(m) * 5u128.pow(n), e)
    };
    if x >= 1_000_000_000_000_000_000 || x % 10 != 5 {
        return None;
    }
    let cut = x_unit + 1;
    let (upper, lower) = ((x + 5) / 10, (x - 5) / 10);
    // `shown` must be the upper candidate, written at the cut's scale.
    let scale = u32::try_from(shown_unit - cut).ok()?;
    if shown_int.checked_mul(10u128.checked_pow(scale)?)? != upper || upper % 2 == 0 || lower == 0 {
        return None;
    }
    let text = plain_decimal(neg, lower, cut);
    reads_back(&text).then_some(text)
}

/// `q * 10^unit` as `core::fmt` lays a float out: no exponent, no trailing fractional zero.
fn plain_decimal(neg: bool, q: u128, unit: i32) -> String {
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let digits = q.to_string();
    if unit >= 0 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', unit.unsigned_abs() as usize));
        return out;
    }
    let frac = unit.unsigned_abs() as usize;
    let (int_part, frac_part) = if digits.len() > frac {
        let (a, b) = digits.split_at(digits.len() - frac);
        (a.to_string(), b.to_string())
    } else {
        (
            "0".to_string(),
            format!("{}{digits}", "0".repeat(frac - digits.len())),
        )
    };
    out.push_str(&int_part);
    let frac_part = frac_part.trim_end_matches('0');
    if !frac_part.is_empty() {
        out.push('.');
        out.push_str(frac_part);
    }
    out
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

    /// @C156 — the shortest form breaks an exact tie to the even digit, and otherwise answers
    /// what `core::fmt` answers.
    #[test]
    fn shortest_breaks_a_tie_to_even() {
        // 1549.40625 is exactly halfway between the 8-digit 1549.4062 and 1549.4063 as a single.
        // 49581 / 32 is 1549.40625 exactly.
        let tie = 49_581.0_f32 / 32.0;
        assert_eq!(shortest_f32(tie), "1549.4062");
        assert_eq!(shortest_f32(-tie), "-1549.4062");
        // Not ties: the nearest shortest form, as core::fmt writes it.
        for v in [
            0.1_f64,
            0.3,
            1.0 / 3.0,
            2.5,
            1e21,
            123.456,
            4.9e-324,
            1e-310,
            2f64.powi(53) + 2.0,
        ] {
            assert_eq!(shortest_f64(v), format!("{v}"), "{v:e}");
        }
        // A tie whose upper candidate is already even keeps it.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut ties = 0;
        for _ in 0..200_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Short binary fractions in the single range, where exact ties live.
            let v = (seed % 1_000_000) as f32 / 2f32.powi((seed >> 40) as i32 % 12);
            let got = shortest_f32(v);
            assert_eq!(got.parse::<f32>().unwrap(), v, "{v:e} reads back");
            let core = format!("{v}");
            if got != core {
                ties += 1;
                let last = got.bytes().rev().find(u8::is_ascii_digit).unwrap();
                assert_eq!((last - b'0') % 2, 0, "{v:e}: {got} is the even candidate");
            }
        }
        assert!(ties > 0, "the sample holds ties");
    }
}
