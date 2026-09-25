<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN11 — `Data` as a store (IR mirrors the `--native` data model)  ·  [loft-lang/plans#11](https://github.com/loft-lang/plans/issues/11)  ·  *(was `@PLN52`)*

## Status — FINISHED (2026-06-05)

**The C71 native-library execution model + the store-backed IR foundation shipped and are proven, the live tail is routed to its canonical homes, and the architectural endpoint is deferred — this plan is closed.**  An interpreted script that `use`s a library auto-compiles the library's native-compilable subgraph to a cdylib and dispatches over the **shared store** (interpreting the rest), byte-identical to the all-interpreted run and **63.6×** faster than interpreting the library; the program cache is **default-on** (3–3.6× startup, parse-skip); the IR is store-backed on both backends (G2/M5 — `generate_inner` / `output_code_node` read through the `IrNode` handle, proven byte-identical to native).  The decision is automatic and invisible (`use <lib>` is native; `LOFT_NO_NATIVE_LIBS` opts out) with a dev-interpret-on-edit fallback.

The **closure ledger** (what shipped · routed-out · deferred) is the next section; everything below it is the dated **build/closure record** (historical) — kept as the how-it-was-built archive, not live work.

### Closure ledger — shipped · routed-out · deferred (2026-06-05)

The deliverable **shipped**: program cache default-on (3–3.6× startup,
parse-skip) + the C71 native-library execution model live end-to-end
(Arc N, Steps 1–4; 63.6× execution speedup measured & guarded) + the
store-backed IR on both backends.  Where the rest went:

1. **Merge (per finish-then-merge).**  The whole branch lands on `main`
   as one finished unit — a coherent, all-opt-in (default-off) foundation
   (the `IrNode` handle, the cross-backing equivalence harness,
   store-backed codegen on both backends, the cache).  The only action
   left for this plan.

2. **F1 — nextest native-test reliability.**  CI-masked (green under
   `make test` + the `ci` nextest profile's `retries = 1`) but flaky
   under raw `cargo nextest` full-suite.
   - **Mode A ✅ FIXED (2026-06-05)** — root cause was **not** a deps
     race but **shared stem-keyed scratch output paths** in
     `tests/native.rs`: `native_tuple_script`,
     `native_tuple_return_script`, and `native_scripts` all compile
     `50-tuples.loft`, and under nextest's process-per-test they
     truncated each other's `loft_native_<stem>_bin` mid-link → SIGBUS.
     Fixed by per-process temp output + atomic `rename(2)` publish (no
     serial group; full parallelism kept).  Forced-collision 0/12,
     broad cold 0/4, cache intact.
   - **Mode B (open, lower urgency)** — `ring/rustls/webpki … not in
     rlib format` from a test-profile dep-layout mismatch.  **Not
     reproduced** in this env (consistent unhashed
     `release/deps/libloft.rlib`).  Fix direction unchanged: resolve the
     cdylib build against the rlib set nextest actually produces, or
     build a consistent set first.
   → § Discovered follow-ups F1.

3. **Arc N dispatch completeness — ROUTED OUT to [`NATIVE.md` § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71) (2026-06-05).**
   The C71 core is shipped + live; the remaining items are *enhancements*
   on a complete, graceful core (a construct that can't cross
   **interprets** — not a bug): closures (`__closure`), `generate_interface`
   aggregate names (`sorted<Item[k]>`), D2a binary schema interface,
   `hash`/`index`/`spacial` coverage, gate-driven dispatch (N4 tail),
   background build (N3 polish).  No longer this plan's scope.

4. **N1 — native-artifact idle-TTL cache (deferred).**  The eviction
   model + its application to the program cache are done; applying the
   idle-TTL to a *native-artifact* cache waits on the registry
   build-cache surface (`~/.loft/build-cache/`) or a new shared local
   native cache — rather than shipping untested registry-gated GC
   (Goal-A "verify, don't assert").  → § Arc N (N1 row).

5. **Architectural endpoint — M6 / M7 / E2 (deprioritised for perf;
   future fresh-context arc, NOT a branch extension).**  M6 (cold-path
   native-graph drop / zero-copy reads — E2-gated), M7 (parser emits
   store IR directly), E2 (locked-mmap writable store).  **E2's perf
   rationale is superseded by C71** — in the native-library model the
   library bodies + variable tables are never materialised at startup,
   so the allocation cost E2 removes isn't incurred; there is also no
   cheap `read_function` win (the cost is allocation-bound, measured).
   E2's value is now purely architectural (self-hosting, store-backed
   IR, mmap), worth it only if warm startup for large whole-program
   bundles becomes a headline goal.  → § Recommendation #2–#3.

### Recommendation — where to go next (2026-06-04)

The codegen migration (M2–M6.warm) is **done and proven**, but the session's
measurements reframed the cost/benefit and point to a clear priority order:

1. ✅ **SHIPPED (2026-06-04) — the program cache is now default-on (track 1).**
   The **3–3.6× startup speedup** (G1, parse-skip) was invisible because it sat
   behind `LOFT_PROGRAM_CACHE`; it is now **on by default**
   (`cache::program_cache_enabled`).  Delivered as four pieces:
   - **Default-on with a kill switch** — `LOFT_NO_CACHE` opts out;
     `LOFT_PROGRAM_CACHE` still force-enables (used by the cache's own tests).
   - **Auto-off under Cargo** — when `CARGO_MANIFEST_DIR` is in the environment
     (`cargo run` / `cargo test`) the cache disables itself, so the whole
     integration-test suite and the compiler-debug loop neither write nor read
     bundles **with zero per-test wiring** (verified: 20 `exit_codes` subprocess
     tests wrote 0 bundles to an isolated cache dir).  This *is* the dev-safety
     default the caveat below called for.
   - **Dev-safety signature** — `build_signature` now also folds in the running
     executable's **mtime** (`binary_signature_tag`), so an *uncommitted* rebuild
     invalidates bundles too (git-HEAD `BUILD_ID` alone did not — see § caveat).
     The release-upgrade build-signature (format/version/build-id/target/features)
     landed earlier in commit `0b9e69a`.
   - **Bounded growth** — `cache::prune_program_cache` evicts whole
     `(.store + .manifest)` pairs oldest-first after each cold save, keeping the
     dir under `LOFT_CACHE_MAX_MB` (default 512 MiB).
   - **First-run save cost — decision:** *accepted.*  The one cold save (~7 MiB
     bundle) per `(script, binary)` is the price of every future warm hit; it is
     paid by real installed invocations only (dev/test/CI are auto-off or use
     `LOFT_NO_CACHE`), bounded by eviction.
   Tests: `cache_decision_precedence`, `prune_dir_evicts_oldest_over_budget`,
   `build_signature_is_deterministic_and_carries_version` (lib) + the existing
   `arc_e_program_cache` / `g2_m6_warm_store` / `p254_cache_poisoning` suites.

2. **Do NOT chase E2 / the full native-graph drop for perf right now.**  The
   key measured finding: **M6-warm gives only ~5%** because it skips only the
   *body* trees (20% of `read_data` — see #3's breakdown), not the def-fields +
   variable tables (80%) that dominate.  The *real* M6/E2 prize is skipping the
   **entire** `read_data` reconstruction — but on the stdlib that is only **~0.7 ms**
   (it scales with def + variable count; a large whole-program bundle is more), and
   it needs **E2** (a writable store IR so `scopes` can rewrite bodies), a `VH`,
   memory-safety-sensitive rewrite of the compiler's largest mutating pass.  Worth it
   *eventually* if startup latency is a headline goal, but as a **planned,
   fresh-context arc**, not a rushed branch extension.

   **E2 deprioritised further (2026-06-04) — superseded-for-perf by the
   native-library execution model** ([DESIGN_DECISIONS.md §
   C71](../../DESIGN_DECISIONS.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model),
   [BROADENING.md § Native-library execution
   model](../../BROADENING.md#native-library-execution-model--the-steady-state-design)).
   In the native-library model (stable/published libraries compile to native
   artifacts; user scripts interpret) the library bodies + variable tables are
   NEVER materialised at startup — the allocation cost E2 eliminates is simply
   not incurred.  E2 therefore drops to low priority for performance.  Its
   architectural value (self-hosting, store-backed IR, mmap) stands on its own;
   the perf rationale no longer applies.  **The forward work is now § Arc N —
   native-library execution model (C71 build-out)** below: the phased,
   timeline-tracked plan for building that architecture (everything except the
   deferred library validation layer).

3. ✅ **PROFILED (2026-06-04) — there is no cheap `read_function` win; the cost is
   allocation-bound, so E2 (zero-copy) is the only lever.**  `bench_read_data_breakdown`
   (`ir_read.rs`, `--ignored`) splits the warm `read_data` on the real stdlib bundle:
   **693 µs total = def-fields 453 µs (65%) + variable-tables 98 µs (14%) + bodies
   142 µs (20%)**.  The variable-table decode is **~0.39 µs/variable** (98 µs / 251
   vars) — so the plan's earlier "~2.2 ms variable tables" figure is a *whole-program*
   bundle (a real consumer with ~5–6k vars), not the stdlib, and it scales **linearly
   with allocation count**: each variable rebuilds a `String` name + a boxed `Type`
   (`read_type_child`), each def rebuilds its attribute + return-type `Type` trees.
   That cost **is** the native-graph materialisation — exactly what E2/zero-copy
   skips wholesale — so a "targeted decode optimisation" cannot beat E2 (there is no
   redundant work to shave; `collect()` already pre-sizes the `names` map from the
   Vec size-hint).  **Conclusion:** drop the "optimise `read_function`" idea; if warm
   startup ever becomes a headline goal for large programs, E2 is the lever, and its
   prize is **the whole `read_data` (~0.7 ms stdlib, scaling with def+var count)**,
   not the ~2.2 ms the variable-table framing implied.

4. **Land this branch.**  ~54 green, all-opt-in (default-off) commits.  The
   `IrNode` handle, the cross-backing equivalence harness, store-backed codegen
   on both backends, and the cache are a coherent, safe-to-merge foundation
   whose architectural value (self-hosting, the handle abstraction) stands on
   its own; the longer it diverges from `main`, the more the rebase costs.

**Cost split that drives this (measured):** cold store-codegen = `materialize_data`
**~5.5 ms** (the tax M6-warm avoids by using the mmap'd cache) + store reads
**~0.8 ms** (vs 0.6 ms native — handle navigation is near-native).  Warm `read_data`
on the **stdlib bundle = ~0.7 ms** — def-fields **65%** (attribute + return-type
`Type` trees) + variable tables **14%** (~0.39 µs/var) + bodies **20%**
(`bench_read_data_breakdown`).  So M6-warm (skip bodies only) trims ~20% of an
already-sub-ms read; the whole-`read_data` skip is the E2-gated prize and the cost
is **allocation-bound** — it scales with def + variable count and is exactly what
zero-copy eliminates (see recommendation #3).

### Debugging-iteration cost + the default-on dev-safety caveat (2026-06-04)

Evaluating the cache against the **debug-a-loft-feature loop** (edit compiler →
rebuild → run many tests to inspect behaviour) surfaced a hard prerequisite for
rec #1 — and reframed what the cache is even worth in that loop.

**1. The debug loop's floor is rustc, not loft startup.**  An edit to `src/*.rs`
costs tens of seconds of `cargo build --release` + a test-binary relink before a
single test runs.  loft's own startup (15 ms cold / 4–5 ms warm) is *noise* next
to that.  Optimising startup buys ~10 ms against a loop whose floor is the Rust
recompile.  The real levers for this loop are unchanged: **targeted suites**
(`issues` ~6 s, `expressions` ~1 s) over the ~7-min full run, `cached_default()`
(amortises the per-test stdlib parse), and `./scripts/find_problems.sh --bg` for
the wide net while editing.

**2. The two test-running modes have opposite cache relevance.**
   - **`cargo test` (the `code!`/`expr!` in-process path — the primary loop)
     never touches the program cache** — the cache lives in `main.rs` (the binary
     path), not the `State::execute` path.  So `LOFT_PROGRAM_CACHE` is irrelevant
     to `cargo test --test issues`: zero benefit, zero cost.
   - **Direct `loft script.loft` runs** (a hand-written reproducer run
     repeatedly) *do* use the cache — 3–3.6× startup — but that's the secondary
     loop, and it carries the hazard below.

**3. The dev-safety gap (the hard prerequisite for default-on).**
`cache::build_signature()` mixes in `LOFT_BUILD_ID`, which `build.rs` sets to
`git rev-parse --short HEAD` (re-run only when `.git/HEAD`/`refs`/`build.rs`
change).  **So an uncommitted compiler rebuild leaves the signature unchanged** —
the release-upgrade invalidation (`0b9e69a`) protects users moving between
*committed* builds, but does **not** fire during a debug session.  With a
default-on cache, a re-run cached script would then silently warm-load a
**parse/scopes-stale** bundle:

   | You edited (uncommitted) | Default-on cache, re-running a cached `loft script.loft` |
   |---|---|
   | `src/state/codegen.rs`, `src/fill.rs` (codegen/runtime) | **Safe** — codegen re-runs from `Data` each time |
   | `src/parser/*`, `src/typedef.rs`, `src/scopes.rs` | **Staled** — warm load skips parse+scopes → the change appears to do *nothing* |

That is the worst failure mode to hit while debugging: a correct parser/scopes
fix silently no-ops until you commit (or edit a `.loft` source, which the drift
manifest *does* hash).  `LOFT_STDLIB_CACHE` shares the blind spot with a smaller
blast radius (only the `default/` parse).

**Consequences (✅ both resolved by track 1, 2026-06-04):**
   - **While debugging the compiler, the caches are OFF automatically.**  The
     default-on flip disables the cache whenever `CARGO_MANIFEST_DIR` is in the
     environment — i.e. under `cargo run` and `cargo test` — so the compiler-debug
     loop never warm-loads a stale parse, with no flag to remember.  `LOFT_NO_CACHE`
     is the explicit kill switch for any other context.
   - **`build_signature()` now folds in the running binary's mtime**
     (`binary_signature_tag`), so *any* rebuild — committed or not — invalidates
     bundles, closing the git-HEAD-`BUILD_ID` blind spot.  This was the
     not-yet-done prerequisite beyond the release-upgrade invalidation
     (`0b9e69a`); it is now done.

---

### Arc N — native-library execution model (C71 build-out)

**The forward arc.**  Per [DESIGN_DECISIONS § C71](../../DESIGN_DECISIONS.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)
the steady state is **native libraries + interpreted scripts**; this arc builds
that architecture.  It **supersedes the G2 zero-copy endgame for perf** (E2 /
M6-cold / M7 are parked — see recommendation #2): native libraries are never
materialised, so the allocation-bound `read_data` cost is *avoided*, not chased
with a `VH` rewrite.  Scope = **the full interpreted+native architecture
EXCEPT the library validation layer** (deferred, customer-facing — see § Excluded).
Each phase lands as a **self-contained, default-safe increment in `main`** (the
dispatch primitive + caches stay opt-in / behind the automatic policy until N3+N5
make them safe-by-default).

**Builds on (already landed):** the `OpStaticCall` → `library_names` →
`extensions::wire_native_fns` → `try_dlsym` dispatch primitive (+ `native_packages`);
BUILD2's memoised `libloft.rlib` content hash (`native_utils::native_cache_key`);
the complete `Definition` read seam (M1a/M1b/M1c); the D2a database-type-schema
cache; and the cross-mode byte-identical equivalence harness.

| Phase | Deliverable | Validation (done-when) | Status | Effort |
|---|---|---|---|---|
| **N0 — build fingerprint** ✅ | **Done.**  `cache::loft_build_fingerprint()` = the loft rlib's sha256 **content** hash (memoised) — the single seam.  **Audit finding:** the user-binary cache key (`native_cache_key`) was *already* content-hash (BUILD2) and **no** native key folds git-HEAD — nothing to migrate.  The real gap was that `auto_build_native` / `add_native_extern_flags` reused a cached package rlib / cdylib on **existence only**, so a loft change linked the *stale* one (the `make rebuild-native-cdylibs` hazard).  Fix = the "do both" **per-artifact backstop**: a `.loft-build-fp` sidecar stamps each built artifact with the fingerprint; reuse is gated on a match, so a loft change rebuilds it.  (The per-artifact gate subsumes the coarse startup nuke — a rebuild overwrites in place, so no orphans accumulate.)  Goal-A veil-lifter as designed. | `cache::native_artifact_fingerprint_sidecar_gate` (unit) + `tests/n0_fingerprint.rs` (stale sidecar → rebuild + re-stamp) + a real `loft --native` run stamps the fp; native suite green | ✅ | S — **done** |
| **N1 — per-library native artifact cache** 🔄 | **Idle-TTL eviction model done + applied to the program cache.**  `cache::cache_ttl()` (`LOFT_CACHE_TTL_HOURS`, default 24 h) + `cache::touch_now()` (touch-on-use mtime bump) + `prune_dir` now evicts **idle-TTL first** (drop bundles unused past the TTL) with the size-cap as a runaway backstop; a warm program-cache hit `touch_now`s its bundle, so actively-run programs persist and one-offs age out.  (The "compile each library once, fingerprint-validated" half *is* N0's `auto_build_native` + sidecar.)  **Remaining — the idle-TTL on a *native-artifact* cache:** the default local build compiles libraries **in-tree** (`lib/<pkg>/native/target/`), which is the dev build, **not an evictable cache** — so the native-artifact idle-TTL needs the registry build-cache (`~/.loft/build-cache/`, feature-gated) or a new shared local native cache.  Deferred until that surface exists rather than shipping untested registry-gated GC (Goal-A "verify, don't assert"). | `prune_dir_idle_ttl_evicts_unused` (unit) + `arc_e_program_cache` warm-hit touch path; suites green | 🔄 | M — **model + program-cache done; native-cache application deferred** |
| **N2 — native dispatch + lean interface load** *(all common types PROVEN, both directions)* | The headline: an interpreted script calling a compiled user library over the shared store.  **Scalar slice + store-touching across ALL common types (scalars, vectors, structs, text, plain+data enums — both directions) landed** — 13 green end-to-end tests in `tests/n2_cdylib.rs`.  **Scalar:** `generate_cdylib_lib_rs` + a `loft_<name>(scalars) -> ret` wrapper that is **ABI-identical to a hand-written scalar `#native` symbol**, so it reuses the existing dispatch wholesale (`OpStaticCall` → `load_all` → `wire_native_fns` dlsym) — `double(21) → 42`.  **Store-touching:** an auto-generated cdylib links libloft, so the bridge **shares the caller's `*mut Stores` by pointer** (zero-marshalling, *not* the `LoftStore` handle): `generate_shared_cdylib_lib_rs` + `LibArg` uniform slot + `shared_store_dispatch`/`wire_shared_native_fns` — `vec_sum([10,20,30]) → 60` (non-scalar arg, raw `DbRef` crosses unchanged) and `range_vec(4) → [0,1,2,3]→6` (vector **return** — native allocates in the shared store via the hidden `ref_return` destination the bridge wrapper allocates itself; the `DbRef` is valid back in the interpreter).  Structs/text/enums + keyed `sorted` cross too (`point_sum`, `make_point`, `str_len`, `shout`, `dir_code`/`dir_from`, `area`/`make_rect`, `sum_values`); **schema agreement is proven** — an identically-defined struct gets the same type id + field offsets in the separate library and script `Data`.  Fixed a latent `loft_register_v1` guard bug (per-library `uses_v1`, preserving #119).  **Lean interface DONE (source form):** `generate_interface` emits the library's type defs + `#native` decls as loft source; a script using only it dispatches (`lean_interface_drives_shared_dispatch`) — no manual redefinition.  **Remaining → routed to [`NATIVE.md` § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71)** (closures `__closure`; `generate_interface` aggregate names `sorted<Item[k]>`; D2a binary schema interface; `hash`/`index`/`spacial` coverage — all enhancements on a graceful core).  Auto-deriving the dispatch from `use <lib>` is N3 (core proven). | a script using a native lib dispatches to its compiled subgraph, interprets the rest; output byte-identical to the all-interpreted run (Goal D) | 🔄 (all common types + lean interface done) | M |
| **N3 — native/interpret decision policy** *(core mechanism PROVEN)* | Make the native-vs-interpret choice **automatic and invisible** (Goal F): a stable/published dependency → native; a library under active edit → interpret (no `rustc` per save) — the **dev-interpret fallback**.  No user annotation, no flag.  **Core landed (in-process):** `native_lib::mark_native_exports(data, candidates)` sets `def.native = "loft_shared_<name>"` on a *normal* library function's shared-store-dispatchable subset — so `byte_code` routes its calls through `OpStaticCall`, the stub registers, and `wire_shared_native_fns` wires the bridge after the auto-built cdylib loads; `output_native_library` emits the cdylib with **no `main` bootstrap** even when the consuming script's `n_main` shares the `Data`.  `auto_native_marks_and_dispatches_normal_library_fn`: `double(21) → 42` with **no `#native` decl anywhere** — the in-process shape of `use <lib>`.  **Phase A DONE (2026-06-04) — the headline works on the real binary:** `[library] compile = "native"` opt-in (`manifest.rs`) → `Parser::pending_native_compile` (`apply_manifest_side_effects`) → `main.rs` marks (`mark_library_native`) + builds (`build_shared_cdylib` into `<pkg>/native-auto/`) + loads + `wire_shared_native_fns`; `tests/n3_use_native.rs` runs `use mathnative;` through the binary and gets `42/7/120` from an auto-built cdylib.  **Partial Phase B landed:** the silent per-function gate-split (a `CallRef`/`parallel` function interprets while the rest dispatches native), the synthetic-exclusion fix (dispatch targets = top-level user-named public fns), and cdylib caching (rebuild only when the source changes or the loft-build fingerprint moves).  **Critical path (re-derived via the rigor discipline) — COMPLETE:** ✅ **Step 1** parity instrument (the gate — native ≡ interpreted byte-for-byte; `tests/n3_parity.rs`, store-touching corpus) → ✅ **Step 2** decide native/interpret *before* `byte_code` so a build failure silently interprets (build-before-mark) → ✅ **Step 3** default-native (the opt-in is dropped; `use <lib>` is native, `LOFT_NO_NATIVE_LIBS` is the escape) → ✅ **Step 4** dev-interpret-on-edit (edit → interpret the new code with no `rustc`; settle → rebuild → native).  See § Landing sequence.  *Open: option-3 background build → routed to [`NATIVE.md` § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71); F1 nextest reliability (§ Discovered follow-ups; F2 interdependent-libs ✅ fixed).* | editing a library re-interprets it (fast loop); a stable dep links its cached artifact; the programmer never declares an execution mode | ✅ (Steps 1–4 done; the C71 model is live end-to-end) | M |
| **N4 — compilability gate + silent interpret fallback** *(re-scoped 2026-06-04; gate analysis 🔄 done)* | **Gate analysis landed — `src/native_gate.rs::native_compilable(data) -> HashSet<u32>`**: the maximal native subgraph, computed by a transitive, **exhaustive** (no `_` arm — Goal-F-safe: an un-native-able construct can never silently slip through) `Value`-tree walk.  **Empirical finding (the de-risk made real):** the `--native` backend already emits *everything* — structs, enums, vectors, **generics, closures** — so the denylist is just the concurrency constructs `parallel{}` / `par_for` / `yield` (`emit.rs` writes a non-code comment for those; `NATIVE_SKIP`/`SCRIPTS_NATIVE_SKIP` are both empty).  The "generics/closures research problem" was a **phantom** — measured **461/461 stdlib functions native-compilable (100%)**.  The gate is transitive (native iff the fn *and all `Call` callees* are native) so the subgraph is **closed** → the boundary is only ever interpret→native (`OpStaticCall`); `CallRef` is conservatively excluded (dynamic callee unprovable).  Tests: `walk_classifies_leaves_and_denylist`, `walk_finds_nested_denylist_construct`, `stdlib_is_mostly_native_compilable`.  **Remaining → routed to [`NATIVE.md` § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71)** (gate-driven dispatch tail — select the subgraph from `native_compilable`; making concurrency itself native is the only later optional item, and it is tiny). | gate: the native subgraph excludes exactly the concurrency users; library runs native where compilable, interprets where not, no user-visible error | 🔄 | gate **done** · dispatch (with N2) S–M |
| **N5 — mixed-boundary soundness + parity** *(woven through, per C71 guardrails)* | Extend the sanitizer (Miri / ASan / `stack_align_guard`, esp. macOS-ARM alignment) + the differential sweep to the **interp-script + native-lib** combination (A/D); extend `LOFT_STORE_GUARD` to the mixed path (E).  **Not a trailing phase** — a coverage leg lands *alongside* each of N1–N4 as its surface appears.  **Landed (2026-06-05) — D + E legs on `tests/n3_parity.rs`:** `assert_three_mode_parity` now (a) runs a broadened store-touching corpus (BOTH `Shape` enum variants → enum-tag discrimination crosses the boundary, both directions) interp≡mixed≡native, **positively controlled** by the reference-output anchor (rules out "parity holds but all three wrong"); (b) arms `LOFT_STORE_GUARD` on all three runs so the confinement detector runs over the script AND the library's codegen-time scope analysis.  **A leg is sanitizer-blind by construction** — ASan sees interpreter targets only, the `stack_align_guard` sweep can't see spawned binaries (ci.yml `guard` job), Miri can't `dlopen` a cdylib at all; so the *only* runtime soundness signal the mixed path has is the differential parity (a cross-boundary corruption diverges), and the one real A-leg extension — **ASan-on-cdylib** (nightly, propagate `-Zsanitizer` into `build_shared_cdylib`) — is **routed out to the sanitizer plan [@PLN54 § S9](../54-sanitizer-coverage-expansion/README.md) (2026-06-05)**, its canonical Goal-A home.  **E-leg is positively controlled:** arming `LOFT_STORE_GUARD` arms the live **Plan-57 Phase-4 guard** (`reclaim_unfreed_eligible == 0` assertion — the `[store-guard]` eprintln is superseded by it), which a fire trips into a panic caught by `r.success`.  Proven falsifiable by `watermark.rs::phase4_goal_e_guard_is_falsifiable` (a `LOFT_STORE_GUARD_INJECT` fault makes an 11× reassign program panic; silent without it) — so the corpus-silence is non-vacuous.  See § Discovered follow-ups F5. | the mixed run agrees byte-for-byte with all-interp **and** all-native (D); zero sanitizer fires across the mixed boundary (A); `LOFT_STORE_GUARD` silent on the mixed path (E) | ✅ in-plan (D + E landed; A-leg routed to @PLN54 § S9) | M (continuous) |

**Sequencing.**  N0 first (correctness + unblocks; small, dev-facing) → N1 + N2 are
the core mechanism (cache + dispatch) → N3 makes it invisible (F).  **N5 is woven
through each phase, not appended** — per C71's guardrail that the A/D detectors grow
to the mixed boundary *as part of* the work, and per [GOALS.md](../../GOALS.md)
"two floors," this arc is **gated on the soundness floor it extends** (it adds a new
interp↔native surface to that floor rather than clearing it).

### Landing sequence — the critical path to the C71 steady state

The **acceptance test for the ideal state**: a developer writes a *normal* loft
library, a script does `use mylib`, and — with **no annotations, no flags, no
`#native`, no execution-mode declaration** — the library runs native and the script
interprets, dispatch is invisible + zero-marshalling, output is **byte-identical to
the all-interpreted run**, it is sound, and *editing the library re-runs instantly*
(no `rustc` per save).

The whole dispatch mechanism is ✅ **proven and green today** (all common types, both
directions; the lean interface; `mark_native_exports` / `output_native_library` /
`build_shared_cdylib` / `wire_shared_native_fns`).  What remains is integration, one
policy decision, and the soundness sweep — **no research**.  Ordered, each a landable
working unit:

**Phase A — wire `use` → native dispatch (the headline, on the real binary)** ✅ **DONE (2026-06-04)** [N3 productization]
- **A1 ✅ Manifest opt-in** — `[library] compile = "native"` field + parse + tests (`manifest.rs`).  Explicit for now; B drops it.
- **A2 ✅ Parser hook** — `apply_manifest_side_effects` records the package dir in `Parser::pending_native_compile` when `compile == "native"` (hand-written `native` takes precedence).
- **A3 ✅ `main.rs` orchestration** — after `scopes::check`: `mark_library_native` marks each opted-in library's public shared-store-dispatchable functions native (file-prefix ownership ∩ the gate).  After `byte_code`: `build_shared_cdylib` builds each into `<pkg>/native-auto/`, loaded alongside the hand-written natives; then `wire_shared_native_fns`.  Auto-native programs bypass the program cache for now (warm load would lack the rebuilt cdylib — D1 persists this).  `find_loft_rlib` fixed to return the `<profile>/deps/` link-search dir in both the real-binary and test contexts.
- **A4 ✅ Fixture + subprocess test** — `tests/lib/mathnative/` (plain loft `double`/`add`/`factorial`, `compile = "native"`); `tests/n3_use_native.rs` runs the real binary on `use mathnative;` and asserts `42/7/120` **and** that the cdylib was built.  *The ideal-state core, proven end-to-end.*

**Already landed beyond Phase A (partial Phase B, 2026-06-04..05):**
- **Silent per-function gate-split** ✅ — `mark_native_exports` marks only the
  `shared_store_dispatchable` subset, so a `parallel{}`/`par_for`/`yield`/`CallRef`
  function stays interpreted while the rest of the *same* library dispatches native;
  proven end-to-end (`mixed_library_dispatches_native_and_interprets_rest`).
- **Synthetic-exclusion fix** ✅ — *Invariant: a dispatch target is a top-level,
  user-named, `pub` function the script can directly `Call`.*  A `pub fn`'s parse
  sprays `pub_visible` over its nested lambda (`__lambda_N`), so `mark_library_native`
  excludes `__`-synthetic names (the whole class, not the lambda instance).
- **Cdylib caching (the rebuild half)** ✅ — `cached_or_build_shared_cdylib` reuses a
  fresh cached `native-auto/<so>` (source-mtime unchanged **and** N0 build-fingerprint
  matches), rebuilds otherwise.

**The converged critical path (re-derived via the engineering-rigor discipline,
2026-06-05).**  The naive order was "wire → make invisible → make sound."  Probing
it (Design Protocol 1) inverted two things: **invisibility is only safe once parity
is *proven*** (so the soundness sweep is the *gate*, not the trailer), and
**default-native is an architecture change, not a flag** (a build failure must
degrade to interpret, which the current build-after-`byte_code` ordering can't do).
Each step is a *design* step — stated as its **invariant** + the **probe that
falsifies it**:

**Step 1 — Parity instrument** ✅ *(core leg landed 2026-06-04; the gate; subsumes the old Phase C / N5-D)*
- *Invariant:* a function run native ≡ run interpreted, byte-for-byte.
- *Falsifier:* a differential harness running each corpus program **all-interp ·
  all-`--native` · mixed (`use` auto-native lib)** and diffing stdout; the first
  divergence is a real bug caught before it ships invisibly.  Seed with
  `mathnative`/`mathmixed`.
- *Why first:* "invisible" is honest only once equivalence is proven across the
  **class**, not spot-checked per test.  **N5 sanitizers** (Miri/ASan/
  `stack_align_guard`, esp. macOS-ARM) + `LOFT_STORE_GUARD` on the mixed path land
  alongside.
- **Landed:** `tests/n3_parity.rs` — `assert_three_mode_parity(prog)` runs a program
  three ways (interp via `LOFT_NO_NATIVE_LIBS=1` · mixed default · `--native`) and
  asserts byte-identical stdout.  `datalib_store_touching_types_parity` drives the
  `datalib` fixture, which crosses **every store-touching type** (vector/struct/text/
  enum) over the boundary in **both** directions (native factory return *and*
  interpreter-constructed `pub` type passed in).  **Remaining (off the immediate
  path):** the N5 sanitizer leg + broadening the corpus.

**Step 2 — Decide native/interpret *before* `byte_code`** ✅ *(landed 2026-06-05; B1's prerequisite)*
- *Invariant:* a library that can't compile native (build failure, no `rustc`)
  **silently interprets** — byte-identical, no `exit`, no user-facing error.
- *Falsifier:* a fixture with a deliberate codegen-gap function → the program must
  still run (interpreted), not crash.  ~~Today it `exit(1)`s, because the build is
  *after* `byte_code` already emitted `OpStaticCall`.~~ — fixed.
- *Design choice, probed then taken:* **build the cdylib from the type schema before
  `byte_code`** (the re-`byte_code` fallback was rejected).  A probe confirmed a
  cdylib builds + dispatches from the post-parse schema (`p.database`, pre-`byte_code`);
  `native_lib` now splits `library_export_set` (compute) from `mark_exports` (mark),
  and `main.rs` builds first, marking **only on success** — a build failure warns to
  stderr and leaves the library unmarked, so `byte_code` emits ordinary calls and it
  interprets.  `LOFT_FORCE_NATIVE_BUILD_FAIL=1` is the falsifier hook; locked in by
  `tests/n3_parity.rs::build_failure_silently_interprets` (forced failure → exit 0,
  byte-identical to the interpreted reference, no cdylib built).

**Step 3 — Default-native (B1)** ✅ *(landed 2026-06-05; the "invisible" the goal names)*
- *Invariant:* a `use`d library defaults to native; **no opt-in, no flag, no
  `#native`** (the Phase-A `compile = "native"` manifest field is now a redundant
  no-op).
- *Falsifier:* a library with a non-compilable function still works (gate + Step 2);
  a fresh `use` of any normal library dispatches native with no annotation.
- **Landed:** `apply_manifest_side_effects` records **every** `use`d non-`#native`
  library into `pending_native_compile`; the driver builds + marks + dispatches it
  (build-before-mark).  `LOFT_NO_NATIVE_LIBS=1` is the interpret escape.  Proven on a
  real consumer first (instrument): `lib/markdown` — 900 lines, structs/vectors/
  multi-arg text fns — auto-builds a cdylib + dispatches byte-identically (~1.4s cold,
  ~0.27s warm).  `tests/n3_parity.rs::default_native_dispatches_unopted_library`
  locks it in on the un-opted-in `plainlib` fixture (default → native + cdylib; escape
  → interpret + no cdylib).
- **Landing choice — gate dev/CI to interpret** *(user decision 2026-06-05)*: flip
  now, accept that the bare edit loop pays cold builds until Step 4.  The flip is
  correctness-clean across the whole suite (Step 2's silent fallback); the only
  general subprocess test that auto-builds is the crystal GL gold test (gated to
  `LOFT_NO_NATIVE_LIBS=1` — it validates pixels, not dispatch).  Everything else runs
  in-process (no build loop), tests a library *as a program* (no `use` edge), or is a
  dedicated native/parity test.

**Step 4 — Dev-interpret-on-edit (B2 policy)** ✅ *(landed 2026-06-05 — the "no `rustc` per save")*
- *Invariant:* a library being edited interprets (instant loop); a stable/cached
  library links its artifact.  Native *when stable*, interpreted *when fresh* — this
  reconciles "library = always native" with "no rebuild while you iterate."
- *Falsifier:* an edit-run-edit-run loop fires no `rustc`; stop editing → the next
  run is native.
- **Landed (option 2, refined — hash-stability with an eager first build).**
  `cached_or_build_shared_cdylib` → `Result<Option<PathBuf>, String>` (`Ok(Some)`
  native · `Ok(None)` interpret-this-run · `Err` build-failed) decides per run:
  (1) fresh artifact → native; (2) **no artifact yet / `loft` changed → build eagerly**
  → native (first use + deployed deps run native from the start; also keeps the
  parity tests' single-run-dispatches-native true); (3) stale artifact (edited) →
  compare `source_content_hash` (sha256 over sorted `.loft`/`loft.toml`) to the
  `.loft-run-hash` sidecar — *changed since last run* → interpret (`Ok(None)`, no
  `rustc`), *unchanged* → rebuild → native.  `main.rs` also sets `any_dev_interpret`,
  which gates `save_program` so a warm program-cache load can't pin the interpreted
  image and skip the rebuild-when-stable check.  Guard:
  `tests/n3_parity.rs::editing_a_library_interprets_then_rebuilds_when_stable`
  (edit → cdylib mtime UNCHANGED + new code interpreted; settle → rebuild).
- **Deferred to Polish (option 3 — background build).** The one wart of option 2: the
  first run *after* you stop editing pays a foreground `rustc` (the "settling" run
  builds, then native).  Option 3 — interpret-on-stale + a detached build (atomic
  temp-then-rename + per-package lock), so even the settling run never blocks — is the
  truly-instant-loop upgrade; `~/.loft/build-cache/` (Polish, N1) is its natural home.

**Polish — off the critical path** ⬜ *(fast · robust · complete)*
- **Routed to [`NATIVE.md` § N9](../../NATIVE.md#n9--native-library-shared-store-dispatch-c71)** (2026-06-05) — background build (Step 4 option 3),
  **D2a binary schema interface** (type ids agree without redefinition, bodies never
  re-parsed; replaces the source-form lean interface ✅), and the coverage tails
  (closures `__closure`; `generate_interface` aggregate names `sorted<Item[k]>`).
- **Stays here (deferred-with-trigger):** idle-TTL artifact eviction on a shared
  `~/.loft/build-cache/` (N1, model ✅, fingerprint-gate ✅ N0 — trigger: the
  registry build-cache surface); the daily-builds **validation layer** (deferred,
  see § Excluded).

### Discovered follow-ups (surfaced landing Step 3, 2026-06-05)

Neither blocks Step 4; both are real and worth a focused pass.  Recorded here so the
scope isn't re-derived later.

**F1 — nextest native-test reliability (test-infra, CI-masked).**  The flip is green
under `cargo test` (the `make test` gate: n2_cdylib 16, native 6, n3_parity 3,
n3_use_native 2, issues 684) and in CI (the `ci` nextest profile has `retries = 1`).
It is **flaky under raw `cargo nextest` full-suite runs** (`find_problems.sh`, the
`default` profile, no retries).  The failures **move between runs** and every failing
test **passes in isolation** — so it is a harness interaction, not a flip-logic bug
(`n2_cdylib`/`native` predate this arc; their build paths are unchanged).

**✅ Mode A RESOLVED (2026-06-05) — root cause found by reproduction, and it was
NOT a deps-read race.**  The matrix-first instrument falsified the long-assumed
"concurrent links compete for `deps/*.rlib` reads" story: **160 concurrent
`loft --native` builds reading the shared `release/deps/` dependency closure
produced zero link errors** — because `loft --native` writes **PID-suffixed**
scratch paths.  The real cause is in `tests/native.rs`: the native test harness
wrote **stem-keyed** scratch output paths (`loft_native_<stem>_bin` +
`.rs`/`.key`/`_args.txt`), and three tests compile the *same* stem —
`native_tuple_script`, `native_tuple_return_script`, and `native_scripts` all
build `50-tuples.loft`.  The in-process `native_suite_lock()` serialises them
within ONE process, but **nextest runs each test in its own process**, so under
nextest they ran concurrently and their rustc/lld processes wrote the **same**
output binary, truncating each other's file in place mid-link → **SIGBUS in
`rust-lld` / `linking with cc failed`** (the same shared-fixed-`/tmp`-path shape
the `html_wasm` race already documents in `nextest.toml`).  Reproduced
deterministically (wipe the `50-tuples` cache to force concurrent recompiles →
1/6 fails); the `linking` symptom is Mode A's "momentarily unusable artifact"
family — **not** a deps race, and **not** the loft-binary-fires-`cargo-build`
mechanism the reverted commit guessed.  *Fix (`tests/native.rs`):* compile each
job to a **per-process temp output** and **publish atomically via `rename(2)`** —
`rename` swaps the directory entry, so a process executing/linking the old binary
keeps its inode (no in-place truncation → no SIGBUS), and the BUILD2 cache path is
always a complete artifact.  **No nextest serial group needed** (full parallelism
kept); the previously-reverted `native-build-serial` idea is **moot** — it would
only have masked the path collision.  *Verified:* forced-collision **0/12** (was
1/6); broad cold `native + n2_cdylib + n3_parity + n3_use_native` set **0/4**, all
30 pass (was ~1/3); cache intact (251 warm hits); `cargo test --test native` 6/6;
fmt + clippy clean.
- *Mode B — build-layout mismatch (`ring/rustls/webpki … not in rlib format`).*
  **Not reproduced in this environment** and untouched by the Mode-A fix: here
  `crate-type=["cdylib","rlib"]` yields a single consistent unhashed
  `release/deps/libloft.rlib` whose transitive dep `.rlib`s all resolve, so the
  `.rmeta`-only-SVH layout that triggers it never arose.  Remains **open** and
  pre-existing — the original *fix direction* stands (resolve the cdylib build
  against the rlib set nextest actually produces, or build a consistent set
  first).  Lower urgency than believed: Mode A was the symptom `find_problems.sh`
  actually hit.

**F2 — interdependent libraries are fully native.** ✅ *(fixed 2026-06-05)*  When a
library was *both* a transitive dep and directly `use`d (consumer uses `top`, `top`
uses `base`, consumer also uses `base`), only `top`'s cdylib was built — `base`'s
direct calls interpreted.  *Root cause:* the **direct-resolution / sibling-package**
path (`register_native_manifest`) never got Step 3's default-native record — that
landed only in `apply_manifest_side_effects` (the `lib_path_manifest` path); a
transitively-loaded library (or one directly `use`d *after* being loaded
transitively, so the direct `use` dedups) hit the diverged path and was never
recorded.  *Fix:* `register_native_manifest` now mirrors the record (a normal loft
library `m.native.is_none()` → `pending_native_compile`).  Both libraries build their
own cdylib and dispatch native; output stays byte-identical.  Verified the main
program's own package is NOT over-recorded (`library_suite` builds no `native-auto/`).
Guard: `tests/n3_parity.rs::interdependent_libraries_are_fully_native` (the diamond).

**F3 — ✅ the C71 speedup is REAL (63.6×): a measured value + a false alarm that taught the method.** *(2026-06-05 — gate ②, the first time anyone measured the C71 *execution* path)*

**The value, measured right:** an interpreted script calling an auto-native library is
**63.6× faster** than interpreting the library — `benchlib::compute` (200M-iter loop),
`--interpret`, **runtime** arg, warm cdylib, min-of-5: interp-lib **25049 ms** vs
auto-native-lib **394 ms**, with the bridge dispatch **confirmed live by a sentinel**
(`SHARED_DISPATCH_HITS`).  Two real fixes fell out:
- **opt-level (landed):** `build_shared_cdylib` was `-C opt-level=0`; the bridge calls
  `ops::op_*` per arithmetic op, un-inlined when unoptimized.  opt-0 = 4706 ms (5.3×),
  opt-2 = 394 ms (63.6×) — an **11.9× difference**.  Now opt-2 (builds run only when a
  library is stable, Step 4, so the slower build is the right trade).
- **liveness guard (landed):** output-parity asserts the RESULT (correct whether the
  call dispatches or interprets its body), so it can't catch a silent revert to
  interpret-the-body.  `tests/n2_cdylib.rs::f3_body_bearing_marked_fn_dispatch_vs_interpret`
  asserts the bridge sentinel *moved*, with a no-body `#native` call as the positive
  control.  This is the speed/liveness assertion the arc was missing.

**The false alarm — and why it's worth recording (the method, not the bug).**  An
earlier draft of this entry screamed "0× speedup / dispatch is dead / `static_call` is
dead code."  All of it was **measurement artifact** from three compounding broken
instruments — the canonical engineering-rigor failure (build the instrument *and verify
it* before trusting it):
1. **Wrong mode.** `loft <file>` defaults to whole-program `--native` (main.rs:101,
   "native is default").  So both "interp" and "native" runs compiled the *same* whole
   program → trivially 1.0×, and the interpreter (where the cdylib dispatch lives) never
   ran.  The C71 path is `--interpret`; the benchmark never used it.
2. **Const-fold.** The first benchmark passed a constant arg → interpreted at compile
   time (a real user-facing surprise too: a constant-arg library call is const-folded).
3. **Broken sensors.** CLI `eprintln`/counter probes read 0 because the interpreter
   wasn't running at all — and the silence was read as "dead dispatch" instead of "wrong
   sensor."  The fix was a **positive control**: an in-process harness (which *uses* the
   interpreter) fired the bridge, and a Drop-guard sentinel showed `exec[argv=0]` for the
   default CLI mode → "native is default" → the whole benchmark was the wrong mode.

*Lesson folded back into the rigor skill:* the usage sentinel generalized beyond
removal, plus the **silence-is-evidence-only-after-a-positive-control** clause.

**F4 — the default-on program cache (track 1) broke `#cwd` (relative-path) programs.** ✅ *(fixed 2026-06-05; commit `65ae5f0`)*  Surfaced via `make index`: scan.loft (a `#cwd` cwd-relative repo-root scanner) produced a **partial scan on a warm cache hit** — 543 tags cold, **17 warm**, degrading toward 0 as the bad cached program kept being re-served.  *Root cause (matrix-first; isolated cleanly by cache-on = 17 vs `LOFT_NO_CACHE` = 543 vs warm + `LOFT_PATHS=cwd` = 543):* the `#cwd` directive sets `program_relative = false` at **parse time** (`parser/mod.rs`), but the whole-program cache skips parsing on a warm load and the manifest didn't persist the flag, so it reverted to the program-relative default → relative paths resolved against the script's own dir instead of cwd.  *Fix:* persist the mode in the cache manifest as a `prel <0|1>` header line (`save_program`) and restore `p.database.program_relative` on warm load (`warm_load_program` — `manifest_matches` → `manifest_program_relative`); `source_dir` needs no restore (`main.rs` sets it every run from the script path).  Verified on both backends (scan.loft `--native-release` warm 17 → 543; plain `make index` restored).  Guard: `tests/arc_e_program_cache.rs::cwd_directive_survives_warm_load` (`--interpret`).  *Class note:* `program_relative` is the only parse-time path-config flag the warm load lost; any future file-level directive that mutates runtime state would need the same manifest round-trip.

---

**Critical path to the *ideal* state: Step 1 → 2 → 3 → 4.**  Step 1 (parity) is the
unlock — simultaneously the soundness floor *and* the instrument that turns the
default-native flip (Step 3) from hopeful into proven.  Polish is not required for
"invisible + sound."  *This sequence has been probed twice without a new correction —
it has converged.*

**Timeline (the interpret-fallback re-scope, 2026-06-04).**  Letting the
un-native-able cases interpret (N4 above) drops the generics/closures **research off
the critical path**, so the shippable mixed-mode is **N1-remainder + N2 (with the
compilability gate) + N3 + N5 ≈ 2–2.5 months**, with the **headline at ~5–6 weeks**
(an interpreted script calling a compiled user library over the shared store, for
the native-compilable subgraph).  Native coverage of the hard cases is a *later,
measured* optimization.  Effort is **front-loaded for value**: each of N1/N2/N3 and
each new type pattern is independently shippable to `main`.  Skip an N4 "can we make
generics native?" spike — the fallback makes that a non-blocker.  The two
**non-negotiable, not-front-loaded** costs are N5 (mixed-boundary soundness, esp.
macOS-ARM alignment) and the conservative-silent compilability detector in N2.
*Caveat: a fallback function gets no native speedup — the win is bounded by how much
of a library's hot path sits in the native subgraph, which is measurable per
library, not guessed.*

**Update (gate landed, 2026-06-04) — the de-risk is now *measured*, not estimated.**
The compilability gate (`native_gate::native_compilable`, N4 row) shows the
`--native` backend already compiles everything except the concurrency constructs:
**461/461 stdlib functions are native-compilable (100%)**.  So the "fallback" is
needed for almost nothing — the conservative-silent detector (flagged as a core
risk above) turned out to be a small exhaustive `Value`-walk, and the fallback path
fires only for `parallel{}`/`par_for`/`yield` users.  This collapses N4 to "wire the
dispatch" and removes the last real uncertainty from the headline estimate.

**Update (scalar dispatch landed, 2026-06-04) — the headline is half-proven.**  The
N2 scalar slice is **end-to-end green**: an interpreted script calling an
auto-generated, auto-compiled cdylib (`tests/n2_cdylib.rs`, `double(21) → 42`).  The
decisive finding is that the auto-generated export wrapper is **ABI-identical to a
hand-written scalar `#native` symbol**, so the entire dispatch path (`OpStaticCall`
→ dlsym → auto-marshal) is **reused with zero new runtime code** — the cdylib
*generation* (`native_lib::generate_cdylib_lib_rs`) was the only new piece.  The
remaining headline work is the **store-touching slice** (the `&UnsafeCell<Stores>`
↔ `LoftStore` FFI-handle bridge) + the `use`-driven decl auto-derivation (N3).

**F5 — the Goal-E store guard is now falsifiable (the `[store-guard]` eprintln was a red herring; the live guard is the Phase-4 assertion).** ✅ *(resolved 2026-06-05)*  Implementing N5's E leg surfaced that `[store-guard]` (the `store_lifetime_guard` eprintln) **cannot fire from loft source** — tried branch-reassign, loop-rebind, nested-escape, the plan-57 probe corpus, plain and under `LOFT_CONF_RECOVER`, all silent — because post-plan-57 the cluster-I fix relocates its single-store entries and excludes multi-store cases, so its reported set is empty by construction.  *The resolution:* that eprintln is **superseded** (scopes.rs:312) by the **Plan-57 Phase-4 guard** — `scopes::check`'s `reclaim_unfreed_eligible == 0` `assert_eq!`, armed by `cfg(debug_assertions) || LOFT_STORE_GUARD`.  *That* is the live Goal-E enforcement (a panic, not a print), and it **is** falsifiable: a `LOFT_STORE_GUARD_INJECT` test-only fault skips the early-free while keeping the guard armed, so an 11× reassign program panics `plan-57 Phase 4: n_main left 9 reclaim-eligible store(s) live-but-dead past a later alloc`, and is silent without the fault (reclaim frees them).  *Positive + negative control:* `tests/watermark.rs::phase4_goal_e_guard_is_falsifiable` (subprocess so the env fault can't leak).  *Consequence for N5:* the E-leg arming `LOFT_STORE_GUARD` arms this live guard over the script AND the library's codegen-time scope analysis; a fire is caught by the harness's `r.success` check.  So Goal E's "silent corpus-wide" check is **non-vacuous** — the detector demonstrably fires on a reclaim regression.

**F6 — a warm cache load drops cross-source synthetic-wrapper `def_names` bindings → `OpDatabase(db_tp=u16::MAX)` → `claim(size=0)` startup panic.** ✅ *(fixed 2026-06-08; PR [#291](https://github.com/loft-lang/loft/pull/291))*  Surfaced by the **crawler** `build_walls` consumer as a "flaky" startup panic (`store.rs:473 "Incomplete record"`), filed as a runtime store-pressure desync ([#290](https://github.com/loft-lang/loft/issues/290), C18/C19).  *The "store-pressure" framing was a red herring* — the matrix-first isolation was **cache-on = panics, `LOFT_NO_CACHE=1` = clean**, and run-1 (cold, writes the bundle) is fine while every run-2+ (warm) dies, so it is **deterministic on a warm load**, not load-dependent.  *Root cause:* the synthetic global wrappers (`Data::vector_def` → `main_vector<T>`, `tuple_def` → `__tuple<…>`, `fn_ref_def` → `__fn_ref`) register a **global** `def_names[(name, 0)]` binding so any source can resolve them, but `add_def` leaves the def's own `source` set to whichever file first requested it (e.g. `main_vector<WallSeg>` first created while compiling `wallgeo`, source 6).  `rebuild_indices` (the warm-load index rebuild) re-keys `def_names` on **each def's own `source`**, so it only re-creates `("main_vector<WallSeg>", 6)` and never the global `(name, 0)`.  Codegen for a `vector<WallSeg>` local in `main` (source 0) then does `name_type("main_vector<WallSeg>", 0)` → `u16::MAX` → `OpDatabase(db_tp=u16::MAX)` → `enum_parent_size(u16::MAX) = 0` → `claim(size=0)`.  A cold compile works because the cross-source `(name, 0)` binding is live in memory; only the cache round-trip loses it.  *Fix (`src/data.rs`):* stamp those three synthetic wrappers `source = 0` so `rebuild_indices` reproduces the global binding.  *Guard:* `data::caller_graph_tests::synthetic_vector_wrapper_is_global_source_zero` (fails without the stamp).  *Class note:* `rebuild_indices` reproduces **only** `(def.name, def.source)` per def; ANY `def_names` entry keyed at a source other than the def's owner — the whole "a derived index isn't faithfully rebuilt from the serialized definitions" class — is dropped on a warm load.  The general detector + the remaining members are **F7**.

**F7 — derived-index round-trip completeness: the audit oracle + two more (benign) members of the F6 class.** 🔄 *(audit + benign verification done on branch `cache_verify`; the general fix is blocked)*  *The blind spot that let F6 ship:* `compare_data` (the warm round-trip oracle, `ir_schema.rs`) checks only the **serialized** `definitions` + the `Data` header — **not** the DERIVED indices (`def_names` / `operators` / `possible` / `use_names`) that `rebuild_indices` reconstructs on load.  So a dropped derived binding (F6 lived in `def_names`) is invisible to it, and `read_whole_stdlib_compare_data_green` stayed green through the bug.  *New oracle:* `Data::derived_indices_diff` (`src/data.rs`) compares the rebuilt indices, plus two tests in `src/ir_read.rs`: `stdlib_round_trip_preserves_derived_indices` (PASSES — the single-source stdlib's indices round-trip) and `multi_source_round_trip_preserves_derived_indices` (`#[ignore]`'d executable backlog spec — un-ignore when the fix lands).  Run on a multi-source program (importing `main` + a `use`d lib) the audit surfaced two more members of the F6 class, **both verified benign on the current warm path**:
- **`use lib::*` import bindings** — `def_names[(imported_name, importing_source)]` (e.g. `("Point", 0)` for a type defined in `importlib`, source 1) is dropped; only `(name, def.source)` survives.  *Benign because* codegen for struct construction / calls uses the **baked `def_nr`** from the serialized IR, not a `def_names` by-name lookup.  Verified: an importing program (`use importlib::*`, constructs `Point`, calls `add`) run through the cache gives the right answer on the cold save AND every warm load — no panic.
- **the `use_names` module map** (`{"importlib": 1}`) is not serialized, so a warm load keeps only `{"std": 0}`.  *Benign because* `Data::get_source` (the only `use_names` reader) is used **only in the parser** (import resolution, `parser/mod.rs:4227`), and a warm load skips parsing.

  So the only **load-bearing** warm-path by-name lookup was the F6 `main_vector<T>` wrapper (now fixed); these two are **latent** — they would bite only a *future* codegen-time by-name lookup (`name_type` / `def_nr`) of an imported name keyed at the importing source.  *The general fix (closes the whole class):* serialize the cross-source bindings + `use_names` into the bundle and have `rebuild_indices` reproduce them.  *Blocked:* this needs new `Data` bundle fields, but the schema regen pipeline (`loft --native --show-rust tools/ir_schema/ir.loft` → `tools/ir_schema/extract.py` → `src/ir_schema_gen.rs`) is **broken here** — `--show-rust` fails with a separate native-codegen defect (`E0425: cannot find value t115`, the `Block` struct id out of scope in the generated `register_ir_schema`), emitting zero output.  Deferred until that regen defect is fixed (then add the two `Data` fields, regenerate, materialize+read them, reproduce in `rebuild_indices`, and un-ignore the spec).  *Class note:* the audit's `derived_indices_diff` now makes this entire class **visible** — a warm load that silently drops a derived binding is no longer invisible to a round-trip test, the gap that made F6 unfindable.

### N2 dispatch — implementation design (2026-06-04, post-investigation)

**The dispatch *decision* already exists.**  The interpreter emits `OpStaticCall`
for any function whose symbol is in `State.library_names`, resolved at runtime via
`extensions::wire_native_fns` → dlsym.  So N2 does not build a new dispatch path —
it registers the auto-compiled subgraph's symbols and reuses the stdlib's.

**Strategy: auto-generate a native *package* crate from the native subgraph.**  A
native package (`lib/<pkg>/native/`) is already a cdylib of `#native` exports that
`auto_build_native` (N0) builds and the interpreter dispatches to.  N2 = generate
that crate's `lib.rs` from the library's native-compilable functions (`--native`
`output_function` for the bodies + the `#native` export wrappers), then reuse N0's
build + the existing load/dispatch.

**THE CRUX — REFRAMED (2026-06-04, post scalar-slice).**  The original framing —
"bridge `&UnsafeCell<Stores>` ↔ the `LoftStore` FFI handle" — was **the wrong bridge
for an auto-generated cdylib.**  `output_function` emits `fn n_<name>(_cell:
&UnsafeCell<Stores>, args…) -> ret`, touching the heap through a Rust
`&UnsafeCell<Stores>`.  The investigation of the dispatch internals
(`extensions::make_loft_store`) showed the `LoftStore` handle exposes only **one
store's raw buffer + alloc callbacks** — it exists for **hand-written** cdylibs that
**don't link `loft::database::Stores`**.  But an **auto-generated** cdylib **does
link libloft** (`--extern loft=libloft.rlib`; its body already calls
`loft::database::Stores` / `loft::keys::DbRef`), so `Stores`/`DbRef`/`Store` are the
**same Rust types with the same layout on both sides**.  ⇒ The store-touching wrapper
**shares the real `Stores` by pointer** — `loft_n_<name>(stores: *mut Stores, args…)
-> ret` casts `*mut Stores` → `&UnsafeCell<Stores>` (`UnsafeCell` is
`repr(transparent)`) and forwards to the inner fn, **no per-call cell, no `init`, no
marshalling** (the caller's `Stores` is live + initialised; `DbRef` args are already
valid in the shared store).  **This is C71's zero-marshalling shared-store ABI
literally** — and it's *simpler* than the FFI handle, not a multi-day bridge.

**Schema agreement (the one real soundness dependency) is already proven.**  A
store-touching `--native` body reads struct fields at offsets computed by
`db.finish()`; the shared `Stores` must lay those fields out identically to what the
interpreter built.  The **cross-mode byte-identical equivalence tests** (interp vs
`--native` across the whole corpus) already require exactly this — identical type IDs
+ field offsets — so the shared-pointer read is sound for every type the corpus
covers.  The scalar slice didn't exercise this; the store-touching slice rests on it.

**Remaining ABI plumbing (the now-bounded store-touching work):** the scalar slice
reused the existing `#native` dispatch (a *per-call* `Stores` cell, fine because no
ref crosses).  Store-touching needs a **shared-pointer dispatch arm** that passes
`stores as *mut Stores` + the raw stack args (scalars by value, `DbRef` by its 12
bytes), distinct from the `LoftValue`/`LoftStore` marshalling.  Start with
`vector<integer>` (schema-*independent* generic inline layout) to isolate the ABI
from the type-schema concern, then struct/`Text`/`Reference` signatures.

**Sequencing (scalar-first — the store bridge is deferred, not skipped):**
1. **Scalar-only functions** — ✅ **DONE (proven end-to-end, 2026-06-04).**  The
   export wrapper is trivial: `loft_<name>(scalars) -> ret` stands up a per-call
   `UnsafeCell<Stores>` + `init()` and forwards to the `--native` inner fn.  Safe
   because a scalar-return function cannot leak a store ref out, so any internal
   store use is contained and dropped with the cell — **no store-free body walk was
   needed** (the original plan's gate refinement turned out unnecessary; the
   per-call cell handles a store-touching-but-scalar-signature body too).
   `native_lib::generate_cdylib_lib_rs` is the crate generator (`scalar_dispatchable`
   from `native_gate` selects the export set; `output_native_reachable` emits only
   the export set + its transitive deps, so no unreachable operator stubs surface).
   The end-to-end proof is `tests/n2_cdylib.rs` — `double(21) → 42` over a real
   auto-built cdylib, dispatched from an interpreted script.  **The decisive
   realisation:** the wrapper ABI **equals** a hand-written scalar `#native` symbol,
   so registration + dispatch is the *existing* path (`def.native` → `OpStaticCall`
   → `load_all`/`wire_native_fns` → dlsym auto-marshal) — zero new runtime code.
2. **Store-touching functions** — ✅ **non-scalar args + vector returns DONE
   (2026-06-04).**  Reframed (see THE CRUX above): an auto-generated cdylib links
   libloft, so the bridge **shares the caller's real `*mut Stores` by pointer**
   (zero-marshalling) — *not* the `LoftStore` FFI handle.  Built:
   - `native_lib::LibArg` — a `#[repr(C)]` uniform arg/return slot (`{ scalar:
     i64, dbref: DbRef }`), linked from libloft by both sides so they agree on
     layout with no marshalling.
   - `generate_shared_cdylib_lib_rs` + `shared_bridge_wrapper` — emit
     `loft_shared_<name>(stores: *mut Stores, args: *const LibArg, n, ret)` that
     casts `*mut Stores` → `&UnsafeCell<Stores>` (no per-call cell — caller's store
     is live), reads visible params from `LibArg` slots, **allocates each hidden
     `ref_return` destination** (`Attribute::hidden`) itself via `null_named` +
     `OpDatabase(<type_id>)`, forwards, writes the return.
   - `native_gate::shared_store_dispatchable` — the gate (params: any bridge type;
     returns: scalar / void / **vector**).
   - `extensions::shared_store_dispatch` + `wire_shared_native_fns` — pack stack
     args into `LibArg` (the **raw** `DbRef`, no deref — `--native` consumes the
     indirect-header form), pass `*mut Stores` directly, call the bridge, write the
     return.  `wire_native_fns` skips `loft_shared_*` (disjoint ABI).
   - **Ground-truth finding:** `--native` passes a vector arg as the indirect-header
     `DbRef` (interpreter agrees → no translation), and returns a vector via a
     hidden destination param the caller pre-allocates.
   - End-to-end proofs (`tests/n2_cdylib.rs`): `vec_sum([10,20,30]) → 60` (non-scalar
     arg), `range_vec(4) → [0,1,2,3]` summed to `6` (vector **return** — native
     allocates in the shared store, the `DbRef` is valid back in the interpreter).
   - **Drive-by product fix:** the `loft_register_v1` guard keyed on a global
     "registry non-empty" proxy → false-positived a *second* zero-registration
     cdylib; now keyed per-library (`LOADED_LIBS` tracks `uses_v1`), preserving
     issue #119.
   - **Schema-agreement PROVEN for structs (the de-risk made real):** a struct
     `reference` **arg** crosses correctly — `point_sum(Point{x:3,y:4}) → 7`
     (`dispatches_struct_arg_into_shared_cdylib`).  The library cdylib and the
     interpreter, built from **separate `Data`**, assign an *identically-defined*
     `Point` the **same type id + field offsets** (identical stdlib prefix +
     identical struct def → same definition index → same `db.finish()` layout), so
     the shared `DbRef` reads `p.x`/`p.y` correctly.  This validates the
     zero-marshalling shared-store ABI for schema-dependent types; the *lean
     interface* would formalise "the script adopts the library's exact schema"
     rather than relying on identical redefinition.
   - **Struct returns also DONE** — `make_point(3,4) → Point` read as `34`
     (`dispatches_struct_return_from_shared_cdylib`).  Unlike a vector return, a
     struct `reference` return uses **no** hidden destination (`--native` emits
     `n_make_point(cell, a, b) -> DbRef` — the body allocates the record fresh and
     returns its `DbRef`), so the gate just admits `reference` returns; the native
     allocation is valid back in the interpreter via the shared store.
   - **Text DONE (both directions)** — `str_len("hello") → 5` (arg: `--native`
     takes `&str` = ptr+len, *not* a `DbRef`; the bridge borrows the store-backed
     bytes) and `shout("hi") → "hi!"` (return: `--native` uses `text_return`'s
     `&mut String` work buffer — a hidden param the bridge owns as a local
     `String`, then copies the result into the shared store's `scratch` so it
     survives, mirroring the legacy `bridge_push_str`).  `LibArg` gained
     `text_ptr`/`text_len`.
   - **Enums DONE (both directions)** — a plain (tag-only) enum is a `u8` tag in
     the scalar slot (`dir_code(South) → 2`, `dir_from(1) → East`); a data enum is
     a `DbRef` like a struct, allocated fresh on return (`area(Circle{r:2}) → 12`,
     `make_rect(3,4)` → 12).
   - **N2 store-touching now covers ALL the common types — scalars, vectors,
     structs, text, and enums (plain + data) — both directions** (14 green tests in
     `tests/n2_cdylib.rs`).
   - **Lean interface (source form) DONE** — `native_lib::generate_interface(data,
     export_set)` emits the library's public type defs + `#native "loft_shared_…"`
     forward-decls as **loft source** (types in the library's definition order via
     `Type::name` + `children_of`, skipping the auto-added `enum` discriminant
     field).  `lean_interface_drives_shared_dispatch`: a script whose *only*
     declaration is the generated interface dispatches `make_rect → area == 12` —
     **no hand-written type redefinition, no hand-written `#native` decl**.  The
     script parses only the interface (layouts + signatures + symbols), never the
     library bodies, and adopts the library's exact types in order, so the schema
     agrees **by construction**.  (A binary schema load — the D2a cache — is the
     robust successor covering non-public type ordering; this source form covers
     the common case where the public types are the only ones.)
   - **Keyed aggregates route too** — `sum_values(sorted<Item[k]>) → 30`
     (`dispatches_sorted_arg_into_shared_cdylib`): a `sorted` collection crosses as
     a `DbRef` (same ABI as a vector/struct), and the native body walks it through
     the shared store.  `hash`/`index`/`spacial` use the identical code path.
   - **Remaining:** closures (`__closure` param); `generate_interface` rendering of
     aggregate type names (`Type::name` debug-formats the key — `sorted<Item,[("k",
     true)]>` not `sorted<Item[k]>` — so a `sorted`-typed public fn needs a
     hand-written decl until the renderer reconstructs `[key]`); auto-deriving the
     whole flow (compile cdylib + interface) from `use <lib>` (N3 policy /
     productization).
3. **N5 soundness** of the boundary, woven through both.

The acceptance gate for each slice is **end-to-end** (an interpreted script calls an
auto-compiled lib function and gets the right answer), not a per-step check — so it
lands as one working unit, never a half-built cdylib in the tree.

### N3 productization design (the `use`-flow wiring, 2026-06-04)

The N3 **core** is proven in-process (`auto_native_marks_and_dispatches_normal_library_fn`):
mark → build → load → dispatch, with no `#native` decl.  Productization wires it into
`use <lib>` on the real binary.  The pieces and where they hook (per the
`use`/native-package flow in `parser/mod.rs` + `main.rs`):

1. **Opt-in (manifest).**  A library's `loft.toml` declares it wants native
   compilation — e.g. `[library] compile = "native"` (a new field beside the
   existing `native = "<stem>"` hand-written-cdylib field).  N4's gate already says
   *which* functions compile; this says *whether* the library does.  The eventual
   policy (N3 proper) makes it automatic: a stable/installed dep → native, a library
   under active edit (mtime newer than its cached artifact) → interpret.
2. **Mark at parse.**  When `use foo` resolves such a library (`lib_path` →
   `lexer.switch` parses it into the `Data`), record `(lib_source, pkg_dir)`.  After
   parsing, before `byte_code`: `candidates = {d | data.def(d).source() == lib_source}`,
   then `mark_native_exports(data, candidates)`.
3. **Build after `byte_code`.**  ✅ **Production helper landed:**
   `native_lib::build_shared_cdylib(data, stores, export_set, out_dir, stem)` does
   it all — `find_loft_rlib()` (locates this build's `libloft.rlib` + `deps/`,
   **handling both contexts**: a real `cargo run` unhashed `target/<prof>/libloft.rlib`
   *and* a test's hashed `libloft-<hash>.rlib` in `deps/` — the cross-context risk,
   resolved) → `generate_shared_cdylib_lib_rs` → `output_native_library` (no main
   bootstrap) → write `lib.rs` → rustc (`--crate-type cdylib`, edition 2024,
   `--extern loft=` + feature-dep externs).  Proven in the test context by
   `auto_native_marks_and_dispatches_normal_library_fn`.  **Remaining for `main.rs`:**
   call it after `byte_code`, cache the `.so` (N0's `.loft-build-fp` sidecar gates
   staleness; N1's idle-TTL evicts), push to `pending_native_libs`.
4. **Load + wire.**  `main.rs` already calls `load_all(pending_native_libs)` +
   `wire_native_fns`; add `wire_shared_native_fns(&mut state, &p.data)` alongside it.

This is mostly *connecting proven pieces*; the real risk is the cross-context
rlib-location (step 3) which only the real `cargo run` path exercises — so it lands
with a real on-disk fixture library + an `exit_codes`-style subprocess test, not just
an in-process one.

**Excluded — the library validation layer (deferred, customer-facing).**  The
fingerprint's eventual owner: an artifact's validity = content · target · features ·
loft-build-fingerprint · signature, plus registry distribution of per-target
artifacts.  Becomes load-bearing when **daily builds** ship (C71's
developer-vs-customer framing).  Tracked as a future arc, **not here**.

---

**In-progress — arc A0 + arc A landed; arc B's write path COMPLETE; arc C's store→native reader COMPLETE and the round-trip is now FULLY LOSSLESS.**  The whole real `default/` stdlib round-trips `native → store → native` **bit-for-bit** — the full `compare_data` oracle (every `Definition` field including the per-function variable table: `vars` + `names` + `inline_refs`) is green across all definitions.  The store schema was grown (`Function` gained `names: vector<NameNr>` + `inline_refs: vector<integer>`) after confirming neither is reconstructible from the variable list and that codegen reads both on the load path.  A store-materialised `Data` is now indistinguishable from a fresh parse.  **Arc D probe (proven end-to-end):** the full mmap loop works today — materialize the stdlib into a **file-backed** store, drop, reopen via `Store::open` (mmap), and `read_data` rebuilds the native `Data` with **no re-parse and no schema registration** — **~12× faster than `parse_dir`** (0.92 ms vs 11.4 ms median; see § Open design questions Q5).

- **Arc A0** (typed field cursor, commit `a07ed8d`) — landed as `RecordCursor`/`RecordCursorMut` wrapping `Store`'s raw primitives.  That cursor form has since been superseded by the typed handle layer (see § Arc A0 — handle layer below); `src/data_store.rs` is now the accessor seam, not a bare cursor.
- **Arc A** (IR store schema, commit `ed21b3e`) — landed as `tools/ir_schema/` (hybrid generate-extract pipeline) + `src/ir_schema_gen.rs` (generated, checked-in).  The full IR is registered via `register_ir_schema(db: &mut Stores) -> IrSchemaIds`; every struct/enum is in the schema; `db.finish()` computes all field positions, record sizes, and discriminants including the 34-variant `Node` enum size.
- **Typed handle layer** (`src/data_store.rs`, commit `9d860c5`) — minimum accessor seam: `Value`/`ValuesVector` thin `DbRef` handles with `ValueType` enum covering the IR-walker's current match surface.  Three tests pass (NdCall round-trip, NdBlock round-trip, layout guard).  Fmt-clean, clippy-0.
- **Arc B fork-cleanup (prerequisite, done)** — removed the dead shells-only `ir_schema::register_ir_schema` + its consts/tests, leaving exactly one schema registration (`ir_schema_gen`).  The @PLN82 JSON codec stays (interim — arc B's traversal skeleton + `compare_data` oracle); its 30 lib tests + 6 round-trip tests still pass.
- **Arc B write path (in-progress)** — both recursive IR enums now materialize fully. `src/data_store.rs` is the write/layout authority: per-variant `Node` writers + generic typed field accessors (`field_int`/`set_field_int`, …float/single/bool/str, `field_vec`/`field_recvec`, `set_discriminant`), `ValueType`/`value_type` over all 34 `Node` variants and `TypeKind`/`type_kind` over all 24 `TypeT` variants, plus a **generic non-`Node` struct-vector layer** (`Record` + `RecVector`, stride-parameterised) — built on the probed fact that **every IR vector is inline `Parts::Vector`, never a linked `Array`**, so one handle serves `vector<Key>`/`vector<TypeT>`/`vector<integer>`/`vector<SortKey>`/`vector<NameRef>`.  `src/ir_store.rs` materializes **all 34 `Node` variants** (`materialize_node`, now an exhaustive match — no deferred arm) **and all 24 `TypeT` variants** (`materialize_type`, with `IntegerSpec` inline + `SortKey`/`NameRef`/`integer` dep lists + box-of-one recursion).  Every baked discriminant + offset + stride is pinned by the `baked_layout_mirrors_loft_schema` guard (probed from the real schema, not guessed; inline sub-struct offsets verified as base + relative).  `Attribute`, `LinkedFieldGroup`, the full `Block`, and the top-level structs `Variable`/`Function` (via the `variables/mod.rs` snapshot seam) + `Definition` (23 fields, inlining `Position` + `Function`) + `Data` now materialize.  **`ir_store::materialize_data(&Data) -> DbRef` is the capstone entry point — the entire native `Data` writes into a store, exercised on the real `default/` stdlib** (`materialize_whole_stdlib_smoke`: every definition name, attribute count, and variable count round-trips through the store).  **Arc B's write path is complete.**  18 lib tests green; whole 438-test lib suite green; fmt-clean, clippy-0.

  Finding (fixed): `Store::claim` reuses freed blocks without zeroing, so a freshly-pushed vector element carried garbage in its unwritten vector-header sub-fields, and the next nested push dereferenced a junk record id (SIGSEGV deep in the real-stdlib walk).  Added `Store::zero_range`; `ValuesVector::push`/`RecVector::push` now clear each new element (mirrors the generated `--native` code that zeroes vector-header slots, `codegen_runtime.rs:1481`).

  **Remaining (arc C territory):** a store→native **read** path so the materialized store can be validated bit-for-bit by `compare_data` against a fresh parse.  ✅ **Done** — `src/ir_read.rs` (`read_value`/`read_type`/`read_data`) + the `Function.names`/`inline_refs` schema growth make `compare_data` green on the whole stdlib (see the arc C bullets below).  Arcs D/E remain open; arc C's bulk read-site migration remains.

- **Arc C read path (in-progress)** — `src/ir_read.rs` is the store→native reader, the exact inverse of `ir_store.rs`.  **`read_value(&Stores, Node) -> Value`** rebuilds all 34 `Node` variants and **`read_type(&Stores, Record) -> Type`** rebuilds all 24 `TypeT` variants, plus every sub-struct reachable from them (`Block`, `ParForBody`, `Position`, `Key`, `IntegerSpec`, `vector<SortKey>`/`vector<NameRef>` key lists, `vector<integer>` dep lists).  Box-of-one `vector<…>` fields read back as `Box<Value>`/`Box<Type>`; N-element vectors as `Vec`.  `Block.name` (`&'static str`) is reconstructed via a bounded `Box::leak`, mirroring the @PLN82 JSON decoder (open question 2).  Validated by **`native → store → native` round-trips asserted with the IR's own derived `PartialEq`** — a stronger oracle than the JSON re-encode, and needing no JSON.  7 round-trip tests (all `Value` leaves + recursive/box-of-one/Block/Loop/Span/ParFor/Keys/FnRef variants; all 24 `Type` variants + nested recursion; an explicit `forced_size` check since `IntegerSpec`'s `PartialEq` ignores it).  445-test lib suite green; fmt-clean, clippy-0.

- **Arc C Definition/Data reader (complete)** — `src/ir_read.rs` now also has **`read_data(&Stores, DbRef) -> Data`** (the inverse of `materialize_data`) plus `read_definition` / `read_attribute` / `read_field_group` / `read_function`, the inline-`Position`/`Function` readers, `def_type` / `purity` integer-code inverses, and `Vec<u32>`/`Vec<String>`/`Vec<u16>` list readers.  Derived state is reset exactly as the @PLN82 JSON loader does (`attr_names` rebuilt from the attribute list; `code_position`/`code_length`/`const_ref` recomputed by the compile pass; `Data::rebuild_indices` re-derives the lookup maps).  Two whole-stdlib capstones, both green on the real `default/`: (1) `read_whole_stdlib_round_trips_except_var_names` — **every** definition's non-variable fields round-trip **bit-for-bit** (`definition_to_json` equality with the variable block blanked) and the per-variable nine codegen-read fields round-trip exactly; (2) `read_stdlib_type_level_defs_full_compare_data_green` — the **full** `compare_data` oracle (including the variable block) is green for all 50+ type-level defs (empty variable tables).  447-test lib suite green; fmt-clean, clippy-0.

  **Finding (confirmed, now resolved) — `Function.names` / `inline_ref_vars` were not reconstructible; the store schema grew to hold them.**  The plan's earlier note ("rebuildable from the variable list on load") did **not** hold: `names` is pruned on scope exit (a finished function's `names` map is a *subset* of its variable list — scope-removed entries are gone — so the var list can't faithfully rebuild it), and `inline_ref_vars` is compile-derived (`insert_inline_ref` during scope analysis), absent from the nine stored per-`Variable` fields entirely.  Both are needed for the **mmap end goal**: the @PLN82 snapshot seam (`variables/mod.rs`) is explicit that codegen **reads** `names` + `inline_ref_vars` on the load path, so a mmap'd `Data` is unusable without them.

- **Arc C schema-growth pass (complete)** — `Function` in `tools/ir_schema/ir.loft` gained `names: vector<NameNr>` (`struct NameNr { name: text, nr: integer }`) and `inline_refs: vector<integer>`; `extract.py` learned the new `NameNr` type; `ir_schema_gen.rs` was regenerated.  The growth shifted the inlined-`Function` tail of `Definition` by +8 bytes (`Function` 12→20; `DEFINITION_STRIDE` 142→150; `DEF_MUTATED_CAPTURES`…`DEF_PUB_VISIBLE` +8) and added `FN_NAMES`/`FN_INLINE_REFS` + the `NameNr` consts — all probed from the regenerated schema and pinned by the `baked_layout_mirrors_loft_schema` guard.  `ir_store::write_function` now writes both vectors; `ir_read::read_function` reads them (no more best-effort reconstruction).  **Result:** `read_whole_stdlib_compare_data_green` — the full `compare_data` oracle on the entire real stdlib — is green; `read_stdlib_function_variables_round_trip` confirms 20+ populated function variable tables re-encode identically.  447-test lib suite green; fmt-clean, clippy-0.

### Bulk read-site migration — slice 1: `state/` `Definition` accessor seam (complete)

The store↔native representation is proven lossless; arc C's remaining work is the **bulk read-site migration** (route the ~940 IR-read sites through accessor methods, so each subsystem's representation can later swap to store-backed — § Incremental migration).  Slicing decision (user, 2026-06-02): do it **subsystem by subsystem**, and **within `state/`, the `Definition` field-accessor seam first** (the tractable, pure-refactor part), deferring the 451 `Value`/`Type` enum-match sites in `state/codegen.rs` to a later slice (those need handle-based dispatch, a real restructuring, not an `as_call()`-style seam).

Slice 1 landed: added read-accessor methods on `Definition` for the **store-backed** fields `state/` reads — `name()` / `native()` / `source()` / `position()` / `attributes()` / `code()` / `returned()` / `op_code()` / `known_type()` / `variables()` — returning the shapes a future store swap can produce (`&str` / `&[Attribute]` / `&Type` / `&Value` / `&Position` / `&Function` / Copy scalars).  Converted every `data.def(d).FIELD` and local `def.FIELD` read in `state/{mod,debug,codegen}.rs` (~120 sites) to the methods.  The codegen-**derived** fields `code_position` / `code_length` are deliberately **not** seamed — they are recomputed on load, never stored, so they stay native field reads.  Pure refactor, **no behaviour change**; the full integration suite passes.  fmt-clean, clippy-0.

**Slice 2 — `generation/` `Definition` seam (M1b, done 2026-06-04):** the same
pure-refactor pattern applied to the native backend — ~345 `Definition`
field-reads across all 14 `generation/*.rs` + `ops/*.rs` files routed through the
accessors (`name`/`variables`/`known_type`/`returned`/`attributes`/`position`/
`native`/`code` + two new ones `def_type()`/`rust()`).  `const_ref` (derived) and
`tuple_group()` (already a method) stay direct.  No behaviour change; full suite
2024/2024 green.

**Slice 3 — `parser/`+`compile.rs` `Definition` seam (M1c, done 2026-06-04):** the
read-site seam completed across the last subsystem — ~430 reads routed through the
accessors (reads only; the parser builds Definitions by construction, so writes stay
direct).  Added `parent`/`closure_record`/`mutated_captures`/`scalars_to_box`/`synthetic`
accessors.  With this, **the whole-codebase `Definition` read seam is complete**
(`state/` + `generation/` + `parser/`).

**Next slices (open):** `state/codegen.rs`'s `Value`/`Type` walk (handle-based dispatch — the 451 matches); then the per-subsystem representation swap (dual-backed `Data` + equivalence assertion).

### Arc D probe — the mmap load loop works end-to-end (~12× faster than parse)

`ir_store::materialize_data_at(stores, root, data)` (a thin variant of `materialize_data` that writes into a caller-provided root record) lets the IR materialize **directly into a file-backed store** (`Store::open(path)`).  The regression test `ir_read::tests::mmap_file_round_trip_stdlib` proves the whole loop on the real stdlib: materialize → file-backed store → drop (mmap flush) → reopen via `Store::open` (mmap) → `read_data` rebuilds the native `Data`, with **no re-parse and no schema registration** (the reader walks the mapped bytes through baked offsets; `DbRef` is store-relative so the root is rebuilt against the reopened store's slot; the whole IR — records, inline vectors, interned strings — lives in one store, so one file captures everything).  The result is bit-for-bit identical to a fresh parse (`compare_data`).

**Measured (`bench_stdlib_load_mmap_vs_parse`, warm page cache, 25 iters):** producing the native stdlib `Data` via `parse_dir` is **11.4 ms** median; via `Store::open` + `read_data` it is **0.92 ms** median — **~12.4×** (12.6× min).  Store file ≈ **6.9 MiB**.  This is *with* the full native rebuild (`read_data` allocates the `Vec`/`Box`/`String` graph); the speedup comes from skipping lexing + two-pass parsing + type resolution + scope analysis.  Both paths still run codegen→bytecode afterward (unchanged), and @PLN82 measured parse as ~14.7 ms of the ~17 ms cold-start, so this attacks the dominant chunk.  The representation migration (zero-copy reads, § Incremental migration) removes even the ~0.9 ms rebuild later — but the rebuild path is already a large win, confirming the risk posture that "the store layout is good enough to build on" (Q5).

Still open in arc D: wiring this into the real startup path — the bundle cache key + drift detection (Q4), the locked-mmap mutability split (Q1), and the `caller_index` rebuild (Q3).

Original note: This is the **mmap end-goal** that
[@PLN82 startup-cache](../82-const-store/STARTUP_CACHE_PLAN.md)
named but deferred: rework the compiler's in-memory IR (`Data` and the
`Value` / `Type` / `Definition` / `Function` graph) so it lives in a
`Stores` instance addressed by `DbRef` — **the same representation
`loft --native` already generates for user struct-enums** — instead of
native Rust `Vec<Definition>` / `Box<Value>` / `String`.  Once the IR
is store-backed, `Store::open(path)` (which already mmaps, zero-copy —
`src/store.rs`) loads a precompiled `Data` with **no rebuild step**,
collapsing cold-start parse time (~14.7 ms of the ~17 ms baseline,
measured in @PLN82 Step 0) to a page-fault.

Large and invasive — touches the ~940 `data.def(...)` read sites and
every `match value { Value::Call(..) }` in parser / codegen / scope
analysis / native generation.  **Not** required for @PLN82's cold-start
win (a rebuild-on-load snapshot gets that); required for the
*zero-rebuild, mmap-the-shipped-file* model.

**Now promoted to next (2026-06-01).**  @PLN82 proved by measurement that
a JSON snapshot **cannot** beat the parser — both deserialize text into the
same heap graph (~15–24 ms load ≈ ~11–23 ms parse; see @PLN82 § Step 3).
So the cold-start goal is unreachable by any serialization format and falls
to *this* plan's zero-copy mmap.  @PLN82 did not ship the cold-start win,
but it shipped the **reusable foundation** this plan needs:

- the exhaustive `Data` / `Value` / `Type` / `Definition` / `Function`
  traversal (`src/ir_schema.rs`) — arc B's native→store materializer is the
  same walk with a store-writer sink instead of a JSON sink;
- the database-schema enumeration (`src/database/snapshot.rs`) — arc A's
  schema spec;
- `compare_data` — arc C's native-vs-store equivalence oracle;
- `LOFT_DUMP_SNAPSHOT` + the `from_snapshot` `done=true` skip-`scopes::check`
  insight — arc D debugging / load wiring.

**First concrete steps: arc A0 and arc A (both done, 2026-06-01).**  Arc A0
landed as a typed field cursor (`a07ed8d`), then evolved into the typed handle
layer (§ Arc A0 — handle layer).  Arc A landed as the `tools/ir_schema/`
hybrid pipeline + `src/ir_schema_gen.rs` (`ed21b3e`).  The minimum accessor
seam (`src/data_store.rs`) landed in commit `9d860c5`.

**A second, mmap-independent payoff — IR locality (user, 2026-06-01).**  The
win here is not only zero-copy load.  The native IR is a pointer graph of
separately-allocated `Box<Value>` / `Vec<Definition>` / `String` / `Box<Type>`
nodes scattered across the heap; the store packs a record's fields contiguously
in one rigorously-laid-out buffer.  That tight layout is cache/prefetch-
friendly, so traversing the store-backed IR may touch far fewer cache lines
than chasing the equivalent `Box` graph — potentially a **net speedup on the
hot walk even before mmap**.  A hypothesis to confirm with numbers (open
question 5), but it means "Data-as-store" can be a structural optimisation in
its own right, not just the cold-start enabler.

**Standalone upside — a functional serialise/inspect layer that *converges*
on the database's own JSON (2026-06-01).**  Independent of the cold-start
goal, @PLN82's codec already gives a rich, working **serialise / deserialise
/ compare** layer over the IR and the store schema (`ir_schema::*_to_json` /
`*_from_json`, `database::snapshot::schema_to_json`, `compare_data`,
`LOFT_DUMP_SNAPSHOT`), proven lossless on the real stdlib — immediately useful
as inspection + debugging tooling (dump any parsed `Data`/`Stores` to readable
JSON, diff two compilations field-by-field, regression-pin IR shapes), and
useful *throughout* this plan (every arc can dump-and-eyeball or `compare_data`
its intermediate state against a fresh parse).

**But it is NOT yet the database's own JSON — and that gap is the point.**
There are two distinct JSON producers today:

| Producer | Walks | Driven by | Shape |
|---|---|---|---|
| `Stores::show_json` (`src/database/format.rs:69`) | **store records** via `DbRef` + `tp` | the database type schema | the database's native record-JSON |
| `ir_schema::data_to_json` + `database::snapshot::schema_to_json` | the **native** `Data` / `Vec<Definition>` / `Box<Value>` graph (+ `Stores.types` as native structs) | hand-rolled per-type walks | tagged objects `{"k":…}` |

The @PLN82 codec is a *hand-rolled walk over native Rust IR* — it does **not**
emit the same bytes as `show_json`, because the IR does not yet live in store
records.  **The convergence is exactly arc B:** once the IR is materialised
into store records (arc A schema + B write), `Stores::show_json` walks the IR
*directly* — and the hand-rolled native walk is subsumed by the database's own
serialiser.

**Decision (user, 2026-06-02) — the JSON codec is bootstrap scaffolding, slated
to go.**  The @PLN82 JSON layer (`ir_schema::*_to_json` / `*_from_json` /
`compare_data` / `LOFT_DUMP_SNAPSHOT`) "was useful to get the wagon rolling but
[is] not for the final goal."  It earns its keep *now* — exhaustive native-IR
walk reused as arc B's traversal skeleton (just swap the JSON sink for a
store-writer sink), and `compare_data` as arc B's equivalence oracle — but it is
**not** a permanent facility and is **not** to be polished into a parallel
`show_json` alternative.  The final state is `Stores::show_json` over the
store-backed IR; the native-JSON codec is **retired** once arc B's store walk is
proven (its `compare_data` validation having served its purpose).  Treat it as
interim throughout: lean on it freely while building, but do not invest in it as
an end state.

## Goal

Represent the compiler IR (`Data` + `Value`/`Type`/`Definition`/
`Function`) as `Stores` records using the same struct-enum store schema
`loft --native` emits, so a precompiled `Data` can be `mmap`-ed from
disk into a live, queryable IR with zero deserialization.

## What gets cached — two snapshots, both whole-prefix

Scope is deliberately **two** snapshot kinds, both *deterministic-
parse-order prefixes* — never independent per-library files:

1. **Core stdlib** — the always-loaded `default/*.loft` prefix.  Parsed
   first, in fixed order, on every run, so its def_nr / `known_type`
   layout is identical every time → one shared, shipped `stdlib.store`
   that every program mmaps.
2. **Full per-script bundle** — core **plus the exact set of libraries
   the script `use`s**, snapshotted as one unit (core + sorted lib-set).
   Keyed on the bundle (`stdlib_cache_key` + the sorted lib list + lib
   content hashes).  A repeated run of the *same* script / app mmaps its
   whole compiled `Data` — stdlib **and** its libs — with zero parse.

**Explicitly out of scope — settled, not just deferred (user, 2026-06-02):**
independent per-library mmap / per-library IR snapshot that composes arbitrary
libs on demand.  Two reasons, both permanent:

1. **A library cannot cleanly write its own IR.**  The IR is global-index
   (def_nr / `known_type` are absolute, parse-order-dependent — see § Why the
   global-index model is fine), so a library snapshotted in isolation would need
   name-based relocation into whatever prefix it lands in.  That relocation is
   the brittlest possible part of the system and it buys the least-common case.
2. **The loft source is the better representation of a library's state anyway.**
   For distributing / versioning / inspecting a library, the `.loft` source —
   not a serialized IR image — is the right artifact.  And there is **no
   efficiency case** for a serialized per-library form: @PLN82 already
   established that (de)serialization is not faster than parsing natural loft
   source (~15–24 ms load ≈ ~11–23 ms parse — see § Status).  So a per-library
   IR cache would be a worse, harder-to-relocate stand-in for something the
   source already expresses well *and* parses just as fast.

Caching the **whole bundle** (core + the script's sorted lib-set) sidesteps both
— every index inside one image is internally consistent, no relocation anywhere.
Closed in the decision register: [DESIGN_DECISIONS.md § C70](../../DESIGN_DECISIONS.md#c70--no-per-library-ir-snapshot--cache).

**Interim stop-gap (precedes this plan):** @PLN82 Step 2 ships a
**whole-stdlib / whole-bundle JSON snapshot** (loft's own database JSON,
not serde — user-accepted 2026-05-31) that rebuilds native `Data` on
load.  Second-class (JSON is re-parsed, not mmap'd) but delivers the
cold-start win without the IR rewrite.  This plan **supersedes** it: the
store struct-enum format replaces JSON and turns the rebuild into a
zero-copy mmap of the same whole bundle.

**Per-library snapshot — dropped, not deferred (user, 2026-06-02; first raised
2026-05-31):** a per-library deliverable would close the first-landing gap for a
brand-new `use` combination, but a library **cannot cleanly write its own IR**
(global indices need name-based relocation into an arbitrary prefix — the
brittlest part of the stop-gap, optimizing the least-common case), and the
`.loft` **source is the better representation of a library's state anyway** (see
§ What gets cached).  So neither @PLN82 nor this plan does per-library: both
operate on the **whole bundle as one image** with absolute, internally-
consistent indices — no relocation anywhere.  @PLN82 builds the stop-gap
format-agnostic so this plan swaps the bundle encoder underneath without
touching startup wiring.

## Why the global-index model is fine for this scope

`Data.definitions` is one global `Vec`; core and every `use`d library
**append into it** (`add_def` → `rec = self.definitions()`).
Cross-references are global indices — `Type::Reference(u32,…)`,
`Type::Enum(u32,…)`, `Value::Call(u32,…)` carry a global `def_nr`;
`known_type: u16` indexes the global `database.types` schema.  So a
compiled `Data` is **position-dependent on parse order**.

That is exactly why the scope is whole-prefix snapshots, not per-library
files: a snapshot freezes a *complete* parse-order prefix (core, or
core+libs), so every global index inside it is valid as-is when mmap'd
back — no relocation, zero-copy.  Independent per-library mmap would
need source-relative indexing or a relocation pass; whole-bundle caching
avoids the question.  (`--native` itself uses the same global-index
model — it rebuilds one type space at runtime by calling each crate's
`init()` in sequence — so "mirror `--native`" inherits global indices,
consistent with whole-bundle caching.)

The only cost: the bundle cache key is the whole lib-set, so the
**first** run of a never-seen `use` combination still parses fully; the
win lands on every subsequent run.  For the dogfood consumers (games,
servers, the indexer/viewer) run repeatedly, that's the case that
matters.

## Effort + design

- **Effort:** L (large multi-arc — IR rewrite + access-site migration)
- **Design:** arc A0/A settled; arc C seam minimum landed; B/D/E open
- **Last touched:** 2026-06-01

## Why mirror `--native` specifically

`loft --native` already solves "represent loft struct-enums as store
records" (NATIVE.md § Architecture): generated code uses
`loft::database::Stores` + `loft::keys::{DbRef, Str, Key}`, and an
`init(db: &mut Stores)` function registers every type schema via
`db.structure()` / `db.enumerate()` / `db.value()` / `db.vector()`
(NATIVE.md § `output_init`).  The compiler's IR enums (`Value`, 34
variants; `Type`, 24 variants) are themselves recursive struct-enums, so
the representation problem is **already solved for user types** — this
plan applies that machinery to the compiler's own types.

Mirroring `--native` rather than inventing a third format buys:

- **One representation to maintain.**  The store struct-enum format is
  exercised by every `--native` build; the IR rides the same code.
- **mmap for free.**  `Store::open` already maps a file and marks it
  `borrowed` + `locked` (`src/store.rs:309`); a store-backed `Data`
  inherits that with no new persistence code.
- **Position-independent records.**  `DbRef { store_nr, rec, pos }` is
  offset-based (verified in the @PLN82 audit), so a mapped store is
  valid at any base address — the precondition for mmap.  (Global
  *def_nr* indices are a separate axis, handled by whole-prefix
  snapshotting above.)

## Arc A reference — the IR transcribed as loft types (verified 2026-06-01)

The most efficient way to pin arc A's store schema is **not** to hand-write
`db.structure`/`enumerate`/`value` calls — it is to **transcribe the whole IR
as loft `struct`/`enum` declarations and let `loft --native` generate the
schema + record accessors for it.**  The generated Rust *is* arc A's
`init`-equivalent, produced by loft itself.

The transcription below **parses + lays out + runs under `--interpret`**
(empty `Data`), exercising every type:

```loft
// Mapping from native Rust IR (src/data.rs) to loft types:
//   Box<Self>          -> see findings: reference<OtherType> works; a SELF
//                         reference must be vector<Self> (box-of-one)
//   Vec<Self>          -> vector<Self>
//   u8/u16/u32/i32/i64 -> integer   ;  bool -> boolean
//   f64 -> float ; f32 -> single ; String -> text
//   Vec<u16>           -> vector<integer>
//   Vec<(String,bool)> -> vector<SortKey>   ;  Vec<String> -> vector<NameRef>
//   Option<T>          -> sentinel field (0 / "" = None)

struct Position { file: text, line: integer, pos: integer }
struct Key { type_nr: integer, position: integer }      // i8, u16
struct SortKey { name: text, asc: boolean }
struct NameRef { name: text }
struct IntegerSpec { min: integer, max: integer, not_null: boolean, forced_size: integer }

// Enum variant names are GLOBAL type names → must be unique across all enums
// and not collide with builtins; hence the Ty / Nd CamelCase prefixes.
enum TypeT {
  TyUnknown { n: integer }, TyNull, TyVoid, TyNever,
  TyInteger { spec: IntegerSpec }, TyBoolean, TyFloat, TySingle, TyCharacter,
  TyText { dep: vector<integer> }, TyKeys,
  TyEnum { n: integer, is_ref: boolean, dep: vector<integer> },
  TyReference { n: integer, dep: vector<integer> },
  TyRefVar { inner: vector<TypeT> },
  TyVector { inner: vector<TypeT>, dep: vector<integer> },
  TyRoutine { n: integer },
  TyIterator { step: vector<TypeT>, inner: vector<TypeT> },
  TySorted { n: integer, keys: vector<SortKey>, dep: vector<integer> },
  TyIndex { n: integer, keys: vector<SortKey>, dep: vector<integer> },
  TySpacial { n: integer, names: vector<NameRef>, dep: vector<integer> },
  TyHash { n: integer, names: vector<NameRef>, dep: vector<integer> },
  TyFunction { args: vector<TypeT>, result: vector<TypeT>, dep: vector<integer>, consts: integer },
  TyRewritten { inner: vector<TypeT> }, TyTuple { elems: vector<TypeT> }
}

enum Node {
  NdNull, NdLine { n: integer },
  NdSpan { pos: Position, inner: vector<Node> },
  NdInt { n: integer }, NdEnum { ord: integer, tp: integer },
  NdBoolean { b: boolean }, NdFloat { f: float }, NdLong { n: integer },
  NdSingle { f: single }, NdText { s: text },
  NdCall { def_nr: integer, args: vector<Node> },
  NdCallRef { var: integer, args: vector<Node> },
  NdBlock { block: reference<Block> }, NdInsert { items: vector<Node> },
  NdVar { n: integer }, NdSet { var: integer, inner: vector<Node> },
  NdReturn { inner: vector<Node> }, NdBreak { n: integer },
  NdBreakWith { n: integer, inner: vector<Node> }, NdContinue { n: integer },
  NdIf { cond: vector<Node>, t: vector<Node>, f: vector<Node> },
  NdLoop { block: reference<Block> }, NdDrop { inner: vector<Node> },
  NdIter { var: integer, create: vector<Node>, next: vector<Node>, init: vector<Node> },
  NdKeys { keys: vector<Key> }, NdTuple { items: vector<Node> },
  NdTupleGet { var: integer, idx: integer },
  NdTuplePut { var: integer, idx: integer, inner: vector<Node> },
  NdYield { inner: vector<Node> },
  NdFnRef { def_nr: integer, var: integer, t: vector<TypeT> },
  NdFnRefDnr { n: integer }, NdParallel { arms: vector<Node> },
  NdParFor { body: reference<ParForBody> }, NdRawExpr { s: text }
}

struct Block { name: text, operators: vector<Node>, result: vector<TypeT>, scope: integer, var_size: integer }
struct ParForBody { input: vector<Node>, x_var: integer, r_var: integer,
                    worker: vector<Node>, threads: vector<Node>, body: vector<Node>, stitch_id: integer }
struct Attribute { name: text, typedef: vector<TypeT>, mutable: boolean, constant: boolean,
                   init: boolean, nullable: boolean, primary: boolean, hidden: boolean,
                   value: vector<Node>, check: vector<Node>, check_message: vector<Node>,
                   alias_d_nr: integer, assigned_lambda_d_nr: integer }
struct Variable { name: text, type_def: vector<TypeT>, stack_pos: integer, uses: integer,
                  argument: boolean, stack_allocated: boolean, skip_free: boolean,
                  captured: boolean, caller_hidden_buf: boolean }
struct Function { name: text, file: text, variables: vector<Variable> }
struct LinkedFieldGroup { kind: integer, instance: integer, field_indices: vector<integer>, alignment: integer, size: integer }
struct Definition { name: text, source: integer, def_type: integer, parent: integer, position: Position,
                    attributes: vector<Attribute>, code: vector<Node>, returned: vector<TypeT>,
                    returned_not_null: boolean, rust: text, native: text, op_code: integer, known_type: integer,
                    variables: Function, pub_visible: boolean, closure_record: integer,
                    mutated_captures: vector<NameRef>, scalars_to_box: vector<NameRef>, bounds: vector<integer>,
                    forced_size: integer, purity: integer, field_groups: vector<LinkedFieldGroup>, synthetic: text }
struct Data { definitions: vector<Definition>, source: integer }
```

**Findings (these are the arc A design constraints, learned the cheap way):**

1. **Enum variant names are global type names.**  Every variant (`TyBoolean`,
   `NdCall`, …) registers a `db.structure(name, ord)` whose name lives in the
   one global type namespace, so it must be unique across *all* enums and must
   not collide with a builtin (`boolean`, `vector`, `text`, …).  The real IR
   has `Value::Boolean` *and* `Type::Boolean`; in the store model these need
   distinct registered names (the `Ty`/`Nd` prefixes here).  Variant names must
   also be CamelCase (no underscores).

2. **A self-referential single child cannot be `reference<Self>`; use
   `vector<Self>` (box-of-one).**  `reference<OtherType>` lays out fine
   (`NdBlock { block: reference<Block> }` works — `Block` is a distinct type),
   but `reference<Node>` *inside* `Node` fails layout (`inner:Node@?..?`,
   unresolved size).  The recursion has to route through either a distinct
   wrapper type or a `vector<Self>` (which is a length-prefixed out-of-line
   chunk, so it has a fixed in-record size).  Every `Box<Value>` /
   `Box<Type>` single-child in the real IR maps to a `vector<…>` of length ≤ 1
   here.  **Arc A must decide:** box-of-one vector, or a dedicated indirection
   record (a `NodeRef { target: reference<NodeBox> }` shim).  The box-of-one is
   simplest and is what laid out.

3. **`--native` needs definition order to be dependency-respecting.**  The
   interpreter lays out the whole graph regardless of order, but the generated
   native code referenced `Block`'s type id (`t112`) before it was bound
   (`E0425`) because `Block` is declared after `Node` (which references it).
   Arc A's emitter must topologically order (or forward-declare) type
   registrations; for the reference script under `--interpret` this is moot.

The other open questions (`&'static str` interning, `OnceLock` caller-index,
`Option` mapping, mutability of a locked mmap) are unchanged from § Open design
questions; finding 2 in particular is the first concrete arc A decision.

### What the generated `--native` Rust gives us — and what it does NOT (2026-06-01)

Running `loft --introspect --show-rust` on the transcription (after
dependency-ordering the defs per finding 3) produces ~139 KB of Rust.  Reading
it settles exactly how much of arc A/A0/C the compiler hands us for free:

| Generated artifact | Form | Reuse verdict |
|---|---|---|
| **Schema registration** — the `init(db)` body: `db.enumerate("Node")`, `db.structure("Block",0)`, `db.field(t98,"name",t5)`, `db.value(t65,"NdCall",…)`, `db.vector(...)` | declarative calls into `Stores`, **all offsets / widths / enum discriminants / vector wrappers resolved by the compiler** | **Directly reusable — this IS arc A.**  Arc A's deliverable can be exactly this `init` block (emitted by the build, not hand-written). |
| **Field access** — every read is inline `stores.store(&db).get_int(db.rec, db.pos + 8)`, the enum tag via `get_byte(db.rec, db.pos + 32, 0)`, strings via `get_str(get_u32_raw(...))` | open-coded raw `(rec, fld)` arithmetic at each use site | **Template, not code.**  It documents the exact width + offset recipe per field; it is *not* factored into anything callable. |
| **A Rust `struct`/`enum` for the IR types, or per-type accessor fns** | — | **Does not exist.**  Confirmed: there is no `enum Node`, no `Node::call_def_nr(r)`.  An IR "type" exists only as a store schema + scattered inline reads.  The ~171 generated `fn`s are the program's own functions (each doing inline `get_*`), never type accessors. |

**Consequences for the plan:**

1. **Arc A becomes "extract the generated `init`," not "hand-design a schema."**
   The compiler already computes the authoritative layout; arc A's job is to
   capture that `init` block for the IR types (and topo-order it, finding 3) as
   the schema artifact.  This is the efficiency the transcribe-and-generate
   approach was after.
2. **Arc A0 is confirmed necessary and non-redundant with codegen.**  The
   generated reads are precisely the raw `db.pos + N` / `get_byte(…,32,0)`
   arithmetic A0's typed cursor wraps.  `--native` does **not** emit an
   accessor layer — in a normal loft program the *parser* holds field offsets
   and inlines them, so there is no "accessor object" to generate.  That gap is
   exactly arc A0 (the cursor) + arc C (the seam).  The generated inline reads
   are the **reference recipe** for the bodies of those accessors (which width,
   which offset, per field).
3. **`Data`-as-store is not a generated Rust type.**  It is
   `{ stores: Stores, <the init schema> }` plus a hand-built (A0-cursor-based)
   accessor layer mapping `data.def(d).name()` → `record(rec).str(FLD_NAME)`.
   The generated `get_*(db.pos+offset)` lines are the field-offset source of
   truth for writing those accessors; they are not themselves the accessors.

**Net:** the generated Rust is useful as the **schema source-of-truth (reuse
directly)** and the **per-field access recipe (reuse as template)** — not as
linkable code.  Schema = generated; typed accessor layer = built by us (A0 +
seam).  The two compose; neither alone is the deliverable.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A0** — typed `Store` field cursor | A `Record` / `RecordMut` wrapper over `Store`'s raw `get_int`/`set_int`/`addr::<T>` primitives: named, bounds-checked, typed field reads/writes so no IR accessor does `(rec, fld)` offset arithmetic directly.  Pure-additive precondition for A/C; ships value standalone (safer `--native` + fill.rs reads). | **Done** (cursor `a07ed8d`, superseded by the typed handle layer `src/data_store.rs` in commit `9d860c5`); see § Arc A0 — handle layer |
| **A** — IR store schema | **Extract** the `init(db)` schema-registration block `--native` already generates for the IR transcription (§ Arc A reference / § What the generated Rust gives us) — not hand-design.  The compiler resolves all offsets/widths/discriminants; arc A captures that block (topo-ordered, finding 3) as the schema artifact, after deciding finding 2 (box-of-one `vector<Self>` vs a wrapper record for single recursive children). | **Done** (commit `ed21b3e`) — `tools/ir_schema/` pipeline + `src/ir_schema_gen.rs` generated and checked in; see § Arc A reference |
| **B** — write path | Materialize a parsed native `Data` into store records (validates the schema).  **Greenfield, not "largely done":** @PLN82 shipped a *JSON* snapshot (native-side, `LOFT_DUMP_SNAPSHOT` → `data_to_json`), **not** a store-format one.  Reuses `ir_schema::data_to_json`'s exhaustive native-IR walk as the traversal skeleton (JSON sink → `data_store`-handle store-writer sink) + `compare_data` as the equivalence oracle.  **Write path COMPLETE:** `src/ir_store.rs` materializes the **entire** native `Data` — all 34 Node + 24 TypeT variants, every struct (`Attribute`/`Variable`/`Function`/`Definition`/`Block`/`LinkedFieldGroup`/`Data`), via the generic `Record`/`RecVector` layer — through `materialize_data(&Data) -> DbRef`, exercised on the real stdlib (`materialize_whole_stdlib_smoke`).  All offsets/strides guard-pinned; `Store::zero_range` clears reused element memory.  **Remaining (→ arc C):** a store→native read path for bit-for-bit `compare_data` validation (smoke test currently validates structure: names + per-def attribute/variable counts). | Write path done — whole `Data` materializes on real stdlib; `compare_data` equivalence needs the arc C read path |
| **C** — read accessors | `data.def(dnr)` + `value` / `type` matching read from the store instead of `Vec`/`Box`.  The ~940-site migration — **done incrementally via the accessor seam, never at once** (see § Incremental migration).  Minimum seam (`src/data_store.rs` handle layer) + the full **store→native reader** (`src/ir_read.rs` — `read_value`/`read_type`/`read_data` over all 34 Node + 24 TypeT variants + every struct) landed, and the schema grew to hold `Function.names`/`inline_refs` so the whole real stdlib round-trips **fully bit-for-bit** (`compare_data` green).  Bulk read-site migration: the **whole-codebase `Definition` read seam is complete** — `state/` (~120 sites, M1a) + `generation/` (~345, M1b) + `parser/`+`compile.rs` (~430, M1c).  Remainder: `state/codegen.rs` `Value`/`Type` walk (handle dispatch — the 451 matches), then per-subsystem representation swap. | In-progress — lossless reader done; `Definition` read seam complete (M1a+M1b+M1c) |
| **D** — mmap load | `Data::open(path)` → `Store::open` → live IR, zero rebuild.  Wire into the startup path behind the bundle cache key. | **Cold-start cache (G1) shipped, opt-in.**  Probe (~12× vs `parse_dir`) + **D1** (`save`/`open`) + **D2a** (schema cached via the `Bundle` root — @PLN82 schema-rebuild wall resolved) + **D2b** (`main.rs` env-gated `LOFT_STDLIB_CACHE` wiring; cold saves the keyed `.store` bundle, warm mmaps it & skips parsing `default/`; off/cold/warm output byte-identical via `tests/d2b_stdlib_cache.rs`) all landed.  Remaining (arc E): per-bundle key for `use`d libs + drift Q4, mutability Q1 (locked mmap), `caller_index` Q3. |
| **E** — bundle snapshots | Core `stdlib.store` (shared) + per-script bundle snapshot (core + sorted lib-set), each keyed for drift. | **Whole-program cache landed (opt-in).**  Robustness ✅ (atomic temp+rename write; `Store::is_store_file` pre-check + graceful reparse on a corrupt/foreign file).  **Whole-program cache ✅** — `LOFT_PROGRAM_CACHE` caches the *entire* parsed program (stdlib + lazily-loaded libs + user file) keyed on the script path, validated by a **drift manifest** of every parsed source's content hash; a warm run mmaps it and **skips ALL parsing** (589 defs in ~2.8 ms).  The lazy-auto-`use` boundary is sidestepped by caching the whole program rather than core+libs.  `tests/arc_e_program_cache.rs`: off/cold/warm identical, edit→drift→reparse.  Open: E2 mutability Q1 (locked mmap — only needed for the zero-copy G2 path) / `caller_index` Q3. |

## Phase ordering

0. **A0 (typed field cursor)** — ✅ done (`a07ed8d`).  Cursor form superseded by
   the typed handle layer (commit `9d860c5`); see § Arc A0 — handle layer.
1. **A (schema)** — ✅ done (`ed21b3e`).  `tools/ir_schema/` pipeline + generated
   `src/ir_schema_gen.rs` register the full IR schema.  Finding 2 (box-of-one
   `vector<Self>` for recursive single-child) resolved in the transcription.
2. **B (write)** — native `Data` → store.  Testable artifact; validates A
   before touching read sites.  Greenfield (@PLN82 shipped JSON, not a
   store-format snapshot — see arc B row); reuses `data_to_json`'s walk as the
   traversal skeleton + `compare_data` as the oracle.  Prerequisite (module
   fork, below) ✅ done.  **Write path ✅ done:** `ir_store::materialize_data`
   materializes the whole native `Data` (every Node/TypeT variant + every
   struct) into a store, exercised on the real stdlib.  The remaining piece —
   a store→native read path for full `compare_data` equivalence — folds into
   arc C (read accessors).
3. **D (mmap load, read-mostly)** — load a store-backed `Data` and run
   execution against it *while the parser still builds native `Data`*.
   Proves the read path on the hot loop before the full migration.
4. **C (access-site migration)** — convert `data.def()` and the IR
   `match` arms incrementally, by subsystem (state/ first, then
   generation/, then parser/).
5. **E (bundle snapshots)** — core snapshot first (deterministic, shared);
   per-script bundle snapshot second.

## Module fork to reconcile before arc B (found 2026-06-02; step 1 ✅ done)

Arc A left **two `ir_schema` modules with two same-named `register_ir_schema`
functions** in the tree.  This is a transition artifact, not a design — cleaned
up (step 1) before arc B built on top of it:

| Module | `register_ir_schema` | Role | Disposition |
|---|---|---|---|
| `src/ir_schema_gen.rs` (arc A, generated) | `-> IrSchemaIds` | the **complete** store schema (all fields/variants/offsets, `db.finish()`) | **keep** — this is arc A's deliverable |
| `src/ir_schema.rs` (@PLN82) | `-> usize` | **shells-only** schema (S1 rung, no fields wired); only its own test (`ir_schema.rs:1541`) calls it | **dead — delete** the shells-only register; its job is done by `ir_schema_gen` |
| `src/ir_schema.rs` (@PLN82) | — | the **JSON codec** (`*_to_json`/`*_from_json`/`compare_data`) | **interim — keep until arc B lands**, then retire (see § Standalone upside decision); reused as arc B's traversal skeleton + oracle meanwhile |

Concretely, before arc B:

1. ✅ **Done** — deleted the shells-only `ir_schema::register_ir_schema` (and its
   `shells_register_without_collision` / `prefix_is_not_a_legal_identifier`
   tests + the `IR_PREFIX`/`IR_ENUMS`/`IR_STRUCTS` consts), so there is exactly
   one schema registration in the codebase (`ir_schema_gen`).  The JSON codec
   (`*_to_json` / `compare_data`) stays.
2. Keep the JSON codec **only** as arc B's scaffolding — its walk is copied into
   the store-writer, and `compare_data` validates the result; it is deleted once
   that validation has run green and `show_json`-over-store works.

## Arc A0 — handle layer (landed; supersedes the cursor design)

**Original plan:** a `RecordCursor`/`RecordCursorMut` that bound `&Store + rec`
once and named the width method, so no accessor did open-coded `(rec, fld)`
arithmetic.  That cursor landed in commit `a07ed8d`.

**As-built:** `src/data_store.rs` has since been rewritten as the **typed
handle layer** (commit `9d860c5`) — the minimum accessor seam the plan's arc C
migration requires.  The cursor form (still green at `a07ed8d`) is superseded;
the handles subsume it.

### Design (three principles, user, 2026-06-01)

**Principle 1 — reuse, don't reimplement.**  Each accessor locates its field
and hands the read or write to an *already-written* primitive:
`Store::get_int`/`get_str`/`set_str`/`get_u32_raw`/`get_byte`,
`vector::length_vector`/`get_vector`/`insert_vector`.  NOT `Stores::show_json`
or `field_content` — that would rebuild the database's schema-walker from
scratch.

**Principle 2 — baked layout constants, no runtime indirection.**  Variant
discriminants (`DISC_NULL=1`, `DISC_INT=4`, `DISC_CALL=11`, `DISC_BLOCK=13`),
field byte offsets (`NDCALL_ARGS=4`, `NDCALL_DEF_NR=8`, `NDBLOCK_BLOCK=8`,
`BLOCK_NAME=16`, `BLOCK_OPERATORS=20`), and the `vector<Node>` element stride
(`NODE_STRIDE=48`) are hard-coded `const`s mirroring loft's schema.  Rationale:
accessors run **millions of times** on every IR walk; a runtime `position()`
lookup or schema name-match is indirection the compiler cannot fold, making the
layer unusable.  Each accessor folds to one store primitive at one constant
offset.  Methods take `&Stores`/`&mut Stores`; `IrSchemaIds` is not needed at
runtime.

**Principle 3 — guard test pins hand-typed consts to loft's real layout.**
`baked_layout_mirrors_loft_schema` (`src/data_store.rs::tests`) asserts every
const equals what `register_ir_schema` + `db.finish()` actually computed
(`stores.position(tp, field)`, `stores.size(node)`, variant discriminants).
The most important assertion is `NODE_STRIDE == size(node)`: the enum size
aggregates over all 34 variants and cannot be eyeballed; it is correct only
because `register_ir_schema` is the **complete** definition run through loft's
layout routine.  A mistyped constant compiles fine and silently reads the wrong
bytes millions of times — the guard turns that into an immediate CI failure.

### Public API (`src/data_store.rs`)

```rust
pub enum ValueType { Null, Int, Call, Block, Other(u8) }

pub struct Value { rec: DbRef }       // handle to one Node record
pub struct ValuesVector { rec: DbRef } // handle to a vector<Node> field

impl Value {
    pub fn new(rec: DbRef) -> Self;
    pub fn db_ref(&self) -> DbRef;     // for callers driving existing fns
    pub fn value_type(&self, stores: &Stores) -> ValueType;
    pub fn call_to(&self, stores: &Stores) -> u32;          // NdCall.def_nr
    pub fn call_parameters(&self) -> ValuesVector;          // NdCall.args
    pub fn block_name<'a>(&self, stores: &'a Stores) -> &'a str;  // NdBlock → Block.name
    pub fn block_name_set(&self, stores: &mut Stores, name: &str); // NdBlock → Block.name
    pub fn block_operators(&self) -> ValuesVector;           // NdBlock → Block.operators
}
impl ValuesVector {
    pub fn len(&self, stores: &Stores) -> u32;
    pub fn is_empty(&self, stores: &Stores) -> bool;
    pub fn get(&self, i: u32, stores: &Stores) -> Value;
}
```

### Verified layout facts (from probing the registered schema)

- `reference<Block>` inside `NdBlock` is **inlined**: a 28-byte `Block` struct
  at offset 8 — no pointer deref.
- `vector<Node>` is stored **inline** (the `is_linked`/P376 `Array` promotion
  is not triggered here); stride = 48 bytes.
- `integer` fields are 8 bytes, read via `Store::get_int` (returns `i64`).

### Status and tests

Minimum implementation: covers `NdNull`/`NdInt`/`NdCall`/`NdBlock` and the
`Block.name`/`Block.operators` sub-fields.  Three tests pass:
`ndcall_reads_back_through_handles`, `ndblock_name_and_operators_round_trip`,
`baked_layout_mirrors_loft_schema`.  Fmt-clean, clippy-0.

### Future direction (not done)

Replace the hand-typed const block by generating it from loft's own output:
write the accessors as *methods* in `ir.loft` (compiling to
`t_<len><Type>_<method>` functions with the offsets baked, uninstrumented
unlike `n_` free functions), then a script lifts those functions and their
offset literals into the generated layer.  The handle API and the layout guard
stay identical across that swap; it is a generation-automation improvement, not
a design change.

## Incremental migration — arc C is many small plans, never one

The ~940 `data.def(...)` reads and the `match value { … }` /
`match type { … }` arms cannot move to a store-backed representation in
a single change without breaking the project.  Arc C is therefore a
**series of follow-up plans**, each green and shippable on its own,
enabled by an **accessor seam** introduced *before* any representation
changes.

**The seam (precondition, cheap, additive).**  Route every IR read
through accessor methods instead of touching fields directly:
`data.def(d).name()`, `.returned()`, `.code()`, … and small helpers
over `Value` / `Type` (e.g. `value.as_call()`, `ty.as_reference()`)
that today just `match` the native enum.  This is a pure refactor with
**no behaviour change** — the native `Vec`/`Box` stays underneath — so it
lands incrementally under the normal green-commit discipline and is a
valid stop at any point.  Once a subsystem reads only through the seam,
its representation can be swapped without touching that subsystem again.

**Then migrate behind the seam, one slice per follow-up plan:**

1. **Seam-only plan** — introduce the accessor methods; convert
   call-sites to them mechanically, subsystem by subsystem
   (`state/` → `generation/` → `parser/`).  Representation unchanged.
   Each subsystem is its own commit; the build is green throughout.
2. **Per-subsystem representation swap** — with `state/` reading only
   through the seam, move *its* reads to the store accessor; leave the
   rest on native.  A **dual-backed `Data`** (native `Vec` *and* the
   store, kept in sync during the transition) lets one subsystem read
   from the store while others still read native — this is what makes
   "not at once" possible.  Repeat per subsystem.
3. **Drop the native backing** — once every subsystem reads from the
   store, delete the native `Vec`/`Box` fields and the sync.  Only now
   is `Data` truly store-backed; only now does mmap (arc D) become
   zero-copy for *reads*, not just load.

**Why this is safe the same way @PLN82's ladder is:** each step is
additive (the seam adds methods, doesn't remove fields), off the
critical path until proven (dual-backing runs both representations and
can assert they agree), and reversible (revert one subsystem's swap
without touching others).  A per-subsystem **equivalence assertion**
(native read == store read, behind a debug flag) is the analogue of
@PLN82 S3's bytecode gate.

**Plan shape:** the seam is one small plan; each subsystem swap is its
own follow-up plan (or `## Open work` row if it stays small).  None of
them is the whole arc — that is the point.

**Pacing discipline (the real constraint — user, 2026-05-31):** the plan
document is long-lived and that is fine; what matters is that **every
pass finishes fully — lands as a complete, reviewed, merged PR with CI
green — before the next pass begins.**  This is stronger than "one plan
at a time": no pass may start while a previous pass is half-done on a
branch.  One pass = one PR = `main` is releasable again.  A "pass" is a
single seam-conversion-of-one-subsystem, or a single subsystem's
representation swap — sized so it completes and merges as a unit.  The
dual-backing + equivalence assertion exist precisely so each such PR is
independently mergeable without the rest of the arc.  This same
finish-before-continue rule governs @PLN82's S1–S5 rungs.

## Migration step plan — native `Data` → store-backed reads (small steps)

Two distinct payoffs, sequenced **cheapest-first**.  Each step is green and
shippable on its own (one step = one PR, § Incremental migration pacing).

**G1 — cold-start cache (rebuild-on-load).** Parser + codegen stay native; on a
cache hit, `read_data` rebuilds the native `Data` from a mmap'd store (**12×
faster than `parse_dir`**, proven — § arc D probe).  No read-site migration
needed; this is mostly startup wiring and delivers the big user-visible win.

| Step | Deliverable | Validation | Effort |
|---|---|---|---|
| **D1** ✅ | `Data::save(path)` / `Data::open(path)` (thin wrappers over `ir_store::save_data` / `ir_read::open_data`).  Save materializes into a fresh file-backed store with the root at the well-known first record (`IR_ROOT_REC`=1, pos 8) so load needs no sidecar; open mmaps + `read_data`, returning `NotFound` on a missing file (clean cache-miss).  `scopes::check`-skip is deferred to **D2** (only matters once the loaded `Data` is compiled). | `data_save_open_round_trip_stdlib`: save→open→`compare_data` bit-for-bit + `NotFound` check | S — done |
| **D2a** ✅ | **Cache the database type schema** (chosen path 2; user 2026-06-02) so a loaded `Data`'s baked `known_type`s stay valid with no parse-time `fill_all` rebuild.  **(1) ✅** transcribe the database schema types into `ir.loft` (`DbType`/`DbParts`[17]/`DbField`/`DbContent` + `EnumPair`/`KeyField`; `Type.parents` derived/not stored) + a `Bundle { data, types }` root; regenerate `ir_schema_gen.rs`.  **(2) ✅** `ir_store::materialize_schema` writes `Stores.types` → `vector<DbType>` (guard-pinned consts; `Type`/`Field` read seam; `Record` float/single accessors).  **(3) ✅** `ir_read::read_schema` (inverse) + `Type::from_stored`/`Field::from_stored` constructors (`parents` empty — read only by parse-time layout validation + debug, never by codegen).  **(4) ✅** `ir_store::save_bundle` / `ir_read::open_bundle` via the `Bundle` root (`Data` inlined at 0, schema vector at `BUNDLE_TYPES`) + `Stores::install_schema`.  **(5) ✅** warm-path capstone green.  Done. | guard; `materialize_stdlib_schema_smoke`; `read_stdlib_schema_round_trips`; `bundle_save_open_round_trips_stdlib` (cold-parse vs warm-load: `compare_data` + schema-equality + install) | M–L — **done** |
| **D2b** ✅ | Startup wiring: `src/startup_cache.rs` (`warm_load_stdlib` / `save_stdlib_cache`) wired into `main.rs` around the `default/` parse, gated on `LOFT_STDLIB_CACHE` (default off → no-op).  A warm run `open_bundle_into`s the keyed `.store` bundle (mmap, schema installed) and skips parsing `default/`; a cold run parses then `save_bundle`s.  Keyed by `cache::stdlib_cache_key` (`stdlib_cache_path` now `.store`).  Validated end-to-end: `tests/d2b_stdlib_cache.rs` runs a real program off / cold / warm through the binary — byte-identical output, cold writes the bundle. | `stdlib_cache_off_cold_warm_match` (binary, 3-way) | M — **done** |
| **E1** ✅ | Whole-program bundle keyed for drift.  Robustness ✅ (atomic `save_bundle` + `open_bundle` `Store::is_store_file` pre-check → corrupt/partial/foreign bundle = clean cache miss).  **Whole-program key ✅** — sidestepped the lazy-auto-`use` boundary by caching the *entire* program (not core+libs): `Parser.parsed_sources` tracks every parsed file (gated on `track_sources`, zero cost off); `cache::program_cache_paths` keys the bundle on the script path; `startup_cache::{warm_load_program,save_program}` write/validate a **drift manifest** (`<hash> <path>` per source) so any input change (stdlib / lib / script) → reparse.  Wired into `main.rs` (`LOFT_PROGRAM_CACHE`, separate from D2b's `LOFT_STDLIB_CACHE`). | `corrupt_bundle_falls_back_to_parse`; `program_cache_cold_warm_then_drift` | M — **done** |
| **E2** | Mutability split (Q1): locked mmap bundle store + writable store for user-program defs; `caller_index` rebuilt on load (Q3). | full suite under cache-on | M |

**G2 — zero-copy store-backed reads.** Removes even the rebuild: codegen / exec
read store fields directly.  Larger; the self-hosting foundation.  Incremental,
behind the accessor seam, validated by a **dual-backing equivalence harness**
(read native AND store, assert equal) so every step is reversible.

| Step | Deliverable | Validation | Effort |
|---|---|---|---|
| **M0** ✅ | Equivalence harness landed — `ir_read::ir_roundtrip_check(&Data)` materialises into a store, reads back, and asserts bit-for-bit equality (`compare_data`); wired into the run path behind `LOFT_IR_CHECK` so it validates the store-mirror invariant on **any real program** (user code + lazily-loaded libs), not just the stdlib round-trip tests.  Additive; nothing switches yet.  *(Scope note: this is the whole-`Data` oracle — the strongest mirror check.  Per-accessor `DefView` dual-backing is folded into M2/M5, where a subsystem actually reads from the store and the swap is verified store-vs-native.)* | `ir_roundtrip_check_stdlib_ok` (lib) + `ir_check_passes_on_real_program` (integration, struct+fn+for) | M — **done** |
| **M1a** ✅ | `state/` `Definition` field-accessor seam (done). | suite green | — |
| **M1b** ✅ | `generation/` `Definition` field-read seam (done).  ~345 reads across all 14 `generation/*.rs` + `ops/*.rs` files routed through the `Definition` accessors; added `def_type()` (owned — a store read decodes the discriminant) + `rust()` to the seam.  Derived `const_ref` and the `tuple_group()` method stay direct.  Pure refactor, no behaviour change — native 6/6, codegen_emitter 19/19, full suite 2024/2024 green. | suite green | S — **done** |
| **M1c** ✅ | `parser/` + `compile.rs` `Definition` **read-site** seam (done).  ~430 reads across all `parser/*.rs` + `compile.rs` routed through the accessors — reads only: the parser builds Definitions by construction (`.def_mut(` used once, zero `def.FIELD =` writes), so immutable `.def(…)` access is safe to seam while writes stay direct.  Added five accessors for fields the parser reads that `state/`/`generation/` did not: `parent()`/`closure_record()` (u32), `mutated_captures()`/`scalars_to_box()` (`&[String]`), `synthetic()` (`Option<&'static str>`).  Derived `attr_names`/`const_ref` and the `original_name()`/`header()`/`is_operator()` methods stay direct.  No behaviour change — parse_errors 137, issues 684, expressions 127, imports 5, format 11, full suite green. | suite green | S–M — **done** |
| **M2** ✅ | **Persistent program store landed.**  Rather than a `DefView` wrapper, the simpler form proven: `byte_code_from` (interpreter) / `output_functions` (native) materialise the **whole `Data` into a persistent store once** (under `LOFT_CODEGEN_STORE`) and read each function's body node from it via `ir_read::def_body_node` (root → `definitions` → `def[d_nr]` → `code`), lowering through `IrNode::Store`.  Replaces M5's per-function re-materialise.  Proven: issues (684) + expressions (127) + native suite all green from the persistent store, byte-identical to native.  *The codegen side reads the IR from the store, not the native graph.* | suites green | **done (proven)** |
| **M3.0** ✅ | Node-walk handle landed (`src/ir_node.rs`).  **Key discovery:** the *store* half already exists — `data_store::Value` answers `value_type() -> ValueType` + has field accessors (`read_value` proves it).  So M3.0 built the **native mirror under one backing-agnostic `IrNode<'a>` enum** (`Native(&Value)` \| `Store(&Stores, Node)`): `kind()` (full native dispatch + store delegate), an accessor spread proving every category (scalar, string, child, child-list `IrNodeList`, compound `If`, `Span` passthrough via `unspan`), `native()` bridge for the M3.1→M4 recursion, and `IrType::kind()`.  Accessors fill **just-in-time** per M3.1 group (never dead).  Bedrock guard: a **cross-backing equivalence test** — `IrNode::Native` and `IrNode::Store` of the same node must agree on `kind()` + every accessor.  *(Chose an enum over a `trait NodeView` so codegen's deep `&mut State` recursion stays non-generic — rationale in the module doc.)* | `cross_backing_accessors_agree` / `cross_backing_unspan_agrees` / `native_kind_is_total` | S — **done** |
| **M3.1** ✅ | First codegen group converted: `generate_inner`'s **scalar/leaf arms** (13 — Int, Long, Single, Float, Boolean, Enum, Text, Var, Break, Continue, Null, Keys, Line) now dispatch on `node.kind()` and read payloads via `IrNode` accessors (native backing).  Pattern: a `match node.kind()` block whose arms `return`, ahead of the remaining `match val` (un-converted variants); the converted kinds are listed as `unreachable!` in `match val` so the compiler proves coverage.  Added accessors `single_value`/`float_value`/`bool_value` + extended the cross-backing oracle to cover them.  Behaviour identical — issues (684) / expressions (127) / wrap (50) / leak / threading all green. | suites green | S — **done** |
| **M3.2–M3.5** ✅ | `generate_inner` now dispatches **29 of its 34 arms** on `node.kind()`: M3.2 single-child/compound (If/Return/Drop/Set/BreakWith/Yield, via `as_native()` recursion bridge), M3.3 list-child + scalar (Insert/Call/CallRef/Tuple/Parallel/FnRefDnr, via `IrNodeList::as_native()`), M3.4 Span + Iter/ParFor (`span_pos`/`span_inner`), M3.5 FnRef (`fnref_dnr`/`fnref_clos_var`/`fnref_type`).  The accessor catalogue + cross-backing oracle grew with each.  **Residual `match val` (5 arms):** Block, Loop (need a `Block` handle — M3.6), TupleGet, TuplePut (long stack-offset bodies — M3.6), RawExpr (native-codegen-internal panic, stays).  Each group: clippy-0, issues/expressions/wrap green. | suites green per group | several S — **5 done** |
| **M3.6** ✅ | `generate_inner` **fully on `node.kind()`** — Block/Loop/TupleGet/TuplePut moved up (bodies re-bound from `as_native()`), RawExpr→panic arm, `Other(d)` guards a corrupt discriminant.  The second `match val` + its `unreachable!` coverage list are **deleted**; the function is one `kind()` match (its tail). | suites green | done |
| **M3.7** ✅ | **Handle-based recursion entry.**  `generate` keeps `&Value` (42 callers untouched) but wraps → new `generate_node(IrNode, …)`; `generate_inner` takes `IrNode`.  Insert/Tuple iterate the `IrNodeList` (`iter()`) + Span/Yield pass `IrNode` children to `generate_node` — those 4 bridges gone.  M5's entry just calls `generate_node(IrNode::Store(…))`. | suites green | done |
| **M3.8** ✅ | `gen_return` / `gen_drop` take `IrNode` (recurse via `generate_node`); Return/Drop/BreakWith bridges dropped.  (`gen_drop` keeps one `as_native()` for `size_code`.) | suites green | done |
| **M3.9–M3.14** ✅ | Helper conversions — `gen_if` (M3.9, with `==Null`→`kind()`), `generate_call_ref`+`gen_parallel`→`IrNodeList` (M3.10), `IrBlock` handle + `generate_block`/`gen_loop` (M3.11), `TupleGet`/`TuplePut` scalar accessors (M3.12), `is_divergent`→`IrNode` (M3.13), `Stack::size_code`→`IrNode` (M3.14).  **Every codegen helper reachable from `generate_inner` is now on the handle except the two native-`Value`-clone clusters** (below).  Remaining `as_native()` bridges in `generate_inner`: `Set` (`generate_set`), `Call` (`generate_call`), `TuplePut`'s value child. | suites green per group | done |
| **M3.15** ✅ | The two native-`Value`-clone clusters resolved via **materialise-at-boundary**: a new `IrNode::to_owned_value()` (native clones / store `read_value`s, works on both backings) lets `generate_set` and `generate_call` take `IrNode`/`IrNodeList`, materialise their assigned-value / param-list to native once at entry, and run their intricate ownership / `gather_key` bodies **unchanged** (a compile-time clone, no risk to the hazardous logic).  TuplePut's value child (M3.15a) also converted.  **`generate_inner` and every helper it reaches now read the IR exclusively through the `IrNode`/`IrNodeList`/`IrBlock` handles — it is fully store-capable.** | suites green | M — **done** |

**Interpreter-codegen lowering (`generate_inner`) is fully store-capable** (M3.0–M3.15, 2026-06-03).  Reaching an actual store-backed run (M5) now needs **M2** — `data.def(d).code()` returning an `IrNode::Store` body (the `DefView` work) — to construct the store-backed entry, plus the codegen entry points *outside* the `generate_inner` tree (`gen_fn_ref_value`, `def_code`).  Then M4 mirrors all of this for the `src/generation/` native backend.
| **M4** 🔄 | Same handle conversion for `src/generation/` (native codegen, 8010 lines / ~727 `Value`/`Type` matches across 7 files).  **Started + pattern proven:** `output_code_inner` (emit.rs — the native `generate_inner`) converted M4.1 (9 constant leaves) + M4.2 (Line/Break/Continue scalars) on `node.kind()`, reusing the *exact same* `IrNode`/`ValueType` accessors — no new infra.  Native suite (full rustc compile+run) green per group.  **`output_code_node` + `output_block` fully store-capable (M4.1–M4.8):** all leaf/scalar/recursive arms on `node.kind()`; the large `&Value`/`&Block` helper cluster (output_if/output_set/output_call/output_block-body) sits below a **materialise-at-boundary** (zero-cost for native, materialise-once for store).  This was sufficient to land **M5-native** (the body flip, proven).  *Remaining (zero-copy only, → M6):* converting those helper bodies + the 6 helper files (`dispatch.rs`/`calls.rs`/`coroutine.rs`/`pre_eval.rs`/`text.rs`/`mod.rs` walkers) to the handle so store-backed bodies don't materialise. | native suite green per group | M5-relevant parts **done** |
| **M5 (interpreter)** ✅ | **The backing flip — proven.**  `def_code` gains an env-gated (`LOFT_CODEGEN_STORE`) store-backed path: it materialises each function body into a store and lowers it via `IrNode::Store`; `generate_inner` reads the body from the store, the Definition table from the native `Data` (hybrid).  With the flip on, the **entire program compiles store-backed** with **byte-identical output to native** — verified across all of `tests/scripts/*.loft` (0 real divergences) + `tests/g2_m5_codegen_store.rs` (recursion / struct / for / interpolation / tuple get-put / indexing).  The per-function re-materialise is the proof harness; M6 has the bodies already in the store.  *Coverage is complete:* the only store reader is `generate_inner` + its handle helpers; every path that needs a native `Value` (the `generate_set` / `generate_call` clusters, and `gen_fn_ref_value` downstream of them) sits **below a materialise-at-boundary**, so it always receives native input — store-safe by construction.  Full suite green (2010/2013; the 3 `native_*` are the unrelated stale-rlib issue). | M5 equivalence test green | **done (proven)** |
| **M5 (native backend)** ✅ | **The native flip — proven.**  `output_code_node` (M4.1–M4.7) + `output_block` (M4.8) are store-capable (handle arms + materialise-at-boundary for the large `&Value`/`&Block` helper cluster); `output_function` gains the `LOFT_CODEGEN_STORE` store-backed body path (`IrBlock::Store`).  Verified: `LOFT_CODEGEN_STORE=1 cargo test --test native` (all 6 scripts, full rustc compile+run) green + `m5_test.loft --native` byte-identical store-vs-native.  **Both backends now run store-backed and proven equivalent.** | native suite green | **done (proven)** |
| **M6** 🔄 | Drop the native body graph; reads become **zero-copy**.  **Codegen pipeline fully store-backed** (M6.1–M6.3: `extract_literal_values` / `build_const_vectors` / `def_code` checks all read from the store).  **M6-warm SHIPPED (opt-in):** `LOFT_PROGRAM_CACHE`+`LOFT_CODEGEN_STORE` warm hit does a **skeleton load** (`open_program_store` / `read_data_skeleton` — def table only) and codegen reads bodies straight from the mmap'd bundle via `def_body_node`, skipping `read_data`'s body rebuild.  The warm path is **NOT E2-gated** (`scopes` skipped on a hit; bundle is post-`scopes`).  Measured ~0.4 ms / ~5% end-to-end on top of the cache's 3–3.6× parse win (modest: variable tables, not bodies, dominate `read_data`).  Proven byte-identical (`tests/g2_m6_warm_store.rs`).  **Remaining:** the bigger win needs the *variable tables* also store-backed + the **cold** path's physical native-graph drop, which is **E2-gated** (`scopes` *rewrites* bodies → needs a writable store). | suite green; bench | M — **warm path shipped** |
| **M7** | (Optional, self-hosting) parser emits store-backed IR directly — removes the post-parse materialize. | compare_data vs golden | L |

**Sequencing note:** G1 ships the speed win without touching read sites, so do
it first (or in parallel) — it is independently valuable and de-risks G2 by
exercising the store on the real startup path.  G2's M0 harness is the
prerequisite for every swap; M3/M4 (the `Value`/`Type` walk, ~451+ matches) is
the dominant cost and is deliberately the most finely sliced.

> ⚠ **D2 blocker (confirmed 2026-06-02 — the @PLN82 wall) — schema rebuild
> from a loaded `Data`.**  `Data::open` rebuilds the native `Data` (proven, 12×),
> but a program needs the **database type schema** (`Stores.types`, the record
> layouts indexed by `known_type`) too.  That schema is built **during parse**
> by `typedef::fill_all` → `fill_database`, which *assigns* each `known_type`
> incrementally (`s_type = database.structure(name); def.known_type = s_type`) —
> in lockstep with declaration order growing the table.  A cache-loaded `Data`
> already has **absolute** `known_type`s baked in, so re-running `fill_all` on it
> forward-references a type not yet registered in the fresh database and panics
> (`database/types.rs:96`, "index out of bounds: len 24 index 24").  Two
> resolution paths, to design in **D2a**:
> 1. **Load-aware schema rebuild** — register every type *shell* first (so all
>    `known_type` slots exist), then fill fields; or register each type *at* its
>    baked index.  Smallest if `fill_database` can be split shell/field.
> 2. **Cache the schema too** — persist `Stores.types` (native
>    `Vec<database::Type>`) alongside the IR and reload it, so no rebuild and the
>    baked `known_type`s stay valid.  Cleaner long-term (matches "everything is
>    store records"), but a separate serialization effort (the schema `Type`/
>    `Parts` graph, like the IR).
>
> Landed toward D2 meanwhile: the `done=true` prerequisite (`from_snapshot`), so
> a load-path `scopes::check` correctly skips loaded functions.  The warm-path
> proof test is drafted but red until D2a lands.

## Open design questions

1. **Mutability.**  The parser mutates `Data` heavily during two-pass
   parsing; a mmap'd store is `locked`.  Likely resolution: parse into a
   writable store (or native `Data`), freeze + persist; at runtime read
   the locked mmap'd bundle store + a writable store for any
   user-program defs.  Mirrors the CONST_STORE locked-after-build pattern
   (`src/compile.rs:52`).
2. **`&'static str` / interned labels.**  `Block.name`,
   `Definition.synthetic` are `&'static str`.  In a store they become
   `Str`/record offsets; native `match name { "if" => }` sites need a
   store-string comparison path (or an interned-id scheme).
3. **`OnceLock` caller-index.**  `Data.caller_index` is a derived cache —
   rebuilt on load, never stored.
4. **Bundle drift detection.**  A mmap'd bundle must be rejected if the
   inputs changed — reuse + extend the @PLN82 `stdlib_cache_key`
   (version + build-id + feature set) with the sorted lib list and lib
   content hashes.
5. **Store-read vs `Vec`-index perf — cost AND a locality upside (user,
   2026-06-01).**  `data.def()` is a `Vec` index today (~940 sites, hot); a
   `DbRef` read adds a store indirection — measure the hot-path delta and
   whether a read-through cache is needed.  **First data point (2026-06-02,
   `bench_stdlib_load_mmap_vs_parse`):** *loading* the whole stdlib via
   `Store::open` + `read_data` (rebuild native) is **~12× faster** than
   `parse_dir` (0.92 ms vs 11.4 ms median, warm cache) — so even the
   rebuild-on-load path is a large net win, and the store layout is decisively
   "good enough to build on."  This measures the *load* path, not yet the
   per-`data.def()` hot-read delta (that is what the per-subsystem swap will
   measure).  **But the store layout is not purely a cost.**  The native IR is a graph of separately-heap-allocated
   nodes — `Box<Value>`, `Vec<Definition>`, `String`, nested `Box<Type>` —
   scattered across the allocator, so walking a definition's `code` chases
   pointers into cold cache lines.  The store packs a record's fields (and,
   with co-located `ChildRec`/inline layouts, its sub-records) **contiguously
   in one rigorously-laid-out buffer**.  That tight, sequential layout is
   exactly what caches and prefetchers reward: walking an IR node in the
   store can touch far fewer cache lines than chasing the equivalent `Box`
   graph.  Rust's per-node layout is locally optimal but globally scattered;
   the store trades a small per-access indirection for **whole-IR locality**.

   So the honest hypothesis is a *trade*, not a strict regression: indirection
   cost vs. locality/prefetch win, and the balance is empirical.  It may even
   come out **net-positive on the hot walk** before mmap is considered —
   making "Data-as-store" a structural optimisation in its own right, not only
   the enabler for zero-copy load.  This is the second thing arc C's
   per-subsystem equivalence/bench harness must measure (alongside
   correctness): not just "is store-read fast enough?" but "is the packed
   layout actually faster to traverse?"  Treat the locality win as a
   hypothesis to confirm with numbers, not a given.

   **Risk-posture consequence (user, 2026-06-01) — why "slow IR" is not a
   thing to fear.**  The locality argument is **not a clear win**; it may net
   out slower.  Its real value is as a *floor on the downside*: the store is a
   rigorous, contiguous, cache-coherent layout, so even in the worst case the
   store-backed IR is **a reasonable representation, not a pathological one**.
   That is what makes it safe to commit to the store representation directly,
   rather than treating "land in the store" as risky "slow IR" territory to be
   avoided.  Combined with the migration safety net (the accessor seam +
   dual-backing + per-subsystem equivalence assertion, § Incremental
   migration), the perf question stops being a gate on *whether* to migrate
   and becomes a tuning question *after*: if a hot subsystem measures slower,
   add a read-through cache or keep that subsystem native — the dual-backing
   makes either reversible.  So the design proceeds on "the store layout is
   good enough to build on," with the locality upside as a possible bonus, not
   a load-bearing assumption.

## Cross-arc dependencies

- **[@PLN82 startup-cache](../82-const-store/STARTUP_CACHE_PLAN.md)**
  — direct predecessor.  @PLN82's rebuild-on-load snapshot delivers the
  cold-start win first; this plan removes the rebuild.  @PLN82 shipped a
  **JSON** snapshot (native-side), **not** the store struct-enum format — so
  arc B is greenfield, not "mostly done"; what carries over is the JSON codec's
  IR walk (as arc B's skeleton) and `compare_data` (as its oracle), both interim
  (§ Standalone upside decision).  serde is forbidden project-wide (CODE.md) —
  this plan uses the store format, consistent with that.
- **[@PLN43 loft-store-durable](../43-loft-store-durable)** — shares the
  `Store::open_durable` / persistence surface; coordinate the on-disk
  store format so the IR store and durable user stores stay compatible.
- **NATIVE.md / `src/generation/`** — source of the struct-enum
  schema-emission pattern this plan mirrors; arc A reuses `output_init`'s
  registration approach.

## Relationship to self-hosting (loft compiler in loft)

A full loft-in-loft rewrite — parser, type checker, scope analysis, and
codegen written in loft, running on the interpreter, fast enough to
compile itself — is anticipated but is **a 2.0-scale undertaking, far on
the horizon** (not a 1.0 goal).  This plan is **not an alternative to it;
it is a strict down-payment on it**, chosen because it is the smallest
reversible slice of the same problem and is valuable on its own merits
(cold-start) long before the rewrite is on the table.

**Shared hard problem.**  Self-hosting must represent the compiler's IR
(`Data` / `Value` / `Type` / `Definition`) as loft's own data — there is
no way to write a loft compiler in loft without it.  This plan answers
exactly that question for the data model alone, in the
already-`--native`-validated store format.  Whatever schema this plan
pins (arc A) is the schema a self-hosted front-end would consume.

**Reversibility ladder.**  Each rung's non-throwaway work feeds the
next; enter self-hosting through this keyhole, not head-on:

| Rung | Effort | Permanent contribution to self-hosting |
|---|---|---|
| @PLN82 JSON stop-gap | days | proves loft data *can* hold the IR; ships the whole-bundle cold-start win (per-library JSON considered + deferred as too brittle) |
| **@PLN11 (this)** | L | the store-backed IR schema + read accessors — the first *permanent* self-hosting foundation |
| full loft-in-loft | multi-quarter | the destination |

**This plan removes a self-hosting blocker.**  Self-hosting makes the
interpreter's parse-bound cold-start *worse* (a loft compiler is a
compile-heavy workload on the interpreter).  The startup cache + this
plan attack precisely that bottleneck, so they clear the runway rather
than compete for it.

**Gate, not commitment — and a 2.0 horizon.**  Full self-hosting is a
**2.0-scale** target, deliberately past 1.0.  Two gates keep it there:
(1) language maturity — writing a large compiler in loft before the
syntax settles means writing it twice, so it waits until the 1.x line is
stable; (2) this plan must first prove the IR-in-loft-data model is
**both ergonomic to express and fast enough to read** (open questions 2
and 5).  If `Data`-as-store turns out pleasant and the hot-path read
delta acceptable, self-hosting is materially de-risked; if painful, that
lesson is learned here cheaply, on the smallest slice, before betting a
2.0-scale arc on the rewrite.  Nothing in this plan *commits* to the
rewrite — it only makes the eventual decision cheaper and better-informed.

## See also

- [NATIVE.md](../../NATIVE.md) — how `--native` represents data as
  `Stores` records (the model this plan mirrors); § Architecture,
  § `output_init`.
- [DATABASE.md](../../DATABASE.md) — `Stores`, `Store`, `DbRef`,
  word-addressed records, CONST_STORE.
- [@PLN82 STARTUP_CACHE_PLAN.md](../82-const-store/STARTUP_CACHE_PLAN.md)
  — the cold-start cache; its "Architecture C — Data *is* the store" is
  this plan's seed.
- `src/store.rs::Store::open` — the mmap entry point this plan loads
  through.
- `src/data.rs` — the IR types (`Data`, `Value`, `Type`, `Definition`)
  being migrated.
