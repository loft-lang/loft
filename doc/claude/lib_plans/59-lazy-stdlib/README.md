
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Pay-for-what-you-use stdlib — via `use`-loaded libraries

Goal: a CLI script (or any program) should not pay the parse + bytecode
+ symbol-table cost of stdlib it never touches.

Status: **CLOSED 2026-07-09 — superseded (re-premised).**  The superseding
mechanism already ships and is proven: `lib/*` packages load only on `use <name>`
(the `crypto` precedent moved its natives out of the always-loaded core), and
~20 libraries adopt this today.  The bespoke trigger-registry this plan first
proposed is explicitly **not being built**; the cold-start motive is further
mitigated by the shipped stdlib cache (@PLN52, closed — `src/startup_cache.rs`).
Nothing left to build.  Original re-premise note follows.

The original
plan proposed a bespoke **trigger registry** that auto-loads
`default/*.loft` modules when the parser sees a type/identifier
trigger.  A code-path audit during the `scripting` branch found that
**loft already has the exact mechanism this needs** — packages in
`lib/*` load only when a program does `use <name>` — and that the
`crypto` module has *already* used it to move its natives out of the
always-loaded core (`src/native.rs:2056`).  The plan is therefore
re-premised:

> **Don't build deferral for the stdlib.  Ship new modules as
> `use`-loaded `lib/*` packages (the crypto precedent), and leave the
> existing `default/*` core where it is.**

See [§ Decision — supersede the trigger registry](#decision--supersede-the-trigger-registry-with-use-loaded-libraries)
for the full rationale, the one ergonomic tradeoff, and the two-track
recommendation.  The trigger-registry design below
([§ Mechanism](#mechanism) onward) is **retained as superseded
context** — its *analysis* (three-layer model, host-bridge strategy,
datetime/Unicode case studies) still applies to the `lib/*` approach;
only its *loading machinery* is dropped.  The verified audit is at
[§ Implementation notes](#implementation-notes--verified-code-path-audit-scripting-branch).

---

## Motivation

Every `default/*.loft` file today loads at startup regardless of
whether the user needs it.  This is fine for 2–3 core files (code,
images, text) but does not scale as loft broadens (BROADENING.md):

- Regex, datetime, HTTP, crypto, renderer, audio each want a home
  in the "always-available" surface so users don't need `use x;`.
- Every always-loaded module adds cold-start cost and bytecode size
  to every program, including CLI scripts where startup is
  user-visible.
- Games import everything anyway; CLI scripts pay for a renderer
  they never touch.

Conditional loading resolves the conflict.  The original plan resolved
it with a trigger registry (below); the **re-premised plan resolves it
with `use`-loaded libraries** (next section), trading "always-available
syntax" for one `use` line per module and gaining a mechanism that
already ships.

---

## Decision — supersede the trigger registry with `use`-loaded libraries

**The mechanism already exists and is proven.**  `default/*.loft`
loads unconditionally via `Parser::parse_dir` at startup.  `lib/*`
packages — there are ~20 today (markdown, server, web, imaging, time,
…), each with a `loft.toml` manifest — load **only when a program does
`use <name>`**, through the `pending_imports` / `ImportSpec` path in
`src/parser/mod.rs`.  Moving a unit of functionality from `default/`
to `lib/` **is** the defer: its lines stop being parsed at startup for
every program that doesn't `use` it.  No trigger registry, no
mid-parse re-entry, no lexer hooks, no new error-attribution surface.

**The precedent is already in the tree.**  `crypto`'s native `n_*`
implementations were moved to `lib/crypto/native/src/lib.rs` (cdylib),
loaded at runtime via `extensions::wire_native_fns` *"when a program
does `use crypto`"* (`src/native.rs:2056-2060`).  That is exactly this
proposal, already shipped and tested.

### The one real tradeoff (a product decision)

The trigger registry's whole reason for existing was to keep modules
**always-available without `use`** — `regex(...)` would just work, no
import line.  The `lib/*` approach replaces that with an explicit
`use regex;`.  This is the entire substance of the choice:

| | Trigger registry (original) | `use`-loaded libs (re-premised) |
|---|---|---|
| Cold-start when unused | zero | zero |
| New code to build | registry + triggers + mid-parse loader + error attribution (~4-day bootstrap) | **none** — mechanism ships today |
| User boilerplate | none (auto-loaded) | one `use x;` per module |
| Risk | new parser machinery, ambiguity/cycle/error-attribution edge cases | well-trodden path (~20 libs use it) |
| Discoverability | implicit (magic) | explicit (greppable `use`) |

The cost is one `use` line; the saving is an entire subsystem of
parser machinery that no longer needs to be built or maintained.  The
explicit `use` is also arguably *better* for the CLI/scripting
audience this targets — it is the same import discipline as Rust,
Python, Go, and it makes the dependency surface greppable.

### Two-track recommendation

1. **New modules (regex, datetime, http, crypto, csv, …): ship as
   `lib/*`, loaded via `use`.**  Captures all the *forward-looking*
   cold-start value — which the audit established is the actual point
   (the present-day stdlib saving is small; the win is keeping
   cold-start flat as breadth grows).  Zero new machinery.  This is
   what makes the trigger registry unnecessary.

2. **Existing `default/*` core: leave it.**  The present-day deferral
   win is ~9 % (json alone; see the audit), and json is the *hardest*
   piece to extract — its types and methods are wired into the
   compiler across 13 source files (`src/parser/fields.rs` lowers
   `.to_json()`; `src/state/codegen.rs` + `src/codegen_runtime.rs`
   carry `--native` JsonValue codegen; `stores.name("JsonValue")`
   lookups assume it is loaded).  `coroutine` *cannot* move (its
   opcodes live in core `default/02_files.loft` and feed the iterator
   path); `stacktrace` has byte-offset native coupling
   (`src/native.rs:1812`).  Migrating any of these costs more than the
   cold-start it buys.  Revisit only if a concrete need appears.

### What this means for the rest of this document

- **Superseded:** [§ Mechanism](#mechanism), [§ Triggers](#triggers),
  [§ Load sequence](#load-sequence),
  [§ Sub-modules and multi-tier loading](#sub-modules-and-multi-tier-loading),
  [§ Implementation scope](#implementation-scope), and
  [§ Recommended adoption order](#recommended-adoption-order-re-premised--use-loaded-libs) describe
  the trigger-registry machinery that is **not being built**.  Kept for
  context and in case the "no-`use` ergonomic" is ever judged worth the
  cost.
- **Still valid, retargeted to `lib/*`:**
  [§ The three-layer model](#the-three-layer-model--what-lazy-loading-saves-and-what-it-doesnt)
  (loft-level / Rust-level / host-bridge cost separation applies
  identically to a `use`-loaded lib),
  [§ Broader principle — "use the host" on WASM](#broader-principle--use-the-host-on-wasm),
  and both case studies
  ([datetime](#case-study--datetime-as-a-two-tier-lazy-module),
  [Unicode](#case-study--unicode-and-the-rust-registry-problem)) — a
  `lib/datetime` still wants core vs `tzdata` vs `locale` split, still
  bridges tzdata to `Intl.DateTimeFormat` on WASM, etc.  Read
  "sub-module trigger" there as "a finer-grained `use` or a
  feature-gated reader."

---

## Mechanism

> **Superseded** (see [§ Decision](#decision--supersede-the-trigger-registry-with-use-loaded-libraries)).
> This and the next three sub-sections describe the trigger-registry
> machinery that the re-premised plan does **not** build; the `use`
> mechanism in `lib/*` replaces it.

The compiler already has the pieces:

- Two-pass parser with in-band loading of `default/*.loft`.
- `#rust "..."` annotations bind loft declarations to Rust
  implementations.
- Symbol table already absorbs types introduced by `default/` files.

What is missing is a **trigger-based registry** that lets the parser
say "I need module X now" instead of loading everything upfront.

### Triggers

A module registers one or more trigger conditions.  When the parser
encounters a trigger, it loads the module before resuming.

| Trigger kind | Example | Consumer |
|---|---|---|
| **Type reference** | `Date`, `Time`, `Regex` | `datetime`, `regex` |
| **Identifier reference** | `http.get(...)` | `http` (client) |
| **Construction** | `Scene { ... }` | `renderer` |
| **Function call** | `play(sound)`, `regex(...)` | `audio`, `regex` |

All triggers resolve through a single `load_module(name)` call that
is idempotent — once loaded, the module stays for the rest of the
parse.

### Load sequence

When a trigger fires for an unloaded module:

1. Parser pauses at the trigger site.
2. `load_module("regex")` — parses `default/04_regex.loft`,
   registers its Rust-backed native functions into the registry
   (see INTERNALS.md § Native Function Registry), resolves any
   types it introduces into the symbol table.
3. Parser resumes.  The trigger site now sees a valid symbol.

This preserves the two-pass invariant: the module's own contents
go through the same two-pass parse as `default/01_code.loft`.

### Sub-modules and multi-tier loading

A module can declare **sub-modules** with their own triggers.  The
parent module loads on its own trigger set; sub-modules stay dormant
until their narrower triggers fire.  This matters whenever a library
has a small core plus one or more large assets that only some users
need.

Declaration shape (design sketch):

```
module datetime {
    triggers: type_ref(Date, Time, DateTime, Duration, Instant, Period);
    sub_module tzdata {
        triggers: type_ref(TimeZone, ZoneId),
                  call(in_zone, to_zone, with_zone);
    }
    sub_module locale {
        triggers: call(format_locale, parse_locale);
    }
}
```

Load sequence for a sub-module is identical to a top-level module,
except the parent is guaranteed already loaded (the sub-module's
triggers are only reachable once the parent's types are in scope).
`.loftc` cache keys include the sub-module set as well as the
module set.

Sub-modules are the right shape whenever:

- The parent has a small core (types + basic operations) that most
  users want.
- Additional capability requires a large data asset (IANA tzdata,
  locale tables, Unicode normalisation data, crypto curve params).
- The additional capability is opt-in at the call-site level, not
  the import level.

### What lazy loading does *not* change

- Module contents still go through the same two-pass parse.
- Once loaded, the module is indistinguishable from one that was
  always loaded — same type resolution, same native registry, same
  bytecode path.
- `.loftc` cache keys include the set of loaded modules, so two
  programs with different module sets get different caches.
- Programs that use a module pay the same cost as today; programs
  that don't use it pay nothing.

---

## Candidate modules

| Module | Status | Trigger |
|---|---|---|
| `code` (01) | always-load core | (always) |
| `images` (02) | always-load core | (always) |
| `text` (03) | always-load core | (always) |
| `json` | currently always-loaded | `JsonValue` reference; `json_parse` / `json_stringify` call |
| `regex` | **new** — first lazy consumer | `Regex` type reference; `regex(...)` compile call |
| `datetime` | planned — **two-tier** case study (see below) | core: `Date` / `Time` / `DateTime` / `Duration` / `Instant` / `Period` type reference.  Sub-module `tzdata`: `TimeZone` / `ZoneId` reference or zone-aware conversion.  Sub-module `locale`: locale-aware format/parse call. |
| `http_client` | planned (BROADENING.md) | `http.get` / `http.post` / `http.Client` reference |
| `server` | planned (WEB_SERVER_LIB.md) | `Server` / `Route` type reference |
| `audio` | already exists (G5/G6) | `play(...)` / `Sound` construction |
| `opengl` | already exists (lib_plans/58-graphics/README.md) | `Scene` / `Mesh` / `Shader` reference |
| `renderer` | planned (lib_plans/58-graphics/RENDERER.md) | `Renderer` / `Camera` reference |
| `crypto` | planned | `hash(...)` / `hmac(...)` / AEAD primitives |
| `csv` / `parquet` | planned | reader / writer type reference |

The "currently always-loaded" row for `json` flags a sensible
validation target: refactor `json` to lazy-load, run the full test
suite, confirm startup cost drops for non-JSON programs.  That
closes the loop on the mechanism without shipping a new feature.

---

## What this unlocks

- **Cold-start stays fast** as loft broadens — critical for CLI
  scripting (BROADENING.md Tier 2) where startup is user-visible.
- **The always-available surface grows** without a startup-cost
  penalty.  Users get regex, datetime, HTTP, crypto "for free" in
  source — no `use x;` boilerplate, no tax when unused.
- **Library upgrades become invisible.**  Swapping the regex engine
  (e.g. linear-time NFA → compile-generated DFA, see
  [REGEX.md](../57-regex/README.md)) does not change the user's source.
- **Bytecode / `.loftc` size scales with usage.**  Matters for WASM
  ([HTML export pipeline](../../HTML_EXPORT.md)) where payload size affects load time.
- **Template for external packages** (PACKAGES.md).  The same
  trigger-registry pattern generalises to third-party libraries
  installed via `loft install`: register triggers, load on first
  use.  Blurs the line between stdlib and ecosystem.

**What loft-level lazy loading does *not* unlock:** shrinking the
Rust interpreter binary or the static data linked into it (Unicode
tables, crate-embedded tzdata, etc.).  Those need a different
mechanism — see the two-layer model below.

---

## The three-layer model — what lazy loading saves, and what it doesn't

Three distinct layers each control a different cost, and picking the
right mix is essential.  Lazy loading at the **loft level** is not
the same as tree-shaking at the **Rust level**, and neither is the
same as **bridging to the host** on WASM.  Many modules want all
three.

| Layer | Controls | Affected by loft-level lazy loading? |
|---|---|---|
| **Loft bytecode / `.loftc`** | size of the compiled program and its module set | **yes** — unloaded modules contribute zero bytes |
| **Loft parse / startup time** | cold-start cost | **yes** — unloaded modules skip the parse |
| **Loft symbol table / type registry** | memory footprint at runtime | **yes** — unloaded modules don't register types |
| **Rust interpreter binary** (native `.exe` or `.wasm`) | disk size of the shipped interpreter | **no** — determined at Rust compile time |
| **Rust static data** (Unicode tables, crate-linked tzdata, embedded crypto params) | size contribution from Rust-crate-linked data | **no** — same as above |
| **Host-bridged data / code** (browser `Intl.*`, `String.normalize`, `crypto.subtle`, Node.js equivalents) | WASM-artifact cost for features the host already provides | **no** — but goes to **zero bytes** when bridged |

### Why Rust-level cost is unaffected

The native function registry (`src/native.rs`, `FUNCTIONS`) is an
unconditional const table of Rust function pointers.  Every function
pointer in that table is statically reachable from the interpreter
entry point.  DCE / wasm-opt cannot remove a function reachable
through a const table; it cannot remove the Unicode tables that
function pulls in.  Consequence: **every WASM artifact today carries
Rust's full Unicode property tables (~50–80 KB compressed) whether
or not the user's program calls `is_alphabetic`.**  Adding a lazy
`unicode` module at the loft level does not change this.

Worse, Rust's `char::is_*` predicates *share* the underlying property
tables.  Removing one predicate from the registry wouldn't shrink
anything.  Every predicate in the registry, and every other Rust
path that touches Unicode-aware `char` methods, would have to go too.

### Getting Rust-level and WASM-level savings

Three complementary mechanisms, separate from the loft-level work:

1. **Feature-gate the native registry.**  Wrap every entry for a
   lazy-module consumer with `#[cfg(feature = "<module>")]`.  Build
   variants of the interpreter: "full" (default) and "minimal" for
   size-sensitive WASM demos.  Coarse but simple; matches the
   existing `#[cfg(feature = "threading")]` precedent already in
   `src/native.rs`.
2. **Per-program interpreter build for HTML export.**  `loft --html`
   regenerates `src/native.rs`'s registry with only the functions
   the user's program uses, then compiles that Rust to WASM.
   Precise tree-shaking, but requires a Rust toolchain at loft-user
   build time — a real dependency add.  Consider only if a
   concrete user case demands sub-100 KB WASM artifacts.
3. **Host-bridge on WASM.**  For any capability the host already
   provides (browser `Intl.*`, `String.prototype.normalize`,
   `crypto.subtle`, `fetch`, Node.js equivalents), the Rust side
   branches `#[cfg(feature = "wasm")]` into a `loftHost.<fn>()` call
   instead of bundling the implementation.  Same pattern already in
   use for FS, random, time, env, logging, storage (WASM.md § Host
   Bridge API).  **Zero bytes added to the WASM artifact** for
   bridged capabilities, because the host ships them anyway.  This
   is the single biggest WASM-size lever available to loft — bigger
   than feature gating.

### How the two layers compose

- Module with **code-only payload** (regex core): loft-level lazy
  loading saves `.loftc` size and parse cost.  Rust-level feature
  gating is optional; mostly matters for WASM artifact size.
- Module with **large data asset delivered as an external blob**
  (disk file, separate WASM-side file, bundled resource):
  loft-level lazy loading controls both the loft module load and
  whether the blob is bundled into the artifact at all.  Rust-level
  feature gating controls the parser / reader functions.
- Module with **large data asset linked via a Rust crate**
  (Unicode tables via `libcore`, `chrono-tz` if used as-is,
  `icu4x` data): loft-level lazy loading saves **nothing** of the
  data itself on WASM.  Only Rust-level feature gating or
  per-program rebuild closes the gap.

### Design principles

1. **On WASM, prefer host bridge over everything else** when the
   host provides the capability.  Zero artifact bytes beats any
   other saving.
2. **On native, prefer external blob delivery over
   Rust-crate-embedded data for large assets**, so the same
   mechanism (loft-level lazy loading) controls both the module
   load and the artifact size.
3. **Fall back to Rust-level feature gating** when a crate
   dependency is unavoidable (and accept that the data is paid for
   by every interpreter build that includes the feature).

Applied to the candidate list, each module often wants different
strategies for native vs WASM:

| Module | Data shape | Native strategy | WASM strategy |
|---|---|---|---|
| `regex` | code-only | loft-level lazy loading | same (code-only; no host API to bridge to) |
| `datetime` core | code-only | loft-level lazy loading | same |
| `datetime.tzdata` | IANA database | loft-level lazy loading + external blob + `#[cfg(feature = "tzdata")]` reader | **bridge** to `Intl.DateTimeFormat` with `timeZone` option — browser/Node ship full IANA; **zero bytes** in artifact |
| `datetime.locale` | locale tables | external blob + Rust feature gate, or ICU4X crate | **bridge** to `Intl.DateTimeFormat` / `Intl.NumberFormat` / `Intl.RelativeTimeFormat` — zero bytes |
| `unicode.normalization` | `unicode-normalization` crate | Rust feature gate + loft trigger | **bridge** to `String.prototype.normalize(form)` — zero bytes |
| `unicode.case (full)` | `unicode-case-mapping` crate | Rust feature gate + loft trigger | **bridge** to `String.prototype.toLocale{Upper,Lower}Case` — zero bytes |
| `unicode.segmentation` | `unicode-segmentation` crate | Rust feature gate + loft trigger | **bridge** to `Intl.Segmenter` (Safari 14.1+, Chrome 87+, Firefox 125+) — zero bytes |
| `unicode.collation` | `icu_collator` / similar | Rust feature gate + loft trigger | **bridge** to `Intl.Collator` — zero bytes |
| `unicode.bidi` | `unicode-bidi` crate | Rust feature gate + loft trigger | **no stable JS API** — keep Rust crate or rely on CSS `direction` for rendering only |
| `unicode.line_break` | `unicode-linebreak` crate | Rust feature gate + loft trigger | `Intl.Segmenter({granularity: "line"})` uneven across engines — keep Rust fallback until baseline improves |
| **Existing `char::is_*` predicates** | `libcore` embedded | currently unconditional — Rust feature gate planned | **bridge** to `/\p{…}/u.test(c)` or `Intl.Segmenter` — zero bytes |
| `http_client` | runtime I/O | loft-level lazy loading | **bridge** to `fetch()` — zero bytes |
| `crypto` | small constants + code | loft-level lazy loading; Rust feature gate for algorithm families | **bridge** to `crypto.subtle` (SHA family, HMAC, AES-GCM, ECDSA) — zero bytes for bridged algorithms |
| `base64` / URL encoding | code-only | loft-level lazy loading | **bridge** to `btoa`/`atob`/`encodeURIComponent` — zero bytes |
| `json` | currently always-loaded | refactor to lazy loft module | **audit**: bridge to `JSON.parse`/`JSON.stringify` if not already; potential easy win |

### Per-call overhead vs binary size

Host bridges save bytes but each call crosses the WASM↔JS boundary
(~100 ns) plus marshals strings (UTF-8 ↔ UTF-16).  For bulk
operations (`normalize(whole_string)`, `collator.compare(a, b)`) the
crossing is negligible.  For per-codepoint hot loops
(`is_alphabetic` on every char of an MB text) it can dominate.

Mitigation: **design bridge APIs as bulk-first.**  Pass whole
strings, whole arrays, whole comparisons through a single call.
Rust-side wrappers batch per-codepoint loft calls into single bridge
calls where possible.  Keep Rust-native fallbacks for genuinely
compute-bound hot loops — use profiling, not guesswork, to decide
which.

---

## Case study — datetime as a two-tier lazy module

Datetime is the canonical demonstration of sub-module loading.  A
fully-capable datetime library is not a single uniform payload — it
has a small computational core and two large data assets (timezone
database, locale tables) that only some callers need.

### Tier structure

| Tier | Size (rough) | Contains | Trigger |
|---|---|---|---|
| Core | few KB of code | `Date`, `Time`, `DateTime`, `Duration`, `Instant`, `Period`; Gregorian calendar; add / sub / diff arithmetic; ISO-8601 and RFC-3339 parse + format; monotonic clock + UTC wall clock | `Date` / `Time` / `DateTime` / `Duration` / `Instant` / `Period` type reference; `time.now()` / `time.today()` / `time.parse(...)` call; optional `@YYYY-MM-DD` literal |
| Sub-module `tzdata` | ~600 KB IANA database | `TimeZone`, `ZoneId`, zone-aware conversions, DST rules | `TimeZone` / `ZoneId` type reference; zone-aware conversion call (`in_zone`, `to_zone`, `with_zone`) |
| Sub-module `locale` | ~1–2 MB locale tables | month / weekday names, calendar variants, locale-aware parse / format | `format_locale(...)` / `parse_locale(...)` call |

A CLI script that logs UTC timestamps pays for the core only.  A
user app doing "next Tuesday 09:00 Europe/Amsterdam" pays core +
tzdata.  A program formatting dates for human display pays all
three.

### Why datetime needs this more than regex does

Regex is code-only; one module, one payload, modest size.  Datetime
without sub-modules forces one of two bad choices:

- Bundle tzdata + locale into the core — every program pays the
  full ~2–3 MB cost, even if all it does is `time.now()`.
- Exclude tzdata + locale — users needing zones reach for an
  external library and the "always-available" promise breaks.

Sub-modules resolve the tension.  The `TimeZone` type is part of the
language surface, the large data asset behind it is not paid for
until used.

### tzdata-specific concerns

- **Build-time update.**  IANA publishes tzdata releases; loft
  should pull the latest at release build time and ship a compiled
  binary table.  Design: `scripts/update_tzdata.sh` runs before
  each loft release; output is checked into the release artifact,
  not the source repo.
- **Post-install refresh.**  DST rules change after loft ships.
  Plan for a `loft --update-tzdata` command that replaces the
  compiled table on disk without recompiling the interpreter.
  Location: alongside the stdlib, read at `tzdata` sub-module
  load.
- **Const-store fit.**  tzdata is large, read-only, shareable
  across processes, `mmap`-friendly — exactly the const-store
  ([`plans/82-const-store/`](../../plans/82-const-store/README.md))
  use case.  Implementing datetime validates the
  const-store design for non-code payloads.  Promote tzdata into
  the const store once CS.C1/C2/C3 lands.
- **WASM consideration — bridge, don't bundle.**  Browsers and
  Node.js already ship the full IANA tzdata inside ICU, accessible
  via `Intl.DateTimeFormat(locale, { timeZone: "Europe/Amsterdam"
  })`.  The `tzdata` sub-module's Rust backing branches
  `#[cfg(feature = "wasm")]` into `loftHost.tz_offset_at(zone, ms)`
  / `loftHost.tz_format(zone, ms, pattern)` calls, exactly the
  same pattern FS and random already use.  Result: **zero tzdata
  bytes added to the WASM artifact** — the 600 KB blob stays a
  native-only concern.  Native builds still use the external blob
  + `#[cfg(feature = "tzdata")]` reader.  See "The three-layer
  model" section above.

### OS boundary

"What time is it now?" is a syscall.  Loft wraps via `#rust "..."`
annotations, layered over Rust's `std::time` or the `time` /
`chrono` crate.  Core datetime depends on the host OS; no extra
design.

### Scope estimate

- **Core datetime:** ~1 week (types + arithmetic + ISO/RFC parse +
  OS clock + tests + docs).
- **Sub-module `tzdata`:** ~3–5 days (build-time integration,
  zone-aware conversions, DST transition tests; add 1–2 days if
  const-store integration is done at the same time).
- **Sub-module `locale`:** ~1 week, scope-depending — defer until
  a concrete user case.

Useful datetime available in ~2 weeks; full capability in ~3
weeks.  Cheaper than regex because no matcher engine is needed.

### Why datetime should be the second lazy consumer

Regex validates **token-level triggers** and a single-module
payload.  Datetime validates **type-reference triggers**,
**sub-module loading**, and **large data assets in the const
store**.  Together they cover the full trigger-and-payload design
space and de-risk every subsequent lazy module (http, crypto,
renderer, server).

---

## Case study — Unicode and the Rust-registry problem

Unicode is the case that surfaces the two-layer model.  The naive
assumption ("lazy-load Unicode support like we lazy-load regex")
breaks down once you look at what's actually in the Rust binary.

### Current state (as of this design)

`src/native.rs` lines 45–51 register seven character predicates
unconditionally:

```rust
("t_9character_is_lowercase", t_9character_is_lowercase),
("t_9character_is_uppercase", t_9character_is_uppercase),
("t_9character_is_numeric", t_9character_is_numeric),
("t_9character_is_alphanumeric", t_9character_is_alphanumeric),
("t_9character_is_alphabetic", t_9character_is_alphabetic),
("t_9character_is_whitespace", t_9character_is_whitespace),
("t_9character_is_control", t_9character_is_control),
```

Each body is a one-line delegation to `char::is_*`.  Rust's
`char::is_*` methods use `core::unicode::unicode_data`'s compressed
interval tables (~50–80 KB) embedded in `libcore`.  The predicates
give **full Unicode coverage** already — not ASCII-only, not a
limited subset — but the tables ship in every loft binary
unconditionally, because:

1. `FUNCTIONS` is a const table reachable from program entry.
2. Every function pointer in the table is therefore reachable.
3. Each function calls into `libcore`'s Unicode tables.
4. DCE / wasm-opt cannot prove any of this unreachable.

**Consequence:** the full Unicode property tables ship in every WASM
artifact today, whether or not the user's `.loft` program uses any
character predicate.  Loft-level lazy loading cannot change this.

### What the lazy pattern can and can't do for Unicode

What **loft-level lazy loading** can do:

- Defer parsing of `default/NN_unicode.loft` until a Unicode feature
  is triggered.  Saves parse time and `.loftc` bytes.
- Cleanly expose normalization / segmentation / collation / bidi /
  line-break as always-available syntax via triggers, with no
  `use unicode;` boilerplate.

What **loft-level lazy loading** cannot do:

- Remove the ~50–80 KB `libcore` Unicode tables from the WASM
  interpreter binary.  Those are linked unconditionally.
- Remove any Rust crate's embedded Unicode tables once the crate
  is in the interpreter's `Cargo.toml` dependencies.

What **Rust-level feature gating** can do:

- Remove the seven `char::is_*` registry entries and their bodies
  behind `#[cfg(feature = "unicode-props")]`.  When the feature is
  off, the Unicode table references drop out and wasm-opt can
  strip the data.  Rough saving: ~50–80 KB on WASM, zero on
  native (which already pays for `libcore` regardless).
- Same pattern for `unicode-normalization`, `unicode-segmentation`,
  `unicode-bidi`, ICU-style collation: each becomes a feature
  whose absence strips both the Rust code and the Unicode data it
  references.

### Proposed structure — three strategies, pick per target

| Payload | Native strategy | WASM strategy |
|---|---|---|
| `text.is_alphabetic` etc. (current 7 predicates) | `#[cfg(feature = "unicode-props")]` gate in `src/native.rs`.  Default on; off for `loft --html --minimal` builds. | **bridge** to `/\p{L}/u.test(c)` and siblings.  Zero bytes for the `libcore` tables.  Bulk API (`unicode_count_alphabetic(text)`) for hot-loop amortisation. |
| `text.normalize(NFC/NFD/NFKC/NFKD)` | Loft-level trigger loads `default/NN_unicode_normalization.loft`; Rust side behind `#[cfg(feature = "unicode-normalization")]` gating the `unicode-normalization` crate. | **bridge** to `String.prototype.normalize(form)`.  Zero bytes. |
| `text.to_upper_full` / `text.fold_case` (full, locale-aware) | Rust feature gate + `unicode-case-mapping` crate. | **bridge** to `String.prototype.toLocaleUpperCase(locale)` / equivalent.  Zero bytes. |
| `text.graphemes()` / `text.words()` / `text.sentences()` | `unicode-segmentation` crate, feature-gated. | **bridge** to `Intl.Segmenter(locale, { granularity: ... })`.  Zero bytes.  Baseline: Safari 14.1+, Chrome 87+, Firefox 125+. |
| `text.compare_locale(...)` | `icu_collator` or similar crate, feature-gated (largest native payload — ~1–2 MB). | **bridge** to `Intl.Collator(locale, opts).compare(a, b)`.  Zero bytes — the biggest single WASM win. |
| `text.bidi_reorder(...)` | `unicode-bidi` crate, feature-gated. | **no stable JS API.**  Keep Rust crate for WASM, or rely on browser CSS `direction` for rendering-only use cases and skip programmatic bidi. |
| `text.line_breaks(...)` | line-break crate, feature-gated. | `Intl.Segmenter({granularity: "line"})` uneven across engines as of early 2026; keep Rust fallback until Firefox ships it. |

### Estimated WASM size savings from full host-bridging

For a WASM artifact using full Unicode + timezone-aware datetime +
locale-aware formatting, measured against the "bundle everything
in Rust" alternative:

| Component | Rust-bundled size | Host-bridged size |
|---|---|---|
| `libcore` Unicode tables (current `char::is_*`) | ~50–80 KB | 0 |
| `unicode-normalization` crate | ~500 KB | 0 |
| `unicode-segmentation` crate | ~50 KB | 0 |
| ICU collation crate | ~1–2 MB | 0 |
| `chrono-tz` / equivalent tzdata | ~600 KB | 0 |
| `Intl.*` locale data | ~1–2 MB | 0 |

**Cumulative WASM saving: 2–4 MB.**  For an itch.io-hosted game
(BROADENING.md G7.P), web IDE demo, or interactive tutorial, this
is the difference between an instant load and a visible spinner on
first paint.  On cold caches over slow links (mobile, remote), the
saving is felt as seconds, not kilobytes.

### Why this case study matters

It proves three things at once:

1. **"Lazy" is not a synonym for "tree-shaken."**  The current WASM
   artifact already ships more Unicode than most programs use, and
   no amount of loft-side lazy loading changes that.
2. **The three layers really do stack.**  Loft-level lazy loading
   (parse + bytecode), Rust-level feature gating (interpreter
   binary size on all targets), and host bridging (WASM-only, zero
   bytes for bridged capabilities) each solve a different problem
   and often want to be combined for one module.
3. **Host bridging is the single biggest WASM-size lever.**  On the
   Unicode+datetime+locale axes, it dwarfs every other optimisation
   by an order of magnitude.  Any future design that proposes
   bundling a Rust crate for a capability the host already provides
   should be challenged before accepting.

### Open question — audit before implementing

Before adopting this design, audit other paths in `src/native.rs`
and the loft interpreter itself (lexer, parser, formatter,
documentation generator) for calls to Rust's Unicode-aware `char`
methods.  Anything reachable there also pulls in `libcore`'s
Unicode tables, and feature-gating only the registry entries won't
be sufficient if (say) the lexer calls `char::is_alphabetic` on
every identifier.

Known call sites from a quick grep:
- `src/documentation.rs:810` — `chars[pos].is_alphabetic()`
- `src/formatter.rs:155` — `c.is_alphabetic()`

These are host-side tools (doc generator, formatter), not the
interpreter's hot path.  They probably *should* be feature-gated
out of a minimal WASM build — they don't belong in a browser demo
anyway — but verify before assuming.

---

## Broader principle — "use the host" on WASM

The Unicode bridge pattern generalises.  Anything the browser or
Node.js already provides, loft on WASM should prefer to bridge
rather than bundle.  Candidates beyond Unicode and datetime:

| loft capability | Host API | Status |
|---|---|---|
| File I/O | `loftHost.fs_*` | **already bridged** (WASM.md § Filesystem bridge) |
| Random | `loftHost.random_int` → `crypto.getRandomValues()` | **already bridged** |
| Clock | `loftHost.time_now` / `time_ticks` → `Date.now()` / `performance.now()` | **already bridged** |
| Environment / args | `loftHost.env_variable` / `arguments` | **already bridged** |
| Logging | `loftHost.log_write` → `console.*` | **already bridged** |
| Storage | `loftHost.storage_*` → IndexedDB / localStorage | **already bridged** (browser) |
| HTTP client | `fetch()` | **bridge** when `http_client` module ships |
| Crypto (hash / HMAC / AEAD / ECDSA) | `crypto.subtle` | **bridge** when `crypto` module ships |
| Base64 / URL-encode | `btoa` / `atob` / `encodeURIComponent` | **bridge** when these land |
| JSON parse / stringify | `JSON.parse` / `JSON.stringify` | **audit** — may already be bundled in Rust; worth checking if bridging is an easy win |
| WebSocket | `WebSocket` constructor | would be bridged when networking library ships |
| Audio (beyond existing) | Web Audio API | partially bridged (G5/G6) — audit for bundle content |

### Design principle — restated for WASM

**On WASM, bundle only what the host can't provide.**  Every bridge
is a smaller artifact and often a faster implementation (engine
JIT'd, native data).  The only reasons to bundle instead are:

- No stable host API exists (e.g. Unicode bidi, line-break in some
  engines).
- Per-call overhead from WASM↔JS crossing dominates a profiled hot
  path despite bulk API design.
- Deterministic behaviour across hosts is required (rare — tests
  are usually happy with browser + Node agreement).

### Audit action — JSON on WASM

Before shipping the lazy-stdlib mechanism, check whether loft's
`json_parse` / `json_stringify` on WASM currently uses a Rust
parser or bridges to `JSON.parse` / `JSON.stringify`.  If the
former, the fix is trivial (add `loftHost.json_parse` /
`json_stringify` to the bridge, branch `#[cfg(feature = "wasm")]`
in `src/native.rs` for the two entries) and the WASM artifact
shrinks by however large the Rust JSON parser is.  Zero design
risk; probably a same-day change.

Add this audit as a task for whoever lands the lazy-stdlib
mechanism — it's a concrete way to prove the "use the host"
principle on existing code before extending it to new modules.

---

## Risks and mitigations

- **Error attribution.**  When a lazily-loaded module fails to parse
  (bug in the module itself), the error location must point to the
  module's source, not the user's trigger site.  Parser needs to
  record the module's file path separately and emit diagnostics
  against it.  Test coverage: force a syntax error in
  `default/04_regex.loft` and confirm the diagnostic.
- **Ambiguity at the trigger site.**  A trigger like "identifier
  `http` referenced" could fire spuriously if the user names a
  local variable `http`.  Triggers must be conservative — prefer
  type-reference and token-level triggers; treat identifier
  triggers as a last resort with a clear scoping rule.
- **Circular module dependencies.**  If `regex` triggers `text` and
  `text` triggers `regex`, the loader must detect the cycle.  Core
  modules (`code` / `images` / `text`) always preload, which
  eliminates most realistic cycles; enforce a rule that lazy
  modules cannot trigger each other except through core.
- **Compile-time overhead unpredictability.**  A user pasting a
  regex into a CLI script triggers a multi-hundred-line module
  load.  Acceptable for parse, but `.loftc` should cache the
  module-loaded state so repeated runs don't re-parse.  Already
  aligns with BYTECODE_CACHE.md.
- **Versioning and the 1.0 contract.**  Lazily-loaded modules are
  part of the language surface.  Their API is stable under 1.0
  just like `text.starts_with` is stable.  Treat them identically
  for versioning purposes.

---

## Implementation scope

> **Superseded** (see [§ Decision](#decision--supersede-the-trigger-registry-with-use-loaded-libraries)).
> The ~4-day bootstrap below is the trigger-registry build that the
> re-premised plan does **not** do.  The `use`-loaded-lib path needs
> **zero** of this — the registry, trigger hooks, mid-parse insertion,
> error attribution, and module-set cache key are all already provided
> by the existing `lib/*` + `use` machinery.  The only remaining work
> is per-module: author the `lib/<name>/` package and its `loft.toml`
> (the markdown/crypto packages are the template).

Independent of any consumer:

- **Lazy module registry** — refactor `default/*.loft` loading into
  a lookup keyed by name, with `loaded: bool` state.  ~1 day.
- **Trigger hooks** — two integration points: lexer-level (token
  trigger) and parser-level (type/identifier/construction
  trigger).  ~1 day.
- **Symbol-table mid-parse insertion** — verify / harden the
  existing path used by `default/` files for in-parse type
  introduction.  ~0.5 day.
- **Error attribution tests** — force errors in a lazy module and
  confirm diagnostic location.  ~0.5 day.
- **`.loftc` cache key** — include the set of loaded modules.
  ~0.5 day.
- **First consumer refactor (`json`)** — prove the mechanism
  without shipping a new feature.  ~0.5 day.

**Total bootstrap: ~4 days.**  Each subsequent consumer (regex,
datetime, http, ...) reuses the mechanism.

---

## Recommended adoption order (re-premised — `use`-loaded libs)

This replaces the trigger-registry adoption order (kept below as
superseded context).  The forward-looking value — keeping cold-start
flat as breadth grows — is captured entirely by shipping new modules
as `lib/*`:

1. **No mechanism step.**  There is nothing to "land" — `use`-loading
   ships today.  Skip straight to consumers.
2. **Ship `regex` as `lib/regex`** (see [REGEX.md](../57-regex/README.md)),
   loaded via `use regex;`.  Code-only payload; the cleanest first
   case.  Confirms the pattern for a brand-new module with native
   backing (crypto is the precedent for the native-cdylib half).
3. **Ship `datetime` as `lib/datetime`** with the core / `tzdata` /
   `locale` split from the case study below — expressed as finer
   `use` granularity and/or feature-gated readers rather than
   sub-module triggers.  Bridge `tzdata` to `Intl.DateTimeFormat` on
   WASM (zero bytes).  See [`lib_plans/21-datetime`](../21-datetime/README.md).
4. **Apply the Rust-level / host-bridge levers independently** (still
   valid): feature-gate the `char::is_*` predicates, add WASM bridge
   branches, ship `loft --html --minimal`.  These are orthogonal to
   how loft *code* is loaded — see the three-layer model.
5. **New modules (http, crypto, csv, parquet) ship as `use`-loaded
   libs from day one.**  `http` bridges to `fetch`; `crypto` to
   `crypto.subtle` (already done).
6. **Leave existing `default/*` core in place** (json/stacktrace/
   coroutine) — see the Decision section for why migrating them costs
   more than it saves.

### Superseded adoption order (trigger registry — not being built)

1. **Land lazy-stdlib mechanism** with `json` as the validation
   consumer.  Ship the refactor; measure cold-start delta on a
   no-regex, no-JSON script.
2. **Audit and bridge `json` on WASM** (if not already).  Switch
   `json_parse` / `json_stringify` to `loftHost.json_*` under
   `#[cfg(feature = "wasm")]`.  Measure WASM artifact delta.
   Validates the **host-bridge layer** on an existing feature
   before any new modules ship.
3. **Ship regex (see [REGEX.md](../57-regex/README.md)) as the first new consumer.**
   Validates type-reference and function-call triggers, plus
   single-module payloads (code-only; no host bridge needed).
   Regex is a **standalone library** — no embedding into `match`
   syntax; users call `regex(...)`, match on its capture struct, or
   pipe the captures into a regular `match` arm.
4. **Ship datetime core + `tzdata` sub-module, with tzdata bridged
   to `Intl.DateTimeFormat` on WASM.**  Validates type-reference
   triggers, sub-module loading, external-blob data on native, and
   host bridging for the WASM case.  The 600 KB tzdata blob never
   enters a WASM artifact.
5. **Land the Rust-level feature-gating layer.**  Gate the seven
   `char::is_*` predicates behind `#[cfg(feature = "unicode-props")]`
   in `src/native.rs` (matching the existing `threading` precedent).
   Add `#[cfg(feature = "wasm")]` bridge branches to `/\p{…}/u.test`
   for the WASM target.  Add a `loft --html --minimal` flag that
   builds the interpreter without the feature for tiny-WASM-demo
   use.  Measure artifact delta.  Validates the **three-layer
   model** end-to-end.
6. **Ship Unicode sub-modules** (normalization first, then
   segmentation, then collation; bidi and line-break last, as
   bridges are shakier there).  Each uses loft-level lazy loading,
   Rust-level feature gating for native, and host bridging on WASM,
   per the case study.
7. **Migrate audio / opengl / renderer** to lazy-load where they
   aren't already.  Pure win: no user-visible change, cold-start
   improves for non-game programs.
8. **New planned modules** (http, crypto, server, `locale`
   sub-module, csv, parquet) ship lazy from day one, reusing the
   patterns proven by regex, datetime, Unicode, and `json`.
   `http` bridges to `fetch` on WASM; `crypto` bridges to
   `crypto.subtle`; `server` and `csv` / `parquet` are native-only
   in practice but still lazy.

---

## Implementation notes — verified code-path audit (scripting branch)

Recorded from a read of the actual loading path before any code
changes.  Line numbers are as-of the `scripting` branch tip and will
drift; the function names are the stable anchors.  Where a location
could not be re-confirmed cleanly, the function is named without a
line number.

### The actual default set is six files, not the design's "code / images / text"

```
default/01_code.loft        1408 lines   core
default/02_files.loft        592 lines   core (this is "files", NOT "images")
default/03_text.loft         203 lines   core
default/04_stacktrace.loft    70 lines   deferrable but marginal
default/05_coroutine.loft     24 lines   NOT cleanly deferrable (see below)
default/06_json.loft         233 lines   real candidate (the doc's validation consumer)
                            ─────
                            2530 lines
```

**Doc correction:** the candidate-modules table above lists the
always-load core as `code (01) / images (02) / text (03)`.  The real
`default/02` is **`files`**, and there is no standalone `images`
default file.  The true always-loaded core is **code / files /
text**.

**Per-file deferral verdict (verified against the code, not just
file numbering):**

- **`06_json` — genuinely deferrable.**  Core (01/02/03) contains no
  reference to `JsonValue` / `json_parse` / `json_stringify`; the only
  Rust coupling (`src/native.rs:2068`) lives in the json native, which
  only runs when json is used.  Clean type-reference +
  function-call triggers.  This is the one real win and the design's
  named validation consumer.
- **`04_stacktrace` — deferrable but marginal.**  No core references,
  so it *can* defer (trigger: a `stack_trace()` call or
  `StackFrame` / `ArgValue` type reference).  But it is only 70 lines
  and `n_stack_trace` (`src/native.rs:1804`) reads the `StackFrame`
  schema by name with **hardcoded byte-offset coupling** to the file's
  field order (`src/native.rs:1812` warns about drift).  Deferring
  trades a load-ordering constraint for a ~3 % parse saving.
- **`05_coroutine` — NOT cleanly deferrable.**  The coroutine
  *opcodes* (`OpCoroutineCreate/Next/Yield/Return/Exhausted`) are
  declared in **`default/02_files.loft` — core, always-loaded**
  (lines 451–467), and are woven into always-on machinery:
  `src/state/codegen.rs` emits `OpCoroutineYield` / `OpCoroutineReturn`
  for any generator function, and `src/parser/collections.rs:186`
  resolves `def_nr("OpCoroutineNext")` in the **for-loop / iterator
  materialisation path**.  The standalone `05_coroutine.loft` is only
  24 lines (a `CoroutineStatus` enum + a one-line `exhausted()`
  wrapper); deferring it saves nothing and its surface is entangled
  with the core iterator path.

### Loading path — two ingestion routes that both need the lazy hook

| Target | Entry | Mechanism |
|---|---|---|
| **Native / interpreter** | `src/main.rs` calls `Parser::parse_dir(default/, default=true, debug=false)` | `parse_dir` (`src/parser/mod.rs:652`) collects `.loft` files into a `BTreeSet` (so load order is lexical 01→06), then calls `Parser::parse(file, default)` (`src/parser/mod.rs:401`) once per file. |
| **WASM** | embedded, not disk | The same six files are `include_str!`-baked in `src/wasm.rs:629–650` and fed through `parse_virtual` (`src/parser/mod.rs:622`). |

**Consequence for implementation:** the lazy-module registry must be
populated from *both* sources — the disk walk (`parse_dir`) and the
static embedded list (`wasm.rs`).  A single registry keyed by module
name with a `loaded: bool` flag (per the design's Implementation
scope) can back both, but the embedded list in `wasm.rs` must be
restructured from "parse all six now" into "register all six,
parse on trigger".

### `Parser::parse` runs both passes per file

Each `parse(file, default)` call internally runs pass 1 (with
`resolve_deferred_unknowns`) then pass 2 on the same file.  Lazy
loading a module mid-parse therefore means re-entering this two-pass
machinery for the triggered module while the user file's parse is
suspended — the design's "parser pauses at the trigger site" step.
The existing `deferred_unknown` stub mechanism (`src/parser/mod.rs`
~530–589) already defers *unresolved type references to the end of a
pass*; the trigger hook is a narrower, eager variant of the same idea
(resolve by loading the owning module rather than by waiting).

### Native registry is wired at compile time, unconditionally — confirms the three-layer model

`FUNCTIONS` (`src/native.rs`) is installed into the runtime by
`native::init`, invoked from `compile.rs::byte_code_from` at first
compile — i.e. at **compile time, not parse time**, and over the
whole table at once.  So deferring a loft module's *parse* does **not**
by itself defer its native registration.  This is exactly what the
"three-layer model" section already predicts: loft-level lazy loading
saves parse + bytecode + symbol-table cost; the Rust-level
function-pointer table and its linked data are a separate lever
(feature-gating).  No surprise, but it confirms the audit task in the
adoption order: the loft-side and Rust-side savings are independent.

The only existing conditional-registry precedent is
`#[cfg(feature = "threading")]` on the `FUNCTIONS` entries — there is
**no existing on-demand / lazy loading of any default file** today.

### `default/`-origin is detected by path prefix in several places

`position.file.starts_with("default/")` (and `/default/` for absolute
paths) gates behaviour in `src/compile.rs:263` and `:399`,
`src/introspect.rs:441`, and `src/main.rs` (two sites).  **Lazy
modules must keep their `default/<NN>_<name>.loft` path** so these
checks continue to classify them as stdlib.  No change needed there —
just a constraint on the registry: load by path, not by stripping the
prefix.

### Revised win estimate — quantifying the design's claim

The design motivates lazy loading as "cold-start stays fast as loft
broadens."  The audit lets us put numbers on *today's* stdlib — and
the per-file verdict above corrects an earlier, looser estimate that
lumped all three non-core files together:

- **Genuinely deferrable today: `json` alone — 233 lines (~9 %)** of
  the 2530-line stdlib.  `stacktrace` (70 lines, ~3 %) is a marginal
  add-on with a schema-coupling caveat; `coroutine` (24 lines) is not
  deferrable because its opcodes are core.
- Effectively always-loaded (core + entangled): **~2297 lines
  (~91 %)**, dominated by `01_code` at 1408 lines alone (~56 %).

So loft-level lazy loading of the *current* stdlib is at best a ~9 %
parse-cost reduction — smaller than a first pass suggested, and that
*strengthens* the framing below.  **The mechanism's real payoff is
forward-looking:** it keeps cold-start flat as regex, datetime, http,
crypto, csv/parquet join the always-available surface.  Without it,
each new always-loaded module taxes every CLI invocation (the
BROADENING.md Tier 2 concern); with it, a script that touches none of
them pays nothing.  This sharpens — rather than contradicts — the
Motivation section: the win is not in shrinking what exists, it is in
making future breadth free for non-users.

### Baseline measurement (scripting branch, native `--release`)

Trivial script (`fn main() { print("hi"); }`) — full stdlib parse
(all 6 default files) + compile + run, 5 warm runs:

```
0.18s  0.17s  0.19s  0.20s  0.19s   wall   (~0.18s median)
~30 MB RSS
```

So native cold-start is already **~180 ms**, with the full 2530-line
stdlib parsed on every invocation.  The only cleanly deferrable file
today is `json` (~233 lines, ~9 %; see the per-file verdict above);
at a roughly proportional parse share that is a single-digit-
millisecond saving on *today's* stdlib — confirming the win is
forward-looking (keeping this ~180 ms flat as regex/datetime/http/
crypto join), not a large reduction now.  The interpreter `--release`
parse path is also far faster than the WASM ~90–180 ms re-parse the
const-store doc measures, so the *browser* target stands to gain more
per deferred line than native does.

### Open items for this branch

- **Per-file parse attribution** (pending): instrument `parse_dir` to
  print per-file parse time so the ~9 % line-count proxy can be
  replaced with a real per-module cost split (01_code likely dominates
  disproportionately given its 1408 lines).  (Informational only — the
  re-premise means we are not deferring `default/*`, so this just
  confirms there is no large win being left on the table.)
- **Verify the `use`-load cold-start delta**: measure a
  `use markdown;` program vs a no-`use` program to confirm `lib/*`
  loading really does skip the package parse at startup (expected from
  the `pending_imports` path, but measure it).  This is the empirical
  proof that the re-premised approach delivers the cold-start win
  without new machinery.

The trigger-registry build items (registry shape, mid-parse loader,
error attribution, `json` extraction) are **dropped** per the Decision
section — `use`-loading already provides them, and existing
`default/*` stays put.

---

## Related documents

- [REGEX.md](../57-regex/README.md) — regex library design; the first lazy consumer.
- [LOFT_CONTROL.md](../../LOFT.md) § Match expressions — base match semantics (regex
  is a library, not a match-pattern kind).
- [BROADENING.md](../../BROADENING.md) — why cold-start matters for
  loft's non-game reach.
- `BYTECODE_CACHE.md` (planned, not yet written) — `.loftc`
  cache design that must key on loaded-module set.
- [`plans/82-const-store/`](../../plans/82-const-store/README.md) —
  separate but complementary cold-start work; Phase A shipped
  (CONST_STORE + OpConstRef), Phase D shipped-then-retired
  (.loftc cache), Phase B + C deferred.
  Reference for shipped Phase A in
  [DATABASE_STORES.md § Constant store](../../DATABASE_STORES.md#constant-store-const_store).
- [PACKAGES.md](../../PACKAGES.md) — external-package loading model that
  this mechanism generalises to.
- [INTERNALS.md](../../INTERNALS.md) § Native Function Registry —
  where lazy-loaded modules register their Rust-backed primitives.
- [STDLIB.md](../../STDLIB.md) — stdlib API surface.
