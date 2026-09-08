<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/VERIFICATION.md — detailed verification worklist for the operational rules

The operational-semantics files written 2026-07-04 ([heap](heap.md), [iteration](iteration.md),
[coroutines](coroutines.md), [concurrency](concurrency.md), [calls](calls.md),
[matching](matching.md), [tuples](tuples.md), [closures](closures.md)) were each grounded in
**targeted probes** before writing — but a *targeted* probe is not a *systematic* one. Two
imprecisions were caught exactly by re-checking a written rule in detail:

- `I-Text` said "characters" — a detailed check (`"e" + U+0301`) proved it is **codepoints, not
  grapheme clusters**, and the rule was wrong-because-vague.
- `closures.md` claimed `|…|` is non-capturing — a detailed check proved it *should* capture
  (pure sugar), surfacing **D-clo-1** (fixed) and **D-clo-2** (fixed — the stored-short-lambda
  crash is now a clean diagnostic). Both closed 2026-07-04.

This worklist drives that detailed pass over **every rule**: for each, the single falsifiable
claim, its status on **BOTH backends**, and the standing guard (an [oracle](../../../tests/oracle/)
program or a `tests/scripts/` guard) that should keep it true. It is the concrete D-op-1 coverage
plan for the new rules — the oracle already guards each *area*; this drives it down to each *rule*.

## Method

1. **One falsifiable claim per rule.** Write the smallest program whose output *distinguishes* the
   rule holding from it failing (a value AND, where relevant, a length / a compile accept-vs-reject).
2. **Run it on BOTH backends** (`--interpret` AND `--native`) and hand-compute the expected value —
   agreement between the two is necessary but NOT sufficient (both could be wrong the same way);
   the hand-computed value is the oracle.
3. **Graduate it to a standing guard** — add it to `tests/oracle/*.loft` (both-backends +
   leak + driver-agreement, the nightly gate) or a `tests/scripts/*.loft` guard. A rule is
   "verified" only once a guard pins it.
4. **Static claims also need driver-agreement** — a rule that says "X is a compile error"
   (exhaustiveness, tuple-OOB, `&a;`) must be checked to REJECT on `--dump` / `--interpret` /
   `--native` alike (the D-op-2 facet).

## Legend

- ✓ — verified on both backends this cycle AND has a standing guard.
- ~ — probed once (usually interpreter only); needs the both-backends run + a graduated guard.
- ☐ — not yet verified in detail.

---

> **Stage 1–2 (2026-07-04) — the worklist earned its keep.** Two real findings: (1) **H-Copy was
> imprecise** — a struct-typed projection is a VIEW, not a copy (heap.md corrected: added
> `H-View`); (2) **coroutine laziness diverges** for a loop body of MORE than one statement (native eager) —
> reclassified as a DECIDED EDGE (a rustc restriction, the maker's accepted trade-off), not a bug.
> Everything else on the priority + runtime + static-reject rows verified ✓ on BOTH backends
> (F-ParamRebind incl. native, `par` N-independence, null/OOB-continue, range/reduce-left/map-fresh,
> and the static rejects — exhaustiveness / tuple-OOB / wildcard-not-last reject identically).
>
> **Graduated to standing oracle guards (nightly gate, both-backends + leak + driver-agreement):**
> `tests/oracle/24-heap-copy-vs-view.loft` (H-Copy/H-View), `25-parameter-binding.loft` (F-Param*),
> `26-coroutine-laziness.loft` (straight-line G-Call/G-Next). The full `--ignored` sweep passes.
>
> **Stage 3 — remaining rows, no new divergences.** Verified ✓ both backends: H-Alloc zero-init,
> H-WriteLocked (a `#lock`ed write FAULTS on both — the intended tripwire), F-Rec (factorial),
> F-Return (implicit tail), T-Paren (`(e)` is grouping), L-Escape (a returned closure works on
> native). Still ☐ / covered-elsewhere: **F-ParamRef** (`&` write-back — binding.md's domain, 0
> deviations, PR#436), **C-Order hash-par** (a documented edge; probe syntax pending),
> **G-YieldDepth** (stackful yield — likely needs `yield from`, deferred to 1.1+). (**D-clo-2**
> — the stored-short-lambda crash — is now CLOSED, see below.) The load-bearing rules are now
> pinned; the residue is edge/deferred.

## heap.md

- ✓ **H-Alloc / H-NewRec** — a struct's un-set field is `0`/null, both backends. *Guard: oracle `03`/`24`.*
- ✓ **H-Copy** — a WHOLE-VALUE / vector bind COPIES: `c = o; c.v=9` ⇒ `o.v==1`; `fv = e.items;
  fv[0]=99` ⇒ `e.items[0]==1`; `r=&v; r[0]=99` ⇒ `v[0]==1` (both backends). *Guard: `oracle/24`.*
- ✓ **H-View (the Stage-1 catch)** — a STRUCT-typed projection ALIASES: `c = o.i; c.v=9` ⇒
  `o.i.v==9`; `s = v[0]; s.v=9` (struct element) ⇒ `v[0].v==9` (both backends). heap.md corrected.
  *Guard: `oracle/24-heap-copy-vs-view`.* (A `value struct` INVERTS this — its projection COPIES;
  see [§ Value structs](#value-structs-pln101-2026-07-08).)
- ✓ **H-Copy (keyed)** — a hash whole-value bind COPIES (`g = h; g += …` leaves `len(h)`), both
  backends. *Guard: oracle `16` (keyed).*
- ✓ **H-Read / H-ReadNull / H-Index** — a field of `nullref` ⇒ `null`; `v[i≥len]` ⇒ `null`, both
  continue, both backends. *Guard: oracle `18` (nullability).*
- ✓ **H-Write / H-WriteNull / H-WriteOOB** — `v[9]=…` on a len-3 vector is a no-op, `len` stays 3,
  continues, both backends. *Guard.*
- ✓ **H-WriteLocked** — a `#lock`ed write FAULTS on both (the intended tripwire; `18-locks.loft`).
- ✓ **H-Free* + H-Sound** — the LIFO / no-stack / no-double-free discipline. *Standing guards:
  `LOFT_POISON` suite + the ownership fuzz gate + `LOFT_NATIVE_LEAK_CHECK` (ownership.md is 0 open).*

## iteration.md

- ✓ **I-For / I-Next / I-Done** — index order (map/for values agree, both backends). *Guard: oracle `13`.*
- ✓ **I-Range** — `2..5` ⇒ `2 3 4` (half-open), both backends. *Guard.*
- ✓ **I-Text** — CODEPOINTS not graphemes (`"e"+U+0301+"X"` ⇒ 3 iterations, `c#index=0,1,3`); byte
  cursor; `t.map` is a static error. *Guard: oracle `15`.*
- ✓ **I-Map / I-Filter / I-Reduce** — reduce folds LEFT (`[1,2,3,4].reduce(0,\|a,x\|a-x)==-10`,
  a non-commutative `g` pins direction); map/filter order agree, both backends. *Guard: oracle `13`.*
- ✓ **I-Comp** — the result is a FRESH vector (`ys = xs.map(…)` leaves `xs[0]`), both backends. *Guard.*
- ✓ **I-Empty / I-NullSrc** — a typed empty / null source iterates zero times, continues.

## coroutines.md

- ✓/edge **G-Call / G-Next laziness** — STRAIGHT-LINE yields are lazy on BOTH backends
  (`a g1 b g2`). A loop body of MORE than one statement is lazy on interp, EAGER on native — a
  DECIDED EDGE (rustc restriction, CL-9, [loft#836](https://github.com/loft-lang/loft/issues/836)),
  NOT a bug. The edge is one STATEMENT wide, not one construct wide: `for x in it { yield x }` is
  lazy on native too (the `YieldFrom` segment), verified over a 1e9 range. *Guard:
  `oracle/26-coroutine-laziness` (straight-line).* A record yielded from an EAGER loop body is
  a per-yield SNAPSHOT, so a read sees the value at the yield on both backends; a write through
  the consumer's binding lands in the copy where the interpreter's live frame would take it —
  the same edge one row deeper, not a second one. *Guard: `tests/scripts/1356-…`.*
- ✓ **G-Next values / G-Done** — one value per advance (sum 30); exhaustion (take-first-2 ⇒ 1),
  both backends. Nested CALL between yields works too. *Guard: oracle `12`.*
- deferred **G-YieldDepth** — a `yield` INSIDE a helper (true stackful) needs `yield from` (CO1.4,
  deferred to 1.1+). A nested non-yielding call between yields ✓.
- ✓ **G-For** — `for x in gen()` visits the produced sequence (both backends). *Guard: oracle `12`.*

## concurrency.md

- ✓ **C-Det** — `par(b=sq(a),N)` sum is `30` for N=1 AND N=4, equals the sequential loop, both
  backends. *Guard: extend oracle `14` with a multi-N assertion.*
- ✓ **C-Par** — `par` over a vector; result values agree with the sequential loop, both backends. *Guard: oracle `14`.*
- ✓/edge **C-Order** — hash-par values agree both backends (`(1+2+3)*2==12`); the walk-ORDER
  exception is a documented edge, unobservable under a commutative reduction. *(order-sensitive body: future guard).*
- n/a **C-Impure** — undefined by contract (a data race is a program error); not a testable rule,
  but a lint/doc check that an impure worker is discouraged.

## calls.md

- ✓ **F-Args** — arguments left-to-right (`add(tag("A"),tag("B"))` prints `AB`). *Guard: oracle `06`.*
- ✓ **F-Call / F-Return / F-Rec** — `fac(5)==120`, implicit tail `add(3,4)==7`, both backends. *Guard: oracle `17`/`21`/`25`.*
- ✓ **F-ParamScalar / F-ParamHeap / F-ParamRebind / F-Ret** — the by-type contract, all four
  verified BOTH backends (incl. the subtle @PLN87 P2.4 rebind on native). *Guard: `oracle/25-parameter-binding.loft`.*
- covered **F-ParamRef** — `&T` write-back is [binding.md](binding.md)'s ladder (L1–L6, 0 deviations,
  PR#436, its own tests). (The `&`-at-call-site is a type annotation, not an operator — no separate probe needed here.)

## matching.md

- ✓ **M-Match / M-Variant / M-Expr** — arm selection + payload bind + expression value (oracle `07`/`20`). *Guarded.*
- ✓ **M-Wild** — `_` matches any; an arm AFTER `_` REJECTS on both backends (driver-agreement). *Guard.*
- ✓ **M-Exhaust** — a missing variant REJECTS on both backends (driver-agreement). *Guard: oracle `19`-style.*

## matching.md — PEG patterns (@PLN35, SHIPPED)

Rules SHIPPED ([matching.md § Rules — PEG patterns](matching.md)) — phases 1–7 + PC1–PC5 landed
(350e660c #554, 3fda4e1e #558, 50cc4c18 #561, a37917ff #562) and pass on both backends via
`tests/scripts/35*.loft` (in the full suite). Each row below tracks the stricter @PLN89
DIFFERENTIAL-oracle pin (both-backends + leak + driver-agreement); ☐ = shipped + both-backends but
not yet graduated to that oracle. Design + phase↔rule map:
[plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

- ☐ **M-Total (the invariant)** — a match with a non-total pattern and no total final arm REJECTS on
  `--dump` / `--interpret` / `--native` alike (driver-agreement); adding a `_` makes it compile and
  always select an arm. *Pin: P0 falsifier `35-invariant-*.loft`.*
- ☐ **P-Point (L2 nested)** — `V { f: Inner { x } }` binds the nested field; both backends. *Pin: P1.*
- ☐ **P-Seq / P-Whole / P-Rest** — a sequence matches exact length; `[a,b,c]` REJECTS a length-4
  slice (D-F1 whole-consume); `..rest` binds the sub-slice by value AND length, leak-clean, and
  fixed elements MAY follow it (`t`), so `[a, ..mid, z]` leaves `mid` empty on a 2-element subject
  and does not match a 1-element one. *Pin: P2, plus
  `tests/scripts/a-named-rest-leaves-room-for-the-elements-after-it.loft` (D-match-3).*
- ☐ **P-Multi** — a multi-pattern arm binds from the first matching shape; both backends. *Pin: P3.*
- ☐ **P-Alt / P-Atomic** — ordered choice takes the first match; a partial-then-failed alternative
  leaves the cursor reset (the next element still matches); different-name captures ⟹ `τ?`. *Pin: P4.*
- ☐ **P-Opt** — present ⟹ bound, absent ⟹ null capture, cursor intact. *Pin: P5.*
- ☐ **P-Rep** — `(a)*` collects a `vector<τ>` (count + values + length + leak); `+` needs ≥1; a
  separator is consumed, not captured. *Pin: P6.*
- ☐ **P-Anchor / P-Revert / P-IterBound** — an iterator match backtracks via the memo buffer; bounded
  by `max_lookahead` (no hang); native parity via a custom `OpEmitter`. *Pin: P7.*
- ☐ **Capture typing (types.md § Pattern captures)** — alternation-unify (`⊔`), optional / absent
  (`τ?`), repetition / rest (`vector<τ>`) — NO new type former. *Pin: P4–P6 typecheck cases.*

## tuples.md

- ✓ **T-Cons / T-Proj / T-Destr / T-Ret** — construct + `.i` + `(a,b)=…` + tuple return, both
  backends (oracle `17`). *Guarded.*
- ✓ **T-Paren** — `(3+4)` is grouping, not a 1-tuple, both backends. *Guard.*
- ✓ **T-Proj OOB** — `t.5` on a 2-tuple REJECTS on both backends (static, driver-agreement). *Guard.*

## closures.md

- ✓ **L-Fn (both forms capture)** — `|y|{y+x}` and `fn(y){y+x}` both yield `11`; heap capture `9`;
  non-capturing `2` (both backends). *Guard: `tests/scripts/85-short-lambda-capture.loft` + oracle `04`/`22`.*
- ✓ **L-CapScalar / L-CapHeap** — scalar by value at creation; heap shared (`b.v=9`⇒`9`). *Guard: `85`.*
- ✓ **L-Apply / L-Escape** — return (`mk(7)()==7`) + struct-field (`h.f()==42`) work on native too. *Guard: oracle `04`/`22`.*
- ✓ **D-clo-2 (CLOSED 2026-07-04)** — `g=|y|{y*2}; xs.map(g)` no longer panics; parse_map guards a
  `void`/Unknown fn-ref return and emits the clean "cannot infer" diagnostic. *Guard: `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash`.*

---

## FINALIZED (2026-07-04)

**Every load-bearing operational rule is verified on both backends.** The three stages ran the
whole worklist; the only surprises were the two that a detailed pass is *for* — a doc correction
(**H-View**: struct-typed projections alias, not copy) and a confirmed **decided edge** (native
eager loop yields, CL-9). Everything else holds identically on `--interpret` and `--native`.

**Standing guards graduated to the nightly differential oracle:** `24-heap-copy-vs-view`,
`25-parameter-binding`, `26-coroutine-laziness` (+ the pre-existing 01–23). The full `--ignored`
sweep is green.

**Residue (resolved, none blocking):**
- **keyed-collection copy** — a hash whole-value bind COPIES (`g = h; g += …` leaves `len(h)`
  unchanged), both backends ✓ (oracle `16` guards keyed behaviour).
- **hash-par** — values agree both backends ✓; the walk-ORDER exception (C-Order) is a documented
  edge, unobservable under a commutative reduction.
- **F-ParamRef** — [binding.md](binding.md)'s domain (0 deviations, its own ladder tests).
- **stackful yield** — a nested CALL between yields works both backends ✓; a `yield` INSIDE a
  helper (G-YieldDepth) needs `yield from` (CO1.4, deferred to 1.1+).

**Still open after verification (not testable-to-close — they need action, not probes):**
1. **CL-9 / the coroutine decided edge** — native eager loop yields. Removal DESIGN written
   ([COROUTINE.md § lazy loop yields](../COROUTINE.md#design-lazy-loop-yields-cl-9)); needs the build.
2. **D-op-1 / D-op-2** — the differential-oracle meta-deviations; open BY DESIGN. "More testing" =
   growing the corpus, which this worklist did (3 new programs) and which continues.

(**D-clo-2** — the stored-short-lambda→`map` crash — was on this list; CLOSED 2026-07-04 with a
clean diagnostic + `dclo2_stored_short_lambda_map_no_crash` guard, verified on both backends.)

The newly-written operational rules are now not just *written* but *verified + pinned*. This file
stays as the standing per-rule ledger; each future rule addition gets a row and an oracle case.

---

## Value structs (@PLN101, 2026-07-08)

`value struct` — a copy-semantics struct kind, zero-cost as a field / vector element — landed
AFTER the 2026-07-04 sealing, so it adds rules the operational family does not yet state in
[heap.md](heap.md) / [types.md](types.md) (**rule text = a follow-up**). Its behaviour is pinned
here by guard, as a client of D-op-1 (differential — the oracle + the matrix below — not yet a
definitional proof). The load-bearing new fact is the copy-pass ELISION: the copy is *skipped*
when a plain view is observationally identical to it, which is what makes the kind zero-cost — so
the claim to pin is that this optimization is observation-preserving.

- ✓ **VS-Copy** — a value-struct projection COPIES (the INVERSE of H-View): `c = rec.f; c.x=9` ⇒
  `rec.f.x==1`; `e = vec[i]; e.x=9` ⇒ `vec[i].x==1`; lifetime fields (`text`/`vector`) DEEP-copy;
  `self` bound to a method-local copies. Both backends. *Guard: `tests/scripts/516`, `517`, `518`.*
- ✓ **VS-Elide** — the read-only ELISION is OBSERVATION-PRESERVING: a value-struct view-bind that
  is only read, over a base not mutated for the view's lifetime, may be left as a zero-cost view
  yet still behaves as a SNAPSHOT. The falsifying program is a mid-iteration SOURCE mutation —
  `for p in b.items { b.items[0].x = 99; s += p.x } ⇒ 6` (snapshot), NOT `104` (alias): if the
  elision wrongly kept the alias it would read `104` (H-View's reference-struct behaviour). Also
  pinned: straight-line snapshot, co-alias `w = b; w.f.x = …`, and escape-returns-a-copy. Both
  backends. *Guard: `tests/scripts/519-value-struct-zero-cost.loft`.*
- ✓ **VS-Zero (the cost side of VS-Elide)** — eliding a read-only view-bind allocates NO
  per-element / per-field store: a `vector<value struct>` and a value-struct record field are O(1)
  `stores_allocated`, FLAT in N, at parity with a reference struct (a per-element copy would be
  ~N). *Guard: `tests/value_struct_alloc.rs`.*

**Residual — the D-op-1 frontier for this rule.** VS-Elide's soundness rests on the copy pass's
taint analysis (`scopes::value_struct_copy`: field-writes + escapes, alias-edge-closed,
loop-body-scoped) — verified by the S1–S5 matrix + the differential oracle, **not machine-proven**.
The concrete "drive it down to the rule" steps:
1. **Graduate `519` to `tests/oracle/*`** (nightly gate: both-backends + leak + driver-agreement),
   so VS-Copy/VS-Elide become oracle-guarded like `24-heap-copy-vs-view`, not just a script guard.
2. **Add a fuzz axis** randomizing {read-only | field-mutated | escaping} × {loop | straight-line}
   × source-mutation timing, so a mis-scoped taint set is caught BY CONSTRUCTION, not only by the
   hand-built matrix.
3. **Write the heap.md / types.md rule text** for the value-struct copy law + the elision's
   observation-preservation side-condition, so VS-* cite a stated rule rather than @PLN101.
