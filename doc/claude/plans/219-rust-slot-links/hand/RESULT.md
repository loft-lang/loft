<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN219 — the C hand forms of the routines over 3× Rust

Tip `94f610006`; macOS on Apple silicon, Apple clang, `-std=c99 -O3` (the `c` lane's flags,
`bench/portal/hand_price_c.sh`); the `_nt` column adds `-DLOFT_NO_THREADS`.  Each routine has
its own driver: the bench with `main` cut to that routine, on both sides
(`<routine>/driver-bench.loft.txt`, `driver-bench.rs.txt`).  `m.sh.txt`: the median of 7 runs, `--n`
calibrated to about 300 ms a run, not pinned.  Every form prints the routine's hash, which the
Rust twin and an independent Python run of the algorithm also print.  The leak check was not
armed on the C side: a form's hash is right, and its release of what it no longer holds rests
on the reading in § Mechanisms of the plan's README.

Each `<routine>/` keeps `FORMS.txt` (the commit and how to rebuild), `sN.c.diff` (the unit after
step N, cumulative, against the emitted base with the driver's path written `"DRIVER.loft"`),
`stepN.py.txt` (the edit as a script) and the drivers.  `bash compare.sh.txt <routine>` re-emits the base
from the current tree, restores every form onto it and measures base, forms and the Rust twin.

## Where each routine lands

| routine | Rust ns/op | base | base `_nt` | best form | form `_nt` | form |
|---|--:|--:|--:|--:|--:|---|
| tree_delete | 67,608 | 3.65× | 3.18× | **0.65×** | 0.57× | s2 |
| list_ops | 79,493 | 4.10× | 3.36× | **0.57×** | 0.57× | s2 |
| lookup_copy | 144,856 | 1405× | 1117× | **1.24×** | 1.22× | s1 |
| enum_list | 59,400 | 6798× | 5539× | **0.58×** | 0.56× | s2 |
| build_rec | 94,007 | 3.14× | 2.68× | **1.93×** | 1.76× | s3 |
| expr_build | 46,109 | 10.4× | 8.73× | **1.64×** | 1.43× | s1 |
| expr_simplify | 35,246 | 13.4× | 11.6× | **1.17×** | 0.93× | s1 |
| lru_mixed | 193,677 | 3.50× | 3.51× | 3.16× | **2.95×** | s2 |

## The steps

### tree_delete

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted | 255,472 | 3.76 |
| 1 | the unlink MOVES (`loft_move_child_rec`): `cur = cur.left` in `remove_key` and `slot = slot.right` in `take_min` install the child's record number and release only the replaced node | 137,958 | 2.03 |
| 2 | the leaf literal of `insert` (`cur = Tree { … }` through a field link) is built in the field's own child record (`loft_new_child_rec`), not in a fresh store copied in and freed | 47,227 | 0.68 |

### list_ops

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted | 315,873 | 4.02 |
| 1 | the three read-only cursors (`cur: Node? = h`, `cur = r`, `p: Node? = h`) are views: no `loft_bind_copy` of the list, no owner witness | 179,339 | 2.28 |
| 2 | each `(R-Demote)` is one claim, one 16-byte copy and the field write (`loft_demote`), not a temporary record, two `loft_move_field`s and `loft_free_record_in` | 45,018 | 0.57 |

### lookup_copy

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted: `cur: Tree? = t` copies the 2,000-node tree per lookup | 195,290,500 | 1349 |
| 1 | the cursor is a view of the parameter: no copy, no witness | 172,579 | 1.19 |

### enum_list

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted | 387,667,000 | ~6400 |
| 1 | `l = Cons { v, next: l }` and `r = Cons { v, next: r }` demote: the old record moves into one child of the same store; the tag and `v` are written into the destination's record (no work store, no copy of the list in and back) | 193,833,500 | — (the walk still copies) |
| 2 | the walks are views: `cur = l`, the `match` payload `next` and `cur = next` copy nothing | 33,787 | 0.56 |

### build_rec

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted | 300,324 | 3.22 |
| 1 | `t.left = insert_rec(t.left, k)` hands the callee a buffer naming t's STORE; the leaf is claimed there and adopted by number (no store per insert) | 242,530 | 2.60 |
| 2 | pricing only: the self-call skips the frame wrapper — no change; the per-call frame is not the cost | 243,252 | 2.61 |
| — | `-DLOFT_NO_THREADS` on step 1 (profile: `_tlv_get_addr` 13 % — every store access reads the thread-local store table) | 208,529 | 2.23 |
| 3 | the argument written back is recognised inline (one load and compare) instead of inside the out-of-line `loft_set_child_rec` | 156,856 (`_nt`) | 1.65 |

### expr_build

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted (`_nt`): a store per node, each child subtree copied into its parent's store and its store freed | 405,823 | 9.13 |
| 1 | each node claimed in the store its caller names; children built in the node's store and adopted by number (`_nt`) | 66,445 | 1.50 |

### expr_simplify

| step | edit | ns/op | × Rust |
|---|---|--:|--:|
| 0 | as emitted (`_nt`) | 406,842 | 11.7 |
| 1 | one destination store for the recursion: results claimed where the caller names, a returned child handed back as it is, a new node adopting its children by number, a dropped child released record by record (`_nt`) | 32,712 | 0.94 |

### lru_mixed

| step | edit | ns/op `_nt` | × Rust |
|---|---|--:|--:|
| 0 | as emitted | 652,903 | 3.38 |
| 1 | `c.nodes[old] = null` removes in ONE probe (`loft_h_take_i64`) — `remove(find(k))` found the record by key and probed again by its digest | 576,987 | 2.98 |
| 2 | `m = c.nodes[k]` right after `c.nodes += [Node { key: k, … }]` reads the record the insert filed | 544,931 | 2.82 |
| 3 | runtime only: the reference accessors and their bit-set calls hot-inline — 2.5 %, not kept | 521,256 | 2.70 |

Profile of step 2 without inlining (sample, 4 s): `loft_h_find_i64` 12 %, `loft_h_take_i64` 12 %,
`loft_sip_le` 10 % (SipHash, which the twin's `HashMap` pays too), `loft_h_probe_for_insert` 8 %,
`loft_set_dbref` + `loft_bits_insert` 9 % (every reference write marks its target for
`(H-Removed)`), `loft_get_dbref` + `loft_record_removed` + `loft_bits_contains` 8 %.  Not at 2×:
§ The rewrites, lru, of the plan's README.
