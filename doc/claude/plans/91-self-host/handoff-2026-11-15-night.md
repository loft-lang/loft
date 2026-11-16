<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-15 (night) — every layer gated; the front end is the work

Branch `tuxedo-pln204-int32`.  Goal (owner, /goal): make the full loft interpreter work, the
in-between data of the two implementations compared at every layer.  The pipeline is source →
front end (`lex`, `syntax`, `el*` = elaboration) → scope pass and IR passes (NOT ported:
`scope.loft` is a first slice) → bytecode writer (`bgen*`) → runner (`state*`, `fill*`).  The
writer and the runner agree with Rust almost everywhere; the front end is where the work is.

## Where it stands (suite pin 38d4877cb)

| Layer | Gate | Position |
|---|---|---|
| writer, plain programs: functions / streams | `bgen_survey.py --check` | 9007 of 9009 / 1307 of 1308 (2 long text constants) |
| writer, test functions: functions / streams | `bgen_survey.py --functions --check` (new) | 5559 of 5559 / 2562 of 2603 (41: natives of `--lib tests/lib` packages) |
| runner, loft-written code: programs / test functions | `run_image_survey.py --written [--functions] --check` | 1306 of 1308 / 3900 of 3912 |
| front end (gate E) | `selfhost/front/elab_gate.py --check elab_gate.baseline.tsv` (new) | 725 of 2096 agree, 1369 refused, **2 differ** |

`--check` fails on any `differ` (a wrong image), listed or not, so gate E's check is RED until
the two differences below are fixed.  Run each gate with `LOFT_SUITE_DIR` at a checkout of
`tests/loft-suite.pin` (the shared `../loft-suite` lags it); gate E with `--jobs 8` under
`ulimit -v 6000000`.  For one program, a prebuilt elaborator saves the 8-minute build:
`elab_gate.one(loft, defs, elab, std, path, tmp, env, i)` (the night's scratch driver did that).

## Open, in order

1. **Work in progress, uncommitted in the tree** (also saved as
   `~/workspace/.pln91-wip/literal-exits-into-buffer.patch`): the `__retbuf` half of
   `Parser::literal_exits_into_buffer` (`control.rs:4014`) — `record_return` keeps a fresh literal
   exit in its own work ref, `el_expressions::literal_exits_into_buffer` rebuilds the exits into
   the buffer only when EVERY exit is a fresh literal, a chain over the buffer or a same-type
   by-value parameter.  Cures gate E's differ on
   `tests/scripts/a-small-record-built-into-a-work-local-and-returned-is-read-as-its-fields.loft`.
   Not measured yet: build the elaborator, gate that file, then the full gate E against
   `elab_gate.baseline.tsv` (`LOFT_ELAB_VERDICTS` and a diff of the verdict files: every moved
   line is a program to read).  The chain-renamed buffer's half (`chain_return_buffer_var`) still
   builds per exit in `record_return`.
2. **Gate E differ, N-Join 2478**: `n_nested_below`, `(maybe_below(k) ?? one(7)).n` with
   `maybe_below` declared below — Rust's first pass does not know its type and makes `__ref_5`
   where the loft elaborator makes `__ncc_1`.  A pass-order port.
3. **Gate E families**, in `elab_gate.py --plan` order.  The operator family is mostly done: what
   remains of it is `== on TyReference, TyInteger` / `TyText` (short lambdas and generics), a
   stored `__tuple<…>` comparison and an absent tuple (refused by name until
   `unbox_tuple_from_dbref` and `coalesce_not_null` are ported).
4. **Writer**: lay the constant store out as Rust does (a text constant of 256 bytes or more);
   the natives of a `--lib` package's compiled bodies (`native_table.loft` holds none).
5. **Runner**: the 12 test functions left (`a format of a keyed collection` 5, a native through
   a function value 2, …: `run_image_survey.py --written --functions --show stopped`).
6. Then the scope pass and the IR passes, each gated by the image before and after.

## Ported tonight (the method: read Rust's function, transcribe, gate)

- Writer (`bgen.loft`, `binds.loft`): the first bind joining on a choice (`first_bind_choice_join`
  → `FirstArm::CallJoin`; `choice_leaves` reads an `ncc` block's tail past its frees), a tagged
  nullable slot in the projection walk (`tag_read_payload`), `OpCallRefStore` with
  `store_text_mask`, a narrow tuple member through `OpSetInt4`.
- Front end: `el_compare.loft` (`tuple_compare`), `builtin_op`'s null tests (a `&` link peeled,
  tuples, a variant's `OpRefIsNull`), one range guard around a narrowing return dropped.
- Compiler: the value-record twin crashed the interpreter on a recursive nullable small record
  bound in a loop (83aad7478, in PR #2513).

## Traps met

- **One rule, one home**: content equality already lives in `el_ex_null::builtin_op`; a copy in
  `call_op` was a second home.  Grep the loft port for the Rust name AND for the IR op
  (`OpEqContent`) before porting.
- **`use` order changes a parse**: importing `el_compare` (which uses `el_ex_null`) from
  `el_ex_base` made `el_ex_null.loft` fail on `a.tp is TyNull { b } else { a }`.  Not reduced yet
  — a language defect candidate.  `el_ex_base` imports no expression file; keep it so.
- **Two loft `--native-c` builds at once** while cargo rebuilt the runtime gave "the compiled
  standard-library bodies were not written" and a link error; one build at a time.
- A survey's test program path is a text constant of it: keep names short (now hashed), or a
  long TMPDIR measures the long-constant layout instead of the test.
- A `for r, n in …` loop in a script leaks `n` (gate E's baseline header read `of 2`).
