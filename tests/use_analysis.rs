// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN25 — the USE-analysis copy-vs-borrow VERDICT, tested in isolation via the
// behaviour-neutral `LOFT_MATERIALIZE_DUMP` (it prints one `MAT fn=… verdict=…` line
// per recognised vector-copy binding and changes no codegen). This is the dependable
// layer the elision builds on; the test pins the verdict per boundary cell so we can
// iterate the analysis without wiring it into emission. Design:
// doc/claude/plans/25-nullable-sequences/{use-analysis-prework,materialization-algorithm}-design.md.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Write `src` to a probe path no concurrent test can also produce, and return it.
///
/// Every helper here spawns the loft binary on a source it first writes to a temp
/// file. They used to name that file after a HASH OF THE SOURCE, so two tests
/// sharing one `const …_SRC` — `OWN_SRC` backs three — resolved to the same path.
/// Both write identical bytes, which is why it read as safe, but writing
/// TRUNCATES first: under nextest each test is its own process, so one test's
/// `loft` could open the file while another was rewriting it and analyse a
/// truncated program. It surfaced as `ownership_resolves_the_borrow_base`
/// reporting a borrow base derived from nothing on a fraction of runs, and
/// passing every time it was run alone.
///
/// Uniqueness comes from the pid (distinct per test process under nextest) plus
/// a per-process counter (distinct per call, which is what `cargo test`'s
/// threads need). `stem` only makes the file recognisable while debugging.
/// Callers delete the file once the run is done — with a unique name per call,
/// nothing else ever reclaims it.
fn write_probe(dir_name: &str, stem: &str, src: &str) -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(dir_name);
    std::fs::create_dir_all(&dir).expect("probe dir");
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{}_{n}_{stem}.loft", std::process::id()));
    std::fs::write(&path, src).expect("write probe");
    path
}

/// Run the loft binary on a source string with the verdict dump on; return stderr.
fn dump(src: &str) -> String {
    let path = write_probe("loft_use_analysis", "probe", src);
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_NO_CACHE", "1")
        // The oracle assertions are hand-computed against the RAW shapes; the
        // (now default-on) match-return synthesis rewrites the borrowed arm to
        // an owned copy (`_mvcopy_N`) before the oracle reads it.  Opt out so
        // the ground truth stays expressible; the synthesis's own observable
        // effect is pinned separately (join_own_match_return_strips_the_borrow).
        .env("LOFT_NO_JOIN_OWN", "1")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Assert the verdict line for a function contains the expected verdict.
fn assert_verdict(stderr: &str, func: &str, want_verdict: &str) {
    let line = stderr
        .lines()
        .find(|l| l.contains(&format!("fn=n_{func} ")))
        .unwrap_or_else(|| panic!("no MAT line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("verdict={want_verdict}")),
        "{func}: expected {want_verdict}, got: {line}"
    );
}

/// Assert the Stage-1 ownership classification of a function's RETURN value
/// (`OWN fn=n_<func> return=<Owned|Borrowed|Join>`).
fn assert_own_return(stderr: &str, func: &str, want: &str) {
    let needle = format!("OWN fn=n_{func} return=");
    let line = stderr
        .lines()
        .find(|l| l.contains(&needle))
        .unwrap_or_else(|| panic!("no OWN return line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("return={want}")),
        "{func} return: expected {want}, got: {line}"
    );
}

/// Assert an owned-slot REASSIGNMENT of `var` in `func` carries the expected
/// prior/rhs classes (`OWN fn=n_<func> reassign v=…(var) prior=… rhs=…`).
fn assert_reassign(stderr: &str, func: &str, var: &str, prior: &str, rhs: &str) {
    let head = format!("OWN fn=n_{func} reassign ");
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&head) && l.contains(&format!("({var})")))
        .unwrap_or_else(|| panic!("no OWN reassign line for {func}/{var}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("prior={prior} rhs={rhs}")),
        "{func}/{var}: expected prior={prior} rhs={rhs}, got: {line}"
    );
}

/// Assert NO owned-slot reassignment is reported for `func` (the WORKING
/// discriminator — a single-def slot carries no displaced owned store).
fn assert_no_reassign(stderr: &str, func: &str) {
    let head = format!("OWN fn=n_{func} reassign ");
    if let Some(line) = stderr.lines().find(|l| l.starts_with(&head)) {
        panic!("{func}: expected no reassign site, got: {line}");
    }
}

/// Assert a Stage-1.5 free SITE of the given kind in `func` carries the expected
/// class and borrow base (`OWN fn=n_<func> free kind=<kind> … class=<class> base=<base>`).
fn assert_free_site(stderr: &str, func: &str, kind: &str, class: &str, base: &str) {
    let head = format!("OWN fn=n_{func} free kind={kind} ");
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&head))
        .unwrap_or_else(|| panic!("no OWN free {kind} line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("class={class}")) && line.ends_with(&format!("base={base}")),
        "{func} free {kind}: expected class={class} base={base}, got: {line}"
    );
}

/// Assert NO free site is reported for `func` (a clean shape has no over-free site).
fn assert_no_free_site(stderr: &str, func: &str) {
    let head = format!("OWN fn=n_{func} free ");
    if let Some(line) = stderr.lines().find(|l| l.starts_with(&head)) {
        panic!("{func}: expected no free site, got: {line}");
    }
}

const SRC: &str = r#"
struct Sim { tiles: vector<integer> not null, walls: vector<integer> not null }

// read-only local off a param field -> BORROW (the tile_at / edge_wall_raw shape)
fn tile_at(s: Sim, i: integer) -> integer {
  t = s.tiles;
  if i < len(t) { t[i] ?? 1 } else { 1 }
}
fn edge_at(s: Sim, i: integer) -> integer {
  w = s.walls;
  if i < len(w) { w[i] ?? 0 } else { 0 }
}

// the local is element-mutated -> COPY
fn mutate(s: Sim, i: integer) -> integer {
  t = s.tiles;
  t[0] = 9;
  t[i] ?? 0
}

// the source is a LOCAL (not a parameter) — Tier-0 cannot prove its store
// outlives the local, so it stays a COPY (widened in a later tier)
fn local_src(i: integer) -> integer {
  tmp = Sim { tiles: [4, 5, 6], walls: [0] };
  u = tmp.tiles;
  u[i] ?? 0
}

// the local is handed to a user function (may mutate / escape) -> COPY
fn sink(x: vector<integer>) -> integer { len(x) }
fn to_user(s: Sim) -> integer {
  w = s.tiles;
  sink(w)
}

// the SOURCE is read by a non-mutating callee -> still BORROW. This is the
// interprocedural precision the shared `find_written_vars` buys: a purely
// intraprocedural source check would have to assume `peek` might mutate `s`.
fn peek(s: Sim) -> integer { len(s.tiles) }
fn via_peek(s: Sim, i: integer) -> integer {
  u = s.tiles;
  d = peek(s);
  u[i] ?? d
}

// the SOURCE is mutated through a callee while the local is live -> COPY
// (caught only because the mutation analysis is interprocedural).
fn poke(s: &Sim) { s.tiles[0] = 1; }
fn via_poke(s: Sim, i: integer) -> integer {
  u = s.tiles;
  poke(s);
  u[i] ?? 0
}

fn main() {
  s = Sim { tiles: [1, 2, 3], walls: [0, 0, 0] };
  print(
    "{tile_at(s, 1)} {edge_at(s, 1)} {mutate(s, 1)} {local_src(1)} {to_user(s)} {via_peek(s, 1)} {via_poke(s, 1)}\n"
  );
}
"#;

#[test]
fn verdicts_per_boundary_cell() {
    let stderr = dump(SRC);
    // read-only param-field accessors elide to a borrow
    assert_verdict(&stderr, "tile_at", "Borrow");
    assert_verdict(&stderr, "edge_at", "Borrow");
    // every divergence event forces the conservative copy
    assert_verdict(&stderr, "mutate", "Copy"); // D1: local written
    assert_verdict(&stderr, "local_src", "Copy"); // D3: non-param source (Tier-0 limit)
    assert_verdict(&stderr, "to_user", "Copy"); // D3/escape: passed to a user fn
    // interprocedural source-mutation (¬D2) via the shared find_written_vars
    assert_verdict(&stderr, "via_peek", "Borrow"); // callee only reads the source
    assert_verdict(&stderr, "via_poke", "Copy"); // callee mutates the source
}

// ── Tier 1 (LOFT_ELIDE_T1): read-only LOCAL source, ordering-proven ────────────

/// Run with the verdict dump on at an explicit elision tier (env-selected).
fn dump_at_tier(src: &str, tier: u8) -> String {
    let path = write_probe("loft_use_analysis_t1", &format!("probe_t{tier}"), src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_NO_CACHE", "1")
        // Same raw-shape contract as `dump` — see the note there.
        .env("LOFT_NO_JOIN_OWN", "1");
    if tier >= 1 {
        cmd.env("LOFT_ELIDE_T1", "1");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const T1_SRC: &str = r#"
struct G { c: vector<integer> not null }
fn bump(g: &G) { g.c[0] = 99; }
fn ro_local() -> integer { x = G { c: [10,20,30] }; v = x.c; v[1] ?? -1 }
fn d2_mut_src() -> integer { x = G { c: [10,20,30] }; v = x.c; x.c[0] = 99; v[0] ?? -1 }
fn callee_mut() -> integer { x = G { c: [10,20,30] }; v = x.c; bump(x); v[0] ?? -1 }
fn rebind() -> integer { x = G { c: [10,20,30] }; v = x.c; x = G { c: [7,8,9] }; v[0] ?? -1 }
fn in_loop() -> integer {
  acc = 0;
  for i in 0..3 { x = G { c: [10,20,30] }; v = x.c; acc += v[0] ?? 0; }
  acc
}
fn main() { print("{ro_local()} {d2_mut_src()} {callee_mut()} {rebind()} {in_loop()}\n"); }
"#;

/// The tier GATE: at Tier 0 a read-only local source stays Copy; Tier 1 flips
/// exactly that case to Borrow and nothing else.
#[test]
fn tier1_gate_flips_only_readonly_local_source() {
    // Tier 0 (default) — every local-source case is Copy.
    let t0 = dump_at_tier(T1_SRC, 0);
    for f in ["ro_local", "d2_mut_src", "callee_mut", "rebind", "in_loop"] {
        assert_verdict(&t0, f, "Copy");
    }
    // Tier 1 — ONLY the read-only local source borrows; the divergence /
    // adversarial cells stay Copy (¬D2 via callee, rebind, loop back-edge).
    let t1 = dump_at_tier(T1_SRC, 1);
    assert_verdict(&t1, "ro_local", "Borrow");
    assert_verdict(&t1, "d2_mut_src", "Copy"); // source mutated after the fill
    assert_verdict(&t1, "callee_mut", "Copy"); // mutated through a callee
    assert_verdict(&t1, "rebind", "Copy"); // source reconstructed after the fill
    assert_verdict(&t1, "in_loop", "Copy"); // copy-fill under a loop back-edge
}

/// Cross-check: the existing `local_src` boundary cell (Copy at Tier 0) is exactly
/// a Tier-1 shape and must flip to Borrow under the flag.
#[test]
fn tier1_flips_existing_local_src_cell() {
    assert_verdict(&dump_at_tier(SRC, 0), "local_src", "Copy");
    assert_verdict(&dump_at_tier(SRC, 1), "local_src", "Borrow");
}

/// RUNTIME guard: with Tier 1 actually wired into the elision (the flag makes
/// `elide_borrows` consume the new freeable-LOCAL-source plans), the full matrix —
/// safe borrows + unsafe copies — stays value-correct on BOTH backends with no
/// leak. A wrong borrow on a freeable local would be a UAF / wrong value here.
#[test]
fn tier1_runtime_correct_both_backends() {
    let script = "tests/scripts/85-tier1-local-source-matrix.loft";
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend, script])
            .env("LOFT_ELIDE_T1", "1")
            .env("LOFT_STORES", "warn")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stdout.contains("tier1-local-source-matrix ok"),
            "{backend} tier1 matrix failed:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} tier1 matrix leaked a store:\n{stderr}"
        );
    }
}

// ── @PLN85 Stage 1: the Owned|Borrowed|Join ownership classification ───────────
// The over-free class needs ONE carried fact — for a value escaping into an owned
// position (return / reassign / append), is its store Owned, Borrowed, or a runtime
// Join? This pins that classification (still INERT — printed under the same dump,
// wired into no codegen) on the three live over-free shapes + the FIXED field-view
// family. Design + boundary map:
// doc/claude/plans/85-store-lifetime-retirement/{over-free-class-study,NEXT-SESSION-join-ownership-analysis}.md

const OWN_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn dflt() -> M { M{hp:0, name:""} }

// elem_accumulate ROOT: `t[i] ?? dflt()` is owned on the dflt() arm, borrowed on
// the t[i] arm — a runtime JOIN the flattened return dep (M["t"]) hides. The same
// `pick` underlies BOTH the source-free (UAF) and the all-owned (CLEAN) repros:
// they are statically IDENTICAL — Join — and the fix (materialise the borrow arm
// to owned) makes both correct regardless of which branch runs.
fn pick(t: vector<M>, i: integer) -> M { t[i] ?? dflt() }

// elem_accumulate: `out += [pick(t,i)]` lowers to an OpCopyRecord WITHOUT the 0x8000
// source-free bit. `pick` may hand back an element view of `t`, and this site emits no
// @P290 bracket, so there is no runtime witness to tell its borrow arm from its mint
// arm — the conservative answer, and no AppendSource site.
fn collect(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..len(t) { out += [pick(t, i)]; } out }

// local_source ROOT (#462 leak): `chosen` first OWNS dflt(), then is reassigned to
// a JOIN — the displaced owned store leaks. The over-free shape = prior Owned, rhs
// Join. The return itself is Owned (a materialized_view_return mints a fresh store).
fn pick_cond(t: vector<M>, salt: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = dflt();
  np = len(pool);
  for wj in 0..np { if salt % np == wj { chosen = pool[wj] ?? dflt(); } }
  chosen
}
// WORKING discriminator: a single-def `chosen` displaces no owned store → no
// reassign site → no leak.
fn pick_uncond(t: vector<M>, idx: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = pool[idx] ?? dflt();
  chosen
}

// match_return ROOT: a match arm delivers a borrowed enum-field view; the other
// arm an empty owned vector → the return is a JOIN.
struct E { hp: integer not null, name: text }
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }

// field-view family (all FIXED / clean on the boundary map): a struct-field view
// and a whole-arg view are plain BORROWS of a parameter — safe to return.
struct Box { items: vector<integer> }
fn getf(b: Box) -> vector<integer> { b.items }
fn whole(v: vector<integer>) -> vector<integer> { v }
// a nested-field view roots its base through the projection chain to `o`.
struct Inner { rows: vector<E> }
struct Outer { inner: Inner }
fn nested(o: Outer) -> vector<E> { o.inner.rows }

fn main() {
  t: vector<M> = []; for k in 0..3 { t += [M{hp:k, name:"m"}]; }
  a = pick(t, 0); b = pick_cond(t, 1); c = pick_uncond(t, 0); cc = collect(t);
  cell = Filled { items: [] }; d = deliver(cell);
  bx = Box{ items: [1, 2, 3] }; r = getf(bx); w = whole([4, 5]);
  oo = Outer{ inner: Inner{ rows: [] } }; nn = nested(oo);
  print("{a.hp} {b.hp} {c.hp} {len(cc)} {len(d)} {len(r)} {len(w)} {len(nn)}\n");
}
"#;

/// The Stage-1 ownership fact, pinned per over-free shape. This is the VERDICT the
/// Stage-3 free sites will read; the test gates the analysis in isolation (nothing
/// emits off it yet), so it can be iterated before any codegen change.
#[test]
fn ownership_classifies_the_over_free_shapes() {
    let stderr = dump(OWN_SRC);

    // dflt mints a fresh struct -> Owned (the owned arm of every `??` join below).
    assert_own_return(&stderr, "dflt", "Owned");

    // elem_accumulate: `t[i] ?? dflt()` is a runtime Join. (The CLEAN/all-owned
    // repro uses this SAME `pick` — statically Join too; the static fact cannot
    // and need not distinguish the runtime branch, and Join-awareness covers both.)
    assert_own_return(&stderr, "pick", "Join");

    // local_source: the displaced-owned-store leak is `chosen` reassigned from an
    // OWNED init to a JOIN. The WORKING (single-def) form has no such site.
    assert_reassign(&stderr, "pick_cond", "chosen", "Owned", "Join");
    assert_no_reassign(&stderr, "pick_uncond");
    // pick_cond's RETURN is the fresh materialized store, not the join slot.
    assert_own_return(&stderr, "pick_cond", "Owned");

    // match_return: borrowed enum-field arm vs empty owned arm -> Join. The empty arm
    // reaches the verdict through `Defs::filled` (loft#704): it APPENDS into the retbuf
    // rather than assigning it, and a `Set`-only scan saw no owned alternative at all,
    // so the split read as a plain Borrowed-of-`e`. `vector<integer>` items had always
    // read Borrowed here for that reason; both element types now read Join.
    assert_own_return(&stderr, "deliver", "Join");

    // field-view family (fixed/clean): a field view and a whole-arg view are plain
    // parameter Borrows — never freed at the return.
    assert_own_return(&stderr, "getf", "Borrowed");
    assert_own_return(&stderr, "whole", "Borrowed");
}

/// Stage 1.5 (Gaps A+B): the analysis surfaces the FREE SITES the value
/// classification alone does not — the append element source-free
/// (`elem_accumulate`) and the return-buffer-aliasing delivery (`match_return`) —
/// each with the freed value's class and the borrow base to materialise from. Still
/// inert; this is the context the Stage-3 fix reads. See ownership-analysis-gaps.md.
#[test]
fn ownership_surfaces_free_sites() {
    let stderr = dump(OWN_SRC);

    // elem_accumulate USED to source-free pick's JOIN return here. It no longer does:
    // `pick` may hand back an element view of `t`, and a site that emits no @P290
    // bracket has no runtime witness to tell the borrow arm from the mint arm, so it
    // sets no bit. The over-free this pinned is the fault, not the feature — the
    // fixture's own comment on `pick` calls it "the source-free (UAF) repro". See the
    // companion assertion in `ownership_resolves_the_borrow_base` for the measurement
    // that no leak replaces it.
    assert_no_free_site(&stderr, "collect");

    // match_return: the retbuf `_mv_items_1` is reassigned to a BORROWED enum-field
    // view (`OpGetField(e,…)`) → freeing the buffer over-frees `e`'s field. The
    // borrow base `e` is carried so the materialise copies the field into the buffer.
    assert_free_site(&stderr, "deliver", "ParamDeliver", "Borrowed", "e");

    // the clean shapes carry NO over-free site: a returned param borrow is not freed.
    assert_no_free_site(&stderr, "getf");
    assert_no_free_site(&stderr, "whole");
    // local_source's bug is the reassign, not an append/deliver: its pool-build
    // append lowers WITHOUT the 0x8000 source-free bit, so no AppendSource site.
    assert_no_free_site(&stderr, "pick_cond");
}

/// Assert the FULL ownership of a function's return, base included
/// (`OWN fn=n_<func> return=<Owned|Borrowed(base=…)|Join(base=…)>`).
fn assert_return_own(stderr: &str, func: &str, want: &str) {
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&format!("OWN fn=n_{func} return=")))
        .unwrap_or_else(|| panic!("no OWN return line for {func}; dump:\n{stderr}"));
    assert!(
        line.ends_with(&format!("return={want}")),
        "{func} return: expected {want}, got: {line}"
    );
}

/// The unification oracle's BORROW BASE — the witness the Stage-3 runtime guard
/// needs — compared against hand-computed ground truth across the shapes, including
/// the INTERPROCEDURAL call→arg translation (the new piece) and the documented
/// retbuf-delivery approximation. This is the fact every own-vs-borrow site will
/// read; the test validates it in isolation before any chokepoint consumes it.
#[test]
fn ownership_resolves_the_borrow_base() {
    let stderr = dump(OWN_SRC);

    // direct projection / `??` join roots its base to the borrowed PARAM.
    assert_return_own(&stderr, "pick", "Join(base=t)"); // t[i] ?? dflt()
    assert_return_own(&stderr, "deliver", "Join(base=e)"); // match arm of e
    assert_return_own(&stderr, "nested", "Borrowed(base=o)"); // o.inner.rows — chain → o

    // `out += [pick(t,i)]` no longer HAS an over-free site, so there is none to pin
    // (loft#1647 + the split that followed it): `pick` may hand back an element view
    // of `t`, the vector-append site emits no @P290 bracket, and with no runtime
    // witness the conservative answer is to set no source-free bit at all. Same
    // reason `pick_cond` carries no AppendSource site two assertions down.
    //
    // Measured before re-blessing, because the conservative answer has a stated cost
    // and a pin must not hide it: the MINT arm of this exact shape
    // (`out += [pick(t, i + 900)]`, every index out of range) leaks NOTHING on either
    // backend — the `??` join owns `dflt()`'s store, not the append. So the site is
    // gone and no leak replaces it.
    //
    // What this no longer exercises, said plainly: the oracle's INTERPROCEDURAL base
    // step (pick's borrowed param `t` -> collect's argument `t`) was visible only
    // through that free site. `pick`'s own `Join(base=t)` above still pins the
    // intraprocedural half. Restoring the interprocedural half needs a site that
    // still sets the bit for a borrowing callee, which today means a bracket-emitting
    // site — `expressions.rs`'s collection bind is the only one.  The record spelling
    // of the may-borrow class cannot be a corpus cell at all yet: it raises a
    // pre-existing `BUG (#306)` stack-store free refusal (loft#1651).
    assert_no_free_site(&stderr, "collect");

    // the displaced-owned reassign's borrow arm roots to the local `pool`.
    assert_reassign(&stderr, "pick_cond", "chosen", "Owned", "Join"); // rhs base = pool
    let cond_line = stderr
        .lines()
        .find(|l| l.contains("fn=n_pick_cond reassign") && l.contains("(chosen)"))
        .unwrap();
    assert!(
        cond_line.ends_with("rhs=Join(base=pool)"),
        "pick_cond chosen rhs base: {cond_line}"
    );

    // KNOWN APPROXIMATION (retbuf delivery): a whole-field / whole-arg return is
    // delivered through `__retbuf`, whose fill is not a tracked Set — so the base
    // resolves to `__retbuf`, not the true source `b`/`v`. Harmless: these are clean
    // field-return sites, never an over-free. Pinned so a future fix is a visible diff.
    assert_return_own(&stderr, "getf", "Borrowed(base=__retbuf)");
    assert_return_own(&stderr, "whole", "Borrowed(base=__retbuf)");
}

// ── @PLN85 match_return — the resisting case, pinned against the oracle ─────────
// The match_return codegen collapse is still open (the retbuf-promotion site). These
// tests pin what the NEW routine (`ownership_of`) classifies for the resisting
// variants, so the fix has a VERIFIED SPEC and the precise discriminator is locked.
// Key finding: the RETURN verdict `Join` is NOT the discriminator — it over-classifies
// the fresh-build case (`deliver3 → Join(base=o)`, the retbuf-param approximation,
// runtime-clean). The precise fix signal is the `ParamDeliver` FREE SITE: a retbuf
// reassigned to a borrowed enum-field view (base = an EXTERNAL var `e`), which the
// genuinely-leaking arms have and the fresh-build does not.
const MATCH_VARIANTS_SRC: &str = r#"
struct E { hp: integer not null, name: text }
fn e_default() -> E { E{hp:0, name:""} }
// V1: Filled arm borrows `e`; else arm is owned `[]` — a runtime JOIN (LEAKS today).
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }
// V2: BOTH field arms borrow `e` (+ implicit owned default) — also a borrow-of-`e`.
enum Two { A { xs: vector<E> }, B { ys: vector<E> } }
fn deliver2(e: Two) -> vector<E> { match e { A { xs } => { xs }, B { ys } => { ys } } }
// V3: the Filled arm builds a FRESH owned vector `o` — owned, runtime-clean. The
// oracle over-classifies its RETURN as Join(base=o) (the retbuf approximation), but
// it has NO ParamDeliver site (the precise discriminator excludes it).
fn deliver3(e: Cell) -> vector<E> {
  match e { Filled { items } => { o: vector<E> = []; for x in 0..len(items) { o += [items[x] ?? e_default()]; } o }, _ => { [] } }
}
fn main() {
  c = Filled { items: [] }; r = deliver(c);
  t = A { xs: [] }; r2 = deliver2(t);
  r3 = deliver3(c);
  print("{len(r)} {len(r2)} {len(r3)}\n");
}
"#;

/// The match_return resisting cases pinned against the oracle. The `ParamDeliver`
/// FREE SITE — a retbuf aliased to a borrowed enum-field view whose base is an
/// EXTERNAL var — is the precise fix discriminator; the `Join` RETURN verdict alone
/// is not (it over-classifies the fresh-build `deliver3`). This is the verified spec
/// the still-open codegen fix must act on: materialise exactly the ParamDeliver arms.
#[test]
fn ownership_pins_match_return_resisting_cases() {
    let stderr = dump(MATCH_VARIANTS_SRC);

    // V1/V2 — the GENUINE over-free arms: the retbuf is aliased to a borrowed
    // enum-field view of the EXTERNAL subject `e`. The ParamDeliver site (base=e) is
    // the precise fix signal; the return verdict is Join(base=e). V1 reaches it through
    // `Defs::filled` (loft#704) — its owned `[]` arm fills the retbuf instead of
    // assigning it — while V2 has no `[]` arm and never needed it.
    assert_free_site(&stderr, "deliver", "ParamDeliver", "Borrowed", "e");
    assert_return_own(&stderr, "deliver", "Join(base=e)");
    assert_free_site(&stderr, "deliver2", "ParamDeliver", "Borrowed", "e");
    assert_return_own(&stderr, "deliver2", "Join(base=e)");

    // V3 — the FRESH-BUILD arm: owned, runtime-clean, and BOTH arms own their value, so
    // there is no split to name. What it gets is the retbuf-param approximation (`o`,
    // the owned retbuf, classified as borrowed-of-itself) — `o` is never re-bound, only
    // filled, so loft#704's owned alternative has nothing to join with and deliberately
    // does not fire: claiming Owned here would tell a callee it may free the CALLER's
    // buffer. There is no ParamDeliver site either, so the fix keyed on that (NOT on the
    // return verdict) correctly leaves deliver3 alone.
    assert_return_own(&stderr, "deliver3", "Borrowed(base=o)"); // documented over-classification
    assert_no_free_site(&stderr, "deliver3"); // the precise discriminator excludes it
}

/// loft#704 — the in-place FILL as an owned alternative, pinned in BOTH directions.
///
/// A var comes to hold a value two ways: a `Set` re-binds it, and an append/clear fills
/// it. The classifier scanned only the first, so a `match` whose borrowed arm re-binds
/// and whose `[]` arm fills read as a plain Borrowed — the split `Join` exists to name
/// was invisible. `filled_or_borrowed` is that shape.
///
/// The other direction is the one that has to hold for the fix to be SOUND, and it is
/// what keeps the rule from being "fills are owned": `fill_only` never re-binds its
/// buffer, so there is nothing to join with and it keeps the documented retbuf-param
/// reading. Its value really is owned — but the buffer belongs to the CALLER, and a
/// callee told `Owned` may free it. An `Owned` here would be the unsound direction.
const FILL_SRC: &str = r#"
struct Fe { hp: integer }
enum FCell { FEmpty, FFilled { items: vector<Fe> } }
enum ICell { IEmpty, IFilled { items: vector<integer> } }

// A borrowed arm (re-binds the retbuf) and an owned `[]` arm (fills it) -> Join.
fn filled_or_borrowed(e: FCell) -> vector<Fe> {
  match e { FFilled { items } => { items }, _ => { [] } }
}
// The element type must not change the verdict: this read Borrowed even before
// loft#699 made every element type materialise its `[]` arm alike.
fn filled_or_borrowed_int(e: ICell) -> vector<integer> {
  match e { IFilled { items } => { items }, _ => { [] } }
}
// FILL-ONLY: the buffer is never re-bound, so the owned alternative must NOT fire.
fn fill_only(v: vector<integer>) -> vector<integer> { o: vector<integer> = []; o += v; o }
// Both arms owned, both fills — likewise no re-bind, so likewise unchanged.
fn both_owned(c: boolean) -> vector<integer> { if c { [1, 2] } else { [] } }

fn main() {
  c = FFilled { items: [Fe { hp: 1 }] };  a = filled_or_borrowed(c);
  i = IFilled { items: [7] };             b = filled_or_borrowed_int(i);
  d = fill_only([5, 6]);
  e = both_owned(true);
  print("{len(a)} {len(b)} {len(d)} {len(e)}\n");
}
"#;

#[test]
fn ownership_counts_an_in_place_fill_as_an_owned_alternative() {
    let stderr = dump(FILL_SRC);

    // The recovered Join — the borrowed arm's base survives as the witness.
    assert_return_own(&stderr, "filled_or_borrowed", "Join(base=e)");
    assert_return_own(&stderr, "filled_or_borrowed_int", "Join(base=e)");

    // The soundness half: a buffer that is only ever FILLED keeps the retbuf-param
    // reading. `Owned` here would license a callee to free the caller's buffer.
    assert_return_own(&stderr, "fill_only", "Borrowed(base=o)");
    assert_return_own(&stderr, "both_owned", "Borrowed(base=__retbuf)");
}

// ── @PLN85 Stage-3 site 1: the `local_source` compiler wiring (LOFT_JOIN_OWN) ───
// The displaced-owned-store leak: `chosen = dflt()` move-adopts a fresh store into
// `chosen` (the source retbuf the cleanup guards is left null), then `chosen =
// pool[wj]` orphans it. The fix strips `chosen`'s flattened `["pool"]` dep so it is
// OWNED everywhere — the owned path deep-copies the borrow into `chosen`'s store and
// frees it. VALUE-correct either way (the leak does not corrupt); only the leak
// differs, so the gate is the `not freed` warning. See ownership-analysis-gaps.md.

const LOCAL_SOURCE_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn dflt() -> M { M{hp:-1, name:"d"} }
fn pick(t: vector<M>, salt: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = dflt();
  np = len(pool);
  for wj in 0..np { if salt % np == wj { chosen = pool[wj] ?? dflt(); } }
  chosen
}
fn main() {
  t: vector<M> = []; for k in 0..4 { t += [M{hp:k * 10, name:"m"}]; }
  // np=4: pick(t, salt) selects pool[salt % 4] -> hp = (salt % 4) * 10.
  for i in 0..8 {
    r = pick(t, i);
    assert(r.hp == (i % 4) * 10, "pick i{i} hp={r.hp} want={(i % 4) * 10}");
    assert(len(t) == 4, "src corrupted i{i} len={len(t)}");
  }
  print("local-source ok\n");
}
"#;

/// Run a source on a backend, optionally with `LOFT_JOIN_OWN`; return (stdout, stderr).
fn run_backend(src: &str, backend: &str, join_own: bool) -> (String, String) {
    let path = write_probe("loft_join_own", "probe", src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args([backend])
        .arg(&path)
        .env("LOFT_STORES", "warn")
        .env("LOFT_NATIVE_LEAK_CHECK", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "180");
    if join_own {
        // Post-flip the fixes are DEFAULT-ON: the on-leg actively removes the
        // opt-out so an ambient `LOFT_NO_JOIN_OWN` cannot invert the premise.
        cmd.env_remove("LOFT_NO_JOIN_OWN");
        cmd.env_remove("LOFT_NO_OWNER_WITNESS");
    } else {
        // The control leg (the documented pre-fix behaviour) opts out — of BOTH cures.
        // loft#1336's owner witness closes the `local_source` leak on its own, so the
        // A/B that pins join-own as the closing mechanism has to hold the witness off
        // too, or the control leg reads clean and the gate proves nothing.
        cmd.env("LOFT_NO_JOIN_OWN", "1");
        cmd.env("LOFT_NO_OWNER_WITNESS", "1");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The fix: under `LOFT_JOIN_OWN` the displaced-owned `local_source` shape is
/// VALUE-correct AND leak-free on BOTH backends.
#[test]
fn join_own_fixes_local_source_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(LOCAL_SOURCE_SRC, backend, true);
        assert!(
            stdout.contains("local-source ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} still leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

// ── @PLN85 unification (first chokepoint collapse): elem_accumulate, interp ────
// `out += [pick(t,i)]` source-frees pick's `t[i] ?? m_none()` JOIN return. The
// interp first-bind now reads the ownership oracle: OpBindOrCopy adopts the owned
// `m_none()` arm and materialises the borrowed `t[i]` arm, witnessed by the oracle's
// interprocedurally-resolved base `t`. BOTH arms must be value-correct + clean.
const ELEM_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn m_none() -> M { M{hp:-1, name:"n"} }
fn pick(t: vector<M>, i: integer) -> M { t[i] ?? m_none() }
fn collect(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..len(t) { out += [pick(t, i)]; } out }
fn collect_owned(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..3 { out += [pick(t, i + 100)]; } out }
fn filler(n: integer) -> integer { es: vector<M> = []; for j in 0..n { es += [M{hp:j, name:"f"}]; } return len(es); }
fn main() {
  t: vector<M> = []; for k in 0..3 { t += [M{hp:k * 10, name:"m"}]; }
  for i in 0..6 {
    r = collect(t); acc = 0; for f in 0..6 { acc += filler(6); }
    assert(len(t) == 3, "src corrupted i{i}={len(t)}");
    assert(len(r) == 3, "borrow-arm len i{i}={len(r)}");
    o = collect_owned(t);
    assert(len(o) == 3, "owned-arm len i{i}={len(o)}");
  }
  print("elem-accumulate ok\n");
}
"#;

/// The unification collapse fixes BOTH arms of elem_accumulate on BOTH backends (the
/// borrow arm's interp UAF and the owned arm's leak — native's `_src == _dst` guard
/// leaked the owned arm too) — value-correct and clean under LOFT_JOIN_OWN, witnessed
/// by the oracle's interprocedurally-resolved base.
#[test]
fn join_own_fixes_elem_accumulate_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(ELEM_SRC, backend, true);
        assert!(
            stdout.contains("elem-accumulate ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} still leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

/// The GATE: WITHOUT the flag the same shape is value-correct but LEAKS the
/// displaced owned store — pins that the flag is what closes the leak (so a future
/// regression that silently stops stripping is caught), on both backends.
#[test]
fn local_source_leaks_without_join_own() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(LOCAL_SOURCE_SRC, backend, false);
        assert!(stdout.contains("local-source ok"), "{backend}: {stderr}");
        assert!(
            stderr.contains("not freed"),
            "{backend}: expected the displaced-owned LEAK without the flag, got none:\n{stderr}"
        );
    }
}

// ── @PLN85 match_return — the gated owned-copy synthesis (P1) ───────────────────
// `jo_copy_borrowed_arm_yield` (src/parser/control.rs) rewrites a borrowed enum-field
// arm yield (`Filled { items } => { items }`) into an owned copy
// (`{ o = []; o += items; o }`), so the return escapes OWNED rather than as a view
// into the match subject. Two plain-loft codegen bugs blocked it — the gen_if arm-join
// discard and the empty value-block push (tests/scripts/441 + 442) — and are now fixed,
// so the whole-append owned copy "just works" (no element-loop synthesis needed).

const MATCH_RETURN_SRC: &str = r#"
struct E { hp: integer not null, name: text }
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }
fn filler(n: integer) -> integer { es: vector<E> = []; for j in 0..n { es += [E{hp:j, name:"f"}]; } return len(es); }
fn main() {
  inner: vector<E> = []; for k in 0..3 { inner += [E{hp:k * 10, name:"m"}]; }
  cell = Filled { items: inner };
  for i in 0..6 {
    r = deliver(cell); acc = 0; for f in 0..6 { acc += filler(6); }
    assert(len(inner) == 3, "src corrupted i{i}={len(inner)}");
    assert(len(r) == 3, "match-return len i{i}={len(r)}");
    assert(r[0].hp == 0 && r[2].hp == 20, "match-return values i{i}");
    e = deliver(Empty {});
    assert(len(e) == 0, "empty-arm i{i}={len(e)}");
  }
  print("match-return ok\n");
}
"#;

/// Run `loft introspect` on a source, optionally with `LOFT_JOIN_OWN`; return stdout
/// (the IR dump, used to read the emitted return-type dependency).
fn introspect(src: &str, join_own: bool) -> String {
    let path = write_probe("loft_join_own", "introspect", src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("introspect").arg(&path).env("LOFT_NO_CACHE", "1");
    if join_own {
        cmd.env_remove("LOFT_NO_JOIN_OWN");
    } else {
        cmd.env("LOFT_NO_JOIN_OWN", "1");
    }
    let out = cmd.output().expect("spawn loft introspect");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The `deliver` function's IR signature line from an introspect dump (carries the
/// top-block dependency set, e.g. `…["__retbuf", "e"]`).
fn deliver_signature(dump: &str) -> String {
    dump.lines()
        .find(|l| l.starts_with("fn n_deliver(e:ref"))
        .unwrap_or_else(|| panic!("no deliver signature in introspect dump:\n{dump}"))
        .to_string()
}

/// The gated synthesis is value-correct AND leak-free on BOTH backends now that the
/// blocking codegen bugs are fixed — the whole-append owned copy "just works".
#[test]
fn join_own_match_return_synthesis_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(MATCH_RETURN_SRC, backend, true);
        assert!(
            stdout.contains("match-return ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

/// The GATE discriminator. The borrowed-arm return BORROWS the match subject `e`
/// without the flag (`["__retbuf", "e"]`) and is OWNED with it (`["__retbuf"]`). The
/// runtime is clean either way now (the over-free no longer surfaces as a leak once
/// the codegen bugs are fixed), so the emitted return dependency is the flag's
/// observable effect — this pins that the synthesis is what strips the borrow.
#[test]
fn join_own_match_return_strips_the_borrow() {
    let off = deliver_signature(&introspect(MATCH_RETURN_SRC, false));
    let on = deliver_signature(&introspect(MATCH_RETURN_SRC, true));
    assert!(
        off.contains("[\"__retbuf\", \"e\"]"),
        "without the flag deliver should borrow `e`, got: {off}"
    );
    assert!(
        on.contains("[\"__retbuf\"]"),
        "with the flag deliver should be owned (no `e` borrow), got: {on}"
    );
}

// ── @PLN90 phase 1 — construction / field-append copies are covered by the verdict ──
// A struct/enum field built from an existing vector deep-copies it (the field owns its
// data). Before @PLN90 the verdict saw only the var-buffer copy idiom (`o = src` / `o +=
// src`) and missed this dominant category entirely; now it emits a Copy row so the
// copy-vs-borrow decision covers it (diagnostic only — always Copy, never an ElidePlan).

const CONSTRUCT_SRC: &str = r#"
struct E { hp: integer not null, name: text }
struct Box { rows: vector<E> }
fn make_box(s: vector<E>) -> Box { Box { rows: s } }
fn main() {
  s: vector<E> = []; for k in 0..3 { s += [E{hp:k, name:"s"}]; }
  b = make_box(s);
  assert(len(b.rows) == 3, "rows");
  print("construct ok\n");
}
"#;

#[test]
fn construction_copy_is_covered_by_the_verdict() {
    let stderr = dump(CONSTRUCT_SRC);
    // `Box { rows: s }` deep-copies `s` into the new record's field — a Copy the verdict
    // now classifies (it formerly recorded only var-target appends).
    assert_verdict(&stderr, "make_box", "Copy");
    assert!(
        stderr.contains("construction/field-append copy"),
        "the construction copy should carry the construction reason; dump:\n{stderr}"
    );
    // @PLN90 phase 2 — a construction copy is IMPLICIT to the model (the field owns its
    // data) → silent, NOT warned. It is not the avoidable worklist.
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_make_box ") && l.contains("bucket=implicit")),
        "construction copy should be bucket=implicit (silent); dump:\n{stderr}"
    );
}

// ── @PLN90 phase 1 — the return-buffer (field / whole-vector return) copy is covered ──
// `fn f(b: Box) -> vector { b.rows }` materialises `b.rows` into the passed-in return
// buffer (`__retbuf`). The buffer is an argument, not a fresh `OpDatabase` local, so the
// var-buffer copy idiom skipped it; now a Copy row covers it (diagnostic only — eliding it
// to a borrow would be the P4 borrowed-return).
const FIELD_RETURN_SRC: &str = r#"
struct E { hp: integer not null, name: text }
struct Box { rows: vector<E> }
fn field_ret(b: Box) -> vector<E> { b.rows }
fn main() {
  s: vector<E> = []; for k in 0..3 { s += [E{hp:k, name:"s"}]; }
  bx = Box { rows: s };
  r = field_ret(bx);
  assert(len(r) == 3, "rows");
  print("field-return ok\n");
}
"#;

#[test]
fn field_return_copy_is_covered_by_the_verdict() {
    let stderr = dump(FIELD_RETURN_SRC);
    assert_verdict(&stderr, "field_ret", "Copy");
    assert!(
        stderr.contains("materialised into the return buffer"),
        "the field-return copy should carry the return-buffer reason; dump:\n{stderr}"
    );
    // @PLN90 phase 2 — a field-return copy is AVOIDABLE (bucket 2, the elimination
    // worklist): a borrowed-view return is sound; the copy is only there because the
    // borrowed return path is not yet correct (@PLN85 P4).
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_field_ret ") && l.contains("bucket=AVOIDABLE")),
        "field-return copy should be bucket=AVOIDABLE; dump:\n{stderr}"
    );
}

// ── @PLN90 phase 1 — the `OpCopyRecord` record copy is covered (the last gap) ──
// `v[i] = e` deep-copies the record `e` into the element slot. This is not append-based,
// so neither the var-buffer idiom nor the construction / return-buffer branches see it;
// now a Copy row covers it (diagnostic only). The same-var no-op alias is excluded.
const RECORD_COPY_SRC: &str = r#"
struct E { hp: integer not null, name: text }
fn set_one(v: vector<E>, e: E) -> integer { v[1] = e; return v[1].hp; }
fn main() {
  v: vector<E> = []; for k in 0..3 { v += [E{hp:k, name:"v"}]; }
  e = E{hp:99, name:"z"};
  r = set_one(v, e);
  assert(r == 99, "set");
  print("record-copy ok\n");
}
"#;

#[test]
fn record_copy_is_covered_by_the_verdict() {
    let stderr = dump(RECORD_COPY_SRC);
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_set_one ") && l.contains("record deep-copy (OpCopyRecord)")),
        "the `v[i] = e` record copy should be classified Copy [record deep-copy]; dump:\n{stderr}"
    );
}

// ── @PLN90 phase A — the bound-vs-unbound survival split (items 1–4) ──
// Under `LOFT_COPY_SURVIVAL` a construction / record copy is classified by its SOURCE FATE:
// a MOVE (source consumed) is silent (Implicit); a still-live source duplicated is indicated
// (Avoidable read-only / Forced written-after). Item 4: a copy inside a loop whose source is
// defined OUTSIDE the loop is duplicated every iteration → Avoidable; a per-iteration LOCAL
// source is a move → Implicit. (Item 1's `Internal` — a compiler-generated `_`-prefixed source
// that genuinely SURVIVES — is real but does not fire in this corpus: every compiler temp here
// is a per-iteration move; the tally still carries `internal_copies=`.)
const SURVIVAL_SRC: &str = r#"
struct Wrap { data: vector<integer> }
fn mk(n: integer) -> Wrap { Wrap { data: [n] } }
fn cmove() { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; print("{s.data[0]}\n"); }
fn csurv() { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; print("{inner[0]} {s.data[0]}\n"); }
fn cmut()  { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; inner += [4]; print("{s.data[0]} {inner[3]}\n"); }
fn loop_outside() {
    x = mk(9);                              // defined OUTSIDE the loop
    v: vector<Wrap> = [mk(0), mk(0), mk(0)];
    for i in 0..3 { v[i] = x; }             // x duplicated every iteration → Avoidable (item 4)
    print("{v[0].data[0]}\n");
}
fn loop_local() {
    v: vector<Wrap> = [mk(0), mk(0), mk(0)];
    for i in 0..3 { y = mk(i); v[i] = y; }  // y is a fresh per-iteration local → move → Implicit
    print("{v[1].data[0]}\n");
}
fn recset() {
    v: vector<Wrap> = [Wrap { data: [0] }];
    e = Wrap { data: [1, 2, 3] };
    v[0] = e;
    print("{v[0].data[0]}\n");
}
fn main() { cmove(); csurv(); cmut(); loop_outside(); loop_local(); recset(); }
"#;

/// Like `dump`, but with the survival split flag on.
fn dump_survival(src: &str) -> String {
    let path = write_probe("loft_use_analysis", "survival", src);
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_COPY_SURVIVAL", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_JOIN_OWN", "1")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_bucket(stderr: &str, func: &str, bucket: &str) {
    assert!(
        stderr.lines().any(
            |l| l.contains(&format!("fn=n_{func} ")) && l.contains(&format!("bucket={bucket}"))
        ),
        "{func}: expected a copy row bucket={bucket}; dump:\n{stderr}"
    );
}

#[test]
fn survival_split_bound_vs_unbound_and_internal() {
    let stderr = dump_survival(SURVIVAL_SRC);
    // Bound → silent; unbound → indicated.
    assert_bucket(&stderr, "cmove", "implicit"); // move: source consumed
    assert_bucket(&stderr, "csurv", "AVOIDABLE"); // unbound, read-only survivor
    assert_bucket(&stderr, "cmut", "forced"); // unbound, written after
    // Item 4 — loop source scope: outside the loop ⇒ duplicated every iteration (Avoidable);
    // a per-iteration local ⇒ a move (Implicit).
    assert_bucket(&stderr, "loop_outside", "AVOIDABLE");
    assert_bucket(&stderr, "loop_local", "implicit");

    // Item 1: the Internal mechanism is present in the tally (a compiler-generated surviving
    // source would land there, excluded from the user-facing avoidable/forced set). It does not
    // fire in this corpus (every compiler temp here is a per-iteration move → Implicit).
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("MAT-WORKLIST") && l.contains("internal_copies=")),
        "the worklist tally must carry an internal_copies count; dump:\n{stderr}"
    );

    // Item 2: a `v[i] = e` record copy carries its source location `at <file>:<line>:<col>`,
    // so a row that has no useful target name is still actionable. The exact target
    // attribution varies with lowering, so key only on a LOCATED Copy row in `recset`.
    assert!(
        stderr.lines().any(|l| l.contains("fn=n_recset ")
            && l.contains("verdict=Copy")
            && l.contains(" at ")
            && l.contains(".loft:")),
        "the record-set copy row should carry an `at <file>:<line>` location; dump:\n{stderr}"
    );

    // Default (flag OFF) keeps every construction copy Implicit AND emits no location suffix
    // (position tracking is gated) — byte-identical to the phase-1 dump.
    let off = dump(SURVIVAL_SRC);
    for f in ["cmove", "csurv", "cmut"] {
        assert_bucket(&off, f, "implicit");
    }
    assert!(
        !off.lines()
            .any(|l| l.contains("MAT fn=") && l.contains(" at ")),
        "flag-off dump must carry no ` at <loc>` suffix (byte-identical); dump:\n{off}"
    );
}

// ── @PLN90 phase B (B1.3) — the RECORD-shape last-use MOVE-elision rewrite ──
// Under `LOFT_MOVE_ELIDE`, a construction/record copy whose owned source is DEAD after the copy
// is lowered by building the source's fields DIRECTLY into the destination slot (retarget every
// `OpSet*`, drop the `OpDatabase` / `OpCopyRecord` / source free) instead of copy-then-free. The
// rewrite is a pure IR pass emitting NO new op, so BOTH backends must preserve values AND stay
// leak-free; the survivor case (source read after the copy) must still COPY. Gated: OFF is a
// no-op (byte-identical).
const MOVE_ELIDE_SRC: &str = r#"
struct E { hp: integer, name: text }
struct Outer { tag: integer, inner: E }
fn t1_elem() -> integer {           // v[i] = e — element set, source dead → MOVE
    v: vector<E> = [E{hp:1,name:"a"}, E{hp:2,name:"b"}];
    e = E { hp: 9, name: "z" };
    v[0] = e;
    v[0].hp
}
fn t2_field() -> integer {          // o.inner = src — record field replacement, dead → MOVE
    o = Outer { tag: 7, inner: E{hp:1,name:"a"} };
    src = E { hp: 40, name: "z" };
    o.inner = src;
    o.inner.hp + o.tag
}
fn t5_survive() -> integer {        // source read after the set → must COPY (no move)
    v: vector<E> = [E{hp:1,name:"a"}];
    e = E { hp: 9, name: "z" };
    v[0] = e;
    e.hp + v[0].hp
}
fn main() { print("{t1_elem()} {t2_field()} {t5_survive()}\n"); }
"#;

/// Run a move-elision probe on one backend, optionally with the gate on. Returns
/// (stdout, stderr, success). `--native` compiles via `rustc`, so the leak check for that mode
/// rides on `LOFT_NATIVE_LEAK_CHECK`.
fn run_move_elide_src(
    src: &str,
    stem: &str,
    native: bool,
    move_on: bool,
) -> (String, String, bool) {
    let path = write_probe("loft_use_analysis", stem, src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg(if native { "--native" } else { "--interpret" })
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_POISON", "1")
        .env("LOFT_NATIVE_LEAK_CHECK", "1");
    // The move-elision is DEFAULT ON (B1.5 flip); the OFF baseline opts out.
    if !move_on {
        cmd.env("LOFT_NO_MOVE_ELIDE", "1");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Assert a probe's stdout is `expected` and leak-free on BOTH backends, gate on and off.
fn assert_move_elide_preserves(src: &str, stem: &str, expected: &str) {
    for native in [false, true] {
        let (off_out, _, off_ok) = run_move_elide_src(src, stem, native, false);
        let (on_out, on_err, on_ok) = run_move_elide_src(src, stem, native, true);
        let mode = if native { "native" } else { "interp" };
        assert!(
            off_ok && on_ok,
            "{mode}: run failed (off={off_ok} on={on_ok})"
        );
        assert!(
            off_out.trim() == expected && on_out.trim() == expected,
            "{mode}: value changed under move-elision\n off={off_out:?}\n on={on_out:?}"
        );
        assert!(
            !on_err.contains("stores not freed"),
            "{mode}: move-elision leaked a store\n---- stderr ----\n{on_err}"
        );
    }
}

// @speed 1.1
#[test]
fn move_elide_record_preserves_values_and_stays_leak_free() {
    // t1=9, t2=47 (40+7), t5=18 (9+9, two independent copies). The gate must not change ANY
    // value on either backend, and must not leak (the freed `e`/`src` store is now the moved-in
    // one; the survivor still copies + frees its own).
    assert_move_elide_preserves(MOVE_ELIDE_SRC, "move_elide_b13_probe", "9 47 18");
}

/// The IR block for `fn n_<name>(…` in an `introspect` dump (up to the next `fn`/`byte-code`).
fn ir_block(introspect: &str, name: &str) -> String {
    let start = format!("fn n_{name}(");
    let mut out = String::new();
    let mut in_block = false;
    for line in introspect.lines() {
        if line.starts_with(&start) {
            in_block = true;
        } else if in_block && (line.starts_with("fn n_") || line.starts_with("byte-code for")) {
            break;
        }
        if in_block {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Count of a given deep-copy op in one function's introspect IR block.
fn op_count(introspect: &str, name: &str, op: &str) -> usize {
    ir_block(introspect, name).matches(op).count()
}

fn introspect_move_elide_src(src: &str, stem: &str, move_on: bool) -> String {
    let path = write_probe("loft_use_analysis", stem, src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["introspect"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1");
    // The move-elision is DEFAULT ON (B1.5 flip); the OFF baseline opts out.
    if !move_on {
        cmd.env("LOFT_NO_MOVE_ELIDE", "1");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn move_elide_record_drops_the_copy_but_keeps_the_survivor_copy() {
    let off = introspect_move_elide_src(MOVE_ELIDE_SRC, "move_elide_b13_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_SRC, "move_elide_b13_ir", true);
    // Each dead-source move drops exactly its own `v[i]=e` / `o.f=src` deep copy (a function may
    // retain OTHER copies — e.g. `t2_field` still copies the literal inner `E` when building `o`).
    for moved in ["t1_elem", "t2_field"] {
        let (b, a) = (
            op_count(&off, moved, "OpCopyRecord"),
            op_count(&on, moved, "OpCopyRecord"),
        );
        assert!(
            a == b - 1,
            "{moved}: move-elision should drop exactly one OpCopyRecord (off={b} on={a})"
        );
    }
    // The survivor is read after the set → its copy is load-bearing and must remain.
    let (sb, sa) = (
        op_count(&off, "t5_survive", "OpCopyRecord"),
        op_count(&on, "t5_survive", "OpCopyRecord"),
    );
    assert!(
        sa == sb && sb >= 1,
        "t5_survive: a read-after-set survivor must still COPY (off={sb} on={sa})"
    );
}

// ── @PLN90 phase B (B1.3b) — the CONSTRUCT field-append move-elision (reorder-free only) ──
// `x.field += src` with `src` dead after: build src's elements DIRECTLY into `x.field` (drop the
// copying `OpAppendVector`), ONLY when the container already exists (local built before src, or a
// param). Fresh construction (`a = Bag { items: base }`, container built AFTER base) needs a
// reorder and must stay a copy. Same both-backend value + leak invariant as the Record case.
const MOVE_ELIDE_CONSTRUCT_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
fn t1_local() -> integer {                  // local container built before src → MOVE
    x = Bag { id: 1, items: [1, 2] };
    src: vector<integer> = [10, 20, 30];
    x.items += src;
    (x.items[4] ?? 0)
}
fn t2_param(x: Bag) -> integer {            // param container → MOVE (no reorder ever needed)
    src: vector<integer> = [7, 8];
    x.items += src;
    (x.items[2] ?? 0)
}
fn t4_fresh() -> integer {                  // container built AFTER base → NOT reorder-free → COPY
    base: vector<integer> = [10, 20, 30];
    a = Bag { id: 5, items: base };
    (a.items[1] ?? 0) + a.id
}
fn main() { print("{t1_local()} {t2_param(Bag{id:0,items:[100]})} {t4_fresh()}\n"); }
"#;

// @speed 1.1
#[test]
fn move_elide_construct_field_append_preserves_values_and_stays_leak_free() {
    // t1=30, t2=8 (param [100] + [7,8] → index 2), t4=25 (20 + 5).
    assert_move_elide_preserves(MOVE_ELIDE_CONSTRUCT_SRC, "move_elide_b13b_probe", "30 8 25");
}

#[test]
fn move_elide_construct_elides_field_append_and_fresh_construction() {
    let off = introspect_move_elide_src(MOVE_ELIDE_CONSTRUCT_SRC, "move_elide_b13b_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_CONSTRUCT_SRC, "move_elide_b13b_ir", true);
    // B1.3b field-appends (local + param container) AND the B1.3c fresh construction (`t4_fresh`,
    // container built after the source → hoist + retarget) all drop their copying OpAppendVector.
    for moved in ["t1_local", "t2_param", "t4_fresh"] {
        let (b, a) = (
            op_count(&off, moved, "OpAppendVector"),
            op_count(&on, moved, "OpAppendVector"),
        );
        assert!(
            a == b - 1,
            "{moved}: construct move-elision should drop one OpAppendVector (off={b} on={a})"
        );
    }
}

// ── @PLN90 phase B (B1.3c) — fresh-construction reorder + its conservative skip guards ──
const MOVE_ELIDE_FRESH_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
fn e1_single() -> integer {                  // single, literal-only fields → ELIDE
    base: vector<integer> = [10, 20, 30];
    a = Bag { id: 5, items: base };
    (a.items[1] ?? 0) + a.id
}
fn e2_two() -> integer {                     // TWO fresh constructs → "exactly one" guard → SKIP both
    b1: vector<integer> = [1, 2, 3];
    a1 = Bag { id: 1, items: b1 };
    b2: vector<integer> = [4, 5, 6];
    a2 = Bag { id: 2, items: b2 };
    (a1.items[0] ?? 0) + (a2.items[2] ?? 0)
}
fn e3_single_late() -> integer {             // single fresh construct, items last field → ELIDE
    base: vector<integer> = [10, 20, 30];
    a = Bag { id: 9, items: base };
    (a.items[2] ?? 0) + a.id
}
fn main() { print("{e1_single()} {e2_two()} {e3_single_late()}\n"); }
"#;

// @speed 1.2
#[test]
fn move_elide_fresh_construction_values_and_conservative_skips() {
    // Values preserved on both backends, leak-free (e1=25, e2=7, e3=39).
    assert_move_elide_preserves(MOVE_ELIDE_FRESH_SRC, "move_elide_b13c_probe", "25 7 39");
    let off = introspect_move_elide_src(MOVE_ELIDE_FRESH_SRC, "move_elide_b13c_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_FRESH_SRC, "move_elide_b13c_ir", true);
    // Single literal-field fresh constructs elide their copy…
    for moved in ["e1_single", "e3_single_late"] {
        assert!(
            op_count(&on, moved, "OpAppendVector") == op_count(&off, moved, "OpAppendVector") - 1,
            "{moved}: a single literal-field fresh construct should elide its copy"
        );
    }
    // …and two independent fresh constructs in one fn BOTH elide (B1.4 lifted the one-construct cap;
    // the cross-dependent / mutated-param skips are pinned in the multi + param widening tests).
    assert_eq!(
        op_count(&on, "e2_two", "OpAppendVector"),
        0,
        "e2_two: two independent fresh constructs should both elide"
    );
}

// ── @PLN90 phase B (B1.4) — allow a NEVER-WRITTEN parameter as a hoisted field value ──
// A param's value is constant, so hoisting `a`'s construction past `base`'s build reads the same
// value → SAFE to elide. But a param that IS mutated (reassigned, OR mutated via a callee's
// `&`-param in ANY arg position — caught by the interprocedural `find_written_vars`) must stay a
// copy, else the hoist would read a stale value. This is the soundness line B1.4 rides on.
const MOVE_ELIDE_PARAM_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
fn add(x: integer, y: &integer) { y = y + x; }
fn p1_const(n: integer) -> integer {         // n never written → hoist-safe → ELIDE
    base: vector<integer> = [10, 20, 30];
    a = Bag { id: n, items: base };
    (a.items[1] ?? 0) + a.id                 // 20 + n
}
fn p2_reassigned(n: integer) -> integer {    // n reassigned → written → SKIP (stale-read guard)
    base: vector<integer> = [10, 20, 30];
    n = n * 10;
    a = Bag { id: n, items: base };
    (a.items[2] ?? 0) + a.id                 // 30 + n*10
}
fn p3_argmut(n: integer) -> integer {        // n mutated at a callee's arg1 &-param → written → SKIP
    base: vector<integer> = [1, 2, 3];
    add(100, n);
    a = Bag { id: n, items: base };
    (a.items[0] ?? 0) + a.id                 // 1 + (n+100)
}
fn main() { print("{p1_const(7)} {p2_reassigned(4)} {p3_argmut(5)}\n"); }
"#;

// @speed 1.2
#[test]
fn move_elide_param_field_widening_is_sound() {
    // p1=27 (20+7), p2=70 (30+40), p3=106 (1+105). Values must hold on BOTH backends: a wrong
    // hoist of p2/p3 would surface here as a stale-value read, not just a missing optimisation.
    assert_move_elide_preserves(MOVE_ELIDE_PARAM_SRC, "move_elide_b14_probe", "27 70 106");
    let off = introspect_move_elide_src(MOVE_ELIDE_PARAM_SRC, "move_elide_b14_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_PARAM_SRC, "move_elide_b14_ir", true);
    // A never-written param field value elides.
    assert!(
        op_count(&on, "p1_const", "OpAppendVector")
            == op_count(&off, "p1_const", "OpAppendVector") - 1,
        "p1_const: a never-written param field value should elide"
    );
    // A mutated param (reassigned, or mutated via a callee's arg1 &-param) must stay a copy.
    for skipped in ["p2_reassigned", "p3_argmut"] {
        assert_eq!(
            op_count(&on, skipped, "OpAppendVector"),
            op_count(&off, skipped, "OpAppendVector"),
            "{skipped}: a mutated param must NOT be hoisted (stays a copy)"
        );
    }
}

// ── @PLN90 phase B (B1.4) — MULTIPLE fresh constructs in one fn (the one-construct cap lifted) ──
const MOVE_ELIDE_MULTI_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
fn m1_three() -> integer {                   // three independent fresh constructs → all elide
    b1: vector<integer> = [1, 2];
    a1 = Bag { id: 10, items: b1 };
    b2: vector<integer> = [3, 4];
    a2 = Bag { id: 20, items: b2 };
    b3: vector<integer> = [5, 6];
    a3 = Bag { id: 30, items: b3 };
    (a1.items[0] ?? 0) + (a2.items[1] ?? 0) + (a3.items[0] ?? 0)
}
fn m2_crossdep() -> integer {                // a2's field value reads a1 (a local) → a2 SKIPs, a1 elides
    b1: vector<integer> = [7, 8];
    a1 = Bag { id: 100, items: b1 };
    b2: vector<integer> = [9, 10];
    a2 = Bag { id: (a1.id ?? 0), items: b2 };
    (a1.items[0] ?? 0) + (a2.items[1] ?? 0) + (a2.id ?? 0)
}
fn main() { print("{m1_three()} {m2_crossdep()}\n"); }
"#;

// @speed 1.2
#[test]
fn move_elide_multiple_fresh_constructs_per_fn() {
    // m1=10 (1+4+5), m2=117 (7+10+100). Values hold on both backends.
    assert_move_elide_preserves(MOVE_ELIDE_MULTI_SRC, "move_elide_b14multi_probe", "10 117");
    let off = introspect_move_elide_src(MOVE_ELIDE_MULTI_SRC, "move_elide_b14multi_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_MULTI_SRC, "move_elide_b14multi_ir", true);
    // All three constructs in m1 elide (the one-construct-per-fn cap is lifted).
    assert_eq!(
        op_count(&on, "m1_three", "OpAppendVector"),
        0,
        "m1_three: three independent fresh constructs should all elide"
    );
    // m2: a1 elides, but a2 (its field value reads the local a1) is not hoistable → exactly one copy
    // remains — proof both that multiple constructs are handled AND that the cross-dep is caught.
    assert_eq!(
        op_count(&off, "m2_crossdep", "OpAppendVector"),
        2,
        "m2_crossdep baseline: two copies before elision"
    );
    assert_eq!(
        op_count(&on, "m2_crossdep", "OpAppendVector"),
        1,
        "m2_crossdep: a1 elides, the a1-dependent a2 stays a copy"
    );
}

// ── @PLN90 phase B (B1.3d) — the `a.field = base` whole-vector REPLACEMENT (a DOUBLE copy) ──
// `a.field = base` lowers as `base → __p154_rhs → a.field` with an OpClearVector between (two
// OpAppendVector copies). B1.3d detects the idiom structurally and, when `base` is a dead-after
// local, builds it directly into the cleared field — eliminating BOTH copies. A survivor (base read
// after) and a param source stay copies.
const MOVE_ELIDE_REPLACE_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
struct Doc { title: text, lines: vector<text> }
fn r1_replace() -> integer {                 // dead-after local → eliminate both copies
    a = Bag { id: 1, items: [0] };
    base: vector<integer> = [10, 20, 30];
    a.items = base;
    (a.items[1] ?? 0) + a.id                 // 20 + 1 = 21
}
fn r2_heaptext() -> integer {                // old heap-text contents cleared without leaking
    d = Doc { title: "t", lines: ["old1", "old2"] };
    fresh: vector<text> = ["new-a", "new-b", "new-c"];
    d.lines = fresh;
    d.lines.len() + d.title.len()            // 3 + 1 = 4
}
fn r3_survive() -> integer {                 // base read AFTER → must stay a COPY
    a = Bag { id: 1, items: [0] };
    base: vector<integer> = [10, 20, 30];
    a.items = base;
    (base[0] ?? 0) + (a.items[1] ?? 0)       // 10 + 20 = 30
}
fn r4_param(base: vector<integer>) -> integer {  // param source → caller owns → COPY
    a = Bag { id: 2, items: [0] };
    a.items = base;
    (a.items[0] ?? 0) + a.id
}
fn main() { print("{r1_replace()} {r2_heaptext()} {r3_survive()} {r4_param([99, 98])}\n"); }
"#;

// @speed 1.3
#[test]
fn move_elide_whole_vector_replacement_eliminates_double_copy() {
    // r1=21, r2=4, r3=30, r4=101 (99+2). A wrong move of r3/r4 would surface as a wrong value or a
    // leak here (r2 is the old-content-free stress: heap-text lines cleared before the move-in).
    assert_move_elide_preserves(
        MOVE_ELIDE_REPLACE_SRC,
        "move_elide_b13d_probe",
        "21 4 30 101",
    );
    let off = introspect_move_elide_src(MOVE_ELIDE_REPLACE_SRC, "move_elide_b13d_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_REPLACE_SRC, "move_elide_b13d_ir", true);
    // The replacement's TWO copies both vanish for a dead-after local (int + heap-text field).
    for (moved, before) in [("r1_replace", 2usize), ("r2_heaptext", 2)] {
        assert_eq!(
            op_count(&off, moved, "OpAppendVector"),
            before,
            "{moved} baseline: two OpAppendVector copies before elision"
        );
        assert_eq!(
            op_count(&on, moved, "OpAppendVector"),
            0,
            "{moved}: both copies of the whole-vector replacement should be eliminated"
        );
    }
    // A surviving source and a param source keep their copies.
    for skipped in ["r3_survive", "r4_param"] {
        assert_eq!(
            op_count(&on, skipped, "OpAppendVector"),
            op_count(&off, skipped, "OpAppendVector"),
            "{skipped}: a survivor / param source must stay a copy"
        );
    }
}

// ── @PLN90 phase B (B1.4 nested) — constructs inside nested blocks (if / loop bodies) ──
// The reorder-based rewrites walk EVERY block, so a fresh construction or an `a.field = base`
// replacement inside an `if`/loop body is elided too (its a-alloc + base-build + copy are flat
// within that block). A construct whose field value reads an OUTER local still SKIPs (the run-guard
// only allows `a` or a never-written param), and the loop reorder must stay per-iteration correct.
const MOVE_ELIDE_NESTED_SRC: &str = r#"
struct Bag { id: integer, items: vector<integer> }
fn if_branch(cond: boolean) -> integer {     // fresh construct in an if-branch → elide
    if cond {
        base: vector<integer> = [10, 20, 30];
        a = Bag { id: 2, items: base };
        (a.items[1] ?? 0) + a.id             // 22
    } else { 5 }
}
fn loop_body() -> integer {                  // fresh construct per loop iteration (literal) → elide
    total = 0;
    for i in 0..4 {
        base: vector<integer> = [10, 20, 30];
        a = Bag { id: 5, items: base };
        total = total + (a.items[2] ?? 0) + a.id;   // (30+5)*4 = 140
    }
    total
}
fn nested_replace(cond: boolean) -> integer {  // `a.field = base` in an if-branch → elide
    a = Bag { id: 7, items: [0] };
    if cond {
        base: vector<integer> = [100, 200];
        a.items = base;
        (a.items[0] ?? 0) + a.id             // 107
    } else { 9 }
}
fn outer_ref(cond: boolean) -> integer {     // field value reads an OUTER local → SKIP (copy)
    k = 50;
    if cond {
        base: vector<integer> = [1, 2, 3];
        a = Bag { id: k, items: base };
        (a.items[2] ?? 0) + a.id             // 53
    } else { 8 }
}
fn main() {
    print("{if_branch(true)} {loop_body()} {nested_replace(true)} {outer_ref(true)}\n");
}
"#;

// @speed 1.3
#[test]
fn move_elide_handles_constructs_in_nested_blocks() {
    // Values hold on both backends (if=22, loop=140 — per-iteration, replace=107, outer=53).
    assert_move_elide_preserves(
        MOVE_ELIDE_NESTED_SRC,
        "move_elide_nested_probe",
        "22 140 107 53",
    );
    let off = introspect_move_elide_src(MOVE_ELIDE_NESTED_SRC, "move_elide_nested_ir", false);
    let on = introspect_move_elide_src(MOVE_ELIDE_NESTED_SRC, "move_elide_nested_ir", true);
    // Constructs / replacements inside nested blocks elide.
    for moved in ["if_branch", "loop_body", "nested_replace"] {
        assert!(
            op_count(&on, moved, "OpAppendVector") < op_count(&off, moved, "OpAppendVector"),
            "{moved}: a construct in a nested block should elide (off={} on={})",
            op_count(&off, moved, "OpAppendVector"),
            op_count(&on, moved, "OpAppendVector"),
        );
    }
    // A field value reading an OUTER local (not `a`, not a param) stays a copy — the walk reaches it,
    // but the run-guard correctly refuses to hoist past a build it can't prove constant.
    assert_eq!(
        op_count(&on, "outer_ref", "OpAppendVector"),
        op_count(&off, "outer_ref", "OpAppendVector"),
        "outer_ref: a construct reading an outer local must stay a copy"
    );
}

// ── @PLN90 phase B (B1.5 hardening) — corpus shapes the narrow probes missed, found by the
// flag-ON exposure run. Each MUST stay correct (a wrong elision is a use-before-def / self-corrupt
// / build-into-unallocated-container) — the elision either handles it or (safely) leaves the copy.
const MOVE_ELIDE_HARDENING_SRC: &str = r#"
struct S { v: vector<integer> }
struct Inner { x: integer }
struct Outer { items: vector<Inner> }
fn vlit_element() -> integer {              // `[e]` — dest is a FRESH OpNewRecord element (q4 shape)
    e = Inner { x: 42 };
    o = Outer { items: [e] };               // e copied into a fresh element defined AFTER e
    (o.items[0].x ?? -1)                     // 42
}
fn self_assign() -> integer {               // `s.v = s.v[1..]` — source reads the destination (p287)
    s = S { v: [1, 2, 3, 4] };
    s.v = s.v[1..];
    s.v.len()                                // 3
}
fn empty_field_replace() -> integer {       // `s.v = fresh`, s.v starts EMPTY, s built after fresh (p154)
    fresh: vector<integer> = [7, 8, 9];
    s = S { v: [] };
    s.v = fresh;
    (s.v[0] ?? -1) + s.v.len()               // 7 + 3 = 10
}
fn main() { print("{vlit_element()} {self_assign()} {empty_field_replace()}\n"); }
"#;

// @speed 1.2
#[test]
fn move_elide_corpus_hardening_shapes_stay_correct() {
    // A wrong elision of ANY of these surfaces here as a wrong value or a crash, on either backend.
    assert_move_elide_preserves(
        MOVE_ELIDE_HARDENING_SRC,
        "move_elide_harden_probe",
        "42 3 10",
    );
}

// ── @PLN90 Step 5 — the user-facing `--report-copies` report ──
const REPORT_SRC: &str = r#"
struct Item { tags: vector<integer> }
fn main() {
    base: vector<integer> = [1, 2, 3];
    a = Item { tags: base };                 // base survives → avoidable (construction copy)
    print("{a.tags[0]} {base[0]}\n");
    v: vector<Item> = [Item { tags: [0] }];
    e = Item { tags: [9] };
    v[0] = e;                                 // e survives → avoidable (record copy, located)
    print("{v[0].tags[0]} {e.tags[0]}\n");
}
"#;

/// Spawn `loft --report-copies --check` and return its stderr (the report).
fn report(src: &str) -> String {
    let path = write_probe("loft_use_analysis", "report", src);
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--report-copies", "--interpret", "--check"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn report_copies_is_user_facing_and_prints_once() {
    let out = report(REPORT_SRC);
    // Printed exactly ONCE (from main after the whole program is loaded — not per file-load).
    assert_eq!(
        out.matches("loft copy report").count(),
        1,
        "the report must print exactly once; stderr:\n{out}"
    );
    // The user's copies appear (the record copy `v[0] = e` names `Item`).
    assert!(
        out.contains("fn n_main") && out.contains("copies Item"),
        "the user's copies should be reported; stderr:\n{out}"
    );
    // Stdlib copies are excluded — the report is only the survival-split (source-duplication)
    // class, and the stdlib's survival baseline is 0. No return-buffer rows, no `t_*` fns.
    assert!(
        !out.contains("return buffer") && !out.contains("fn t_"),
        "the report must exclude stdlib / return-buffer copies; stderr:\n{out}"
    );
    // The rollup line is present.
    assert!(
        out.contains("unbound cop") && out.contains("avoidable"),
        "the report should carry a rollup; stderr:\n{out}"
    );
}

/// Spawn `loft --check` with (or without) `LOFT_WARN_COPIES` and return its stderr.
fn warn(src: &str, gated_on: bool) -> String {
    let path = write_probe("loft_use_analysis", "warn", src);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1");
    if gated_on {
        cmd.env("LOFT_WARN_COPIES", "1");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// @PLN130 F5 — the copy notice is DEFAULT-ON and needs no flag.
///
/// It was gated (`LOFT_WARN_COPIES`) and silent by default, on the reasoning that it should
/// wait until the Avoidable set was drained. Measured instead: across 585 `tests/scripts`
/// only 27 produce any row (55 rows), and each responds to a small source change — keep the
/// source unused after the copy and it becomes a move, or build the value in place. A notice
/// that is quiet on 95% of programs and actionable on the rest does not need draining first.
///
/// Emitted as `Advice`, not `Warning`: the program is CORRECT and merely pays a copy, and the
/// repo rule is that a diagnostic gates only when ignoring it can produce a wrong result.
#[test]
fn warn_copies_is_default_on_advice() {
    let off = warn(REPORT_SRC, false);
    // @PLN131 gave the notice its code, so the level renders as `advice[avoidable-copy]:`.
    // Matching the tag rather than a bare `advice:` is the point: the code is the frozen
    // handle a fix and a doc section attach to, and losing it would be the regression.
    assert!(
        off.contains("advice[avoidable-copy]:") && off.contains("copy of"),
        "the copy notice must fire with no flag set, carrying its code; stderr:\n{off}"
    );
    assert!(
        !off.contains("warning:"),
        "a copy is correct-but-costly, so it must be advice rather than a warning; stderr:\n{off}"
    );
    // The text must name what the author can DO, not what the analysis concluded.
    assert!(
        off.contains("still used after this point"),
        "the notice must name the lever (the surviving source); stderr:\n{off}"
    );
}

/// loft#781 — a copy inside a DEPENDENCY is reported against the DEPENDENCY's file.
///
/// The copy op carries no span of its own, so it borrows the enclosing line marker: a
/// line number with an EMPTY file. Substituting the entry file there paired a
/// dependency's line with the consumer's path — a real line in the wrong file, which
/// lands on whatever happens to sit there. In one consumer 29 of 67 notices echoed a
/// comment, a blank line or a `const`, and 43% wrong is what makes a reader stop
/// reading the other 57%. The failure mode a DEFAULT-ON diagnostic can least afford.
///
/// The LINE was always correct; only the file was wrong. So this pins the file and the
/// echoed source line TOGETHER — asserting the line alone passes on the bug. The entry
/// file carries a `const` at exactly the library's copy line, so a regression does not
/// merely fail, it reproduces the original symptom.
///
/// loft#1260 put a `loft.toml` at the probe root, which is what makes the notice ADDRESSED
/// here: a lint reaches only whoever can act on its cure, and with the entry and the library
/// in one project that is this author. The attribution rule is unchanged and still needs two
/// files to reproduce — what changed is that the same shape WITHOUT a manifest is a consumer
/// reading someone else's library, and the notice correctly does not reach them at all. The
/// twin that pins that half is `dead_code_lint::a_dependency_dead_store_does_not_reach_a_consumer`.
#[test]
fn a_dependency_copy_is_reported_against_the_dependency_file() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("loft_781_{}_{n}", std::process::id()));
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("probe dirs");
    // One project, so the library is the author's own code and its lints are addressed to
    // this build (loft#1260).
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"probe781\"\nversion = \"0.1.0\"\ncategories = [\"testing\"]\n",
    )
    .expect("write manifest");

    // The copy is line 6 of the library: `xs` is named into the struct and used again
    // on the next line, so it survives and cannot be moved.
    std::fs::write(
        lib.join("copier781.loft"),
        "// 1\npub struct Holder { v: vector<integer> }\n\n\
         pub fn make_it() -> integer {\n  xs = [1, 2, 3];\n  \
         h = Holder { v: xs };\n  return len(xs) + len(h.v);\n}\n",
    )
    .expect("write lib");

    // Line 6 of the ENTRY is a `const` — where the notice used to land.
    let entry = root.join("main781.loft");
    std::fs::write(
        &entry,
        "use copier781;\n// 2\n// 3\n// 4\n// 5\nconst UNRELATED = 0.75;\n// 7\n\
         fn main() {\n  println(\"{make_it()}\");\n}\n",
    )
    .expect("write entry");

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg("--lib")
        .arg(&lib)
        .arg(&entry)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let all = format!("{}{err}", String::from_utf8_lossy(&out.stdout));
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        all.contains("copy of vector<integer>") && all.contains("`xs`"),
        "the cross-file copy notice must still fire; output:\n{all}"
    );
    assert!(
        all.contains("copier781.loft:6"),
        "a copy in a dependency must name the DEPENDENCY's file and line; output:\n{all}"
    );
    assert!(
        !all.contains("main781.loft:6"),
        "the notice must not be attributed to the entry file (loft#781); output:\n{all}"
    );
    // The echoed source line proves the location resolves to the copy, not to whatever
    // sits at that line number elsewhere.
    assert!(
        all.contains("h = Holder { v: xs }"),
        "the echoed line must be the copy site, not a `const` or a comment; output:\n{all}"
    );
}
