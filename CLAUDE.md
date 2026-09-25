
# Claude Code Instructions for the Loft Project

> These instructions OVERRIDE default behavior — follow them exactly. The MANDATORY
> sections (branch / debugging / bug-filing / git-safety) are hard rules.

## What loft is

**loft** is a tree-walking interpreter (Rust) for the **loft** language: statically typed,
expression-oriented, struct/enum, store-based heap, stdlib from `default/*.loft`. Two backends:
the interpreter and `--native` (compiles via `rustc`). It's the **language** layer of a stack:
**lavition** (the engine/brand) → **loft** (this repo) → games **moros**/**dryopea** + consumer
libs (crawler, `lib/markdown`) that dogfood the language. History: [LAVITION.md](doc/claude/LAVITION.md).
Developed almost entirely by AI agents (steered; docs + tooling prioritized above code), so
everything needed to work on loft is in this repo — [BUS_FACTOR.md](doc/claude/BUS_FACTOR.md).

## Dogfood loop

**Build a real consumer → harvest the lessons → fix the language → ship.** Prefer the path that
exercises a real consumer; when a slice surfaces a gap, fix on the spot if XS/S, else route to its
canonical home ([DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)).
**Two-agent split:** this stream BUILDS + FIXES the language and documents the contract; the
consumer's own agent USES + adversarially BREAKS it and reports gaps.
**Library work is in scope HERE (owner, 2026-09-25)** — the `loft-libs-*` repos (the published
libraries: fix, test, republish via the **loft-ship skill**) are this stream's to edit, never
delegated to a dogfood project's agent — a consumer reports the gap, this stream builds it and
tests it on the library's TESTBED (its own CI run locally, declared dependencies only, no consumer
in the loop): [LIBRARY_AUTHORING.md § The testbed](doc/claude/LIBRARY_AUTHORING.md).  The
consumer APPLICATIONS below stay read-only.
**Edit ONLY this repo and the libraries** — the symmetric half of the consumer's "the engine is read-only" rule.
Read their tree freely (source, docs, `git log`, their `LOFT_HANDOFF.md`); never write to it. They
are often working in it concurrently, so a staged test file or a `git checkout` lands in someone
else's uncommitted work. Verify a consumer-reported bug from a **scratchpad** package that points at
their libs by path (`--lib <their>/lib`, or a `path =` dep) — note `loft test` inside their package
is NOT read-only: it rebuilds `native-auto/`, writes `.loft/` caches, and a file-writing test can
delete a tracked file of theirs. Report back in our docs, not in their files.

## Key commands

```bash
cargo run --bin loft -- prog.loft        # run     |  -- repl  |  -- introspect prog.loft  |  -- --help
loft debug prog.loft:12 [--lib dir]      # STOP at line 12: read/edit the live frame, step
                                         #   (pipe commands on stdin; `--rpc` = scripted NDJSON)
                                         #   reach for this INSTEAD of adding println — DEBUG.md
cargo run --bin gendoc                   # regenerate doc/*.html
make ci                                  # fmt → clippy → test (full local gate).  Unreliable
                                         #   here (memory, a sibling gate)?  Run the SAME gate
                                         #   on GitHub, no PR needed: `gh workflow run ci.yml
                                         #   --ref <branch> -f os=ubuntu-latest` — CI_BUDGET.md
                                         #   § When the local gate is unreliable
make test                                # clippy + test → result.txt
make check-rlib                          # 1s pre-flight: is libloft.rlib current? RUN IT
                                         #   BEFORE a bare `cargo test` — `cargo build
                                         #   --bin loft` never rebuilds the lib rlib the
                                         #   native tests link, and a bare `cargo test`
                                         #   builds no rlib either (`make ci` builds all
                                         #   three itself, so it needs no pre-flight)
./scripts/find_problems.sh --changed [ref]      # SECONDS — the tight loop: the subjects YOUR
                                         #   DIFF touches (uncommitted edits, or vs `ref`),
                                         #   an edited tests/<x>.rs picks its own binary, an
                                         #   edited corpus file picks the corpus runners.
                                         #   Falls back to the curated set, saying why, when
                                         #   the diff touches what every binary depends on.
./scripts/find_problems.sh --subject <name>     # one area by name; use this or --changed while
                                         #   iterating, not `make ci`.  Subjects: parser scopes
                                         #   codegen runtime store wasm packages lsp sql docs
                                         #   host (`--list-subjects` to see them + exclusions).
                                         #   Shape: --changed/--subject while iterating → the
                                         #   two clippy variants + fmt → ONE `make ci` before
                                         #   committing (it runs your diff's subjects FIRST, so
                                         #   a red shows in its first minute; and it QUEUES
                                         #   behind another checkout's gate — one at a time on
                                         #   this box, LOFT_GATE_PARALLEL=1 to run beside it).
                                         #   `make ci` is ~10 min and only that if the box is
                                         #   idle — two checkouts running gates at once doubles
                                         #   it (CI_BUDGET.md § A LOCAL `make ci`).
./scripts/find_problems.sh --bg|--peek|--wait   # background full-suite run + inspect/block
cargo test --release --test ir_schema_roundtrip   # the IR codec over the whole stdlib + every
                                         #   tests/scripts file; run it after an IR-schema/
                                         #   serialiser change (`make ci` runs it in the pool)
make release-gate                        # every nightly against THIS commit in ONE CI run, one
                                         #   verdict — the release evidence (`release-checklist`
                                         #   reads it by HEAD's sha).  The 03:00 daily starts
                                         #   03:34–14:45 UTC on whatever main was; this does not
                                         #   excuse a red nightly — fix those the day they appear
make falsify GUARD=<guard.loft> REF=<commit>   # does this guard FAIL on the build it was
                                         #   written to catch?  Compares exit/asserts/leak/
                                         #   panic apart and names the channel that moved.
                                         #   Every new tests/scripts file records its answer
                                         #   (`@falsified-at:`, gated) — TESTING.md
make speed                               # what got slower/faster — a REPORT, never a gate
make perf-portal                         # WHERE NATIVE STANDS AGAINST RUST, BY CLASS: measures
                                         #   every bench lane here and renders ONE page,
                                         #   doc/claude/PERF_PORTAL.md — a median per mechanism
                                         #   class (keyed, vector-build, call, record-field, …),
                                         #   every routine under its class, and the surveyed
                                         #   library routines still waiting for a row (@PLN158).
                                         #   `perf-portal-render` re-renders from saved runs;
                                         #   PACKAGES="--package <scratch clone>/drawing=drawing"
                                         #   adds a library's own bench.  A REPORT, never a gate
python3 bench/stats.py                   # native vs the Rust reference per bench routine, as
                                         #   STATISTICS: --n calibrated per lane, a warm-up
                                         #   round dropped, 7 interleaved samples pinned to the
                                         #   fastest core, the ratio with its RANGE and an
                                         #   ok / OVER / unclear verdict against --bar (2.0);
                                         #   hashes must agree across lanes.  `--tsv` keeps a
                                         #   run to diff against the next — bench/README.md
make ops-census                          # which bytecode operators anything still EMITS:
                                         #   live / unexercised (a site emits it, no program
                                         #   does — a test gap) / orphan (nothing emits it and
                                         #   nothing in src/ names it — a retirement CANDIDATE,
                                         #   never a verdict).  A grep cannot answer this: the
                                         #   parser COMPUTES most operator names, so `OpEqText`
                                         #   is emitted by every text `==` and appears nowhere
                                         #   in src/.  A REPORT, read per release — RELEASE.md
make profile ARGS="--interpret p.loft"   # which loft FN/LINE/PATH burns the time; PROFILE_FLAGS=
                                         #   "--mem" heap by loft line at the PEAK, "--paths" the
                                         #   paths that reached each allocation, "--engine" perf
                                         #   over loft's own Rust.  `make profile-corpus` checks
                                         #   the instruments against known answers — PERFORMANCE.md
make index ; ./scripts/idx tag:@P259     # rebuild + query the tracker index (prefer over grep -rn)
make work                                # the open issues that are PICK-UP work: minus
                                         #   `fixed-pending-merge` (fix landed, awaiting merge)
                                         #   and `status:planned` (fix path is a PLAN, measured
                                         #   there).  START HERE to find the next task, and use
                                         #   `ARGS=--count` for the scalar.  A failed query is
                                         #   exit 2, never an empty list — ISSUE_TRACKING.md
make sweep-scratch                       # reclaim loft's temp scratch (dead-process native
                                         #   artefacts, aged test caches, old agent sessions);
                                         #   `df -h /` before a gate — a full disk fails the
                                         #   NATIVE corpus with a code-shaped message
make view                                # branch-aware doc/code viewer; binds LOOPBACK,
                                         #   LOFT_VIEW_PORT (default 8765).  Remote:
                                         #   ssh -N -L 8765:127.0.0.1:8765 <host>
```

**Bound ad-hoc runs** (loft is unbounded by default; tests already arm a 300s watchdog). Especially
for `--native` (rustc can hang): `LOFT_TIMEOUT=60 loft --native p.loft` or `loft --timeout 60 p.loft`
(0 = off). Hard-kills at `timeout+grace` (grace 2s, `LOFT_TIMEOUT_GRACE`). Ref: DEBUG.md, TESTING.md.

**A time bound does not bound MEMORY.** A corrupted length ends in a bad dereference on one run
and an unbounded ALLOCATION on the next — loft#796 reached 59.6 GiB in seconds and the global OOM
killer took two unrelated agent sessions with it. Test runs (`--tests` / `loft test`) therefore
carry a **2 GiB store-heap ceiling**; crossing it stops the run at that growth and names the TYPE
that filled the heap, with a one-store-vs-many breakdown that tells a runaway length from a leak.
`LOFT_MEMORY_LIMIT=<2G|512M|0>` overrides it; ordinary runs are never capped. When writing a
repeat-run harness for a corruption repro, cap the process too (`ulimit -v`) — the runaway is not
necessarily the process the kernel kills. TESTING.md § Store-memory ceiling.

**Under debug assertions a third bound applies:** the interpreter stops after `LOFT_MAX_OPS`
operations (default 4e9, `0` = off) and prints the last sixteen ops as `function+offset: OpName` —
reach for it, set LOW, when hunting a hang, because it names the loop a timeout can only time out
in. Absent from every release build. ⚠ **And absent from your ordinary debug build too** —
`[profile.dev.package.loft] debug-assertions = false` in `Cargo.toml` strips it (and the other
92 `#[cfg(debug_assertions)]` items in `src/`) from both `cargo build --bin loft` and the test
binaries; flip that line and rebuild into a separate `--target-dir` to use it. TESTING.md § Hang
guard has the recipe and the measurement. It is a count, so it cannot tell a long run from a hung one:
at 100M it was tripping legitimate library tests and reporting them as infinite loops, which read
the debug-assertions gate as known-red (loft#919). TESTING.md § Hang guard.

For any multi-failure refactor, start `find_problems.sh --bg` before editing (detached
`cargo test --release --no-fail-fast` → `/tmp/loft_problems.txt`).

## Tracker tags <!--noindex-->

`@`-prefixed so regex is unambiguous: **`@P259`** P-issues; **`@PLN3`** = a `loft-lang/plans` issue
(canonical; plan id = issue number); **`@PLAN22`** = legacy local plan dir (migrating to `@PLN`);
**`@F7`/`@I81`** = a `loft-lang/features` issue (the feature/infra catalogue, @PLN92) — the ISSUE
is canonical; `index/features.json` + `doc/features/` + `tests/docs/features/*.loft` are its
GENERATED shadow (never edit them: edit the issue, then `make features-fetch && make features-gen`;
the `features-check` drift guard fails on hand-edits).
File a NEW plan as a `loft-lang/plans` issue with one `status:*` + one `subject:*` label. Look refs
up with `./scripts/idx` (`make index` first if stale; `./scripts/idx help` for queries).

**A FORMAL RULE is `@FR-`-tagged — `@FR-B-Copy`, `@FR-L-Null`, `@FR-D-bind-11`** — and a code
site that enforces one CITES it, so *"which sites enforce this rule?"* is a grep and *"is this
rule already implemented somewhere?"* is a lookup. `scripts/rule_tags.py` is the tool
(`list` · `check` · `sites <tag>` · `dups` · `registers`); `check` gates that every citation
resolves and no rule is defined twice, and **`registers` reads each chapter's stated
`OPEN: n` against the entries actually attributed to it** — its `-history` companion's
included, because a chapter that delegates its register states a number nothing beside it
can check (`types.md` read `OPEN: 0` over an open `D-Domain-Guard` next door). With
`--issues` it also names an open deviation whose issue the tracker has closed, which is a
pair to RE-MEASURE and not a closure: of the first five, four were stale and one was not.

⚠ **A bare `@Name` is NOT unambiguous here** — `@` already carries the tracker tags above, the
worked-example family (`@AAA-###`) and the corpus annotations (`@ARGS`, `@NAME`, `@IGNORE`,
`@EXPECT_ERROR`); a bare-`@` reading of `src/` returned **4142 hits, not one of them a rule**.
`@FR-` cannot be confused with `@F<digits>`, whose next character is a digit. Citations are
**boundary-exact** (`@FR-B-View` does not match `@FR-B-View-Base`) because 21 of the 285 defined
rules are a prefix of another — a general rule and its refinements share a stem, and renaming
them to dodge a matcher is the worse trade. Only a DEFINED rule is a citation target: `B-Ref`,
`D-op`, `D-own`, `D-cap` and `D-op-null` read like rules and are family PREFIXES used in prose.
Detail: [formal/README.md § Rule tags](doc/claude/formal/README.md),
[formal/IMPLEMENTATIONS.md](doc/claude/formal/IMPLEMENTATIONS.md).

## Architecture — execution path

```
src/main.rs            CLI; loads default/ then user file
 └ src/parser/         two-pass recursive-descent → Value IR
     mod.rs(core) definitions.rs(enum/struct/typedef/fn) expressions.rs(expr/assign)
     operators.rs(op dispatch/coercion) vectors.rs fields.rs(index/field) objects.rs(vars/construct)
     collections.rs(for/map/filter/par) control.rs(match/parse_call) builtins.rs
   src/lexer.rs  src/typedef.rs(type resolution+offsets)  src/variables/  src/scopes.rs(scope/lifetime)
 └ src/compile.rs      IR → flat bytecode; inits native registry
 └ src/state/          executes bytecode: mod.rs(State/execute) text.rs io.rs(file/db) codegen.rs debug.rs
   src/fill.rs         opcode implementations
```

## Key data structures

| Type | File | Purpose |
|---|---|---|
| `Value` / `Type` / `Data` | `src/data.rs` | IR node / static type / table of named defs |
| `State` | `src/state/mod.rs` | bytecode stream + runtime stack |
| `Stores` / `Store` | `src/database/mod.rs` / `src/store.rs` | all stores+schema / raw word-addressed heap |
| `DbRef` | `src/keys.rs` | universal pointer (store_nr, rec, pos) |

## Conventions

- User fns stored as `"n_<name>"` — `data.def_nr("n_foo")`, not `"foo"`.
- Native stdlib: globals `n_<func>`; methods `t_<LEN><Type>_<method>` (LEN = chars in type name),
  e.g. `t_4text_starts_with`. Operators `OpCamelCase` (loft) → `op_snake_case` (`fill.rs`).
- `#rust "..."` in `default/*.loft` supplies the Rust body for codegen. Full naming + null-sentinel
  rules: [CODE.md](doc/claude/CODE.md).
- **A new op or builtin renumbers `index/target_surface.json`** — run `make surface-gen`, or
  `make ci` fails a drift guard no targeted suite shows. It asks a PREBUILT wasm rlib which
  methods exist, so rebuild that rlib first or it records the new builtin as unavailable in
  the browser and commits that as derived truth.
- stdlib load order: `01_code.loft` (operators/math/text/collections) → `02_files.loft` (I/O) →
  `03_text.loft`.
- **Before non-trivial functionality, check the library catalogue (`make libcatalogue`) + `loft install`** — don't reimplement.
  Writing/reviewing `.loft`: **loft-write skill**. Language ref: [LOFT.md](doc/claude/LOFT.md), [STDLIB.md](doc/claude/STDLIB.md).
- **A library's API: build the catalogue with `make libcatalogue`, then read the (local, git-ignored)
  `doc/claude/LIBRARIES.md` — NEVER a clone or installed copy (@PLN112).** The catalogue is a **local
  build, not committed data** — a generated view of `published` + each lib's `origin/main` (breakage-
  flagged), rebuilt on demand so it can't go stale (committing it only churned the repo for no benefit);
  a local clone / `~/.loft/registry/<pkg>-<ver>/` can silently lag `origin/main`
  (the `find`→`search` failure that motivated @PLN112). For the machine-/context sources run
  the overlay: `scripts/lib-overlay.py <name>` (local checkout + this project's pin),
  `scripts/proposal-review.py <name> <ref>` (a proposed candidate). We never auto-delete a
  copy — each is a legitimate source.
- **Three optimisation tiers, kept apart** (NATIVE.md § Optimisation tiers): a SEMANTICS run
  (`--native`, the test runner) keeps every tier and a fast compile; a PERFORMANCE lane
  (`native_ratio.sh`, `run_bench.sh`, a consumer bench) and a SHIPPED binary
  (`--native-release`, every library cdylib) are lean and fully optimised
  (`-C opt-level=3 -C codegen-units=1`).  A perf number taken on the semantics build
  is not a measurement of what ships.
- **User-facing output** (anything a command PRINTS): silence when nothing needs acting
  on; no plan tags / phase names / "not yet implemented" in it; the full explanation only
  on failure. loft is meant to be BORING — noticed only in its absence
  ([GOALS.md](doc/claude/GOALS.md), [DOC_QUALITY.md § D](doc/claude/DOC_QUALITY.md)).
- **Response shape:** lead with the ONE highest-leverage item in full (decision + minimum to act),
  then a one-line summary of the rest; no long dumps.
- **Advice vs action:** asked for advice/evaluation → give the recommendation (best option + why),
  don't bounce it back. Asked a question → answer it. Act/edit only on an explicit do-it instruction.
- **Shared knowledge:** anything durable an agent records to private memory must ALSO land in the
  right repo doc (memory is per-agent; the repo is the shared channel). Keep machine-specific values
  out of shared docs.

---

## Branch policy — MANDATORY

`main` is the release branch; every commit on it must be releasable.
1. **Never commit on `main`** — land on a feature branch, reach main only via a GitHub PR (never a
   local `git merge`).
2. **Push proactively — it's a SAFETY rule.** Once a change settles (compiles / tests green), commit
   + push to the feature branch so it isn't lost. Separate from opening a PR.
3. **Never create a branch, open, or merge a PR without an explicit user ask** ("create PR",
   "merge", "switch branch"). "fix X" / "push" / "retry" are NOT such asks; a prior ask doesn't carry
   over. If a protected branch blocks a commit, surface it and ask — don't invent a branch.
   ⚠ **Nor is "the work looks finished" an ask.** A PR is opened when the work reaches a stable
   point — a closed arc, a clean endpoint — and **the owner is the one who determines that**, not
   the agent that judges its own branch ready. Keep committing and pushing to the branch; wait to
   be told.
   ⚠ **Target cadence is ONE OR TWO STABLE PRs A DAY** — that cadence works and is not the thing
   to change. Shape the work into arcs that FINISH inside it (one rule's family of defects, one
   subsystem's, one plan phase) rather than starting something that will straddle. And the bar for
   such a unit is not "green" — a PR that looks OK while carrying internal regressions is worse
   for a language's users than no PR, so a walk ships only with its own verification complete
   (both backends, the matrix built and hand-checked, guards falsified, side-findings filed rather
   than left implicit).
   ⚠ **What the cadence does NOT mean is a slow START.** Once the owner asks, opening is minutes,
   not an hour: the PR's own `ci.yml` is a required check that runs the SAME gate on the exact sha,
   so **do not hold the opening for a fresh local `make ci` on the joined tree** — that is a slower
   duplicate of a check the PR is about to run, and on a shared box a sibling's `pkill` can kill it
   (measured 2026-09-10: three local gates killed, ~50 minutes lost, PR still unopened while three
   streams kept re-joining). Run a local gate AFTER opening if you want the earlier signal.
   What must be true BEFORE opening is cheap to check and is the whole list: the head is current
   on `origin/main` (rule 6), everything is pushed, the tree is clean, derived artefacts are
   REGENERATED rather than picked (audit rows re-measured, `falsified_docs.baseline`, the browser
   bundle), and each fix in the arc carries the verification it owed when it landed.
   ⚠ **A branch a sibling has joined TWICE is overdue** — a backstop, not the cadence: at one or
   two a day it should never fire. It replaces "accumulated commits are not a reason", whose
   premise was *cherry-picking makes batching free*. It is not free: measured 2026-09-10, one
   branch carried **9 joins** before its PR, the walker-audit rows were re-measured five times,
   `falsified_docs.baseline` was regenerated and the browser bundle rebuilt — and the siblings paid the same cost in their own trees
   (@PLN157 carries its own "re-measure the derived rows, re-falsify both guards" commit). `main`
   is the only place a resolution is shared ONCE; until then every stream re-derives it. Two
   join-only defects also existed only on that union, invisible from either side.
   ⚠ **Both directions serialise the stream — pick the cheaper one, and it is time-to-MERGE that
   decides.** GitHub has no stacked PRs, so a second PR branches off the first's tip and waits for
   it ([DEVELOPMENT.md § Owner directive](doc/claude/DEVELOPMENT.md)); that is the cost of PRing
   too often. But NOT PRing serialises through joins, which is the cost above. So a PR is cheap
   exactly when it merges promptly and expensive when it sits — which makes "keep the PR
   mergeable and land it promptly" (rule 5) the load-bearing half, not an aside.
4. With an **open PR**, hold non-blocking pushes for the user's consent (force-push/rebase/surprise
   commits) — EXCEPT a push that unblocks a red required check (allowed; it can't merge while red).
5. **While a PR is unmerged, branch from the TIP of that in-flight work — NEVER fork a fresh
   branch off `main`.** `main` lacks the unmerged foundation, so a `main`-based branch can't build
   on it and **development there is impossible** (new work almost always needs what's still in the
   open PR — e.g. @PLN85's fuzz-proof needs @PLN25, which sits in the PR). Stack the new branch on
   the PR branch; rebase the whole stack onto `main` only AFTER the PR merges. Fork from `main`
   only when there is no in-flight work to build on. **Trade-off (respect it):** stacking couples
   the new work to the PR's merge clock — a clean PR merges in minutes, but a problematic one
   blocks everything stacked on it for **hours**, so keep the PR mergeable and land it promptly.
   General branch name; prefix agent branches with the hostname (`<host>-work`); the cycle's
   long-lived branch is the **monthly release branch** `YYYY-MM`; only a substantial design-doc'd
   plan earns a specific name.
6. **Before opening a PR and before requesting merge, verify the head is current on `origin/main`**
   (`git fetch`; `git merge-base --is-ancestor origin/main <head>`). `mergeStateStatus: BEHIND`
   merges as BLOCKED even when `mergeable=MERGEABLE` — rebase + re-push first.

## Debugging policy — MANDATORY

**Never `git bisect` or `git checkout HEAD -- <file>`** — both destroy uncommitted work. Instead:
read the failing test's dump (`tests/dumps/*.txt` — IR+bytecode+trace), narrow with
`LOFT_LOG=minimal`/`crash_tail:50`, read the 3–5 relevant files, `git show <commit>` for regressions.

**READ THE FORMAL SPEC FIRST when the fix has a choice in it** — `doc/claude/formal/` is the
STRICT definition (rules + a numbered deviation list driven to zero), and its doctrine is *"the
rules do not change to match the code; the code changes to match the rules."* So a rule already
written there SETTLES a question an issue may present as open. Reach for it before you deliberate
— when an issue says **"a design call"** or **"two ways to close it"**, before shipping a
**REFUSAL** (*"X is not supported"* — a rule may say it must work, making the refusal a deviation),
before changing a shipped surface's observable semantics, and whenever the **two backends
disagree**. Measured: loft#1002 was filed as *"the choice is a design call"* while
`formal/collections.md` already carried `(Slice-Open) xs[(x,y)..] open outward walk from a point`
— the tail was the deviation, so only one of the two "ways" was ever admissible. Both directions
apply: an edge the rules CANNOT express means the RULE wants extending. And an **"OPEN: 0" line is
a claim to re-measure**, only as strong as the oracle under it — `tuples.md` read 0 while
loft#1004/#1005 were live, because its oracle is all-`(integer, integer)` and carries no `text`.

**Matrix-first for any non-trivial bug (esp. crash / silent corruption) — the urge to fix is the
signal you haven't earned it:**
1. Don't fix on the first read — a clean one-line story is a hypothesis.
2. Build the matrix in throwaway `/tmp` probes on `--interpret` (`scripts/probe-matrix`), varying ONE
   composition axis per probe, distinctive values everywhere. But **count the axes you HELD FIXED** —
   a sweep varying one while pinning four reads as proof and isn't (@PLN130's broken cell needed
   nesting depth, Set-count, param kind and caller-count moved TOGETHER). Counting them by hand
   is what keeps failing, so **run `python3 scripts/matrix_axes.py file <guard.loft>`**: it
   carries a fixed vocabulary of composition axes, each with the DOMAIN the language offers, and
   answers which values your cells reach and which they do not. DERIVED, not declared — an axis
   named in your own closing paragraph is not an axis measured by it (`formal/ownership.md`
   D-own-6). `cross <A> <B>` asks the sharper question, which pairs the whole corpus never
   crosses; and an absence it reports is checkable by building that case and asking again.
3. **Hand-compute each cell's expected value** (agreement between two binaries is NOT a pass);
   prove the harness can fail (a no-output cell is vacuous); assert **value AND length AND leak**.
4. Map pass/fail → find the REAL boundary (filed scope is usually wrong). Resisting a read twice →
   instrument with one env-gated `eprintln`, don't theorize.
5. Fix at the chokepoint enforcing exactly the violated invariant — no narrower, no wider.
   The invariant is often already NAMED in `doc/claude/formal/` — cite the rule rather than
   re-deriving it, and close its deviation entry if it had one.
6. Verify the full matrix on **BOTH backends**; graduate guarantee probes to `tests/scripts/`.
7. **Propose 3+ cases to check → write them ALL down BEFORE working the first.** Detail decays
   while you work: the headline of case three survives, its specifics (which axis, which shape,
   why suspected) do not. Writing the list first also makes it reviewable while it's cheap.
8. **A new case found = a new probe, always** — the suite is the only thing that remembers.

Full flow: [DEBUG.md](doc/claude/DEBUG.md), [plans/_INVESTIGATION_TEMPLATE.md](doc/claude/plans/_INVESTIGATION_TEMPLATE.md),
the rules: [formal/README.md](doc/claude/formal/README.md) § When to reach for this doc.

## Bug-filing policy — MANDATORY

**Default is FIX, not file** — bugs surfaced while fixing another are the cheapest to fix (paths
loaded, repro warm). **An open issue is DEFERRED work, and the owner does not defer: filing is
fine as the record, but an open issue is never ignored for a release and rarely for a PR** — so
it is fixed before the release (normally before the PR) by the filer or a named peer, and
"ship with #N open" is never the recommendation. **Inside an INVESTIGATION plan, filing is not
correct at all**: a bug met while digging into why something is broken is fixed on the spot,
because leaving it in the way hides the full picture (such a plan is itself a last resort, for
when the ordinary tools no longer work). In **stability work** the file-instead-of-fix escape
hatches do NOT apply either: fix in the same session with a regression test. Record scope + root
cause, never origin commit.

**File only when NOT fixing now:** it blocks the current task (bookmark + workaround), or it's
genuinely M+/needs-design (route to its canonical home). When you file: a **GitHub Issue**
(`gh issue create`, `bug_report` template) — NOT a PROBLEMS.md row (that's the closed archive) —
with a minimal both-backend repro, `sev:`/`area:` + a VERIFIED `wa:*` label, **a `hit-by:*`
label**, and `Fixes #NNN`. **Add `silent-wrong` whenever the program answers WRONG and nothing
says so** (no diagnostic, no refusal, no crash), or a type-system promise does not hold. It is
the FREEZE axis and outranks both `sev:` and `wa:`: a clean workaround only helps someone who
learns they need one, and a `sev:low` edge that answers quietly wrong still can't be frozen into
the contract, while a `sev:high` crash can — a crash tells you. Not for a crash, a refusal, an
ICE, a wrong error message, or a leak ([.github/LABELS.md § silent-wrong](.github/LABELS.md)). `hit-by:` names the project that RAN INTO it, one per issue, **at
filing time** — loft is one of those projects, so a find of your own is `hit-by:loft`, NEVER a
blank (a consumer filters `hit-by:<their project>`, and an unlabelled issue reads as "not
established", not "nobody"). It says who hit it and nothing more: a follow-on you file while
fixing something else is still `hit-by:loft` even when a consumer's report sent you into that
subsystem. Lineage is separate and goes in the BODY as `Found-via: #N`
([.github/LABELS.md § hit-by](.github/LABELS.md)).

**Fixing an existing issue not yet on `main`:** push the fix, write `Fixes #NNN`, keep the issue open
(the `fixed-pending-merge` label is automated off that trailer) — never hand-close. **Add a
`Contract: settled|strained — <why>` trailer beside it** — settled = the formal rules already gave
the right answer and the fix makes that promise hold; strained = closing it extended a rule, changed
a documented surface, or needed a design call. The fixing commit is the ONLY moment that answer
exists, and the monthly ratio is what the contract-1 decision reads (`make bug-review` § 5,
[.github/LABELS.md § `contract:`](.github/LABELS.md)); absence counts as UNJUDGED, never settled —
the same push run that applies `fixed-pending-merge` reads this trailer and sets the `contract:`
label, and warns when a `Fixes #N` arrives without one. **Inside a
plan:** file only if it reproduces on `main`; branch-internal breakage stays in the plan's docs.
Don't scope-creep the active fix with unrelated bugs.

## Git safety — MANDATORY

**Never `git stash pop` / `git pull` / `git checkout HEAD -- <file>` with uncommitted changes** —
they merge-conflict across files and have destroyed sessions. **Always commit before any operation
that changes the working tree.** Compare without switching: `git diff main -- <file>`,
`git show origin/main:<file>`.
The plain `git checkout -- <file>` and `git checkout-index -f -- <file>` forms are the SAME
hazard wearing an innocent name: each has reverted an hour of unstaged work while "cleaning
up" a probe. Undo a probe or a debug `eprintln` with the inverse edit, never with git, and
commit BEFORE inserting it.

Run **`make hooks`** once per clone: the `commit-msg` hook reports an issue mentioned without a
`Fixes #N` trailer (that trailer is what the push workflow labels `fixed-pending-merge` off — see
the bug-filing policy above), and a Claude Code edit hook runs the documentation lint on every
file you write, reporting only what the edit added ([DOC_CONTRACT.md](doc/claude/DOC_CONTRACT.md)).
Neither blocks.

---

## Documentation index

**Writing or changing any doc or comment: [DOC_CONTRACT.md](doc/claude/DOC_CONTRACT.md) is the
rule set** (one line per rule, each pointing at its home).

**Language / stdlib:** [LOFT.md](doc/claude/LOFT.md) syntax · [STDLIB.md](doc/claude/STDLIB.md) stdlib API ·
[INTERFACES.md](doc/claude/INTERFACES.md) traits/generics · [TUPLES.md](doc/claude/TUPLES.md) ·
[COROUTINE.md](doc/claude/COROUTINE.md) (1.1+) · [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) ·
[SUBJECTS.md](doc/claude/SUBJECTS.md) the axes loft made a choice on — one row per subject with its
RATIONALE (a link to the `C` entry, never a restatement) and how the claim is VERIFIED; the
comparison pages, the bars and the registers all link into it, and a row reading `— none
recorded` is a decision nobody wrote down ·
`tests/comparisons/*.loft` the comparison claims AS PROGRAMS (`wrap::comparisons`): until
2026-09-23 `00-vs-rust.html` and `00-vs-python.html` were the only reference pages with no
`.loft` source, so their 28 samples were the only published code nothing ran ·
[bars/](doc/claude/bars/README.md) — [OCAML_BAR.md](doc/claude/bars/OCAML_BAR.md) / [LUA_BAR.md](doc/claude/bars/LUA_BAR.md) expressiveness probes against OCaml and
Lua, measured (reports, never gates — and run a generator probe inside a memory cap).

**Compiler / internals:** [COMPILER.md](doc/claude/COMPILER.md) parser/two-pass/types ·
[INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) Value/Type/opcodes/State · [INTERNALS.md](doc/claude/INTERNALS.md) ·
[SLOTS.md](doc/claude/SLOTS.md) stack slots · [NATIVE.md](doc/claude/NATIVE.md) `--native` codegen ·
[THREADING.md](doc/claude/THREADING.md) par · [CODEGEN_METHOD.md](doc/claude/CODEGEN_METHOD.md) how to do compiler work.

**Runtime / memory:** [DATABASE.md](doc/claude/DATABASE.md) stores/DbRef ·
[REMOTE_STORES.md](doc/claude/REMOTE_STORES.md) serving static data over HTTP range (paged
`store_load_key*`, no server-side code) · [LAZY_STORES.md](doc/claude/LAZY_STORES.md) a collection
bound to an image or `sqlite:` fetches on a MISS, query derived from its own type ·
[LIFETIME.md](doc/claude/LIFETIME.md) deps/freeing ·
[OWNERSHIP_MODEL.md](doc/claude/OWNERSHIP_MODEL.md) the deps north-star (borrow system) ·
[PLACEMENT.md](doc/claude/PLACEMENT.md) a library runs in this process, a worker, or another
machine — one manifest line, consumers unchanged; **the four rules for writing one that can
be placed** (a `pub fn` must not BE a native; answer a value, not a cursor; closures do not
cross; a returned VIEW cannot be placed) ·
[LOGGER.md](doc/claude/LOGGER.md) · [WASM.md](doc/claude/WASM.md) · [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) ·
[BROWSER_INTEROP.md](doc/claude/BROWSER_INTEROP.md) · [GALLERY_CI.md](doc/claude/GALLERY_CI.md) (the two browser
artefacts that go stale independently, and the gate that catches it) · [WINDOWS.md](doc/claude/WINDOWS.md) / [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md).

**Networked runs:** `LOFT_NET_PROFILE=1|trace` reports socket operations by **margin**
(a call that finished close to its deadline is a failure that has not happened yet) with
wall-clock stamps that merge two processes' streams. It records at the sockets the RUNTIME
owns — `engine_host`, `loft debug --serve`, placed-library workers; a networking LIBRARY
joins by calling `loft::net_profile::time(…)` from its Rust bridge, and the armed-but-empty
report says so rather than printing nothing (loft#1088). PERFORMANCE.md § LOFT_NET_PROFILE.

**Diagnostics:** [DIAGNOSTICS.md](doc/claude/DIAGNOSTICS.md) the code index (`advice[avoidable-copy]`)
+ `--explain` fix lines — a code is a FROZEN public surface, and a new one lands with its row.

**Testing / debug:** [TESTING.md](doc/claude/TESTING.md) framework/`LOFT_LOG`/LogConfig ·
[DEBUG.md](doc/claude/DEBUG.md) tools + boundary-matrix runner · [CAVEATS.md](doc/claude/CAVEATS.md) edge cases ·
the bisect switches: [NATIVE_SWITCHES.md](doc/claude/NATIVE_SWITCHES.md) (`--native` rewrites) /
[BOTH_BACKEND_SWITCHES.md](doc/claude/BOTH_BACKEND_SWITCHES.md) (lowering + runtime store) ·
[PROFILING.md](doc/claude/PROFILING.md) which switch arms which profiler ·
[PERFORMANCE.md](doc/claude/PERFORMANCE.md) benchmarks + profiling (its oracle: [PROFILE_ORACLE.md](doc/claude/PROFILE_ORACLE.md)) · [PERF_PORTAL.md](doc/claude/PERF_PORTAL.md) (GENERATED: every measured routine against its Rust twin, by mechanism class — `make perf-portal`) · [CI_BUDGET.md](doc/claude/CI_BUDGET.md) what runs
when + the 20-min PR rule.

**Quality / stability / formal:** [CODE.md](doc/claude/CODE.md) · [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) ·
[QUALITY.md](doc/claude/QUALITY.md) open work · [GOALS.md](doc/claude/GOALS.md) (purpose + goals A–G) ·
[BUS_FACTOR.md](doc/claude/BUS_FACTOR.md) (the development model — repo + agent, no single point of failure) ·
[STRONG_POINTS.md](doc/claude/STRONG_POINTS.md) ·
[CONTROL.md](doc/claude/CONTROL.md) the census of where the programmer is NOT in
control — the SEE/SAY test, and the count that must not grow ·
[DESIGN.md](doc/claude/DESIGN.md) algorithms ·
[DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) declined-features register ·
[DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) / [DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) ·
[FORMATTER.md](doc/claude/FORMATTER.md) · stability: [STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md)
(the tracking view) · [STABILITY_METHOD.md](doc/claude/STABILITY_METHOD.md) — incl. **§ The
rule-led walk**, the STANDING practice: pick a `@FR-` rule (not a site), split it into the
questions its sites actually ask, find each question's ONE home, then verify the RELATED cases
against it — the defects are in the disagreements, and the citation is the receipt, never the
task.  `make rule-coverage` reports the position against the contract-1 FLOORS — 70 % of rules
carrying a code annotation, 40 % an active guard — and how many rules short it is.  Those are
MINIMUMS for the freeze, not targets to stop at; no position figure is written here, because it
rots and the floor does not.  Read it as work LEFT and never readiness, since the walk takes the
most-changed and most-error-prone rules FIRST and the tail is the low-yield half /
[_SWEEP](doc/claude/STABILITY_SWEEP.md) / [_HOTSPOTS](doc/claude/STABILITY_HOTSPOTS.md) /
[_REDFLAGS](doc/claude/STABILITY_REDFLAGS.md) · [BRITTLE.md](doc/claude/BRITTLE.md) (the
survey of routines most likely to answer silently wrong, each with its hardening path) · [DEPS_INVENTORY.md](doc/claude/DEPS_INVENTORY.md) ·
formal lens: [FORMALIZATION.md](doc/claude/FORMALIZATION.md) / [TYPING_RELATION.md](doc/claude/TYPING_RELATION.md) ·
strict: [formal/README.md](doc/claude/formal/README.md) (rules + deviations driven to zero).

**Plans / roadmap:** [plans/README.md](doc/claude/plans/README.md) · [PLANNING.md](doc/claude/PLANNING.md) backlog ·
[ROADMAP.md](doc/claude/ROADMAP.md) by milestone · [BROADENING.md](doc/claude/BROADENING.md) beyond games ·
[WEB_STACK.md](doc/claude/WEB_STACK.md) the **better-PHP** end-to-end design — a client that calls a
web service (JSON + auth) and its counterpart HTTPS server (Let's Encrypt with rotation, auth,
SQL, a JSON and an HTML page), the four libraries they need, the sandbox profile for
third-party scripts, and how each part is verified on instruments the repo already has;
printable with `make pdf-doc` ·
[lib_plans/README.md](doc/claude/lib_plans/README.md) (legacy) · [STACKTRACE.md](doc/claude/STACKTRACE.md) · [SANDBOX.md](doc/claude/SANDBOX.md).

**Libraries / registry / packages:** **any library work: read
[LIBRARY_AUTHORING.md § The library contract](doc/claude/LIBRARY_AUTHORING.md) first** (the
standing rules, one line each, each pointing at its home); the rest of LIBRARY_AUTHORING is the
how (testbed → develop → publish → maintain), [LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md)
what a registry review checks, and publishing is the **loft-ship skill**.  A library's API:
`LIBRARIES.md` (generated on demand — `make libcatalogue`, not committed) · in-flight branches:
[LIBRARY_BRANCHES.md](doc/claude/LIBRARY_BRANCHES.md) · reference: [PACKAGES.md](doc/claude/PACKAGES.md)
format/targets · [API_SURFACE.md](doc/claude/API_SURFACE.md) · [REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) /
[REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) / [REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) ·
[PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) the registry's design record. REPL: [REPL.md](doc/claude/REPL.md).

**Process / issues / release:** [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) workflow ·
[ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) (open→Issues, closed→[PROBLEMS.md](doc/claude/PROBLEMS.md)) ·
[BUG_REVIEW.md](doc/claude/BUG_REVIEW.md) (the monthly bug review: `make bug-review` reports which
mechanism classes are still producing bugs + whether last cycle's keystone actually moved its
class; the pass converts ONE rising class into ONE generalization — a report, never a gate) ·
[.github/LABELS.md](.github/LABELS.md) · [RELEASE.md](doc/claude/RELEASE.md) (the process) · [releases/](doc/claude/releases/README.md) (one directory
per cycle: its state write-up and its committed checklist evidence) · [LIBRARY_DOC_REVIEW.md](doc/claude/LIBRARY_DOC_REVIEW.md) (the monthly by-hand doc review, both
halves: `make libraries-review` says which libraries owe a review or have moved since their
watermark, `make features-review` does the same for the `@F` catalogue, `scripts/doc-review.sh
--since` drills into one library's functions — all three REPORT, none gates) ·
[SKILLS_REVIEW.md](doc/claude/SKILLS_REVIEW.md) (the same watermark pass for the agent
skills: `make skills-review` says which skills owe a read because they or the docs they
cite moved; the read itself is by hand on three axes — content, usability, conciseness) · [COMPATIBILITY.md](doc/claude/COMPATIBILITY.md) (the breaking-change policy, @PLN102 arc A) · [MOVING.md](doc/claude/MOVING.md) ·
[CHANGELOG.md](CHANGELOG.md) / [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) ·
[DOC.md](doc/claude/DOC.md) (how `gendoc` renders a topic) ·
[USER_DOCS.md](doc/claude/USER_DOCS.md) — the design for the documentation a DISTRIBUTION
owes its users: the three tiers a reader's three questions need (what is there / how do I
start / what is the signature), the one-home rule that kills the drift already measured
between a library's guide and its copy in this repo, the REPL+debugger panel for the doc
pages (`src/wasm_debug.rs` is built and the pages expose only ▶ Run), and the README's
repositioning from one-game project to distribution ·
[LAVITION.md](doc/claude/LAVITION.md) · [PROMPTS.md](doc/PROMPTS.md).

**Skills** (`.claude/skills/`): `loft-write` (.loft authoring) · `loft-debug` (runtime crashes) ·
`loft-test` · `loft-codegen` · `loft-ship` (library cross-target + publish) · `engineering-rigor` /
`design-protocol` (rigor) · `doc-quality` · `draw` ([DRAWING.md](doc/claude/DRAWING.md), the method it follows) · `loft-plan-workflow`.

## Environment switches

Every `LOFT_*` switch has ONE home doc; this section keeps the dump controls and the two
diagnostic rules every session needs, and points at the rest by family.

### `LOFT_LOG` quick reference

Set before `cargo test` (controls `tests/dumps/*.txt`; also works with `cargo run` → stderr):
`full` (default: IR+bytecode+exec+slots) · `static` (IR+bytecode only) · `minimal` (exec trace) ·
`crash_tail:N` (last N lines, flushed on panic) · `fn:<name>` · `variables` · `ref_debug` ·
`bridging` · `all_fns` · `type_timeline:<var>` (every write to a variable's type, naming the
SOURCE LINE; `LOFT_TIMELINE_BT=1` adds the stack). It traces deps being REMOVED
(`make_independent`) as well as added — without that half it showed a borrow being created
and never promoted to an owner, so a container-destroying free had to be hunted by reading
every strip site by hand (@PLN130 F1). DbRef dumps tune via `LOFT_DUMP_DEPTH` (2),
`LOFT_DUMP_ELEMENTS` (8). Separately, **`LOFT_VAR_TABLE=<fn>`** prints that function's
variable table with every type dep resolved to `name(index)` plus its ownership flags —
reach for it when a borrow points somewhere impossible, because the IR dump names variables
without numbering them and a code/table desync then reads as one consistent story (loft#666).
For a `--native` wrong-type fault — a sized `f#read` answering null, a keyed lookup naming a
type the program never used — reach for **`LOFT_STRICT_SCHEMA_IDS=1`**: generated `init()`
REPLAYS the parse-time type order, so one type created a position early renames every id
after it, and this makes that drift fatal instead of a report (loft#739, NATIVE.md §
Architecture). `LOFT_TRACE_MINT=1` is its companion — it names the lookup that minted the
extra type. Full API: [TESTING.md § LogConfig](doc/claude/TESTING.md), [DEBUG.md](doc/claude/DEBUG.md).

### Two diagnostic tiers

`warning` GATES a library's CI (`LOFT_DENY_WARNINGS=1`);
`advice` never does and has no deny switch. The rule: **a diagnostic gates if and only if
ignoring it can produce a wrong result** — lost writes, char/byte index confusion,
null-into-non-null gate; deprecations, perf notes and spellings advise. The split exists
because one tier made the compat doctrine self-contradictory: `not null` is a deliberate
no-op kept parseable so unrepublished libs load, yet it hard-failed those libs' own CI.
Renders as `advice:`, LSP severity Hint; `@EXPECT_WARNING` and `Test::advice()` match it.

**And a second, orthogonal axis — REACH (loft#1260): a diagnostic reaches only whoever can
act on its cure.** The tier decides whether it gates; reach decides who sees it. Enforced ONCE,
in `Diagnostics::add_at_coded`, so **a lint site must not add its own ownership test**; the
scope is the PROJECT (nearest `loft.toml` above the entry), never the entry FILE, and errors
are never dropped. [DIAGNOSTICS.md § Who a diagnostic is addressed to](doc/claude/DIAGNOSTICS.md).

### The switches, by family

- **Diagnostics** — `LOFT_ERRORS=pretty|compact` and the opt-out for every lint
  (`LOFT_NO_DEAD_STORES`, `LOFT_NO_DOUBLE_MOVE`, `LOFT_NO_OMITTED_FIELD`, …), each with what the
  lint catches and where it is quiet: [DIAGNOSTICS.md § Switching a diagnostic off](doc/claude/DIAGNOSTICS.md).
- **Profiling** — `LOFT_PROFILE`, `LOFT_ALLOC_SITES`, `LOFT_ALLOC_PATHS`, `LOFT_PROFILE_EVERY`
  (`--interpret`) and `LOFT_NATIVE_CHECKPOINTS` (native, wasm), and what each cannot see:
  [PROFILING.md](doc/claude/PROFILING.md). Prefer `make profile`, which picks the instrument.
- **`--native` rewrites** — every generation-time rewrite (vector-header hoist, element bases,
  record addresses, push windows, guarded arithmetic, value records, …) has a `LOFT_NO_*`
  switch that is the FIRST bisect step for a native-only wrong answer, `LOFT_HOIST_VERIFY=1` as
  its falsifier and, where one exists, a `LOFT_TRACE_*` naming each decline: [NATIVE_SWITCHES.md](doc/claude/NATIVE_SWITCHES.md).
  Run `scripts/emission_audit.py <emitted.rs>` on any emission that looks wrong.
- **Both-backend optimisations** — parse-time and scope-pass lowering (record placement, call
  buffers, `??` chains, store confinement, …) and runtime store policies (keyed collections,
  clears, prefill, the free tree): [BOTH_BACKEND_SWITCHES.md](doc/claude/BOTH_BACKEND_SWITCHES.md).
  Both backends share these, so agreement between them is no evidence; the falsifiers are
  `LOFT_STRICT_STORES=1`, `LOFT_POISON=1` and `LOFT_POISON_CLAIM=1`.
