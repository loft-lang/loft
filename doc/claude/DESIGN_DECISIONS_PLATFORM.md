<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — backends, execution and libraries

The entries of the [decision register](DESIGN_DECISIONS.md) about the two backends, the browser and `par`, the C boundary, and how libraries ship and bind.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

## C3 — WASM `par()` runs sequentially

**Decision.** In the browser a program that uses `par` gets the THREADED runtime — Web Workers
over a `SharedArrayBuffer` — and runs its bodies in parallel; `--threads` / `--no-threads`
override the choice.  Where that runtime is not available (no COOP/COEP headers, or a build
without the nightly toolchain its atomics std needs, which the build says) `par` runs
sequentially with the same answer.  **Why.** Bundle size, worker start-up and COOP/COEP are costs
paid only by a page that uses `par`, and a page without the headers still runs.

**Revisit when.** The threaded runtime no longer needs a nightly toolchain to build.  Decided
2026-04, changed by @PLN117 — [record](DESIGN_DECISIONS-history.md#c3--wasm-par-runs-sequentially).
**Holds at:** `@C3` — `tests/wasm/html-thread-proof.sh`, `tests/wasm/par-ui-responsive.sh` (the
`browser-threads` workflow).
**Catalogue:** @F33 (par), @F54 (WASM/browser target).

## C70 — No per-library IR snapshot / cache

**Decision.** Only whole-prefix bundles are cached (core, and core plus a script's sorted
library set); a library never ships or caches its own IR.  **Why.** The IR's definition and type
numbers are global and parse-order dependent, so a lone library image would need relocation by
name — the brittlest mechanism available; and the `.loft` source is the better artefact, parsed
as fast as serialized IR loads.

**Revisit when.** First-run parse of a never-seen `use` set is measured to be a real bottleneck
AND a relocation scheme is prototyped without the global-index brittleness.  Decided 2026-06-02 — [record](DESIGN_DECISIONS-history.md#c70--no-per-library-ir-snapshot--cache).

## C71 — Native libraries compile, scripts interpret — the steady-state execution model

**Decision.** Stable libraries compile to native once, cached per library and keyed on the loft
build fingerprint (`libloft.rlib` content hash, rustc version, target, features — never git HEAD or
mtime); the user's script interprets.  The choice is automatic: code that cannot cross falls back to
the interpreter silently.  WASM and `--html` stay whole-program compiled.  **Why.** Native speed
where code is stable, no `rustc` per edit, and the shared store heap passes `DbRef`s unmarshalled.

**Revisit when.** A dispatch-coverage audit shows the boundary cannot express a critical class
of library API without marshalling cost that erases the speed gain, measured for a real
consumer.  Decided 2026-06-04 — [record](DESIGN_DECISIONS-history.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model).
**Holds at:** [BROADENING.md § Native-library execution model](BROADENING.md#native-library-execution-model--the-steady-state-design).
**Catalogue:** @F53 (native backend), @F55 (package management).

## C72 — REPL session resume does not persist RNG generator state

**Decision.** Resume restores stored values verbatim but not the random generator's state; the
generator continues fresh, seeded from entropy as on any launch.  A reproducible stream is an
explicit `rand_seed`.  **Why.** A saved generator state would let anyone who reads the session
file predict future `rand()` output, and an explicit seed already covers determinism.

**Revisit when.** A non-security use case needs byte-identical RNG continuation across resume
that an explicit seed cannot give.  Decided 2026-06-08 — [record](DESIGN_DECISIONS-history.md#c72--repl-session-resume-does-not-persist-rng-generator-state).
**Holds at:** the session records definitions and binding VALUES, never a statement
(`src/repl.rs`, `record_input`); `tests/repl_session.rs::a_resumed_session_does_not_continue_the_random_stream`.
**Catalogue:** @F49 (REPL), @F43 (random numbers).

## C78 — The Rust-engine ↔ loft-library boundary: mechanism not genre, and no black boxes above the engine

**Decision.** The Rust core ships mechanism, never a genre: an opinionated game model (the hex
streaming world) stays an optional, replaceable, unprivileged loft library.  The line is "should a
game developer ever need to open this?" — no (codegen, type checker, allocator) goes to Rust; yes
(world model, gameplay primitives, rendering composition) stays loft, even when hard.  Libraries are
kits of primitives lifted one at a time; needing to move game-facing code into Rust is a language
bug to fix in loft.  **Why.** A genre in the compiler taxes every other author, and Rust code is a
black box the developer cannot read or fork.  Orthogonal to C71 (how a library runs).

**Revisit when.** A genre-neutral mechanism in a library (likely the streaming substrate) is a
measured bottleneck only the engine can fix, and moving it buries neither a worldview nor a
primitive a developer needs to read.  "Hard" or "reused" alone is not enough.
Decided 2026-06-23 — [record](DESIGN_DECISIONS-history.md#c78--the-rust-engine--loft-library-boundary-mechanism-not-genre-and-no-black-boxes-above-the-engine).

## C84 — `server` ships as minimal TCP/WS primitives, not a fully-featured HTTP framework

**Decision.** `server` ships transport primitives and nothing above them: HTTP `listen` (TLS
included) / `next` / `respond*`, single-client WebSocket and a multi-client event pump.  A program
may route with its own `match` on the request path.  A framework — a route table, middleware,
authentication, sessions, certificate automation — is a SEPARATE library over `server`, never
folded into it; @PLN148 builds that stack as `webapp`, `auth`, `acme` and `sql`
([WEB_STACK.md](WEB_STACK.md)).  **Why.** Every consumer of `server` needs "answer a request" and
"run a WebSocket"; auth, certificates and routing are each a library-sized problem, and one that
lives in `server` forces its dependencies on every program that only answers requests.

**Revisit when.** A primitive `server` lacks blocks a library above it; then add that primitive
(as @PLN148's `reload_tls`), not the layer.  Decided 2026-07-02, amended
2026-09-29 — [record](DESIGN_DECISIONS-history.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework).
**Holds at:** `@C84` — `loft-libs-net` `server/tests/c84_surface.loft` (the package's public
surface is the transport set; a new name fails its CI until someone adds it on purpose).
**Catalogue:** the `server` library (`loft-lang/loft-libs-net`).

## C87 — `#rust"..."` template path is KEPT; do NOT migrate it away to per-Op emitters (@PLN81 closed)

**Decision.** The `#rust"..."` template path stays as the STANDARD LIBRARY's way to write an
operator or builtin in one line.  It is not a library route: outside `default/`, `#rust` and
`#iterator` are refused by name, and the refusal points at `#native` with a native crate, which
reaches all four targets.  A native crate bridges a capability loft lacks; it is never a speed-up
for a routine loft can express (`(Perf-Cure)`, [formal/performance.md](formal/performance.md)).
**Why.** Migrating the ~200 templates to per-Op emitters makes a new operator cost a struct and a
registration instead of one line; the emission-bug class is better served by hardening the path.

**Revisit when.** Emission genuinely needs one source of truth — then fold the few hand-written
emitters INTO `#rust`, never the reverse.  Decided 2026-07-08, amended 2026-09-25 and 2026-09-29 — [record](DESIGN_DECISIONS-history.md#c87--rust-template-path-is-kept-do-not-migrate-it-away-to-per-op-emitters-pln81-closed).
**Holds at:** `@C87` — `tests/scripts/c87-a-rust-template-outside-the-stdlib-names-native.loft`
(the refusal and its cure) and `stdlib_rust_templates_resolve_on_wasm` (the stdlib's templates).

## C93 — A `par` worker's captured parent state is read-only; a write to it is a compile error

**Decision.** State a `par` worker captures from its parent is read-only inside the worker; a
write to it, direct or through a call that writes an aliased argument, is a compile error at the
write.  The worker may read captured state, use its own locals and element, do host I/O and PRNG,
and return a value that is folded sequentially.  `par` is deliberately strict: a possibly benign
write is refused too.  **Why.** A data race has no defined value that a null could stand for, so
the only faithful answer is to make it unexpressible.

**Revisit when.** Never by loosening `par`; broader shared-mutation parallelism would be a new,
inherently safe construct, and none is envisioned.  Decided 2026-07-14 — [record](DESIGN_DECISIONS-history.md#c93--a-par-workers-captured-parent-state-is-read-only-a-write-to-it-is-a-compile-error).
**Catalogue:** @F2 (operators) / threading. Instances the platform rule *no runtime errors, ever* (DESIGN_DECISIONS [C80](DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation)): we do not fault at runtime — we either DISALLOW what cannot work (a compile error) or make it work in a lesser state (null). A `par` data race cannot be made to "work" as null, so it is DISALLOWED. Sibling of the sandbox host-data read-only model.

## C96 — Library shipping is keyed on trust-root presence: a key-present machine ships autonomously, a key-absent one defers

**Decision.** A machine holding the trust-root key signs and ships (validate, package, sign artifact
and index, append, push); one without it validates and defers through a submission PR to
`submissions/`, which a key-holding ship pass folds in and signs.  Each artifact is signed alone,
the index is append-only and written with its `.sig`, and shipping is single-writer with a
compare-and-swap push.  `scripts/registry-sign.sh` signs with a card when a PKCS#11 module is
present, else with the key file after a typed `yes` or under `--expect <pkg>@<ver>`.  **Why.** The
key must never enter CI, and one human signing every release would bottleneck shipping.

**Revisit when.** Unattended CI publishing becomes a real need (a scoped, delegated CI key).
Decided 2026-07-15, amended 2026-09-25 — [record](DESIGN_DECISIONS-history.md#c96--library-shipping-is-keyed-on-trust-root-presence-a-key-present-machine-ships-autonomously-a-key-absent-one-defers).
**Catalogue:** @F/registry (publishing). Fixes the trust model of the file-based registry ([PKG_REGISTRY.md](PKG_REGISTRY.md), [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)) at contract 1: the reliable surface (a signed, immutable, append-only index) may only grow, and shipping into it must not bottleneck on one human.

## C102 — a release binary says nothing when it falls back to the interpreter

**Decision.** A run without a usable `rustc` falls back to the interpreter silently; the
explanation prints only when native was explicitly asked for (`--native`), and stays in `--help`
and the release `QUICKSTART.md`.  Every fallback still records `native_fallback_reason`, which
`LOFT_REQUIRE_NATIVE=1` turns into a hard error.  A source checkout without `rustc` still explains
itself, because there the message is actionable.  **Why.** Someone who downloaded a release chose
not to install a toolchain, so "install Rust and rebuild" on a run that succeeds is noise, not a
warning; gating on a per-user marker file was rejected because a fresh container has no `~/.loft`.

**Revisit when.** No trigger recorded.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c102--a-release-binary-says-nothing-when-it-falls-back-to-the-interpreter).
**Holds at:** `src/main.rs`'s fallback arms; `tests/exit_codes.rs::a_missing_rustc_falls_back_quietly_except_where_asked`.

## C106 — `#c` has ONE arity ceiling for both backends, and it was raised rather than lowered

**Decision.** `MAX_C_ARITY = 32` C slots on both backends.  A declaration is checked in code you own
(stdlib or entry project), a call site everywhere, so an unused over-ceiling binding in a dependency
does not fail your build; past 32, write an ANSI-C shim.  The diagnostic keeps its code
`c-binding-not-interpretable`.  **Why.** A backend-specific ceiling is a portability trap that fires
downstream.  Arity is refused at the declaration
(unlike `#c` on wasm, at the call) because an over-ceiling binding works on no target.

**Revisit when.** A real C API needs more than 32 integer-class slots; the ceiling then moves for
both backends at once.  Float-by-value does not reopen it.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c106--c-has-one-arity-ceiling-for-both-backends-and-it-was-raised-rather-than-lowered).
**Catalogue:** @F92 (direct C binding), @F53 (native backend)

## C107 — the C signature decides whether a `vector` carries a count, not the loft type

**Decision.** A `vector` parameter crosses `#c` as a bare element pointer where the C signature has
no integer for it, else as pointer-then-count.  `c_signature::plan` derives the assignment once
(lookahead, not greedy), read by the declaration check, the interpreter's `dispatch` and `--native`
emission.  **Why.** BLAS/LAPACK-style libraries take bare pointers and a by-reference `n`; the
assignment is provably unique when it exists.  Cost: a signature omitting a count the C function
takes is accepted, like any wrong `#c` signature.

**Revisit when.** loft grows an address-of, so a by-reference scalar no longer needs a 1-element
vector.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c107--the-c-signature-decides-whether-a-vector-carries-a-count-not-the-loft-type).
**Holds at:** `src/c_signature.rs::every_reachable_shape_has_at_most_one_reading`.
**Catalogue:** @F92 (direct C binding)

## C108 — a `vector<T>` and the C pointee are two spellings of ONE layout

**Decision.** A `vector<T>` binds to a C pointer only when the pointee has one loft element's width
and class (integer vs float); signedness is not checked.  `void *`, and any unresolved pointee, is
opaque and always accepted.  `vector<i8>` / `vector<i16>` (stored offset by their minimum) and
`vector<text>` do not cross.  One home: `CElement::of_loft` reads `data::element_storage_size`;
`c_signature::options` is the only matcher.  **Why.** The vector crosses as a pointer into loft's
own element bytes, so a width mismatch reads plausible wrong numbers or writes past the allocation;
a converting copy would hide the LP64/ILP64 question and defeat calling BLAS.

**Revisit when.** A `#c` declaration can name the library's integer model, so the ILP64 question
is asked once per package.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c108--a-vectort-and-the-c-pointee-are-two-spellings-of-one-layout).
**Catalogue:** @F92 (direct C binding)

## C109 — a float RETURN crosses `#c`; a float ARGUMENT still does not

**Decision.** A C `double` return binds to loft `float` and a C `float` return to loft `single`,
exactly (no widening).  A float argument stays refused; pass it through a pointer.  **Why.** The
return has one axis (the value is in `xmm0` or not), so it costs one more expansion of the same
trampoline arity list; an argument would need a rung per subset of positions.  Without it every
level-1 BLAS function and every LAPACK routine that answers a number needed a C shim.

**Revisit when.** Someone needs a float argument badly enough to pay for an SSE-aware ladder; it
moves for both backends at once, as C106 requires.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c109--a-float-return-crosses-c-a-float-argument-still-does-not).
**Catalogue:** @F92 (direct C binding)

## C122 — The contract is semantics; a rewrite is free wherever its conditions are validated, and a library API is the one boundary

**Decision.** The contract is what a program computes and can observe — values, faults, effect order
— never representation or copy count.  A rewrite needs only its conditions, validated on the IR, and
declines otherwise.  A construction escaping the unit keeps the promised representation; which API
is a boundary is a BUILD fact (`--native-release` compiles its loft libraries into the unit; a
`#rust` native, live reload, a stored layout and a placed library stay boundaries).  **Why.** What a
program cannot observe is the compiler's; what it can is the language's (C120, other side).

**Revisit when.** Not stated.  Decided 2026-09-15 — [record](DESIGN_DECISIONS-history.md#c122--the-contract-is-semantics-a-rewrite-is-free-wherever-its-conditions-are-validated-and-a-library-api-is-the-one-boundary).
Holds at `(R-Escape)`, [formal/rewrites.md](formal/rewrites.md).

## C135 — loft has no unsafe construct; representation is observable only at a native boundary or a file-backed store

**Decision.** No loft construct lets a program observe how a value is represented: no address,
no raw buffer, no layout, no `unsafe` block.  A Rust body belongs to the standard library
(`#rust`, C87); a package binds Rust only through `#native` and its own native crate.
Representation is observable at exactly two boundaries: a package's native code, and a store
whose bytes persist or are mapped from a file — there the store layout IS the contract, read
back by another run, process or tool.  Everywhere else a value's representation is the
compiler's (C122).  Reports of representation — `store_memory()`, the `LOFT_TRACE_*` switches —
describe the build, not the program; their output is not part of the contract.  **Why.** Every
representation rewrite (`(R-Escape)`, `(R-ValueRecord)`, the store rewrites, the proposed
`(R-Apart)` in [APART_VALUES.md](APART_VALUES.md)) is sound only because a program cannot tell
the forms apart.  Rust's optimiser stays conservative because `unsafe` code may depend on a
`Vec`'s layout; loft keeps Rust's guarantees underneath without that door on top.

**Revisit when.** A use case needs a program to observe representation that neither a native
package nor a file-backed store serves.  Opening that door makes every representation rewrite a
soundness question at once, so the proposal must name each rewrite it invalidates.  Decided
2026-10-04 — [record](DESIGN_DECISIONS-history.md#c135--loft-has-no-unsafe-construct-representation-is-observable-only-at-a-native-boundary-or-a-file-backed-store).
**Holds at:** `@C135` — the `#rust` refusal in `src/parser/definitions.rs` (with C87),
`(R-Escape)` in [formal/rewrites.md](formal/rewrites.md); guard
`tests/scripts/c135-representation-is-observable-only-at-a-boundary.loft`; the file-backed
boundary by the store format's guards (`tests/store_durable_format.rs`).

## C133 — A development build uses the startup cache; LOFT_NO_CACHE=1 turns it off

**Decision.** The whole-program startup cache is on for every build — `cargo run`, `cargo test`
and a binary under `target/` as much as an installed `loft` — and `LOFT_NO_CACHE=1` is the one
switch that turns it off, shown in `loft --help`, STARTUP_CACHE.md, DEBUG.md and
RUNNING_TESTS.md.  **Why.** Every dev run and every test that runs the binary paid the full
stdlib parse, to guard against a stale bundle answering for a changed compiler; both cache keys
fold in the build signature, so a rebuild already invalidates.  What no key sees — a compiler
instrumented without a rebuild, a test measuring a cold parse — is what the switch is for.

**Revisit when.** A warm run is measured answering differently from the cold run it stands in
for, on a dev build.  Decided 2026-09-30 — loft#1762.
**Guard:** `tests/arc_e_program_cache.rs::a_development_build_caches_unless_told_not_to`.
**Catalogue:** STARTUP_CACHE.md § Default-on behaviour and the off switch · `src/cache.rs`
`cache_decision`.
