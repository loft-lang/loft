// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN107 dead-code lint — the ORACLE (regression net for `use_analysis::warn_dead_stores`).
//!
//! Locks the diagnostics emitted for the plan's shape corpus
//! (`doc/claude/plans/107-dead-code-lint/spec.loft`) on **both** backends. The lint is
//! **default ON** (S5): it warns when a non-escaping local OWNS a copy that is mutated via an
//! `OpSet*` but never read (the copy-mutate footgun) — `LOFT_NO_DEAD_STORES` opts out.
//!
//! Two disjoint diagnostics share the substring "is never read", so they are counted apart:
//!   - `test_used` (`unused_variables`, shipped): "Variable X is never read" — three locals in
//!     the corpus (`a`, `total`, `x`). Unaffected by this lint.
//!   - this lint: "'d' is mutated but its value is never read" — exactly one (the W-copy `d`),
//!     an OWNED copy. Aliases (`&`, reference-struct fields, element views) must stay SILENT
//!     (the S4a false-positive fix), verified against real corpus scripts below.
//!
//! Why the binary (not the in-process harness): these are end-to-end compile diagnostics on
//! stderr — same approach as `tests/runtime_warnings.rs`. `LOFT_NO_CACHE` because the program
//! cache skips the re-parse (hence the diagnostics) on a warm run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn spec_path() -> PathBuf {
    workspace("doc/claude/plans/107-dead-code-lint/spec.loft")
}

const DEAD_STORE_MSG: &str = "is mutated but its value is never read";

/// The three locals `test_used` (unused_variables) flags in the corpus — disjoint from this
/// lint (a var mutated via `OpSet` was read as the write base at parse, so `uses>0`).
const EXPECT_NEVER_READ: [&str; 3] = [
    "Variable a is never read", // W-scalar    — a = 3; a += 1  (self-read doesn't rescue)
    "Variable total is never read", // W-accumulator — total += i, unread after the loop
    "Variable x is never read", // N-effectful — binding unread; the RHS effect would stay
];

/// Run `file` on `backend` (`--interpret` / `--native`) with the given extra env, returning
/// `(stdout, stderr, exit_code)`. `LOFT_NO_CACHE` forces a cold compile so the parse-time
/// diagnostics actually fire; `LOFT_TIMEOUT` bounds the native rustc step.
fn run_env(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Dead-store warnings emitted (this lint).
fn dead_stores(diag: &str) -> usize {
    diag.matches(DEAD_STORE_MSG).count()
}

/// `test_used` never-read warnings — the dead-store message also contains "is never read", so
/// subtract those to isolate `unused_variables`.
fn never_reads(diag: &str) -> usize {
    diag.matches("is never read").count() - dead_stores(diag)
}

// ── Default ON: the corpus warns exactly on the W-copy dead store ────────────────────────────

fn assert_default(backend: &str) {
    let (stdout, diag, code) = run_env(backend, &spec_path(), &[]);

    // Compiles + runs clean (native: the real codegen guard).
    assert_eq!(
        code,
        Some(0),
        "[{backend}] corpus did not exit 0\n{stdout}\n---\n{diag}"
    );
    assert!(
        stdout.contains("N-effect (binding x unread"),
        "[{backend}] corpus did not run to completion\n{stdout}"
    );

    // Exactly three dead stores — the W-copy `d` (an owned vector copy) plus the two value-struct
    // copies `vf` (field) and `ve` (element), which OWN their copy (`value_struct_copy`). More = a
    // false positive returned; fewer = the lint regressed / got silently disabled.
    let ds = dead_stores(&diag);
    assert_eq!(
        ds, 3,
        "[{backend}] want exactly 3 dead-store warnings (W-copy d, W-vstruct vf/ve), got {ds}\n{diag}"
    );
    for v in ["d", "vf", "ve"] {
        assert!(
            diag.contains(&format!("'{v}' {DEAD_STORE_MSG}")),
            "[{backend}] the dead store on `{v}` must fire\n{diag}"
        );
    }
    // @PLN102 alias-where-correct step 6 — the lint is the arc-C write-through steer: it points at
    // the explicit `&` fix AND offers the read-back alternative. The write-through is never inferred
    // (copy stays the semantics), so the lint teaching `&` is how the intent is recovered.
    //
    // @PLN131 moved both out of the message and into fix lines, so the steer is asserted where it
    // now lives — behind `--explain`. What is checked is unchanged: BOTH ways out are still offered,
    // because dropping either would leave the author with one option and no way to see the other.
    let (_, explained, _) = run_env(backend, &spec_path(), &[("LOFT_EXPLAIN", "1")]);
    assert!(
        explained.contains("`d = &…`") && explained.contains("read `d` after"),
        "[{backend}] the dead-store fix must steer to `&` (write-through) + offer read-back\n{explained}"
    );
    // …and the message itself must NOT repeat them: a reader pays for that duplication on every
    // diagnostic, which is the whole reason the resolution moved.
    assert!(
        !diag.contains("`d = &…`"),
        "[{backend}] the message must say what is WRONG; the fix says what to write instead\n{diag}"
    );
    // Load-bearing guard: a reference-struct alias (`al`) has the SAME (reads=0, write_targets=1)
    // signal as a value-struct copy, but the write PROPAGATES to the source, so it must stay
    // SILENT — the value-struct fix must be distinguished by ownership, not over-fire on aliases.
    assert!(
        !diag.contains(&format!("'al' {DEAD_STORE_MSG}")),
        "[{backend}] reference-struct alias `al` must NOT warn (the write propagates)\n{diag}"
    );

    // `test_used` untouched — its three never-read warnings still fire, and `d` gets ONLY the
    // dead-store message (no double warning).
    for w in EXPECT_NEVER_READ {
        assert!(
            diag.contains(w),
            "[{backend}] never-read {w:?} must still fire\n{diag}"
        );
    }
    assert_eq!(
        never_reads(&diag),
        3,
        "[{backend}] never-read set drifted\n{diag}"
    );
    assert!(
        !diag.contains("Variable d is never read"),
        "[{backend}] `d` must get ONLY the dead-store warning\n{diag}"
    );
}

#[test]
fn default_on_warns_w_copy_interpret() {
    assert_default("--interpret");
}

#[test]
fn default_on_warns_w_copy_native() {
    assert_default("--native");
}

// ── Opt-out: LOFT_NO_DEAD_STORES silences this lint (but not test_used) ──────────────────────

fn assert_opt_out(backend: &str) {
    let (_out, diag, code) = run_env(backend, &spec_path(), &[("LOFT_NO_DEAD_STORES", "1")]);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] opt-out run did not exit 0\n{diag}"
    );
    assert_eq!(
        dead_stores(&diag),
        0,
        "[{backend}] LOFT_NO_DEAD_STORES must silence the lint\n{diag}"
    );
    // `test_used` is a different lint — the opt-out must not touch it.
    assert_eq!(
        never_reads(&diag),
        3,
        "[{backend}] opt-out must not affect unused_variables\n{diag}"
    );
}

#[test]
fn opt_out_silences_interpret() {
    assert_opt_out("--interpret");
}

#[test]
fn opt_out_silences_native() {
    assert_opt_out("--native");
}

// ── S4a regression: ALIASES stay silent (the write propagates → not a dead store) ────────────

/// Real corpus scripts whose mutated-but-unread local is a BORROW (a `&`-reference, a
/// reference-struct field alias, or a vector-element view). The write reaches the borrowed
/// source, so the lint must NOT fire — this locks the S4a ownership fix against regressions.
#[test]
fn s4a_aliases_stay_silent() {
    let aliases = [
        "tests/scripts/434-pln87-scalar-reference.loft", // & scalar reference
        "tests/scripts/25-index-elision-borrower.loft",  // e = v[i] element view
        "tests/scripts/85-borrow-elision-element-borrower.loft",
    ];
    for rel in aliases {
        let (_o, diag, _c) = run_env("--interpret", &workspace(rel), &[]);
        assert_eq!(
            dead_stores(&diag),
            0,
            "alias script {rel} must emit NO dead-store warning (the write propagates)\n{diag}"
        );
    }
}

// ── S1: the read / write-target classifier (observable via LOFT_DUMP_READS) ──────────────────

/// Parse the `LOFT_DUMP_READS` dump into `(fn, var) -> (reads, write_targets)`.
fn classifier_dump() -> HashMap<(String, String), (u32, u32)> {
    let (_o, stderr, _c) = run_env("--interpret", &spec_path(), &[("LOFT_DUMP_READS", "1")]);
    let mut m = HashMap::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("dead-store-dbg: ") else {
            continue;
        };
        let (mut f, mut v, mut r, mut w) = (None, None, None, None);
        for tok in rest.split_whitespace() {
            if let Some(x) = tok.strip_prefix("fn=") {
                f = Some(x.to_string());
            } else if let Some(x) = tok.strip_prefix("var=") {
                v = Some(x.to_string());
            } else if let Some(x) = tok.strip_prefix("reads=") {
                r = x.parse().ok();
            } else if let Some(x) = tok.strip_prefix("write_targets=") {
                w = x.parse().ok();
            }
        }
        if let (Some(f), Some(v), Some(r), Some(w)) = (f, v, r, w) {
            m.insert((f, v), (r, w));
        }
    }
    m
}

#[test]
fn s1_classifier_isolates_w_copy_dead_store() {
    let m = classifier_dump();
    let cell = |f: &str, v: &str| -> (u32, u32) {
        *m.get(&(f.to_string(), v.to_string())).unwrap_or_else(|| {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            panic!("no classifier dump for {f}/{v}; keys={keys:?}")
        })
    };

    // THE signal — W-copy `d` is mutated but never read (the copy-fill `d = b.data` OpAppendVector
    // must NOT count as a read).
    assert_eq!(
        cell("n_w_copy", "d"),
        (0, 1),
        "W-copy d must be reads=0 write_targets=1"
    );

    // Non-signals — an N-row either keeps a real read or has no write-target.
    assert!(cell("n_n_read", "d").0 >= 1, "N-read d must have reads>=1");
    assert!(
        cell("n_n_fresh_used", "e").0 >= 1,
        "N-fresh e must have reads>=1"
    );
    assert_eq!(
        cell("n_n_copy_read", "d").1,
        0,
        "N-copy-read d must have write_targets=0"
    );
    // `test_used`-owned scalars present no S2 signal → no double warning.
    assert_eq!(
        cell("n_w_scalar", "a"),
        (1, 0),
        "W-scalar a: self-read only, no OpSet write-target"
    );
    assert_eq!(
        cell("n_n_effectful", "x"),
        (0, 0),
        "N-effectful x: no write-target"
    );

    // S3 escape/branch/loop guards — each mutated copy is also genuinely READ (reads>=1).
    assert!(
        cell("n_n_escape_call", "d").0 >= 1,
        "N-escape-call d: passed to a call"
    );
    assert!(
        cell("n_n_cond_read", "d").0 >= 1,
        "N-cond-read d: read on a branch"
    );
    assert!(
        cell("n_n_loop_read", "buf").0 >= 1,
        "N-loop-read buf: cross-iteration read"
    );

    // Construction guard — `z = Box{…}` fills READ z (reads>0), so despite write_targets>0 the
    // reads==0 signal is absent (no false positive on construction).
    let (zr, zw) = cell("n_n_construct_unread", "z");
    assert!(
        zr >= 1 && zw >= 1,
        "construct-unread z: reads>=1 (fills) + wt>=1: got ({zr},{zw})"
    );

    // Value-struct completeness — a `value struct` copy read out of a field/element is OWNED (the
    // `value_struct_copy` pass), so its never-read mutation is the dead-store signal, exactly like
    // W-copy.
    assert_eq!(
        cell("n_w_vstruct_field", "vf"),
        (0, 1),
        "W-vstruct-field vf: reads=0 write_targets=1"
    );
    assert_eq!(
        cell("n_w_vstruct_elem", "ve"),
        (0, 1),
        "W-vstruct-elem ve: reads=0 write_targets=1"
    );
    assert!(
        cell("n_n_vstruct_read", "vr").0 >= 1,
        "N-vstruct-read vr: the copy is read → reads>=1"
    );
    // The reference-struct alias `al` has the IDENTICAL (0,1) signal as a value-struct copy — proof
    // the classifier alone cannot separate them; `warn_dead_stores`' ownership check does (al is a
    // reference struct → Borrowed → silent, verified in `assert_default`).
    assert_eq!(
        cell("n_n_ref_alias", "al"),
        (0, 1),
        "N-ref-alias al: same (0,1) signal as a value-struct copy, filtered by ownership"
    );
}

// ── loft#670: the alias false POSITIVE and the element-assign false NEGATIVE ─────────────────

/// Both edges a consumer walked into, on both backends (`tests/data/dead_store_670.loft`).
///
/// **False negative** — `w[i] = Row{…}` lowers to `OpCopyRecord(obj, OpGetVector(w,…))`, whose
/// destination is arg **1**, so the first-arg write rule never saw it and the lost write was
/// silent.  A BARE-var destination stays uncounted: that is the definitional value-struct fill,
/// not a mutation.
///
/// **False positive** — `w = &v` on a local vector mints no buffer, so `w` takes `v`'s own
/// backing and resolves to a synth base like a private copy does.  The lint warned on it,
/// recommending the `&` the author had already written; warnings gate a library's CI.  Sharing
/// that backing with a live sibling is what separates the two.
///
/// The values are asserted alongside the diagnostics: a lint that warns for the wrong reason
/// passes a warning-only check.
fn assert_670(backend: &str) {
    let (stdout, diag, code) = run_env(backend, &workspace("tests/data/dead_store_670.loft"), &[]);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] fixture did not exit 0\n{stdout}\n---\n{diag}"
    );

    // Exactly the two private copies warn.
    let ds = dead_stores(&diag);
    assert_eq!(
        ds, 2,
        "[{backend}] want 2 dead stores (copy_elem_assign, copy_field_write), got {ds}\n{diag}"
    );
    for v in ["copy_elem_assign", "copy_field_write"] {
        assert!(
            diag.contains(&format!("'{v}' {DEAD_STORE_MSG}")),
            "[{backend}] the dead store on `{v}` must fire\n{diag}"
        );
    }
    // Every alias form stays silent — these are the write-through cures the message names.
    for v in ["ref_local", "ref_field", "elem"] {
        assert!(
            !diag.contains(&format!("'{v}' {DEAD_STORE_MSG}")),
            "[{backend}] `{v}` aliases its source — the write propagates, so it must NOT warn\n{diag}"
        );
    }

    // The values behind the verdicts: the warned copies really do lose the write, and the
    // silent aliases really do propagate it.
    for want in [
        "elem_assign=0",     // whole-element assign into a copy — discarded
        "field_write=0",     // field write into a copy — discarded
        "ref_local=7.25",    // `&` on a local vector — propagates
        "ref_field=9.25",    // `&` on a field: 7.25 written back + 2 elements after the append
        "element_view=7.25", // an element view writes into the vector it views
    ] {
        assert!(
            stdout.contains(want),
            "[{backend}] expected `{want}`\n{stdout}\n---\n{diag}"
        );
    }
}

#[test]
fn alias_and_element_assign_670_interpret() {
    assert_670("--interpret");
}

#[test]
fn alias_and_element_assign_670_native() {
    assert_670("--native");
}

/// A `len()` BOUND GUARD must not discharge the dead-store lint.
///
/// `d = self.items; if i < len(d) { d[i] = x }` is the copy-mutate footgun in full, but
/// the lint counted `len(d)` as "the copy was read" and stayed silent — and a length
/// observation cannot witness an element write.  Worse, the guard is the idiom the `v[i]`
/// may-be-null warning ASKS for (skip-pattern 5), so the two lints were in tension:
/// satisfying one silenced the other.
///
/// Not hypothetical.  The published `graphics` canvas is written this way, so `set_pixel`
/// / `fill_rect` / `h_line` / `v_line` were all no-ops, every `--html` page uploaded a
/// blank texture, and no diagnostic fired anywhere.
///
/// Both cures the message names must stay silent, so the fix cannot be "warn more":
/// a live `&` reference, and a copy that is read back after the mutation.
fn assert_len_guard(backend: &str) {
    let (stdout, diag, code) = run_env(
        backend,
        &workspace("tests/data/dead_store_len_guard.loft"),
        &[],
    );
    assert_eq!(
        code,
        Some(0),
        "[{backend}] fixture did not exit 0\n{stdout}\n---\n{diag}"
    );

    // Exactly the one guarded COPY warns.
    let ds = dead_stores(&diag);
    assert_eq!(
        ds, 1,
        "[{backend}] want 1 dead store (guarded_copy's `gc_d`), got {ds}\n{diag}"
    );
    assert!(
        diag.contains(&format!("'gc_d' {DEAD_STORE_MSG}")),
        "[{backend}] a `len()`-guarded copy-mutate must warn — this is the shape the \
         published graphics canvas shipped\n{diag}"
    );
    for v in ["gr_d", "rb_d"] {
        assert!(
            !diag.contains(&format!("'{v}' {DEAD_STORE_MSG}")),
            "[{backend}] `{v}` is one of the two cures the message names — it must NOT \
             warn\n{diag}"
        );
    }

    // The values behind the verdicts: the warned copy really does lose the write, and
    // the silent forms really do behave as the message claims.
    for want in [
        "guarded_copy=1",  // the write landed in the copy — source unchanged
        "guarded_ref=8",   // `&` wrote through
        "read_back=42",    // the copy was read back, so it was a real copy
        "source_intact=1", // …and reading it back did not touch the source
    ] {
        assert!(
            stdout.contains(want),
            "[{backend}] expected `{want}`\n{stdout}\n---\n{diag}"
        );
    }
}

#[test]
fn len_guard_does_not_silence_dead_store_interpret() {
    assert_len_guard("--interpret");
}

#[test]
fn len_guard_does_not_silence_dead_store_native() {
    assert_len_guard("--native");
}

/// loft#781 (the sibling defect) — a dead store inside a DEPENDENCY is reported
/// against the DEPENDENCY's file.
///
/// `var_source` gives a line in the file the DEFINITION was parsed from, and that line
/// was paired with the ENTRY file: a real line number in the wrong file, echoing
/// whatever sits there. Found by sweeping the producers after the copy notice was fixed
/// — the copy notice was the reported case, not the whole rule.
///
/// It bites harder here than it did there. The copy notice is `advice`; this is a
/// `warning`, and a warning gates a library's CI under `LOFT_DENY_WARNINGS`. So a
/// dependency's dead store failed a consumer's build at a line the consumer could look
/// at and find a `const`.
///
/// The entry file carries a `const` at exactly the library's dead-store line, so a
/// regression reproduces the original symptom rather than merely failing.
///
/// loft#1260 put a `loft.toml` at the probe root, which is what makes the warning ADDRESSED
/// here: a lint reaches only whoever can act on its cure, and with the entry and the library
/// in one project that is this author. The attribution rule being pinned is unchanged and
/// still needs two files to reproduce — what changed is that the same shape without a
/// manifest is a consumer reading someone else's library, and
/// [`a_dependency_dead_store_does_not_reach_a_consumer`] pins that it stays quiet. The two
/// together say the whole rule: report it to the author, against the author's own file.
#[test]
fn a_dependency_dead_store_is_reported_against_the_dependency_file() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_781ds_{}_{n}", std::process::id()));
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("probe dirs");
    // One project, so the library is the author's own code and its lints are addressed to
    // this build (loft#1260).
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"probe781ds\"\nversion = \"0.1.0\"\ncategories = [\"testing\"]\n",
    )
    .expect("write manifest");

    // The dead store is line 5 of the library: the whole-value bind `d = s.items` COPIES
    // (C86), `d` is mutated, and `d` is never read — so the write is lost.
    std::fs::write(
        lib.join("dstore781.loft"),
        "// 1\npub struct Data781 { items: vector<integer> }\n// 3\n\
         pub fn lose_it(s: Data781) -> integer {\n  d = s.items;\n  d[0] = 99;\n  \
         return len(s.items);\n}\n",
    )
    .expect("write lib");

    // Line 5 of the ENTRY is a `const` — where the warning used to land.
    let entry = root.join("main781ds.loft");
    std::fs::write(
        &entry,
        "use dstore781;\n// 2\n// 3\n// 4\nconst UNRELATED781 = 1;\n// 6\n\
         fn main() {\n  s = Data781 { items: [1, 2, 3] };\n  println(\"{lose_it(s)}\");\n}\n",
    )
    .expect("write entry");

    let out = Command::new(loft_bin())
        .args(["--interpret", "--check"])
        .arg("--lib")
        .arg(&lib)
        .arg(&entry)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        all.contains("is mutated but its value is never read"),
        "the cross-file dead-store warning must still fire; output:\n{all}"
    );
    assert!(
        all.contains("dstore781.loft:5"),
        "a dead store in a dependency must name the DEPENDENCY's file and line; output:\n{all}"
    );
    assert!(
        !all.contains("main781ds.loft:5"),
        "the warning must not be attributed to the entry file (loft#781); output:\n{all}"
    );
    assert!(
        !all.contains("const UNRELATED781"),
        "the echoed line must be the dead store, not a `const`; output:\n{all}"
    );
}

/// loft#1260 — the same dead store, read by a CONSUMER, does not reach them.
///
/// The twin of [`a_dependency_dead_store_is_reported_against_the_dependency_file`], and the
/// pair is the point: identical sources, differing only in whether a `loft.toml` puts the
/// library inside the project being built. Without one it is someone else's library, the
/// cure is an edit the reader cannot make, and `dead-store` is a `warning` — so under
/// `LOFT_DENY_WARNINGS` it would fail a consumer's build over a line in a file they do not
/// own. Correct attribution (loft#781) made that failure name the right file; it did not
/// make it theirs to fix.
///
/// Scored against the twin rather than against silence: a build that prints nothing passes
/// this on its own.
#[test]
fn a_dependency_dead_store_does_not_reach_a_consumer() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_1260ds_{}_{n}", std::process::id()));
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("probe dirs");
    // Deliberately NO manifest at the root: the entry is a bare script, so the library is a
    // dependency rather than part of what is being built.
    std::fs::write(
        lib.join("dstore1260.loft"),
        "// 1\npub struct Data1260 { items: vector<integer> }\n// 3\n\
         pub fn lose_it(s: Data1260) -> integer {\n  d = s.items;\n  d[0] = 99;\n  \
         return len(s.items);\n}\n",
    )
    .expect("write lib");
    let entry = root.join("main1260ds.loft");
    std::fs::write(
        &entry,
        "use dstore1260;\n// 2\n// 3\n// 4\nconst UNRELATED1260 = 1;\n// 6\n\
         fn main() {\n  s = Data1260 { items: [1, 2, 3] };\n  println(\"{lose_it(s)}\");\n}\n",
    )
    .expect("write entry");

    let out = Command::new(loft_bin())
        .args(["--interpret", "--check"])
        .arg("--lib")
        .arg(&lib)
        .arg(&entry)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        !all.contains("is mutated but its value is never read"),
        "a consumer cannot edit the dependency this points at; output:\n{all}"
    );
    assert!(
        !all.contains("dstore1260.loft"),
        "nothing about the dependency's internals belongs in a consumer's build; \
         output:\n{all}"
    );
}

/// loft#883 — a compile ERROR must not drag a FALSE `lost-write` along with it.
///
/// Every lint in this family reads the RESOLVED types and the ownership verdicts derived
/// from them. When resolution aborts, an unresolved type carries empty deps and
/// `ownership_of` reads empty deps as OWNED — so a borrowing `for` loop variable in an
/// unrelated library reads as a lost write. The reported case was an ambiguous bare struct
/// name (two libraries declaring `Slot`): the error is correct and names its own fix, and
/// beside it came a `lost-write` pointing at correct code in a different package.
///
/// That matters more than an ordinary false positive: `lost-write` names loft's most
/// expensive real bug class, so a false one teaches the reader to distrust it — and this
/// state is unreachable by a warning-clean gate, because a green suite never aborts.
///
/// The positive half is asserted too: with the ambiguity removed the same library, same
/// loop, must still compile clean, so the fix is a suppression tied to the failing compile
/// and not a silent disabling of the lint.
#[test]
fn a_compile_error_does_not_emit_a_false_lost_write() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_883amb_{}_{n}", std::process::id()));
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("probe dirs");

    // The loop-variable mutation the false warning pointed at. `lw_t` BORROWS the element,
    // so the write persists and `lw_v` is read two lines down — nothing is lost here.
    std::fs::write(
        lib.join("slot883.loft"),
        "pub struct Slot883 { idx: integer, taken: boolean }\n\
         pub fn mutate_through_a_loop_variable() -> boolean {\n  \
           lw_v: vector<Slot883> = [];\n  \
           for lw_i in 0..3 { lw_v += [Slot883 { idx: lw_i, taken: false }]; }\n  \
           lw_at = 0;\n  \
           for lw_t in lw_v {\n    \
             if lw_at == 1 { lw_t.taken = true; }\n    \
             lw_at = lw_at + 1;\n  \
           }\n  \
           (lw_v[1] ?? Slot883 {}).taken\n}\n",
    )
    .expect("write lib");
    // A second library declaring the SAME name is what makes the bare use ambiguous.
    std::fs::write(
        lib.join("other883.loft"),
        "pub struct Slot883 { other: text }\n",
    )
    .expect("write other lib");

    let run = |entry: &std::path::Path| -> String {
        let out = Command::new(loft_bin())
            .args(["--interpret", "--check"])
            .arg("--lib")
            .arg(&lib)
            .arg(entry)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke loft binary");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // The bug: a bare `Slot883` is ambiguous, so the compile aborts.
    let amb = root.join("amb883.loft");
    std::fs::write(
        &amb,
        "use slot883;\nuse other883;\nfn main() {\n  \
           s = Slot883 { idx: 0, taken: false };\n  \
           println(\"{s.idx} {mutate_through_a_loop_variable()}\");\n}\n",
    )
    .expect("write amb entry");
    let all = run(&amb);

    assert!(
        all.contains("declared by more than one package"),
        "the ambiguity error must still be reported; output:\n{all}"
    );
    assert!(
        !all.contains("is mutated but its value is never read"),
        "a failing compile must not emit a lost-write derived from unresolved types \
         (loft#883); output:\n{all}"
    );

    // The control: same library, same loop, ambiguity removed — must compile clean, which
    // is what proves the lint was suppressed for the error and not switched off.
    let ok = root.join("ok883.loft");
    std::fs::write(
        &ok,
        "use slot883;\nfn main() {\n  println(\"{mutate_through_a_loop_variable()}\");\n}\n",
    )
    .expect("write ok entry");
    let clean = run(&ok);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        !clean.contains("is mutated but its value is never read"),
        "the loop-variable mutation is not a lost write and must stay silent; output:\n{clean}"
    );
    assert!(
        !clean.contains("error"),
        "the unambiguous control must compile clean; output:\n{clean}"
    );
}

// ── A default expression is a READ of the parameter it names ─────────────────────────────
//
// `fn window(rows: integer, height: integer = rows * 10)` counted no use of `rows`, so
// `test_used` warned "Parameter rows is never read" and offered to "drop the parameter
// `rows` — and its callers' argument". Taking that advice deletes what the default reads.
// The default is parsed against TEMPORARY variables that are removed again before the real
// parameter slots exist (`definitions.rs`, the `injected` mapping), so the read landed on a
// throwaway slot; it is answered off the stored signature instead.
//
// Same class as the closure-capture false positive (`tests/expressions.rs`,
// `closure_capture_after_change`): a lint that must never make a program wrong.
#[test]
fn a_parameter_read_by_a_later_default_is_not_dead() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_pdflt_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&root).expect("probe dir");

    // Each case pairs a shape with the values that prove the default actually ran, so a
    // silenced warning can never be silence over a broken default.
    // The fourth column is the parameter the lint SHOULD name, if any — not a bare flag,
    // because "a warning fired" and "the right one fired" are the two things that come
    // apart here: the fix is name-based, so a blanket silence and a correct silence look
    // identical from a count.
    let cases: [(&str, &str, &str, Option<&str>); 5] = [
        (
            "arith",
            "fn window(rows: integer, height: integer = rows * 10) -> integer { height }\n\
             fn main() { println(\"{window(4)} {window(4, 7)}\"); }\n",
            "40 7",
            None,
        ),
        (
            "call",
            "fn twice(n: integer) -> integer { n * 2 }\n\
             fn window(rows: integer, height: integer = twice(rows)) -> integer { height }\n\
             fn main() { println(\"{window(4)}\"); }\n",
            "8",
            None,
        ),
        // A default needing a temporary is lifted into a function of its own (loft#699); the
        // parameters it reads become that function's arguments, so the same lookup has to
        // reach through the generated call.
        (
            "lifted",
            "fn window(rows: integer, tags: vector<integer> = [rows, rows]) -> integer \
               { len(tags) + rows }\n\
             fn main() { println(\"{window(4)}\"); }\n",
            "6",
            None,
        ),
        // The lint is not blunted: a parameter no default and no body reads still warns.
        (
            "unused",
            "fn window(rows: integer, height: integer = 40) -> integer { height }\n\
             fn main() { println(\"{window(4)}\"); }\n",
            "40",
            Some("Parameter rows is never read"),
        ),
        // And it is per-parameter, not per-function: `spare` is dead beside a live `rows`.
        (
            "beside",
            "fn window(rows: integer, spare: integer, height: integer = rows * 10) -> integer \
               { height }\n\
             fn main() { println(\"{window(4, 9)}\"); }\n",
            "40",
            Some("Parameter spare is never read"),
        ),
    ];

    for backend in ["--interpret", "--native"] {
        for (name, src, expect_out, expect_named) in &cases {
            let file = root.join(format!("{name}.loft"));
            std::fs::write(&file, src).expect("write probe");
            let (out, err, code) = run_env(backend, &file, &[]);
            assert_eq!(
                code,
                Some(0),
                "[{backend}/{name}] must compile and run\n{err}"
            );
            assert_eq!(
                out.trim(),
                *expect_out,
                "[{backend}/{name}] the default must still produce its value\n{err}"
            );
            match expect_named {
                Some(msg) => assert!(
                    err.contains(msg),
                    "[{backend}/{name}] the lint must still name a genuinely dead parameter \
                     ({msg})\n{err}"
                ),
                None => assert!(
                    !err.contains("is never read"),
                    "[{backend}/{name}] the parameter is read by a default — warning it dead \
                     advertises a deletion that stops the program compiling\n{err}"
                ),
            }
            assert_eq!(
                err.matches("is never read").count(),
                usize::from(expect_named.is_some()),
                "[{backend}/{name}] exactly the expected never-read set must fire\n{err}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ── An unresolved method with a named argument reports ONE error ─────────────────────────
//
// `skip_remaining_args` is the shared "consume a call I cannot resolve" loop, and it parsed
// each argument as an expression — so a NAMED argument stopped it at the `:`. On the error
// path that turned one typo into five cascading messages with no `Unknown field` among
// them; on the FIRST-PASS path (the same loop, reached before a forward-declared method is
// known) it made a legal call fail depending on where its method was declared, which the
// worked-example file holds directly.
#[test]
fn an_unknown_method_with_a_named_argument_reports_one_error() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_nmrec_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&root).expect("probe dir");

    let file = root.join("recover.loft");
    std::fs::write(
        &file,
        "struct S986 { a: integer }\n\
         fn main() { s = S986 { a: 1 }; println(\"{s.nosuch(width: 3)}\"); }\n",
    )
    .expect("write probe");

    let (_out, err, code) = run_env("--interpret", &file, &[]);
    let _ = std::fs::remove_dir_all(&root);

    assert_ne!(code, Some(0), "an unknown member must still fail\n{err}");
    assert!(
        err.contains("Unknown field S986.nosuch"),
        "the one message worth reading must survive the recovery\n{err}"
    );
    let errors = err.matches("error:").count();
    assert_eq!(
        errors, 2,
        "one diagnostic plus the `aborting due to` line — a named argument must not cascade\n{err}"
    );
}

// ── loft#1633: a mutated COPY of a parameter is a lost write ─────────────────────────────────
//
// `v = p` from a record parameter — plain or `&` — deep-copies (`@FR-B-Copy`; through a `&` by
// `@FR-C-Ref`), so `v.a = 7` with `v` never read goes nowhere.  `ownership_of` follows the bind
// to the parameter and answers `Borrowed`, the answer for a PROJECTION off it (`m = w.f`), whose
// write does reach the caller — so the lint stayed silent on the copy.  The pair is the claim:
// both copies warn, and the projection, whose write is not lost, stays silent.

const PARAM_COPY: &str = "struct Foo { a: integer, b: integer }\n\
fn plain(s: Foo) { snap = s; snap.a = 7; }\n\
fn linked(s: &Foo) { snap = s; snap.a = 7; s.b = 1; }\n\
fn main() { f = Foo { a: 0, b: 5 }; plain(f); linked(f); print(\"r={f.a},{f.b}\"); }\n";
const PARAM_PROJECTION: &str = "struct Foo { a: integer, b: integer }\n\
struct W { f: Foo }\n\
fn g(w: W) { m = w.f; m.a = 7; }\n\
fn main() { x = W { f: Foo { a: 0, b: 5 } }; g(x); print(\"r={x.f.a}\"); }\n";

fn run_body(body: &str, backend: &str, tag: &str) -> (String, String) {
    let path = std::env::temp_dir().join(format!("loft_dcl_{}_{tag}.loft", std::process::id()));
    std::fs::write(&path, body).expect("write script");
    let (out, diag, _code) = run_env(backend, &path, &[]);
    let _ = std::fs::remove_file(&path);
    (out, diag)
}

#[test]
fn a_mutated_copy_of_a_parameter_is_a_lost_write() {
    for backend in ["--interpret", "--native"] {
        let (out, diag) = run_body(PARAM_COPY, backend, &format!("pcopy_{backend}"));
        assert!(
            out.contains("r=0,1"),
            "[{backend}] both binds copy: {out:?}\n{diag}"
        );
        assert_eq!(
            dead_stores(&diag),
            2,
            "[{backend}] a copy of a plain AND of a `&` record parameter, mutated and never \
             read, are lost writes (loft#1633)\n{diag}"
        );
    }
}

#[test]
fn a_mutated_projection_of_a_parameter_stays_silent() {
    for backend in ["--interpret", "--native"] {
        let (out, diag) = run_body(PARAM_PROJECTION, backend, &format!("pproj_{backend}"));
        assert!(
            out.contains("r=7"),
            "[{backend}] a projection is a view: {out:?}\n{diag}"
        );
        assert_eq!(
            dead_stores(&diag),
            0,
            "[{backend}] the write reaches the caller\n{diag}"
        );
    }
}
