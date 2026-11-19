<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: phases A and B as built

The record of what [the plan](README.md)'s first two phases built and measured.

## Phase A as built

| Register | Rows | What it holds |
|---|---|---|
| `scripts/syntax_slots.tsv` | 132 slots | id, family, the rule that states the count, count class, expected runs, type, the statement |
| `scripts/syntax_fillers.tsv` | 47 fillers | 9 types, each with its plain variable and 1 to 10 other forms |
| `scripts/syntax_outside.tsv` | 40 lexemes | 12 `none`, 16 `decl`, 12 `unreached` |
| `scripts/syntax_slots_prelude.loft` | — | the world a cell runs in |

- **The lexicon is read from the source.**  `check` takes it from the lexer's two tables
  and from every literal the parser asks the lexer for, 103 lexemes today.  A lexeme in
  no slot, no filler and not listed outside fails `doc_hygiene`.
- **Twelve lexemes take an operand the matrix cannot reach** (`unreached`): the field
  expressions (`init`, `computed`, `default`, `$`), the file attributes (`#next`, `#read`,
  `#format`), `#lock`, `T.parse`, `yield from` and `panic`.  loft#2575 sits in that list
  (`f#next`).  They are owed, and each needs the prelude extended.
- **Eleven slots cite no rule**, because no rule states their count: the `while`
  condition, a field read, the three text reads, `is`, the two `assert` operands and the
  record-literal fields.  `slot_matrix.py list` prints both lists.
- **Every slot compiles with its plain variable, and every filler bumps its counter once
  and gives its type's plain value** (`run`, `fillers`): 158 and 47 cells, none failing.

**The first whole run at depth one**: 1,286 cells, 133 s on the interpreter with one process
a cell; 1,222 pass, 35 are refused, 29 fail.  The failing cells are three causes, on the
branch at `6ec7ddf15`:

1. A comprehension over a `sorted` or `index` source runs the source once per step (8
   cells) — in the pilot.
2. A collection assigned to a field at a computed place addresses the place three times,
   with every form of index (10 cells) — in the pilot.
3. **New.**  A block expression as the index of a compound write runs twice, where a call
   in the same position runs once: `vi[{ c.calls += 1; 1 }] += 3` (11 cells: all five
   compound operators, the text and collection appends through a path, and three runs under
   `?`).  The same on `--native-c` and on the release build of 2026-11-18.
   `(E-Asgn-Compound)` says the place is addressed exactly once.

## Phase B as built

The cells of one collection kind are the test functions of one generated program, run by
`loft --tests` with `LOFT_LEAK_CHECK=1`; a refused cell is taken out by the line its error
names and the program is run again, and a cell that takes the program down is found by
halving.  That makes the whole matrix a handful of processes a backend.

| Run | Cells | Time |
|---|---|---|
| every slot with its plain variable, interpreter | 158 | under a second |
| the whole matrix, interpreter | 2,868 with twins and order cells | 3 s |
| the whole matrix, interpreter and `--native-c` | the same | 39 s |

Of the 2,868 cells 2,746 pass, 61 are refused and 61 fail, 333 of them check order.
**Falsified:** against the release build of 2026-11-18, which has loft#2573, `run --slot
it-for --filler all` fails 8 of 36 cells; on the branch it fails none.

**The ten causes behind the 61 failing cells**, on the branch at `6ec7ddf15`, each
reproduced in a program of its own.  Both backends unless a row says otherwise.

| # | Slot | What happens | Channel |
|---|---|---|---|
| 1 | `it-comp-source` | a comprehension over a `sorted` or `index` source runs the source once per step | count |
| 2 | `wr-path-coll` | `h.ws[e].items = […]` addresses the place three times, with every form of `e` | count |
| 3 | the compound writes and appends | a block of one statement and a value as the index runs twice (`vi[{ c.calls += 1; 1 }] += 3`); a block of two statements runs once | count |
| 4 | `wr-elem-index` and the compound and removal forms | a write to an element whose index is a function-value call is lost: after `vi[fnv(at(c, 0))] = 3` the vector is unchanged | value |
| 5 | `wr-elem-value` | in `vi[i] = box(c, h).n` the right side runs before the index | order |
| 6 | `rd-text-slice` | in `x[h.ones[e]..]` the index runs before the text it indexes | order |
| 7 | `it-slice-source` | `for n in held(c, h).v[i..j]` frees a store twice | leak |
| 8 | `rd-coal-coll-absent` | `for p in h.ws[9].items ?? mk(c)` leaves a store behind, for the three keyed kinds | leak |
| 9 | `ct-yield` | `yield box(c, h).n` in a generator ends the interpreter with SIGSEGV; the C backend answers right | crash, interpreter only |
| 10 | `ct-parallel` | an arm that calls a function writing a record of the enclosing scope: the interpreter's worker panics or the process takes SIGBUS, the C backend runs it and the write lands.  The same write spelled in the arm itself is refused at compile time | crash, and the backends differ |

Cause 4 is a wrong answer with nothing said.  Causes 5 and 6 are `(E-Asgn)` and `(E-Left)`
not held for one form of operand each.  None of the ten is filed or fixed; they are phase
C's first work, in this order of severity: 4, 10, 9, 7, 8, then the counts and the orders.

**What the first runs taught about the tables.**

- A filler must be a primary expression or stand in parentheses.  `tx(c) + ""` in a slot
  followed by `[i]` parsed as another statement than its twin and read as a defect.
- The form of a filler matters below what its name says: a block of one statement is
  lowered differently from a block of two, and only one of them shows cause 3.  Depth two
  will multiply such differences; the fillers table is where they are written down.
- A receiver slot cannot take its expected value from the plain variable, because a call
  that returns `h.ws[0]` writes somewhere else than `r`.  The bound-first twin is the
  oracle wherever the operand runs exactly once.

