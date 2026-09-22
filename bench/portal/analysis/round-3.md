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

### The record push window under a branch — `(R-PushFill)`'s record clause, 2026-09-22

Built as the record clause of `(R-PushFill)` (`formal/rewrites.md`, the in-words paragraph
beside the window clause's), switch `LOFT_NO_PUSH_WINDOW`, trace `LOFT_TRACE_PUSH_FILL`,
cells `tests/scripts/158-record-window.loft` r1–r11 (identical on the interpreter and on
native under `LOFT_HOIST_VERIFY`, `LOFT_POISON`, `LOFT_POISON_CLAIM`, `LOFT_STRICT_STORES`
and `LOFT_NATIVE_LEAK_CHECK`; falsified by the finish emitted as nothing — r1 `0 0 0 0` for
`300 7261 6400 6566`), pins `tests/record_window.rs`.  A counted loop whose pushes are
record MINT GROUPS, at the top level or under `if` arms, reserves its trips, opens one
window, mints every group through it (the slot `base + len·size`, zeroed through the held
address; the field sets and `(R-RecPtr)`'s mint address are that pointer; the finish is the
window's bump) and closes it once after every copy of the loop:

| `enum_match` row (3 000 mints, three arms) | µs | ratio |
|---|---:|---|
| before | 35.9 | 3.98× |
| hand price (the emission edited by script) | 20.4 | ~2.3× |
| built, the fast path a CALL | 23.4 | — |
| built, `push_record_windowed` `#[inline(always)]` | **23.2–23.6** | **2.58×** |

`bench/stats.py --only 16` pins it, and the row is BIMODAL on both lanes — three runs, 7–9
interleaved samples each, the box idle (load 0.5): native alternates 23.2–23.6k and
27.2–27.4k ns, its Rust twin 8.9–9.0k and 13.1k, the modes visibly alternating across the
interleaved samples (`--show-samples`: native `23,246 27,369 27,197 27,211 23,609 27,292
23,232`, Rust `9,011 13,086 13,135 9,031 8,971 8,913 8,989`).  Mode to mode the ratio is
**2.58×** (fast / fast) and **2.08×** (slow / slow); the harness's median-of-medians reads
2.59–3.02× depending on which mode each lane's median lands in, and flags the row noisy at
14–16 %.  `mesh_emit` shows the same two modes on both lanes (native 104k / 116k, Rust 37k /
61–67k).  Both are the lane's rows that GROW a vector to thousands of elements across
several reallocations; the earlier window-clause ledger met the same bimodality on
`record_append` and left its cause unestablished, and so does this one — it is a property of
the measurement on this box, not of the emission (byte-identical across runs), and the fast
mode's figure is the row's price.

The declines are cells: a group whose literal fills the element's own vector field (r4 — a
claim in the store the base is held in), a read of the vector in the body (r7), a `break`
(r8), groups on two vectors (r10), a view of the vector read inside the body (r11); a view
read only AFTER the close is admitted (r9), and so is a nested-record element (r5) — its
`pos.x` set has a field on the root, and the ROOT being the fresh element is what decides.
Three lessons: the window's close must stand after EVERY guarded copy of the loop (the
first hand placement closed the checked copy alone and read hash `0` — a length nothing
wrote is an empty vector to the consumer, and only the value channel says so); a fast path
LLVM keeps as a call costs 15 % of the gain until it is forced inline; and the finish's own
element operand is a mention of the vector to a naive count, which would have declined
every group.

### The split table — `(R-SplitTable)` + `(R-TextBorrow)`'s discharge clause, 2026-09-22

Built as the rule `(R-SplitTable)` (`formal/rewrites.md`), switch `LOFT_NO_SPLIT_TABLE`,
trace `LOFT_TRACE_SPLIT_TABLE`, cells `tests/scripts/158-split-table.loft` s1–s21 (identical
on the interpreter and on native under every falsifier; falsified by a getter answering the
slice at `from + 1`), pins `tests/split_table.rs`.  Hand-priced first on the lane's own
emission, then built to the price:

| `split` row (200 pieces) | µs | ratio |
|---|---:|---|
| before | 21.2 | 7.8× |
| the table alone — `Vec<&str>` at the bind, `len` and `v[i]` from it | 6.5 (−69 %) | 2.4× |
| + the `?`-discharge temp BORROWING its slice (the two copies gone) | **3.1 (−85 %)** | **~1.15×** |

The twin is 2.7 µs; `bench/stats.py --only 13` pins it at **3,175 ns vs 2,753, 1.15× (range
1.05–1.24)**.  (That run read `char_walk` at 3.16× against its pinned 2.48×; its emission
is byte-identical before and after, and the two binaries time 12.5–12.9 µs back to back —
every row of the run was flagged noisy, so the box, not the row.)  Two units, each a rule:

1. **The table** is `(R-LazySplit)`'s sibling for the shape it declines — the vector bound
   to a name — and the shape every library bind takes (14 binds, all local, walked / indexed
   / `len`'d).  The bind collects the lazy split's own iterator (`lazy_split(src, c)
   .collect()`), so the pieces are the same by construction; `len` is the table's length;
   `v[i]` goes through `codegen_runtime::split_table_get` / `_or_raise`, which answer what
   the two element-read twins answer (negative from the end; the null text + the oob note for
   the nullable read; the recoverable `IndexOutOfBounds` / `NegativeIndex` for the raising
   one — `LOFT_DEV_SOFT_HALT` halts both forms alike, checked); the walk `for p in parts` is
   the parser's `_vector_N = parts` plus the two readers, read as an ALIAS of the table, so
   `p` borrows its slice with no store condition and — the table being random access — a
   `rev` and a comprehension over the name are walks too, where the lazy iterator must
   decline them.  Every mention must stand inside the binding block after the bind (the
   table is a Rust `let`); a `parts` returned, appended, written, handed to a call, copied,
   rebound, a variable separator, a generator decline and keep the vector.  It rides on
   `(R-Wrapper)`: with `LOFT_NO_WRAPPER_INLINE` `len` stays a call and no table is admitted.
2. **The discharge clause** of `(R-TextBorrow)`: `parts[i]?` was one copy into the `__ncc`
   temp and a second at the block's tail, where `_ret.to_string()` materialised a value that
   MIGHT borrow a block-local.  The temp is now the block-local it might have borrowed — a
   `&str` into the table's source, which outlives the block and the statement — so the
   temp binds the slice, the null test reads it bare, the block yields it, and the void form
   (`x = parts[i] ?? d`) copies once into the local that owns it.  `hoist::text_escapes`
   learned the discharge's own value arm (`if bool(p) { p } else { d }`) and the one bind the
   caller admits; nothing else in the walk rule moved.

What it found on the way: (a) the parser picks between TWO element-read twins by the site —
`q: text? = parts[1]` followed by a null test is the nullable read, the same bind formatted
untested is the RAISING one — so a table that answered only the nullable twin declined the
"all readers" cell; both are served now, each by its own getter mirroring its op's
template.  (b) The nullable read's template notes an oob fault for the `(oob)` suffix a
formatted hole renders; the getter notes it too, and the text hole never rendered the
suffix on either backend (a cell pins `null` bare), so the note is parity kept, not a
value seen.  (c) `LOFT_NO_LAZY_SPLIT`'s pin grepped for `lazy_split(` — which the table
now emits under its own switch; the pin keys on the loop form's marker.  (d) A NULL text
under `?` is `""` (`?` is the type's default), where the walk's piece is the null text of
size 1: the first s2 expectation was hand-computed wrong and the interpreter corrected it.

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
row **41.3 → 36.0 µs (−13 %), 4.49× → 3.97×** (`stats.py --only 16`, pinned).  What remains per record is four
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

**Diagnosis CORRECTED before it was built (2026-09-22).**  The paragraph below blames the
setter's untypeable target; a four-variant probe (the loop with the setter removed, the same
loop over an all-scalar record, the nested setter alone) under `LOFT_TRACE_HOIST_DECLINE` says
otherwise.  The nested setter alone HOLDS the loop's header (`(R-InPlace)` admits an
`IN_PLACE_SET_OPS` call whatever its target, and `element_target` types the target off the
field's schema — `hexes: vector<Hex>` — so the scalar walk types it too).  What declines the
headers is the three `m.chunks[i]?` JOINS: each mints its absent-arm buffer
(`OpDatabaseNP(__ref_p2_N)`), and `null_buffer_alloc` admitted that mint only for an
ALL-SCALAR record — `Chunk` holds `hexes: vector<Hex>`, so the mint read as a store writer.
The restriction was over-conservative: the buffer's store is reachable only through the
site's temp, rebound on every run, so no loop-invariant path names it and no header or base
can go stale; a growth of its vector field from the body is a push through a non-pure path,
declined on its own.  Built as the one-line relaxation of the hidden-buffer allowance
(`hoist::null_buffer_alloc`; rule text in `formal/rewrites.md` § (R-InPlace)), cells
`tests/scripts/158-heap-discharge-buffer.loft` h1–h6, pins `tests/heap_discharge.rs`.  The
join reads then fold through the held base by `(R-Base)`'s join clause on their own.
**Measured** (`bench/16_consumer_shapes`, `hand_price.sh` on the emission with the allowance on
and `LOFT_NO_NULL_BUFFER_HOIST=1` as the before, one core, three runs each): `chunk_lookup`
**1 777 → 816 µs per call (−54 %)**, hash `26d76` — past the ledger's −30 %, because the base
and the folded joins came together.  Left on the row, unpriced: the range's bound
`len(m.chunks)` is still a CALL per iteration — `(R-Wrapper)` inlines a one-op wrapper only
over LEAF arguments (the pre-eval map keys on node addresses), and `m.chunks` is a field
path; with the header held it should read `__vh_1.len`.
**What it found on the way** — a pre-existing silent-wrong on native, independent of P1: a
`&`-bound LINK to a view the loop rebinds passed for a loop-invariant root, so `g = &e; for
i in … { e = h[i]; … g.tags[0] … }` hoisted `g.tags`' header before the loop and read
`h[0]`'s tags on every pass (15 for 16 — with every switch on or off; `LOFT_HOIST_VERIFY=1`
panicked on the stale header).  The all-scalar restriction had hidden the SCALAR twin of it
(`g.b` over a `?`-discharged view) by declining the whole loop, which is how P1 surfaced it:
`157-null-buffer-hoist.loft`'s `alias_no_default` answered 15 for 7.  Fixed at the one home
— `hoist::rebound_vars` closes the rebind set over links (a link to a link included) and
`hoist::rebinds_root` answers the single-root form for a view's header, a record's address
and a mint group's path; cells `158-link-rebind.loft` l1–l7, and the `(R-InPlace)` rule
text carries the link clause.

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

## `split` and the vector it returns — the owner's question, answered (2026-09-22)

Asked: *can `split` return slices? I am not married to the vector of texts it returns.*
And the wider one: *can't every vector be a lazy iterator, materialised only when it is
stored in a way that demands it?*

**The answer the rules give: keep the API, take the representation.**  `(R-Escape)` (C121)
already says a construction that does not ESCAPE the unit may be represented any way its
conditions allow — the vector `split` promises is the contract at a library API and nowhere
else.  A count of every use in the repo and the library checkouts (`grep` over `*.loft`,
2026-09-22): repo — 36 walks `for p in x.split(c)` (lazy since `(R-LazySplit)`), 3 binds,
1 inline index; libraries — 14 binds, 2 inline indexes, 1 walk.  **Every bind is a LOCAL
that never leaves its function**: `lines = src.split('\n'); for line in lines`,
`nl = len(lines)`, `lines[0] ?? ""`, `parts[i] ?? ""`, and the `nth_word` idiom
`line.split(' ')[i] ?? ""`.  None is returned, stored in a record, appended to or handed to
a call.  Changing the API to an iterator would break those 14 sites (an iterator has no
`[i]` and no `len`) to reach a representation the compiler may take without asking.

**The unit — the split TABLE, `(R-LazySplit)`'s sibling.**  `parts = src.split(c)` where
`parts`'s only mentions are `len(parts)`, `parts[i]` / `parts[i]?` / `?? d`, `for p in
parts` and `parts#…`: on native the vector is never built — one pass over the source records
the piece offsets (the Rust twin's own `collect::<Vec<&str>>` work), `len` is the table's
length, `parts[i]` is a `&str` slice served like `(R-TextBorrow)`'s element, and the walk
through the name is today's lazy split (which currently DECLINES a vector bound to a name
first — that decline goes).  The source is borrowed where nothing writes it or copied ONCE
at the bind, `(R-LazySplit)`'s condition verbatim; a NUL separator, an element write, an
append, a return, a call taking `parts`, a second binding — decline and keep the vector.
The `nth` shape `x.split(c)[i]` needs no table: walk to the i-th piece.  The interpreter
keeps the vector and is the oracle.  Expected, unpriced: `split` 20 µs toward the twin's
2.7 (7.5× → near 1×) — the table pass IS what the twin does.  Hand-price on `t_split` first.

**Slices as a TEXT representation — recorded, not proposed.**  A text element that is a
view into another text ((base, offset, length) in a store) is what "return slices" means
literally, and it is the only route for a split that ESCAPES (returned, kept in a struct
beyond the source's life).  It is a store-format change touching every text op on both
backends — the keyed rows' class of work — and nothing found needs it.

**The general question — `(R-Fuse)`, a rewrite rule, never a semantics.**  "Every vector
is lazy until stored" as language SEMANTICS makes evaluation ORDER observable: the
producer's work interleaves with the consumer's (`for x in build(v) { touch(v) }` reads a
different `v`; a producer's prints or store writes reorder), so C120 and the feature freeze
rule it out as a default, and it would leave no oracle (an eager interpreter would disagree
with native; a lazy one would check nothing).  As a REWRITE it is what `(R-Escape)`
licences and what the tree is half-way to: `(R-LazySplit)` is this rule for one function —
`split`'s body is a builder (`out = []; for c in self { … out += [piece] }; out`), the
consumer one forward walk, the vector never built — and `(R-RetAdopt)`, `(R-MoveAppend)`,
`(R-Place)`, `(R-ValueRecord)`, `(O-Buffer)` are the same idea one representation each.
The generalisation:

```
  (R-Fuse)       a vector built by a loft-bodied BUILDER (one append loop over an
                 invariant source, no effect a consumer could reorder) and consumed by
                 exactly ONE forward walk, whose body writes no store the builder reads,
                 is never materialised: the builder's loop body is emitted at the
                 consumer's element read.  Random access materialises the builder's
                 RECIPE (the source and its offsets), never its elements.  Every other
                 use — returned, stored in a field, appended, captured, handed to a
                 call, consumed twice, indexed out of order, `rev` — DECLINES and keeps
                 the vector: the vector is the safe default, and a half-consumed
                 producer is never replayed or buffered.
```

Three conditions, each with a home already: the builder shape (`push_loop`, the
comprehension recogniser), the effect condition (`may_write_store`, `call_writes_store`,
`in_place_only_writer`), and non-escape (`(R-Escape)`'s escape walk).  **Where it pays:**
an eager builder consumed once already costs little for SCALARS (the buffer is adopted and
reused, its capacity kept — `comprehension` 1.70×, `push` 0.50×; fusion would reach ~1.2×);
the cost concentrates where the elements OWN HEAP — `split`'s 7.5× is its per-piece text
copies, not the vector — so the rule pays first on builders of texts and heap records
(`split`, `lines`, `words`, tokenisers), which is where the two library shapes live.

**Order recommended:** (1) the split table now — the worst row, a bounded unit, and it
settles the recipe representation the general rule reuses; (2) a `@PLN` issue for
`(R-Fuse)` (claim the number FIRST, `loft-lang/plans`, one `status:*` + one `subject:*`
label) opened with a CENSUS before any design: a script over the corpus and the library
checkouts counting `v = build(…); for x in v` by element kind (scalar / text /
heap-record) and consumer shape, then a price on one real row before the emitter is
touched.  The test of the general rule is that it RE-DERIVES the special case: if `(R-Fuse)`
is right, the stdlib's `split` body is the spec and `codegen_runtime::lazy_split` its
first generated instance — no more hand-written iterators per builder.

## Order — and where it stands after 2026-09-22

Built, in this order: W2 (`join` 1.96×, four more text rows to ~1×), W1 (`char_walk` 2.48×,
`split` 7.8×), the single-row harness (`--names`), `mesh_aabb` attributed and moved
(2.73×), `enum_match` attributed and moved a step (3.97×), **the split table** (`split`
7.8× → ~1.15×, § Built), **C1** as `(R-GuardFree)` (`fib` −34 %; the stack-pointer half
DECLINED — `MAX_CALL_DEPTH` is a cap both backends report identically, D-op-1), **P1** as
the relaxed hidden-buffer allowance (`chunk_lookup` −54 %, § P1's corrected diagnosis).
The portal was re-measured at `9496b1622` (median 1.85× every routine, 1.60× shipped, 16
over 3×).  **Round 4 (2026-09-22, evening)** takes the levers found on the way, smallest
first: **`len(<field path>)` under a held header** — BUILT: `(R-Wrapper)` now inlines a
one-op wrapper over a pure vector path (`hoist::vector_path`, no pre-evaluation binding),
so `for i in 0..len(m.chunks)` reads `__vh_1.len` per iteration instead of calling
`t_6vector_len` — `chunk_lookup` 816 → 751 µs (−8 %, hash `26d76`), six of the consumer
bench's seven `len` calls gone; 88 loops in the corpus and the libraries are bounded that
way.  **What the rebase onto main found**: main's join brought the loft#1575 cells, and P1
on that tree lost one — a literal handed to a call (`s = me(Bx { v: [i], n: 7 })`) is built
in a `__ref_p2_` buffer re-minted per pass, the IR spells the re-mint as a bare call, and a
push header hoisted off the buffer's vector field pushed into the record the previous pass
claimed (`c3` read `null(oob)` for `1`; `LOFT_NO_NULL_BUFFER_HOIST=1` and
`LOFT_NO_VECTOR_HOIST=1` were the two switches that cured it).  The allowance's all-scalar
restriction had hidden it.  Fixed at the one home: a pass-2 buffer's re-mint counts as a
rebind of the buffer (`hoist::rebound_vars`), so no holder is taken off a path rooted at
it; the rule text carries the clause, and two pins read P1's admissions (`copy_in_place`
c8's nested-record write-back loop, `null_buffer_hoist` d13).  **The text element read through the held base** — BUILT as `(R-Base)`'s text clause:
the element's record number through the base, the text sliced off the store's data span
derived once beside it; `join` 11.0 → **6.6 µs** (−40 %, 2.11× → ~1.15×), hash `2df9`.
Three prices on the way, each a lesson: the helper resolving the store per element gave
10.2 (a bounds-checked index LLVM could not hoist); with the span hoisted but the helper's
cold half folded in, 8.4, and OUTLINED, 9.0 — the past-the-end read a walk makes on its
last pass was a CALL in the loop either way; answering it as the null text without a call
(what `get_vector` answers for any non-negative index past the length) gave 6.6, under the
hand price.  `bench/stats.py --only 13` pins it: **`join` 6,032 ns vs 5,716, 1.06× (range
1.05–1.06)**; the text lane's median is 1.19×, nine of its ten rows within 2× and only
`char_walk` (3.14×) over.
**The trip-count bound for a text walk's accumulator** — BUILT as `(R-Range)`'s
accumulator clause (`range::seed_accumulators`): a local seeded once by a ranged value and
stepped only by literals inside a character walk over an unwritten text is ranged by
`seed ± Σ|c| · u32::MAX`, so its steps emit plain; `char_walk` 12.5 → **10.45 µs** (−17 %),
hash `2e18` — `stats.py --only 13` pins it at **10,495 ns vs 5,063, 2.07× (range 2.07–2.08)**,
down from 3.14×; the text lane's median is 1.20× and its worst row is this one — the W1 ledger's 8.3 was measured on an older emission and is not reached: the
row's remaining cost is the two null-aware character compares against literals, which LLVM
folds already (W1's finding).  Cells a1–a11 walk the admissions (a negative step, two walks,
an `if`, an outer loop re-seeding) and the declines (the seed outside the enclosing loop, a
non-literal step, an appended text, a second seed, a counted loop, a parameter seed).
**The record push window under a branch — HAND-PRICED** (the emitted `c_enum_match` build
loop edited by script: the vector reserved for its 3 000 trips before the loop, a
`vector::push_window` opened on the header, each arm's mint taking its slot from the window
— `base + len·32`, zeroed through it — its address the same pointer, its finish `len += 1`,
and the record's length written once by `push_window_close` after the loop, on every copy
the guarded chain emits): **35.9 → 20.4 µs (−43 %, 3.98× → ~2.3×)**, hash `1493`.  The
first placement closed only the checked copy of the loop and read hash `0` — a window whose
close does not run leaves the length at 0, which the consumer loop reads as empty; the
price stands only with the hash.  To build: `(R-PushFill)`'s window clause extended to
record MINT GROUPS under `if` arms of a counted loop — at most one mint per pass, so the
trip-count reservation bounds the total and the window never grows inside the loop.
**BUILT** as the record clause (§ Built): 35.9 → 23.2–23.6 µs, the fast path forced inline;
`stats.py --only 16` pins it at **2.58× mode to mode** (the row is bimodal on both lanes, §
Built has the samples), down from 3.98×.  Next: `for c in s.trim()`'s per-iteration call
source.  Levers this round FOUND
and left unpriced, each with its number: a
record push WINDOW under a branch (`enum_match`'s build, 8 ns a record), a text element read
through the held base (`join`'s remaining 11.2 → ~7.8), a trip-count range bound for a text
walk's accumulator (`char_walk`'s 12.9 → 8.3), and the parser's per-iteration evaluation of
a call source in `for c in s.trim()`.  The closing step is unchanged: one full
`make perf-portal` ALONE on the box.

**The order as written:** W1 and W2 first: two rows in one subsystem, −65 % and −75 %, the largest prices here,
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
