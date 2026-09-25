<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 166 — loft's own compile speed: instrument it, attribute it, gate it

## Status

**Active — A1, A2, C1 and B2 shipped; A3 needs the engine profile grouped by module; B1 (found by A2) and B3 are next, behind the C1 gate.** What follows under *Why now* is the state at filing. loft's
front end is timed at three points (`parse_default`, `native_init`, `codegen`), profiled
nowhere repeatable, and guarded by nothing: a 2–5× front-end regression is invisible to
every gate in the repo. The one quadratic ever found in the front end (loft#854) was
found by a consumer, not by any instrument here.

## Goal

Make loft's own compile/analysis speed **measurable by phase, attributable to a named
mechanism, and protected by a gate that cannot drift with the machine it runs on.**

## Effort + design

- **Effort:** M (A is S, B is M, C is S)
- **Design:** ~ — the arcs are settled; the counter's collection mechanism is open (Q1)
- **Last touched:** 2026-09-17

## Composition matrix — Stage A

**N/A, stated rather than skipped:** this plan adds instrumentation, a bench corpus and a
gate. It introduces no value, type or operation, so there is no composition surface to
enumerate. Arc B *changes* name resolution, and its cell matrix is an exact IR comparison
(B2's `Verify`), not a composition table.

## The measurement that motivates this

Measured 2026-09-17, installed `loft 2026.9.0`. The user-facing cost is the **edit loop**
— a cache miss — not the warm path:

| path | cost |
|---|---:|
| interpreter, warm cache hit | 9 ms |
| interpreter, cold (`LOFT_NO_CACHE=1`) | 44 ms |
| interpreter, **edit loop** — 3 / 3045 / 12045 lines | 48 / 68 / 180 ms |
| **native, edit loop** — 3 / 3045 lines | **373 / 697 ms** |

`rustc` is 85–88 % of a cold native run. Front-end self time (`perf`, cold
`--interpret --check`): libc allocator + mem **39–46 %**, `loft::data` 14–16 %, SipHash
6–7 %, lexer 6 %, parser 6 %, scopes 4–5 %, and `typedef` — the intuitive suspect —
**0.26–0.31 %**. Scaling is linear to 3 200 functions; there is no algorithmic disaster
left, so the whole opportunity is constant-factor.

Two mechanisms carry it, both named in code:

- `Data::def_nr` calls `name.to_string()` **twice per lookup** on the own-source-miss →
  stdlib-fallback path (`src/data.rs:9065-9074`); `source_nr`, `name_type` and
  `source_name` repeat the shape.
- Method dispatch **builds the mangled key per query** — `format!("n_{fn_name}")` then
  `mangle_method(…)` across two spellings, with `bound_holder` probing every arity
  (`src/data.rs:8139`ff, `:8352-8370`).

These are hash lookups, not linear scans: the cost is allocation and hashing.

## The load-bearing decision: the gate counts, it does not time

A time-based gate cannot work here, and the evidence is first-hand from this plan's own
preparation. Wall-clock varied **±30 %** under load on one box, and a headline figure was
wrong by 5× because it measured a `target/` build — whose whole-program cache is disabled
by design (`src/cache.rs:178`, `running_a_dev_build`) — instead of the installed binary.
Absolute times are not portable across machines, checkouts, or load, and a gate that
reddens on a busy box teaches people to ignore it.

Allocation and lookup **counts** are exact, reproducible and machine-independent, and the
repo already reasons this way (*count before you time for asymptotic questions*,
PERFORMANCE.md). So: **the gate ratchets counts; time stays a report**, consistent with
`make speed` being a report and never a gate.

## Sub-arcs

`Verify` names the comparison that goes RED if the phase is done wrong.

| Item | Source | Verify | Status |
|---|---|---|---|
| **A1** — wire `parse_user=` and `scopes=` into `LOFT_TIMING` | `src/main.rs:8789`, `:8956`; `src/compile.rs:64-121` | the three existing fields byte-identical; new fields present and summing to within 10 % of measured total — a mis-wired timer reporting 0 or double-counting fails | **Shipped 2026-09-25**: `LOFT_TIMING parse_user= scopes= lints= front_end=`; the four phases sum to `front_end` within 0.01 ms on both probes, pinned by `tests/compile_scaling.rs::loft_timing_phases_sum_to_the_front_end`.  First reading: on a cold run the stdlib parse is ~90 % of the front end (42.4 of 47.1 ms on a 5-line program) |
| **A2** — front-end bench corpus + harness (cold / warm / edit-loop, both backends) | new `bench/frontend/` | harness must report an **injected** slowdown behind an env var; every corpus input hash-distinct per run, printed — a harness whose input stops varying is the failure this guards | **Shipped 2026-09-25**: `bench/frontend/frontend.py`, a generated corpus frozen by `CORPUS_VERSION` (6 / 3226 / 12826 lines); `--self-test` injects 60 ms through `LOFT_TIMING_INJECT_MS` (read only under `LOFT_TIMING`) and sees +60.0 ms in `scopes`, +55.6 ms wall; an edit run whose input hash repeats is refused.  Modes are measured INTERLEAVED and `--counts` adds instruction counts, because a first run on a loaded box (load 42 on 24 CPUs) measured the modes one after another and reported the edit loop at 2× the cold run — the instruction count said +3.7 %, and that reading is retracted.  **Finding, in instructions:** the edit loop costs what a cold start costs (tiny 664 M vs 638 M, large 6575 M vs 6341 M; warm 60 M / 1216 M), because a program-cache miss re-parses the stdlib by design — see B1 |
| **A3** — point `--engine` at a *compile*, record the first oracle row | `scripts/profile.sh`, `bench/profile_oracle.tsv` | `make profile-corpus` re-runs the row and disagrees if attribution shifts — today the engine profiler is the one instrument with **no oracle** | Open — **measured first: a SYMBOL-level row cannot hold.**  Over a compile the top `perf` symbol is ~5–8 % and ties with allocator symbols (`_int_free`, `malloc`); two identical runs of a comment-heavy compile moved the top share 8.15 % → 6.79 %.  The oracle row needs the engine profile grouped by MODULE (lexer / parser / scopes / allocator), which `profile.sh --engine` does not do yet.  Leads the profile surfaced for B: the lexer materialises text as `Vec<char>` (7–8 % on a comment-heavy input) |
| **B1** — the edit loop reuses the stdlib cache | `src/main.rs` (the `stdlib_warm` decision), `src/startup_cache.rs` | `bench/frontend --counts`: the tiny edit run drops from ~660 M instructions toward the warm run's ~60 M plus the user file's parse; a stdlib edit must still invalidate — a matrix like B3's: stdlib file edited, added, removed, each must re-parse, and a program bundle must never be served over a changed stdlib | Open — found by A2.  With the program cache on (the default), a miss parses `default/` fresh on purpose, so every stdlib file lands in the new bundle's drift manifest; the D2b stdlib cache that would skip it is opt-in (`LOFT_STDLIB_CACHE`) and bypassed.  The cure: warm-load the stdlib and record its files in the manifest from the sources the cache key already reads.  After C1, by the ordering below |
| **B2** — intern the name keys behind `def_nr` + method dispatch, behind `LOFT_NO_INTERN=1` | `src/data.rs:9065-9074`, `:8139`ff | `introspect` IR **byte-identical** with and without the switch over the stdlib and all `tests/scripts`; allocation count strictly drops. A wrong intern resolves a name to another definition and the IR differs | **Half 1 shipped 2026-09-25 — the definition index.**  No interning was needed: the index stays `HashMap<(String, u16), u32>` and a lookup borrows its key (`NameKey`, which `(&str, u16)` and the owned tuple both implement and hash alike), so 20 lookup sites stop allocating a `String` per question.  No switch either: a lookup's answer cannot change with the key's ownership, so the proof is a before/after binary comparison — `--interpret --dump` (every function's IR and bytecode, stdlib included) **byte-identical over all 1 773 `tests/scripts` + `tests/docs` files**.  C1: tiny **1 077 867 → 704 186 (−34.7 %)**, medium **3 562 376 → 2 550 667 (−28.4 %)**, re-pinned; instructions −10 % (tiny, cold) and −8 % (12 826 lines).  **Half 2 measured and closed — not worth it:** counted on the medium corpus with temporary per-site probes, method-key building (`mangle_method` 9 672 calls, the `n_` keys ~2 065) is ~12 k of the 2.55 M remaining allocations, under 0.5 %; a composite borrowed key would have to hash its parts exactly as the concatenated `String` does, a coupling to std's hasher that 0.5 % does not pay for |
| **B3** — key the native cache on `.loft` source, not generated Rust | PERFORMANCE.md § N6 follow-up | mutation matrix: comment-only edit, codegen-changing edit, dependency change, flag change, stdlib change — each must hit/miss correctly. A wrong key serves a **stale binary**, so this is the phase that can do real damage | Open |
| **C1** — count-based ratchet over the fixed corpus, wired into the gate | new, beside `compile_scaling.rs` | a **sabotage** commit adding one `to_string()` to a hot path must turn the gate red; the ratchet's own `@falsified-at:`-style receipt records that it did | **Shipped 2026-09-25**: `tests/frontend_counts.rs` counts the front end's heap allocations in-process (a global allocator armed around the stdlib parse, the corpus parse, the scope pass and the lints) on `bench/frontend`'s corpus, against `bench/frontend/allocations.tsv`.  The count is EXACT — three runs in one process and a second process agree — so the pin is exact and may only fall; it is keyed by OS and cargo profile (read off the test binary's path: an unoptimised build allocates ~1 350 more on the medium corpus, and `cfg!(debug_assertions)` cannot tell the profiles apart in this repo); a build with no pin reports and passes.  **Sabotage receipt:** one `name.to_string()` added to `Data::def_nr` → `tiny +322 955, medium +639 819` → FAIL; reverted → pass.  The same number measures B2's target: `def_nr` runs ~640 k times on the medium corpus, and each lookup already allocates two keys |

Deliberately **not** a phase: "record the baseline attribution". It would end with a
number written down and nothing able to go red — green by construction. The baseline is
the ratchet's initial value, so it lands inside C1 where a wrong baseline fails.

## Phase ordering

1. **A1 first** — it is XS and everything after it is guesswork without a phase split.
   Today `scopes::check`, the only phase that ever went quadratic, is timed by nothing.
2. **A2, then A3.** A2 gives the corpus that C1 later ratchets; A3 records where the time
   goes inside a compile, which no doc in the repo currently states.
3. **C1 before B2/B3.** The gate must exist *before* the optimisation, or the
   optimisation's own regression cannot be caught — and C1's sabotage test is what proves
   the gate can fire at all.
4. **B2, then B3.** B2 is the measured front-end hot spot. B3 is the larger user-visible
   win (the native edit loop is 8–10× the interpreter's) but carries stale-binary risk, so
   it goes last, after the gate and the bench can both see it.

## Open design questions

1. **How are allocations counted?** A wrapping global allocator in a dedicated test binary
   is exact and standard, but it counts the harness too; a `#[cfg]` counter at the named
   call sites is narrower but must not drift from the sites it claims to cover. Resolve
   before C1. *Recommendation: the wrapping allocator, with the harness's own allocations
   measured once and subtracted, because a per-site counter is a deny-list that drifts —
   the same argument PERFORMANCE.md § P8 makes for allow-lists.*
2. **Corpus stability.** The ratchet is only meaningful over a corpus that does not move;
   if the corpus is `tests/scripts`, every new guard changes the count. *Recommendation: a
   frozen `bench/frontend/` corpus, versioned, changed only deliberately.*
3. **Does C1 gate `make ci`, or report?** Counts are deterministic, so gating is defensible
   where timing is not. *Recommendation: gate, with the ratchet only ever shrinking.*
4. **B3's key must cover every codegen input** — stdlib version, `--lib` set, dependency
   versions, flags, target. Enumerate them exhaustively before implementing; a missed input
   is a stale binary, not a slow one.

## Cross-arc dependencies

- **@PLN52** (finished) — stdlib fast-start / precompiled-stdlib cache. The predecessor:
  it built the whole-program cache whose warm path this plan measures.
- **@PLN82** (parked) — constant store; its Phase C targets the in-browser stdlib re-parse,
  the same cost on a different target.
- **@PLN164** (active) — store lifetime. Shares the theme: allocation churn dominates
  compiler-shaped workloads, in loft's own Rust here and in generated code there.
- **@PLN158** (future) — release performance pass; sibling, but *library runtime* rather
  than loft's own front end.

## See also

- [`doc/claude/PERFORMANCE.md`](../PERFORMANCE.md) — § Open work (all runtime today), § N6
  follow-up (the native cache key), the `(Perf-Weight)` rule this plan's gate obeys.
- [`doc/claude/COMPILER.md`](../COMPILER.md) — two-pass parse, lexer backtracking,
  `typedef::fill_all` re-entry.
- [`doc/claude/plans/157-native-4x-drawing/`](157-native-4x-drawing/README.md) and
  [`164-activation-arena/`](164-activation-arena/README.md) — the arc whose method this
  reuses: instrument first, attribute before building, measure the ceiling by hand, ship
  every change with a switch and a falsifier.
- [`loft-lang/plans#166`](https://github.com/loft-lang/plans/issues/166) — `@PLN166`, the
  issue this plan is.
