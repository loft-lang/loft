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
- A call into a 64-bit file has its integer answer (any but `i64` and `u32`) and a plain
  `float` answer narrowed where the call returns.  A value that does not fit reads null.  A
  generic instance counts as its template's file: `sum` over a `vector<i32>` runs the
  stdlib's 64-bit body, so its answer is narrowed too.  A stdlib function whose answer needs
  64 bits declares `i64` and is not narrowed: `now()`, `ticks()`, `mtime()`, a file's
  `position()`, `JsonValue.as_long()`.
- The two arms of an `if` that are both 32-bit join to a 32-bit value.
- A plain `integer` that is not a call (an element's `#index`, a field of a 64-bit file's
  struct) stored into an `i32` is narrowed there, null when it does not fit.  A target
  narrower than `i32` (`u8`, a `limit`) still refuses it, as everywhere.
- A count — a collection's `len`, a text's `size` — is an `i32`.  A collection past `i32::MAX`
  elements is outside the regime: it does not fit a 32-bit address space.

**Binary files take the value's width.**  `f += x` on a `LittleEndian` / `BigEndian` file
writes as many bytes as `x`'s type holds, so under the regime a plain `integer` writes 4 bytes
and a plain `float` writes a 4-byte `single`.  A file format a library shares with 64-bit
programs spells each width at the write (`as i64`, `as f64`, `as i32`), as `hex_field`'s
fixture test shows when it does not.

A program under the regime answers the same on every backend and every processor: the width
is in the source, not in the target.  So the interpreter on an ordinary CPU is the oracle
for the C build on the device.

## The C backend

The C backend holds a LOCAL as `int32_t` (null `LOFT_NULL32`) when its type fits 32 bits (`i32`,
`u8`, `u16`, `i16`, `i8`, a `limit` range inside `i32`) and its function's body was written under
the regime (`Data::body_width32`).  There every value is narrowed before it is held, so it fits.
A 64-bit body's `i32` local keeps the wide value it was given, as the interpreter's 8-byte slot
does, so it stays `int64_t`; and a parameter, a result, a tuple member and a function type stay
`int64_t` in every program, so two bodies meet through `loft_w64` / `loft_n32`, which keep null
as null.  A 32-bit operation is written in 32 bits (`loft_add32`, `loft_sub32`, `loft_mul32`,
`loft_div32`, `loft_rem32` in `src/c/loft_rt.h`), and two 32-bit values compare in 32 bits.  The
C runtime itself is written in `int64_t`, so a 32-bit value passed to it is widened.

`LOFT_NO_C_INT32=1` holds every integer as `int64_t` again.  It is the first bisect step for
a C-only wrong answer.

## Verifying it

- The probes under `tests/scripts/` tagged `@PLN204` carry hand-computed values at the `i32`
  edges and run on the interpreter, `--native` and `--native-c`.
- `scripts/c_sweep.py` with and without `LOFT_NO_C_INT32=1` compares the C backend's
  32-bit and 64-bit forms over the corpus.
- `scripts/c_sweep.py --m32` builds the C as 32-bit x86 (`gcc -m32`) and `--riscv32` as 32-bit
  RISC-V Linux (`zig cc -target riscv32-linux-musl`, run under `qemu-riscv32`): the target's
  ISA and its ILP32 C, against the interpreter.  The tools: `sudo apt install qemu-user
  gcc-multilib` and `sudo snap install zig --classic --beta`.
- `LOFT_WIDTH=32 scripts/revalidate_libs_local.sh` runs every published library's tests under
  the regime.
