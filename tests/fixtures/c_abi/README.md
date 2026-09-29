<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `lc_types` — a C library with one loft type per function

The fixture [@PLN24](https://github.com/loft-lang/plans/issues/24) is measured
against: **`#c "<symbol>"`**, a loft function bound straight to a C symbol with
no Rust wrapper, no rustc and no libffi. Each function here takes exactly one
loft type, so a marshalling fault names its type instead of just failing.

Plain C and libc. The whole build is `cc` — which is the property the plan
exists for, so the build demonstrates it rather than claiming it.

```sh
make check     # build + run the self-test (this is what validates the oracle)
make probe     # run the @PLN24 trampoline probe against it — see below
make strict    # rebuild under -std=c89 -pedantic: it really is ANSI C
make           # just the shared library (liblc_types.so / .dylib)
```

`make check` is not optional before using this as a reference. An oracle nobody
validated is a second opinion, and every expected value in `lc_selftest.c` is
hand-computed so that when a loft binding later disagrees with this library,
the disagreement is the binding's.

## The two halves, which are the design under test

**DIRECT** — every argument and the return are integer-class, so they collapse
to a `u64` slot and fit the fixed per-arity trampolines (`extern "C" fn(u64, …)
-> u64`, one per arity). This is the path that needs no libffi.

**BOUNDARY** — a `double`/`float` argument, a struct by value, varargs, or an
out-parameter. No trampoline can express these: floats travel in SSE registers,
not the integer registers, so it is a different register file rather than a
width problem. Each boundary function is paired with an `lc_shim_*` that
presents the same capability in integer-class shape. **The shims are the
evidence for the plan's central trade** — "keep loft-core minimal, put the
complexity in an ANSI-C shim" is only a good trade if a shim is three lines,
and here you can read them and see.

## The matrix

| loft type | C parameter | function | direct / shim |
|---|---|---|---|
| `integer` | `int64_t` | `lc_i64`, `lc_raw_i64` | direct |
| `i32` / `i16` / `i8` | `int32_t` / `int16_t` / `int8_t` | `lc_i32`, `lc_i16`, `lc_i8` | direct |
| `u64` / `u32` / `u16` / `u8` | matching unsigned | `lc_u64` … `lc_u8` | direct |
| `boolean` | `int32_t` | `lc_bool`, `lc_raw_bool` | direct |
| `character` | `uint32_t` codepoint | `lc_char` | direct |
| `text` | `const char *` (NUL-terminated) | `lc_strlen`, `lc_byte_at`, `lc_len_upto` | direct |
| `text` return, borrowed | `const char *` static | `lc_static_text` | direct |
| `text` return, owned | `char *` + `lc_free` | `lc_alloc_text` | **shim** — loft never frees |
| `text?` return, NULL | `const char *` = 0 | `lc_maybe_text` | direct |
| `text` return, not UTF-8 | `const char *` Latin-1 | `lc_latin1_text` | direct |
| `vector<integer>` | `const int64_t *`, count | `lc_i64_sum` | direct |
| `vector<i32>` | `const int32_t *`, count | `lc_i32_sum` | direct |
| `vector<u32>` / `<u16>` / `<u8>` | matching unsigned pointer | `lc_u32_dot`, `lc_u16_dot`, `lc_u8_dot` | direct |
| `vector<character>` / `<boolean>` | `const uint32_t *` / `const unsigned char *` | `lc_char_dot`, `lc_bool_dot` | direct |
| `vector<single>` | `const float *` | `lc_f32_dot_milli` | direct |
| `vector<i8>` / `<i16>` | — | — | **REFUSED** — loft stores these biased (`val - min`) |
| `double` RETURNED by value | `double(...)` | `lc_ddot_` | direct |
| `float` RETURNED by value | `float(...)` | `lc_sdot_` | direct |
| `vector<text>` | — | `lc_strv_total` | **REFUSED** — loft's elements are heap handles, not `char *` |
| opaque handle (`PGconn *`) | `void *` as loft `integer` | `lc_open` / `lc_read` / `lc_bump` / `lc_close` | direct |
| `null` (any type) | whatever the binding sends | `lc_is_null`, `lc_raw_i64`, `lc_raw_bool` | direct |
| arity 0 / 1 / 6 / 7 / 12 | `int64_t …` | `lc_arity*` | direct |
| Fortran: 5 **bare** pointers, no counts | `const double *` … | `lc_daxpby_` | direct |
| Fortran: `dgemm_`'s 13 by-reference arguments | `const char *`, `const int64_t *`, `const double *` | `lc_dgemm_` | direct |
| counted and bare vectors **mixed** in one signature | `const double *, int64_t, const double *, int64_t` | `lc_split_` | direct |
| `float` | `double` | `lc_f64` | **shim** `lc_shim_f64` |
| `single` | `float` | `lc_f32` | **shim** `lc_shim_f32` |
| struct by value | `struct lc_pair` | `lc_pair_dot` | **shim** `lc_shim_pair_dot` |
| out-parameter | `int64_t *` | `lc_divmod` | **shim** `lc_shim_div` / `_mod` |
| varargs | `...` | `lc_var_sum` | **shim** `lc_shim_sum3` |

## The FOREIGN half — the rows written by the other side (@PLN128 arcs D and E)

Everything above them was written **for loft**: each function takes a `vector` as
a pointer *and* a count, because that is the shape loft used to insist on. That
made the fixture agree with loft about the one thing it should have been
questioning, and a whole class of libraries stayed invisible behind it — the
matrix could report "14 argument slots ✅" while no real BLAS routine was
bindable at all.

`lc_daxpby_`, `lc_dgemm_` and `lc_split_` are written to a **foreign**
convention instead. Fortran passes every argument by reference, so each one is a
bare pointer and the routine learns its length from a separate `n` — itself a
bare pointer. Measured against them before the fix: the honest declaration was
refused for arity, and the declaration loft accepted delivered each count where
the callee expected the next pointer, which SIGSEGV'd the interpreter and
produced nothing under `--native`.

`lc_split_` is the composition cell: `sel` is an ordinary integer sitting where
the first vector's count would go, and the count that IS present belongs to the
second vector. A left-to-right walk that takes an integer whenever one follows a
pointer assigns it to the wrong vector — and because the function is
position-weighted, that answers a different number instead of the right one by
luck.

**A fixture written to the shape under test cannot falsify it.** That is the
lesson those three rows exist to keep.

`lc_ddot_` and `lc_sdot_` are the fourth and fifth, added by @PLN128 arc E: the
level-1 BLAS *function* shape, where the answer comes back **by value** in an
SSE register. Every earlier function here returns an integer or writes through a
pointer, so the whole family — `ddot_`, `dnrm2_`, `dasum_`, and every LAPACK
auxiliary that answers a number — was invisible to the matrix while the boundary
refused it. Both widths are here because a C `float` return is a *single* in
that register, not a narrowed double.

**And the limit of a purpose-built fixture, learned the same way.** Arc E bound
**real OpenBLAS** and found three things this library could not have shown,
because everything in it was written by whoever was writing the declarations:

- a `vector`'s element WIDTH was never checked against the C pointee, so
  `vector<integer>` at LAPACK's `int *` answered `8589934593, 0` for the pivots
  `1, 2`, and `vector<single>` at `double *` let C write 24 bytes into a 12-byte
  loft vector — both silent, both backends;
- loft stores narrow SIGNED elements as `val - min`, so `vector<i8>` and
  `vector<i16>` never were C arrays;
- the `vector<text>` row in the matrix above was **false** — it SIGSEGVs, and
  nothing had ever bound it.

The `lc_*_dot` readers below are the repair: one per element width, each
position-weighted, so the layout loft writes and the layout C reads are pinned
against each other rather than assumed.

Values are chosen so a wrong answer is *reachable*, not merely unlikely:

- The integer functions XOR with a mask that has bits set in **both halves** of
  the word, so a 32-bit truncation of a 64-bit value cannot come out right.
  They are self-inverse, so a probe can round-trip instead of hard-coding.
- `lc_neg_i32` returns a **negative** number. A binding that declares the return
  64-bit while C returns `int` leaves the upper half undefined and −1 arrives as
  4294967295 — which every `>= 0` check accepts. A positive-only probe cannot
  see that, and this shape has caught it in a shipped library before.
- Vector sums are **position-weighted**, so elements delivered in the wrong
  order, or a `vector<i32>` read at 64-bit stride, give a wrong total.
- Arities are 0, 1, 6, 7, 12 — not a ladder. On the SysV x86-64 ABI the first
  **six** integer arguments go in registers and the seventh onward on the stack,
  so 6 and 7 straddle the boundary a hand-written trampoline is likeliest to get
  wrong. Each argument is weighted by a distinct prime, so swapping any two
  arguments changes the answer.
- The handle accessors check a magic word, so a mangled handle — a pointer
  truncated to 32 bits, or an `integer` null (`i64::MIN`) passed where a handle
  belongs — answers −1 instead of dereferencing it. A fixture that segfaults on
  a bad binding tells you less than one that reports.

## What the probe found — `make probe`

`probe_trampoline.rs` is the plan's architecture run against this whole matrix:
`dlopen` + `dlsym`, then the symbol reinterpreted as `extern "C" fn(u64, …) ->
u64` at each arity. Rust, not C, because the trampolines land in loft-core and
the question is whether *Rust* can express them soundly. One `rustc`, no cargo,
no crates. Every assertion passes; three cells only report, because they are
undefined behaviour and asserting on them would pin whatever this machine did
today.

**The claim holds for arguments.** Every integer-class argument — `int`, `long`,
a pointer, a `char *`, a bool — crosses correctly through a `u64` slot, at
arity 0, 1, 6, 7 and 12, including across the register/stack boundary, with no
libffi and no generated code. The `dlopen`-and-transmute path is sound.

**It does not hold for returns, and that is a design change.** `int32_t
lc_neg_i32(1)` returns −1; read back through a `u64` trampoline it is
`0x00000000FFFFFFFF` = **4294967295**. SysV leaves the upper half unspecified
for a 32-bit return and x86-64 zero-extends, so the failure is not a crash but a
plausible large *positive* — the shape that silently defeated loft-libs-net's
`server::listen` `handle >= 0` check. So "arity alone parameterises the
trampoline set" is true of the *call* and false of the *answer*: the set stays
~13 functions, but the declaration must carry the C return width. **This settles
the plan's load-bearing question in favour of option (a)** — spell the C type,
`#c "PQstatus" "int(void*)"`.

**The boundary is not self-enforcing.** Of the three deliberate misuses, one
returned garbage (a `double` argument, which went to `rdi` while the callee read
`xmm0`) and **two returned the right answer by luck**: a variadic function
called through a non-variadic trampoline, and an arity-1 symbol called through
an arity-3 one. Extra register arguments are simply ignored. Nothing at runtime
distinguishes a correct binding from a wrong one here, so the check against the
C signature has to be at COMPILE time — there is no runtime signal to fall back
on, and a design that assumes "we'd notice" is assuming nothing.

**The shims all work**, and each is three lines. That is the evidence for the
trade the plan is built on: complexity in an ANSI-C shim rather than in
loft-core.

## The three questions this fixture exists to answer

1. **C integer width** (the plan's stated load-bearing question) — **answered:
   the declaration must spell the C type.** Measured above: a 32-bit C return
   read as 64 bits turns −1 into 4294967295. Arguments are fine; the return is
   not.
2. **loft `text` is not a C string.** loft text is UTF-8 **plus a length**; C is
   a pointer that ends at the first NUL. Give `lc_strlen` and `lc_len_upto` the
   same loft text containing an interior NUL and they disagree exactly when the
   binding has silently truncated it. (The Rust `#native` path passes `ptr, len`
   and has no such failure, so this is a real difference between the two binding
   paths, not a detail — a library moving from `#native` to `#c` meets it.) The
   `lc_static_text` / `lc_alloc_text` pair asks the other half: who frees a
   returned buffer, which the C type system cannot express, so the binding must
   carry the answer per function. **Answered (@PLN24 arc D): loft never frees.**
   `const` does not separate the two — POSIX spells both `char *` — so a rule
   read off the signature would free static memory on a wrong guess, and that
   failure is not recoverable while a leak is. `lc_alloc_text` is therefore the
   case that goes through a shim, and it is bound in the package as nothing at
   all. `lc_maybe_text` and `lc_latin1_text` ask the two remaining questions a
   `char *` return raises — a NULL answer and bytes that are not UTF-8 — because
   both are ordinary in C and both are places the two backends could quietly
   disagree.
3. **`boolean` is three-state and C has no such type.** loft has false 0 / true 1
   / **null 255**. `lc_raw_bool` reports whatever arrives rather than judging it,
   so the answer is measured instead of assumed.

## The loft side

Live, in [`pkg/lcabi/src/lcabi.loft`](pkg/lcabi/src/lcabi.loft) — one `#c`
declaration per shape in the table above, plus the `[c] libs` entry that points
at the `.so` this directory builds. Read that file rather than an example here:
it is what the matrix test actually runs, so it cannot drift.

`lc_alloc_text` is deliberately **not** bound. It is the caller-frees half of the
ownership pair, and loft never frees a returned pointer — it is the case that
belongs to a shim, and leaving it unbound is what says so.

## Status

`#c` works on both backends, and this directory is its composition matrix:
`tests/native.rs::c_binding_matrix_against_a_declared_library` builds the `.so`,
runs the package under `--interpret` and `--native`, and requires byte-identical
output against hand-computed expectations. `make check` is the calibration that
says the C oracle itself is sound before any loft is involved.

The probe is a *design* instrument, deliberately not a test: it answers a
question that is settled once, and wiring it into CI would make every machine
carry a `cc` + `rustc` dependency to re-derive an answer that is written down
here. Re-run it if the trampoline design changes, or on a new target — the
argument-register boundary it probes is per-ABI, and AArch64 splits at eight
arguments where x86-64 splits at six.

See also: [PACKAGES_C_BINDING.md](../../../doc/claude/PACKAGES_C_BINDING.md) ·
[@PLN24](https://github.com/loft-lang/plans/issues/24) ·
[@PLN23](https://github.com/loft-lang/plans/issues/23) (the MariaDB/PostgreSQL
clients — the first consumer, and the reason the handle shape is here).
