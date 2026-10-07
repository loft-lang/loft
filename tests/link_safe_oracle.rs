// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 transparent-link widening — the SAFETY oracle guard (build step 2).
//!
//! Pins `use_analysis::link_safety_of`'s verdicts (surfaced by `LOFT_DUMP_LINK_SAFE`) against
//! Matrix S (`tests/scripts/link-safe-oracle.loft`), on BOTH backends. The oracle answers, per
//! copy-fill bind `a = s.v`, the conservative "would a shared-store LINK be UAF-safe?" question that
//! step 4 will combine with observability. It is SOUND BY CONSERVATISM (favours `unsafe`), so it can
//! never green-light a #415 dangle; this guard proves it says `safe` ONLY for the genuinely-safe
//! shape and `unsafe` (or not-a-candidate) for source-dead / reassigned / escaping ones.
//!
//! Report-only in step 2 — no codegen consumes it yet. See alias-where-correct-build.md § Matrix S.

use std::collections::HashMap;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/link-safe-oracle.loft")
}

/// `fn-name -> safe` for every `var=a` link-safe verdict the oracle dumps (the probe uses a distinct
/// `a` per cell). A cell absent from the map was not a link candidate at all (e.g. an escaping
/// return goes through the return-buffer path) — recorded as `None` by the callers below.
fn verdicts(backend: &str) -> HashMap<String, bool> {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(probe())
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_DUMP_LINK_SAFE", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("run loft");
    assert!(out.status.success(), "[{backend}] probe must run clean");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut m = HashMap::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("link-safe-dbg: ") else {
            continue;
        };
        let (mut f, mut is_a, mut safe) = (None, false, None);
        for tok in rest.split_whitespace() {
            if let Some(x) = tok.strip_prefix("fn=") {
                f = Some(x.to_string());
            } else if tok == "var=a" {
                is_a = true;
            } else if let Some(x) = tok.strip_prefix("safe=") {
                safe = x.parse::<u8>().ok().map(|n| n == 1);
            }
        }
        if is_a && let (Some(f), Some(s)) = (f, safe) {
            m.insert(f, s);
        }
    }
    m
}

fn assert_matrix_s(backend: &str) {
    let v = verdicts(backend);

    // SAFE (positive control) — the only genuinely-safe shape reads safe=1.
    assert_eq!(
        v.get("n_s1_safe"),
        Some(&true),
        "[{backend}] S1 (source outlives, local read-only, non-escaping) must be SAFE\n{v:?}"
    );
    // UNSAFE (negative controls) — a wrong `safe=1` here is a #415 UAF the oracle must never emit.
    assert_eq!(
        v.get("n_s2_source_dead"),
        Some(&false),
        "[{backend}] S2 (source dead after the bind) must be UNSAFE\n{v:?}"
    );
    assert_eq!(
        v.get("n_s3_source_reassigned"),
        Some(&false),
        "[{backend}] S3 (source reassigned) must be UNSAFE\n{v:?}"
    );
    // S4 (escaping return) must NEVER read safe=1 — either not a link candidate (absent, the
    // return-buffer path) or explicitly unsafe. A safe=1 would let an escaping link dangle.
    assert_ne!(
        v.get("n_s4_escape"),
        Some(&true),
        "[{backend}] S4 (local escapes via return) must NOT be SAFE\n{v:?}"
    );
    // S5 (self-append `a += a`, source == local) must be UNSAFE — a link would borrow itself and go
    // slotless (the codegen panic the widening's first gate-on run surfaced).
    assert_eq!(
        v.get("n_s5_self_append"),
        Some(&false),
        "[{backend}] S5 (self-append, source is the local) must be UNSAFE\n{v:?}"
    );

    // Non-vacuous: the oracle demonstrably emits BOTH verdicts on this corpus.
    assert!(
        v.values().any(|&s| s) && v.values().any(|&s| !s),
        "[{backend}] the oracle must emit both safe and unsafe on Matrix S (not vacuous)\n{v:?}"
    );
}

#[test]
fn matrix_s_interpret() {
    assert_matrix_s("--interpret");
}

#[test]
fn matrix_s_native() {
    assert_matrix_s("--native");
}
