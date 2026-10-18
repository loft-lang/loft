// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `advice[omitted-field-zero]` beside the `(D-NoEnumF)` refusal.
//!
//! A struct literal that omits a field leaves it its type's zero, and the advice names that.
//! A BARE enum field has no zero to take — an enum's 0 is its null — so `@FR-D-NoEnumF`
//! refuses the literal outright, and the advice beside the error ("which takes the type's
//! zero") said the opposite of what happens.  The refusal is the suite's to pin
//! (`loft-suite/rules/types/D-NoEnumF/`); a notice that must NOT appear is visible to no channel
//! the suite scores, so it is asserted here, with a scalar field as the control that must still
//! advise.
//!
//! Binary-invoked like `tests/group_apart_lint.rs`: end-to-end compile diagnostics on stderr,
//! `LOFT_NO_CACHE` because a warm run skips the re-parse that produces them.

use loft::file_access as fa;
use std::path::PathBuf;

const CODE: &str = "advice[omitted-field-zero]";

fn diagnostics_of(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join("loft_omitted_field_lint");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, src).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_omitted_bare_enum_field_is_refused_without_the_zero_advice() {
    let err = diagnostics_of(
        "bare_enum",
        "enum C { Red, Green }\nstruct S { c: C, n: integer }\n\
         fn main() { s = S { n: 1 }; print(\"{s.n}\"); }\n",
    );
    assert!(
        err.contains("field `c: C` is an enum with no default"),
        "the (D-NoEnumF) refusal must fire:\n{err}"
    );
    assert!(
        !err.contains(CODE),
        "an omitted bare enum field takes no zero — the advice contradicts the refusal:\n{err}"
    );
}

#[test]
fn an_omitted_scalar_field_is_still_advised() {
    // The control: without it the silent case above is vacuous.
    let err = diagnostics_of(
        "scalar",
        "struct S { hover: integer, palette_pick: integer }\n\
         fn main() { s = S { hover: 3 }; print(\"{s.palette_pick}\"); }\n",
    );
    assert!(
        err.contains(CODE),
        "an omitted scalar field takes its zero:\n{err}"
    );
}
