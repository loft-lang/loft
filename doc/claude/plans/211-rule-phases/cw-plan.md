<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `(R-CompleteWrite)` in the IR phase

**Status: designed, not built.**  Arc C's next move ([README.md § Arc C](README.md#arc-c--moving-them)).

## The rule

A record minted by `OpDatabase` (a local) or `OpNewRecord` (an element) is prefilled with its
type's default image, and then the literal group that follows writes every field anyway: the
parser's lowering spells each field (a named value, the declared default, the interned empty
text, the null sentinel, `false`, the variant tag).  Where the group covers every field position,
the prefill writes nothing that survives, and the mint can skip it (`formal/rewrites.md`
`(R-CompleteWrite)`).

## Where it is decided today

| consumer | how | adjustments |
|---|---|---|
| rustc | `generation/mod.rs` calls `opt::ret_adopt::complete_writes` per function; `misc_ops` writes `OpDatabaseNP`, `vector_ops` `OpNewRecordNP`; `ref_ops::fresh_field` writes the null arm | the switch; a `(R-RefillText)` twin's buffer keeps its prefill (`db_vars.remove`); `(R-RefillText)`'s keep needs the no-prefill append |
| C | `FnWriter::complete_writes_once` calls the same analysis; `mint_complete`; `fresh_null_arm` | the switch |
| interpreter | — it prefills every mint and is the oracle | — |

One analysis, computed per writer on the IR it emits, plus a null arm keyed by node address
(`CompleteWrites::fresh_fields`).

## What was measured (S0)

- **The interpreter's gain is small.**  A loop pushing a five-field complete literal
  (`v += [Pt {…}]`): 5 185 → 5 086 instructions a pass with the element prefill skipped, about
  **−2 %**.  So the move is worth doing for harmonisation; the interpreter's speed hardly moves.
- **329 IR reader sites in 97 files** name `OpDatabase` or `OpNewRecord`; 43 of them already name
  the no-prefill spelling too.  Whatever spells the decision in the IR, these readers see it.
- **The null arm has two meanings today.**  A null record copied into a just-created place should
  be absent (`formal/binding.md` (B-Copy), `@C137`).  Two places get two answers, on every
  backend, on the 2026.11.0 release as well:

  ```loft
  struct Hx { h: integer = 4, m: integer? }
  one = [Hx { h: 1, m: 2 }];
  s.es += [E { q: 1, prev: one[5] }];   // a FIELD of a new element: m reads null
  w: vector<Hx> = [one[0], one[5]];     // a vector-literal SLOT:     m reads 0
  ```

  The field arm writes the type's default (`set_default_value`: nullable fields null); the slot
  is a `COPY_FRESH_DEST` copy, whose null source zeroes the bytes (`Stores::copy_null_into`), so
  `m` reads `0`, not `null`.  This is a silent-wrong answer.

## The design

**One decision, spelled in the IR, read by every backend:** a pass `opt::complete_write` in
`close_program`, ahead of `opt::loop_record`, rewrites each covered mint to its no-prefill twin
and marks each fresh-field copy as a copy into a fresh destination.

1. **Twin ops, not an operand bit.**  `OpDatabaseNP(pos, db_tp)` and
   `OpNewRecordNP(data, parent_tp, fld)` become real operators in `default/01_code.loft`.  The
   interpreter runs the mint without `set_default_value`; the writers already have runtime
   functions with these names (`codegen_runtime.rs`) and the C's spelling.  design.md proposed a
   bit on the type operand instead.  The twin is safer: a reader that misses it treats the call
   as an op it does not know and declines, and the census gate names that drop.  A reader that
   misses a bit reads `tp | 0x8000` as a type number, which describes the wrong layout without a
   word.
2. **One null arm.**  A null copied into a just-created destination writes the type's default,
   whatever made the destination (`Stores::copy_null_into` writes `set_default_value`, not
   zeros).  That fixes the slot answer above.  The pass then flags each fresh-field copy
   `COPY_FRESH_DEST` (it IS a copy into a just-created place), and the writers' address-keyed
   `fresh_fields` arm goes away.
3. **Ownership.**  The buffer a `(R-RefillText)` twin refills keeps its prefill: a refill twin
   reads every text slot it holds.  That twin is rustc's, so the IR cannot see it.  The pass asks
   `opt::buffer_owner` (`@FR-R-RefillText` becomes an owner there), as `(R-LoopBuffer)` and
   `(R-LoopRecord)` do, and leaves an owned buffer's mint as it is.

## The steps

| # | step | done when |
|---|---|---|
| S1 | **The null arm made one.**  `copy_null_into` writes the default on every backend (`codegen_runtime`, the C runtime); claim and fix the issue (main has it) | the slot and the field answer `null` for `m` on all three backends; a cell for both shapes, falsified by restoring the zero fill |
| S2 | **The twin ops.**  `OpDatabaseNP` and `OpNewRecordNP` in the stdlib, in the interpreter's three operator tables, and in both writers as plain emitters; `make surface-gen` | a cell minting through each twin answers the hand-computed values on the interpreter, the C and rustc |
| S3 | **The pass, beside** (`LOFT_IR_COMPLETE_WRITE=1`): `opt::complete_write` rewrites the covered mints and flags the fresh-field copies, leaving the owned buffers | rustc's and the C's emission for each bench program the same with the pass on (and the writers' clause reading the twins) as with the writers' clause alone; the interpreter's probe saves its prefill |
| S4 | **The move.**  Default-on; both writers' `complete_writes` calls, `fresh_field`, `fresh_null_arm` and the switch handling deleted; the readers that name only `OpDatabase` or `OpNewRecord` checked against the census | `rule_tags.py sites @FR-R-CompleteWrite` shows no writer clause; the census applied by `ir`; switch_ab, c_sweep (plain, `--strict-stores`, `--poison`, claim-unwritten), the rule's cells on three backends |

## Risks

- **A reader of the twin that should not decline.**  A reader that decides SAFETY rather than
  an optimisation (a free, an owner test) and names only `OpDatabase` would treat the twin's
  local as unminted.  S4 lists every such reader before the default flips.
- **A first-pass prefill that someone reads.**  `(R-ElemFirst)` says "every field is written
  before the finish — by the prelude mint's prefill, a paired build, or the kept sets".  A
  pair whose mint the pass makes no-prefill must have no field left to the prefill; the pass
  reads the same coverage, so it holds — S3's emission comparison is the check.
