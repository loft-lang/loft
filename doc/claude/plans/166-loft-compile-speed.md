<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 166 — loft's own compile speed: instrument it, attribute it, gate it

## Status

**Every sub-arc shipped (A1–A3, B1–B3, C1), plus B4 and B5 — the hot spots A3 attributed, cut: 35 % of a compile's instructions in B4, a further 12 % and a third of its allocations in B5.** What follows under *Why now* is the state at filing. loft's
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
| **A3** — point `--engine` at a *compile*, record the first oracle row | `scripts/profile.sh`, `bench/profile_oracle.tsv` | `make profile-corpus` re-runs the row and disagrees if attribution shifts — today the engine profiler is the one instrument with **no oracle** | **Shipped 2026-09-26.**  Measured first: a SYMBOL-level row cannot hold — over a compile the top `perf` symbol is ~5–8 % and ties with allocator symbols (`_int_free`, `malloc`); two identical runs moved the top share 8.15 % → 6.79 %.  So `profile.sh --engine` now prints the same samples BY MODULE (every symbol folded onto its loft module; the runtime onto `allocator` / `mem` / `hashing` / `vec/string`), `profile_corpus.sh` reads an `engine` row off that table over a generated `bench/frontend` corpus (SKIPPED, never counted, on a box without perf), and the first row is `frontend_large engine \bdata\b 25` — `data` 34.3 / 33.3 / 32.0 % over three runs against the allocator's 20–21 %; falsified by setting its `expect` to `\blexer\b`.  **Finding for B, a negative:** a 19 MB comment-only file does NOT make the lexer the top module (9.6 %): the allocator (28 %), the `Vec<char>` materialisation (11 %), String hashing (10 %) and `file_has_pending_fn` (8 %) all sit above it, and the four whole-file pre-scans that run before the lexer (`script::split_top_level`, `file_has_pending_fn`, `libscan::scan_method_calls`, `scan_qualified_lib_refs`) together cost more than lexing.  PROFILE_ORACLE.md § Engine |
| **B1** — the edit loop reuses the stdlib cache | `src/main.rs` (the `stdlib_warm` decision), `src/startup_cache.rs` | `bench/frontend --counts`: the tiny edit run drops toward the warm run; a stdlib edit must still invalidate — edited, added, removed | **Shipped 2026-09-25.**  A program-cache miss now takes the stdlib from its cache (enabled with the program cache), and the program manifest pins the stdlib it was built against with a `stdk` line — the stdlib cache key over every `default/` file — required on a warm load.  **Cells first** (`tests/arc_e_program_cache.rs`, three): an edited stdlib function returns its new value, an added file's function resolves, a removed file's function is refused.  They held on the old code and did not reach the hazard until they warmed through an edit (the bundle must come from a run that took the stdlib from its cache); the naive change (cache without `stdk`) then FAILED two of them — `v=1` served for an edited and for a removed function — and `stdk` fixed both.  **Equivalence found a second defect:** 82 of 1 773 dumps differed with the stdlib from cache — the bundle did not carry `Data::type_var_bound_keys`, so a user `<T>` minted a duplicate placeholder (`T#5`, `vector<T#5>` …) instead of reusing the stdlib's; the map is now in the IR schema (`TypeVarBound`, `CACHE_FORMAT_VERSION` 13), and the dumps are **byte-identical to cold over all 1 773 files, on both the edit-loop and the warm path**.  Instructions, edit loop: tiny **573 M → 88.5 M (6.5×)**, medium −22 %, large −5 % (the user file's own parse dominates there) |
| **B2** — intern the name keys behind `def_nr` + method dispatch, behind `LOFT_NO_INTERN=1` | `src/data.rs:9065-9074`, `:8139`ff | `introspect` IR **byte-identical** with and without the switch over the stdlib and all `tests/scripts`; allocation count strictly drops. A wrong intern resolves a name to another definition and the IR differs | **Half 1 shipped 2026-09-25 — the definition index.**  No interning was needed: the index stays `HashMap<(String, u16), u32>` and a lookup borrows its key (`NameKey`, which `(&str, u16)` and the owned tuple both implement and hash alike), so 20 lookup sites stop allocating a `String` per question.  No switch either: a lookup's answer cannot change with the key's ownership, so the proof is a before/after binary comparison — `--interpret --dump` (every function's IR and bytecode, stdlib included) **byte-identical over all 1 773 `tests/scripts` + `tests/docs` files**.  C1: tiny **1 077 867 → 704 186 (−34.7 %)**, medium **3 562 376 → 2 550 667 (−28.4 %)**, re-pinned; instructions −10 % (tiny, cold) and −8 % (12 826 lines).  **Half 2 measured and closed — not worth it:** counted on the medium corpus with temporary per-site probes, method-key building (`mangle_method` 9 672 calls, the `n_` keys ~2 065) is ~12 k of the 2.55 M remaining allocations, under 0.5 %; a composite borrowed key would have to hash its parts exactly as the concatenated `String` does, a coupling to std's hasher that 0.5 % does not pay for |
| **B3** — key the native cache on `.loft` source, not generated Rust | PERFORMANCE.md § N6 follow-up | mutation matrix: comment-only edit, codegen-changing edit, dependency change, flag change, stdlib change — each must hit/miss correctly. A wrong key serves a **stale binary**, so this is the phase that can do real damage | **Shipped 2026-09-26** as a LAYER in front of the generated-Rust key, not a replacement: the program's drift manifest (the oracle the interpreter already trusts to skip parsing) plus a fingerprint over the remaining inputs names the binary in a sidecar beside the manifest (`startup_cache::native_fast_path`, `program-<key>.native`), and a run whose both are current execs it before any parse.  **The design decision is what it refuses:** any `LOFT_*` outside an inert allow-list, any manifest registration beyond loft sources (native crate / lib, placement, wasm bridge), every non-run mode, a sandbox policy, Windows — each an input the key cannot see, and a decline costs only what every warm run paid before.  Matrix in `tests/native_source_key.rs` (11 cells, every one asserting the program's ANSWER, not only the marker); sabotage receipt: with the manifest check removed the edit cell answers `sum=30` for a program that says `sum=100`.  Cell 3 corrected while building it: a comment-only edit is a new BINARY on `--native`, not only a new key, because the live tier embeds the program's source in the generated Rust.  Measured, release build, warm `hello`: **113 M → 4.8 M instructions (23×)**, wall 20–40 ms → under 5 ms, the check 0.6–0.9 ms (the manifest's hash walk over the stdlib); `bench/frontend --counts`, native warm: tiny **4.8 M**, medium **14.4 M** instructions (the interpreter's warm path reads 60 M / 1 216 M in A2).  PERFORMANCE.md § N6 Ceiling |
| **B4** — attribute the front end and cut what A3 named | `src/fxhash.rs`, `env_once!` (`src/lib.rs`), `Data::set_parent` + child links, `scopes` / `use_analysis` / lexer tables | every corpus dump (1 860 files) byte-identical to the pre-change binary after EACH step; C1's ratchet never grows; callgrind Ir before/after on the large corpus | **Shipped 2026-09-26** — five steps, each measured on its own: `env_once!` (178 732 `getenv` calls per compile, −3.4 %), the definition index on an in-tree Fx hasher (3 M SipHashed lookups, −11.8 %), child links replacing three whole-table scans (−7.0 %; rebuilt zero-alloc after the first cut cost the ratchet +249), the analysis passes' and the lexer's tables on Fx plus `peek_token` comparing in place (684 206 `String`s per compile, −17.9 %), `has_private_type` scanning cheapest-field-first (−0.6 %).  **Large corpus 5 984 M → 3 870 M Ir (−35.3 %)**; ratchet re-pinned tiny 705 011 → 491 038, medium 2 551 492 → 1 723 159.  Next lead, with numbers: `Position.file: String` cloned per token (~1.4 M allocations per compile) — PERFORMANCE.md § F1 |
| **B5** — the file name a position carries is shared, not owned | `src/lexer.rs` (`Position.file: Arc<str>`, `no_file()`) and the 70 sites that build or read one | the same falsifier as B4: every corpus dump byte-identical to the pre-change binary; the ratchet never grows; callgrind Ir on the large corpus | **Shipped 2026-09-26.**  `Position { file: String }` was cloned per token and per operator (`LexResult::clone` 480 040 times, `Lexer::cont` 202 816, `parse_operators` 684 206 on the large corpus), each a heap copy of a name that never changes within a file.  `Arc<str>` makes each a refcount bump; `Arc<str>::default()` is backed by a static, so a position with no file allocates nothing either — the ratchet's own two-runs-agree check caught the first draft, a `LazyLock` that minted the shared empty name once and read one allocation more on the first run.  **Large corpus 3 870 M → 3 396 M Ir (−12.2 %)**; ratchet re-pinned tiny 491 038 → 335 537 (−31.7 %), medium 1 723 159 → 1 172 738 (−31.9 %).  Dumps: 1 857 of 1 860 byte-identical; the other three (`157-null-buffer-hoist`, `177-reclaim-early-return`, `a-non-escaping-vector-local-is-a-caller-buffer`) vary between two runs of the SAME binary, before and after alike — the order of three `OpFreeRefIfDistinct(v, __ref_N)` at a return, a run-to-run permutation that predates this plan (loft#1685) |
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
   it goes last, after the gate and the bench can both see it.  (Built: the win is the WARM
   native run, not the edit loop — an edit is a new binary and rustc's cost stands.)

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
   is a stale binary, not a slow one.  *Resolved: the sources, stdlib and `--lib` set are the
   program manifest's (already exhaustive, or the interpreter's warm load would be wrong
   too); the flags, rlib, C-ABI mode and `RUSTFLAGS` are fingerprinted; and the two surfaces
   too scattered to enumerate — the `LOFT_*` environment and the parse-time registrations —
   are REFUSED rather than keyed.*

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
