<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_SWEEP.md — the brittleness hunt work list

> Open items from this catalog are ORDERED in
> [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) — the single tracking view
> across all stability docs.  This file stays the canonical home for the
> findings themselves.

A systematic sweep of the whole body of code for **invariants implemented in
more than one way** — the disease class behind #313 (layout decided in three
places), #314 (capture shape frozen before the decision was final), #323
(ownership transferred on one path, freed on another), and #328 (pointer-ness
erased between parse and layout).  The method is pass 1 of
[STABILITY_METHOD.md](STABILITY_METHOD.md) (sweep → move algorithms to their
data structures → de-duplicate), using the
[engineering-rigor](../../.claude/skills/engineering-rigor/SKILL.md)
instruments for *breakage discovery*:

1. **Find the fact with two homes.**  Grep/read the module for a fact that is
   derived, cached, or re-asserted in a second place (a flag AND a layout; a
   parse-time decision AND a codegen-time re-derivation; an interp op AND its
   native twin; a sentinel AND a zero-default).
2. **Write the probe that makes the homes disagree.**  Throwaway `/tmp`
   probes on `--interpret` first; distinctive collision-resistant values;
   vary ONE axis per probe (the composition axes: construction-path ×
   context × depth × null × cardinality × ordering × backend).
3. **Document — do not fix (sweep rule, user call 2026-06-11).**  For every
   finding, write down in the Findings log: the ONE invariant (a single
   sentence naming the fact and its single rightful home), every home where
   the code re-asserts it today, the probe that shows the homes disagreeing,
   and the observed damage.  Breaks become GitHub issues with the matrix and
   a `/tmp/p_followups/` repro; demonstrating tests land as
   `#[ignore = "stability-sweep: #NNN"]` so the suite stays green.  Fixes are
   a SEPARATE later pass — the sweep's output is the documented invariant
   catalog, not patches (fixing mid-sweep would re-shuffle the very ground
   being surveyed).  **The later pass's shape (user, 2026-06-11): move each
   algorithm toward the data structure it runs on — so the fact and the
   logic that asserts it share one home — and THEN remove the duplications.**
   The catalog should therefore name, per finding, which data structure is
   the invariant's natural home (where the algorithm should eventually live).
4. **A probe that holds is also a result.**  Record it in the row — "probed,
   held" tells the next session what was covered.

## Status legend

`☐ todo` · `▶ in progress` · `✅ swept (findings linked)` · `➖ low-risk
(thin/leaf module, sweep last)`

## Cross-cutting invariant families (highest yield — sweep these THROUGH the modules)

These are single facts asserted in N places across module boundaries.  Each
gets probed wherever its homes can drift apart.

| # | Family | The fact | Known homes | Status / findings |
|---|---|---|---|---|
| F1 | **Two-pass parse determinism** | pass 1 and pass 2 must derive identical facts (def numbering, lambda numbering, attr types, flags) | every `first_pass` branch (≈40 sites in `parser/*`); `data.reset()` keeps definitions but pass 2 re-parses bodies | ▶ lib-closure-factory cell held (`f1.loft`); #313/#314 historical instances fixed; DEFERRED: diagnostics-altering-flow and todo_files-reorder cells |
| F2 | **Ownership of a store** | who frees a store, exactly once | `scopes.rs get_free_vars` (dep-empty = owned, captured-ref exemption, work-refs, in_ret); `scan_set owned_refs` (#316); `state/codegen.rs generate_set` pre-Set free + S1 guard; `check_ref_leaks` (debug assert); `free_named` cascade (`__closure_*` only); `paired_witness`/`witness_buffer` OpFreeRefIfDistinct | ▶ **#330** (see log); `value_reads_var` default arm misses read-bearing variants (`FnRef(_, w, _)` reads w — audit the ~30 default-arm variants when pass 2 centralises the predicate); par cell held (`fpar.loft`); witness-pair cells DEFERRED |
| F3 | **Null representation of a DbRef** | what bytes mean "null pointer" | `OpNullRefSentinel` (store_nr = u16::MAX); zero-default record bytes (store 0, rec 0); `free_named`'s skip pattern (store 0 ∧ rec 0); `OpEqRef` comparisons | ✅ probed, HELD (see log) — comparator normalises; dual encoding persists as latent risk for byte-level consumers |
| F4 | **Interp op ≡ native op** | every opcode's semantics on both backends | `fill.rs` (interp) vs `generation/ops/*` + `codegen_runtime.rs` (native); `cross_mode` harness covers a sample, not the op set | ▶ first finding: **#333** (float `/0.0`: interp aborts, native nulls); int `/0` `%0`, neg div/mod, float assoc AGREE (`f4.loft`); DEFERRED: the op-coverage sentinel (enumerate ops lacking cross_mode cells) |
| F5 | **Stack slot layout** | a value's slot size/alignment as parser, codegen, and runtime each compute it | `variables/mod.rs` size table (fn-ref = 20B …); `state/codegen.rs stack_step`/`size_*`; `state/mod.rs` runtime reads; LOFT_ALIGN duality | ▶ LOFT_ALIGN=1 sample held on DbRef-heavy program; tuple struct-field round-trip held (`ftup.loft`); DEFERRED: full odd-size adjacency matrix |
| F6 | **DB field layout** | a struct field's position/width as the parser reads it vs fill_database wrote it | `parser/mod.rs get_val/set_field_check` width selection (alias forced_size, byte_width, vector_narrow_width); `typedef.rs fill_database` arms; `database/types.rs finish_type` packing; native `generation/mod.rs emit_field` re-derivation | ▶ finding: **#332** (nullable narrow null doesn't round-trip); HELD: u8/i16/i32/limit field values, narrow vectors u8/i16/u16, enum+bool packing, tuple fields, hash insert/update — both backends (`f6a–c`, `ftup`, `fhash`) |
| F7 | **Value vs pointer copy semantics** | what `a = b` / field-assign / capture copies | LOFT.md:570 says struct var assign copies the DbRef; codegen `gen_set_first_ref_var_copy` deep-copies; `OpCopyRecord` field copies; `reference<T>` repoints (#328); captures share by DbRef (P260) | ✅ probed, BROKEN at the spec home: **#331** (see log) |
| F8 | **Deps as liveness vs deps as ownership vs deps as markers** | what `Type::*(…, deps)` means | borrow liveness (scopes); ownership negation (dep-empty = owned); the `u16::MAX` share marker (#328/closure records); `@P302` self-dep = ownership marker for keyed locals; attribute-index deps in returned types (`dep_has_var` resolves attr-index vs var-nr with a FALLBACK comparing raw numbers) | ▶ view-dep basic cell held (`f8a.loft`); #328's self-dep strip is a documented instance of meaning-collision; DEFERRED: crafted attr-index/var-nr collision (needs IR-level var numbering control) — pass-2 should split the four meanings into distinct fields |
| F9 | **The startup/stdlib caches vs live state** | cached parse must equal fresh parse | `startup_cache.rs` manifest (sources, sig); `cache.rs` keys; `ir_store`/`ir_read` round-trip; REPL `rollback_to` | ▶ first cells probed, HELD: #328 marker-dep types and #313 split fn-fields round-trip warm≡cold (`/tmp/claude/sweep/f9*.loft`, 2026-06-11); REPL rollback + lib-path axes todo |
| F10 | **Text ownership** | who frees a String buffer | `free_text` / `OpFreeText`; work-text result buffers; `skip_free` texts; text-returning-fn cell exemption (02d-vii) | ▶ struct-field text churn held leak-free (`f10.loft`); plan-53 historical; DEFERRED: par text buffers, text in returned structs |
| F11 | **Error-path state** | a fallible operation either completes its state transition or leaves observable state unchanged — no half-transition survives the error exit | every mutate-then-fallible pair: file/connection tables vs the open/upgrade that validates them; allocate-then-fail in store paths; cache/manifest writes vs the validation that blesses them; registration tables (defs, todo_files, flip table, breakpoints) vs the resolve/load that makes the entry real | ✅ swept + ALL FOUR FIXED (2026-06-12): **F11-1..4** (see log; each entry carries its fix site + regression) — interp read-path dangling file ref, `files()` listing truncation, reenter completion clearing the standing `parallel_ctx`, driver-mode native path anchor. Six agent claims refuted (held). Archetype: `database/io.rs:664` write_file |
| F12 | **Vector element nullability** | is a `vector<S>` element nullable (`__nullable<S>`) or dense (`S`) — and is that choice a *carried fact* or *re-derived per site*? | DECIDED at parse by `e2_nullable_elem` (definitions.rs `sub_type` vector arm), which depends on whether `S` is defined yet (order-sensitive); a FORWARD-ref element resolves later in `copy_unknown_fields` (typedef.rs) to raw `Reference(S)` *without* the rewrite; literal/local synth at `vectors.rs:1409/1701`; the synth-enum layout-ordering hack (`typedef.rs:291`); ~188 `__nullable<` references across `parser/*` + `typedef.rs`. The `not null` opt-out is **unrecoverable** once the type collapses to dense `Reference(S)` — so dense-by-`not null` and dense-by-forward-ref-miss are indistinguishable downstream. | ✅ **FIXED 2026-06-21 — loft#417** (@PLN25 / dogfood: unblocked `cbor` ZT-A). A forward-ref element (`enum E { V { f: vector<S> } }` with `struct S` defined AFTER `E`) missed the parse rewrite → the field stayed raw `vector<S>` while the SAME `vector<S>` as a param/local/literal WAS rewritten to `vector<__nullable<S>>` → element-stride mismatch → corrupted enum-discriminant reads on BOTH backends (cbor map encode printed `162 1 160 4`, want `162 1 2 3 4`). **Fix (landed):** the nullable-element decision now lives in ONE home — `typedef::nullable_vector_elem` — called by BOTH the parse chokepoint (`e2_nullable_elem`) and the deferred forward-ref resolver (`copy_unknown_fields`), so a `vector<S>` element resolves identically regardless of definition order; the two resolvers can no longer disagree. (The fuller evolution — carry an explicit element-nullable bit that also records `not null`, collapsing all ~188 sites to reads — remains the eventual pass-2 home; the single shared decision already retires the corruption class.) Regression: `loft-suite/attic/scripts/85-store-lifetime-forward-ref-nullable-vector.loft` (both backends, leak-free). **Probe:** `ef.loft` (enum-first) vs `sf.loft` (struct-first), single-variable on definition order — now agree. The structure that was right for plain vectors became the burden when nullable-by-default landed — see [STABILITY_METHOD.md § trigger](STABILITY_METHOD.md). |

## Module work list (every file; sweep top-down by risk)

| Module | Lines | Suspected dual-invariants to hunt | Status |
|---|---|---|---|
| `src/parser/mod.rs` | 6863 | F1, F6, F8; `fn_ref_field_is_split` single home verified (#313); `base_var_of` vs other lvalue walks | ▶ family-covered (#332 cells); base_var_of unification → pass 2 |
| `src/scopes.rs` | 4969 | F2, F8; `value_reads_var` default-arm gap NOTED (FnRef reads w) | ▶ #330; predicate centralisation → pass 2 |
| `src/state/codegen.rs` | 3745 | F2, F5; the S1 top-level-arg predicate IS #330's broken home | ✅ swept → #330 |
| `src/state/mod.rs` | 4616 | F5; `fn_call_ref` 20B slot | ✅ covered by closure/mut_closure matrices (both backends, 44 cells) | |
| `src/parser/control.rs` | 4823 | F1; match/if result-type unification vs codegen's branch layout | ▶ enum/tuple cells held (`f6c`, `ftup`); match-unification DEFERRED |
| `src/parser/vectors.rs` | 4424 | F1 lambda-epilogue asymmetry (#313/#314 fixed); factory cell held (`f1.loft`) | ✅ swept via #313/#314/#323 arcs |
| `src/parser/collections.rs` | 4186 | `towards_set` dual discriminators (shape vs type) — documented in #328 fix comments | ▶ keyed insert/update held (`fhash`); discriminator unification → pass 2 |
| `src/data.rs` | 4304 | `def_nr` convention sentinel grep: only system/type names lack prefixes — convention holds | ✅ swept (static sentinel) |
| `src/database/allocation.rs` | 2061 | F2, F3; free-bitmap vs `store.free` (@P317 tripwire) | ▶ F3 held; bitmap/flag disagreement cells DEFERRED (tripwire already guards) |
| `src/database/types.rs` | 2929 | F6 | ▶ width/packing cells held (#332 is the narrow-null exception); P191 late-mutation DEFERRED (validator exists) |
| `src/store.rs` | 2788 | F3 held; LLRB vs `needs_coalesce`, `claims` vs headers | ▶ DEFERRED (needs store-level fuzz harness — pass-2 candidate instrument) |
| `src/typedef.rs` | ~800 | `has_value_cycle` skip ≡ fill arms — aligned in #328 | ✅ swept |
| `src/fill.rs` | 2168 | F4 | ▶ #333 (float div); arg-width audit → the F4 op sentinel (DEFERRED) |
| `src/generation/mod.rs` | 3142 | F4, F6; emit_field re-derivation (fn-fields single-homed in #313) | ▶ narrow/enum/tuple/hash native cells held; remaining Type arms DEFERRED |
| `src/generation/emit.rs` | 1768 | F4 | ▶ see F4 row (#333; sentinel deferred) |
| `src/generation/ops/*` | ~3000 | F4; post-plan-57 rc remnants | ▶ DEFERRED to pass 3 (deletion candidates by usage sentinel) |
| `src/codegen_runtime.rs` | 4118 | F4, F5 | ▶ covered by cross-backend probes this sweep; helper-duplication inventory → pass 2 |
| `src/main.rs` | 5803 | F9 CLI dispatch | ➖ exercised by every suite mode; no dual-invariant suspect found |
| `src/cache.rs` + `src/startup_cache.rs` | ~900 | F9; manifest vs inputs (#322 fixed) | ▶ warm≡cold held for new shapes; lib-PATH/env axes DEFERRED |
| `src/ir_schema*.rs` + `ir_store.rs` + `ir_read.rs` + `data_store.rs` | ~7000 | F9 codec vs struct | ▶ marker-dep + split-field shapes round-trip held (f9/f9b); field-by-field codec audit DEFERRED (pass-2: derive codec from one schema) |
| `src/parser/expressions.rs` | 2697 | F7; `change_var` merges | ✅ swept: `x = x` corrupts both backends, divergently — logged on #330 (the predicate hole's degenerate cell) |
| `src/parser/operators.rs` | 2173 | `call_to_set_op` GET→SET table vs get_val table | ▶ assignment shapes this sweep all held; table-completeness sentinel DEFERRED |
| `src/parser/objects.rs` | 2058 | construction vs post-hoc assignment parity | ✅ swept: text/narrow/enum/tuple/hash/reference cells held (#328 fixed the reference cell) |
| `src/parser/definitions.rs` | 2236 | F1; sub_type single funnel verified (#318 R3, #328 marker) | ✅ swept |
| `src/parser/fields.rs` + `builtins.rs` | ~1500 | field-access dispatch | ▶ chained/nested reads held across #328 probes; iterator-op dispatch DEFERRED |
| `src/variables/mod.rs` | 1704 | F5 size table vs Type variants | ▶ exercised broadly; add-a-variant drift is a pass-2 chokepoint candidate (exhaustive match) |
| `src/parallel.rs` + `src/state/io.rs` | ~3500 | F2 across threads | ▶ par result cell held (`fpar`, both backends); dispenser/swap-back stress DEFERRED |
| `src/extensions.rs` + `src/native_lib.rs` | ~3000 | #303 marshallability single home | ✅ verified by cdylib suites each run |
| `src/lexer.rs` | 1759 | position/link invariants under `revert` | ➖ low-risk |
| `src/repl.rs` | 3141 | F9 `rollback_to` | ▶ error-recovery cell held; counter/caches-after-rollback cells DEFERRED |
| `src/state/text.rs` + `debug.rs` | ~2500 | F10; dumper deref-safety | ✅ re-probed post-#328: dumper clean on reference-field + walk programs (the crash was the fixed frame corruption) |
| `src/database/format.rs` + `journal.rs` | ~3000 | F9 persistence | ▶ covered by store_durable/persist suites; new-shape persistence cells DEFERRED |
| `src/keys.rs`, `src/log_config.rs`, misc leaf files | ~2000 | — | ➖ low-risk |
| `default/01_code.loft` (+02,03…) | ~3000 | `#rust` twin bodies | ▶ #333 IS this dual for float-div; systematic twin-audit = the F4 sentinel (DEFERRED) |
| `tools/indexer`, `lib/*` consumers | — | dogfood surface; run, don't sweep | ➖ low-risk |

## Findings log

### F2-1 — #330: self-reading struct-literal reassignment reads the recycled store (2026-06-11)

- **Invariant**: a reassignment's old store must outlive every read the RHS
  performs; equivalently, exactly one predicate decides "does the RHS read
  `v`" for every free site.
- **Homes today**: `state/codegen.rs generate_set` pre-Set `OpFreeRef` with
  the S1 guard (top-level Call args only; struct-literal RHS explicitly
  bails toward freeing) vs `scopes.rs value_reads_var` (recursive, used only
  by the #316 transition free).
- **Natural home**: ONE RHS-reads-v predicate owned by the IR (`Value`),
  consumed by both free sites; longer term the free belongs to the store
  allocator's reassignment path, not codegen.
- **Probe / damage**: `x = S { v: x.v + 1 }` → `x.v == null` through a
  `not null` field, BOTH backends (`/tmp/p_followups/f2b.loft`, `f2c.loft`);
  degenerate `x = x` corrupts divergently (interp reads the recycled slot,
  native nulls — `fself.loft`).  Nested ref-param call shape held; #328's
  borrow shape held (deps gate).
- **Artifacts**: issue #330; `tests/issues.rs::issue_330_self_reading_literal_reassignment`
  (`#[ignore]`, fails-as-demonstration when forced).

### F7-1 — #331: LOFT.md:570 alias claim vs codegen deep copy (2026-06-11)

- **Invariant**: `a = b` for struct vars has ONE defined meaning, asserted
  identically by the spec and the implementation.
- **Homes today**: LOFT.md:570 ("the DbRef is copied — both point to the
  same record") vs `gen_set_first_ref_var_copy` (deep copy, relied on by
  #316's ownership tracking and C38's capture rationale).
- **Natural home**: the spec section + one codegen arm; which semantics wins
  is a pass-2 design decision (alias would need the lifetime model to carry
  it; deep-copy needs a sanctioned borrow idiom for walks).
- **Probe / damage**: `b.v = 42` after `a = b` → `a.v == 1` (probe
  `/tmp/p_followups/f7.loft`); doc misleads; the #316 walk-leak residual is
  a downstream cost of the copy.
- **Artifacts**: issue #331.

### F6-1 — #332: nullable narrow field null never round-trips (2026-06-11)

- **Invariant**: a nullable field's null state survives write→read→compare,
  whatever the storage width.
- **Homes today**: narrow-width selection in `get_val`/`set_field_check`
  (Parts::Short legacy `+1` vs ShortRaw vs Int sentinels) vs the null write
  encoding vs the `== null` decode — the omitted-field zero default matches
  none of them.
- **Natural home**: the `Parts` variant — each storage width owns its null
  sentinel and BOTH the write and the compare ask it.
- **Probe / damage**: `a: i16` omitted → `== null` false; `a = null` →
  still false; values round-trip fine.  Both backends
  (`/tmp/p_followups/f6d.loft`).
- **Artifacts**: issue #332; ignored test
  `issue_332_nullable_narrow_field_null_roundtrip`.

### F4-1 — #333: float ÷0 — interp aborts, native nulls (2026-06-11)

- **Invariant**: one semantics per operator edge case, asserted by the lint
  text, the interp body, and the native body alike (null per @C66/@C67).
- **Homes today**: `fill.rs` float-div (traps) vs the generated native body
  (nulls) vs the lint's own promise ("may produce null").
- **Natural home**: the op's single definition — `default/01_code.loft`'s
  `#rust` template and the interp body must be one artifact (the F4 family's
  general cure).
- **Probe / damage**: `1.0 / 0.0` halts interp with exit 1; native prints
  `null` and continues (`/tmp/p_followups/f4.loft`).  Integer edges agree.
- **Artifacts**: issue #333; ignored test
  `issue_333_float_div_by_zero_yields_null`.

### Held-cells log (2026-06-11 sweep day 1)

`f6a/b/c` narrow+enum+limit fields and vectors (both backends) · `f6d`
value cells · `ftup` tuple field repoint+destructure (both) · `fhash` keyed
insert/update (both) · `fpar` par loop (both) · `f8a` view dep · `f1` lib
closure factory · `f10` text-field churn leak-free · `f9/f9b` program-cache
warm≡cold for #328/#313 shapes · LOFT_ALIGN=1 sample · REPL error rollback ·
trace dumper on reference-field programs.

### F5/260-1 — native declaration point derived from IR init position (#260; pass-2 entry)

- **Invariant**: a native local is declared exactly once, in scope for every
  use; the IR's `Set(v, Null)` position means *initialization order* only —
  it must not double as the declaration point.
- **Homes today**: `scopes.rs lastuse_reclaim` relocates the null-init for
  watermark benefit; `generation/dispatch.rs`'s `declared` set derives the
  `let mut var_…` placement from wherever that init landed.  Fix A
  (`has_free_before_alloc`) is a GUARD compensating for the coupling — it
  excludes early-return-shadowed stores from reclaim so the derived `let`
  never strands a free (rustc E0425, 92× on the brick-buster `--html`
  build pre-A).
- **Natural home**: the native function emitter, declaring ref-typed locals
  up front FROM THE VARIABLE TABLE (the structure that already knows every
  local and its type); `Set(v, Null)` lowers to a plain assignment.  After
  the move, any IR reordering is safe by construction and the Fix-A guard
  is pass-3 dead code — deleting it restores the watermark reclaim for the
  excluded stores.
- **Instrument before the move**: an env-gated sentinel in
  `lastuse_reclaim` counting excluded stores per function across the suite
  + the brick-buster build — quantifies the recoverable reclaim (if ~zero,
  the move is cleanliness only; if large, it carries Goal-E value).
- **Validation matrix when executed (low-churn window)**: the original 92×
  E0425 `--html` build (now reproducible locally — binaryen + chromium
  installed 2026-06-11, `make game` + `tests/html_render.rs` green), full
  native + wasm suites, ASan gate.
- **Status**: EXECUTED 2026-06-11 (#260 Fix B, branch bugs325).  The
  instrument showed brick-buster's generator forfeited 46/46 owning
  stores; the move landed as a prologue on BOTH backends (native:
  sentinel `let mut` per `__vdb` from the variable table + the
  non-`Block`-body branch covered; interpreter: `OpInitRefSentinel` per
  slot after the frame reserve), `Set(v, Null)` keeps its IR position
  (allocation timing unchanged; native first-Set still emits the named
  `null_named` + `OpDatabase` pair via the `predeclared` set), and the
  Fix-A guard (`has_free_before_alloc` + `contains_free`) is deleted.
  Validated: 92×-E0425 `--html` build clean, full suite, headless GL
  render gate, 177 regression on both backends.

### F3-1 — two null encodings, comparator normalises: probed, HELD (2026-06-11)

- **Invariant**: one byte-pattern means "null DbRef" — or every consumer
  must accept both.
- **Homes today**: `OpNullRefSentinel` (store u16::MAX) for explicit null /
  clears; zero-default bytes (0,0) from `set_default_value` for omitted
  fields; `OpEqRef`; `free_named`'s skip patterns (MAX early-return AND the
  (0,0) cascade skip).
- **Probe**: defaults vs explicit null — `== null` true for both, the two
  nulls compare EQUAL to each other, both deref gracefully
  (`/tmp/claude/sweep/f3a.loft`, `f3b.loft`).  Verdict: held — the
  comparison home normalises.  Residual risk: byte-level consumers (codecs,
  `get_u32_raw` walkers, future ops) must keep accepting both; the dual
  encoding itself is a pass-3 dedup candidate (pick one encoding at write
  time).

### F9-2 — registry native build-cache laundered stale cdylibs on a loft upgrade (2026-06-12, FIXED)

- **Invariant**: a cached native artifact is served only when the loft
  build that produced it is the one running (the `loft_build_fingerprint`
  rlib-content seam — @PLN11 Arc N / N0).
- **Homes today**: the fingerprint sidecar (the cheap gate) vs cargo's own
  fingerprinting underneath (the assumed authority).  The gap: cargo is
  structurally BLIND to a loft upgrade — the package crate's sources and
  the captured RUSTFLAGS can be byte-identical while the generated ABI
  changed — so the fp-mismatch-triggered `cargo build` no-ops, and
  `auto_build_native`'s success arm then stamped the NEW fingerprint over
  the UNREBUILT artifact: laundered as fresh forever (the store-65535
  sentinel-deref class at some later date).
- **Probe**: a REAL rlib change (temporary const in lib.rs) → the
  native-auto path rebuilt correctly; `~/.loft/build-cache/graphics-0.1.0`
  did NOT rebuild and its sidecar was re-stamped with the new fp.  (An
  earlier sidecar-corruption probe was unfaithful — cargo's no-op is
  CORRECT when the rlib genuinely didn't change.)
- **Fix (the chokepoint)**: `cache::clear_stale_native_target` — an
  existing artifact whose sidecar NAMES ANOTHER loft build clears the
  target before the build, so cargo cannot no-op over it.  Two
  suite-caught narrowings shaped the final scope: (1) an ABSENT sidecar
  does NOT clear — unknown provenance is commonly a legitimate hand-built
  artifact (the documented `cargo build` workflow; the tests/lib fixture
  cdylibs — the first cut wiped those and broke `native_loader`); (2) the
  clear applies only to the REDIRECTED root (`~/.loft/build-cache/…`),
  which is private to the locked resolution path — an in-tree
  `native/target` is shared with direct-path consumers (the fixture
  loaders read the `.so` without the lock; the second cut raced them
  mid-suite), and in-tree crates remain the documented dev workflow
  (`make rebuild-native-cdylibs` / the stub-panic hint).  `fp == 0` keeps
  the existence-only fallback.  Unit-tested
  (`cache::tests::clear_stale_native_target_clears_only_mismatched_artifacts`);
  e2e probed with two real rlib flips + a post-narrowing sidecar flip.
- **Residual boundary (by design)**: a SHIPPED prebuilt
  (`<pkg>/native/<libname>`) wins unconditionally in `resolve_native_lib`
  — that is the package author's pinned-artifact promise, not a cache.
- **The E0514 sibling, diagnosed + guarded (2026-06-12)**: which rustc a
  spawned build resolves is CWD-dependent under rustup (the repo's
  toolchain pin vs the default elsewhere).  Two build families differ:
  a PACKAGE native crate depends on loft-ffi (the C-ABI contract) and
  never the loft rlib — any rustc builds it correctly, cargo re-builds on
  toolchain flips, NO guard (a first cut guarded it and wrongly blocked
  valid /tmp builds) — while `build_shared_cdylib` (the auto-shared
  cdylib for in-repo libs) links the SVH-locked rlib DIRECTLY and is
  doomed under a mismatch (this was the E0514's true source, surfacing
  through main.rs' "could not compile native (…); interpreting it").
  Fix: `cache::rustc_mismatch()` (the memoised stamp-vs-live check,
  hoisted out of main.rs' native path into one home) consulted by
  `build_shared_cdylib` — a one-line reason + interp fallback instead of
  a compiler spew.  Flap-probed: 1.96-cwd builds the FFI crate ✓, refuses
  the rlib-linking cdylibs with the reason ✓; 1.95-cwd builds everything
  natively ✓; back to 1.96-cwd serves caches silently ✓.
- **The class's MILD symptom, diagnosed (2026-06-12)**: the
  `1 stores not freed: Scene×1` warning on `map_export_glb` runs was NOT
  a source bug — it reproduces exactly and only with a CROSS-BUILD
  loft↔cdylib pair (probed both ways: 5 consistent-pair cells × 0 leaks;
  an old binary against new-build cdylibs leaks Scene×1 on demand).  The
  ABI mismatch corrupts store-ownership accounting instead of crashing —
  the gentle face of the store-65535 class.  An earlier "pre-existing on
  main" conclusion was itself a cross-pair measurement (a baseline
  worktree binary run against current-build cdylibs).  The cache fix
  above closes the only user-reachable route; deliberate cross-pairs
  (worktree experiments, hand-built mismatches) remain possible and now
  diagnosable by this signature.

### The armed-channel restoration (2026-06-12) — four stale duals fixed, one LIVE release bug found

The armed-lib-debug channel (`profile.dev.package.loft`
debug-assertions=true) was globally red — worse than dead, it misled
(this session twice nearly drew conclusions from a silently-unarmed
binary).  Peeling the onion, 279/291 corpus files failing → **12**:

- **adopt_store half-cleared the @P317 bitmap-vs-flag dual** (F3 family):
  it cleared the slot's free *bit* but never the store's own `free`
  *flag*, so every armed `validate()` on an adopted bundle store died
  ("Using a freed store" in `ir_store::write_definition`).  Fixed at the
  registration site; release keys on the bitmap and never noticed.
  Callers audited: codec-only (workers adopt via separate plumbing).
- **`Store::open` constructed file stores as `free: true`** then
  self-validated → the warm-load twin of the same dual.  An opened
  file-backed store is in use by definition — flag cleared at
  construction.
- **The claims-vs-headers dual** (the store.rs row's other entry): a
  reopened file store has live headers but an EMPTY in-memory `claims`
  set, so armed `Store::valid` rejected every warm-load read.
  `read_only` is the check's designed exemption (worker stores) — the
  three ir_read loaders now route through one `adopt_read_surface`
  helper that sets it (also guarding the cached artifact against
  mutation).
- **I7 enforced a deleted allocator's invariant**: state/codegen's
  `validate_slots` call still passed scope-aware=V1 after @PLAN53 made
  the scope-blind V2 the ONLY allocator — false-positives on
  legitimately zone-2-placed small vars (`_reduce_acc_N`).  Both call
  sites now agree (skip I7).
- **The set_var width assert's model is incomplete** (pushed ==
  Variable-slot width fails for compensating put-op families: fn-ref —
  already noted in its own comment — Str→String text conversion, and a
  16 B call result into a 12 B DbRef slot in `t_4File_lines`).  Taught
  the text case; demoted the rest to a warning with
  `LOFT_WIDTH_ASSERT=1` escalation until a per-op width table exists.
- **`check_ref_leaks` was uniformly stale on the retbuf shape** — FIXED
  2026-06-12: the runtime trace showed the "missing" frees were
  `OpFreeRefIfDistinct` (the witness-pair conditional release of a
  ref-returning call's work ref) — the gate's `freed` collector matched
  only plain `OpFreeRef`.  It now counts the whole frees-arg-0 family
  (`OpFreeRef` / `OpFreeRefTag` / `OpFreeRefIfDistinct`); all ~130
  corpus false positives gone, no `LOFT_REF_LEAK_WARN` crutch needed
  (the env stays as a deliberate escape hatch for chasing runtime bugs
  past a known leak shape).
- **THE FIND — pass-2 arity growth is a LIVE release crash**: the
  restored channel's first catch.  A forward CALLER of a
  multi-return-site fn halts codegen with "Too few parameters on n_pick
  (got 2, need 4)" on BOTH backends.  CORRECTED mechanism (the rr
  traces): for a FORWARD callee the site's type is Unknown in pass 1,
  so `ref_return` never runs there at all — ALL growth happens
  pass-2-first, and a caller parsed earlier in pass 2 keeps the
  signature arity.  F1 (two-pass determinism) × @PLN55.
  **A per-site pre-provision fix was tried and REVERTED (2026-06-12)**:
  counting syntactic return sites in pass 1 and pre-adding
  `__retbuf2…N` fixed the 2-level cell — but the matrix's next cell
  showed buffer need CASCADES: a caller's single tail call materializes
  one work ref PER hidden buffer of its callee (`use_pick`'s one site
  → ls of three `__ref_N`), so no pass-1 syntactic count can bound a
  forward CHAIN (the 3-level cell crashed one level up).  The class
  fix needs a BETWEEN-PASS buffer-count fixpoint (the
  typedef-resolution slot runs after every pass-1 signature + body
  exists; resolve counts in dependency order) — or caller-LOCAL backing
  for the non-chained work refs so only one buffer ever promotes.
  Demonstrating tests (`#[ignore]`):
  `tests/issues.rs::pass2_arity_growth_forward_caller` +
  `…_forward_chain`; repros `/tmp/p_followups/p14_forward_caller.loft`,
  `p15_three_level.loft`.
  **DESIGN ROUND (2026-06-12) — the stated question is RESOLVED by
  probes; the fixpoint candidate is FALSIFIED:**
  - **Self-recursion diverges**: a multi-site fn whose tail calls
    ITSELF gives `buffers(f) = 2 + buffers(f)` — no finite fixpoint
    exists, every pass adds attrs, codegen halts ("got 5, need 7").
    No forward reference involved.  A between-pass fixpoint CANNOT
    terminate here, so it is unsound for the class, not merely
    incomplete (`casc_recursive.loft`,
    `tests/issues.rs::pass2_arity_growth_self_recursive`).
  - **Mutual recursion crashes** the same way — a call cycle always
    contains a forward edge, so no resolution ORDER fixes it
    (`casc_mutual.loft`, `…_mutual_recursion`).
  - **The per-site ABI leaks into the user-facing fn TYPE**: two fns
    with the identical declared signature `(text) -> S` cannot share a
    fn-ref variable — `fn(text, S) -> S` vs `fn(text, S, S, S) -> S`
    (compile error, `casc_fnref.loft`).  Hidden machinery visible at
    the language surface (Goal F); any multi-buffer design keeps this.
  - **Surviving design — ONE buffer per fn** (H1's own invariant made
    exact): *a heap-returning fn's arity is signature+1, fixed at
    declaration; every return path delivers its value in THE
    `__retbuf`* — chaining the site's call into it when safe, copying
    when the site's value aliases the buffer's current contents.
    Recursion becomes trivial (self-call passes own buffer); the
    fn-ref type unifies; the pass-2 growth assert becomes a true
    invariant.
  **BUILT 2026-06-12 for `Type::Reference` returns** (branch
  windows-probe): `ref_return`'s growth branch now binds later sites
  to the one buffer — a site's OUTERMOST work ref (its tail call's
  buffer arg, `site_value_ref`) substitutes to the buffer var across
  the body (`unregister_work_ref` keeps the orphan out of the preamble
  /free sets — a leftover free flips the tail-`If` emission into the
  @P378 `Return(Null)` trap) and gets the canonical
  `{ buf = call(...); buf }` value shape (`chain_site_set_shape` — a
  bare `{call; var}` block would make the call a witness-freed
  DISCARD); inner refs stay plain locals (binding both aliases the
  outer call's dest with its own argument — the C-a hazard,
  `casc_nested.loft`); named locals read by sibling sites deep-copy
  via `materialize_return_into`.  All four crash cells fixed +
  un-ignored (`tests/issues.rs::pass2_arity_growth_*`); 10-cell
  matrix interp + native green; armed corpus at the 4 pre-existing
  singletons; full suite green incl. the moros glb chain.
  **Found en route (pre-existing, filed)**: #355 — the Vector arm
  still grows per-site, and a forward caller of a multi-site VECTOR
  fn silently returns the WRONG element (both backends); the
  one-buffer binding must extend to the Vector/struct-Enum arms.
  #356 — mid-body `return f(g(x))` (lifted args) returns the null
  sentinel on native (interp masks via eval-stack-top; the scopes
  `Return(Null)` fall-through needs to learn buffer-valued tails).
  Also: fn-refs to struct-returning fns cannot be CALLED at all
  (pre-existing compile error "expects 2 arguments, got 1" — the
  CallRef path never fills hidden buffer args; surfaced by the
  fn-ref-type probe).  Lambdas keep in-place growth (sound — no
  earlier callers).

Remaining armed-corpus failures (5 files, no env crutch; the corpus =
tests/scripts + tests/docs + examples on the armed interp binary —
rebuild it with
`cargo --config 'profile.dev.package.loft.debug-assertions=true' build --bin loft`):

| Site | Files | Read |
|---|---|---|
| ~~`vector.rs:331`~~ | ~~`132-vector-elemset-inline-literal-uaf`~~ | **FIXED 2026-06-14** — see below |
| `codegen.rs:1871` | `166-p390-self-slice-assign` | **VERIFIED over-strict 2026-06-14** — see below |
| ~~`codegen.rs:2560`~~ | ~~`examples/collections.loft`~~ | **Both bugs FIXED 2026-06-14** (empty `{}` + dup-key vector+hash) — see below |
| `compile.rs:306` | `75-native-stub` | now clean on the rebuilt armed binary (the native-stub panic is that test's SUBJECT — expected-fail by design) |
| `control.rs:3487` (pass-2 growth) | `298-multi-return-site-ref-buffer` | the mapped arity-cascade class (see THE FIND above) |

**2026-06-14 armed-residual triage** (branch `stability2nd`; reproduce any of
these with `./target/debug/loft --tests <file>` on the armed binary — the CLI
`--tests` runner is the harness's multi-top-level-entry path, the live shape):

- **`132` — FIXED (was a SILENT use-after-free, interpreter only).**  A
  field-of-element SET to a struct literal carrying a vector
  (`xs[i].field = V{a: [literal]}`) built the nested vector IN PLACE by
  appending via `OpNewRecord`, reusing one element-ref temp `_elm_2` across the
  appends.  `generate_set`'s reassignment path emitted an UNCONDITIONAL pre-Set
  `OpFreeRef` of the old `_elm_2` — but an `OpNewRecord` RHS returns an INTERIOR
  ref into the vector's backing store, so consecutive elements share one store
  and the whole-store free killed the live backing.  Correct bytes survived
  until the next allocation recycled the freed store (NON-deterministic data
  loss under churn; deterministic only under the armed `get_vector` assert).
  The native generator's `if _old.store_nr != new.store_nr` guard already
  protected it.  **Fix** (`src/state/codegen.rs` `generate_set`): route an
  `OpNewRecord` RHS through the runtime-guarded post-free
  (`OpFreeRefIfDistinct`).  Regression: `tests/scripts/372-field-elem-set-
  nested-vector-uaf.loft` (armed-corpus member + release-CI over-fix baselines).
- **`166` — VERIFIED over-strict armed assert (no runtime bug).**  The
  `generate_set` first-assignment self-reference assert
  (`!ir_contains_var(value, v)`) fires on the @P390 text/ref-element self-slice
  (`v = v[a..b]`, `v: vector<text>`): `materialize_iterator` emits
  `v_set(slice_elm, *next)` where `*next` legitimately re-reads `slice_elm`'s
  slot (the materialization writes it before the outer Set re-reads), a benign
  slot reuse.  Stress (50 rounds × 20-elem text vectors) is correct on BOTH
  backends; the assert is over-approximating.  Lower priority (cosmetic for the
  armed channel) — left as a known over-strict assert under the feature freeze.
- **`collections.loft` — TWO bugs.  Bug 1 FIXED 2026-06-14; bug 2 is
  design-level, localized + documented, example corrected to sidestep it.**

  - **Bug 1 — empty `{}` keyed-literal silently corrupts. FIXED.**  loft's empty
    collection literal is `[]`, NOT `{}` (`{}` is an empty Void BLOCK).  In a
    collection-typed struct field, `Counter { by_text: {} }` lowered the `{}` to a
    0-byte empty block that UNDER-FILLED the struct record (sibling vector field
    mis-located → SIGSEGV interp / E0308 native; armed assert
    `generate_call: mutable arg expected 16B but Block "empty block" pushed 0B`).
    Fix (`src/parser/objects.rs` `parse_object_field`): in a collection-field
    position, accept `{}` as the already-primed empty collection and WARN toward
    `[]` (Goal F — warn + continue over silent corruption).  Regression
    `loft-suite/attic/scripts/373-empty-braces-collection-field.loft`.

  - **Bug 2 — duplicate hash key in a vector+hash sibling pair corrupts the
    vector. FIXED.**  `Stores::field` (`src/database/types.rs` ~149) deliberately
    LINKS an index-type field (`hash`/`sorted`/`index`) to a same-content sibling
    field via `other_indexes` — the "two views share records" secondary-index
    pattern (append to `vector<Word>` auto-maintains the `hash<Word[text]>`
    index).  A duplicate key (`["apple","banana","apple"]`) routed through
    `dedup_keyed`, which DELETED the displaced record to enforce the hash's
    unique-key invariant — but the vector still held it positionally → element [0]
    read null / `Unknown record` SIGSEGV (matrix: same struct + same element type
    + a `hash` sibling (not `sorted`) + a KEY-field duplicate; hash elsewhere or
    distinct keys were clean).  Fix (`src/database/search.rs` `dedup_keyed` gains a
    `secondary` flag; `structures.rs` `record_finish` passes `true` for the
    `other_indexes` loop): a SECONDARY index only UNLINKS the stale key→record
    mapping, never deletes the record — the `other_indexes` analogue of @P305's
    `keyed_field_is_linked` update-only path.  Semantics: the vector keeps every
    record (insertion order, duplicates included); the hash keeps one per key, the
    latest appended.  Regression `loft-suite/attic/scripts/374-vector-hash-sibling-dup-key.loft`.

**The `store.rs:1640` row (7 files) is RESOLVED 2026-06-12** — the
"keyed armed UAF" was not a use-after-free; the one assert site hid
THREE distinct mechanisms, two of them real out-of-record writes
silent in release:

- **A — header-as-`room` read via the wrong accessor** (the 4 keyed
  files): a hash bucket's `room` IS its record-size header
  (`hash.rs::add` claims `room` words), but `copy_claims_hash_body` /
  `remove_claims` read it through `get_u32_raw(cur, 0)`, whose
  `valid()` gate rightly rejects data reads below fld 4.  Fixed by a
  dedicated `Store::record_words(rec)` accessor (claims + liveness
  asserts kept, fld-0 allowed) now used by every room read in
  `hash.rs` + `allocation.rs` — the dual-use invariant has one named
  home.
- **B — parallel s_pos array stomped its own record header**
  (`22-threading`, `22c-par-sources`): `run_parallel_text`'s
  per-worker result array claimed `ceil(rows*4/8)` words and wrote
  from fld 0 — index 0 landed on the record's size header.  Fixed:
  claim `ceil((4+rows*4)/8)` words, read/write at `4 + idx*4`.
- **C — OpDatabase under-claims 1-byte payloads — a REAL release
  out-of-record write** (`292-pln17-three-state-boolean`):
  `Stores::claim` takes WORDS; both OpDatabase twins passed the type
  size in BYTES.  Probe matrix: size ≤ 1 byte (a one-boolean struct, a
  closure capturing one boolean) claimed 1 word total = header + type
  tag, ZERO payload bytes — every field write landed one byte past the
  record (silent in release); size ≥ 2 over-allocated ~8× and masked
  the class.  Fixed at both twins with the exact formula
  `1 + ceil(size/8)` (`state/io.rs::database`,
  `codegen_runtime::OpDatabase`); the native twin also gained the
  interp's B2 `enum_parent_size` (it used the variant's own size — a
  latent F4 divergence for unit enum variants).  Guard:
  `loft-suite/attic/scripts/299-single-byte-record.loft` (cross-mode).

Full release suite + armed corpus green after; the armed corpus is
down 12 → 5 files with no new sites.

### F11 sweep day (2026-06-12) — error-path state: four breaks, six refuted claims

Family probe corpus: `/tmp/p_followups/f11_*.loft` (+ `f11_ws.py`); issue
drafts pending user go-ahead in `/tmp/p_followups/f11_issues.md`.

### F11-1 — interp `f#read` on an unopenable file: dangling file ref → OOB panic

- **Invariant**: a failed `File::open` leaves the file table and the record's
  ref slot exactly as they were (the archetype's invariant; the WRITE path
  enforces it since the #340 io.rs fix, the READ path does not).
- **Homes today**: `state/io.rs read_file` (line 429 sets `file_ref = f_nr`
  even when the open failed; line 437 indexes `files[len]` → panic) vs the
  fixed `database/io.rs write_file` (warn + skip) vs the native runtime's
  read (warn + null — the desired posture).
- **Natural home**: one failed-open posture owned by the file table; the
  interp read mirrors the write fix (or both ask one helper).
- **Probe / damage**: `f = file("missing"); f#read(4)` → interp PANICS
  (`state/io.rs:437` index out of bounds); native warns + returns null —
  an F4 divergence layered on the F11 break (`f11_read.loft`).  Control:
  existing-file read fine.  Workaround verified BOTH backends: guard on
  `f.format != Format.NotExists` (`f11_read_wa.loft`).
- **Severity / labels**: sev:high (panic on a recoverable fault), wa:clean.
- **Status**: FIXED 2026-06-12 (branch windows-probe) at the natural home —
  the interp read mirrors native (`match` on the open; Err → warn + return;
  the two later index sites hardened to `get_mut`).  Output is now
  byte-identical across backends.  Regression:
  `tests/scripts/296-file-error-paths.loft` (both backends via wrap/native
  runners).

### F11-2 — `files()` silently truncates a directory listing at the first unstat-able entry

- **Invariant**: a listing returns every entry `read_dir` yielded; a
  per-entry stat failure degrades that ENTRY, not the rest of the listing.
- **Homes today**: `database/io.rs get_dir` (mid-loop `return false` at the
  `fill_file` failure — the partially-filled out-vector survives the error
  exit) vs `02_files.loft files()` (discards the boolean entirely) vs
  `fill_file` itself (already writes a well-formed `NotExists` entry on the
  error path — the abort is redundant damage).
- **Natural home**: `fill_file`'s own error entry IS the degraded result;
  `get_dir` should keep iterating (`continue`, not `return false`).
- **Probe / damage**: dir `[a.txt, b_dangle → missing, c.txt]` →
  `files()` = 2 entries, `c.txt` SILENTLY DROPPED; dangle-first → 1 entry
  (everything after dropped); dangle-last → all 3 (`f11_dir.loft`,
  `f11_dir2.loft`).  Reproduces identically on BOTH backends (one impl).
  Non-UTF-8 names hit a second, earlier `return false` (unprobed cell).
- **Severity / labels**: sev:medium (silent wrong data), wa:none (no
  loft-level way to recover the dropped tail).
- **Status**: FIXED 2026-06-12 (branch windows-probe) at the natural home —
  both mid-loop aborts became per-entry degradation (`fill_file`'s
  `NotExists` element stands; a non-UTF-8 name skips that entry only).
  Probe cells now 3/3/3 (dangle mid/first/last).  Regression:
  `tests/error_path_state.rs::f11_2_listing_survives_a_dangling_symlink`.

### F11-3 — reenter completion clears the standing `parallel_ctx`: par in a live-flipped fn kills the kernel on call 2

- **Invariant**: `parallel_ctx`'s lifetime is the PROGRAM's (who wired it
  unwires it); a re-entered call's completion is not program exit.
- **Homes today**: `live_dispatch.rs:145` wires the ctx for the parked
  State at boot (program scope) vs `state/mod.rs resume()` :3923-3924 and
  `debug_step()` :4010-4011, which pop-and-clear on EVERY loop completion —
  including each `reenter()`'d dispatch (call scope) vs `native.rs`
  `n_parallel_*`'s `.expect("called outside State::execute()")` (panics on
  None).  Three homes, two scopes, one field.
- **Natural home**: the wiring site owns the clear — `execute_argv` (and
  the live_dispatch bootstrap) unwire at program scope; `resume()`'s shared
  exit stops doubling as program teardown.
- **Probe / damage**: kernel with flipped `bump_events` containing
  `par_fold`: ping 1 → `#1100` (par works once), ping 2 → kernel DEAD,
  `panicked at src/native.rs:1645: parallel_fold called outside
  State::execute()` (`f11_flip.loft` + `f11_ws.py`, LOFT_LIVE_FLIP=1
  LOFT_FLIP_FNS=bump_events).  This was the brittleness evaluation's named
  residual; the sweep gives it the mechanism and the e2e repro.
- **Severity / labels**: sev:high (engine death in the live-edit loop),
  wa:partial (keep `par_*` out of flipped fns — flipped-without-par and
  par-without-flip are both suite-covered).
- **Status**: FIXED 2026-06-12 (branch windows-probe) at the natural home —
  `resume()`/`debug_step()` no longer clear `parallel_ctx` (their exits
  serve per-call re-entry too); the wirers unwire at program scope
  (`execute_argv`'s existing clear; the frame-yield driver loop in main.rs
  clears after the final resume; the wasm session drop is its teardown).
  Probe: 4/4 pings with `par_fold` in the flipped fn, zero panics.
  Regression:
  `tests/engine_host_kernel.rs::s2_flipped_fn_with_par_survives_repeat_dispatch`.

### F11-4 — driver-mode native anchors relative paths at the artifact dir: file I/O silently lands in /tmp

- **Invariant**: interp and native resolve a program's relative path to the
  SAME file (`resolve_path`'s doc claims one anchor: "the program's own
  directory").
- **Homes today**: `main.rs:4486` (interp: anchor = the `.loft` source
  dir) vs the generated native `main` (`generation/mod.rs:1263`:
  `stores.source_dir = Stores::source_dir_native()` = the EXECUTABLE's
  dir).  Right for a shipped standalone bundle (the #255/@PLN9 intent);
  wrong for default driver mode, where the executable is a generated
  artifact under /tmp and the user's program + assets live at the source.
- **Natural home**: the driver passes the source anchor into the artifact
  it spawns (env or embedded); `source_dir_native()` remains only for
  standalone-packaged binaries.
- **Probe / damage**: `file("f11_marker_zq9.txt").write(...)` run natively
  from any cwd → the marker lands at **/tmp/f11_marker_zq9.txt**
  (`f11_anchor.loft`); `files("f11ok")` returns 0 entries for a dir that
  interp lists fine (`f11_dirok.loft`).  Every relative read/write in
  default native mode misses the program's assets — silent misplacement.
  Workaround verified BOTH backends: `LOFT_PATHS=cwd` + run from the asset
  dir restores parity (2/2 ≡ 2/2).
- **Severity / labels**: sev:high (silent data loss), wa:clean.  Also an
  F4-parity instance (one fact, two derivations).
- **Status**: FIXED 2026-06-12 (branch windows-probe) at the natural home —
  the driver hands the program dir down at spawn (`LOFT_SOURCE_DIR`,
  explicit-user-wins, the LOFT_LIVE_* pattern); `source_dir_native()`
  honours it and keeps the executable-dir anchor for standalone bundles
  (no env).  One chokepoint covers both consumers (the generated `main`
  and the `source_dir()` builtin); `resolve_path`'s doc claim updated.
  Regression:
  `tests/error_path_state.rs::f11_4_driver_mode_anchors_at_the_program_dir`
  (asserts the parity invariant, backend-independent).

### F11 held / refuted cells (2026-06-12)

Agent-hunted candidates that did NOT survive verification — recorded so the
next session doesn't re-chase them: `rebuild_start` Failed state allows
retry (only `Building` blocks respawn) · `n_kernel_broadcast` counts only
successful delivers · a failed `ws_upgrade` registers nothing (assignment
is after success) · `DEBUG_BPS` keeping unresolved names is the by-design
re-resolve-each-reload contract · `fill_file` writes a well-formed
`NotExists` type byte on its error path (no garbage byte) · the rebuild
artifact existence TOCTOU is contained (swap_start re-validates by
spawning; rollback-by-default).  Documented-unprobed residuals (low
reachability): `use_add` before `switch_to_dep` (phantom lib registration
needs a mid-parse file disappearance); `auto_use_scan_cache` populated
before the scanned lib's parse completes; startup-cache orphan `.store` on
a failed manifest rename (self-healing: next run cold-parses); `dispatch`'s
world-swap not restored on panic (latent — no catch_unwind wraps dispatch
today; becomes live the moment one does); the debug pause loop spins
forever if the control client dies (mechanics-alive by design, but no
resume timeout).
