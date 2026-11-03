<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Commands — the full reference

Every `make` target, script and tool an agent working on loft reaches for, with what it does
and when to use it.  `CLAUDE.md` § Key commands keeps the everyday ones; this is the rest, and
the longer form of those.  The bounds every run carries are [RUN_BOUNDS.md](RUN_BOUNDS.md); how
to wait on a run is [CODE.md § shell](CODE.md).

```bash
cargo run --bin loft -- prog.loft        # run     |  -- repl  |  -- introspect prog.loft  |  -- --help
loft debug prog.loft:12 [--lib dir]      # STOP at line 12: read/edit the live frame, step
                                         #   (pipe commands on stdin; `--rpc` = scripted NDJSON)
                                         #   reach for this INSTEAD of adding println — DEBUG.md
make doc                                 # regenerate doc/*.html: fetches the library packages,
                                         #   then gendoc.  A bare `cargo run --bin gendoc` on a box
                                         #   without them keeps the committed lib pages
make ci                                  # fmt → clippy → test (full local gate).  Unreliable
                                         #   here (memory, a sibling gate)?  Run the SAME gate
                                         #   on GitHub, no PR needed: `gh workflow run ci.yml
                                         #   --ref <branch> -f os=ubuntu-latest` — CI_BUDGET.md
                                         #   § When the local gate is unreliable.  Either gate
                                         #   VALIDATES; never re-run one for a green notice once
                                         #   its failures are fixed and checked — the PR's CI is
                                         #   the verdict (CI_BUDGET.md § After a red gate)
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
                                         #   two clippy variants → ONE gate per change whose
                                         #   reach you cannot bound, via `scripts/ci-run.sh
                                         #   start` (a ~15 s pre-flight — fmt, audit rows, doc
                                         #   drift — refuses a gate that would stop on them).
                                         #   AFTER a gate, red or not: `ci-run.sh recheck` —
                                         #   its failed tests + what changed since — NOT a
                                         #   restart; the push names gate + recheck.  A new
                                         #   gate only for a change of unknown reach (a new
                                         #   refusal, inference, a new op) — CI_BUDGET.md § After
                                         #   a red gate.  The gate runs your diff's subjects
                                         #   FIRST and QUEUES behind another checkout's (one at a
                                         #   time on this box, LOFT_GATE_PARALLEL=1 beside it);
                                         #   ~20 min, twice that with two gates live.
./scripts/find_problems.sh --bg|--peek|--wait   # background full-suite run + inspect/block
make after-join / make after-fix         # WHAT A CHANGE NEEDS, AND NOTHING MORE: reads the
                                         #   changed files from git (a join: what the join
                                         #   added; a fix: since the last green run here) and
                                         #   runs only the steps they call for — fmt, the lint
                                         #   gates, clippy, the ratchets, the changed guards on
                                         #   both backends, the diff's test subjects (a fix) —
                                         #   on the DEV binary, each with its reason and its
                                         #   seconds; what needs a release build or a runner is
                                         #   REPORTED as the dispatch it needs (`--dispatch`
                                         #   sends them).  `--dry-run` is the plan.  Every
                                         #   step stays inside twenty minutes; a release build
                                         #   here is never part of it — JOINING.md § The script
cargo test --release --test ir_schema_roundtrip   # the IR codec over the whole stdlib + every
                                         #   tests/scripts file + every program of the language
                                         #   suite beside the tree (../loft-suite, LOFT_SUITE_DIR);
                                         #   run it after an IR-schema/serialiser change
                                         #   (`make ci` runs it in the pool)
make suite ARGS="@FR-B-Copy --mode both-c" # the LANGUAGE SUITE (loft-lang/loft-suite, beside
                                         #   this tree): one rule's cells on the interpreter and
                                         #   the C backend (`all` adds rustc-native), a chapter
                                         #   (`--chapter heap`), or everything.  A new .loft test
                                         #   is a CELL there, under its rule; `tests/scripts`
                                         #   keeps only what this repo names — TESTING.md.  Every
                                         #   corpus SWEEP (round-trip, leak scan, valgrind, switch
                                         #   A/B, wasm, axes, …) walks the suite too, through
                                         #   scripts/corpus_files.sh · scripts/corpus.py ·
                                         #   tests/common/corpus.rs — the one list
make release-gate                        # every nightly against THIS commit in ONE CI run, one
                                         #   verdict — the release evidence (`release-checklist`
                                         #   reads it by HEAD's sha).  The 03:00 daily starts
                                         #   03:34–14:45 UTC on whatever main was; this does not
                                         #   excuse a red nightly — fix those the day they appear
make falsify GUARD=<guard.loft> [REF=<commit>]  # does this guard FAIL on the build it was
                                         #   written to catch?  Compares exit/asserts/leak/
                                         #   panic apart and names the channel that moved.
                                         #   Every new tests/scripts file records its answer
                                         #   (`@falsified-at:`, gated) — GUARDS.md.  Without
                                         #   REF the control is the INSTALLED main, no build:
                                         #   `make install-main` once per main change, and
                                         #   score BEFORE the branch is joined (once main has
                                         #   the fix, REF= and a full build per control)
make speed                               # what got slower/faster — a REPORT, never a gate
make speed-gate                          # the one speed GATE: a test 3x and +5 s slower than on
                                         #   main (median speed divided out) that is STILL that slow
                                         #   rerun alone.  Runs in `make ci` and on every Linux PR leg;
                                         #   contention never repeats alone — scripts/test_speed_gate.py
make rewrite-census                      # which REWRITES still fire where they did: per bench
                                         #   and per compiled writer (Rust, C), the admissions of
                                         #   each R- rule against a committed baseline — a DROP (a
                                         #   fix tightened a condition) fails, named by rule, bench
                                         #   and writer, and so does a rule applied by another set of
                                         #   phases (`scripts/rule_phases.py`); seconds, exact, in `make ci`
                                         #   and on the PR.  `-bless` records a deliberate decline
make decline-census                      # HOW FAR EACH REWRITE REACHES: per R- rule, the sites it
                                         #   admitted against the sites it REFUSED, the refusals
                                         #   ranked by reason — a routine over its bar is a shape a
                                         #   rewrite declined, and the top reason of a rule that
                                         #   matters is the next arc, by yield.  Rules without
                                         #   decline reporting read `not reported`, never covered.
                                         #   A REPORT (`ARGS="--record"` keeps the snapshot)
make perf-portal                         # WHERE NATIVE STANDS AGAINST RUST, BY CLASS: measures
                                         #   every bench lane here and renders ONE page,
                                         #   doc/claude/PERF_PORTAL.md — a median per mechanism
                                         #   class (keyed, vector-build, call, record-field, …),
                                         #   every routine under its class, and the surveyed
                                         #   library routines still waiting for a row (@PLN158).
                                         #   `perf-portal-render` re-renders from saved runs;
                                         #   PACKAGES="--package <scratch clone>/drawing=drawing"
                                         #   adds a library's own bench; ARGS="--lanes
                                         #   native,rust,ship" adds a SHIP column per routine:
                                         #   the program as a game's release build would be
                                         #   compiled, fault checks dropped (@PLN201);
                                         #   `,c,cship` adds the C99 backend checked and
                                         #   probed against the same twin, its hash
                                         #   VALIDATED (bench/README.md § The C lanes).  A
                                         #   REPORT, never a gate
make perf-check [ARGS="--only 16,17"]     # ONCE PER ARC, IN THE BACKGROUND, never a foreground
                                         #   wait — a commit needs the matrix, the pins, clippy
                                         #   and one targeted A/B on its own row; CI_BUDGET.md
                                         #   § What a compiler change needs.  It measures the
                                         #   lanes the census says the change touched (or
                                         #   ARGS' lanes) and compares each routine's ratio to
                                         #   Rust with this machine's last committed row
                                         #   (bench/portal/results/<host>.tsv); a move ≥ 15 %
                                         #   is listed, a slowdown exits 1.  `--record` makes
                                         #   the run the new baseline — commit it with the
                                         #   change, that file IS the trend history
make perf-trend [ARGS="--routine ease"]   # how each routine MOVED over the committed portal
                                         #   history: the movers between their last two
                                         #   measurements, or one routine's whole series with
                                         #   the commits each point sits between (the bisect
                                         #   range).  A report; ratios compare within one
                                         #   machine only
make perf-local [ARGS="13/split"]        # every measurement THIS checkout took — each stats.py
                                         #   run, full or partial, per routine, tagged by
                                         #   --subject; hand prices via `local.py add`.  Local,
                                         #   never committed — bench/README.md § Local results
python3 bench/stats.py                   # native vs the Rust reference per bench routine, as
                                         #   STATISTICS: --n calibrated per lane, a warm-up
                                         #   round dropped, 7 interleaved samples pinned to the
                                         #   fastest core, the ratio with its RANGE and an
                                         #   ok / OVER / unclear verdict against --bar (2.0);
                                         #   hashes must agree across lanes.  `--tsv` keeps a
                                         #   run to diff against the next — bench/README.md
make interp-gap [ARGS="--only 14"]       # INTERPRETER vs NATIVE per bench routine: the
                                         #   ratio, what the interpreter executes (ops by
                                         #   family, bytes copied) and the native-only
                                         #   rewrites in those functions — the candidates to
                                         #   move into the IR phase.  A report, by hand
                                         #   (bench/README.md § Interpreter against native)
make ops-census                          # which bytecode operators anything still EMITS:
                                         #   live / unexercised (src/ names it, no program
                                         #   emits it — a test gap or a leftover reader) / orphan (nothing emits it and
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
make disk-headroom                       # ROOM FOR A GATE: every gate runs this first; by hand
                                         #   when a build says `No space left on device`.  This
                                         #   checkout's scratch only, never a sibling's live run
make janitor-install                     # once per box: scripts/disk_janitor.sh runs, detached,
                                         #   beside every agent build (a user-level hook) and
                                         #   beside find_problems/stats.py — every checkout's
                                         #   stale artefacts, under cargo's build lock, and what
                                         #   git IGNORES there once nothing wrote it for 3 days
                                         #   (probe caches, scratch builds, finished agent
                                         #   worktrees).  Keep an artefact with a line in
                                         #   scripts/janitor_keep.txt; `scripts/sweep_ignored.sh
                                         #   --dry-run .` shows what would go — RUN_BOUNDS.md
make sweep-scratch                       # reclaim loft's temp scratch (dead-process native
                                         #   artefacts, aged test caches, old agent sessions);
                                         #   `df -h /` before a gate — a full disk fails the
                                         #   NATIVE corpus with a code-shaped message
scripts/join.py run [SRC …]              # JOIN sibling branches: survey what this tree lacks
                                         #   (by content, not ancestry), pick source by source,
                                         #   re-derive every artefact in scripts/derived_
                                         #   artefacts.json, verify — stops at each decision.
                                         #   Steps one by one: survey/apply/rederive/verify/
                                         #   guards — JOINING.md § The script
scripts/derive-ci.sh                     # the PURE derived artefacts (compiled stdlib, surface,
                                         #   browser bundle, doc pages, …) regenerated ON A
                                         #   RUNNER and committed back; ratchets stay local —
                                         #   JOINING.md § Derived artefacts on a runner
make view                                # branch-aware doc/code viewer; binds LOOPBACK,
                                         #   LOFT_VIEW_PORT (default 8765).  Remote:
                                         #   ssh -N -L 8765:127.0.0.1:8765 <host>
```
