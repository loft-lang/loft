<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# USER_DOCS.md — the documentation a distribution owes its users

> loft stopped being one interpreter with a tutorial and became a **distribution**: a
> language, a stdlib, 42 published libraries, a registry, four build targets and a
> compatibility contract being driven toward absolute. The documentation still describes the
> first thing. This is the design for the second.
>
> It covers three surfaces that fail for the same reason — **the text exists but no reader
> path reaches it**: the library documentation, the front-door README, and the code examples
> on the website, which can already be driven by a REPL and a debugger that the pages do not
> expose.
>
> **Plan: @PLN149 — SHIPPED 2026-09-10.** All four tiers, the guide contract, the REPL and
> debug panel, and the retirement of the hardcoded delegation lists are built and published.
> Written as a design, not a plan: one invariant, a count of what has to re-state it, the
> failure paths written down before any code, and a check pinned to each claim
> ([design-protocol](../../.claude/skills/design-protocol/SKILL.md)). The build order is
> last, deliberately — the order is a consequence of the design, not the design.
>
> **Its tail is a standing practice, not an open plan.** Tier 1 is one guide per library and
> a package published next month owes one too, so there is no state in which it is finished
> and nothing to close. `make libraries-review` reports the queue — *N have one · N owe one ·
> N not measured* — and [LIBRARY_DOC_REVIEW.md § 3a](LIBRARY_DOC_REVIEW.md) is the pass that
> works it. This document stays the design; it is no longer a plan of record.

---

## Two constraints that shape everything below

**1. The machinery is mostly built. This is a wiring problem, not a construction problem.**
Every measurement in *What already exists* below was taken by running the command, not by
reading a doc. `loft doc graphics` renders nineteen API sections and a guide into HTML
today. `loft api --registry` lists all 42 packages with descriptions today. `src/wasm_debug.rs`
implements breakpoints and expression evaluation in the browser today, end-to-end tested.
None of it is reachable from a page a user lands on. A design that proposes building any of
it again is wrong before it starts.

**2. A reader arrives with one of four questions, and they are not interchangeable.**
*What is there?* — *How do I start?* — *What is the exact signature?* — *How does it actually
work?* Today loft answers the third one well, the second one twice out of forty-two, and the
first and fourth not at all on the web. Every tier below exists because a reader in one of
those states is served badly by the answer to a different one. An API list is not an
introduction, an introduction is not a reference, and neither is the source.

---

## The invariant

> **A library's user-facing text has exactly one home — the library itself — and every
> reader path renders that source rather than restating it.**

This is the one-home rule the repo already applies to code (`QUALITY.md`), turned on prose.
It is worth stating as an invariant rather than a preference because the alternative has
already failed here, measurably, and the failure is the kind nobody notices: two copies that
agree when written and drift in silence.

### Counting the re-assertion sites

Nine places currently carry, or could carry, a description of what a library is and how to
use it:

| # | Site | Hand-written? | Reaches a user? |
|---|---|---|---|
| 1 | the library's `README.md` | **yes** | only via GitHub |
| 2 | the library's `docs/*.loft` guide | **yes** | only via `loft doc` |
| 3 | ~~`tests/docs/NN-<lib>.loft` in the loft repo~~ — **retired, step 9** | — | — |
| 4 | the registry `index.json` description | **yes** | via `loft api --registry` |
| 5 | `LIBRARIES.md` | generated | no — local build, git-ignored |
| 6 | `loft api <name>` | generated | CLI only |
| 7 | `loft api --registry` | generated | CLI only |
| 8 | `loft doc <name>` | generated | local HTML only |
| 9 | a library page on the doc site | — | **does not exist** |

Four hand-written homes for the same facts. **The drift was not hypothetical: `random`'s guide
existed at sites 2 and 3 and the two files no longer matched** — `docs/21-random.loft` in
`loft-libs-core` hashed `fb3cf619…`, `tests/docs/21-random.loft` in this repo hashed
`3f80cc26…`. Nobody edited both. Nothing reported it. Both were published, to different
readers. Site 3 is now gone (step 9): the two files were merged into one, in the library.

The design collapses the four to **two**, each with a job the other cannot do:

- **`docs/*.loft` in the library** — the guide. Executed prose; the single source for every
  rendered page.
- **the registry `index.json` description** — the one-line hook. Already the registry's job,
  already the thing `loft api --registry` prints, already reviewed at publish time.

Sites 1 and 3 stop being sources. The README is *generated* from the guide plus the manifest
(§ *The README*, below, applies the same rule to loft's own front door); the loft repo's
`tests/docs/NN-<lib>.loft` files move out to the libraries they document.

---

## What already exists — measured

Run, not read. Each line is a command whose output was checked.

| Capability | State | Evidence |
|---|---|---|
| API extraction from `pub` + doc comments | **complete** | `loft api graphics` → 618 lines, signatures *with* their doc comments |
| Registry catalogue, CLI | **complete** | `loft api --registry` → 42 packages, each with a description |
| Per-package HTML generation | **complete** | `loft doc graphics` → *"1 guide(s), 19 API section(s) → ~/.loft/doc/graphics-0.8.0"* |
| Guide format (executed prose) | **complete** | `@NAME`/`@TITLE` topic files; the same format the 37 executed language pages use |
| Maintainer catalogue | **complete** | `make libcatalogue` → `LIBRARIES.md`, 42 libraries with full public API |
| Browser breakpoints + expression eval | **complete** | `src/wasm_debug.rs`; `tests/wasm_debug_relay.rs` drives `bp` / `run` / `eval` / `resume` headless |
| Doc-site generation | **complete** | `gendoc` → 74 pages, nav, sitemap, search index, PDF |
| Shell-transcript execution in docs | **complete** | `tests/doc_commands.rs` — an indented `$ ` line is run, its shown output checked |
| Library revalidation against loft | **complete** | `scripts/revalidate_libs_local.sh` → 42 pass, 0 compile-break |

The generator, the extractor, the executor, the catalogue and the debugger all exist and all
work. What does not exist is any path from a reader to their output.

### And what a reader actually gets

| | |
|---|---|
| Published libraries | **42** |
| …with a user doc page on the site | **3** — `imaging` (14-image), `random` (21-random), `time` (32-time), all three in the LOFT repo rather than their own |
| …with a hand-written guide in their own repo | **2** — `graphics`, `random` |
| …with any `doc/` directory | **0** |
| …with any examples | **3** — `game_protocol` (17), `drawing` (1), `graphics` (1) |
| …with no `README.md` at all | **2** — `pluginabi`, `hex_grid` |
| …whose README is nine lines of boilerplate | **3** — `html`, `markdown`, `input` |
| Library pages on the doc site | **0** |
| Links from the doc site to any library repo | **0** |
| Libraries named anywhere on the doc site | **2** (`imaging`, `random`, in an install line) |
| `@F` catalogue entries covering a library | **0 of 117** — the catalogue is core-only by construction |

`html` and `markdown` had no real README, and both are load-bearing for the better-PHP
direction [WEB_STACK.md](WEB_STACK.md) designs. `stage` and `graphics` carry 136 public items
each, documented by one README apiece.

⚠ **Every count here is read from `origin/main`, and that is not a detail.** A first pass
measured the local sibling checkouts and got two of these rows wrong in both directions:
`cbor` was reported as having no README when `origin/main` carries 55 lines, and `input` was
reported as fine on the strength of an 84-line README that exists only on an unmerged branch
— on `origin/main` it is the nine-line scaffold. `LIBRARIES.md`'s own header says to read a
library from `origin/main` rather than from a clone; this is what ignoring it costs, and the
same rule applies to deciding which library needs work at all.

### The plan of record was stale — retired in step 10

[DOC.md](DOC.md) carried a design for this under *Bundled library documentation* and *Two
tiers*, and none of it matched the tree:

- It said bundled libraries live in `lib/` and are `imaging` and `random`. `lib/` holds
  `audience_crystal`, `engine_host` and `git`; `imaging` and `random` are registry packages.
- It specified a **Package Catalog** page generated from `registry.txt`, with an optional
  `docs` column. `registry.txt` does not exist — the registry is `index.json` in
  `loft-lang/registry` (@PLN112), and no catalog page was ever generated.
- Its four discoverability paths ("index cards, search, nav bar, catalog") described a site
  that had none of them for libraries.

- And the machinery underneath: it said `gendoc` discovers `lib/*/loft.toml`, calls
  `generate_pkg_docs` on each and writes `lib/<name>/doc/`, feeding a third index card grid,
  a nav section and the search index. **`gendoc` has no `lib/` path at all** — the only
  caller of `generate_pkg_docs` is `loft doc <name>`, which writes locally and puts nothing
  on the site.

**Retired (step 10).** `DOC.md § Library documentation` is now the mechanical half only —
the five page kinds, what each is read from, which two need the package extracted, and the
release rule — and points here for the design. The dead `lib/` machinery and the
`registry.txt` catalog are gone rather than corrected, because there was nothing under them
to correct.

---

## What is missing

Three things, and only the third needs new mechanism:

1. **A reader path to output that already exists.** `loft doc` writes HTML nobody publishes;
   `loft api --registry` prints a catalogue no page shows.
2. **Guides.** 2 of 42 libraries have one. The format, the executor and the renderer are all
   in place; the prose is not written.
3. **One home enforced.** Nothing today would catch the `random` drift, or a README that
   stopped matching its guide.

---

## The design

Four tiers, each answering exactly one of the reader's four questions, each generated
from the one home.

### Tier 0 — Discovery: *what is there?*

**One `doc/libraries.html`, generated from the registry index**, listed in the nav on every
page and in the sitemap. One row per package: name, version, the one-line description the
registry already carries, its category, and links to its guide, its API reference and its
repository.

Generated from `index.json` alone, so it needs no library to be installed, cannot lag the
registry, and costs the doc build nothing. It is the web twin of `loft api --registry`, which
already prints exactly this — the same source, a second renderer.

This is the single highest-value item in the document and it depends on nothing else.

#### The library card — the top-level facts, beside the tags

The categories (`graphics`, `game`, `text`, `world`, …) are the tag system, and they answer
*where does this belong*. They do not answer the question a reader actually has before
clicking into a guide: **should I depend on this?** That needs a per-library record, and
almost all of it is already declared — it is simply never rendered anywhere a user looks.

| Field | Where it already lives |
|---|---|
| One-line description, homepage, categories | `index.json`, per package |
| Version, publication date | `index.json`, per version |
| **Minimum loft version** | `index.json` `loft` — the floor, already enforced at install |
| **API / data compatibility floors** | `index.json` `api_compatible_with`, `data_compatible_with` |
| Download size | `index.json` `size` |
| **Dependencies** | `index.json` `deps` |
| Auto-use triggers | `index.json` `triggers` — why a method resolves with no `use` |
| Public API item count | computed by `gen-library-catalogue.py` for `LIBRARIES.md` |
| Unreleased work / breaking-change flag | computed by the same script against `origin/main` |
| wasm capability, or the reason it is exempt | the package's `.wasm_exempt`, whose contents CI already prints |

**The card is a renderer, not a new dataset**, which is what keeps it inside the invariant:
no field on it is a fourth hand-written home for anything. `loft api --registry` gets the
same fields as a `--long` form, so the CLI and the page keep agreeing by construction.

Exactly two facts on the card do not exist yet and are worth the work:

- **Reverse dependencies — "what uses this".** Derivable by inverting every package's `deps`,
  and it is the field that tells a reader whether they are the first person to try something.
  `stage` being used by four other packages is a stronger signal than any adjective.
- **A health line.** Last published, whether `origin/main` has moved since, whether the
  library is `status:parked`. `LIBRARIES.md` already computes the first two for maintainers;
  a user deciding whether to depend on something needs them more than a maintainer does.
  ⚠ **Only the publication date is reachable from the index.** The drift flag needs a CLONE
  and wasm capability needs the package source, and the site build has neither — so both stay
  maintainer views rather than becoming a field the page guesses at. Measured while building
  the card, not assumed.

Do **not** put a maturity adjective on the card — "stable", "beta", "experimental". It is a
judgement with no source, so it becomes a fourth hand-written home that drifts and that
nobody can check. The version, the floors, the dependents and the health line are facts, and
a reader draws the conclusion the adjective would have handed them.

### Tier 1 — The guide: *how do I start?*

**One executed guide per library, at `docs/01-getting-started.loft` in the library repo**, in
the `@NAME` / `@TITLE` topic format the 37 executed language pages already use. It is a `.loft` file,
so it compiles and runs; its prose is comments and its examples are the program.

The contract for a guide, in order — the shape the good language pages already have:

1. **What it is**, in one paragraph, and what it is *not* — the neighbouring library a reader
   might have wanted instead.
2. **The smallest complete program that does something.** Runnable, with its real output.
3. **The three or four calls that carry the library**, each with a worked example. Not the
   whole surface — that is Tier 2's job.
4. **The one thing that surprises people.** Every library has one; `graphics`'s is that
   `rgb()` and a hand-written hex literal differ in the alpha byte.
5. **Where to go next** — the API reference, and the library's own tests as examples.

Executed by the **library's** CI, because the library owns it — and **`main` has to call
every section**, or the assertions do not run at all. A zero-parameter function nobody calls
still compiles, so a renamed API is still caught while the half that checks the example is
*right* rather than merely *spelled correctly* silently does nothing. Measured: `graphics`'s
guide defines `drawing_shapes()` and `aa_example()` and calls neither, so two of its three
sections read as verified and are not. This is what removes the
hardcoded delegation list: `tests/wrap.rs:52` and `tests/doc_lib_examples.rs:63` name
`14-image.loft` and `21-random.loft` as string literals, so a new library page is silently
uncovered until someone edits both. A guide that lives in its library is run by that library's
own suite, by the rule that it is a `.loft` file in the package — no list to forget.

### Tier 2 — The API reference: *what is the exact signature?*

**Already built, and reachable more cheaply than `loft doc`.** The registry index carries
each version's own `api` array — every `pub` signature with its doc comment, re-derived from
the package source by registry CI at publish time. So the reference renders from the index
alone: no package installed, no clone, and no way to drift from the version it describes.

Generated from `pub` declarations and their doc comments, which is why it is the tier that
already works — and the argument for pushing the other two toward generation as well.

**A reference nobody can find is half-published**, so every name goes into the site's search
index too, qualified as `pkg::name`. The qualifier is not a narrowing: the match is a
substring test, so `render` still finds `markdown::render`, and the qualifier is what tells
four packages' `render` apart in the result list. The search covered the bundled stdlib only
(228 entries); with the distribution in it, 1296.

⚠ **13 of the 42 published versions carry no `api` array** — the field was added after they
were published. Their pages say exactly that and name the two routes that do work
(`loft api <name>`, which reads the package itself, and the source), because an empty list
reads as *this library exports nothing*, which is false for every one of them. The gap closes
itself: the field is filled at each package's next release, so this is a shrinking number and
the page needs no edit when it shrinks.

### Tier 3 — The source: *how does it actually work?*

**Every `.loft` file of every library, rendered as syntax-highlighted HTML straight from the
sources**, with each function linked to its definition and to the examples that call it.

This tier exists because of something specific to loft: **the libraries are written in loft.**
Reading `graphics` is reading the language. So the source browser is simultaneously the
largest body of idiomatic loft in existence — 42 packages, thousands of lines, all of it
compiling and tested — and today a reader who wants to see how a real library is *built* has
nowhere to look but GitHub. A reference tells you a function's type; the source tells you what
good loft looks like at scale, which is the question a reader has once the tutorial is behind
them.

Like the rest of this document, it is a renderer over data that exists:

- **Highlighting.** `highlight_loft` (`src/documentation.rs:812`) already emits the nine
  classes `DOC.md § Syntax highlighting classes` documents, and already wraps an identifier in
  an `<a href>` when its name is in the link map. Cross-linking a call to its definition is
  therefore the *existing behaviour* pointed at a larger link map, not a new feature.
- **Example links.** The `@AAA-###` worked-example convention (@PLN141) already resolves a
  `// Example: @GFX-001` citation above a `pub fn` to the test or application function that
  demonstrates it. `examples-index.tsv` holds the resolved set — tag, `file:line`, function
  name and a git blob URL — generated by `scripts/check_doc_drift.sh` and gated in every
  library's CI.

  ⚠ **The browser does not need that file.** A tag is DEFINED in a comment block above the
  demonstrating `fn`, and a package ships its own `tests/` in the tarball — so **96 of the 99
  citations across 18 packages resolve from the package alone**, with no external index and
  no clone. The three that do not point at an application in another repo; those name the tag
  without linking it, which still beats a blank because the tag is greppable. Measured, and
  it is why this tier stays self-contained.

The links go both ways: from a function to the examples that use it, and from an example back
to every function it calls.

**The honest limit, and the thing that closes it.** A citation exists only where an author
wrote one, and [LIBRARY_AUTHORING.md § 2a](LIBRARY_AUTHORING.md) deliberately refuses a
retroactive sweep — tagging every obvious accessor would turn the gate red on hundreds of
functions that teach nothing. So a page that showed only curated examples would imply that an
untagged function is unused, which is false and is the worse failure: it makes the
best-documented libraries look the same as the least. The complement costs nothing here,
because loft can parse loft: **a call-site index derived from the sources themselves**, which
finds every use whether or not anyone tagged it. The two are different signals and the page
says which is which — *worked example* (someone chose this as the thing to read first) versus
*call sites* (every place it is used, mechanically).

### The one-home rule, and what it retires

| Today | After |
|---|---|
| ~~`tests/docs/14-image.loft` (loft repo)~~ | `imaging/docs/01-getting-started.loft` — **landed, imaging 0.3.1** |
| ~~`tests/docs/32-time.loft` (loft repo)~~ | `time/docs/01-getting-started.loft` — **landed, time 0.3.1** |
| ~~`tests/docs/21-random.loft` **and** `random/docs/21-random.loft`, drifted~~ | `random/docs/01-getting-started.loft`, one file — **landed, random 0.3.2** |
| 42 hand-written `README.md` | generated from the guide + manifest, with a drift guard |
| ~~`tests/wrap.rs` SUITE_SKIP, `doc_lib_examples.rs` filename list~~ | nothing — the library runs its own guide, in `library-ci-reusable.yml` — **landed** |

The doc site renders a library's guide from the **registry cache** — the same source
`loft api graphics` reads without a clone — so the loft repo never holds a copy of a library's
prose.

---

## The pages a reader can drive: REPL and debugger in the browser

### What is built

`src/wasm_debug.rs` implements the browser side of the debugger against the running wasm
program: `bp <fn>` sets a breakpoint, `run` / `resume` / `step` move, and `eval <expr>`
evaluates an arbitrary expression over the paused frame. `eval` is not a value peek — it binds
the live locals as arguments, compiles the expression as a synthetic function and runs it, so
`eval len(v)`, `eval h["a"].v` and `eval fib(10)` are all in range, the last one because the
program's own definitions are in scope. `tests/wasm_debug_relay.rs` drives the whole chain
headless — an agent, through a server, into a browser wasm client — and asserts the replies
for `bp`, `run`, `eval` and `resume`.

### What the pages expose

⚠ **This section describes the state before step 8; it is kept as the measurement that
motivated it.** `doc/playground.html` has **one button: ▶ Run.** `doc/examples.js` carries 99
examples — every `tests/docs/*.loft` file, folded in by
`scripts/build-playground-examples.loft` — and every one of them is run-only. The 37 executed
language topic pages show their code as static text.

So the capability that most distinguishes loft from a language with a syntax-highlighted
snippet on a page is built, tested, and invisible.

**Since step 8 it is not.** Each executed topic page carries the panel
(`doc/loft-panel.js`, markup from `documentation.rs::panel_html`), over
`debug_start` / `debug_command` in `src/wasm_debug.rs`. `doc/playground.html` is unchanged
and still the place to EDIT code; the panel is for driving the page in front of you.

### The design

**Every code block on a doc page becomes drivable, in three steps that share one runtime.**

1. **Run** — what exists. The block executes, output appears beneath it.
2. **REPL** — a prompt beneath the output. The reader types an expression; it is evaluated
   against the program that just ran, through the same `eval` path the debugger uses. Calling
   the page's own functions is the point: on the vector page a reader types `sum(v)`, on the
   text page `caesar("hello", 3)`.
3. **Debug** — a gutter click sets a breakpoint (`bp`), execution pauses, the frame's locals
   are listed, and the REPL prompt is now evaluating *over the paused frame*. Step and resume
   from the same panel.

One honest constraint, and it shapes the surface: **`eval` evaluates over a frame**. A REPL
against a program that has finished has no frame to bind. So the page's Run auto-pauses at
the end of `main` before the frame unwinds — the reader's prompt is a paused frame that
happens to be the last one — and the panel says so, rather than presenting a stateless
calculator that mysteriously cannot see `v`.

### The example that makes it concrete

One new page, `tests/docs/38-call-it-yourself.loft`, whose entire purpose is to be driven
rather than read. It defines a handful of small, pure, zero-dependency functions — a
`fib(n)`, a `primes_below(n)`, a `caesar(s, k)`, a `stats(v)` returning a struct — and its
prose is the invitation: *these four are live below; call them.* The REPL panel lists the
callable names off the program's own definitions, so the reader does not have to scroll up to
find out what to type.

It is a page in `tests/docs/`, so it is executed on both backends like every other, and folded
into `doc/examples.js` like every other. The interactive panel is the page's renderer, not a
one-off.

Why a purpose-built page and not just the panel: a capability nobody is *told* to use is
indistinguishable from one that is missing. The page is the invitation, and it is also the
thing that fails loudly if the REPL wiring breaks — every other page would merely look
slightly less interactive.

---

## The README

### What is wrong with the current one

Nothing is inaccurate — I checked every claim: `doc/learn-loft.md` exists, `examples/` holds
exactly seven files, Brick Buster is 1,849 lines, `editors/vscode` is there, ten skills ship.
One number is stale: *"~73k lines of documentation"* is now **115,560**.

The problem is the **positioning**. The title is *"build small games, share a link, anyone
plays"* and the first 40% is one arcade game. That was the right README for a project proving
it could produce something; it is the wrong one for a distribution with 42 libraries, four
targets, a registry, a compatibility contract and eight applications built on it. A reader
evaluating loft as a language they might depend on has to get past a game to find out that
`loft install` exists, and the libraries appear as a five-row table two-thirds of the way
down, described as "batteries".

Nothing that follows argues for making it drier. Brick Buster is the best evidence in the
repository that the thing works, and it stays. It stops being the *thesis* and becomes the
*proof*.

### What it must say instead, in order

1. **What loft is**, in one sentence a reader can repeat: a statically typed language that
   reads like Python, compiles to a native binary or to the browser, and ships as a
   distribution with a registry.
2. **Evidence it works** — the playground, Brick Buster, the gallery. Three links, compact.
3. **The code**, unchanged; it is good and it is short.
4. **Install and first program.**
5. **The distribution.** The 42 libraries by category, with `loft install <name>` and
   `loft api --registry`. This is the section that does not exist today and is the single
   biggest gap between what loft is and what its front page says it is.
6. **Maturity and compatibility.** Calendar versions, the contract-1 goal, what is stable
   today and what is not — the honest paragraph a reader needs before depending on it.
7. **Documentation** — the site, the reference PDF, `loft doc`.
8. **How loft is built**, the bus-factor section. Distinctive, true, and it belongs late,
   because it answers a question a reader only has after deciding the thing is interesting.
9. **Contributing**, **License**.

### The one-home rule, applied here too

The library table in the README is the same facts the registry index carries, and a
hand-maintained table of 42 rows is a drift generator. It is generated from `index.json`
between markers, refreshed by the same script that builds `LIBRARIES.md`, with a CI check that
fails if the committed block does not match. Same rule as § *The invariant*: one home, every
reader path renders it.

---

## Failure paths

Written before the code, because each one is silent by default.

| Failure | Why it is silent | The design's answer |
|---|---|---|
| A library is not installed when the site builds | Its page simply does not appear; the nav is one shorter | The page is generated from the registry index regardless, with the API and guide sections marked *not built*. The build **reports the count** and the count is expected to be zero. |
| A guide stops compiling | Nothing on the site runs a registry library's guide today | `revalidate_libs_local.sh` already extracts and runs every published library's `tests`. It runs `docs/*.loft` too. |
| The site renders 0.8.0 while the registry serves 0.9.0 | Both look correct in isolation | Every generated page carries the version it was built from; `registry-index-snapshot.json` is the oracle and the drift is a check, not a reading. |
| A generated README is hand-edited | It looks better and is now a second home | The same drift-guard shape `tests/doc_hygiene.rs` already uses for `doc/examples.js`. |
| A library has no guide | Its page 404s, or worse, silently vanishes from the nav | The page always exists, shows the API, and says *no guide yet*. The count of guide-less libraries is the metric the monthly review reads. |
| A card field is missing for one library | The card still renders, one row blanker than the rest, and reads as "this library has no dependencies" rather than "nobody declared any" | Absent and empty are rendered differently, and the build reports how many packages are missing each field. A blank `deps` on a package that has them is a registry defect the card is the first thing to surface. |
| A worked-example citation goes dangling | The source page links to a function that no longer exists | Already gated: `check_doc_drift.sh examples` fails on `dangling` / `duplicate` / `unregistered` in every library's CI, and `examples-index.tsv` is regenerated rather than hand-written. |
| The source browser shows no examples for a function | A reader reads it as "nothing uses this" | Curated examples and derived call sites are labelled as different things, and the derived index is complete by construction — so *no call sites* is a real finding and *no worked example* never is. |
| The source browser goes stale against a published version | Highlighted code that no longer matches what `loft install` fetches | Rendered from the same registry-cache source `loft api` reads, stamped with the version, and covered by the same snapshot oracle as the pages above. |
| The REPL panel breaks | Every page still renders; the code blocks just stop being interactive | `38-call-it-yourself.loft` exists to fail. Its whole content is the interaction. |
| `eval` returns `<unavailable>` for a `text` local | The reader reads it as "loft cannot do this" | The panel distinguishes *not supported here* from *no value*, and the page says which expressions are in range. This is a known limit of `eval_expr`, documented rather than hidden. |

---

## Verification — on rails that already exist

No new harness. Each claim below is pinned to an instrument the repo already runs.

| Claim | Check |
|---|---|
| Every library page renders | `gendoc` reports libraries rendered / total; a mismatch fails the build |
| Every guide compiles and runs, both backends | `revalidate_libs_local.sh` (already 42/42) extended to `docs/*.loft` |
| Every shell line a guide shows is real | `tests/doc_commands.rs` — the indented `$ ` rule, unchanged |
| Generated README matches its source | drift guard, the `doc/examples.js` shape in `tests/doc_hygiene.rs` |
| The library table in this repo's README matches the registry | same guard, same script that writes `LIBRARIES.md` |
| The REPL panel actually evaluates | `tests/wasm_debug_relay.rs` already proves the protocol; the page adds a headless driver asserting one `eval` round-trip |
| Every card field traces to a declared source | the card generator reads `index.json` + `gen-library-catalogue.py`'s computed fields only; a hand-written field is a review finding, and there is no code path that accepts one |
| Every example link resolves | `scripts/check_doc_drift.sh examples` — already live in every library's CI since loft#971 — plus `examples-index.tsv`, regenerated by `make examples-index` and verified by `check_doc_drift.sh examples-index` |
| The rendered source matches the shipped source | byte-compare the highlighted page's extracted text against the package file it came from; a highlighter that drops a line is otherwise invisible |
| A guard would fail if the thing broke | `make falsify` on each new guard, recorded as `@falsified-at:` |
| Guide quality does not rot | `make libraries-review` and the watermark table in [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md), which already exist and currently have almost nothing to review |

The API-surface lint (`scripts/api_lint.py --check <lib>`) and the per-library documentation
sections of [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) are already the release gate for a
library. The guide becomes a checklist row there rather than a new process.

---

## Build order

A consequence of the design, not the design. Ordered by value-per-unit-work; each step is
independently shippable and none blocks the next except where marked.

1. **Baseline the missing text.** ~~READMEs for `pluginabi` and `hex_grid`; real content for
   `html`, `markdown` and `input`.~~ **Three of five landed** — `hex_grid`, `html`,
   `markdown`. `input` and `pluginabi` are left alone deliberately: a sibling checkout has
   unmerged work in both right now (`loft-libs-game` and `loft-libs-plugins`, branch
   `worked-examples`), and `input`'s README is already rewritten there. Writing into a
   package under active edit on someone else's branch is how two people's work collides.
2. ~~**`doc/libraries.html` from the registry index**, in the nav and the sitemap.~~
   **Landed** — 42 packages in 15 categories, in the nav on every page, a featured index card,
   and the sitemap by rule (74 → 75 pages).
3. ~~**The new README.**~~ **Landed** — it was independent of everything above, which is why
   it went first.
4. ~~**The library card** on each Tier 0 entry.~~ **Landed** — one `doc/lib-<name>.html` per
   package, linked from the catalogue, which is also where the guide, the API reference and
   the source browser will hang. Two fields the design listed were dropped rather than faked:
   see the health-line note above.
5. ~~**Publish `loft doc` output** for each library, linked from Tier 0.~~ **Landed**, and by
   a cheaper route than `loft doc`: the registry index's own `api` array already carries every
   `pub` signature with its doc comment, so the reference needs no package installed and no
   clone. One `doc/lib-<name>-api.html` per package, linked from the card, plus every name in
   the site's search index — a reference nobody can find is half-published. ⚠ **13 of the 42
   versions predate the `api` field**, and those pages say so and name the two routes that do
   work, rather than rendering an empty list that reads as *this library exports nothing*.
6. ~~**The source browser**~~ **Landed** — one `doc/lib-<name>-src.html` per package, every
   `.loft` file it ships, highlighted and line-anchored, with a public-item table carrying
   both signals. ⚠ Two things the design got wrong and the build corrected: the citations do
   **not** need `examples-index.tsv` (96 of 99 resolve inside the package's own `tests/`,
   which ship in the tarball), and the source **does** need the registry cache — it is the one
   tier the index cannot serve.
7. **The guide contract** — **the contract and the rendering have landed**; two of the six
   guides are written. `LIBRARY_AUTHORING.md § 2c` carries the five-part shape and the rule
   the build found (⚠ **`main` must call every section** — `graphics`'s guide defines two
   sections `main` never calls, so their assertions have never run), `LIBRARY_CHECKLIST.md`
   carries the row, and `doc/lib-<name>-guide.html` renders any guide a package ships.
   `html` and `markdown` are written, verified and **published** (loft-libs-docs `8a40d5d`,
   html 0.1.1 + markdown 0.2.1) — the write landed in `4efe2c3` and stopped one step short,
   since a guide that never goes through a release is one no reader outside its repo can
   reach. Both were run through the real `Guide` step before the bump, which is the first
   time anything had executed them; green on both backends, and the backends agree.
   `graphics` has one already (with the defect above, in a repo currently on an active
   branch); `stage`, `server` and `web` need an environment their examples can be run in and
   are not written.
8. ~~**The REPL and debug panel**, plus `38-call-it-yourself.loft`.~~ **Landed.** Every
   executed topic page carries a Run / REPL / Debug panel driven by two new wasm exports
   (`debug_start`, `debug_command`) over the debugger's own command grammar, plus
   `tests/docs/38-call-it-yourself.loft`, the page whose whole purpose is to be driven.
   Verified in headless Chrome: Run pauses, the frame's locals are listed, `nth_prime(10)`
   typed at the prompt answers 29, and a click on a source line sets a breakpoint. ⚠ Three
   things the design got wrong, all found by measuring before building — see below.
9. ~~**Move the three guides home** — `14-image`, `32-time`, `21-random` — and delete the
   hardcoded delegation lists.~~ **Landed** (loft-libs-core `38081d1` random 0.3.2,
   loft-libs-graphics `2791593` imaging 0.3.1, loft-libs-game `f3f0a45` time 0.3.1). Each is
   one `docs/01-getting-started.loft` in the five-part shape, run on both backends with
   `LOFT_DENY_WARNINGS=1`. `tests/docs/{14-image,32-time,21-random}.loft`,
   `tests/doc_lib_examples.rs` and the `SUITE_SKIP` / `WASM_SKIP` / `NATIVE_SKIP` entries are
   deleted; all three skip lists are now EMPTY.

   ⚠ **The step could not be pure cleanup, because the coverage it deletes had no
   replacement.** The design says a guide is "run by its own CI"; it was not.
   `library-ci-reusable.yml` runs `loft --tests tests`, and a guide is a program, not a
   suite — so `html` and `markdown` (step 7) have never been run by CI, and deleting
   `doc_lib_examples.rs` would have taken `imaging` and `random` from *both backends
   compared* to *nothing*. A `Guide — <pkg>` step now runs every `docs/*.loft` on both
   backends and diffs the two outputs, which is the property inherited from the test being
   deleted. Falsified both ways before landing: a broken assert and a forced backend
   divergence each turn it red with the finding in the job summary.

   Two things the step measured. `parse`'s contract is **normalise, not validate** —
   `time::parse("2026-13-45")` answers a non-null 2027-02-14, so a null check proves the text
   was readable and nothing more; the first draft of the guide asserted the opposite and the
   file said so on its first run. And `imaging`'s page had been describing a three-channel
   `Pixel` since `a` landed in 0.3.0 — the drift this rule exists to end, found by moving the
   text next to the code it describes.
10. ~~**Retire [DOC.md § Two tiers](DOC.md)**, replacing it with a pointer here.~~
    **Landed.** The whole `## Bundled library documentation` block went, not just the one
    section: measuring it first showed `gendoc` has no `lib/` path, so its *How it works*,
    *Index integration*, *Navigation*, *Search* and *Adding a new bundled library* described
    machinery that does not exist — as did the `registry.txt` Package Catalog and the four
    discoverability paths. `DOC.md § Library documentation` replaces it with what is true:
    the five page kinds and their sources, the two that need the package extracted, the
    release rule, and `loft doc` named as the separate local tool it is.

Steps 1, 2 and 4 are each under a day and together change what a new user experiences more
than everything below them combined; all three have landed.

**All ten steps have landed.** What is left is not a step but a queue of guides: `input` and
`pluginabi` (step 1) and `stage`, `server` and `web` (step 7), each waiting on a checkout or
on an environment their examples can be run in. Writing one is the whole of what it takes —
the contract, the rendering, the CI gate and the release path all exist now.

**Six packages carry a guide and all six are published** — `graphics`, `random`, `imaging`,
`time`, `html`, `markdown` — so `make doc` renders six guide pages where it rendered two.
A guide only reaches a reader through a RELEASE: `gendoc` reads it out of the extracted
tarball, never out of a checkout, so writing the file is half the step and bumping the
version is the other half.

**Step 8 was blocked by two defects in the machinery it sits on, and is capped by a third.**
The design read `src/wasm_debug.rs` and believed its doc comment. Measured instead:

- **The program's own functions were not in scope.** `eval len("abc")` answered 3 and
  `eval fib(10)` answered `<unavailable>` — the stdlib reachable, the page's own definitions
  not, which is the inverse of what the panel is for. Every `eval` compiles through
  `parse_str`, which resolves under `STD_SOURCE`; the program had been parsed through
  `parse_source`, which registers it under its own. One line. The NATIVE debugger never had
  this, and that contrast is what said the code was wrong rather than the claim.
- **A failed eval ended the session.** One expression that did not evaluate left its
  half-parsed definition behind and did not advance the name counter, so every later eval
  collided with the wreck — a REPL a reader ends with their first typo.
- **A `text` or `vector` result cannot be read at all** (loft#1187). A scalar works and a
  struct works (`stats(3,17)` → `{"lo":3,"hi":17,"span":14}`); a text does not, and all three
  routes to making it work corrupt the store, so `<unavailable>` is the *safe* answer. This
  is the ceiling on the panel, and it is why `38-call-it-yourself.loft` has `vowels(s) ->
  integer` where the design named `caesar(s, k) -> text`, and `nth_prime(n) -> integer`
  where it named `primes_below(n) -> vector`. The panel says so when it happens rather than
  letting it read as a typo.

**And the auto-pause is one statement earlier than the design assumed.** *"Run auto-pauses at
the end of `main` before the frame unwinds"* — there is no breakpoint AFTER a function's last
statement, so `bp end` stops ON main's last line, before it runs. Everything main assigned is
live, which is what the prompt needs; but a program whose only `print` IS that last line
pauses with no output shown, which is why page 38 prints first and asserts after. Resume
finishes it.

**The committed `doc/pkg` cannot carry the two new exports yet (loft#1189).** It is a tracked
binary last rebuilt at `c68aa37d`, and two browser tests load it rather than a fresh build —
so rebuilding it to carry `debug_start` / `debug_command` turns them red on an `engine_host`
native that is a stub in a wasm build. The bundle therefore stays as it was; the release job
rebuilds it before every deploy, so the LIVE panel has the exports, and a page served from a
tree with the old bundle says *"this site's loft bundle predates the panel — rebuild it with
`make wasm`"* rather than showing a dead widget. The panel imports the module dynamically for
exactly that reason: a static named import of a missing export is a link-time error that
stops the whole script before it can report anything.

**The panel drives the page's whole program, not each code block.** The design said *"every
code block on a doc page becomes drivable"*; a topic page's blocks are fragments of ONE `.loft`
file and the session is per program, so the panel runs the page and its line breakpoints
address the page. Measured: 38 of the 39 topic pages start a session (`31-ref-forward` `use`s
a library the browser build cannot resolve, and its panel says so).

**Step 5 is cheaper than this document assumed.** The registry index carries each version's
full `api` array — every `pub` signature *with its doc comment*, re-derived from source by
registry CI so it cannot drift. Tier 2 therefore needs no package installed and no clone: it
is the render the card already does, over a field that is already sitting there. Step 6 is the largest single piece of work in the
document and the only one that is not mostly wiring — it is also the one with no substitute,
because 42 packages of idiomatic loft currently have no reader.

---

## Open questions

- **Does the doc site build get to install 42 libraries?** ⚠ **Answered, and it moved.**
  Tiers 0 and 2 need nothing installed — the index carries the descriptions AND the `api`
  array, so the catalogue, the cards and the API references all render from it. **Tier 3 is
  the one that needs the packages**: the index carries no source. All 42 latest versions were
  in this box's cache, so the pages built; a box without them renders a page saying so and
  the build reports the count. The remaining question is narrower than it was — whether CI
  populates the cache, or whether the source browser is built only where a cache already
  exists.
- **Should the `@F` catalogue grow a library tier?** It is core-only by construction and that
  is defensible — a catalogue of the *language* is a different artefact from a catalogue of the
  *distribution*, and Tier 0 is the second one. The alternative is one catalogue with a `kind`
  of `library`, which would put 42 more entries in a 117-entry set and change what `@F` means.
  Recommendation: keep them separate, and cross-link.
- **Who owns a guide for a library the loft org does not maintain?** Today all 42 are
  first-party, so the question is theoretical — but the answer decides whether Tier 1 is a
  publish requirement or a request.
- **Does the browser REPL need history and multi-line input?** Deferred deliberately: the
  design's claim is that one-expression evaluation against a live frame is the valuable
  90%, and that claim should be tested on `38-call-it-yourself.loft` before any editor work.

---

## See also

- [DOC.md](DOC.md) — how `gendoc` renders a topic, and § *Library documentation*: which
  library pages it writes and what each is generated from
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose itself should read
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the monthly by-hand pass and its watermarks
- [LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md) / [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) — where the guide contract lands
- [WEB_STACK.md](WEB_STACK.md) — the design whose two libraries (`html`, `markdown`) are the worst-documented in the distribution
- [BUS_FACTOR.md](BUS_FACTOR.md) — why documentation outranks code here
