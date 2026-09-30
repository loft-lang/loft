<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 180 — Small records as tuples, decided in the IR phase

Tracker: [@PLN180](https://github.com/loft-lang/plans/issues/180).

## Status

Active on `laptop-superinstructions` (`src/value_record.rs`).  **Built:** slice 1 (tuple
return), slice 3 (tuple parameters), slice 4 (one tuple-returning function serves every
caller: forwards, tuple TWINS for the sites that owe a record, a tuple copied into a place)
and slice 5 (nested layouts).  `resolve_move` returns and receives tuples on both backends.
**Open:** slice 2 (a written tuple local, `TuplePut`).

The bar the interpreter work answers to is in PERFORMANCE.md § Why the interpreter is
optimised at all: native is the target, a cliff is about 100× (optimised interpreter against
optimised native, real time), and no change may make native or any routine's ratio worse.

Building slice 1 also found and fixed a `--native` defect in the existing rewrite.
Native built the tuple of a value-returned literal in FIELD order, so a literal whose
fields are written out of schema order ran its field expressions reordered, and nothing
reported it.  The fix is in `value_record_parts` and its emitter.

## Goal

A small record a function returns, and a local bound from such a call, are carried as
tuples on the interpreter as well as `--native`.  The interpreter then claims no store for
them, just as native already does not.

## Why

`R-ValueRecord` and `R-ValueLocal` (formal/rewrites.md) are native-only today.  The IR both
backends read still binds `p:ref(V3) = n_v3(…, __ref_1)`.  On the interpreter that claims a
buffer store per call and frees it through `OpFreeRefIfDistinct`.  The store census
(`make interp-gap`, `LOFT_STORE_CENSUS`) counts per routine op, interpreter against native:

| routine | interpreter stores | native stores |
|---|--:|--:|
| resolve_move | 146,756 | 0 |
| slope_path_with_undo | 4,227 | 1 |
| panel_build | 46k | 10k |

A hand-written probe computed the same hash in every cell: records took 29.8 ms and 60,000
stores on the interpreter, tuples 13.6 ms and 0.

## Measured (lane 17, `make interp-gap --only 17`, pass off → on, 2026-09-30)

| routine | interpreter stores/op | interpreter time/op | native time/op | interp/native |
|---|--:|--:|--:|--:|
| resolve_move | 146,756 → 35,460 | 150.4 → 110.3 ms | 2.23 → 2.25 ms | 67.6× → 49.0× |
| panel_build | 46,336 → 10,336 (native 10,003) | 50.4 → 30.9 ms | 10.25 → 10.20 ms | 4.9× → 3.0× |
| emit_to_material | unchanged | 22.6 → 13.1 ms | 1.34 → 1.33 ms | 16.8× → 9.9× |
| slope_path_with_undo | 4,227 (native 1) | unchanged | unchanged | 38× |

Native time is unchanged within noise on every routine.  `resolve_move`'s remaining stores
are in `map_get_hex` (a record VIEW into the map: native's view-leaf rule, @PLN164 C5) and
`floor_y_at`; `slope_path_with_undo`'s are in functions this pass declines.  Both are the
next candidates.

## Kernels — a stop-gap, not the method

Native avoids a loop's per-element cost; the interpreter, running the same loop element by
element, was 200–360× slower on some routines.  A KERNEL (one Rust body both backends call as an
ordinary function, never an opcode) closes such a gap for one name.  Kernels are a stop-gap and
must never dominate: the preferred fix makes the LOFT code fast, because that fixes every loop
of the same shape, users' own included, and optimisation work here slightly favours it (owner,
2026-09-30).  Every kernel, why it exists and when it goes, is in the register,
[KERNELS.md](../KERNELS.md); `make kernel-ratio` measures each against its loft body on both
backends.  Today: `vector_sum_int` (the `sum` reduction), `split`, and `text_lines` under
`File.lines()`.

**Where the work comes from: the @PLN179 script ports.** @PLN179 ports the project's own
scripts to loft, each a twin run beside its original (`make script-twin`), and scores every port
on performance against the original ([SCOREBOARD.md](179-scripts-in-loft/SCOREBOARD.md)).  Those
ports are REAL interpreter workloads (a script runs interpreted by default, MODES.md), so they
are the corpus this plan optimises alongside the bench routines.  `LOFT_PROFILE=1` on a slow
port names the loop; the finding in [REASONS.md](179-scripts-in-loft/REASONS.md) records it.
The first question is then what makes that LOFT code slow on the interpreter; a kernel follows
only on the register's admission list.  Finding 013 was answered with two kernels before this
rule was written down, and both carry a removal trigger.

Measured on the two ports finding 013 names (interpreted, this box, same bytes out as the
original on every run; Python is the original):

| port | Python | interp before | interp after | what remains (`LOFT_PROFILE=1`) |
|---|--:|--:|--:|---|
| `tests/dump_ignored_tests` (584k lines through `lines()`) | 0.22 s | 3.67 s | 0.62 s | — |
| `scripts/opl_points` (200k OPL lines through `split`) | 0.31 s | 4.13 s | 1.79 s | the per-field loop in `main` (62 %, `x = field[1..]` its top line) and the hand-written `%hex%` unescape (30 %) |

The kernels' exit is the standard library compiled like any other library (C71), after which
an interpreted script runs a kernel's compiled loft body and the Rust version goes as soon as
that is viable ([KERNELS.md § Open](../KERNELS.md#open)); `split` and `text_lines` are already
within 1.5× compiled.  Until then, next, in that order: the interpreter's per-character text
walk and the slice appended to a vector (it is `opl_points`' own remaining cost, every script's text loop, and the trigger that
retires the `split` and `text_lines` kernels); then a reduction the interpreter runs as one
operation whatever its spelling (retires `vector_sum_int`; a program's `for x in v { r = r + x
}` still pays over 150×); then whatever the next slow port names.

## Design

**One admission.** The candidates are exactly what `generation::hoist::value_records`
admits, so the rule keeps one definition.  This pass narrows them to the shapes it can
rewrite.  Native's own pass then finds only the functions this pass declined, so the two
never apply to the same function.  Deleting native's pass is the end state, and it becomes
possible only once every shape it covers has moved here.

**Where it runs.** At the top of `compile::byte_code_from`, not in `scopes::check`, because
the rewrite changes SIGNATURES.  `check` runs once per source load, and a file loaded later
would type a call by the rewritten signature (`p.x` on a tuple does not parse).  A `Data`
that is parsed against again after compiling is marked `Data::open_world` and keeps
records.  That covers the REPL, the debugger (which runs through it), live reload, the
browser debugger panel and a host that calls functions by name.  A warm-cache start also
keeps records, because it reads bodies from the cached store.

**One tuple order per record type: schema order.** Tuples flow from returns into
parameters, so every source of one record must agree.  A literal that writes its fields in
another order binds each value to a temporary first, in the order it wrote them, and builds
the tuple from those — so a field expression with an effect runs where the program put it
(the native defect slice 1 found was exactly this reordering).

**A carrier** is a local bound from an admitted call or an admitted tuple parameter.  It may
be read field-wise (through a path into an inline sub-record too), handed whole to an
admitted parameter of its record, returned by a forward, copied into a place (as field
writes), freed, and — a local — null-initialised (the null tuple, which is what a field read
of a null record answers).

**Twins, not materialisation.** A local that cannot carry the tuple, an element or field the
result is built into, a forward whose function keeps its record: such a site OWES a record.
The function keeps its record form for exactly those sites, and a TWIN (`<name>_tuple`)
returning the tuple serves the carriers — so a site that owes a record runs the code it ran
before, on both backends.  Twinning grows to a fixpoint: a twinned forward keeps its record,
so the function it forwards is owed there too.  The first cut MATERIALISED the tuple at such
a site instead (the callee's mint test run on the caller's buffer, then the field sets).  It
was a use-after-free under the loop-pooled buffer protocol (`copy_in_place` under
`LOFT_STRICT_STORES`), and it hid the builder call `--native`'s own rewrites place directly
into an element (six emission-shape suites went red).

**Literals as parts.** A literal is its opening test, its field FILLS (a setter on the buffer
or on a path into its sub-records, or a copy of a record into such a place, filling every
field of its extent), frees that only witness the buffer (free unconditionally: a tuple is
never the freed store), and other statements.  The values run where the literal ran them,
staged in temporaries when the fill order is not the schema order or a statement follows the
first fill.

**Forwards** (`return g(…)` lowered three ways: a local bound in the function's own buffer,
the same with the buffer renamed and a witness kept, and the one-buffer chain that rebinds the
buffer itself) return the tuple too.  Materialising at a forward instead turned three bench
functions back into record returns on `--native`, a native regression the forward rule
removes.  `scripts/value_record_sigcheck.sh bench/*/bench.loft` is the check that no
native tuple is lost: it compares the native signatures with the pass on and off.

**Frame layout.** The rewritten functions are laid out again by the same two steps
`scopes::check` ends with (`compute_function_intervals`, `assign_function_slots`), after
`Function::reset_intervals`.

**Switch.** `LOFT_NO_IR_VALUE_RECORD=1`.

## Slices

| # | Shape | Status |
|---|---|---|
| # | Shape | Status |
|---|---|---|
| 1 | Tuple RETURN: the callee is not `pub`, loft calls it, its record is flat (two or more fields: 8-byte integer, float, single, boolean) and its result positions are object literals.  Buffers straight-line or loop-hoisted. | built |
| 2 | Tuple LOCALS written to: a field write becomes `TuplePut`. | open |
| 3 | Tuple PARAMETERS: a by-value parameter native's write gate admits; a carrier is handed on whole, a plain record local is read into a tuple at the call. | built |
| 4 | One function serves every caller: forwards (three spellings), twins for the sites that owe a record, a tuple copied into a place. | built |
| 5 | NESTED layouts: a record with inline sub-records is one flat tuple (`MoveResult { mr_pos: Vec3, … }`, two levels deep too). | built |

After this plan, the same move applies to the push family (R-Push, R-PushFill, R-PushRec,
R-Mint), then R-TextBorrow.  Those are separate plans.

## Verification per slice

This is the proportional rule (CI_BUDGET.md § Sizing the checks for a performance change).
Each slice brings: a guard with hand-computed cells, one planted defect recorded in its
`@falsified-at`, the switch, `find_problems.sh --changed`, and the census row it moves.
Slice 1 reaches every program, so it also runs the broader suite once.

Guards (each `@falsified-at` records its plant):
`a-small-record-returned-to-a-reader-is-a-tuple.loft` (slice 1),
`a-small-record-parameter-is-a-tuple.loft` (slice 3 — admitting a parameter past the native
write gate fails `aliased`), `a-small-record-is-forwarded-or-built-where-it-is-owed.loft`
(slice 4 — a materialised field from the wrong element failed `appended`; written against
the materialising cut, the cells hold for the twins), and
`a-small-record-with-an-inline-sub-record-is-a-tuple.loft` (slice 5 — a read through a
sub-record that drops the path's offset fails `two sub-records`; the first attempt changed
nothing, because every other cell's sub-record sits at offset 0).  Slice 1's plants
fail exactly one cell on both backends:

- A tuple built in schema order runs `tick` as 3, 2, 4, 1 and fails `fill order`.
- The old native emitter runs 7, 5, 8, 6 and fails `fill order when handed on`.

The native R-ValueRecord guards (`157-value-record.loft`, `157-value-chain.loft`,
`157-value-tail.loft`, `158-*`, `164-*`) now run through this pass as well.

Probed, clean on both backends:

- a callee reached from a `par` worker (rewritten);
- a caller in a generator body (declined by `value_records`, so both passes keep records);
- a literal with a whole sub-record copied in from a call (the copy's source runs once).

## Open questions

- `LOFT_TRACE_IR_VALUEREC=1` names each declined carrier and why.
- The rewrite census (`make rewrite-census`) will record native `R-ValueRecord` admissions
  moving to the `ir` phase.  That shows as a native DROP on the benches this slice takes, and
  the move is deliberate: bless it with this plan named.
- The debugger shows a tuple where the program wrote a record, but only on a program it did
  not open, since `open_world` keeps records wherever it parses.  If a debugger ever attaches
  to a closed program, the variable table should remember the record type the tuple stands
  for.

## See also

[formal/rewrites.md](../formal/rewrites.md) R-ValueRecord / R-ValueLocal ·
[PERFORMANCE.md § How the interpreter executes](../PERFORMANCE.md) (store work) ·
[NATIVE_SWITCHES.md](../NATIVE_SWITCHES.md) (`LOFT_NO_VALUE_RECORD`, `LOFT_NO_VALUE_LOCAL`).
