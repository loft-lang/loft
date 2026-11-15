<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-15 (evening) — the back end compares equal; the front end is next

Branch `tuxedo-pln204-int32`, pushed.  The arc that ends here: the loft bytecode WRITER
(`selfhost/src/bgen*.loft`) and the loft RUNNER (`selfhost/bench/run_image_run.loft` over
`selfhost/src/state*.loft`, `fill*.loft`) made to work for more of the corpus, with every
intermediate product compared between the loft and the Rust implementation, not only the output.
The next arc is the FRONT END (main-switch phase 1: source in, run).

## Where it stands (pinned suite 38d4877cb)

| Layer | This morning | Now |
|---|---|---|
| writer: functions byte-identical to the Rust writer's | 8884 of 8995, 2 differ | 8997 of 8999, 0 differ |
| writer: whole code stream, starts and tables | 1242 of 1305 | 1305 of 1306 |
| runner on the Rust writer's code (`image`) | 1304 of 1306 | 1306 of 1307 |
| runner on the loft writer's code (`written`) | 1241 of 1306 | 1305 of 1307 |
| runner, every test function as a program (`image-functions`) | 3871 of 3911 | 3900 of 3911 |
| the same on the loft writer's code (`written-functions`) | 3856 of 3911 | 3883 of 3911 (12 unwritten, 7 fault) |

What is left on the program layers: one `store_memory()` report (the store table is the C
runtime's, 1b-fill-port.md difference 3) and one text constant of 256 bytes or more
(`store_handoff_residency.loft`: the writer must lay out the constant store as Rust does — Rust's
records start at 101 there, after the other constants; it belongs to the image-tables step).

Left on the test functions: a native called through a function value (L-FnRef 1658, 2 tests —
the `_dest` branch of `fn_ref_native` needs the buffer's text pushed after the native returns),
a keyed collection inside a reference's display (F-Render-Ref 2268, 5), a `par` worker's view of
its source (tests/scripts/1044), a segfault in tests/scripts/945's JSON builders, and one
package library file run alone (`noimage`, F-Package — `loft test` compiles it in its package).

**A gap in the gates:** `bgen_survey.py` compares the writer's bytes for PLAIN programs only;
the test functions (`--functions`) are run on loft-written code but not byte-compared.  Five
faults occur only there — B-Copy's coalesce arm rebound twice, three B-OutOfLine views across a
nullable field's write, B-Ref-Lvalue 1602 through a function value (a segfault) — so they are
most likely byte differences no gate sees yet.  Give `bgen_survey.py` a `--functions` mode (the
programs `run_image_survey.test_functions` writes) and a baseline layer first; then fix what it
shows.

## The gates (run them; a change is done when they read equal again)

```bash
export LOFT_SUITE_DIR=<a checkout of loft-suite at tests/loft-suite.pin>   # a stale one is refused
python3 selfhost/bench/bgen_survey.py --check selfhost/bench/bgen_survey.baseline.tsv
python3 selfhost/bench/run_image_survey.py [--written] [--functions] --check selfhost/bench/run_image_survey.baseline.tsv
```

`--write-baseline` re-pins one layer; re-pin in the commit whose change moves it.  The format
and the check are `selfhost/bench/baseline.py` (shared; loft-c2 was offered it for
`cgen_survey.py`).  `bgen_survey.py --examples N` names functions per refusal reason.  Times here
at 10 jobs: writer 270 s, image 130 s, written 230 s, image-functions 165 s, written-functions
535 s; on a 4-core GitHub runner image 259 s, written 1071 s.

## Traps met (each cost a run)

- **Never edit `selfhost/src` or `selfhost/crt` while a survey runs**: each survey builds its
  runner or checker from the tree when it STARTS, so a chain of re-pins straddling an edit pins
  a mixed tree.  Stop the chain, edit, start again.
- **The warm cache ignores `LOFT_NO_TEXT_BORROW`**: emit each image twice before comparing by
  hand, as the surveys do (two images from one setting).
- **Concurrent test functions share fixed temp paths**: the survey now gives each job its own
  `TMPDIR`; a probe run by hand does not need it.
- loft syntax met in passing: `parallel` and `loop` are not names (`while true`); `x is ir::TyVoid`
  does not parse (use a `match`); `match` cannot take an `if` as its subject.
- **`loft test`'s coverage report is wrong** about functions a test calls (selfhost/tests/narrow.loft
  lists `slot_of` as never entered while every assert depends on it, and a wrong expected value
  fails it).  Not fixed; worth its own look.

## Shared with other sessions

- loft3 gathers `83a350b34` whole (the Windows fixes for #2502 — `crt_bodies` 4/4 and `selfhost`
  1/1 on windows-latest — the two MSVC fixes, the strand-1b gates).  Later commits go in the next
  gather.
- loft-c2 made the same `deferred_first_bind` clause in `binds.loft` on laptop-c-perf (ours is
  f1feb2e4b; one copy stays at the join) and is adding `c_libraries` to the program image's
  Bundle (agreed).  The narrow slot moved to `selfhost/src/narrow.loft`; el_ex_base delegates.
- Owner decisions loft3 raised: freezing the image format for the switch window; who owns the
  IR passes (main-switch.md puts them in this stream's phase 1, the C plan elsewhere).

## Next: the front end (main-switch phase 1)

0. First the gap above: byte-compare the test functions too, and fix the five written-only
   faults it explains.
1. Measure gate E fresh: `selfhost/front/elab_gate.py --plan` (its last figure, 718 of 1825 at
   6ca9cf280, is old).  Run it with `--jobs 6` under `ulimit -v 6000000` — at its default jobs an
   elaboration runaway took the session down once.
2. Give gate E a committed baseline in `baseline.py`'s format, as the two surveys have, so a family
   ported cannot lose programs elsewhere unnoticed; its intermediate layers are compare.py's (ir,
   types, schema, ctable, ownership, slots, binds).
3. Port the elaborator's families in `--plan`'s order, one family per commit, each checked on the
   gate and on the IR layers.  The scope pass, the variable facts and the IR passes follow (each IR
   pass gated by the image byte-identical before and after, which the writer survey now checks) —
   start the IR passes only once their owner is named.
4. Alongside, the runner's short list above, and the long text constant with the image tables.
