<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: the defects the matrix reports

The hand-off list for whoever fixes them.  Each row is a fold of failing cells of
`scripts/slot_matrix.py` on the branch as it stood late on 2026-11-19 (191 slots, 96 fillers, 6,061 cells); the cells themselves are the lines of
`scripts/slot_matrix.baseline`.  This file is the record of what was found on 2026-11-19;
the baseline is what holds today, and § What the later axes added says what changed in this
list after its first writing.

**To see one:** the command in its row, which prints every cell of the slot with its
statement and the compiler's own words.  Add `--context all --frame all` where the row names
a context.  **To debug one:** `python3 scripts/slot_matrix.py explain <cell>` with a cell's
name from that output — its program is kept, loft's diagnostics are on, and the IR of the
cell is diffed against the IR of its bound-first twin.
**When one is fixed:** its cells pass, the gate (`tests/slot_matrix.rs`) fails on their
baseline lines, and `python3 scripts/slot_matrix.py gate --update` removes them — read the
diff, it must only remove lines.

A statement below uses the names of `scripts/syntax_slots_prelude.loft`: `c` is the counter,
`h` a record holding `ws: vector<W>`, `v` and `ones`; `vi` a `vector<integer>`; `xs` a
collection of `P { k, t }` of the kind in the cell's name; `at(c, n)` answers `n`, `box(c,
h)` answers `h.ws[0]`, `held(c, h)` answers `h`, `mk(c)` a fresh collection, each bumping `c`.

## Wrong answers and crashes

| # | Statement | What happens | Where | See it |
|---|---|---|---|---|
| 1 | `vi[fnv(at(c, 0))] = j;`, `vi[poke(c, vi)] = j;`, `vi[grow(c, vi)] = j;` and the `+=` `-=` `*=` `/=` `%=` and `= null` forms | the write is lost: the vector is unchanged.  Any local or parameter vector whose index holds a function-value call, or a call that writes that vector (`poke` writes an element, `grow` appends).  An index that only READS the vector through a call (`ident(c, vi)[0]`) writes correctly | both backends; **new since v2026.11.0** | `run --filler all --slot wr-elem-index --backend both` |
| 2 | `parallel { add2(at(c, 1), j); add2(i, j); }` | an arm calls a function that writes a record of the enclosing scope: the interpreter's worker panics or the process takes SIGBUS; the C backend runs it and the write lands.  The same write spelled in the arm is refused at compile time | backends differ | `run --filler all --slot ct-parallel --backend both -v` |
| 3 | `yield box(c, h).n;` in a generator (also a block of two statements as the value) | the interpreter ends with SIGSEGV in `OpAppendText`; the C backend answers right | interpreter | `run --filler int-field --slot ct-yield --backend both -v` |
| 3b | `fn gen(c: Ctr, h: Hold, i: integer, j: integer, xs: vector<P>) -> iterator<integer> { yield i; yield j; }` walked by `for n in gen(c, h, i, j, xs)` in the cell's frame | a generator with the world's collection of records among its parameters, or the tuple holding one (`tk: (vector<P>, integer)`), ends the interpreter before it yields; with neither it runs, and a `vector<integer>`, a record and a function value as parameters are fine | interpreter | `cell 'for n in gen(<ARGS: c h i j xs>) { s += n; }' --decl 'fn gen(<ENV: c h i j xs>) -> iterator<integer> { yield <S>; yield j; }' --type int` |
| 4 | `s = (pairw(c).0 ?? r).n;` | loft#2577.  Where the statement's values are locals it is a wrong value on the interpreter and a store freed twice on C; where they are parameters (`--frame params`) it is a store freed twice on both | both | `run --filler nrec-tuple-call --slot rd-coal-rec-left --backend both -v` |

**Cause of 1, measured.**  `scopes::args::copy_written_vector_operands` (the fix of
loft#2503, `@FR-E-Left` for a vector operand) copies the indexed vector into a `__lift_N`
store whenever `writes_vector` says the index may write it, and `writes_vector` answers yes
for every `Value::CallRef`.  It does so for an `OpGetVector` that is the PLACE of a write as
well as for one that is read, so the write lands in the copy.  `loft introspect` shows the
`!! INSERT` block (`OpDatabase`, `OpAppendVector(…, a, …)`) under `OpSetInt`;
`LOFT_NO_COPY_WRITTEN_VECTOR=1` makes every form write again.  An index KNOWN to write the
vector loses the write the same way (measured: the fillers `int-writes-elem` and
`int-writes-length`).  A view bound from such an element and written later (`r = ws[f(0)];
r.n = 5`) is expected to lose it too; that was not run.  A place must
address the live vector: the copy belongs to a READ of the element only.

## Stores: freed twice, or left behind

| # | Statement | What happens | Where | See it |
|---|---|---|---|---|
| 5 | `for n in held(c, h).v[i..j] { s += n; }` — a SLICE of a vector field of a call's result.  As an operand of its own (`held(c, h).v[0..]`) it fails in six slots, and bound to a local first as well | a store is freed twice | both; **new since v2026.11.0** | `run --filler all --depth 2 --slot it-for-vint --backend both` |
| 6 | `s = (tk.0[at(c, 2)] ?? P { k: 5, t: "d" }).k;`, `tk` a tuple whose keyed member was built from a literal | loft#2576: a store is freed twice, for `sorted`, `index` and `hash` | both; new since v2026.11.0 | `run --filler nitem-tuple-member --slot rd-coal-item-left --backend both -v` |
| 7 | `for p in h.ws[9].items ?? mk(c) { s += p.k; }` | a store is left behind, for the three keyed kinds | both | `run --filler coll-call --slot rd-coal-coll-absent --backend both -v` |
| 8 | `if b { for p in box(c, h).items { s += p.k; } }` — a `for`, a filtered `for` or a `??` over a keyed collection that is a field of a call or a `?? []`, standing in an `if` or an `else` branch | a store is left behind; the bare statement is clean.  For the `??` form a plain variable leaks too | both | `run --filler all --slot it-for --context branch --backend both -v` |

## Evaluation count and order

| # | Statement | What happens | Where | See it |
|---|---|---|---|---|
| 9 | `q = [for p in mk(c) { p.k }];` over `sorted` or `index` | the source runs once per step (five times for three elements); loft#2573's fix reached the `for` statement only | both | `run --filler all --slot it-comp-source --backend both -v` |
| 10 | `h.ws[at(c, 1)].items = [P { k: 7, t: "q" }];` | the place is addressed three times, with every form of index | both | `run --filler all --slot wr-path-coll --backend both -v` |
| 11 | `vi[{ c.calls += 1; 1 }] += j;` — a block of ONE statement and a value as the index of a compound write or an append through a path | the block runs twice (three times under `?`, four with two appended items); a block of two statements runs once | both | `run --filler int-block-one --slot wr-add-index --backend both -v` |
| 12 | `vi[i] = box(c, h).n;` — a FIELD of a call's record as the right side, whatever the call (`own(c, r).n`, `same(box(c, h)).n`) and whatever the element type: the same for a `vector<float>` (`vf[i] = box(c, h).f`), a `vector<boolean>`, a vector of an enum, and a `u8`, `i16` or `i32` field read as the right side | the right side runs before the index, against `(E-Asgn)` | both | `run --filler int-field --slot wr-elem-value --backend both -v` |
| 13 | `o = x[h.ones[at(c, 0)]..];` — an ELEMENT read or a FIELD read as the slice bound, whatever stands inside it (depth two: every filler in `h.ones[…]` and in `….n`) | the bound runs before the text it indexes, against `(E-Left)` | both | `run --filler int-elem --slot rd-text-slice --backend both -v` |

## What the later axes added

After this list was first written the matrix grew: operands that alias or write what the
statement works on, a loop and a branch around the statement, parameters and views in place
of locals, fillers inside fillers, an anchor per slot, and forty slots for `float`, `single`,
`character`, the narrow integers, `boolean` and enum elements.  What that changed here:

- **Row 1 is wider**: every index that WRITES its vector loses the write, not only a
  function-value call.
- **Row 5 is wider**: the slice of a call's vector field fails in six slots, and bound to a
  local first as well.
- **Row 8 is new**: it shows only with a branch around the statement.
- **Rows 12 and 13 are wider**: any element or field read as the operand, any element type.
- **Row 4 depends on the frame**: a wrong value with locals, a double free with parameters.
- **Rows 10 and 11 hold for the keyed kinds too**: with `sorted`, `index` and `hash` as the
  collection, the place of a collection assignment is still addressed three times, and a
  one-statement block as the index of an append through a path still runs twice.  The
  appends themselves, the keyed lookup by key and the replace of a record whose key is
  already present are right for every form of operand.
- **Nothing else.**  The views frame, the anchor (174 hand-computed expectations, all
  held by both backends) and the forty slots of other operand types found no cause that is
  not in the rows above.  Against the release of 2026-10-01 the walk shows row 12 as the
  last open instance of an order defect that then stood in twelve slots.

What the matrix lists as REFUSED is now only what the language refuses: ten cells, in the
three entries of § For the rules about a slice and the two `parallel` arms that write a
variable of the enclosing scope.  Over a hundred earlier refusals were gaps of the tables —
a function declared by a slot that was not handed the values its fillers name, a slot
generated for a collection kind it has no meaning in (formatting a keyed collection,
`#index` over an `index`), a nullable filler in a narrow-integer slot — and are gone; a name
the cell's world lacks now FAILS the run as a fault of the tables.

Two things that look like defects in a run and are not:

- A writing operand on the right of a compound assignment (`r.n += renum(c, r)`) differs
  from its bound-first twin.  `(E-Asgn-Compound)` reads the place before the right side
  runs, so the statement is right and the twin is not the oracle there.
- `for n in 0..{ … } { … }` never ends: it is an open-ended range whose body is the first
  block.  loft's time limit stops it; the slots put such an operand in parentheses.

## For the rules

- Settled (owner): the right side of `??` is evaluated only when the left is null.
  `(E-Left)`'s prose in `formal/operational.md` said otherwise and now agrees with
  `(E-Coalesce)`, which states it; the slots `rd-coal-right-present` and
  `rd-coal-right-absent` hold it on both backends.
- Every slot cites a rule now.  The eight that cited none are covered by `(E-Once)`,
  `(E-Field)`, `(E-Index)`, `(E-Is)`, `(E-Record)` and `(I-While)`, written for them.
- Settled (owner): an `assert`'s message is evaluated only when the assertion fails.  It
  is `(E-Assert)` in `formal/operational.md` now, cited by the slots `ct-assert` and
  `ct-assert-message`, and by the lowering in `src/parser/control.rs`.  The change from the
  release of 2026-10-01, which evaluated the message always, was meant (loft#2090);
  `GUARDS.md` still taught the old behaviour and is corrected.
- **Indexing a slice in place is refused with the wrong words.**  `mkv(c)[0..][i]` — an
  element of a slice of a call's vector — is refused with "Indexing a non vector — keyed
  collections (hash/sorted/index/spatial) have no generic-constructor expression", which
  describes another mistake.  Either the refusal is lifted or it says what it refuses.
- **`map`, `filter` and `reduce` refuse a slice as their source** (`map(mkv(c)[0..], inc1)`:
  "first argument must be a vector"), while `LOFT_DATA.md` says a slice is a vector wherever
  a vector is expected.  Either the refusal is lifted or that sentence gains its exception.
- Settled by a rule that was there: `vi[i] = null` on a `vector<integer>` stores the null
  and keeps the length (`[10,null,30,40]`), with a warning.  That is `(N-Store)` — a vector
  element reserves a null it can hold — and not `(Col-Remove)`, which the slot had cited
  wrongly; the slot `wr-remove-index` cites `(N-Store)` now and has its anchor.

## What the matrix cannot say about these

It compares a statement with its bound-first twin, so each row above is a place where the
FORM of the operand changes the outcome.  It gives no expected value of its own: a fix is
right when the cell agrees with its twin AND the twin is right, which the defect's own
regression cell must state by hand.
