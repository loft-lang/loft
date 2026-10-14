<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 close-out — concrete steps to finish the plan

The verified remainder, sequenced, with per-item build steps (file · fn · line), effort,
risk, dependencies, and exit criteria. Produced 2026-07-06 by re-verifying every open item
against the live tree (not the docs) — see the "verified state" line on each. Companion:
[REMAINING.md](REMAINING.md) (the prioritised release-gate view) · [README.md](README.md) (status) ·
[borrow-return/DESIGN.md](borrow-return/DESIGN.md) (the A1b/A2 mechanism, grounded in captured IR).

## What is already done (do not rebuild)

- **Phase B move-elision** — DEFAULT ON, merged (#514, squash `46ecd3dc`). All four copy
  shapes, flat + nested. `LOFT_NO_MOVE_ELIDE` opts out.
- **Phase A `--report-copies` report** — merged (#510). `report_copies`
  (`src/use_analysis.rs:1772`), the `--report-copies` CLI flag (`src/main.rs:3810`), the
  `CopyClass` 4-way bucket (`:42`), and the **bound-vs-unbound survival split**
  (`survival_class`, `:855`, gated on `report_copies_enabled()`) are BUILT. Only the
  *enforced* (default-on `Level::Warning`) channel is missing — that is W5 below, **not** a
  rebuild.
- **#462** — CLOSED (native record leak, subsumed by the @PLN85 store/adopt-free siblings;
  compact repros verified clean both backends). Off the list; re-check only if a crawler-scale
  record leak resurfaces (no in-repo corpus to test it now).

## loft2 division-of-labor check (done 2026-07-06)

loft2 (`../loft2`, branch `tuxedo-fix-511`) carries **no fix on any @PLN90-remainder surface**
(its diff vs `origin/main` on `scopes.rs`, `parser/control.rs`, `generation/dispatch.rs`,
`generation/ops/parallel.rs`, `use_analysis.rs` is empty). Its active work is @PLN93
(collection-capture-into-closures, "borrow by shared DbRef", issue #511) + a branch-local #513
store-persist fix. **Zero duplication risk.** One adjacency to honour: @PLN93's capture uses the
same **Reference-DbRef borrow chokepoint** that W1's caller-side materialise touches — when W1
lands, re-check it against @PLN93's captured-collection borrows (and loft2 must rebase onto main
to pick up #514).

---

## Work items

### W1 — A1b: temporary-subject borrow-return UAF  ·  P0 · **✅ DONE (2026-07-06)**

**Landed (default ON, `LOFT_NO_A1B` opts out):** a coordinated promotion-verdict change in
`classify_ret_promotion` (`src/parser/control.rs`). The collapse was two verdicts merging three
roles onto one work-ref — the subject ref `Bind{substitute}`'d into the buffer, the buffer ref
`Rename`'d to `__retbuf`. Detected by `tail_call_borrows_temp_subject` (tail is a buffer-ABI vector
call whose visible heap-param arg is an inline construct, not a bare `Var` — robust across both parse
passes), the fix SKIPS the subject ref (keeps it a distinct local, freed after the copy) and
MATERIALISES the buffer ref into a separate `__retbuf` (owned copy) instead of renaming it → three
distinct stores, no collapse. Matrix green both backends under `LOFT_POISON` (all six cells);
gate-off byte-identical; flag-on exposure sweep clean (issues 748 / wrap 51 / leak 49 / native 9 /
native_scripts / native_dir, 0 failures). Guard: `tests/scripts/85-temp-subject-borrow-return-uaf.loft`.
Full mechanism: [borrow-return/DESIGN.md § A1b gate-5](borrow-return/DESIGN.md).

**Verified state (before fix):** OPEN. [borrow-return/cell-escape-temp.loft](borrow-return/cell-escape-temp.loft)
fails **both** backends — interp assertion-fails (`len(a)=0`), native panics — loud under
`LOFT_POISON`. A1 (`f70a729d`) fixed the interp `gen_if` derail but did **not** close the
lifetime hole; `--native` carries it latently.

**Root (DESIGN.md §A1b, captured):** the `one_buffer_chain` NRVO optimisation reuses ONE store
(`__ref_1`) as **both** the `Filled` subject **and** the return buffer. `g` returns
`__ref_1.items` — a borrow of that very store — and the materialise's `OpDatabase(_dst=__ref_1,64)`
**reallocates the store the borrowed `_src` points into, freeing the source before the copy**.
The native materialise gate (`dispatch.rs`, `if _src.store == _dst.store { alias } else { copy }`)
is downstream of the wrong buffer choice — the root fix is the buffer, not the alias/copy test.

**Target IR (PROVEN clean, both backends + POISON — [cell-escape-temp-FIXED.loft](borrow-return/cell-escape-temp-FIXED.loft)):**
materialise the borrow into a store DISTINCT from the subject, free the subject AFTER the copy —
`r = n_g(c,…); OpClearVector(out); OpAppendVector(out, r); OpFreeRef(c); return out`.

**Localized to the verdict level (2026-07-06 — instrumented, reverted).** `LOFT_TRACE_RR` on
`cell-escape-temp`'s `n_h` shows TWO work-refs: `__ref_1` → `Rename{buf_attr:1}` (becomes
`__retbuf`) and `__ref_2` (the temp `Filled` subject) → `Bind{buf_var:6, substitute:true}` — the
`substitute_work_ref` merges the subject store into the buffer store. **That merge is the collapse.**
cell-escape (safe) has ONE work-ref (subject is a param, not a work-ref) — so the UAF axis is "the
borrowed subject is a temporary the fn itself constructs → a subject work-ref that gets
`Bind{substitute}`." Falsified en route: a single "suppress the Rename" guard is **insufficient**,
and `g.returned()` deps name the hidden buffer (attr 1), NOT the visible `e` (attr 0) —
`filter_hidden` strips the borrow; the "buffer borrows a visible param" fact must be read from g's
BODY. Full trace in [borrow-return/DESIGN.md § A1b gate-2](borrow-return/DESIGN.md).

**Chokepoint localized to the CONSTRUCT-STORE ALLOCATION (gate-4, 2026-07-06).** A gated
delivery-level fix (`LOFT_A1B`, `tail_call_borrows_temp_subject` → `CopyBorrow`) was built, tested,
and **reverted as insufficient**: the predicate fired correctly but the UAF persisted because the
inline subject construct is built with the fn's `__retbuf` work-ref (`__ref_1`) as its store —
`OpDatabase(__ref_1, 67)` inside the `Object` block, where `__ref_1` IS `n_h`'s `__retbuf`. The
`Filled` subject and the return buffer are the **same store before delivery runs**; the
`one_buffer_chain` optimisation collapses all three roles (subject / g-buffer / `__retbuf`) onto one
`__ref_N`. Full trace + reverted-IR: [borrow-return/DESIGN.md § A1b gate-4](borrow-return/DESIGN.md).

**Build steps (the remaining work — upstream store split + delivery materialise):**
1. Find where the inline heap construct passed as a call arg is allocated the fn's `__retbuf`
   work-ref (`src/parser/objects.rs` construct lowering / the call-arg lifting / `one_buffer_chain`
   in `chain_site_set_shape`). Give a **borrowed** heap-subject construct a **distinct** work-ref, so
   the subject store ≠ `__retbuf` and ≠ g's scratch buffer (three distinct stores).
2. With the subject distinct, the delivery-level materialise (gate-3 plan: route to `CopyBorrow` /
   `materialize_vector_return_into`, copy g's borrowed result into the owned `__retbuf`, free the
   subject after) becomes correct — the two halves compose. The reverted `tail_call_borrows_temp_subject`
   predicate + `LOFT_A1B` gate are validated and cheap to reconstruct.
3. Confirm the native materialise gate (`src/generation/dispatch.rs`) hits the copy branch with a
   distinct store (no double-alloc).
4. **Gate** behind `LOFT_A1B` / `LOFT_JOIN_OWN`; suite byte-identical gate-off.
5. Walk the boundary matrix (named / temp / escape × value + length + leak × `LOFT_POISON` × both
   backends), then the full suite gate-on + the @PLN89 differential oracle.
6. **@PLN93 interaction check** (see loft2 note) before flipping.
7. Promote `cell-escape-temp.loft` to a regression guard under `tests/scripts/`
   (e.g. `85-temp-subject-borrow-return-uaf.loft`).

**Depends on:** nothing (A1 already landed). **Blocks:** W2, W3, W4, W5.
**Exit:** all 6 matrix cells + `cell-escape-temp` clean both backends under `LOFT_POISON`; suite
byte-identical gate-off, green gate-on; oracle green.

### W2 — A2: struct-field `b.rows` copy→alias  ·  **✅ RESOLVED — WON'T DO (2026-07-06)**

**A2 is INVALID — the field-return copy is the DECIDED value-semantics contract, not an avoidable
copy.** The three-attempt prototype (below) landed the alias with a green matrix, then the flag-on
exposure sweep broke `tests/scripts/85-store-lifetime-field-read-copy.loft` — an explicit **@PLN85 /
#415** regression guard whose header states the contract verbatim: *"binding or returning a heap
vector FIELD must **COPY** (establish a new owner), not alias the field's store"* and asserts
`ag = getv(by); by.v += [9]; assert(len(ag)==3, "must COPY")` for `fn getv(b) { b.v }` — **A2's exact
target**. The @C86 register confirms it: *"whole-value heap binds COPY by contract (`af=bx.v`) — #415
is the SEMANTIC not a stopgap."* So a struct-field return **must** copy so the caller's result is an
independent owner; **aliasing it violates value semantics** (the mutated source would change the
result). A2's framing as an "avoidable copy optimisation" mis-reads a **required** copy. **Closed:
the current copy behaviour is correct; W3 (drop the buffer) is moot; the field-return copy is
FORCED, not Avoidable.** Three matrix-guarded prototype attempts, all reverted clean (record below).

**Prototype history (verified state OPEN before resolution):** [borrow-return/br_field.loft](borrow-return/br_field.loft)
(`fn f(b: Box) -> vector<E> { b.rows }`) copies into `__retbuf` on both backends
(`OpClearVector(__retbuf); OpAppendVector(__retbuf, OpGetField(b,0,66))`) — **and per #415/C86 it
should.**

**Build steps:** route the struct-field-read tail — `classify_vector_delivery`
(`src/parser/control.rs:1043`) returns `Delivery::CopyBorrow` at `:1109`, gated by
`tail_is_struct_field_read` (`:1082`), dispatching to `copy_borrow_tail_into_retbuf` (`:5388`) —
to the **alias-return** path (`Delivery::Rename` → `ref_return`, the path the A1-fixed match arm
already uses), plus the callee-ABI change (drop the `__retbuf` fill so the alias returns as a
plain `DbRef`; DESIGN slice 1 / option (c)).

**Depends on:** **W1 (hard).** A bare alias of a *temporary* subject dangles (F1) — A2 needs W1's
caller-side materialise in place first. **Exit:** `br_field` returns the alias (`OpGetField`, no
`OpAppendVector`) both backends; matrix cells clean; suite gate-off byte-identical.

**Prototyped + reverted (2026-07-06) — the coupling REFINED (corrects the earlier read).** A gated
`LOFT_A2` route (skip `CopyBorrow` for `tail_is_struct_field_read` → alias via `Rename`) makes `f`
return `OpGetField(b,0)` directly (block dep `["b"]`). Matrix under `LOFT_POISON`, both backends:
- **named** (`b=Box{..}; a=f(b)`) — SAFE (subject out-lives).
- **temp at a BINDING site** (`a = f(Box{..})`) — **SAFE**. The bind does not dangle; **no
  binding-site materialise foundation is needed** (this corrects the prior assumption that A2
  re-opens the UAF at bindings).
- **temp ESCAPING via a return** (`h() { f(Box{..}) }`) — **CRASHES** (interp SIGSEGV / native).
  `LOFT_TRACE_RR` shows `classify_ret_promotion` is **never called** for `h`: A2's alias reroutes the
  field-return down a different delivery path that **bypasses W1's return-site materialise** (`h`
  frees the temp `Box` then `return null`).

**Third attempt (2026-07-06) — the DECISIVE one: A2 is a SEMANTIC change, not an optimization.**
The right shape is to **alias the field INTO the buffer** (a `Set(__retbuf, OpGetField(b,fld))`, the
match-arm `g`'s shape) rather than return `["b"]` directly — this keeps the buffer ABI, so **W1's
temp-subject materialise handles the return-of-temp for free** (f matches g). With that,
`copy_borrow_tail_into_retbuf`'s A2 branch made **all three matrix cells (named / tempbind /
escapetemp) PASS** under `LOFT_POISON` on both backends, and `issues` (748) + `leak` (49) stayed
green. **But the flag-on exposure sweep failed on `wrap` + `native`:** 3 `@PLN85` store-lifetime
tests — `85_store_lifetime_field_read_copy`, `85_store_lifetime_implicit_param_return_copy`,
`85_store_lifetime_borrowed_view_query` — **assert the COPY** and now break (native UAF panics +
value asserts). So A2 changes documented field-return semantics from **copy → view**: the
@C86 "projections stay views (#426)" position vs the `@PLN85` "field-read copies for lifetime safety"
contract those tests pin. Reverted clean (three attempts, tree unaffected). **UPDATE — the decision
is ALREADY MADE (see the resolution at the top of this section): reading the #415 guard's header +
the @C86 register settles it — field-return MUST copy (#415 IS the semantic). A2 is invalid; not a
pending decision. The `*_copy` tests are correct and stay.**

Earlier notes preserved for the record — **Second attempt (2026-07-06) — the
caller-side effect is DEEPER than a delivery reroute.** Adding a `classify_vector_delivery` branch to
route `h`'s return of `f(<temp>)` to `CopyBorrow` did **not** fire: with A2, `f.returned()` is
`["b"]` (a borrow of a VISIBLE param), and `h`'s return of `f(Box{..})` never reaches the vector
delivery at all — `block_result` classifies it as a **non-delivering** return (emits
`OpFreeRef(subject); return null`, block dep bare). So A2's ABI change (buffer-fill → borrow-of-param
signature) breaks the caller's **return-TYPE resolution / value delivery**, not just its delivery
routing. The real A2 work is the caller-ABI adaptation at that resolution layer (make a caller that
returns/binds a borrow-of-param-of-temp result materialise it) — a genuine cross-function ABI pass,
now precisely scoped. Both prototype attempts reverted clean; repro shape:
`a2_{named,tempbind,escapetemp}.loft`. **Not a binding-site foundation** (bindings are safe) — a
return-type-resolution + delivery adaptation for f's borrow-of-param ABI.

### W3 — Wasted empty-buffer alloc  ·  P1-opt · S · rides W2's ABI

**Verified state:** CONFIRMED. A borrowed-alias return still takes + `OpClearVector`s a
`__retbuf` it discards. Removing it **is** the slice-1 ABI change (borrowed-view returns drop the
`__retbuf` param), so it lands with/after W2 — correct-as-is today.
**Exit:** borrowed-view fn signatures carry no `__retbuf` param; no clear of a discarded buffer.

### W4 — O-Complete: analysis totality  ·  **✅ DONE — already CLOSED 2026-07-04 (formal D-own-2)**

**This item was ALREADY CLOSED before this session** — my earlier "open / needs oracle infra" read
was **stale** (I hadn't checked the formal register). Per `doc/claude/formal/ownership.md:187` +
`ROADMAP.md:32`: **D-own-2 (O-Complete / completeness) CLOSED 2026-07-04 (@PLN90)** — the ownership
fact is TOTAL. Specifically: (1) `ownership_of` (use_analysis.rs) computes an `Own` for **every**
`Value` — the `_ => Owned` tail is **not a hole** (it covers only literals / scalar-void ops /
payload-less control, which ARE fresh-owned or heap-irrelevant, verified against the classifier);
(2) the free side reads that one fact (`scan_set`'s `ref_rhs_ownership`), and the three-valued gap
`RefRhs::Unknown` is DELETED (dead once the oracle covers every value); (3) the inherently-runtime
JOIN is completed per-path at runtime (`_own_store` witness, loft#495). Validated: suite 2601/2601,
native_scripts, poison, fuzz-gate controls, **@PLN89 differential oracle** (confirmed green locally
this session — `cargo test --test differential_oracle`), fuzzer. **O-Complete is total and closed;
the register's OWNERSHIP row is 0-open.**

### W5 — Enforced copy lint (the plan's namesake)  ·  Phase 2 · **✅ CHANNEL BUILT (2026-07-06, gated off)**

**Landed (gated `LOFT_WARN_COPIES`, default OFF):** `use_analysis::warn_copies` routes every
**Avoidable** survival-split copy through the normal `Level::Warning` diagnostics channel
(`diags.add_at`), so it surfaces as a compiler warning with the copied type + the `&`/restructure
hint. Called from `main.rs` after parse (before the diagnostics render); the `survival_on` gate now
includes `warn_copies_enabled()` so the split is computed under the lint too. `Forced` /
`Implicit` / `Eliminated` stay silent — only the actionable set warns. Default OFF is byte-identical
(short-circuit). Test: `tests/use_analysis.rs::warn_copies_enforced_lint_gated` (on = warns, off =
silent); suite 29/0.

**S5.2 (precise locations) — DONE (2026-07-06).** The walk now captures `Value::Line` markers into
`cur_pos` as a coarse fallback (empty `file`, filled with the source path by `warn_copies` /
rendered as `line N` by the report) — so survival copies that sit under no span still get a line.
Both the lint warnings and `--report-copies` now show the copy's line (was `<location unknown>`).
Gated on `track_pos` → default path byte-identical.

**Remaining before default-on:** the flip waits on W2/A2 draining the Avoidable set (else it
over-warns on copies the compiler is about to stop making). **Report** (`--report-copies`) was
already built (#510).

### W6 — par-dispatch native E0308  ·  P0 · S · **✅ DONE (2026-07-06)**

**Landed:** `tuple_arg_prep` (`src/generation/ops/parallel.rs`) gained a `Type::Function` arm —
reads the `i32` fn-index at offset 0 of the element and pairs it with `DbRef::NULL`
(vector-stored fn-refs are non-capturing), yielding the `(u32, DbRef)` tuple the worker expects.
Note refined during the fix: **`par` compiles its worker to native under *both* backends**, so the
E0308 blocked the interpret par-path too (not native-only). Guard:
`tests/scripts/507-par-vector-fnref.loft` (single + multi fn-refs, runs under both backends);
native_scripts + wrap `loft_suite` + the three `p4d_a2` tests green.

**Verified state (before fix):** native emitted a bare `DbRef` where `(u32, DbRef)` is expected
(`error[E0308]`): `tuple_arg_prep` had **no `Type::Function` arm**, so a `vector<fn-ref>` element
fell through to `("", "elm")`.
**Build steps:** add a `Type::Function(_,_,_)` arm to `tuple_arg_prep` that builds the fn-ref
tuple from the element record, mirroring the working for-loop unpack
(`tests/generated/issues_p4d_a2_vector_fn_ref_for_loop.rs:334`) — read the fn-index `i32` at
offset 0 of `elm`, emit `(idx as u32, DbRef::NULL)` (vector-stored fn-refs are non-capturing), pass
`_p`.
**Depends on:** nothing. **Exit:** `par_fnref.loft` compiles + runs `--native`; un-ignore /
un-timeout the native side of `p4d_a2` (`tests/issues.rs:11806`).

### W7 — Phase 3: explicit copy-intent syntax  ·  **✅ RESOLVED — DEFERRED (correctly the not-yet-active final phase)**

**W7 is the plan's Phase 3 — by design the LAST phase, dependent on Phase 2 (the enforced lint)
being active — and it should NOT be built now.** Three reasons, two of them established THIS session:
1. **Its prerequisite isn't active.** Phase 3's whole job is "opt into a copy and **silence the
   lint** at that site." W5's enforced lint is gated OFF (default-on deferred). There is nothing to
   silence yet — the plan's own "Open questions" defers the Phase-3 syntax until the lint is a
   default warning.
2. **Its silencing premise is undermined by the A2/#415/C86 finding.** The copies the lint flags as
   "Avoidable" (construct / field / record survival copies) are, per #415/C86, **contract-Forced**
   (whole-value binds copy by contract, `af=bx.v` — "#415 is the SEMANTIC"). So they are not
   avoidable and should not warn — leaving `copy <expr>` almost nothing to silence.
3. **Its force-a-copy semantics is largely redundant** in loft's model: whole-value binds already
   COPY (#415/C86), so `copy x` rarely changes lowering; and reserving `copy` as a keyword is a
   **permanent language commitment** — a deliberate design decision that should await a
   *demonstrated* need, not be minted speculatively.

**Resolution:** deferred as the correct final phase — not a blocker, not current-priority work; its
value is reduced by this session's contract findings. If a real need arises once the lint is
default-on, the build recipe (below) mirrors the `&` machinery. **Not open work.**

**Build recipe (if ever pursued):** design only — COPY_DIAGNOSTICS.md:234-244, :332-334: "the inverse
of `&` — opt into an independent copy, silence the report at *that* site; sparse/per-site, never a
global `allow`".
**Build steps (mirror the `&` machinery):** proposal — a **prefix keyword `copy <expr>`** (the
exact mirror of `&`; a method `.copy()` collides with user methods). Add `"copy"` to `KEYWORDS`
(`src/lexer.rs:138`); detect it beside `&` in `parse_operators` (`src/parser/operators.rs:493-497`),
setting a one-shot `copy_pending` flag (twin of `amp_pending`, declared `src/parser/mod.rs:184`); at
the bind-site decision (`src/parser/expressions.rs:~1236`) force a materialise even where the
default borrows; stamp the copy's `VerdictRow.class` as intended (Implicit) so **W5's lint stays
silent** there.
**Depends on:** **W5** (its *silencing* half has nothing to silence until the lint fires).
**Exit:** `copy expr` forces an independent copy both backends; the lint is silent at that site; tests.

### W8 — Empty-arm parse normalise  ·  **✅ DONE (2026-07-06)**

**Landed:** thread a `Type::Vector` result type into the block's value-tail (`parse_block:449` —
extend the enum-tail hint to vectors), so an empty `[]` arm is TYPED and materialises a **real empty
vector** instead of folding to `Void`. Result — the layout-sensitive fragility is GONE: BOTH
formattings now have **0 `return null`** and both build a real empty vector (was: single-line → a
`Null` `else ;`, multi-line → a broken `return null` fallthrough — a latent divergence where the
empty arm delivered a null vector that only read `len 0`). The fresh-vector-in-arm double `OpFreeRef`
this surfaces is **pre-existing** (a non-empty `_ => { [7,8,9] }` arm has it too — verified) and
POISON-idempotent, not introduced here. **Full suite green; POISON green; guard
`tests/scripts/508-empty-arm-real-empty-vector.loft` (both backends — checks the empty arm is a
*usable* empty vector, iterable, not a null).** Residual: the two formattings still differ in buffer
naming (`_mvcopy_1` vs `__retbuf`) — cosmetic, both deliver a correct real empty vector; the deeper
value-`if` vs statement-`if` unification is a separate non-fragility refinement.

**History — RE-SEQUENCED after W1/W2 (superseded by the fix above):**

**Verified state (2026-07-06, introspected — deeper than filed):** the divergence is **not**
cosmetic. Single-line `_ => { [] }` → return buffer `_mvcopy_1`, `jo_arm_copy` delivery,
`return if … else ;` (value-if, Null else). Multi-line → return buffer `__retbuf` **plus** an
extra `__vdb_1` scratch ref, the `if` demoted to a bare statement, then
`OpFreeRef(__vdb_1); return null` — the trailing `[]` in a block is parsed as a discarded
statement + a separate null value-return. Both still run clean both backends (the repro only
exercises the empty arm), so it stays robustness, not correctness.
**Why re-sequenced:** the two formattings pick **different delivery paths** (`_mvcopy_1`/jo_arm_copy
vs `__retbuf`/materialise) — the exact return-buffer-selection machinery W1/W2 rewrite. Normalising
it first would churn the code the blocker touches (violates "no simplify during debug"). Do it
**after** W1/W2, once the borrow-return delivery is unified, so both formattings converge on the
settled arm-value delivery.
**Root-caused (2026-07-06, introspected).** `[]` (an EMPTY vector literal) parses to an empty
`Value::Insert([])`, which `move_insert_elements` folds into the block as **nothing** — so the
`{ [] }` arm value collapses in BOTH formattings: single-line → a `Null` else (a value-`if`,
`jo_arm_copy` into `_mvcopy_1`), multi-line → an empty `{#block}` + a `return null` fallthrough (a
statement-`if`, materialise via `__vdb_1`/`__retbuf`). Both yield `len 0`; the delivery difference is
the surrounding `Value::Line` structure (multi-line inserts line markers) steering `block_result` down
the two paths. The trailing-`Line` pop (`control.rs:522`) already fires; the residual is the empty-Insert
fold + the leading line marker.
**Prototype attempted + reverted (2026-07-06) — the clean fix is deeper than type-threading.** Tried
threading the vector result type into the block's value-tail (`parse_block:449` — extend the enum-tail
hint to `Type::Vector`), so an empty `[]` arm is TYPED → materialises a real empty vector instead of
folding to `Void`. It **worked in part**: both formattings now deliver a real empty vector (the
multi-line's broken `return null` shape is gone), and the **full suite + POISON stayed green**. BUT it
**introduced a match-arm-specific double `OpFreeRef`** (the empty-vector build's cleanup + the arm's
materialise both free the backing store — a plain typed `fn e() -> vector { [] }` has 0 frees, so this
is new, not pre-existing). POISON proves it idempotent (the free machinery nulls the ref), but a
robustness fix that trades the fold fragility for a **double-free fragility** (a latent UAF if that
nulling ever changes) is a net wash, and the IR still isn't byte-identical. Reverted. The clean fix
must also resolve the arm-delivery/empty-vector-cleanup double-free and unify the two delivery paths
(`jo_arm_copy` vs materialise) — deeper delivery-machinery work.
**Original build sketch:** make an empty vector literal `[]` in a value block lower to an explicit
empty-vector VALUE (not a folded-away empty `Insert`), so the arm carries a real value in both
formattings and `block_result` picks one delivery. **Risk: HIGH** — touches empty-literal parsing ×
block-value × match/if-arm delivery; broad blast radius for **no correctness gain** (both run
correctly today).
**Depends on:** W1, W2. **Exit:** both formattings produce identical IR; suite green.

### W9 — Drain bucket 2 (grow the auto-elision set)  ·  north-star · L incremental · FOLLOW-ON, not a close gate

Extend the `Borrow`→`ElidePlan` engine to more Avoidable copies (var-buffer cases the analysis
can't yet prove; construction where the source provably out-lives a non-escaping record). Each
Avoidable row surfaced by W5 is a candidate. This is the **perpetual north-star worklist** — it
does **not** gate closing @PLN90 (else the plan never closes); track it as ongoing after close.

---

## Execution order & dependency graph

```
independent, start now ─┬─ W6  par-native E0308            (S, P0)
                        └─ W8  empty-arm parse normalise   (S–M, robustness)

borrow-ABI critical path ── W1  A1b UAF  (M, HIGH, BLOCKER)
                              ├─→ W2  A2 field→alias  (M) ─→ W3 wasted-buffer (S)
                              └─→ W4  O-Complete  (L)  [W1 + remove 2 fallbacks + oracle/fuzz]

lint thread (after W1+W2 drain Avoidable) ── W5  enforced lint (S–M) ─→ W7  Phase 3 copy-syntax (M)

follow-on (does NOT gate close) ── W9  drain bucket 2  (L, ongoing)
```

**Recommended sequence:** W6 + W8 first (cheap, independent, one is a P0) → **W1** (the blocker,
sequenced with care — #1-weakness machinery, gated + full-matrix + oracle) → W2 → W3, with W4
closing as W1 lands and the two fallbacks are removed → W5 → W7.

## Close criteria for @PLN90

The plan closes when: **P0** — W1 (A1b) closed ⇒ W4 (O-Complete) closed, and W6 (par-native)
closed (#462 already closed); **namesake** — W5 (enforced lint) shipped; **design complete** — W7
(Phase 3); **robustness** — W3, W8. **W9 (drain bucket 2) is explicitly a follow-on north-star, not
a close gate.**

## Effort roll-up

| item | effort | risk | status |
|---|---|---|---|
| W1 A1b UAF | M | **HIGH** | ✅ DONE (P0 blocker) |
| W6 par-native | S | low | ✅ DONE (P0) |
| W5 enforced lint | S–M | med | ✅ DONE (channel; default-on deferred) |
| S5.2 lint locations | S | low | ✅ DONE |
| W2 A2 field→alias | — | — | ✅ RESOLVED — WON'T DO (#415/C86: copy is the semantic) |
| W3 wasted-buffer | — | — | MOOT (rode A2's alias — the buffer holds the required copy) |
| W4 O-Complete | — | — | ✅ DONE — already CLOSED 2026-07-04 (formal D-own-2, oracle green) |
| W7 Phase 3 syntax | — | — | ✅ RESOLVED — DEFERRED (the not-yet-active final phase; premise reduced by A2/#415/C86) |
| W8 empty-arm parse | S–M | low | ✅ DONE (empty `[]` arm materialises a real empty vector; suite green) |
| W9 drain bucket 2 | L | — | not a close gate — remaining Avoidable are @C86-contract copies (per the A2 finding), no safe drain |

---

## Landable increments (small steps, each independently ship-able)

Each `S…` step is one landable slice — a few hours, its own verify boundary, commit + push
when green. The method (memory `analysis-first-instrument-gated`): **instrument the fact →
build it inert → apply gated → matrix both backends → suite → guard test.** Start with the two
independents (W6, W8), then W1.

**W6 — par-native (do first: P0, independent, ~S):**
- `S6.1` Check in the failing native repro (`par_fnref.loft` → `tests/scripts/`); confirm
  `--native` E0308, `--interpret` ok. *(instrument — prove the harness fails)*
- `S6.2` Read the working for-loop unpack (`tests/generated/issues_p4d_a2_vector_fn_ref_for_loop.rs:334`)
  to capture the exact tuple-build codegen. *(capture the answer)*
- `S6.3` Add the `Type::Function(_,_,_)` arm to `tuple_arg_prep`
  (`src/generation/ops/parallel.rs:157-211`): offset-0 `i32` → `(idx as u32, DbRef::NULL)`. *(apply)*
- `S6.4` Rebuild native; repro passes both backends; un-ignore/un-timeout `p4d_a2`
  (`tests/issues.rs:11806`). *(verify + guard)*
- `S6.5` `native_scripts` + `issues` green; commit + push. *(ship)*

**W8 — empty-arm parse normalise (independent, ~S–M):**
- `S8.1` `introspect` both formattings; pin the two IRs in a golden. *(instrument)*
- `S8.2` Locate the empty match/if-arm parse; choose the canonical form (empty-block). *(analysis)*
- `S8.3` Normalise both formattings to the empty-block IR. *(apply)*
- `S8.4` Re-introspect → identical IR; suite green; guard cell carrying both formattings. *(verify)*

**W1 — A1b UAF (the blocker — instrument-first, ~M, HIGH risk):**
- `S1.1` Promote `cell-escape-temp.loft` to a checked-in POISON matrix (named/temp/escape ×
  value+len+leak); confirm temp/escape fail loud both backends under `LOFT_POISON`. *(instrument — prove failure)*
- `S1.2` Hand-write + prove the clean separate-buffer materialise IR for the `temp` cell against
  the live tree (re-verify `A1b-TARGET-escape-temp.txt`). *(capture-and-diff)*
- `S1.3` Add the suppression predicate in `chain_site_set_shape` (callee returns borrow-of-param
  AND that param's arg == reuse var `w`) as a **log-only** boolean (env-gated); verify it fires
  ONLY on temp/escape, nowhere else in the suite. *(build the fact inert)*
- `S1.4` Behind `LOFT_JOIN_OWN`, on suppression: keep `w`, allocate a separate `out`, emit
  `OpClearVector(out); OpAppendVector(out, call)`, move `OpFreeRef(w)` after the append. *(apply — interp)*
- `S1.5` Confirm the native materialise gate (`dispatch.rs`) hits the distinct-store copy branch;
  no double-alloc. *(apply — native)*
- `S1.6` Matrix: 6 cells value+len+leak both backends under `LOFT_POISON`. *(verify)*
- `S1.7` Suite gate-on (issues/leak/native/wrap/native_scripts) + @PLN89 oracle; byte-identical
  gate-off. *(verify)*
- `S1.8` @PLN93 interaction check; guard `tests/scripts/85-temp-subject-borrow-return-uaf.loft`;
  flip default-on / fold the gate. *(ship)*

**W2 — A2 field→alias (after W1, ~M):**
- `S2.1` `introspect br_field.loft`; pin the current copy IR both backends. *(instrument)*
- `S2.2` Route `tail_is_struct_field_read` in `classify_vector_delivery` (`control.rs:1082/1109`)
  → `Delivery::Rename` (alias) instead of `CopyBorrow`, gated. *(apply)*
- `S2.3` Drop the `__retbuf` fill for the borrowed-view return (option (c)) — **this is W3**. *(apply)*
- `S2.4` Matrix (subject-outlives/temp/escape) — W1's materialise catches the temp case; both
  backends. *(verify)*
- `S2.5` Suite + guard; flip. *(ship)*

**W4 — O-Complete (spans W1; fallback-removal steps, ~L):**
- `S4.1` Enumerate reachable `_ => Owned` catch-all sites in `ownership_of`; debug-assert/log when
  hit on a borrow-return path. *(instrument)*
- `S4.2` Replace each reachable catch-all with an explicit typed classification; close the
  `r = x` value-vs-bind gap. *(apply)*
- `S4.3` @PLN89 oracle + fuzz gate green across the borrow-return corpus. *(verify)*
- `S4.4` STABILITY_ROADMAP `return-bind-ownership` → CLOSED. *(ship)*

**W5 — enforced lint (after W1+W2, ~S–M):**
- `S5.1` Add `warn_copies_enabled()` in `keys.rs`, default OFF. *(scaffold)*
- `S5.2` Resolve `VerdictRow.loc` to real source spans (the location gap). *(build the fact)*
- `S5.3` For each Avoidable row → `diags.add_at(Level::Warning, "copy", loc, msg+hint)`; Forced =
  info; Implicit/Eliminated = silent. *(apply)*
- `S5.4` Golden tests (`--report-copies` + warning output); corpus warning-count = Avoidable set. *(verify)*
- `S5.5` Once Avoidable near-zero (post W1/W2), promote `warn_copies_enabled()` default-on; doc. *(ship)*

**W7 — Phase 3 copy-syntax (after W5, ~S+M):**
- `S7.1` Add `"copy"` to `KEYWORDS` (`lexer.rs:138`) + a lexer test. *(scaffold)*
- `S7.2` Detect `copy` beside `&` in `parse_operators` (`operators.rs:493-497`); set `copy_pending`
  (twin of `amp_pending`, `mod.rs:184`). *(parse)*
- `S7.3` At the bind-site (`expressions.rs:~1236`), `copy_pending` forces a materialise even where
  default borrows. *(apply)*
- `S7.4` Stamp `VerdictRow.class = Implicit` at a `copy`-site so W5's lint is silent there. *(integrate)*
- `S7.5` Tests both backends: forces an independent copy; lint silent at that site. *(verify)*
