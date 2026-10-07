// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2 / A4 — JSON / `text as vector<S>` deserialisation into nullable
//! struct elements.
//!
//! The canonical incoherence probe (see
//! `doc/claude/plans/25-nullable-sequences/README.md` § Decision): a `null`
//! inside a typed JSON array. Gate-on, a `vector<S>` element is the synthetic
//! `__nullable<S>` enum, so the cast must preserve the `null` as a real absent
//! element AND keep every present object's fields — the case enums cannot
//! cover (a deserialised `null` has no faithful enum target).
//!
//! Before A4(b) the present objects were dropped: the walker's `Parts::Enum`
//! arm rejected a bare struct object as an unknown variant tag, so the `?` in
//! the array loop aborted the parse at the first present element (`[{…},null]`
//! → len 0). The fix routes a bare object to the `Some` variant.
//!
//! Subprocess test (own process per case) so the process-global `LOFT_E2_SYNTH`
//! gate is set via the child env, never the test runner's. `LOFT_NO_CACHE=1` is
//! mandatory: the warm program cache keys on source content, NOT on the gate, so
//! a cached gate-off bundle would otherwise mask the rewrite.

use loft::file_access as fa;
use std::path::Path;
use std::process::Command;

/// Run the loft binary on `prog`, gate-on, cache-off, with extra `args`.
/// Returns (success, stdout).
fn run(args: &[&str], prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_E2_SYNTH", "1")
        .output()
        .expect("spawn loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn rustc_available() -> bool {
    Command::new("rustc").arg("--version").output().is_ok()
}

/// Write `body` to a temp `.loft` file under a unique dir, returning the path.
fn probe(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_e2_json_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, body).expect("write probe");
    path
}

/// The probe runs on the interpreter (always) and `--native` (when rustc is
/// present); `want` must appear in stdout on both. Cross-mode divergence is a
/// real hazard, so both backends are checked against the same expectation.
fn assert_both(name: &str, body: &str, want: &str) {
    let path = probe(name, body);

    let (ok, out) = run(&[], &path);
    assert!(ok, "{name}: interpret run failed; stdout={out:?}");
    assert!(
        out.contains(want),
        "{name}: interpret stdout missing {want:?}; got {out:?}"
    );

    if rustc_available() {
        let (ok, out) = run(&["--native"], &path);
        assert!(ok, "{name}: native run failed; stdout={out:?}");
        assert!(
            out.contains(want),
            "{name}: native stdout missing {want:?}; got {out:?}"
        );
    }
}

const STRUCT: &str = "struct Item { name: text, value: integer }\n";

#[test]
fn null_in_the_middle_preserved_present_objects_kept() {
    // The canonical probe: `[ {a}, null, {c} ]`. len 3, the null is a real
    // absent element, the present objects carry their fields.
    assert_both(
        "null_mid",
        &format!(
            "{STRUCT}fn main() {{\n  \
             v = \"[ {{{{name: \\\"a\\\", value: 1}}}}, null, {{{{name: \\\"c\\\", value: 3}}}} ]\" as vector < Item? >;\n  \
             print(\"len={{len(v)}} i0={{v[0].name}}/{{v[0].value}} i1null={{v[1] == null}} i2={{v[2].name}}/{{v[2].value}}\\n\");\n\
             }}\n"
        ),
        "len=3 i0=a/1 i1null=true i2=c/3",
    );
}

#[test]
fn null_leading_then_present() {
    assert_both(
        "null_lead",
        &format!(
            "{STRUCT}fn main() {{\n  \
             v = \"[ null, {{{{name: \\\"b\\\", value: 2}}}} ]\" as vector < Item? >;\n  \
             print(\"len={{len(v)}} i0null={{v[0] == null}} i1null={{v[1] == null}} i1v={{v[1].value}}\\n\");\n\
             }}\n"
        ),
        "len=2 i0null=true i1null=false i1v=2",
    );
}

#[test]
fn all_null_elements() {
    assert_both(
        "all_null",
        &format!(
            "{STRUCT}fn main() {{\n  \
             v = \"[ null, null, null ]\" as vector < Item? >;\n  \
             print(\"len={{len(v)}} all=[{{v[0] == null}},{{v[1] == null}},{{v[2] == null}}]\\n\");\n\
             }}\n"
        ),
        "len=3 all=[true,true,true]",
    );
}

#[test]
fn all_present_no_null() {
    // The all-present case is what the gate-off suite (`11-vectors`) exercises
    // dense; gate-on it must still carry every field through the `Some` wrap.
    assert_both(
        "all_present",
        &format!(
            "{STRUCT}fn main() {{\n  \
             v = \"[ {{{{name: \\\"a\\\", value: 1}}}}, {{{{name: \\\"b\\\", value: 2}}}} ]\" as vector < Item >;\n  \
             print(\"len={{len(v)}} i0={{v[0].name}}/{{v[0].value}} i1={{v[1].name}}/{{v[1].value}}\\n\");\n\
             }}\n"
        ),
        "len=2 i0=a/1 i1=b/2",
    );
}
