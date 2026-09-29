<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 24 — `#c`: call a C library with no Rust in it

## Status — DONE 2026-08-08. All seven arcs shipped.

A loft library binds directly to a system C library — **no rustc anywhere in the
library**, no libffi — and reaches the same standard of proof as a Rust `#native`
library.

**The reference lives in [PACKAGES.md § Direct C binding](../../PACKAGES_C_BINDING.md#direct-c-binding--c-pln24)**:
the declaration and its type mapping, the `char *` return, `[c] libs` /
`optional-libs` / `shim`, `c_library_available`, the wasm refusal, and the sandbox
rule. Read that to *use* `#c`. What follows is the closure record — why it is
shaped this way, and what the build corrected — kept because every item below was
a measurement that changed a design, not a note about the code.

| Arc | Shipped |
|---|---|
| **A** — the declaration + `CSignature` + the compile-time check | `src/c_signature.rs`, landed inert; widths resolve per target |
| **B** — the interpreter caller: `dlsym` + per-arity trampolines + return-width dispatch | `src/c_call.rs`; both backends answer identically |
| **C** — the `--native` caller: a typed `extern "C"` emitted from `CSignature` | loft calls libc directly, no rustc in any library |
| **D** — packaging | `[c] libs` declares + loads + links; a `char *` return crosses as `text` / `text?`; `[c] shim` is ANSI C loft compiles itself (`cc`, never rustc) at parse and at `loft install` |
| **E** — the other two targets | both wasm shapes refuse a REACHABLE `#c` call by name; an unused declaration still builds; `c_library_available` compiles and answers `false` there |
| **F** — a real system library | @PLN23 S1: `libmariadb.so.3` through a versioned soname, both backends identical, zero rustc |
| **G** — optional libraries | `[c] optional-libs` + `c_library_available`; a duckdb backend in `tests/fixtures/sqldb/duckdb/`, both backends byte-identical present AND absent |

**Not built, and deliberately so:** [LIBRARY_NAMING.md](LIBRARY_NAMING.md) — a `[c]`
library named ONCE by identity (`sqlite3@0`), with every filename, link flag and
search path derived from it. Written off four measured failures, each of which
currently carries its own local workaround. Tracked as **PKG.CNAME** in
[PACKAGES.md § Open work](../../PACKAGES.md#open-work) with its trigger.

## The invariant

> **The `#c` declaration is the sole authority on the C signature, and every
> caller derives its call from that one parsed signature.**

Not a preference — a measurement. The architecture probe pointed loft's proposed
caller at deliberately wrong signatures and **nothing failed**: an arity-1 symbol
called through an arity-3 trampoline returned the right answer (extra register
arguments are ignored), and a variadic function through a non-variadic one
returned the right answer too, by luck. There is **no runtime signal**
distinguishing a correct binding from a wrong one, so nothing can reconcile a
second source of truth against the first. One authority, checked at compile time,
or none.

Forced, not chosen. `#native` is authoritative in the other direction — the
`#[loft_native]` macro reads the real Rust fn and generates the marshal bridge, so
the declaration can be loose. A `#c` library has no Rust to read.

**Re-assertion sites, counted before any code:** the interpreter's trampoline
choice, the `--native` `extern "C"` emission, the compile-time check, and each
target. Omission is **silent at every one**, so `N × silence` was the whole risk
of this plan and the design collapses N to one parsed `CSignature`. That risk
report is the single most useful thing the plan produced — arcs D, E and G each
found a site nobody had listed, and each one was silent.

## What the measurements changed

Ten findings, in the order they arrived. Each one is here because the design said
otherwise first.

1. **The trampolines work for arguments and not for returns.** Every
   integer-class argument crosses correctly at arity 0/1/6/7/12, across the SysV
   register/stack boundary, with no libffi. But `int32_t` returning −1 read back
   as `u64` is **4294967295** — SysV leaves the upper half unspecified. Not a
   crash: a plausible large positive, the shape that silently defeated
   `loft-libs-net`'s `server::listen`. So the declaration must carry the C return
   width, and the "spell the C type" option became the only one left.
2. **The pointee spelling is not needed for emission.** Every pointer is `*const
   c_void` — pointers share one ABI whatever they point at. The spelling stays for
   diagnostics, and earns one decision: a pointer return bound to `text` must be
   spelled `char*`, because `void*` bound to `text` is either a mistake or a handle
   that wanted `integer`, and nothing at runtime tells them apart.
3. **"Inert" was not inert.** A `#c` call under `--interpret` compiled and
   returned a plausible wrong number — `strlen("hello")` answered **7562**. A
   program correct on one backend and silently wrong on the other is the
   divergence class the ship gate exists to catch, so the interpreter refused the
   call until arc B could make it.
4. **A shape that works on one backend only is worse than one that works on
   neither.** Arc C had written half a `char *` return that no test covered; arc
   B's first attempt at the same shape SIGSEGV'd the interpreter while rustc
   rejected the native side — two failures for one gap. It was refused at the
   DECLARATION, on both backends, until arc D built it.
5. **`text?` is the right spelling for a `char *` return, and was refused.** The
   arc-A check rejected it with a message written for the ARGUMENT direction. The
   two directions are not symmetric: loft can hand C no value for a null, so a
   nullable argument is unrepresentable — while C's NULL *return* is exactly "no
   string", and `text?` is the one spelling that lets null-flow SEE it. Declared
   `text`, the same NULL still arrives, silently, as the content sentinel.
6. **Three defects the fixture found that libc could not, all the same shape:**
   *other code claiming a `#c` definition*. The handle convention was half-built (a
   pointer RETURN could be held as an `integer` but not passed back as a pointer
   ARGUMENT); the auto-native driver claimed every body-less definition and
   generated a cdylib exporting bridges for symbols that live in C; and the script
   classifier read a `#c` library as a beginner script, because it consumed one
   string per annotation and `#c` takes two. **A new kind of definition has to be
   excluded from every path that pattern-matches on *body-less*, and the paths do
   not announce themselves.**
7. **The attribution rule for `c_library_available` was wrong in the direction
   that mattered.** Listing a package's symbols under each of its libraries is not
   conservative, it is useless: a package binding sqlite AND duckdb reports
   *sqlite* unavailable because a duckdb symbol is missing. A symbol is
   attributable only when its library is the only OPTIONAL one its package
   declares — and "optional" was itself a correction, because a `#c` package almost
   always ships a `[c] shim`, which registers as an ordinary library and so read as
   a competing one.
8. **Process-global state does not reach a package cdylib.** The runtime tables
   were static, which is right in the interpreter and in a `--native` binary and
   wrong in the third configuration nobody had listed: an auto-built package cdylib
   links its own copy of loft. `duckdb_available()` answered **false** from inside
   the package while the identical call from the program answered **true**, on the
   same run. **"Both backends" is not the same as "every linkage unit."**
9. **wasm has a C ABI, and that was the problem.** The plan wrote it off as having
   none. `wasm32-wasip2` links a libc, so `#c "strlen"` resolved, LINKED with only
   a warning, and trapped at the call — `signature_mismatch`, `(i32)->i64` against
   the sysroot's `(i32)->i32`, because wasm32 is a third data model (ILP32) while
   the extern carried `CTarget::host()`'s widths. One of the targets is not the
   host, and no re-assertion site said so. Two neighbours were equally undefined: a
   symbol the sysroot does not export gave a raw `rust-lld: undefined symbol`, and
   a package declaring `[c] optional-libs` gave `E0433: cannot find c_call in
   loft`, once per symbol, **for bindings the program never called**.
   The cell that decided the fix was a fourth: `c_library_available` — the query a
   library is *told* to ask before calling into an optional backend — did not
   compile under `--html`, because its tables were emitted only on non-browser
   targets. **A refusal that names a cure has to leave the cure reachable.**
10. **A `#c` binding escaped the sandbox's FFI ban.** A sandboxed script reaching
    one tagged `db#read`, under a profile granting `db#read` with `native_ffi` at
    its default false, was admitted and ran the C. Both the ban and
    `reachable_ffi_bridges` key on `def.native()`, which arc A leaves empty *on
    purpose* — the inverse of finding 6, where paths matching on *body-less*
    wrongly CLAIMED a `#c` def. Here a path matching on `#native` silently
    ADMITTED one. The rule: **a capability grant says what DATA a script may touch;
    it cannot say "and arbitrary machine code may run in this process".**

## Decisions taken, and what they cost

**`#c` is the declared edge of loft's totality (C80).** C traps or corrupts; loft
does not model that. The concession is smaller than it sounds: arguments cost
nothing (non-null is already the default and null-flow refuses a `τ?` without a
discharge, so the illegal call does not compile), a NULL pointer return maps to
loft null, and a fault *inside* C is the failure mode `#native` already has,
through the same crash handler. A scalar return that legitimately collides with a
sentinel — `i64::MIN`, `NaN`, 255-as-a-bool — cannot be bound directly and gets a
shim. Isolation for C that cannot be trusted is @PLN119's, not this plan's.

**Complexity in the shim, not in loft-core.** Every signature the fixed
trampolines cannot express routes to ANSI C the library ships, and each wrapper is
three lines. That trade only holds because loft can BUILD those lines — a shim
loft cannot compile is a claim, not an escape hatch, and the author's alternative
would be the rustc toolchain `#c` exists to avoid. A built shim registers
**exactly like a `[c] libs` entry**, so nothing downstream can tell them apart and
the `dlopen`, the link line and the resolver stay one code path — one fewer site
for `N × silence`.

**Parity means "a defined answer in every column", not "works everywhere".**
`#native` does not get the browser for free either — it needs a hand-built bridge
crate. That is what made arc E affordable to state honestly, and why a named
refusal is a green cell rather than a gap.

**The static `clang --target=wasm32-wasi` route stays unbuilt.** No C
cross-compiler was available to prove a single cell of it, and it reaches only a
pure-computation shim — never a database client, whose capability does not exist
in a browser at all. It would change the refusal into a build step; it does not
change the refusal's shape. The manifest key it needs lands with it, because a key
with one legal value is not a choice. @PLN119 (out-of-process) is the route the
refusal message names.

## See also

- [PACKAGES.md § Direct C binding](../../PACKAGES.md) — the reference home.
- [LIBRARY_NAMING.md](LIBRARY_NAMING.md) — design, not built; tracked as PKG.CNAME.
- [NATIVE.md](../../NATIVE.md) — the Rust path this was measured against.
- [SANDBOX.md § S2](../../SANDBOX.md) — the `native_ffi` gate `#c` sits behind.
- [`tests/fixtures/c_abi/`](../../../../tests/fixtures/c_abi/) — the fixture and
  the architecture probe, with the measured numbers.
- [@PLN23](https://github.com/loft-lang/plans/issues/23) — the first consumer
  (four SQL backends over `#c`); [PLATFORMS.md](../23-db-clients/PLATFORMS.md) is
  its per-platform ladder.
- [@PLN24](https://github.com/loft-lang/plans/issues/24) — the tracker issue.
