<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# VS_RUST.md — what sets loft apart from Rust, and what it lacks

This page answers one question for the maintainers: where loft stands against Rust, at the
level of what a program can rely on rather than syntax.  The user-facing page
[`doc/00-vs-rust.html`](../00-vs-rust.html) compares the syntax and is built from
`tests/comparisons/*.loft`.  Every claim below names the check behind it: a probe program, a
guard or the doc that owns it.  A claim without a check has no place here.

A probe is a few-line program of the shape quoted in its row; each gives the same answer on the
interpreter and on `--native` (the C backend).

## What sets loft apart

| # | Claim | Checked by |
|---|---|---|
| 1 | **The heap is a database.**  Stores, keyed `hash`/`sorted`/`index` collections and store images (`store_persist_copy`, `store_load`) are part of the language.  A collection can be bound to an image or to `sqlite:` and fetch on a miss. | `default/02_files.loft`; [LAZY_STORES.md](LAZY_STORES.md); [DATABASE.md](DATABASE.md) |
| 2 | **Stored data survives a changed definition.**  A store written under one struct layout loads under the next: a new field takes its default, a removed one is dropped, a removed variant reads null.  A rename is two steps and a one-step rename is refused. | `tests/store_upgrade.rs`; [STRONG_POINTS.md § 2](STRONG_POINTS.md) |
| 3 | **A calculation fault is a value, not a stop.**  Overflow (`i64::MAX + 1`), an index out of range (`v[99]`) and division by zero each give null and the program runs on.  A text parse must be discharged where it is written (`"abc" as integer ?? -1`): a bare `as integer` is a compile error. | a probe of the quoted shape, both backends; [@C80](DESIGN_DECISIONS_FAILURE.md) |
| 4 | **An explicit failure stops only during development.**  A failed `assert` or a `panic` halts a normal run, including a `--native --ship` build; under `--production` it is logged and the program continues. | a probe of the quoted shape with and without `--production`; [LOGGER.md § Production Mode](LOGGER.md); [@C66](DESIGN_DECISIONS_FAILURE.md) |
| 5 | **One source, every target, through two backends.**  The interpreter (and the REPL) and the C backend; wasm (`--native-wasm`), a browser page (`--html`), Android (`--native-android`) and ESP32 (`--native-esp`) all build through C.  A backend stays only while it answers every program as the others do. | `loft --help`; [@C170](DESIGN_DECISIONS_PLATFORM.md); [C_BACKEND.md](C_BACKEND.md) |
| 6 | **No lifetimes and no capture modes.**  A struct or collection parameter is shared with the caller; `&` adds whole-value replacement; `const` refuses every write through the name.  A closure picks its capture from the type. | a probe of the quoted shape; [00-vs-rust.html § 3, § 11](../00-vs-rust.html) |
| 7 | **A type receives the parts of `"{…}"`.**  With `lit` and `hole_<kind>` the target type gets the literal text and each hole separately, so a SQL or HTML builder never sees a spliced string. | `tests/reference/interpolation-values.loft`; [INTERFACES.md](INTERFACES.md) (@PLN124) |
| 8 | **Parallel loops and generators are built in.**  `for x in v par(b=f(x), n)` and `yield`/`iterator<T>`, with no crate and no async runtime. | a probe of the quoted shape, both backends; [THREADING.md](THREADING.md); [COROUTINE.md](COROUTINE.md) |
| 9 | **Scripts can be admitted into a host under a capability policy.** | `src/sandbox.rs`; [SANDBOX.md](SANDBOX.md) (@PLN86) |
| 10 | **Data layout is chosen in the source** (narrow widths `u8`…`i32`, placement), and a `--ship` build drops the fault checks for speed. | [LOFT.md § Primitive types](LOFT.md); [NATIVE.md § Optimisation tiers](NATIVE.md) |

## What loft lacks

| # | Gap | Checked by |
|---|---|---|
| 1 | **No compile-time proof against aliasing.**  `const` covers the name it marks; another name for the same struct still writes it, and one struct passed to two parameters of a call is aliased inside it.  No two `par` workers write one store: each reads an isolated clone and is refused a captured reference.  A worker that prints or touches a file has an undefined result (`@FR-C-Impure`), and nothing checks for it.  The compiler checks its OWN output (the store-records checker reports a leak, double free or out-of-scope use as a compiler defect), but nothing proves a user's aliasing safe the way the borrow checker does. | a probe of the quoted shape; [@PLN231](plans/231-against-rust/README.md) probe 16; [STORE_RECORDS.md](STORE_RECORDS.md) (@PLN228) |
| 2 | **No moves.**  A whole-value bind copies (`b = a; b.x = 5` leaves `a.x` at 1), and there is no consuming `self`: a method's receiver is shared with the caller. | a probe of the quoted shape |
| 3 | **No dynamic dispatch on interfaces.**  `x: Named = A{…}` is refused (an interface only constrains a type), an interface method cannot have a default body, and an interface cannot extend another.  Enum variants do dispatch per variant. | a probe of the quoted shape; [INTERFACES.md § What is out of scope](INTERFACES.md) |
| 4 | **No metaprogramming.**  The language has no macro, `derive` or procedural-macro facility; the lexer's keyword table has none. | `src/lexer.rs` `KEYWORDS` |
| 5 | **No lazy iterator chain in the standard library.**  `map`, `filter` and `reduce` take and return a `vector`, so each stage builds a new one.  The language can do it: a generic generator adapter (`fn lmap<T, U>(it: iterator<T>, f: fn(T) -> U) -> iterator<U>`) is lazy on both backends, so this is a library gap. | `default/01_code.loft`; [@PLN231](plans/231-against-rust/README.md) probes 10, 11 |
| 6 | **Two closure limits.**  A capturing closure cannot be stored in a collection (a struct field holds one); a `par` worker is a named function and refuses a captured reference: only scalars and the loop element reach it. | a probe of the quoted shape |
| 7 | **Slower than `rustc -O` on most routines.**  The C backend's median is 1.43× the Rust reference over 220 routines, 4 over 3× and 44 at or under 1×; the `--ship` build's median is 1.30×, none over 3×.  Keyed collections and short-lived temporaries are the slow classes. | `bench/portal/results/laptop.tsv` (one x86_64 machine); [PERF_PORTAL.md](PERF_PORTAL.md) |
| 8 | **A small ecosystem, and no access to Rust's.**  A library's native part is a C shim bound by `#c`; one written as a Rust crate is refused.  The registry holds dozens of libraries, not tens of thousands. | [LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md); [plans/225-retire-rustc-writer](plans/225-retire-rustc-writer/README.md) |
| 9 | **The compatibility promise is not in force yet.**  [COMPATIBILITY.md](COMPATIBILITY.md) promises that a working program keeps working from `contract` 1, the 1.0 freeze, and that freeze has not been declared. | [COMPATIBILITY.md](COMPATIBILITY.md) |

## Absent on purpose

These look like gaps from Rust's side and are decisions; reopening one is a design question,
not a missing feature.

- **No `Result` and no unwinding.**  A failure is a value (null), discharged with `??` (my
  default) or postfix `?` (the type's default) — [@C80](DESIGN_DECISIONS_FAILURE.md),
  [LOFT_OPERATORS.md § The `x?` operator](LOFT_OPERATORS.md).
- **No user-visible borrow checker** — ownership is internal ([@C79](DESIGN_DECISIONS_OWNERSHIP.md)).
- **No `x[i]` on a user type**: `[…]` belongs to the built-in collections, a type reads through
  a named method ([INTERFACES.md § Indexing](INTERFACES.md), @C132).
- **`==` is always structural**; no type defines its own ([@C134](DESIGN_DECISIONS_SYNTAX.md)).

## Keeping this page true

A row changes when its check changes.  Before restating a row elsewhere, re-run its probe or
guard on a fresh build of `main`; when a limitation stops holding, delete the row and update
[`00-vs-rust.html`](../00-vs-rust.html)'s cell in `tests/comparisons/` in the same commit.
