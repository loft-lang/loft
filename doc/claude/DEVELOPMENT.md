
# Development Workflow

How a change travels from a working tree to `main`: the branch it lives on, how it is cut into
commits, what each commit must pass, and what reaches the owner.  The hard rules this workflow
runs under — branch policy, debugging, bug filing, git safety — are CLAUDE.md's, and this doc
points at them rather than restating them.

**Session start:** read [CLAUDE.md](../../CLAUDE.md) at the project root.

**Who develops loft:** almost entirely AI coding agents, steered by the owner, who put
documentation and tooling above code.  Everything needed to work on loft is in this repo —
[BUS_FACTOR.md](BUS_FACTOR.md).  The incidents behind the rules below are kept in
[DEVELOPMENT-history.md](DEVELOPMENT-history.md).

---

## Contents
- [Branches](#branches) — [Stay close to `main`](#stay-close-to-main--rebase-rigorously) ·
  [Opening a PR is the owner's call](#opening-a-pr-is-the-owners-call)
- [Working on a change](#working-on-a-change) —
  [Inserting Discovered Enhancements](#inserting-discovered-enhancements-into-the-active-plan) ·
  [Where a rule's letter and the code disagree](#where-a-rules-letter-and-the-code-disagree) ·
  [Bug-filing During a Hunt](#bug-filing-during-a-hunt--mandatory)
- [Commits](#commits) — [Documentation updates](#documentation-updates) ·
  [Moving a doc](#moving-a-doc-and-repairing-links)
- [CI Validation](#ci-validation) — [Local gate](#local-ci-gate) ·
  [Common pitfalls](#common-pitfalls) · [Remote CI](#remote-ci)
- [GitHub Issues and Releases — Hard Limits](#github-issues-and-releases--hard-limits)
- [Closed-by-Decision Register](#closed-by-decision-register)
- Elsewhere: joining branches — [JOINING.md](JOINING.md); does a change break a shipped
  library — [REVALIDATE_LIBS.md](REVALIDATE_LIBS.md); adding a bytecode operator —
  [INTERMEDIATE.md § Adding an operator](INTERMEDIATE.md#adding-an-operator)

---

## Branches

The rules are [CLAUDE.md § Branch policy](../../CLAUDE.md): never commit on `main`; push to
the feature branch once a change settles; never create a branch or open or merge a PR without
an explicit ask; branch from the TIP of unmerged in-flight work, not from `main`; and verify
the head is current on `origin/main` before opening a PR.  Branch names follow its rule 5: a
general `<host>-work`, the monthly release branch `YYYY-MM`, and a specific name only for a
substantial plan.

**The working branch is ONE accumulating unit.**  It carries everything worked on in the
cycle: the feature, and every bug found along the way.
- **Fix bugs on sight.**  A bug met while doing other work (paths warm, repro cheap) is fixed
  on the same branch, not filed and deferred.
- **PR the whole branch, not a slice.**  Asked to "PR the <feature>", the PR is the entire
  branch as one review unit; do not cherry-pick a subset onto a fresh branch.
- **Bundle many subjects into one PR**, even unrelated ones: a docs stream, a soundness fix and
  a language feature ride together.  Split only when the owner wants an independent merge
  clock for one item.

**Renaming a branch** whose name has drifted from its work: `git branch -m <from> <to>` before
the PR is opened.  Replacing an already-pushed name on the remote needs the user's instruction.

### Stay close to `main` — rebase rigorously

Build everything on the ONE working branch — mixed topics are fine — and rebase it on
`origin/main` after every merge into `main`, not once at the end:

1. **You keep track of the work.**  One branch is one whole; many branches scatter it.
2. **Overlapping work merges cleanly.**  Small, continuous reconciliation instead of one large
   collision on a diverged base.
3. **`git diff main` is a compass.**  Close to `main`, it shows exactly what you changed, and
   `git diff main -- f` / `git show origin/main:f` give a clean baseline for any file.  A
   diverged branch buries both under merge noise.
4. **The compass survives others' work.**  After a rebase, the diff is still your delta on the
   current base, with their fixes absorbed.

A long-lived branch that skips this ends with squash-merged duplicates of its own commits that
git cannot drop, and the only way out is to cherry-pick the genuine delta onto a fresh `main`.
The mechanics of rebasing onto a squash and of joining sibling checkouts are
[JOINING.md](JOINING.md).

### Opening a PR is the owner's call

Do not propose a PR, hint that work is ready for one, or treat a finished issue as a milestone
that wants one.  Fix, gate, push, and say what is done.  A PR is a GATHERING: when the work
across the agents' active branches is stabilising, the owner asks for it, and it joins those
branches in one.  Nothing that lands on one branch — a fix, a feature, a workflow that only
works from `main` — is a reason to mention one.  The cadence — one or two stable PRs a
day, arcs that finish inside it — is [CLAUDE.md § Branch policy](../../CLAUDE.md) rule 3, with
the short list of what must hold before opening.

The two costs it balances:
- **A PR serialises the stream.**  GitHub has no stacked PRs, so a second PR branches off the
  first one's tip and waits for its merge.  Hence never one PR per issue.
- **Withholding a PR serialises it too**, through joins: every sibling re-joins the branch and
  re-derives its measured rows, and a join-only defect exists only on the union.  `main` is
  the only place a resolution is shared once.

They reconcile on time-to-MERGE: a PR is cheap exactly when it lands promptly.  So once the
owner asks, opening takes minutes — the PR's own `ci.yml` runs the same gate on the exact sha,
so the opening is not held for a local `make ci`.  **A branch a sibling has joined twice is
overdue**; that is a fact to report, not a request to make.

A language change can retro-break a shipped library while every branch gate stays green,
because the library gate runs only on PRs and on `main`: run
[REVALIDATE_LIBS.md](REVALIDATE_LIBS.md)'s script from the branch.

---

## Working on a change

### Announce each step

State the name of every step as you start or finish it, with the issue or item ID when one
exists: *"Starting #1703: reducing the repro"*, *"Finished: clippy clean, both variants"*.  The
user sees text output, not tool calls; a running status line lets them interrupt early when
the plan is wrong and resume efficiently when context runs out.

### Splitting a large change

A change too large to review as one commit is split into steps that each:
1. pass `make ci` on their own;
2. carry at least one test that exercises the step;
3. leave the tree better or equal — no regression, no dead path, no half-working feature a
   loft program can see.

Split at natural seams: independent areas of the code (a parser change, a runtime change),
phases of a design, a guard before the full fix, the common case before the edge cases.  A
commit should rarely exceed about 200 lines of non-test Rust.  For a plan, write the steps into
the plan before the first commit lands; if you cannot write a test for a step, the boundary is
wrong.  Plan phases: the **loft-plan-workflow** skill.

### Inserting Discovered Enhancements Into the Active Plan

Building a real consumer surfaces gaps in the language and stdlib that toy programs never hit.
At the moment of discovery there are three choices:

1. **Work around it in the consumer**, with a comment naming the gap's home.
2. **Route it to its canonical home** and keep building.
3. **Insert a step into the active plan that fixes the gap**, then resume the feature work.

**Default to (3) when the fix is XS or S** (under half a day): the fix becomes a step of the
plan's CURRENT phase, landed before the next phase opens.  You already understand the site,
and the compiler or stdlib code is one or two edits away.

Use (1) + (2) instead when the fix needs design (typer architecture, a multi-file refactor, an
ABI break), is M+ and would push the phase past its budget, or changes observable language
semantics (that needs a design and a migration of its own).  Do not let a small fix grow into
an unrelated subsystem's arc.

#### How items are documented (one canonical home each, no duplicates)

A gap that is not fixed now goes to its ONE home; never invent a parallel catalogue.  The full
split is [ISSUE_TRACKING.md § The split](ISSUE_TRACKING.md#the-split--what-lives-where); for
discovered gaps:

| Item | Home |
|---|---|
| **Bug** — observable wrong behaviour, a panic, a rejection | a GitHub issue in `loft-lang/loft` ([CLAUDE.md § Bug-filing policy](../../CLAUDE.md)) |
| **Stdlib gap** that fits the existing API surface | a row in [STDLIB.md § Open work](STDLIB.md#open-work) |
| **Native codegen follow-up** | a row in [NATIVE.md § Open work](NATIVE.md#open-work) |
| **Known tradeoff**, not a defect | [QUALITY.md](QUALITY.md) |
| **Feature or infra catalogue entry** | a `loft-lang/features` issue |
| **New library, or an M+ feature** needing its own design | a `loft-lang/plans` issue (`@PLN<n>`) |

**The workaround comment is mandatory** whichever choice you make, and names the home so it
explains itself.  It is removed in the commit that ships the fix.

```loft
// `s[i] ?? '<char>'` fails to compile on native (loft#<N>); the
// surrounding `i < n` guard already keeps `i` in range, so read it bare.
c = line[i];
```

#### Schedule-to-fix lives in the active plan

The canonical home holds the design, the details and the reproducer.  The active plan holds
the SCHEDULE: a step row naming the home, the files to touch and the test.  Without that row a
home entry can wait indefinitely.  A gap found mid-phase gets both: filed in its home, then a
step appended to the SAME phase.  A gap too big for one step gets its own plan issue and a
tracking row that closes when that plan ships its first phase.

### Where a rule's letter and the code disagree

**Apply the strict reading of the written rule, without asking** (the owner's standing
ruling).  The formal doctrine already says the code changes to match the rules
([formal/README.md](formal/README.md)).  When a fix meets a rule whose letter says one thing and
the code another, apply the letter, land the fix with its verification, and add the trailer
`Contract: strained — letter applied: <rule> <what moved>`.  The owner reads these trailers in
one sitting; disagreeing is a revert of a commit that names what it changed.

**Only three kinds of question reach the owner:**
1. **a rule that does not exist yet** — the letter is silent, so there is nothing to apply;
2. **a change to a shipped surface** — a refusal, a spelling, an observable behaviour a
   published program may depend on ([COMPATIBILITY.md](COMPATIBILITY.md));
3. **two rules in conflict** — the letters disagree with each other, not only with the code.

Everything else is the agent's to decide.  A question the rules already answer, asked anyway,
spends the owner's attention, which is meant for the engine
([STABILITY_ROADMAP.md § The owner's directive](STABILITY_ROADMAP.md)).

### Validation against CODE.md

Before committing, check new code against [CODE.md](CODE.md):

| Check | How | Note |
|---|---|---|
| No clippy warnings | both variants: `cargo clippy -- -D warnings`, `cargo clippy --all-targets --all-features -- -D warnings` | a pre-existing lint in code you did not write is still yours to clear — [Pre-existing vs. newly-introduced failures](#pre-existing-vs-newly-introduced-failures--always-irrelevant) |
| Formatted | `cargo fmt`, then `cargo fmt -- --check` | |
| Naming | by hand | `n_<name>` for global natives; `t_<LEN><Type>_<method>` for methods |
| Function length | clippy | a function you change loses its `inherited` exemption: split it in its own commit, or write the real reason — [CODE.md § Functions](CODE.md#functions) |
| Null sentinels | by hand | a numeric function returning null uses `i32::MIN` / `i64::MIN` / `f64::NAN`, never `0` |

A refactor of a function a feature merely touched goes in its own behaviour-neutral commit, so
the feature's diff stays reviewable.

### Debugging a regression

The rules are [CLAUDE.md § Debugging policy](../../CLAUDE.md): never `git bisect`, never
`git checkout HEAD -- <file>`, matrix first for anything non-trivial.  The flow is
[DEBUG.md](DEBUG.md).  In short:

1. Write a minimal `.loft` reproducer in `tests/scripts/` with `fn test_*()` entry points;
   mark a known failure `// @EXPECT_FAIL: <message>` (a compile error: `// @EXPECT_ERROR:`)
   so the suite stays green while you work.
2. Read the dump: `LOFT_LOG=minimal cargo test --test <suite> <name>`, then
   `tests/dumps/<name>.txt` (IR, bytecode, trace); `LOFT_LOG=crash_tail:50` for the steps
   before a panic.  Stop at a line with `loft debug prog.loft:<line>` instead of adding prints.
3. Read the 3–5 source files the trace names; for what a recent commit changed, `git show
   <sha>` — read the diff, never re-run old code.
4. Fix forward, then remove the annotation: the runner prints
   `FIXED <path>::<name> (was @EXPECT_FAIL, now passes)` for a function that passes.

### Bug-filing During a Hunt — MANDATORY

The policy is [CLAUDE.md § Bug-filing policy](../../CLAUDE.md): **default is FIX, not file**;
file a GitHub issue only when the finding blocks the current fix or needs design, with a
both-backend repro and its labels; inside an investigation plan, never file.  A bug you just
fixed gets a regression test and no issue.  Filing is not a licence to grow the active fix:
bundle a second bug into it only when both share one fix site.

A survey that finds the siblings of the bug you are on:
- `grep -E "(workaround|caveat|but the|but is|currently ignored|FIXME|TODO)"` over the diff and
  the surrounding files finds self-flagged latent bugs;
- variant probes (another left-hand side, another scope, another element type) find sibling
  shapes — `python3 scripts/matrix_axes.py file <guard.loft>` names the axes you held fixed;
- comparing the fix with every symmetric path finds the one that did not get it.

---

## Commits

**Every commit passes the local gate** ([Local CI gate](#local-ci-gate)); a branch may hold any
number of them.  Each is one coherent change:

- a fix and its guard ship together; a guard records `@falsified-at:` — the run showing it
  fails on the build it was written to catch ([GUARDS.md](GUARDS.md), `make falsify`);
- a test written before its implementation, in a separate commit, is marked
  `#[ignore = "<reason>"]` or `@EXPECT_FAIL` and enabled in the commit that makes it pass;
- a behaviour-neutral refactor gets its own commit;
- documentation gets its own commit, on the same branch ([Documentation updates](#documentation-updates)).

When one branch carries several items, or several phases of one item, each gets its own
commits, so the history pins a change to one item.

### Commit message style

The **subject** is one sentence saying what is now true, in the present tense — *"A vector
copied into an empty destination claims its length, so a builder written in its element keeps
the copy's size"* — not the edit made (*"fix vector_add"*).  The **body** gives the cause, the
fix, the guard and how it was falsified; name a function only when it is the thing changed, and
never list the files touched (the diff shows them).  **Trailers:** `Fixes #N` with a
`Contract: settled|strained — <why>` line beside it ([CLAUDE.md § Bug-filing policy](../../CLAUDE.md)),
then the co-author line the session names.  `make hooks` installs the `commit-msg` hook that
reports an issue mentioned without its `Fixes` trailer.

### Documentation updates

Documentation ships in the same branch as the code it describes, in its own commit — never a
separate docs branch or PR, and a small edit gets no PR of its own.  Skip rows the change
clearly does not affect:

| Document | Update when… |
|---|---|
| `doc/claude/CHANGELOG_TECHNICAL.md` | Always — a detailed entry under `## [Unreleased]` |
| `CHANGELOG.md` | The change is user-visible — a plain-language entry under the current `## YYYY-MM` |
| `doc/claude/PLANNING.md`, `ROADMAP.md` | An item completed (remove it — completion history is git and the changelog) or a new one was found |
| A GitHub issue | A bug found and NOT fixed ([ISSUE_TRACKING.md](ISSUE_TRACKING.md)); a fixed one gets `Fixes #N` in its commit |
| `doc/claude/CAVEATS.md` | An edge case was fixed, or a workaround found (with its test) |
| `doc/claude/TESTING.md` § Open work | Coverage improved, or a gap was found |
| `doc/claude/STDLIB.md` / `LOFT.md` | A stdlib function, or the language's syntax or semantics, changed |
| `doc/claude/INTERNALS.md` / `INTERMEDIATE.md` | A new operator, runtime state or native function |
| `doc/claude/INCONSISTENCIES.md` / `NATIVE.md` | A language quirk was found or resolved; a native design correction |
| `README.md` | A new user-facing feature, command or example |
| The feature's design doc or plan | The implementation diverged from the design, or a phase completed |
| `.claude/skills/loft-write/SKILL.md` | A new pattern, caveat or convention for writing `.loft` |

Then grep `doc/claude/` for the feature's key nouns and update what else describes it.  The
rules every doc edit follows are [DOC_CONTRACT.md](DOC_CONTRACT.md); the edit hook reports what
a script can check.

### Moving a doc, and repairing links

**Move a file or directory with `make plan-move FROM=<path> TO=<path>`, never a bare
`git mv`.**  The rename decides every link it breaks, so the command rewrites them as it moves:
links into the moved path from anywhere in the tree, links out of it from its new depth, and
repo-rooted spellings of the old path in code, tests and scripts.  It then rebuilds the index
and runs the drift checker.  `python3 tools/indexer/rewrite_links.py FROM TO` is the dry run.

**`make doc-fix` repairs links that are already broken.**  A link is repaired only when
exactly one tracked path ends in every segment the link names.  Anything else is printed as a
`flag` for a person: a target that has left the tree (a library now in a `loft-libs-*` repo
wants that repo's URL) or a name several files share.
A `#fragment` into a markdown file must name an anchor it renders — a heading's slug as
GitHub computes it (`-1`, `-2` for repeats), an `id=`/`name=` attribute, or a heading's
`{#id}` — and a dead one is always a `flag`: a renamed heading has no successor a tool can
pick, so point it at the heading that holds the content now or drop the fragment.
`tests/doc_hygiene.rs::every_markdown_link_resolves` fails on any `fix` or `flag`, and
`the_link_check_answers_its_known_cases` runs the tool's `--self-test`.  Not links: fenced
blocks (closed the CommonMark way — a bare fence of the same character, at least as long),
inline code spans, `<placeholder>` targets, a line marked `<!--noindex-->`, and
`tests/fixtures/`.

### Pushing `.github/workflows/` changes — use the SSH remote

GitHub rejects a push that creates or updates a file under `.github/workflows/` when the
credential is an OAuth-app token without `workflow` scope:

```
! [remote rejected]  <branch> -> <branch>
  (refusing to allow an OAuth App to create or update workflow
   `.github/workflows/ci.yml` without `workflow` scope)
```

In agent sessions the git credential helper is `gh auth git-credential`, whose token carries
`repo` but not `workflow`.  The refusal is server-side, so disabling the command sandbox does
not help.  The SSH key has no per-scope gate:

```bash
git push git@github.com:loft-lang/loft.git HEAD:<branch>     # one-shot
git remote set-url origin git@github.com:loft-lang/loft.git  # or every push
```

Other changes push fine over HTTPS.

---

## CI Validation

### Pre-existing vs. newly-introduced failures — always irrelevant

**A red gate is a red gate, whoever made it red.**  If `make ci` is red when you reach for
`git commit`, fix it first — whether the failure was already on the base branch, came with a
toolchain upgrade, or is yours.  Nobody after you can tell "broken by this commit" from
"broken earlier" from the symptom, and a gate left red turns every later run into noise that
hides the next real regression.  Leave the gate cleaner than you found it.

- A toolchain upgrade brings new clippy lints → fix them (apply the suggestion, or a scoped
  `#[allow(...)]` with a comment saying why the lint is a false positive).  Do not revert the
  toolchain.
- `cargo fmt --check` reports drift in a file you did not touch → run `cargo fmt` and say so in
  the commit message.
- A dead-code warning from a build-profile `cfg` → gate the item with the same `cfg`, or
  `#[allow(dead_code)]` with a comment naming the gated call site.
- A test fails on your branch AND on `origin/main` → it still blocks your commit.  When its
  cause is outside your work, repair it in a separate preparatory commit on your branch.

### Local CI gate

**One full gate per change whose reach you cannot bound — not one per commit.**  `make ci` runs
fmt, both clippy variants, the no-default-features and wasm builds, the native fixture cdylibs,
then the suite (the stages before the suite stop at their first failure; the suite collects up
to `CI_MAX_FAIL`, default 5).  It takes ~20 minutes, so the gate is used once and the cheap
tools around it do the rest:

```bash
./scripts/find_problems.sh --changed      # seconds: the subjects your diff touches, while iterating
scripts/ci-run.sh start                   # the full gate; a ~15 s pre-flight refuses it first
                                          #   when fmt, an audit row or doc drift would stop it
scripts/ci-run.sh recheck                 # AFTER a gate: its failed tests + what changed since
```

**After a red gate, `recheck` — do not restart it.**  Fix what it named, then `recheck` re-runs
exactly the failed tests, the pre-flight, both clippy variants when Rust changed since the
gate, and `find_problems.sh --changed <gate sha>`.  The gate plus a green recheck is the
evidence for a push; name both.  A NEW full gate is owed only when the fix's reach is not known —
the cases [CI_BUDGET.md § After a red gate](CI_BUDGET.md#after-a-red-gate-recheck-do-not-restart)
lists — and never before opening a PR, whose own `ci.yml` is the full gate on the exact sha.

Start a long gate with `scripts/ci-run.sh start` and wait on its recorded pid, never on a
process name ([CLAUDE.md § Key commands](../../CLAUDE.md)).  When this box cannot run the gate
reliably, run the same gate on GitHub: `gh workflow run ci.yml --ref <branch> -f
os=ubuntu-latest` ([CI_BUDGET.md § When the local gate is unreliable](CI_BUDGET.md)).

Keep the **installed** loft current: a stale one builds consumer libraries against an old rlib.
`make install-user-fast` (native only) or `make install-user` (everything) install into
`~/.local` without root, and check that `command -v loft` is the binary just installed.

#### Common pitfalls

| Pitfall | What fails | Cure |
|---|---|---|
| `cargo clippy` without `-D warnings`, or only one variant | CI denies warnings, and runs the `--all-targets --all-features` variant too | both variants, with `-D warnings` |
| A bare `cargo test` after a library change | `cargo build --bin loft` never rebuilds the `libloft.rlib` native tests link, and a bare `cargo test` builds none | `make check-rlib` first; `make ci` builds all three itself |
| Stale wasm rlib | `--html` / `html_wasm` tests fail with rustc errors citing an older source | `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random` — never `--features wasm`, whose bundle imports from `__wbindgen_placeholder__` |
| Stale `tests/lib/native_pkg/native` fixture cdylib | `native_loader` tests misread memory and report "expected N, got M" | `make rebuild-native-cdylibs` (`make ci` and `make run-tests` run it) |
| `#[cfg(feature = "X")]` on an entry of a registration table | registration order shifts, and tests crash with "index out of bounds" | keep every entry, gate only what it includes |
| New files with crypto or FFI constants | pedantic lints (`unreadable_literal`, `many_single_char_names`, `cast_lossless`) | `#[allow(clippy::…)]` on the function or constant |
| A new operator or builtin | `index/target_surface.json` drifts and `make ci` fails a guard no targeted suite shows | `make surface-gen`, after rebuilding the wasm rlib |

### Remote CI

`main` requires five checks: **Test (ubuntu-latest)**, **Test (macos-latest)**, **Test
(windows-latest)**, **Clippy** and **Format**.  `ci.yml` runs many more jobs, most advisory;
what runs when, and the 20-minute PR budget, are [CI_BUDGET.md](CI_BUDGET.md).  A failure on one
platform only is usually a path separator or timing; reproduce it on that platform through
`gh workflow run ci.yml --ref <branch> -f os=<os>`.

Pushing a green branch needs no ask; opening a PR does, and with a PR open only a push that
unblocks a red required check goes without consent ([CLAUDE.md § Branch policy](../../CLAUDE.md)
rule 4).

---

## GitHub Issues and Releases — Hard Limits

**Issues follow [CLAUDE.md § Bug-filing policy](../../CLAUDE.md).**  A bug you are not fixing
now is a GitHub issue with a both-backend repro and its labels; a bug fixed in the same session
gets a regression test and no issue.  Plans are `loft-lang/plans` issues.  Design decisions and
standing state live in `doc/claude/`: an issue is a pick-up, never the home of a decision.

**Never trigger or automate a release.**  A release has a manual validation phase
([RELEASE.md](RELEASE.md)): hands-on testing of the built binaries on each platform, a review
of the changelog, and a deliberate version decision.  Do not push release tags, trigger release
workflows or draft GitHub Releases.

**Merging is squash or rebase, never a merge commit.**  The `Use branches` ruleset on `main`
requires linear history, so `gh pr merge --merge` is refused with *"Merge method merge commits
are not allowed on this repository"*.  The classic branch-protection API reports
`required_linear_history: false` because the rule lives in the ruleset
(`/repos/<owner>/<repo>/rulesets`); read both.  Use `--squash`.  A refused `--merge` arms
nothing, so it is a safe probe; a `--squash` on a PR with green required checks merges almost
at once, so arm it only when a merge is wanted.

---

## Closed-by-Decision Register

Before proposing a feature, fix or language change, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  It records questions evaluated and explicitly
declined — feature proposals, accepted limitations, design choices — so the same questions do
not resurface every session.

- Closed items are **not** backlog: they do not belong in ROADMAP.md's milestones,
  PLANNING.md's priorities or QUALITY.md's active tables.  A cross-reference in an "Out of
  scope" section is enough.
- **Re-opening** requires new evidence (a use case, an incident, a measurement) not available
  at the decision; put it at the top of the revived entry.
- **Adding** an entry requires the question, the evaluation, the decision with its date, and
  a "revisit when" trigger: the compact entry goes in the register, the deliberation in
  [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md) under the same heading
  ([DESIGN_DECISIONS_RULES.md § Using the register](DESIGN_DECISIONS_RULES.md#using-the-register)).

When declining a proposal, strike it (`~~…~~`) in its source doc and append a pointer to its
DESIGN_DECISIONS.md entry.

---

## See also

- [CODE.md](CODE.md) — naming, function length, clippy policy, null sentinels
- [JOINING.md](JOINING.md) — rebasing onto a squash, joining sibling checkouts
- [REVALIDATE_LIBS.md](REVALIDATE_LIBS.md) — checking the shipped libraries against this loft
- [TESTING.md](TESTING.md) — the test framework; [GUARDS.md](GUARDS.md) — guards and `@falsified-at:`
- [ISSUE_TRACKING.md](ISSUE_TRACKING.md) — where open work lives
- [RELEASE.md](RELEASE.md) — gate criteria and the release checklist
- [DEVELOPMENT-history.md](DEVELOPMENT-history.md) — the incidents behind these rules
