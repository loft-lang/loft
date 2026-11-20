<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 217 — One source, two widths

Tracker: [@PLN217](https://github.com/loft-lang/plans/issues/217) — the goal, the invariant and
the seven phases are in the issue.  This file is where the work stands and what is left.
The reference is [WIDTH32.md](../WIDTH32.md).

## Status

Active.  S1 to S3 and S5 are in the tree; S4 is under way, family by family; S6 and S7 are open.

| phase | state |
|---|---|
| S1 explicit primitives, S2 the float pairs spelled apart, S3 the standard library under the regime | shipped (loft#2418) |
| S4 the runtime at the build's width | the width header leads the runtime; the families below are moved; the rest is the ledger's `runtime` list |
| S5 32-bit boundaries in the C backend | shipped: a parameter, a result, a tuple member and a function type are `int32_t` in a 32-bit build (`Cx::ctype`) |
| the counters (found on the way) | a loop's, a pattern match's and a slice's counters take the file's width (`Parser::count_type`, `(W-Count)`) |
| S6 libraries and `--native-esp` | `--native-esp` builds 32-bit by default; open: the three `pln217-width32-tanmatsu` library branches |
| S7 the gate | the wide ledger is held to a baseline, and a 32-bit `--native-esp` build refuses 64-bit helper calls without an exception (`src/esp_width64.rs`); the runtime's fixed widths are held to a baseline (`tests/width32.rs`) |

## The owner's rulings (2026-11-20)

- The runtime is ONE implementation compiled at either width, through typedefs and macros
  (`src/c/loft_width.h`), not a second runtime.
- The counters the compiler makes move to the file's width too: leaving them 64-bit was an
  oversight.
- The gate REFUSES, with specific exceptions, each with its reason: "we cannot in good faith
  declare that we have a Tanmatsu backend when it suddenly behaves poorly in places."  So no
  blanket exception for runtime code that is only not converted yet.
- The compiler knows what it writes: the test of the program's own code is the wide ledger,
  not a guess from the linked image.

## The instruments

- **The wide ledger** — `LOFT_WIDE_LEDGER=<file>` per unit, `make wide-ledger` over the corpus
  against `scripts/wide_ledger.baseline` (WIDTH32.md § The wide ledger).  Its `runtime` lines
  are the S4 worklist by use; its `undeclared` lines are compiler-made values still wide.
- **The truncation detector** — a 32-bit unit compiled by clang with `-Wshorten-64-to-32`
  (and its default pointer-type warning): each report in the program part is a 64-bit value
  handed to a 32-bit parameter without a conversion, where a null would arrive as 0, or the
  address of a 32-bit value handed where 64 bits are written.  Sixteen findings stand over a
  quarter of the corpus and predate this plan: `struct timeval` built from a 64-bit
  millisecond count, a `par` tuple member read as a short, a file cursor's decode.
- **The sweeps** — `LOFT_WIDTH=32 scripts/c_sweep.py` (C against the interpreter, every macro
  mode: strict C refuses a pointer of the wrong width) and the same without the switch.
- **`tests/width32.rs`** — the hand-computed cells, both backends, both widths.

## Moved to the build's width

| what | how |
|---|---|
| a vector's length, the held header's `len`, every `loft_vh_*` index | `loft_int` parameters; the emitter converts through `as_wint` |
| a text's length and size | `loft_int` results |
| a decimal print, a 4-byte field write, a 4-byte push, an element read, integer to single | a `…32` form beside the 64-bit one, picked by the value's kind |
| a removal inside a vector walk | `loft_vec_walk_remove32`: the index's address at its own width |

Two faults this work made and caught, each with its cell: a non-negative claim made about an
index after narrowing (`index_arg`), and a walk-removal writing 64 bits through the address of
a 32-bit counter.  Both are the same lesson: a fact proven about a 64-bit value, and an address
taken of one, do not survive its narrowing.

## Left, in order

1. **The runtime families the ledger lists** — `loft_format_float` (a single is printed through
   a `double`: a 32-bit form needs a single-precision shortest-digits writer), the slice and
   text-position helpers (`loft_vec_slice_view`, `loft_text_sub`, `loft_text_char`), the narrow
   field helpers (`loft_range_default_at`, `loft_byte_raw`, `loft_get_byte`), `loft_array_get`.
   `loft_kv_int` is a keyed collection's key, 64 bits in the table's own format.
2. **`(x as integer)` in a regime file answers 64 bits** (`OpCastIntFromSingle` through
   `loft_f2i_at`, which takes a `double`): the cast's type should be the file's `integer`
   (`(W-Name)`), and the conversion a single-to-`i32` one.
3. **The undeclared values left** — staging, discharge and lift temporaries (`__stg_p`,
   `__dc_i`, `__lift`, `__ref`), `text_len.rs`'s `_len`.
4. **The `value` and `op` classes judged** — the ledger does not yet check that an operand of
   such a conversion is a `declared` wide value; until it does a compiler-made 64-bit
   computation can stand in them (a negated literal is one: `LOFT_PLAIN_NEG(INT64_C(1))`).
5. **The runtime's fixed widths to zero** — `tests/width32.rs` holds the lines that name
   a plain `int64_t` or `double` to a baseline, 1152 over 32 headers (`loft_rt.h` 422); each
   is to become `loft_int` / `loft_float`, or `loft_word` / `loft_fword` where the value is
   64 bits in every build.
6. **`tile-world-game` as a 32-bit app** — measured on a scratch copy of its tree (`cd9ee29`
   plus its 24 local commits) with this branch's loft: `loft --native-esp --width 64` builds it
   as before (5.8 MB), and the 32-bit default stops at 37 errors — 21 in the library copies
   its `libs/` holds (graphics 0.13.0, gpu 0.3.0, audio 0.2.3, scene3d 0.6.0; graphics 0.13.4
   and audio 0.2.4 are the releases written for the regime) and the rest in the game itself:
   `Timing.since` holds `ticks()`, an `i64`; colour and bit literals past 32 bits; `f64`
   arguments where a library takes a `single`.  The game is its own agent's to change
   (`218-tanmatsu-stage-engine/kit/tile-world-game-width32.diff` was the report); the failing
   build ends with the note that `--width 64` builds the program as written.  The image gate
   has therefore not seen the game, only a small reference app.
   The libraries' own TEST files are not regime-clean either: under `LOFT_WIDTH=32` the suites
   of gpu, scene3d, graphics, gridmesh, tiles and imaging stop on literals past 32 bits, with
   this loft and with the installed one.
7. **On the device** — the demo world from a 32-bit build, measured against 60 frames.

## Not this plan's

`float` as `single` in the 64-bit build too (the issue's open question): left as `f64`.
