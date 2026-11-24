# Broadening — loft beyond games

Strategic evaluation of where loft can be useful beyond its current
game-centric framing, and what it takes to get there.

Current milestone framing (ROADMAP.md) is "Awesome Brick Buster — a
game worth sharing".  That is a good flagship demo but should not be
the language's identity.  This document records the broader-reach
analysis so the ecosystem work below can be sequenced deliberately
instead of reactively.

---

## Loft's genuine differentiators

Four capabilities that give loft a defensible identity outside games:

1. **Store-based heap as a language-level database.**  Data-heavy
   apps (servers, CRUD tools, ETL) feel coherent in a way
   Python/Ruby/Go cannot match because persistence and in-memory
   state share one model.  See DATABASE.md.
2. **WASM single-file HTML export** (`loft --html`, HTML_EXPORT.md).
   A one-command path from `.loft` source to a shareable interactive
   artifact.  Frictionless deployment of demos, tools, toys.
3. **`par` / `par_light` + store isolation** (THREADING.md).
   Approachable parallelism without shared-mutable footguns.
4. **It binds the native ecosystem through the C ABI.**  loft does not re-implement
   zlib, sqlite, libpng or BLAS — a library binds the C library it needs with `#c`
   declarations and, where the fixed trampolines cannot express a signature, a small
   ANSI-C shim it ships ([PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md)).  The binding is
   built by the C compiler the C backend already uses, so it needs no second toolchain,
   and a published library ships its C, built for each platform.

   The filter is **well-grounded**, not the source language: a decades-hardened C
   library (sqlite, zlib, libpng) is as stable as safe code, while *un*grounded native
   code brings C's crash surface with it — the way a buggy C extension hard-crashes the
   Python interpreter.  So a library binds **one mature C library per real need**, the
   dogfood way, not through a speculative auto-binder.  On functionality alone this is
   the same move Python makes (numpy/scipy are C/Fortran, the C-API is glue); the honest
   gap is binding **ergonomics, not library existence**.  Mechanism: PACKAGES.md + §
   Native-library execution model below.

Everything below flows from these.  Broadening loft is mostly about
ecosystem work around them (chiefly #4 — binding the C libraries, incrementally on
need), not language rework.

---

## Domain fit matrix

| Domain | Natural fit | Gap to close |
|---|---|---|
| **CLI scripting / tooling** | Strong — readable syntax, static types, good error locality | Fast startup (CS.C1/C2/C3 const store + stdlib `.loftc`); stdlib for regex, shell, env, path, glob; single-binary installer |
| **Server-side web** | Very strong — store maps naturally to request/session/DB model; JSON landed | `server` library (lib_plans/future/08-server/README.md), async / non-blocking I/O, route helpers, migrations story |
| **Embedded-DB DSL** | Unique — nothing else has store + language co-designed | Mostly packaging and "SQLite + scripting as one thing" narrative; the tech exists |
| **Data / ETL** | Good — iterators, parallel-for, DbRef, JSON | CSV/Parquet, decimal/BigInt, date/time, streaming — **bindable C libraries** through `#c`, not missing libraries |
| **Educational language** | Good — Python-like surface, strong types, good diagnostics | Playground (Web IDE planned), tutorial content |
| **Scientific / analytics** | Bindable — the C/Fortran numerical stack (BLAS, LAPACK) binds through `#c` ([PACKAGES_C_BINDING.md § Numeric libraries](PACKAGES_C_BINDING.md#numeric-libraries--what-the-boundary-already-does-pln128)); **not** an ecosystem gap | binding ergonomics + the interactive / notebook / viz + community layer (that, not library *existence*, is Python's real moat here); pursue head-on only if a killer differentiator emerges |
| **Small 32-bit devices** | A target ([@PLN204](https://github.com/loft-lang/plans/issues/204)): the C backend builds an ESP-IDF app, `#width 32` keeps 64 bits a deliberate choice | The device port ([TANMATSU.md](TANMATSU.md)); a game demo at 60 fps, then nightly 32-bit sweeps |

Games remain a flagship demo (onboarding, Web IDE, visual appeal) but
become one entry point among several rather than the identity.

---

## What needs to happen

### Tier 1 — adoption blockers

Nothing broadens without these.  They are prerequisites for every
other domain.

- **1.0 stability contract** (ROADMAP.md).  No one builds production
  code in a 0.x language that can still reshape syntax.  Already on
  the roadmap; dominates the critical path.
- **Package registry + lock file** (PKG.7 + REG.1–4, PLANNING.md).
  Without `loft install <name>`, there is no ecosystem.
- **LSP + editor integrations** (SH.1 TextMate, SH.2 VS Code).  A
  language without syntax highlighting and go-to-definition feels
  unfinished regardless of merit.
- **User-facing documentation + tutorial.**  Current `doc/claude/*`
  is Claude-internal (excellent for me, opaque to a newcomer).
  Needs a "learn loft in 30 minutes" plus a cookbook.  Existing
  `doc/*.html` (gendoc) is the starting point.

### Tier 2 — domain-unlocking

Each item opens a specific segment.

- **Fast cold-start** (CS.C1/C2/C3 const store + stdlib `.loftc` +
  lazy-stdlib loading, LAZY_STDLIB.md).  Unlocks CLI scripting.
  Today's startup cost rules out shell-integration use.  See
  CONST_STORE.md, BYTECODE_CACHE.md, LAZY_STDLIB.md.
- **Standard-library breadth:**
  - *Scripting:* regex, date/time, path/glob, subprocess, env,
    logging (✓ LOGGER.md).
  - *Server:* HTTP client + server (lib_plans/future/08-server/README.md), routing,
    TLS/ACME, sessions, CSRF.
  - *Data:* CSV, Parquet, decimal, streaming I/O, compression,
    crypto (hash/HMAC/AEAD).
- **Async / non-blocking I/O.**  Servers without it hit a ceiling
  fast.  `par_light` covers CPU work; network I/O needs its own
  story.  C56 `?? return` + I13 iterator protocol
  (SERVER_FEATURES.md) are partial enablers, not a complete model.
- **Coroutines / `yield`** (COROUTINE.md, planned 1.1+).  Required
  for streaming parsers, generator-based iterators, some server
  patterns.
- **FFI maturity** (FFI.1–4, PLANNING.md).  Opens access to C
  libraries — inevitable for crypto, compression, DB drivers,
  system APIs.

### Tier 3 — narrative and positioning

How people find and remember loft.

- **A "store + language" killer demo.**  A 100-line persistent
  multi-user chat or CRUD app where the store *is* the database.
  This is the pitch nothing else can make.
- **A scripting demo.**  20-line file-processing one-liner that
  rivals Python for readability and ships as a single binary.
- **A server demo.**  REST API + store-backed persistence + JSON in
  under 50 lines.
- **Brand separate from "game language."**  Brick Buster is a good
  demo of a language, not of loft's identity.  The identity should
  be **coherent data + code**, with games as one application.

---

## Pragmatic sequencing

Maximum broadening per unit effort:

1. **Finish 1.0 stability gate** — already the plan (ROADMAP.md).
2. **LSP + VS Code extension + tutorial** — ecosystem baseline;
   unblocks every other domain.
3. **Package registry + lock file** — ecosystem multiplier.
4. **Fast cold-start** (const store + stdlib `.loftc`) — unlocks
   CLI.  Cheapest broadening per line of code changed.
5. **`server` library v1 + async I/O** — unlocks web.  Largest
   addressable audience.
6. **Store-as-DB demo + narrative shift** — positioning work.
   Costs almost nothing in code, re-frames the whole project.
7. **Coroutines, FFI breadth, richer numerics** — second-wave;
   pursue as concrete demand appears.

---

## Better PHP — the `2026-08` cycle theme

**Targeted at the `2026-08` release cycle, NOT `2026-07`.**  `2026-07` is the
stability / registry / library-hardening cycle; `2026-08` turns to the
server-side-web + database stack — the concrete realisation of the "server-side
web" domain row above.  The shorthand is **"become a better PHP"**: write a
request handler that talks to a database and renders HTML with near-zero
ceremony — and fix PHP's weaknesses.

**Why loft can be a *better* PHP, not a clone:**

- **A persistent, typed, natively-fast process.**  The server is one long-lived
  process (the @PLN4 reactor) that holds connections, prepared statements, and
  caches *across* requests — not PHP's fork-per-request-and-die model.
- **Static types** catch what PHP ships to production.
- **A direct, high-performance path into the database** (@PLN24 +
  @PLN23) — see "the `#c` tier" below.

**The stack + its plans (the critical path, in build order):**

1. **@PLN24 — the `#c` direct C-ABI tier.**  Bind a C library straight from loft
   with **no libffi** (a small fixed per-arity C-ABI caller in
   loft-core; pure-loft `#c` decls + optional `cc`-compiled ANSI-C shims).
   Foundational — unblocks the DB layer and every future C binding.
   **Native-only by nature** (wasm has no C ABI / `dlopen`).
2. **@PLN23 — MariaDB + PostgreSQL clients**, uniform API (prepared statements,
   transactions, bulk-edits) over `#c`.  PHP's killer feature; loft's biggest
   current gap.  **Closed 2026-08-10 on the claim, not on a package**: one
   uniform interface drives four C libraries unchanged, and a loft object has a
   canonical SQL shape — both running in `tests/fixtures/sqldb/`
   ([plans/23-db-clients](plans/23-db-clients/README.md)).  What a shipping
   library still needs — data enums, tuples, concurrency, and the schema-reading
   direction below — is listed in that plan's closure record rather than left
   implied.
3. **@PLN4 — the real HTTP server** (routing + the native-reactor architecture,
   designed `2026-06-14` in `lib_plans/future/08-server/ARCHITECTURE.md`) for the
   web layer.
4. **A server-side view/templating story** — programmatic HTML via format-strings
   + the `html` escape lib, or template files; to be decided.

**The end-to-end design for all of this is [WEB_STACK.md](WEB_STACK.md)** — the client and
server examples, the four libraries they need, the Let's Encrypt rotation protocol, the
sandbox profile for third-party scripts, and how each part is verified on instruments the
repo already has.

**The `#c` tier is bigger than databases.**  It is loft's general "bind any C
library" capability; DB is its first consumer, with image codecs,
compression, crypto, BLAS, and C UI toolkits as later riders.  That is the
"there will be more" — recorded so the tier is designed as reusable, not a DB
sub-feature.

**Keep the loft side minimal — just the tooling to link.**  loft-core gains only
a *generic linking tool* (the `#c` annotation + a `dlopen` + fixed per-arity
C-ABI caller, no libffi + the smallest viable marshalling); it carries **zero**
database or per-library knowledge.  All complexity — type-specific conversions, structs/callbacks, the
SQL + result-set logic — lives in the libraries (loft `#c` decls + `cc`-compiled
ANSI-C shims).  The shim is the deliberate sink for complexity, so the success
test for the `#c` work is *how little it adds to loft-core* — same ethos as
draining library code out of the compiler crate (@PLN3).

---

## What not to do

- **Don't compete with Python in scientific computing head-on.**
  The ecosystem race is unwinnable from here; pursue that domain
  only if an exceptional differentiator emerges.
- **Don't broaden the language surface before 1.0.**  Every domain
  above is served by ecosystem and stdlib work, not by new syntax
  or new type-system features.
- **Do serve small 32-bit devices.**  32-bit hardware is becoming
  efficient and affordable at once, so it has a clear future in
  industry and in user devices: low cost, low energy, retro
  computing (owner).  loft reaches it as
  C ([TANMATSU.md](TANMATSU.md), [WIDTH32.md](WIDTH32.md)), not as
  bare metal: the target runs an RTOS with a heap, which the store
  model needs.
- **Don't target appliance firmware.**  The chip in a refrigerator
  has its purpose, but deeply embedded control is not loft's goal;
  the old "don't chase embedded MCUs" was right about that class and
  too narrow about the rest.  Those chips are fast approaching the
  Tanmatsu's class, so as they do, the 32-bit build (loft-32) gains
  that audience without loft aiming at it.
- **Give industrial users the tools, not the paperwork.**  A device
  maker who needs loft certified does the certification.  loft
  supplies what that work rests on: the formal rules, the code sites
  that cite them, guards with falsification receipts and the rule
  crossings ledger (owner).

---

## Native-library execution model — the steady-state design

_Decided 2026-06-04. Canonical decision record: [DESIGN_DECISIONS.md § @C71](DESIGN_DECISIONS_PLATFORM.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)._

### The model

**Native for stable/published libraries; interpreted for the user's active
script.** Compiled-once libraries are cached as native artifacts and reused across
runs; the user's own code stays interpreted for fast iteration — no native compile
per save.

This IS the lavition model: native engine + libraries, interpreted game scripts
for rapid prototyping.  The decision applies to loft at every deployment scale
from local scripting to the engine.

### Why loft is unusually suited to this model

The hard part of mixed-mode execution in other languages (Python/JNI/FFI) is data
crossing the language boundary — typically via costly marshalling, copying, or
type-negotiation.  Loft gets the crossing for free: **the store / `DbRef` heap is
already a shared ABI between the interpreter and native code**.  Data crosses the
boundary as `DbRef`s into the same `Stores` instance — no marshalling, no copying.
This is a structural property of loft's memory model (DATABASE.md), not something
to be engineered.

### The dispatch primitive already exists

`OpStaticCall` → `library_names` → `extensions::wire_native_fns` → `try_dlsym`
(and `native_packages` / `register_native_manifest` in `src/extensions.rs`)
implement exactly this model: interpreted bytecode calls compiled code through
the `#native` mechanism the stdlib uses, and a `use`d library's compiled C unit is
loaded and called the same way.

The native backend's cross-mode byte-identical equivalence (the @PLN11 harness)
guarantees a library behaves the same whether run interpreted (during development)
or native (shipped).

### The performance implication — supersedes E2 / full zero-copy as the perf endgame

The `bench_read_data_breakdown` profiling (2026-06-04, documented in
PERFORMANCE-history.md § Open work E2 row) measured that warm-load cost in the startup
cache is allocation-bound: the dominant work is materialising library bodies +
variable tables into native `String` / `Box<Type>`.  In the native-library model
you never materialise those — libraries are native artifacts, loaded via `dlopen`;
you load only the small library interface (type schema + function signatures +
symbol map).  The allocation cost is **avoided**, not eliminated via a `VH`
zero-copy rewrite.

E2 (zero-copy `read_data`) therefore drops from "startup-perf endgame" to
**low-priority / deferred-by-this-decision for performance purposes**.  The
store-IR foundation, the `IrNode` handle, and the completed `Definition` read seam
keep their architectural and self-hosting value — now serving the interface load
and the shared ABI — not zero-copy interpretation of library bodies.  See
[plans/11-data-as-store/README.md § Recommendation](plans/11-data-as-store/README.md).

### The build fingerprint — correctness crux

A native library artifact is the library's C, compiled against the C runtime the loft
binary carries.  It is valid only for the loft build that made it: change loft's C
writer OR its runtime OR the build target, and an old artifact does not match what
the program expects.

Cache key requirements:

- **MUST fold**: the loft executable's CONTENT hash (`cache::loft_build_fingerprint`,
  memoised once per process) + target + feature-set.
- **Must NOT fold**: git-HEAD `BUILD_ID` (unchanged across uncommitted rebuilds)
  or mtime (over-invalidates / fragile).

Two enforcement points:

1. **Nuke-on-recompile** (startup self-check): compare current loft build
   fingerprint against a stored marker; on mismatch, clear the native artifact
   cache.  Fast cleanup of rebuild orphans.
2. **Fingerprint in every per-artifact cache key** (lazy backstop).

### Three-layer model

| Layer | Mechanism | Goal |
|---|---|---|
| build fingerprint in each artifact key | cache key invalidation | correctness — never load a stale artifact |
| nuke-on-recompile (startup marker check) | cache sweep on next startup | fast cleanup of rebuild orphans |
| idle-TTL GC (24 h, touch-on-use) | background eviction | space — unused library sets age out |

Idle-TTL (touch-on-use) is the primary eviction policy.  The current
`cache::prune_program_cache` (oldest-first size-cap) stays as a runaway backstop.

### Validation layer and developer-vs-customer / daily-builds framing

The build fingerprint is eventually owned by a library **validation layer**: an
artifact's validity = content-hash · target · features · loft-build-fingerprint ·
(eventually) signature.  The cache then becomes dumb storage + the idle-TTL
janitor; the nuke trigger is subsumed by validation (stale-fingerprint artifacts
fail `is_valid` → never linked → swept).

It is one fingerprint serving different audiences on different timelines:
- **loft developers** need it now (the loft build changes constantly, often
  uncommitted; git-HEAD `BUILD_ID` is useless here).
- **customers on releases** are covered today by `LOFT_VERSION`.
- **customers on daily builds** will need the fingerprint (same/rolling version,
  different codegen → `LOFT_VERSION` fails).

Building it now for developers IS the customer/daily-build mechanism.

### Cache scope (local, 2026-06-04)

- Purely **local workspace** for now — no registry/per-target distribution, no
  WASM concerns.  First-use compile latency is accepted.
- Cache native artifacts **per library** (max reuse; native `init()`-sequencing
  composes independently-compiled libs at load — this is why per-library native
  artifacts work where per-library IR snapshots did not; see @C70 in
  DESIGN_DECISIONS.md).

### Remaining risks and open points

- **Dispatch coverage** is the real engineering risk: simple `#native` functions
  work, but generics, closures crossing the boundary, and complex exported types
  are hard (see PACKAGES.md "What must be native" / the C-ABI boundary).  Needs
  a coverage audit before declaring the model generally applicable.
- **Dev-interpret fallback**: a library under active edit should still interpret
  (no native compile per save); "always native" = once the library is stable/published.
- **WASM/browser is a different model** — no C compiler at runtime, so `--html` stays
  whole-program AOT-to-WASM; native-library caching does not apply there.

### Sequencing

_Tactical now (developer-facing, local)_: per-library native artifact cache with
the build-fingerprint key (`cache::loft_build_fingerprint`) + nuke trigger + idle-TTL.

_Eventually (customer-facing)_: fold the fingerprint into the library validation
layer; becomes load-bearing when daily builds ship.

---

## Summary

Loft is closer to broadly useful than the current game-centric
framing suggests.  The unlock is:

**1.0 + ecosystem + one server demo + one CLI demo** — not new
language features.

Related documents:
- [ROADMAP.md](ROADMAP.md) — milestone ordering
- [PLANNING.md](PLANNING.md) — priority backlog (registry, FFI, LSP)
- [WEB_SERVER_LIB.md](lib_plans/future/08-server/README.md) — server library design
- [DATABASE_STORES.md § Constant store](DATABASE_STORES.md#constant-store-const_store) — Phase A startup-speed mechanism (deferred Phase B/C in [`plans/82-const-store/`](plans/82-const-store))
- [PACKAGES.md](PACKAGES.md) — package format + registry
- [SERVER_FEATURES.md](plans/37-server-features/README.md) — language features for server ergonomics
