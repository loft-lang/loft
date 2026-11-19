<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: the defects the matrix reports

The hand-off list for whoever fixes them.  Each row is a fold of failing cells of
`scripts/slot_matrix.py` on the branch at `27ae8d01a`; the cells themselves are the lines of
`scripts/slot_matrix.baseline`.  This file is the record of what was found on 2026-11-19;
the baseline is what holds today.

**To see one:** the command in its row, which prints every cell of the slot with the
compiler's own words.  Add `--context all --frame all` where the row names a context.
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
| 4 | `s = (pairw(c).0 ?? r).n;` | loft#2577 | both | `run --filler nrec-tuple-call --slot rd-coal-rec-left --backend both -v` |

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
| 12 | `vi[i] = box(c, h).n;` — a FIELD of a call's record as the right side, whatever the call (`own(c, r).n`, `same(box(c, h)).n`) | the right side runs before the index, against `(E-Asgn)` | both | `run --filler int-field --slot wr-elem-value --backend both -v` |
| 13 | `o = x[h.ones[at(c, 0)]..];` — an ELEMENT read or a FIELD read as the slice bound, whatever stands inside it (depth two: every filler in `h.ones[…]` and in `….n`) | the bound runs before the text it indexes, against `(E-Left)` | both | `run --filler int-elem --slot rd-text-slice --backend both -v` |

## For the rules

- Settled (owner): the right side of `??` is evaluated only when the left is null.
  `(E-Left)`'s prose in `formal/operational.md` said otherwise and now agrees with
  `(E-Coalesce)`, which states it; the slots `rd-coal-right-present` and
  `rd-coal-right-absent` hold it on both backends.
- Eleven slots cite no rule because none states their count; `slot_matrix.py list` names
  them.

## What the matrix cannot say about these

It compares a statement with its bound-first twin, so each row above is a place where the
FORM of the operand changes the outcome.  It gives no expected value of its own: a fix is
right when the cell agrees with its twin AND the twin is right, which the defect's own
regression cell must state by hand.
