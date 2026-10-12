<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library compatibility contract — declared floors, verified by the versions they name

> **Status: COMPLETE — steps 0-7 built, and every published package has adopted them.** Step 5 gates only for a
> package that declares a floor; step 5a gates ADMISSION TO THE REGISTRY (not packaging); step 6
> makes the floors change what a consumer resolves to. Steps are ordered so each one lands on
> its own, is useful on its own, and cannot break anything before it.
>
> **All 35 packages across 9 repos now declare measured floors** (step 5b, seeded 2026-07-28),
> so the gates have something to act on. The floors were MEASURED by `loft compat floor`, not
> guessed — a registry of self-referential floors would have been adoption without information.
>
> **Nothing is enforced against a package that declares no floor**, which is why steps 5, 5a
> and 6 could all land at once without breaking a single published package. Declaring is what
> enters the contract.
>
> **Step 0 is BLOCKED and cannot be completed as written** — but its *generalisable* half,
> the retention guard, is BUILT and nightly. See its section.

## The problem, measured

A library can silently break its consumers today. Verified on the current tree:

- **No library repo carries an API baseline** (0 of 8), and `library-ci-reusable.yml` has no
  api-surface step. A signature can change and nothing notices. `api-compat.yml` guards
  *loft's own* surface against baselines bundled in `tests/fixtures/api_compat/`, on PRs to
  loft, and is deliberately non-blocking.
- **Nothing runs a published version's tests against new source.** Library CI runs the
  library's *current* tests — which the same commit may edit, so a break is invisible by
  construction. `revalidate-libs` runs published libs against *new loft*: the other axis.
- **Resolution is version-only.** `registry_index.rs` picks the highest satisfying version
  with no compatibility input, so a consumer asking `foo = ">=0.1"` silently receives a
  release that broke its contract.
- **Two retention holes, at different layers.** `PKG_REGISTRY.md` promises yanked versions stay
  listed so `loft.lock`-pinned consumers don't break. **`web 0.2.2` is absent from its
  `versions` map** entirely, so a lock pinned there cannot resolve *(now guarded — step 0; the
  loss itself is permanent)*. And `shapes 0.1.0` / `glb 0.1.1` are listed correctly yet the
  RESOLVER refused them on an exact pin, so the promise failed anyway *(fixed — step 5b)*.
  Keeping the entry and honouring it are two separate obligations; only the first was written
  down, so only the first was met.

## The invariant

> **A library's declared compatibility floor is a promise about versions that still exist —
> so it must be both verifiable by running what it names, and only ever changed on purpose.**

Two halves, and neither works alone. A floor nobody checks is decoration; a floor whose named
artifacts have been deleted cannot be checked at all.

## Why libraries do not need the absolute promise

loft itself owes **absolute** compatibility: at contract 1 no functioning program ever breaks,
and the language pays for change by carrying both behaviours edition-style
([COMPATIBILITY.md](../../COMPATIBILITY.md)). That obligation covers the **language, stdlib and
runtime** — the floor everyone stands on, which nobody can opt out of and no version of which
can be chosen.

**Libraries are different, and this system is why.** A library breaking its consumers is
survivable as long as two things hold: the break was *declared*, and the *previous version is
still distributable*. A consumer that cannot take the break keeps resolving to the last version
that works for it. So the library's old behaviour is carried by **distribution** rather than by
folding it into the code — which is exactly why libraries never needed the folding machinery
the language has, and why a per-library `contract`-style epoch was the wrong shape.

That is `COMPATIBILITY.md`'s "fold ours, host theirs" made concrete: loft folds, the registry
hosts. It also sets the bar for everything below — the design is not trying to prevent library
breaks. It is trying to make them **impossible to make silently**, and impossible to make
without leaving a working version behind.

## The two numbers

Real versions, not abstract epochs — they mean something on sight, they can appear in
`loft.lock`, and they name artifacts that exist:

```toml
[package]
version              = "0.7.0"
api_compatible_with  = "0.3.0"   # I am still a drop-in for anything >= 0.3.0
data_compatible_with = "0.1.0"   # I still read data written since 0.1.0
```

Separate because the failures differ in kind: an API break costs a recompile, a data break
costs someone's stored file. `hex_terrain` is the worked example — it kept its API and changed
what it computed over stored heights. One number cannot say "safe to upgrade, but migrate".

**Breaking is raising the number**, in the same commit as the break. That edit is the explicit
choice; there is no other way to declare one, and in particular the release version is NOT an
indicator — releases must stay clean and mean only "newer".

## Failure paths (enumerated first, per DESIGN_PROTOCOL)

| # | What happens | Required behaviour |
|---|---|---|
| F1 | API breaks, floor not raised | **Fail.** The unclaimed break. |
| F2 | Floor raised, nothing actually broke | Advice, not failure — a conservative author is not a defect. |
| F3 | The named floor version is not in the registry | **Fail loudly.** An unverifiable claim must never read as a verified one. |
| F4 | Old version's tests fail for a *loft* change, not a library change | Re-classify as environmental (the `revalidate-libs` discipline) or the first language change turns every history red and the check gets switched off. |
| F5 | A random sample fails, the job is re-run, a different version is drawn | **Must reproduce.** Print the drawn version; honour a pin. A real break that vanishes on re-run is worse than no check. |
| F6 | A version's tests can no longer run at all (fixtures gone, dep removed) | Explicit per-version exemption, recorded — never a silent skip. |
| F7 | Window is huge and suites are slow | Bounded per run by construction (see cost), with a budget at the release gate. |
| F8 | Data format breaks, `data_compatible_with` not raised | **Fail** — same rule as F1 on the other axis. |

## Cost, and why it is bounded

Measured today: median 2 published versions per package (mean 2.9, max 10), a pure-loft suite
runs in 0.06–0.12 s, and 6 of 25 packages carry a native crate needing a cargo build per
version. **But today's numbers are a snapshot of a young registry** — mature libraries mean
more versions and slower suites, so the design must not depend on them.

So cost is **O(1) per PR** by construction:

| when | what runs | cost |
|---|---|---|
| every PR | new + floor + latest + **one random interior version** | 4 suites, flat |
| release / ship gate | the full window, under a time budget | O(k), rare |

The random interior sample is what makes it mean: a break cannot hide in a version nobody
looks at. Coverage accumulates — with a 10-version window, ~29 runs puts any given version
under 5% chance of never having been sampled.

Cost is proportional to the *claim*: a library declaring a floor 20 releases back is asserting
something large and pays to verify it; one that raised its floor last release pays nothing.
That is the correct incentive, and narrowing the claim is always available.

**On failure, report the floor the author should have written** rather than merely accusing
them — `loft compat floor` (step 5b). It walks the window from the newest release **downward**
and stops at the first break.

An earlier draft said "bisect" here. That is wrong, and the reason matters: a floor `F` claims
drop-in for *everything* at or above `F`, so a single failure above a candidate disqualifies it.
Compatibility is not guaranteed monotone across a release history — a package can break against
0.2.0 and pass against 0.1.0 — and a bisect would return a floor with a break sitting above it,
which is precisely the unverified claim this design exists to prevent. The downward walk is
O(k) on the failing path instead of O(log k), and that is the correct trade.

## Steps

Each step is independently landable and verifiable. Nothing gates until its noise has been
measured — the lesson of every check in this repo that had to be walked back.

### Step 0 — restore `web 0.2.2` — **BLOCKED: unrecoverable. Replaced by a guard — BUILT**

Investigated 2026-07-28. It cannot be restored, and attempting to would be worse than the
gap.

The history is exact — and reading it with the guard below **corrected** the attribution
recorded on the first pass. `c8893db5` (06-24) added it; `e471bd26` (07-03) yanked it
**correctly**, leaving it listed as the rule requires; the very next index.json commit,
`d8ff94c` (07-03) — *"sign: commit index.json + regenerate index.json.sig"* — dropped the
entry, and that deletion was its **only** index change. Not a later publish, as first written.
So the retention rule was honoured and then lost as collateral of a **signing** commit that
committed a working-tree deletion nobody decided to make.

But the artifact is gone too: the release asset 404s, and **no `web-v0.2.2` tag exists** on
`loft-libs-net`. Source, tag, release and index entry are all erased; only the `yanked` marker
survives, which now *implies* a version that exists and is discouraged when in fact there is
nothing to install.

Recreating it is not an option worth taking: without the tag there is no source to package,
and republishing a rebuild that did not reproduce sha256 `59518a5…` would substitute different
code under a version consumers may trust — a silent substitution is worse than an honest
absence. The one consumer that needed it, `routing`, already survived by **vendoring** the
source into `routing/lib/web/` — which is what people do when distribution cannot be relied on,
and is the clearest evidence available that this gap has a cost.

**What replaces this step — BUILT.** `scripts/registry_retention_check.py`, a nightly job in
`registry-validation.yml`. Restoring one lost tarball was never the point; making the loss
impossible to repeat is.

Two halves, because a version stops resolving in two independent ways and neither check sees
the other's failure:

| half | what it reads | the loss it catches |
|---|---|---|
| history | every revision of `index.json`, oldest-first | a version **leaves** the index |
| artifact | a ranged GET per listed URL | a version is listed but no longer **downloads** |

The nightly matrix could never have caught either: it is built *from* the index, so a deleted
version is simply absent from the work list and no red night appears.

**Measured, and both halves proven able to fail:**

| cell | result |
|---|---|
| real registry, 57 revisions | OK — one drop in the entire history, the known one |
| real registry, 99 listed versions | OK — all 99 resolve |
| a constructed drop in a later commit | **FAIL**, naming the revision it was last listed at and the one that dropped it |
| a listed version pointed at a dead asset | **FAIL**, naming the URL and `HTTP 404` |
| unreadable repo | **exit 2**, distinct from a pass — "the guard is broken" must never read as "the registry is fine" |

`web 0.2.2` is the one recorded exemption, printed on **every** run including green ones: an
accepted loss that stops being mentioned is one nobody remembers is still owed. The comment on
`EXEMPT` states the rule that keeps the list from growing — a dropped version is normally
*repairable*, because the tarball URL and its sha256 are still in the history the check just
read, so exemption is only for a loss repair cannot reach.

### Step 1 — parse the two fields, inert

`api_compatible_with` / `data_compatible_with` in `[package]`, validated as versions, exposed
on the manifest. Nothing reads them. Additive optional fields: a manifest without them parses
exactly as today. **Verify:** both forms parse; a malformed value is rejected loudly (never
silently ignored — the `check_version` lesson, where an unparseable bound was accepted as
"any").

### Step 2 — `loft compat api <published-version>`, advisory — **BUILT**

Fetch that version's source, diff its API surface against the working tree, report
additive-or-not. Reuses `loft api-surface --check`. A standalone command, no CI. **Verify:**
run it across all 25 published packages and read the output; a known break must be reported.

### Step 3 — `loft compat test <published-version>`, advisory — **BUILT**

Runs a published release's tests against the working tree's source, staged together in a temp
directory. Those tests are the only description of the behaviour written *before* this change
existed — and unlike the working tree's tests they cannot have been edited to match the new
behaviour in the same commit, which is why library CI running the CURRENT tests can never
catch a self-inflicted break.

**The control runs first** (F4): the published tests against their own source, establishing the
corpus can pass at all on today's loft. Three outcomes, all reachable —

| control | subject | verdict |
|---|---|---|
| fail | — | `UNVERIFIABLE` — stale corpus, the working tree is not blamed |
| pass | fail | `BREAK` — released behaviour changed |
| pass | pass | `drop-in` |

**Measured, and it decides the design:** of 55 published versions with a test suite, **51 (92%)
still pass their own tests on today's loft**; 4 are stale (`cbor 0.1.1`, `graphics 0.3.0`,
`hex_terrain 0.1.0`, `markdown 0.1.0`). So F4 — "old failures are mostly loft's fault" — is
NOT the common case, which was the condition that would have sunk this whole design.

**The cell that justifies the step:** inverting the result of `arguments::parse` while keeping
its signature reads `API: drop-in` on the api axis and `BREAK` here. An API diff proves the
SHAPE of a surface; only the old tests prove it still does the same thing.

**One bug the matrix caught.** The staged directory must be named after the package: a test
does `use <name>;`, which resolves by DIRECTORY NAME, so `control/` and `subject/` silently
fell back to the installed copy in `~/.loft/registry` — comparing a release against itself and
reporting drop-in whatever the working tree said. It read green on a deliberate break until
the directories were renamed.

### Step 4 — wire both into library CI as **advice** — **BUILT**

`loft compat check` picks the sample (latest + declared floor + one random interior), reports
its draw, and always exits 0. Wired into `library-ci-reusable.yml` with `continue-on-error`.

**The calibration found a real defect, which is what this step is for.** The first sweep
reported `server` as an API BREAK. It was a **false positive in the differ**: `server` 0.5.0
*added* the method `bound` (plus `listen_on` / `listen_tls`), and `api_diff` treated any
textual change to a struct's member list as a break. Wired to block, a purely additive release
would have failed CI.

Fixed with a finer rule than "additions are free", because that is also wrong: an added
**method** is additive, but an added **field** is not — a consumer writing `Server { … }` must
now supply it. Both directions proven rather than argued: adding a field reports
*"added field `extra_field` (a literal construction must supply it)"*, removing a method that
exists in the baseline reports *"removed method `Server.broadcast`"*, and adding a method
reports drop-in. An unparseable member list still reports "shape changed" — the conservative
direction for a check whose value is that a silent break is impossible.

**Result after the fix: 9 comparable packages, 0 breaks, 17 with only one release installed.**
That quiet baseline is the precondition for step 5.

### Step 5 — flip to blocking — **BUILT**

`loft compat check` exits 1 on a violated floor, and `library-ci-reusable.yml` no longer
carries `continue-on-error`.

**It gates only for a package that DECLARES a floor.** Declaring `api_compatible_with` is the
act of entering the contract: a package that declares nothing has promised nothing, so there
is nothing to enforce. That is the model — a library may break its consumers as long as the
break is an explicit choice — and it is also what makes the flip safe, since no published
package declares a floor today, so gating cannot fail anyone's build until they opt in.

**Raising the floor must not buy silence.** The first implementation dropped sub-floor releases
from the comparison, so bumping the number made every check go quiet — the reflex that turns
floors into decoration. Sub-floor releases are now still compared and reported as a
`DECLARED BREAK`. Keeping the promise is the quiet path; withdrawing one is the loud path.

Four cells, all verified:

| source | floor | result |
|---|---|---|
| break | none | advisory, exit 0, nudged to declare one |
| break | below the break | **FAIL**, exit 1 |
| same break | raised past it | exit 0, prints `DECLARED BREAK` |
| unmodified | declared | exit 0 |

### Step 5a — the three levels, required before a register/PR — **BUILT**

A library declares **three** compatibility levels, all real versions:

| field | means | shape |
|---|---|---|
| `loft` | which loft this needs | a **range** (`">=0.8"`) — the platform is the one axis a library does not pick a point on |
| `api_compatible_with` | oldest own release this is a drop-in for | a **bare version** |
| `data_compatible_with` | oldest own release whose data this still reads | a **bare version** |

**All three must be present before a package may be registered.** They are what a consumer
needs to decide whether an upgrade is safe on each of the three axes it can be hurt on: the
platform, the call sites, the stored data — and a consumer cannot ask the package a question.

**The gate is on ADMISSION TO THE REGISTRY, not on packaging.** A library in its current form
must keep building, testing and packaging exactly as it does today — none of that involves
anyone else. Asking to enter the registry is the act that requires the declaration, because
that is the point where other people start depending on the answer.

| surface | behaviour | why |
|---|---|---|
| `loft compat levels` | **fatal** — the gate | asks precisely "may this be registered?" |
| `loft publish` | **fatal** | it emits the registry-PR entry; checked *before* the GitHub-release lookup, so an author learns it while the fix is still a commit rather than after tagging |
| `scripts/registry_maintain.sh` | **skips the package**, before any tag or release is cut | a refusal after the artifact exists leaves something nobody can register, and deleting a release is the move that lost `web 0.2.2` |
| `scripts/vet-lib.sh` | **fails** (gate V5, separate from V1 packaging) | pre-admission vetting is exactly this question |
| `loft package` | **warns, exits 0** | packaging is a library talking to itself |
| library CI | **untouched** | a library that has not opted in must keep passing |

An earlier revision put the fatal check in `loft package`. That was wrong: it would have broken
every existing library's republish before any of them had a chance to declare anything, which
inverts the rule that nothing may break a library in its current form.

All of them call **one** `package::declared_levels` — the N=4 rule below.

Rejections are hand-verified, each cell differing in one line:

| manifest | result |
|---|---|
| all three, floors below the release | entry printed, carrying all three |
| floors == own version (a **first** release) | accepted — the bootstrap the error message tells authors to write |
| any one missing | rejected, **naming that field** |
| nothing declared | rejected with **all three** reported at once |
| `api_compatible_with = ">=0.2"` | rejected — a floor names ONE release; a set names nothing to fetch |
| `api_compatible_with` newer than the release | rejected — claims compatibility with a version that does not exist yet |

All problems are reported together rather than one at a time: each round trip costs a publish
cycle, so a first-failure gate is several cycles of the same two-line edit.

**One defect found while wiring it.** The index entry hardcoded `"loft": ">=0.8"` for every
package regardless of what its manifest declared — so a library needing a newer loft published
an entry saying it did not. Now emitted from the manifest, with the two floors beside it, so a
resolver can read a release's promises straight from the index instead of downloading and
unpacking its tarball (which is what step 6 needs).

**Not covered here — F3, "the named floor version is not in the registry."** That check needs
the index, so it belongs at the registry PR rather than in an offline `loft package`. The
shape-and-bound half above (bare version, at or below the release) is what can be decided
without a network, and it is what rules out the floors that could never name anything.

**The incentive to protect, in both this and step 5:** raising a floor is a promise withdrawn,
and should read like one. The constant updating is the chore, not the default — a library that
raises its floor most releases has taught its consumers that its numbers mean nothing.

### Step 5b — migrate the 34 published packages onto MEASURED floors — **TOOL BUILT, MIGRATION PENDING**

Every gate above is inert at zero adoption: **0 of 99 published versions declare a floor.** The
contract binds nobody until the libraries carry numbers, so this is what turns it on.

**The failure to avoid is a registry full of self-referential floors.** Asked to name a number,
an author writes the version being cut. That is true and claims nothing, and if every package
does it the floors carry no information — the contract would be adopted and worthless in the
same move. So the migration does not ask; it **measures**.

#### `loft compat floor [--with-tests]`

Walks the package's published releases from the **newest downward**, stopping at the first that
is not a drop-in, and prints the two lines to paste.

Newest-downward, not a bisect, and the reason is the invariant: a floor `F` claims drop-in for
*everything* at or above `F`, so one failure above a candidate disqualifies it — even if older
releases happen to pass. Compatibility is not guaranteed monotone across a release history, and
a bisect would cheerfully return a floor with a break sitting above it.

Axes, in increasing cost:

| axis | what it proves | always on |
|---|---|---|
| API surface (`api_diff`) | the shape of the callable surface | yes |
| stored layout (`schema_sidecar`) | a value type's byte layout — a silent DATA break the API axis green-lights | yes |
| the release's own tests | what it *did*, not just what it exposed | `--with-tests` |

The third axis is why the migration should run **with** tests: step 3's justifying cell is a
release that kept `arguments::parse`'s signature and inverted its result — `API: drop-in` on the
first axis, a break on the third. Cost is small (a pure-loft suite runs in 0.06–0.12 s; median 2
published versions per package), and this runs once per package, ever.

A gap in the installed window **stops the walk** rather than being stepped over — a floor must
not claim a version nobody looked at.

Worked example: `crypto 0.3.6` measures back to **0.3.4** (both earlier releases drop-in), so it
declares `api_compatible_with = "0.3.4"` — a real claim, where a self-declaration would have
said `0.3.6` and meant nothing.

#### The migration, in order

1. **Install every published version** of each package so the walk has a window to read (the
   tool reports what it could not see rather than skipping silently).
2. **Run `loft compat floor --with-tests`** per package against its `origin/main`. Record the
   measured floor, the stopping version, and the reason it stopped.
3. **Review `data_compatible_with` by hand.** The tool seeds it from the same measurement, but
   the data axis is about somebody's stored file and only the author knows whether a computed
   value changed under a format that stayed put. `hex_terrain` is the known case.
4. **One PR per lib repo** adding the two lines. Additive manifest fields, so the change is
   inert until the package is next published.
5. **Re-run the sweep** and record the distribution of floor *depth* (how many releases back
   each package reaches). That number is the migration's actual result: a registry where most
   packages reach back several releases means the contract carries information.

**What would say the migration failed:** most packages measuring back only to their own latest
release. That would mean either the libraries genuinely break compatibility every release — worth
knowing on its own — or the measurement is too strict to be useful, and the axes need revisiting
before anyone is asked to declare anything.

#### The measurement — run 2026-07-28, all 34 published packages

Every published version installed (99), every package measured against its `origin/main`.

| outcome | n | packages |
|---|---|---|
| reaches back ≥1 release | **14** | crypto **4**; game_protocol, glb, imaging, pluginabi, server, web **2**; cbor, hex_terrain, hex_world, input, markdown, mesh3d, shapes **1** |
| only one release ever published | 15 | the `hex_*` family, `html`, `ssh`, `hexbody` |
| reaches back to nothing — a declared break is owed | 5 | `arguments`, `gridmesh`, `random`, `regex`, `time` |
| unmeasurable | **0** | — (**loft#656** fixed; `graphics` measures at depth 2) |

**The falsifier did not fire.** The plan said the migration fails if most packages measure back
only to their own latest release. Of the **20** packages with more than one release, **14 reach
back at least one** and 5 reach nothing. **Every package is measurable** — the two that were
not are fixed (loft#656). Depth histogram over those 20: `0`×5, `1`×7, `2`×7, `4`×1. The floors carry information, so the migration is worth
doing.

**The zero-depth packages have changed their API, which is allowed.** `arguments` changed
`Args.error_msg`, `gridmesh` changed `clear_dirty`, `random` changed `rand`, `time` changed
`Duration.to_text`, and `regex`'s main renamed its verbs and dropped `match_groups` / `replace`
/ `replace_all` — deliberately, to dodge the stdlib collisions @C95 made fatal.

**None of that is a defect, and none of it is something to fix.** This design never tried to
prevent library breaks; it exists so a break cannot happen *silently* or without leaving a
working version behind, and the registry keeps every earlier release installable so a consumer
that cannot follow simply stays where it is. What these five are missing is not a repair, it is
a **sentence**: `api_compatible_with = "<this release>"`, which says "a drop-in for myself and
nothing older". That is the supported way to break, and writing it costs one line.

Read the depth number the same way: a shallow floor is a library that moved recently, not a
badly-behaved one. The only failure the contract recognises here is a break that nobody wrote
down.

**A correction to what the unmeasurable packages meant — and one of them is now fixed.** They
were first recorded as "their own main does not parse". That was wrong: `regex`'s main **passes
its own test suite** (7 tests) on the same loft that `api-surface` refused to read, and
`graphics` failed identically on its *published* 0.5.0. The libraries were fine; `loft
api-surface` disagreed with every other way of reading them — **loft#656**.

**Both are fixed** (loft#656), and they were two faults wearing one symptom:

| | what happened | fix |
|---|---|---|
| `regex` | qualifying a call with its own name made the parser re-parse the file it was already parsing, re-registering every definition | a library load must not re-enter the current source |
| `graphics` | with another library loaded (`use mesh3d; use glb;`), the main file's own name resolved to nothing at all — `use_names` never holds the main file | a package's own name resolves to its own source |

The second only shows up once *another* library is in play, which is why the first fix did not
cover it and why a probe without a dependency could not reproduce it. Both now measure:
`graphics` at depth 2 (back to 0.4.2), `regex` at depth 0.

It mattered beyond the tool, because `compat api` /
`compat floor` / `compat check` all route through the same `api_surface_of`, so the contract
reports `UNMEASURABLE` for these packages for a reason that has nothing to do with them.

**Two findings the sweep produced, both fixed in the same commit:**

- *`compat floor` blamed the wrong side.* A failure to read the WORKING TREE's surface was
  reported as "could not read its surface" against the published release being compared, which
  is a claim about somebody else's shipped version. It also failed identically at every step, so
  a package that was never compared at all read as "reaches back to nothing". The working tree
  is now read ONCE before the walk; failing there is `UNMEASURABLE` and measures nothing. That
  reclassification is exactly what moved `graphics` and `regex` out of the zero-depth row.
- *A yanked version could not be installed by exact pin.* `PKG_REGISTRY.md` keeps yanked
  versions listed so a `loft.lock` pinned to one still resolves — but `find_best_version` skipped
  yanked unconditionally, so `loft install glb@0.1.1` refused a version the index plainly
  carries. **The same retention promise `web 0.2.2` broke, one layer down**: there the entry was
  deleted, here the entry survives and the resolver refuses it anyway. An exact pin now resolves
  a yanked version; a range or `*` still skips it, so nothing new picks one up by accident.
  Restoring those two windows deepened `glb` from 1 to **2** (it can now see `0.1.1`); `shapes`
  was unchanged, since its window still stops at an unparseable `0.2.0`.

**A floor stopped by "its published source no longer parses" is a LOWER BOUND**, not the answer:
`server`, `cbor`, `hex_world` and `shapes` each hit a release that no longer parses on today's
loft, so their real reach may be further back. That is failure path F6 on the API axis, and the
tool reports it rather than stepping over it — a floor must never claim a version nobody could
look at.

#### The `--with-tests` pass — run 2026-07-28

Same 34 packages, now with each candidate release's own published suite run against the working
tree. **Result: every floor is unchanged**, and the axis is worth having anyway — 17 of the
version comparisons now rest on behaviour evidence rather than shape alone.

| verdict | n | meaning |
|---|---|---|
| `drop-in` | 17 | the release's tests ran and still pass — real evidence |
| `UNVERIFIABLE` | 5 | its suite no longer passes against its OWN source, so it judges nothing |
| **`BREAK`** | **0** | tests pass on their own source and fail here |

**F4 turned out to be live, not hypothetical, and it caught a defect in this tool.** The first
implementation stopped the walk on *any* non-pass, including `UNVERIFIABLE`. That moved five
floors — `cbor`, `game_protocol`, `hex_terrain` and `markdown` collapsed to *nothing*, `imaging`
went shallower — and **not one of those was a behaviour break**. Every one was a stale corpus:
`game_protocol 0.1.1`'s test file uses `use game_protocol::(a, b, …)`, a multi-name import loft
no longer accepts. Left alone, each loft language change would quietly shorten every library's
history, which is exactly how a check earns its way into being switched off.

Corrected to the rule F4 implies: **only a `BREAK` lowers a floor.** A `BREAK` is evidence about
the *library*; `UNVERIFIABLE` and "could not run" are evidence about the *environment* and say
nothing about compatibility. The API and layout axes still verified those releases, so the claim
stands — and the versions the behaviour axis could not speak for are **named in the report**, so
nobody reads the floor as more thoroughly verified than it is:

| package | behaviour axis silent on |
|---|---|
| `cbor` | 0.1.1 |
| `game_protocol` | 0.1.1, 0.1.0 |
| `hex_terrain` | 0.1.0 |
| `imaging` | 0.1.0 |
| `markdown` | 0.1.0 |

Those five are a finding in their own right, on the other axis this plan does not own: published
artifacts whose tests no longer run on today's loft. Pre-contract-1 that is permitted, but it is
the population contract 1 has to stop growing.

**The floors are ready to seed.** Both passes agree, and the numbers now carry behaviour evidence
wherever a corpus could still run.

#### Seeded — 2026-07-28

**Done.** All 35 packages across 9 repos now declare both floors, pushed to each repo's `main`
(one commit per repo, `loft.toml` only, additive).

Validated before pushing: every one of the 34 published packages passes **both**
`loft compat levels` and `loft compat check` with its floor applied, so the seed reddens no
library's CI. It could not have anyway — `origin/main`'s reusable still carries
`continue-on-error: true` on the compat step and main's `compat_check` always returns 0, since
the gating half (step 5) is not merged yet. That ordering is deliberate: the numbers land
first, the gate follows.

Two things the push surfaced:

- **`web` had already declared its own floor** (`loft-libs-net@3876149`), at `0.3.0` — the same
  number the measurement independently arrived at. Left untouched. An author and the tool
  agreeing on a floor with no coordination is the best evidence so far that the measurement
  says something real.
- **`hexbody` declares no `loft` field at all.** Pre-existing, and it is the one unpublished
  package. Its two floors are correct but it still cannot be registered until someone chooses
  its `loft` range — an owner's call, not a measurement.

Remaining for the contract: step 7, and merging the gating half so the floors start being
enforced.

### Step 6 — resolution honours the floors — **BUILT**

This is where the contract stops being paperwork: the floors now change what a consumer
actually gets. `registry_index::find_compatible_version` picks the newest satisfying release
that still declares itself a drop-in for what the consumer holds, rather than simply the
highest — the one computation, called by both `loft update` and `loft install` (the N=4 rule).

The two floors travel **in the index** (step 5a emits them), so resolution reads a release's
promise without downloading and unpacking its tarball.

A candidate `R` declaring `api_compatible_with = F` promises it replaces anything from `F`
onward, so it is safe for a consumer on `held` exactly when `F <= held`.

**It never silently stops.** Withheld releases are returned alongside the choice and named —
`loft update` prints *"0.5.0 held back: declares a break past 0.2.0"*, `loft install` a
`note:`. A resolver that quietly settles on an older release teaches its consumer that no
upgrade exists, which is the opposite of what a DECLARED break is for.

Three cases mean unconstrained, and all three are deliberate:

| case | why |
|---|---|
| no held version (a **fresh install**) | nothing to be a drop-in *for*. Constraining would hand a first-time user an ancient release because the library broke compatibility three versions ago — they have no old call sites to protect |
| the candidate **declares no floor** | it has promised nothing, so nothing is enforced. This is what keeps the change **inert for every version published today** |
| the constraint is an **exact pin** | it names one release, so the caller already chose. Filtering it would report "no version satisfies the constraint" for a version that plainly exists |

Verified on a constructed index (`lib` raises its floor at 0.3.0, `legacy` declares nothing):

| held | resolves to | withheld |
|---|---|---|
| — (fresh) | 0.4.0 | — |
| 0.1.0 | **0.2.0** | 0.4.0, 0.3.0 |
| 0.2.0 | **0.2.0** | 0.4.0, 0.3.0 |
| 0.3.0 | 0.4.0 | — |
| 0.4.0 | 0.4.0 | — |
| 0.1.0 of `legacy` | 0.4.0 — *identical to the pre-step-6 answer, which is the actual claim* | — |
| 0.1.0, pinned `"0.4.0"` | 0.4.0 | — |

Proven non-vacuous by forcing the break test to `false`: `resolution_honours_declared_floors`
fails, `resolution_is_inert_without_declared_floors` still passes — which is the right split,
since only the first is claiming the floors do anything.

One consequence worth stating: `loft update --check` exits 0 when the ONLY lines are
held-back notes. A consumer correctly staying put must not turn a CI check red — that is the
pressure that gets a floor ignored, or the gate removed.

### Step 7 — the release gate runs the full window — **BUILT**

`loft compat check --full` verifies a package's **entire** claim — every installed release,
oldest first — under a wall-clock budget (`LOFT_COMPAT_BUDGET`, default 600s). Wired into
`registry_maintain.sh`'s publish path, before the tag and GitHub release are cut, for the same
reason the admission gate is: a refusal after the artifact exists leaves something nobody can
register.

**Two different questions.** A PR asks *"did this change break something"* and pays O(1) for the
answer — latest + floor + one random interior release. Publishing asks *"is everything this
package promises actually true"*, and that is the only one of the two that is a promise to
people who cannot ask the package a question. Sampling answers the first honestly and the
second not at all.

**Overrun is a FAILURE, never a truncation.** The tempting alternative — verify what fits,
report green — produces a release claiming a floor it never checked, which is worse than no
check at all because it carries the authority of one. Cost is proportional to the *claim*, so
the remedy is in the author's hands and the message names it: narrow the floor, or split the
suite. Releases are walked **oldest first**, so a timeout leaves the deepest part of the claim
— the part the floor is actually asserting, and the part nobody else looks at — already proven.

| cell | exit |
|---|---|
| full window, claim holds | 0, *"whole claim verified"* |
| budget exhausted | **1**, naming how many went unchecked and what to do |
| break at or above the declared floor | **1** |
| break BELOW the floor | 0, reported as `DECLARED BREAK` — the promise never covered it |
| a release that cannot be read | named as NOT verified, never silently dropped |
| sampled (per-PR) mode | unchanged |

Verified against all 34 published packages: every one passes the full window, so publishing
stays unblocked. The regression test drives the real binary — the property is an **exit code**,
and a test of the message would have passed while the gate returned 0 — and is proven
non-vacuous by making the gate truncate-and-pass, which fails it.

## Re-assertion sites — the brittleness count

Four places must agree on "is this a break": the CI check, the release gate, the resolver, and
`loft compat` run by hand. **N = 4, and a miss at any of them is silent** — which is the
alarm DESIGN_PROTOCOL step 2 exists to raise. The cure is that all four call **one**
computation (the `loft compat` core), never their own. If any step is tempted to re-derive it,
that is the signal to stop and route it back through the shared one.

## What would falsify this design

- **F4 turns out to be the common case.** If most old-version test failures are loft-caused
  rather than library-caused, the check is noise wearing a contract's clothes, and step 4's
  measurement is where that shows up — before anything blocks.
- **The random sample proves unreproducible in practice.** If pinning does not actually make a
  red re-runnable, the check will be treated as flaky and ignored; F5 is the reason the pin is
  a requirement and not a convenience.
- **Authors raise the floor reflexively** to silence the gate, making every release a declared
  break and the floors meaningless. Watch the rate of floor raises after step 5; if a library
  raises on most releases, the check has taught the wrong lesson.
