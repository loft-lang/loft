<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 164 — Activation arenas, adopt-at-bind and build-in-place

Tracker: [@PLN164](https://github.com/loft-lang/plans/issues/164) · `status:finished` ·
`subject:store-lifetime`.

**Status — DONE 2026-09-17.**  Every unit that survived pricing is built and default-ON.  The
reference for what each mechanism does lives in [`LIFETIME.md`](../../LIFETIME.md) (the ownership
units, each with its switch and receipts), [`formal/ownership.md`](../../formal/ownership.md) /
[`formal/rewrites.md`](../../formal/rewrites.md) / [`formal/heap.md`](../../formal/heap.md) (the
rules the units enforce), `CLAUDE.md` (every switch and trace variable) and
[`PERFORMANCE.md`](../../PERFORMANCE.md) § 3e (the measured result and where the next factor
sits).  This file is the closure record; the cells each phase was verified against stay in
[`bytecode-comparisons/`](bytecode-comparisons) and are run by the pins in `tests/*.rs`.

**What it delivered.**  The drawing library's `parse` row went from **7.6×** its Rust reference
to **3.11×** (20 740 ns against 6 675), and one parse mints **13 stores where it minted 31** —
with no line of the consumer changing.  That was the premise: the programmer is not assumed to
know the store model, so the most NATURAL form of a program is the one the compiler optimises
([GOALS.md](../../GOALS.md) § Goal F), and a record-returning style should not cost a store per
call.  Six silent-wrong defects were found and fixed inside the arc, each with a falsified guard.

## What one parsed line cost — the census the plan was cut from

`loft-libs-graphics`, `drawing/src/drawing.loft`, `parse_poly` and its callees were the FIXED
input, written in the natural style: build a record, return it, put it in a field.  Per `Poly`
line the compiler minted:

| temporary | mechanism that minted it | closed by |
|---|---|---|
| 4 stores at `parse_poly` entry (`__ref_3..6`) | every record-returning callee gets a caller-minted return buffer, one STORE each | A0 (minted at first use), B1b (pooled across activations) |
| a store for `pp_raw` + a deep copy of its `PointList` | the first bind of a call result copied | B1 (adopt the callee's minted store) |
| a store per `?`-discharged record element + a copy of the `Elem` | `?` on a record element materialised a record | C3 (the discharge already views; the call-side disturbance was the real half) |
| a store for `Elem {…}` + a copy into `sc.elems[idx]` | an indexed overwrite from a literal built in a temp | C1 (written into the slot) |
| a deep copy of `Paint` into the `Op` | a field assignment copied at the source's last use | B2 (claimed in the destination's store, stored by relocation) |
| the points vector copied into `Op.pts` and again into `Mark.pts` | each destination copied | E-1 (the returned record's field is a view), E-2 (the local is built in its element) |
| a default prefill of every minted record | a partial literal defaulted field by field | C4 (one per-type image, one block write) |

## Outcome per phase

| Unit | Outcome |
|---|---|
| **P0** — price tier 1; count the store-identity sites | Done 2026-09-15 — 287 identity sites in one emission, ~88 mints per parse at ≈ 60 ns a pair: tier 1's ceiling ≈ 10 % of the row |
| **P0b** — charge the release row to loft lines (`scripts/native_attrib.py`) | Done 2026-09-17 — runtime 72 %, store mint/free 20 %, half of it minted at function entry |
| **B1** — adopt at first bind (`@FR-O-Move`) | Shipped 2026-09-15 — store census 139 → 108 |
| **B1b** — the caller's buffer pooled for a promoted-local callee (`@FR-O-Buffer`) | Built 2026-09-17, `LOFT_NO_ADOPT_BUFFER_REUSE` — `Mark` mints 24 → 14 per two parses; closed D-own-43 |
| **B2** — the result claimed in its destination's store, stored by relocation (`@FR-R-Place`, `@FR-R-MoveLast`) | Shipped 2026-09-16 — census 184 → 50; a wash on the bench row (the relocated `Paint` carries no heap there), structural gain |
| **B2 unit 1 over a chain** — a literal exit writes the buffer a chain renamed | Built 2026-09-17, `LOFT_NO_LITERAL_EXIT_BUFFER` — `Mark` mints 21 → 12, the row −2.5 % |
| **C1** — an element overwritten from a literal, in place (`@FR-R-InPlaceLiteral`) | Shipped 2026-09-16 — `acc_pts` census 3 → 2; the STAGING clause was a both-backend silent-wrong on the shipped FIELD road |
| **C2** — the destination as return buffer (`@FR-R-Place`, `@FR-O-Buffer`) | Shipped 2026-09-16 — no admitted site in the consumer (the emission is byte-identical under the switch); structural |
| **C3** — the read-only `?`-discharge (`@FR-B-Disturb`, `@FR-B-View`) | Shipped 2026-09-16 — the discharge ALREADY views; the phase's content was `(B-Disturb)` across a CALL |
| **C4** — the per-type prefill image (`@FR-R-Prefill`) | Shipped 2026-09-15 — the row −11 % |
| **C5/E-1** — a returned record's heap field as a view leaf, and the forward (`@FR-O-ViewField`, `@FR-R-ValueRecord`) | Built 2026-09-16 opt-in, default-ON 2026-09-17 — the row 3.39× → 3.16×; the per-PATH proof (`hoist::fresh_leaf`) replaced the per-exit test that four cells falsified while it was opt-in |
| **C6** — a nested record literal built inside its field (`@FR-R-InPlaceLiteral`) | Built 2026-09-17, `LOFT_NO_NESTED_IN_PLACE` — the row −6–7 %; loft#1548 found and fixed on the way |
| **A0** — the hidden buffer minted at its first use (`@FR-O-LazyBuffer`) | Built 2026-09-17, `LOFT_NO_LAZY_BUFFER` — the row −7 % |
| **E-2** — a single-consumer vector built in its element (`@FR-R-ElemFirst`) | E-2a and E-2b built 2026-09-17, `LOFT_NO_ELEMENT_PLACE` — 19 → 13 stores per parse; loft#1552, #1553 and #1554 found and fixed on the way |
| **The wilderness** — the store's tail free block held beside the free tree (`@FR-H-Wilderness`) | Built 2026-09-17, `LOFT_NO_WILDERNESS` — the row −6.3 %, `fronds` −2.6 % |
| **Runtime pass** — the write check inlined, the no-heap claims walk skipped, a fresh store initialised once | Shipped 2026-09-17 — the row 28.3–28.5 k → 25.0–25.3 k ns/op; an accessor reorder measured and DROPPED (pixel rows +15–38 %) |
| **A1/A2** — the activation arena | **Superseded 2026-09-17** (owner ruling C125): the temporaries are REMOVED, not co-located, and the store model stays *one store is one object* |
| **E-3** — `PointList` reused per call site | Priced by hand patch 2026-09-17 and **DROPPED** — a wash once E-2 removed the copies around it |
| **Text per character** — `#[inline]` on the two per-character runtime calls | Measured and **DROPPED** 2026-09-17 (−1.0 % instructions, +1.1 % cycles) |

Every built unit carries the same evidence: a switch, cells in `bytecode-comparisons/` with
hand-computed values on both backends under `LOFT_STRICT_STORES`, `LOFT_POISON`,
`LOFT_POISON_CLAIM` and the leak gate, a sabotage that the cells catch, a guard in
`tests/scripts/` with its `@falsified-at:` receipt, pins in `tests/<unit>.rs`, and — for an
emission unit — the default emission proved byte-identical with the switch off.

## The elimination queue, and its two remaining units

After the owner's ruling C125 the plan's remaining order was elimination, unit by unit.  The
census it worked from (stores minted per parse, `LOFT_TRACE_DB=1`):

| stores | what | outcome |
|---:|---|---|
| 6 | `Mark` buffers, one per pooled call site per activation | E-1 — removed (the tuple with a view) |
| 9 | `vector<Pt>` locals, each deep-copied into `Op.pts` AND `Mark.pts` | E-2 — removed (single-consumer, built in the element) |
| 6 | `vector<float>` widths, one consumer | E-2 — removed, except the value branch below |
| 4.5 | `PointList` from `read_points` — scratch, only read | E-3 — priced a wash, dropped |
| 2 + 3 + 1 | `Sketch`, `Paint`, `FrondSpec`, `vector<integer>`, `Row` | real data, or unclassified and small |

**Two units are dropped on price, not on doubt**, both hand-patched and measured against the
bar in [PERFORMANCE.md § The clear case first](../../PERFORMANCE.md):

- **E-2's value branch** — `parse_poly`'s widths, `if smooth_applies { smooth_vals(…) } else
  { copy }`, each arm filling the element's field: **≈ −0.5 %** (the hand patch's −4.0 % less
  what E-2a and E-2b already take).  It wants a new temp shape, a call arm and a copy arm.
- **E-2c** — `parse_fronds`' `pf_all`, which lives only in the returned record: **−0.57 %**.

They are re-priced only if the runtime lever below lands and the row's remaining gap then traces
back to these copies; at 3.11× neither is what stands between the row and the 3× bar.

## Where the row stands, and why the next lever is a runtime one

`perf record` on the release binary, self time, grouped by the runtime function's own name, is
reproduced with its table in [`PERFORMANCE.md`](../../PERFORMANCE.md) § 3e.  The short of it:
**about 1.2 of the 2.1 units the row is over Rust is the per-RECORD and per-PUSH work every KEPT
object pays** — claim and free, `vector_append`/`append_f64`, the append path's `heap_facts` and
`nullable_field_parent` tests, `store_mut`.  The elimination queue removed what it could; what
is left is not temporaries, so it is a RUNTIME lever on both backends, not another emission unit,
and it belongs with the role/lifetime/element-layout work § 3e registers.  Price it the way this
arc priced every unit: a hand patch in the runtime first, `perf` on the release binary, all
fourteen rows before a unit is cut.

*Measurement recipe used throughout*: the release-tier emission (`loft --native-release
--native-emit`), rustc against `target/release`, `perf stat -r 5` × 3 interleaved under
`taskset`, stores counted exactly with callgrind (n = 20 minus n = 10 calls of `database_named`),
hash `33f6d2b8` checked on every variant so a faster run is still the same program.

## How each row was judged — the five questions, in order

The owner reviews the PROCEDURE, not twenty tastes (2026-09-15).  The first question that
decides, decides: **1.** spelling, or mechanism property?  **2.** is the spelling natural —
MEASURED against the consumer corpus, not judged?  **3.** does a written rule in `formal/`
already answer it (and a question no rule can express means the RULE wants extending — the
owner's call)?  **4.** what falsifies the verdict (a cell with a hand-computed value on both
backends under every falsifier)?  **5.** what does being wrong cost — admitting wrongly is
silent-wrong, a leak or a double free; declining wrongly is a missed optimisation the compiler
pays itself, so every doubt DECLINES.  The naturalness measurement is what kept the mechanism off
contrived spellings: a record `?`-discharge bound to a local occurs 45 times in the corpus, a
field assigned from a local 382, a bind-then-return 96, `x = f(x, …)` on a record or vector 0.

## Open design questions at close

1. **E1 — store identity once buffers share a store: WITHDRAWN** with tier 1 (C125).  The store
   model stays *one store is one object*.
2. **Is the arena an IR fact or an emission fact:** moot with E1.
3. **Where a moved-from local's `deps` go:** answered by B2 — the path state is a compile-time
   fact written into the IR (`OpMoveRecord`, `OpFreeRecordIn`), never a runtime stand-in.
4. **`(O-ViewField)`:** DECIDED admitted (C122) and recorded as `(R-Escape)` in
   `formal/rewrites.md` — a rewrite needs its validated conditions and nothing else; the one
   boundary is a library API whose callers are unseen.
5. **`(R-Place)` across stores** stands as a rule: the placement is per call, decided by the
   destination.

## Bugs found and fixed inside the arc

Each reproduces on `main`, so each is an issue with a falsified guard (all `fixed-pending-merge`
until this branch lands):

| issue | what answered wrong | deviation closed |
|---|---|---|
| loft#1548 | an appended literal whose field expression reads its own container lost its fields (`0` for `1030`, both backends) | D-op-10 |
| loft#1549 | a pooled return buffer stranded the old vector of a heap-owning record | D-heap-12 |
| loft#1550 | a callee answering a view of one of several arguments was copied at every bind | D-own-45 |
| loft#1552 | a `--native` element-first vector local REBOUND before its append left the element empty | D-rw-2 |
| loft#1553 | a `--native` view read between an element-first temp's declaration and its append read a relocated block | D-rw-3 |
| loft#1554 | `(B-Ref-Reshape)`'s call-site refusal covered only a removal through a `&vector` parameter | D-bind-47 |

`D-own-44` and `D-own-46` were closed by the units themselves (the entry witness and E-1's
per-path proof).

## See also

- [`LIFETIME.md`](../../LIFETIME.md) — the ownership units as reference, each with its switch,
  its declines and its receipts.
- [`formal/ownership.md`](../../formal/ownership.md) (`O-Buffer`, `O-Move`, `O-LazyBuffer`,
  `O-ViewField`), [`formal/rewrites.md`](../../formal/rewrites.md) (`R-Place`, `R-MoveLast`,
  `R-InPlaceLiteral`, `R-ElemFirst`, `R-ValueRecord`, `R-Prefill`, `R-Escape`),
  [`formal/heap.md`](../../formal/heap.md) (`H-Wilderness`, `H-ClearRelease`,
  `H-FreeFooter`), [`formal/binding.md`](../../formal/binding.md) (`B-Disturb`,
  `B-Ref-Reshape`).
- [`PERFORMANCE.md`](../../PERFORMANCE.md) § 3e — the row's remaining composition and the next
  lever; § The clear case first — the bar the two dropped units were priced against.
- [`../157-native-4x-drawing/README.md`](../157-native-4x-drawing/README.md) — the drawing arc
  this plan is the parse unit of; the 14-row bench and its § V rewrite family.
- [`DESIGN_DECISIONS.md`](../../DESIGN_DECISIONS.md) C122 (`(R-Escape)`), C125 (eliminate the
  object rather than co-locate it).
