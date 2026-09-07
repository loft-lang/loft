
# Release Planning

## What this file is — and isn't

This file answers one question: **what must be true before we tag
and publish a release of the loft language?**  Every line below
is a gate.  If an item here is still open on release day, the
release slips.  If an item you think matters is not here, it does
not block a release (and probably belongs in
[PLANNING.md](PLANNING.md) or [ROADMAP.md](ROADMAP.md) instead).

RELEASE.md is the **ship checklist**.  The full project backlog,
priorities, and ambitions live elsewhere:

| File | Scope | Question it answers |
|---|---|---|
| **RELEASE.md** (this file) | Ship checklist — the process | "What must be true before we can publish?" |
| **[releases/](releases/README.md)** | One directory per cycle | "What did THIS release need, find, and decide?" |
| **[ROADMAP.md](ROADMAP.md)** | Things we want to do, grouped by milestone | "What's the arc of work for the project, in what order?" |
| **[PLANNING.md](PLANNING.md)** | Priority-ordered backlog, all features | "What's the next best thing to pick up?" |
| **[PROBLEMS.md](PROBLEMS.md)** | Known bugs with severity | "What's broken today?" |
| **[QUALITY.md](QUALITY.md)** | Open programmer-biting issues and active sprints | "Which open issues bite users, and what are we actively working on?" |

RELEASE.md only cites items from those four files — it doesn't
define new work, it promotes existing work to a "must close before
publish" status.  When a ROADMAP.md item becomes a release blocker,
it gets a RELEASE.md row.  When it ships, the RELEASE.md row is
crossed out (the underlying item stays in its home file with its
fix date).

Demo applications (Brick Buster, Moros editor, the Web IDE shell,
the server / game-client libraries, and the scene scripting layer)
follow their own lifecycle and are deliberately out of scope here
— they can ship on their own cadence without gating the language
releases they depend on.  Their individual backlogs live in
[PLANNING.md](PLANNING.md) and [ROADMAP.md](ROADMAP.md).

## Release cadence

Releases follow a **monthly rhythm**.  Each cycle has one long-lived
branch named for its **release month**, in `YYYY-MM` form (e.g.
`2026-07`).  All cross-theme work for the cycle lands on that branch,
and it ships at the **start of that month** — but only once the
language is **stable with a low bug count**.

A release is gated on **stability, not a fixed feature set**: if the
bug count is still high at the month boundary, the release slips and
the branch keeps stabilising.  When a cycle ships, the next month's
branch starts fresh from the new `main` tip (`2026-07` → `2026-08`).

What work is in scope during a cycle (the warm feature freeze that
began with the `2026-07` cycle) is described in
[ROADMAP.md § Feature freeze](ROADMAP.md#feature-freeze--heading-into-the-2026-07-cycle-added-2026-06-07).

**Cycle themes:**

- **`2026-07`** — stability, the package registry, and library hardening
  (extraction finished, registry maintenance + discovery, reproducible packaging).
- **`2026-08` — "become a better PHP"**: the server-side-web + database stack —
  the `#c` direct-C-ABI tier (@PLN24), the MariaDB/PostgreSQL clients (@PLN23),
  and the real HTTP server (@PLN4). Explicitly **not** `2026-07` work. Full
  rationale + critical path: [BROADENING.md § Better PHP](BROADENING.md#better-php--the-2026-08-cycle-theme).

### Monthly documentation review (by hand) — libraries + feature catalogue

Each cycle, before tagging, run the **documentation review** —
[LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md). The automated
`check_doc_drift.sh examples` gate (blocked on by CI) catches worked-example tags
that *dangle* or *duplicate*, but not the two failures only a human sees: a doc
that still resolves yet no longer describes what the code does (**staleness**),
and an example that is valid but no longer the clearest one (**quality**).

The pass has two halves, and each starts with an aid that bounds the reading:

```bash
make libcatalogue && make libraries-review   # which libraries owe a review / moved
make features-review SINCE=<watermark>       # the @F catalogue's gaps + worklist
```

Both are **reports, never release blockers** — they say what is structurally
missing and what actually moved, and stop there; whether a doc is still *true* and
whether an example is still the *clearest* are judgements they deliberately do not
make. Per-library watermarks bound the library half (libraries publish on their own
cadence — § What forces a release — so one global ref would mean nothing across
thirty-four packages); a single cycle watermark bounds the feature half. A quiet
month is a five-minute pass. Fix XS drift on the spot; bump the watermark; route M+
findings to an issue.

### Monthly bug review (by hand) — one month, one generalization

Also each cycle, before tagging, run the **bug review** —
[BUG_REVIEW.md](BUG_REVIEW.md):

```bash
make bug-review                    # which mechanism classes are still producing bugs
make bug-review ARGS="--bands 6"   # finer slicing on a busy cycle
```

Same beat and the same **report, never a blocker** status as the two documentation
halves above, aimed at a different question. Those ask whether the docs still
describe the code. This one asks whether the month's bugs shared a *cause* — because
fixing a bug answers "is this case right now?" and cannot answer "will this place
keep manufacturing bugs?", which is what decides whether next month is quieter.

The pass converts **one** rising class into **one** generalization: find the
duplicated case analysis behind it, check whether a keystone already exists that the
site simply did not adopt, and collapse it. It also runs the payoff check on the
previous cycle's conversion, so a keystone that did NOT move its class gets its
premise re-opened instead of accumulating more sites. It never files the rest of the
class — per [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md)'s standing rule the
deliverable is the collapsed structure, and the cases that matter most have no ticket
to file.

### What forces a release — keep the list bounded

*Producing* a release is cheap — CI builds every target binary automatically — but every **category of
change that forces one** is a standing tax, so that list must stay **bounded**. An unbounded
release-coupled list is itself a contract-1 red flag: it means more and more work can only ship on the
monthly beat. What legitimately forces a loft release is exactly **a change to the loft binary**:
FFI / `#native` macro changes, opcode / semantics changes (e.g. the @PLN110 `len/size` flip),
performance fixes, and the occasional language feature. A tree-walker's behavior *is* its binary, so
these are inherently release-coupled and the set is naturally small.

**Everything else stays off the release axis** and ships on its own cadence — **libraries, the registry,
and docs are never release-coupled.** Coupling them would balloon the release-tied list and drag all
work to the monthly beat. The mechanism that keeps *libraries* off the axis is the resolver
dependency-gate (@PLN113 arc D): a library declares the loft version / contract it needs, the resolver
matches it, so a library update publishes independently and an older binary falls back — no coordinated
release. A binary-baked change libraries must adapt to (the flip) creates a **one-time** "libs need this
release to exist" dependency; after that release the libraries decouple again. Keep such couplings
one-time, never standing.

**Cadence preference: fewer releases, the monthly rhythm** — for people who want the latest performance
fixes and the occasional feature (not planned to be many, but not ruled out). Not a proliferation of
point releases; a point release off the monthly beat is for a genuine binary fix that cannot wait, not
a routine tool.

### Closing plans when the release merges

Plans live in [`loft-lang/plans`](https://github.com/loft-lang/plans); GitHub's
`Fixes #N` auto-close is **same-repo only**, so a loft PR can never auto-close a
plan.  Closing is explicit and cross-repo:

- **A PR that completes a plan** carries a close directive in its body —
  `Closes @PLN<n>` (or `Closes loft-lang/plans#<n>`).  The plan stays
  `status:active` while the work is only on the cycle branch.
- **On merge to `main`** (the release), the
  [`close-plans` workflow](../../.github/workflows/close-plans.yml) reads the
  merge PR's directives and runs
  [`scripts/close-shipped-plans.sh`](../../scripts/close-shipped-plans.sh) —
  setting each plan `status:finished` + closing it.  (Needs a `PLANS_TOKEN`
  secret: Issues:write on the plans repo; without it the job no-ops.)
- **Drift safety net (runs daily):** the nightly checks
  ([`miri.yml`](../../.github/workflows/miri.yml) → `stale-plans-audit`
  job) run `scripts/audit-stale-plans.sh` every day.  It *warns* when a
  `status:active` plan's close directive is already on `main` — so a missed
  close surfaces within a day, not at the next audit-by-hand (the drift this
  caught manually in `2026-06`: @PLN1/5/10/16/21) — and it **fails the nightly**
  when a CLOSED plan still carries a live status label, which is a contradiction
  rather than a judgement and takes one command to fix.  A closed plan wearing
  `status:next` stays in everyone's next-up queue; @PLN48 and @PLN102 did for a
  month.
- **Manual fallback:** run `scripts/close-shipped-plans.sh --range
  <prev-release>..main` once after the merge if the on-merge workflow didn't fire.

## Release records — one directory per cycle

What a PARTICULAR release needed, found, and decided is not process, and it does not belong
in this file: a section per cycle made RELEASE.md grow by a screen a month and buried the
gates under the history of meeting them.  Each cycle has its own directory instead —
[releases/README.md](releases/README.md) is the index — holding the release's state
write-up (`README.md`) and the checklist's recorded evidence (`checklist.json`, which
`make release-checklist` reads and writes).  This file stays the process: what must be
true before ANY release, and how to prove it.

## What each milestone means

**0.9.0 — Fully working loft language.**
The language is feature-complete, well-documented, and tooling-friendly.
PROBLEMS.md has zero "appears fixed but unverified" entries and no
open compiler-correctness bugs.  A REPL and decent error recovery
ship.  Audience: developers who want to write loft as a real language.

**1.0.0 — Stability contract.**
1.0.0 is the stability contract: any program valid on 1.0.0 compiles
and runs identically on any 1.0.x or 1.x.0 release.  The contract
covers:
- The core language surface (syntax, type system, documented stdlib API, CLI flags).
- The public IDE API (WASM `compileAndRun` / `getSymbols` JS interface).
- A user can write, run, and share a real program — from the terminal or the browser.

Safety (no crashes, no memory corruption, no leaks) is NOT a 1.0
addition — it is the floor for every release, tracked under the
[Safety gate](#safety-gate--blocks-every-release) below.  1.0.0
additionally requires the four-platform-binary stability gate
and a full INCONSISTENCIES.md sweep; see
[ROADMAP.md § 1.0.0](ROADMAP.md).

---

## Safety gate — blocks EVERY release

**We do not ship broken builds.  Ever.**  The items below block
every tag from the next patch release onward, not just 1.0.  A
release that crashes, corrupts memory, or leaks per iteration is
not a release — it's a bug report on a schedule.  If a safety
blocker is open on release day, the release slips.  There is no
"we'll fix it next version" for crashes and leaks.

This bar applies to patch releases, minor releases, and major
releases alike.  It applies whether the target is 0.8.4 or 1.0.0.
A "quick fix" tag that closes one bug but leaves another open is
still a broken build and still gets blocked.

### The nightlies: prove them green, don't read last night's badge

**Every nightly test must be green for a release — which is a different claim
from "last night's nightly run was green."**  The two come apart in both
directions, so neither substitutes for the other:

- **A red nightly run does NOT block the tag.**  A nightly goes red for reasons
  that have nothing to do with the code being released — a runner without ALSA
  or a GL device, an expired token, a network blip reaching the registry, an
  upstream toolchain bump.  What blocks a release is a test that is *actually
  failing*, not a workflow that reported failure.
- **A green nightly run does NOT discharge the bar either.**  It proves the
  tests that RAN passed on the tree they ran against, which is neither this
  tag's tree nor necessarily the whole suite.  And the schedule is not a clock:
  the 03:00 UTC daily has started anywhere between 03:34 and 14:45 (measured
  2026-08-16..09-04), on whatever `main` was at that moment.

So the release evidence is a **current, deliberate run on the candidate's
commit**, not a historical result — and it is one command:

```
make release-gate          # every nightly, THIS commit, one CI run, one verdict
make release-checklist     # `A-release-gate` reads the newest run for HEAD's sha
```

`release-gate.yml` calls the six nightlies as reusable workflows — the full
`ci.yml` matrix incl. Windows with the stdlib round-trip and the differential
oracle, every `miri.yml` sanitizer and invariant gate, `registry-validation`,
`revalidate-libs`, `browser-threads`, `repro-build` — and a `verdict` job goes
red if any leg did not succeed, `cancelled` and `skipped` included.  It also
counts the jobs a PR shows as **advisory**: informational on a diff, blocking
on a release.  It is keyed by commit on purpose — a green run on any other
commit, last night's `main` included, is not evidence for this one — and it
dispatches only on a pushed ref, so what it tests is what GitHub holds.

The gate is the release's evidence; it is **not** a licence to leave the nightly
red.  The schedule keeps running and a red nightly is still fixed the day it
appears: a gate at release time is where a month of deferred reds would pile
up, and one per day is a fix while six at once is a slip.

The three cases that a red LEG can be, all of which end in evidence rather than
a badge:

| the leg | what clears it |
|---|---|
| **red for an environment reason** (missing ALSA/GL, expired token, registry unreachable, toolchain bump) | fix the environment or run that suite here and show it green; record the reason for the red — that is a real CI finding, and the release proceeds on a re-run of the gate |
| **red for a REAL failure that we then FIXED** (e.g. Windows was genuinely broken) | the fix lands, the gate is re-run on the fixed commit, and THAT run is the evidence.  Do **not** wait a cycle for the next nightly to agree — a release is not gated on the CI cadence catching up |
| **green** | still name what it covered: a green run also covers whatever skipped itself, which is why the `verdict` job treats a skipped leg as not green |

The second row is the one worth stating out loud, because the instinct is to
wait for a green nightly before tagging.  That instinct trades a day for no new
information: if the failure is understood and the fix is proven on a current
run, the next nightly can only repeat what you already have.  Waiting is
warranted when the fix is NOT proven — when "we think that fixed it" is doing
the work — and then the thing to get is proof, not another night.

A nightly run reports one bit; the release needs the state behind it.

### WASM endpoint — our primary deliverable must work

The browser WASM bundle (`doc/pkg/loft_bg.wasm` + `doc/pkg/loft.js`)
is the primary way users encounter loft — the gallery, the playground,
Brick Buster, and `loft --html` all depend on it.  A release where the
WASM path is broken is a release that doesn't work for most users.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **WASM-build gate** | H | `cargo build --release --lib --target wasm32-unknown-unknown --no-default-features --features wasm` must succeed with the current stable `rustc`.  The `doc/pkg/` bundle must be rebuilt from this output before tagging. | `Cargo.toml` features, `.github/workflows/ci.yml` |
| **WASM-runtime gate** | H | `tests/html_wasm.rs` must pass: the 5 P137/Q9 tests compile a trivial `.loft` to `--html`, extract the embedded WASM, and run it under Node with stub host imports.  Any `unreachable` trap or instantiation failure blocks. | `tests/html_wasm.rs`, `tools/wasm_repro.mjs` |
| **Gallery smoke** | M | `make gallery` must complete and `doc/gallery.html` must load all 24 examples in a browser without console errors.  Verified by CI (`make test-gl-headless`) where Xvfb is available. | `doc/gallery.html`, `.github/workflows/ci.yml` |

### Crashes — no release may crash on valid input

**No open crash blockers as of 2026-04-15.**  All previously-listed
crash gates closed:

- B2-runtime — closed 2026-04-13 (unit-variant retrofit).
- B3 — closed 2026-04-13 (hidden caller pre-alloc for struct-enum returns).
- B5 — all three layers closed (layers 1+2 2026-04-14; layer 3 closed
  as a side-effect of struct-enum return-slot work in PR #168→#174).
  All four `p54_b5_*` guards green.
- B7 — closed as a side-effect of the B2-runtime / B5 / dep-inference /
  lock-args work across PR #168→#172.  All five `b7_*` guards green
  (the old `_crashes` suffix stays for search-back compatibility).
- P136 — closed (`gen_if` divergent-true-branch fix).
  `tests/wrap.rs::sigsegv_repro_79_alone` and `loft_suite` (which
  walks `79-null-early-exit.loft`) both green; `ignored_scripts()`
  is empty.

### Memory safety — no release may corrupt memory

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Valgrind-clean gate** | H | `scripts/valgrind-sweep.sh`: every script in `tests/scripts/` and every doc in `tests/docs/` under memcheck, on the interpreter and as the compiled native program, must show no invalid access and `definitely lost: 0 bytes in 0 blocks`.  Runs nightly (`miri.yml` `valgrind` job); run it on the tag candidate before release. | ROADMAP.md |

### Memory leaks — no release may leak on valid programs

Long-running programs — servers, game loops, REPLs — cannot
tolerate per-iteration leaks.  A release that leaks even one
store per loop iteration is unusable for production workloads;
users hit out-of-memory before the language gets a chance to
prove itself.  This bar isn't a 1.0 feature — it's the floor for
every release.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-leak gate** | H | `State::check_store_leaks` must emit no `Warning: N stores not freed at program exit` lines across the full test suite AND a hands-on run of every `tests/scripts/*.loft`.  As of 2026-04-21 the wrap suite's `loft_suite` produces no `stores not freed` warnings, and bare-interpret runs on the historically-flagged scripts (42, 62, 76, 95) are clean under `LOFT_STORES=warn` — the gate is currently green but must be re-verified on the tag candidate (including `LOFT_LOG=stores` on the parallel scripts, see below). | `src/state/mod.rs:1486` check_store_leaks |
| **P122** | H | Store leak in game loops — struct/vector temps not freed at end-of-iteration.  Originally scoped as a Brick Buster ergonomics fix; **generalises** to any loop-body struct/vector construction.  Status-unknown (previously listed as "appears fixed"); must be re-verified in the zero-leak gate above. | PROBLEMS.md |
| **Parallel leak audit** | M | `parallel { ... }` blocks — the A15 structured-concurrency path spawns workers that hold `ParallelCtx`; confirm no worker Stores remain after join.  Run the zero-leak gate with `LOFT_LOG=stores` on `tests/scripts/22-threading.loft`, `80-parallel-block.loft`. | THREADING.md |

### Test suite integrity — no release may silently skip tests

An ignored test is a bug you promised you would fix, then pulled
out of CI.  Every `#[ignore]` hides a known failure — if the
suite is silently skipping them, the release's "all green"
status is a lie.  The bar is simple: **no `#[ignore]` attribute
ships unless explicitly approved with a documented rationale
and a linked issue**.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-ignore gate** | H | Every `#[ignore]` (and every `#[ignore = "..."]`) must either be (a) removed because the underlying bug is fixed, or (b) explicitly approved by the release owner with a one-line rationale in `tests/ignored_tests.baseline`.  The approval must cite the blocking issue ID (e.g. `B7 family — ...`, `CI harness SIGABRT (P136-adjacent)`) so the ignore traces back to the open bug.  Unreviewed ignores — where the reason is vague or the owner didn't sign off — block the release. | `tests/ignored_tests.baseline` + `tests/doc_hygiene.rs::ignored_tests_baseline_is_current` |
| **Skip-list audit** | H | Every `SKIP` / `NATIVE_SKIP` / `SCRIPTS_NATIVE_SKIP` / `ignored_scripts()` entry must be traceable to a specific open blocker issue.  "Currently worked around by skipping" counts as an ignore and must appear in the same baseline approval flow. | `tests/native.rs`, `tests/wrap.rs::ignored_scripts`, `tests/native_loader.rs` |

Baseline as of 2026-04-21 — only one entry remains:
- `regen_fill_rs` → maintenance-only, not a test of runtime
  behaviour (regenerates `src/fill.rs`); candidate for
  explicit permanent exemption.

(B5/B7 ignores all removed once the underlying bugs were
confirmed closed; `file_content_nonexistent_trace` and
`sigsegv_repro_79_alone` no longer carry `#[ignore]` attrs.
`tests/wrap.rs::sigsegv_repro_79_alone` and the P136 skip of
`79-null-early-exit.loft` in `loft_suite` are also gone —
`cargo test --release --test wrap` reports 47 passed, 0 ignored.)

---

## Milestone-specific blockers

The items below gate a SPECIFIC milestone (0.9.0 or 1.0.0) without
blocking earlier patch releases that don't claim to ship them.

### Language-surface gaps (0.9.0 blockers)

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **L1** | H | Error recovery — cascading errors after one bad token; high UX impact. | PLANNING.md § L1 |
| **P2** | H | REPL / interactive mode — needed for the "write real loft" story once the browser IDE is deferred past 1.0. | PLANNING.md § P2 |
| **W-warn** | M | Clippy-inspired developer warnings in the interpreter. | PLANNING.md § W-warn |
| **C52** | M | stdlib name clash + `std::` prefix hygiene. | PLANNING.md § C52 |
| **P117** | M | Re-verify the original `file()` pattern with `LOFT_STORES=warn` — fix landed but not re-run end-to-end. | PROBLEMS.md |
| **P120** | M | Full GL example suite end-to-end on a display (fix appears verified; one hands-on pass needed). | PROBLEMS.md |
| **P121** | M | Debug-build valgrind pass over `tests/scripts/50-tuples.loft`. | PROBLEMS.md |
| **P124** | M | `--native-emit` inspection of generated Rust (fix appears verified; one hands-on pass needed). | PROBLEMS.md |

### Stability gate (1.0 blocker)

Safety (valgrind-clean, zero-leak, zero-crash) is tracked under the
[Safety gate](#safety-gate--blocks-every-release) above and is a
blocker for every release, not just 1.0.  The items below are the
1.0-specific additions on top of that floor.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Multi-platform binaries** | H | Pre-built binaries published for Linux x86_64-musl, macOS x86_64, macOS aarch64, Windows x86_64-msvc.  Hands-on smoke test of each before publishing the tag. | ROADMAP.md § 1.0.0 |
| **Zero open High issues** | H | No entry in PROBLEMS.md or QUALITY.md tagged **High** severity at release time. | PROBLEMS.md |
| **INCONSISTENCIES sweep** | M | 6 open entries in INCONSISTENCIES.md — none are code blockers but #6 (plain enums cannot have methods) and #10 (sizeof(u8) = 4) need documentation coverage before 1.0. | INCONSISTENCIES.md |

### Code-debt cleanup (nice-to-have for 1.0)

| ID | Summary |
|---|---|
| **P54-U phase 3** | Delete ~540 lines of legacy `src/database/structures.rs::parsing` scanner once a walker-native `Diagnostic` shape replaces the `"line N:M path:X"` error-path format.  Walker already covers the success path (zero fallback hits across the full test suite).  See QUALITY.md § P54-U. |
| **T2-0** | `loft --format` code formatter — professional tooling polish; zero correctness risk. |
| **T1-2** | Wildcard imports (`use mylib::*`) — friction removal; medium payoff. |
| **T1-4** | Match expressions — largest language feature gap.  If deferred past 1.0, INCONSISTENCY #6 must be prominently documented in CHANGELOG.md and the HTML reference. |

Completed historical gate items (T0-1 through T0-7, T1-5, PROBLEMS #10,
#37–#40, P117/P120–P131 fixes, A4 pre-gate, Cargo.toml, README, CHANGELOG,
CI pipeline, R1) are recorded in CHANGELOG.md.

---

## Explicitly out of scope here

The following have their own lifecycles and are **not** tracked as
release blockers in this file.  They may ship before, during, or
after any of the language milestones above — independently:

- **Brick Buster** demo (G3/G5/G6 audio-graphics, BK.*, G7.P itch.io).
- **Moros hex RPG editor** demo (MO.*).
- **Web IDE** shell and multi-file support (W1.1 HTML export kept
  here because it is a language-side feature; W2–W6 are IDE work
  and deferred).
- **Server library** (SRV.*), **game-client library** (GC.*), and
  **scene scripting** layer (SC.*) — these are applications/libraries
  built on top of the language, not part of the language surface.

See [PLANNING.md](PLANNING.md) / [ROADMAP.md](ROADMAP.md) for the
backlogs of those projects.

---

## Explicitly 1.1+ language work

Deferred past 1.0 by design — they are either additive (can land in
a minor) or too large a change to block the stability contract on.

| Item | Notes |
|---|---|
| A2 logger production mode | Low user impact until logger is widely used |
| A4 spatial<T> full implementation | After pre-gate added in 0.8.0 |
| A5 closure capture | Very high effort; depends on P1 |
| C57 route decorator syntax | `@get` / `@post` / `@ws` annotations |
| W1.14 WASM Tier 2 | Web Worker pool + `par()` parallelism |

---

## Project Structure Changes

### For 1.0 — no crate split needed

The current single-crate layout is correct for the project's scale.  A Cargo workspace split is warranted only when W1 (WASM) starts, so that the `loft-core` library can use `crate-type = ["cdylib","rlib"]` without affecting the CLI binary.

### Cargo.toml changes before 1.0

```toml
[package]
name        = "loft"          # ✓ done 2026-03-15
version     = "1.0.0"             # bump at release
description = "loft — interpreter for the loft scripting language"  # ✓ done 2026-03-15
homepage    = "https://github.com/loft-lang/loft"  # ✓ done 2026-03-15
repository  = "https://github.com/loft-lang/loft"  # ✓ done 2026-03-15
keywords    = ["language", "interpreter", "scripting"]  # ✓ done 2026-03-15
categories  = ["command-line-utilities", "compilers"]   # ✓ done 2026-03-15
```

**Note:** `rand_core` and `rand_pcg` are actively used in `src/native.rs` for random number generation — do **not** remove them.  The earlier claim that they were unused was wrong.

**Note on renaming to "loft":** ~~Do it now.~~  **Done 2026-03-15.**  Renaming was free because the package had not yet been published to crates.io.

### Future workspace layout (for W1)

```
Cargo.toml                  (workspace root)
loft-core/              (Cargo.toml: crate-type = ["cdylib","rlib"])
  src/
loft-cli/               (Cargo.toml: [[bin]])
  src/main.rs
loft-gendoc/            (Cargo.toml: [[bin]])
  src/gendoc.rs
default/                    (standard library .loft files)
tests/
doc/
ide/                        (web IDE — added at W1)
```

---

## No Automated Releases

**Releases must never be created or triggered automatically.**  Every release
requires a human validation phase (the checklist below) that cannot be scripted:
hands-on testing of pre-built binaries on each platform, review of the CHANGELOG,
and a deliberate decision to tag and publish.

Do not push release tags, trigger release workflows, draft GitHub Releases, or
run `cargo publish` programmatically.  Always wait for the owner to do this
manually after completing the validation checklist below.

### Tag & publish — the mechanics (draft-first, under immutable releases)

The org enforces **immutable releases**: a release's assets freeze the moment it
is published and cannot be added afterwards.  So the four platform bundles MUST be
attached while the release is still a **draft**.  The pipeline is built around this
ordering — the owner never publishes an empty release and then waits for binaries:

1. **Push the annotated tag** — `git tag -a vX.Y.Z -m "…" && git push origin vX.Y.Z`.
   The tag push (not a published release) is what triggers `release.yml`.
2. **Let CI build the draft.**  `release.yml` builds all four targets (linux-musl,
   macos-x64, macos-arm64, windows-msvc) and creates the GitHub release as a
   **draft** with every bundle + `.sha256` attached and notes generated.  If any
   build leg fails, no draft appears — investigate, don't ship a partial release.
   The draft job also attaches two derived assets: `loft-<v>-src.zip` (the source
   archive the registry entry names for the version itself) and
   `loft-<v>-registry-entry.json`.
3. **Review, then publish.**  Open the draft: confirm the four bundles are present
   (smoke-test each per step 10), edit the title/body if wanted, then click
   **Publish**.  Only this click freezes the release — by which point the binaries
   are already attached.  Publishing an existing-tag draft does not re-trigger the
   build.
4. **Submit the registry entry.**  Take `loft-<v>-registry-entry.json` from the
   published release, splice it into `loft-lang/registry`'s `index.json` under
   `packages.loft`, and re-sign (`scripts/registry-sign.sh`).  This is what makes
   the release reachable by `loft self-update`, and it is the *only* step that puts
   the binaries under a signature: the `.zip.sha256` sidecars travel over the same
   transport as the zips, so they catch a corrupted download, not a substituted one.
   The signed index is the root; everything below hangs off its hashes:

   ```
   index.json                        ← the ONE signature (Ed25519, 4 trust roots)
    ├ binaries[triple].sha256          → loft-<v>-<triple>.zip   checked once, at download
    │  └ manifest_sha256              → SHA256SUMS             checked any time, on what is INSTALLED
    │     └ bin/loft, default/*.loft, and every other file the bundle shipped
    └ version.sha256                   → loft-<v>-src.zip        the source the release was built from
   ```

   Do not hand-edit the hashes.  The entry is generated from the artifacts of the
   run that built them, so it cannot drift; retyping it reintroduces exactly the
   failure a signature cannot catch — an index that is correctly signed and names
   the wrong bytes.

> **Measured 2026-08-31 — step 4 has never been completed, for any release.**  The
> signed index carries 42 library packages and no `loft` package at all, so
> `loft self-update` has never had a release to resolve and `loft verify-self` has
> never been able to answer its third question (the signed-index anchor) on any
> installation, anywhere.  The cause is not neglect: the 2026.8.0 submission was
> attempted and **rejected by the registry's own validator**, which had no toolchain
> case — gate 3 re-packages a source tree with `loft package`, and loft's repo root
> has no `loft.toml`, so it failed with `` `loft package` failed: exit status 1 ``.
> Both are closed as of 2026-08-31: [registry#22](https://github.com/loft-lang/registry/pull/22)
> (gate 2b + a narrow gate-3 exemption) merged, and [registry#31](https://github.com/loft-lang/registry/pull/31)
> (`loft 2026.8.0`, the first toolchain entry there has ever been) merged and signed.
> `loft self-update --dry-run --refresh` now answers `2026.8.0 is the newest release`,
> and the published bundle's `verify-self` reports `matches the release published in the
> signed registry index`.  **Pass `--refresh` when you check**: the index is cached under
> a TTL, and a cache predating the merge says `no releases published to compare against`
> — the same words as an empty index.
>
> The whole chain was verified end-to-end on 2026-08-31 in a throwaway clone, rather
> than assumed: splicing 2026.8.0's entry and running #22's validator passes all four
> gates (gate 2b downloads each of the four platform zips and re-checks its sha256);
> running the *current* validator on the same index reproduces the original
> `loft package` failure exactly.  So #22 is both necessary and sufficient, and the
> entry regenerated by `gen-toolchain-entry.py --splice-into` is byte-identical to the
> `loft-2026.8.0-registry-entry.json` the release itself attached.
>
> Order matters for the next release: `check-release-published.py` gates the PR that
> bumps `Cargo.toml`, and it fires as soon as the tree's version differs from the
> latest published release.  **2026.8.0's entry has to land before the version bump
> PR can merge.**

**Forgetting step 4 is caught on THIS release now, not only the next one (@PLN156).**
The postscript's commands stopped being advice: `make release-checklist` measures the
whole of step 4's effect as four automatic items — `A-validator-dryrun`
(`scripts/validator-dryrun.py`: the registry's OWN validator run against this
release's generated entry, spliced into a clone of the live index, BEFORE the
submission — the rehearsal that would have caught 2026.8.0's rejection months early;
`--replay`/`--corrupt` are how the instrument itself is falsified), `A-registry-this`
(the entry, every published triple, and each `manifest_sha256` present in the signed
index — asked of the index directly, because `self-update`'s `Current` verdict prints
the RUNNING version whether the index carries this release or merely an older one, so
the CLI output alone cannot witness a forgotten splice), `A-selfupdate-resolves` (the
postscript's own command, run and read: the empty-index message is a FAIL, never a
quiet line), and `A-acquisition` (`scripts/acquisition-chain.sh`: install.sh over the
real transport → `--version` → resolution → the literal `origin: matches the signed
registry index` line → a program executed; the 2026-08-31 throwaway-clone verification
as a per-release gate, with `.github/workflows/post-publish-verify.yml` as its
standing copy on every publish — dispatchable mid-cycle against the PREVIOUS release,
since the live chain staying acquirable is a stability property, not a release-day
one).  `M-verify-anchored` retired into `A-acquisition`, which asserts the anchor line
on an installation it just made.

**And it is still caught on the NEXT release as the backstop.**  Only
step 2 fails loudly; a missing registry entry just leaves `loft self-update`
reporting "no releases published to compare against" forever, which nobody is paged
by.  So the `previous release reached the registry` CI job goes red on the PR that
bumps `Cargo.toml`, unless the last release's entry is in the signed index with a
binary per published triple and a `manifest_sha256` on each
(`scripts/check-release-published.py`).  It gates that PR only — red on every PR
during the publish→merge window would just teach everyone to merge past it.  A
release with no `loft-<v>-src.zip` is exempt as predating the mechanism, derived
from the assets rather than a version constant someone has to maintain.

**Never** create-and-publish a release in one step (the pre-2026.7 flow):
publishing creates the tag and freezes the release before the binaries are built,
so immutable releases then reject the upload — v2026.7.0 shipped binary-less
exactly this way.

---

## Pre-Release Documentation Review

> **Load the `doc-quality` skill first** (`/doc-quality`) — and at the start of
> *any* documentation review, not just the release. It carries the comment/doc
> rules (legible-on-contact, serve-the-reader, matches-reality + stamp-vs-pointer)
> these steps apply; reviewing without it is how stale stamps and author-bookkeeping
> creep back in.

Run these steps before tagging a release.  **They are advisory, not blocking** —
only the [Safety gate](#safety-gate--blocks-every-release) (crashes / memory / leaks
/ test integrity) blocks a release.  A doc-quality finding must **never** hold a bug
fix that unblocks users.  The lints get their teeth elsewhere: a library earns the
registry **`verified`** mark only with clean lints, but it **releases and installs
regardless**.  Same rule as `lint_comments.sh` — advisory by design, never fails CI.

### 0 — User-visual documentation review (stdlib API + guides + comparison)

The clear, **advisory** review of everything a *programmer* reads: the stdlib API
reference, the guide pages, the comparison/perf pages, and the **flags & routines**
(the `make help` block and CLI flags).  It is built to neither
**gloss** (the tool visits every unit — page, example, symbol, claim — so nothing is
skimmed) nor be **diff-scoped** (every check runs over the WHOLE corpus, so a stale
remark from any past release surfaces now, not only what this release touched).  Run
it every release to *see* the state and fix what's cheap — it never blocks the tag.
Check definitions: [API_SURFACE.md § S7](API_SURFACE.md).

| # | Check | Command | Status |
|---|---|---|---|
| 0a | **Stdlib API surface** — no missing docs, no doc-quality (plan-tag/history) violations, no duplicate `pub fn`s | `scripts/api_lint.py --check default/*.loft` → `0 active` | **[now]** |
| 0b | **Guide-page code runs** — every example in `tests/docs/*.loft` executes on both backends (they are tests) | `make test` (the `docs` suite) | **[now]** |
| 0c | **No stale language in prose** — temporal/hedge words (`currently`, `planned`, `for now`, `not yet`, `TODO`, `Qn`) in guide + comparison prose; each removed or justified | `api_lint --check` over the doc corpus | **[build]** — fallback: `grep -rnEi '(currently\|planned\|for now\|not yet\|TODO\|coming soon)' tests/docs/*.loft doc/*.md` |
| 0d | **References resolve** — every `` `make <target>` `` / `--flag` named in prose is a real Makefile target / CLI flag; ([build]) every function/type/symbol too | `doc_review` (target+flag resolution) | **[now]** targets/flags · **[build]** API symbols |
| 0g | **Flags & routines** — the `make help` block is split into clear routine groups; CLI flags grouped; no oversized undivided block | `doc_review` (corpus E, sections) | **[now]** |
| 0e | **Capability & comparison claims** — negative claims ("no way to X") and the `00-vs-*`/`00-performance` tables rot when *other* code changes; reviewed via a per-page content-hash ratchet (re-surfaced only when the page changed, an example broke, a symbol vanished, or on a fixed every-N-release cadence) | doc-review baseline | **[build]** — fallback: manual review of `doc/00-vs-rust.html`, `doc/00-vs-python.html`, `doc/00-performance.html` + capability statements |
| 0f | **Regenerate + eyeball** — `gendoc` completes with no warnings; spot-check pages render | `cargo run --bin gendoc` | **[now]** |

**Why it won't gloss:** the unit is *page × (each example, each symbol, each listed
claim)* — the tool lists every one and a red item can't be skipped silently.
**First run flags everything** (empty baseline), forcing one complete pass over the
whole surface; thereafter the ratchet re-surfaces only what changed or is scheduled,
so coverage stays total without re-reading unchanged, still-valid prose.

**Current stdlib baseline (0a):** 36 findings (15 missing docs + 21 doc-quality),
tracked by the tool (`scripts/api_lint.py -c`) — a burn-down **goal**, not a release
precondition (loft's own findings never block loft's release).

### Deferred for pre-external-developer releases (2026-05-15)

Step 0's tooled checks (0a, 0b, 0f, and the auto parts of 0c/0d once built) run every
release as **advisory** signals — they surface silent-wrong content (e.g. "no way to
read raw bytes" while `byte_at` exists) regardless of external users, but like gendoc
they *inform* the release, they do not *block* it.  Only the subjective judgment in
0e and step 7 (topic flow) waits for external signal.

Until the project has regular external-developer interactions
that exercise the user-facing examples, **steps 5, 6, 7, and
the cross-platform smoke test below** are explicitly deferred.

Rationale: those steps validate the user-facing surface
(`.loft` examples, comparison pages, walkthrough topic flow,
fresh-install smoke).  Without external users hitting them,
the validation is closed-loop — the same author who wrote
the example reads it, sees nothing wrong, ships.  The
validation PAYS OFF once external users surface friction (a
stale example, a confusing topic order, a Windows symlink
issue); running it before that point is busywork that
delays the release without strengthening it.

**The author will do these manually** when they have the
feedback signal that makes them meaningful.  Until then:

  - Step 5 (user docs vs Unreleased changelog) — defer.
  - Step 6 (DEVELOPERS.md + comparison pages) — defer.
  - Step 7 (topic-flow ordering) — defer.
  - Cross-platform smoke test (Linux + macOS + Windows
    walkthrough run, VS Code extension install,
    example-open) — defer.
    ⚠ **This deferral predates external users** (2026-05-15, before the
    registry, `loft install` and `self-update` shipped), and
    `make release-checklist` lists the three hands-on runs
    (`M-hands-linux` / `-macos` / `-windows`) as outstanding.  Whether
    the deferral still holds is the owner's call; until it is made, the
    checklist asks for them and this line says they may be waived.

Steps 1-4 + 8 + 9 (internal-doc hygiene, broken-link
audit, clippy-suppression review, gendoc + PDF) are NOT
deferred — they protect the shipped artefact regardless of
external-user presence and stay as release gates.

The safety gate above (crashes / memory / leaks / test-suite
integrity) is also NOT deferred — it blocks every release,
external users or not.

**Lift this deferral** when external developers start filing
issues / opening PRs / asking documentation questions.  At
that point the validation steps gain real signal and become
worth running pre-tag.  Update this section when that
happens.

### 1 — Audit doc/claude/ for stale problem documentation

- Open PROBLEMS.md: every bug entry there should either be open or clearly crossed out / labelled FIXED with the fix date.  Remove entries that are fixed and already recorded in CHANGELOG.md.
- Open PLANNING.md: every item should be open.  Done items must have been removed (not marked done in-place) before this release.
- Open project_status.md in memory/: verify it reflects current state.

### 2 — Verify code links in doc/claude/

Walk every file in `doc/claude/` looking for references of the form `src/foo.rs`, `src/foo/bar.rs`, function names, struct names, or opcode names.  For each:
- Confirm the file/symbol still exists at that path/name.
- Update any that have moved or been renamed.

Helpful command: `grep -rn 'src/' doc/claude/` and cross-check against `ls src/`.

### 3 — Verify doc/claude/ discoverability

- Every file in `doc/claude/` must be reachable from at least one other file or from the MEMORY.md index.
- Files that are only referenced from MEMORY.md should still link to at least one sibling document.
- Orphaned files (nothing links to them) must be added to an existing doc or removed.

### 4 — Compact verbose sections

Read through any doc/claude/ file that has grown since the previous release and identify passages that are longer than necessary (e.g. multi-paragraph context that can be reduced to a bullet list, repeated caveats, implementation notes already captured in CHANGELOG.md).  Shorten these in place.

### 5 — Validate user documentation against this release

> The corpus-wide checks here are now the **step 0** gate (0a–0e).  This step
> remains the *changelog-driven* cross-check: that each shipped change is reflected.

For each feature and bug-fix entry in CHANGELOG.md under `[Unreleased]`:
- Find the corresponding section in the HTML reference (a file in `tests/docs/*.loft` or `doc/`).
- Confirm the user-visible behaviour is correctly described.
- If the feature has no user documentation, add it (either a new `.loft` example or an update to an existing one).

### 5b — A contract doc carries the contract, not its own history

Every doc that says what is TRUE — the language reference, the formal rules, the tooling
guides — is read by SKIMMING, and a doc that narrates its own repairs cannot be skimmed.  The
history is worth keeping: it is what stops the next reader re-deriving a decision, and it is
where a rule's deviation register lives.  It is simply not what the contract doc is for.

One companion file per doc, named so it is recognisable at a glance:

```
<doc>.md            the contract, plus the CURRENT state — what is open, what is pending
<doc>-history.md    the timeline — what changed, when, what it cost, and what closed it
```

Run the report and work the head of the list:

```bash
python3 scripts/doc_history_report.py            # every contract doc, worst first
python3 scripts/doc_history_report.py <doc.md>   # the lines it flagged, and why
```

It is a REPORT, not a gate, and it has to be: whether a date is timeline or contract is a
judgement — *"`@F7` shipped in 1.1"* is a compatibility FACT that belongs in the contract — and
a gate over a judgement gets satisfied rather than obeyed.  Two rules make the split hold:

- **The latest state stays in the contract doc.**  A reader must not have to open the companion
  to learn that two deviations are open.  Keep the count and one line per open item; move the
  narrative.  A companion nobody has to read to know where things stand is the point.
- **MOVE, never copy.**  `scripts/rule_tags.py` resolves `@FR-` citations by scanning
  `doc/claude/formal/*.md`, so a companion beside its rules doc keeps every citation resolving
  — but a register that exists in two files defines its entries twice, and the checker says so.

For a doc near the top of the report, either move its history into the companion or record in
the release notes why it stays.

### 6 — Validate DEVELOPERS.md caveats and language-comparison pages

- **`doc/DEVELOPERS.md`**: re-read the compiler pipeline description and all "caveat" or "known limitation" callouts.  Update any that are stale relative to source changes in this release.
- **`doc/00-vs-rust.html`** and **`doc/00-vs-python.html`**: verify that the claims in each comparison table remain accurate for the current language surface (null safety, type inference, collection API, etc.).  Update any cell that no longer holds.

### 7 — Validate user documentation topic flow

- Open `doc/` and list all `NN-*.html` files in order.
- Read the first sentence of each page and verify the sequencing makes sense for a reader progressing top-to-bottom (introductory concepts before advanced ones).
- If a topic added in this release landed at the end of the sequence but logically belongs earlier, renumber and update all cross-links.

### 8 — Validate coding standards and review clippy suppressions

```bash
cargo clippy -- -D warnings
make clippy-review                        # which suppressions are dead, which are live but unexplained
make clippy-review ARGS="--legs all"      # + the warnings CI never lints (debug assertions ON, wasm32)
```

All warnings must be errors-free.  `make clippy-review` then measures every
`#[allow(clippy::…)]` under `src/` instead of grepping for them: in a throwaway
worktree each one becomes an `#[expect]`, clippy runs CI's three lines, and the
compiler names each expectation nothing fulfilled — the function that shrank under
the line limit, the parameter that was removed — beside whether anything on or
above the line says why it is there.  A report, never a gate; it edits nothing.

For each suppression the report says which of three things it is:
- **dead** — remove the `#[allow]`; clippy stays silent, and the report is the proof.
- **live and unjustified** — keep it, and add a brief comment saying which structural
  constraint it covers (a dispatch function that cannot be split without losing clarity).
- **redundant with a crate-root `#![allow]`** — `src/lib.rs` / `src/main.rs` already switch
  the lint off for the whole crate, so the attribute is an intent marker at best.

The goal is to keep suppressions intentional and minimal, not to accumulate them as a
release-over-release debt.

> **Measured 2026-09-04 (`e4366d4d`) — a census, not a cleanup.**  257 attributes name
> a clippy lint (244 on items, 13 file-scope, 3 via `cfg_attr`), 329 lint mentions over
> 54 lints.  A grep counts 22 more: `#![allow]` text inside string literals that a
> generator emits into another file (`src/create.rs`, `src/generation/mod.rs`,
> `src/android.rs`), which the tool excludes.
> **Justification:** 150 of the 244 item-level attributes have a comment on the line
> or on the line above; 94 do not (159 if the item's own `///` doc line does not count).
> **Dead:** 51 attributes outright and 7 in part (one of several lints named) — 66 of
> the 329 mentions: `too_many_lines` 16 (the function is now under 100 lines),
> `too_many_arguments` 15 (7 parameters or fewer now), `unused_self` 5,
> `cast_precision_loss` 3, the rest 1–2 each.  34 of the 54 dead item-level attributes
> carry a justification comment, so the comment describes a constraint that no longer
> exists; 3 were dead when written (2026-09-02, seven-parameter functions).
> **Redundant with a crate root:** 80 (`too_many_lines` 57, the three `cast_*` lints
> 29, `type_complexity` 2).  The crate-root lists themselves: `src/main.rs`'s
> `match_same_arms`, `redundant_closure`, `implicit_hasher`, `unnecessary_wraps` and
> `must_use_candidate` fire nowhere in the bin; every entry of `src/lib.rs`'s list is
> live.
> **What CI never lints:** with debug assertions ON (`[profile.dev.package.loft]`
> strips them) 10 pedantic warnings hide — `ptr_as_ptr` ×4, `ref_as_ptr` ×2,
> `borrow_as_ptr` ×2, `useless_borrows_in_formatting`, `missing_panics_doc`; in the
> browser wasm rlib 7 — `needless_return` ×2, `format_push_string` ×2,
> `drop_non_drop` ×2, `unnested_or_patterns`.  Either configuration fails `-D warnings`
> today if it is ever gated.
> **Outside the census:** 167 `#[allow]` naming only rustc lints (`dead_code` …), 111 of
> them dead in every compiled leg.
> The per-suppression table is the tool's output; `make clippy-review ARGS="--legs all"`
> regenerates it in about two minutes on a warm target.

### 9 — Generate HTML and PDF

```sh
# Regenerate HTML reference
cargo run --bin gendoc

# Compile PDF
typst compile doc/loft-reference.typ
```

Verify that `gendoc` completes without warnings and that the generated HTML files look correct in a browser.  Attach `loft-reference.pdf` to the GitHub release.

### 10 — Per-OS binaries + stdlib checksums → registry

The registry ([PKG_REGISTRY.md](PKG_REGISTRY.md)) is the trusted distribution
point, so the toolchain itself ships through it — signed, with checksums users
can verify offline.

- **Build a release bundle per supported target.**  `release.yml` does this
  automatically on a tag push (see § "Tag & publish" above) via
  `scripts/make-release.sh`, building the four shipped triples:
  - `x86_64-unknown-linux-musl`
  - `x86_64-apple-darwin`, `aarch64-apple-darwin`
  - `x86_64-pc-windows-msvc`
  - (no `aarch64-unknown-linux-*` yet — add a matrix row when it is needed.)
- **Each bundle is a self-contained zip** — `bin/loft` + `default/` stdlib +
  examples + `loft-reference.pdf` + `SHA256SUMS` — attached to the **draft**
  release as `loft-<version>-<triple>.zip` (+ its `.zip.sha256`).
- **One manifest per bundle.**  `SHA256SUMS` covers every file it ships,
  `bin/loft` and each `default/*.loft` included, and is the authoritative list of
  what a bundle owns (`self_update::owned_files` reads the same file).  There is
  deliberately no second stdlib-only manifest: it described a subset of this one,
  which made two ways to validate a single installation.
- **Publish to the registry:** splice the generated entry into the signed
  `index.json` (`loft-lang/registry`) and re-sign per
  [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).
  Per target it carries the bundle URL + sha256 **and `manifest_sha256`** — the
  digest of that bundle's `SHA256SUMS`.  The zip's own hash is checkable exactly
  once, at download; the manifest digest is what lets `loft verify-self` re-check
  an INSTALLED tree against the signature at any time.
- **Verify:** on a clean host, `loft self-update` resolves a bundle, checks its
  hash against the signed index, and installs it; `loft verify-self` then reports
  "matches the release published in the signed registry index".
- **Verify on Windows specifically — the one case no test can cover.**  Run a real
  `loft self-update` on Windows, from the previous release to this one.  Replacing
  a *running* executable is the only genuinely platform-divergent step in the
  chain: `apply_bundle` renames the target aside and copies in, because a running
  binary cannot be overwritten there but can be renamed.  The unit tests exercise
  rename-then-copy on the daily Windows leg, but never against the `loft.exe` that
  is executing them, so this needs a published release and a Windows box.  Do it
  once per release, before announcing.

### Open work — reproducible builds (@PLN78 step 7)

`make-release.sh` emits `SHA256SUMS`, which is integrity, not a byte-identical
rebuild.  Everything above works without it; what it would upgrade is the
*meaning* of the published hash — from "this is the artifact the maintainer
uploaded" to "this is the artifact the source produces", which is the stronger
claim.  Deliberately off the critical path: it was sequenced last so it could
never block a user-visible installer, and closing @PLN78 does not make it urgent.
The registry already re-checks reproducibility for *libraries* (gate 3 clones the
tag and re-runs `loft package`); the toolchain is exempt because it is not a
`loft package`, so this is the gap that exemption leaves.

**Measured 2026-07-31 — what actually blocks it, so this is not re-derived.**

1. *The compiler already matches.*  The published v2026.7.2 binary embeds
   `/rustc/8bab26f4f68e0e26f0bb7960be334d5b520ea452`, which is byte-for-byte the
   local stable 1.97.1.  The usual hardest variable is already pinned by
   `dtolnay/rust-toolchain@stable` plus `Cargo.lock`.
2. *Absolute build paths are the blocker.*  The release binary carries
   `/home/runner/.cargo/registry/...`; a local build carries `/home/jurjens/...`
   — **192 occurrences**.  v2026.7.2 is therefore unreproducible by anyone,
   including us: the runner's paths cannot be recreated.
3. *`trim-paths` is NOT the answer.*  The `[profile.release] trim-paths` option is
   still unstable in Cargo 1.97.1 and refuses to parse the manifest.
4. *`--remap-path-prefix` works — 192 → 2.*  Stable rustc, no nightly.
5. *…and the last 2 are self-inflicted, in a way that has no cheap fix.*
   `build.rs` exports `LOFT_BUILD_RUSTFLAGS` **verbatim**, so the remap flags —
   whose text contains the very paths being removed — are baked in as a string
   literal.  **Hashing it away does not work:** `cache.rs` only needs a
   fingerprint, but `extensions.rs` passes the *string* to child cargo builds so
   a shared transitive dep gets an SVH matching loft's own (#274); replace it
   with a `u64` and `--native` breaks at link with a colliding `StableCrateId`.
   (This was proposed here on 2026-07-31 after reading only the `cache.rs`
   consumer, and disproved the same day by reading the other one.)

   So the string must stay, which means the string must be **machine-independent**
   — the remap prefixes have to be paths that are identical everywhere.  That is
   a canonical **build environment**, not a compiler flag: a fixed source
   directory, a fixed `CARGO_HOME`, and a fixed `RUSTUP_HOME` (the toolchain path
   accounts for 31 of the 192).  In practice that means building the release in a
   container, which is what reproducible-build systems do and what the plan's
   "M, 3-5 days" was probably right to reserve.

   Do NOT add `--remap-path-prefix` to `make-release.sh` on its own: it strips
   190 of 192 paths but leaves the build machine-specific anyway, while
   perturbing the RUSTFLAGS string that #274's SVH matching depends on — motion
   with the risk and none of the payoff.
6. *Comparing to a GitHub artifact needs the musl target.*  Releases ship
   `x86_64-unknown-linux-musl`; a local `cargo build --release` is `-gnu`.  Those
   are different binaries by construction — `rustup target add
   x86_64-unknown-linux-musl` plus `musl-tools` before any comparison means
   anything.

So the remaining work is: a canonical (containerised) build environment, the
remap flags derived from *its* fixed paths, and a CI leg that builds twice from
different original locations and diffs.  The first of those is the real cost, and
it is the piece a flag-level fix cannot substitute for.

---

## Tooling prerequisites for release verification

These are the host-side tools used to verify a release before
tagging.  Install instructions live with each tool's upstream
docs (don't duplicate them here — they rot).  When a release
adds an item that needs a new tool, add the tool here.

| Tool | Used for | Install hint |
|---|---|---|
| Rust toolchain (`cargo`, `rustc`) | Build + test loft itself | https://rustup.rs |
| `cargo nextest` | CI-locally test runner (matches CI matrix) | `cargo install cargo-nextest` |
| VS Code | SH.1 grammar visual sanity + SH.2 extension verification | https://code.visualstudio.com |
| `vsce` | VS Code extension packager (`vsce package` for SH.2) | `npm install -g vsce` (needs Node 20+) |
| `gdb` | NDB.0 quality gate (Linux primary debugger) | OS package manager |
| `lldb` | NDB.0 quality gate (macOS primary, Linux alternative) | OS package manager / Xcode CLI tools |
| `objdump` | DWARF inspection for NDB.0 (`-h` lists debug sections) | OS package manager (GNU binutils) |
| `node` | JS-glue probes for browser quality gate; `vsce` runtime | https://nodejs.org (20.x+) |
| `python3` | JSON validation (`python3 -m json.tool`); generic scripting | OS package manager |
| `gh` | `make release-gate` (dispatch + watch) and the checklist's CI-reading items (`A-release-gate`, `A-draft`, `A-smoke`) | https://cli.github.com (needs the `workflow` scope) |
| `chromium` / `google-chrome` | WASM HTML build verification (already used by `make wasm-html-test`) | OS package manager |

### The per-release checklist — `make release-checklist`

**Work the generated list, not this document.**  Everything a release needs a
human to do is one command:

```
make release-checklist                     # the list for Cargo.toml's version
make release-checklist ARGS="--fetch"      # refresh origin/main + tags first
make release-checklist ARGS="--done M-install-sh --note 'ran on the NUC'"
```

It exists because the alternative was three overlapping partial lists in this
file, and the steps that lived in **none** of them — the Windows `self-update`,
the registry splice, `scripts/install.sh` — are precisely the ones that got
skipped.  Not because anyone decided to skip them: because no list said them.

Three things make it worth working through rather than reading:

- **Automatic items are measured on every run and cannot be ticked.**  "Is
  `make ci` green" is not a promise a human gets to make — it is `result.txt`'s
  verdict line, and a verdict older than the newest source file reports STALE
  rather than pass.  A gate you can tick is a gate that gets ticked.
- **Manual items carry the exact command and what counts as a pass**, and are
  the only ones `--done` accepts.  Progress lives in `releases/<cycle>/checklist.json`, committed
  (local, gitignored) with a timestamp and your note as the evidence.
- **Items for work this release did not touch stay hidden.**  The VS Code
  extension pass and the native-debug gate are rituals for code most releases
  never change; the script asks git whether they moved since the last tag.  A
  list that includes work nobody needs to do is one people learn to skim.

Two properties added by @PLN156, because 2026.8.0's real cost was discovering by hand
that several "done" things had never been true:

- **Every item carries its CADENCE, and the early views are commands.**  `[mid]` marks
  an item that runs meaningfully at the cycle's HALFWAY point, because it measures
  overall stability rather than a release artifact (the leak/valgrind sweeps, the
  monthly reviews, the liveness census, `A-registry-prev` — the step-4 backstop, worth
  asking early); `[pre]` marks what can be finished in the month's LAST DAYS as
  pre-work, so the release does not spill deep into the new month (changelogs, the PDF,
  the reference review, the release gate on a near-final candidate); unmarked items
  need the release window itself (the tag, the draft, or the published assets).
  `make release-checklist ARGS="--phase mid"` (or `pre`) shows and measures exactly
  that slice.  An early run of a tag-candidate item (valgrind, leaks, release-gate,
  wasm) is early warning, not its tick — redo it on the candidate.
- **A check that could not run never reads as a check that passed.**  The summary names
  every automatic item that stayed UNKNOWN ("not green: … never ran"), the exit code
  keeps red (1) apart from not-yet-evidence (3) and green (0), and the header stamps
  the commit the run measured — release evidence is "these gates ran on this commit",
  never "nothing was red".

**The liveness census — a gate that checks the gates are live.**  `make
release-liveness` (the `M-liveness` item makes it per-release; it is a REPORT, never a
gate) walks the three drifts that accumulate BETWEEN releases: suppressions whose
justifying issue has since CLOSED, gate workflows that quietly stopped firing or whose
last verdict was red, and checklist items never recorded as run in any committed cycle
— the 2026.8.0 class, rescued by hand exactly once and now asked continuously.

Two corrections it carries that this document used to get wrong:

- The hands-on smoke is run **from the release ZIP, not a fresh git clone**.
  The clone is a different path from the one users take, and it was the only
  one anybody ever exercised — so the thing we smoke-tested was not the thing
  we shipped.  (The tag pipeline now runs each bundle too; see below.)
- `scripts/install.sh` — the documented `curl | sh` path — is run end to end by
  `tests/self_update_swap.rs` against a bundle built the way `make-release.sh`
  builds one, served over `file://` (Linux x86_64, the host the script's `uname`
  mapping names in CI); `tests/doc_hygiene.rs` checks statically that the mapping
  matches `PUBLISHED_TRIPLES`.  What only the by-hand run covers is the real
  transport — the GitHub CDN and the sidecar it serves — and the SHIPPED binary's
  own `verify-self`.  The script used to copy `bin/` and `default/` only and then
  hand `verify-self` the manifest of the whole bundle, so every installation it
  made ended in *the installation does not verify*; 2026.8.0 shipped that way and
  the by-hand item did not catch it.

The per-item landing procedures in the release's plans are separate and still
apply (e.g. NDB.0 in [`plans/34-native-debug/`](plans/34-native-debug)).

**What it covers, audited against this document (2026-08-31).**  Every gate this
file calls a release blocker is an item: the safety gate's valgrind, zero-leak,
zero-ignore and skip-list rows (`M-valgrind`, `M-leaks`, `M-ignores`, with
`A-ignores` checking the rationales mechanically), the WASM endpoint gate
(`M-wasm`), the nightlies (`A-release-gate`: one deliberate run of all six against
HEAD's commit, measured — it replaced six hand-dispatched, hand-ticked items), step 9's
artefacts, step 10's
binaries and registry entry, and the monthly reviews the cadence makes
per-release work (`M-monthly-docs`, `M-monthly-bugs`, `M-close-plans`).

One of those is worth calling out because it is invisible and it ships:
**`make-release.sh` copies `doc/loft-reference.pdf` into all four bundles and
never builds it.**  The HTML docs are regenerated by the tag's `docs` job, so
they cannot go stale; the PDF is a committed file that only `gendoc` + `make
pdf` update by hand, so a release can ship four bundles carrying a reference
that does not describe it, in silence.

Three checks cover it, and only the first is about regeneration:

- **`A-pdf`** — current against what actually decides its content.  Not against
  `doc/loft-reference.typ`: that file is *itself* generated by `gendoc`, so
  comparing the two answers a question nobody asked — when the real inputs move
  and nobody re-runs `gendoc`, both derived files sit still and the comparison
  reads green.  The inputs are `tests/docs/`, `default/`, `src/gendoc.rs`,
  `src/documentation.rs` and `Cargo.toml`.
- **`A-pdf-version`** — the PDF *says* it is this release, read out of its own
  bytes.  `gendoc` stamps the title page and the document keywords from
  `CARGO_PKG_VERSION`, so bumping `Cargo.toml` without re-running it leaves a
  reference headed "Version «previous»" — freshly dated, correct-looking, and
  wrong on the one page every reader sees first.  A timestamp cannot catch that.
- **`A-pdf-content`** — what is INSIDE it, chapter by chapter.  A PDF can be
  freshly built, correctly versioned, and still be missing a chapter, because
  **every way a chapter enters this document can drop it in silence**:
  `documentation::get_topic_sources` builds the 35 topics with `.ok()` and
  `filter_map`, so a topic file it cannot read is skipped; and *Getting
  Started*, *vs Rust*, *vs Python* and *Roadmap* are each read from a
  `doc/*.html` file with `if let Ok(…)`, so a missing file takes the chapter
  with it.  Either way the build succeeds, the page count is still three
  figures, and the page is missing only to the reader.

  So the check walks every level-1 part: each topic's `@NAME` (the heading
  `gendoc` emits — not `@TITLE`) and each of those four chapters must appear in
  the PDF's text.  *Standard Library* needs asking about twice, because its
  heading is pushed **unconditionally** — the heading proves only that `gendoc`
  ran, and an empty chapter carries it just as well as a full one, so the check
  also requires that the chapter names at least one stdlib function.  It matches
  on word boundaries: a plain substring test counts `map` as present because the
  chapter list contains "Road**map**", which is enough to stop that guard ever
  reaching zero.  Finally, no placeholder marker (`TODO`, `FIXME`, `TBD`, "not
  yet implemented") may ship in a document read offline.

  The stdlib count rides along as evidence rather than a gate: the reference
  documents a good share of functions as methods on their receiver, so "every
  `pub fn` appears" would be a false failure and a percentage would be an
  invented threshold.  A presence check can still pass on a chapter that was
  dropped but whose name occurs in prose — that residual is the right way round,
  a possible false pass on a name collision rather than a false alarm.

None of the three reads a sentence, so all three stay green on a chapter that
describes behaviour the language dropped two releases ago.  That half is
[REFERENCE_REVIEW.md](REFERENCE_REVIEW.md) — a per-chapter pass over what the
reference *promises*, tracked by a watermark so it can be done **early and
continuously** rather than as a day of reading under tag-day pressure.  Read a
chapter the week its source moves and the list is short by construction:

```
make reference-review                                   # what owes a read
make reference-review ARGS="--done tests/docs/07-vector.loft"
```

`A-reference-review` reports the count on the release checklist.

The same watermark pass covers the **agent skills** (`.claude/skills/`), which are
loaded *instead of* the canonical docs they paraphrase and so drift the same way the
reference does — [SKILLS_REVIEW.md](SKILLS_REVIEW.md) defines the three axes of that
read (content / usability / conciseness), `make skills-review` is the worklist, and
`A-skills-review` reports the count.

### What the tag pipeline proves about the artifacts

`release.yml` used to build four bundles and upload them without executing one,
so every artifact-level property — does the binary run, is it the version the
tag claims, does the manifest still describe the files, do the shipped examples
work — was checked by nobody.  Those properties do not exist before the zip
does, which makes the tag run the only place they can be checked at all.

Each build leg now unpacks **its own zip** (not the staging directory: the
round-trip is part of what is under test) and asserts `--version` against the
tag, `loft verify-self`, and every `examples/*.loft` under `--interpret`.  The
example check asserts **empty stderr**, not just exit 0 — a loft program that
cannot write its output file prints `… — write skipped` and exits 0, so an
exit-code-only smoke passes on a bundle whose examples do nothing.

One leg cannot always run its own artifact: `x86_64-apple-darwin` is
cross-built on an arm64 runner and needs Rosetta 2.  It reports a loud skip
rather than failing the release, and `make release-checklist` reads the run's
annotations so a skipped bundle becomes a manual item instead of a silence.

---

## Release Artifacts Checklist

| Artifact | Required | How |
|---|---|---|
| GitHub release tag `v1.0.0` | Yes | `git tag v1.0.0` |
| Linux static binary (`x86_64-unknown-linux-musl`) | Yes | GitHub Actions + `cross` |
| macOS Intel binary (`x86_64-apple-darwin`) | Yes | GitHub Actions matrix |
| macOS ARM binary (`aarch64-apple-darwin`) | Yes | GitHub Actions matrix |
| Windows binary (`x86_64-pc-windows-msvc`) | Recommended | GitHub Actions matrix |
| `loft-reference.pdf` attached to release | Yes | `typst compile doc/loft-reference.typ` |
| HTML docs on GitHub Pages | Recommended | `cargo run --bin gendoc` → `gh-pages` branch (automated in release.yml) |
| crates.io publish as `loft` | Recommended | `cargo publish` (automated in release.yml via `CARGO_REGISTRY_TOKEN`) |
| `loft.1` man page | Optional | Generate from README with `pandoc` |

---

## Post-1.0.0 Versioning Policy

**Semantic versioning with a roughly monthly release cadence:**

- **1.0.x patch** — bug fixes only; no new language features; no behaviour changes; always backward-compatible.  Example: fix a crash found after 1.0.0 ships.
- **1.x.0 minor** — new language features that are strictly additive (new syntax, new stdlib functions, new CLI flags, new IDE capabilities).  Any program valid on 1.0.0 must compile and run identically on 1.x.0.  Candidates: P2 (REPL), A5 (closures), A7 (native extensions), Tier N (native codegen).
- **2.0** — reserved for breaking language changes.  Not expected in the near term.

The stability guarantee applies to the **loft language surface** (syntax, type system, documented stdlib, CLI flags) and the **public IDE API** (`compileAndRun` / `getSymbols` JS interface).  The Rust library API (`lib.rs`) is not a public stable API until explicitly stabilised.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog; source for gate-item IDs
- [ROADMAP.md](ROADMAP.md) — Items grouped by milestone with effort estimates
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — All known inconsistencies must be resolved or accepted before 1.0.0
