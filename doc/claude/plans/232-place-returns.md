<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 232 — A function returns a place: `-> &T` accessors

## Status

Needs approval ([@PLN232](https://github.com/loft-lang/plans/issues/232), from loft#2659).  The owner
rules on § Open design questions before anything is built.  Nothing is implemented.

## Goal

A function may return a place inside an argument, and the call is then that place wherever it
is used:

```loft
fn first(s: State) -> &Enemy { s.enemies[0] }
hurt(first(s), 10);        // lands, as hurt(s.enemies[0], 10) does
first(s).hp = 3;           // lands, as s.enemies[0].hp = 3 does
```

## The invariant

> **A call of a `-> &T` function IS the place its body returns, each parameter replaced by its
> argument — in every position a place can stand.**

**What is refused: only what would introduce a problem** (owner, 2026-11-27: *"we resolve all
cases that do not introduce problems"*).  A shape the rule can answer soundly is accepted, however
it is written; a refusal names the one fact that makes the place unsound to hand back.

Every case below follows from that rule.  A program never written in a test answers as its
place spelling does, for the same reason.  The rule reuses what places already mean
(`@FR-B-View`, `@FR-B-Copy`, `@FR-B-Ref-Alias`, `@FR-B-Ref-Reshape`, `@FR-B-Disturb`) instead of
adding a second meaning.  `(F-Ret)` already states the exception
(`formal/calls.md`: *"a `&T` return … is an explicit borrow of an addressable input"*), and
[@C77](../DESIGN_DECISIONS_OWNERSHIP.md)'s 2026-11-06 ruling admits *"what a Rust programmer
writes with `&mut` … in its `&` spelling"*, which a `fn first(&mut self) -> &mut Enemy` accessor is.

**The spec, measured on main's tree today with the place spelling** (`--interpret` and
`--native`, the same answer on each):

| Use | Place spelling today | `first(s)` today | With the lift |
|---|---|---|---|
| a writing callee: `hurt(·, 10)` | 90 | 100, `lost-write` | 90 |
| a field write: `·.hp = 3` | 3 | 100, **no warning** | 3 |
| a method chain: `s.first().hp -= 10` | 90 | 100, **no warning** | 90 |
| a plain bind of a record: `e = ·; e.hp -= 10` | 90 (`@FR-B-View`) | 100 (copy) | 90 |
| the bind, then the container grows | the view materialises | (copy) | materialises |
| a link: `e = &·` | link (90) | refused | link |
| a link, then the container grows while it is live | refused at compile time | refused | refused |
| a link, then a grow after its last use | runs | refused | runs |
| a scalar: `x = v[1]; x += 5` | copy (`[1,2,3]`) | — | copy; `&at(v, 1)` links |
| a `const` parameter's place, written | refused at compile time | — | refused |
| a temporary root: `e = &mk().enemies[0]` | refused at compile time | — | refused |
| a temporary root, passed: `hurt(mk().enemies[0], 10)` | the write goes into a fresh value: nothing the caller reaches is lost, and `lost-write` rightly stays silent | — | the same |
| branches into ONE parameter: `if c { s.enemies[0] } else { s.boss }`, match arms, an early `return` | the return records `Deps [0]` | — | accepted (Q4): the caller pins every container a branch reaches |
| a link into `s.enemies`, then `s.others` grows | runs (90): pinning is per container | — | the same |
| places in TWO parameters: `if c { a.boss } else { b.boss }` | the return records `Deps [0, 1]` | — | refused (Q4) |

The two **no warning** rows were a defect on main (phase 0, fixed apart from the lift): a write
into a call result's field was lost with no warning, and so was a METHOD accessor's result handed
to a writing callee (the lint read only a free function's result).

## Impact analysis

**Language surface.**  This is an addition: a refusal is lifted, and no program that compiles
today changes meaning (`-> T` keeps `(F-Ret)`'s fresh copy).  It is no contract or freeze break.
The refusal's wording changes (it keeps fields, elements and tuple members), and
`tests/parse_errors.rs` pins that text.

**The compiler — record places (`-> &Struct`).**  The type shape already exists.  A local record
link `e = &s.enemies[0]` is a `Reference(E, deps=[s])` view plus the per-variable `amp_link`
flag, and a call returning a part of parameter 0 is already typed `Reference(…, Deps{[0]})`
(`(O-Move)`'s record, `control.rs` `ref_return`/`borrowed_param_deps`).  The work:

- **Parse.**  `refuse_link_type` (`parser/definitions.rs`) covers every position at once.  It is
  split so a return (`parse_type_full(…, returned = true)`) takes the `&` and the rest still
  refuse.  Function, method and interface signatures share the `->` parse.
- **The callee.**  A `-> &T` whose returned place names no parameter (a local, a fresh value, a
  literal) is refused at compile time: there is no place to hand back.  `(B-Ref-AnnotationOnly)`
  keeps `return &a` refused, so the body's tail is the plain place.
- **The caller's bind.**  `bind_lowering::first_arm` chooses `CallCopy` today.  A `&` callee takes
  a view arm instead: keep the deps and skip the free.  `use_analysis::view_elision_bind`
  (`scopes/scan_set.rs` `mark_view_elided`) is that arm already, read-only, for a call borrowing
  a value-const argument.  `e = &first(s)` admits a `-> &T` call as an addressable operand.
- **Dependencies.**  `call_args.rs` `call_dependencies` maps callee-space deps onto the caller's
  arguments.  It has no arm for this and passes them through verbatim, the loft#938/#1335 class:
  **the main correctness risk**.
- **Reshape and disturb.**  `scopes/view_walk.rs` `is_place_link` and `scopes/places.rs`
  `value_view_places_depth` treat a call as a mint, so naming no container.  They must map a
  `-> &T` call to its argument's place, or the refusals and the materialisation never see the
  link.  `@FR-B-Ref-Reshape` has 12 citing sites and `@FR-B-Disturb` 23.
- **Lints.**  `lost-write` (`use_analysis.rs` `warn_lost_temp_writes`, `report_lost_field_writes`)
  skips a `&` callee, and its fix text names `-> &T`.  `double-move`
  (`report_returned_parameter_member`) must not fire for it.  The needless-`&` parameter lint
  counts a parameter a `-> &T` names as used.
- **Store records** (`@FR-O-Move`).  A `-> &T` callee is the `Arg`/`Slot` summary
  `store_records.rs` already has.  The bind is read as a view, with no release (`eval.rs`'s
  `first_arm` reading).  The store-lifetime rule holds: any defect found is fixed in the
  compiler, the records and the checker together (CLAUDE.md).
- **Return delivery.**  A record `&` return takes no `__retbuf` (it delivers nothing).
  `place_result.rs`, `opt/ret_adopt.rs` and `forward_result.rs` already decline borrowed
  returns; each is rechecked against the new arm.
- **The backends.**  A record `&` return is a `loft_ref` in C and a DbRef on the interpreter,
  as a `-> Struct` is, so there is no new emission shape.

**The compiler — scalar and text places (`-> &integer`, `-> &text`)** are a different job:

- They need a real `RefVar` return, which neither backend emits: `c/mod.rs` `Cx::ctype` has no
  `RefVar` arm, and the interpreter's `gen_return` sizes a 12-byte cell no caller expects.
- A `&text` link has a KIND, a text variable or a store field/element (`text-link-kind`,
  `parser/places.rs`).  A returned one would carry its kind in the signature, or need a twin per
  kind as `&text` parameters have.
- Hence phase C, behind its own question (Q5).

**The self-hosted front end.**  `selfhost/src/syntax.loft` `fn_ret` hard-codes `amp: false`, and
`ty` has no `&`.  The ports of `returns_borrowed_view`, `borrowed_param_deps`, `set_amp_link`
and the call-deps mapping (`el_retpromo.loft`, `el_ex_assign.loft`, `el_ex_call.loft`) mirror
the Rust change.  Gate E counts the new cells as refused until the port follows; that is the
porter's stream.

**Tests that change:**

- `tests/scripts/a-link-type-outside-a-variable-is-refused.loft`: its return cell flips, and
  the element and tuple-member cells stay.
- Its patch receipts, `tests/falsified/a-link-*.patch`, re-score.
- `tests/parse_errors.rs` `link_field_type`: the message text.
- These stay as they are and are run against the change:
  - the `(F-Ret)` copy cells (`loft-suite rules/calls/F-Ret/2585*`,
    `…/a-parameter-returned-on-any-of-several-paths…`);
  - loft#894's attic guard, `894-lost-write-through-returned-struct.loft`, which keeps `-> E`.

**Consumers and libraries:**

- dryopea:
  - its LOFT_GOTCHAS.md § Values, copies and tail calls says a get-the-element helper is "fine
    to read through and never to write through";
  - it carries 239 inline `state.enemies[i] ?? Enemy {}` uses;
  - 12 files cite loft#894.
  - Migrating them is its agent's choice once this ships.
- moros: 7 read-only inline uses.
- The `loft-libs-*` repos: none (they use local `&` links), so no library republishes.

**Cost.**  A `-> &T` call removes the copy an accessor pays today.  Every other program must
emit byte-for-byte what it does now: the phase gates below prove it.

## Phases

| Item | What | Verify (goes red if done wrong) | Effort |
|---|---|---|---|
| **0** | The silent lost writes (a defect on main, fixed whatever the ruling): `first(s).hp = 3`, `s.first().hp -= 10` and `hurt(s.first(), 10)` get `lost-write` | **Done** on `tuxedo-pr2657-next`: `loft-suite rules/calls/F-Ret/a-write-into-a-call-result-is-lost-and-says-so.loft`, which fails on the build without it (0 reports, 4 with it); every corpus program's warnings otherwise unchanged | S |
| **A** | The spec as cells: the table above, one cell per row under `@FR-B-RefType` (or Q6's rule), each with its place-spelling twin as the control, written before any compiler edit and marked expected-to-fail | the twins pass today, and each `-> &T` cell fails for the reason named (refused) | XS |
| **B1** | Parse a return `&`, and refuse a `-> &T` whose places name no parameter, or more than one lifetime (Q4) | B1 refusal cells (a local, a fresh value, a literal, two parameters' places) fail without the callee check, and the accepted branch cells (one parameter, several containers) pass; every other program's IR byte-identical (`introspect_diff.sh`, both writers) | S |
| **B2** | A `-> &T` call in place positions: an argument, a field write, a method chain | A's cells for those rows pass on both backends, leak- and strict-checked; byte-identical elsewhere | M |
| **B3** | Binds: `e = first(s)` as a view, `e = &first(s)` as a link, the dependency mapped onto the argument, and the reshape refusals and materialisation reaching it — for a branching body, the union of the containers its return sites reach | A's bind and grow rows; `ownership_drop_gate` on both backends; the poison and stack sweeps; `own_sweep.py` reads CAUGHT with the new arm switched off | M |
| **B4** | Lints and records: `lost-write` and `double-move` skip a `&` callee, the needless-`&` lint, the store-records reading | a cell per lint proving silence on `-> &T` and the warning kept on `-> T`; the records checker matches the runtime on A's cells | S |
| **C** | Scalar and text places (`-> &integer`, `-> &text`): a `RefVar` return on both backends and the text kind in the signature, if Q5 says yes | A's scalar and text rows; the C sweep in strict mode on clang | M–MH |
| **D** | The self-host port mirrors B (and C) | gate E agrees on A's cells; no agreeing program lost | S–M |

Each B phase keeps the old path for every `-> T` program, compared exactly: the byte-identical
emission diff is the parallel run.

## Open design questions

Each comes with the recommendation, and with what was measured behind it.

1. **Lift or drop.**  This plan is the lift.  Recommended: the rule, @C77's ruling and a consumer
   all ask for it.  The alternative is a one-line edit taking the exception out of `(F-Ret)`.
2. **A plain bind `e = first(s)`.**  Recommended: as the place binds, so a VIEW for a record
   (`@FR-B-View`, 90 today with `e = s.enemies[0]`) and a COPY for a scalar or a text (`x = v[1]`
   copies today), with `&` for a link in every case.  The alternative, a link for every bind,
   makes one call spelling mean two things depending on how the function was declared.
3. **A temporary argument (`first(mk())`).**  Recommended: as its place.  `e = &first(mk())` is
   refused (as `&mk().enemies[0]` is), and the plain bind and the argument answer as
   `mk().enemies[0]` does: a write into it loses nothing the caller can reach.  No new rule.
4. **Branches.**  **Decided (owner, 2026-11-27): accepted as long as every place the body can
   return lives in the SAME lifetime; refused only when they do not.**  Branches, match arms and
   early returns into one parameter (`if c { s.enemies[0] } else { s.boss }`) are accepted.  The
   caller's reshape check pins the union of the containers the return sites reach (`s.enemies`
   for the first arm; `s.boss` is inline, so its disturbance is overwriting `s.boss`), which is
   exact per container as a local link is (measured: growing `s.others` under a link into
   `s.enemies` runs).  Places in two parameters (`if c { a.boss } else { b.boss }`) are refused at
   compile time (phase B1); that two-lifetime variant is a feature of its own, to investigate if a
   program needs it.  The fact already exists: the return records `Deps [0]` for the first shape
   and `Deps [0, 1]` for the second, measured on both backends.  Phase A adds cells for an
   accessor that returns another `-> &T` accessor's place (one lifetime through two calls) and for
   a place reached through a stored `reference<T>` field (another store: its own lifetime).
5. **Scalar and text places (phase C).**  Recommended: build B first and C only when a consumer
   asks.  dryopea's case is records, and C carries the new emission plus the text-link kind.
6. **The rule's home.**  Recommended: a return clause in `formal/binding.md` beside
   `(B-RefType-OfVar)` (*"a function's return may be `&τ`: the call is the place it returns"*),
   cited at the parse, the bind and the reshape sites, with @C77 amended.  `(F-Ret)`'s exception
   then cites it.
7. **What may declare one.**  Recommended for B: named functions and methods (`self`).  A
   fn-ref type `fn(S) -> &E`, an interface method and a generic `-> &T` over a type variable
   are refused until a program needs them.  A generator's `&` is refused as its parameters are
   (`@FR-G-NoRefParam`).

## Cross-arc dependencies

- [@PLN231](https://github.com/loft-lang/plans/issues/231) found this (R10, probes 18–30; its
  branch carries D-call-32).  D-call-32 closes with phase B.
- The self-host port (@PLN91) owns phase D.
- loft#894 is dryopea's original report.  Its guard stays as the `-> T` control.

## Documentation

- `LOFT_DECLARATIONS.md`: the `&` ladder's "no `&T` return type" becomes the return form.
- `LOFT.md`: § the "a function return is a value" sentence gets the `-> &T` carve-out.
- `tests/docs/08-struct.loft`: a worked accessor (regenerated with `gendoc`).
- `formal/binding.md`: the return clause (Q6) and `(B-Ref-AnnotationOnly)`'s note.
- `formal/calls.md`: `(F-Ret)` cites it, and D-call-32 closes.
- `DIAGNOSTICS.md`: `lost-write`'s fix column names `-> &T`; the refusal gets a code if it
  becomes one.
- `@F21` on loft-lang/features (references `&T`): the return form, then
  `make features-fetch && make features-gen`.
- `CHANGELOG.md`: New.
- The `loft-write` skill: the accessor spelling, and the "write through a helper" trap struck.
- dryopea's LOFT_GOTCHAS.md is its own agent's to update.  We report the release to them.

## See also

- [formal/calls.md](../formal/calls.md) `(F-Ret)`; [formal/binding.md](../formal/binding.md)
  `(B-RefType)`, `(B-View)`, `(B-Ref-*)`, `(B-Disturb)`.
- [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md); [STORE_RECORDS.md](../STORE_RECORDS.md).
- [@C77, @C79, @C86](../DESIGN_DECISIONS_OWNERSHIP.md).
- loft#2659, loft#894, [@PLN232](https://github.com/loft-lang/plans/issues/232).
