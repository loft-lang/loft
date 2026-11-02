<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 91 — Self-hosting epic: loft-in-loft, bundle-modded core, ANSI-C backend, SBC portability

## Status

**The objective (owner, 2026-11-01): an interpreter written in loft with no Rust code left.**
The front end, the bytecode writer and the interpreter are loft, compiled through the C
backend and running on the C runtime (`src/c/`), with no rustc dependency at all.  The C
runtime is the one part that is not loft.  **Self-hosting is the first objective, so 1a
goes first again:** the lexer and the parser in loft, now; 1b's open items wait behind them.

The path, in order, with the gate each stage passes before the next starts, is
[§ The path](#the-path--stages-and-gates).  **Where it stands:** 1b's interpreter runs
766 of the 1,078 corpus programs (the latest handoff, `handoff-*.md` beside this file,
has the census); 1a starts now, with the lexer.

**One rule for every stage — a reimplementation, not a new language (owner,
2026-10-30):** the same IR, the same bytecode, the same memory structures, the same
operator semantics, and from strand 12 the same C.  A difference from the Rust
implementation is a defect to fix, never a decision to take.  The implementation's
intricacy (the scope pass, the ownership oracle, the bind decisions) is the price of
combining ease of use with optimal code, so it is ported faithfully, not redesigned.
Strand 1b: [1b-interpreter.md](1b-interpreter.md); ownership: [ownership.md](ownership.md).

**Beside the path:** the bundle strands (2–4) and the small-board strands (6–7) are goals
of their own that the path enables; nothing on the path waits for them.  This README is
the single source of truth for per-strand status; `@PLN91` carries the summary + labels.
Superseded text: [README-history.md](README-history.md).

## Goal

**The objective:** loft's compiler, interpreter and tool written **in loft itself**,
compiled to C and running on the C runtime, with no Rust code and no rustc dependency
left (strands 1a, 1b, 8–12).

**Beside it:** restructure the core so the language is composed from drop-in,
scanner-wired **bundles** (closed curated core, additive registration seams, no editing
canonical `src/`; strands 2–4), and a **build-time portability config** — a bounded /
freestanding **store profile** (with a narrowed `DbRef`) plus a configurable default
integer width — so loft targets small single-board computers (strands 6–7).

## Effort + design

- **Effort:** VH (an epic; each strand is its own M–H plan once split out).
- **Design:** ~ (strands + ordering + falsification probes defined; per-strand detail pending).
- **Last touched:** 2026-11-01.

## Composition matrix — Stage A

This spine plan adds **no** new value/type/operation surface itself. The genuine
composition surface lives in child strands and is matrixed when *those* are designed:
strand 5 (ANSI-C) is a 4th emission target under [Goal D](../../GOALS.md) parity,
policed by the differential oracle (@PLN89); strands 6–7 (int width, narrowed
`DbRef`) cross every numeric op, the null-sentinel model, and every store access. The
de-risking probes below are the Stage-A spec for the spine.

## Why this is reachable today

- **Bootstrap exists:** the `loft→Rust→binary` native backend is the stage-0 compiler.
- **stdlib is already data-driven:** `default/*.loft` is the base "bundle" in all but name.
- **Opcodes are already generated:** `fill.rs` is emitted by `regen_fill_rs` — a declaration-fed codegen seam.
- **Self-host-friendly by intent:** the `IrNode` handle was retained "for self-hosting value" (DESIGN_DECISIONS.md:931).
- **Bundle pattern proven downstream:** the crawler's bundle system; the library/registry system is the same pattern for the loft-code layer.
- **Embedded idiom already writable:** fixed-capacity collections (pre-sized array + count + index-write, the loft#320 pattern) are proven in crawler loft today.
- **The C backend exists (P-cemit is past):** `src/c/` on the @PLN200 B5 branch emits
  ANSI C beside the Rust writer, checked by `scripts/c_sweep.py` on three macro modes with
  ASan and UBSan; 1789 tests pass across the modes and the sanitizers (owner, 2026-10-24),
  the refusals concentrated in file and system I/O, generators, `&` locals, `par`, native
  library calls, spatial and trie.
- **The IR is already written in loft:** `selfhost/src/ir.loft` (moved from `tools/ir_schema/` in 1b S0.3) transcribes the complete
  IR as loft `struct`/`enum` declarations (hand-mirrored from `src/data.rs`), and the IR
  codec round-trips the stdlib, every corpus file and the language suite
  (`cargo test --release --test ir_schema_roundtrip`) — so a front end written in loft has
  a defined thing to produce and an exact oracle to be judged by.
- **The front end has been written in loft before, at small scale:** `tools/fmt` is a
  surface lexer and parser in loft (1.6k lines), the indexer and the viewer are 2k lines
  each, and @PLN179 ports the repo's scripts with a findings register of what the language
  lacked each time.
- **A second emitter now has a defined input:** @PLN200 separates `opt` (what every backend
  asks) from the writers, and its B3 makes `lower` produce data with no Rust text in it.

## Sub-arcs

| Item | Status | Notes |
|---|---|---|
| **1a — The compiler in loft** | Open — **the trunk, first** (owner, 2026-11-01) | Lexer, parser, scopes and types in loft, emitting the codec IR (`selfhost/src/ir.loft` is the type vocabulary), compiled through the C backend. Gate: the token stream, then a byte-identical image diff against the Rust front end over the corpus and the language suite; the measurement is HOW MANY programs agree (§ The path, stage 1). **The AST holds its children in a `vector<Node>`, in a keyed collection (records that do not move), or as arena indices — never `&v[i]` into a vector that grows** (loft#2163: the interpreter reads null, `--native` the moved record's old bytes). The bar rows 1a uses, measured, and the steps that close them: @PLN202. |
| **1b — The runtime in loft** | Open — 766 of 1,078 corpus programs run; waits behind 1a's lexer and parser (owner, 2026-11-01) | The interpreter in loft, reading the image as `ir.loft` records — the Rust front end's until 1a's replaces it; designed one step at a time in [1b-interpreter.md](1b-interpreter.md), with ownership as track O ([ownership.md](ownership.md)). Complete when its bytecode is byte-identical (F-2) and every comparison layer agrees (§ The path, stage 2). Pulls strands 2–4 with it. |
| **2 — Bundle/mod system for the core** | Open | Convert central switches (`fill.rs` opcode match, parser operator/precedence table, native registry) into additive scanner-wired registration seams. No `src/` edits to extend. |
| **3 — Migrate types/structures into bundles** | Open | Base language = the default bundle set; no privileged core path; enables subsettable / "limited" loft builds. |
| **4 — Opcodes-as-bundles** | Open | A *consequence* of (1): once the VM loop is loft, an opcode = name + native body + loft body, all bundle data. `regen_fill_rs` is the seam. |
| **5 — ANSI-C backend, replacing rustc** | **Under way** — @PLN200 B5 | Minimal portable bootstrap floor; auditable output (no Rust supply chain in the artifact); reach — and the fast compile 1a wants for itself. Measured 2026-10-24: on a hoisted loop the emitted C and the emitted Rust do the same work per element, and the remaining gap between them is the compiler (gcc absorbs loft's negative-index fold, LLVM pays 2× for it), cured once in the shared decisions, never per backend. It is the oracle strand 12 is diffed against and the compiler that builds stages 1–2; it is retired with the rest of the Rust at stage 6, once strand 12 replaces it. Still owed until then: the stdlib's 355 `#rust` bodies reach C only through the generator's routine table (`src/c/*.rs`), and wasm/html/android are Rust-only. |
| **6 — Configurable default int width** | Open | Build-time width knob (incl. `DbRef` field widths). Co-design with @PLN88 (i64 unification) — see open questions. |
| **7 — Bounded/freestanding store profile** | Open | **The real SBC door — *and* a mainline native-perf refactor.** Fixed static arena (no grow), freestanding/`no_std` runtime, and the narrowed-`DbRef` static-store binding below (narrower refs + static store resolution speed up generated code on *every* target — justified independent of C54.F). C backend + widths are its *enablers*, not the door. |
| **8 — The `loft` tool in loft** | Open (owner, 2026-11-01) | Everything the `loft` command does besides running a program, each part ported to loft: the driver and its options, the `loft test` runner, the package manager (`loft install`, the registry, lock files), the debugger (`loft debug`, `--rpc`), the REPL, the language server, the doc generator (`gendoc`) and the exports (`--html`, wasm, Android). Each part is diffed against the Rust tool's output on the same input before the Rust part is removed. |
| **9 — Every native in loft; the C runtime for system integration** | Open (owner, 2026-11-01) | The rule: what can be written in loft is loft, and the C runtime (`src/c/*.h`) is the layer that integrates with the system — files and mapped files, processes, sockets and TLS, clocks, threads for `par`, and the `#c` bindings to system libraries. Today's Rust-only natives move to loft: `n_json_parse`, `n_stack_trace`, `n_parallel_queue`'s scheduling, the store-image sidecar (`layout_dump`, `layout_algo_hash`, `render_facts`), the rest of `src/native.rs`, and the table in `src/c/*.rs` that maps each of `default/*.loft`'s 355 `#rust` bodies to a C routine (it goes with the C generator into strand 12). A computing routine the C runtime holds today is a candidate to move to loft, measured against the C before it moves. |
| **10 — Libraries without Rust** | **Nearly there** (surveyed 2026-11-01) | Every published `loft-libs-*` library is loft, with C only where it reaches the system; no Rust native bridge and no cdylib built by rustc. Of the 47 packages in the eight repositories, 35 are loft only; 9 ship their own C99 (`[c]`: crypto, random, regex, graphics, imaging, tiles, audio, input, screen); 3 (web, server, ssh) call the `net` component, which is C over the runtime's own HTTP, TLS, WebSocket and SSH code (`src/c/components/net.c`). Rust left: `loft-libs-net/plugin_runtime`, not a package — a staging area for @PLN84 ZT-E whose host loader (on `wasmtime`) and wasm shim are Cargo crates; the `bench/bench.rs` Rust twin in 38 packages (the perf portal's reference, not shipped); and the library CI (`library-ci-reusable.yml`), which builds loft with cargo and runs a `--native` (rustc) leg beside the mandatory `--native-c` one. The component loader (`src/components.rs`) and its builder (`src/c/component.rs`) are loft's Rust, strands 8 and 9. |
| **11 — The tooling in loft** | Open (owner, 2026-11-01) | The selfhost's harness (`tests/selfhost.rs`, a cargo test) becomes a `loft test` package and `selfhost/compare/compare.py` a loft program. The Rust interpreter stays the comparison's oracle until it is retired; after that the comparison is against the language suite's recorded answers. |
| **12 — The C generator in loft** | Open — after the interpreter is complete (owner, 2026-11-01) | Its own set of steps.  The loft-written compiler generates C byte-identical to what the Rust generator writes for the same program, so every optimisation is ported with it: a decision left out is a different unit.  **G-0 — the oracle:** the `--emit-c` unit per program diffed byte for byte (a `c` layer in `selfhost/compare/`), the measure HOW MANY programs of the corpus and the language suite agree.  **G-1 — the decisions** (`src/opt/`, about 20.5k lines: `hoist`, `range`, `writes`, `push_window`, `store_text_borrow`, `apart`, `value_records`, `tail_self`, `ret_adopt`, `non_sentinel` and the rest), each pass's admissions and refusals diffed against `make rewrite-census` / `make decline-census` and its `LOFT_TRACE_*` lines before its text is.  **G-2 — the writer** (`src/c/`, about 24k lines: `mod.rs`, `body.rs`, `body/` — keyed, value, transient, deferred, apart, par, generator, files, chain), from the simplest program up, the unit byte-identical at every step.  **G-3 — building:** the shim and link driver (`src/c/component.rs`, `c_driver.rs`), which spawns `cc` — process integration from the C runtime (strand 9).  The IR passes `close_program` runs for both backends (`compile.rs`: `ledger_sites`, `arg_copy`, `value_record`, `leaf_inline`, `tail_self`, … `text_len`) come earlier, in 1a, since the bytecode depends on them too.  The C runtime headers are C already and are not ported. |

## Small-board store profile (strand 7 — the design)

The unfit-for-bare-metal properties are **profile knobs of the existing model**, not a
redesign:

- **Growth → fixed-capacity arena.** `grow_words`/`claim_grow` *trap* at a build-set
  ceiling instead of reallocating; the free-tree still operates inside the fixed buffer.
- **Hosted/std runtime → freestanding profile.** The core store is integer arithmetic
  over a byte buffer (inherently `no_std`-able); `dirs`/`ureq`/`rayon` are already
  optional features. Drop filesystem (`02_files`), threads (`par`), heap-growing
  collections (subset them out via strand 3); I/O via a board hook, not stdio.
- **General-purpose free-tree → optional fixed-size pools** for hard-real-time
  (soft-real-time: the bounded coalescing tree is fine).

**The `DbRef` refinement (the intended core change).** Treat a reference as carrying
**only the coordinates the compiler cannot resolve statically** — drop whatever the
type/variable binding already pins. Today `DbRef = (store_nr, rec, pos)` is uniformly
self-describing (3×i32, 12 bytes); the small-board profile makes the width a
**per-reference-site static choice**, in three tiers:

| What the type/binding pins | Reference carries | Bytes |
|---|---|---|
| Store known **and** target is always the store **root** | nothing — the reference *is* the statically-bound store; the root sits at a fixed position | 0 (a bare store handle only if the store identity is itself dynamic) |
| Store known, target varies within it | `(rec, pos)` | 8 |
| Store **not** statically known (cross-store — the edge case) | `(store_nr, rec, pos)` | 12 |

Decided at **compile time per site** (not a runtime-tagged union): the common case
pays nothing, there is no branch on deref, and it rides loft's existing per-type
field-width packing, so mixed-width `DbRef` fields need no new storage mechanism.
**Most loft code functions exactly as today** — a build profile + static resolution,
not a semantics change — and it is the pointer-level realization of strand 6 (narrower
still with configurable widths). Dynamic multi-store features (@PLN43, @PLN15) ride the
12-byte tier rather than being dropped.

**Mainline performance — this justifies strand 7 independent of C54.F.** The untangling
speeds up the *common-case generated code on every target*, not just SBCs:

- **Narrower references** (12→8, or 0 for root-bound) shrink every reference-bearing
  struct, vector element, and fn-ref slot — less memory traffic, better cache density,
  unconditionally.
- **Static store resolution** turns the per-deref `store_nr → Stores → Store` lookup into
  a **compile-time constant**: one fewer indirection per dereference, and the optimizer
  can register-pin a store base and prove cross-store non-aliasing. *This deepest win is
  contingent on store access being inline-able, so it **compounds with self-hosting** —
  where the runtime is emitted code, not an opaque `loft-ffi` boundary.*
- **Likely secondary win:** one static-resolution path can retire the per-shape `DbRef`
  marshalling special-cases (the @P251/@P238 family) → simpler, smaller emitted code →
  faster rustc compile + smaller binary.

Honest trim on "all cases": tier-3 cross-store refs keep the dynamic lookup; the
resolution analysis is added compile-time work; the win scales with deref-density (large
for store-bound code — i.e. most loft — small for scalar-bound). Net: strand 7 pays off
on the flagship native target even if C54.F never re-opens and no MCU is ever a target.

## The path — stages and gates

Each stage is judged by a byte-identical diff against the Rust implementation before it is
judged by behaviour, and measured as HOW MANY programs of the corpus and the language suite
agree.  The comparison instrument is `selfhost/compare/` ([1b-compare.md](1b-compare.md)).

1. **The front end in loft (strand 1a).**  Lexer → parser → scopes and types → the IR
   passes `close_program` runs for both backends (`compile.rs`: `ledger_sites`,
   `arg_copy`, `value_record`, … `text_len`), compiled through the C backend.  Gates: the
   token stream equals `src/lexer.rs`'s; the image equals the Rust front end's
   `--emit-image`, before and after the IR passes.  **The parser is written with PEG match
   patterns** (@PLN35, [formal/matching.md](../../formal/matching.md)) over a token cursor
   (owner, 2026-11-01).  The tokens may come from a lexer in the C runtime: the runtime
   already ports `src/lexer.rs`'s JSON mode (`loft_json.h`, `loft_jl_*`), and that port is
   extended to the full loft mode wherever the parser needs it.  P-front is this stage's probe: what
   the language lacks is filed on @PLN179's register and fixed in the language, never
   worked around in the compiler.
2. **The interpreter in loft, complete (strand 1b, with strand 9).**  It reads the loft
   front end's image.  Its lowering becomes the port of the bytecode writer
   (`src/state/codegen.rs`, [1b-frame.md](1b-frame.md) F-2): the operators numbered by
   `Data::op_code`'s rule (their `default/*.loft` declaration order within the
   `#hot` / normal / `#cold` classes) and the bytecode byte-identical.  The natives that
   exist only in Rust today move to loft or, where they reach the system, to the C runtime
   (strand 9).  Gate: every layer of the comparison equal — opcodes, bytecode, slots,
   binds, out, alloc, leak.  **Speed is out of this stage's scope** (owner, 2026-11-01):
   the aim is an interpreter comparable in speed to the Rust one, and it rests on the C
   optimisation layer (the shared decisions in `src/opt/` and the C writer), whose target
   is every individual routine below 3× its rustc version (@C136) — work that continues
   throughout the project on its own, outside the interpreter's implementation.  The
   interpreter is written as the faithful port, never hand-tuned in its source or its C.
   P-perf measures the inner loop against the Rust interpreter, as input to that layer; it
   does not decide whether 1b happens.
3. **The C generator in loft (strand 12).**  `src/opt/`'s decisions and `src/c/`'s writer,
   the C byte-identical to the Rust generator's for every program.
4. **Bootstrap.**  Stage 1: the loft-written compiler and interpreter, compiled to C by the
   Rust generator.  Stage 2: the same sources compiled by stage 1.  Gate: stage 2's C for
   itself equals stage 1's.  From then on loft builds from its own committed C with
   nothing but a C compiler.
5. **The tool and the tooling (strands 8 and 11).**  The driver, `loft test`, the package
   manager, the debugger, the REPL, the language server, `gendoc` and the exports, then the
   selfhost harness and the comparison as loft programs; each part diffed against the Rust
   tool's output on the same input before the Rust part is removed.
6. **Rust retired.**  The library leftovers (strand 10: `plugin_runtime`'s crates, the
   library CI's cargo build and rustc leg, the `bench.rs` twins), the rustc backend
   (`src/generation/`) and every `src/*.rs` file removed; no Cargo in the build.  Gate: loft
   built from C alone, its full suite green on the interpreter and the C backend.

**Beside the path:** strands 2–4 (bundles) ride stage 2 — 4 falls out of it.  Strand 7 is
the small-board lead, with 5 and 6 as its enablers; its probes P-footprint and P-dbref-perf
are its own.  P-cemit is done (@PLN200 B5, `scripts/c_sweep.py` the diff).

## Honest floors + costs (do not design these away)

- **Irreducible bootstrap floor remains** — after stage 4 it is a C compiler and loft's own committed C; never zero.
- **Special types keep a kernel impl** — `text`/`vector`/`integer` move their *declaration* to bundles; impls touch opcodes/IR until self-host pulls them up.
- **Backend parity is ongoing cost** — interpreter / C / wasm must match (Goal D; @PLN89 polices it); the rustc backend leaves at stage 6.
- **Until stage 6 the stdlib has two authorings** — the 355 `#rust` bodies beside the C routines the generator maps them to; they go with the rustc backend.
- **A compiler is the heaviest consumer of what the bars score lowest** — the OCaml bar reads closures 2/6, nullability 0/2, pattern depth 6/8, and @PLN179's register lists what tooling in loft hit (a character loop to tokenise, no set of scalars, no membership test, no `exit(code)`). 1a will surface these faster than any consumer; each is a language fix, not a compiler workaround.
- **@C89 vs @C99 is a real fork** — @C99 (`stdint.h`, `long long`) maps i64/UTF-8 cleanly; true @C89 forces width typedefs + config. Decide before strand 5.
- **The C optimisation layer is the speed lever** — the interpreter in loft is as fast as the C it compiles to, so its ratio to the Rust interpreter moves with `src/opt/` and the C writer, then with their port (strand 12); reach, trust and bootstrap are the other half of the case.
- **`DbRef` is used everywhere** (keys, store, `deps`/lifetime, fn-ref slots, vector/text addressing) — the narrowed profile must keep the `deps` system working at 8 bytes; it is a parameterization of an existing abstraction, but a broad one.

## Open design questions

1. **Int-model reconciliation (6 ↔ @PLN88).** @PLN88 unifies to ONE i64 range; strand 6
   wants a build-time-configurable (possibly narrower) default. Resolve as: i64 canonical,
   width a build knob that narrows it. Design together; do not let them conflict.
2. **Mod replace/override policy for the core.** Can a bundle replace `OpAdd`? shadow
   `len`? last-wins / explicit `override` / error? (Security/stability gate for a *language*.)
3. **Bundle dependency + load order for the core stdlib** (acyclic; missing-key = hard
   error), riding the @PLN11 stdlib cache (delivered; @PLN52 closed) so startup doesn't regress.
4. **Trust boundary** of a native (`#rust`/C) opcode body vs a loft opcode body (the
   latter sandboxable) — ties to the zero-trust line and @PLN86.
5. **Store-binding granularity (strand 7).** Is the store fixed per *type* (coarse,
   simplest) or per *variable/field* (finer, needs the type system to carry store
   annotations)? Generic containers (`vector<T>`) complicate type-level binding.
6. **Reference-width expression (strand 7).** The three tiers (root / intra-store /
   cross-store) are a per-site *static* choice — multi-store features are not forgone,
   they ride the 12-byte tier. Open detail: the type-system surface marking each tier (a
   store-qualified / root-bound reference type?), the fixed-position **root convention**
   per store, and confirming @PLN43/@PLN15 ride the wide tier. Validate tier prevalence
   against the crawler.
7. **C54.F re-open (model vs profile).** Frame the re-open of "store model is the wrong
   shape for bare-metal" as *the hosted/growable profile is unfit, the model is not* —
   with P-footprint as the new evidence the design register requires.
8. **What 1a emits (owner).** The codec IR as the Rust parser produces it today, so the
   diff is exact — or the `lower` data of @PLN200 B3 once that exists, which is smaller
   and backend-neutral but moves the oracle. Start on the codec IR; the diff is the whole
   case for the stage.
9. **How much of the C runtime stays C (strand 9).**  System integration stays C by rule.
   The store allocator, the keyed collections and the text and formatting routines are C
   today and are not system integration: whether each moves to loft is open, measured
   against the C before it moves.

## Cross-arc dependencies

- **@PLN202** (the OCaml bar, for the compiler) — 1a's child: the bar rows a front end uses, measured, and the parser steps that close them.
- **@PLN200** (the interpreter and the code generators as defined parts) — strand 5's home (B5) and the seam 1a writes into (B3's `lower`); B6 is the minimal binary.
- **@PLN179** (scripts in loft) — the findings register 1a files its language gaps on; its measured weights of the collection kinds are strand 3's input.
- **@PLN11** (IR as stores) — `selfhost/src/ir.loft`, the type vocabulary of 1a and 1b.
- **@PLN35** (PEG match patterns) — self-host *enabler* (the compiler's front end).
- **@PLN52** (stdlib fast-start cache) — perf prerequisite for a bundle-ized stdlib; **MET** — delivered via @PLN11 arc D/D2b (opt-in `LOFT_STDLIB_CACHE`); @PLN52 closed 2026-07-09.
- **@PLN86** (sandbox-subset-flag) — enabled by subsettable type bundles (strand 3).
- **@PLN24** (`#c` C-ABI binding + ANSI-C shim) — adjacent C-boundary prior art for strand 5.
- **@PLN89** (interp↔native differential oracle) — extends to police the ANSI-C backend.
- **@PLN88** (unify integer model to i64) — co-design with strand 6.
- **@PLN43** (store durability) / **@PLN15** (cross-branch record refs) — multi-store
  features the narrowed-`DbRef` profile (strand 7) must reconcile with or forgo.

## See also

- [GOALS.md](../../GOALS.md) — Goal D (cross-backend parity); the "trust + forget" aim (strand 5 auditability).
- [BROADENING.md](../../BROADENING.md) — the **C54.F** decision (don't chase MCUs; 32-bit SBC floor) this plan challenges on the model-vs-profile axis.
- [NATIVE.md](../../NATIVE.md) — the rustc backend strand 5 parallels; `loft-ffi` runtime.
- [DATABASE.md](../../DATABASE.md) / `src/keys.rs` / `src/store.rs` — `DbRef`, `Stores`/`Store`, the growth path strand 7 bounds.
- [INTERMEDIATE.md § Adding an operator](../../INTERMEDIATE.md#adding-an-operator) — the `regen_fill_rs` opcode bootstrap (strand 4 generalizes it).
- `../../../../crawler/BUNDLE.md` — the working consumer-side bundle system strand 2 mirrors.
- `@PLN91` — the tracker issue this plan realizes.
