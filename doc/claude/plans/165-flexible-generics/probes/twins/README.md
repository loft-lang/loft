<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The twin matrix (@PLN165 step A0)

`G-Mono` says an instance answers exactly what a hand-written concrete twin answers.  Each
cell here prints a generic's answer BESIDE its twin's, and the `@EXPECT` line was computed by
hand — agreement between the two columns is not the pass, the hand value is.

```bash
LOFT_STRICT_STORES=1 LOFT_POISON=1 scripts/probe-matrix \
  doc/claude/plans/165-flexible-generics/probes/twins --backend both
```

Every later step's acceptance is *"add its cells here"*; a cell that needs a library puts it
under `lib/` (resolved as the script's own directory's `lib/`).

## What it holds fixed

`scripts/matrix_axes.py file` over the cells joined, at A0:

| Axis | Reaches | Held fixed |
|---|---|---|
| A1 container kind | vector, tuple, hash (bound to `T`) | sorted, index, spatial |
| A2 container provenance | local literal, parameter | global, callee return |
| A3 argument spelling | all 8 | — |
| A4 statement context | 8 of 10 | a bare block, the `??` right-hand side |
| A5 nullability | both | — |
| A6 default shape | literal | variable, call |
| A7 element type | all 8 | — |
| A9 evaluation count | both | — |

The keyed kinds a template cannot NAME (`hash<T[k]>` over a variable) are refused at the
definition by `D-Keyed`; a keyed collection BOUND to `T` is t10's cell.  One variable, in the
first parameter, throughout — arcs C and D move those.
