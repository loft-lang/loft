
# Release Planning

## What this file is — and isn't

This file answers one question: **what must be true before we tag and publish a release of
the loft language, and how is that proven?**  How a release is then tagged, built, published
and made installable is [RELEASE_PUBLISH.md](RELEASE_PUBLISH.md).  The measurements behind
the rules here are [RELEASE-history.md](RELEASE-history.md).

RELEASE.md defines no work.  A release blocker is an open issue or a red gate, and an open
issue is never ignored for a release ([CLAUDE.md § Bug-filing policy](../../CLAUDE.md)).

| File | Question it answers |
|---|---|
| **RELEASE.md** (this file) | "What must be true before we can publish, and how is it proven?" |
| **[RELEASE_PUBLISH.md](RELEASE_PUBLISH.md)** | "How is a release tagged, built, published and made installable?" |
| **[releases/](releases/README.md)** | "What did THIS release need, find and decide?" — one directory per cycle |
| **[ROADMAP.md](ROADMAP.md)** / **[PLANNING.md](PLANNING.md)** | "What is the arc of work, and what is next?" |
| **GitHub Issues** (`make work`) | "What is broken today?" — [PROBLEMS.md](PROBLEMS.md) is the closed archive |

## Release cadence

Releases follow a **monthly rhythm**.  Each cycle has one long-lived branch named for its
**release month**, in `YYYY-MM` form.  All cross-theme work for the cycle lands on that
branch, and it ships at the **start of that month** — but only once the language is **stable
with a low bug count**.

A release is gated on **stability, not a fixed feature set**: if the bug count is still high
at the month boundary, the release slips and the branch keeps stabilising.  When a cycle
ships, the next month's branch starts fresh from the new `main` tip.  A cycle's theme is
written in its `releases/<cycle>/README.md`; what work is in scope during a cycle is
[ROADMAP.md § Feature freeze](ROADMAP.md#feature-freeze--heading-into-the-2026-07-cycle-added-2026-06-07).

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

### File split per release (by hand) — one release, one file, one PR

Also each release: split the top file of the pick, with the `split-file` skill.

```bash
make file-sizes ARGS="--pick 2"    # this release's files, most movable lines first
```

This one is a GATE row (`M-file-split`), not a report: a report read once per cycle is one
nothing acts on.  A file too long to read whole is edited in slices, and code that is locally
right and globally inconsistent is what that produces.

- **The pick ranks by split value**: the lines a split can move, which is the file
  minus its largest item.  An `impl` block counts as its items, not as one subject.
  A file that is one long item scores near zero, however long it is; its length is that
  function's own `too_many_lines` question ([CODE.md § Functions](CODE.md#functions)).
- **One file per PR, a pure move**: no signature, behaviour or comment change, so the
  diff reads as moves and the review is a check that it does.  Take two picks when the
  previous release's split landed clean, one otherwise.
- **No per-PR file-size limit.**  How large a file has grown is this row's question; a
  fix never has to carve a file first.
- **Payoff check**, read in `M-file-sizes`: did last release's split land, and did the
  file it came from stay down?  A split whose parts have regrown had its seam chosen
  wrong.  Re-open that seam; do not add a third part.

### The operator census (by hand) — which operators anything still emits

Also each cycle, before tagging, read the **operator census**:

```bash
make ops-census                    # live / unexercised / orphan, and the table's occupancy
make ops-census ARGS=--verbose     # every operator with its emission count
```

An operator is cheap to add and invisible to retire. A `fn Op…` in `default/*.loft`
becomes an entry in the generated `fill::OPERATORS`, a `#rust` template the
interpreter runs, and a template the native generator rewrites — and nothing ever
asked whether a program still reaches one. Same **report, never a blocker** status as
the passes above.

It asks the compiler rather than the text, because a grep cannot answer it: the parser
names most operators by *computing* the name (`format!("Op{}", rename(op))`, then
overload resolution on the operand types), so `OpEqText` is emitted by every `a == b`
on text while the string `"OpEqText"` appears nowhere in `src/`. Three verdicts, and
only the third is a deletion candidate:

- **live** — something emits it.
- **unexercised** — a site in `src/` emits it, no program in the tree does. A *test
  gap*, and usually the more actionable finding: live emitter, zero coverage.
- **orphan** — never emitted and nothing in `src/` names it.

An orphan is a candidate, not a verdict — confirm one before acting on it (the census
reads this tree, not a consumer's). What it is *not* for is freeing slots: the table
holds 511 opcodes and is nowhere near full (INTERMEDIATE.md § Opcode budget).

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
  close surfaces within a day, not at the next audit by hand — and it **fails the nightly**
  when a CLOSED plan still carries a live status label, which is a contradiction
  rather than a judgement and takes one command to fix: a closed plan wearing
  `status:next` stays in everyone's next-up queue.
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

## Versioning and the stability contract

Releases are calendar-versioned, `YYYY.M.P` (`2026.9.0`), one per monthly cycle
([releases/2026-06](releases/2026-06/README.md) records the switch); a point release off the
beat is for a binary fix that cannot wait.  What a release may change is
[COMPATIBILITY.md](COMPATIBILITY.md), not a semver clause: its § *The road to contract 1*
holds the floors the freeze needs and `make rule-coverage` measures the position against
them.  Safety (no crash, no corruption, no leak) is not a contract addition; it is the
floor for every release, tracked under the [Safety gate](#safety-gate--blocks-every-release)
below.

---

## Safety gate — blocks EVERY release

**We do not ship broken builds.**  The items below block every release, a point release
included.  A release that crashes, corrupts memory, or leaks per iteration is not a release —
it is a bug report on a schedule.  If a safety blocker is open on release day, the release
slips; there is no "we'll fix it next version" for crashes and leaks, and a "quick fix" tag
that closes one bug while another stays open is still blocked.

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
  the 03:00 UTC daily starts hours late, on whatever `main` is at that moment.

So the release evidence is a **current, deliberate run on the candidate's
commit**, not a historical result — and it is one command:

```
make release-gate          # every nightly, THIS commit, one CI run, one verdict
make release-checklist     # `A-release-gate` reads the newest run for HEAD's sha
```

`release-gate.yml` calls the nightlies as reusable workflows — the full
`ci.yml` matrix incl. Windows with the stdlib round-trip and the differential
oracle, every `miri.yml` sanitizer and invariant gate, `revalidate-libs` (both
backends: a native failure gates unless the library's manifest declares
`[native] build-deps` the runner may lack — `revalidate_matrix.py
--native-policy`), `browser-threads`, `repro-build`, and `consumer-main-health`
(every package of the applications built with loft, both backends — the same
file `release.yml` calls at the tag, here BEFORE the candidate is chosen) — and a
`verdict` job goes
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

**A red leg that is not the candidate's is WAIVED on the record, never ignored.**
`make release-checklist ARGS="--waive <leg> --note '<why>'"` records the leg (`ci.yml`,
`repro-build.yml`, …), the run it was red in and the reason, beside the manual ticks in
the cycle's `checklist.json`; `A-release-gate` then reads the run's jobs and answers
*green, leg waived: <why>* when every red leg is waived, and names the unwaived ones
otherwise.  A waiver names ONE run, so the next run starts with none and a leg that stays
red is re-justified each time.  It is the table above made mechanical: without it, a gate
whose legs go red for environment reasons can never end in evidence.

**`registry-validation` is not a leg.**  It validates every published package against the
registry's own rules, so a library's defect would turn the toolchain's gate red — and
§ What forces a release says the registry is never release-coupled.  `revalidate-libs` stays:
*does this loft break a shipped library* is the coupling that matters.

### WASM endpoint — our primary deliverable must work

The browser WASM bundle (`doc/pkg/loft_bg.wasm` + `doc/pkg/loft.js`)
is the primary way users encounter loft — the gallery, the playground,
Brick Buster, and `loft --html` all depend on it.  A release where the
WASM path is broken is a release that doesn't work for most users.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **WASM-build gate** | H | `cargo build --release --lib --target wasm32-unknown-unknown --no-default-features --features wasm` must succeed with the current stable `rustc`.  The `doc/pkg/` bundle must be rebuilt from this output before tagging. | `Cargo.toml` features, `.github/workflows/ci.yml` |
| **WASM-runtime gate** | H | `tests/html_wasm.rs` must pass: its tests compile a trivial `.loft` to `--html`, extract the embedded WASM, and run it under Node with stub host imports.  Any `unreachable` trap or instantiation failure blocks. | `tests/html_wasm.rs`, `tools/wasm_repro.mjs` |
| **Gallery smoke** | M | `make gallery` must complete and `doc/gallery.html` must load every example in a browser without console errors.  Verified by CI (`make test-gl-headless`) where Xvfb is available. | `doc/gallery.html`, `.github/workflows/ci.yml` |

### Crashes — no release may crash on valid input

The gate is the corpus on both backends under `make ci`, and the sanitizer nightlies in
`miri.yml` (ASan, the native-backend ASan, TSan, `LOFT_POISON`, the debug-assertions and
`LOFT_VERIFY_STACK` sweeps), all of which the release gate runs on the candidate's commit.
An open crash issue blocks the release whatever its severity label says
([bug-filing policy](../../CLAUDE.md): an open issue is never ignored for a release).

### Memory safety — no release may corrupt memory

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Valgrind-clean gate** | H | `scripts/valgrind-sweep.sh`: every script in `tests/scripts/` and every doc in `tests/docs/` under memcheck, on the interpreter and as the compiled native program, must show no invalid access and `definitely lost: 0 bytes in 0 blocks`.  Runs nightly (`miri.yml` `valgrind` job) and inside the release gate; `M-valgrind` is satisfied by that job on the candidate's commit, or by the sweep run here. | TEST_ENVIRONMENTS.md § Occasional valgrind pass |

### Memory leaks — no release may leak on valid programs

Long-running programs — servers, game loops, REPLs — cannot tolerate per-iteration
leaks, so the floor is a store count of zero at every program's exit.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-leak gate** | H | The wrap suite hard-fails any `tests/scripts` file that leaves a store unfreed at exit (`State::check_store_leaks`; `SCRIPTS_LEAK_ALLOW` is empty by design), on every `make ci` and inside the release gate; the `par` scripts (`22-threading`, `80-parallel-block`) are in that corpus.  There is no separate hand sweep. | `tests/wrap.rs` `loft_suite`, `src/state/mod.rs` `check_store_leaks` |

### Test suite integrity — no release may silently skip tests

An ignored test is a bug you promised you would fix, then pulled out of CI.  Every
`#[ignore]` hides a known failure — if the suite is silently skipping them, the release's
"all green" status is a lie.  The bar: **no `#[ignore]` ships unless it carries a rationale
that names the run it rides**, and the owner signs off on the set each release.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-ignore gate** | H | Every `#[ignore]` carries a one-line rationale in `tests/ignored_tests.baseline` naming how it runs instead (a measurement by hand, a nightly sweep, a platform cap) — held mechanically by `A-ignores`, and judged for acceptability by the owner in `M-ignores`. | `tests/ignored_tests.baseline`, `tests/doc_hygiene.rs::{ignored_tests_baseline_is_current, every_ignore_reason_says_how_it_runs}` |
| **Skip-list audit** | H | Every `SKIP` / `NATIVE_SKIP` / `ignored_scripts()` entry is a suppression of the same kind and is in the same sign-off; `make release-liveness` reports any whose justifying issue has since closed.  TESTING.md § *Every skip says why, how it runs instead, and when it ends* is the rule's home. | `tests/*.rs` skip lists (found by shape), `scripts/release-liveness.py` |

---

## Explicitly out of scope here

The applications built with loft (the games, the crawler, the web IDE shell), the published
libraries and the registry follow their own lifecycles and are **not** release blockers here
(§ What forces a release).  The consumers are PROVEN against a release at its tag
([RELEASE_PUBLISH.md § What the tag pipeline proves](RELEASE_PUBLISH.md#what-the-tag-pipeline-proves-about-the-artifacts)),
and the owner decides on the record whether a red consumer holds it.

---

## No Automated Releases

**Releases must never be created or triggered automatically.**  Every release
requires a human validation phase (the checklist below) that cannot be scripted:
hands-on testing of pre-built binaries on each platform, review of the CHANGELOG,
and a deliberate decision to tag and publish.

Do not push release tags, trigger release workflows, draft GitHub Releases, or
run `cargo publish` programmatically.  Always wait for the owner to do this
manually after completing the validation checklist below.  The mechanics the owner follows —
draft-first tagging, the registry splice, and what the tag pipeline proves — are
[RELEASE_PUBLISH.md](RELEASE_PUBLISH.md).

---

## Pre-Release Documentation Review

> **Run it as a reviewer session** ([DOC_CONTRACT.md § Reviewer pass](DOC_CONTRACT.md)):
> a fresh session that loads the `doc-quality` skill, never the session that wrote the
> text.  The skill carries the judgment the lint cannot make; what a script can check is
> `scripts/doc_lint.py`'s, reported by `make docs-lint`.  Two checklist rows sit beside
> this review on the `[mid]` beat: **`A-docs-lint`** (the lint's count did not grow since
> its baseline, and a burn-down is re-pinned) and **`M-doc-review`** (one maintainer doc
> brought fully under the contract this cycle, the top of the lint's worklist).

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

**The stdlib's position (0a)** is what `scripts/api_lint.py -c default/*.loft` reports; the goal
is `0 active`.  It is a burn-down goal, not a release precondition: loft's own findings never
block loft's release.

### Steps deferred until external developers take part

Steps **5, 6 and 7** validate the user-facing surface (examples against the changelog, the
comparison pages, the topic order).  Without external users hitting that surface, the
validation is closed-loop — the author of an example reads it, sees nothing wrong, and ships —
so they wait for the signal that makes them meaningful: external developers filing issues,
opening PRs or asking documentation questions.  Update this section when that happens.  Step
0's tooled checks (0a, 0b, 0f, and the automatic parts of 0c/0d) still run every release; only
the judgement in 0e waits with them.

Not deferred: steps 1–4, 8 and 9, which protect the shipped artefact whether or not external
users exist, and the cross-platform walkthrough, which IS the tag pipeline's per-bundle smoke
([RELEASE_PUBLISH.md § What the tag pipeline proves](RELEASE_PUBLISH.md#what-the-tag-pipeline-proves-about-the-artifacts)).
Like the rest of this review they are advisory; only the safety gate blocks a release.

### 1 — Audit doc/claude/ for stale problem documentation

- Open bugs live in GitHub Issues (`make work`); PROBLEMS.md is the closed archive and has no open rows to audit.  Check that every issue the cycle fixed carries its `Fixes #N` commit.
- Open PLANNING.md: every item should be open.  Done items must have been removed (not marked done in-place) before this release.
- Agent memory is per-agent and is not release evidence; anything durable in it belongs in a repo doc (CLAUDE.md § Conventions).

### 2 — Verify code links in doc/claude/

Walk every file in `doc/claude/` looking for references of the form `src/foo.rs`, `src/foo/bar.rs`, function names, struct names, or opcode names.  For each:
- Confirm the file/symbol still exists at that path/name.
- Update any that have moved or been renamed.

Helpful command: `grep -rn 'src/' doc/claude/` and cross-check against `ls src/`.

### 3 — Verify doc/claude/ discoverability

- Every file in `doc/claude/` is reachable from the CLAUDE.md index in at most two hops ([DOC_CONTRACT.md](DOC_CONTRACT.md) rule 30).  Agent memory is per-agent and never counts as a route.
- Orphaned files (nothing links to them) are added to the doc that owns their topic, or removed.

### 4 — Compact verbose sections

Read through any doc/claude/ file that has grown since the previous release and identify passages that are longer than necessary (e.g. multi-paragraph context that can be reduced to a bullet list, repeated caveats, implementation notes already captured in CHANGELOG.md).  Shorten these in place.

### 5 — Validate user documentation against this release

> The corpus-wide checks are step 0 (0a–0e); this step is the *changelog-driven*
> cross-check that each shipped change is reflected.

For each feature and bug-fix entry in CHANGELOG.md under this cycle's `## YYYY-MM` section:
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

A dated census of the suppressions, with what CI never lints, is in
[RELEASE-history.md](RELEASE-history.md); the current one is `make clippy-review`'s output.

### 9 — Generate HTML and PDF

```sh
# Regenerate HTML reference
cargo run --bin gendoc

# Compile PDF
typst compile doc/loft-reference.typ
```

Verify that `gendoc` completes without warnings and that the generated HTML files look
correct in a browser.  The PDF ships inside every bundle; the checks that it is current, says
this version and holds every chapter are
[RELEASE_PUBLISH.md § The reference PDF](RELEASE_PUBLISH.md#the-reference-pdf).

The per-OS bundles, their checksums and the registry entry are
[RELEASE_PUBLISH.md § The bundles and the registry entry](RELEASE_PUBLISH.md#the-bundles-and-the-registry-entry).

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
| `cargo audit` | `A-audit`: RUSTSEC advisories over `Cargo.lock` (the nightly `audit` job asks the same on the schedule) | `cargo install cargo-audit --locked` |

## The per-release checklist — `make release-checklist`

**Work the generated list, not this document.**  Everything a release needs a
human to do is one command:

```
make release-checklist                     # the list for Cargo.toml's version
make release-checklist ARGS="--fetch"      # refresh origin/main + tags first
make release-checklist ARGS="--done M-install-sh --note 'ran on the NUC'"
```

It is the one list: the steps a partial list leaves out — the Windows `self-update`, the
registry splice, `scripts/install.sh` — are the ones that get skipped, not by decision but
because no list said them.

Three things make it worth working through rather than reading:

- **Automatic items are measured on every run and cannot be ticked.**  "Is
  `make ci` green" is not a promise a human gets to make — it is `result.txt`'s
  verdict line, and a verdict older than the newest source file reports STALE
  rather than pass.  A gate you can tick is a gate that gets ticked.
- **Manual items carry the exact command and what counts as a pass**, and are
  the only ones `--done` accepts.  Progress lives in `releases/<cycle>/checklist.json`,
  committed, with a timestamp and your note as the evidence.
- **Items for work this release did not touch stay hidden.**  The VS Code
  extension pass and the native-debug gate are rituals for code most releases
  never change; the script asks git whether they moved since the last tag.  A
  list that includes work nobody needs to do is one people learn to skim.

Two more properties:

- **Every item carries its CADENCE, and the early views are commands.**  The test a
  marker applies is whether the item can be FINISHED in that phase — whether its
  evidence stays valid as the tree moves on — because a tick is a claim about the
  release, not about the day it was made.  `[mid]` marks what is completable at the
  cycle's HALFWAY point, because it measures overall stability or a process state
  rather than a release artifact (the monthly reviews, the liveness census, the
  falsification receipts, the performance pass, the file-shape census,
  `A-registry-prev` — the step-4 backstop, worth asking early); `[cand]` marks what is
  worth RUNNING early as warning but whose tick must name the tag candidate (the
  leak and valgrind sweeps, the wasm endpoint); `[pre]` marks what can be finished in
  the month's LAST DAYS as pre-work, so the release does not spill deep into the new
  month (changelogs, the PDF, the reference review, the release gate on a near-final
  candidate); unmarked items need the release window itself (the tag, the draft, or
  the published assets).  `make release-checklist ARGS="--phase mid"` (or `pre`) shows
  and measures exactly that slice, listing the `[cand]` rows apart and counting them
  nowhere.

  ⚠ **A row that cannot be finished in a phase does not belong in that phase's
  tally.**  A denominator no halfway run can reach reads as permanent unfinished work and
  teaches the reader to skim the list; `[cand]` rows are run early and counted nowhere.
- **A check that could not run never reads as a check that passed.**  The summary names
  every automatic item that stayed UNKNOWN ("not green: … never ran"), the exit code
  keeps red (1) apart from not-yet-evidence (3) and green (0), and the header stamps
  the commit the run measured — release evidence is "these gates ran on this commit",
  never "nothing was red".

**The liveness census — a gate that checks the gates are live.**  `make
release-liveness` (the `M-liveness` item makes it per-release; it is a REPORT, never a
gate) walks the three drifts that accumulate BETWEEN releases: suppressions whose
justifying issue has since CLOSED, gate workflows that quietly stopped firing or whose
last verdict was red, and checklist items never recorded as run in any committed cycle.

The per-item landing procedures in the release's plans are separate and still
apply (e.g. NDB.0 in [`plans/34-native-debug/`](plans/34-native-debug)).

**What it covers, audited against this document.**  Every gate this
file calls a release blocker is an item: the safety gate's valgrind, zero-ignore and
skip-list rows (`M-valgrind`, `M-ignores`, with `A-ignores` checking the rationales
mechanically; the zero-leak gate is the suite's own assertion, carried by `A-ci` and the
gate), the WASM endpoint gate (`M-wasm`), the nightlies (`A-release-gate`: one deliberate
run of every nightly against HEAD's commit, measured), the dependency audit (`A-audit`), the consumers at the tag
(`A-consumers`), step 9's
artefacts, step 10's
binaries and registry entry, and the monthly reviews the cadence makes
per-release work (`M-monthly-docs`, `M-monthly-bugs`, `M-close-plans`, `M-file-split`, and
`M-perf-pass` — the performance read over loft AND its libraries; @PLN158 grows it
into per-routine benches with industry reference twins).  `A-deviations` blocks a release on
any open formal deviation a release can resolve: each has a tracking issue, and the only ones
allowed to ship are marked `not resolvable in a release` with their reason
(`formal/README.md` § Deviation entry format).

**Three classes of row.**  A GATE row must be true (`A-ci`, `M-valgrind`); a `[report]`
row must be READ — `M-monthly-bugs`, `M-file-sizes`, `A-reference-review`, the passes this
document calls *a report, never a blocker* — and the tally counts the two apart; a DERIVED
row is a manual row a green gate job on the candidate's commit satisfies without a hand-run
(`M-valgrind`, `M-libs`, `M-wasm`, each naming the job that covers it), the hand-run staying
as the fallback when the gate cannot run.  A tick records the COMMIT it was made on, and a
`[cand]` tick whose commit differs from HEAD in what ships (`src/`, `default/`, `tests/`, the
manifests) reads `[~] STALE` rather than done.  A new blocking row must be able to name the
past defect it would have caught; a row that cannot is a report.

The artefact items — the bundle smoke, the registry entry, the acquisition chain and the
three reference-PDF checks — are described with the pipeline they read,
[RELEASE_PUBLISH.md](RELEASE_PUBLISH.md).  Whether the reference still describes the language
is [REFERENCE_REVIEW.md](REFERENCE_REVIEW.md), a per-chapter watermark pass done early and
continuously rather than on tag day:

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

## See also
- [RELEASE_PUBLISH.md](RELEASE_PUBLISH.md) — tagging, the bundles, the registry entry, what the tag pipeline proves
- [RELEASE-history.md](RELEASE-history.md) — the measurements behind these rules
- [releases/](releases/README.md) — one directory per cycle
- [PLANNING.md](PLANNING.md) / [ROADMAP.md](ROADMAP.md) — the backlog and the milestones
- [DEVELOPMENT.md](DEVELOPMENT.md) — branches, commits and CI
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — every known inconsistency is resolved or accepted before contract 1

