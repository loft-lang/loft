// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The math functions answer the routine they name, on both backends: `exp(v)` is the
//! exponential, not `pow(E, v)` — a power of the rounded E is another function, several units
//! in the last place away, and a C compiler rewrites one into the other at will, so the
//! interpreter, a hosted C unit and a whole-program C build answered apart.
//!
//! Each value is hand-computed (the correctly rounded e^v, against `pow(E, v)`'s): within one
//! unit in the last place of the first, several away from the second.
//!
//! @falsified-at: none — a SABOTAGE (2026-11-23): with `exp` back on `OpPowFloat(E, self)`
//!   (`default/01_code.loft`), `exp(10.0)` answers 22026.465794806703.  CHANNEL: the exit
//!   status.  WITNESS: "exp(10) is e^10".

mod common;

// e^10 = 22026.46579480671652…: the nearest double is 22026.465794806718, one unit 3.6e-12;
// `pow(E, 10.0)` answers 22026.465794806703, four units below.  e^-0.75 = 0.47236655274101470…
// (`pow` 0.47236655274101474); e^2.5 = 12.182493960703473… (`pow` 12.182493960703471).
cross_mode!(
    exp_is_the_exponential,
    r#"
    fn near(a: float, b: float, unit: float) -> boolean {
        d = a - b;
        if d < 0.0 { d = 0.0 - d; }
        d <= unit
    }

    fn test() {
        assert(near(exp(10.0), 22026.465794806718, 0.0000000000037), "exp(10) is e^10: {exp(10.0)}");
        assert(near(exp(0.0 - 0.75), 0.4723665527410147, 0.000000000000000056), "exp(-0.75): {exp(0.0 - 0.75)}");
        assert(near(exp(2.5), 12.182493960703473, 0.0000000000000018), "exp(2.5): {exp(2.5)}");
        assert(exp(0.0) == 1.0, "exp(0) is 1");
        println("ok");
    }
    "#
);
