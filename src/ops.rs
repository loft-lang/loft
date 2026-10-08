// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! Pure Rust scalar operations used by the bytecode executor (`fill.rs`) and
//! by native function implementations (`native.rs`).
//!
//! ## Naming conventions
//!
//! * `op_cast_X_from_Y`  — narrowing or lossy conversion (may truncate or clamp).
//!   Example: `op_cast_int_from_long` truncates a 64-bit value to 32 bits.
//! * `op_conv_X_from_Y`  — widening or safe conversion (no precision loss).
//!   Example: `op_conv_long_from_int` zero-extends a 32-bit integer to 64 bits.
//! * `op_negate_X`       — unary negation (single operand; not a minimum-of-two).
//! * `op_abs_X`          — absolute value.
//! * `op_<verb>_X`       — binary arithmetic (`add`, `min`, `mul`, `div`, `rem`, …).
#![allow(clippy::cast_precision_loss)]
#![allow(dead_code)]
use std::cmp::Ordering;

// ── the `--dev-soft-halt` overflow report (loft#1265) ────────────────────────────────────
//
// `(E-Report)` promises the flag surfaces the recoverable faults uniformly -- div0, overflow,
// OOB -- and overflow was the one it missed.  It is also the one with no other signal at all:
// div0 writes a Warn log at an undefended site and an overrun has its own, while overflow is
// silent everywhere by design, the null being the signal.  So the flag was the whole of its
// observability, and it was not answering.
//
// Reporting from here rather than from the ops' callers is what makes it free.  Overflow
// becomes the sentinel in exactly one place -- `checked_long!`'s `None` arm -- so the test
// that detects it is the branch that was already building `i64::MIN`.  The alternative, a
// `r == i64::MIN && v1 != i64::MIN && v2 != i64::MIN` test at the call sites, would put a new
// branch on the hottest ops in the language to learn what this arm already knows.
//
// A free function over process-level state, not a method on `Stores`: the native emitter
// inlines an op's `#rust` body into the surrounding expression, and a body writing through
// `stores` lands inside another `stores.` call's argument list (E0502).
static OVERFLOW_SURFACED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--dev-soft-halt` / `LOFT_DEV_SOFT_HALT=1`, read once per process.
fn dev_soft_halt() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var("LOFT_DEV_SOFT_HALT").is_ok_and(|v| v == "1" || v == "true"))
}

/// Whether a `--dev-soft-halt` run has surfaced an integer overflow, so the run can end
/// non-zero the way its div0 and out-of-bounds peers already do.  A triage flag whose only
/// fault exits 0 reports the run as clean.
#[must_use]
pub fn overflow_surfaced() -> bool {
    OVERFLOW_SURFACED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Surface one integer overflow under `--dev-soft-halt`.  Silent otherwise: `(E-Report)` keeps
/// overflow logless at every site, and this flag is a debugging tool rather than a mode, so it
/// prints instead of writing a log record.
///
/// `#[cold]` + `#[inline(never)]`: the caller is the arm that already had to exist, and keeping
/// this out of line is what stops it growing the ops' inlined bodies in generated code.
///
/// A `None` from `checked_div` / `checked_rem` is a division by zero as well as an overflow,
/// and a zero divisor is reported by the div0 path that owns it -- so this declines that case
/// rather than naming the same fault twice under two different words.
#[cold]
#[inline(never)]
pub fn note_integer_overflow(op: &str, v1: i64, v2: i64) {
    if !dev_soft_halt() {
        return;
    }
    if (op == "/" || op == "%") && v2 == 0 {
        return;
    }
    OVERFLOW_SURFACED.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::loft_eprintln!("soft-halt: integer overflow: {v1} {op} {v2}");
}

// @PLAN12 phase 3.5a (2026-05-24) — `RNG` thread-local + the
// associated `rand_int` / `rand_seed` / `shuffle_ints` helpers
// removed.  random's drain to lib/random/native/ makes the cdylib
// the single source of RNG state for both backends (interpreter
// dispatches via dlopen; native codegen via `loft::native_call`).
// `rand_pcg` is now only depended on transitively through
// lib/random/native/ — it stays in the loft crate's Cargo.toml
// `[dependencies]` only because the workspace doesn't yet share
// crate deps cleanly; safe to remove from src/Cargo.toml once the
// `random` feature flag is also retired.

/// C80 / E-Uncomp (formal/operational.md) — arithmetic overflow is
/// uncomputable, so the result is the integer null sentinel (`i64::MIN`) and
/// execution CONTINUES.  It does NOT trap.  This reverses C54.G ("overflow
/// traps, NOT a silent null"): under the spreadsheet model a value that can't
/// be computed shows null and the rest of the program runs on, identically in
/// development, test, and production (mode-independent).  The fault is silent by
/// default — the null itself is the visible signal; trace via the opt-in debug
/// log.  `checked_long_nullable!` is now behaviourally identical; both stay only
/// until the `??`-context op split (`OpAddIntNullable` / etc.) is retired.
/// The `$op` / `$v1` / `$v2` metavariables name the operands for
/// [`note_integer_overflow`], the `--dev-soft-halt` report.
///
/// That report rides the `None` arm `checked_*` already produces, so an
/// operation that does NOT overflow pays nothing for it: there is no added
/// test, only a branch that existed to build the sentinel.  Both backends
/// call these same functions -- `#rust"ops::op_add_int(@v1, @v2)"` -- so one
/// site covers the interpreter and `--native` alike.
macro_rules! checked_long {
    ($checked:expr, $op:expr, $v1:expr, $v2:expr) => {{
        match $checked {
            Some(r) => r,
            None => {
                $crate::ops::note_integer_overflow($op, $v1, $v2);
                i64::MIN
            }
        }
    }};
}

/// The GUARDED peer, and silent on purpose.  `(E-Report)` gives a defended site
/// -- the operand of `??`, or one a null-check follows -- the `*Nullable` op
/// precisely so it reports nothing, the guard having said how the null is
/// handled.  Its divide-by-zero half is already silent here; overflow is
/// silent for the same reason, so `--dev-soft-halt` surfaces exactly the
/// undefended faults on both halves rather than one rule per fault kind.
macro_rules! checked_long_nullable {
    ($checked:expr) => {{ $checked.unwrap_or(i64::MIN) }};
}

macro_rules! sentinel_long {
    ($expr:expr, $op:expr, $v1:expr, $v2:expr) => {{
        let r = $expr;
        #[cfg(debug_assertions)]
        assert!(
            r != i64::MIN,
            "long null-sentinel collision: {} {} {} = i64::MIN",
            $v1,
            $op,
            $v2
        );
        r
    }};
}

/// The character of `val` that holds byte `from` (a negative `from` counts back from the
/// end), or `'\0'` when `from` lies outside the text.
///
/// A byte below `0x80` is a whole character, so that case is answered here, inline: a
/// character-by-character walk of ASCII text costs one bounds test and one load per
/// character, in generated code as well as in the interpreter.  Every other byte belongs
/// to a multi-byte character and goes to [`text_character_wide`].
///
/// Enforces `@FR-R-Cold`: the snap-back and the decode are outlined so this test inlines.
#[inline]
#[must_use]
pub fn text_character(val: &str, from: i64) -> char {
    let len = val.len() as i64;
    let idx = if from < 0 { from + len } else { from };
    if idx < 0 || idx >= len {
        return char::from(0);
    }
    let b = val.as_bytes()[idx as usize];
    if b < 0x80 {
        return char::from(b);
    }
    text_character_wide(val, idx as usize)
}

/// The multi-byte character holding byte `idx` of `val`.  An index inside the character
/// snaps back to its first byte, so every byte of a character answers that character.
#[inline(never)]
fn text_character_wide(val: &str, mut idx: usize) -> char {
    let bytes = val.as_bytes();
    while bytes[idx] & 0xC0 == 0x80 && idx > 0 {
        idx -= 1;
    }
    val[idx..].chars().next().unwrap_or(char::from(0))
}

#[must_use]
pub fn sub_text(val: &str, from: i32, till: i32) -> &str {
    let size = val.len() as i32;
    let mut f = if from < 0 { from + size } else { from };
    let mut t = if till == i32::MIN {
        f + 1
    } else if till < 0 {
        till + size
    } else if till > size {
        size
    } else {
        till
    };
    if f < 0 || f > size || t < f || t > size {
        return "";
    }
    // when till is inside a UTF-8 token: increase it
    while t < size && !val.is_char_boundary(t as usize) {
        t += 1;
    }
    // when from is inside a UTF-8 token: decrease it
    while f > 0 && !val.is_char_boundary(f as usize) {
        f -= 1;
    }
    &val[f as usize..t as usize]
}

#[inline]
#[must_use]
pub fn to_char(val: i32) -> char {
    unsafe { char::from_u32_unchecked(val as u32) }
}

#[inline]
pub fn format_text(s: &mut String, val: &str, width: i64, dir: i8, token: u8) {
    // @PLN10 — render the text null sentinel ("\0") as `null`, mirroring
    // `format_long`'s `i64::MIN → null`.  A loft text equal to `STRING_NULL`
    // IS null (the content-based null model — `conv_bool_from_text`), so the
    // substitution is consistent with `?? ` / `!`.  Both backends route text
    // interpolation through this fn (interp `OpFormatText` + the native
    // `generation/text.rs` emitter), so this one site fixes both.
    let val = if val == crate::state::STRING_NULL {
        "null"
    } else {
        val
    };
    // dir=2 means "unset default"; text defaults to left-align (-1)
    let dir = if dir == 2 { -1 } else { dir };
    // @FR-F-Spec — a width is a MINIMUM field size, so anything at or below zero asks for
    // no padding.  Callers reach a negative one by arithmetic rather than by spelling it:
    // `format_prefixed` and `format_signed` subtract the sign or the `0x` marker they
    // emitted themselves, and a spec with no width at all starts that subtraction from 0.
    // `as usize` turned those into ~1.8e19 pad characters, which is an unbounded
    // allocation rather than a wrong string — `println("{-1:0}")` was OOM-killed.
    let mut tokens = width.max(0) as usize;
    for _ in val.chars() {
        if tokens == 0 {
            break;
        }
        tokens -= 1;
    }
    match dir.cmp(&0) {
        Ordering::Less => {
            *s += val;
            while tokens > 0 {
                s.push(token as char);
                tokens -= 1;
            }
        }
        Ordering::Greater => {
            while tokens > 0 {
                s.push(token as char);
                tokens -= 1;
            }
            *s += val;
        }
        Ordering::Equal => {
            let mut ct = 0;
            while ct < tokens / 2 {
                s.push(token as char);
                ct += 1;
            }
            *s += val;
            while ct < tokens {
                s.push(token as char);
                ct += 1;
            }
        }
    }
}
#[inline]
#[must_use]
pub fn op_abs_long(val: i64) -> i64 {
    if val == i64::MIN { val } else { val.abs() }
}

#[inline]
#[must_use]
pub fn op_negate_long(val: i64) -> i64 {
    if val == i64::MIN { val } else { -val }
}

#[inline]
#[must_use]
pub fn op_cast_int_from_long(val: i64) -> i64 {
    // Post-2c: integer IS i64.  Identity preserving i64::MIN as null.
    val
}

#[inline]
#[must_use]
pub fn op_cast_int_from_single(val: f32) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_cast_long_from_single(val: f32) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_cast_int_from_float(val: f64) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_cast_long_from_float(val: f64) -> i64 {
    if val.is_nan() { i64::MIN } else { val as i64 }
}

#[inline]
#[must_use]
pub fn op_conv_float_from_long(val: i64) -> f64 {
    if val == i64::MIN {
        f64::NAN
    } else {
        val as f64
    }
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_long(val: i64) -> bool {
    val != i64::MIN
}

#[inline]
#[must_use]
pub fn op_add_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_add(v2), "+", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_min_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_sub(v2), "-", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_mul_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long!(v1.checked_mul(v2), "*", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_div_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long!(v1.checked_div(v2), "/", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long!(v1.checked_rem(v2), "%", v1, v2)
    } else {
        i64::MIN
    }
}

// ── C54.G-hybrid — nullable long arithmetic ──────────────────────────────
// Sibling of op_add_int_nullable / etc. for the long path.

#[inline]
#[must_use]
pub fn op_add_long_nullable(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long_nullable!(v1.checked_add(v2))
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_min_long_nullable(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long_nullable!(v1.checked_sub(v2))
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_mul_long_nullable(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        checked_long_nullable!(v1.checked_mul(v2))
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_div_long_nullable(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long_nullable!(v1.checked_div(v2))
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_long_nullable(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN && v2 != 0 {
        checked_long_nullable!(v1.checked_rem(v2))
    } else {
        i64::MIN
    }
}

// ── O6: Non-null long variants ────────────────────────────────────────────
// Skip the i64::MIN sentinel check when both operands are known non-null
// (local variables with definite assignment).  Used by native codegen.

/// `@FR-R-Range`'s and `@FR-R-GuardedChain`'s falsifier (`LOFT_HOIST_VERIFY=1`): the plain
/// operator's answer beside the checked template's, compared at the operator.
///
/// # Panics
///
/// When the two answers differ — the proof or the guard admitted an operation that faults,
/// which is the defect this instrument exists to surface.  Never in the emitted default.
#[inline]
#[must_use]
pub fn range_verify(plain: i64, checked: i64, op: &'static str) -> i64 {
    assert!(
        plain == checked,
        "range proof: plain {op} answered {plain} where the checked form answers {checked} — the proof admitted an operation that faults"
    );
    plain
}

/// `LOFT_HOIST_VERIFY=1` inside a guarded plain nest (`@FR-R-BoundedNest`): the plain
/// operator's answer beside the checked template's, so a guard that admitted a nest whose
/// arithmetic faults is caught at the operator that faulted, not read off a wrong result.
///
/// # Panics
///
/// When the two answers differ — the nest's guard admitted an operation that faults, which
/// is the defect this instrument exists to surface.  Never in the emitted default.
#[inline]
#[must_use]
pub fn nest_verify(plain: i64, checked: i64, op: &'static str) -> i64 {
    assert!(
        plain == checked,
        "bounded nest: plain {op} answered {plain} where the checked form answers {checked} — the guard admitted an operation that faults"
    );
    plain
}

#[inline]
#[must_use]
pub fn op_add_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_add(v2), "+", v1, v2)
}

#[inline]
#[must_use]
pub fn op_min_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_sub(v2), "-", v1, v2)
}

#[inline]
#[must_use]
pub fn op_mul_long_nn(v1: i64, v2: i64) -> i64 {
    checked_long!(v1.checked_mul(v2), "*", v1, v2)
}

#[inline]
#[must_use]
pub fn op_div_long_nn(v1: i64, v2: i64) -> i64 {
    if v2 != 0 {
        checked_long!(v1.checked_div(v2), "/", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_rem_long_nn(v1: i64, v2: i64) -> i64 {
    if v2 != 0 {
        checked_long!(v1.checked_rem(v2), "%", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_neg_long_nn(v1: i64) -> i64 {
    checked_long!(v1.checked_neg(), "-", v1, 0)
}

#[inline]
#[must_use]
pub fn op_logical_and_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 & v2, "&", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_logical_or_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 | v2, "|", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_exclusive_or_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        sentinel_long!(v1 ^ v2, "^", v1, v2)
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_shift_left_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        // An out-of-range shift amount is C85-null (not an internal invariant violation), so
        // return the sentinel — debug and release then AGREE (release used to wrap by coincidence).
        if !(0..64).contains(&v2) {
            return i64::MIN;
        }
        // A left shift LEGITIMATELY produces i64::MIN (e.g. `1 << 63`), which IS the null sentinel
        // — loft treats it as null (C85), exactly what pln102-const-out-of-range expects. So do NOT
        // wrap in `sentinel_long!`: its `debug_assert!(r != i64::MIN)` is a false positive here
        // (fires under `-C debug-assertions=on` — the nightly Debug-assertions gate red). The bare
        // shift is identical in release (the assert compiles out); this only drops the bad assert.
        v1 << v2
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_shift_right_long(v1: i64, v2: i64) -> i64 {
    if v1 != i64::MIN && v2 != i64::MIN {
        // Out-of-range shift amount → C85-null (same as `<<`); a bare `v1 >> v2` would panic
        // in debug and wrap in release on an out-of-range amount.
        if !(0..64).contains(&v2) {
            return i64::MIN;
        }
        v1 >> v2
    } else {
        i64::MIN
    }
}

#[inline]
#[must_use]
pub fn op_abs_int(val: i64) -> i64 {
    op_abs_long(val)
}

#[inline]
#[must_use]
pub fn op_negate_int(val: i64) -> i64 {
    op_negate_long(val)
}

#[inline]
#[must_use]
pub fn op_conv_long_from_int(val: i64) -> i64 {
    // Post-2c: integer IS i64.  Identity.
    val
}

#[inline]
#[must_use]
pub fn op_conv_float_from_int(val: i64) -> f64 {
    op_conv_float_from_long(val)
}

#[inline]
#[must_use]
pub fn op_conv_single_from_int(val: i64) -> f32 {
    // Narrow i64 → f32 with null-preservation.
    if val == i64::MIN {
        f32::NAN
    } else {
        val as f32
    }
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_int(v: i64) -> bool {
    op_conv_bool_from_long(v)
}

#[inline]
#[must_use]
pub fn op_conv_bool_from_character(v: char) -> bool {
    // callers must read raw bytes via `char::from_u32(...).unwrap_or('\0')`
    // (handled in `create.rs::generate_code_to`), so an invalid bit pattern
    // — including the `i32::MIN` (0x80000000) coroutine-exhaustion sentinel
    // pushed by `push_null_value` for `iterator<character>` — is mapped to
    // `'\0'` *before* this function is called. The check below therefore only
    // needs to recognise the explicit null character. Reading raw stack bytes
    // directly as `char` would be undefined behaviour: Rust assumes every
    // `char` is a valid Unicode scalar value, and the release-mode optimiser
    // would constant-fold the sentinel check away.
    v != '\0'
}

/// The integer operators a fused interpreter op can carry (`OpIntVV` and its siblings in
/// `default/02_files.loft`), by the `kind` byte the bytecode generator writes.  Each arm is
/// the unfused operator's own function, so fusing an op changes how its operands arrive and
/// nothing about what it computes.
pub mod fused {
    pub const ADD: u8 = 0;
    pub const MIN: u8 = 1;
    pub const MUL: u8 = 2;
    pub const LAND: u8 = 3;
    pub const LOR: u8 = 4;
    pub const EOR: u8 = 5;
    pub const EQ: u8 = 0;
    pub const NE: u8 = 1;
    pub const LT: u8 = 2;
    pub const LE: u8 = 3;
    /// `@FR-R-Fuse`'s mirror clause — a comparison whose literal stood on the LEFT, mirrored so the local reads first:
    /// `c < v` is `v > c`, `c <= v` is `v >= c` — exact on `i64`, the null sentinel included.
    pub const GT: u8 = 4;
    pub const GE: u8 = 5;
}

/// The arithmetic `kind` of a fused op applied to `v1` and `v2` (`OpAddInt` … `OpEorInt`).
#[inline]
#[must_use]
pub fn fused_int(kind: u8, v1: i64, v2: i64) -> i64 {
    match kind {
        fused::ADD => op_add_int(v1, v2),
        fused::MIN => op_min_int(v1, v2),
        fused::MUL => op_mul_int(v1, v2),
        fused::LAND => op_logical_and_int(v1, v2),
        fused::LOR => op_logical_or_int(v1, v2),
        _ => op_exclusive_or_int(v1, v2),
    }
}

/// The comparison `kind` of a fused op applied to `v1` and `v2` (`OpEqInt` … `OpLeInt`).
#[inline]
#[must_use]
pub fn fused_cmp(kind: u8, v1: i64, v2: i64) -> bool {
    match kind {
        fused::EQ => v1 == v2,
        fused::NE => v1 != v2,
        fused::LT => v1 < v2,
        fused::GT => v1 > v2,
        fused::GE => v1 >= v2,
        _ => v1 <= v2,
    }
}

// C54.A (Phase 2c) — int arithmetic is now i64.  Functions forward to
// long counterparts; stdlib `#rust"ops::op_add_int(@v1, @v2)"` calls
// keep working unchanged because integer's Rust type is now i64.

#[inline]
#[must_use]
pub fn op_add_int(v1: i64, v2: i64) -> i64 {
    op_add_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_min_int(v1: i64, v2: i64) -> i64 {
    op_min_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_mul_int(v1: i64, v2: i64) -> i64 {
    op_mul_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_div_int(v1: i64, v2: i64) -> i64 {
    op_div_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_rem_int(v1: i64, v2: i64) -> i64 {
    op_rem_long(v1, v2)
}

// ── C54.G-hybrid — nullable integer arithmetic ────────────────────────────
// These mirror op_add_int / op_min_int / op_mul_int / op_div_int / op_rem_int
// but return `i32::MIN` on overflow or divide-by-zero instead of panicking.
// Emitted only when codegen detects the op's result is the immediate LHS of
// a `??` expression — the null is then caught and discharged.

#[inline]
#[must_use]
pub fn op_add_int_nullable(v1: i64, v2: i64) -> i64 {
    op_add_long_nullable(v1, v2)
}

#[inline]
#[must_use]
pub fn op_min_int_nullable(v1: i64, v2: i64) -> i64 {
    op_min_long_nullable(v1, v2)
}

#[inline]
#[must_use]
pub fn op_mul_int_nullable(v1: i64, v2: i64) -> i64 {
    op_mul_long_nullable(v1, v2)
}

#[inline]
#[must_use]
pub fn op_div_int_nullable(v1: i64, v2: i64) -> i64 {
    op_div_long_nullable(v1, v2)
}

#[inline]
#[must_use]
pub fn op_rem_int_nullable(v1: i64, v2: i64) -> i64 {
    op_rem_long_nullable(v1, v2)
}

#[inline]
#[must_use]
pub fn op_logical_and_int(v1: i64, v2: i64) -> i64 {
    op_logical_and_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_logical_or_int(v1: i64, v2: i64) -> i64 {
    op_logical_or_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_exclusive_or_int(v1: i64, v2: i64) -> i64 {
    op_exclusive_or_long(v1, v2)
}

/// Text ordering compare `<`.  Coerces both operands to `&str` so the
/// comparison works whether native codegen hands us a `&String` (a text
/// local) or a `&str` (an indexed `vector<text>` element via `get_str`).
/// A bare `&String < &str` fails to compile — `String: PartialOrd<str>`
/// doesn't exist — even though `==` works via the cross-type `PartialEq`.
/// Routing `OpLtText` / `OpLeText` through these helpers unifies both
/// provenances.  @P347.
///
/// `@FR-E-NullArg` — ordering places `null` at the LOW extreme for every ordered type, `text`
/// included: `null < v` is true, `v < null` false, `null < null` false.  The null text is the
/// one-byte `STRING_NULL` (`"\0"`), which a plain byte compare sorts ABOVE `""` and below
/// `"a"`, so `null < ""` answered false and `"" < null` true (loft#1924).  Asked here, the one
/// home both backends call, rather than at each operator site.
#[inline]
#[must_use]
pub fn op_lt_text<A: AsRef<str>, B: AsRef<str>>(a: A, b: B) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    let a_null = a == crate::state::STRING_NULL;
    let b_null = b == crate::state::STRING_NULL;
    if a_null || b_null {
        return a_null && !b_null;
    }
    a < b
}

/// Text ordering compare `<=` — see [`op_lt_text`], whose null order it shares: a null left
/// operand is `<=` everything, and nothing non-null is `<=` a null.
#[inline]
#[must_use]
pub fn op_le_text<A: AsRef<str>, B: AsRef<str>>(a: A, b: B) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    let a_null = a == crate::state::STRING_NULL;
    if a_null || b == crate::state::STRING_NULL {
        return a_null;
    }
    a <= b
}

/// Text concatenation's append step, `dst += src`, on both backends — `@FR-E-NullArg`: a null
/// operand makes the concatenation null.  A null `dst` (an accumulator that already met a null)
/// stays null, and a null `src` turns `dst` null — appending the sentinel's NUL byte made
/// `"a" + t` the two-byte `"a\0"`, a present text no null test sees (loft#1924).
#[inline]
pub fn append_text(dst: &mut String, src: &str) {
    if dst.as_str() == crate::state::STRING_NULL {
        return;
    }
    if src == crate::state::STRING_NULL {
        dst.clear();
        dst.push_str(crate::state::STRING_NULL);
        return;
    }
    dst.push_str(src);
}

/// Text equality — the `==` twin of [`op_lt_text`], and for the same reason.
///
/// Native codegen hands each operand in whatever shape its provenance produced:
/// `&String` for a text local, `&str` for an indexed `vector<text>` element, and
/// an OWNED `String` for a `??` null-coalescing block (which ends in
/// `.to_string()`).  A bare `@v1 == @v2` only compiles when the two shapes
/// happen to agree — `String == &String` has no `PartialEq` impl, so
/// `assert((v[i] ?? "") == t, …)` failed to native-compile with E0277 while the
/// identical expression bound to a local first compiled fine (#622).  Taking
/// `AsRef<str>` meets every provenance at `&str` at the call boundary, so the
/// generator needs no per-operand-shape rule.
#[inline]
#[must_use]
pub fn op_eq_text<A: AsRef<str>, B: AsRef<str>>(a: A, b: B) -> bool {
    a.as_ref() == b.as_ref()
}

/// Text inequality — see [`op_eq_text`].  @P347 / #622.
#[inline]
#[must_use]
pub fn op_ne_text<A: AsRef<str>, B: AsRef<str>>(a: A, b: B) -> bool {
    a.as_ref() != b.as_ref()
}

#[inline]
#[must_use]
pub fn op_shift_left_int(v1: i64, v2: i64) -> i64 {
    op_shift_left_long(v1, v2)
}

#[inline]
#[must_use]
pub fn op_shift_right_int(v1: i64, v2: i64) -> i64 {
    op_shift_right_long(v1, v2)
}

/// The character that pads a `null` out to its field.
///
/// @FR-F-Spec-Zero — a zero pad fills a NUMBER, and `null` is a sentinel, not a number:
/// F-Spec already draws that line for the sign flag ("`null` … takes none"), and the pad
/// is the same question. `0000null` reads as a numeric value and is a rendering of nothing,
/// so a null keeps the space pad whatever the spec asked for. Every other pad character is
/// field-shaping and applies unchanged.
fn null_pad(token: u8) -> u8 {
    if token == b'0' { b' ' } else { token }
}

/// Whether an already-rendered number is the null sentinel's text rather than digits.
fn is_null_text(res: &str) -> bool {
    res == "null"
}

/// The pseudo-radix that renders hexadecimal in UPPER case — `{255:X}` is `FF`.
///
/// @FR-F-Spec — the spec's radix field is already a render MODE wearing a radix's name
/// (`radix_for` answers `-1` for JSON and `1` for scientific), so upper-case hex takes a
/// value of its own rather than a second argument on four opcodes.  It has to be
/// distinguishable from `16`: `x` and `X` both mapped to `16` and the renderer wrote
/// `{val:x}`, so `X` — a spelling the compiler's own "use `x`, `X`, `b`, `o` or `d`"
/// diagnostic tells the reader to reach for — answered lower case with nothing saying so.
pub const HEX_UPPER: u8 = 17;

/// Emit `digits` behind a `prefix` — a sign, or a `0b` / `0o` / `0x` radix marker.
///
/// @FR-F-Spec-Zero — a ZERO pad fills the number, so the prefix stays in front of the
/// zeros it adds: `{-1:04}` is `-001`, not `0-01`, and `{255:#06x}` is `0x00ff`, not
/// `000xff`.  Every other pad token pads the rendering as a whole, so there the prefix
/// simply travels with the digits.  One home for both, because the decimal arm carried
/// the sign half alone and the three radix arms kept producing a prefix a reader cannot
/// paste back into a program.
fn format_prefixed(s: &mut String, prefix: &str, digits: &str, width: i64, dir: i8, token: u8) {
    if token == b'0' && !prefix.is_empty() {
        *s += prefix;
        format_text(s, digits, width - prefix.chars().count() as i64, dir, token);
        return;
    }
    let mut res = String::with_capacity(prefix.len() + digits.len());
    res += prefix;
    res += digits;
    format_text(s, &res, width, dir, token);
}

/**
Format an integer.
# Panics
When unknown radix values are asked.
*/
#[inline]
pub fn format_int(
    s: &mut String,
    val: i32,
    radix: u8,
    width: i64,
    token: u8,
    plus: bool,
    note: bool,
) {
    if val == i32::MIN {
        format_text(s, "null", width, 1, null_pad(token));
        return;
    }
    let mut res = String::new();
    let prefix = match radix {
        2 => {
            write!(res, "{val:b}").unwrap();
            if note { "0b" } else { "" }
        }
        8 => {
            write!(res, "{val:o}").unwrap();
            if note { "0o" } else { "" }
        }
        10 => {
            write!(res, "{}", val.abs()).unwrap();
            if val < 0 {
                "-"
            } else if plus {
                "+"
            } else {
                ""
            }
        }
        16 => {
            write!(res, "{val:x}").unwrap();
            if note { "0x" } else { "" }
        }
        HEX_UPPER => {
            write!(res, "{val:X}").unwrap();
            if note { "0x" } else { "" }
        }
        _ => panic!("Unknown radix"),
    };
    format_prefixed(s, prefix, &res, width, 1, token);
}

/**
Format a long integer.
# Panics
When unknown radix values are asked.
*/
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn format_long(
    s: &mut String,
    val: i64,
    radix: u8,
    width: i64,
    token: u8,
    plus: bool,
    note: bool,
    dir: i8,
) {
    // Numbers default to right-align; dir=-1 means "unset" from the parser
    // (left-align is the text default, but for numbers right-align is conventional).
    // Explicit `<` sets dir=-1, `^` sets dir=0, `>` sets dir=1.
    // We use dir=2 as "unset/default" from the parser, mapped to right-align here.
    let dir = if dir == 2 { 1 } else { dir };
    if val == i64::MIN {
        format_text(s, "null", width, dir, null_pad(token));
        return;
    }
    let mut res = String::new();
    let prefix = match radix {
        2 => {
            write!(res, "{val:b}").unwrap();
            if note { "0b" } else { "" }
        }
        8 => {
            write!(res, "{val:o}").unwrap();
            if note { "0o" } else { "" }
        }
        10 => {
            write!(res, "{}", val.abs()).unwrap();
            if val < 0 {
                "-"
            } else if plus {
                "+"
            } else {
                ""
            }
        }
        16 => {
            write!(res, "{val:x}").unwrap();
            if note { "0x" } else { "" }
        }
        HEX_UPPER => {
            write!(res, "{val:X}").unwrap();
            if note { "0x" } else { "" }
        }
        _ => panic!("Unknown radix"),
    };
    format_prefixed(s, prefix, &res, width, dir, token);
}

use std::fmt::Write as _;

#[allow(clippy::too_many_arguments)]
pub fn format_float(
    s: &mut String,
    val: f64,
    width: i64,
    precision: i64,
    token: u8,
    plus: bool,
    dir: i8,
) {
    let dir = if dir == 2 { 1 } else { dir };
    let mut res = String::new();
    // @PLN10 — NaN is the float null sentinel (`?? ` / `!` treat it as null) and
    // is not a JSON value; render it as `null`, mirroring text "\0" and integer
    // i64::MIN.  `inf`/`-inf` are real (non-null) values and render normally.
    if val.is_nan() {
        res.push_str("null");
    } else if precision >= 0 {
        write!(res, "{val:.p$}", p = precision as usize).unwrap();
    } else {
        write!(res, "{val}").unwrap();
    }
    sign_a_number(&mut res, plus);
    format_signed(s, &res, width, dir, token);
}

#[allow(clippy::too_many_arguments)]
pub fn format_single(
    s: &mut String,
    val: f32,
    width: i64,
    precision: i64,
    token: u8,
    plus: bool,
    dir: i8,
) {
    let dir = if dir == 2 { 1 } else { dir };
    let mut res = String::new();
    // @PLN10 — NaN is the float null sentinel; render as `null` (see `format_float`).
    if val.is_nan() {
        res.push_str("null");
    } else if precision >= 0 {
        write!(res, "{val:.p$}", p = precision as usize).unwrap();
    } else {
        write!(res, "{val}").unwrap();
    }
    sign_a_number(&mut res, plus);
    format_signed(s, &res, width, dir, token);
}

/// Pad an already-rendered and already-signed number, keeping a zero pad behind its sign.
///
/// @FR-F-Spec-Zero — the float twin of [`format_prefixed`]: a `-` or `+` is part of the number,
/// not of the digits a zero pad fills, so `{-3.5:08}` is `-00003.5`.  `null` (the NaN
/// sentinel) is not a number and takes no zero pad — `{nf:08}` is five spaces and `null`,
/// which is what `format_long` already answers for the integer sentinel.
fn format_signed(s: &mut String, res: &str, width: i64, dir: i8, token: u8) {
    if token == b'0'
        && !is_null_text(res)
        && let Some(sign) = res.strip_prefix(['-', '+'])
    {
        s.push(res.as_bytes()[0] as char);
        format_text(s, sign, width - 1, dir, token);
        return;
    }
    // `null_pad` applies to the SENTINEL only: a real number keeps whatever pad the spec
    // asked for, which is the whole point of the zero pad reaching these renderers.
    let pad = if is_null_text(res) {
        null_pad(token)
    } else {
        token
    };
    format_text(s, res, width, dir, pad);
}

/// Give an already-rendered number the `+` the format asked for.
///
/// Applied to the rendered TEXT rather than to the value, because that is the only form
/// that answers every case with one rule: `-0.0` and `-1e-9` already print a `-` while
/// comparing `>= 0.0` or rounding to `0.000` at the requested precision, and `null` (the
/// NaN sentinel) is not a number and takes no sign at all.  Before the width is applied,
/// so a signed number fills the field the same way an unsigned one does.
fn sign_a_number(res: &mut String, plus: bool) {
    if plus && !res.starts_with('-') && !res.starts_with("null") {
        res.insert(0, '+');
    }
}

#[must_use]
pub fn fix_from(from: i32, s: &str) -> usize {
    let size = s.len() as i32;
    let mut f = if from < 0 { from + size } else { from };
    if f < 0 {
        return 0;
    }
    let b = s.as_bytes();
    // when from is inside a UTF-8 token: decrease it
    while f > 0 && b[f as usize] >= 128 && b[f as usize] < 192 {
        f -= 1;
    }
    f as usize
}

#[must_use]
pub fn fix_till(till: i32, from: usize, s: &str) -> usize {
    let size = s.len() as i32;
    let mut t = if till == i32::MIN {
        from as i32 + 1
    } else if till < 0 {
        till + size
    } else if till > size {
        size
    } else {
        till
    };
    if t < from as i32 || t > size {
        return from;
    }
    let b = s.as_bytes();
    // when till is inside a UTF-8 token: increase it
    while t < size && b[t as usize] >= 128 && b[t as usize] < 192 {
        t += 1;
    }
    t as usize
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_layouts() {
        let mut s = String::new();
        format_text(&mut s, "aa", 5, 0, b'_');
        assert_eq!("_aa__", s);
        s.clear();
        format_text(&mut s, "aa", 6, 0, b'_');
        assert_eq!("__aa__", s);
        s.clear();
        format_int(&mut s, 0x1234, 16, 0, b' ', false, true);
        assert_eq!("0x1234", s);
        s.clear();
        format_long(&mut s, 0x123_4567, 16, 0, b' ', false, true, 1);
        assert_eq!("0x1234567", s);
        s.clear();
        format_int(&mut s, -1, 10, 3, b'0', false, false);
        assert_eq!("-01", s);
        s.clear();
        format_int(&mut s, -1, 10, 4, b'0', false, false);
        assert_eq!("-001", s);
        s.clear();
        format_long(&mut s, -1, 10, 3, b'0', false, false, 1);
        assert_eq!("-01", s);
        s.clear();
        format_int(&mut s, 1, 10, 3, b'0', true, false);
        assert_eq!("+01", s);
    }

    // --- T1-31: checked integer arithmetic tests ---

    #[test]
    fn add_int_normal() {
        assert_eq!(op_add_int(3, 4), 7);
    }

    #[test]
    fn add_int_null_propagation() {
        // Post-2c: integer IS i64.  Null sentinel is i64::MIN.
        assert_eq!(op_add_int(i64::MIN, 5), i64::MIN);
    }

    // C80 / E-Uncomp (formal/operational.md): integer `+`/`-`/`*` overflow is
    // uncomputable — the result is the null sentinel (i64::MIN) and execution
    // CONTINUES; it does NOT panic.  This reverses the old C54.G "overflow traps,
    // NOT a silent null" contract (these tests formerly asserted `should_panic`).
    #[test]
    fn add_int_overflow_is_null() {
        assert_eq!(op_add_int(i64::MAX, 1), i64::MIN);
    }

    #[test]
    fn sub_int_overflow_is_null() {
        assert_eq!(op_min_int(i64::MIN + 1, 2), i64::MIN);
    }

    #[test]
    fn mul_int_overflow_is_null() {
        assert_eq!(op_mul_int(i64::MAX, 2), i64::MIN);
    }

    // A computation whose exact result lands ON the sentinel reads back as null —
    // the accepted in-band-sentinel cost: (i64::MIN + 1) - 1 == i64::MIN.
    #[test]
    fn sub_int_onto_sentinel_is_null() {
        assert_eq!(op_min_int(-9_223_372_036_854_775_807, 1), i64::MIN);
    }

    // Bitwise ops keep `sentinel_long!` (a debug-only collision assert), untouched
    // by C80 — they cannot overflow, only land on the sentinel.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "long null-sentinel collision")]
    fn and_int_sentinel() {
        // Post-2c: int arithmetic forwards to long; the sentinel is i64::MIN.
        let _ = op_logical_and_int(i64::MIN + 1, i64::MIN + 2);
    }

    #[test]
    fn add_long_normal() {
        assert_eq!(op_add_long(100, 200), 300);
    }

    #[test]
    fn add_long_overflow_is_null() {
        assert_eq!(op_add_long(i64::MAX, 1), i64::MIN);
    }

    #[test]
    fn sub_long_onto_sentinel_is_null() {
        assert_eq!(op_min_long(i64::MIN + 1, 1), i64::MIN);
    }

    // `no_i64_sentinel_in_int_functions` removed in C54 Phase 2c.  Int
    // functions now legitimately reference `i64::MIN` (they forward to
    // the long equivalents), so the old compile-time guard is inverted.
}
