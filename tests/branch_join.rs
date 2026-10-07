// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#978 — a value delivered by a branch whose arms DISAGREE about ownership borrows
//! whatever any arm can hand it.
//!
//! `it = if fresh { Item { … } } else { b.items["one"]? }` typed `it` as
//! `ref(Item)` with an EMPTY dep list, which every free site reads as owned, so scope
//! exit released the container's record. The next unrelated allocation claimed the
//! recycled slot and every later read answered out of it: `2, 0, 0` where the same
//! program without the fresh arm reads `2, 2, 2`. Silent, both backends, exit 0.
//!
//! The cause is a HINT taken for a lifetime fact. `parse_if` parses the `else` block
//! with the THEN arm's type as its expected type, and a block adopted that expected
//! type whole — deps included. An expected type says what SHAPE belongs in a position;
//! it was written before the value in hand existed, so it cannot say what that value
//! aliases. Adopting it republished the sibling arm's borrow list, which is why the
//! defect was ARM-ORDER sensitive: put the view in the THEN arm and the program read
//! correctly, because then it was the fresh arm being handed someone else's deps.
//!
//! Two oracles, answering different questions:
//!
//!   - **Static** ([`a_split_ownership_join_names_what_it_can_alias`]). The local's
//!     recorded type must NAME the container it can alias. Deterministic: it does not
//!     depend on whether a freed slot happened to be reused, which is what let this
//!     through every gate the project runs (`LOFT_NO_SLOT_REUSE=1` reads correctly WITH
//!     the defect present).
//!   - **Behavioural** (the script cells). `tests/scripts/978-…loft` on both backends,
//!     plus strict-store and leak runs.
//!
//! [`harness_can_fail`] is the control for the harness itself.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/978-branch-join-carries-both-arms-borrows.loft")
}

/// Run `file` on `backend` with extra env; return `(ok, stdout, stderr)`.
fn run(backend: &str, file: &PathBuf, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const OK: &str = "978 branch join OK";

fn assert_cells_green(backend: &str, env: &[(&str, &str)], tag: &str) {
    let (ok, stdout, stderr) = run(backend, &probe(), env);
    assert!(
        ok && stdout.contains(OK),
        "[{backend}/{tag}] every branch-join cell must be green\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The smallest program with the defect, and its ARM-REVERSED twin. Both are here
/// because the pair is the finding: one order read correctly against the defect, so a
/// guard written on that order alone proves nothing.
const SPLIT: &str = "struct Y978 { name: text, limbs: vector<float> }\n\
struct B978 { items: hash<Y978[name]> }\n\
fn y978_read(b: B978, fresh: boolean) -> integer {\n\
\x20 it = if fresh { Y978 { name: \"f\", limbs: [] } } else { b.items[\"one\"] ?? Y978 { name: \"\", limbs: [] } };\n\
\x20 len(it.limbs)\n\
}\n\
fn main() {\n\
\x20 b = B978 { items: [] };\n\
\x20 one = Y978 { name: \"one\", limbs: [] };\n\
\x20 one.limbs += [1.0];\n\
\x20 b.items += [one];\n\
\x20 println(\"{y978_read(b, false)}\");\n\
}\n";

const BOTH_FRESH: &str = "struct Y978 { name: text, limbs: vector<float> }\n\
struct B978 { items: hash<Y978[name]> }\n\
fn y978_read(b: B978, fresh: boolean) -> integer {\n\
\x20 n = len(b.items);\n\
\x20 it = if fresh { Y978 { name: \"a\", limbs: [] } } else { Y978 { name: \"b\", limbs: [] } };\n\
\x20 len(it.limbs) + n - n\n\
}\n\
fn main() {\n\
\x20 b = B978 { items: [] };\n\
\x20 println(\"{y978_read(b, false)}\");\n\
}\n";

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_978_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

fn var_table(tag: &str, src: &str) -> String {
    let path = write_temp(tag, src);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_VAR_TABLE", "y978_read")
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| l.contains("[vartable]"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Static: the local's type names what it can alias ────────────────────────────────

/// The deterministic half, and the one that states the invariant.
///
/// A local fed by one owned arm and one borrowed arm must record the borrow. An empty
/// dep list is the OWNED reading, and the emitter frees on it — so the container's
/// record dies while the container still points at it, and nothing anywhere reports it.
#[test]
fn a_split_ownership_join_names_what_it_can_alias() {
    let table = var_table("split", SPLIT);
    let it = table
        .lines()
        .find(|l| l.split_whitespace().any(|w| w == "it"))
        .unwrap_or_default()
        .to_string();
    assert!(
        it.contains("deps=[b("),
        "a local that ONE arm hands a view of `b` must record that borrow — an empty \
         dep list reads as owned and scope exit frees the container's record \
         (loft#978)\nit: {it}\n{table}"
    );
    assert!(
        !it.contains("OWNS"),
        "and it must not also claim ownership — the two readings pick opposite free \
         decisions\nit: {it}\n{table}"
    );
}

/// The other direction, and the reason the rule is a UNION over the arms rather than a
/// widening of every join: when the arms AGREE that the value is fresh, it is genuinely
/// owned and must still be freed. A fix that simply stopped freeing at joins passes
/// every value cell above and leaks here instead.
#[test]
fn a_join_of_two_owned_arms_stays_owned() {
    let table = var_table("bothfresh", BOTH_FRESH);
    let it = table
        .lines()
        .find(|l| l.split_whitespace().any(|w| w == "it"))
        .unwrap_or_default()
        .to_string();
    assert!(
        it.contains("OWNS") && !it.contains("deps=[b("),
        "two fresh arms borrow nothing — the join rule must not invent a dep from the \
         container that merely happens to be in scope (loft#978)\nit: {it}\n{table}"
    );
}

// ── Value, both backends ────────────────────────────────────────────────────────────

#[test]
fn branch_join_cells_interpret() {
    assert_cells_green("--interpret", &[], "value");
}

#[test]
fn branch_join_cells_native() {
    assert_cells_green("--native", &[], "value");
}

/// Strict store lifetime: a freed store stays dead, and any access through a reference
/// naming it is an error. It also implies `LOFT_NO_SLOT_REUSE`, which is why it is a
/// SEPARATE cell and not the whole gate — with reuse off, this program answered
/// correctly WITH the defect present.
#[test]
fn branch_join_cells_strict_stores_interpret() {
    assert_cells_green("--interpret", &[("LOFT_STRICT_STORES", "1")], "strict");
}

/// A fix that stopped the over-free by never freeing anything would pass every cell
/// above and leak instead; this is the neighbour that catches it.
///
/// The script deliberately keeps the one shape that DOES still leak out of its
/// fresh-arm loop — an escaping value whose return correctly says it borrows a
/// parameter, whose other arm mints a record nobody then owns. That is not this fix's
/// doing (it is measured identical on the released binary for a plain `?? Item { … }`
/// accessor) and it needs a runtime adopt-or-materialise decision at the return, not a
/// static one: loft#981.
#[test]
fn branch_join_cells_leak_clean_native() {
    let (_ok, _out, stderr) = run("--native", &probe(), &[("LOFT_NATIVE_LEAK_CHECK", "1")]);
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[native] a store was not freed at exit — the join fix must not turn an \
         over-free into a leak\n{stderr}"
    );
}

#[test]
fn branch_join_cells_leak_clean_interpret() {
    let (_ok, _out, stderr) = run("--interpret", &probe(), &[]);
    assert!(
        !stderr.to_lowercase().contains("not freed"),
        "[interpret] a store was not freed at exit — the join fix must not turn an \
         over-free into a leak\n{stderr}"
    );
}

// ── The control for the harness ─────────────────────────────────────────────────────

/// A script whose assertion is deliberately false must be REPORTED as a failure —
/// otherwise "the cells printed OK" proves only that the file ran.
#[test]
fn harness_can_fail() {
    let path = write_temp(
        "canfail",
        "fn main() { assert(1 == 2, \"deliberate\"); print(\"978 branch join OK\\n\"); }\n",
    );
    let (ok, stdout, _stderr) = run("--interpret", &path, &[]);
    let _ = fa::remove_file(&path);
    assert!(
        !(ok && stdout.contains(OK)),
        "the harness must report a failing script as failing"
    );
}

// loft#1081 / D-own-8 lives one file over: a vector-valued join BOUND to a local the
// function then returns was NRVO-renamed onto the caller's buffer, which a join REBINDS
// rather than builds into — abandoning the buffer and handing back an arm store with no
// owner.  Its guard is `tests/scripts/1081-a-join-bound-to-a-returned-local.loft`, where
// the wrap harness's leak gate pins the stores alongside the values (both halves are
// needed: silencing the leak by freeing the DELIVERED store passes one and fails the
// other).  Kept there rather than duplicated here.

// ── Static: a vector local's backing is built for ITS element type ──────────────────

/// `LOFT_VAR_TABLE` for function `func`, the table lines only.
fn var_table_of(tag: &str, src: &str, func: &str) -> String {
    let path = write_temp(tag, src);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_VAR_TABLE", func)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = fa::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| l.contains("[vartable]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The type every `__vdb_N` backing in a var table is declared with (`ref(<def>)`).
fn backing_types(table: &str) -> Vec<String> {
    table
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .nth(2)
                .is_some_and(|n| n.starts_with("__vdb_"))
        })
        .filter_map(|l| l.split_whitespace().nth(3).map(str::to_string))
        .collect()
}

/// loft#1757, `@FR-H-ClearRelease` — a `vector<T>` local's backing store is the one-field
/// `main_vector<T>` record, and a clear that releases element heap reads `T` off it.  A
/// tuple-destructured local and the owned copy a borrowed vector arm is given (`_mvcopy_N`)
/// were backed by `main_vector<vector<T>>`: their builders handed `vector_db` the vector's
/// whole type where it takes the element type.  Every local here is a `vector<integer>`, so
/// every backing must be the SAME definition as the plain local's.
#[test]
fn a_vector_local_is_backed_by_its_own_element_type() {
    let destructure = var_table_of(
        "vdb_destructure",
        "fn pair(n: integer) -> (vector<integer>, vector<integer>) { ([n, n + 1], [n * 2]) }\n\
         fn main() {\n\
         \x20 (xa, xb) = pair(3);\n\
         \x20 ys: vector<integer> = [];\n\
         \x20 ys += [1];\n\
         \x20 println(\"{len(xa)} {len(xb)} {len(ys)}\");\n\
         }\n",
        "n_main",
    );
    // The owned copy of a borrowed arm keeps a backing of its own in this corpus file's `g1`
    // (a smaller spelling promotes it into the return buffer, which has none to check).
    let arm_copy = var_table_of(
        "vdb_arm_copy",
        &fa::read_to_string("tests/scripts/508-empty-arm-real-empty-vector.loft")
            .expect("read the 508 corpus file"),
        "n_g1",
    );
    for (what, table, want) in [("destructure", &destructure, 3), ("arm copy", &arm_copy, 2)] {
        let types = backing_types(table);
        assert_eq!(
            types.len(),
            want,
            "{what}: expected {want} backings:\n{table}"
        );
        assert!(
            types.iter().all(|t| *t == types[0]),
            "{what}: every vector<integer> local must be backed by one main_vector<integer> \
             def, got {types:?}:\n{table}"
        );
    }
}
