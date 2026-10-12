<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Fuzz-proof gate — prove the store-lifetime class closed *by construction*

> **Part of [@PLN85](README.md)** (store-lifetime retirement). **Status:** STANDING —
> the harness is built, the positive controls are live pairs, and the gate runs in
> `cargo test` (`tests/ownership_fuzz_gate.rs`); see § Done criteria for the recorded
> budget and the explicitly-open expansions. **This is wide-release gate 1** (the floor that does not betray you,
> [GOALS.md § The deeper aim](../../GOALS.md); the bar lives in
> [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md)). Written as a
> `design-protocol` hypothesis: the fuzz-proof is the **falsification instrument** for the
> claim "the store-lifetime class is closed," designed to BREAK the invariant, not confirm it.

## Why a separate slot when @PLN85 already closed each cluster

The investigation reached outcome (b): the store-lifetime bugs are *independent mechanisms*,
each fixed at its own chokepoint with a per-cluster regression guard (clusters II / III / V /
C / 462 — see [README.md](README.md)). Those guards prove the **known shapes stay fixed**.
They do **not** prove an **unknown composition** can't violate the invariant. "No new report
this week" is anecdotal silence, not proof — and at one dogfooding agent the class kept
spawning bugs precisely because each new *composition* found a hole the last fix didn't cover.

This slot closes that gap: turn "every known dangerous shape is fixed + guarded" into "the
**class** is closed by construction," proven by a standing generative instrument run at scale.
That is the difference between *quiet* and *sealed* — and only *sealed* clears the gate.

## The one invariant being proven

(From [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md); the `deps` ownership chokepoint.)

> At every program point each heap store has **exactly one owner**; all mutation flows through
> that owner; a non-owning alias is **read-only** and **never outlives** its owner.

The whole class is this one invariant violated four ways — the instrument must catch all four:

| Violation | Observable signal the oracle checks |
|---|---|
| **leak** — ownership dropped, no free | store count grows across runs (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) |
| **double-free** — ownership duplicated | native sanitizer hit / store-table corruption |
| **use-after-free** — alias outlives owner | sanitizer hit / OOB store index (the `65535` family) |
| **silent corruption** — two owners mutate one store | cross-backend **value + length divergence** (the #437 NRVO shape) |

Silent corruption is the dangerous one: no crash, no leak — only a wrong value. So a leak-and-
crash oracle is **insufficient**; the cross-backend value diff is the load-bearing check.

## The instrument (what to build)

**What already EXISTS — reuse, do not rebuild (inventory 2026-06-29).** The cross-backend ORACLE
is built: `tests/differential_oracle.rs` (@PLN89) — `divergences(interp, native)` checks normalised
stdout + exit-code + leak, *with its own positive-control test*; the leak signal is the identical
string `"stores not freed"` on both backends (`LOFT_NATIVE_LEAK_CHECK=1` for native). A `fuzz/`
cargo-fuzz crate exists (libfuzzer + `arbitrary`) with *store-level* targets. **The genuine gaps are
three:** (1) a **program-level generator** (@PLN53 F1/F2, unbuilt); (2) the **`LOFT_POISON`** arena
poison-on-free detector (@PLN54 S3, unbuilt) — the *only* thing that catches the store double-free/UAF
class, because loft's arena reuse defeats stock ASan/Miri/Valgrind; (3) **native-backend ASan**
(@PLN54 S6, unbuilt). So this gate = build the generator (1) + add the arena UAF detector (2), feeding
the existing oracle.

A generative harness that emits **valid loft programs** over the ownership-composition space, runs
each through the oracle, and turns any finding into a minimized regression.

- **Generator grammar — seed from the existing corpus.** The `probes/` directory already
  encodes the known dangerous shapes (matrix A–F, borrowed-view, adopt-free, 462, coalesce);
  the generator's grammar must *reach at least those*, then mutate/compose beyond them.
  Composition axes (the bug-bearing ones from the clusters):
  `delivery {return | bind | arg-pass | field-store}` ×
  `source {local | param | borrowed-view (v[i] / match-arm / if-arm) | nested}` ×
  `value {dense vector | nullable | struct | enum | hash}` ×
  `churn {none | reuse-slot | par}` × `backend {interpret | native+ASan}`.
- **Oracle (all four, every program):** (a) cross-backend **value + length** equality, interp
  as the reference; (b) **zero leak** on both backends; (c) **zero sanitizer finding** on the
  ASan/UBSan native build; (d) **clean process exit** (the teardown-crash trap — "PASSED prints"
  is not enough, check the exit code).
- **Minimize + graduate.** Each counterexample shrinks to a `tests/scripts/85-*.loft` regression
  (the same per-cluster guard mechanism, now fed by the fuzzer instead of by hand).
- **Don't reinvent the harness — focus the existing instrument plans.** @PLN53 (program-level
  fuzzing) and @PLN54 (sanitizer coverage expansion) are the standing instruments; the store-level
  fuzz harness (store.rs LLRB / coalesce / claims) is the layer below. This slot is the
  **@PLN85-specific focusing** of those onto the ownership invariant + the cross-backend oracle —
  it consumes them, it does not duplicate them.

## Build-order dependency — BLOCKED BY @PLN25 (the load-bearing constraint)

The generator's value grammar and the oracle's ownership model **both depend on the settled
value/null model**: what a value *is* (dense vs nullable) and how it copies vs borrows is exactly
what @PLN25 decides, and ownership flows through the `deps` facts that model defines. Fuzzing a
moving value model proves nothing. This is why earlier @PLN85 attempts flailed — there wasn't
enough of @PLN25 settled to know what to build.

Consequence: **@PLN25 leads.** The vectors-half is settled (dense default, merged), so the
fuzz-proof can **start now on the vectors-settled subset** and **expand as scalars land**
(scalars in flight — see [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md)). Do not gate the
whole instrument on @PLN25 being 100% done; gate each composition axis on its value-model piece
being settled.

## Done criteria — what "gate met" means, and where it stands (2026-07-03)

1. The harness runs as a standing job with **zero findings across all four oracles, both
   backends**, over a recorded budget — **MET** for the vectors-settled subset.  The standing
   job is `tests/ownership_fuzz_gate.rs`:
   - **every `cargo test` run**: the positive-control PAIRS (2 generated cells × 2 configs,
     both backends — § self-test below) + the **54-cell interp+poison fast loop**
     (crash / leak / poison-UAF channels; native runs only on a flagged cell, so the clean
     path costs no rustc);
   - **the release-gate sweep** (`cargo test --release --test ownership_fuzz_gate --
     --ignored`): the full 54-cell map with `--poison --native-replay` — all four channels
     on BOTH backends.  Current reading: **0/54** (default gate; 2026-07-03).
   The recorded budget is exactly that — 54 deterministic cells + 4 control runs per CI
   pass, full both-backends replay per release; **no silent cap** (the axes still to add
   are listed under *Open expansions* below).
2. **Coverage is non-vacuous — MET:** the grammar's 9 shapes contain every historical
   cluster shape — match_return (P14 / cluster C), elem_accumulate (P10),
   local_source (the #462 conditional-local-view root), the field-view family
   (clusters II/III), if_return (cluster V), index_read (#426B), nested_field (P13) —
   and the self-test proves the detectors FIRE on exactly those shapes via the preserved
   `LOFT_NO_JOIN_OWN=1` path (measured 6/54 gate-OFF vs 0/54 default, so zero-findings is
   a property of the FIX, not of a narrow grammar).  The historical shapes are also each
   pinned by a graduated `tests/scripts/85-store-lifetime-*.loft` guard (25 as of today).
3. The class is **closed by construction** for the settled composition space; the memory
   model is *sealed on the mapped axes*.  **Open expansions (explicit, not silent):**
   the value axis lacks enum-payload / nested-record / hash pieces (widen as @PLN25-adjacent
   pieces settle), keyed-container views + `par` churn are unmapped, the in-process
   libfuzzer port (below) would add coverage-guided composition BEYOND the grid — and the
   FULL-SUITE poison run is a named open worklist (item 3 below): the mapped axes are
   poison-clean, but `LOFT_POISON=1 cargo test` surfaces ~13 latent stale-reads in
   closure-capture / `&`-place / `par` shapes the grid does not yet generate.

## Status — first increment BUILT (2026-06-29): `fuzz/ownership_fuzz.py`

A first generator + runner: [fuzz/ownership_fuzz.py](fuzz/ownership_fuzz.py). Two-stage per the perf
split (native compiles via rustc — too slow for a tight loop): **interp fast-loop** with leak check,
**native replay** on every flagged program (cross-backend divergence + native leak). Mutates the
**churn axis** — scales every `0..N` loop/pressure bound — because the over-free class only corrupts
once a freed slot is REUSED. Violation modes: `CRASH` (signal only — a clean `exit=1` that *agrees*
across backends is not a bug, only a `DIVERGENCE` if they disagree), `LEAK`, `DIVERGENCE`.

**Positive control proven (engineering-rigor: the harness can fail).** `--self-test` requires
`probes/over-free-sweep/P14-enum-field-vec.loft` to be flagged — interp **SIGSEGV (signal 11)**, native
passes. It is. A harness silent on P14 would be vacuous.

**Calibration overturned the dated sweep table** — verify against observation, not a stale doc:

| Shape | over-free-sweep/README verdict | ACTUAL on current build (harness) |
|---|---|---|
| **P14** (enum-field vector via match arm + churn) | 🔴 open, interp-only | **LIVE** — interp SIGSEGV, native ok (CRASH + DIVERGENCE) |
| **P10** (accumulate borrowed-view results) | "PASS" | **LIVE** — interp `len(t)=7` (corrupt, own assert fails), native ok (DIVERGENCE) |
| **P3** (mon_one-cond native leak) | 🟠 leak open | **FIXED** — clean both backends, no leak |
| **P9** (struct-field vector borrow + churn) | 🔴 open (interp r=0, native crash) | **FIXED** — clean both backends |
| adopt-re-return NRVO leak (leak-462) | leak open | **FIXED** — no `stores not freed` either backend |

So the live positive controls are **P14 (crash) + P10 (value divergence)**; the leak arm is validated
by the existing oracle's own positive-control test. Baseline run: **2/28 over-free-sweep probes
flagged** (P10, P14), churn-mutation reproduces P14 across all variants.

### Increment 2 — full grammar BUILT (2026-06-29): `fuzz/grammar_gen.py`

Widens past churn to the cross-product **shape (source × delivery) × value × churn**: 6 grounded
shapes × {struct, scalar} × {none, heavy} = **24 self-checking programs**, each asserting the
source-length + delivered-length invariant the over-free violates. Generated programs are valid loft
(the compiler is the validity oracle; agreed-compile-errors drop out). **3/24 flagged — and the
grammar LOCALIZES the live class precisely:**

| cell | result |
|---|---|
| `match_return × struct × heavy` | CRASH(interp **SIGSEGV**) + divergence — the P14 class, regenerated from a clean grammar (not a mutated probe) |
| `match_return × struct × none` | CRASH(interp **SIGABRT**) + divergence — **new:** the match-arm borrow is unsound even WITHOUT slot-reuse |
| `elem_accumulate × struct × heavy` | DIVERGENCE (interp assert-fail, native ok) — the P10 class, regenerated |
| field_return / field_local / field_reassign / if_return (any value) | clean — the **field-view** shapes are FIXED |
| every `scalar` cell | clean — the class is **record-store-specific**; `vector<integer>` does not hit it |

So the live over-free is precisely **struct-value × {match-arm, element-accumulate} source**. The
generator reproduced both live classes from a clean grammar and pinned a new no-churn abort variant —
this is the boundary-matrix the chokepoint fix (Cluster C) must be measured against.

### Increment 3 — `LOFT_POISON` built + validated (2026-06-29)

The arena poison-on-free keystone (@PLN54 S3). A dedicated `LOFT_POISON=1`
(`keys.rs::poison_enabled`) now gates the poison-on-free in `Stores::free_named` — overwriting a
freed store's payload (past the 8-byte header) with `0xDEADBEEF`, so a dangling-`DbRef` read after
free hits loud, deterministic garbage instead of silent stale data. **Works on both backends** —
native-generated code calls the same `free_named`, so one cached env-read covers both; any rustc, no
nightly. This is the store-internal UAF blind spot Miri/ASan/Valgrind share (loft's arena "free" is
not a libc `free()`).

**Validated (design-protocol positive control):**
- **No false positives** — ~46 clean programs stay correct under `LOFT_POISON` on both backends.
- **Exposes a SILENT UAF the differential alone missed** — `elem_accumulate × struct × none` is
  clean without poison (exit 0, plausible stale data) but **SIGSEGVs with it**. That is the detection
  power the cross-backend oracle lacks (both backends *agreed* it was fine), and exactly what S3 is for.
- Makes known bugs louder (P10 assert-fail → crash).

Driven by the harness `--poison` flag (sets `LOFT_POISON=1` for both backends). **Note:**
`LOFT_POISON=1 cargo test` is NOT yet green — it exposes the open over-free class (the same
elem-accumulate / match-arm bugs); that green is a FUTURE done-criterion, met when the Cluster-C
chokepoint fix lands. The detector is the instrument; the class is still open.

### Increment 4 — full probe of the over-free class (2026-06-29)

Added the study's missing axes to the generator — `local_source` (the #462 conditional-local-view
root), `index_read` (#426B), `nested_field` (P13), and a `stress` churn — and ran the **54-cell**
cross-product under differential + `LOFT_POISON` + leak, both backends. The complete boundary map is
in [over-free-class-study.md § Generated boundary map](over-free-class-study.md). Result: the live
class is exactly **struct-value × {match-arm, element-accumulate, conditional-local-view}** with
three distinct signatures (SIGABRT / SIGSEGV / **leak-on-both-backends**); the field-view family +
index-read (#426B) + nested-field are all **clean** (fixed). `local_source` is a **deterministic
both-backends repro of #462** at none-churn — the minimal driving case for the fix. Adjacent axes
left for follow-up: keyed-container views (`hash`/`sorted`), nested-record values, `par`.

**Next, in order:**
1. **Correct the stale catalog** — over-free-sweep/README P10 verdict PASS → divergence; mark P3/P9/leak fixed. ✅ done.
2. **Graduate the generator in-process** — ✅ **DONE (2026-07-03 night):**
   `fuzz/fuzz_targets/program_ownership.rs` — the grammar port, in-process
   (parse → scopes → bytecode → interpret, cloned-cache stdlib, arena
   `poison_free` on), driven by `Arbitrary` past the python grid: variable
   source lengths, churn scales, and SHAPE COMPOSITION (two shapes interleaved
   in one program), plus the @PLN25-settled value axes (single-payload ENUM
   elements, NESTED-record elements; keyed containers remain a follow-up).
   Findings panic (libfuzzer artifacts); oracle = compile-ICE / any runtime
   fault in the total, self-checking programs / store leak at exit.
   Run: `cargo +nightly fuzz run program_ownership` (seed corpus committed:
   the 9×4 grid + composition samples).
   **The port caught TWO REAL BUGS in its first five minutes:**
   - **F1 — the H5 lazy-wrapper mint — FIXED (2026-07-03, at the CONTRACT).**
     Every early theory fell to the instrument.  Not preloaded-stdlib, not
     the fuzzer, not `parse_str`, not even release-vs-debug behavior: the
     divergence is in the BARE stdlib parse in EVERY build, and the "only
     the fuzzer trips" mirage was pure assert placement —
     `[profile.dev.package.loft] debug-assertions = false` compiles the H5
     assert OUT of the lib in every dev/test build, so only cargo-fuzz
     (forces `-Cdebug-assertions`) and explicit
     `RUSTFLAGS="-C debug-assertions=on" cargo test --release` builds ever
     CHECK it (calibration failure: the instrument was absent where we
     believed it green).  A mint-site trace named the producer:
     `parse_reduce` (and the whole reduce/map/filter builtin family)
     early-returns the result type on pass 1 and runs the full desugar on
     pass 2 only, whose iterator machinery lazily mints synthetic
     `vector<T>` + `main_vector<T>` wrapper defs (`Data::vector_def`) —
     `sum_of`'s `reduce(v, 0, __add_int)` minted `vector<integer>` pass-2-
     only (`01_code.loft:1500`, pass1=438 → pass2=440, identical in debug).
     Symmetric pass-1 minting is STRUCTURALLY impossible for the family
     (`map`'s output-element wrapper is the lambda's return type, unresolved
     in pass 1), and the mints are name-keyed idempotent APPENDS — every
     pass-1 def number is untouched, so H5's numbering contract holds.  Fix:
     `assert_pass2_def_attr_stable` now exempts EXACTLY that form (trailing
     `DefType::Vector` named `vector<…>` / `DefType::Struct` named
     `main_vector<…>`); any other pass-2-only def still fails, named.  The
     `LOFT_H5_OFF` escape is REMOVED — the fuzzer runs with H5 fully live
     (post-fix smoke: 2218 execs / 90 s, zero findings).  Standing coverage
     note: the cargo-fuzz build is the ONLY standing job that checks H5
     (test-profile builds compile it out per the Cargo.toml override); the
     one-off class check is `RUSTFLAGS="-C debug-assertions=on" cargo test
     --release`.
     **The DA CALIBRATION RUN (F1's class check — the first time the whole
     suite ever executed with lib debug-assertions ON) found the FULL lazy-
     append law plus a latent-assert inventory.**  Harness note first: a
     non-standard target dir needs `target-da/release/default → ../../default`
     (`project_dir()` hardcodes `target/release|debug`), else every CLI-
     spawning test fails on a missing stdlib.  The one law behind all H5
     firings — *pass 2 may append name-keyed synthetic facts that pass 1
     could not know; every pass-1 def number and attr index stays frozen* —
     now encoded in `assert_pass2_def_attr_stable` as FOUR documented lazy
     forms (each verified long-latent by reproducing on an origin/main
     control build in a throwaway worktree):
     1. `vector<T>`/`main_vector<T>` wrapper defs (the original F1 fix);
     2. generic-INSTANTIATION defs (`t_<LEN><Type>_<fn>` whose `n_<fn>`
        template is `DefType::Generic`) — pass-2-only BY DESIGN
        (`parse_call`: pass 1 predicts the return type only; instantiating
        there would capture the template's still-being-built body IR);
        found via `repro_p205.loft`;
     3. the `__closure` hidden attr (`parse_lambda*` adds it `!first_pass`
        from pass 1's closure record — captures unknown until the body
        parses); minimal repro: a lambda capturing a mutated local
        (`f = fn() { frames += 1; }`), attr 0→1;
     4. a trailing `__work_N` text-return work-buffer promotion the pass-1
        classify could not yet see (repro:
        `tests/scripts/111-format-string-self-slice.loft`, `n_render` 3→4 —
        the self-slice-reassigned text param earns a second work buffer in
        pass 2 only).
     `__ref_N` / `__retbuf` growth — the ref_return drift class the assert
     was BUILT for — stays fatal, as does any non-synthetic append.
     **Latent-assert inventory fixed on the spot:** journal unit tests used
     `Store::new` without `free = false` (→ `new_in_use`, the documented
     entry point); `Store::valid()`'s claims assert now skips FILE-BACKED
     stores (claims are in-memory bookkeeping, not persisted — a reopened
     mmap image has an empty set; same reason poison skips them; fixes
     `ir_read::mmap_file_round_trip_stdlib`); `translate_passes_through_unmapped`
     now tests the PARENT-SHARED row honestly (`with_parent_count(6)`) and a
     new `translate_cross_worker_is_a_debug_panic` twin pins the D11c
     debug-panic/release-passthrough split via
     `cfg_attr(debug_assertions, should_panic)`; `Type::dep_att` (display
     helper) no longer indexes out of bounds when a dep list holds
     post-scopes FRAME numbers (p235 par defs) — unknown indices render
     positionally (`#7`).
     **The FINAL honest DA map** (after the fixes above; three passes —
     53 → 26 → 9 failed targets, the drop from 26 being these fixes): the
     journal `index_grows` cell traced to PRODUCTION code — `Journal::create`
     built its private index store with `Store::new` (`free: true` by
     contract; registration into a `Stores`, which never happens for this
     store, is what clears it) → `new_in_use`; verified 17/17 under DA.
     Post-fix fuzz smoke with the complete law: 2879 execs / 120 s, zero
     findings.
     **OPEN cells routed to the DA-inventory worklist** (each needs its own
     matrix session; all reproduce ONLY under a DA lib build):
     - ~~the 4 "Database N not correctly freed" exit-leak tests
       (`expressions::closure_capture_text`, `issues::n8/p179/pln87`)~~ —
       **ALL FOUR CLOSED (2026-07-04)**, both backends + DA gate + graduated
       `tests/scripts/85-*` guards; see
       [NEXT-SESSION-da-leak-cells.md](NEXT-SESSION-da-leak-cells.md) for the
       fix table + the resolved `skip_free` pass-poison class decision.  The
       `sorted<>`-return store leaking at exit (p188 CLI probe) — real store
       leak, still OPEN, likely the same object as the `format.rs:1213` cell
       below;
     - `[set_var]` width mismatches in stdlib text fns (`_elm_N
       (i_parse_errors)` pushes 16B into a 12B slot in `t_4File_lines` /
       `t_4text_split*`; `prev_cr`/`walked` booleans 8B→1B) — ONE producer
       to find; its warning FLOOD also contaminates every output-comparison
       test under DA (`error_messages::baselines`, `exit_codes::p196`,
       `store_persist_loft`, `wrap::dir` stderr) — fixing the width bug
       clears those as a class;
     - `[generate_set] first-assignment of 'c' contains a Var(0)
       self-reference — parser bug` (the assert's own wording; fires in
       `native_scripts` + `loft_suite`);
     - `generate_call [n_main]: mutable arg expected 8B, generate(Text)
       pushed 16B` (a typed-slot mismatch on a suite script);
     - `get_stack<DbRef>: OOB store_nr=30 (allocations.len()=3) — corrupt
       DbRef on the interpreter stack` (`wrap::dir`);
     - ~~`format.rs:1213` indexes `types[65535]`~~ — **FIXED (2026-07-04)** as
       a byproduct of the p188 leak fix: the panic was the DA exit dumper
       rendering a LEAKED sentinel-typed `sorted<>` store (the discarded
       owned keyed-collection return); freeing the leak removes the render.
     The four items above (stdlib slot-width, `generate_set`/`generate_call`,
     `wrap::dir`) are a SEPARATE codegen-slot-correctness class, NOT leaks —
     see [NEXT-SESSION-da-leak-cells.md](NEXT-SESSION-da-leak-cells.md)
     § "Also open from the DA map".  @PLN85's store-lifetime LEAK class is
     retired (all five DA leak cells closed).
     Lens artifacts (not bugs): the `target-da` stdlib symlink (above) and
     the stale-wasm-rlib guard in `html_wasm` (rebuild the wasm rlib after
     lib edits).
   - **F2 — the literal-`??`-default join leak — FIXED (2026-07-03 night,
     the fuzzer's first ownership catch).**  The matrix redrew the axis:
     LITERAL default vs CALL default (nested-vs-flat was irrelevant).  A
     return joining a borrowed element view with an owned struct-LITERAL
     default (`t[i] ?? N{..}`) put the literal arm's work-ref into
     `return_sources`, whose suppression dropped its free entirely — the
     PRESENT path leaked the (interp-allocating) preamble store once per
     call.  The CALL-default twin was always clean: a Call arm contributes
     no source var, so the work-ref kept its plain pre-return free (the
     captured working emission).  Fix (scopes.rs): a MULTI-source return
     mixing an owned record work-ref with view arms is a runtime JOIN even
     without a null arm — the owned sources route through the same hoist +
     `OpFreeRefIfDistinct` leg as the null-arm join.  Guard:
     `tests/scripts/85-ncc-literal-default-join.loft`.
     **Recorded residual:** when the literal DEFAULT arm actually RUNS, the
     transferred store leaks at the interp CALLER (the fn's returned deps
     type the join as a pure borrow of `t`; native's adopt machinery is
     clean) — the deps-carried-join completion, D-own-1/D-own-2 territory.
     Note the grammar never takes the default path (in-range indices only) —
     an unfuzzed axis, listed, not silent.
     **Post-fix budget: 5,534 executions in 180 s, ZERO findings** (~31
     exec/s in-process — ~150× the rustc-per-program harness).
3. **Build `LOFT_POISON`** (@PLN54 S3) — ✅ done — wired (`keys.rs::poison_enabled` + `allocation.rs` free path), both backends, harness `--poison`; positive control proven. Follow-up: poison freed STACK slots too (S3's second half).  **The poison-green worklist (measured 2026-07-03) — 10 of 13 FIXED same-day**, root: the
return-tail UAF family (return-site frees ran BEFORE the tail expression evaluated —
silent stale reads without poison).  Three fixes, guard
`tests/scripts/85-poison-return-tail-uaf.loft`:
(1) the B5-L3 hoist extended INTO block tails (`scopes::insert_free` — `Set(__ret_N,
tail); frees; Return(__ret_N)`) for value AND text results — fixes the closure/field
captures (p213 ×2, p227, p241) and the general class;
(2) a `RefVar`-typed Var tail derefs its place DbRef AT the Return → excluded from the
fast path so it hoists too — fixes the @PLN87 L3/L4 live reads + amp_rhs (the D-own-5
scalar-place sliver made real);
(3) a fn-ref FIELD READ bound to a local (`c = k.cb`) is a BORROWED fn-ref → marked
`skip_free` (the established convention) so scope exit never frees the CALLER's closure
record through the alias — fixes issue_313 cross-fn + the p15 leak test.
The env-conflict test now self-skips under ambient `LOFT_POISON`.
**The P4-records class — FIXED (2026-07-03, the last 3 of the original 13).**  Root: a
record (Reference / struct-enum) arm-return's store fate is a runtime JOIN — transferred
to the caller on the present/winning arm, an orphan on the other path — and the old
emission resolved it statically-wrong in both directions (unconditional free → the
present path returned a freed store off the eval stack; suppression → the null path
orphaned the preamble store).  Three coordinated mechanisms (scopes.rs), guard
`tests/scripts/85-record-arm-return-join.loft`:
(1) `returned_var_null_unified` — a NULL-arm terminal (`Value::Null` /
`OpNullRefSentinel()`) unifies as a WILDCARD with the other arm's var (P236 extension;
the strict `is_null_terminal` walker refuses to unify through a complex arm), so the
match/if value rides `Return(Var)` / `Return(expr)` instead of the freed-TOS channel —
fixes p54 (+ its layout-sensitivity: multi-line vs single-line match arms only changed
poison VISIBILITY via allocation order, never the bug);
(2) the null-arm record sources stay SUPPRESSED and the return leg hoists the value to
`__ret_N` then frees each source via `OpFreeRefIfDistinct(src, __ret_N)` — the runtime
decides (present: kept/transferred; null: the preamble-allocated placeholder freed, no
orphan) — fixes pln85_nullable_return_caller_binding_freed leak-free on both backends;
(3) at any record-returning return site, every record work-ref `OpFreeRef` becomes
`OpFreeRefIfDistinct(w, ret_var)` — a named local adopting one of TWO candidate arm
stores (`v = Pass{..}; if c { v = Fail{..} }; v`) no longer has the winner freed under
it — fixes par_struct_to_struct_enum_t4.
**Poison round 2 (2026-07-03, same day): the 4 residual cells FIXED, 2 more surfaced.**
- `p241_singleton_text` — the block-VALUE variant of the B5-L3 rule: a non-Void
  block's exit frees ran before the enclosing consumer copied the value out
  (`test_value = { mk()[0] }` — the text tail borrowed the block-local vector's
  element bytes).  Fix: hoist to a `__blk_N` temp (the Set deep-copies the bytes),
  text-typed only; the temp's `String` is LIFTED to function scope (`lift_texts`,
  the `lift_vars` mechanism) because a block-local `String` behind the block's
  `Str` value is E0597 on native (caught by `native_dir`'s 29_match).
- `index_dev_elision_borrower_{interpret,native}` — `block_result`'s Reference arm
  matched raw `Type::Reference`, so a `-> Item?` (`Optional(Reference)`) escaping
  borrowed view fell through every delivery arm and was returned raw while its
  block-local copy store was freed.  Fix: the `.base()` peel on the arm (the same
  peel family as maybe_row / the `-> text?` gate).
- `store_persist_loft::fresh_then_reload_round_trip` — an INSTRUMENT bug, not a
  program bug: a file-backed (`store_persist_bind` mmap) store's memory IS the
  file, so poison-on-free persisted `0xDEADBEEF` into durable state.  Fix:
  `free` skips poisoning file-backed stores (`Store::is_file_backed`).
**Poison round 3 (2026-07-03): 150-i306 FIXED; one leak cell surfaced by the fix.**
- `150-i306-view-return-ownership` — root: at a record-returning return site the
  P4 conditional-free swap covered only `__ref_N` work refs; a NAMED record
  local aliased into the hidden return-buffer param (`best = cand` — the NRVO
  buffer keeps raw-alias Sets by design) kept its UNCONDITIONAL free, killing
  the returned store on the reassigned path.  Fix: the swap covers ANY
  record-typed local's free at such a site (`OpFreeRefIfDistinct(v, ret_var)`;
  distinct stores free exactly as before).  Interp + native value-correct,
  interp leak-free.
- **The native adopt-arm placeholder leak — FIXED (same day):** the ADOPT arm
  replaced the destination slot with `_src`, orphaning `_dst` when real (the
  first-bind `null_named` pre-allocation, or a displaced prior store on
  reassignment).  The arm now frees the real, distinct placeholder first
  (`generation/dispatch.rs` — the same exclusive-ownership assumption the
  COPY arm already makes by clearing `_dst` in place); a same-store NRVO
  adopt and the null-sentinel `_dst` are guard-excluded.  The i306 bisect
  cells + 150-i306 are leak-free on both backends.
- `85-store-lifetime-enum-match-borrowed-view-overfree` — **CLOSED** (verified
  2026-07-03 evening): the conditional-free widening + the adopt-placeholder
  fix covered its shapes; green natively under poison.
- (`html_asyncify` / `html_wasm` under a loaded box are chrome-harness timeout
  flakes, not cells — serial-green every time.)

**The LAST poison cell — CLOSED (2026-07-03 night): the chained
field-of-call class.**  The boundary matrix REDREW the filed scope: chain
depth ≥ 2 rooted at a call result read freed memory at ANY consumption site
(bind / argument / return / implicit tail / loop) on BOTH backends — plus a
second face where `return t.inner.value` (chained field of a named LOCAL)
NRVO-renamed the buffer onto the container and DISCARDED the tail (interp
survived via the eval-stack channel; native returned the typed null — a
silent backend divergence).  Two producer fixes, with depth-1's
`materialized_view_return` emission (copy-then-free) as the captured spec:
(1) `operators.rs` — the inline-ref capture COPIES a projection of a call
    (`OpDatabase` + `OpCopyRecord`, the @C86 escape rule) instead of aliasing
    it into an owning work-ref that argument-lifting later frees mid-chain;
    a direct call result still binds raw (it genuinely adopts the store);
(2) `control.rs::return_field_base_var` — the #425 base walk recurses through
    chained projections and capture blocks (the #489 pattern), so the
    chained-field-of-local return takes the Bind-COPY leg instead of the
    container rename.
Guard: `tests/scripts/85-chained-field-of-call.loft` (t1-t6 + the loop faces,
poison-proven both backends).

**`LOFT_POISON=1 cargo test` IS FULLY GREEN (2026-07-03 21:15 run: 2600/2602,
the two fails are the chrome-harness timeout flakes, both serial-green) — the
deferred @PLN54-S3 done-criterion is MET.**  The poison campaign closed 23
latent memory bugs across four rounds; the memory model is sealed on every
axis the instruments currently see (residual instrument gaps: poison skips
file-backed stores by design; freed STACK slots are S3's unpoisoned second
half).
4. **Minimize + wire as a standing job** — ✅ done (2026-07-03): the historical shapes are pinned as 25 graduated `tests/scripts/85-store-lifetime-*.loft` guards; the harness runs in `cargo test` via `tests/ownership_fuzz_gate.rs` (fast loop + control pairs un-ignored, full both-backends replay `--ignored` as the release sweep).  **Self-test re-pinned (2026-07-03):** the join_own default-ON flip cleaned the P14 probe file on BOTH configs, so the positive control re-anchored on generated cells that still reproduce gate-OFF — a crash/divergence-channel control (`elem_accumulate__struct__heavy`) + a leak-channel control (`local_source__struct__none`), each as a buggy(flagged)/fixed(clean) PAIR, so a flip regression fails the self-test too.
- **Method gate:** every M+ step runs the `design-protocol` skill; this doc IS the hypothesis.
- **Blocked-by reminder:** widen the value axis only as @PLN25 settles it (above).

## See also

- [README.md](README.md) — the @PLN85 clusters this generalizes.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the `deps` invariant the oracle checks.
- [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md) — gate 1, and the
  @PLN25 dependency.
- @PLN53 (program-level fuzzing) · @PLN54 (sanitizer coverage expansion) — the instruments this consumes.
- [@PLN25 RESUME.md](../25-nullable-sequences/RESUME.md) — the value model this is blocked by.
