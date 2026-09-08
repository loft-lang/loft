<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library authoring

End-to-end guide for publishing a loft library to the package
registry.  Walks through scaffolding a fresh library →
developing the API → packaging + publishing → maintaining
(yanking when a vulnerability surfaces).

This consolidates the author-facing surface that lives across
[`PACKAGES.md`](PACKAGES.md), [`PKG_REGISTRY.md`](PKG_REGISTRY.md),
and the `lib_plans/12-library-extraction/` topical files into
a single narrative.  The CLI commands referenced here all live
in the loft binary — no extra tooling install needed.

---

## Who a library belongs to — the standing rule

**A library is used by MULTIPLE first projects, any of them may ADD to it at
will, and the one obligation is to keep the library contract in place.**

There is no single owning project and no gatekeeper to ask.  A consumer that
needs something the library does not yet do adds it, in the library, and every
other consumer gets it.  That is the normal way these libraries grow — not an
exception to be justified.

**And a library is written IN loft, readable first.**  The libraries double as the
project's teaching corpus, so the "fast pass" pattern — readable code shadowed by an
optimized native twin nobody can follow — is refused as a matter of contract: a routine
that measures slow is fixed in the ENGINE (or in its own loft algorithm), and a native
rewrite is a per-routine edge case with its reason recorded beside it
([formal/performance.md](formal/performance.md) `(Perf-Cure)`; the measuring pass itself
is `(Perf-Weight)` — every routine against an industry-language reference twin,
per release).

Three things follow, and they are the whole rule in practice:

- **Adding is free; breaking is not.**  New functions, new types, new optional
  parameters, wider accepted input — all additive, all fine without ceremony.
  Changing what an existing call already promises is the one move that needs
  the compatibility process ([COMPATIBILITY.md](COMPATIBILITY.md)), because
  the other consumers did not ask for it and will not survive it.
- **Whichever project gets there first is the author.**  `hex_grid` and
  `hex_terrain` entered [`loft-libs-world`](https://github.com/loft-lang/loft-libs-world)
  from **crawler**, not from the game the plan expected — that is the rule
  working, not a process failure.  Do not read "another project wrote it" as
  drift.
- **A private copy IS the failure mode.**  A consumer keeping its own
  version of shared behaviour is the thing to fix, because it splits the
  contract in two and neither half gets the other's fixes.  Fold it in
  instead of forking it.

The corollary for planning: a plan that waits for "a second consumer to
justify extraction" has the economics backwards.  The library exists so that
several projects share one contract; the second consumer is the *point*, not
the *permission*.

---

## Quick reference

```
loft new <name>           # scaffold a fresh library skeleton
loft new <name> --native  # also creates native/ cdylib skeleton
loft new <name> --chunk   # also creates .github/workflows/library-ci.yml
loft test                 # exercise tests/
loft package              # build a deterministic tarball
loft publish              # emit registry-PR index.json entry
loft yank <pkg>@<ver>     # emit yank PR blocks (security advisory)
```

The author loop, in five steps:

```
loft new my_lib                       1. scaffold
$EDITOR my_lib/src/my_lib.loft       2. develop
cd my_lib && loft test                3. test
git tag + push + gh release create    4. release source + binary
loft publish                          5. submit to registry
```

**Maintaining the loft-lang family libs** (`loft-libs-*`) — run from a
`loft-lang/loft` checkout, not a single package dir:

```
scripts/registry_maintain.sh   # publish every stale/missing own lib, sign the index, push
scripts/registry-sign.sh       # re-sign the index after a hand-merge (review-then-sign)
scripts/sync-fixtures.sh       # regenerate tests/fixtures/libs/<pkg>/ from the pinned tags
```

These automate the publish + sign + fixture steps (§4–§5) for our own
libraries; the per-package `loft publish` flow below is the mechanism they
wrap (and the path external contributors use — [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)).

---

## 1. Scaffold a fresh library

```
$ loft new my_lib
Created library `my_lib/`:
  loft.toml
  src/my_lib.loft
  tests/01-smoke.loft
  README.md
  release.sh
```

`loft new` validates the name (lowercase ascii + digits +
underscore), refuses a **reserved namespace name** (`std`, `core` — these resolve to a
built-in namespace, not a package; see [C101](DESIGN_DECISIONS.md)), and refuses to
overwrite an existing dir.

`release.sh` (executable) is a one-command release: it reads name + version
from `loft.toml`, runs the test gate + a deterministic-package check, commits any
version bump, tags `<name>-v<version>`, pushes, packages, and `gh release
create`s.  `./release.sh` releases the current version; `./release.sh 0.2.0`
bumps first.  It refuses to re-cut an existing tag (releases are immutable —
bump instead).  Automates §4a–4b; afterwards an own lib is picked up by
`registry_maintain.sh`, an external lib opens a registry PR.

Flags:

- `--native` — adds `native/Cargo.toml` + `native/build.rs` +
  `native/src/lib.rs` for the Rust cdylib.  Cargo deps already
  point at the registry versions of `loft-ffi` /
  `loft-ffi-build` — the scaffolded native crate works
  immediately on `cargo build --release` post-publish.
- `--chunk` — adds `.github/workflows/library-ci.yml` with the
  canonical CI template (mold + loft source build +
  `loft --interpret --tests tests` + `loft --native --tests
  tests`).  Use when starting a fresh `loft-libs-<family>/`
  chunk repo whose first member is this library.

What the scaffold contains:

- `loft.toml` — `[package]` (name + version 0.1.0 + loft >=0.8)
  + `[library]` (entry = src/<name>.loft) + empty
  `[dependencies]`.
- `src/<name>.loft` — placeholder `pub fn hello() -> text`.
- `tests/01-smoke.loft` — single `test_hello` that asserts the
  placeholder's return value.  Passes immediately, so `loft
  test` is green from the first invocation.
- `README.md` — install + usage snippets.

## 2. Develop the API

Edit `src/<name>.loft`.  Add `[dependencies]` to `loft.toml`
as you need them (`loft install <dep>` to auto-populate the
lockfile).  Add more tests under `tests/`; loft discovers
every `fn test_*()` in `.loft` files there.

Resolution chain for `use X;` in your source:
1. Sidecar `<script>.loft.lock` (when you've `loft pin`'d a
   one-file script).
2. Walk-up `loft.toml` + adjacent `loft.lock` (the project
   mode you get from `loft new` — most common).
3. Cwd `loft.lock` (script-mode fallback).
4. Auto-install from the registry (when nothing else
   resolved).
5. Flat fallbacks.

Run tests:

```
$ loft test                            # interpreter, all tests in tests/
$ loft --native test                   # native codegen
$ LOFT_DENY_WARNINGS=1 loft test       # strict — fails on any warning
```

The `.allow_warnings` opt-out file at the package root lets a
package temporarily ship with warnings; remove it once the
warning count reaches zero.  Chunk CI flips the
`LOFT_DENY_WARNINGS` env to `0` when the opt-out file is
present.

`.wasm_exempt` is its sibling, and the same shape: a package
whose `[native] crate` cannot cross-build for `wasm32-wasip2`
puts one at the package root, with the REASON as its contents.
Without it, CI cross-builds every native crate and a failure is
red — because one dependency lacking a wasm32 target takes the
whole package off `--native-wasm`, the parts that never wanted a
device included.  Prefer fixing it to declaring it: usually only
one or two dependencies are the problem and they can be moved
under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
([WASM.md § When the crate needs a device the target does not
have](WASM.md)).  An exemption is right where the unbuildable
part IS the package — the TLS in `server` / `web` / `ssh` — not
where it is a device the package merely touches.

**Your warning count moves even when you don't touch the code** —
loft keeps adding deprecations (`not null`, the `&`-parameter
lint, null-flow), so a package that was clean at publish time
warns against a newer loft.  Nothing fails while you leave it
alone, and then your next PR goes red on code you never edited.
The nightly `revalidate-libs` job reports this in advance, per
package, in two columns:

| column | what it measures | how you clear it |
|---|---|---|
| **published** | the warnings a *user* of your library sees today | publish a fixed version |
| **source** | what your own CI will do on your next PR | clean the source on `main` |

Read it in the job summary of the latest `revalidate-libs` run
(loft repo → Actions), or reproduce a single reading locally:

```
$ LOFT=target/release/loft \
  scripts/lib_warning_scan.py scan <pkg-dir> --label source
```

### 2a. Worked examples — point at a real call site

When a function's **correct use is not obvious from its signature and doc**, point
the reader at a **real call site** instead of a prose snippet.  A snippet rots
silently at the first signature change; a tagged test cannot drift, because it **is**
working code that runs every CI.  This is the shared `@PLN141` convention — one tag
family, one gate, across the whole loft library ecosystem.

**Scope — the whole discipline.** Tag a function **only where a real call site
teaches more than its signature does**.  A one-line accessor, a function whose use is
self-evident from its type, needs none — the doc already suffices.  There is no
retroactive sweep of every `pub fn`; the gate would go red on hundreds of obvious
functions the day it landed.  The signal for *which* functions owe an example is
concrete: **a reader — often an AI agent — who knows the function exists but cannot
use it from its doc alone, and only succeeds once shown a program that already uses
it**, is exactly the case that owes a worked example.  Lift that usage into a tagged
test so the next reader never needs the pointer.

**How it works — two halves and a registry.**

- **Citation** — a comment `// Example: @AAA-###` directly above the documented
  `pub fn` in `src/`.
- **Definition** — the tag `// @AAA-###` in the comment block directly above the `fn`
  that demonstrates it: a `test_*` in your `tests/` (author one in retrospect if none
  is clear), or a real function in a first-class application's own source.
- **Naming a tag in prose** — a comment block directly above a `fn` *defines* the
  first tag it names, so a passing mention there ("see @ARG-004 for the failure
  path") would claim that tag.  Two rules keep ordinary cross-referencing safe: the
  **first** tag in a block is the one it defines, and a block containing an
  `// Example:` line is a **citation** — it defines nothing, before or after that
  line.  So mention a sibling freely inside an example's own prose or under a
  citation; just never open a block above a `fn` with a tag you do not mean to
  define.
- **Acronym** — `AAA` is three uppercase letters, hyphen, three digits — distinct
  from loft's `@P`/`@PLN`/`@F`/`@GH` families (none has that interior hyphen).  Each
  acronym names **one repo, ecosystem-globally**, registered once in loft's
  [`scripts/example_repos.tsv`](../../scripts/example_repos.tsv) (monorepos map several
  acronyms to one repo).  A citation can point *across* repos and is still validated +
  linked; a foreign repo is read-only, never a build edge.
  **Register the acronym in the same pass that writes the first tag** — the
  registration lives in a different repo from the tags, so it is the step that gets
  forgotten, and an unregistered acronym is a hard `unregistered` fault on every
  citation using it.  `loft-libs-game`'s `fixstep` carried thirteen tags for weeks
  with no `FIX` row; nothing said so, because the gate was not yet live in library
  CI.  If your CI is green and you never touched the registry, that is not evidence.
  It stopped being hypothetical on 2026-08-18: loft #971 made the gate live, and
  `loft-libs-game`'s `main` went red on those thirteen citations the same day, with
  `loft-libs-graphics` red beside it on an `examples-index.tsv` that was one line
  stale.  Neither repo had changed — the CHECK arrived.  So the rule has a second
  half, for whoever turns a ratchet on: **re-run the new gate against every repo that
  already adopted the convention, on the day it goes live.**  Until then those repos
  were green on a promise, and a promise and a check look identical in a CI badge.
  The gate also runs on `pull_request`, so an unregistered acronym reddens the very PR
  that would fix it: the registry row must merge FIRST, before any library PR carrying
  its tags.

**The gate runs in your CI already.**  Every `loft-libs-*` `library-ci.yml` is a thin
caller of loft's reusable workflow, which checks loft out into `loft-src/` — so the
shared gate `loft-src/scripts/check_doc_drift.sh examples` and the acronym registry
are present in your CI run with no per-repo copy.  It is **vacuously green until you
author your first citation**.  Faults: `dangling` (cited, no fn carries it),
`duplicate` (one tag on two fns here), `unregistered` (acronym missing from the
registry).

⚠ **It ADVISES here, it does not gate** — the findings go to your PR's job summary in
full, with the cure, and cannot block a merge.  The rules arrive from whatever loft
`main` is, so a gate would redden your PR for a change loft made; and by loft's own
tier rule a dangling doc citation cannot produce a wrong result.  It still gates inside
loft.  See [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) § Why a by-hand pass exists.

**To get a pass/fail back before you push**, from a loft checkout:

```
make examples-preflight REPO=/path/to/your-library
```

It gates exactly the citation faults CI would report — `dangling`, `duplicate`,
`unregistered` — and exits non-zero on any of them.  ⚠ It deliberately does NOT demand
an `examples-index.tsv`: libraries no longer commit one, so a preflight that insisted on
the file would fail for the correct state.  With no `EXAMPLES_CITE_ROOTS` set it scans
your `*/src` and `*/tests`; if it finds **zero** citations it says so rather than
reporting green, because a check that examined nothing is not a pass.

⚠⚠ **Do not commit `examples-index.tsv` — CI builds it.**  The index (`tag ⇥ file:line
⇥ fn ⇥ git-link`) is DERIVED, and its generator lives in loft, so a copy committed here
can only rot: you cannot regenerate it where it sits.  CI now emits it every run,
publishes it folded into the job summary and uploads it as an artifact, so a derived
file that is never committed cannot be stale.  A leftover committed copy is not an
error — CI says once that it is safe to delete.

⚠ One thing DOES read a leftover copy, and it is built not to depend on it: with no
sibling checkout, `EXAMPLES_ONLINE=1` fetches the owning repo's committed
`examples-index.tsv` to resolve a cross-repo citation it could otherwise not check at
all.  It may **confirm** a tag and never **refute** one, precisely because that file is
being retired — so deleting yours costs a link, never a red gate.

**Recording that a package owes nothing.**  Because there is no retroactive sweep, an
untagged package is ambiguous: nobody can tell *no function here needs an example* from
*nobody has looked yet*.  Resolve it in one line in `examples-exempt.tsv` at your repo
root — `package ⇥ exempt|deferred ⇥ reason` — where **exempt** means no function teaches
more from a call site than from its signature, and **deferred** means one does but not
yet, with the reason naming what unblocks it (the monthly review picks those up).
Nothing gates on this file; it is what lets a whole repo be called done.
`make examples-progress REPO=../<your-repo>` in a loft checkout lists every package as
tagged / exempt / deferred / TODO.

The automated gate only sees a citation that *dangles* or *duplicates* — staleness
(still resolves, no longer matches the code) and quality (valid but no longer the
clearest) are caught by the monthly by-hand pass in
[LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md).

## 2a. Declare which side of the sandbox boundary the library is on

Decide this **while the API is being written**. @PLN86 shipped admission control
(`src/sandbox.rs`, `[sandbox]` in `loft.toml`): a designated function is admitted only if
proven safe at load — capability, termination, data integrity, backend. A library is therefore
either

- **trusted engine** — internals may be unbounded, and it exposes an API an admitted caller
  can reach; or
- **admissible loft** — the library's own code passes admission.

The choice decides which internals may loop without a bound, so it is a **property of the
API**, not a deployment flag. Made up front it costs a sentence; made afterwards it is a
re-architecture. Getting it right is also what makes user scripting cheap later — a mod is
then just more admitted code, with no second code path to keep in step, and the negative gate
is writable: **remove a capability from the policy and the corresponding call must fail to
load.**

See [SANDBOX.md](SANDBOX.md).

## 2a1. A surface proven only by its own tests is a surface nobody has agreed to

Before a `pub fn` is published, ask whether anything **outside the package's own test suite**
calls it. Not *"is it tested?"* — *"is it used?"*

⚠ **These are different questions and only one of them is usually asked.** A library grown
beside its own tests accumulates `pub` for reachability: a helper is made public so a test can
get at it, and it stays public for ever afterwards. The `pub` list then describes the **test
suite's** needs rather than a consumer's, and nothing in a green run says so.

**Measured, 2026-08-20, in `lavition_ui`** (moros's, and their agent's own count while
answering @PLN145's `D0` request): **15 of 31 public functions had no production caller.**
`panel_hit_test` — the one function @PLN145 asked to depend on — was *"built, tested green and
invoked by nothing"*, in that tree's own words.

⚠⚠ **The follow-up is stronger than the measurement, and it is why the bar is not a style
rule.** moros gave `panel_hit_test` a caller the next day, and the commit subject is the
finding: *"A click on the panel turned the camera, because `panel_hit_test` had no caller."*
The function was not merely unused — under the one consumer that finally called it, the
program was **wrong**, and no amount of its own green tests could say so, because a test
asks *does it answer what I expect* and a consumer asks *is this the question I have*.
Re-counted 2026-08-21: **13 of 33** public functions still have no production caller, and
that tree's README now names all thirteen and calls them *a proposal*.

So a promotion check of the form *"its own tests pass unchanged after the move"* verifies the
**move** and says nothing about whether the surface was ever **honoured**. Both are needed:

> **The bar:** every public function a consumer will depend on has at least one **production**
> caller in the tree that owns it, or is explicitly documented as a surface built *for*
> consumers with none yet.

The second clause is not a loophole — it is the honest label for a package published ahead of
its first consumer, and writing it down is what stops *"tested"* being read as *"agreed"*.

⚠ **This cuts our own trees too.** A library built in `loft-libs-*` before any game consumes it
is in exactly that position: every caller is a test. That is not a reason to delay it — the
dogfood loop deliberately builds the library first — but it IS a reason to say which functions
have never been used in anger, so a consumer knows which parts of the surface are a proposal.

## 2a2. `use self::<m>` and a `m::` qualifier are mutually exclusive

loft#976's cure is `use self::<module>;` for a module your own package ships, and the
compiler suggests it. ⚠ **It cannot be applied to a file that QUALIFIES that module.**
`use self::x;` binds `x`'s names bare and gives no `x::` qualifier — deliberately, since
that slot is shared by the whole dependency graph — so a file writing `x::f()` must either
drop the qualifier or bind one with `use self::x as <alias>;`.

Measured (moros, 2026-08-20): a tree-wide rewrite of **129 `use` lines across 39 files and
7 packages** broke exactly the two files that qualified their own module, and the failure
arrived as `Unknown library 'surfaces'` plus cascading `Unknown variable` errors — which
reads as *that module is missing* rather than *bound under a name you cannot qualify*.
loft#1043 fixed the diagnostic; the planning point stands: **budget for the qualifying
files before starting the sweep**, because nothing marks them until they fail.

⚠⚠ An alias re-enters the shared slot, so choose one no other package would take
(`<pkg>_<module>`), not the module's own name — otherwise the sweep hands back the
capture it was performed to prevent.

## 2b. Never leave a capability in the fixture only

`tests/fixtures/libs/` snapshots each chunk repo at a pinned tag. A fixture that is **behind**
its tag is ordinary and recoverable — re-sync. A fixture carrying a **file the canonical repo
has never had** is a fork: nothing recovers it, an installed package silently lacks whatever it
provides, and the fixture's own tests stay green while saying nothing about that.

That is not hypothetical. @PLN106's Android GL backend (`android_gl.rs`) lived in the graphics
fixture and on no branch of `loft-libs-graphics`, so `--native-android` shipped while
`loft install graphics` had no Android rendering, input or audio at all — and the goldens that
proved it proved the fixture.

`scripts/sync-fixtures.sh --check-unreleased` gates this (CI job `fixtures-unreleased`, unlike
the advisory full-drift check). Either take the file upstream, or declare it in that script's
`UNRELEASED_FILES` **with a tracking reference** — a row without one is refused, since knowing
about it is what the previous arrangement already had.

## 2c. The guide — `docs/01-getting-started.loft`

A reference tells a reader what a function's type is. It does not tell them **where to
start**, and those are different questions asked by different people: one has already decided
to use the library, the other has not. The API reference is generated and complete; the guide
is written and short, and only one of them can be either.

**One file, in the package, in the topic format the language pages use** — `@NAME` / `@TITLE`,
prose in `//` comments, code between. That format is not decoration: the guide is a **running
loft program**, so an example that stops working stops compiling. A guide that cannot rot is
the only kind worth a reader's trust.

**Your CI runs it, in its own step.** Not `loft test` — that scans `tests/`, and a guide is a
program, not a suite, which is why the guides written before this rule existed were never
executed by anything. The `Guide` step in `library-ci-reusable.yml` runs every `docs/*.loft`
on **both backends** and diffs the two outputs; a package with no guide says so and passes.
Run it yourself the same way:

```sh
LOFT_DENY_WARNINGS=1 loft --interpret docs/01-getting-started.loft
LOFT_DENY_WARNINGS=1 loft --native    docs/01-getting-started.loft
```

Warnings are denied there as they are in the suite, so a guide that teaches an idiom the
compiler warns about does not ship.

It is rendered in two places from that one file, with no further work: `loft doc <name>`
locally, and `doc/lib-<name>-guide.html` on the published site, linked from the library's
card. Writing the file is the whole of what it takes to appear there.

### What goes in it, in order

1. **What it is, in one paragraph — and what it is not.** Name the neighbouring library a
   reader might have wanted instead. This is the paragraph that saves the wrong reader ten
   minutes, and it is the one most guides skip.
2. **The smallest complete program that does something**, with its real output. Not a
   fragment: something a reader can paste and run.
3. **The three or four calls that carry the library**, each with a worked example. Not the
   whole surface — that is the reference's job, and duplicating it here creates a second home
   that drifts.
4. **The one thing that surprises people.** Every library has one. `graphics`'s is that
   `rgb()` and a hand-written hex literal differ in the alpha byte; `markdown`'s is that three
   of `render`'s four arguments are URL rewriting and `""` turns each off. It is invisible
   from the signature, which is exactly why the reference cannot carry it.
5. **Where to go next** — the reference, and the package's own tests, which are the
   best-explained calls in it.

### `main` must call every section

Split the guide into a function per section if you like — it reads better than one long
`main`. But **`main` has to call each of them**, because nothing else does. A zero-parameter
function that no one calls still compiles, so a renamed API is still caught; its **asserts
never run**, and those are the half that checks the example is *right* rather than merely
*spelled correctly*.

Measured, not hypothetical: `graphics`'s guide defines `drawing_shapes()` and `aa_example()`
and `main` calls neither, so two of its three sections have never executed an assertion. The
page reads as verified and one third of it is.

### What it is not

Not the reference: no attempt at completeness, and no per-function catalogue. Not the README
either — the README is what GitHub shows and is read before installing; the guide is read
after, by someone with the package in hand. Where they overlap, the README is the shorter one.

## 3. Pre-release checklist

**Declare your three compatibility levels first.** They are required before a package may be
registered or open a registry PR, because they are what a consumer needs to decide whether an
upgrade is safe on each axis it can be hurt on:

```toml
[package]
version              = "0.7.0"
loft                 = ">=0.8"    # which loft this needs
api_compatible_with  = "0.3.0"    # oldest release of THIS package it is a drop-in for
data_compatible_with = "0.1.0"    # oldest release whose stored data it still reads
```

All three are real versions, so each claim is checkable: `loft compat check` fetches the
release a floor names and runs that release's own tests against your working tree.

Two checks, asking different questions. `loft compat check` runs on every PR and pays O(1) —
the latest release, your declared floor, and one random release in between. `loft compat
check --full` runs at PUBLISH time and verifies the whole claim: every release, under a time
budget. If the budget runs out the release FAILS rather than reporting a partial pass, because
a release that claims a floor it did not check is worse than one that claims nothing. The cost
is proportional to your claim, so narrowing the floor is always the available remedy.

`loft package` and `loft publish` **refuse to emit a registry entry** without all three, and
report every missing or malformed one at once rather than one per attempt. For a **first
release** the two floors are that release itself (`api_compatible_with = "0.1.0"` on 0.1.0) —
trivially true, and the natural starting point. If you only want the tarball and are not
registering anything, `loft package --tarball-only` skips both the entry and the check.

**Raising a floor is how you declare a break** — it is the only way, and the release version is
deliberately not an indicator. It is also meant to be rare. The number is a promise to your
consumers, not a per-release chore: a library that raises it most releases has taught its
consumers that its numbers mean nothing. Keeping it still is the default; moving it is the
exception you write a CHANGELOG line for. Breaking is allowed — the registry keeps your older
releases installable, so a consumer that cannot follow you keeps resolving to the last version
that suits them — but it must be a choice you made, not one you shipped by accident.

**Run `--full` before the tag, even when the change feels additive — the check reads the
TYPE, not your reasoning.**  The trap is a struct gaining a field with a default.  It looks
like the safest change there is: every existing `Pixel { r, g, b }` still compiles, because a
declared default is exactly what makes the new field optional at construction.  That reasoning
is true and is not the whole claim.  Measured on `P3 { r, g, b }` versus
`P4 { r, g, b, a = 255 }`:

```
P3 writes: {"r":10,"g":2,"b":3}
P4 writes: {"r":10,"g":2,"b":3,"a":255}
```

Two different axes move, and only one of them is the one authors think about:

- **What you WRITE changes.**  `to_json()` gains a key, and so does any reflection walk
  (`fields`, `keys`).  A consumer comparing serialised output, or a reader that rejects
  unknown fields, sees a new document.  Reading in the other direction is fine — an old
  document parses into the new type and the default fills the gap (`a=255` above), which is
  why authors probe the read side, find it clean, and conclude the change was additive.
- **The record LAYOUT changes.**  A type has exactly one `(size, align, offset-vector)`
  ([formal/layout.md](formal/layout.md) `(L-Total)`), and that rule's own gloss is the
  warning: it is *"what makes a store written by one build readable by another **of the same
  layout**"*.  Add a field and it is no longer the same layout, so a record persisted under
  the old one is read at offsets computed for a different type.  That is a
  `data_compatible_with` break, and it is invisible from source entirely.

So the two floors move for different reasons and neither is implied by "existing source still
compiles".  `loft compat check --full` reports both, against every prior release, before the
release exists — which is the whole point of running it at PUBLISH time rather than after a
consumer finds out.  (Worked example: `imaging` 0.3.0 claimed `api_compatible_with = "0.1.0"`
on exactly this reasoning and the check answered API BREAK + DATA BREAK against all four prior
releases.  ⚠ Note that loft has **no positional struct construction** — `Pixel { 1, 2, 3 }` is
a parse error — so field ORDER is not one of the axes here, however plausible it sounds.)

**A pinned layout fingerprint moves on the edit you MEANT, and that is the step it exists
for.**  A pack is a store, so a library's struct definitions are its file layout; a test
pinning that layout's fingerprint fails both when the schema is edited (deliberate) and
when loft's layout rules move underneath it (no edit at all — which is why it is pinned
rather than merely computed).  When it fails on a change you meant, re-pin AND move
`data_compatible_with` in the SAME commit, keeping the old number written beside the new
one — a pin nobody can compare against says a format is *different*, not that it
*changed* — and `loft compat check --full` must then report `DECLARED BREAK (below the
floor)` rather than a bare BREAK.  The fingerprint is identical on `--interpret` and
`--native`, which is the other half of the claim: a pack built on one backend is readable
by the other.

The mechanical `[auto]` core of the full correctness bar — see
[LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) for the Goal-by-Goal + doc-quality
`[review]` items and the registry `verified` administration.

Before you ship a version:

- [ ] All tests pass under both `loft test` and `loft --native test`.
- [ ] `LOFT_DENY_WARNINGS=1 loft test` is green (or you've
      kept `.allow_warnings` as an opt-out only when the
      package isn't ready yet).
- [ ] **The publish gate's own command is green** — it is a *different* gate
      from `loft test`, and it is the one that can stop a release:

      ```
      $ <loft-checkout>/target/release/loft --interpret --tests tests
      ```

      `loft test` runs the **installed** loft; `registry_maintain.sh` runs the
      loft built in the **checkout it is invoked from**, and the two can
      disagree while both report the same version — a local shadowing a stdlib
      function name (`now = …`) passed every `loft test` and blocked the
      publish of `imaging` 0.2.2 with *"Cannot redefine function 'now' as a
      variable"*. The gate is the right authority: a published library must
      parse under whatever loft its **consumers** hold, not just the one on the
      publishing machine. Run it before tagging, not after.
- [ ] `loft.toml` has the new version under `[package] version`.
- [ ] `[package] description` is a real one-line summary (not the `loft new`
      placeholder) — it's the official registry catalog text (`loft search` /
      `loft api --registry`); registry tooling prefers it over the README.
      **⚠ Adding this field where there was none OVERWRITES the catalogue text
      already in the index.** A package that never declared one has its
      description living only in `loft-registry/index.json`, and the first
      `loft.toml` to declare one wins silently — the publish does not fail and
      nothing warns. `loft-libs-net` lost three good descriptions this way
      (which crate backs the library; `ssh`'s *"Native-only"*), caught only by
      reading the publish diff afterwards. So before adding the field, read
      what the index already says and merge rather than replace; if the text
      there is already right, copy it in verbatim so a later edit cannot drop
      it.
- [ ] `[package] categories = ["…"]` — **required for a package the registry
      index has never seen**, and the publish REFUSES without it.  The registry's
      own gate 1 rejects an empty `categories`, so a package that goes in without
      one leaves an index that turns every later submission PR red on a check
      that has nothing to do with that submission — and clearing it needs the
      signing key.  `zttext` and `fixstep` reached `main` that way.  Reuse a tag
      the catalogue already uses (`geometry` `graphics` `text` `net` `world`
      `game` `time` `math` `random` `crypto` `encoding` `cli` `plugins`
      `asset-format` `animation`) before minting a new one — a category nothing
      else carries groups nothing.  Same refresh rule as `description`: the
      manifest is authoritative and propagates on every publish, and a manifest
      that declares none leaves the index's curated list alone.
- [ ] README, doc comments on every `pub fn` / `pub struct`.
- [ ] CHANGELOG note for the version (free-form).
- [ ] Local re-package produces a byte-identical sha256 across
      two runs (verifies deterministic packaging):

      ```
      $ loft package && shasum -a 256 *.tar.gz
      $ rm *.tar.gz && loft package && shasum -a 256 *.tar.gz
      # Both hashes must match.
      ```

## 4. Publish

The publish flow has four phases: tag → release source + binary →
emit registry entry → registry PR.

> **Two paths — pick by who owns the library.**
>
> - **A loft-lang family library** (`loft-libs-*`, maintained from a
>   `loft-lang/loft` checkout): don't paste-and-PR by hand.  Tag + release the
>   version (§4a–4b), then run
>   **[`scripts/registry_maintain.sh`](../../scripts/registry_maintain.sh)** — it
>   clones every family repo fresh, publishes each stale/missing version against
>   the live index (`loft publish` under the hood), **signs** the index (the
>   trust-root step — it shows the diff to review first), and pushes.  One
>   reviewed run catches up the whole registry; §4c–4d + §5a below are the
>   mechanism it automates.  Before it signs it re-downloads every tarball and
>   verifies the sha256 — on any mismatch it **refuses to sign** (so a stale
>   release artifact can never reach the signed index; fix that release and
>   re-run).
> - **An external contributor's library**: the manual tag → package → upload →
>   PR flow in §4a–4d (the maintainer signs the index on merge, you sign
>   nothing) — see [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).
>
> **Never hand-edit `index.json` for an own-lib release.**  `registry_maintain.sh`
> (or `registry-sign.sh` after a hand-merge) regenerates + re-signs it.  A hand
> edit that isn't re-signed leaves the published signature invalid and breaks
> `loft install` / `loft search` for everyone.

### 4a. Tag the version

```
$ git tag <name>-v<version>           # convention: name then dash-v
$ git push origin <name>-v<version>
```

Tag convention is `<name>-v<version>` (e.g. `gridmesh-v0.1.1`).
Required by the registry's gate-3 reproducible-build check —
the validator clones at this tag to re-package.  Set
`repository = "<monorepo>"` in `loft.toml` (e.g. `loft-libs-graphics`)
so `loft package` emits the matching `<name>-v<version>` tag + release
URL automatically (see PACKAGES.md § Manifest).

### 4b. Build + upload the release artifact

```
$ loft package                                          # writes <name>-<ver>.tar.gz
$ gh release create <name>-v<version> <name>-<version>.tar.gz \
    --title "<name> v<version>" \
    --notes "<release notes>"
```

`loft package` produces a deterministic tarball (zero mtimes
in gzip + tar headers).  The sha256 is stable across
machines; the registry's gate-3 re-runs `loft package` from
the tagged source to verify byte-for-byte equality.

**Prebuilt cdylibs build automatically (@PLN21).**  A *native*
library's own CI calls the reusable producer workflow so a
consumer can `use` it with **no Rust toolchain** — and so a
broken host build is caught before merge:

```yaml
# .github/workflows/prebuild.yml
on:
  pull_request:
    paths: ['native/**', 'loft.toml']   # PR → validate it builds on every host
  push:
    tags: ['v*']                        # tag → build + attach to the release
jobs:
  prebuild:
    uses: loft-lang/loft/.github/workflows/prebuild-native.yml@main
    with:
      publish: ${{ startsWith(github.ref, 'refs/tags/') }}
```

On a PR it builds the cdylib per host triple (a validation
gate); on the tag it attaches each `lib<stem>.<ext>` to the
release and prints the `binaries[<triple>]` entry (with its
`loft_ffi_fp`) to paste into the registry PR below.  Prebuilts
are optional and per-platform: ship the common targets, others
fall back to a source build on first use.

### 4c. Emit the registry entry

From the package dir:

```
$ loft publish
# Paste this entry into `loft-lang/registry/index.json` under
# `"packages": { "<name>": { "versions": { ... } } }`:

"0.1.0": {
  "url": "https://github.com/<org>/<chunk>/releases/download/<name>-v0.1.0/<name>-0.1.0.tar.gz",
  "sha256": "<sha>",
  "size": <bytes>,
  "loft": ">=0.8",
  "subpath": "<name>",
  "deps": { ... from your loft.toml ... },
  "published": "<now ISO-8601>"
}

[publish] verified release <name>-v0.1.0 exists with asset <name>-0.1.0.tar.gz
[publish] next step: open registry PR with the entry above
```

`loft publish` auto-detects the chunk repo from `git remote
get-url origin`, verifies the GitHub release exists with the
expected asset, re-packages locally to compute the sha256 +
size (no chance of hash mismatch), and emits the
index.json-ready entry.

Flags:

- `--dry-run` — skip the GitHub-release verification (use
  when developing this loop locally).

### 4d. Open the registry PR

Manually clone `loft-lang/registry`, paste the emitted block
into `index.json`, and open a PR:

```
$ git clone git@github.com:loft-lang/registry.git
$ cd registry
$ git checkout -b add-<name>-<version>
$ $EDITOR index.json   # paste the emitted entry
$ git commit -am "Add <name> <version>"
$ gh pr create --title "Add <name> <version>" --body "<rationale>"
```

The registry's CI runs three gates:

1. **Schema lint** — `index.json` validates against the
   schema.
2. **Tarball verify** — sha256 + size of the published
   tarball match what your PR claims.
3. **Reproducible-build re-check** — clones your source at
   the tag, runs `loft package`, compares the resulting
   sha256.  Catches "the GitHub release tarball is stale" +
   "the git tag was force-pushed" + supply-chain swaps.

Maintainer reviews + merges.  The new version is then live;
`loft install <name>` works.

If you're publishing a NEW package (no prior versions),
also add the package block above the versions:

```json
"<name>": {
  "description": "<one-liner>",
  "homepage": "https://github.com/<org>/<chunk>/tree/main/<name>",
  "categories": ["<category>"],
  "yanked": [],
  "versions": { ... your version block ... }
}
```

`loft publish` emits a commented-out stub for this — copy +
customize.

## 5. Maintain

### 5a. Patch releases

For a `0.1.0` → `0.1.1` patch:

1. Bump `version` in `loft.toml`.
2. Commit + tag `<name>-v0.1.1`.
3. `loft package` + `gh release create <name>-v0.1.1 ...`.
4. Publish to the registry:
   - **Own (loft-lang family) lib:** run
     [`scripts/registry_maintain.sh`](../../scripts/registry_maintain.sh) from a
     `loft-lang/loft` checkout — it sees the new version as "stale", publishes it,
     signs, and pushes (review the diff at the prompt).  No manual PR.
   - **External lib:** `loft publish` + registry PR with the new block (under the
     same package, in `versions`).

The registry keeps all versions; old releases stay reachable
unless explicitly yanked.

### 5b. Yank a vulnerable version

When a CVE surfaces against a published version:

1. **Publish the fix first.**  Don't yank before the fixed
   version is available; that strands consumers.
2. Run `loft yank`:

   ```
   $ loft yank web@0.1.0 \
       --severity security_critical \
       --advisory GHSA-xxxx-yyyy-zzzz \
       --summary "TLS bypass in ws_client_connect" \
       --affected ">=0.1.0, <0.1.1" \
       --fixed-in "0.1.1"
   ```

3. The CLI emits two blocks:
   - **Edit 1**: typed `status` field for `index.json`'s
     affected version entry.
   - **Edit 2**: cross-referenced row for
     `advisories.json`'s `advisories[]` array.

4. Apply both edits to your local registry checkout, commit,
   open the PR:

   ```
   $ git checkout -b yank-web-0.1.0
   $ $EDITOR index.json advisories.json
   $ git commit -am 'yank: web 0.1.0 (GHSA-xxxx-yyyy-zzzz)'
   $ gh pr create --title 'yank: web 0.1.0' \
       --body "<advisory rationale + reference URLs>"
   ```

Severity tiers (effects on the loft binary's runtime check):

| Tier | Behaviour |
|---|---|
| `security_critical` | Loud error block at start of every run; user proceeds anyway (default).  Opt-in refusal via `LOFT_STRICT_SECURITY=1` / `--strict-security` (CI gates). |
| `security_high` | Loud warning, always proceeds. |
| `security_low` / `bug` | One-line warning per run. |
| `deprecated` | One-line note. |

The default-warn policy mirrors `cargo audit` — security fixes
can introduce breaking changes, and the user often needs to
run their cached vulnerable code while porting.

### 5c. Update the dep set

When a downstream consumer is on an old version that's been
patched:

```
$ loft update                 # refresh lockfile to latest in-range
$ loft update <pkg>           # scoped to one package
$ loft update --check         # CI gate: exit 1 if updates available
```

`loft update` auto-skips yanked versions via the same
`find_best_version` filter that picks installs.

### 5d. Re-sync the loft monorepo fixture (in-tree-tested libs only)

This step is **only** for libraries that the loft compiler's own
test-suite exercises through a pinned source mirror under
[`loft-lang/loft`](https://github.com/loft-lang/loft)'s
`tests/fixtures/libs/<pkg>/` — the dogfood libraries (`arguments`,
`graphics`, `gridmesh`, `shapes`, `imaging`, `game_protocol`, `web`,
`hex_world`, `time`).  A pure registry-only library has no fixture; skip
to step 5c's registry PR and you're done.

The fixture is a **deliberate snapshot, not auto-latest** — so a
library change that affects the compiler tests is a reviewable commit in
the loft repo, never silent drift.  After the new tag exists in the
chunk repo (5a step 2):

1. In `loft-lang/loft`, bump the tag in `scripts/sync-fixtures.sh`'s
   `PINNED_REFS` table for your package
   (`graphics  graphics-v0.1.0` → `graphics  graphics-v0.1.1`).
2. Refresh the snapshot:
   ```
   $ scripts/sync-fixtures.sh            # clones the tag, copies <pkg>/ into the fixture
   ```
3. Run the suites the fixture feeds — at minimum `cargo test --release
   --test wrap` (interpreter) and any package-specific gold tests
   (e.g. `graphics_gold`) — to confirm the new snapshot still passes.
4. Commit the `PINNED_REFS` bump **and** the `tests/fixtures/libs/<pkg>/`
   diff together as one reviewable commit (per the branch policy: on a
   feature branch, PR to `main`).
5. The CI invariant `scripts/sync-fixtures.sh --check` (exit 1 on
   fixture-vs-tag drift) now passes for that package.

Why the fixture and not a registry install: zero network during `cargo
test`, reproducible across machines + CI, and it survives the eventual
removal of the in-monorepo `lib/<pkg>/` source.  Full rationale +
`PINNED_REFS` semantics live in the `scripts/sync-fixtures.sh` header.

### 5e. Fix a library bug — the clean dev-checkout flow

A library's source lives **only** in its chunk repo; loft consumes a pinned,
read-only **snapshot** under `tests/fixtures/libs/<pkg>/` (§ 5d).  So a library
fix never happens in the loft tree, and never by editing the fixture directly
(that's drift — `scripts/sync-fixtures.sh --check` fails).  This holds even when
a *language* change breaks the fixture: when @PLN22 removed flat-list
`use lib::a, b;`, `game_protocol`'s fixture stopped compiling — the right fix was
to re-release the lib (`game_protocol-v0.1.2`, grouped `use`) and bump the pin,
**not** to hand-patch the fixture to compile (that hides the drift and ships a
library whose own tests no longer build on current loft).  Work in the chunk
repo, **out of the loft tree**, so no stale artifacts accrue in loft:

1. **Issue home.** File / find the bug in the **chunk repo's** tracker
   (`loft-lang/loft-libs-<chunk>`), per
   [ISSUE_TRACKING.md § Convention](ISSUE_TRACKING.md) — so `Fixes #N` is
   same-repo and the `fixed-pending-merge` lifecycle works.  (A bug mis-filed in
   `loft-lang/loft` whose fix is library code gets re-homed there.)
2. **Checkout — out of tree.** Clone the chunk repo to a dedicated dev dir
   *outside* the loft working tree (e.g. `~/loft-dev/<chunk>`), never into
   `loft/lib/<pkg>/`.  The pre-extraction `lib/<pkg>/` layout is being removed;
   any leftover skeleton there (build cruft, no source, no `.git`) is **stale and
   should be deleted** — it only pollutes loft's `git status` and creates "is the
   source here?" ambiguity (the trap that hid `graphics/native/src/text.rs`
   during the @P340 / `@GH252` follow-up — the real source was in the fixture +
   chunk repo, never in `lib/graphics/`).
3. **Fix + test.** Edit the package source in the checkout; run the library's own
   suite there, or test it against loft with `--lib ~/loft-dev/<chunk>`.  The
   checkout shadows nothing in loft's tree and builds in its **own** `target/`.
4. **Tag + push.** Commit with `Fixes #N` (chunk-repo issue), tag
   `<pkg>-vX.Y.Z`, push.  The chunk repo's own apply/strip workflows label then
   close the issue on merge.
5. **Re-sync the loft fixture (§ 5d).** Bump `PINNED_REFS` + run
   `sync-fixtures.sh`, and commit the `tests/fixtures/libs/<pkg>/` diff in loft as
   **one reviewable commit** — separate from the issue close.  loft now tracks the
   fixed snapshot.
6. **Teardown.** `rm -rf ~/loft-dev/<chunk>`, then `loft cache prune` — it drops the
   generations this loft can no longer select and leaves the live one, so the next
   build does not start cold (`loft cache status` first if you want the figure).
   The loft tree is pristine; no stale artifacts remain.

The principle is the project's anti-stale-artifact rule (GOALS.md § "the method
mirrors the goals"): one source of truth for where the code lives (the chunk
repo), isolated + disposable build artifacts (out-of-tree checkout, own
`target/`), and a clean teardown — so the shared medium (the build) can't drift
out of sync and lie.

## Reference

| Topic | Source |
|---|---|
| Package format ([package] / [library] / [dependencies] / [native] / [wasm.bridge]) | [PACKAGES.md](PACKAGES.md) |
| Registry index.json + advisories.json schema | [PKG_REGISTRY.md](PKG_REGISTRY.md) |
| Air-gap deployment workflow | [`lib_plans/12-library-extraction/offline.md`](lib_plans/12-library-extraction/offline.md) |
| Security advisory channel (consumer side) | [`lib_plans/12-library-extraction/security.md`](lib_plans/12-library-extraction/security.md) |
| Canonical `library-ci.yml` template | [`lib_plans/12-library-extraction/library-ci.yml.example`](lib_plans/12-library-extraction/library-ci.yml.example) |
| Cross-package consumer matrix (moros / dryopea / bumper) | [`lib_plans/12-library-extraction/README.md` § Cross-project consumers](lib_plans/12-library-extraction/README.md#cross-project-consumers--moros--dryopea--bumper-airplanes) |
| Monorepo test-fixture re-sync (dogfood libs — step 5d) | [`scripts/sync-fixtures.sh`](../../scripts/sync-fixtures.sh) header (`PINNED_REFS`, `--check`) |

## Troubleshooting

**A test passes `loft test` and fails the CI command with `Library 'raster' not found`** —
`loft test` sets up the package context; the CI gate runs the raw
`loft --interpret --tests tests` (and its `--native` twin) from the package directory,
which does not.  A sibling MODULE (`src/raster.loft`) is reachable only once the package's
entry file has been loaded and pulled it in, so the `use` order is load-bearing:
`use <pkg>;` first, then `use <pkg>::raster;`.  Run the CI command itself before tagging
(§ 3).

**`native symbol 'loft_x' was not registered via loft_register!`** — a native crate registers
each C-ABI entry point in FOUR hand-maintained places in `native/src/lib.rs`: `pub use
m::{…}` at the crate root (what `--native` codegen calls — `pub`, because `--native-wasm`
links the rlib and writes the Rust PATH, so a private `use` compiles natively and fails
every wasm build with E0603 in code the author never sees), `use m::{…__loft_bridge}` for
the proc-macro's bridge idents, `loft_ffi::loft_register!{…}` (the legacy raw-pointer
registry, `loft_register_v1`) and `loft_ffi::loft_register_bridges!{…}` (the uniform-ABI
registry, `loft_register_bridges_v1`).  Missing the legacy one panics with the message
above although the build succeeded.

**`loft publish` says "release `<tag>` not found"** — you
haven't created the GitHub release yet, OR the asset name in
the release doesn't match `<name>-<version>.tar.gz`.  Re-run
`loft package` to ensure the filename is canonical; then
`gh release create <tag> <tarball>`.

**Registry CI's gate-3 fails with sha256 mismatch** — the
tarball on the GitHub release doesn't byte-for-byte match
what `loft package` produces from the tagged source tree.
Common causes:
- You ran `loft package` against a working tree with
  uncommitted changes, then committed + tagged afterwards.
  Re-run `loft package` from a clean checkout at the tag.
- You uploaded a manually-edited tarball.  Don't.

**`use <pkg>;` resolves to the wrong version** — check the
resolution chain via `loft list-installed` + the closest
`loft.lock`.  Sidecar `<script>.loft.lock` takes precedence
over walk-up `loft.lock`.  `LOFT_OFFLINE=1` blocks
auto-install.

**Native crate fails to build from `~/.loft/registry/.../native/`**
— the cargo build redirects to `~/.loft/build-cache/<pkg>-<ver>/`
(per Phase 6b's `auto_build_native` redirect).  Check that
`~/.loft/build-cache/` is writable + that `loft-ffi` /
`loft-ffi-build` are reachable on crates.io (or via your
configured `[source]` mirror).
