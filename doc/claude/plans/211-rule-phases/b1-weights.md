<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN211 B1 — the weight of each rule on the interpreter (measured 2026-11-03)

The input to B2's third question (*worth it?*).  Produced by

```bash
make interp-gap ARGS="--writer c --no-timing <every library package>"       # target/interp-gap-c/
make interp-gap ARGS="--writer native --no-timing <every library package>"  # target/interp-gap-native/
```

over the 56 bench programs the interpreter can run (`ssh` and `web` call library natives the
census build does not load).  27 of them are read whole, as one `(all rows)` region: a library
bench builds all its rows under one line of `main`, and five programs (`random`, `gridmesh`,
`drawing`, `markdown`, `html`) pair their rows with row lines while the work runs elsewhere —
their regions held 0 % of their operators, so before this fix every one of their routines read
as idle and their rules weighed nothing (bench/README.md § Interpreter against native).

**What the weight is.**  *Worth* sums, over the routines, the share of each routine's
interpreter operators spent in the functions the rule is admitted in under that writer — every
routine counts once, whatever its speed.  It is where the interpreter does the work a rule
removes on the compiled side, NOT what moving the rule saves: a rule admitted in `main` or in a
routine's one big function carries that whole function.  B2's probe prices the saving.  No
timing was taken, so there is no `interp ns` weight.

**The checks.**  The join is by function name: of the 5080 functions the C census credits,
5079 carry the same name in the rustc census, and every function the interpreter executed that
admits a rewrite is named alike in both censuses.  For the rules both writers apply, the
functions they fire in agree on 4109 (rule, function) pairs within the functions both writers
credit; 346 fire on the C only and 876 on rustc only (77 % agree).  The rest is real coverage:
rustc applies `(R-Range)` in `n_fib`, the C does not.

## The work and rustc-only rules

The inventory's WORK rules (an allocation, a copy or store traffic removed) and its rustc-only
rules, per census name; *applied by* is `bench/portal/rule_phases.tsv`.

| census name | applied by | kind | worth (C) | worth (rustc) | routines |
|---|---|---|--:|--:|--:|
| `R-LeafChain` | native | rustc-only | 0.0 | 60.6 | 94 |
| `R-Leaf` | native | rustc-only | 0.0 | 42.4 | 116 |
| `R-Wrapper` | native | rustc-only | 0.0 | 40.9 | 69 |
| `R-LitDiv` | native | rustc-only | 0.0 | 25.5 | 49 |
| `R-Header/kept` | native | work | 0.0 | 19.7 | 37 |
| `R-Push` | c,native | work | 6.2 | 17.8 | 41 |
| `R-PushFill` | c,native | work | 7.9 | 17.2 | 41 |
| `R-GuardFree` | native | rustc-only | 0.0 | 15.1 | 57 |
| `R-WorkPark` | c | work | 13.8 | 0.0 | 53 |
| `R-TextBorrow` | c,native | work | 13.6 | 13.7 | 28 |
| `R-Header/function` | native | work | 0.0 | 10.7 | 23 |
| `R-Mint` | c,native | work | 7.8 | 8.8 | 28 |
| `R-TextClear` | native | rustc-only | 0.0 | 7.8 | 47 |
| `R-PushRec` | c,native | work | 3.6 | 6.7 | 24 |
| `R-RangedCall` | native | rustc-only | 0.0 | 5.8 | 39 |
| `R-RefillBuffer` | native | rustc-only | 0.0 | 4.4 | 27 |
| `R-KeyedEntry` | c,native | work | 4.1 | 4.2 | 7 |
| `R-Push/sole` | c | work | 3.7 | 0.0 | 13 |
| `R-LazySplit` | c,native | rustc-only | 3.5 | 3.5 | 6 |
| `R-PushFill/record` | native | work | 0.0 | 3.2 | 9 |
| `R-TypedKeyed` | c,native | rustc-only | 2.7 | 2.7 | 6 |
| `R-Header/grown` | c | work | 2.7 | 0.0 | 4 |
| `R-FieldAppend/bare` | c,native | work | 2.6 | 1.5 | 22 |
| `R-Apart/vector` | c,native | work | 2.4 | 0.0 | 5 |
| `R-Header/grown-held` | c | work | 2.4 | 0.0 | 3 |
| `R-Header/kept-growth` | c | work | 2.4 | 0.0 | 3 |
| `R-Header/kept-twin` | c,native | work | 2.4 | 2.4 | 20 |
| `R-TypedKeyed/text` | c,native | rustc-only | 2.0 | 2.0 | 4 |
| `R-TypedKeyed/compound` | c,native | rustc-only | 1.4 | 1.4 | 4 |
| `R-Apart/records` | c,native | work | 1.1 | 0.0 | 2 |
| `R-PushFill/run` | c,native | work | 1.1 | 0.0 | 22 |
| `R-FieldAppend` | c,native | work | 1.1 | 1.1 | 14 |
| `R-FlatCopy` | c,native | work | 1.1 | 1.1 | 6 |
| `R-FaultFlag` | native | rustc-only | 0.0 | 1.0 | 2 |
| `R-PushFill/run-header` | c | work | 1.0 | 0.0 | 18 |
| `R-ElemFirst` | c,native | work | 0.0 | 1.0 | 5 |
| `R-Transient` | c | work | 0.6 | 0.0 | 4 |
| `R-RefillText` | native | rustc-only | 0.0 | 0.6 | 4 |
| `R-TypedRead` | c,native | rustc-only | 0.3 | 0.6 | 2 |
| `R-LoopBuffer` | native | rustc-only | 0.0 | 0.6 | 11 |
| `R-PushRec/apart` | c,native | work | 0.5 | 0.5 | 4 |
| `R-SliceArg/range` | c | work | 0.5 | 0.0 | 2 |
| `R-Destination` | c,native | work | 0.5 | 0.5 | 1 |
| `R-SliceInto` | c | work | 0.5 | 0.0 | 1 |
| `R-Fill` | c,native | work | 0.4 | 0.4 | 2 |
| `R-Fill/guarded` | c,native | work | 0.4 | 0.4 | 1 |
| `R-SliceBorrow` | native | rustc-only | 0.0 | 0.4 | 1 |
| `R-RetAdopt` | c,native | work | 0.3 | 0.3 | 14 |
| `R-AppendTwin` | c,native | work | 0.2 | 0.2 | 2 |
| `R-GroupPush` | native | rustc-only | 0.0 | 0.2 | 4 |
| `R-HandOver/rotate` | c,native | work | 0.1 | 0.1 | 2 |
| `R-Apart/records/boundary` | c | work | 0.1 | 0.0 | 2 |
| `R-LoopRecord/refill` | c,native | work | 0.1 | 0.1 | 1 |
| `R-KeyedEntry/subscript` | c,native | work | 0.1 | 0.1 | 1 |
| `R-StreamWalk` | native | rustc-only | 0.0 | 0.1 | 1 |
| `R-TextBorrow/tested` | c,native | work | 0.1 | 0.1 | 2 |
| `R-AppendTwin/loop-window` | c | work | 0.1 | 0.0 | 1 |
| `R-FnRefBuffer` | c | work | 0.0 | 0.0 | 1 |
| `R-FnRefBuffer/frame` | c | work | 0.0 | 0.0 | 1 |
| `R-FnRefValue` | native | rustc-only | 0.0 | 0.0 | 1 |
| `R-PushFill/hash` | c,native | work | 0.0 | 0.0 | 1 |
| `R-AppendTwin/mint` | c | work | 0.0 | 0.0 | 1 |
| `R-Apart` | c,native | work | 0.0 | 0.0 | 1 |
| `R-LoopRecord` | native | work | 0.0 | 0.0 | 1 |
| `R-SliceInto/transient` | c | work | 0.0 | 0.0 | 1 |
| `R-RebindBuffer` | native | rustc-only | 0.0 | 0.0 | 1 |
| `R-FoldCompare` | native | rustc-only | 0.0 | 0.0 | 0 |

## What it says for B2

- **The heaviest rustc-only rules are not memory work:** `(R-LeafChain)` 60.6, `(R-Leaf)` 42.4,
  `(R-Wrapper)` 40.9, `(R-LitDiv)` 25.5 are call and arithmetic shapes.  They decide PORT or a
  recorded loss (the C lacks them), not MOVE.
- **The heaviest work rules:** `(R-Header)`'s kept clause 19.7 (rustc only), `(R-Push)` 17.8 /
  6.2, `(R-PushFill)` 17.2 / 7.9, `(R-WorkPark)` 13.8 (C only), `(R-TextBorrow)` 13.7,
  `(R-Header)`'s function clause 10.7 (rustc only), `(R-Mint)` 8.8 / 7.8.  B2's probes start
  there, wave 1 first (design.md § The first verdicts).
- **Light everywhere:** `(R-SliceArg)` 0.5, `(R-SliceInto)` 0.5, `(R-Transient)` 0.6,
  `(R-Destination)` 0.5, `(R-SliceBorrow)` 0.4 (rustc, markdown), `(R-RetAdopt)` 0.3,
  `(R-HandOver)` 0.1.  The move protocol's first move (`(R-SliceArg)`'s range clause) was chosen
  for being smallest, and its weight is small too: its probe decides whether it is worth the
  move at all.
- **Admitted where the interpreter spends nothing measurable:** `(R-FoldCompare)` (no executed
  function), the plain `(R-LoopRecord)` and `(R-PushFill)`'s hash clause (one routine each, 0.0).
