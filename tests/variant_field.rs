// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#980 — a struct-enum field access that only SOME variants answer.
//!
//! `c.field` resolves at COMPILE time to the first variant declaring the name, and the
//! layout gives a shared name+type one slot — so the read is right for the variants that
//! declare it and reads another variant's bytes for the rest. The tag is never consulted,
//! on either backend: `a.n` on an `Anon` answers `Anon.k`'s value as if it were
//! `Named.n`, and `a.label = "x"` writes into a record whose tag still says `Anon`, after
//! which `match` still reports `Anon`.
//!
//! Direct payload access STAYS — [C89](../doc/claude/DESIGN_DECISIONS.md) decided
//! permanently that enum payloads are named fields you read straight, with matching for
//! *dispatch* and never for *extraction*, and rewriting that would force a matcher on
//! every read. The **silence** was the defect, and it is what this closes.
//!
//! The exemptions below are the ones that make it usable, and each is measured:
//!
//!   - **every variant declares the field** — one shared slot, any tag finds it. This is
//!     the common-prefix case C89 promises works, and it does: measured correct today
//!     even where the variants' preceding fields differ in width.
//!   - **`match` / `is`** — the bindings are per-arm, so they are not enum field accesses
//!     at all. They are also the cure the warning names, and a cure that warned would be
//!     worse than useless.
//!   - **a synthetic `__nullable<S>`** — its payload access is @PLN25's null model rather
//!     than a user-visible variant question.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const CODE: &str = "variant-field-unchecked";

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_980_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

/// Compile+run `src` and return everything it said, diagnostics included.
fn diagnostics(tag: &str, src: &str, env: &[(&str, &str)]) -> String {
    let path = write_temp(tag, src);
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const PARTIAL: &str = "enum Node { Named { label: text, n: integer }, Anon { k: integer } }\n\
fn main() {\n\
\x20 a: Node = Anon { k: 7 };\n\
\x20 print(\"{a.n}\\n\");\n\
}\n";

const PARTIAL_WRITE: &str = "enum Node { Named { label: text, n: integer }, Anon { k: integer } }\n\
fn main() {\n\
\x20 a: Node = Anon { k: 7 };\n\
\x20 a.label = \"written\";\n\
\x20 print(\"{a.k}\\n\");\n\
}\n";

/// Every variant declares `tag`, and they do NOT share a preceding field layout — the
/// case that would look like the defect and is not one.
const COMMON_PREFIX: &str = "enum T { A { p1: integer, p2: integer, p3: float, tag: text }, B { tag: text } }\n\
fn main() {\n\
\x20 b: T = B { tag: \"bee\" };\n\
\x20 a: T = A { p1: 1, p2: 2, p3: 3.0, tag: \"ay\" };\n\
\x20 print(\"{b.tag} {a.tag}\\n\");\n\
}\n";

const VIA_MATCH: &str = "enum Node { Named { label: text, n: integer }, Anon { k: integer } }\n\
fn main() {\n\
\x20 a: Node = Named { label: \"hi\", n: 1 };\n\
\x20 match a { Named { label, n } => print(\"{label}{n}\\n\"), Anon { k } => print(\"{k}\\n\") }\n\
\x20 if a is Named { label } { print(\"{label}\\n\") }\n\
}\n";

/// A `vector<S>` element is a `__nullable<S>`, so reading a field off one goes through
/// the very same resolver — and must stay quiet, or @PLN25's null model starts warning
/// about itself.
const NULLABLE_PAYLOAD: &str = "struct P { hp: integer, name: text }\n\
fn main() {\n\
\x20 v: vector<P> = [];\n\
\x20 v += [P { hp: 3, name: \"p\" }];\n\
\x20 e = v[0];\n\
\x20 print(\"{e.hp}\\n\");\n\
}\n";

// ── It fires where the value's variant decides the answer ───────────────────────────

#[test]
fn a_field_only_some_variants_declare_is_reported() {
    let out = diagnostics("partial", PARTIAL, &[]);
    assert!(
        out.contains(CODE),
        "reading `n` — declared by `Named` alone — off an `Anon` value answers another \
         variant's bytes, and must not do so silently (loft#980)\n{out}"
    );
    assert!(
        out.contains("Named") && out.contains('a'),
        "the message must name the variant that HAS the field and the value being read, \
         or the reader cannot tell which of the two to change\n{out}"
    );
}

/// The write is the more damaging half: it lands in another variant's bytes and leaves
/// the tag alone, so a later `match` still reports the original variant and nothing
/// downstream can notice.
#[test]
fn writing_a_field_the_variant_does_not_have_is_reported() {
    let out = diagnostics("write", PARTIAL_WRITE, &[]);
    assert!(
        out.contains(CODE),
        "writing `label` into an `Anon` value must be reported (loft#980)\n{out}"
    );
}

// ── And stays quiet where the access is answerable ──────────────────────────────────

#[test]
fn a_field_every_variant_declares_is_not_reported() {
    let out = diagnostics("prefix", COMMON_PREFIX, &[]);
    assert!(
        !out.contains(CODE),
        "every variant declares `tag`, so one slot holds it and any tag finds it — this \
         is the direct payload access C89 promises works, and warning on it would make \
         the diagnostic one to filter out\n{out}"
    );
    assert!(
        out.contains("bee ay"),
        "and it must still read correctly from both variants\n{out}"
    );
}

#[test]
fn match_and_is_bindings_are_not_reported() {
    let out = diagnostics("match", VIA_MATCH, &[]);
    assert!(
        !out.contains(CODE),
        "`match` / `is` bind per-arm and are the cure this warning names — a cure that \
         warns is worse than no cure\n{out}"
    );
}

#[test]
fn a_nullable_payload_access_is_not_reported() {
    let out = diagnostics("nullable", NULLABLE_PAYLOAD, &[]);
    assert!(
        !out.contains(CODE),
        "a `vector<S>` element is a synthetic `__nullable<S>`, and its payload access is \
         @PLN25's null model, not a user-visible variant question\n{out}"
    );
}

// ── The opt-out ────────────────────────────────────────────────────────────────────

#[test]
fn the_opt_out_silences_it() {
    let out = diagnostics("optout", PARTIAL, &[("LOFT_NO_VARIANT_FIELD", "1")]);
    assert!(
        !out.contains(CODE),
        "LOFT_NO_VARIANT_FIELD=1 must silence the warning\n{out}"
    );
}

/// The control for the harness: the quiet cells above prove nothing unless this same
/// helper can SEE the code when it is there.
#[test]
fn harness_can_see_the_code() {
    let noisy = diagnostics("control_noisy", PARTIAL, &[]);
    let quiet = diagnostics("control_quiet", COMMON_PREFIX, &[]);
    assert!(
        noisy.contains(CODE) && !quiet.contains(CODE),
        "the harness must distinguish a reported program from a clean one\n\
         noisy:\n{noisy}\nquiet:\n{quiet}"
    );
}
