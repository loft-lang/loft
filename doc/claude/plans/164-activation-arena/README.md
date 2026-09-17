<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 164 — Activation arenas, adopt-at-bind and build-in-place

Tracker: [@PLN164](https://github.com/loft-lang/plans/issues/164) · `status:active` ·
`subject:store-lifetime`.

## Status (REQUIRED)

Active (the owner's go, 2026-09-15).  **P0 done**, **B1, C4, B2, C3, C1 and C2 shipped**, **C5
step 1 built OPT-IN** (`LOFT_VIEW_FIELD=1`) — B2's caller side and C2 are both a wash on the parse
row, structural gain only (§ B2 and § C2 *Measured*), and C5 does not reach the consumer yet, with
the gate that declines each library function measured per function (§ C5 *Built*).  **The
re-profile's verdict is CORRECTED (2026-09-16, § Re-profiled *The correction*): "the row is the
scanner, the class is paid down" was read off operator COUNTS and INTERPRETED profiles, and `perf`
on the release binary itself reads loft runtime 63 %, program 31 % — the class is still two thirds
of the row.  **P0b is done (2026-09-17, § P0b):** charged to loft lines through the inline chain,
the runtime is 72 % and the store mint/free family 20 % — half of it buffers minted at FUNCTION
ENTRY on paths that never use them.  Moving those mints to their first use measured −9–10 % by
hand, so **A0** (§ A0) is cut, after @PLN157's prelude elision (−11–14 %).  Both are BUILT
(2026-09-17), and so is **C6** (§ C6): the parse row is 38.8–41.7 k → 29.8–29.9 k ns/op, every
other row equal or faster.  The last-use vector field, measured next, is not a move; the
release profile named three runtime fast paths instead, which took the row to 25.0–25.3 k
(≈ 3.5× the Rust reference) and every other row equal or faster, and loft#1549 was fixed on the way (§ The runtime under the
row).**  The
measurements are under § P0 below and the mechanism under § B1.  What P0 changed in the
plan: tier 1's ceiling is ~10 % of the parse row, not a fifth, and its store-identity cost
(287 `store_nr !=` sites in one emission) is real, so A1/A2 stay behind B and C in the
queue and the owner's E1 pick is still open.  B1 took the minimal sound shape — adopt the
callee's minted store, keep the caller's buffer null — and the matrix found the reason the
wider one (reuse the buffer across activations, E7) must wait: the interpreter's rebind of a
promoted buffer local frees a caller-supplied buffer where native guards it (§ B1b).
Routes here per the docs-vs-plans rule: the mechanisms belong to `LIFETIME.md` /
`OWNERSHIP_MODEL.md` / `formal/ownership.md` once shipped; this directory is the design and
the matrix until then.

## Goal (REQUIRED)

**The principle this plan serves (owner, 2026-09-15; GOALS.md § Goal F):** the
programmer is not assumed to know the store model inside, so the most NATURAL form of a
program is its CANONICAL form and therefore the one the compiler optimises — not an idiom
the author learns from a survival guide.  **And the scoping (owner, same day):** this is
not "optimise everything" — each spelling is first asked *is this natural to write?*, the
mechanism goes to the spellings that pass, and a contrived spelling keeps today's correct
copy and is deferred with its row kept.  Most cells of the matrix below may be deferred
at any moment; the natural ones are where the gain sits.  **And the licence (owner,
2026-09-15, C122, `(R-Escape)`):** the contract is semantics, not representation — a
rewrite needs its validated conditions and nothing else; the one boundary is a library API,
whose callers are unseen, and a construction that does not escape it may be rewritten in
any way its conditions allow.  The compiler removes the per-call temporaries a record-returning style mints — a store
per hidden buffer per activation, a deep copy at a first bind, at a last-use field
assignment and at an element overwrite, a copy for a read-only `?`-discharge — without a
line of the consumer's code changing, and the drawing library's `parse` row goes from
7.6× the Rust reference toward 3× as the measurement of it.

## Effort + design

- **Effort:** M (tiers 1–2) · MH (tier 3)
- **Design:** ~ — the invariants are named; the store-identity question (§ Edge cases E1)
  is open and decides tier 1's shape.
- **Last touched:** 2026-09-17 (P0b, row 11, A0 and C6 built, loft#1548 and D-own-44 fixed on
  the way; the 14-row table and the parse row re-measured — § Re-measured; the runtime fast
  paths and loft#1549 — § The runtime under the row; B1b, D-own-43 and loft#1550 — § B1b;
  the chain-renamed literal exits — § B2 unit 1 over a chain)

## The evaluation — what one parsed line costs, and why

The owner's premise, stated 2026-09-15: *knowing this fact of loft I would look at the
parse implementation to make fewer objects; but what I really want is a compiler that
determines this problem itself and makes efficient code for it.*  So the library
(`loft-libs-graphics`, `drawing/src/drawing.loft`, `parse_poly` and its callees) is the
FIXED input; it is written in the natural style — build a record, return it, put it in a
field — and the question is what the compiler mints around that style.

A census of the emitted Rust (`--native-emit` of the bench's `parse_only.loft`, counting
store mints, record mints, deep copies and frees per function and attributing each to its
loft line) says, for ONE `Poly` line:

| temporary | mechanism that mints it | needed by the program? |
|---|---|---:|
| 4 stores at `parse_poly` entry (`__ref_3..6`) | every record-returning callee gets a caller-minted return buffer, one STORE each, freed at exit | no — dead at exit |
| a store for `pp_raw` + a deep copy of the `PointList` (4 vectors) | the first bind of a call result copies: `OpBindOrCopy` adopts only a store the local already holds, and a fresh local holds none (`O-Buffer`) | no — the buffer could be adopted |
| a store per `?`-discharged record element (`ap_e = sc.elems[idx]?` in `acc_pts`) + a copy of the `Elem`, its `ename` text included | `?` on a record element materialises a record; `Elem` owns text, so § V-ad's all-scalar buffer does not apply | no — only read, field by field |
| a store for `Elem {…}` + a copy into `sc.elems[idx]` | an indexed overwrite from a literal builds in a temp and copies | no — the slot exists |
| a deep copy of `Paint` (with its `spec` vector) into the `Op` | `paint: pp_paint` copies at `pp_paint`'s last use | no — a move would do |
| the points vector copied into `Op.pts` and again into `Mark.pts` | `smooth_pts` builds in its own buffer; each destination copies | one copy at most |
| a default prefill of every minted record | a partial literal defaults the rest field by field (`set_default_value_nullable`) | no — a per-type image is one block write |

Per line that is a dozen store mint/free pairs, four deep copies and three copies of the
points, on a scene of thirteen lines.  The `Op` itself is ALREADY built in place in the
scene's store (`@FR-R-Mint`/`@FR-R-PushRec`), and the `Mark` already lands in a buffer
`parse_scene_at` mints once for the whole scene and hands down — so two of the objects
already live in a longer-lived container; everything below `parse_poly` does not.

The profile agrees (program samples only, 20 000 calls, this box):

| class | share | routines |
|---|---:|---|
| record and vector lifecycle | ≈ 57 % | `copy_claims` 8, `set_default_value_nullable` 7, `begin_write_inner` 4, `remove_claims_mode` 3.5, `store_mut` 4.7, `free_named` 3, `claim` 2.7, `Store::init` 1.8, `store_budget` 1.8, `close_file_handle` 1.2, `owned_walk` 1.4 … |
| the library's byte scan | ≈ 19 % | `matches_at`, `find_option` (rescans the line per key) — the library's own |
| the parse logic | ≈ 8 % | `parse_scene_at`, `acc_pts`, `fronds`, the trig |

## Re-profiled after the copy phases (2026-09-16) — first read as the SCANNER, corrected the same day

**Read § *The correction* below before the four instruments: their numbers stand, the conclusion
drawn from them does not.**  The evaluation table above is what this plan was cut from, and four
phases later it is stale in the way a measurement makes a plan stale rather than wrong.
Re-measured on the same bench, by four instruments, each answering a different question:

**1. Which operators run (`LOFT_NATIVE_CHECKPOINTS=count`, `--native-release`, 300 parses —
1585 sites, 16.99 M executions, ≈ 56.6 k operators per parse).**  Counts are this instrument's
trustworthy column:

| function | share of operator executions |
|---|---:|
| `at` | 17.8 % |
| `matches_at` | 12.9 % |
| `is_word_byte` | 11.8 % |
| `find_option` | 9.9 % |
| `t_4text_size` | 9.7 % |
| `word_boundary` | 9.5 % |
| `t_4text_split` | 6.7 % |
| `lower_byte` | 2.4 % |
| **the byte scanner, together** | **≈ 80 %** |
| the parse logic (`acc_pts`, `smooth_pts`, `fronds`, `circle_pts`, `read_points`, `parse_fronds`) | ≈ 10 % |

**2. Where the loft-level time goes (`LOFT_PROFILE=1 --interpret`, loft's own sampler).**  A
different instrument on a different backend, and it agrees: `at` 16.3 %, `is_word_byte` 15.2 %,
`matches_at` 14.0 %, `find_option` 12.8 %, `word_boundary` 8.0 %, `t_4text_size` 6.3 %,
`t_4text_split` 4.6 %, `lower_byte` 3.4 % — **the same ≈ 80 %**.

**3. Which of loft's own routines burn cycles (perf over the engine, interpreted).**
`State::execute_argv` 22 % (the interpreter's dispatch, which the native lane does not pay),
`store_mut` 4.0 %, malloc/free ≈ 4.5 %, `copy_block` 1.0 %, `begin_write_inner` 1.0 %.  **The
two routines that headed P0's lifecycle table — `copy_claims` at 8 % and
`set_default_value_nullable` at 7 % — are no longer in the top eighteen.**

**4. What the program still mints (`LOFT_TRACE_DB=1`, `--native-release`, per parse).**  69
stores: `vector<Pt>` 18, `vector<float>` 16, `Paint` 15, `Mark` 12, `PointList` 3, `Sketch` 1.
At the ≈ 60 ns a mint/free pair costs (§ P0) that is **≈ 4 µs of a 40 µs row — 10 %, which is
P0's own tier-1 ceiling and is now the ceiling for ALL remaining store-lifecycle work in this
row.**  With C5 armed the census reads 9 `Mark`s instead of 12 — three of the scene's lines lose
their buffer, 0.45 % of the row, which is the wash § C5 measured.

### The correction (2026-09-16, later the same day) — none of the four instruments measures native cycles

The verdict first drawn from them — *the row is the scanner, the plan's class is paid down* —
was an instrument artefact, and the way it was reached is the lesson: instrument 1 counts operator
EXECUTIONS (and its own doc says a count is not time), instrument 2 samples the INTERPRETED run,
instrument 3 is `perf` over the interpreter (whose dispatch is 22 % of it and whose costs are not
the native lane's), and instrument 4 is a store census that prices only the store mint/free pair.
A scanner executes many cheap operators, so it dominates every COUNT; on the release binary those
operators are a few instructions each, and the time is where the out-of-line runtime calls are.

**`perf` on the `--native-release` binary itself** (the parse bench's release emission compiled
with loft's own rustc line — `-C opt-level=3 -C codegen-units=1`, 3.3 s — `perf record -F 5000`,
main core 3K samples, the E-core section's 55 samples discarded; hash `33f6d2b8`, the row at
39.8–42.4 k ns/op).  Self time by home:

| where the cycles go | share |
|---|---:|
| loft runtime | **63 %** |
| the program (scanner + parse logic, the leaves inlined into their callers) | 31 % |
| std/libc (`dec2flt` for `as float?`, `memcpy`) | 5 % |

And the runtime's share by family:

| family | routines | share |
|---|---|---:|
| store allocator | `claim`, `claim_block`, `finish_claim`, `fl_insert_node` / `fl_take_ge` / `fl_delete_node` (the free tree) | 12 % |
| store access | `begin_write_inner` 7 %, `store_mut`, `get_elem` | 12 % |
| record mint / copy / free-walk | `record_new`, `OpFinishRecord`, `link_siblings`, **`copy_claims` 3 %**, **`remove_claims_mode` 2 %** | 10.5 % |
| store mint / free | `op_database_inner`, `free_named`, `close_file_handle`, `store_budget`, `Store::init` | 9 % |
| vector append | `vector_add`, `vector_append`, `append_byte` | 8 % |
| other | `OpFreeRef`, `enum_parent_size`, `record_finish`, text ops | 11 % |

Top self symbols: `n_find_option` 8.5 %, `begin_write_inner` 7.2 %, `n_parse_scene_at` 7.0 %,
`n_acc_pts` 3.5 %, `claim` 3.1 %, `copy_claims` 3.0 %, `vector_append` 2.7 %, `vector_add`
2.4 %, `n_fronds` 2.2 %, `remove_claims_mode` 2.1 %.

*What this settles.*  P0's arithmetic holds exactly for what it priced: the store mint/free pair
is 9 % of the row, which is tier 1's whole ceiling.  What P0 never priced is the per-RECORD churn
inside and around those stores — every claim through the allocator, the free tree that a release
inside a live store feeds, the copies the engine profile had lost sight of (`copy_claims` and
`remove_claims_mode` are still here), the append path — and that is ≈ 30 % of the row.  The plan's
class is not paid down; it is two thirds of the row, and only the store-pair tenth of it has a
priced phase.

*What it does not settle: WHICH loft lines drive that churn.*  Both call-graph modes broke at the
runtime boundary (`--call-graph dwarf` on a `-Cdebuginfo=1 -Cforce-frame-pointers=yes` program
resolved no chain into the rlib; `lbr` attributed 84 % to an `(inlined)` pseudo-frame), so every
inclusive column read equal to self.  Attribution needs the rlib built with frame pointers
(`RUSTFLAGS=-Cforce-frame-pointers=yes cargo build --release --lib` into a separate
`--target-dir`), and it is the P0-style step before any phase is cut — it decides between the
arena's release-to-mark (which takes the free-tree half of the temporaries' churn) and the copy
sites the profile still shows.

**The lever outside the plan, measured on the same emission.**  Every `at(s, i)` call pays the
frame prelude — `cr_call_push_lean`, `CallGuard`, `FnRefBufGuard` — because `at` calls the
stdlib's `size`, a loft-bodied `t_` function, and so fails § N4's leaf test; `matches_at`,
`word_boundary`, `skip_space`, `find_option` and the readers fail it the same way.  The prelude
hand-removed from those functions in the release emission (`bytecode-comparisons/` carries no
cell for it; the edit is the deletion of the three prelude lines):

| variant | ns/op (ABAB ×5, `--n 4000`) | hash |
|---|---|---|
| as emitted | 39.8–42.4 k | `33f6d2b8` |
| prelude removed from the scanner's non-recursive functions | **34.5–40.0 k** (−11 to −14 %) | `33f6d2b8` |

That is N4 made TRANSITIVE — a function whose callees are all leaves cannot recurse, so it needs
no depth entry either — and in the lean tier it loses nothing observable but the depth cap's
exactness on an acyclic chain.  It is @PLN157's item (its queue carries it as row 11), S-sized.
The other candidate did nothing: the rlib's `text_byte_at_native` hand-inlined into the crate
moved the row by noise — LLVM already inlines it across the rlib — so *"LTO into the rlib"* is not
this row's lever.  After the prelude lever the row reads ≈ 5.4× the Rust reference: the
program's own 31 % is the floor the consumer's algorithm sets, and the runtime's 63 % is this
plan's.

### What that means for the queue

1. **P0b — attribute the runtime's 63 % to loft lines** on a frame-pointer rlib build (above).
   A phase cut before this is a phase cut on a hypothesis, which is how the last four went.
2. **@PLN157's transitive-leaf prelude elision** (its queue row 11) — measured −11–14 %, S,
   independent of this plan and worth landing first.
3. **Then, by the attribution:** A1/A2 as a pooled store or a release-to-mark arena if the
   free-tree and store-pair families charge to activation temporaries; the remaining copy sites
   (`copy_claims`, `remove_claims_mode`) if they charge to a bind or a field assignment the B/C
   phases missed.  C5 step 3 and the vector `place_result` stay parked: their class measured
   under 2 % and the correction does not change that.
4. Re-measure the 14-row `compare.py` table after each of these; the parse row's re-profile is
   re-taken with `perf` on the release binary, never with a count.

*P0b answered item 1 — § P0b below.  It re-cut the queue a second time.*

## P0b — the release row charged to loft lines (DONE 2026-09-17)

**The instrument.**  `scripts/native_attrib.py` takes a `--native-release` binary built with
frame pointers and line tables — the program AND the runtime rlib it links
(`RUSTFLAGS=-Cforce-frame-pointers=yes cargo build --profile profiling --lib`; the rustc line
is in the script's header) — plus the emission and a `perf record --call-graph fp`.  It
expands each physical frame to its inline chain with `llvm-symbolizer`, charges the sample to
the innermost frame of the emitted program, and reads the loft line off that frame's
`// loft:` comment.  A runtime routine is reported under the ENTRY the program called, grouped
into families.  Run: `taskset -c 2`, `--n 80000`, 16 835 samples, hash `33f6d2b8`, 42.2 k ns/op
(frame pointers cost a few per cent; shares come from this build, times from the release one).

**Two corrections to the earlier read.**  The runtime share is **72 %**, not 63 %: the self-time
table counted a runtime helper inlined into a program function (`op_add_long`, `store_mut`,
`offset_in_bounds`) as program.  And the store mint/free family is **20 %**, not 9 %: the 9 %
was the store routines' own SELF time, while the entry-inclusive cost also carries the claim,
the zero fill, `Store::init`'s free header and the release walk each mint pays.

| family (by the entry the program called) | share | where it is charged |
|---|---:|---|
| store mint/free | **20.0 %** | `parse_poly` 5.6, `parse_circle` 3.9, `parse_fronds` 2.7, `read_paint` 2.4, `parse_scene_at` 1.6, `no_mark` 1.1 |
| record mint/copy/move | 11.0 % | `parse_fronds` 4.7, `read_points` 1.6, `parse_poly` 1.1 |
| store access | 9.6 % | `acc_pts` 2.4, `parse_fronds` 2.0 |
| vector field copy (`vector_add`) | 9.2 % | `parse_fronds` 5.5, `parse_poly` 2.0, `parse_circle` 1.2 |
| vector append/grow | 6.7 % | `read_points` 2.1, `fronds` 1.8 |
| frame prelude | 6.4 % | `matches_at` 2.2, `word_boundary` 1.6, `at` 1.4 |
| std (`dec2flt`, malloc, memcpy) | 5.0 % | |
| text | 4.8 % | `t_4text_split` 2.4 |
| checked arithmetic | 3.8 % | `matches_at` 2.1 |
| libm | 1.3 % | `circle_pts` |
| **the program's own code** | **21.2 %** | `matches_at` 5.2, `find_option` 2.8, `t_4text_split` 2.5 |

**By line, two findings carry most of the plan's class.**

1. **Half of the store family is minted at FUNCTION ENTRY, on every path.**  The function
   prologues alone — before the first loft line runs — take `parse_poly` 4.0 %, `parse_circle`
   3.1 %, `read_paint` 2.0 % (9.1 % together, their exit frees on top).  The preamble's
   `__ref_N = null` lowers to a store mint on both backends, and it runs whether or not the
   path that uses the buffer is taken.  `parse_circle` is called for 7 of the scene's lines and
   matches ONE: on the other six it mints its two buffers (`Paint`, `vector<Pt>`) and frees them
   unused.  `parse_poly` mints four, one per `smooth_pts` / `smooth_vals` site, and its three
   exits take one or two of them.  P0 judged this pattern not worth a hand patch by arithmetic
   (≈ 0.7 µs); measured, it is the largest single lever the row has.
2. **`drawing.loft:704` — the `Op` literal appended in the frond loop — is 12.5 % of the row.**
   `vector_add` 5.1 % (the `pts` and `widths` copies), `OpCopyRecord` 2.5 % + `OpDatabaseNP`
   1.3 % (the nested `Paint { … }` literal is built in a store of its own and deep-copied into
   the element's inline `paint` field — per frond), `OpNewRecord` 1.1 %.  The same nested
   literal is at `:594` (`parse_line_cmd`).  Of the two vector copies, `widths: pf_wids` is the
   variable's last use; `pts: pf_line` is not (line 707 reads it again), so that copy is the
   two-destination copy tier 3 keeps.

**The first finding, measured by hand** (`bytecode-comparisons/A0-lazy-mint-hand-patch.py`):
all 16 entry mints in 7 functions moved to a null-guarded mint in front of the call that takes
the buffer; the declaration holds the null sentinel; the exits' `OpFreeRef` already ignores a
null store, so no free moved.

| variant | ns/op (ABAB ×4, `--n 4000`, P-core) | `perf stat -r 5` instructions / cycles | hash |
|---|---|---|---|
| as emitted | 38.6–39.8 k | 2 499 M / 583 M | `33f6d2b8` |
| entry mints lazy | **35.3–35.8 k** (−9 to −10 %) | **2 319 M / 536 M** (−7.2 % / −8.0 %) | `33f6d2b8` |

Both variants run clean under `LOFT_STRICT_STORES=1 LOFT_NATIVE_LEAK_CHECK=1`, and the
instrument can fail: the same emission with one exit's `OpFreeRef` deleted reports
`1 stores not freed … main_vector<Pt>×20`.

**Smaller items the attribution names.**  `strict_stores()` shows as 1.8 % of leaf time: the
store accessors test the switch BEFORE the slot's `free` flag, so the common path pays an atomic
load that `s.free && strict_stores()` would skip.  `offset_in_bounds` (6.4 % leaf) and
`begin_write_inner` (4.9 %) are the per-access checks of the store family, which no
lifecycle phase changes.

### The queue after P0b

1. ~~**@PLN157's transitive-leaf prelude elision**~~ (its row 11) — SHIPPED 2026-09-17
   (`@FR-R-LeafChain`): the row 38.8–41.7 k → 33.5–34.1 k ns/op, instructions −19.1 %.
2. ~~**A0 — the buffer minted at its first use**~~ (§ A0) — BUILT 2026-09-17: the row
   33.5–34.1 k → 31.2–31.4 k ns/op on top of row 11.
3. ~~**C6 — a nested record literal built inside the element**~~ (§ C6) — BUILT 2026-09-17:
   hand-measured −8.5 %, built −6–7 %; loft#1548 fixed on the way.
4. ~~**The last-use vector field**~~ — measured before it was cut, and it is not a move:
   every copy it named crosses stores and reads a view or a reused buffer (§ The runtime
   under the row).  What the profile named instead — three runtime fast paths — is BUILT
   (2026-09-17, same section): the row 28.3–28.5 k → 25.0–25.3 k ns/op.
5. **The `Mark` result path** — now the largest store class, 12 of the 30 stores a parse
   mints (§ The runtime under the row, *The census*).
6. **A1/A2** — re-priced: the store family is 13.7 % of the row now (`parse_poly` 3.8,
   `parse_scene_at` 2.1, `parse_fronds` 1.6, `no_mark` 1.6, `parse_circle` 1.1).

## A0 — the buffer minted at its first use (`@FR-O-Buffer`)

*Invariant:* a hidden return buffer is minted at most once per activation, before the first
statement that hands it to a callee on the path that runs; a path that never uses it never
mints it.  `(O-Buffer)` says who owns a buffer and when it is freed; it does not require the
mint to precede the path.  The free side already holds: every exit frees the buffer with a
null-tolerant free (native `OpFreeRef` returns on the sentinel; the interpreter's `FreeRef`
is asked the same question in the cells).

*The shape:* the preamble's `__ref_N = null` becomes the non-allocating sentinel (the
`OpInitRefSentinel` the interpreter already has for an inline ref, `DbRef::NULL` on native),
and each statement that passes the buffer to a call is preceded by
`if OpVectorIsNull(__ref_N) { __ref_N = null }` — the guarded-mint shape a vector `+=` already
uses (`vectors.rs`, loft#1219), idempotent in a loop.

*Cases to settle before the first cell is written:*

| # | case | why it bites |
|---|---|---|
| A0-1 | the buffer used on one branch only, freed at every exit | the plain win; the exit free on the un-minting path must be a no-op on BOTH backends |
| A0-2 | the buffer used inside a loop | the guard must mint once, not per iteration — a re-mint clears what the previous iteration's result local still reads |
| A0-3 | a result local ADOPTS the buffer (`p = mk(s)`, deps `["__ref_1"]`) and is read after | the local IS the buffer; the witness compare at the exits (`p.store_nr != __ref_1.store_nr`) must still decline |
| A0-4 | two call sites in exclusive arms sharing nothing but the exit | each arm mints its own; the exit frees both, one of them null |
| A0-5 | a buffer the H7 loop rotation (`OpPutRef(__ref_1, __ref_2)`) swaps | the partner must be minted before the swap reads it — a use that is not a call argument |
| A0-6 | a buffer promoted to `__retbuf` / an adopting `_rb_w_` witness | the promoted path is the caller's store, never minted here — excluded |
| A0-7 | a generator body and a `par` arm | declined, as every hoist declines them |
| A0-8 | the interpreter's slot allocator | the preamble `Set` is the buffer's first def (plan 51 cluster IV); the sentinel init must keep that role |

Switch `LOFT_NO_LAZY_BUFFER=1`; falsifiers `LOFT_STRICT_STORES=1`, `LOFT_NATIVE_LEAK_CHECK=1`,
`LOFT_POISON=1`; cells `bytecode-comparisons/A0-lazy-buffer-cells.loft` on both backends.

### Built 2026-09-17 (`@FR-O-LazyBuffer`, default ON)

*Where it lives.*  `scopes::lazy_buffer_mints`, after the scan, so every exit free is already
in the body and can be told apart from a use (`names_outside_free`: `OpFreeRef`,
`OpFreeRefIfDistinct`, `OpFreeRefOrHandUp`, the tag ops and `OpDistinctStore` are not uses).
`insert_before_uses` puts `if OpRefIsNull(b) { b = null }` in front of the innermost statement
that names the buffer — descending blocks, loops, inserts and `if` arms; a use in an `if`
condition or a bare arm takes the guard in front of the whole `if`.  The record-buffer pool
(`reuse_record_buffers`, § V's Route R) places its `OpDatabase` the same way unless its
ungated control is armed.  Declined: a body that yields or runs `par`, a buffer with a second
assignment, a null-init that is not a top-level statement (the one left in the parse bench,
`parse_scene_at`'s `split` buffer, is used on every path anyway).

*The fact both emitters read* is a new `Variable::lazy_buffer` (IR schema, store and JSON
codecs, cache format 7): the vector null-init writes the sentinel (`OpInitRefSentinel` /
`DbRef::NULL`) and the interpreter lowers the later `Set(b, Null)` to the owned-vector mint
(`gen_owned_vector_store`, the same ops the entry init emitted); native's reassignment arm was
already `OpDatabase`, which mints from the sentinel.  A record buffer needs no mark — its mint
is an explicit `OpDatabase` in the IR.  The history's warning was the design constraint:
`mark_inline_ref` would also have RELOCATED the null-init (formal/ownership-history.md, the
`__vdb` entry), so the mark changes alloc-vs-sentinel and nothing else.

*Verified.*  The 14 cells (13 functions rewritten, `a5`'s rotated pair guarded at the call and
at the rotation, `f15` the record pool) hold on both backends in both switch states under
`LOFT_STRICT_STORES`, `LOFT_POISON` and `LOFT_NATIVE_LEAK_CHECK`; with the switch off the
native emission is byte-identical to the build before A0 and the interpreter's bytecode for
`f1` is too.  Falsified: an unconditional mint (the null test replaced by `true`) turns `a5`
into `0 0` on both backends — a re-mint clears the store the loop-carried result holds.  Pins
`tests/lazy_buffer.rs`, subject `codegen`.

*Measured* on the parse bench (release lean tier, after @PLN157 row 11): 15 of the 16 entry
mints now guarded, the row 33.5–34.1 k → **31.2–31.4 k ns/op** (−7 %), `perf stat`
instructions 2 023 M → 1 871 M (−7.5 %), cycles −6.8 %, hash `33f6d2b8`, clean under all three
falsifiers.  The hand patch measured −9–10 % on the pre-row-11 emission; together the two
levers take the row from 38.8–41.7 k to 31.2–31.4 k.

## Re-measured after row 11, A0 and C6 (2026-09-17)

*The 14-row table*, one binary, the four new switches on vs off (`LOFT_NO_LEAF_CHAIN`,
`LOFT_NO_LAZY_BUFFER`, `LOFT_NO_NESTED_IN_PLACE`, `LOFT_NO_APPEND_STAGING`), `compare.py
--skip-interp --repeat 3`, two passes each, 14/14 hashes, `LOFT_HOIST_VERIFY=1` clean over the
whole bench:

| row | on (ns/op) | off (ns/op) | change |
|---|---:|---:|---:|
| parse | 29.8–29.9 k | 39.3–40.0 k | **−25 %** |
| smooth | 1.18–1.20 k | 1.28–1.36 k | −8 to −12 % |
| hair | 26.7–27.3 k | 28.8–29.3 k | −7 % |
| fill_star | 22.1–22.3 k | 23.9–24.1 k | −7 % |
| wide_line | 12.8 k | 13.7 k | −7 % |
| lock | 3.03 M | 3.18–3.19 M | −5 % |
| lock_curved | 3.13 M | 3.22 M | −3 % |
| fill_circle | 59.5 k | 61.0–61.2 k | −2.5 % |
| render_lock | 15.06 M | 15.29–15.34 M | −1.7 % |
| fronds | 175–177 k | 178 k | −1 to −2 % |
| hash, composite, render_marks, resize | — | — | equal |

Most of the rows other than `parse` move through row 11 (the lean frame).  Two costs the first
build of A0 carried were found by this table and removed before it: a dead join buffer's
guard in `lock_ribbons`' loop (`hoist::dead_buffers` now counts the witness of a free guarded
for a value local), and a lazy mint counted as growth for `@FR-R-Base`, which took the
element bases out of `lock_layer`'s loop (+3.9 % instructions on `lock_curved`, found by
`perf stat` on a `lock_curved`-only bench).  The `render_*` and `resize` rows move ±3 % with
code LAYOUT alone — measured: `resize`'s own functions are byte-identical under
`LOFT_NO_LAZY_BUFFER` and its time still moved — so a ±3 % change on those rows is not
evidence either way; read them with `perf stat` instructions.

*The parse row, attributed again* (`scripts/native_attrib.py`, 14 742 samples):

| family | § P0b | now |
|---|---:|---:|
| store mint/free | 20.0 % | **13.7 %** |
| store access | 9.6 % | 13.3 % |
| vector field copy | 9.2 % | 12.6 % |
| record mint/copy/move | 11.0 % | 11.8 % |
| vector append/grow | 6.7 % | 9.0 % |
| frame prelude | 6.4 % | **gone** |

(Shares of a row that is a quarter shorter, so a family that held still in time grew in share.)
By line: `:704` is still first at 11.3 %, now almost all `vector_add` (6.9 %) — the `pts` and
`widths` copies into the element.  Then `no_mark()` minting a `Mark` store per call (`:499`,
1.9 %), the `Mark` results' `pts` copies at the parser exits (`:709`, `:545`, `:831`, `:836`,
≈ 5 % together), and `:986` copying `parse_fronds`' `Mark` (1.2 %).

## C6 — a nested record literal built inside the element (BUILT 2026-09-17, `@FR-R-InPlaceLiteral`)

*The row.*  P0b charged `drawing.loft:704` with a `Paint { … }` built in a store of its own
and deep-copied into the element's inline `paint` field, per frond; the same shape is at
`:594` and in `parse_lock`/`parse_background`.  The hand patch
(`bytecode-comparisons/C6-nested-literal-hand-patch.py`, the five sites written straight into
the field) measured **31.2–32.0 k → 28.5–28.7 k ns/op** (−8.5 %), instructions −5.4 %, hash
unchanged — twice what the attribution predicted, because the removed store churn also
relieves the cache.

*The shape built.*  `Parser::nested_literal_place` primes an EMBEDDED record field's value
with the field's own place (`OpGetField(outer, pos, kt)`) when the value starts with the
field type's name and `{`, so `parse_object` takes the field road it already has (the
precedent is `parse_some_payload_object`) and writes the nested fields in place; omitted
fields take their declared defaults through `object_init`.  Declined: a `reference<T>`
field (a pointer), a nullable one (a tagged enum), and any destination the program can read
(`x = S { … }`, `o.f = S { … }`, `v[i] = S { … }` — a nested field expression could read the
old value after the outer literal has begun overwriting it).  A value that turns out to be
more than the literal (a postfix) is parsed again the ordinary way, as loft#1304's retry is.

*What the cells found first — loft#1548.*  Cell n4 (a nested field expression that appends
to the SAME container) answered `0` for `1030` on both backends on the build BEFORE C6: the
element road minted the element before its field expressions ran, against
`(E-Asgn-Compound)`.  It was the whole class, not the nested case — `s.qs += [Q { a: i, b:
g(s) }]` read 0 for 24.  Filed and fixed in this arc (`formal/operational-history.md`
D-op-10): `Parser::stage_append_fields` evaluates every scalar or text field value, and runs
every nested construction, ahead of the mint whenever a field value reads the container's
root.  The parse bench's emission is byte-identical under it (no field there reads its own
container).  Guard `tests/scripts/1548-an-appended-literal-reads-its-own-container.loft`.

*Verified.*  Seven cells (fresh element, a vector in the nested record, a partial literal,
the loft#1548 shape, two levels deep, a rebound local, a readable destination) hold on both
backends in both switch states under `LOFT_STRICT_STORES`, `LOFT_POISON` and
`LOFT_NATIVE_LEAK_CHECK`; with the switch off the parse bench's emission is byte-identical
to the build before C6.  Pins `tests/nested_in_place.rs`, subject `codegen`.

*Measured* on the parse bench: the emission reaches the hand patch's instruction count
(1 870 M → **1 770 M**, −5.4 %), the row **31.2 k → 29.1–29.5 k ns/op** (−6 to −7 %), cycles
−7.0 %, hash `33f6d2b8`, clean under all three falsifiers.  Together with row 11 and A0 the
row has moved from 38.8–41.7 k to 29.1–29.5 k this session.

## The runtime under the row (2026-09-17, later the same day)

*Item 4 re-measured before it was cut — and it is not a move.*  At `:704` both copies read a
VIEW or a reused BUFFER: `pf_line = f.fpts` and `pf_wids = f.fwid` view an element of
`fronds`' result, and on the smoothed path they are `smooth_pts`/`smooth_vals`' buffers, which
A0's guard mints once and every later iteration reuses.  Every copy lands in the scene's store,
a different one.  Moving a vector across stores still copies its bytes, and moving a reused
buffer's vector out would make its next call re-grow it — so there is no move to make.  The
parsers' exit `Mark { pts: <local> }` copies are the same shape into a store of their own.
The copied lengths say where the cost actually is (`LOFT_COPY_DUMP=1`, per parse): 29
`vector_add`s, 22 of them two elements long.  The time is the destination's CLAIM, not the
bytes.

*What the release profile named instead* (`perf` by symbol, then the branch stack for callers):
`begin_write_inner::<u32>` 7.5 % — the write check, out of line, and called mostly by the
allocator's own metadata writes (`claim`, the free-tree inserts and rotations, `Store::init`);
`Stores::store_mut` 3.4 % in two out-of-line copies; and `vector_add`'s per-element
`copy_claims` walk for an element that owns no heap.  Three runtime changes stayed.  None
changes the IR, and all three apply to both backends:

| change (`perf stat -r 5`, one emission, one core) | instructions | cycles |
|---|---:|---:|
| `vector_add` skips the claims walk when the element owns no heap | −3.3 % | −3.3 % |
| the write check inlined, the lock refusal outlined | −8.5 % | −7 % |
| a fresh store initialised once (`null` then `clear` initialised it twice, on both backends and in B2's placement fallback); a retype moves no budget nothing reads | −1.1 % | −2 % |

The row: **28.3–28.5 k → 25.0–25.3 k ns/op** (−12 %), instructions 1 742 M → 1 523 M, cycles
429 M → 380 M, hash `33f6d2b8`, about 3.5× the Rust reference (≈ 7.2 k).  Same-moment A/B of
all fourteen rows (the pre-session rlib against this one, one emission, best of three, 14/14
hashes): every row equal or faster — `lock_curved` −21.5 %, `lock` −12 %, `render_marks`
−12 %, `render_lock` −11 %, `fronds` −10 %, `parse` −8.5 %, `smooth` −5 %, `hair` −4 %,
`resize` −3.5 %, the rest within ±3 %.  `LOFT_STRICT_STORES` still reports a read and a write
of a freed store — a positive control by hand on the emission, since no test asserts that the
report fires.

*With every row judged.*  The drawing bench's own `bench.rs` carried no reference for the four
late rows; @PLN157's `bench-reference-scene.rs` is that reference, and it is now appended to
the package's `bench.rs` (loft-libs-graphics `drawing-lock`, 47947ec).  Measured with it
(`compare.py --skip-interp --repeat 3`, 14/14 hashes agree, the four late hashes the recorded
`33f6d2b8`, `432ddd47`, `fa8b1c64`, `77de7581`; the reference lane moves a few per cent run to
run, the package's own run the same hour read 3.7×, 4.1×, 4.8× and 5.8×):

| row | Rust ns/op | native ns/op | native / Rust | @PLN157's last full table |
|---|---:|---:|---:|---:|
| parse | 7 280 | 25 940 | **3.56** | 7.59 |
| render_lock | 2 882 380 | 12 795 200 | 4.44 | 5.46 |
| resize | 17 739 680 | 84 760 660 | 4.78 | 5.57 |
| render_marks | 887 700 | 5 582 880 | 6.29 | 7.96 |

The reference lane reads what it read then (render_marks 887 k both times, render_lock and
resize within 2 %), so the ratios moved on the native side.  `parse` is under the 4× bar for
the first time; the three rows still over it are the resample's (@PLN157), not this plan's.

Three levers measured and dropped.  Testing the slot's `free` flag before the strict-mode switch
in `store`/`store_mut` (with the report outlined) measured −2 % instructions on this row — and
the fourteen-row A/B then read `smooth` +38 %, `fill_circle` +15 %, `fill_star` +7 %,
`wide_line` +6 % with instructions flat: the pixel loops load the flag per access
(`cmpb $0x0, 0x106(%rbx,%r15)`) and spill a loop variable, where the static switch test left
them alone — which is what `store`'s own doc says the order is for.  Keeping the switch first
and only outlining the report did not recover it, so the accessors are as they were.
Reserving the whole copy before `vector_add` claims (+0.2 % instructions — an empty
destination's first claim already has room for eleven elements), and the capacity test without
its division (cycles unchanged; the `divl` samples were skid).  The lesson is the one § Re-measured
already states: a runtime change reaches every row, so the fourteen-row A/B is owed before it
stands, and a single-row `perf stat` that moves cycles without instructions is a code-shape
effect to look at, not noise.

*The census.*  A parse mints 30 stores: `Mark` 12, `vector<Pt>` 6, `vector<float>` 4,
`PointList` 3, and one each of `vector<text>`, `vector<Frond>`, `Sketch`, `Paint` and
`FrondSpec`.  So `Mark` is the largest class: `parse_circle`'s `no_mark()` on the five lines it
does not match, and the literal exits of `parse_circle`, `parse_fronds` and `parse_line_cmd`.
C5 armed removes three of the twelve (`parse_poly`'s) and still measures a wash (−2 %
instructions, +1 % cycles).  It declines the other three functions, each for its own reason:
a source whose copy lands in either arm of an `if`, a source with no place outside the frame
(`pf_all`), and a body that names the container inside a loop.

*`:986` is B1's shape behind a null-init.*  `ps_f = parse_fronds(…)` sits inside an `if`, and
loft scopes `ps_f` to the loop body, so the scope pass puts `ps_f = null` in front of the `if`.
On both backends the call bind is then the SECOND `Set`, which takes the rebind copy: the
interpreter mints a store at the null-init (`OpInitRef`) and copies into it, and native mints
in its copy arm.  Either way that is two mints and a deep copy where B1's adopt would do.
Probe: `if n == 4 { f = try_a(n + 1); … }` against a top-level `m = try_a(n)`.  The rule it
wants: the one call bind after the scope's null-init, with no loop between them, is a first
bind — and the null-init is then a sentinel on the interpreter too.  Worth about 1 % of this
row.

*What reusing the `Mark` buffers first ran into — loft#1549.*  Pooling a buffer whose record
owns heap (`(R-Reuse)`, shipped with #1465) leaked on both backends: the callee's literal
overwrote the record's handles on every refill, and what the previous call left there stayed
claimed in the buffer's store — about 100 bytes a turn for `S { n, v: [..] }`, 600 for a
vector of heap-owning records, unbounded in the loop length, with right values and no gate
that counts stores able to see it.  Fixed on the way (`formal/heap.md` D-heap-12, the record
clause of `(H-ClearRelease)`): the pool's lazy guard releases on reuse,
`if OpRefIsNull(b) { mint } else OpClear(b, T)`.  The first cut released in the callee's
offered arm and measured +1 % instructions on this row — B2 offers a freshly placed `Paint` to
every `read_paint`, where the walk finds nothing — so the release is the pool's, and the row's
emission differs from before only in one arm that never runs (`parse_circle` returns after its
one `read_paint`).  Twelve cells and the resident-memory guard
(`tests/scripts/1549-a-pooled-buffer-releases-its-previous-occupant.loft`, falsified against
04cc50b1 on both backends); pins `tests/pooled_buffer_release.rs`.  What it unblocks here:
the pool is now sound for a `Mark`, which owns a vector.

### The queue after the runtime pass

1. **The `Mark` class** — twelve stores a parse.  Two separate questions: `no_mark()` behind
   `parse_circle`'s tail (a constant, heap-free result handed up a buffer chain), and the
   three literal exits C5 declines.  Pooling the callers' buffers is sound for a heap-owning
   record since loft#1549; what still keeps them null is B1's exclusion (D-own-43) and the
   null-init below.
2. **B1 behind a null-init** (`:986`, above) — both backends, one home for the admission.
3. **The free tree under a live store.**  Once a store has freed anything, `bump_tail` is off
   for good, so every claim is a tree delete, a split and an insert.  In the scene's store
   that is every element vector.  A tail block kept OUT of the tree (a wilderness) would give
   the common claim `bump_tail`'s cost back — about 6 % of the row sits in `fl_*`,
   `claim_block` and `set_free_header`.  It is an allocator change with `@FR-H-FreeFooter`
   and store images in its blast radius, so it gets its own cells.
4. **Text per character** — `split` calls `text_character` and `OpLengthCharacter` per
   character, both out of line across the rlib (2 %).
5. **A1/A2** — the store family, re-priced after items 1–2.

## B1b — the caller's buffer handed in (BUILT 2026-09-17, `@FR-O-Buffer`, default ON)

*The queue's first two items were one question.*  The `Mark` buffers `parse_scene_at` hands
its parsers were kept null by B1's exclusion (`minted_pairs`, D-own-43), and the one bound
inside an `if` (`ps_f`) took the rebind copy because of the pre-init in front of it.  Lifting
the exclusion (`LOFT_NO_ADOPT_BUFFER_REUSE=1` restores it) was the plan; the cells
(`bytecode-comparisons/B1b-adopt-buffer-reuse-cells.loft`, r1–r25, hand-computed, each in a
loop) said what it owed first.

*What the matrix found — one false sentence at three sites, and two gaps beside it.*  The scope
pass's own doc for the promoted buffer read *"this function mints its store"*, true only while
the caller hands the sentinel.  Handed a live store:

| # | where | shape | measured |
|---|---|---|---|
| 1 | the interpreter's rebind (D-own-43) | `cv = Canvas { … }; cv = alloc(…); cv` (r1), the value reading `cv` (r4), a `Var` rhs (r5) | `USE AFTER FREE` on `--interpret`; native's `_rb_w_` guards its own rebind |
| 2 | the scope pass's exit leg (loft#688) | a chain `return no_mk()` beside a literal `return Mk { … }` (r9, r10 — `parse_circle`'s shape) | `USE AFTER FREE` on BOTH backends: the literal exit freed the caller's buffer |
| 3 | native's copy arm at a pre-init rebind | `if c { m = scan(…) }` in a loop (r22 — `ps_f`) | the copy's source-free released the pooled store, native only; the interpreter's post-wrap reset freed it too, which only defeated the pool |
| 4 | every top-of-body insertion | a body the scan wraps as `Insert([Set(sc, null), Block])` because its result is a hoisted local (r25 — `parse_scene_at` itself) | the pool, A0's lazy mints and the `__rbo_`/witness initialisers all matched a bare `Block` and skipped it in silence |
| 5 | native's value-record gate | any promoted buffer the snapshot below mentions (`find_word_from`, `read_number`) | "the return buffer is used": the scanner lost its registers, 248 `Scan` mints per two parses |
| 6 | native's first bind of a PREDECLARED `__ret_N` | a pure chain `fn outer() -> Mk { scan() }` (found after B1b shipped, by the chain cells' c5) | the prologue binds a returned `__ret_N` up front, so its one bind read as a reassignment: the copy freed `scan`'s answer — the caller's pooled buffer.  Unarmed native answered `m4 417` for 827 |

*The mechanism.*  (1–2) One fact, read at every free: the scope pass snapshots the store a
promoted record buffer was handed (`__rbw_<buf> = OpRefAlias(buf)`, `Function::entry_witness`,
kept live as long as the buffer), guards the exit legs with `OpDistinctStore(buf, __rbw_<buf>)`,
and the interpreter's rebind frees read the same variable (`OpFreeRefIfDistinct(v, entry)`, or
the new `OpFreeRefUnlessEntry` where the displaced store is already on the stack).  (3) The one
bind after an `if` pre-init is recorded as a first bind (`Variable::deferred_first_bind`, cache
format 8) when it is the local's only assignment and a B1 adopt; both bind arms take the adopt.
(4) `scopes::body_block_mut` finds the body through the wrapper, for every site.  (5) The
value-record gate and emitters account for the snapshot: a phantom buffer's alias is the null
reference.  (6) A predeclared local's first body `Set` is a first bind in native's B1 arm too
(`Output::predeclared`), as it always was on the interpreter; guard
`tests/scripts/164-a-chain-of-chains-adopts-its-callees-answer.loft` with its sabotage patch.
`LOFT_TRACE_POOL=1` names the gate that declines a buffer.  `(O-Buffer)` gained the
callee-side clause, and D-own-43 is closed (`formal/ownership.md`).

*Two defects the cells found that predate this unit*, both reproducing on the pre-session
build (a0ae1ad0):

* **r23 — FIXED here.**  `scan_mk(3, i).pts[0]?` binds the call to an inline container temp
  (`Parser::bind_inline_container`).  The block pre-registration in `scan_inner` hoists that
  temp out of its block only when its type has no deps YET, and for a B1 callee (a promoted
  local, a chain beside a literal exit) the parser's dep on the call's buffer is stripped
  later, inside the block — so the temp stayed there, the block's exit free was suppressed
  because the block's result is the temp, and one record leaked per call on both backends.
  The hoist now asks `adopts_minted_at_bind` as well (and the hoisted null-init makes the
  bind a `deferred_first_bind`), and `adopts_minted_at_bind` no longer reads `__ref_p2_N` as
  a buffer name.  Under `LOFT_NO_ADOPT_FIRST_BIND=1` the leak returns with B1's own lowering.
* **r17 — filed as loft#1550, FIXED 2026-09-17** (`silent-wrong`, both backends, on `main`):
  `if first { a } else { h.c }` joined two borrows with different bases into
  `Join { base: a }`, which `formal/ownership.md`'s own `γ(Join(b))` does not cover, and every
  `Join` reader assumes an owned arm — the caller's record was freed and read back as another
  record's bytes.  Changing the lattice regressed four guards (1257, 1257b, 1318, 1323), so the
  rule sits at the call boundary instead: a callee whose return may hand back any of several
  arguments (`use_analysis::returns_one_of_several_args`) answers a base-less `Join`, and
  every bind of it copies with no runtime guard against one argument — the nullable first
  bind included, and the interpreter's rebind through the fresh path (`D-own-45`).  Cells
  `bytecode-comparisons/1550-two-borrow-join-cells.loft` (j1–j12, five red on both backends
  before), guard `tests/scripts/1550-a-view-of-one-of-several-arguments-is-copied.loft` with
  its sabotage patch, pins `tests/one_of_several_args.rs`.

*Receipts.*  Guard `tests/scripts/164-adopt-buffer-reuse.loft` (26 cells) with the sabotage
patch `tests/falsified/164-adopt-buffer-reuse.patch` (the witness and the deferred bind removed,
the pool kept); pins `tests/adopt_buffer_reuse.rs` (the witness in the IR, the guarded exit, the
pool reaching a wrapped body, the switch, the adopt at r22 on both backends, a value record kept,
the census 303 → 246 interpret / 270 → 204 native, the inline container released).  B1's pins now run with B1b off, and B1's
own census moved 108 → 98 / 106 → 101 with the deferred bind and the wrapped body.  The
`wrap` and `native` corpora are green.  On the parse bench the `Mark` mints are 24 → 14 per two
parses, hash `33f6d2b8`, clean under `LOFT_STRICT_STORES`, `LOFT_POISON` and the leak gate.

*Measured — and the first measurement said no.*  With the pool on, the parse row was SLOWER:
+1.5 % instructions, +1.1 % cycles (`perf stat -r 5`, one emission per state, hash
`33f6d2b8`).  The store mints it removes were paid back by the release the pool owes each
reuse (`OpClear` → `remove_claims`, loft#1549): `owned_walk` builds its child list for every
heap-OWNING type, also when the record holds nothing — and a `Mark` refilled after `no_mark()`
holds nothing on most lines.  `Stores::holds_no_heap` (a runtime change, both backends) reads
the heap-owning slots first and skips the walk when all are empty, conservatively for every
kind it does not read.  After it, pool on against pool off on one rlib: **1 499.6 M against
1 506.8 M instructions, 373.5 M against 377.5 M cycles** — and the exit alone moved the pool-off
build from 1 515 M to 1 507 M.

The fourteen rows (`--n 50`, best of five interleaved rounds, one core, 14/14 hashes agree).
The runtime change first, one emission against the rlib before and after it: every row equal
or faster, `smooth` −6.7 %, `parse` −3.2 %, `fill_circle` −2.6 %; `fill_star` and `composite`
read +2.6 % / +1.2 % at `--n 50` and −0.7 % / −0.1 % instructions at `--n 500` (their function
bodies are byte-identical; the short run carries loader noise on this hybrid CPU).  Then the
pool, one rlib:

| row | pool on | pool off | on / off | pool off, before the exit |
|---|---:|---:|---:|---:|
| hash | 108 440 | 110 080 | 0.985 | 107 700 |
| hair | 26 440 | 27 520 | 0.961 | 26 820 |
| smooth | 1 160 | 1 180 | 0.983 | 1 140 |
| fronds | 153 660 | 154 500 | 0.995 | 155 520 |
| lock | 2 659 300 | 2 755 320 | 0.965 | 2 722 340 |
| lock_curved | 2 458 040 | 2 493 980 | 0.986 | 2 514 640 |
| composite | 196 700 | 196 240 | 1.002 | 195 600 |
| fill_circle | 56 860 | 58 020 | 0.980 | 59 320 |
| fill_star | 21 480 | 21 240 | 1.011 | 21 900 |
| wide_line | 12 280 | 12 240 | 1.003 | 12 280 |
| **parse** | **25 360** | 25 840 | **0.981** | 26 500 |
| render_lock | 12 973 180 | 13 068 060 | 0.993 | 12 821 880 |
| render_marks | 5 665 660 | 5 762 140 | 0.983 | 5 612 900 |
| resize | 85 537 320 | 86 836 960 | 0.985 | 84 812 480 |

So the unit's time gain on its own row is small (−1.9 %, ≈ −4 % with the exit), as P0's
arithmetic said a store-pair class would be; what it removes structurally is five `Mark` stores
a parse and the three frees that could release a caller's store.

### B2 unit 1 over a chain (BUILT 2026-09-17, `@FR-R-Place`'s callee clause)

*The shape.*  `parse_circle` answers `no_mark()` on two exits and `Mark { … }` on one.  A
chain exit hands the function's buffer to the callee and RENAMES it after the chain's work
ref (`__retbuf` → `__ref_N`), so unit 1's `unpromoted_return_buffer_var` found nothing and
the literal exit minted a store of its own on every matched line, while the chain exits
wrote the caller's pooled buffer.  The same held for a literal exit before a chain TAIL
(`outer2`, the scanners' `… scan_fail()`) and for a literal tail after chain exits
(`read_uint`).

*The rule.*  `Parser::chain_return_buffer_var` accepts a compiler-generated `__ref_N` buffer
that nothing but a chain names — a `one_buffer_chain` block, or the tail call it is handed to
(`tail_hands_on`) — and a chain exit counts as compatible in `literal_exits_into_buffer`; a
literal tail beside chain exits takes the same buffer in `classify_reference_delivery`.  A
chain always returns, so a literal exit never meets a value another statement put in the
buffer.  A promoted NAMED local is never such a buffer (`pick`'s literal keeps its store —
`164-adopt-first-bind` c3's reason).  Same switch: `LOFT_NO_LITERAL_EXIT_BUFFER=1`.

*What it cost to get right.*  The first draft un-admitted every scanner on native (258 `Scan`
mints in a two-parse run where there had been none): the chain's buffer is a PHANTOM value
local, and once the literal wrote it, its mentions inside the converted `Object` were uses
`local_uses_ok` did not account — so `scan_fail`'s site "consumed its record" and the
admission fixpoint emptied.  Those mentions vanish with the block, as `retbuf_uses_ok`
already said for a phantom that is no local; `local_uses_ok` now says it too, and the
value-record verdicts on the bench are identical to the pre-unit build.  The value channel
could not see this — only `tests/literal_exit_buffer.rs`'s signature pin did (confirmed by
reverting the clause).  The chain cells also found row 6 of the B1b table.

*Receipts.*  Cells `bytecode-comparisons/B2-chain-literal-exit-cells.loft` (c1–c17,
hand-computed: the chain mid-body, at the tail, both, recursive, two levels deep; a vector
field from a local also appended into a parameter; an omitted default; a loop local, a
rebind, a bind inside an `if`, an append, a discarded call, a fn-ref, two live results; the
value record in both shapes), clean on both backends under `LOFT_STRICT_STORES`,
`LOFT_POISON`, `LOFT_POISON_CLAIM` and the leak gate, pooled and unpooled and under the
switch; every other @PLN164 cell corpus and the @PLN157 § V-a value-record corpora likewise.
Guard `tests/scripts/164-a-literal-exit-writes-its-chains-buffer.loft` (INERT at 49040d27 —
an optimisation), pins in `tests/literal_exit_buffer.rs`; the B1b census pins moved with it
(pooled 246 → 197 interpret, 204 → 174 native).  `wrap` and `native` green.

*Measured* (the pre-unit generator against this one, both emissions compiled against ONE
rlib, since the unit changes generation only; hash `33f6d2b8`):

| | before | after | |
|---|---:|---:|---:|
| `Mark` mints, `--n 2` run (lean tier and `--native` alike) | 21 | 12 | −9 |
| parse row, instructions (`perf stat -r 3`, `--n 5000`, whole process) | 1 875.3 M | 1 849.8 M | −1.4 % |
| parse row, cycles | 461.4 M | 450.9 M | −2.3 % |
| parse row, ns/op (`--n 5000`) | 24 066 | 23 471 | −2.5 % |

The fourteen rows (`--n 50`, best of five and of seven interleaved rounds, one core): `parse`
−3.3 % in both runs, every other row within noise — `lock` read +3.4 % in the first run and
−0.8 % in the second, on function bodies that differ only in a source-path comment.

### The queue after B1b

1. ~~**The `Mark` class** and **B1 behind a null-init**~~ — built above, and the literal exits
   of the chain-renamed parsers with it (§ B2 unit 1 over a chain).  The 12 `Mark` mints left
   in a two-parse run are the pool's own: one per pooled call site per activation (five
   parser buffers), which only the activation arena (A1/A2) removes.
2. ~~**loft#1550** (r17)~~ — fixed above, at the call boundary.
3. **The free tree under a live store** and **text per character** — as before.
4. **A1/A2** — re-priced after item 1.

## The three tiers — the invariant each rests on

Each tier is a situation the compiler PROVES (C120: no rewrite may change a value after a
fault; only situations we know), switchable and falsifiable in the @PLN157 style, and
lands on BOTH backends where it changes the IR (the interpreter is the values oracle, and
`O-NoDiverge` says the two translate the same `deps` facts).

**Tier 1 — the activation arena.**  *Invariant:* a hidden return buffer (`__ref_N`, a
`__ref_p2_N` discharge buffer, a literal's temp) never outlives the activation that minted
it (`O-Buffer`: the buffer is the caller's store, freed at frame exit; a result that must
survive is adopted, copied or moved OUT of it).  So every buffer of an activation may be a
RECORD in one store instead of a store each, allocated past a mark taken at entry and
released to that mark at every exit.  The store is the CALLER's, threaded down like the
buffer itself is today (`R-Callee`: "a return buffer may be a record the caller offered"),
so a loop such as `parse_scene_at`'s line loop resets one arena per iteration — the
compiler can see nothing minted since the loop head survives except what was appended to
`sc`.  Removes the store mint/free pair per temporary: `Store::init`, `database_named`,
`store_budget`, `close_file_handle`, `owned_walk`, `free_named` — about a fifth of the row.
*What it must keep:* every rule keyed on STORE IDENTITY (§ Edge cases E1).

**Tier 2 — adopt at first bind, move at last use.**  *Invariant:* `O-Move` — a returned
heap value's ownership transfers to the caller's binding.  Today `OpBindOrCopy` honours it
only when the local already holds a store; the first bind copies.  Adopting the buffer at
the first bind makes iteration one what iteration two already is.  And `B-Copy`'s copy at
`paint: pp_paint` is a copy of a value nobody reads again: when the ownership oracle says
the local OWNS its store and liveness says it is dead on every path after the assignment,
the field takes the record by MOVE (bytes relocate, heap handles never change store —
`@FR-R-MoveAppend`'s relocation, applied to one record).  Removes the deep-copy class, about
a sixth of the row.

**Tier 3 — build where it will live.**  *Invariant:* a value whose ONLY destination is
known at the site it is built may be built THERE (`@FR-R-ElemFirst` states it for a vector
consumed by one append; this generalises the destination).  Three shapes: an indexed
overwrite from a literal writes the slot's fields (the old element's owned heap released
first, a partial literal's omitted fields set to their defaults); a call whose result is
stored exactly once gets the destination slot as its return buffer, so `smooth_pts` fills
`Op.pts` directly; a `?`-discharged record element that is only read is a VIEW (`B-View`),
not a copy.  The per-type prefill image belongs here.  Removes the remaining copies and
most of the prefill.  The one copy that stays: a vector stored in TWO destinations
(`Op.pts` and `Mark.pts`) — a record field owns its data, and that is the right answer.

## P0 — what one activation's buffers cost, measured (2026-09-15, this x86-64 box)

The parse bench (`parse_only.loft --n 2000`, `--native-release`, hash `33f6d2b8`) read
**50.5–54.8 k ns/op** before any change.  Its emission (`--native-emit`) carries **287**
`store_nr !=` and **39** `store_nr ==` tests — the sites E1 would touch — 37 `OpDatabase`
+ 45 `OpDatabaseNP` mints and 26 `OpCopyRecord` copies.  The runtime census
(`LOFT_TRACE_DB=1`, two parses) puts the store mints per parse at **~88**: Paint 19, Mark
19, `vector<Pt>` 18, `vector<float>` 16, PointList 6, Elem 6, Sketch 2.

**What a mint/free pair costs.**  A vector declared inside a loop re-mints its store per
iteration under `LOFT_NO_LOOP_BUFFER_REUSE=1` and keeps it by default (§ V-al), so the A/B on
one program prices the pair: **28 → 88 ns/op, ≈ 60 ns** for the smallest buffer (a mint, a
first claim, a free).  Perf on that probe splits the 60 ns about evenly between the
store-specific half (`op_database_inner`, `Store::init`, `Stores::clear`, the zeroing
memset) and the buffer's first claim (`claim`, `claim_block`, `finish_claim`,
`pre_alloc_vector`), which an arena RECORD pays too.

**So tier 1's ceiling is 88 × 60 ns ≈ 5.3 µs ≈ 10 % of the row**, half of that the
store-specific part an arena or a pooled store can remove; the plan's "a fifth" counted the
profile's lifecycle routines, which include per-access costs (`store_mut`, `begin_write`)
that no store discipline changes.  The hand patch of `parse_poly`'s four eager buffers is
therefore not worth building: those are 3 of 4 unused per activation (one `smooth_pts` path
runs), 12 pairs per parse, ≈ 0.7 µs — the arithmetic answers it.  Verdict for the queue:
**B (the copies, 18.5 % inclusive) and C (the prefill and the discharge copies) before A**,
and for E1 the pooled-store option is the one to price first when A1 is cut, because it
keeps identity as it is.  The leak-gate extension (E19) waits for A1's shape.

## B1 — adopt at first bind (SHIPPED 2026-09-15, `@FR-O-Move`)

*The two protocols, side by side.*  `a = mk_lit(i)` (every return a literal, deps `[]`)
and `b = mk_loc(i)` (`o = P { … }; …; o`, the local promoted onto the buffer, deps
`["o"]`) lowered differently: `a` adopts a buffer the caller mints once and reuses
(`OpFreeRefIfDistinct(a, __ref_1)`); `b`'s caller minted a SECOND store and deep-copied
(`OpCopyRefOrNull` / the `_dst`/`_src` delivery), freeing the callee's — two mints, a copy
and two frees per call where the callee's one mint would do.  The parse row's `pp_raw =
read_points(s)` (a `PointList` of four vectors) and the bench's `bs_sk = parse_scene(…)`
(the whole `Sketch`) are that shape.

*The rule.*  `use_analysis::adopts_minted_at_bind` — ONE home for the three readers — admits
a direct call to a loft-defined callee whose return deps name exactly its hidden return
buffer attribute, bound to a plain local (not a parameter — a promoted local is one — not a
caller-hidden buffer, not handed to the call as its buffer).  `scopes::scan_set` then strips
the local's deps (it OWNS the minted store) and pairs it with the call's buffer for the
identity-guarded free the fresh-adopting shape already takes; the interpreter's first-bind
arm delivers by `OpPutRef` and native's dispatch takes the plain assignment.  A rebind keeps
the in-place copy on both backends.  Switch `LOFT_NO_ADOPT_FIRST_BIND=1` (parse time).

*What the matrix found.*  `bytecode-comparisons/B1-adopt-first-bind-cells.loft` (c1–c17,
hand-computed, both backends under `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_POISON_CLAIM`
and the leak gate) was green before the change and RED after the first cut on the
interpreter alone: c6, plan 51 cluster 3's `render_lit_then_call`.  The pairing enrolled the
buffer in `reuse_record_buffers` (the entry-time pool, `@FR-R-Reuse`), so the callee received
it non-null, its promoted local's literal built into it, and the rebind `cv = alloc_canvas(…)`
FREED it — the interpreter's reassignment path has no twin of native's `_rb_w_` witness — after
which the caller's next `Q` took the recycled slot and `p.tag` read it.  The callee's IR is
identical before and after; the free was a no-op on a null buffer.  So B1 excludes its
pairings from the pool (`Scopes::minted_pairs`), the buffer stays null, and the callee mints
per call: **139 → 108 mints** over the cells on the interpreter, 137 → 106 on native.

*Receipts.*  Guard `tests/scripts/164-adopt-first-bind.loft` (its `@falsified-at:` is the
pool sabotage above), pins `tests/adopt_first_bind.rs`, the ten plan-51 guards green on
both backends under both switch states.

*Measured.*  The parse bench (`parse_only.loft --n 2000`, `--native-release`, this quiet
x86-64 box, hash `33f6d2b8` throughout): **50.5–54.8 k → 43.3–44.0 k ns/op**, about
−14 % on the row — the `PointList` copy at every `read_points` bind and the whole-`Sketch`
copy at the bench's own `bs_sk = parse_scene(…)` are the binds it removes.  The consumer
table (`compare.py`, 14 rows) is re-measured when the next unit lands, per the plan's
phase 5.

**B1b — reuse the buffer across activations for this callee shape** (E7's steady state, 0
mints per call): blocked on the interpreter's reassignment path freeing a caller-supplied
buffer held by a promoted buffer local (`143`'s shape); native guards that free with its
entry-time `_rb_w_<buffer>` witness (loft#1126).  Closing the divergence is the whole of
B1b, and c6 under the pool without the exclusion is its falsifier.

## C4 — the prefill image (SHIPPED 2026-09-15, `@FR-R-Prefill`)

*The mechanism.*  `Stores::set_default_value_nullable`'s `Shape::Record` arm walked the
type's fields on every mint that is not proven complete-write — a store resolution per
field, a recursion per inline record, a typed setter per sentinel — for bytes that are the
same on every mint of a type (7 % of the parse row).  The arm now asks
`prefill_from_image` first: the type row carries a `PrefillImage` cell beside its
`TypeFacts`; the first `Absent::Prefill` of a COMPLETE type zeroes the span, runs the walk
once and reads the span back as the image; every later mint is one `Store::write_image`
(a bounds-checked block copy, shadow-tagged as `zero_range` is).  The image is what the
walk writes BY CONSTRUCTION — it is never computed a second way — and
`LOFT_PREFILL_VERIFY=1` is the falsifier: the walk re-run after every image write, a panic
naming the type where the bytes disagree.  Declines, each keeping the walk: a type with a
field not laid out yet (`u16::MAX` content — `layout_complete` walks the inline record
tree; an image captured then would freeze the field's zero where the walk, once the field
is laid out, writes its sentinel), an image whose length no longer matches the layout, and
`Absent::Final` (the `text as Struct` fill: declared defaults and interned text are not a
fixed byte pattern).  A table rollback forgets the image with the facts.  The all-zero
fast path (`heap_facts(tp).1` → `zero_range`) stays in front of it.  One runtime home, so
the interpreter, `--native` and wasm take it together.  Switch `LOFT_NO_PREFILL_IMAGE=1`.

*What the matrix found.*  The prefill is VALUE-INVISIBLE in every shape the cells reach:
a literal writes each omitted field's default explicitly (`to_default`, loft#914), a
`?`-discharge of an absent element answers `construct_default(T)` (LOFT.md § `?`:
`points[i]?` → `Point{}`, the declared default and the inline record's defaults included
— c8 measured it equal to a bare `Mix {}` on every binary), and the `text as` fill is
`Final`.  So the value channel cannot move under a wrong image; the receipt for "no
value changes" is the VERIFY census: all 1432 `tests/scripts` files on the interpreter
(the one panic is `75-native-stub`'s `@EXPECT_FAIL`), the parse bench on native, and the
cells on both backends under `LOFT_PREFILL_VERIFY`, `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES` and the leak gate.  Two side-findings, neither C4's: a NEGATIVE
declared default (`n: integer = -1`) was dropped by a `text as Struct` cast because
loft#876's fold matched plain literals and `-1` is `OpMinSingleInt(1)` — fixed in
`typedef::fold_declared_default` with guard
`tests/scripts/a-negative-declared-default-survives-a-cast.loft`; and a `single`-typed
declared default (`a: single = 1.5`) writes 0 in a struct LITERAL on the interpreter and
fails native compilation (`set_single` handed an `f64`), on the pre-B1 binary too — to be
filed (`hit-by:loft`, `silent-wrong`), repro in the next session's notes.  And a doc
inconsistency for C3: `formal/binding.md` `(B-View)`'s discharge clause says "an absent
element discharges to null" where LOFT.md § `?` and every backend answer the type's
default record; the rule text is the one to correct when C3 is cut.

*Measured.*  The parse bench (`parse_only.loft --n 2000`, `--native-release`, hash
`33f6d2b8`, interleaved A/B on one binary): **46.4–48.5 k (`LOFT_NO_PREFILL_IMAGE=1`) →
42.3–42.8 k ns/op** (≈ −11 %).  The 14-row `compare.py` table is re-measured when B2
lands.

*Receipt and gate (the next session).*  The sabotage receipt: `prefill_from_image` made to
capture the image BEFORE the walk (an all-zero image), and under `LOFT_PREFILL_VERIFY=1`
both backends panic at the first image USE — but only after cell c11 was added: on native
every `Mix` literal is a complete-write mint (`OpDatabaseNP`) and `mk`'s buffer is minted
once per activation of its caller, so c1–c10 CAPTURE the image on native and never use it
(the native half of the receipt was vacuous, which `LOFT_TRACE_PREFILL=1` now makes
visible: it names each capture and each use, and `tests/prefill_image.rs` pins a use on
BOTH backends — 2368 / 15 uses of `Mix`, interpret / native).  The guard is
`tests/scripts/164-prefill-image.loft` with the panic text as its `@falsified-at:`; the
five pins pass; fmt and both clippy legs are green.  The negative-default guard's
`make falsify` against `7fd2a664` could not run on this box (`/tmp` is a 7.4 GB RAM tmpfs
and the control build needs more; point `LOFT_FALSIFY_CACHE` at a disk path) — its receipt
stays the hand measurement recorded in the file.

## B2 — the result built where it will live (SHIPPED 2026-09-16, `@FR-R-Place`, `@FR-R-MoveLast`)

*What the instrument found first.*  `loft introspect` on the B2 cells' `read_paint` (four
literal exits, the library's shape) showed the callee answering a DIFFERENT store per
exit: the three mid-body `return Paint { … }` each minted a `__ref_p2_N` work-ref store
and returned it, and only the tail literal wrote the `__retbuf` the caller handed
(@PLN157 § V's `BuildIntoBuffer` rewrote the tail alone).  `(R-Place)` declines exactly
that callee — *on some exit answers a store other than the buffer it was handed* — so the
row's own callee could not be placed until the callee clause held.

*Unit 1 — every literal exit writes the handed buffer (SHIPPED).*
`Parser::literal_exits_into_buffer`, run on the function body right AFTER the tail's
delivery is dispatched: when the buffer is still the unpromoted `__retbuf` and every
mid-body exit is a fresh literal of its type, each `return S { … }` takes the tail's own
rewrite (`build_into_return_buffer`: the literal's `OpDatabase` behind the "caller offered
a record" guard, its writes on `__retbuf`, the work-ref `skip_free`).  It is decided after
the tail and not at the `return` because the first cut — a mirror inside `parse_return` —
ran BEFORE the tail promoted a local onto the buffer, so `mk_mix`'s early literal and its
promoted local `o` shared one buffer and the caller freed a stale ref (the corpus's
`164-adopt-first-bind` c3, a `BUG (#306)` stack-store free refusal).  A function with any
non-literal exit keeps its per-exit stores.  Switch `LOFT_NO_LITERAL_EXIT_BUFFER=1`; pins
`tests/literal_exit_buffer.rs` (the IR shape on `read_paint`: 4 of 4 exits on `__retbuf`,
the switch restores 1 + 3, the promoted-local shape declined, the cells on both backends).
No value moves and, under B1's null buffer, no store count moves: the unit changes the
callee's CONTRACT, which is what units 2–3 consume.

*Units 2–3 — the caller side (SHIPPED 2026-09-16).*  `src/place_result.rs`, one IR pass
for both backends, run inside `scopes::check` after the scan phases have settled the
function's frees (E18).  For a plain local `v` bound once from a loft-defined callee whose
every exit is a fresh literal into its `__retbuf` (unit 1's contract), read only as the
receiver of native operations while it holds the record, and stored ONCE per path into a
record-literal field of an `_elm_` element `OpNewRecord` appended to a PARAMETER's
collection, three sites move: `__ref_N = null` → `OpPlaceRecord(X, tp)` (a record claimed in
`X`'s store, prefilled through the C4 image), `OpCopyRecord(v, dst, tp)` → `OpMoveRecord(v,
dst, tp)` (the bytes relocate, the heap handles keep their claims, the source's block is
released on the spot), and the exit pair → one `OpFreeRecordIn(v, tp)` on a path that still
holds the record and NOTHING on a path that stored it.  The ops are `#rust` templates
(`default/01_code.loft`); the runtime is `Stores::place_record_prefilled`, `move_record_out`
and `free_record_in` (with a store-root arm for the null-host fallback, which answers a
fresh-store buffer).  Switch `LOFT_NO_PLACE_RESULT=1`; `LOFT_TRACE_PLACE=1` names every
admission and decline.

*What the matrix found — four things the design did not say.*  (1) The candidate gate is
not B1's: `adopts_minted_at_bind` admits the promoted-local callee shape; an all-literal
callee (`read_paint`) takes the older fresh-adopting pairing, so the pass gates on its own
`placeable_bind` and asks the callee's exits directly.  (2) An exit has THREE spellings —
`return { Object …; __retbuf }`, `{ Object …; return __retbuf }` (a function with text
work-refs frees them between the writes and the return), and the parser's bare tail
`{ Object …; __retbuf }` before the scope pass wraps it (what the copy notice's preview
sees) — and the local's exit free has two
as well: `OpFreeRef(v)`, or in a record-returning caller `OpFreeRefIfDistinct(v, __retbuf)`
(the library's `parse_poly` answers a `Mark`; the first cut declined it).  (3) The design's
"`v = null` after the move" is unsound: the interpreter lowers a rebind of an owned record
local as a store-level free of what it held — it would free the HOST — where native emits a
plain null; and the first cut's zeroing of the source was a runtime stand-in for the path
state (the owner's question: why zero a source once zero-on-claim is gone?).  So the pass
records the state at every exit and writes the free that is right there, and a path that
stored the record rejoining one that holds it declines; `(R-MoveLast)`'s mechanics clause
now says so.  (4) The copy notice reads the parser's IR on the program path (the lint family
runs before the scope check there and after it under `loft test` — a pre-existing difference
the dead-store lint's tests pin, so it stays), and reported a relocated field as a copy that
"could not be moved"; it now asks the pass for its verdict in preview
(`place_result::admits`, the same walk on the parser's IR, tolerant of the null init the
scope pass has not yet prepended).

*Receipts.*  Cells b1–b15 exact on both backends under `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES`, `LOFT_HOIST_VERIFY` and `LOFT_NATIVE_LEAK_CHECK`; every intended decline
fires for its own reason (`LOFT_TRACE_PLACE`); the census 184 → 50 mints, one fewer per
admitted call (`add_poly` ×64, `add_px` ×30 no-heap, `add_card` ×40 text-owning).  Guard
`tests/scripts/164-place-result.loft`, `@falsified-at:` the pass made to free on every exit
(both backends die with `Store access out of bounds … the reference is corrupt`).  Pins
`tests/place_result.rs`: the IR shape of `add_poly`, the switch, the silent copy notice, the
census.  The corpus (`tests/scripts` + `tests/docs`) has NO admitted bind — the shape is the
library's, not the corpus's — so the guard is the only coverage.
The gate's one red, the browser kernel differential, was a pre-existing dangling `Data`
pointer in `wasm::resume_frame` that the changed bytecode made visible (loft#1541, fixed in
the same arc with `State::rebind_data`).

*Measured.*  The parse row (`parse_only.loft --n 2000`, `--native-release`, hash `33f6d2b8`,
this x86-64 box, 2026-09-16): `pp_paint` admitted (three moves, one held exit), and a WASH.
Wall time on this box now spreads ±10 % between runs of one binary, so the receipt is
`perf stat -r 5` on the cached binaries: user instructions 8.39 G against 8.39 G, cycles
3.18 G against 3.23 G, cache misses 6.09 M against 6.11 M (placement / switch off), every
difference inside its own run-to-run spread.  The reason is the scene: no gradient is used,
so the `Paint` relocated carries no `spec` data — the copy it replaces was ~30 bytes and the
store it removes is matched by a claim and a delete in the host store.  What B2 buys on this
row is structural (no per-call store, no per-call copy), not time; the copy class the
profile measured at 18.5 % is the points (`pts: smooth_pts(…)`, `Mark.pts`), which are C2
and C5.  A leak-probe build (the placed block leaked instead of deleted) measured the same,
so the host store's free-tree churn is not a cost either on this row.

## C3 — the read-only `?`-discharge (SHIPPED 2026-09-16, `@FR-B-Disturb`, `@FR-B-View`)

*The premise did not survive the matrix, and that is the result.*  C3 was cut to make
`ap_e = sc.elems[idx]?` a VIEW instead of a copy.  It already is one, on both backends:
the bind carries the base's dep (`ap_e(1):ref(Elem)["sc"]`), no copy op is emitted, and
the `__ref_p2_N` store is minted only on the ABSENT arm — which `acc_pts` never takes,
because `elem_index` appends the element before `acc_pts` is called.  A store census of
the shape (`LOFT_TRACE_DB=1`, the library's `acc_pts` reproduced against the same `Elem`)
mints exactly THREE stores: the `Sketch`, the points vector, and the `Elem` temp of
`sc.elems[idx] = Elem { … }`.  Thirteen discharge shapes were measured — off a parameter,
off a local, a hash lookup, a nullable field, an all-scalar element, an element with a
vector field, a struct-enum element, a `&`-based base, a `const` bind, a nested field, a
rebind in a loop, an explicit `?? default`, a plain undischarged bind — and every one is
already a view.  The only copies the sweep found are the two that are RIGHT: a
materialise under `(B-Disturb)`, and the return of a view as a value.

So the evaluation table's row *"a store per `?`-discharged record element + a copy of the
`Elem`"* is the SAME store and copy as its own next row, *"a store for `Elem {…}` + a copy
into `sc.elems[idx]`"*, counted twice.  That row is C1's, and C1 is where the `acc_pts`
temporary actually is.

*What C3 turned out to be.*  Its own condition, as the rewrite list states it: *"`sc.elems`
not disturbed between bind and last read, **in this frame or any callee**"*.  The view
existed; the condition was only half-checked.  `ViewWalk::disturb` asserted `(B-Disturb)`
at one site but computed its input from THIS frame's ops alone, so a view live across a
call kept reading an address its elements had left:

| shape | inline | through a callee |
|---|---:|---:|
| `e = sc.els[0]?; grow(sc, 200); e.a + e.b` | 3 | **4294967401** |
| the same with two appends (no reallocation) | 3 | 3 |
| `len(e.nm)` after the growth | 2 | **1** |
| `e = sc.els[2]?; remove_first(sc); e.a` | 30 | **40** |

Both backends, identical — the divergence is IR-level.  One shape with two meanings,
decided by which side of a call the append sits on, and by an allocation the author cannot
see.  `(B-Ref-Reshape)` already states the reach — *"the disturbance may be in this frame
or in anything the frame CALLS … at any depth"* — and `(B-View)` keys its materialise on
the same four events, so the rules settled it and only the implementation was short.

*The mechanism.*  `disturbed_params_map`: the container places each definition GROWS or
REMOVES FROM through a visible parameter, closed over the call graph by the same worklist
`removed_params_map` uses, unioned into `ViewWalk::disturb` at every call site.  Read
through the same two producers the inline walk uses, so the `OpClearVector` subtraction
comes with them and a callee that REBUILDS the field it was handed disturbs nothing,
exactly as that statement written inline does.  The forward edge does not ask whether the
parameter is spelled `&`: a plain heap parameter aliases the caller's container identically
(`calls.md` F-ParamHeap, probe 40 cell X9), so keying on the spelling would let an author
lose the materialise by taking loft's own `warn_redundant_amp` advice.  Switch
`LOFT_NO_CALLEE_DISTURB=1`, trace `LOFT_TRACE_DISTURB=1`.

*What the matrix found that the design did not say.*  (1) A binding that names a container
WHOLE (`d = &cv.data`) resolves to the same place as one that names an element inside it
(`e = sc.els[i]?`), because the place model carries one field offset — but a growth moves
every ELEMENT while it merely repoints the field SLOT the first one re-reads.  Shaking it
broke `157-view-header`'s `grown_between` (11 → 0).  `view_source_place_indexed` answers,
off the same walk that answers the place, whether the chain crossed an element read.
(2) The notice had to name the CALLEE: nothing in `c_grow_callee` appends to `sc`, and a
reader given only *"`sc` grows"* goes looking for a statement that is not in the function —
the same failure the `Grown` sentence was split off from `Reshaped` to avoid.
(3) A hidden RETURN BUFFER is an argument slot, so without excluding it every
record-returning function that fills a vector field reports a disturbance at each of its
call sites.

*Receipts.*  `tests/scripts/164-callee-disturb.loft`, fifteen PAIRS — each callee cell
beside the same program written inline, because the inline path has implemented
`(B-Disturb)` since loft#1373 and is therefore the file's own oracle — with hand-computed
absolute values beside them, clean on both backends under `LOFT_POISON`,
`LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES` and `LOFT_NATIVE_LEAK_CHECK`.  Five cells are
CONTROLS that must keep aliasing and prove it by writing through the view and reading the
container back.  `@falsified-at:` is measured, not claimed: under the switch seven cells
move, the same seven on both backends.  Pins `tests/callee_disturb.rs`.

*The two honest costs.*  c7 — a removal BELOW the viewed element — now materialises where
today's alias answers correctly, because `(B-Disturb)` ends the place a removal renumbers
whatever happened to sit where, and because the same program written inline has answered
that way since @PLN130 F2.  That is probe 38 cell C1's objection, and it is a behaviour
change with no wrong answer behind it.  And the `&`-link twin `link_inline` reads `0` where
`11` is due — a `&` link silently downgraded to a copy, which `(B-Ref-Reshape)` says loft
will not do.  It is pre-existing (it reads `0` on `main` and under this unit's switch
alike), it is a `&` question rather than a plain-view one, and it is FILED rather than
widened into here.

*Measured on the parse row:* nothing.  C3 removes no copy, because there was none to
remove — what it removes is a silent wrong answer.  The row's copies are C1, C2 and C5.

*The cost it does have is COMPILE time, and it was worth checking.*  `disturbed_params_map`
walks every definition once, and `scopes::check` runs per FILE LOAD, so a program with several
`use`d libraries runs it again over an ever-larger definition table — the shape that would be
quadratic.  A/B on one binary (`LOFT_NO_CALLEE_DISTURB`, three runs each): a stdlib-only script
72 ms against 70, `07-control-flow` 57 against 57, and the two library-heavy corpus files
145 ms against 145 and 144 against 142 — inside the run-to-run spread at the size where it would
have shown.  The whole-program map is built once per `check` rather than re-derived per call
site, which is what keeps it there.

## C1 — the element overwritten from a literal (step 1 SHIPPED 2026-09-16, `@FR-R-InPlaceLiteral`)

*Re-measured first, per C3's lesson, and the row is real this time.*  `sc.elems[idx] = Elem {
… }` mints a `__ref_p2_N` store, builds the literal in it and `OpCopyRecord`s it into the
slot — confirmed on the `acc_pts` shape and on four element shapes (full literal, partial
literal, all-scalar element, element with a vector field).  The store census of the library
shape mints exactly three stores and this is the third.

*And the re-measurement found the phase is half built already.*  `(R-InPlaceLiteral)` names
two destinations, an element `v[i] = R { … }` and a field `o.f = R { … }`, and the FIELD one
has taken the in-place road since long before this plan: its writes take the place as their
receiver, it mints no store, and it already writes the declared default of every omitted
field.  Only the ELEMENT destination copies.  So C1's optimisation is narrower than the rule
reads.

**Step 1 — the staging clause, which was broken (SHIPPED).**  That shipped in-place road
staged nothing, so an initialiser read storage an earlier field's write had already replaced:

| shape | got | want |
|---|---:|---:|
| `o.f = El { a: o.f.b, b: o.f.a }` over `{a:1, b:2}` | **2,2** | 2,1 |
| the same with the second field read through a call | **2,2** | 2,1 |

Both backends, in silence.  The ELEMENT twin answers correctly — because it copies — which is
what makes it the oracle and what makes this the prerequisite: redirecting elements to the
place FIRST would have propagated the defect to them.

#330's hoist is exactly the cure and already existed; it only ever armed for a WHOLE-VARIABLE
rebind (`v = S { … }`), because `in_place_var` doubles as the retry path's target and that is
meaningful only for a variable.  `parse_object` now takes a separate hoist root from a
PROJECTION destination too, read through `projection_container_place`.  The root carries the
destination's field OFFSET and `Parser::reads_place` spares a read of that variable at a
DIFFERENT field, so `tw.a = El { a: tw.b.a }` takes no temp; the spare is taken only on proof,
because sparing wrongly costs the value and staging wrongly costs a stack temp.

*The one conservatism, stated because it is a choice.*  Only `OpGetField` proves disjointness.
A field read also arrives through the typed accessors (`bx.tag` is `OpGetText(bx, 28)`), whose
second argument is an offset for some ops and a SIZE for others, so separating them needs a
list of ops that reads offsets — and such a list drifts silently against a new op, which is
PERFORMANCE.md § Design P8's measured failure mode for the five mutation deny-lists.  So a
sibling read spelled with a typed accessor stages a temp it does not need.  It is bounded by
the literal's own field count, it is pinned either way so a later narrowing is deliberate, and
closing it wants the field's declared SPAN rather than another list.

*Receipts.*  `tests/scripts/164-in-place-literal.loft`, eighteen cells hand-computed from the
declarations, clean on both backends under `LOFT_POISON`, `LOFT_POISON_CLAIM`,
`LOFT_STRICT_STORES` and `LOFT_NATIVE_LEAK_CHECK`; `@falsified-at:` measured by disabling the
projection arm — c1 and c8 go red on both backends and every other cell is unmoved, which is
what says the arm stages the initialisers that read the place rather than staging everything.
Pins `tests/in_place_literal.rs` hold the IR shape.

**Step 2 — the element destination written in place (SHIPPED 2026-09-16).**  The gate was one
op name.  `parse_object` builds into whatever `code` it is handed — the probe shows it receiving
`Call(OpGetField)` for a field and `Call(OpGetVector)` for an element alike — and the arm that
mints a work-ref instead asked `is_field(code)`, which is `Call(OpGetField)` and nothing else.
`Parser::builds_into_element` admits the element place; switch `LOFT_NO_ELEMENT_IN_PLACE=1`.

*Four declines, and every one was bought by a measurement rather than by the design.*  (1) The receiver is emitted ONCE PER FIELD
WRITE, so the place must be re-derivable with no effect the program can see: a repeatable base
and an index that is a LITERAL or a BARE VARIABLE.  `v[bump()]` would call `bump` once per
field; `v[len(v) - 2]` would re-read a length the writes may have moved.  (2) The index is read
from the END of the argument list — `OpGetVector(base, size, index)` and `OpVectorRef(base,
index)` put it at different positions, and a fixed slot read the SIZE as the index and declined
every `OpVectorRef` place by accident.  (3) A COLLECTION field declines the whole type, and this
is the one that was MEASURED rather than reasoned: the staging clause binds a field expression
to a temp, and what that bind MEANS depends on the type — `(B-Copy)` copies a text, so a
literal reading the slot's own text is safe, while `(B-View-Base)` makes a collection projection
a VIEW.  In place, `v[i] = S { c: e.c }` staged a view of the slot's own vector, the write
released that vector before storing the handle back to it, and the field read `0` on BOTH
backends.  Deciding it per FIELD needs the self-read answer, which is not known until the fields
are parsed and the road is already taken; deciding it by TYPE is decidable up front.  Coarse,
and the safe direction — and the shape this phase exists for keeps the road, because
`acc_pts`'s `Elem` is text and scalars.

(4) A member of a LINKED COLLECTION GROUP declines, and the CORPUS bought this one — the cells
did not reach it.  `@FR-Col-Group` makes two collections over one element type two ROUTES to a
single record set, so an element write owes them the unlink-and-relink that `group_elem_write`
wraps the copy with; written straight into the slot the whole of it is skipped and `by_k` goes
on holding the record under the hash of its OLD key, which is loft#900's defect by another road.
`a-group-element-written-through-the-vector-member-reaches-every-member` is what caught it, and
its own header had already written the sentence.  The predicate is new
(`is_grouped_vector_elem`) rather than the existing `vector_group_elem_site`, because that one
recognises `OpVectorRef` alone — the site it serves sees nothing else — while this admission also
allows `OpGetVector`, so reusing it would have let the other spelling through.

*The asymmetry between the two lists is deliberate:* the ADMISSION names two element ops, the
DECLINE names four including both nullable spellings.  A miss in the admission costs the
optimisation; a miss in the decline costs the group's agreement.

*And `reads_place` grew a clause for it:* a read of a variable whose DEPS name the root counts
as a read of the place.  `e = sc.els[i]?; sc.els[i] = El { a: e.b, … }` reads the very slot the
literal overwrites, under another name — `acc_pts`'s own shape — so without it step 1's swap
would have come back one spelling over, on the very road step 2 opens.

*Measured:* the `acc_pts` shape's store census **3 → 2**, which is the `Elem` temp the
evaluation table charged to the `?`-discharge and C3 showed belongs here.  The row's remaining
copies are the POINTS, which are C2 and C5.

*Receipts.*  The guard reaches 25 cells, clean on both backends under every falsifier; all four
declines are cells, because a decline that silently stopped declining is the expensive
direction.  Step 2's falsifier is STRUCTURAL and the pins carry it — with the switch every cell
still passes, since the copy is correct, so what the switch costs is a store and a deep copy
that no value can see.  The pin asserts BOTH element spellings, having first passed vacuously by
asserting one.

## C2 — the destination as return buffer (SHIPPED 2026-09-16, `@FR-R-Place`, `@FR-O-Buffer`)

*Re-measured first, and this row IS real* — unlike C3's.  `h.v = mkints(n)` on an EXISTING
field place emits, per assignment:

```
__ref_1(1):vector<integer> = null;                        // the callee's buffer, a store of its own
__p154_rhs_1(1) = n_mkints(n(0), __ref_1(1));             // the callee fills it
OpClearVector(OpGetField(h(0), 8i32, 24i32));             // the destination is emptied
OpAppendVector(OpGetField(h(0), 8i32, 24i32), __p154_rhs_1(1), 0i32);   // every element copied in
OpFreeRef(__ref_1(1));                                    // the buffer freed
```

and what it should emit is

```
__ref_1(1):vector<integer>["h"] = OpGetField(h(0), 8i32, 24i32);   // skip_free, inline_ref
n_mkints(n(0), __ref_1(1));
```

— a store, an element-by-element copy, a free and a temp gone per assignment.  The caller's
clear is subsumed: the callee's own FIRST op is `OpClearVector(o)`.

**It is a pure-IR rewrite, and a first reading of this section said otherwise.**  That reading
assumed the buffer must become a place-valued ARGUMENT, which would break the dep model — the
call site mints the buffer as `Value::Var(vr)` and gives the result `Deps::frame1(vr)`, and B1's
adopt, the delivery arms and the scope pass's free sweep all key on that variable.  The buffer
does not have to stop being a variable.  Only what it HOLDS changes: the destination's DbRef
instead of a store of its own.  Every downstream reader is untouched, and the one real edit is
re-pointing the variable's deps from `Deps::none()` (owned) to the destination's base, plus
`skip_free` / `inline_ref` — the pattern `group_elem_write` already uses for its `found` temp.

What licenses it is the CALLEE's side: `n_mkints(n, o)` only ever does `OpClearVector(o)`,
`OpPreAllocVector(o, …)`, `OpPushInt(o, …)`, `return o` — it never mints a store for `o`, and
the caller already emits those same ops against a field DbRef today, so a field place is a valid
buffer.

⚠ **But that is a property of SOME callees, not of the lowering, and a first cut read it as the
lowering.**  Three callees were sampled and all three filled the buffer they were handed, which
made `(R-Place)`'s *"a callee that may hand back a store it did not mint"* look vacuous.  It is
not: a callee returning a vector LITERAL lowers to `OpDatabase(__vdb_1)` on its buffer
PARAMETER — it MINTS into the buffer — so handed the destination it mints over the place, and
the write lands in a record the destination does not name.  The corpus found it
(`1152-a-vector-value-into-a-group-reaches-every-member`, a refusal from loft#810's guard:
*"record N claims size 0 … freed or never written"*).  The admission therefore READS THE CALLEE
and declines a body that mints into any argument slot — the positive form of the rule's decline,
as B2 unit 1 is for records.  Generalising from three samples is the error, and the rule had
already written the answer down.

### The restrictions, re-derived against the measurements

`(R-Place)` lists five declines.  Measured against what the vector lowering actually emits, they
do not all survive as written, and saying which is the point of this section:

| the rule's decline | verdict, measured |
|---|---|
| an ARGUMENT reaches the destination's store | **essential, and true by construction** — `grow`'s first op is `OpClearVector(o)`, before any read of `v`, so `h.v = grow(h.v)` would read an emptied vector |
| a path READS the result after the store | **not needed for this clause.**  It belongs to the other `(R-Place)` clause — a buffer claimed in S and then relocated, where the source is moved-from.  Here the result NAMES the destination, so a read of it reads exactly what was written |
| a callee that may hand back a store it did NOT mint (`O-Opaque`) | **survives, and the first reading of this row was wrong.**  `passthrough` copies its parameter INTO the buffer and answers the buffer, which made the decline look vacuous across three sampled callees — but a callee returning a vector LITERAL mints into its buffer parameter (`OpDatabase(__vdb_1)`) and would mint over the destination.  Implemented as a positive admission that READS the callee's body |
| a callee that on some exit answers a store OTHER than the buffer | **already true for every callee measured** — `two_exits`' early `return [99]` still clears and appends into `o` and answers `o`.  This is the contract B2 unit 1 had to CREATE for records; the vector lowering establishes it already.  Kept as a cheap ASSERTED check, never re-derived |
| a `?` / `??` discharge on the result | **structurally excluded** — the right-hand side is an `ncc` BLOCK, not a `Call`, so the admission never sees it |

And two the rule does not list.  A destination that is a member of a LINKED COLLECTION GROUP is
**not** a decline here, where it was in C1: the maintenance is `OpClearKeyed(g.by_x)` …
`OpIndexGroup(g.es, g.by_x)` BRACKETING the fill, and a call sits inside that bracket, which
C1's in-place literal had nowhere to hang.  It needed
`group_reindex_after_vector_write` to learn the shape — it keyed on an `OpAppendVector` naming
the field, and this rewrite emits none, so the vector filled and the keyed view stayed empty
(`3,0,0` for `3,3,4`).  And a field of a struct-ENUM VARIANT **is** a decline: it lowers through
a variant check whose other arm is the null sentinel, so the place exists only if the enum holds
that variant and does not unconditionally EXIST as `(R-Place)` requires.

### Where the rules are short — `(O-Buffer)`

`(O-Buffer)` says a hidden return buffer IS THE CALLER'S STORE, freed at frame exit, with the
result's free guarded by store identity against it *"which, no static bit can say
(`O-Opaque`)"*.  A buffer that IS the destination place is not a store, is not the caller's to
free, and outlives the frame.  The two rules describe different objects.

This is **not** an open design call.  `(R-Place)` is the newer rule, written for this plan, and
it already decides it — *"the buffer IS the place and nothing moves"*; `(O-Buffer)` predates it
and is simply short.  So `(O-Buffer)` gains the clause rather than `(R-Place)` giving way, and it
is recorded as a change to a SHIPPED rule's text, which is the kind that gets said out loud.

### What it needs

A `LOFT_NO_BUFFER_IS_PLACE` switch, one aliasing test, a positive admission (direct call,
loft-defined, hidden vector buffer, every exit answers it), the group bracketing preserved, and
the 11-cell oracle in `bytecode-comparisons/C2-buffer-is-the-place-cells.loft` — whose values are
TODAY's answers and which C2 may not move.

### Shipped — and what the bench then measured

*Mechanism.*  `Parser::buffer_is_the_place` (`src/parser/expressions.rs`) re-points the buffer
VARIABLE at the destination — `inline_ref` plus `skip_free`, the way `group_elem_write` re-points
its own temp — where the right-hand side is a direct call to a loft-defined callee with a hidden
vector buffer, the destination place exists, no argument of the call reaches it, every exit
answers the buffer and no exit MINTS into it; `group_reindex_after_vector_write` keeps a grouped
destination's maintenance around the fill.  Switch `LOFT_NO_BUFFER_IS_PLACE=1` (parse time, both
backends).  Guard `tests/scripts/164-buffer-is-the-place.loft` (11 cells, the two corpus-bought
declines among them), structural pins `tests/buffer_is_place.rs`, registered under the `scopes`
subject.

*Measured, and the measurement is a FINDING rather than a confirmation.*  The row is real where
it is spelled: on `h.v = mkints(n)` the rewrite removes a store, an element-by-element copy, a
free and a temp per assignment.  **The parse bench spells it nowhere.**  `--native-emit` of
`bench/parse_only.loft` is byte-identical with the switch on and off (25 686 lines either way,
under `LOFT_NO_CACHE=1` too, with `parse_poly` and `smooth_pts` in the emission), and the row is
where B1 left it: 41.8–44.8 k ns/op, hash `33f6d2b8`.  The naturalness question — § How a verdict
is reached, question 2, which this cut asked of a probe of its own construction instead of the
corpus — answers the same way: the 15 library files of `loft-libs-graphics` carry **13**
`place = call(…)` sites and not one of them has a VECTOR result (twelve scalar, one `text`).

*So the row the rewrite list credits to C2 is a different DESTINATION.*  `pts: smooth_pts(…)` is
a record-LITERAL field of an element being appended, reached through a local
(`pp_pts = smooth_pts(…)`) — B2's `place_result` shape with a vector instead of a record — and it
declines today because `pp_pts` has TWO owning destinations, `Op.pts` and the returned `Mark.pts`.
C5 removes the second one; only then is there ONE destination to place into.  C2 therefore ships
with its row credited honestly: **the rewrite for the spelling it admits, structural, zero sites
in the consumer measured so far** — and the parse row's points class stays with C5 and the
vector `place_result` that follows it.

## C5 — a returned record's heap field as a view leaf (`@FR-O-ViewField`, `@FR-R-ValueRecord`)

*The row, re-measured before the phase is cut* (the lesson C3 and C2 each cost).  A parse of the
bench scene mints **~68 stores** (`LOFT_TRACE_DB=1`, three parses divided out): `vector<Pt>` 18,
`vector<float>` 16, `Paint` 15, **`Mark` 12**, `PointList` 3, `Sketch` 1.  The `Mark` row is one
store per parsed line — the hidden buffer for a record of two booleans and a vector — and beside
it one deep copy of the points into that record.  `Mark` is `(R-ValueRecord)`'s shape in
everything but the vector: two scalars and one heap field.

*The three rungs, PROVEN as emitted Rust before a line of the compiler changed* (the codegen
gate: the target hand-written from the real emission, compiled against the release rlib, run
beside the record form).  Probes `bytecode-comparisons/C5-*.loft`; each target answers exactly
what the record form answers and is clean under `LOFT_NATIVE_LEAK_CHECK`, `LOFT_POISON`,
`LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES` and `LOFT_STORES=warn` — and the leak instrument was
shown able to FAIL on the same binary (dropping one `OpFreeRef` reports `1 stores not freed at
program exit: kt=88 Sc×1`).

| rung | the loft shape | the leaf the tuple carries |
|---|---|---|
| **R1** (⚠ NOT admitted — see § *Built* below: the discharge's ownership is a JOIN) | `sc.ops += [Op { opts: [] }]; o = sc.ops[len(sc.ops) - 1]?; …; Mark { …, mpts: o.opts }` — the field's source IS a view of a parameter-rooted place | the source expression itself: `DbRef { …var_o…, pos: pos + 0 }` |
| **R2** | `p = mk_pts(n); sc.ops += [Op { opts: p }]; Mark { …, mpts: p }` — the NATURAL form: the source is a local whose COPY landed in a parameter-rooted place | the append's own destination, the element temp: `DbRef { …var__elm_1…, pos: pos + 0 }` |
| **R3** | `if n < 0 { return Mark { matched: true, bad: true, mpts: [] } }` — an exit with NO place to view | `DbRef::NULL`.  A null view reads as the empty vector the record form built: `len` 0, an iteration of nothing, `v[0]?.px` 0, a `const` argument 0 — measured equal on every read the oracle makes |

**R3 is what decides whether C5 reaches the consumer at all**, and it was the phase's real
question: the library's own `parse_poly` carries `return Mark { matched: true, bad: true, pts: [] }`
and `no_mark()` is that literal whole, so an implementation that declined a place-less exit would
decline the function the row belongs to.  The empty literal writes NOTHING in the record form —
it is the prefill's own zero — so the value form owes exactly a reference that reads empty, and
`DbRef::NULL` is that reference.

**R2 is what the consumer spells**, and it is the rung that also unlocks the vector
`place_result` C2 could not reach: once the returned field views the element's copy, `pp_pts` has
ONE owning destination and the call that fills it can build there.

### The cases, written down before the first is worked

Admissions R1–R3 above.  The declines are the falsifier list, one cell each:

| # | case | why it declines |
|---|---|---|
| D1 | the source's owning destination is in the FRAME (`p = mk_pts(n); Mark { mpts: p }`, no append into a parameter) | the view would dangle at the return — `(O-ViewField)`'s first condition |
| D2 | a site WRITES through the field (`m.mpts += [Pt { … }]`) | the rule's site condition: a write through a view is a write into the container the caller never named |
| D3 | a DISTURBANCE between the call and the last read (`m = add(sc, 3); sc.ops += [Op { … }]; len(m.mpts)`) | growing `sc.ops` relocates the element the view names — `(B-Disturb)`.  Both directions: a disturbance AFTER the last read still admits |
| D4 | the disturbance is through a CALL (`m = add(sc, 3); grow(sc); len(m.mpts)`) | C3's `disturbed_params_map` is the reach; the argument the leaf roots in is what must stay undisturbed |
| D5 | a site stores, returns or hands the whole record to a by-value parameter | the existing site gate already declines it; kept as a cell so a gate change cannot quietly admit it |
| D6 | the function is a library's exported `pub fn` | `(R-Escape)`: an unseen caller never receives a view, so the boundary materialises |
| D7 | the source local has TWO owning destinations outside the frame | which place the view names is not decided; declining costs the optimisation only |
| D8 | the callee grows the viewed container after establishing the view (appends a SECOND element, views the first) | the same relocation as D3, inside the frame |
| D9 | a `text` field, a keyed-collection field, a nullable result | outside step 1's leaf types; the record form stands |

Each decline is also a cell that must keep answering the record form's value, and D2/D3/D4 are the
three that would be SILENT — the class `silent-wrong` names — so each gets a cell whose value
MOVES when the decline is removed, not merely a structural pin.

### What it needs

`LOFT_VIEW_FIELD=1` (generation time, native only — the interpreter keeps the record form and is
the values oracle, as § V-aa's value record already does), the leaf analysis with ONE home read by
the gate and the emitter, the site gate extended with the read-only and disturbance conditions,
the live-reload arm reading the field's reference out of the returned record, and the cells in
`tests/scripts/164-view-field.loft` with structural pins in `tests/view_field.rs`.

### Built 2026-09-16, opt-in — and what the build corrected

*Mechanism.*  `hoist::value_records` admits a record that owns heap when every heap field is a
plain `vector<T>` the body can deliver as a view: `leaf_source` reads the exit's own
`OpAppendVector` into the buffer — the deep copy the leaf removes — and answers either the PLACE
to view or `Null` for an exit that writes the field not at all.  `leaf_root` resolves that place
to a `(parameter, field)` pair by SHAPE, following a local through its single assignment; the
tuple part is `DbRef`, the site's `OpGetField` read becomes a tuple index
(`ViewFieldReadEmitter`), the live-reload arm reads the field slot out of the record the
interpreter answers, and a value local's declaration binds `DbRef::NULL` for the leaf because
`Default` has no null for a reference.

*R1 is NOT admitted, and the reason corrects the rung proven by hand.*  The hand-written form
`o = sc.ops[len(sc.ops) - 1]?; … Mark { mpts: o.opts }` reads a `?`-DISCHARGE, whose ownership is
a JOIN: the absent arm mints its record in the frame's own store, so a leaf naming the container
would be a view of a store freed at the return — wherever the element was absent.  The
hand-written proof was sound only because that probe's element is always present, which is
exactly what a compiler may not assume.  So the ONE admitted callee shape is R2, the natural one,
and R3's null view rides with it.

*The site conditions, and the measurement that cut them.*  A leaf read is admitted in two
contexts — an argument at a `const` parameter, and an operand of an op that answers a VALUE
(`OpLengthVector`, the null tests).  An ELEMENT READ is not one, and that is measured rather than
argued: `m.pts[0]?.px = 100` reads the leaf with `OpGetVectorNullable` — an op that only reads its
container — and then WRITES through the element it answered.  On the emission the gate produced
before the list was narrowed, that write landed in `s.ops[0].opts`, which read 109 where the
oracle says 9.  The other measured decline is the REMOVAL: with the span test disabled,
`a = mk(s, 2); b = mk(s, 5); s.ops.remove(0); len(a.mpts)` read `5,30` — the second element's
points — where `a`'s own copy holds `2,3`.  An APPEND does not move the value in any shape
measured (fifty growths in a row included), so `b2`/`b3` stand on `(B-Disturb)` rather than on a
wrong answer of their own, and that is recorded in the guard rather than smoothed over.

*The span test is an UPPER bound, and it had to be written as one.*  `scopes::grown_containers`
is a documented LOWER bound — a missed disturbance costs a materialise there — and reusing it
admitted `b2` and `b3`, because an `OpNewRecord` naming its container as `(var, field)` is
uncollected without the store and because the op's own argument looked like a licensed call
argument.  The rule the site gate uses instead is on MENTIONS: between the bind and the last read
the container's root variable may be named ONLY as an argument of a user call whose disturbance
summary does not reach the place.  Every way to grow a container names the variable that reaches
it, so a mention cannot be evaded; what it costs is a statement that merely READS the container
in that span, which declines.

*What declines today, each costing the rewrite and never a value:* a body that names the
container in more than one statement (so `parse_poly`'s per-path appends decline — the per-PATH
version is step 2), a site that reads the container between the bind and the last read, an
element read or an iteration at the site, a `pub` function (`(R-Escape)`: the cdylib bridge
materialises a tuple field by field, and a reference is not a field it can write), and a `text`
or keyed field.

*Two declines the CORPUS bought, and both are one mistake: an answer that was a FALLBACK where
it had to be a proof.*  `leaf_source` read "no `OpAppendVector` into the buffer's field" as "the
exit leaves the field empty", and a field can be filled without an append — a returned vector
LITERAL pushes element by element (`OpPushInt(OpGetField(buf, off, _), …)`) and a record
literal's vector field appends ELEMENTS through the record (`OpNewRecord(buf, <record>, <field
nr>)`).  Read as empty, each delivered a NULL view for a vector the program had filled:
`723-ncc-loop-element-bind` measured `len` 0 where its program built eight, on the corpus, with
nothing else to say so.  The empty answer is now positive — every mention of the buffer in the
exit must be one the value form ACCOUNTS for (the allocate-or-reuse guard, a scalar `OpSet*`, the
one recognised append, the block's own yield), and anything else declines — because the value
form drops the whole block, so an unaccounted mention is work the tuple would lose.  Cells `d6`
and `d7` are the two shapes, pinned.

*Receipts.*  `tests/scripts/164-view-field.loft` — 2 admissions, 2 positive controls and 15
declines, hand-computed against the record form, green on both backends and under
`LOFT_POISON`, `LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES` and the leak gate; structural pins in
`tests/view_field.rs` (the tuple signature, the dropped buffer, the site's tuple read, and every
decline still declining); registered under the `codegen` subject.  The native corpus runs clean
with the unit armed (1364 scripts, 0 compile failures), and the curated local set is green for
the default path: four of its reds were derived rows this unit owed (the three walker audits and
the emitter registry's cap, re-measured rather than picked) and four were the box under load —
`loft_suite` 133 s, `poison_claim` 195 s, `html_asyncify` 41 s and the two server tests each pass
alone.

### Step 2 — the gate reaches the consumer, and the row does not move (2026-09-16)

Step 1's two decline classes are closed, and each was a different kind of blindness:

* **Per BODY became per PATH.**  A parser appends and returns once per branch, so a body-wide
  mention count declined every one of them.  The question is asked per exit now: each exit's
  own element must be built in the SAME statement list, before it, with no other exit's element
  built in between, and no build may stand under a loop.
* **A naming is not a disturbance.**  The mention rule could not tell `sc.unparsed += […]` from
  `sc.ops += […]`, and B2 puts an `OpPlaceRecord(sc, …)` in every function that places a call's
  result.  `namings_avoid_place` asks what each naming REACHES: a claim in the store moves
  nothing, an append names its field by NUMBER (converted through the schema, as
  `grown_containers` does) and its `OpFinishRecord` half the same way one argument further
  along, a projection carries the offset, a fixed-width scalar read or write reaches its own
  field (`IN_PLACE_SET_OPS` and `SCALAR_GETTERS`, the two lists that already carry "moves
  nothing"), a READ of the watched field itself is a read (`len(sc.ops)` between a bind and its
  own read moves nothing — `read_context` is the one home the site gate and this walk share),
  and an argument of a user call is licensed by the call's own disturbance summary.  Everything
  else still declines.

Two spellings cost a cycle each, and both are the same lesson as step 1's: `OpNewRecord` names
its field by NUMBER where a place carries a byte OFFSET — read as an offset, a REMOVAL from the
very container the leaf views compared as a different place, and the `b8` cell measured the
silence (`5,30` where the copy holds `2,3`) — and `OpFinishRecord`, the append's other half,
names the container the same way with the field one argument further along.

*Measured on the library:* `parse_poly` and `parse_lock` are now admitted; `parse_circle`,
`parse_fronds` and `parse_line_cmd` still decline, all three on a source whose copy lands in a
BRANCH — `sc.ops += [Op { … pts: pc_pts … }]` in each arm of an `if`, with the exit outside it,
so the leaf's place is the element one of two appends made and no single expression names it.

**And the parse row does not move: 38.4–40.0 k ns/op with the unit armed against 38.4–42.4
without it, hash `33f6d2b8` both ways — a wash.**  That is the honest answer to the phase's own
premise.  The evaluation table charged this row a `Mark` store and a points copy per line; the
store is ~60 ns and there are twelve per parse, so the whole class is ~1.7 % of a 40 µs row
even when every function is admitted.  What C5 removes is real and what it was expected to be
worth was not: the parse row's remaining cost is not the `Mark` record.

*Measured on the consumer, and it does not reach it yet.*  With the unit armed, every `Mark`-
returning function of the drawing library still declines, and the trace says exactly why —
which is the point of measuring rather than assuming:

| function | the gate that declined it | what step 2 owes |
|---|---|---|
| `parse_poly`, `parse_lock` | *a statement names the leaf's root* — the body appends to `sc.ops` on THREE paths, and the mention count is per BODY | the count has to be per PATH: one append per path is one growth, which is what the leaf lives with |
| `parse_circle`, `parse_fronds` | *no resolvable source* — the points reach the appended element through a chain `leaf_root` does not follow | the resolution has to reach the shapes a parser actually writes, measured one at a time |
| `parse_line_cmd` | *the tail is not a value leaf* | § V-aa's own gate, unrelated to the view leaf |

So C5 ships as the RULE's machinery with its conditions measured, and the row it was cut for
waits on step 2.  It is opt-in for exactly that reason: armed it changes nothing in the
consumer, and a unit that pays nothing must not also carry risk by default.

## The rewrite list — the natural `parse_poly` to its optimal form

**This is not about how a programmer writes loft.**  The programmer writes the natural
`parse_poly` as it stands in the library; the version below is the SPECIFICATION of what
the compiler must reach from it, written by reading the two side by side and asking, for
every difference, what fact on the IR licenses the rewrite and which rule says so.  The
rules were written first (2026-09-15, `formal/rewrites.md` § *A result is built where it
will live…*, `formal/ownership.md` `(O-ViewField)`, `formal/binding.md` `(B-View)`), so a
question met while building a phase is answered there and not decided in the code.

*The target, per `Poly` line:* one `Op` record created in the scene's store, its two
vectors grown in place, nothing else minted, nothing copied, and the `Mark` a value
tuple whose `pts` is a view of the op's own vector.  In hand-written form:

```loft
fn parse_poly(sc: Sketch, s: text, raw: PointList) -> Mark {     // raw: the caller's scratch
  read_points_into(raw, s);
  pk = read_paint_kind(s);
  if len(raw.pts) < (if pk == Stroked { 2 } else { 3 }) { return Mark { matched: true, bad: true } }
  sc.ops += [Op { kind: Stroke }];          // the one record, built where it lives (R-Mint)
  o = sc.ops[len(sc.ops) - 1];              // a view (B-View)
  read_paint_into(o.paint, s);              // the field is the buffer (R-Place)
  if pk != Stroked { o.kind = Fill; smooth_into(o.pts, raw.pts, raw.smooth, true); }
  else {
    o.w = read_width(s, 3); o.color = read_stroke_colour(s);
    smooth_into(o.pts, raw.pts, raw.smooth, false);
    if raw.any_width { /* widths appended into o.widths; smoothed in place */ }
  }
  Mark { matched: true, bad: false, pts: o.pts }   // pts a VIEW LEAF (O-ViewField), Mark in registers
}
```

What the compiler cannot do is the one thing a programmer could: change the contract
(`Mark` carrying an op index instead of the points, or `parse_poly` growing the element's
box itself).  `(O-ViewField)` reaches the same cost with the contract as written, which
is the whole point of Goal F.

| rewrite (per `Poly` line) | IR fact to establish | licence | exists today | phase |
|---|---|---|---|---|
| bind `pp_raw`, `pp_paint` without a copy | the callee's return deps name exactly its own buffer; the destination a plain local | `O-Move`, `O-Buffer` | shipped | B1 |
| `pp_paint`'s buffer claimed in the scene's store; `paint: pp_paint` a relocation, not a deep copy | the result's ONE owning destination is a field of a record in store S on every path that keeps it; `pp_paint` owned and dead on every path after the literal | `R-Place`, `R-MoveLast`, `O-Complete` | the liveness exists as the `avoidable-copy` lint's *"still used after this point"*; codegen does not read it; a buffer claimed in another store is new | B2 |
| `pts: smooth_pts(…)` fills `Op.pts` directly, no buffer | as above, with the destination place existing at the call and no argument reaching it; the callee writes only its buffer and answers it at every exit | `R-Place` ("the buffer IS the place"), `R-Callee`, E15, E16 | `retbuf_only_writer`; `R-ElemFirst` already builds a vector inside an appended element; C2 shipped the EXISTING-place road, which this is not — this destination is a literal field reached through a local, so it is a vector `place_result` and it declines on `pp_pts`'s second destination | C5, then a vector `place_result` |
| `sc.elems[idx] = Elem{…}` written into the slot | the slot exists; every field expression evaluated before the first write (`ename: ap_e.ename` reads the slot); omitted fields defaulted | `R-InPlaceLiteral`, E13, E14 | the FIELD destination already writes in place and defaults the omitted fields; E13's staging shipped as C1 step 1; the ELEMENT receiver is what is left | C1 step 2 |
| `ap_e = sc.elems[idx]?` as a view | `ap_e` only read; `sc.elems` not disturbed between bind and last read, in this frame or any callee | `B-View` (the discharge clause), `B-Disturb` | the VIEW already; the disturbance walk was this frame only, which is what C3 closed | shipped C3 |
| `Op { kind: Stroke, … }` prefilled by one block write | the literal's field set against the type's defaults | `R-Prefill` | complete-write has the set; the per-type image is missing | C4 |
| the four `smooth_pts` buffers not minted at entry | one path runs one call | trivial | goes away with C2 | with C2 |
| the `PointList` scratch reused across lines | the callee's literal rewrites every field when handed a live buffer; the buffer-holder's rebind never frees it | `R-Reuse`, § V-y | D-own-43 (the interpreter's rebind free) must close first | B1b, A2 |
| the points written once: `Mark.pts` a view of `Op.pts` | the field's source has an owning destination in `sc` (outlives the frame); every call site only reads it before any disturbance of `sc.ops` | `O-ViewField`, `R-ValueRecord`'s view leaf | no — the rule is new; § V-aa's site fixpoint is the home to extend | C5 |

*What each phase must build,* in the @PLN157 shape: a `LOFT_NO_<unit>` switch, cells with
hand-computed values on both backends under `LOFT_STRICT_STORES`, `LOFT_POISON`,
`LOFT_POISON_CLAIM` and the leak gate, a guard with its `@falsified-at:` receipt, pins, the
subject registered.  Each rule's decline list is its falsifier list: a cell per decline,
proving the copy still runs there.

*Analyses the phases share* (each wants ONE home, read by the scope pass and both
backends, the loft#810 discipline B1 followed): the def-use classes of a heap value (its
owning destinations against its read-only uses — `R-ElemFirst`'s "consumed exactly once"
generalised); per-path liveness at a set; the disturbance walk between two points for a
named container; destination existence and aliasing against a call's arguments; the
callee writer summary; the return shape over all call sites.

## Composition matrix — Stage A (REQUIRED)

The axes the tiers touch, each with the domain the language offers; every phase's cells
are drawn from these, hand-computed, run on both backends under `LOFT_STRICT_STORES=1`,
`LOFT_POISON=1`, `LOFT_POISON_CLAIM=1` and `LOFT_NATIVE_LEAK_CHECK=1`:

| axis | domain |
|---|---|
| callee kind | user fn · fn-ref (`O-Opaque`: empty deps) · `#rust` native · generic instance · twin (`__inv`) |
| result shape | all-scalar (a § V-aa value record) · record with vectors · record with text · nullable record · struct-enum · tuple |
| binding | first bind · rebind in a loop · a value-branch join (`O-Complete`) · a `?? default` discharge · `return f(…)` (§ V-an's phantom) |
| activation | plain · recursive · generator (`yield` inside) · `par` arm · a closure body |
| exit | fall-through · early `return` · `break`/`continue` · a fault (`LOFT_DEV_SOFT_HALT`) |
| destination | fresh local · existing local · record field · vector element (append / overwrite) · the function's own result · two destinations |
| aliasing | none · the source IS the destination's old value (`ename: ap_e.ename`) · a `&` view live across the write (`B-Disturb`) |

`python3 scripts/matrix_axes.py file <cells>` measures which values a cell corpus reaches;
`cross activation destination` is the pair the corpus must cross, since that is where the
arena and the placement meet.

## How a verdict is reached — the five questions, in order

The owner reviews the PROCEDURE, not twenty tastes (2026-09-15).  Each row is put to
these questions in order, and the first one that decides, decides:

1. **Spelling, or mechanism property?**  A mechanism property (an identity, a mark
   discipline, a gate's accounting) is not natural or contrived — it is an invariant the
   tier must hold for whatever it admits, or the tier is not built.  Its verdict is a
   falsifier and a named failure mode, nothing else.
2. **Is the spelling natural?  MEASURED, not judged.**  Two facts: does the consumer
   corpus write it (a grep over the 15 library files of `loft-libs-graphics`, and the
   games and the crawler when the row matters), and would a programmer write it without
   knowing the store model?  Absent from the corpus and explainable only by the store
   model → contrived → keep today's copy, defer the row.  Present → natural → continue.
   The corpus is the oracle for "natural" the way the interpreter is the oracle for a
   value: a spelling absent today may appear, and then the row is re-measured, never
   re-argued.  Measured 2026-09-15: a record `?`-discharge bound to a local (E17) 45
   times; a field assigned from a local (E9) 382; a bind-then-return (E20) 96; an
   element overwritten from a literal (E13) 3; a record fn with two or more literal
   returns (E15) 6; `x = f(x, …)` on a RECORD or vector (E16) 0 — its 6 hits are scalar
   `min`; a `yield` statement (E3) 0; a `par` block (E4) 0; an `OpDrop` type (E12) 0.
3. **Does a written rule already answer it?**  `formal/*.md` first (the debugging
   policy's *read the formal spec first*).  A rule that answers settles the row and the
   mechanism implements the rule exactly, no narrower, no wider.  A question no rule can
   express means the rule wants extending — the OWNER's design call, recorded under
   § Open design questions; the mechanism never decides it silently.
4. **What falsifies the verdict?**  A cell with a hand-computed value on both backends
   under `LOFT_STRICT_STORES`, `LOFT_POISON`, `LOFT_POISON_CLAIM` and the leak gate, and
   a soft-halt note count where a fault path exists.  A verdict with no falsifier is not
   a verdict; it is the plan-51 shape.
5. **What does being wrong cost?**  Admitting wrongly is silent-wrong, a leak or a double
   free — the freeze-axis class.  Declining wrongly is a missed optimisation the compiler
   pays itself (Goal F).  The asymmetry decides every doubt: DECLINE, and the decline is
   always available.

*Where each row was decided.*  At 1: E1, E2, E5, E6, E18, E19.  At 2 (contrived, deferred):
E3, E4, E12, E16.  At 3 by an existing rule: E7 (`O-Buffer`'s steady state), E8
(`O-Opaque`: empty deps cannot license an adopt), E9 (`O-Complete`: per path), E10
(`O-Borrow`: not owned, no move), E11 (`R-MoveAppend`'s zeroing), E13 (the language's
evaluation order — a literal's fields are evaluated before the assignment stores, so an
in-place build STAGES them), E14 (loft#914: an omitted field takes its default), E17
(`B-View` under `B-Disturb`), E20 (§ V-an's phantom, `O-Buffer`).  At 3 with NO rule —
the owner's call: E1's identity (`O-Buffer` names STORE identity; nothing names a record's
— open question 1).  At 5 (doubt → decline): E15 (a callee with more than one exit writes
its destination only at the single exit or declines), E2's handed-up record (copy when the
mark cannot be re-drawn).

## Edge cases to inspect before a phase is cut

Numbered for the review.  **Verdict** is the proposal; the owner confirms or moves it.
Every row is first judged on NATURALNESS: `natural` means a programmer writes it without
knowing the store model and the mechanism owes it the efficient code; `contrived` means
the spelling keeps today's copy and the row is deferred — a verdict that is always
available and never a defect.  The parse library is the reference for "natural": every
shape it uses (E7, E13, E15, E16, E17, E20) is natural by construction.

| # | naturalness |
|---|---|
| E1–E6, E18, E19 | mechanism-internal — not a spelling; they must hold whatever is admitted |
| E7, E13, E15, E16, E17, E20 | natural — the library writes them today |
| E8, E9, E10, E11, E14 | natural — a branch, a parameter, a partial literal are ordinary |
| E3 (generator), E4 (`par`), E12 (`OpDrop` types) | contrived for this plan — decline the mechanism there, keep the copy, defer |

| # | case | why it bites | proposed verdict | probe |
|---|---|---|---|---|
| E1 | **store identity is the free protocol's key.**  `OpFreeRefIfDistinct(v, __ref_N)` declines the local's free when `v.store_nr == __ref_N.store_nr` (`O-Buffer`), `OpDistinctStore` answers `store_nr !=`, and § V-af's witnesses are store identities | two buffers in one arena share a `store_nr`, so "same store" no longer means "same object": a free declines that should fire (leak) or a witness matches the wrong buffer | identity becomes `(store_nr, rec)`; `free_displaced` and the witness compare both; `emission_audit.py` R-State counts holders per record | count the sites (`grep -c store_nr !=` on an emission), then a hand-patched arena of `parse_poly`'s four buffers under `LOFT_STRICT_STORES` |
| E2 | recursion | a caller-threaded arena grows with depth; a mark/release at each activation exit keeps it a stack, but a result adopted from the arena by the CALLER's local (tier 2) sits above the callee's mark | release-to-mark only what the activation minted and did not hand up; a handed-up record is re-marked to the caller | a recursive record builder, depth 1 000, leak gate |
| E3 | a generator | the activation SUSPENDS; a local arena dies at the yield | the arena is a persistent field of the generator, or tier 1 declines a body with `yield` (as every hoist does) | the coroutine corpus under the switch |
| E4 | `par` arms | one arena, two threads | one arena per arm, marked and released by the arm | THREADING.md's corpus |
| E5 | records that own heap in an arena (`Elem.ename`) | release-to-mark is O(1) only for no-heap records; text handles must be released (`@FR-H-ClearRelease`) | the mark carries a heap-owner list; a walk over that list, never over the store | a cell with 1 000 text-owning temporaries, `LOFT_NATIVE_LEAK_CHECK` |
| E6 | an arena record's vector GROWS | growth relocates within the store; `@FR-R-Base` bases and `@FR-R-Header` headers into arena records | an arena mint counts as growth for the enclosing loop's `growth_free`, as a null-discharge buffer does today | `LOFT_HOIST_VERIFY=1` on the parse bench |
| E7 | adopt at first bind when the buffer is the CALLER's caller's (`ps_p = parse_poly(…, __ref_5)`) | the local and the buffer are one store; the steady state after iteration one already | MEASURED (B1): admitted for a fresh-adopting callee already; for a promoted-local callee the reused buffer is freed by the interpreter's rebind of that local (c6) — B1 keeps the buffer null, B1b takes the reuse | `B1-adopt-first-bind-cells.loft` c6; the plan-51 guards under the switch |
| E8 | adopt when the callee handed back a store it did not mint (`O-Opaque`: a fn-ref, a return that borrows a parameter) | adopting a borrowed store frees someone else's | SHIPPED (B1): `adopts_minted_at_bind` admits only a return whose deps name exactly the callee's buffer attribute; a visible-parameter dep, a `__closure` dep, a `CallRef` and a nullable return decline | c7 (a parameter read), c12 (a branch), c4 (nullable) |
| E9 | move at last use on ONE path only | `if c { s.p = pp } else { use(pp) }` — dead after on the then-path, live on the else | per-path liveness (`O-Complete`); the move only where dead on EVERY path after, else copy | a branch cell with the value read after on one arm |
| E10 | move from a `const` parameter, a view, an element | not owned; a move would steal | decline: the oracle's OWN verdict is the gate, `O-Proxy` is not enough here | cells over each source kind |
| E11 | the moved-from local at scope exit | its free must not run (the record is gone) | the move nulls the source (`@FR-R-MoveAppend` zeroes it); `LOFT_POISON` is the falsifier | poison run |
| E12 | a type with `OpDrop` (@PLN163 copy leases) | a move must not run the drop; a copy must take a lease | a moved record keeps its lease; interacts with @PLN163's rule — decide there | @PLN163's cells |
| E13 | **self-referential overwrite**: `sc.elems[idx] = Elem { ename: ap_e.ename, … }` where `ap_e` views the SAME slot | building in place overwrites a field the literal still reads | evaluate every field expression before the first write (a stage), or decline in-place when any field reads through the destination | the `acc_pts` shape as a cell, hand-computed |
| E14 | a partial literal overwrite | omitted fields must become defaults, not keep the old element's values | write the defaults (the prefill image) before the named fields | a cell with an omitted field that was non-default before |
| E15 | the destination as return buffer when the callee exits EARLY | `if len(pts) < 2 { return Mark{…} }` leaves the destination half-written | a callee that returns on more than one path writes the destination only at its single exit, or declines | `parse_poly`'s three returns |
| E16 | the destination as return buffer when the callee READS the destination (`v = f(v)`) | source and destination alias | decline when any argument reaches the destination's store | a cell `s.pts = grow(s.pts)` |
| E17 | a `?`-discharged read used as a view while the container is DISTURBED before the last read | `ap_e = sc.elems[idx]?` then `sc.elems += […]` then `ap_e.bx0` | `B-Disturb` — a view is admitted only when no disturbance stands between the bind and the last read | a cell with an append between |
| E18 | the interpreter | tier 1 and 3 change the IR (where buffers live, where a literal builds), so both backends must agree — and the interpreter's `OpDatabase` per buffer is the same IR | parse-time change, one IR; the emitter's hoists read the arena as a store | the whole `tests/scripts` corpus on both backends |
| E19 | `LOFT_STRICT_STORES` / the leak gate's accounting | a store that hosts many buffers is one store to the gate; a leaked RECORD inside it is invisible to a store-count gate | the gate counts records above the mark at exit, not stores | extend the gate first (a probe-first phase) |
| E20 | a buffer adopted by a local that is then RETURNED (`n = f(); …; n`) | § V-an's phantom: the result lands in the function's OWN buffer, which is the caller's arena, not this activation's | the phantom is above the mark by construction; pin it | `V-an-chain-cells` under the switch |

## Sub-arcs (REQUIRED)

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — probe first: count the store-identity sites; price tier 1; the hand patch judged not worth building by the arithmetic; the leak-gate extension (E19) deferred to A1's shape | § P0 | 287 identity sites in one emission; ~88 mints per parse at ≈ 60 ns a pair: tier 1's ceiling ≈ 10 % of the row | Done 2026-09-15 |
| **P0b** — attribute the runtime's share of the release row to loft lines: a frame-pointer rlib build, `perf` with call chains, each family charged to a line | § P0b | `scripts/native_attrib.py`; runtime 72 %, store mint/free 20 % (half at function entry), `:704` 12.5 %; A0's lever measured by hand −9–10 % | Done 2026-09-17 |
| **A0** — the buffer minted at its first use on the path that runs, not at function entry | § A0 | 14 cells both backends, both switch states, under the falsifiers; switch-off byte-identical; the parse row −7 % after row 11 | Built 2026-09-17, default ON (`LOFT_NO_LAZY_BUFFER`) |
| **C6** — a nested record literal built inside the element it is a field of (`Op { paint: Paint { … } }`) | § C6 | 7 cells both backends, both switch states, under the falsifiers; switch-off byte-identical; the parse row −6–7 %; loft#1548 found and fixed on the way | Built 2026-09-17, default ON (`LOFT_NO_NESTED_IN_PLACE`) |
| **Runtime pass** — what the release profile named once item 4 proved not to be a move: the write check inlined, the no-heap claims walk skipped, a fresh store initialised once | § The runtime under the row | `perf stat` per change; the fourteen-row same-moment A/B (pre-session rlib vs this one, one emission): every row equal or faster, 14/14 hashes; an accessor reorder measured and dropped (pixel rows +15–38 %) | Shipped 2026-09-17 — the parse row 28.3–28.5 k → 25.0–25.3 k ns/op, 3.56× its reference |
| **loft#1549** — a reused record buffer releases what it held (`(H-ClearRelease)`'s record clause, D-heap-12) | § The runtime under the row | 12 cells both backends under every falsifier and `LOFT_NO_LAZY_BUFFER`; resident memory flat over 1 000 000 refills; guard falsified against 04cc50b1 with a patch receipt; pins `tests/pooled_buffer_release.rs` | Fixed 2026-09-17 (`fixed-pending-merge`) |
| **A1** — arena for one activation's own buffers (`__ref_N`, `__ref_p2_N`, literal temps), mark/release at every exit | § Tier 1 | `tests/scripts/164-arena-activation.loft` both backends; plan 51's ten graduated guards under the switch; `emission_audit.py` R-State per record | Blocked on P0b — the store-pair family is 9 % of the release row; whether the free-tree family (12 %) charges to activation temporaries is what decides the shape |
| **A2** — the caller-threaded arena, reset per loop iteration | § Tier 1 | parse row −20 %; E2/E3/E4 cells | Blocked on A1 |
| **B1** — adopt at first bind | § B1 | cells c1–c17 both backends; the store census 139 → 108; plan-51 guards under both switch states | Shipped 2026-09-15 |
| **B1b** — reuse the buffer across activations for a promoted-local callee (E7's steady state), and B1 behind an `if` pre-init | § B1b | 23 cells both backends, both switch states, under every falsifier; the entry witness closes D-own-43 and the two other frees that held its belief; the body behind a hoisted result reached; `wrap` + `native` green; parse bench `Mark` mints 24 → 14 per two parses | Built 2026-09-17, default ON (`LOFT_NO_ADOPT_BUFFER_REUSE`) |
| **B2** — the result's buffer claimed in its destination's store, the field taking it by relocation at the last use (`R-Place`, `R-MoveLast`) | § B2 | cells b1–b15 both backends under the falsifiers; the census 184 → 50; `parse_poly`'s `paint: pp_paint` emits `OpMoveRecord` and `read_paint`'s buffer is a record in the scene's store; the parse row a wash (`perf stat`) | Shipped 2026-09-16 |
| **C1** — element overwrite from a literal in place (`R-InPlaceLiteral`) | § C1 | step 1 (the STAGING clause, a both-backend silent-wrong on the shipped FIELD road) and step 2 (the element receiver): 25 cells both backends under every falsifier, four declines pinned, the `acc_pts` census 3 → 2 | Shipped 2026-09-16 |
| **C2** — the destination as return buffer (`R-Place`'s "the buffer IS the place") | § C2 | the 11-cell oracle (today's answers, which C2 may not move); the aliasing decline; `(O-Buffer)`'s new clause | Shipped 2026-09-16 — a pure-IR rewrite, the buffer variable re-pointed at the destination; ZERO admitted sites in the parse bench (the emission is byte-identical under the switch) and none in the 15-file consumer corpus, so the gain is structural |
| **C3** — read-only `?`-discharge as a view (`B-View`'s discharge clause) | § C3 | the discharge ALREADY views (13 shapes measured, both backends), so the phase's content was its other half: `(B-Disturb)` across a CALL.  15 pairs both backends; 7 move under the switch | Shipped 2026-09-16 |
| **C4** — per-type prefill image (`R-Prefill`) | § C4 | cells c1–c11 both backends under `LOFT_PREFILL_VERIFY`; the verify census over all 1432 corpus files; the image USED on both backends (`LOFT_TRACE_PREFILL`); parse row −11 % | Shipped 2026-09-15 |
| **C5** — a returned record's heap field as a view leaf (`O-ViewField`, `R-ValueRecord`, `R-Escape`) | § C5 | 2 admissions, 2 positive controls and 15 declines both backends under every falsifier; the native corpus clean with the unit armed; three conditions measured rather than argued (an element read at a site, the disturbance span as an UPPER bound, the exit's empty answer as a proof) | **Step 1 built 2026-09-16, opt-in** (`LOFT_VIEW_FIELD=1`).  The `?`-discharged source is NOT admissible — its ownership is a join — so the admitted shape is the natural one; step 2 owes per-PATH mention counting and a wider source resolution, and the library's own declines are measured per function in § C5 |

Every phase: a switch (`LOFT_NO_<unit>=1`), a falsifier, cells in
`bytecode-comparisons/`, a guard in `tests/scripts/` with its `@falsified-at:` receipt,
the pins in `tests/<unit>.rs`, `scripts/test_subjects.sh` extended — the @PLN157 shape.

## Phase ordering

1. P0 — done: tier 1 priced at ≈ 10 % of the row, E1's site count measured; the owner's
   pick between `(store_nr, rec)` identity and a pooled store stays open until A1 is cut.
2. B1, C4 and B2 — shipped.  B2 measured a wash on the parse row (the `Paint` it relocates
   carries no heap on the bench scene); the copy class the profile measured is the points,
   which C2 and C5 take.
3. ~~**C3 then C1**~~ — both shipped.  C3 moved no copy (the discharge was already a view, and
   the `acc_pts` temporary the evaluation table charged to it was C1's, counted twice); C1
   closed the staging clause on the FIELD road first, because the element road inherits it, and
   then opened the element road — census 3 → 2.
4. ~~**C2**~~ — shipped, and the bench then said the spelling it admits (`h.v = mkints(n)`,
   a call result into an EXISTING vector place) has no site in the consumer: the emission is
   byte-identical under the switch and the corpus's 13 `place = call(…)` sites are all scalar
   or `text`.  The gain is structural; § C2 *Shipped* records it and names where the parse
   row's points class actually sits.
5. ~~**C5 is next**~~ — steps 1 and 2 built opt-in; the row measured a wash and the `Mark`
   class under 2 %, so step 3 and the vector `place_result` are PARKED (§ C5 *Step 2*).
6. ~~**P0b**~~ — done (§ P0b): the store family is 20 % of the release row, half of it minted
   at function entry.
7. **@PLN157 row 11 (the transitive-leaf prelude), then A0, then C6** — § P0b *The queue after
   P0b*.  A1/A2 are re-priced after A0 against the body half of the store family.  **B1b**
   beside them whenever D-own-43 closes (the interpreter's rebind guard).
8. Re-measure the 14-row bench after each phase (`compare.py`, 14/14 hashes), and the parse
   row's profile with `perf` on the release binary — never with a count.

## Open design questions

1. **E1 — what is an object's identity once buffers share a store?**  `(store_nr, rec)`
   is the proposal; it touches `free_displaced`, `OpDistinctStore`, the § V-af witness
   and the audit.  Or: keep one store per buffer and make the STORE cheap (a pooled
   `Store` with no file handle, no budget row) — a smaller change that keeps identity as
   it is and takes less of the fifth.  The owner picks; P0 measures both.
2. **Is the arena a parse-time IR fact (both backends) or an emission fact?**  The
   buffers are IR (`OpDatabase(__ref_N)`), so tier 1 is IR-level unless the interpreter
   keeps stores and only native pools them — which `O-NoDiverge` allows for a lifecycle
   detail with no value, but the leak gate must then read both.
3. **Where does a moved-from local's `deps` go?**  Tier 2's move needs the scopes pass to
   record "handed off", the same predicate the double-move lint counts.
4. **`(O-ViewField)` — DECIDED (owner, 2026-09-15, C122): admitted.**  *"The current
   contract is about semantics, not about optimisations; we can do anything for that as
   long as we can validate the conditions where it is correct.  The biggest problem here
   is a library API where we cannot know how it will be used.  But if a construction
   doesn't escape a library then we can rewrite whatever we want."*  Recorded as
   `(R-Escape)` in `formal/rewrites.md`, which every rule in the rewrite list now reads:
   a rule needs its validated conditions and nothing else; a construction that escapes a
   library's API keeps the promised representation and the boundary materialises; one
   that does not may be rewritten freely.  C5's cells therefore include an ESCAPE row —
   a `pub fn` whose result a consumer could keep — which must read the copy at the bridge.
   And the unit is a BUILD decision (owner, same day): a release copy of a program already
   compiles its `use`d loft libraries into the one program (the bench emission carries
   `parse_poly` itself), so in that lane the drawing library's API is no boundary and C5
   crosses it; only the library's own published cdylib, a `#rust` native, the live-reload
   arm, a stored layout and a placed library keep the promised representation.
5. **`(R-Place)` across stores.**  A result's buffer claimed in the DESTINATION's store
   (the scene's) rather than a store of its own is what makes B2's move a relocation;
   it is also the first place a temporary lives inside another record's store, which is
   E1's identity question in miniature.  If the owner picks the pooled-store answer for
   E1, this rule still stands: the placement is per call, decided by the destination.

## Cross-arc dependencies

- **@PLN157** — the parse row's bar is that plan's; this plan is its parse unit, split
  out because it is a memory-model change, not a rewrite of a loop.
- **plan 51 (hidden-buffer aliasing, finished)** — its five clusters are the shapes an
  arena must not reopen; its ten graduated guards run under every switch here.
- **@PLN163 (copy leases)** — E12: a moved `OpDrop` record and its lease.
- **loft#1336 (owner witness)** — a witness is a store identity; E1 changes it.

## See also

- `LIFETIME.md` (`OpBindOrCopy`, the adopt-or-copy delivery), `OWNERSHIP_MODEL.md`,
  `formal/ownership.md` (`O-Buffer`, `O-Move`, `O-Complete`), `formal/binding.md`
  (`B-Copy`, `B-View`, `B-Disturb`), `formal/rewrites.md` (`R-Callee`, `R-ElemFirst`,
  `R-MoveAppend`, `R-Mint`, `R-PushRec`), `formal/heap.md` (`H-ClearRelease`).
- @PLN157 `DESIGN.md` § V-an (the parse table this plan starts from) and § V-ao (the
  census).
- The tracker issue: [@PLN164](https://github.com/loft-lang/plans/issues/164).
