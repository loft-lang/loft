# NEXT SESSION — the DA-inventory leak cells (@PLN85, task: DA calibration residue)

> **▶▶ ALL FOUR LEAK CELLS CLOSED (2026-07-04, branch `tuxedo-pln85-ownership`).**
> t3/p179 (prior), t4/pln87, t1/closure, t2/n8 — each fixed on BOTH backends,
> verified under the DA gate, with a graduated `tests/scripts/85-*` guard.  The
> `skip_free` pass-poison class residual is RESOLVED (§ below).  Remaining DA-map
> items are NON-leak (format.rs:1213 p188, generate_set/call slot-width,
> wrap::dir, stdlib slot-width) — see [fuzz-proof-gate.md](fuzz-proof-gate.md)
> § the final honest DA map.
>
> | Cell | Fix | Guard |
> |---|---|---|
> | t4/pln87 | displaced-free the `&`-param whole-record writeback CALL twin (ownership-oracle gate + aliasing-safe `OpFreeRefIfDistinct`); interp `codegen.rs` + native `dispatch.rs` | `85-amp-writeback-displaced-free.loft` |
> | t1/closure | discarded closure-factory temp keeps its `CalleeFrame` ownership dep + is freed; `skip_free` stamped pass-2-only (owns_closure) — `operators.rs` | `85-closure-factory-discarded-free.loft` |
> | t2/n8 | a `skip_free` var sentinel-inits (no owned placeholder) — `codegen.rs gen_set_first_ref_null` | `85-amp-default-null-buffer-free.loft` |
> | t3/p179 | pass-1 `skip_free` stamp gated `!first_pass` (prior session) | (in `issues`) |
>
> **skip_free pass-poison class residual — RESOLVED (decision):** the poison bites
> ONLY when an H5 lazy-append SHIFTS the work-ref counter between passes, remapping
> a pass-1-`skip_free`'d `__ref_N` to a different pass-2 role (t1 = closure-dep
> append; t3 = nested-literal).  Fix approach **(a) per-site `!first_pass` gates**
> for the two instances (landed).  The remaining stampers were AUDITED non-vulnerable:
> `expressions.rs:1333` (user var, stable role), `objects.rs:2077` (memoised witness,
> already `mark_inline_ref`-safe), `vectors.rs:2552` (non-capturing `FnRefDnr`, null
> closure), `vectors.rs:2643` (pass-2-only), `vectors.rs:1762` + `fields.rs:463`
> (stable roles, empirically leak-free both backends).  The **chokepoint fix (b)**
> (clear all `skip_free` at the pass boundary) is **DECLINED**: variable tables
> persist across the stdlib↔user parse boundary in one shared `Data`, so a blanket
> clear at the user pass boundary would wrongly wipe the stdlib's finalised bits —
> per-file scoping would be more complex + riskier than the residual it addresses.

State as of 2026-07-04 (branch `tuxedo-pln85-ownership`).  This is the live
worklist from [fuzz-proof-gate.md](fuzz-proof-gate.md) § the final honest DA
map.  Probes live in [`probes/da-leak-cells/`](probes/da-leak-cells/) — tiny
standalone programs derived from the four failing tests; run them with a plain
`target/release/loft` (the exit-leak WARNING is not DA-gated — only the test
harness's exit-leak PANIC is, so the CLI has always shown these).

```bash
target/release/loft --interpret probes/da-leak-cells/t3_p179.loft         # etc.
LOFT_NATIVE_LEAK_CHECK=1 target/release/loft --native <probe>             # native leak read
LOFT_SKIPFREE_TRACE=__ref_1 target/release/loft --interpret <probe>       # who stamps skip_free
```

`LOFT_SKIPFREE_TRACE=<var-name>` (permanent instrument, `src/variables/mod.rs
::set_skip_free`) prints every stamping of that variable with a backtrace —
built for this class, keep using it.

## Cell t3 — p179 `&`-field arg: **FIXED + verified (this session)**

- Test: `issues::p179_ref_field_arg_corrupts_sibling`.  Leak was `P179Inner ×1`
  on BOTH backends.
- Root cause (proven with the trace): `convert`'s complex-expression by-ref
  arm (`src/parser/mod.rs`, the `Insert(Set(wv, orig); OpCreateStack(wv))`
  emission) stamped `set_skip_free(wv)` in **pass 1 too**.  `skip_free` is a
  GLOBAL per-var bit; `wv` is a NAME-pooled `__ref_N` (counter-numbered per
  pass, pass 2 re-finds pass-1 vars by name); the bit persists in the stored
  var table across the pass boundary.  The two passes' `work_refs` sequences
  differ, so pass 1's carrier NAME (`__ref_1`) was pass 2's OWNED
  nested-struct-literal temp — the stamp disarmed its scope-exit free.
  **Instance #4 of the counter-coupling hazard**
  ([COMPILER.md § Synthesised-identity stability](../../COMPILER.md)).
- Fix: gate that one stamp on `!self.first_pass` (pass-1 IR is discarded; the
  stamp's only lasting effect WAS the poison).  The nullable twin arm above it
  was already pass-gated.
- Verified: `t3` clean both backends; `t3a_bare` (nested literal, no call —
  was always clean) still clean; `t3b_loop` (100× `&`-field calls in a loop —
  the case the stamp legitimately guards, "freed once per call" dangling)
  clean on both backends; `issues p179` + `leak_cases` + `expressions
  closure_capture` green.
- **Class residual (OPEN, decide next session):** ANY parser site that stamps
  `skip_free` in pass 1 on a counter-named work var can poison a pass-2 role
  the same way.  Stampers: `parser/expressions.rs:1333` (fn_ref_field_read
  bind), `objects.rs:2077` (rebind witness — also `mark_inline_ref`, which
  would additionally break a pass-2 owner's null-init ALLOCATION!),
  `objects.rs:2200` (RefVar-set transfer), `vectors.rs:1762/2552/2643`.
  Options: (a) audit + gate each on `!first_pass` where pass-1 stamping is
  purposeless; (b) the chokepoint fix — CLEAR all `skip_free`/`inline_ref`
  bits at the pass boundary (Function reload) so pass 2 re-derives them;
  verify first that every stamping site actually re-runs in pass 2.  (b) kills
  the class; prefer it if the verification holds.

## Cell t4 — pln87 `&`-param whole-record writeback: **ROOT-CAUSED, not fixed**

- Test: `issues::pln87_amp_writeback_from_call_writes_back`.  Leak `Obj ×1`,
  BOTH backends.  Probe `t4_pln87.loft`.
- Emission (read via `introspect`): `fn f(o: &Obj) { o = mk(); }` lowers to
  `o = n_mk(__ref_1)` via `SetStackRef(o-place, result)` — it REPOINTS the
  caller's owning place `a` at mk's fresh store **without freeing the
  displaced record** (`a`'s original `Obj{x:1}`).  A plain local reassignment
  gets a displaced-free; this `&`-writeback path misses it.
- Note the comment at `parser/objects.rs:2200` claims "codegen's RefVar-set
  lowering frees the displaced caller store first" — empirically it does NOT
  for this shape (or this shape bypasses that lowering).  Start there: find
  the RefVar-set lowering on both backends, check which side should emit the
  displaced-free (`OpFreeRefIfDistinct(old, new)` is the established runtime-
  join tool if aliasing is possible), matrix: fresh-record RHS (`mk()`),
  same-record RHS (`o = o`-ish aliases), null RHS, loop repetition, both
  backends, value+length+leak.

## Cell t1 — closure record of a discarded fn-ref: **UNEXPLORED**

- Test: `expressions::closure_capture_text`.  Leak `__closure_0 ×1`, BOTH
  backends.  Probe `t1_closure.loft` — the immediately-invoked shape
  `make_greeter("Hello")("world")`: the returned fn-ref (16-byte slot carrying
  the closure DbRef) is a discarded TEMP; nobody frees the closure record it
  carries.
- Likely relatives: loft2's #491 "Drop-arm lift for discarded owned results"
  (merged, `5ec5497c`) — the fn-ref temp may need the same drop treatment,
  reading the embedded closure DbRef out of the 16-byte slot.  Also check the
  non-discarded control (`g = make_greeter(..); g("world")` then g dies at
  scope exit) — does THAT free the closure record?

## Cell t2 — n8 `&Data = null` default: **interp-only, kt=65535 ×2**

- Test: `issues::n8_no_empty_pre_eval_binding`.  Probe `t2_n8.loft`.  Native
  is CLEAN — matches the documented eager-`OpInitRef` class (DEBUG_STORES.md § leak
  debugging: interp null-init allocates, native lowers to `DbRef::NULL`), but
  ×2 untyped stores for ONE call pair suggests the `&Data = null` default
  param's hidden buffer allocates per CALL SITE and never frees.  Read the
  emission first (`introspect`), then decide whether it folds into the eager-
  null-init retirement or needs its own free.

## Also open from the DA map — a SEPARATE codegen-slot-correctness class (NOT leaks, NOT @PLN85's store-lifetime charter) — **filed as #493**

These were the residual DA cells after the five leak cells closed: slot-width /
type-resolution / stale-DbRef bugs surfaced by the debug-assertions width-assert +
codegen asserts — **not** store-lifetime leaks — and all reproduce ONLY under a DA
lib build (dev/release/CI have debug-assertions off, so none affect a shipped
build).  Tracked as issue **#493**; the five enumerated cells are **CLOSED**
(2026-07-04):

- **width-mismatch FLOOD** (`t_4File_lines` / `t_4text_split*` `_elm`, plain
  enums, `single`, `text?`, …) — **FIXED** (`codegen.rs` set_var width-check).
  NB the original diagnosis was off: the `i_parse_errors` label is a harmless
  `Reference(0)` fallback (def 0 renders as `i_parse_errors`), NOT the cause, and
  retyping `_elm` would not silence it (a `Reference` slot is always 12 B).  The
  warning is a **width-check false-positive**: `pushed` is the 8-stepped
  eval-stack width, `slot` is the raw type size, so they disagree for every
  CONVERTING put (OpPutRef 12 B, OpPutBool/Enum 1 B, OpPutSingle/Char 4 B,
  OpAppendText).  Fix: restrict the check to the RAW-put family (`Integer`/
  `Float` — the only stores that copy pushed bytes verbatim and can overrun),
  which clears the whole flood and keeps the integer-overflow detection.
- **`exit_codes::p196`** native fn-ref d_nr projection — **FIXED**: the
  interpreter lowered a fn-ref tuple element into a 4 B d_nr field (OpSetInt4)
  with OpVarFnRef (20 B); project only the d_nr with OpVarInt (`codegen.rs`
  generate_call), mirroring native's `generation/calls.rs`.
- **`generate_set` Var(0) self-reference** — **FIXED** (two roots): the `v = v[a..b]`
  self-slice accumulator was freed uninitialised on an empty slice (null-init it
  before the loop, `expressions.rs::materialize_iterator`); and an unknown
  qualified enum variant (`Color::Bleu`) lowered to `c = c` (recover as a null
  enum value, `objects.rs::parse_constant_value`).
- **`generate_call` 8B/16B mismatch** — **FIXED**: a match arm whose pattern type
  cannot convert to the subject (text literal vs int subject) emitted
  `OpEqInt(int, "text")`; use the discarded `can_convert` result to emit a dead
  `false` condition (`control.rs::parse_match_pattern`).
- **`wrap::dir` `get_stack<DbRef>` OOB** — **FIXED**: `substitute_type` had no
  `Optional` arm, so a generic `-> T?` kept the parametric `Optional(Reference(tv))`
  return and mis-typed the return slot; add the `Optional`/`Tuple` arms
  (`parser/mod.rs`, `variables/mod.rs`).
- ~~`format.rs:1213` `types[65535]`~~ — FIXED earlier as a byproduct of the p188
  leak fix.

**Still open (beyond #493's enumerated scope — all PRE-EXISTING, verified panicking
identically on `origin/main` 49baf726, so NOT regressions from the #493 fixes;
they were masked in `wrap::loft_suite` by the p390 abort that now passes):**

- `scopes.rs:2162` "H2 step-5 sentinel" `debug_assert!` — 6 scripts
  (`450-struct-field-vector-return`, four `85-store-lifetime-*` return/borrow
  cases, `repro_p365`).  Two of them use no slice/match at all, confirming it is
  independent of the #493 fixes.
- `state/text.rs:334` free_text "double free" — `85-store-lifetime-enum-match-borrowed-view-overfree`.
- `compile.rs:303` — `75-native-stub`.

DA-gate failures that are ENVIRONMENTAL, not code bugs: `store_persist_loft`
(`store.rs:1838` store-reload assert when the spawned DA binary reloads);
`wrap::library_suite` (audience_crystal); and `baselines_are_locked_in` (offline:
registry auto-install DNS failure on `38_import_unknown_file`; and, when run via a
`target-da/release/default` symlink, a stdlib path mismatch in error messages).

To re-verify each standalone: `target-da/release/loft --path <repo> --interpret <script>`.

## How to re-verify the whole surface

```bash
# the DA lens (target-da has the default symlink already):
RUSTFLAGS="-C debug-assertions=on" CARGO_TARGET_DIR=target-da \
  cargo test --release --no-fail-fast --test issues --test expressions --lib
# never set RUSTFLAGS against the MAIN target (deps double-generation —
# recovery documented in DEBUG_STORES.md § the calibration run).
```
