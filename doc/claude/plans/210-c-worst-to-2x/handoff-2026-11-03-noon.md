<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN210 handoff — 2026-11-03, noon

Supersedes [handoff-2026-11-03.md](handoff-2026-11-03.md) (the morning one).

## ⚠ First

1. **The release ruling stands** (owner, via loft3): `tuxedo-release-2026-11` takes nothing more
   from `laptop-c-perf` but a FIX to code already on it.  Everything here waits for the release to
   merge, then goes to main — the `mesh_to_floats` note below included (loft3: the release run
   is frozen to fixes for its own reds).
2. **Every routine is under 2.7× on both compilers** (the full table below).  #2379, #2380,
   #2381 and #1743 carry `Fixes` on this branch.  What is left is the 2× aim: the routines at
   2.3–2.7× are listed below, each with what the profile says.
3. **A copied loft binary must live in `target/release/`**, beside `libloft.rlib`: a copy
   elsewhere fails the interpreter side of every program that uses a native library, which the
   sweep reports as `differ` and a table as a missing lane (C_BACKEND.md § the sweep).

## Where things are

- Worktree `/home/jurjen/workspace/loft-c-perf/target-exp/wt-rebase`, local branch
  `rebase-c-perf`, remote `laptop-c-perf`, pushed fast-forward (no `merge -s ours` needed this
  time: the remote tip was an ancestor).
- Built since the morning (each with guard cells falsified by plants, both sweeps against the
  pinned suite clean, c_emit 71/71, both clippy variants, `after.py fix` green after its fixes):
  - **P16** `(R-Peel)`'s exit clause (`c::body::peel`): a `continue`/`break` of the peeled loop
    ends its copy / leaves after the last; an exit of an enclosing loop names that loop.  A body
    holding an INNER loop still refuses — the first build admitted it (an inner loop's iterator
    `break` had refused it before) and copied inner loops whole; c_emit's held-header count
    caught it.
  - **P18** on the C, `(R-RecPtr)`'s text clause (`FnWriter::rec_ptr_text`, `loft_pa_text`) and
    the `for` iterator's `{ i = i + 1; v[i] }` in the held-element bind.
  - **P22** `(R-TextBorrow)`'s local-root clause (`opt::store_text_borrow::local_root`, C only via
    `local_roots`): a `??` temp over a frame local's text borrows where its scope has no store
    writer at all and does not rebind the local.  Falsified: each of its two defences (this
    scope check, the writer's later-writes copy in `arg_with`) is sufficient alone; both removed,
    d4 fails plain, poison and ASan.
- **P17** deferred: once P16 is in, clang inlines `wall_chain_walk`'s chain itself.
- **P20** priced on `parse` after P18: −1.1 % instructions clang, −3.1 % gcc — built, kept out
  (`target-exp/p16/p20.patch` in the worktree, ignored); its guard draft is in this session's
  scratchpad only.  Worth building for a routine where the format is the cost.
- **P19, P21** not started (designs in [last-three.md](last-three.md)).
- Carried from the release branch: c20b424bc (R-InRange's parameter alias), 692174cd7,
  55086b975, 061a588fa; `index_recptr`'s pins now hold `(R-InRange)` off as heap_discharge's do.
- `scripts/after.py` works in a worktree now (its state lives in the real git dir).

## The numbers (C ship lane against the Rust twin, 126 routines, full table v7)

`python3 bench/stats.py --no-suite $(cat target-exp/pkgs-now.txt) --lanes rust,cship --cc gcc|clang
--loft target/release/loft-table --no-lean-build`, every hash agreeing; outputs
`target-exp/table/v7-{gcc,clang}.txt`, compared with the 2026-11-02 evening tables by
`target-exp/p16/cmp.py`.

| | median | p95 | worst | over 2.7× |
|---|---|---|---|---|
| gcc | 1.39× | 2.36× | 2.68× pluginabi `req_state_b64` | 0 |
| clang | 1.42× | 2.50× | 2.63× pluginabi `check_request` | 0 |

The three this session was asked about:

| routine | gcc | clang |
|---|---|---|
| hex_shape `wall_chain_walk` | 2.11 → **1.77** | 3.02 → **1.48** |
| arguments `parse` | 2.07 → **2.03** | 3.14 → **2.51** |
| hex_recover `forms_upto` | 2.70 → **2.29** | 1.65 → **1.41** |

Worst left: gcc — `req_state_b64` 2.68, zttext `flow_layout_full` 2.65, hex_roof `roof_match`
2.65, cbor `decode` 2.60, gridmesh `build_index` 2.57, hex_field `edgeset_count` 2.57; clang —
`check_request` 2.63, graphics `fill_triangle` 2.60 / `fill_polygon` 2.59, drawing
`render_marks` 2.56, `flow_layout_full` 2.56, `parse` 2.51, random `get` 2.50.

**One slowdown, explained, not from today's rewrites:** mesh3d `mesh_to_floats` on clang
1.25× → 1.59× (gcc 1.05× → 0.87×).  None of today's switches moves its instructions;
`LOFT_NO_RECPTR_HELD=1` restores it.  Hand A/B on its C: the VERTEX bind
`mf_v = mf.vertices[mf_idx]` (the morning batch's held-element clause, on the release) takes its
address with a second index test, so clang cannot tie the reference's null test to the
address's and keeps the six `loft_pa_f64` null tests: 394 µs held against 313 µs through
`loft_rec_ptr` on clang, 240 against 269 µs on gcc (held wins there).  Deriving the address
from the bind's own reference (`v.rec ? h.base + (v.pos - 8) : 0`) gives 360 µs on clang,
244 µs on gcc — half the loss back.  A trade between compilers, under the 2× aim; the next
step is that derivation plus a look at what clang still keeps.

## Next

1. The 2× aim, by what each profile names: `parse` on clang — what is left is spread
   (`classify` 20 %, `find_long_opt` 16 %, memcmp 12 %, `find_short_opt` 10 % — the string
   record resolved twice per option); P19 (tokens borrowing argv, [last-three.md](last-three.md))
   is the next real lever.  `forms_upto` on gcc — P21 (the candidate `ef` built outside the
   store).  The 2.5–2.7× rows above have no design yet: profile each, hand-price first.
2. `mesh_to_floats`' held vertex bind (above) — the derivation from the reference is the
   candidate.  Settled with loft3: it does NOT go to the release (the release run is frozen to
   fixes for its own reds; this is a measured trade with right values); it goes in the
   post-release batch for main.
3. #2351 (facts S7, D-fact-2, D-fact-3), then @PLN211.

## Traps met

- **A copied binary outside `target/release/`** (⚠ 3 above): four false `differ` rows per
  sweep mode, all programs using the `script` library.
- **The validation chain builds from the working tree**: an edit made while it runs lands in
  the binary it validates.  Keep new work out of the tree (a patch file) until it finishes.
- **A guard cell must hold the borrowed value ACROSS the write**: `(v[0] ?? "") + grow(v)`
  consumed the view before `grow` ran, so a plant admitting the borrow passed; `both(v[0] ??
  "", grow(v))` holds it across.
- **A pin that counts emitted sites goes stale when a better clause takes them** (`index_recptr`,
  `heap_discharge`): hold the other rule off in the pin, never re-count to the new emission.
