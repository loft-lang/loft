<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Kernels — the register

A **kernel** is one Rust body that stands in for a loop loft code would otherwise run.  It is
spelled with the mechanism loft already has for calling Rust: a body-less standard-library
declaration with a `#rust` template, which `--native` inlines, plus the interpreter's adapter
(a row in `native::FUNCTIONS`, reached through its one native-call operator `OpStaticCall`).
Never an opcode, and never a new mechanism.  Every kernel that exists is listed here, with why
it exists and when it goes.

## The rule

**Kernels have their place, but each is a stop-gap, and none may dominate loft** (the owner's
rule).  A library function with a clear goal and a fast loft implementation is almost
always preferred, and optimisation work slightly favours the loft version over a new kernel.
The reason is reach: a kernel fixes one NAME, while making the loft code fast fixes the
PATTERN everywhere it is written, in users' own programs too.  A slow `split` body was the
interpreter's per-character text walk, which every hand-written text loop in a script pays.

This is where loft parts from Python on purpose.  Much of CPython's speed is hand-written C for
specific goals, because Python code itself cannot be made fast.  Loft compiles through rustc, so
its own code can be: measured below, the compiled loft bodies of `split` and `lines` are within
1.5× of their kernels already.  In the long run loft needs no kernels.  Each one exists only
for a gap the INTERPRETER (or, for `sum`, the code generator) has not closed yet.

**Every kernel is to be removed, and the standard library is to be compiled like any other
library** (the owner's rule).  A `use`d library is already compiled for an interpreted script
(C71, below), so its loops run as compiled loft; the standard library will run the same way,
and then an interpreted script calls a kernel's compiled LOFT body just as a native program
does.  The Rust version of a routine is replaced by loft as soon as that is viable.

**Until then, kernels live only in the standard library** (the owner's rule).  Outside it,
hand-written Rust stays sparse: a library writes its loops in loft, and a `use`d library is
compiled for an interpreted script anyway
([C71](DESIGN_DECISIONS_PLATFORM.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model),
`src/native_lib.rs`).  A library's `#native` Rust is for what loft cannot express (an OS or a
foreign API), never for speed.

**A kernel earns its keep, or it does not stay.**  To be admitted:

1. A profile of a real workload names one standard-library loop as dominant: a @PLN179 script
   port's `LOFT_PROFILE=1`, or a bench routine past the interpreter's 100× cliff.
2. The general fix (making that loft code fast) is named, and is not reachable soon.
3. It stays in the standard library: a loop that belongs in a library (loft over primitives,
   @PLN179 strand 8) moves there and is written in loft, never kernelled.
4. It adds no public surface: it is private, or it sits under a name that already exists.
5. A guard compares it with the loft body it replaced on both backends, and its plant has run
   (the guard went red with the kernel sabotaged).
6. It has a row in `scripts/kernel_ratio` and an entry below, with its removal trigger.

**When one goes.**  `make kernel-ratio` times every kernel against its loft body on the same
input, best of 5, on both backends, and checks that the two answer the same.  A kernel is
**due for removal on a backend when its loft body runs within 2× of it there.**  At that point
the kernel buys less than the gap every user loop of the same shape still pays, and keeping it
hides that gap from the measurements that should drive the general fix.  When it is due on both
backends, the loft body returns, the kernel's Rust and its adapters go, and its entry moves to
§ Removed with the measurement.

**The standard library is compiled like a library** ([@PLN181](plans/181-compiled-stdlib.md)):
an interpreted program runs a looping standard-library function's COMPILED loft body, the one a
native program runs.  So for a kernel whose loft body is not generic, the native column decides
alone — the interpreter pays what native pays.  `split` and `text_lines` were retired that way
(§ Removed).  `vector_sum_int` stays: `sum` is generic, monomorphised per program, so it cannot
be precompiled, and its plain loop is still 4.9× the kernel compiled.

| kernel | interpreted | `--native-release` |
|---|--:|--:|
| `vector_sum_int` | 246× — keep | 5.0× — keep |

Loft body ÷ kernel, best of 5, on this project's perf box; `make kernel-ratio` is the current
answer, and this table is re-measured whenever an entry changes.

## The kernels

### `vector_sum_int` — `loop_kernels::vector_sum_int`

- **Where.** `fn vector_sum_int(v, acc)` in `default/01_code.loft`, private; the scope-pass rule
  `loop_kernels::rewrite` replaces an `acc = acc + v[i]` loop over `0..v.len()` of 8-byte
  integers with one call, in the stdlib `sum` and in a program's own code alike.
  `LOFT_NO_LOOP_KERNELS=1` keeps every loop a loop (the A/B).
- **Why.** `sum` interpreted was 1,174,380 ns against native's 4,300 (273×, past the 100×
  cliff); with the kernel 4,830 ns.
- **What it costs.** A rewrite rule that matches one spelling.  A program's `for x in v { r = r
  + x }` is not matched and still pays over 150×; `kernel_ratio` measures exactly that loop, because
  it is the gap the kernel does not close.
- **What replaces it.** A reduction the interpreter runs as one operation whatever its element
  type, operator and loop spelling (@PLN180's IR work).  On native, block summation for the
  plain loop (the kernel sums in blocks and checks overflow once per block; the loop checks per
  element, which is the 4.9×).
- **Removal.** Both ratios at 2× or under.  Today over 150× and 4.9×.
- **Guard.** `tests/scripts/a-reduction-loop-is-one-kernel-call.loft` (the rewrite's soundness
  cell is `another vector`); `tests/bounded_sum.rs` pins the emission.

## Open

- **A generic function compiled for the interpreter.**  The compiled standard library
  (@PLN181) covers non-generic functions only; `sum<T>` is instantiated per program.  Precompiling
  the common instances (`sum` over `integer`, `float`) the same way would retire
  `vector_sum_int` by the route that retired `split` and `lines`.
- **The interpreter adapter is written by hand.**  `--native` runs a kernel's `#rust` template
  directly, but the interpreter needs its own adapter in `src/native.rs` (196 such rows for
  the standard library's `#rust` functions, `vector_sum_int`'s among them).  Operators already
  avoid this: `make fill` generates the interpreter's side from the same template
  (`src/create.rs::generate_code_into`, kept current by `tests/issues.rs::fill_rs_up_to_date`).
  Extending that generator to body-less functions would make a kernel exactly one `#rust` line
  and retire the hand-written adapters with it.

## Removed

### `split` (`Stores::split_char`) and `text_lines` (under `File.lines()`) — retired by the compiled standard library

Both were admitted for @PLN179 finding 013 — `scripts/opl_points` spent 73 % of its interpreted
run in the loft `split` body, `tests/dump_ignored_tests` 91 % in `lines` — and both restated
loft's rules in Rust (split's empty text answers no parts; lines drops one `\r` before each
`\n`).  With the standard library compiled like a library (@PLN181), their loft bodies came back
and an interpreted program runs them compiled.  Measured at the retirement, best of 5:

| | the kernel | the compiled loft body |
|---|--:|--:|
| `split`, interpreted | 2.3 ms | 3.4 ms (1.5×) |
| `lines`, interpreted | 1.5 ms | 2.0 ms (1.3×) |
| `split`, `--native-release` | 2.3 ms | 0.6 ms (native's split rewrites apply again) |
| `lines`, `--native-release` | 1.7 ms | 2.0 ms (1.2×) |

The ports, through the twin: `dump_ignored_tests` 626 ms → 655 ms, `opl_points` (200k lines)
1.77 s → 1.62 s, identical on every channel.  Their guards stay, renamed for the rules they pin
(`tests/scripts/a-text-split-keeps-its-edge-rules.loft`,
`tests/scripts/a-file-lines-keeps-its-line-rules.loft`), and their plants in the loft bodies
turn them red on both backends.
