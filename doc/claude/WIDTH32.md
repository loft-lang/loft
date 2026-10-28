<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The 32-bit regime — `#width 32`

On a 32-bit processor (the Tanmatsu's ESP32-P4, @PLN204) every 64-bit integer is a register
pair, a 64-bit multiply is several instructions and a 64-bit divide is a library call.  The
32-bit regime makes 32 bits the default for numbers whose width the code does not choose,
so a program pays for 64 bits only where it asks for them.

## Switching it on

| How | Reach |
|---|---|
| `#width 32` as the first line of a file | that file |
| `loft --width 32 …` or `LOFT_WIDTH=32` | every file except the standard library: the program and every library it uses |
| `#width 64` as the first line of a file | keeps that file 64-bit under `--width 32` |

The standard library (`default/*.loft`) is never under the regime.

## What changes in a 32-bit file

| Written | Means |
|---|---|
| `integer` | `i32`: 32 bits, `i32::MIN` is its null |
| `float` | `single` |
| `123` | an `i32` literal; a literal that does not fit is an error that names `123l` |
| `2.5` | a `single` literal |
| `i64`, `123l` | the 64-bit integer |
| `f64`, `2.5d` | the 64-bit float |
| `2.5f` | a `single`, as everywhere |

`i64` and `f64` are legal everywhere.  Outside the regime they are `integer` and `float` by
another name.

## Expressions stay 32-bit

- An integer operation (`+ - * / % << >> & | ^`, compound assignment, unary `-`) on two
  32-bit operands is a 32-bit operation.  Its result is an `i32`, and a result outside the
  `i32` range reads null, as a 64-bit overflow does (C85).  The parser writes this as
  `OpNarrow32` around the operation.
- An operand of type `i64` makes the operation 64-bit.  That is the one way to widen.
- A call into a 64-bit file answers a plain `integer` or `float`, which is narrowed where the
  call returns.  A value that does not fit reads null.  A stdlib function whose answer needs
  64 bits declares `i64` and is not narrowed: `now()`, `ticks()`, `mtime()`, a file's
  `position()`, `JsonValue.as_long()`.
- The two arms of an `if` that are both 32-bit join to a 32-bit value.

A program under the regime answers the same on every backend and every processor: the width
is in the source, not in the target.  So the interpreter on an ordinary CPU is the oracle
for the C build on the device.

## The C backend

The C backend holds every integer whose type fits 32 bits (`i32`, `u8`, `u16`, `i16`, `i8`,
a `limit` range inside `i32`) as `int32_t`, null `LOFT_NULL32`.  This applies to every
program, regime or not.  A 32-bit operation is written in 32 bits (`loft_add32`,
`loft_sub32`, `loft_mul32`, `loft_div32`, `loft_rem32` in `src/c/loft_rt.h`), and two 32-bit
values compare in 32 bits.  The C runtime itself is written in `int64_t`, so a 32-bit value
passed to it is widened (`loft_w64`) and a 64-bit value stored into a 32-bit local is
narrowed (`loft_n32`), each keeping null as null.

`LOFT_NO_C_INT32=1` holds every integer as `int64_t` again.  It is the first bisect step for
a C-only wrong answer.

## Verifying it

- The probes under `tests/scripts/` tagged `@PLN204` carry hand-computed values at the `i32`
  edges and run on the interpreter, `--native` and `--native-c`.
- `scripts/c_sweep.py` with and without `LOFT_NO_C_INT32=1` compares the C backend's
  32-bit and 64-bit forms over the corpus.
- `LOFT_WIDTH=32 scripts/revalidate_libs_local.sh` runs every published library's tests under
  the regime.
