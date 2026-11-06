
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
libraries: fix, test, republish via the **loft-library skill**) are this stream's to edit, never
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
cargo run --bin loft -- prog.loft        # run  |  -- repl  |  -- introspect prog.loft  |  -- --help
loft debug prog.loft:12 [--lib dir]      # stop at line 12, read/edit the frame — INSTEAD of println (DEBUG.md)
make check-rlib                          # before a bare `cargo test`: is libloft.rlib current?
./scripts/find_problems.sh --changed     # the tight loop: the subjects your diff touches (seconds)
./scripts/find_problems.sh --subject <n> # one area; `--bg|--peek|--wait` the full suite in the background
scripts/ci-run.sh start | recheck        # ONE gate per change of unknown reach; after it, recheck — never restart
make ci                                  # the full local gate; on a loaded box the same gate on GitHub:
                                         #   gh workflow run ci.yml --ref <branch> -f os=ubuntu-latest
make after-join / make after-fix         # only the steps a join / a fix calls for (--dry-run = the plan)
make suite ARGS="@FR-B-Copy --mode both-c"  # the language suite: a new .loft test is a CELL there (TESTING.md)
make falsify GUARD=<guard.loft>          # does the guard FAIL on the build it catches? (GUARDS.md)
make work                                # the open issues to pick up;  ./scripts/idx tag:@P259 to look a ref up
scripts/join.py run [SRC …]              # join sibling branches (JOINING.md)
```

The rest — the release gate, the censuses, the perf portal and trend tools, `stats.py`, the
profiler, disk and scratch housekeeping, `derive-ci.sh`, the viewer — and the longer form of
each line above: [COMMANDS.md](doc/claude/COMMANDS.md).

**Never wait on a process by NAME** — a `pgrep -f` loop matches its own command line, and `pkill -f`
by name has killed a sibling checkout's run.  Wait on the PID or the output file:
`scripts/wait_for.sh FILE REGEX [--pid N]` always ends; `scripts/pgrep_others.sh` is the `pgrep -f`
that cannot match its caller ([CODE.md § shell](doc/claude/CODE.md)).

**Bound ad-hoc runs**: `LOFT_TIMEOUT=60 loft --native p.loft` (rustc can hang; tests arm 300 s).  A
time bound does not bound MEMORY: test runs carry a 2 GiB store-heap ceiling (`LOFT_MEMORY_LIMIT`),
and a repeat-run corruption harness caps the process too (`ulimit -v`).  Under debug assertions
`LOFT_MAX_OPS` names the loop a hang spins in — absent from the ordinary debug build
([RUN_BOUNDS.md](doc/claude/RUN_BOUNDS.md) § Store-memory ceiling, § Hang guard).

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

**A DESIGN DECISION is `@C<n>`-tagged** (@PLN175): its sites — code, doc, and at least one guard
under `tests/` — cite it; `./scripts/idx next-decision` names a new id and `make decisions` lists
what each entry still owes ([DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md)).  A decision no site can keep is
reopened, not documented.

**A FORMAL RULE is `@FR-`-tagged** (`@FR-B-Copy`, `@FR-L-Null`) and a code site that enforces one
CITES it, so "which sites enforce this rule?" is a grep: `scripts/rule_tags.py` (`sites <tag>` ·
`check` · `registers` · `pairs --unstated`).  A bare `@Name` is ambiguous here (tracker tags,
`@AAA-###`, corpus annotations); citations are boundary-exact (`@FR-B-View` ≠ `@FR-B-View-Base`),
and `B-Ref`/`D-op`/`D-own` are family prefixes, not rules — [formal/README.md § Rule
tags](doc/claude/formal/README.md), [formal/IMPLEMENTATIONS.md](doc/claude/formal/IMPLEMENTATIONS.md).

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
- **A new op or builtin renumbers `index/target_surface.json`** — rebuild the wasm rlib, then
  `make surface-gen`, or `make ci` fails a drift guard no targeted suite shows (INTERMEDIATE.md).
- stdlib load order: `01_code.loft` (operators/math/text/collections) → `02_files.loft` (I/O) →
  `03_text.loft`.
- **Before non-trivial functionality, check the library catalogue (`make libcatalogue`) + `loft install`** — don't reimplement.
  Writing/reviewing `.loft`: **loft-write skill**. Language ref: [LOFT.md](doc/claude/LOFT.md), [STDLIB.md](doc/claude/STDLIB.md).
- **A library's API: `make libcatalogue`, then the local `doc/claude/LIBRARIES.md` — NEVER a clone or
  an installed copy**, which can lag `origin/main` ([LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) § The library contract).
- **Three optimisation tiers, kept apart** (NATIVE.md § Optimisation tiers): a perf number taken on
  the semantics build (`--native`, the test runner) is not a measurement of what ships (`--ship`,
  every library cdylib: `-C opt-level=3 -C codegen-units=1`).
- **User-facing output** (anything a command PRINTS): silence when nothing needs acting
  on; no plan tags / phase names / "not yet implemented" in it; the full explanation only
  on failure. loft is meant to be BORING — noticed only in its absence
  ([GOALS.md](doc/claude/GOALS.md), [DOC_QUALITY.md § D](doc/claude/DOC_QUALITY.md)).
- **Response shape:** lead with the ONE highest-leverage item in full (decision + minimum to act),
  then a one-line summary of the rest; no long dumps.
- **Advice vs action:** asked for advice/evaluation → give the recommendation (best option + why),
  don't bounce it back. Asked a question → answer it. Act/edit only on an explicit do-it instruction.
  A decision met while working is not a multiple-choice question: take the sensible default, say
  it in one line and continue, so the owner can redirect in prose; ask only at a real fork.
  A recommended RULE or design is probed before it is recommended: build the cell that would
  falsify its premise first — answering feels like advice, so the check gets skipped, and an
  approved wrong rule is built on (`design-protocol` § When you RECOMMEND it).
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
3. **Never create a branch or open a loft PR without an explicit user ask** ("create PR",
   "switch branch"). "fix X" / "push" / "retry" are NOT such asks; a prior ask doesn't carry
   over, and "the work looks finished" is not one either — the owner decides when an arc is
   stable. If a protected branch blocks a commit, surface it and ask.
   ⚠ **A MERGE never waits:** once a PR is open, arm `gh pr merge --auto --squash`.  A
   `loft-libs-*` change is a branch + PR + auto-merge with no ask (their `main` needs a PR);
   publish only from the merged `main`.
   ⚠ **Cadence: one or two stable PRs a day**, arcs that finish inside it, each with its own
   verification complete; on RELEASE day, one PR.  Once asked, open in minutes — the PR's CI is
   the gate, so don't hold it for a local `make ci`.  A branch a sibling has joined twice is
   overdue.  The reasons, the pre-opening list and the measurements:
   [DEVELOPMENT.md § Opening a PR is the owner's call](doc/claude/DEVELOPMENT.md#opening-a-pr-is-the-owners-call).
4. **A running PR is never interrupted: nothing is pushed to it while its run is in flight.**
   Fixes go on the branch stacked on it (`<host>-pr<N>-next`), each checked by its own probe; once
   the run has FINISHED they are pushed together. The first run is discovery (its reds, main's
   nightly reds — watched with `scripts/pr_watch.py`), the second is meant to land it green; a
   later red that does not hold auto-merge waits on that branch for the NEXT PR
   ([DEVELOPMENT.md § The PR cycle](doc/claude/DEVELOPMENT.md#the-pr-cycle)).
5. **While a PR is unmerged, branch from the TIP of that in-flight work, not from `main`** — new
   work almost always needs what is still in the open PR; rebase the stack onto `main` after it
   merges.  That couples the stack to the PR's merge clock, so keep the PR mergeable.  Branch
   names: `<host>-work`, the monthly release branch `YYYY-MM`, a specific name only for a
   substantial plan.
6. **Before opening a PR and before requesting merge, verify the head is current on `origin/main`**
   (`git fetch`; `git merge-base --is-ancestor origin/main <head>`). `mergeStateStatus: BEHIND`
   merges as BLOCKED even when `mergeable=MERGEABLE` — rebase + re-push first.

## Debugging policy — MANDATORY

**Never `git bisect` or `git checkout HEAD -- <file>`** — both destroy uncommitted work. Instead:
read the failing test's dump (`tests/dumps/*.txt` — IR+bytecode+trace), narrow with
`LOFT_LOG=minimal`/`crash_tail:50`, read the 3–5 relevant files, `git show <commit>` for regressions.

**READ THE FORMAL SPEC FIRST when the fix has a choice in it** — `doc/claude/formal/` is the STRICT
definition, and *"the rules do not change to match the code; the code changes to match the
rules."*  A rule written there SETTLES a question an issue presents as open: read it when an
issue says "a design call" or "two ways to close it", before shipping a REFUSAL (a rule may say
it must work), before changing a shipped surface's semantics, and whenever the two backends
disagree.  An edge the rules cannot express means the RULE wants extending, and an "OPEN: 0" line
is a claim to re-measure ([formal/README.md § When to reach for this doc](doc/claude/formal/README.md)).

**Matrix-first for any non-trivial bug (esp. crash / silent corruption) — the urge to fix is the
signal you haven't earned it:**
1. Don't fix on the first read — a clean one-line story is a hypothesis.
2. Build the matrix in throwaway `/tmp` probes on `--interpret` (`scripts/probe-matrix`), varying ONE
   composition axis per probe, distinctive values everywhere — and **count the axes you held CONSTANT**
   with `python3 scripts/matrix_axes.py file <guard.loft>` (`cross <A> <B>` for the pairs nobody
   crosses; [DEBUG_PROBES.md](doc/claude/DEBUG_PROBES.md)).
3. **Hand-compute each cell's expected value** (agreement between two binaries is NOT a pass);
   prove the harness can fail (a no-output cell is vacuous); assert **value AND length AND leak**.
4. Map pass/fail → find the REAL boundary (filed scope is usually wrong). Resisting a read twice →
   instrument with one env-gated `eprintln`, don't theorize.
5. Fix at the chokepoint enforcing exactly the violated invariant — no narrower, no wider.
   The invariant is often already NAMED in `doc/claude/formal/` — cite the rule rather than
   re-deriving it, and close its deviation entry if it had one.
6. Verify the full matrix on **BOTH backends**: the interpreter and the C backend (`--native-c`,
   which `make`, `find_problems.sh`, `make falsify` and CI's PR gate select).  Add rustc-native
   (`--native`, `LOFT_NATIVE_BACKEND` unset) only when the change touches what it emits —
   `src/generation/`, `src/codegen_runtime.rs`, an operator's `#rust` body, a Rust native.
   Graduate guarantee probes to suite cells.
7. **Propose 3+ cases to check → write them ALL down BEFORE working the first** — the specifics of
   case three decay while you work case one.
8. **A new case found = a new probe, always** — the suite is the only thing that remembers.

Full flow: [DEBUG.md](doc/claude/DEBUG.md), [plans/_INVESTIGATION_TEMPLATE.md](doc/claude/plans/_INVESTIGATION_TEMPLATE.md),
the rules: [formal/README.md](doc/claude/formal/README.md) § When to reach for this doc.

## Bug-filing policy — MANDATORY

**Default is FIX, not file** — a bug met while fixing another is the cheapest to fix.  An open issue
is DEFERRED work and the owner does not defer: it is fixed before the release (normally before the
PR), and "ship with #N open" is never the recommendation.  Inside an INVESTIGATION plan and in
stability work, nothing is filed instead of fixed: fix it in the same session with a regression
test.  Record scope + root cause, never origin commit.

**File only what the LATEST `main` has**, and only a defect a release carries: a repro that fails
on a fresh build of `origin/main` on both backends (the body says `measured on main @ <sha>`).  A
defect of a work branch's own unmerged work, or one that came in since the last release, is fixed
and recorded in the commit, not filed
([ISSUE_TRACKING.md § File only what main has](doc/claude/ISSUE_TRACKING.md#file-only-what-the-latest-main-has)).

**Claim before you fix:** `scripts/claim.py take N`, or `scripts/claim.py file …` for a bug you find
and fix at once ([ISSUE_TRACKING.md § Claiming an issue](doc/claude/ISSUE_TRACKING.md)).

**File only when NOT fixing now** (it blocks the current task — bookmark it and work around — or it
is M+/needs a design): a GitHub Issue (`bug_report` template, never a PROBLEMS.md row) with a minimal
both-backend repro, `sev:`/`area:`, a VERIFIED `wa:*`, and a `hit-by:*` label at filing time — your
own find is `hit-by:loft`, never blank; lineage goes in the body as `Found-via: #N`.  **Add
`silent-wrong` whenever the program answers WRONG and nothing says so** (no diagnostic, refusal or
crash), or a type-system promise does not hold — it is the FREEZE axis and outranks `sev:` and
`wa:`; not for a crash, refusal, ICE, wrong message or leak
([.github/LABELS.md](.github/LABELS.md) § silent-wrong, § hit-by).

**Fixing an issue:** push the fix with `Fixes #NNN` and keep the issue open (`fixed-pending-merge`
is automated off the trailer), plus a **`Contract: settled|strained — <why>`** trailer: settled =
the formal rules already gave the answer; strained = the fix extended a rule, changed a documented
surface or needed a design call.  Absence counts as unjudged ([.github/LABELS.md §
`contract:`](.github/LABELS.md)).  Don't scope-creep the active fix with unrelated bugs.

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

**Writing or changing any doc or comment:** [DOC_CONTRACT.md](doc/claude/DOC_CONTRACT.md) is the rule
set; load the `doc-quality` skill first.  Two hard rules: history (anything dated, a past
measurement, a finished item) goes to the doc's `<doc>-history.md` companion, and no working doc
passes 1000 lines.

**Language / stdlib:** [LOFT.md](doc/claude/LOFT.md) syntax · [STDLIB.md](doc/claude/STDLIB.md) API — a LIMITATION
either states is a claim: run the guard or repro it cites before repeating it ·
[INTERFACES.md](doc/claude/INTERFACES.md) traits/generics · [TUPLES.md](doc/claude/TUPLES.md) ·
[COROUTINE.md](doc/claude/COROUTINE.md) · [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md) ·
[SUBJECTS.md](doc/claude/SUBJECTS.md) the axes loft chose on, each rationale and how it is verified ·
`tests/comparisons/*.loft` the comparison pages as programs · [bars/](doc/claude/bars/README.md):
[OCAML_BAR.md](doc/claude/bars/OCAML_BAR.md) / [LUA_BAR.md](doc/claude/bars/LUA_BAR.md) expressiveness probes (reports).

**Compiler / internals:** [COMPILER.md](doc/claude/COMPILER.md) · [INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) IR/opcodes ·
[INTERNALS.md](doc/claude/INTERNALS.md) · [SLOTS.md](doc/claude/SLOTS.md) · [NATIVE.md](doc/claude/NATIVE.md) rustc codegen ·
[C_BACKEND.md](doc/claude/C_BACKEND.md) the C backend and `c_sweep.py` · [THREADING.md](doc/claude/THREADING.md) par ·
[CODEGEN_METHOD.md](doc/claude/CODEGEN_METHOD.md) how to do compiler work.

**Runtime / memory:** [DATABASE.md](doc/claude/DATABASE.md) stores/DbRef · [REMOTE_STORES.md](doc/claude/REMOTE_STORES.md) ·
[LAZY_STORES.md](doc/claude/LAZY_STORES.md) · [LIFETIME.md](doc/claude/LIFETIME.md) · [APART_VALUES.md](doc/claude/APART_VALUES.md) ·
[OWNERSHIP_MODEL.md](doc/claude/OWNERSHIP_MODEL.md) the deps north-star · [PLACEMENT.md](doc/claude/PLACEMENT.md) where a
library runs, and the four rules for one that can be placed · [LOGGER.md](doc/claude/LOGGER.md) ·
[WASM.md](doc/claude/WASM.md) · [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) · [BROWSER_INTEROP.md](doc/claude/BROWSER_INTEROP.md) ·
[TANMATSU.md](doc/claude/TANMATSU.md) 32-bit C target · [HARDWARE_BACKENDS.md](doc/claude/HARDWARE_BACKENDS.md) ·
[GALLERY_CI.md](doc/claude/GALLERY_CI.md) · [WINDOWS.md](doc/claude/WINDOWS.md) / [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md).
Networked runs: `LOFT_NET_PROFILE` (PERFORMANCE.md § LOFT_NET_PROFILE).

**Diagnostics:** [DIAGNOSTICS.md](doc/claude/DIAGNOSTICS.md) the code index — a code is a frozen surface ·
[WARNING_FIXES.md](doc/claude/WARNING_FIXES.md) the construction that clears each warning.

**Testing / debug:** [TESTING.md](doc/claude/TESTING.md) where a test goes (start here) ·
[RUNNING_TESTS.md](doc/claude/RUNNING_TESTS.md) · [GUARDS.md](doc/claude/GUARDS.md) `make falsify` ·
[RUN_BOUNDS.md](doc/claude/RUN_BOUNDS.md) · [TEST_ENVIRONMENTS.md](doc/claude/TEST_ENVIRONMENTS.md) ·
[LOFT_TEST.md](doc/claude/LOFT_TEST.md) · [DEBUG.md](doc/claude/DEBUG.md) · [CAVEATS.md](doc/claude/CAVEATS.md) · bisect switches:
[NATIVE_SWITCHES.md](doc/claude/NATIVE_SWITCHES.md) / [BOTH_BACKEND_SWITCHES.md](doc/claude/BOTH_BACKEND_SWITCHES.md) ·
[PROFILING.md](doc/claude/PROFILING.md) · [PERFORMANCE.md](doc/claude/PERFORMANCE.md) / [PROFILE_ORACLE.md](doc/claude/PROFILE_ORACLE.md) ·
[INTERPRETER_PERFORMANCE.md](doc/claude/INTERPRETER_PERFORMANCE.md) · [PERF_PORTAL.md](doc/claude/PERF_PORTAL.md) (generated) ·
[KERNELS.md](doc/claude/KERNELS.md) the Rust stand-ins register · [CI_BUDGET.md](doc/claude/CI_BUDGET.md) what runs when.

**Quality / stability / formal:** [CODE.md](doc/claude/CODE.md) · [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) ·
[QUALITY.md](doc/claude/QUALITY.md) · [GOALS.md](doc/claude/GOALS.md) · [BUS_FACTOR.md](doc/claude/BUS_FACTOR.md) ·
[STRONG_POINTS.md](doc/claude/STRONG_POINTS.md) · [CONTROL.md](doc/claude/CONTROL.md) · [DESIGN.md](doc/claude/DESIGN.md) ·
[DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) · [DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) /
[DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) · [FORMATTER.md](doc/claude/FORMATTER.md) · stability:
[STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md) · [STABILITY_METHOD.md](doc/claude/STABILITY_METHOD.md) (§ The rule-led
walk, `make rule-coverage`, `make rule-depth`) / [RULE_CROSSINGS.md](doc/claude/RULE_CROSSINGS.md) the rule pairs
already verified, which `rule_tags.py pairs` skips / [_SWEEP](doc/claude/STABILITY_SWEEP.md) /
[_HOTSPOTS](doc/claude/STABILITY_HOTSPOTS.md) / [_REDFLAGS](doc/claude/STABILITY_REDFLAGS.md) ·
[BRITTLE.md](doc/claude/BRITTLE.md) · [DEPS_INVENTORY.md](doc/claude/DEPS_INVENTORY.md) · formal lens:
[FORMALIZATION.md](doc/claude/FORMALIZATION.md) / [TYPING_RELATION.md](doc/claude/TYPING_RELATION.md) · strict:
[formal/README.md](doc/claude/formal/README.md).

**Plans / roadmap:** [plans/README.md](doc/claude/plans/README.md) · [PLANNING.md](doc/claude/PLANNING.md) ·
[ROADMAP.md](doc/claude/ROADMAP.md) · [BROADENING.md](doc/claude/BROADENING.md) · [WEB_STACK.md](doc/claude/WEB_STACK.md)
the better-PHP design · [lib_plans/README.md](doc/claude/lib_plans/README.md) (legacy) ·
[STACKTRACE.md](doc/claude/STACKTRACE.md) · [SANDBOX.md](doc/claude/SANDBOX.md).

**Libraries / registry / packages:** any library work starts at [LIBRARY_AUTHORING.md § The library
contract](doc/claude/LIBRARY_AUTHORING.md); publishing is the `loft-library` skill ·
[LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md) · [THIRDPARTY.md](doc/claude/THIRDPARTY.md) third-party code we ship · a library's API: `LIBRARIES.md` (`make libcatalogue`) ·
[LIBRARY_BRANCHES.md](doc/claude/LIBRARY_BRANCHES.md) · [PACKAGES.md](doc/claude/PACKAGES.md) · [API_SURFACE.md](doc/claude/API_SURFACE.md) ·
[REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) / [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) /
[REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) · [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) · [REPL.md](doc/claude/REPL.md).

**Process / issues / release:** [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) workflow (start here) ·
[JOINING.md](doc/claude/JOINING.md) · [REVALIDATE_LIBS.md](doc/claude/REVALIDATE_LIBS.md) ·
[ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) / [PROBLEMS.md](doc/claude/PROBLEMS.md) · [BUG_REVIEW.md](doc/claude/BUG_REVIEW.md) ·
[.github/LABELS.md](.github/LABELS.md) · [RELEASE.md](doc/claude/RELEASE.md) (start here) /
[RELEASE_PUBLISH.md](doc/claude/RELEASE_PUBLISH.md) / [releases/](doc/claude/releases/README.md) ·
[LIBRARY_DOC_REVIEW.md](doc/claude/LIBRARY_DOC_REVIEW.md) · [SKILLS_REVIEW.md](doc/claude/SKILLS_REVIEW.md) ·
[COMPATIBILITY.md](doc/claude/COMPATIBILITY.md) · [MOVING.md](doc/claude/MOVING.md) · [CHANGELOG.md](CHANGELOG.md) /
[CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) · [DOC.md](doc/claude/DOC.md) · [USER_DOCS.md](doc/claude/USER_DOCS.md) ·
[LAVITION.md](doc/claude/LAVITION.md) · [PROMPTS.md](doc/PROMPTS.md) · the owner's notes live OUTSIDE git
(`$LOFT_NOTES`): read them when a feature is in question, never commit or publish from them ·
posts: [loft-lang/blog](https://github.com/loft-lang/blog), whose `CLAUDE.md` says how one is made.

**Skills** (`.claude/skills/`): `loft-write` · `loft-debug` · `loft-test` · `loft-codegen` · `loft-optimize` ·
`loft-library` · `engineering-rigor` / `design-protocol` · `doc-quality` · `draw`
([DRAWING.md](doc/claude/DRAWING.md)) · `loft-plan-workflow`.

## Environment switches

Every `LOFT_*` switch has ONE home doc.

- **Dumps:** `LOFT_LOG=full|static|minimal|crash_tail:N|fn:<name>|type_timeline:<var>|…` before
  `cargo test` (writes `tests/dumps/*.txt`) or `cargo run` (stderr) —
  [RUNNING_TESTS.md § LogConfig](doc/claude/RUNNING_TESTS.md#logconfig--debug-logging-framework);
  `LOFT_VAR_TABLE=<fn>` a function's variable table with its deps resolved
  ([DEBUG_TRACE.md](doc/claude/DEBUG_TRACE.md)); `LOFT_STRICT_SCHEMA_IDS=1` / `LOFT_TRACE_MINT=1` for a
  `--native` wrong-type fault ([NATIVE.md](doc/claude/NATIVE.md) § Architecture).
- **Two diagnostic tiers:** a diagnostic GATES (`warning`, `LOFT_DENY_WARNINGS=1`) if and only if
  ignoring it can produce a wrong result; everything else is `advice`, which never gates.  And a
  diagnostic REACHES only whoever can act on its cure — enforced once, in
  `Diagnostics::add_at_coded`, so a lint site adds no ownership test of its own
  ([DIAGNOSTICS.md](doc/claude/DIAGNOSTICS.md) § Diagnostic tiers, § Who a diagnostic is addressed to).
- **By family:** lint opt-outs and `LOFT_ERRORS` — [DIAGNOSTICS.md § Switching a diagnostic
  off](doc/claude/DIAGNOSTICS.md) · profilers — [PROFILING.md](doc/claude/PROFILING.md) (prefer `make profile`) ·
  `--native` rewrites, each with a `LOFT_NO_*` switch as the first bisect step —
  [NATIVE_SWITCHES.md](doc/claude/NATIVE_SWITCHES.md) · both-backend lowering and store policies, where
  the two backends agreeing is no evidence — [BOTH_BACKEND_SWITCHES.md](doc/claude/BOTH_BACKEND_SWITCHES.md).
