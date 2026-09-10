<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue labels — what they mean

For anyone (human or agent) triaging or fixing a loft issue **without** reading
the loft source.  Each label's GitHub *description* is the one-line gloss; this
file is the detail.  Convention + workflow: [`doc/claude/ISSUE_TRACKING.md`](../doc/claude/ISSUE_TRACKING.md).

Apply **one `sev:`, one `wa:`, and one or more `area:`** to every bug.

## If you cannot set labels — ask for them in the body

GitHub only lets people with repository **triage** permission attach a label, so
a reporter usually cannot: an issue filed from outside arrives bare, and there is
no setting that grants labelling without granting triage.

You do not need it. Say which labels you want and
[`label-guard`](workflows/label-guard.yml) applies them for you:

- **From the bug form**, nothing to do — your Severity, workaround and Area(s)
  answers land in the issue body, and the guard turns them into labels. (GitHub
  does not do this itself; a form's dropdown answer is text, not a label.)
- **Filed freeform** (`gh issue create`, the API — both ignore issue templates):
  add a `### Triage` section to the description, one value per line.

```
### Triage
sev:medium
wa:clean
area:native
```

Editing the description re-runs the guard, so a missed label is one edit away and
`needs:labels` clears itself. Only the values listed on this page are ever
applied — an unknown one is ignored rather than created.

Two things it will not guess. `sev:` and `wa:` are single-choice, so naming more
than one leaves the category unset (prose that mentions all three severities
labels none of them — issue #626 was labelled with six mutually exclusive tokens
that way). And `area:` is read only from the form's checked boxes or a `### Triage`
block, never from prose, for the same reason.

## `sev:` — severity (how bad WHEN you hit it)

| Label | Meaning |
|---|---|
| `sev:high` | crash / memory corruption / data loss / a soundness hole (the program does something silently wrong) |
| `sev:medium` | wrong result, hang, or miscompile in a real-world shape — but no corruption |
| `sev:low` | cosmetic, an edge case few will hit, a false-positive warning, a confusing-but-recoverable diagnostic |

## `wa:` — workaround (can you KEEP MOVING, or are you blocked)

The agent's primary "route-around or blocked?" signal.  **Always VERIFIED** — the
workaround was actually run on current HEAD, both backends; see
[ISSUE_TRACKING.md § Workarounds](../doc/claude/ISSUE_TRACKING.md#workarounds--the-agents-can-you-keep-moving-signal).
A wrong workaround is worse than `wa:none`.

| Label | Meaning |
|---|---|
| `wa:clean` | a simple, idiomatic alternative exists (verified) |
| `wa:partial` | a workaround exists but is awkward / loses the intended behaviour (verified) |
| `wa:none` | nothing works — this **blocks** whoever hits it (the most urgent triage axis, often above `sev:`) |

## `silent-wrong` — the freeze-blocking axis

**Set it whenever the program answers WRONG and nothing says so**: no diagnostic, no
refusal, no crash — or a promise of the type system does not hold for some value that
reaches it.  A `u8` that reaches 260, a tuple element read one slot high, an accessor
whose second call answers zeros, a `count_if` whose loop was never emitted and left the
destination at its old value, a `verify-self` that exits 0 for a check it could not run.

It is a SEPARATE axis from `sev:` and `wa:`, and for the language freeze it OUTRANKS
both.

- **Above `wa:`** — a clean workaround only helps someone who learns they need one.
  Nobody route-arounds a bug they never find out about, so `wa:clean` + `silent-wrong` is
  not a mild combination; it is the worst one to leave open, because the workaround is
  never reached.
- **Above `sev:`** — `sev:` says how bad it is the day you hit it.  This says whether we
  can freeze the contract at all.  Anything we ship as a promise while it is open, we are
  promising falsely: programs already depend on the wrong answer, and correcting it later
  is a breaking change no consumer can detect they need.  A `sev:low` edge that answers
  quietly wrong is a freeze blocker; a `sev:high` crash is not — a crash tells you.

So it is not a severity, it is a **contract** question: *if we froze today, would this be
in the contract?*  Every open `silent-wrong` is one item on the pre-freeze list.

`sev:high`'s own text mentions "a soundness hole" — that stays, because a soundness hole
IS severe on the day you hit it.  This label is what makes the class QUERYABLE across
every severity, which is what the freeze needs:

```console
gh issue list --state open --label silent-wrong
```

Not `silent-wrong`: a crash, a SIGSEGV, a refusal to compile, an internal compiler error,
a wrong ERROR MESSAGE, or a leak.  All of those announce themselves.  A leak that
eventually OOMs is still not silent-wrong — the answers it gives are right until it dies,
and the death is loud.

## `contract:` — did closing it MOVE the standard? (the convergence axis)

`silent-wrong` asks *if we froze today, would this be in the contract?* — a question about
one bug, answerable when it is FILED.  This asks the other one: *when we fixed it, did the
written standard have to change?*  That is **not knowable at filing** — it is what the fix
turned out to need — so this axis is set when we state the bug is FIXED, and never before.

Over a month the RATIO is the convergence signal the freeze needs, because a raw bug count
fuses two things that mean opposite things:

- **we keep FINDING bugs** — the audits are productive, which is what they are for; and
- **the standard keeps MOVING** — the only one of the two that can make a freeze premature.

| Label | Meaning |
|---|---|
| `contract:settled` | The formal spec ([`doc/claude/formal/`](../doc/claude/formal/README.md)) and the existing tests **already gave the right answer**; the code was wrong, and the fix makes a written promise hold.  Freezing before this bug was found would still have been correct. |
| `contract:strained` | The rules did **not** settle it.  Closing it needed a rule EXTENDED, a documented behaviour CHANGED, or a design call about what *correct* even means.  Freezing before it would have frozen something we then had to move. |

### Where it is written: the fix commit, beside `Fixes #N`

The judgement exists exactly once — in the commit that lands the fix, written by whoever
just found out what the fix needed.  So it goes there, as a trailer, next to the
`Fixes #N` the `fixed-pending-merge` automation already reads:

```
Fixes #1120
Contract: settled — `(E-Coalesce)` and `(N-Index)` already said what the right
  answer was; the two lowerings were what disagreed.
```

`Contract: settled` or `Contract: strained`, then a dash and one line of WHY — the reason
is the part a month-later reader needs, and writing it is what stops the call becoming a
reflex.  `.githooks/commit-msg` reports a `Fixes #N` with no `Contract:` trailer (it never
blocks — same two-tier rule as the rest of that hook).

**The label is then applied for you, at the push.**  The same workflow run that labels the
issue `fixed-pending-merge` reads the trailer off the same commits and sets `contract:` —
[`apply-fixed-pending-merge.yml`](workflows/apply-fixed-pending-merge.yml), which runs
`scripts/contract_labels.py --event`.  A `Fixes #N` that arrives with no trailer gets a
warning naming the commit, and the issue stays UNJUDGED; nothing reads it as settled.
Pushing the same fix twice is harmless, and a later commit that revises the call wins.

That leaves the script as the BACKSTOP rather than the route: `scripts/contract_labels.py`
reads the trailers off a whole branch (`--report` for only the misses, `--apply` to write),
which is what recovers a push the workflow could not see — a longer-than-20-commit push, or
a trailer added by an amend after the fact.  Its parse is held to a fixed corpus by
`make contract-labels-test`, run by `make ci`: every way that regex can be wrong is silent,
because no label and no judgement look identical.

Setting the label by hand on the issue is equally fine; the trailer just puts it where the
knowledge already is.

### Three triggers for `contract:strained`

Any ONE of these makes it strained.  Each is observable in the diff, so the call is not a
feeling:

1. a rule TEXT under `doc/claude/formal/` changed, or a new rule was written, to admit the
   fix — *"an edge the rules cannot express means the RULE wants extending"*;
2. a documented surface changed (LOFT.md, STDLIB.md, a `@F` catalogue entry), or the fix
   needed a `#superseded` steer or a contract key;
3. it carried `design` / `needs-design`, or closing it added a
   [`DESIGN_DECISIONS.md`](../doc/claude/DESIGN_DECISIONS.md) entry — the rules could not
   say what *correct* even meant.

Anything else is `contract:settled`: the deviation register closed it by changing code to
match a rule already written, which is the doctrine
([formal/README.md](../doc/claude/formal/README.md)) — *the rules do not change to match
the code; the code changes to match the rules*.

### Independent of `silent-wrong`, and that independence is the point

A silently wrong answer is USUALLY `contract:settled` — the spec said what the right answer
was and the code did not deliver it, so the fix moves nothing.  `silent-wrong` +
`contract:settled` is the common pair, and reading a rising `silent-wrong` count as a moving
contract is exactly the mistake this axis exists to prevent.

**The freeze wants BOTH, and they are different gates.**  `silent-wrong` → 0 is the per-bug
blocker: no known wrong answer may be frozen into the contract.  `contract:strained` → 0
**sustained over a window long enough to be evidence** is the convergence gate: the written
standard has stopped moving.  The first can be true on any given day; only the second says
the surface has settled.

**Absence means NOT JUDGED, never "settled"** — the same rule [`hit-by:`](#hit-by--which-project-hit-it)
and `steered` follow.  A monthly count that read unlabelled as settled would report
convergence it never measured, so `scripts/bug-review.py` prints the UNJUDGED count beside
the ratio and folds it into neither side.

```console
gh issue list --state all --label contract:strained
```

## `area:` — which part of loft (plain-English, with orienting files — NOT required reading)

loft is a tree-walking interpreter **and** a native code generator for a
statically-typed language.  Source flows: **text → parser → IR → codegen →
{ bytecode interpreter | native Rust }**, over a **store-based heap**.

| Label | What it is | Bugs here look like | Orienting files |
|---|---|---|---|
| `area:parser` | turning source text into typed IR: lexer, two-pass parser, type resolution, scope/lifetime analysis | parse errors, a construct mis-resolved, wrong inferred type/scope, a bad diagnostic | `src/parser/`, `src/lexer.rs`, `src/typedef.rs`, `src/scopes.rs` |
| `area:codegen` | turning correct IR into runnable code — the **bytecode** generator (interpreter) and the **native** Rust generator | the program parsed + type-checked fine, but the emitted code is wrong: wrong value, panic, slot/stack drift, a miscompile on one backend | `src/state/codegen.rs`, `src/state/fill.rs`, `src/generation/` |
| `area:store-lifetime` | the heap: store alloc/free, dependency tracking, value-semantic vector/struct lifetime, the watermark | leaks, use-after-free, double-free, a store freed too early/late, high memory watermark | `src/database/`, `src/store.rs`, `src/scopes.rs` (the free analysis) |
| `area:closures` | fn-refs, lambdas, captured cells, the closure-record layout | capture/ownership bugs, closures stored in containers, fn-ref dispatch/iteration | `src/parser/vectors.rs` (capture), closure-record handling in `src/database/` |
| `area:runtime` | executing bytecode — the 233 opcodes + the runtime stack/store ops (interpreter only) | a specific opcode misbehaves, a runtime panic not in codegen | `src/state/`, `src/fill.rs` |
| `area:native` | the `--native` backend specifically: Rust generation, the native runtime shim, `rustc`/`cc` linkage | a native-only divergence from the interpreter, an ABI/marshalling bug, a link/rlib failure | `src/generation/`, `src/codegen_runtime.rs` |
| `area:wasm` | the browser / WASM target: `wasm32`, the virtual FS, host bridges, threading | WASM-only failures, a missing host bridge, a feature gated off in the browser | `src/wasm.rs`, `wasm/` |
| `area:stdlib` | the standard library (`default/*.loft`) + native stdlib functions | a stdlib function returns the wrong thing / is missing an edge | `default/*.loft`, the native-fn registry |
| `area:packages` | the package format, registry, multi-file `use` resolution, library extraction | a cross-package resolution bug, a manifest/registry issue, a build-pipeline gap | `src/package.rs`, `src/manifest.rs`, `doc/claude/lib_plans/` |

## `hit-by:` — which project hit it

One per issue: a **direct pointer to the project that ran into it**.  loft is one of those
projects, so a find of our own is `hit-by:loft` — not a blank.

| Label | Project | What it is | What it puts the language under |
|---|---|---|---|
| `hit-by:moros` | [jjstwerff/moros](https://github.com/jjstwerff/moros) | role-playing game + blueprint editor | the heaviest consumer by volume — long-lived stores, editor sessions, placed geometry, `--html` |
| `hit-by:dryopea` | [jjstwerff/dryopea](https://github.com/jjstwerff/dryopea) | sci-fi free-build / tower defence | per-frame allocation, palettes, struct-heavy state |
| `hit-by:crawler` | [jjstwerff/crawler](https://github.com/jjstwerff/crawler) | clean-room hex roguelike | the hex library family end to end, renderer-agnostic kernel |
| `hit-by:routing` | [jjstwerff/routing](https://github.com/jjstwerff/routing) | phone-first route planner | the NON-game axis — big read-only stores, `--native-wasm`, remote/paged stores |
| `hit-by:zerotrust` | [jjstwerff/zero-trust-shared-files](https://github.com/jjstwerff/zero-trust-shared-files) | federated shared-file system | crypto, recursive enums, text-returning `cdylib` boundaries |
| `hit-by:planets` | `loft_planet`, a package inside [hstellingwerff-jpg/Moros-Economy-Development](https://github.com/hstellingwerff-jpg/Moros-Economy-Development) | a loft re-implementation of that project's planet generator, plus the comparator that checks it against the C# original | the PORT axis — numeric parity against another language, and a loft package living inside a non-loft repo, which is what puts `--lib` and module resolution under pressure |
| `hit-by:loft` | [loft-lang/loft](https://github.com/loft-lang/loft) | the language itself | a nightly gate, a sanitizer, a sweep, a follow-on investigation |

The libraries those consumers import are `loft-lang/loft-libs-*` (world, net, graphics,
game, assets, core, plugins, docs) plus the registry — a defect surfaced there is still
labelled by the APPLICATION that ran into it, because that is the project that can
reproduce it.

Not every consumer is a repo of its own or even under the same owner: `planets` is a
package inside someone else's project.  That is worth keeping visible rather than
normalising away, because a loft package that is not the root of its checkout is exactly
the shape that finds the resolution bugs a standalone one never will.

**This table is the index for ecosystem analysis, not only a label glossary.**  Asking
*"is loft usable?"* means asking what its consumers hit, and the answer is only readable
if the consumer set is written down: a label with no project behind it cannot be checked,
and a consumer with no label cannot be counted.  Keep a row per project, and add the row
when the label is minted rather than after.

It says who HIT it, nothing more.  A follow-on we filed while fixing something else is
`hit-by:loft` even when a consumer's report is what sent us into that subsystem: we hit it.

**Lineage is `Found-via: #N`, and it is kept SEPARATE on purpose.**  A consumer filters
`hit-by:<their project>` to find what THEY reported.  Inherited credit puts issues in that list
they never ran into and cannot recognise — they read it as us misfiling against them, and the
one label they rely on stops being trustworthy.  That is the cost that lands on a person; the
analytic cost is the same shape, one field carrying two facts, so you could no longer ask "what
did routing actually run into" without unpicking a chain.
Keep the pointer direct and the chain explicit, and the prompted-by view is *derivable*: walk
`Found-via:` to its root and read the root's `hit-by:`.  So the late-July store-lifetime cluster
is `hit-by:loft` (we hit it) with a chain leading back to routing's tickets — and both the
"who hit it" and the "what set us looking" counts stay answerable.

**Record it at filing time.**  Lineage cannot be recovered from prose afterwards, and two
plausible-looking shortcuts both mislabel:

- *by area + date* — sweeps in issues that came from elsewhere: a defect in our OWN ownership
  oracle and a bug found while writing the SQL client both sit inside the store-lifetime window.
- *by citation* — an issue citing another is usually "related to" or "same root cause as", not
  "caused by".  Three unrelated issues cite one old hub ticket; loft#755 cites loft#748 to
  explain a design decision it did not come from.

**An unlabelled issue means "not established", never "nobody".**  Roughly 84 issues filed before
this convention carry no `hit-by:`; treat any count over that period as a floor, not a total.

## Cross-cutting

| Label | Meaning |
|---|---|
| `both-backends` | reproduces on BOTH `--interpret` and `--native` (vs a single-backend divergence) |
| `needs-design` | the fix needs a design decision, not a mechanical change — don't just patch it |
| `steered` | **owner-applied only.** The owner had to step in to get this fixed, or fixed thoroughly — a shallow first fix, a missed sibling, a matrix that needed asking for. One click, no prose: it is a *counter*, not a complaint, and it pairs with [`scripts/steering_rate.py`](../scripts/steering_rate.py) (see [STABILITY_ROADMAP § how much STEERING the fixing took](../doc/claude/STABILITY_ROADMAP.md)). **An agent must never apply or remove it** — the agent that needed steering is the least likely to notice, so self-reporting would bias exactly where the signal lives. Absence therefore means "not marked", never "no steering needed". |
| `bug` / `enhancement` / `documentation` / … | the GitHub defaults; keep `bug` on every bug |
| `proposal` | a proposed new library or API change/rewrite (the `library_proposal` intake → the @PLN112 provenance view) |
| `showcase` | an accepted COMMUNITY app, listed in the @PLN112 applications tier (first-party apps self-describe in their own repo via a `.loft-showcase.toml` + the `loft-showcase` topic, no label) |
| `showcase:pending` | a submitted `application_showcase` awaiting review — the intake queue; NOT listed until a maintainer relabels it `showcase` |

## Triage-state (where an investigation got stuck)

These describe the **state of the hunt**, not the nature of the bug — they tell
the next agent (or the maintainer) *why this one is still open* and what unblocks
it.  Apply at most one; remove it once the blocker clears.

| Label | When to apply | What unblocks it |
|---|---|---|
| `needs-triage` | A **public report has arrived and not yet been triaged** — the maintainer hasn't reproduced it, minimised it, or applied the `sev:`/`area:`/`wa:` labels. The reporter cannot set labels (the public has no label write), so this marks "a maintainer still needs to look." It is the queryable side of the acknowledgement promise: `gh issue list --label needs-triage` is the un-drained intake — nothing here should sit unacknowledged. See [ISSUE_TRACKING.md § The public intake bridge](../doc/claude/ISSUE_TRACKING.md#the-public-intake-bridge-arc-d). | Triage: reproduce + minimise to a both-backend repro, then apply `sev:`/`area:`/`wa:` (ripe bug) or `status:unclear` with a specific ask back to the reporter. Remove `needs-triage` once triaged. |
| `attention` | The bug has been **tried 2+ times with no clear path to a fix** — repeated attempts stalled, the mechanism is still not understood, or every fix tried regressed something else. A flag for "stop grinding solo; this needs a fresh approach or more eyes." | A new diagnostic angle, a different contributor, or escalation — not another identical attempt. |
| `design` | The bug **cannot be solved at all without a user-validated design decision** — the fix hinges on a language/semantics choice only the maintainer can make. Stronger than `needs-design`: `needs-design` means *a* design call is needed (a contributor may propose one); `design` means it is **blocked on the user** signing off. | The maintainer validates a direction; then it usually downgrades to a normal mechanical fix. |
| `by-design` | The reported "bug" reproduces exactly as described but is **intended behavior, traceable to a decision we already made** — the resolution is to *point at the decision*, not to change code. A **closing** label: apply when closing, and cite the [`DESIGN_DECISIONS.md`](../doc/claude/DESIGN_DECISIONS.md) entry (a `C##`) it rests on (record one in the same close if the decision wasn't written down yet). | Nothing — it's terminal. Re-opens only on **new evidence** that invalidates the decision (per the register's "Revisit when"). |

> Rule of thumb: reach for `attention` when you *don't know how* to fix it after
> genuine attempts; reach for `design` when you *can't decide what correct even
> means* without the user; reach for `by-design` when *correct is already
> decided* and the report just rediscovered it.  The three form a lifecycle:
> `needs-design`/`design` (open, decision pending) → `by-design` (closed,
> decision cited).  A bug can carry both `attention` **and** `design` (stuck
> **and** needs a design call).

## Lifecycle (where the fix is, not what kind of bug)

| Label | Meaning | Apply / clear |
|---|---|---|
| `fixed-pending-merge` | The fix has landed on a **long-lived working branch** but is **not yet in `main`** (the release branch).  The issue stays **open** so the tracker doesn't claim "fixed" while released code still has the bug — but it is **not a pick-up**: no agent work remains, only the merge.  The fixing commit's `Fixes #NNN` line **auto-closes it on merge to `main`**, in one clean transition (no manual close → reopen → close ping-pong).  The full lifecycle is **automated off that one trailer**: [`apply-fixed-pending-merge.yml`](workflows/apply-fixed-pending-merge.yml) adds the label (+ a bookkeeping comment) when a `Fixes #NNN` commit is pushed to any non-`main` branch; the merge to `main` auto-closes the issue; [`strip-fixed-pending-merge.yml`](workflows/strip-fixed-pending-merge.yml) removes the label on close (a closed issue is never "pending merge").  So multiple actors can fix bugs on their own branches concurrently with the tracker staying correct. | **Automatic** — just write `Fixes #NNN` in the commit; the label is applied on push.  The author still adds the **substantive comment** (regression test, verified `wa:*`, or a "still unverified" caveat) — the workflow's auto-comment is mechanical only.  Never close such an issue by hand; let the merge close it.  See [ISSUE_TRACKING.md § Issue lifecycle](../doc/claude/ISSUE_TRACKING.md). |

| `status:planned` | The issue is **being worked, but not as a short-term fix**: its fix path is a **PLAN**, and progress is measured there rather than by a fix commit.  For work with no discrete "fixed" moment — a PERFORMANCE class judged against a bar, a multi-phase class fix — where `Fixes #N` never applies cleanly because closing it is a ratio crossing a line, not a commit landing.  The issue stays **open** because it is usually a CONSUMER's surfaced report (`hit-by:`), and their complaint is live until the bar is met; closing it would tell them a resolved thing that is not. | Apply with the plan **named in a comment** — `@PLN157`, or the `doc/claude/plans/<dir>` it lives in.  ⚠ **A `status:planned` issue that does not name a plan is not planned, it is ignored**, and that is the whole difference from `status:deferred`.  Clear it when the plan's last row lands: the work then takes an ordinary `Fixes #N` and the normal `fixed-pending-merge` → close-on-merge lifecycle. |

### Why this label exists: it makes the board self-describing

Without it the board mixes two shapes of work that read identically — **fix-shaped**
(a discrete `Fixes #N`, `fixed-pending-merge`, closes on merge) and **bar-shaped**
(a ratio a plan moves across sessions).  Any goal or query of the form *"no open
issue without a fix"* is then unsatisfiable while one bar-shaped issue is
legitimately open, and its only exits are to **close** or **relabel** the issue —
both of which hide unfinished work from the consumer who reported it, and the first
of which CLAUDE.md forbids outright.

With it the question is one query again:

```sh
gh issue list --state open --json number,labels \
  --jq '.[] | select([.labels[].name] | (contains(["fixed-pending-merge"]) or contains(["status:planned"])) | not) | .number'
```

— *every open issue either has a fix or names the plan that is moving it.*  Nothing
is satisfied by tidying the board.

**It is not a parking label.**  `status:deferred` means nobody is working it (parked,
with an un-defer trigger); `status:planned` means it **is** being worked, on a
cadence a plan sets.  The named plan is the commitment, and it is checkable: open the
plan and read its status.

## `contract:` — what closing it did to the STANDARD

Set at **fix time**, on the issue being closed.  `sev:`/`wa:` describe the bug as
reported; this pair describes the *fix*, and it answers a question nothing else on
the issue does: **did we make the code match a rule we had already written, or did
we have to move the rule?**

| Label | Meaning |
|---|---|
| `contract:settled` | The fix made an **already-written rule hold**.  `doc/claude/formal/` stated what must be true, the code disagreed, and the code changed.  No rule, doc, or design decision moved. |
| `contract:strained` | Closing it **moved the standard** — a rule was added, split, weakened, or reworded; a design decision was taken; a documented promise changed.  The code was not simply wrong against a rule that already covered it. |

Exactly one of the two, on every closed bug.  Neither is a criticism: a `strained`
close is often the more valuable one, because it means the walk reached a case the
rules could not express.

**How to tell them apart, mechanically.**  Look at what the fixing commit changed
under `doc/claude/formal/`:

* nothing, or prose that only records the fix → **`contract:settled`**
* a `(Rule-Name)` line added or rewritten, a deviation entry re-scoped, a
  `DESIGN_DECISIONS.md` entry added → **`contract:strained`**

Worked pair, both closed 2026-08-28.  loft#1138 (an absent nullable struct read as
present across a call) is `settled`: the null model already promised absence
survives a boundary, and `convert` was unwrapping the payload without consulting
the discriminant.  loft#1134 (a nullable tuple element one field high) is
`strained`: closing it required splitting `(L-Null)` — true for every type with a
sentinel, false for an inline struct, which needs a tag — into `(L-Null)` plus the
new `(L-Null-Tag)`.  The rule as written had licensed the defect.

**Why it is worth a label.**  `strained` is the queryable record of where the rules
were incomplete, which is the input the rule-led walk ranks from
([STABILITY_METHOD.md § The rule-led walk](../doc/claude/STABILITY_METHOD.md)) and
what the monthly bug review reads to see whether a class is rising
([BUG_REVIEW.md](../doc/claude/BUG_REVIEW.md)).  A run of `strained` closes in one
area says the specification is thin there, not that the code is.

> **Multi-repo:** `sev:`/`wa:`/cross-cutting are shared across the loft-family
> repos (loft / dryopea / lavition / `loft-lang/*`).  `area:` is loft-specific;
> each game/engine repo defines its own `area:` set in its own `LABELS.md`.
