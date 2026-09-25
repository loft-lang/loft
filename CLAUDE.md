
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
the bug-filing policy above). It never blocks.

---

## Documentation index

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

**Libraries / registry / packages:** `LIBRARIES.md` (generated on demand — `make libcatalogue`, not
committed) state of the loft distribution — core (version + binary sha) + libraries + applications built with loft ·
[LIBRARY_BRANCHES.md](doc/claude/LIBRARY_BRANCHES.md) in-flight (unmerged) lib branches ·
[PACKAGES.md](doc/claude/PACKAGES.md) format/targets · [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) registry MVP ·
[LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) / [LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md) ·
[REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) / [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) /
[REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) · [API_SURFACE.md](doc/claude/API_SURFACE.md) ·
publishing is the **loft-ship skill** (touch-gated signing). REPL: [REPL.md](doc/claude/REPL.md).

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
`design-protocol` (rigor) · `doc-quality` · `draw` · `loft-plan-workflow`.

## `LOFT_LOG` quick reference

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

**Two diagnostic tiers.** `warning` GATES a library's CI (`LOFT_DENY_WARNINGS=1`);
`advice` never does and has no deny switch. The rule: **a diagnostic gates if and only if
ignoring it can produce a wrong result** — lost writes, char/byte index confusion,
null-into-non-null gate; deprecations, perf notes and spellings advise. The split exists
because one tier made the compat doctrine self-contradictory: `not null` is a deliberate
no-op kept parseable so unrepublished libs load, yet it hard-failed those libs' own CI.
Renders as `advice:`, LSP severity Hint; `@EXPECT_WARNING` and `Test::advice()` match it.

**And a second, orthogonal axis — REACH (loft#1260): a diagnostic reaches only whoever can
act on its cure.** The tier decides whether it gates; reach decides who sees it. Every
warning and advice names a cure that is an edit at the site it points at, so one pointing
into a dependency is noise that reads as the reader's defect — the Parser chapter printed 11
notes about two libraries the reader did not write. Enforced ONCE, in
`Diagnostics::add_at_coded`, so a new lint is covered by existing and **a lint site must not
add its own ownership test**. The scope is the PROJECT (nearest `loft.toml` above the entry),
never the entry FILE: `source_is_owned` is `source == MAIN_SOURCE`, which under `loft test`
is `tests/*.loft` and would silence `src/*.loft` in the one run that exists to catch it —
measured on `linked-group-apart`, which had exactly that hole. With no manifest the scope
is the entry's DIRECTORY (the modules beside `main.loft` are the author's; a vendored `lib/`
below is not). Errors are never dropped.
[DIAGNOSTICS.md § Who a diagnostic is addressed to](doc/claude/DIAGNOSTICS.md).

**Error rendering (@PLN28):** `LOFT_ERRORS=pretty|compact` (or `--errors=…`) picks the
user renderer — `pretty` (default: `file:line:col` + source line + caret) vs `compact`
(single line; the test harness pins this). Diagnostic toggles (default-on opt-outs, except
the last two which are opt-in): `LOFT_NO_WARN_RUNTIME` (undefended-fault-site warning) ·
`LOFT_NO_HINT_NOT_NULL` (`not null` field hint) · `LOFT_FORMAT_BARE_NULL` (drop the `(reason)`
suffix on `null`) · `LOFT_NO_DEAD_STORES` (@PLN107 dead-store lint: a copy mutated but never
read, e.g. `d = self.data; d[i]=x` where the bind COPIES so the write is lost — a `len(d)`
BOUND GUARD does not count as reading it, since a length cannot witness an element write;
that hole made the lint silent on `if i < len(d) { d[i]=x }`, the exact shape the `v[i]`
may-be-null warning asks for, and the published `graphics` canvas shipped every drawing
primitive as a no-op through it) ·
`LOFT_NO_DOUBLE_MOVE` (@PLN139 stage G: one droppable handed to TWO owners — `s1 = S{h:c};
s2 = S{h:c}` — where each owner's death releases what it owns, so the resource is released
twice. Counts hand-offs per source with the SAME predicate that suppresses the source's own
drop, so lint and mechanism cannot drift. `warning` because ignoring it produces a wrong
result; therefore an UNDER-approximation — silent across opposite `if` arms, a reassignment
between the hand-offs, and a terminator, and blind to the iteration count of a loop) ·
`LOFT_NO_LOST_TEMP_WRITE` (loft#894, the second `lost-write` shape: a call writing through a
by-value struct parameter GIVEN a value returned by another call — `hurt(first(s), 10.0)`
writes a copy that is freed at the end of the statement, while `hurt(s.es[0] ?? E{}, 10.0)`
lands, and nothing at the call site said which. Needs BOTH facts to meet: the callee writes
through that parameter (read off its own body) and the argument copies a place the caller
can still REACH (read off the return type's deps) — the second is what keeps
`hurt(fresh(), …)` and the write-then-return builder idiom quiet) ·
`LOFT_NO_STEER` (@PLN102 arc C recommended-idiom channel: a call FROM OWNED source to a
`#superseded "Y"` symbol warns *"`X` is superseded — use `Y`"* + a CI fold-lint; inert until a
symbol is marked — see [COMPATIBILITY.md § Folding](doc/claude/COMPATIBILITY.md)) ·
`LOFT_NO_PARAM_COUNT` (≥8 REQUIRED parameters — defaulted and compiler-hidden ones
excluded; separate from complexity because a caller's burden and a reader's burden have
different fixes: a struct vs an extracted function) · `LOFT_NO_API_ADVICE` (the two API-design advices, `@FR-R-Escape`: `api-copies-collection`,
a `pub` function answering a copy of its parameter's collection, and `api-redoes-per-field`,
a `pub` function building a heap-owning intermediate from its parameter and answering ONE
value of it — per-call work the CONTRACT forces on every caller and no rewrite can share
across the calls a consumer makes; measured 11–55× on the pluginabi accessors.  `advice`,
reaching the library's author only; a rewrite-reachable shape such as an edit answered as
a fresh record is NOT flagged — a construction merely outside what the rewrites reach is
not a design fault.  Census 2026-09-25: 13 sites in 42 library packages, every one an
accessor that decodes or derives per call) ·
`LOFT_NO_DEFAULT_HINT` (≥2 trailing
booleans with no default — advertises default parameters, which are under-used and free to
adopt: adding a default is additive, so existing callers keep working) ·
`LOFT_NO_NARROW_FALLBACK` (C127 `narrow-fallback` ADVICE: a compound step into a
DECLARED narrow range whose result does not fit takes the type's DEFAULT — `x: u8 = 250;
x += 10` answers `0`, `h: integer limit(1000, 1100) = 1050; h += 5000` answers `1000` —
and nothing at the site says so.  The WRITTEN-OUT `x = x + 10` is refused at compile
time, so the compound step is the one arithmetic shape where the language cannot ask the
author what an unfitting result should become.  The cure is the pair `@FR-E-Uncomp-Seen`
already fuses (`if !x { … }` as the very next statement) or choosing the value with
`x = (x + n) ?? d`.  `advice`, not `warning`: the default IS what the language promises
for an unfitting value, so ignoring it cannot produce a result the language did not
promise.  Quiet where the target keeps a code for its own failure — a `τ?`, an `i32`, a
plain `integer` — where the check IS fused, and where the author chose the value; census
2026-09-23: 16 of 1 653 corpus files, every one of them a test about narrow overflow) ·
`LOFT_NO_OMITTED_FIELD` (loft#914 `omitted-field-zero` ADVICE: a struct literal that names
SOME fields and leaves another out — the omitted one takes its type's zero and nothing in the
declaration chose it, which bites where zero is a meaningful value of the field's domain
(dryopea's palette index wanted `-1`; `0` is the entry that erases). Advertises the DECLARED
FIELD DEFAULT (`palette_pick: integer = -1`), the cure that already exists and was simply
undiscoverable. `advice`, not `warning`: the zero is documented behaviour, so ignoring it
cannot produce a result the language did not promise. Quiet on a field with a declared
default, on a NULLABLE field (absence is a value it holds), and on a bare `S {}` — that asks
for the whole default record; the ambiguity is only in the PARTIAL literal) ·
`LOFT_NO_VARIANT_OVERWRITTEN` (loft#1397 `variant-overwritten-binding` WARNING: a
`match`/`is` PAYLOAD binding still read after the subject's PLACE is given a DIFFERENT
variant — `match w.st { Holder{inner} => { w.st = Empty{z: 0}; inner.a }, … }` reads
`Empty`'s `z` at `Holder`'s offset. `(B-Disturb)` makes overwriting a place NOT a
disturbance, so the value is what the rules give and both backends agree; what was
missing is loft#980's `variant-field-unchecked`, whose exemption for a per-arm binding
assumes the variant cannot change under it. Keyed on the ARM's own tag test, so it cannot
drift from the parser's numbering. Quiet on a SAME-variant overwrite (the value is
right), on an unrelated field, and on a LOCAL subject — that is a reassignment `(B-View)`
already materialises) ·
`LOFT_NO_LINKED_GROUP` (loft#926 `linked-group-double-fill` ADVICE: one struct literal
gives RECORDS to two members of a linked collection group — two keyed collections over one
element type are two routes to a SINGLE record set, so both end up holding everything and
nothing at the literal says so. Quiet on a member written `[]`, which is how every group is
constructed, and quiet when only one member is filled — those are the deliberate uses.
`advice`, not `warning`: the result IS what the language documents, so ignoring it cannot
produce a result the language did not promise; what is wrong is the author's model) ·
`LOFT_NO_GROUP_APART` (`linked-group-apart` ADVICE, the DECLARATION-side half of the same
question: a linked group whose members are declared APART, with an unrelated field between
them — `{ entities: vector<E>, tick: integer, spawn_index: hash<E[id]> }`. The declaration is
the only place the pairing is decidable; by the time a `len` reads 0 a group that did not
form looks exactly like an empty one. Adjacency is the signal rather than the group itself,
because the idiom is written TOGETHER while a group nobody intended is two fields added at
different times for different reasons. Quiet on adjacent members, on a pair with no keyed
member, and on a LIBRARY's struct, which a consumer cannot rearrange) ·
`LOFT_NO_LIB_OUTRANKED` (loft#1352 `lib-flag-outranked` ADVICE: `use <id>` resolved
somewhere other than a `--lib` directory that also provides it — resolution is first-wins
and a project-local `lib/`, a declared dependency and the script's own directory are
searched BEFORE the flag, so a `--lib` override run from a tree with a `lib/` measures the
original in silence; reports the precedence rather than moving it, once per id, quiet when
the winner lies inside the flag's directory) ·
`LOFT_NO_UNDECLARED_DEP` (loft#968 `undeclared-dependency` ADVICE: `use <pkg>` resolved a
REGISTRY package the project's `loft.toml` never declares — so nothing distinguishes "we
depend on this" from "this happens to be installed on the box that built it", the negative
gate *drop the dependency and the tests must stop compiling* cannot be written, and an
undeclared package is not pinned either (measured: it resolves to the NEWEST installed).
The resolution stays — auto-load is deliberate; the silence was the defect. Quiet for a
bare script with no manifest above it, and for a package parsed out of the registry cache,
whose manifest is someone else's to fix. `advice`, not `warning`: the program computes
what the language promises on this box, and what is wrong is that the manifest does not
describe the project) ·
`LOFT_NO_SHADOWED_BY_METHOD` (loft#940 `shadowed-by-method` WARNING: a LIBRARY's free
`fn f(x: τ, …)` that no bare call can reach, because `find_fn` resolves the method
spelling `t_<τ>_f` before the free `n_f` and reaches it through the stdlib row from
every source — so the shadow covers the declaring file and the library's own other
modules, not just a consumer, and `pub` is not the axis. @PLN102 C97 keeps the
DEFINITION legal on purpose (module-scoped, so the stdlib can grow without breaking a
shipped library) and `lib::f` still reaches it; the silence was the defect. `warning`,
not advice: the published `regex::find(pattern, input)` has the stdlib's exact arity and
argument types, so a bare `find(p, i)` type-checks and answers the wrong thing. Quiet
where the same name is a method on ANOTHER receiver type — arg-type dispatch keeps that
one reachable — and quiet for a collision with a stdlib FREE function, which the import
outranks) ·
`LOFT_NO_VARIANT_FIELD` (loft#980 `variant-field-unchecked` WARNING: `c.field` on a
struct-enum names a field only SOME variants declare. The access resolves at COMPILE
time to the first variant that has it, and the layout gives a shared name+type one
slot — so the read is right for the variants declaring it and reads ANOTHER variant's
bytes for the rest, with the tag never consulted: `a.n` on an `Anon` answered
`Anon.k`'s value, and `a.label = "x"` wrote into a record whose tag still said `Anon`,
after which `match` still reported `Anon`. Direct payload access STAYS — C89 decided
permanently that enum payloads are named fields you read straight, with matching for
DISPATCH and never for extraction; the silence was the defect. `warning`, not advice:
the value read is another variant's, typed as this one's. Quiet when EVERY variant
declares the field (one shared slot — measured correct even where the variants'
preceding fields differ in width), quiet for `match`/`is` bindings, which are per-arm
and are the cure it names, and quiet for a synthetic `__nullable<S>`, whose payload
access is @PLN25's null model rather than a variant question) ·
`LOFT_NO_COMPLEXITY` (function-complexity ADVICE: cognitive complexity ≥ 40 — a
construct costs `1 + nesting`, so 8 sequential `if`s cost 8, 3 nested cost 6, a flat
`match` costs 1 whatever its arm count; counted at PARSE time because the IR is
post-desugar and would charge `??` and `for` as branches the author never wrote;
names the deepest-nesting line, since that is where a split pays) ·
`LOFT_NO_STRICT_INDEX_TEXT` (@PLN110 3a text strict-index units lint: warns on
`for i in 0..len(s) { s[i] }` AND `{ s.byte_at(i) }`, incl. via a local (`n = len(s); 0..n`) —
`len(text)` is a CHARACTER count but both reads are byte-indexed, so the loop truncates
multi-byte text silently (the `cbor` encoder shipped this); advisory, use `for c in s` or
`0..size(s)`) ·
`LOFT_LINT_STRICT_INDEX` (**opt-in**, @PLN102 case-D audit: warns where a for-loop iter var
bounded by `len(<one vector>)` indexes a DIFFERENT vector — `for i in 0..len(v) { w[i] }` types
non-null yet reads C80-null on overrun; advisory, the type is unchanged) ·
`LOFT_DEV_SOFT_HALT` (**opt-in**: demote dev raises to log-and-continue so one run surfaces every fault).

**Profiling (@PLN140, all opt-in, all `--interpret`):** `LOFT_PROFILE=<ops>` samples the loft
call stack — hot FUNCTION, hot LINE, hot PATH (default one sample per 1024 ops; the op counter
picks *when*, a wall clock says *how much*, and the period is JITTERED because a fixed one
samples a single phase of a periodic program and reports it as the whole) ·
`LOFT_ALLOC_SITES=1` ranks live store BYTES by the loft line that allocated them, captured at
the run's PEAK rather than at exit · `LOFT_ALLOC_PATHS=<ops>` adds the call paths that reached
each allocation. **A program whose only exit is a signal — a server — reports through
`LOFT_PROFILE_EVERY=<seconds>` (a report while running, surviving a hard kill), `kill -USR1`
(dump and keep going, which profiles a WINDOW) or `kill -TERM`/Ctrl-C (dump, then leave):
the report used to render at process exit, so the run you most want a profile of was the
one that could not produce one (loft#1089). Handlers are installed only when the profiler
is armed.** `LOFT_PROFILE` / `LOFT_ALLOC_PATHS` also cover **test runs** (`loft test`,
`--tests`), merged into ONE report keyed by resolved `function` + `file:line` — each test
compiles its own bytecode, so positions cannot be merged, only labels (loft#860).
`LOFT_ALLOC_SITES` is program-only and says so under a suite instead of going quiet.
**A NATIVE run is not sampled** — and the default backend IS native, so a bare
`LOFT_PROFILE=1 loft p.loft` announces that rather than exiting empty (loft#865).
**A `use`d library is a cdylib the sampler cannot enter**: its functions cannot appear
and their time lands on the CALLING line, so a library doing the work reads as a hot
caller — one probe inverted from `100 % app_bit` to `99.5 % lib_grind` under
`LOFT_NO_NATIVE_LIBS=1`. The report says so whenever a library was called.
Prefer `make profile`, which picks the instrument. Off costs nothing (the
sampler rides the existing per-op debug branch); armed costs +7–11 %. PERFORMANCE.md § Profiling.

**`LOFT_NATIVE_CHECKPOINTS=count|time[:filter]`** — the SECOND profiler, for where the
sampler cannot reach (a stripped binary, no `perf`, **wasm**). The generator writes a probe
at the one call/op chokepoint, so every OPERATOR is counted at its own loft `file:line`,
with a by-function rollup that is exclusive by construction (a user CALL is deliberately
not a site — timing it would mix inclusive and exclusive rows). `count` needs no clock and
so behaves identically on native, wasip2 and in a browser; `time` adds the cycle counter
where one exists and says so where it does not. **The COUNTS are the trustworthy column; the tick
share is a hint.** Counts validate exactly against the pure-Rust reference (whole-number
operators per call: `chan` 3.00, `color_g` 2.00, `ramp` 9.00). Ticks do not: the counter
advances only every ~41.7 ns against a ~1-3 ns operator, so a total is a sum of dithered
samples, AND the per-operator probe inflates operator-dense functions — measured against
the instrumented reference, the two biggest `lock_curved` rows disagree ~2× in opposite
directions. Never quote a tick share as "where the time goes". Costs 3.0×
(`count`) / 5.4× (`time`), and `:filter` narrows it to one function or module. It changes
what rustc may inline ACROSS a probe, so it tells you WHICH code runs and roughly where
time concentrates — confirm a ratio with `compare.py` or `profile.sh`. It is not the normal
route: `scripts/profile.sh` is, and it perturbs nothing. PERFORMANCE.md §
`LOFT_NATIVE_CHECKPOINTS`.

**Vector-header hoist (loft#885, `--native` only, both switches read at GENERATION time):**
a loop the emitter proves writes NO store derives each vector's `(store_nr, record, length)`
once before the loop, so an element read is a bounds test plus address arithmetic (~2×).
The gate (`src/generation/hoist.rs`) is an ALLOW-list on purpose — an op missing from it
costs the optimisation, never correctness, which is the opposite of the five drifted
mutation deny-lists in PERFORMANCE.md § Design: P8. **`LOFT_HOIST_VERIFY=1`** emits the
checking form of every hoisted read (re-derives the header, panics on a stale one) — run the
suite under it after touching the gate; **`LOFT_NO_VECTOR_HOIST=1`** emits the pre-885 form,
which is the before-half of an A/B on one binary and the first bisect step for a
native-only wrong answer in a vector loop; **`LOFT_NO_ELEM_FUSE=1`** keeps the hoisted header
but leaves the scalar element read UNFUSED, one bisect step finer, and is the middle rung that
showed stage 2 is worth ~3.2× on top of stage 1 (projected ~1.4×) — more than the hoist itself,
because the second store resolution it removes costs more than the arithmetic it saves.
**`LOFT_NO_SCALAR_HOIST=1`** (@PLN157 P4c) reads every record scalar field per iteration
again — a loop that cannot write `lay.x0` otherwise reads it ONCE into a local, keyed by
(record type, offset) over the body's typed write set — and is the bisect step for a wrong
scalar in a loop over a record; `LOFT_HOIST_VERIFY=1` re-reads each hoisted scalar and
panics on a stale one.  **`LOFT_NO_VIEW_HOIST=1`** (@PLN157 § V-n) makes a `&`-bound vector
view (`d = &cv.data`) derive no header at its binding — with it on, the rest of the block
reads `d[i]` through one header derived once — and is the bisect step for a wrong element
read through a view outside a loop.  **`LOFT_NO_WRAPPER_INLINE=1`** (@PLN157 § V-o) emits a
call to a stdlib one-op wrapper (`len(v)`, `sqrt(x)`) as the CALL again instead of as its
op — the bisect step for a wrong length or libm value on native.  **`LOFT_NO_CALLEE_INPUTS=1`**
(@PLN157 § V-p, § V-ac) emits no callee TWIN — with it on, a callee that reads a record
parameter's scalar fields, views or indexes its vector fields, or indexes a plain VECTOR
parameter gets a `<fn>__inv` twin taking those as extra parameters, and a loop that hoisted
them for the argument — a variable, or for a header any pure path such as `br.img` — calls
the twin; a callee answering a scalar record through its return buffer qualifies too — and
is the bisect step for a wrong value read through a record or vector parameter inside a
callee a hoisting loop calls.
**`LOFT_NO_PUSH_HOIST=1`** (@PLN157 § V-q) makes a loop that PUSHES to a vector (`v += [x]`,
a comprehension) hoist nothing — with it on, the pushed path keeps a PUSH header carrying the
record's capacity, a push that fits is one store and a length bump, and every read of the path
serves from it — and is the bisect step for a wrong element or length out of an appending loop.
**`LOFT_NO_MINT_HOIST=1`** (@PLN157 § V-s) makes a loop that appends a RECORD element
(`v += [Pt{…}]`, `v += [pt(…)]`) hoist nothing — with it on, the mint group is admitted as a
mover and the loop's invariant record scalars are read once before it — and is the bisect step
for a wrong scalar read out of a record-appending loop.
**`LOFT_NO_RECORD_PUSH=1`** (@PLN157 § V-t) makes an admitted record append keep its
mint-group templates — with it on, a no-heap struct element is built IN the push header's
next slot (no `record_new` dispatch, no default prefill: the group's writes fill every field
explicitly) and the finish is the length bump — and is the bisect step for a wrong element,
default value or length out of a record-appending loop.
**`LOFT_NO_FIELD_MINT=1`** (`@FR-R-Mint`'s field clause, default-ON, generation time,
`--native` only) keeps a record append to a vector FIELD (`m.verts += [Vertex { … }]`) on
its templates — with it off, such an append in a loop holds the push header a bare-variable
append holds, keyed by the path a read of the field already has (`mesh_emit` −73 %, 18.3× →
4.9× of Rust); a linked group's member, a keyed field, a variant's field and a nullable
element's payload keep the general append — and is the first bisect step for a wrong, missing
or extra element out of a loop that appends records to a record's vector field;
`LOFT_HOIST_VERIFY=1` re-derives the header at the slot and the finish.
**`LOFT_NO_GROUP_PUSH=1`** (`@FR-R-GroupPush`, default-ON, generation time, `--native` only)
makes a record append OUTSIDE any held header keep its templates again — with it off, a
literal group `v += [pt(a, b), pt(c, d)]` that no enclosing loop holds a header for (a group
at function level, in a loop that declined, or on a vector declared per pass) binds a push
header of its own right after its reservation and emits its mints and finishes through it
(`fronds` built its 1 296 points per call that way: 101.7 → 69.6 µs with the two clauses
below, 2.71× → 1.86× of Rust) — and is the first bisect step for a wrong element out of a
literal record append on native; `LOFT_HOIST_VERIFY=1` re-derives the header at the slot and
the finish.  **`LOFT_NO_HEAP_RECORD_PUSH=1`** (`@FR-R-PushRec`'s heap clause) keeps an element
that OWNS heap (a `Frond { fpts, fwid }`) on its templates — with it off, such an element goes
through the header too, its slot ZEROED at the mint, which is the whole of what the prefill did
for its handles — and is the bisect step for a wrong or stale handle in an appended record
whose fields own heap; `LOFT_POISON_CLAIM=1` is the falsifier (the plain run's zero-on-claim
hides a missing zero).  **`LOFT_NO_REBOUND_MOVER=1`** (`@FR-R-Mint`'s rebound clause) makes a
loop whose body REBINDS a pushed or minted vector decline every hoist again — with it off, a
mover rebound to a § V-al loop buffer's or a § V-z element slot's projection simply takes no
holder, the loop keeps its other headers and scalars, and the buffer's own mint, a loop
record's mint and the elided element-first copy are admitted as statements that move nothing
— by the header admission AND by the scalar write-set walk (a loop record's mint evicts the
scalars of its own type) — and is the bisect step for a wrong element or scalar read in a
loop that declares a vector or a record per pass.
**`LOFT_TRACE_HOIST_DECLINE=1`** names the FIRST statement that declines a loop's hoist, and
the node that left its scalar write set untyped — reach for it when a loop that should hoist
does not, before reading the admission.
**`LOFT_NO_MOVE_APPEND=1`** (@PLN157 § V-j) makes `for f in call(…) { v += [f] }` keep the
deep copy — with it on, the call's buffer is PLACED as a record in `v`'s own store, the
append relocates the element's bytes (heap handles included — they never change store) and
zeroes the source, and the buffer's free is record-level — and is the bisect step for a
wrong element, a leak or a double free out of a loop that appends a dying temporary's
elements.  `LOFT_TRACE_MOVE=1` names the gate that declined a pairing.
**`LOFT_NO_LAZY_SPLIT=1`** (`@FR-R-LazySplit`, default-ON, generation time, `--native`
only) makes `for p in t.split(c)` build and walk its `vector<text>` again — with it off, a
loop over the standard library's `split` with a CONSTANT separator takes each piece
straight from the text (a parameter nothing writes is borrowed; any other source is
iterated as a copy taken where the call stood, so a write in the body cannot reach it),
the vector and its per-piece records are never built, and the call's hidden buffer is
never minted (the drawing bench's `parse` row −9 %: its outer loop is
`for raw in src.split('\n')`) — and is the first bisect step for a wrong, missing or
extra piece out of a loop over a `split` on native.  A variable separator, a `rev`, a
split bound to a name first and a generator keep the vector.  `LOFT_TRACE_LAZY_SPLIT=1`
names each loop admitted and each declined with its reason.
**`LOFT_NO_TEXT_BORROW=1`** (`@FR-R-TextBorrow`, default-ON, generation time, `--native`
only) makes the loop variable of `for p in vector<text>` — and of a lazily split text —
copy its element into a `String` again — with it off, `p` is bound as the `&str` the
element read answers and read bare, where the body reads it only as a text VALUE (an
operand at a `text` position, the source of a bind into another slot), never writes,
links or captures it, and (for a vector) writes no store; a text VALUE op — scalar and
text operands, a scalar or text result, or a write to a text VARIABLE — is no store write
to that condition, and `OpGetText` is a reader, so the walk holds its header, length and
base like any other (the stdlib `join` 5.40× → 1.96× of Rust, `split_walk` 1.80× →
1.01×, `parse_num` 1.46× → 0.98×) — and is the first bisect step for a wrong, stale or
crashing text read through such a loop variable on native.  `LOFT_HOIST_VERIFY=1`
re-reads the element at the walk's release and panics when the borrow no longer names it
(address first, so a dangling borrow is never read); `LOFT_TRACE_TEXT_BORROW=1` names each
walk admitted and each declined with its reason.  A `&p` link, `p += …`, `p` handed to a
`&text` parameter, tupled or returned, a store written in the body and a generator keep
the copy.
**`LOFT_NO_CHAR_WALK=1`** (`@FR-R-CharWalk`, default-ON, generation time, `--native` only)
makes `for c in text` take every character through the step as written again — with it
off, an ASCII byte other than NUL is one move (the byte is `c`, `c#next` steps by one;
every other byte, a NUL — whose read notes a fault — and the end take the written step in
the `else` arm), and where nothing in the loop writes the text its null test — a content
compare of the whole text LLVM does not hoist — is asked ONCE before the loop
(`char_walk` 4.62× → 2.48× of Rust; the stdlib `split`, built on such a walk, 12.5× →
7.8×) — and is the first bisect step for a wrong character, a wrong `c#index` or a missed
fault out of a character walk on native.  `LOFT_HOIST_VERIFY=1` runs the written step
beside the fast arm and panics when they disagree, and asserts the hoisted null test
inside the loop; `LOFT_TRACE_CHAR_WALK=1` names each walk and whether its null test moved.
A call or literal source keeps the written step (the lowering evaluates such a source per
iteration).
**`LOFT_NO_VECTOR_BASE=1`** (@PLN157 § V-ak, `@FR-R-Base`, default-ON) makes a
growth-free loop's fused element reads and writes resolve the store per element again —
with it off, a loop that grows no store (no push and no mint, whether or not the mint
emits through a push header — since 2026-09-21: one left on its templates grows its store
all the same, and a base bound beside it answered `null` on native; a null-discharge
buffer's mint is a fresh store and does not count, since 2026-09-18)
binds the address of each hoisted vector's element 0 beside its header and every read
or write is one bounds test and one load or store through it (the resample probe −14 %)
— and is the first bisect step for a wrong element read or write inside a growth-free
loop; `LOFT_HOIST_VERIFY=1` re-derives every base at every use and panics when a store
grew under one, and `LOFT_TRACE_BASE=1` prints each loop's growth-free verdict.
`LOFT_NO_NN_FAST=1` (P3c) also restores the nullable-aware counter step and the guarded
literal division that § V-aj (`@FR-R-Counter`, `@FR-R-LitDiv`) replaced.
**`LOFT_NO_TWIN_BASE=1`** (`@FR-R-Base`'s twin clause, default-ON, generation time) makes
a callee twin (`__inv`) take its header inputs alone again, every element read or write
inside it resolving the store — with it off, the twin takes each header's element BASE
beside it (`__ib_k`; the caller's held `__vb_N`, or one derived from the held header at the
call), a view of the path inside the twin shares it, and the fused reads and writes are one
bounds test and one load or store (`composite`'s pixel accessors) — and is the first bisect
step for a wrong element read or write inside a function a hoisting loop calls;
`LOFT_HOIST_VERIFY=1` re-derives the base at every use.
**`LOFT_NO_RECORD_PTR=1`** (`@FR-R-RecPtr`, default-ON, generation time, `--native` only)
makes every field read and in-place field write of a record VIEW resolve the store again —
with it off, a plain-record local bound by a statement (`e = tbl[i]?`, `s = o.inner`)
carries the address of its record for the rest of its block (`__pa_N`), every scalar
field read is one load and every in-place field write one store through it, and a twin the
view is handed to takes its scalar inputs read through the address at the call
(`wide_line`'s crossing loop −35 %) — and is the first bisect step for a wrong field read
through a record view, or a wrong argument inside a callee handed one, on native.  Admitted
where the block's remainder grows no store, frees no record before a later use, and never
rebinds the view; a nullable view — `e = v[i]` without `?`, a `for e in v` loop variable —
is admitted as its record, the null address answering the sentinel.  `LOFT_HOIST_VERIFY=1` re-derives the
address and re-reads the store at every use; `LOFT_TRACE_RECPTR=1` names each view bound
and each declined with its reason.  `LOFT_NO_VECTOR_BASE=1` switches it off too (one rule).
**`LOFT_NO_BASE_RECPTR=1`** (`@FR-R-RecPtr`'s base clause, default-ON, generation time,
`--native` only) makes the loop variable of `for e in v` resolve the store for its address
again — with it off, the address is the loop's held element base plus index times size
under the in-range test, no `DbRef` consulted, and the iteration's `#index` (never the
sentinel, `@FR-R-Counter`'s iteration clause, `LOFT_NO_NN_FAST`) steps through the non-null
add (`record_walk` −49 %, 2.98× → 1.50× of Rust; `tuple_kernel` −20 %; `entity_tick`
−30 %) — and is the first bisect step for a wrong field read through a `for e in v` loop
variable on native; `LOFT_HOIST_VERIFY=1` compares the address with a fresh `rec_ptr` at
every use.  An address derived from the element's `DbRef` instead of the index measured
+46 % SLOWER and is not what ships.
**`LOFT_NO_NESTED_FIELD=1`** (`@FR-R-RecPtr`'s path clause, default-ON, generation time,
`--native` only) makes a scalar field reached through INLINE sub-records (`v.pos.x`) rebuild
its `DbRef` and resolve the store again — with it off, such a field of a record view is read
and written through the view's address at the summed offset, and a view whose only accesses
are nested binds an address at all (`mesh_aabb` −47 %, 9.2× → 4.7× of Rust) — and is the
first bisect step for a wrong value read or written through a nested field path on native;
`LOFT_HOIST_VERIFY=1` compares every such read with the path walked the unrewritten way.
Emitter-local on purpose: folded in the parser, the read would become an `(R-Scalar)`
candidate typed by the PARENT record, which a write through a sub-record view never evicts.
**`LOFT_NO_MINT_WINDOW=1`** (`@FR-R-RecPtr`'s mint clause, default-ON, generation time,
`--native` only) makes every field write of an appended record resolve the store again —
with it off, a minted plain-record element holds its slot's address from the mint up to its
own finish, and its `integer` / `float` / `single` fields are one store each through it
(`record_append` −45 %, 3.55× → 1.95× of Rust; `mesh_emit` −43 %, 2.83×); a window that
holds a text set, a nested mint, a builder that appends or a delivery copy declines — and is
the first bisect step for a wrong or lost field in an appended record on native;
`LOFT_HOIST_VERIFY=1` compares the address with a fresh derivation at every write.  Its
cells could not fail until one reallocated the store INSIDE a window (`m4b`): small cells
never move the memory a stale address would miss.
**`LOFT_NO_COPY_IN_PLACE=1`** (`@FR-R-InPlace`'s copy clause, default-ON, generation time,
`--native` only) makes a whole-record copy a store writer again, so a loop holding one
hoists nothing — with it off, a flag-free `OpCopyRecord` over a record type that owns no
heap is an in-place write at a larger width: the write-back idiom `e = v[i]?; …; v[i] = e`
(a copy of a view onto the place it views, which the runtime makes a no-op) keeps its
loop's header, base and the address of `e` (`entity_tick` −23 %, 3.19× → 2.45× of Rust) —
and is the first bisect step for a wrong value in a loop that assigns a whole record to an
element or a field; `LOFT_HOIST_VERIFY=1` is the falsifier (the copy writes its type WHOLE
for the scalar hoist).  A heap-owning type and a call's freed result keep the store-writer
verdict.  The copy is not elided: on the absent-element path it is an out-of-range store
with a fault note of its own.
**`LOFT_NO_ENUM_RECORD=1`** (`@FR-R-PushRec`'s and `@FR-R-RecPtr`'s enum clauses, default-ON,
generation time, `--native` only) makes a USER struct-enum value no record to the hoist
family again — with it off, a `vector<Edit>` appends through a push header (the slot zeroed,
the literal writing its own tag), a minted element of any record type holds its window
address, a struct-enum view holds its address and its TAG is one byte read through it, and
`for e in edits` is seen through the `OpGetField` its element is bound behind (`enum_match`
−64 %, 12.4× → 4.5× of Rust) — and is the first bisect step for a wrong variant, a wrong
payload field or a lost element out of a vector of struct-enum values on native.  The
exclusion of enum payloads belongs to `(R-Scalar)`'s (type, offset) key and stays there; the
synthetic `__nullable<S>` stays out of all of it.
**`LOFT_NO_LOOP_BUFFER_REUSE=1`** (@PLN157 § V-al, `@FR-R-LoopBuffer`, default-ON,
generation time) makes a vector local declared `[]` INSIDE a loop re-mint its per-site
buffer every iteration again — with it off, the buffer's store and its vector survive the
iteration, the mint after the first is a length reset that keeps the capacity, and the
literal's zero of the vector field is not emitted (the resample's two per-pixel vectors:
−10 % on the row) — and is the first bisect step for a stale or wrong element read out of
a vector declared inside a loop on native.  Only for elements that own no heap (a `text`
vector keeps the re-mint: a length reset would strand what its elements own), a buffer no
other call reaches, and a declaration outside the loop.  `LOFT_TRACE_LOOP_BUFFER=1` names
each buffer kept and each declined.
**`LOFT_NO_LOOP_RECORD=1`** (`@FR-R-LoopRecord`, default-ON, generation time, `--native`
only) makes a plain no-heap record local declared and minted by a literal INSIDE a loop
(`sub = Spec {…}` per pass) free its store at the iteration's end and take a fresh one next
pass again — with it off, the local is declared at the loop's prelude, keeps its store and
its record across passes (a complete literal re-mints nothing, a partial one re-establishes
its declared defaults) and is freed once after the loop — and is the first bisect step for a
stale field, a leak or a double free out of a record literal in a loop on native.  Declined
where the local is copied, returned, captured, appended, handed to a callee whose return
borrows it, or owns heap.  `LOFT_TRACE_LOOP_RECORD=1` names each kept local and each decline.
**`LOFT_NO_PUSH_FILL=1`** (@PLN157 § V-am, `@FR-R-PushFill`, default-ON, generation time)
makes a counted push loop grow per push again — with it off, `for i in a..b { v += [x, y] }`
reserves two elements times its trip count once before it runs, and `for _ in a..b
{ v += [c] }` with `c` invariant is ONE fill of the vector's tail with the per-element
loop as its fallback (the resample's plane prefill: −2 % on the row) — and is the first
bisect step for a wrong element or length out of a counted push loop on native.  A body
that can `break`, `return` or loop again, a push under a branch, another write to the
path, or a range end that is not a simple invariant declines the loop.
`LOFT_TRACE_PUSH_FILL=1` names each decline; `LOFT_HOIST_VERIFY=1` re-derives the push
header at the fill.  The loop is read in either spelling — the statement and the
comprehension `[for i in a..b { e }]` — and the reservation stands in front of every
guarded copy of the loop (one written after a guard lands in the arm that does not run).
**`LOFT_NO_PUSH_WINDOW=1`** (`@FR-R-PushFill`'s window clause, default-ON, generation time,
`--native` only) makes a reserved counted push loop push through its push header again, the
length written back to the record per push — with it off, a loop whose body reaches its
vector through the pushes alone (an exclusive root, never named outside the pushes, no
variable that may view it, no other store write) pushes through a held element address and
a local length, and writes the record's length once when the loop ends (`push` 48.8 → 8.0 µs,
0.47× its Rust twin; `comprehension` 9.53× → 1.70×; `grid` 8.10× → 2.81×; `f32_build`
−80 %) — and is the first bisect step for a wrong element or length out of a counted push
loop or a comprehension on native.  `LOFT_TRACE_PUSH_FILL=1` names the clause that declined
a window.  ⚠ `LOFT_HOIST_VERIFY=1` checks the window's base and its frozen header at every
push, but CANNOT see what the admission exists to prevent — a runtime reader meeting the
lagging length changes no held fact — so there the interpreter is the falsifier.  The form
is load-bearing: the window is three scalars whose address never reaches a call, because a
counter whose address escaped to the growth arm lived on the stack and cost half the gain.
Since 2026-09-22 the window also takes RECORD mint groups, at the top level or under `if`
arms (the arms are exclusive, so the trip count bounds the appends): the slot is the
window's next, the field sets and the mint address are that pointer, the finish is the
window's bump (`enum_match` 35.9 → 23.4 µs, ~3.98× → ~2.6×); a group whose literal fills
the element's own vector field, a read or a view of the vector in the body, a `break` and a
second pushed vector keep the header mints.
**`LOFT_NO_JOIN_READ=1`** (`@FR-R-Base`'s join clause, default-ON, generation time, `--native`
only) makes `v[i]?.f` run its join on every pass and read the result through the store again
— with it off, a scalar field of a `?`-discharged element (`i` a variable) in a loop that
holds the vector's header and element base is one range test and one load through the base,
the join written once in the fallback arm for every index that test refuses (`record_update`
34.9 → 12 µs, 4.62× → 1.60× of Rust, pinned) — and is the first bisect step for a wrong field out of
`v[i]?.f` inside a loop on native.  The fallback is never a constant: a negative index
addresses from the end there and an absent element answers its default RECORD's field, which
a declared field default makes non-zero — `LOFT_HOIST_VERIFY=1` cannot see a mistake in that
arm, the interpreter can.  ⚠ Built at TWO sites through one recogniser
(`hoist::fused_join_read`): the pre-eval collector lifts every `Block` argument, so an
emitter arm it does not know about never fires — and only a pin shows that, no value does.
**`LOFT_NO_INVARIANT_HOIST=1`** (@PLN157 § V-ao, `@FR-R-Invariant`, default-ON, generation
time) makes every invariant integer chain evaluate at every use again — with it off, a
chain of `+ - * neg & | ^` over literals and variables a loop neither rebinds nor lets
escape is evaluated at its FIRST use and answered from a memo after (the same value on
every path, the overflow note fired where the first evaluation stands; declared at the
innermost loop that spells it so the test peels out — declared one loop out it stayed in
every tap), the resample tap's `yy * iw + xmin` −5 % on the row — and is the first bisect
step for a wrong index or arithmetic value inside a loop on native.  `LOFT_HOIST_VERIFY=1`
re-evaluates the chain at every use and panics when the memo disagrees; that form is what
found loft#1534, a by-reference argument the non-sentinel proof's escape collector could
not see.  `LOFT_TRACE_INVARIANT=1` names each memo.
**`LOFT_NO_BOUNDED_NEST=1`** (@PLN157 `R-BoundedNest`, `@FR-R-BoundedNest`, default-ON,
generation time, `--native` only) keeps every tap nest checked — with it off, an innermost
counted loop that is one accumulate over `?`-discharged element products
(`acc += a[(yy*w + x)*4 + ch]? * k[base + x]?`, a resample's tap) runs with PLAIN operators
behind a guard evaluated once at the loop's entry: no range end or invariant is the sentinel,
every vector read has a known element bound (taken ONCE at the outermost loop whose body
leaves the vector alone, never at the nest's own prelude), and the magnitude bound of every
chain and of `|acc| + trips × bound(term)` fits, so no operation can fault and the plain
answer is the checked one; the checked loop is the `else` arm.  It is C120's admissible
successor built — the fact is established before the arithmetic runs, not assumed — and it
took the drawing lane's three resample rows from 4.9–6.1× to 2.4–2.8× their Rust reference
(`render_marks` −54 %).  **Step 2, `LOFT_NO_NEST_RAW_READS=1`** keeps the arm's bounds-tested
reads — with it off, where every read's chain names the counter at most once (affine) the guard
also proves both range ends in `[0, len)` and the arm reads RAW through the held base with no
null select (the bound already ruled out a stored null): the tap becomes the multiply-accumulate
LLVM vectorises (`render_marks` −40 % again, to ~1.7×).  Both are the first bisect steps for a
wrong accumulate or index out of such a loop on native; `LOFT_HOIST_VERIFY=1` compares every
plain operator's answer with the checked template's and every raw read with the checked read,
panicking on a disagreement; `LOFT_TRACE_NEST=1` names every admission and decline and whether
the reads are raw.
**`LOFT_NO_BOUNDED_SUM=1`** (`@FR-R-BoundedNest`'s reduction clause, default-ON, generation
time, `--native` only) makes `acc = acc + v[i]` over an integer vector pay the checked add on
every element again — with it off, a counted loop that is one such accumulate over a held
header and base sums what it can PLAIN first, a block of 1024 at a time, admitted when every
element lies in `[−2^40, 2^40)` and the running total is more than `2^50` from the i64 edge
(the nest's magnitude-bound proof, taken from the data per block in the same pass, so no
prefix in the block can overflow and the plain sum IS the checked answer), and the checked
loop resumes where the first block declines (the stdlib `sum` 11.9 → 3.7 µs, 6.45× → 2.06× of
Rust, pinned; no answer changes on any input, a null element and a large element take the checked
loop) — and is the first bisect step for a wrong sum out of such a loop on native.
`LOFT_HOIST_VERIFY=1` re-runs every admitted block through the checked add and panics on a
disagreement — the proof itself, so it has no blind spot.  The bound test is spelled in
add / shift / or on purpose: baseline x86-64 is SSE2, with a packed 64-bit add and no packed
64-bit signed compare, and written as two compares the loop stays scalar and gains nothing.
**`LOFT_NO_RANGE_ARITH=1`** (`@FR-R-Range`, default-ON, generation time, `--native` only)
makes every integer operator keep its checked template again — with it off, an operator
whose RESULT provably fits the type (a literal, a mask of a non-sentinel value, `len`/`size`
in `0..=u32::MAX`, a byte, a counted range's counter, a one-expression callee such as
`color_a`, and interval arithmetic over those in i128) emits the processor's operator, since
no fault can occur — and is the first bisect step for a wrong integer value on native where
the range proof admitted a plain operator; `LOFT_HOIST_VERIFY=1` compares the plain answer
with the checked one at every such operator.  It is C120's admissible successor for
straight-line arithmetic; a parameter or a record/element read is never ranged.  ⚠ Its
counted-range-counter clause was INERT until 2026-09-21 (loft#1558): the seeding looked for
the counter's seed INSIDE the loop and the parser emits it as the statement before, so no
counted loop was ever ranged.  Nothing showed it, because the pin that should have
(`range_arith` a4) was recording a plain form the CHAIN GUARD supplied — a pin can borrow
another rewrite's evidence and read as proof of its own clause.
Since 2026-09-22 the proof also reads the STATIC TYPE (`range::type_range`): a non-nullable
`u8`/`i8`/`u16`/`i16` or user `limit(lo, hi)` parameter, local or compiler-typed join (the
cbor decoder's `(bytes[p] ?? 0) * 256`) carries its type's range — a fact since loft#1593 made
every store into such a slot refuse an unprovable value; `i32`/`u32` (the templates) and a
signed alias with a spare bottom code (`limit(-100, 100) size(1)`, whose overflow writes the
sentinel) stay unranged, and a boxed capture is read through its box and stays checked.
**`LOFT_NO_GUARDED_CHAIN=1`** (`@FR-R-GuardedChain`, default-ON, generation time, `--native`
only) makes a counted loop's index chains keep their checked operators — with it off, chains
of `+ - *` and negation over literals, the loop's and nested loops' counters, integer locals
the loop never writes and hoisted record scalars run PLAIN behind a guard evaluated once at
the loop's entry (every leaf not the sentinel, every chain's magnitude bound fits), the
checked loop being the `else` arm (`composite` 103 → 68 µs: `j * lw + i`, `x0 + i`, `y0 + j`
over record scalars no static proof can bound) — and is the first bisect step for a wrong
index or accumulate in a counted loop on native.  A loop with ONE admitted operator
declines on PROFITABILITY (the guard is a fixed cost per loop ENTRY against a saving of one
null test per operator per ITERATION, and the doubled body costs a small hot function its
inline): measured against the guard off, the one-operator loops in `pil_hline` and
`matches_at` were costing `fill_circle` and `fill_star` ~50 %, `wide_line` 22 % and `parse`
18 %, while six-operator `composite_layer` gains 30 %.  `LOFT_HOIST_VERIFY=1` is the falsifier,
`LOFT_TRACE_CHAIN=1` names every loop admitted and declined with its operator count, and
**`LOFT_GUARDED_CHAIN_ONLY=<fn>[,<fn>…]`** admits the guard in the named functions ALONE — the
bisect step for a row that moved under `LOFT_NO_GUARDED_CHAIN`, because a whole-program A/B
cannot say which admitted loop paid (the drawing `parse` row lost 18 % to ONE of six: a 3–8
trip loop in a per-byte helper, whose doubled body stopped inlining into its caller).
**`LOFT_NO_BLOCK_REPEAT=1`** (runtime, BOTH backends) makes `[x; n]` fill one element at a
time again — a `copy_block` and a `copy_claims` call each — where with it off
`Stores::fill_from_template` doubles block copies and walks claims only for a heap-owning
template (`lock_curved` −14 %); first bisect step for a wrong element out of a repeat literal or
a constant comprehension.  **`LOFT_NO_FIELD_FILL=1`** (parse time, BOTH backends) keeps a
constant comprehension in a struct FIELD (`Lay { best: [for _ in 0..n { 2.0 }] }`) on its
per-element push loop — with it off it takes the repeat-literal lowering a local's already has,
when the field is a plain vector (`is_plain_vector`; a keyed collection stays on the loop, since
n appends are not n inserts); together with the block repeat `lock_curved` 4.2× → 2.2×.
**`LOFT_NO_SELF_APPEND_BLOCK=1`** (runtime, BOTH backends) makes `v += v` copy through a
byte snapshot taken before the growth again — with it off, a self-append is one block copy
inside the grown record, its source re-read from the field slot after the growth (which is
also what fixed the heap-owning case: the claims walk read the source through a number
captured before the growth relocated it — a freed block).  The doubling fill a canvas is
built with is this shape run to a ladder; `render_marks` −8 %.  First bisect step for a
wrong element out of a self-append.
**`LOFT_NO_FAST_ORDER=1`** (@PLN158 keyed class, runtime, BOTH backends) makes every search
that ORDERS — an `index` descent, a `sorted` / `ordered` binary search — compare through the
general `key_compare` / `compare` again, an exact `index` lookup take the boundary descent,
and an `index` insert look its duplicate up before it descends — with it off, the key is
resolved once per search (`keys::FastOrder`), a full-key lookup stops at the equal node
(`tree::find_exact`), and a refused `tree::add` names the duplicate its own descent met
(`index` fill-and-find −33 %) — and is the first bisect step for a wrong element, order or
lookup out of a `sorted`, `ordered` or `index`.  **`LOFT_NO_ONE_PROBE_INSERT=1`** makes a
`hash` insert look its duplicate up and then file the entry as two hash-and-probe walks again
— with it off, one walk answers both (`hash::probe_for_insert`; a 5,000-key fill −37 %) — and
is the first bisect step for a lost, duplicated or unfindable `hash` entry.
**`LOFT_KEYED_VERIFY=1`** is the falsifier for both: every pre-resolved comparison, every
exact lookup and every one-probe insert is checked against the general form as it is made,
and a disagreement panics naming both answers — run the keyed cells or the script corpus
under it after touching `keys.rs`, `hash.rs`, `tree.rs` or the `sorted` searches.  Two more
levers of the same pass carry no switch because they have no second answer to bisect to: a
miss on a collection with no lazy binding answers at once (`Stores::lazy_bound`, a lookup
loop that misses half the time −20 %), and a whole word fed to the hasher on a word boundary
is one inline round (`SipHasher13::write_u64`, digest pinned by `siphash_std_parity`).
**`LOFT_NO_HALF_LOAD=1`** (runtime, BOTH backends) rebuilds a `hash` table at three quarters
full again instead of at half — a writer's policy no reader assumes, so stores written under
either read under both; with it off a miss walks ~1 bucket where it walked 5 at the old
threshold (removal −25 %, the vector + hash group −22 %, fill −21 %), for ~3.6 bytes an entry
more bucket array — and is the bisect step for a `hash` whose table size matters.
**`LOFT_NO_TYPED_KEYED=1`** (`@FR-R-TypedKeyed`, generation time, `--native` only) emits a
lookup in a `hash` with ONE integer key as the general `OpGetRecord` again — with it off it
is `OpGetHashLong`, the key handed over as the integer it is, with no `Content` built, no
type-row dispatch and the walk compiled for the key's kind (613 → 444 instructions a lookup,
`hash_find` −13 %) — and is the first bisect step for a wrong or missing record out of such
a lookup on native; `LOFT_KEYED_VERIFY=1` answers every typed lookup through the general
entry too, and checks a removal's recognised slot against the entry's own index.
`bench/portal/analysis/keyed.md` has the ledger and what is left.
**`LOFT_RELEASE_PASS_PROBE=1`** (generation time) is a MEASUREMENT INSTRUMENT, never a
build anyone ships: every integer `+`, `-`, `*`, negation, bit op and non-literal
division emits the processor's wrapping operator and every float comparison the plain
one — what the eventual release build pass for games would emit (DESIGN_DECISIONS.md
C120, NATIVE.md § Optimisation tiers).  The values after a fault are NOT the language's
(`b = MAX + 1; d = (b + 5) * 2` reads `10` for null), so a row is comparable only while
its hash still agrees; its time is the CEILING the checked build is measured against,
which is what says whether a row is bound by the checks or by something else — the
guide for what to optimise next.  The null test itself and the `*Nullable` twins keep
their templates: they are the language's null semantics, not its fault protection.
**`LOFT_NO_VALUE_RECORD=1`** (@PLN157 § V-aa, default-ON since 2026-09-14) makes a
function whose result is a plain no-heap record of ≤6 scalar fields return it through
the buffer again — with it off, such a function whose every call site reads fields off
it and whose body builds it with `Object` blocks returns those fields in REGISTERS with
the call site reading tuple elements (measured: 1.65× on the call, `smooth` −25 %
standalone, `lock_curved` −3 %) — and is the bisect step for a wrong field out of a
record-returning call on native.  It was opt-in for two days because its call-site gate
did not hold over the script corpus (376 compile errors, 218 of them the fn-ref
DISPATCH: every arm of the `match` a `CallRef` emits shares one return type); the gate
now declines every arm by reading the arm set from `fnref::dispatch_arms`, the
emitter's own home for that question, and a mixed-arm branch is declined by shape (a
`__lift_` temp bound from a CALL was too, until `(R-ValueLocal)` made it a value local).
A `__lift_` temp bound from a
VIEW — a selecting tail's arm over a by-value parameter — is a value local bound to the
view's field tuple and never a view leaf (read as one it leaked the store its copy
mints, one record per call; found 2026-09-15 with the generic instance's statement join,
whose arms bind the join local the same way and did not compile).
`LOFT_TRACE_VALUEREC=1` names each admission and decline.
**`LOFT_NO_VALUE_LOCAL=1`** (`@FR-R-ValueLocal`, default-ON since 2026-09-25, generation
time, `--native` only) keeps every by-value record PARAMETER a `DbRef` again — with it
off, a parameter of a plain no-heap record of ≤6 scalars is received as the TUPLE of its
fields where the callee reads it only field-wise, hands it to another such parameter,
copies from it or answers it, AND the callee's body (with everything it calls) writes no
record of that type through any route a caller could view — a by-value record parameter
is a VIEW of its argument (`bump(p, w) { w.x = 5.0; p.x }` called `bump(a, a)` answers 5),
so the write set keyed by record type decides, a callee's return buffer and the frame's
own records set apart from it (`WriteSet::reaches_record`); a call site hands a value
local or an admitted call as it is and reads the fields off any other record at the site
(mesh3d `mat4_transform` 9.9 → 2.64 ms, 5.0× → 1.32× of Rust; `sphere` −38 %) — and is
the first bisect step for a wrong field read through a small-record parameter on native.
The interpreter is the values oracle; `LOFT_TRACE_VALUEREC=1` names each parameter
carried and each declined with the reason, and the aliasing cells are
`tests/scripts/a-small-record-parameter-is-carried-as-a-tuple.loft`.
**`LOFT_POISON_CLAIM=1`** (`Store::poison_fill`) fills a freshly CLAIMED payload with
`0xDEADBEEF` instead of zeros — the claim-side twin of `LOFT_POISON`'s poison-on-free, and
the falsifier for *"does this caller rely on zero-init?"*: a handle or length read out of
unwritten space becomes a loud out-of-range value the store's guards refuse, where
`LOFT_NO_ZERO_CLAIM=1` only leaves stale bytes that often look enough like zeros to pass.
Census 2026-09-12: 1 232 of 1 261 `tests/scripts` clean on the interpreter, 29 dependent
(the buffer/delivery family), every @PLN157 cell corpus clean on both backends.
**`LOFT_NO_STORE_RESET_CLEAR=1`** (@PLN157 § V-ag, runtime, BOTH backends) makes that
release a WALK again — with it off, clearing a store-ROOT vector resets the store in one
step and re-establishes its two records, because the shape `@FR-H-ClearRelease` already
tests says the vector owns the store's whole extent (`fronds` −36 %, the free tree was
28 % of that row) — and is the first bisect step for a wrong value, a leak or a
use-after-free at a recycled vector-returning call.
**`LOFT_NO_RESET_CAPACITY=1`** (@PLN157 § V-ai, runtime, BOTH backends) makes that reset
re-establish the vector at the fresh minimum again — with it off, the vector comes back at
the CAPACITY the previous fill reached (the buffer is reused across calls and the store
already holds the extent), so the growth ladder runs once per buffer instead of once per
call, and the freed rungs that took every later claim off `bump_tail` and into the free
tree are never made (`fronds` −22.5 %, 4.28× → 3.42× on x86-64) — and is the bisect step
for a wrong element or length out of a reused vector-returning call.  `LOFT_TRACE_CLEAR=1`
prints the capacity each reset re-establishes.
**`LOFT_NO_CLEAR_RELEASE=1`** (`@FR-H-ClearRelease`, runtime, BOTH backends) makes a
vector's entry clear a pure length reset again — with it off, clearing a REUSED
store-root vector whose elements own heap releases what they own first, closing an
unbounded leak in every shape-A return buffer (~357 KB per `fronds` call) — and is the
bisect step for a double free or a wrong value at a cleared vector.  `LOFT_TRACE_CLEAR=1`
names each clear's shape and element verdict.
**`LOFT_NO_PREFILL_IMAGE=1`** (@PLN164 C4, `@FR-R-Prefill`, runtime, BOTH backends) makes
every record mint that is not proven complete-write prefill its defaults field by field
again — with it off, the prefill is ONE block write of a per-type image captured from the
walk's first run over a zeroed span (the `parse` row −11 %) — and is the first bisect step
for a wrong default, sentinel or variant tag in a minted record.  `LOFT_PREFILL_VERIFY=1`
re-runs the walk after every image write and panics naming the type where the bytes
disagree; `LOFT_TRACE_PREFILL=1` names each capture and each use — and a cell can pass
without ever reaching the image, because on `--native` a literal's mint is a complete
write that never prefills and a callee's buffer is minted once per caller activation.
**`LOFT_NO_LITERAL_EXIT_BUFFER=1`** (@PLN164 B2, `@FR-R-Place`'s callee clause, parse
time, BOTH backends) makes a mid-body `return S { … }` build its record in a store of its
own again — with it off, every literal exit of a record function writes the `__retbuf`
the caller handed, through the same null-guarded mint the TAIL literal has used since
@PLN157 § V, so a callee with several literal exits answers ONE store; that includes a
buffer a CHAIN renamed (`return no_mk()` beside `return Mk { … }`, `parse_circle`'s
shape), and a literal tail beside chain exits — and is the first bisect step for a wrong
record out of a callee with more than one exit.
**`LOFT_NO_PLACE_RESULT=1`** (@PLN164 B2 units 2–3, `@FR-R-Place` + `@FR-R-MoveLast`, decided
after the scope pass, BOTH backends) keeps a call result minting its own store and deep-copying
into its destination again — with it off, a plain local bound from a callee whose every exit
writes the handed buffer, and whose ONE owning destination on every path is a record-literal
field of an element appended to a PARAMETER's collection (`sc.ops += [Op { paint: pp, … }]`),
gets that buffer CLAIMED IN the parameter's store (`OpPlaceRecord`), the field takes it by
RELOCATION (`OpMoveRecord`: the bytes move, the heap handles keep their claims, the source's
block is released on the spot), and the exit releases the record alone on the one path that
still holds it (`OpFreeRecordIn`) and nothing after a move — the path state is a compile-time
fact the pass writes into the IR, never a runtime stand-in (no zeroed source, no `v = null`,
which the interpreter lowers as a store-level free of what the local held).  It is the first
bisect step for a wrong field, a leak or a double free out of a record built by a call and
stored in an appended element; `LOFT_TRACE_PLACE=1` names each admission and each decline
(a read after the store, a hand-off to a call, a rebind, a second destination or host, a
destination in a local record, a loop, a rejoin of a stored and a held path all keep the copy).
Measured 2026-09-16 on the drawing bench's parse row: one store fewer per line and the copy
gone, and a WASH on every counter (`perf stat`: instructions, cycles and cache misses equal
within noise) because the record relocated there carries no heap on that scene.
**`LOFT_NO_REBIND_PLACE=1`** (`@FR-R-Rebind`, default-ON since 2026-09-24, parse time +
scope pass, BOTH backends) keeps `x = f(x, …)` minting the callee's exit record in a store
of its own and copying it over `x` again — with it off, the call's hidden buffer argument IS
`x`, so the callee's exit literal writes x's record in place (a vector field that views the
same field, `Doc { buf: d.buf, … }`, is `(H-CopySelf)` and costs nothing; a scalar the
literal reads off the parameter is STAGED ahead of the first write; `return d` answers the
buffer itself), and a body whose last statement is an explicit `return` takes the buffer
road at all (zttext `delete_range` 138× → 9.6×) — and is the first bisect step for a wrong
field, a leak or a double free out of a local rebound from a call that takes it.
`LOFT_TRACE_REBIND=1` names each admission and each decline (a text field or a vector read
at ANOTHER field of the parameter, a literal-list vector field, a chain exit, a promoted
buffer, the local handed in twice, a view or witnessed local each decline); the falsifiers
are `LOFT_STRICT_STORES` / `LOFT_POISON` / the native leak check on the cells.
**`LOFT_NO_CONST_VIEW=1`** (`@FR-R-Const`, default-ON since 2026-09-25, parse time + scope
pass, BOTH backends) makes a literal-bodied function build its vector on every call again —
with it off, a zero-parameter function whose whole body is ONE vector literal over literals
(`fn face_rows() -> vector<integer> { [0, 0, 0, 4, …] }`, the idiom the `const` diagnostics
themselves prescribe for what the constant store cannot paste) is given a synthetic constant
twin, pre-built ONCE in `CONST_STORE` like a top-level `NAMES = [ … ]`, and a call whose
result only lands in READ positions — the single bind of a local that is only indexed,
measured, iterated or copied into another such local, or the call itself under an index,
a `len` or an iteration — answers `OpConstRef`, the node a top-level constant's use site
emits; the call's lazy buffer is never minted.  Every other position keeps the call (a
write or append through the local, a hand-off to ANY user call, a store into a field or
element, a return, a `&` link), because the constant store is write-locked and the
program's copy must be its own — and is the first bisect step for a wrong element, a
"write to read-only store" panic or a stale table out of a call of such a function.
`LOFT_TRACE_CONST=1` names each function made a constant (and each that is not, with its
body's shape) and each call admitted or declined; the interpreter and native share the
rewrite, so the switch A/B and the cells' hand-computed values are the falsifier.
**`LOFT_NO_COMPACT=1`** (`@FR-R-Compact`, default-ON since 2026-09-25, scope pass, BOTH
backends) makes a vector rebuilt from a contiguous run of its own elements copy again —
with it off, `t: vector<S> = []; for i in a..b { t += [V[i]?]; } V = t;` (a history
truncated to its cursor, its oldest entries dropped) is ONE guarded op: `if 0 <= lo &&
lo <= hi && hi <= len(V) { OpKeepVectorRange(V, tp, lo, hi) } else { the statements as
written }`, the op releasing every element outside the run where it stands, moving the
run to the front as one block and setting the length, and the fallback arm answering
exactly what the copy answered for any range the guard refuses (a negative start, an end
past the length that pads with default records, a null bound).  RECORD elements read
through `?` only — a scalar in range can hold the null its `??` replaces — and `t` named
nowhere else; the filter, prepend and identity forms of the rule keep their rebuild
(dryopea `truncate_to` 259× → 14×).  It is the first bisect step for a wrong, missing or
stale element after a vector is rebuilt from its own elements; `LOFT_TRACE_COMPACT=1`
names each rebuild admitted and each declined with the reason.
**`LOFT_NO_ELEMENT_IN_PLACE=1`** (@PLN164 C1, `@FR-R-InPlaceLiteral`, parse time, BOTH
backends) makes `v[i] = S { … }` build its record in a temp store and deep-copy it into the
slot again — with it off, the literal writes the slot's fields, as the FIELD destination
`o.f = S { … }` always has, with every field expression STAGED first (the language evaluates
a literal's fields before the assignment stores, so `El { a: o.f.b, b: o.f.a }` reads the
place it is about to overwrite) and an omitted field taking its declared default — and is the
first bisect step for a wrong element, a leak or a double free at an element assigned from a
literal.  A computed index keeps the copy (the receiver is re-derived per field write), as do
a collection-TYPED field and a linked-group member.
**`LOFT_NO_BUFFER_IS_PLACE=1`** (@PLN164 C2, `@FR-R-Place`, `@FR-O-Buffer`, parse time, BOTH
backends) makes `h.v = mkints(n)` mint a buffer store, fill it, clear `h.v` and copy every
element in again — with it off, the call is handed the destination AS its return buffer: the
buffer stays the variable the call site mints and only what it holds changes, so the result's
deps and the free sweep are untouched, and `(O-Buffer)` gains the clause that such a buffer is
never freed — and is the first bisect step for a wrong, empty or stale vector field after an
assignment from a call.  Declined where an argument reaches the destination, where the callee
MINTS into its buffer (a returned vector literal does), and for a struct-enum variant's field.
Measured on the drawing bench's parse row: the consumer spells this nowhere — the emission is
byte-identical under the switch — so the gain is structural.
**`LOFT_NO_VIEW_FIELD=1`** (@PLN164 C5 and E-1, `@FR-O-ViewField`, `@FR-R-ValueRecord`,
**default-ON since 2026-09-17**, generation time, `--native` only) restores the record form —
with it off, a function returns a record of two scalars and a vector as a TUPLE
whose vector element is a REFERENCE to the place the value already lives in — so a
`Mark { matched, bad, pts }` built from a local the function appended into a parameter's
collection costs no buffer store and no deep copy, and an exit with an empty literal delivers
a null view, which reads as the empty vector it replaces.  The interpreter keeps the record
form and is the values oracle.  Admitted only where the leaf's place outlives the frame and
every call site READS it: a `?`-discharged element declines (its ownership is a join — the
absent arm mints in the frame's store), an element read or an iteration at the site declines
(an element read answers a PLACE the site can write through — measured: the write landed in the
container, reading 109 where the copy holds 9), a disturbance between the bind and the last read
declines (measured: a removal made the result read the NEXT element's points), and a `pub`
function declines whole (`(R-Escape)`).  The exit's own answer is a PROOF rather than a
fallback: every mention of the return buffer must be one the tuple accounts for, because a
field filled without an append — a returned vector literal, a record literal's vector field —
read as "empty" delivered a null view for a vector of eight (`723-ncc-loop-element-bind`).  The
callee's half is a per-PATH PROOF (`hoist::fresh_leaf`, a forward walk that joins `if` arms
and runs loops to a fixpoint): on every path to the exit the last change to the container was
the append of the local's copy, and neither the local, a store it views, nor that element
changed after it.  An append in each arm of an `if` views the container's LAST element; an
append under a condition, or a local grown, rebound or written after its copy, declines
(measured wrong under the per-exit test this replaced: 0 points for 3, and 3,9 for 4,109).  A
naming that reaches a SIBLING field or claims a record in the store moves nothing in the one
the leaf views.  The gate stores each exit's leaf and the emitter writes the stored one.  It
is the first bisect step for a wrong or stale vector read out of a record-returning call on
native, and `LOFT_TRACE_VALUEREC=1` names every admission and decline, with the reason and —
for a site — the caller that consumed it.
**`LOFT_NO_FORWARD_TUPLE=1`** (@PLN164 E-1, `@FR-R-ValueRecord`, **default-ON since
2026-09-17**, generation time, `--native` only) makes a forward decline its callee again —
with it off, a function that keeps its record FORWARDS an admitted callee's answer —
`return nm()`, or the call arm of a value branch whose other arm is a record, both lowered as
the callee filling a buffer bound back from the call — by writing the tuple into that buffer
at the site: minted
where it is absent, every scalar set, a view part's vector copied; the call evaluates to the
buffer.  Without it such a forward is a site that consumes the record and declines the callee
everywhere — which is what kept the drawing library's `parse_circle` and `parse_line_cmd` on
buffers (their `no_mark()` tail was forwarded by `parse_fronds`).  It is the first bisect
step for a wrong field, a leak or a null-store panic at a `return g(…)` of a record-returning
function on native.
**`LOFT_NO_CALLEE_DISTURB=1`** (@PLN164 C3, `@FR-B-Disturb`, `@FR-B-Ref-Reshape`, BOTH
backends) makes the disturbance walk read THIS frame's ops only again — with it off, a
container a CALLEE grows or removes from disturbs the caller's live view of it, so
`e = sc.els[0]?; grow(sc, 200); e.a + e.b` materialises the view instead of reading an element
that moved (it answered 4294967401 for 3), the reach closed over the call graph
(`disturbed_params_map`) and the copy notice naming the callee — and is the bisect step for a
wrong field read through a view across a call that appends to or removes from a container.
`LOFT_TRACE_DISTURB=1` names each disturbed place; a hidden `__retbuf` is an argument slot and
is excluded, or every record-returning function that fills a vector field would report one.
**`LOFT_NO_ELEMENT_FIRST=1`** (@PLN157 § V-z) makes a record-literal's vector field
keep its temp-store build and deep copy again — with it off, a local vector consumed
exactly once by one append is built INSIDE the appended element (minted at the temp's
declaration, invisible until the finish's length bump) — and is the bisect step for a
wrong vector field of an appended record on native, or for a wrong value read through an
element view between such a local's declaration and its append (loft#1553).  The declaration must be the local's only
binding (loft#1552: a local declared `[]` and rebound under an `if` left the element empty).
**`LOFT_NO_ELEMENT_PLACE=1`** (@PLN164 E-2, `@FR-R-ElemFirst`, default-ON, generation time,
`--native` only) keeps that build to local vectors and literal-built locals again — with it
off, it also reaches an append into a record's COLLECTION FIELD (`sc.ops += [Op { pts: p }]`)
and a local a CALL fills (`p = smooth(raw)`): the call is handed the early element's field as
its return buffer, provided it fills its buffer and never mints into it, no other argument
names the container, and the buffer serves that call alone.  An append in EACH arm of an `if`
(`parse_circle`'s stroked/filled pair) shares one early element, the second arm's mint an alias
of the first's.  No statement between the declaration and the append may name the container
(a naming that reaches only a sibling field is fine), jump out (a stranded early element keeps
the call's vector inside the container's store, where only a record census sees it), or read a
VIEW the early mint may have moved — the mint is the append's growth brought forward, so a
local viewing the container's element, or any heap parameter where the container is a
caller's, declines (loft#1553; a view of a sibling collection is spared).  It is the first
bisect step for a wrong, empty or leaked vector field of an element a parser appended;
`LOFT_TRACE_ELEMFIRST=1` names every admission and decline, and the variable a declined
window reads.
**`LOFT_NO_COMPLETE_WRITE=1`** (@PLN157 § V-y) makes every record keep its default
prefill again — with it off, a literal group the emitter PROVES writes every field
(declared defaults, sentinels, the variant tag included: the parser's lowering is
complete by construction) calls a no-prefill `OpDatabaseNP`/`OpNewRecordNP` twin —
and is the bisect step for a wrong default or sentinel in a literal-built record
on native.
**`LOFT_NO_LITERAL_HOIST=1`** (@PLN157 § V-x) makes an invariant loop-body vector
literal rebuild per iteration again — with it off, `v: vector<float> = [1.0, 2.0]`
(or an if-of-literals on a const-param field) under a loop builds ONCE per activation,
guarded on the local being unbound — and is the bisect step for a wrong constant
vector inside a loop on native.
**`LOFT_NO_RETBUF_ADOPT=1`** (@PLN157 § V-u) makes a vector-returning function keep its
delivery copies — with it on, a shape-A result local ADOPTS the hidden return buffer
(built where it must end up; the exits deliver nothing; the buffer's backing reused across
calls) — and is the bisect step for a wrong vector return, a leak at a vector-returning
call, or values accumulating across calls.  `LOFT_TRACE_ADOPT=1` names the declining gate.
The family's rules and their citations: `doc/claude/formal/rewrites.md` (`@FR-R-…`);
**`scripts/emission_audit.py <emitted.rs>`** validates a `--native-emit` output against
them (one holder per path per frame, no mover on a held path, a twin handed only live
holders) — run it on any emission that looks wrong before running the program.
PERFORMANCE.md § Design: P2, NATIVE.md.

**A null-discharge buffer does not block the hoist (@PLN157 § V-ad, `--native`, generation
time):** `e = tbl[i]?` on a vector of all-scalar records mints an absent element into a
hidden per-site buffer, and that allocation used to decline every header in the loop
around it — the drawing bench's polygon crossing loop hoisted nothing (`wide_line` −31 %
when it did); since 2026-09-18 it does not block the loop's BASES either (a fresh store,
or a clear of the buffer's own, moves no element a base addresses).
**`LOFT_NO_NULL_BUFFER_HOIST=1`** restores the blocking form and is the
first bisect step for a wrong element read in a loop that discharges a record element
with `?`; `LOFT_HOIST_VERIFY=1` is the falsifier.

**A value branch of calls witnesses every arm's buffer (@PLN157 § V-af, BOTH backends,
parse time):** `v = if c { mk(i) } else { mk2(i) }` in a loop binds `v` to whichever arm's
hidden return buffer ran, so every arm's buffer is `v`'s witness — the buffers are
allocated once per activation and `v`'s per-iteration free declines against each, where
before the branch paired nothing and every iteration minted and freed a store (the
consumer's `smooth` −26 %).  **`LOFT_NO_JOIN_BUFFER_WITNESS=1`** restores the unpaired
form and is the first bisect step for a leak, a double free or a stale record out of a
loop that binds a record from a branch of calls; `LOFT_STRICT_STORES=1` and
`LOFT_POISON=1` are the falsifiers.

**A filling loop is one slice fill (@PLN157 § V-ae, `--native`, generation time):**
`for i in lo..hi { v[base + i] = c }` with `base` and `c` invariant, over a vector a
header is held for, emits ONE range test and a slice fill, the per-element loop kept as
the fallback for every range the fill declines (a negative or partial index, an empty
range, an overflow) — `wide_line` −23 %, the two fills −48 %.  **`LOFT_NO_FILL_HOIST=1`**
emits the per-element form again and is the first bisect step for a wrong element or a
missed write out of a filling loop; `LOFT_TRACE_FILL=1` names the check that declined a
loop the idiom should have taken.

**Store confinement across sibling blocks (default-ON since 2026-08-21, both backends):** a
local reassigned across sibling `if`/`else if`/`match` arms used to keep EVERY arm's store
alive to scope exit, so the watermark grew with the number of reassignment SITES rather than
with how many of them run — a 16-site function peaked at 20 stores whichever single arm was
taken. `recover_backer` confines each block's store to its block: a flat **5** at 2, 4, 8 and
16 sites. **`LOFT_NO_CONF_RECOVER=1`** emits the pre-confinement form and is the first bisect
step for a wrong answer in a function that reassigns a local across sibling blocks. ⚠ The
soundness condition is `store_dead_after_block`, NOT the flag: a local READ after the blocks
does not confine, because freeing a confined store while the local still holds it returns the
wrong element on the branch NOT taken. QUALITY.md § Cluster III Route 2.

**Free-block footers (@PLN157 § V-v, default-ON, both backends, `@FR-H-FreeFooter`):** a
store's FREE block carries its size at both ends, so a record delete coalesces BACKWARD in
O(1) off the footer (tree-confirmed — claimed data can spell a false footer) instead of
leaving adjacent frees to `claim`'s lazy O(blocks) sweep; the sweep stays armed only for
the untracked one-word case.  **`LOFT_NO_FREE_FOOTER=1`** restores the sweep-only form
whole (every delete arms it) and is the first bisect step for a store-layout fault in a
delete-heavy run.  Measured: `fronds` −7.7 % (the § V-u arena's churn), and the class it
retires is the `coalesce_free` cliff PERFORMANCE.md § V-j P2 first measured at 29.5 % of
a shared-arena row.

**The tail free block is a wilderness (@PLN164, default-ON, both backends,
`@FR-H-Wilderness`):** the free block that ends a store is held beside the free tree, and the
tree's insert, remove and best-fit take treat it as the node it would have been — so every
claim takes the block it always took (a seeded side-by-side unit test pins the layout), and a
claim from the tail or a delete into it costs no tree delete, insert or rebalance (64 % of the
parse row's tree claims took the tail; the row −6 % in cycles).  **`LOFT_NO_WILDERNESS=1`**
keeps the tail in the tree again (read per store at construction) and is the first bisect step
for a store-layout fault or a claim that hands out a live block.

**A best-fit claim carves its node in place (default-ON, both backends, `@FR-H-Carve`):** a
claim from a free-tree block at least twice the request leaves the remainder in the block's
place in the tree (links, color, parent pointer) instead of a delete and an insert — the block
is the smallest that fits, so the remainder keeps its order, and every claim takes the block it
always took (`hash_text_keys` −8 %).  **`LOFT_NO_CARVE_IN_PLACE=1`** deletes and inserts again
(read per store at construction) and is the bisect step for a store-layout fault or a corrupt
free tree after a claim.

**Owner witness for a mixed-ownership local (loft#1336, default-ON, both backends):** a
heap-record local that OWNS after one assignment (a copy, a minting call) and VIEWS after
another carries a hidden `__own_<name>` naming the store it minted while it still holds it;
the IR releases it by store identity at the rebind or at scope exit, and the local itself is
never-free (`formal/ownership.md` @FR-O-Witness). **`LOFT_NO_OWNER_WITNESS=1`** emits the
pre-witness form: the first bisect step for a leak or a wrong answer in a local that is both
copy-bound and view-bound (a walker `cur: Node? = a; cur = cur.next`), and what the
`LOFT_NO_JOIN_OWN` positive controls set beside their own switch.

**The counted loop's second counter (@PLN157 § V-ab, default-ON, both backends):** a
`for i in a..b` whose start is not a literal runs a hidden `next` counter seeded AT `a`
(tested, yielded into `i#index`, then stepped) instead of a null-encoded counter that asked
`if !i#index { a } else { i#index + 1 }` on every iteration — a null test and a select LLVM
cannot fold, half the cost of a tight loop; a literal start already took P3b's single
counter seeded at `a - 1`, which a computed start cannot use because `a - 1` is the null
sentinel when `a` is the type's minimum.  **`LOFT_NO_NEXT_COUNTER=1`** emits the
null-encoded form again on both backends: the first bisect step for a wrong value out of a
counted loop whose start is a variable or an expression.  Every value the compare and the
step see is unchanged, so the edges are too — including loft#1525, an inclusive range to
the type MAXIMUM that never terminates on either form.

**Adopt at first bind (@PLN164 B1, `@FR-O-Move`, default-ON, both backends, parse time):**
a plain local first-bound from a callee that returns the local it promoted onto its buffer
(`fn mk() -> P { o = P { … }; …; o }`) adopts the store the callee minted — one mint and one
free per call where there were two mints, a deep copy and two frees — paired with the call's
buffer for an identity-guarded free exactly as a literal-returning callee's result is.  The
buffer stays null on purpose: pooled at function entry it is freed by the interpreter's
rebind of the promoted local (plan 51 cluster 3's shape; native guards that free with
`_rb_w_`), which is the plan's B1b.  **`LOFT_NO_ADOPT_FIRST_BIND=1`** restores the copy and
is the first bisect step for a leak, a double free or a wrong field out of a local bound
from such a callee; `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` are the falsifiers.

**Reuse the adopting call's buffer (@PLN164 B1b, `@FR-O-Buffer`, default-ON, both backends,
scopes pass):** the record-buffer pool now takes those buffers too, so such a callee is handed
the CALLER's store after the first call and fills it — one store per call site per activation
instead of one per call (the parse row's `Mark` class).  A callee's promoted buffer local then
holds either the caller's store or one it minted, so the scope pass snapshots the store handed
in (`__rbw_<buf> = OpRefAlias(buf)`) and every free of that local declines on it; and a local
first bound inside an `if` (whose pre-init makes the bind a rebind) adopts at that bind
(`Variable::deferred_first_bind`).  **`LOFT_NO_ADOPT_BUFFER_REUSE=1`** keeps those buffers
null again and mints no snapshot — the first bisect step for a use-after-free or a wrong
field out of a callee that rebinds or returns past a local it promoted onto its buffer.
`LOFT_TRACE_POOL=1` names the gate that keeps each witnessed buffer out of the pool.

**Mint at first use (@PLN164 A0, `@FR-O-LazyBuffer`, default-ON, both backends, scopes
pass):** a hidden return buffer (`__ref_N`) is minted in front of the statement that hands it
to a callee, behind `OpRefIsNull`, instead of at function entry — a scanner tried on every
line and matching one paid its buffers' mint and free on all the others (the drawing parse
row −7 %).  A vector buffer carries `Variable::lazy_buffer`, so its entry init is the null
sentinel on both backends and its guarded `Set(b, Null)` is the mint.
**`LOFT_NO_LAZY_BUFFER=1`** mints at entry again and is the first bisect step for a leak, a
double free or a wrong value at a call that takes a hidden buffer.

**A frameless call tree carries no prelude (@PLN157 row 11, `@FR-R-LeafChain`, lean tier
only, generation time):** N4's leaf rule made transitive — a function whose every user
callee is loft-bodied, off any cycle and free of fn-refs drops its depth entry and buffer
guard in `--native-release`, since there the frame is only a depth count (parse row −12–14 %).
**`LOFT_NO_LEAF_CHAIN=1`** keeps those frames (one step finer than `LOFT_NO_LEAF_PRELUDE`);
the named tiers never elide a non-leaf.

**A `??` chain is RIGHT-associated (loft#1612, `@FR-G-Assoc`, `@FR-B-Copy`, default-ON, both
backends, parse time):** `x ?? y ?? d` parses as `x ?? (y ?? d)`, so every operand is an ARM of a
plain `if` and an arm's bind is the copy `(B-Copy)` describes.  Left-associated, `(x ?? y) ?? d`
has a subject that is not a variable, which is hoisted into a `__ncc_N` temp that VIEWS the
operand it chose — so the destination shared that operand's store (a record on the interpreter, a
vector and a call-first chain on both).  Either grouping answers the same operand in the same
order, so no program's value moves with it.  **`LOFT_COALESCE_LEFT_ASSOC=1`** parses the old form
and is the first bisect step for a wrong value, a leak or a double release out of a chain of three
or more operands.  A chain the AUTHOR parenthesised still hands the parser a hoisted subject, and
is right-associated in the SCOPES pass instead (**`LOFT_NO_COALESCE_REASSOC=1`** keeps loft#1591's
destination gate, the bisect step for that spelling).  Beside them,
**`LOFT_NO_CHAIN_ARM_SINK=1`** stops a chain BLOCK being a sunk ARM of a branch — with it off,
`x: H = if c { s.h ?? b } else { mk(3) }` writes the chain out as a statement of its own, but ONLY
where every arm of that chain is owned, since an arm that views a place the program can still
reach owes the destination a copy the written-out `Set` cannot make — and is the bisect step for a
double release, a leak or an aliased record out of a branch with a `??` chain in one of its arms.

**A nested literal is written into its field (@PLN164 C6, `@FR-R-InPlaceLiteral`, default-ON,
both backends, parse time):** `sc.ops += [Op { paint: Paint { … } }]` writes `Paint`'s fields
into the element's `paint` field instead of building a store of its own and copying it —
only where the outer record is fresh (an appended element, a construction temporary), so
nothing can read the place while it is written (parse row −6–7 %).
**`LOFT_NO_NESTED_IN_PLACE=1`** builds and copies again, and is the first bisect step for a
wrong field, a leak or a double free in a record built from a nested literal.
**`LOFT_NO_APPEND_STAGING=1`** (loft#1548, `(E-Asgn-Compound)`) lets an appended literal's
field expressions run between the element's mint and its finish again — with it off, a field
that reads or grows its own container (`s.qs += [Q { b: g(s) }]`) is evaluated first; it is
the bisect step for a lost or overwritten element out of such an append.
