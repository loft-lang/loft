<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 91 — Self-hosting epic: loft-in-loft, bundle-modded core, ANSI-C backend, SBC portability

## Status

**Open — future epic, design discussion captured, no implementation.** Multi-phase,
design-before-build, gated on cheap de-risking probes before any large work. **Not
unrealistic:** the foundations already exist — see [Why this is reachable
today](#why-this-is-reachable-today). The small-board reach (strands 6–7) turns on
reshaping the **store *profile*, not the language** — which reframes the standing
**C54.F** decision ("store model is the wrong shape for bare-metal") as *model vs
profile*: the word-addressed arena + no-GC scope-freeing is close to the canonical
embedded shape; only the *hosted, growable* profile is unfit. This README is the
single source of truth for per-strand status; `@PLN91` carries the summary + labels.

## Goal

Rewrite loft's compiler + runtime **in loft itself**, restructuring the core so the
language is composed from drop-in, scanner-wired **bundles** (closed curated core,
additive registration seams, no editing canonical `src/`); add an **ANSI-C emission
backend** beside rustc; and a **build-time portability config** — a bounded /
freestanding **store profile** (with a narrowed `DbRef`) plus a configurable default
integer width — so loft targets small single-board computers.

## Effort + design

- **Effort:** VH (an epic; each strand is its own M–H plan once split out).
- **Design:** ~ (strands + ordering + falsification probes defined; per-strand detail pending).
- **Last touched:** 2026-06-30.

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

## Sub-arcs

| Item | Status | Notes |
|---|---|---|
| **1 — Self-host loft in loft** | Open | The spine. Compiler + runtime in loft, compiled by the stage-0 native backend. |
| **2 — Bundle/mod system for the core** | Open | Convert central switches (`fill.rs` opcode match, parser operator/precedence table, native registry) into additive scanner-wired registration seams. No `src/` edits to extend. |
| **3 — Migrate types/structures into bundles** | Open | Base language = the default bundle set; no privileged core path; enables subsettable / "limited" loft builds. |
| **4 — Opcodes-as-bundles** | Open | A *consequence* of (1): once the VM loop is loft, an opcode = name + native body + loft body, all bundle data. `regen_fill_rs` is the seam. |
| **5 — ANSI-C backend beside rustc** | Open | Minimal portable bootstrap floor; auditable output (no Rust supply chain in the artifact); reach. Most powerful *after* (1) — runtime becomes emittable, no `#rust`/`#c` double-authoring. |
| **6 — Configurable default int width** | Open | Build-time width knob (incl. `DbRef` field widths). Co-design with @PLN88 (i64 unification) — see open questions. |
| **7 — Bounded/freestanding store profile** | Open | **The real SBC door — *and* a mainline native-perf refactor.** Fixed static arena (no grow), freestanding/`no_std` runtime, and the narrowed-`DbRef` static-store binding below (narrower refs + static store resolution speed up generated code on *every* target — justified independent of C54.F). C backend + widths are its *enablers*, not the door. |

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

## Phase ordering

1. **De-risking probes first (cheapest falsification before commitment):**
   - **P-perf (existential gate for self-host):** opcode interpreter inner loop in loft,
     compiled native, vs hand-Rust. Viable if within ~1.5×; ~3× kills the epic. Run first.
   - **P-cemit:** emit one loft fn + minimal store runtime as ANSI-C, `cc` it, diff vs the rustc backend (@PLN89).
   - **P-footprint (the SBC gate / C54.F re-open evidence):** compile the actual `Store`
     `no_std` with a static backing buffer + no-grow, run a trivial compiled program,
     prove **zero `malloc`/grow** and measure flash/RAM on a 32-bit target. The number
     is what re-opens C54.F on the *model-vs-profile* axis.
   - **P-dbref-perf (mainline-native gate for strand 7):** on the *current* native
     backend, prototype static store resolution for one hot deref-heavy type; measure
     runtime + binary size + compile time vs the 12-byte baseline. Confirms the
     common-case speedup independent of the SBC footprint number — and justifies strand 7
     on its own.
2. **Strand 1 (self-host)** is the trunk; **2/3/4** ride it (4 falls out for free).
3. **Strand 7 (store profile)** is the SBC lead; **5 (ANSI-C)** and **6 (widths)** enable it.

## Bootstrap staging

`stage 0` = current Rust loft → `stage 1` = loft-in-loft compiled by stage-0's native
backend → `stage 2` = stage-1 emits C/native for itself → **C-bootstrappable on any
host with a C compiler.** The Rust implementation degrades to the bootstrap compiler.

## Honest floors + costs (do not design these away)

- **Irreducible bootstrap floor remains** — something runs stage-0 (native backend minimizes it; never zero).
- **Special types keep a kernel impl** — `text`/`vector`/`integer` move their *declaration* to bundles; impls touch opcodes/IR until self-host pulls them up.
- **4th-backend parity is ongoing cost** — interp / rustc-native / wasm / ANSI-C must match (Goal D; @PLN89 polices it).
- **Without self-host, strand 5 is a slog** — hand-port `loft-ffi` to C *and* author parallel `#c` stdlib bodies. Self-host removes both.
- **C89 vs C99 is a real fork** — C99 (`stdint.h`, `long long`) maps i64/UTF-8 cleanly; true C89 forces width typedefs + config. Decide before strand 5.
- **ANSI-C is not a perf play** — the case is reach + trust + bootstrap.
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

## Cross-arc dependencies

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
