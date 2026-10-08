<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 phase B — scope: drain the copies the compiler can actually eliminate

Phase A (the survival split + the `--report-copies` report) is landed and gated. Phase B is the
first CODEGEN phase — it removes copies, not just reports them. This scopes it, and it opens with
a finding that **re-frames what phase B is**.

## The finding that reframes phase B (read first)

The plan said phase B = "drain the **Avoidable** worklist." Grounding that against @C86 changes it.

- **@C86 (the law, [OWNERSHIP_MODEL.md § The law](../../OWNERSHIP_MODEL.md)):** a whole-value heap
  bind COPIES (own store, value semantics). The compiler may **elide the copy to an alias/move
  ONLY when the source is provably DEAD afterwards** — the rustc last-use rule; `ElidePlan` *is*
  this analysis. A live source is copied; a borrow of a live source needs the user's explicit `&`.
- **The survival split's `Avoidable` bucket is SURVIVING sources** (16/17 are `S { f: src }` where
  `src` is read after the copy). Under @C86 those are **not compiler-elidable** — the source is
  alive, so the copy is the value-semantics contract; only `&` removes it, and that is the
  **user's** call (which the report already hints: "a `&` borrow … would remove these").
- **The copies the compiler CAN eliminate are the LAST-USE ones** — the survival split labels them
  `Implicit` ("move: source consumed"), but **they still physically copy at runtime.** Probe:
  `a = Item { tags: base }` with `base` dead afterwards →
  `LOFT_COPY_DUMP=1` shows `[copy] vector-append elements=3` + a free of `base`. The last-use
  elision that @C86 sanctions is **implemented only for the var-buffer idiom** (`v = buf; v += src`,
  `use_analysis::ElidePlan` + `scopes::elide_borrows`), **not for construction / record copies.**

> **So phase B is: implement the @C86 last-use MOVE-elision for construction (`S { f: src }`) and
> record (`v[i] = e`) copies** — transfer a dead source's store into the field/element instead of
> copy-then-free. It is @C86's own rule, already the *semantic* (the split calls it a move), only
> the *lowering* lags. The `Avoidable` worklist is a **user `&`-hint**, not a compiler-drain set.

## The two workstreams

### B1 — last-use move-elision (the compiler's job, C86-sanctioned) — the phase

Target: a construction / record copy whose source is a **named var, dead after the copy** (the
split's `move: source consumed` rows) that today lowers to
`OpDatabase(field-store) + OpCopyRecord/OpAppendVector(field, src) + OpFreeRef(src)`. Rewrite to
**transfer** `src`'s store into the field/element (a move) — no fresh alloc, no element copy, no
separate free. This is the same *ownership-transfer* class as the shipped var-buffer `ElidePlan`,
applied to two new shapes.

- **The fact already exists** — the survival split computes last-use (`last_use_pos`,
  `mut_max_pos`, `loop_entry`) and already classifies these as moves. B1 consumes that fact; it
  does not need new analysis, it needs new *lowering*.
- **Both backends** — interp (`state/codegen.rs`) and native (`generation/`) are separate
  generators; a move-elision must land in both and be matrix-clean on both (the both-backends
  rule).
- **The heap invariant is the risk** (loft's #1 weakness). A move that's half-done double-frees
  (field-free + source-free both run) or leaks (neither). Every cell asserts **value + length +
  leak + poison** on both backends before landing.
- **Effort: M–L.** Mechanically it reuses the ElidePlan philosophy, but it's real codegen in two
  generators touching store lifetime, so it earns the full boundary-matrix + gate treatment.

### B2 — the Avoidable `&`-hints (the user's job, already served) — NOT a compiler drain

The report's `Avoidable` rows are correct **as a hint**: "a live source is copied here; add `&` to
borrow instead." That is @C86-compatible and needs no codegen. The only change worth considering:
the survival reason says "a borrow/**move** would avoid this copy" — for a *survivor* a move is
unsound (the source is alive), only `&` applies. Tighten the reason to "add `&` to borrow" so the
report does not imply an auto-elision the model will not do. **XS, optional.**

## The design decision (recommendation, not a blocker)

**Recommended framing:** phase B = **B1** (last-use move-elision), with the Avoidable worklist
staying a **user `&`-hint** (B2). Do **not** have the compiler auto-elide *surviving*-source copies
into field-borrows — that would extend @C86 to let a struct field / vector element hold a live
borrow of a source, coupling the struct's lifetime to the source and re-introducing the
alias-surprise @C86's value-semantics copy exists to prevent. Field-borrow-of-a-live-source is
exactly what the explicit `&` is *for* (the user's decision), not an implicit optimization.

If the maker instead wants implicit field-borrows-of-survivors, that is a **separate, larger
ownership change** (a new borrow-analysis proving the struct never outlives the source and neither
is mutated independently) — flag it as its own plan, gated on that design, not folded into B1.

## Verifiable slices for B1 (each: matrix on BOTH backends, gated)

1. **Gate (capture the working move).** Hand-write / find the WORKING move bytecode for
   `a = S { f: base }` (base dead) — the field-store IS base's store, base is not freed, one owner.
   Capture it beside the current copy-then-free (`loft introspect`), both backends. The diff is the
   spec ([loft-codegen skill](../../../../.claude/skills/loft-codegen/SKILL.md)).
2. **Detect the elidable site.** A construction/record copy whose source is a dead-after named var
   — reuse the survival `move` classification (promote it from a diagnostic row to an elision
   decision, as `ElidePlan` is for the var-buffer case). Gate behind a flag; suite byte-identical
   off.
3. **Rewrite — construction first, then record.** Interp then native. Transfer the store; drop the
   alloc/copy/free. `v[i] = e` (record, the 1 Avoidable + the moves) after `S { f: src }`.
4. **Boundary matrix.** `{construction, record} × {dead source, surviving source (must NOT elide),
   source mutated, nested field} × {interp, native}` — assert value + length + **leak + poison**
   per cell. The surviving-source cells prove B1 does NOT touch what @C86 keeps.
5. **Graduate + flip.** Guards to `tests/scripts/` + `tests/leak_cases/`; flip the gate default-on
   once every cell is green both backends; keep an opt-out. Re-measure the survey (the move rows
   should now show 0 runtime copies via `LOFT_COPY_DUMP`).

## Do-not-ship (revert, don't push through)

- Any cell double-frees or leaks on either backend → the transfer is wrong; revert (heap invariant).
- A **surviving-source** cell elides (a field borrows a live source) → B1 overreached into B2's
  territory / broke @C86 value semantics; revert.
- One backend moves, the other still copies → not landable (both-backends rule).

## Relationship to sibling work

- Reuses the **@C86 last-use elision** philosophy (`ElidePlan` / `elide_borrows`) — B1 is that
  analysis applied to two shapes it doesn't yet cover.
- Adjacent to the **@PLN85 P4 borrow-return** ([borrow-return/DESIGN.md](borrow-return/DESIGN.md)):
  that eliminates the *return*-buffer copy by borrowing a field on the way OUT; B1 eliminates the
  *construction* copy by moving a dead source IN. Same store-lifetime engine, opposite direction.
