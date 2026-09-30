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

**Once the standard library is compiled like a library, the native column decides alone**: the
interpreter then runs the same compiled loft body a native program does, so the interpreted
ratio measures nothing the kernel buys.  By the table below, `split` and `text_lines` are due
the moment it lands; `vector_sum_int` waits for the code generator's reduction.

| kernel | interpreted | `--native-release` |
|---|--:|--:|
| `split` | 12.0× — keep | 1.5× — due |
| `text_lines` (under `lines`) | 38.6× — keep | 1.2× — due |
| `vector_sum_int` | 172.7× — keep | 4.9× — keep |

Loft body ÷ kernel, best of 5, on this project's perf box; `make kernel-ratio` is the current
answer, and this table is re-measured whenever an entry changes.  Native is due for both
text kernels: rustc already runs their loft bodies close to the kernel, so on native they are
kept only because one declaration serves both backends.

## The kernels

### `split` — `Stores::split_char`

- **Where.** `pub fn split(self: text, separator: character)` in `default/02_files.loft` is
  body-less with a `#rust` template; the interpreter adapter is `t_4text_split` in
  `src/native.rs`.  Native's `R-LazySplit` and `R-SplitTable` rewrites read the kernel call.
- **Why.** @PLN179 finding 013: 73 % of `scripts/opl_points` interpreted was the loft `split`
  body.  The bench routines moved `split` 1.83 ms → 46 µs and `split_walk` 2.15 ms → 43 µs,
  interpreted.  The port went from 10× to 5.8× Python.
- **What it costs.** The empty-text rule restated in Rust (Rust's `str::split` answers one empty
  part, loft answers none), and a second spelling for native's split rewrites to match.
- **What replaces it.** A fast character walk on the interpreter: `for c in text` with
  `c#index`, and a slice appended to a vector, which is the loop every text-processing script
  writes.  The fused `OpTextWalkStep` covers the walk; the slice into a fresh element does not
  yet.
- **Removal.** Interpreted ratio at 2× or under.  Today 12×.
- **Guard.** `tests/scripts/text-split-is-one-kernel-call.loft`; the loft body is the oracle in
  `scripts/kernel_ratio`.

### `text_lines` — `Stores::text_lines`, under `File.lines()`

- **Where.** A private `fn text_lines(content: text)` in `default/02_files.loft`; `lines` is
  `text_lines(self.content() ?? "")`.  The interpreter adapter is `n_text_lines`.
- **Why.** @PLN179 finding 013: 91 % of `tests/dump_ignored_tests` interpreted was the loft
  `lines` body; the port went from 3.67 s to 0.62 s (Python 0.22 s), and the finding's probe from
  over its 150 ms bar to 58 ms.
- **What it costs.** The CR/LF rule restated in Rust: every `\n` ends a line and drops ONE
  `\r` before it, and a last piece is a line only when it is not empty, keeping its `\r`.
- **What replaces it.** The same character walk as `split`'s.  Reading a file as lines is work
  the runtime could legitimately own (a streamed read would be one), but the loop this replaced
  is text splitting over content already read, so it is held to the same trigger.
- **Removal.** Interpreted ratio at 2× or under.  Today 39×.
- **Guard.** `tests/scripts/a-file-lines-is-one-kernel-call.loft`, which carries the loft body
  it replaced as its own oracle.

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

- **The standard library compiled like any other library** — the route that retires the
  kernels, planned as [@PLN181](plans/181-compiled-stdlib.md).  Its artifact is built (P1): the
  standard library's looping loft functions already run compiled for an interpreted program,
  and a kernel's loft body, restored, joins them by the same rule (P2).  C71 already compiles a `use`d library for an interpreted script and dispatches its
  functions through the shared-store bridge (`src/native_gate.rs`, `src/native_lib.rs`).  What
  differs for the standard library: it must need no rustc on the user's machine (`--interpret`
  never runs one), so its artifact is built with loft itself, once per loft version, and ships
  with the binary.  That is possible where a library's is not: a library's artifact is keyed on
  the calling program's WHOLE type layout (`type_layout_fingerprint`), one build per distinct
  program, while the standard library's types register before any program's, so one layout
  serves every program.  The artifact is still verified against the running program's type
  table before use (as a library artifact is), and a mismatch runs the loft body interpreted:
  slower, never wrong.
- **The interpreter adapter is written by hand.**  `--native` runs a kernel's `#rust` template
  directly, but the interpreter needs its own adapter in `src/native.rs` (196 such rows for
  the standard library's `#rust` functions, the three kernels among them).  Operators already
  avoid this: `make fill` generates the interpreter's side from the same template
  (`src/create.rs::generate_code_into`, kept current by `tests/issues.rs::fill_rs_up_to_date`).
  Extending that generator to body-less functions would make a kernel exactly one `#rust` line
  and retire the hand-written adapters with it.

## Removed

None yet.
