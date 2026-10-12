<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN87 — Reference-default `&`-binding ownership semantics

**Tracking issue: [loft-lang/plans#87](https://github.com/loft-lang/plans/issues/87)** — the
detailed plan (phases, gates, the matrix) and the eight load-bearing concerns live there.
Implemented by the loft2 agent.

> ## ⚠️ CORRECTED MODEL (2026-06-23) — `&` binds a LIVE REFERENCE. SUPERSEDES the write-back framing.
>
> **`&` means exactly one thing: a binding (or part of a data structure) is a LIVE REFERENCE to its
> source instead of a COPY.** A reference is a normal variable — every operation goes THROUGH it to the
> source, using the EXISTING mutation code (we do NOT special-case assignment and do NOT change the
> mutation code): a READ sees the source's current value, a WRITE writes the source, a field/element
> mutate mutates the source.
>
> **North star:** `a = 3; b = &a; b = 4; a` evaluates to **4** (`b` references `a`; `b = 4` writes `a`).
>
> ### `&` is NOT a general operator — it only appears in a reference-BINDING position
> Its operand must be **addressable**: a variable, a struct field, or a vector element — never a
> temporary. (`&` is not a "normal expression"; it is a binding form.)
>
> | Form | Allowed? | Meaning |
> |---|---|---|
> | `a = &b` | ✅ | `&b` as the WHOLE assignment RHS — bind a reference to `b`. |
> | `a: &T = b` | ✅ | the declared type says reference; `b` is the referent. |
> | `a = &(1+2)`, `a = &foo()` | ❌ error | operand is a temporary, not addressable. |
> | `foo(&x)`, `&x + 1`, `[&a]` | ❌ error | `&` used as a general operator / sub-expression. |
> | `foo(x)` where param is `&T` | ✅ | the reference comes from the param's TYPE — call WITHOUT `&`. |
> | `&var = 3` (`&` on the TARGET) | ❌ error | `&` is a bind-site marker, not an lvalue. |
>
> So a `&` parameter is called **without** `&` at the call site (`foo(x)`), and a `&` local is bound
> with `a = &b` / `a: &T = b`. ✅-ENFORCED: `&var = 3` (`operators.rs`; tests `pln87_amp_on_*`).
>
> ### The ladder — simplest first; each rung a runnable lock-in, both backends, leak-free
> - **L1+L2 — scalar local, live read + write-through ✅ BUILT (interp + native, leak-free).**
>   `a=3; b=&a; a=5; b==5` (live read) and `a=3; b=&a; b=4; a==4` (north star). `b` is a live
>   reference to `a`'s slot, lowered to `b: &T = OpCreateStack(a)` (interp: RefVar deref; native:
>   `*mut T` raw pointer — `&mut` trips Rust's borrow checker on the legal aliasing).
> - **✅ L3 — scalar struct-field reference.** `s.x=3; b=&s.x; b=4; s.x==4`. Live both ways, any field
>   offset. The bind converts the value-read `OpGet*(base, fld)` → `OpGetField(base, fld)` (the record
>   at the field's offset). The formal spec's D-bind-3 #415 gating is about struct *vector* fields
>   (copy-on-bind); a SCALAR field is not gated — the transform fires only for `is_scalar`, leaving
>   vector fields untouched. Both backends + leak-gated; tests
>   `issues::pln87_link_l3_field_{write_through,live_read}`, `leak`, script 434.
> - **✅ L4 — scalar vector-element reference.** `v=[10,20]; c=&v[0]; c=99; v[0]==99`. Live both ways.
>   The bind strips the value-read `OpGet*(OpGetVector(v,..),0)` so `c` holds the element ref. L3 and
>   L4 share one lowering: the value yields the place's DbRef, interp reads/writes it via the uniform
>   RefVar deref, native via a `*mut T` into the store slot (keyed off the `OpGetField`/`OpGetVector`
>   value, no per-variable flag — survives an IR snapshot). Tests
>   `issues::pln87_link_l4_element_{write_through,live_read}`, `leak`, script 434.
> - **✅ L5 — heap whole-value reference.** `o=Obj{..}; p=&o; p.x=5; o.x==5`. A heap local COPIES on
>   `p = o` (value type), so `p = &o` ALIASES instead — **harmonized with the existing `&`-param /
>   #257 alias representation** rather than inventing a new one: `p` lowers to `OpCreateStack(o)`
>   (RefVar, non-owning via skip_free), and is read EXACTLY like a `&Obj` param / a #257 alias — interp
>   a stack-ref (`GetStackRef` deref), native the record DbRef by value. So the read stays uniform (no
>   new flag, no D-bind-0 refactor needed). Field mutation writes through, reads are live; `p = o`
>   without `&` still copies. Both backends + leak-gated; tests
>   `issues::pln87_link_l5_heap_{whole_value_ref,reference_live_read}`, `pln87_plain_heap_assign_still_copies`,
>   `leak`, script 434. (Binding-rebind of `o` is the L7 edge, the same as the #257 alias.)
> - **✅ L6 — `&` function parameter.** `fn f(b:&integer){ b=4 } …; f(a); a==4`. Called WITHOUT `&`. A
>   `&` scalar/struct parameter already links to the caller's lvalue (write-back + field mutation
>   propagate) via the call-site stack-ref — it was working before this rung; the lock-ins just pin it.
>   Both backends + leak-gated; tests `issues::pln87_link_l6_{param_write_through,struct_param_field_writes_back}`, script 434.
> - **✅ L7 — edges** (characterized + made safe/consistent on both backends):
>   - **reference-to-reference** (`c = &b`, `q = &p`) — works: `c`/`q` links to the same source `b`/`p`
>     does. Struct ref-to-ref already worked; the scalar case needed a native pointer-copy branch (it
>     was hitting the #257 DbRef path). Tests `issues::pln87_l7_ref_to_ref_{scalar,struct}`.
>   - **reference inside a data structure** — rejected: `[&a]` hits the general-operator ban, a `&T`
>     struct-FIELD type is "needs type or definition". So a reference cannot be stored in a heap record
>     or collection. Tests `parse_errors::pln87_l7_ref_in_vector_literal_is_error`, script 434.
>   - **source-outlives-reference** — largely prevented by construction: a function cannot DECLARE a
>     `&T` return type, and references cannot be stored (above), so a reference cannot escape its
>     source's scope. An inner-scope reference to an OUTER local is safe (the source outlives it).
>     Test `issues::pln87_l7_inner_scope_reference_to_outer`, script 434. (Full borrow checking — every
>     dangling case — remains the formal spec's deferred `ownership.md`.)
>   - **whole-value reassignment of the source** (`o = S{..}` after `p = &o`) — the alias stays LIVE
>     (the reassignment reuses the record in place). Test `issues::pln87_l7_heap_reference_live_across_reassign`.
>   - **leak-freedom** — pinned on both backends (`leak::pln87_{element,field,heap}_reference_no_leak`).
>
> ### Front-end work
> - **✅ Addressable-operand check** — `&<temporary>` (`&(1+2)`, `&f()`, `&123`) errors; a place
>   (variable / struct field / vector element) is fine. `Parser::is_amp_place` at the prefix-`&` site
>   (`operators.rs`); tests `parse_errors::pln87_amp_on_{temporary_paren,call_result,literal}_is_error`.
> - **✅ `a: &T = b` typed-local form** — the `&` in a local's declared type binds a reference, the same
>   as `a = &b` (sets `amp_pending`, reuses the L1 lowering). Both backends; tests
>   `issues::pln87_typed_local_*`, script 434.
> - **✅ Ban `&` as a general operator** — `&` is valid ONLY as the whole RHS of a binding (`a = &b`,
>   detected by the trailing `;`/`}` at the prefix-`&` site); a call argument or sub-expression
>   (`f(&x)`, `&x + 1`) errors. A `&` parameter is called WITHOUT `&` (`f(x)`). Tests
>   `parse_errors::pln87_amp_{as_call_arg,in_subexpr}_is_error`. **Migration was tiny** — the
>   "~53 stdlib + ~75 test" estimate was `#rust` FFI bodies + comments; the real loft sites were 3 in
>   script 87 + ~5 `&`-param tests, all migrated to `f(x)`. Full suite + both backends green.
>
> **Done: all front-end rules + L1–L7 — the `&`-reference ladder is complete.** Every `&`-reference is
> live read- and write-through on both backends — a scalar local (L1/L2), a struct field (L3), a vector
> element (L4), a heap whole-value (L5), a `&` function parameter (L6) — and the edges (L7) are
> characterized + safe/consistent (ref-to-ref works, references can't escape into containers/returns,
> the alias stays live across reassignment, no leaks). The key to L5 was **harmonizing** the new heap
> reference with the representation the `&`-param / #257 alias already used (one `RefVar` +
> `OpCreateStack` lowering, read as a stack-ref in interp / DbRef-by-value in native) — so the feared
> D-bind-0 refactor was NOT needed; the scalar rungs key off the IR value, the heap rung reuses the
> alias shape. The one genuine deferral is **full borrow checking** (every dangling case) — the formal
> spec's `ownership.md`; the constructs that could create a dangling reference are rejected today, but
> a complete checker is future work. The formal model (`../loft` `formalize` branch: `formal/binding.md`,
> `types.md`) is the spec these close against.
>
> ### Method (per rung)
> Matrix-first probe in `/tmp` on `--interpret`; prove the WORKING bytecode on BOTH backends before
> editing the compiler (loft-codegen skill); verify correct + leak-free with `loft --interpret` / the
> harness (NOT bare `loft` — native, skips the interp leak check); pin a `tests/` lock-in and
> un-ignore it. One rung at a time.

---

Implement loft's binding-ownership model: **heap is reference-by-default** (a binding or
parameter aliases the source; field/element mutation writes through). `&` makes a binding a
LINK to its source (read- and write-through). One uniform meaning for `&`; fixes the
recurring "`&Object` needed to mutate" confusion; realizes the OWNERSHIP_MODEL beacon for
binding semantics. (The "`&` = reassignment write-back" reading is corrected in the box above.)

Design + rationale: [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) +
[DESIGN_DECISIONS.md @C77](../DESIGN_DECISIONS.md). Builds on @PLN85 (the store-lifetime
cluster); the W4 lint joins the @PLN46 warning family.

## Status & handoff — READ FIRST after a context reset

> **CURRENT STATE (2026-07-05): COMPLETE — plan CLOSED (#87 status:finished).**
> P0/P1/P2 all DONE and verified on interp + native + native-wasm; `formal/binding.md` at
> **0 open deviations**. The full P2 reassignment-locality matrix (`{struct, vector, scalar} ×
> {non-&, &} × {reassign, field/element}`) is green + leak-free; `&`-write-back from a CALL/var
> RHS landed (ownership transfer). **P3.2 — W4 redundant-`&` lint FLIPPED on-by-default**
> (`operators.rs::warn_redundant_amp`): the message now names the efficiency cost (a `&`-reference
> is double-indirect / slower), silenceable via `LOFT_NO_WARN_RUNTIME`. Ecosystem cleaned — the
> example/demo code (`audience_crystal` + its `.allow_warnings` removed) and the incidental core
> scripts modernised (drop the redundant `&`); a couple were UPGRADED to validate the write-back
> indirection (`434`, the maker's idea). Residual noise: a handful of deliberate `&`-codegen/
> lifetime regression guards keep `&` on purpose and emit a tolerated warning (internal tests,
> not example code) — no per-site silence directive was added (loft is entering maintenance;
> avoid new surface). **Residual bug filed loft#506:** `&`-write-back to a vector-ELEMENT arg
> silently drops on both backends (caller-side store-back missing) — rare/latent, guarded by
> `tests/scripts/87-amp-element-writeback-limitation.loft`, routed for the eventual codegen fix.
> The dated notes below are the build history.

**P0 ✅ DONE + committed.** Migration sweep across the whole ecosystem (stdlib + all 10 registry
libs + the entire zero-trust-shared-files app) found **zero** real propagation-reliance sites →
**P2 is safe**. (Verdict in § Phases below. The gated `LOFT_SWEEP_P0` sweep instrument has been
removed — D2 done.)

**P1 mechanism ✅ DONE + verified — but BLOCKED on the substrate base.** `&<lvalue>` parses and
binds a single-indirect **VIEW** (NOT a `RefVar` var — that slot is double-indirect,
`codegen.rs:2139`), flagged in `Parser::amp_bindings` for P2's write-back. Verified on BOTH
backends for the reference-default (vector-index) case: `cells = &vv[0]; cells[0] = 99` →
`vv[0][0] == 99` (writes through); the no-`&` control views too. Impl: `operators.rs` base case
(sets `amp_pending`, returns the inner type), `expressions.rs::parse_assign_op` (records
`amp_bindings`, resets the flag), `mod.rs` (the two fields + init + cross-pass reset). Full P1.2
diagnosis on the loft2 agent's task #2.

**Rebased onto main `67710d7d` (2026-06-22): the substrate work MERGED** — #426 + #429 resolved,
the reference-default + `&`-to-reassign design documented (@C77, OWNERSHIP_MODEL). Clean rebase,
build green; P1 **vector** case works on both backends (`cells = &vv[0]; cells[0] = 99` →
`vv[0][0] == 99`).

**The struct-field case gates on the #415 REVERSAL — not on P1, and not on the merge (which
happened).** `cells = &chunk.cells; cells[0] = 99` does NOT write through on main (the struct field
stays `10`/`20`, both backends) because **#415 makes a struct vector-field read COPY on bind**
(OWNERSHIP_MODEL.md:152) — a documented store-lifetime STOPGAP, narrowed to struct fields (vector
INDEX reads keep their view), that CONTRADICTS reference-default. The design's end state reverses it
(struct-field reads become VIEWS via dep-driven ownership — row 102), which is OWNERSHIP_MODEL /
substrate-stream work, NOT P1. Per the revised design (@C77) **`&` means "reassigning writes back"
(P2) and is REDUNDANT for a field/element mutation** (the W4 lint) — so the struct write-through is
REFERENCE-DEFAULT, not `&`. The P1.2 verify (`&chunk.cells; cells[0]=99`) therefore exercises
reference-default struct-field aliasing, which can only pass AFTER the #415 reversal.

**P1's `&`-mechanism is DONE; P2 is the next actionable phase — it does NOT depend on #415.** P1
made `&` ACCEPTED + ALIASING on a local binding and flagged it in `amp_bindings` for P2 — complete
and correct (the vector case proves bind + alias). The struct cell of the P1.4 matrix gates on the
#415 reversal, so it cannot close on this stream. **P2** (non-`&` whole-binding reassignment → local
rebind; `&` → write back; P0 proved the migration empty → safe) changes REASSIGNMENT, not the
field-read copy, so it is the next actionable @PLN87 work on `main`. WIP committed on `tuxedo-work2`:
`operators.rs`/`expressions.rs`/`mod.rs` = P1; `main.rs`/`scopes.rs` = the throwaway P0 sweep (strip
before PR, D2).

### P2 — ✅ COMPLETE on both backends (struct + scalar + vector; the full matrix is green)

**Struct reassignment-locality is implemented, sound, and verified interp == native.** Baseline was
`9 9` (both overwrote — `&` on heap was a no-op); now `1 9` (non-`&` rebinds locally, `&` writes
back). `tests/scripts/87-p2-reassign-locality.loft` is the gate (param/local × struct/scalar,
conditional, repeated rebind, field-then-rebind) — green on both backends and leak-free under
`loft_suite`'s program-exit leak gate (interp) + `--native` (native). The `&`-writeback cell is
EXCLUDED from the gate: its behaviour is correct but the path leaks (see remaining work #1).

**The mechanism (witness + distinct-guarded free — sound across conditional/repeated rebind).**
The naive "null the param slot so `OpDatabase` allocs fresh" is UNSOUND: codegen's owned pre-Set free
then frees the CALLER's store, and the fresh store leaks (store-table exhaustion in a loop). The fix
treats a wholesale-reassigned heap param as an ownership TRANSITION (borrow→owned), guarded at runtime:
- **Detect** (`parser/objects.rs::parse_object`): a struct literal assigned to a user-visible heap
  param — `is_argument && !is_compiler_generated && !is_hidden_param` (the last excludes NRVO
  return-buffers like `result`, which are compiler-promoted but user-named, so keep their in-place
  write). A `&`/`RefVar` param never reaches this branch (`type_matches` is false) → it keeps the
  existing write-back path (P2.2; behaviour correct, but see leak below).
- **Witness** (`Function::rebind_orig` map + `ensure_rebind_witness`): a skip-free + inline_ref
  `__ref_N` work-ref holding the param's caller-supplied `DbRef`.  inline_ref is LOAD-BEARING — it
  makes the witness's entry null-init lower to the non-allocating `OpInitRefSentinel`; a plain owned
  ref would `OpInitRef`→`null()` a kt=65535 store that the stash then orphans (the leak `loft_suite`
  caught — the standalone binary's leak check did NOT, only the harness's cloned-DB run did).
- **Entry stash** (`parser/expressions.rs` preamble): `Set(__orig, Null)` (the inline_ref sentinel
  first-def, for slot assignment) then `OpPutRef(__orig, param)` — a RAW DbRef copy (same store_nr),
  NOT `Set(__orig, param)` which DEEP-COPIES into a fresh store and defeats the distinctness check.
  Snapshots param's ENTRY store; later rebinds move param's slot but not the witness.
- **Rebind** (parser emits, both backends share the IR): `OpFreeRefIfDistinct(o, __orig)` (frees a
  PRIOR rebind store; no-op on the first, `o == __orig`) → `OpInitRefSentinel(o)` (null slot, no free)
  → `OpDatabase(o)` (fresh store).
- **Exit free** (`scopes::check::get_free_vars`, `to_scope == 1` = the function-exit hook every
  `return`/tail routes through; args are otherwise excluded from the free sweep): `OpFreeRefIfDistinct(o,
  __orig)` — frees the rebound store iff distinct from the caller's original. Sound for conditional
  (untaken path: `o == __orig`, no-op) and repeated rebind.
- **Native twins** (`generation/ops/ref_ops.rs` + registered in `ops/mod.rs`): `OpInitRefSentinel`
  (`var = DbRef::NULL`) and `OpPutRef` (`var = value`) were interp-only opcodes; added emitters so the
  shared IR compiles on `--native`. `OpFreeRefIfDistinct` + `OpDatabase` already had emitters.

Whole change is **gated on the rebind detection** (off for all existing code — P0 proved heap-param
reassignment is ~nonexistent), so regression risk to the store-lifetime machinery is contained.
Verified: `issues.rs` 723 pass, `loft_suite` green (incl. the new gate), both backends.

### Method note — when iterating through these multi-pass areas, ALWAYS diff against `main`

The parser is two-pass and the store-lifetime codegen has many interacting sites, so the same area gets
re-read many times and a symptom is easy to mis-attribute to the in-flight change. **When a problem
pops up, compare against `main` first to separate NEW breakage from PRE-EXISTING** — it repeatedly
short-circuited dead ends here (the struct `&` write-back "leak" looked like a P2.1 regression until a
main diff showed the `SetStackRef` path is untouched and the leak is pre-existing; the standalone
binary vs `loft_suite` divergence was another). Concretely:

- **Behaviour / leak:** build a `main`-tip `loft` and run the same probe both ways. `scripts/probe-matrix
  --baseline <main-binary>` auto-classifies each cell REGRESSION vs PRE-EXISTING (CLAUDE.md § matrix-first).
- **IR / bytecode:** `LOFT_LOG=static loft --dump prog.loft` on each binary and diff; or read the source
  diff directly — `git diff main -- <file>`, `git show origin/main:<file>` — to confirm a path is in
  fact unchanged (NO working-tree switch — commit WIP first; CLAUDE.md § Git safety).

If `main` shows the same symptom, it is pre-existing: file it (a `main` bug) or work it as its own item,
don't fold it into the active fix.

**P2 CONSISTENCY MATRIX — ✅ COMPLETE (2026-06-23), both backends, leak-free.** Every cell of
`{struct, vector, scalar} × {non-&, &} × {reassign, field/element}` now behaves uniformly and is pinned
green in `tests/scripts/87-p2-reassign-locality.loft` (interp + `--native` + `--native-wasm`, under the
program-exit leak gate) with the three former gaps' lock-ins un-ignored:

1. **P2.2 `&` write-back leak — ✅ DONE.** `fn f(o: &Obj){ o = Obj{..} }` writes back (`g.x == 9`) AND
   frees the displaced caller store. The RefVar-set lowering frees the OLD `*o` store before installing
   the new value, read LIVE through `o` (interp `OpGetStackRef`+`OpFreeRef` at `codegen.rs`; native
   `OpFreeRef(cell, *var_o)` at `dispatch.rs`) — PATH-SENSITIVE, so conditional `&` reassignment is sound
   with no witness; the transferred construction temp is `skip_free` (the caller owns + frees it). Gated:
   arg + RefVar heap inner + the RHS's `result_var` is the skip_free transfer temp. Lock-in
   `pln87_struct_amp_writeback_is_leak_free`.
2. **Vector non-`&` rebind — ✅ DONE.** `fn f(v: vector<T>){ v = [..] }` REBINDS locally. The `=`-vs-`+=`
   distinction is unavailable at the vector materialiser, so captured EARLIER: in `parse_assign_op`,
   BEFORE the RHS parse, a `=`-to-a-visible-vector-param with next token `[` (peek) marks the param a
   rebind; the RHS parse's `vector_db` then hands it a FRESH `__vdb` backing (skip_free; freed at exit by
   the param's `OpFreeRefIfDistinct(v, witness)`). `+=` / self-concat / `v = other` keep the caller
   backing. Lock-in `pln87_vector_param_reassign_is_local`.
3. **Vector `&` write-back — ✅ DONE.** `fn f(v: &vector<T>){ v = [..] }` WRITES BACK. A `&`-vector ref
   shares the caller's backing and cannot repoint, so the write-back is a CLEAR + refill IN PLACE:
   `parse_assign_op` prepends `OpClearVector(v)` before the literal (which appends to the caller's store)
   → replace, not grow. `+=` keeps the grow. Lock-in `pln87_vector_param_amp_writes_back`.

**REMAINING (not P2-core):**
- **P3 — ✅ IMPLEMENTED, OPT-IN (`LOFT_WARN_REDUNDANT_AMP=1`).** The W4 lint flags a `&` on a heap
  STRUCT param that the body never reassigns (field mutation propagates regardless; `&` only matters for
  write-back). Excludes `self` (the method-receiver convention), scalar `&` (always load-bearing), hidden
  return-buffers, and never-read params (covered by `test_used`). `parser/operators.rs::warn_redundant_amp`,
  called per-fn from `definitions.rs`; tests in `tests/runtime_warnings.rs` (`w4_*`). **Kept OPT-IN**
  because reference-default (P2) only just made the pattern redundant, so ~20 `&` ref-param regression
  tests + ~9 scripts still use it intentionally — enabling by default flags them all at once. **Follow-up:
  an ecosystem cleanup pass (modernise / acknowledge those usages), then flip W4 on-by-default + silenceable
  per the original P3.2 spec.**  Vector/Enum inners and the `&vector` realloc edge (P3.0) are out of this
  first cut.
- **D2 — ✅ DONE** — the throwaway `LOFT_SWEEP_P0` instrument is removed from `scopes::check` + `main.rs`.

**Post-merge probing — `&` write-back RHS shapes (2026-06-23).** The P2 matrix tested the LITERAL
forms only; probing the others found two gaps and a latent concern:
- **#1 (was a LEAK, then a rejection) — ✅ FULLY LANDED.** `&`-struct write-back from a CALL
  (`o = mk()`) or VARIABLE (`o = src`) originally leaked the displaced caller store (the P2.2
  displaced-free fired only when the RHS `result_var` was the skip_free construction temp, i.e. only
  `o = Obj{..}`). It was first shipped as a clean parse rejection (no silent leak), then the full
  **ownership-transfer** support landed: a call/var RHS is routed through a transferable owned temp,
  so the write-back reaches the caller (`a.x == 9`) — verified clean on **both backends**. The parse
  rejection is gone. Lock-in: `issues::pln87_amp_writeback_from_call_writes_back` is now an **active,
  passing** test (was ignored); `leak::pln87_struct_amp_literal_writeback_no_leak` still guards the
  literal path.
- **#2 (was a WRONG MESSAGE) — ✅ FIXED.** `&vector = otherVector` reported the misleading
  "`&` but is never modified"; now the same clear write-back-not-supported error. The duplicate
  "never modified" is suppressed via `Parser::writeback_rejected` (cleared per function).
- **#3 — leak-detection divergence — ✅ RESOLVED: NOT a detection bug.** The detection logic
  (`collect_store_leaks`) is byte-identical everywhere. The apparent "binary misses leaks" had two
  compounding causes, both benign:
  1. **The default run mode is `--native`** (`main.rs` `native_mode = true`). Bare `loft prog.loft`
     compiles + runs the native binary and exits via the subprocess status BEFORE the interpreter's
     `check_store_leaks` (which only runs on the explicit `--interpret` path; `--tests` arms it too).
     Native has its own `collect_store_leaks`, gated opt-in on `LOFT_NATIVE_LEAK_CHECK`.
  2. **The leaks P2 fixed were INTERPRET-ONLY.** The eager `OpInitRef` null-init allocates a store in
     the interpreter (`state/io.rs`) but native lowers the same init to `DbRef::NULL` — *no allocation*
     (`generation/dispatch.rs:687/766`). So the native default genuinely has no such leak to report.

  Verified: `loft --interpret` on the (fix-disabled) v_cond case prints
  `Warning: 1 stores not freed … kt=65535 ?×1` — IDENTICAL to `leaks_for`/`loft_suite`. The harness is
  interpret-based, so it is the correct authority for interpreter store-lifetime work — which is what
  every P2 fix targeted. (Native leak-checking is a separate axis: `LOFT_NATIVE_LEAK_CHECK` + a native
  run.) No code change needed; the takeaway is a doc/process one: **leak-check ad-hoc runs with
  `loft --interpret`, not bare `loft` (which is native and skips the interpreter check).**

Runnable probe (the behavioral pair — now prints `1 9` on both backends):

    struct Obj { x: integer }
    fn f(o: Obj)  { o = Obj { x: 9 } }    // non-& param: REBINDS local → a.x == 1
    fn f2(o: &Obj){ o = Obj { x: 9 } }    // &-param: write-back → a2.x == 9
    fn main() { a = Obj{x:1}; f(a); a2 = Obj{x:1}; f2(&a2); print("{a.x} {a2.x}\n"); }

## Phases
- **P0 — ✅ DONE (2026-06-22): the migration is EMPTY → P2 is SAFE.** Swept the whole
  ecosystem for heap-typed PARAMETERS reassigned wholesale (an IR `Set` on an argument
  slot, which propagates to the caller today) via a gated `scopes::check` instrument
  (`LOFT_SWEEP_P0=1 loft … / --tests …`). Result: **0** in the stdlib (`default/`),
  **0** across all 10 registry libs (cbor, crypto, web, server, time, random, regex,
  arguments, game_protocol, input), and **0** across the entire `zero-trust-shared-files`
  application (20+ packages incl. `server/core`). The only raw hits anywhere are
  crawler's `cube`/`plane`/`sphere` (3) — NRVO return-buffers (`fn cube() -> Mesh`
  builds + RETURNS a Mesh), safe by construction: the caller receives the value via the
  RETURN, not param propagation, so P2's local-rebind change cannot break them. **No code
  relies on non-`&` reassignment propagating — the load-bearing P2 risk is retired.**
  (The sweep instrument has been removed — D2 done.)
- **P1** — `&` on local bindings (additive, non-breaking).
- **P2 — ✅ COMPLETE both backends, leak-free (2026-06-23).** reassignment-locality (**breaking**:
  non-`&` reassignment → local rebind; `&` writes back), uniform across struct / vector / scalar; the
  full consistency matrix is green in `tests/scripts/87-p2-reassign-locality.loft` (see § P2 above).
- **P3 — ✅ IMPLEMENTED (opt-in `LOFT_WARN_REDUNDANT_AMP=1`; on-by-default pending an ecosystem
  cleanup pass).** W4 redundant-`&` lint.

## Concerns (detail in the issue)
P2 breaking-change risk (P0 first) · the `&vector` realloc edge (LOFT.md:1529 may be stale) ·
both-backends parity · borrow-checker integration (read the carried `deps` fact, don't
re-derive) · scalar-vs-heap `&` distinction · store-lifetime safety (the @PLN85 fixes stay
green) · out of scope: partial-move / copy-on-write · dogfood against a real heavy-mutation
consumer.

## Implementation steps (verifiable)

Discipline for EVERY step: small + reviewable; lands GREEN on **both** backends (interp ==
native, concern 3); READS the carried `deps` borrow fact rather than re-deriving per site
(concern 4); keeps `tests/scripts/85-store-lifetime-*` green (concern 6). Each step names a
runnable gate — graduate the passing probe to `tests/scripts/87-*.loft`. Land order: P1 → P3.0
(realloc edge) → P2 → P3.

### P1 — `&` on local bindings (additive, non-breaking)

`&` is load-bearing only for scalar params today; on heap it is a no-op (struct/vector are
already reference-by-default). P1 makes `&` ACCEPTED + ALIASING on a binding RHS; it does NOT
change reassignment (that is P2), so P1 is behaviour-preserving for existing code.

- **P1.1 — Parser accepts `&<lvalue>` on a binding RHS.** Extend the assignment-RHS parse
  (`parser/expressions.rs`) to accept `x = &<binding|field|element>`, recording `x` as a borrow
  of the source — the existing `&`-param notation, now at a local.
  *Verify:* `chunk = Chunk{cells:[1,2,3]}; cells = &chunk.cells` parses with NO error
  (parse-success test); a fresh-temp `&mk()` is handled by P1.3, a scalar `&5` stays as the
  scalar by-ref form (concern 5).
- **P1.2 — Codegen aliases the `&`-binding by reusing the `&`-param lowering.** `x`'s type carries
  the borrow dep `{source}`; codegen views the source store (no copy), reading the carried `deps`
  fact — NO new lowering, reuse the `&vector`/`&struct` param aliasing at the local site.
  *Verify (both backends):* `cells = &chunk.cells; cells[0] = 99; assert(chunk.cells[0] == 99)`.
- **P1.3 — Borrow check rejects `&` to a non-outliving source.** Reuse the source-outlives-binding
  inference the `&`-param already relies on (no lifetime annotation — C38 declined reference
  *types*; this is a binding *notation*, concern 4).
  *Verify:* `o = &mk()` (fresh temp) → load-time error naming the dangling source; `o = &existing`
  (outlives) → admitted.
- **P1.4 — P1 gate matrix → graduate.** `x = &<src>; <mutate x>; assert <src sees it>` ×
  {struct field, vector, nested-field} × {interp, native}, plus a `cells = chunk.cells` (no-`&`)
  control (unchanged). → `tests/scripts/87-p1-amp-local-binding.loft`.
  *Verify:* matrix green both backends, no leak (`LOFT_STORES=warn`).

### P3.0 — Pin the `&vector` realloc edge (do BEFORE P2/P3; concern 2)

`LOFT.md:1529` claims `&vector<T>` is needed for `+=` to propagate, but a grow seems to propagate
without `&`. Settle it with a realloc-FORCING grow on both backends.
- *Verify:* `v = &src.vec; for … { v += [..] }` past the inline capacity; `assert(len(src.vec)`
  grew on both backends. If `&` is NOT needed → `LOFT.md:1529` is stale, fix it. If a realloc cell
  DOES need `&` → record the cell so **P3.1 never flags it**.

### P2 — reassignment-locality (BREAKING — de-risked: P0 migration is empty)

Changes EXACTLY one cell of the issue matrix: "heap reassign, no `&`" propagate → local rebind.
Field/element writes are UNCHANGED (still write through).

- **P2.1 — Non-`&` whole-binding reassignment → LOCAL REBIND.** At the heap-binding assignment
  codegen site, a non-`&` `o = X` allocates a FRESH store for `o`, leaving the source untouched
  (today it overwrites the source in place).
  *Verify (both backends):* `fn f(o: Obj){ o = Obj{x:9} }  a = Obj{x:1}; f(a); assert(a.x == 1)`
  (caller UNCHANGED, was 9 pre-P2); control `fn g(o){ o.x = 9 }` still propagates (`a.x == 9`).
- **P2.2 — `&` whole-binding reassignment → WRITE BACK.** A `&` binding/param reassignment writes
  the new value into the source store — `&` now means "reassigning this binding writes back",
  uniform across scalar and heap.
  *Verify (both backends):* `fn f(o: &Obj){ o = Obj{x:9} }  a = Obj{x:1}; f(&a); assert(a.x == 9)`.
- **P2.3 — Store-lifetime safety (concern 6).** The P2.1 fresh-store rebind drops the source view;
  the old store's ownership/free must stay sound.
  *Verify:* `tests/scripts/85-store-lifetime-*` green after P2.1/P2.2 both backends; P2 matrix
  leak-free (`LOFT_STORES=warn`).
- **P2.4 — P2 gate matrix → graduate.** `fn f(o){o=X}` no-`&` → caller unchanged; `&` → caller
  sees X. × {param, local} × {struct, vector, scalar} × {interp, native}. →
  `tests/scripts/87-p2-reassign-locality.loft`. *Verify:* green both backends, no leak.

### P3 — W4 redundant-`&` lint (the @PLN46 warning family)

- **P3.1 — Detection: redundant `&` = never reassigned, HEAP only.** A `&` binding/param is
  redundant iff its body field/element-mutates but never REASSIGNS it (reuse the reassignment
  detector from the P0 sweep). Scalar `&` is always needed → never flagged (concern 5). Exclude
  any realloc cell P3.0 found load-bearing.
  *Verify:* `fn f(o: &Obj){ o.x = 1 }` → flagged; `fn f(o: &Obj){ o = X }` → NOT flagged;
  `fn f(o: &integer){ o = 5 }` → NOT flagged.
- **P3.2 — W4 warning emission.** `Level::Warning` with actionable text ("`&` here has no effect;
  `&` only matters when you reassign the binding"); silenceable via the warning-suppression flag.
  *Verify:* warning + message for the redundant case; a `tests/runtime_warnings.rs` W4 case.

### Close-out

- **D1 — dogfood (concern 8).** Validate `&`-binding ergonomics against crawler's nested-mutation
  world — does `&` read naturally at depth, or awkward? Surface the signal, don't paper over.
- **D2 — ✅ DONE — stripped the throwaway P0 sweep** (the `LOFT_SWEEP_P0` instrument) from
  `scopes::check` and `main.rs`.
