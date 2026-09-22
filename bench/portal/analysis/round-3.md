# What to optimise next — after the push window, the join read, the bounded sum and the type clause

Taken 2026-09-22 on x86-64 from the portal re-measured at `8009e8c5b` (every routine
**1.85×**, shipped routines **1.63×**, 30 rows at or over 2×, 17 over 3×).  An ANALYSIS: every
figure below is a HAND-PRICE — the emitted Rust of the routine edited to the form a rewrite
would emit, compiled with the `--native-release` flags, run on one core with the result hash
unchanged — and nothing here was built when it was taken; § Built records what has been
since, with the measurement.  Two rows were priced and DID NOT MOVE; they are listed as
what they are, with what was learned.

| # | lever | rows it moves | hand-priced |
|---|---|---|---|
| **W1** | `for c in text` walks bytes with an ASCII fast path; the body's compares against a literal are plain | `char_walk`, every character walk | **−65 %** (4.58× → ~1.6×).  **BUILT** 2026-09-22: −47 %, 2.48× (§ Built — the last step was the checked accumulator adds) |
| **W2** | `for p in vector<text>` borrows its element AND iterates through a held header | `join`, `word_count`'s walk, every walk of texts | **−75 %** (5.40× → ~1.4×) — T1's borrow alone is −45 %.  **BUILT** 2026-09-22: −64 %, 1.96× (§ Built) |
| **C1+** | no fn-ref buffer guard where no registrant is reachable, and the depth count replaced by a stack-pointer check | `fibonacci`, every recursive function | **−39 %** (4.09× → ~2.4×) |
| **P1** | an in-place scalar set through ANY store-free element address does not decline a loop's headers | `chunk_lookup`, every `find-then-write` loop over records | **−30 %** (2.87× → ~2.0×) |

## The population — 30 rows at or over 2×, by mechanism

| mechanism | rows | what decides it |
|---|---:|---|
| keyed | 10 (2.99–5.00×) | store-format and data-structure work, priced in `keyed.md` at 7–17 % apiece — no single lever reaches 2× |
| text | `split` 12.3×, `join` 5.4×, `char_walk` 4.6×, `string_build` 2.6× | W1, W2 here; `split` is a representation question (text slices as views) |
| records | `mesh_aabb` 4.8×, `enum_match` 4.2×, `chunk_lookup` 2.9×, `mesh_emit` 2.8×, `entity_tick` 2.5×, `smooth` 3.1× | P1 here; two rows UNATTRIBUTED (below) |
| the call frame | `fibonacci` 4.1× | C1+ |
| `par` | 5.3× | another subsystem, one row, not looked at |
| the rest | `copy` 2.65×, `parse` 2.6×, `sort` 2.5×, `grid` 2.5×, `catalog_churn` 2.4×, `newton_sqrt` 2.2×, `sum` 2.2× (done, 2× accepted), `collatz` 2.0× | each its own mechanism, none priced this round |

## Built

### W2 — `(R-TextBorrow)`, 2026-09-22

Built as the rule `(R-TextBorrow)` (`formal/rewrites.md`), switch `LOFT_NO_TEXT_BORROW`,
falsifier `LOFT_HOIST_VERIFY=1` (the element re-read at the walk's release), cells
`tests/scripts/158-text-borrow.loft` t1–t18, pins `tests/text_borrow.rs`.  Measured with
`bench/stats.py --only 13` on this box, before (portal at `8009e8c5b`) → after:

| row | before | after | ratio |
|---|---:|---:|---|
| `join` | 30.97 µs | **11.21 µs** (−64 %) | 5.40× → **1.96×** |
| `split_walk` | 4.44 µs | **2.51 µs** (−43 %) | 1.80× → **1.01×** |
| `parse_num` | 61.4 µs | **41.2 µs** (−33 %) | 1.46× → **0.98×** |
| `trim_lower` | 9.82 µs | 7.99 µs (−19 %) | 1.66× → 1.36× |
| `find_contains` | 15.8 µs | 13.4 µs (−15 %) | 1.60× → 1.35× |

Three pieces, in the order the ledger asked for:

1. **T1's refactor first** (`8fc2ed578`): `Output::text_borrowed` is the one home of
   *"this text variable is a `&str`"*, asked by the six sites that spelled it inline;
   proven by `scripts/introspect_diff.sh` IDENTICAL 1629/1629.  The borrowed loop variable
   then needed no site of its own — the bind is the only new emission.
2. **The borrow**: `let var_p: &str = { …get_str(…) }` where the body reads `p` only as a
   text value and writes no store.  `hoist::text_escapes` is the escape walk, and it is
   deliberately conservative — a shape it does not read declines rather than compiles.
3. **The walk through a held header** came for free from the rules that already hold
   headers, once two facts were stated: `OpGetText` is a READER, and a text-VALUE op (scalar
   and text operands, scalar/text/void result; or a write to a text VARIABLE) touches no
   store — `native_op_is_store_free`'s text-value clause.  That clause is what took the
   lazy split's piece too (`split_walk`, `parse_num`, `trim_lower`, `find_contains`): its
   piece borrows the iterator's source and needs no store condition at all.  The ledger's
   *"`iteration_head` taught the text element's head"* was NOT needed: the header hoist
   reads pure paths from the body, not the iteration head.

What it found on the way: (a) the lazy split's hidden vector had been kept out of the
header hoist only by accident — `OpGetText` reading as a writer — and once that fell the
emitter derived a header for a vector the lazy form never declares (rustc E0425 on the
whole text lane); the exclusion is now stated at the candidate loop.  (b) The value
channel cannot see a SHORT overwrite of the borrowed element: `words[p#index] = "zz"`
writes a new position and the old bytes survive the iteration, so the sabotaged build
answered every cell right — only the checking form (address compared first) saw it.  The
cell grows the store with a 64 KiB overwrite, which makes the sabotaged build exit 1 with
no output, the crash the rule prevents.  (c) `hand-price`'s 7.8 µs for join was not
reached (11.2): each element still resolves `stores.store(&db)` for its `get_str` — a text
element read through the held BASE (the position word one load, the text one slice) is
the remaining step, unpriced.

### W1 — `(R-CharWalk)`, 2026-09-22

Built as the rule `(R-CharWalk)` (`formal/rewrites.md`), switch `LOFT_NO_CHAR_WALK`,
falsifier `LOFT_HOIST_VERIFY=1` (the written step run beside the fast arm), cells
`tests/scripts/158-char-walk.loft` w1–w11, pins `tests/char_walk.rs`.  Measured with
`bench/stats.py --only 13`:

| row | before | after | ratio |
|---|---:|---:|---|
| `char_walk` | 23.4 µs | **12.5 µs** (−47 %) | 4.62× → **2.48×** |
| `split` (the stdlib walks its text with `for c in self`) | 34.0 µs | **21.1 µs** (−38 %) | 12.5× → 7.8× |

Re-priced by hand before building, one piece at a time, on the lane's own emission — which
is how the ledger's 8.2 µs came apart: the iterator's ASCII fast arm alone 23.5 → 17.7
(−25 %); the body's compares plain on top → 17.2 (−3 %, LLVM folds them, as § W1 below
says); the null test — `src != "\0"`, a CONTENT compare LLVM does not hoist past the step's
calls — asked once before the loop → **12.9 (−25 %)**; and the two checked accumulator adds
of the body plain → 8.3.  That last step is the ledger's 8.2 and is NOT admissible: `n += 1`
on an unbounded accumulator has no range proof today (`(R-BoundedNest)`'s trip-count bound
over a text walk would be the rule — trips ≤ size(T) ≤ u32::MAX, term ≤ 2 — a lever of its
own, unpriced beyond this number).  So W1 ships the two admissible pieces; the compares are
left as they are.

The fast arm carries NO condition — it is a re-spelling exact for a byte in `1..=0x7F` at an
in-range index — and NUL is excluded on purpose: `text_character` answers it as the null
character and the walk's fault note fires, which the arm would silence.  The null test moves
only where `hoist::text_written` finds no write of the text in the loop (the predicate
`(R-LazySplit)`'s borrow already asked, now shared).  A call source (`for c in s.trim()`) is
evaluated PER ITERATION by the parser's lowering — three times per character, counting the
two bound tests — and keeps the written step; that lowering is a cost of its own, not
priced here.

### The single-row harness, 2026-09-22 — and what it attributed

`loft --native-release --names` already existed (loft#954, the browser build's frame
naming) and only needed the flag routed to the host-native path: every generated function
is `#[inline(never)]`, a row has a symbol, `perf annotate` attributes it.  Recipe in
`PERFORMANCE.md` § Attributing a bench ROW.  The named build times `mesh_aabb` and
`enum_match` the same as the plain one (37.7 / 41.3 µs both), so the instrument does not
move what it measures.

**`mesh_aabb` attributed (the row "no proof could beat"):** the loop was six null-aware
float compares, each `(a.is_nan() && !b.is_nan()) || (!a.is_nan() && !b.is_nan() && a < b)`
— two to three `ucomisd` self-compares and a chain of data-dependent branches per compare,
where the Rust twin is six branchless `minsd`/`maxsd`.  The truth table (b null → false;
a null → true; else `a < b`) is `!b.is_nan() && !(a >= b)`: one null test, one compare.
Hand-priced by editing the emitted twin: **37.8 → 21.5 µs**, hash unchanged.  Built as the
`#rust` template of `OpLtFloat` / `OpLtSingle` (both backends: `fill.rs` is generated from
it), proven against the pre-change interpreter over every pair of {null, ±1e300, −1, ±0,
1.5} and {null, −2, 0, 3} for `<`, `>`, `<=`, `>=`, `==`, `!=` — IDENTICAL on both backends
— and measured **38.7 → 20.9 µs, 4.81× → 2.73×** (`stats.py --only 16`).  One trap on the
way: the first template spelled `!@v2.is_nan() && !(@v1 >= @v2)`, and a template operand
used ONCE is inlined — the field read landed inside the `&&`, conditional on the null
test, and LLVM kept the branch before the load (29.4 µs).  Binding both operands first
(`{{ let _a = @v1; let _b = @v2; … }}`, the interpreter's left-then-right order) gave the
hand-priced form.  The earlier hand-price of "the plain IEEE compare, no change" was made
without attribution and with the operand still conditional; that is the number this
harness exists to correct.

**`enum_match` attributed (4.49×):** its own samples are 75 % of the row and FLAT (nothing
above 6 %), which is the signature of work spread over every operator; `LOFT_RELEASE_PASS_PROBE`
moved it only 41 → 34.5 µs, so the checks are not the ceiling.  Timed in four parts in a
scratch copy: the BUILD of 3 000 struct-enum records **26 µs** (8.8 ns each, linear in the
count — allocation is not it), the match walk 5.5 (1.8 ns an edit, already the R7 form: tag
and payload through the element's address, the element write fused), the sum 6.0 (F1's join
read plus checked adds), the two 4 096-fills 2.4.  Per appended record the emission did one
`rec_ptr` lookup for the three field writes and a SECOND store lookup for the TAG alone:
`OpSetEnum` was not a fusable setter.  Made one (`u8`, the byte `set_byte(…, 0, v)` writes;
`HoistScalar for u8` for the element-write path, on `append_byte`): the build −11 %, the
row **41.3 → 36.4 µs (−12 %), 4.49× → ~4.0×**.  What remains per record is four
`allocations[store]` lookups (the push, the zeroing, the address, the finish) around a
32-byte zero and five stores — the lever is a record push WINDOW under a branch, the
`(R-PushFill)` window clause extended from scalars in a counted loop to record mints under
an `if`, unpriced.

**Also found by the harness:** `entity_tick` runs 12 % SLOWER under `LOFT_RELEASE_PASS_PROBE`
(160 → 178 µs): a build with every check removed made an inlining decision worse.  Not
chased; noted so the probe is read as a ceiling per row and never as a bound.

## W1 — a character walk pays for its bookkeeping, not for any one thing

`for c in src` emits, per character: `text_character` (a bounds test, a byte load, the
ASCII fast path), a fault note, **a call** to `OpLengthCharacter` for the width the byte
already told it, a checked add, a `next <= index` guard, a `STRING_NULL` test and a length
test; and each `c == 'a'` in the body converts `c` through `to_char` and a null test and the
LITERAL through `char::from_u32` at run time.  The Rust twin is `for c in s.chars()`.

**What did not move, and why it is recorded.**  The two obvious halves — the width
inlined from the first byte, and the compares made plain — priced at 23.4 → 25.0 (slower)
and → 24.0 alone, 22.2 together (−4 %).  LLVM was already folding them; the profile of the
loop is FLAT, no instruction above 8 %.  The cost is the sum of the iterator's bookkeeping,
and only the whole walk rewritten moves it: a byte loop, ASCII one step, a multi-byte
character through `text_character`, the body's compares as `u32` compares —
**23.5 → 8.2 µs, 4.58× → ~1.6×**, hash `2e18` unchanged, three runs.  8636 characters:
2.7 ns each today, 0.95 rewritten, the twin 0.6.

Conditions, and where they already live: the text is not written in the body — `(R-LazySplit)`'s
condition, the same predicate (`hoist::may_write_store` over the body); `c` does not escape;
the compares are against NON-null literals.  The null character (a NUL the text holds,
loft#755) is what makes the last one sound without a proof: `null == 'a'` is false and
`'\0' == 'a'` is false, `null > '9'` is false and `0 > 57` is false — the null-aware and the
plain compare agree on every literal, so no fact about `c` is needed.  A compare against a
VARIABLE keeps the null-aware form.  `c#index` is the byte offset the walk already tracks.

## W2 — a walk of texts costs a `String` and a store resolution per element

`for p in words` binds `p` as `{ … get_str(…) }.to_string()` — an allocation and a copy —
and walks through `vector::get_vector` + `length_vector` per pass because a text element's
head (`OpGetText(OpGetVectorNullable(…), 0)`) is not a shape `hoist::iteration_head` reads.
Priced in two steps on the stdlib `join`: the borrow alone (T1) **30.8 → 17.1 µs**; the
borrow with the walk through a held header **→ 7.8 µs, −75 %, 5.40× → ~1.4×**, hash
`2df9`, three runs.  The twin is 5.7.

T1's size is unchanged from `vector-build.md` § T1 re-priced: *"a text variable is a
`&str`"* has no single home (six inline `is_argument && Type::Text` sites, three shipped
bugs), so the borrow is a one-predicate refactor first (proof: an empty
`scripts/introspect_diff.sh`) and the rewrite second.  The walk half is
`iteration_head` taught the text element's head — a shape, not a proof.

## C1+ — the frame decomposed, and a cheaper overflow guard

`fib` is 11.5 ms; with the fn-ref buffer guard, the depth count and the checked arithmetic
ALL removed it runs 2.56 ms — under the 2.96 twin.  So the remaining 4× is entirely the
frame, and it splits (ms): fn-ref guard **3.3** (29 %), depth count **3.1** (27 %), checked
arithmetic 1.3–2.5.

- **C1** (no `FnRefBufGuard`), on its CORRECTED condition from `vector-build.md` — every
  registrant reachable is a blocker (`CallRef`, `parallel`, `yield`, a native user function,
  `OpFreeRefOrHandUp`), over the call-graph closure, cycles included: **11.5 → 8.2 ms**.
- **The depth count is owed** — it is the stack-overflow guard, a reported fault where Rust
  would SIGSEGV — but a thread-local push at entry and pop at exit is one way to ask "is the
  stack nearly full?".  Comparing the stack pointer against a per-thread limit is another,
  with no write: `if (&local as usize) < limit { push }`.  Priced with the limit in a
  `OnceLock` (an atomic load per call, which a thread-local set at thread start would not
  pay): **8.2 → 7.0 ms**.  Together −39 %, 4.09× → ~2.4×.  The check is exact where the depth
  count was a proxy: it asks about the stack itself.
- Plain `n - 1` / `n - 2` (→ ~2.1×) needs a branch fact — `n > 1` in the arm — that the
  range proof does not carry; not admissible today and not proposed.

## P1 — one write declines a whole loop it cannot disturb

`map_set` (the `chunk_lookup` row) is `for i in 0..len(m.chunks) { if m.chunks[i]?.cx == cx
&& … { m.chunks[i].hexes[k].h_material = mat; return } }`.  The loop hoists NOTHING: the
write's target is an element of a vector that is itself a FIELD of an element — a path with
a variable index inside — which `hoist::setter_target` cannot type, so `body_writes` declines
the headers.  The write is an in-place scalar set: it writes a slot that exists and moves no
store, whoever the slot belongs to.  Without a header every one of the three joins per chunk
resolves the store through `get_vector`, `len(m.chunks)` is a CALL per iteration, and each
join's absent arm mints a default record.

Priced with the header and base held and the three joins folded (F1, built): **1113 → 782
µs, −30 %, 2.87× → ~2.0×**, hash `26d76`, three runs.  The write itself is kept verbatim.
The admission: an `OpSet<scalar>` whose target is an element address (`OpGetVector*`) over
a STORE-FREE address expression is `(R-InPlace)`'s in-place set whatever the path — the
element cannot move for being reached through another element.  `(R-Scalar)`'s hoisted
scalars must still be evicted by the write's TYPE (the target's record type is what
`setter_target` answers; an untypeable target evicts every scalar, as today).

## Priced and NOT moved — and the instrument that is missing

**`mesh_aabb` (4.81×) — ATTRIBUTED and moved to 2.73× on 2026-09-22 (§ Built, the
harness): the null-aware compare WAS the cost; the hand-price below missed it because the
operand it made plain stayed conditional.**  The analysis on record said the cost is the null-aware float
compare on fields no proof calls non-null.  Measured: the compare simplified to its
truth-table equivalent (`a < b || (a.is_nan() && !b.is_nan())`) — no change; the field read
twice (once for the test, once for the assignment) CSE'd — no change; **the plain IEEE
compare, the ceiling no proof could beat — no change** (37.5 → 38.8 µs).  LLVM folds all of
it already.  7168 vertices at 38.7 µs is 5.3 ns a vertex for six loads through a held
address and six compares, against the twin's 1.1, and the loop head is tight (a base-relative
address, the reads through it).  **Not attributed.**

**`enum_match` (4.24×).**  Its emission is the R7 form throughout — the tag and the payload
read through the element's address, the appends through a zeroed push header, the sets
fused.  **Not attributed** either.

**Why not: the lane inlines the row into `main`.**  `n_c_mesh_aabb` and `n_c_enum_match`
have NO symbol in a `perf record` of the lane — rustc inlines each into the timing loop — so
`perf annotate` shows `main`'s float math and cannot say which row's.  The instrument this
round lacked is a single-row measurement: an `#[inline(never)]` on every `n_<row>` in the
stats build (a `--native-emit` post-pass, or an attribute the emitter writes for a lane), so
a row has a symbol and `perf annotate` attributes it.  Cheap, and the two unattributed rows
are 4.8× and 4.2× — the largest record-class rows left.  Build it before either row is
guessed at again.

## Not work, unchanged from the earlier rounds

Keyed (ten rows) is store-format work; `split` is a text-representation question; `par` is
a subsystem no analysis has entered.  `sum` at 2.16× is the owner's accepted 2×.

## Order

**W1 and W2 first**: two rows in one subsystem, −65 % and −75 %, the largest prices here,
and W2 carries T1's refactor, which every text lever after it needs.  **Then the single-row
harness** — it decides what `mesh_aabb`, `enum_match` and `entity_tick` are actually paying
before another price is guessed.  **Then C1+** (one row, but every recursive function) and
**P1** (one row here, every find-then-write loop in a game's world code).  Together they take
four of the 17 rows over 3× to about 2× or under, and leave the record class with a real
measurement instead of a story.

## Method

`--native-release --native-emit` for the routine's Rust; the edit a rewrite would make,
applied by script; `bench/portal/hand_price.sh`; the lane binary on one core (`taskset -c 0`),
three runs, the hash column compared.  `perf record -e cycles:u` + `perf annotate` where a
form did not move, which is how the two unattributed rows were found to be unattributable.
