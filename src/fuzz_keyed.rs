// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I90 — Shared utilities & data structures (test/fuzz harness support; @PLN53).
//
//! @PLN53 F2 — keyed-container program generator (F2.1).
//!
//! Generates valid-by-construction, self-checking loft programs that exercise
//! the schema-coupled keyed collections (`hash` / `sorted` / `index`), the way
//! `program_ownership` does for the ownership grammar. Reified as a library so
//! `cargo test` drives generation over the spec space on stable; the libfuzzer
//! target (F2.5) is a shim over [`generate_keyed`].
//!
//! Contract: `doc/claude/plans/53-program-level-fuzzing/F2-DESIGN.md`. Keys are
//! DISTINCT by construction (drawn from an indexed pool), so one invariant is
//! uniform across all three types: the collection is exactly the KEY→VALUE map
//! the op sequence defines — population equals the surviving-key count, every
//! surviving key looks up its value, removed/absent keys look up `null`, and
//! `for` visits exactly the survivors in the declared key order.
//!
//! The brittleness cure (F2-DESIGN.md § re-assertion sites): the survivor set
//! is computed ONCE and drives both the emitted statements and the baked
//! assertions — one model, no drift.

use crate::compile::byte_code;
use crate::fuzz_oracle::stdlib;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

/// Which keyed collection a spec targets. The grammar is parameterized per type
/// (F2-DESIGN.md § over-unification guard): `index` has a multi-part key and no
/// `.len()`; all three share the distinct-key map invariant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hash,
    Sorted,
    Index,
}

/// A generated keyed-collection program: insert `n_keys` distinct keys, then
/// remove the indices in `remove`. When `closures` is set, the program also
/// folds the collection through lambdas (the closure / overlapping-lifetime
/// axis — F2.3, stresses the slot allocator).
#[derive(Clone, Debug)]
pub struct KeyedSpec {
    pub kind: Kind,
    pub n_keys: u32,
    pub remove: Vec<u32>,
    pub closures: bool,
}

/// The distinct value stamped for key index `i` (injective, so a lookup pins
/// the exact key→value mapping).
fn value_of(i: u32) -> i64 {
    i64::from(i) * 7 + 1
}

/// Zero-padded key text for `hash`/`sorted`, so lexicographic order equals index
/// order (keeps the expected iteration order trivial to compute).
fn key_text(i: u32) -> String {
    format!("k{i:03}")
}

/// Emit the shared prefix — struct decls, `fn {entry}()` open, inserts, then
/// removes — plus `(n, removed, survivors)` for the caller's tail. Shared by
/// [`generate_keyed`] (self-check asserts) and [`generate_keyed_summary`]
/// (print). The survivor set is computed once here and returned, so both tails
/// read the same model (F2-DESIGN.md § re-assertion sites).
fn build_prelude(spec: &KeyedSpec, entry: &str) -> (String, u32, BTreeSet<u32>, Vec<u32>) {
    let n = spec.n_keys.clamp(1, 20);
    let removed: BTreeSet<u32> = spec.remove.iter().copied().filter(|&i| i < n).collect();
    let survivors: Vec<u32> = (0..n).filter(|i| !removed.contains(i)).collect();

    let mut s = String::new();
    match spec.kind {
        Kind::Index => {
            s.push_str("struct E { n: integer, k: text, v: integer }\n");
            s.push_str("struct C { m: index<E[n, k]> }\n");
        }
        Kind::Hash => {
            s.push_str("struct E { k: text, v: integer }\n");
            s.push_str("struct C { m: hash<E[k]> }\n");
        }
        Kind::Sorted => {
            s.push_str("struct E { k: text, v: integer }\n");
            s.push_str("struct C { m: sorted<E[k]> }\n");
        }
    }
    let _ = writeln!(s, "fn {entry}() {{");
    s.push_str("  c = C { m: [] };\n");

    // Inserts — all n keys, in index order.
    for i in 0..n {
        match spec.kind {
            Kind::Index => {
                let _ = writeln!(s, "  c.m += [E{{n:{i}, k:\"x\", v:{}}}];", value_of(i));
            }
            _ => {
                let _ = writeln!(
                    s,
                    "  c.m += [E{{k:\"{}\", v:{}}}];",
                    key_text(i),
                    value_of(i)
                );
            }
        }
    }
    // Removes.
    for &i in &removed {
        match spec.kind {
            Kind::Index => {
                let _ = writeln!(s, "  c.m[{i}, \"x\"] = null;");
            }
            _ => {
                let _ = writeln!(s, "  c.m[\"{}\"] = null;", key_text(i));
            }
        }
    }
    (s, n, removed, survivors)
}

/// Build a valid-by-construction, self-checking program (`fn main`) for `spec`.
pub fn generate_keyed(spec: &KeyedSpec) -> String {
    let (mut s, n, removed, survivors) = build_prelude(spec, "main");
    let pop = survivors.len();

    // Population + iteration order (a `;`-separated string, unambiguous for any n).
    s.push_str("  order: text = \"\";\n  cnt: integer = 0;\n");
    match spec.kind {
        Kind::Index => s.push_str("  for e in c.m { cnt += 1; order += \"{e.n};\"; }\n"),
        _ => s.push_str("  for e in c.m { cnt += 1; order += e.k + \";\"; }\n"),
    }
    let _ = writeln!(s, "  assert(cnt == {pop}, \"pop {{cnt}}\");");
    // `index` has no `.len()`; hash/sorted get the extra len() check too.
    if spec.kind != Kind::Index {
        let _ = writeln!(s, "  assert(c.m.len() == {pop}, \"len {{c.m.len()}}\");");
    }

    // Per-key lookups: survivors return their value, removed keys return null.
    for i in 0..n {
        let present = !removed.contains(&i);
        match (spec.kind, present) {
            (Kind::Index, true) => {
                let _ = writeln!(
                    s,
                    "  assert(c.m[{i}, \"x\"].v == {}, \"lk{i}\");",
                    value_of(i)
                );
            }
            (Kind::Index, false) => {
                let _ = writeln!(s, "  assert(!c.m[{i}, \"x\"], \"rm{i}\");");
            }
            (_, true) => {
                let _ = writeln!(
                    s,
                    "  assert(c.m[\"{}\"].v == {}, \"lk{i}\");",
                    key_text(i),
                    value_of(i)
                );
            }
            (_, false) => {
                let _ = writeln!(s, "  assert(!c.m[\"{}\"], \"rm{i}\");", key_text(i));
            }
        }
    }

    // Iteration order: survivors in index order (= declared key order for all
    // three types under this key scheme).
    let expected: String = survivors
        .iter()
        .map(|&i| match spec.kind {
            Kind::Index => format!("{i};"),
            _ => format!("{};", key_text(i)),
        })
        .collect();
    let _ = writeln!(s, "  assert(order == \"{expected}\", \"order {{order}}\");");

    // F2.3 closure / overlapping-lifetime axis: fold the collection through two
    // lambdas kept live alongside the base locals (c, order, cnt) — extra
    // simultaneously-live slots pressure the allocator. Self-check the folded
    // sum, max, and count against values baked from the same survivor set.
    if spec.closures {
        let expected_sum: i64 = survivors.iter().map(|&i| value_of(i)).sum();
        let expected_max: i64 = survivors.iter().map(|&i| value_of(i)).max().unwrap_or(0);
        s.push_str("  sumfn = fn(a: integer, b: integer) -> integer { a + b };\n");
        s.push_str(
            "  maxfn = fn(a: integer, b: integer) -> integer { if a > b { a } else { b } };\n",
        );
        s.push_str("  total: integer = 0;\n  peak: integer = 0;\n  ccnt: integer = 0;\n");
        s.push_str(
            "  for e in c.m { total = sumfn(total, e.v); peak = maxfn(peak, e.v); \
             ccnt = sumfn(ccnt, 1); }\n",
        );
        let _ = writeln!(s, "  assert(total == {expected_sum}, \"csum {{total}}\");");
        let _ = writeln!(s, "  assert(peak == {expected_max}, \"cmax {{peak}}\");");
        let _ = writeln!(s, "  assert(ccnt == {pop}, \"ccnt {{ccnt}}\");");
    }
    s.push_str("}\n");
    s
}

/// Build a program that PRINTS the canonical summary — population, then the
/// surviving `key=value;` pairs in declared key order — instead of
/// self-asserting. This is the F3 deterministic-output subset (F3-DESIGN.md):
/// the stdout is a pure function of the program's semantics, so every backend
/// must print it byte-identically. Emits `fn test()` for `run_cross_mode`,
/// which appends `fn main() { test(); }`. Population is counted in the loop, so
/// this is uniform across all three types (`index` has no `.len()`).
pub fn generate_keyed_summary(spec: &KeyedSpec) -> String {
    let (mut s, _n, _removed, _survivors) = build_prelude(spec, "test");
    s.push_str("  cnt: integer = 0;\n  out: text = \"\";\n");
    match spec.kind {
        Kind::Index => s.push_str("  for e in c.m { cnt += 1; out += \"{e.n}={e.v};\"; }\n"),
        _ => s.push_str("  for e in c.m { cnt += 1; out += \"{e.k}={e.v};\"; }\n"),
    }
    if spec.closures {
        // Fold through lambdas and print the deterministic sum/max — puts the
        // closure / slot-allocator path in the cross-backend diff too (F3.2).
        s.push_str("  sumfn = fn(a: integer, b: integer) -> integer { a + b };\n");
        s.push_str(
            "  maxfn = fn(a: integer, b: integer) -> integer { if a > b { a } else { b } };\n",
        );
        s.push_str("  total: integer = 0;\n  peak: integer = 0;\n");
        s.push_str("  for e in c.m { total = sumfn(total, e.v); peak = maxfn(peak, e.v); }\n");
        s.push_str("  print(\"pop={cnt};{out}fold={total},{peak};\");\n");
    } else {
        s.push_str("  print(\"pop={cnt};{out}\");\n");
    }
    s.push_str("}\n");
    s
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Run one generated program in-process. `Ok(())` means it compiled, ran, and
/// every self-check passed. A compiler panic propagates untouched — the
/// observer (the F2.1 test / libfuzzer) records it.
///
/// # Errors
/// Returns `Err` when the program is rejected at parse (a GENERATOR bug —
/// valid-by-construction should always compile) or when a self-check assertion
/// / runtime fault fires (`had_fatal`), which is a real FINDING.
pub fn check_generated(src: &str) -> Result<(), String> {
    check_generated_with(src, false)
}

/// As [`check_generated`], with `poison` selecting F4's arena poison-on-free
/// amplifier. On, a keyed-collection use-after-free reads a loud sentinel
/// instead of silently-lucky stale data, so it either fails a self-check
/// (returned as `Err`) or aborts (uncatchable SIGSEGV) — see F2.4.
///
/// # Errors
/// Same as [`check_generated`].
pub fn check_generated_with(src: &str, poison: bool) -> Result<(), String> {
    let (data, db) = stdlib();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;

    let tmp = std::env::temp_dir().join(format!(
        "loft_f2_{}_{}.loft",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, src).is_err() {
        return Ok(()); // an I/O error is a harness condition, not a finding
    }
    let path = tmp.to_string_lossy().to_string();
    p.parse(&path, false);
    let _ = std::fs::remove_file(&tmp);

    let rejected = !p.diagnostics.is_empty()
        && p.diagnostics.lines().iter().any(|l| {
            // "not a warning" means "an error" here, so a coded diagnostic that no
            // prefix match recognised used to read as a REJECTION (@PLN131).
            !matches!(
                crate::diagnostics::compact_level(l),
                Some(crate::diagnostics::Level::Warning | crate::diagnostics::Level::Debug)
            )
        });
    if rejected {
        return Err(format!(
            "GENERATOR BUG (rejected): {}\n--- program ---\n{src}",
            p.diagnostics.lines().join(" | ")
        ));
    }

    let mut data = p.data;
    let mut database = p.database;
    scopes::check(&mut data, &mut database);
    let mut state = State::new(database);
    byte_code(&mut state, &mut data);
    state.database.poison_free = poison;
    state.execute("main", &data);
    while state.database.frame_yield {
        state.resume();
    }
    if state.database.had_fatal {
        let msg = state
            .database
            .runtime_error
            .as_ref()
            .map_or_else(|| "runtime fault".to_string(), |e| e.message.clone());
        return Err(format!(
            "FINDING (self-check failed): {msg}\n--- program ---\n{src}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// A representative set of remove patterns for `n` keys: none, first, last,
    /// a middle key, all-but-first, and all.
    fn remove_patterns(n: u32) -> Vec<Vec<u32>> {
        let mut v = vec![vec![], vec![0]];
        if n > 1 {
            v.push(vec![n - 1]);
            v.push(vec![n / 2]);
            v.push((1..n).collect()); // all but first
            v.push((0..n).collect()); // all
        }
        v
    }

    /// Generate and run every spec in the enumeration. Returns
    /// `(total, failures)`; a `poison`-mode use-after-free that aborts
    /// (uncatchable SIGSEGV) kills the process instead — F2.4's expected shape
    /// for a real store bug.
    fn run_sweep(poison: bool) -> (usize, Vec<String>) {
        let mut failures: Vec<String> = Vec::new();
        let mut total = 0;
        for closures in [false, true] {
            for kind in [Kind::Hash, Kind::Sorted, Kind::Index] {
                for &n in &[1u32, 2, 5, 10] {
                    for remove in remove_patterns(n) {
                        let spec = KeyedSpec {
                            kind,
                            n_keys: n,
                            remove,
                            closures,
                        };
                        let src = generate_keyed(&spec);
                        total += 1;
                        match catch_unwind(AssertUnwindSafe(|| check_generated_with(&src, poison)))
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(msg)) => failures.push(format!(
                                "{kind:?} n={n} cl={closures}: {}",
                                msg.lines().next().unwrap_or("")
                            )),
                            Err(_) => failures.push(format!(
                                "{kind:?} n={n} cl={closures}: PANIC (compiler ICE)"
                            )),
                        }
                    }
                }
            }
        }
        (total, failures)
    }

    /// F2.1 exit: every generated program compiles, runs, and passes its
    /// self-checks on the interpreter. A rejection is a generator bug; a failed
    /// self-check or a compiler panic is a finding.
    #[test]
    fn generated_programs_compile_run_and_selfcheck() {
        let (total, failures) = run_sweep(false);
        eprintln!(
            "F2.1: generated + ran {total} keyed programs, {} failure(s)",
            failures.len()
        );
        assert!(
            total > 30,
            "too few specs generated ({total}) — enumeration broken"
        );
        assert!(
            failures.is_empty(),
            "{}/{total} generated programs failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// F2.4 — run the same sweep with arena poison-on-free ON, so a
    /// keyed-collection use-after-free surfaces as a failed self-check (a loud
    /// garbage lookup value) rather than silently-lucky stale data. A pointer
    /// deref into poisoned memory aborts (uncatchable SIGSEGV) and kills the
    /// binary — that is a real finding to triage, exactly like F1's `walk.loft`.
    #[test]
    fn generated_programs_clean_under_poison() {
        let (total, failures) = run_sweep(true);
        eprintln!(
            "F2.4: ran {total} keyed programs under poison, {} finding(s)",
            failures.len()
        );
        assert!(
            failures.is_empty(),
            "{}/{total} programs faulted under poison (keyed-collection UAF?):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// F2.6 — one wide triage pass: a large, seeded-random spec space run under
    /// poison. Approximates a coverage-guided run on stable (no nightly /
    /// cargo-fuzz): the fixed F2.1/F2.4 grid is only 120 specs; this explores
    /// ~1500 varied ones (all key counts 1..=20, random remove subsets, both
    /// closure modes). A failed self-check fails the test; a poison UAF aborts,
    /// naming the culprit spec under `LOFT_F2_TRACE`. `#[ignore]` (heavy) — run
    /// with `--ignored`. Reproducible: fixed PRNG seed.
    #[test]
    #[ignore = "F2.6 wide poison sweep — heavy; runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --lib fuzz_keyed::tests::f26_wide_poison_sweep -- --ignored`"]
    fn f26_wide_poison_sweep() {
        let trace = std::env::var_os("LOFT_F2_TRACE").is_some();
        // xorshift64 with a fixed seed — deterministic + reproducible.
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let n_progs = 1500;
        let mut findings: Vec<String> = Vec::new();
        for _ in 0..n_progs {
            let kind = match next() % 3 {
                0 => Kind::Hash,
                1 => Kind::Sorted,
                _ => Kind::Index,
            };
            let n_keys = (next() % 20) as u32 + 1;
            let closures = next() & 1 == 1;
            let n_remove = (next() % u64::from(n_keys + 1)) as usize;
            let remove: Vec<u32> = (0..n_remove)
                .map(|_| (next() % u64::from(n_keys)) as u32)
                .collect();
            let spec = KeyedSpec {
                kind,
                n_keys,
                remove,
                closures,
            };
            let src = generate_keyed(&spec);
            if trace {
                eprintln!("TRY {spec:?}");
            }
            match catch_unwind(AssertUnwindSafe(|| check_generated_with(&src, true))) {
                Ok(Ok(())) => {}
                Ok(Err(msg)) => {
                    findings.push(format!("{spec:?}: {}", msg.lines().next().unwrap_or("")));
                }
                Err(_) => findings.push(format!("{spec:?}: PANIC (compiler ICE)")),
            }
        }
        eprintln!(
            "F2.6: {n_progs} specs under poison, {} finding(s)",
            findings.len()
        );
        assert!(
            findings.is_empty(),
            "{} finding(s):\n{}",
            findings.len(),
            findings.join("\n")
        );
    }

    /// F2.2 — prove the self-check CAN fail (non-vacuity). A correct program
    /// passes, but corrupting a baked expectation — exactly what a real
    /// collection miscompile would produce — makes `check_generated` report a
    /// FINDING. Without this, a green F2.1 sweep could mean "the assertions
    /// never actually fire". The corruption edits the emitted source directly,
    /// leaving `generate_keyed` (the production path) untouched.
    #[test]
    fn corrupted_expectation_is_reported_as_a_finding() {
        // Hash, 5 keys, remove index 2 → survivors {0,1,3,4}, population 4.
        let spec = KeyedSpec {
            kind: Kind::Hash,
            n_keys: 5,
            remove: vec![2],
            closures: false,
        };
        let good = generate_keyed(&spec);
        assert!(check_generated(&good).is_ok(), "baseline program must pass");

        // (a) wrong population: really 4, claim 7 → the `cnt == N` self-check fails.
        let bad_pop = good.replace("cnt == 4", "cnt == 7");
        assert_ne!(
            bad_pop, good,
            "population corruption must change the source"
        );
        let r = check_generated(&bad_pop);
        assert!(
            r.as_ref().is_err_and(|m| m.contains("FINDING")),
            "corrupted population must be a FINDING, got {r:?}"
        );

        // (b) wrong lookup value: key k000 maps to 1, claim 999.
        let bad_val = good.replacen("[\"k000\"].v == 1", "[\"k000\"].v == 999", 1);
        assert_ne!(bad_val, good, "value corruption must change the source");
        assert!(
            check_generated(&bad_val).is_err(),
            "corrupted lookup value must be a finding"
        );
    }
}
