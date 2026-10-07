// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1286 — `advice[slow-reference-parameter]` must not fire on a `&` parameter that
//! is FORWARDED to another function's `&` parameter.
//!
//! The lint asks whether the body reassigns the parameter, because whole-value
//! replacement is the one thing `&` buys (`formal/calls.md` F-ParamRebind / F-ParamRef).
//! A forwarder never reassigns — its callee does — so the one shape where the `&` carries
//! someone else's write-back looked exactly like a redundant one. Taking the advice
//! silently loses the write-back: the same program answers `[9,9]` with the `&` and `[0]`
//! without it, and nothing reports the difference.
//!
//! **A count, not a snapshot.** `make falsify` cannot score a diagnostic that must NOT
//! fire — there is no corpus channel for an absence — so the assertion here counts the
//! notices on stderr. And the count needs both directions in one file: a lint that never
//! fires at all would pass a suppression test on its own, so the true positive below is
//! what says the instrument is still live.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Compile `source` and return how many `slow-reference-parameter` notices it produced.
fn advice_count(name: &str, source: &str) -> usize {
    let dir = std::env::temp_dir().join(format!("loft-ref-fwd-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join(format!("{name}.loft"));
    fa::write(&file, source).expect("write probe");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft");
    assert!(
        out.status.success(),
        "{name} must compile and run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .matches("slow-reference-parameter")
        .count()
}

const TYPES: &str = "struct B { items: vector<integer> }\n\
                     fn replace_ref(b: &B) { b = B { items: [9, 9] }; }\n";

#[test]
fn a_forwarded_reference_is_not_redundant() {
    let n = advice_count(
        "forward",
        &format!(
            "{TYPES}\
             fn forward(b: &B) {{ replace_ref(b); }}\n\
             fn main() {{ a = B {{ items: [0] }}; forward(a); assert(a.items[0] == 9, \"kept\"); }}\n"
        ),
    );
    assert_eq!(n, 0, "a `&` forwarded to a `&` parameter is load-bearing");
}

/// Depth is not the axis — a reference stays load-bearing however far it is passed.
#[test]
fn a_forwarded_reference_is_not_redundant_at_depth() {
    let n = advice_count(
        "forward_deep",
        &format!(
            "{TYPES}\
             fn lvl2(b: &B) {{ replace_ref(b); }}\n\
             fn lvl1(b: &B) {{ lvl2(b); }}\n\
             fn main() {{ a = B {{ items: [0] }}; lvl1(a); assert(a.items[0] == 9, \"kept\"); }}\n"
        ),
    );
    assert_eq!(n, 0, "two forwards are still two load-bearing references");
}

/// The control. Without it the two tests above also pass on a lint that was deleted.
#[test]
fn a_reference_that_is_only_mutated_through_still_fires() {
    let n = advice_count(
        "redundant",
        "struct B { items: vector<integer> }\n\
         fn only_mutates(b: &B) { b.items += [1]; }\n\
         fn main() { a = B { items: [] }; only_mutates(a); assert(len(a.items) == 1, \"grew\"); }\n",
    );
    assert_eq!(
        n, 1,
        "a `&` that is only mutated through is still redundant"
    );
}

/// Forwarding to a PLAIN parameter does not make the `&` load-bearing, so the advice
/// stands there — the fix keys on the callee's parameter mode, not on "is a call present".
#[test]
fn forwarding_to_a_plain_parameter_does_not_excuse_the_reference() {
    let n = advice_count(
        "forward_plain_callee",
        "struct B { items: vector<integer> }\n\
         fn takes_plain(b: B) { b.items += [1]; }\n\
         fn forward(b: &B) { takes_plain(b); }\n\
         fn main() { a = B { items: [] }; forward(a); assert(len(a.items) == 1, \"grew\"); }\n",
    );
    assert_eq!(n, 1, "a plain callee needs no reference from its caller");
}
