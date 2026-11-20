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
| `loft --width 32 …` or `LOFT_WIDTH=32` | every file: the program, every library it uses and the standard library |
| `loft --native-esp …` | the same as `--width 32`: the Tanmatsu's processor is 32-bit.  `--width 64` or `LOFT_WIDTH=64` keeps 64 bits |
| `#width 64` as the first line of a file | keeps that file 64-bit under `--width 32` |

The standard library (`default/*.loft`) is under the regime only with the whole program: a
`#width 32` file in a 64-bit build calls a 64-bit standard library, and its answers are narrowed
at the call.  Its `fn Op…` declarations name machine widths (`word`, `fword`, `i32`, `single`)
and its double functions say `f64`, so the regime changes what the library layer means and
never which opcode an operation is.

## What changes in a 32-bit file

| Written | Means |
|---|---|
| `integer` | `i32`: 32 bits, `i32::MIN` is its null |
| `float` | `single` |
| `123` | an `i32` literal; a literal that does not fit is an error that names `123l` — unless it stands where a wider type is expected: a `u32` or `i64` slot, or the right operand of an operator whose left operand is one (`c | 0xFF000000`) |
| `2.5` | a `single` literal |
| `i64`, `123l`, `0xFF000000l` | the 64-bit integer (`l` on a decimal, `0x`, `0b` or `0o` literal) |
| `f64`, `2.5d` | the 64-bit float |
| `2.5f` | a `single`, as everywhere |

`i64` and `f64` are legal everywhere.  Outside the regime they are `integer` and `float` by
another name.

## Expressions stay 32-bit

- An integer operation (`+ - * / % << >> & | ^`, compound assignment, unary `-`) on two
  32-bit operands is a 32-bit operation.  Its result is an `i32`, and a result outside the
  `i32` range reads null, as a 64-bit overflow does (C85).  The parser writes this as
  `OpNarrow32` around the operation.
- An operand whose type reaches past `i32` — an `i64`, a `u32` — makes the operation 64-bit.
  So `((a & 255) as u32) << 24` is a 64-bit shift that holds a colour's alpha byte, where
  `(a & 255) << 24` is a 32-bit one that reads null for an alpha of 128 or more.
- A call into a 64-bit file has its integer answer (any but `i64` and `u32`) and a plain
  `float` answer narrowed where the call returns.  A value that does not fit reads null.  A
  generic instance counts as its template's file: `sum` over a `vector<i32>` runs the
  stdlib's 64-bit body, so its answer is narrowed too.  A stdlib function whose answer needs
  64 bits declares `i64` and is not narrowed: `now()`, `ticks()`, `mtime()`, a file's
  `position()`, `JsonValue.as_long()`.
- The other way round, a 64-bit file calling a function whose body is under the regime: an
  `integer` argument for a parameter the regime made 32-bit is narrowed at the call, and a
  `float` argument for a `single` one is rounded.  A value that does not fit reads null.  So a
  library goes under the regime without its 64-bit callers changing a line.  A parameter the
  library declares `i64` or `f64` takes the value as it is: declare a timestamp parameter
  that way.  A colour, a hash or a bit mask is a `u32`: 32 bits on either width, and an
  `integer` narrowed into one is checked, 0 where it does not fit (`@FR-I-Narrow-U32`), so its
  64-bit callers pass their `integer` values unchanged.
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

**What the program holds.**  A value whose type fits 32 bits (`i32`, `u8`, `u16`, `i16`, `i8`, a
`limit` range inside `i32`) is an `int32_t` (null `LOFT_NULL32`) wherever its body was written
under the regime (`Data::body_width32`): a local, and in a whole-program 32-bit build
(`--width 32`) a parameter, a result, a tuple member and a function type too.  A 32-bit
operation is written in 32 bits (`loft_add32`, `loft_sub32`, `loft_mul32`, `loft_div32`,
`loft_rem32` in `src/c/loft_rt.h`), and two 32-bit values compare in 32 bits.  In a 64-bit build
a `#width 32` file's locals are 32-bit and everything that crosses a call stays `int64_t`; the
two meet through `loft_w64` / `loft_n32`, which keep null as null.

**What a loop counts in.**  A counter the compiler makes for a loop — `x#index`, `x#count`,
`x#next`, a walk's position and length — has the file's `integer`, so it is an `int32_t`
stepped by a 32-bit add under the regime (`Parser::count_type`, `(W-Count)`).

**What the runtime takes.**  The C runtime is one source for both widths.  Its first header,
`src/c/loft_width.h`, names the build's types, and a 32-bit unit defines `LOFT_WIDTH32` before
it:

| name | 64-bit build | 32-bit build | is |
|---|---|---|---|
| `loft_int`, `loft_uint` | `int64_t`, `uint64_t` | `int32_t`, `uint32_t` | a loft `integer`: a length, an index, a count |
| `loft_float` | `double` | `float` | a loft `float` |
| `loft_word`, `loft_fword` | `int64_t`, `double` | the same | what is 64 bits in every build: an `i64`, an `f64`, a file offset, a timestamp, a hash |
| `LOFT_INT_NULL`, `LOFT_INT_MIN`, `LOFT_INT_MAX`, `LOFT_INT_C(v)`, `LOFT_INT_BITS` | | | `loft_int`'s null, range, literal and size |
| `loft_int_wide(a)`, `loft_int_of(a)` | the value | `loft_w64(a)`, `loft_n32(a)` | a `loft_int` read as 64 bits, and a 64-bit value held in one |

A runtime function that takes or answers a loft `integer` is being moved from `int64_t` to
`loft_int`, one family at a time.  Moved: a vector's length (`loft_vec_len`, the held header's
`len`) and every index of the held-vector accessors (`loft_vh_*`).  The emitter hands such a
parameter its value through `as_wint` (`src/c/body.rs`): in a 32-bit build a 64-bit index is
narrowed there, and one outside the width reads null, which is outside every vector.  A proof
made about the 64-bit value does not hold for the narrowed one: the non-negative claim
(`loft_nonneg`) is made for a narrowed index only when its range is proven inside the width.

**What is left in fixed widths.**  A plain `int64_t` or `double` in the runtime is 64 bits on a
32-bit core whether the value needs them or not.  `tests/width_source.rs` counts the lines
that name one, per header, against `tests/width_source.baseline`: a header above its count
fails, so new runtime code is written in the width header's names.  It is the one check that
sees an inline 64-bit add, which calls no helper.

**Finding a call site that was missed.**  A 32-bit unit compiles without one
`-Wshorten-64-to-32` warning in its program part (clang).  After retyping a runtime function,
compile the corpus's 32-bit units with that warning: each report is a 64-bit value handed to a
`loft_int` parameter without `as_wint`, where a null would arrive as 0.

**The wide ledger.**  `LOFT_WIDE_LEDGER=<file>` on a 32-bit build writes where the unit holds
or computes 64 bits, one line an entry: the function, a class, what, and a count
(`src/c/wide_ledger.rs`).  The compiler wrote every one of them, so the list is exact for the
program's own code.

| class | is | what to do |
|---|---|---|
| `declared` | a variable or result whose type is wide by declaration (`i64`, `u32`, `f64`) | nothing: the source asked |
| `temp` | a variable the compiler made, of a wide type | explained by a `declared` value of the same function, or a defect |
| `undeclared` | a variable held in 64 bits whose type is the plain `integer` nobody declared wide | a compiler defect: type it by `Parser::count_type` |
| `runtime` | a conversion where a runtime function takes or answers 64 bits | move that function to `loft_int` / `loft_float` |
| `op` | a conversion at a 64-bit operator (`loft_mul`, `LOFT_PLAIN_NEG`) | one operand is a `declared` value; if none is, the compiler computed a 32-bit value in 64 bits |
| `call` | a conversion where one of the program's functions takes or answers 64 bits | that function declares `i64`, or its file is `#width 64` |
| `value` | a conversion in the function's own expression | one of its wide values |

`make wide-ledger` sums it over the corpus and holds the `undeclared` and `runtime` classes to
`scripts/wide_ledger.baseline`: a line that is new or above its count fails, so 64-bit work
removed from a 32-bit build stays removed.  `make wide-ledger ARGS="--top 30"` prints the
worklist — the runtime functions used most, and the compiler-made values still wide;
`make wide-ledger-bless` records a count that fell.  It
reads what the compiler wrote, not what the C compiler made of it; a library's hand-written C
and the linked image are outside it.

`LOFT_NO_C_INT32=1` holds every integer of the program as `int64_t` again.  It is the first
bisect step for a C-only wrong answer.

## The image gate

A 32-bit `loft --native-esp` build — its default — disassembles the app it linked and refuses it when a function
on loft's side — the program, loft's runtime, the C of a library the program uses — calls a
`double` helper (`__adddf3`, `__muldf3`, `__extendsfdf2`, …) or a 64-bit integer one
(`__muldi3`, `__divdi3`, `__ashldi3`, …): the ESP32-P4 has no instruction for either.  The
refusal names each function, where it came from and the helpers it calls.  ESP-IDF's own
archives are not judged.

An exception is one line in an allow file: the function, or a prefix ending in `*`, then the
reason it needs 64 bits.  A line without a reason is refused.

```text
n_cell_key     the cell key is a 64-bit hash
```

| file | covers |
|---|---|
| `src/c/esp_width64.allow` | loft's runtime: a log line's time, an `i64` print, a keyed collection's digest and `f64` key, a table's growth |
| `esp-width64.allow` beside the program's entry file | the program |
| `esp-width64.allow` in a library's package | that library's C |

The build's report (`loft-esp-report.txt`) lists what each exception allowed, and a program's
or a library's exception that allows nothing any more, to remove.

Two limits.  A function is named as the C compiler left it: what it inlined into a caller is
charged to the caller, so a small program can appear as `app_main` alone.  And a 64-bit add,
subtract or compare calls no helper, so the gate does not see one: the wide ledger does for
the program's code, and `scripts/width64_census.py` reports the pattern in the image.

A 64-bit shift by a variable amount is such a call too.  The runtime's bitmaps take the half
a bit lies in (`loft_bit64`, `loft_bit64_test`, `loft_bits64_from`, `loft_bits64_through` in
`src/c/loft_store.h`), which is two 32-bit shifts.

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
