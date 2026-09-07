
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
**Edit ONLY this repo** — the symmetric half of the consumer's "the engine is read-only" rule.
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
make ci                                  # fmt → clippy → test (full local gate)
make test                                # clippy + test → result.txt
make check-rlib                          # 1s pre-flight: is libloft.rlib current? RUN IT
                                         #   BEFORE a bare `cargo test` — `cargo build
                                         #   --bin loft` never rebuilds the lib rlib the
                                         #   native tests link, and a bare `cargo test`
                                         #   builds no rlib either (`make ci` builds all
                                         #   three itself, so it needs no pre-flight)
./scripts/find_problems.sh --subject <name>     # SECONDS — the tight loop; use this while
                                         #   iterating, not `make ci`.  Subjects: parser scopes
                                         #   codegen runtime store wasm packages lsp sql docs
                                         #   host (`--list-subjects` to see them + exclusions).
                                         #   Shape: --subject while iterating → the two clippy
                                         #   variants + fmt → ONE `make ci` before committing.
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
make profile ARGS="--interpret p.loft"   # which loft FN/LINE/PATH burns the time; PROFILE_FLAGS=
                                         #   "--mem" heap by loft line at the PEAK, "--paths" the
                                         #   paths that reached each allocation, "--engine" perf
                                         #   over loft's own Rust.  `make profile-corpus` checks
                                         #   the instruments against known answers — PERFORMANCE.md
make index ; ./scripts/idx tag:@P259     # rebuild + query the tracker index (prefer over grep -rn)
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
(`list` · `check` · `sites <tag>` · `dups`); `check` gates that every citation resolves and no
rule is defined twice.

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
   ⚠ **Nor is "the work looks finished" an ask.** A PR is opened when BOTH streams reach a stable
   point — most issues fixed, a clean endpoint — and **the owner is the one who determines that**,
   not the agent that judges its own branch ready. Accumulated commits on a work branch are not a
   reason to propose one either: the sibling checkout cherry-picks what it needs, so work in
   flight is reachable without a merge. Keep committing and pushing to the branch; wait to be told.
   ⚠ **Target cadence is roughly ONE PR PER DAY of work**, and the owner actively looks for the
   stopping point when work spills into a following day. That shapes the work rather than the
   asking: prefer a coherent unit that FINISHES inside a day over starting something that will
   straddle. And the bar for that unit is not "green" — a PR that looks OK while carrying
   internal regressions is worse for a language's users than no PR, so a walk ships only with its
   own verification complete (both backends, the matrix built and hand-checked, guards falsified,
   side-findings filed rather than left implicit).
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
[COROUTINE.md](doc/claude/COROUTINE.md) (1.1+) · [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md).

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
[PERFORMANCE.md](doc/claude/PERFORMANCE.md) benchmarks + profiling (its oracle: [PROFILE_ORACLE.md](doc/claude/PROFILE_ORACLE.md)) · [CI_BUDGET.md](doc/claude/CI_BUDGET.md) what runs
when + the 20-min PR rule.

**Quality / stability / formal:** [CODE.md](doc/claude/CODE.md) · [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) ·
[QUALITY.md](doc/claude/QUALITY.md) open work · [GOALS.md](doc/claude/GOALS.md) (purpose + goals A–F) ·
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
task.  179 of 257 rules have no code representation, so this is a queue measured in years /
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
different fixes: a struct vs an extracted function) · `LOFT_NO_DEFAULT_HINT` (≥2 trailing
booleans with no default — advertises default parameters, which are under-used and free to
adopt: adding a default is additive, so existing callers keep working) ·
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
PERFORMANCE.md § Design: P2, NATIVE.md.

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

**Owner witness for a mixed-ownership local (loft#1336, default-ON, both backends):** a
heap-record local that OWNS after one assignment (a copy, a minting call) and VIEWS after
another carries a hidden `__own_<name>` naming the store it minted while it still holds it;
the IR releases it by store identity at the rebind or at scope exit, and the local itself is
never-free (`formal/ownership.md` @FR-O-Witness). **`LOFT_NO_OWNER_WITNESS=1`** emits the
pre-witness form: the first bisect step for a leak or a wrong answer in a local that is both
copy-bound and view-bound (a walker `cur: Node? = a; cur = cur.next`), and what the
`LOFT_NO_JOIN_OWN` positive controls set beside their own switch.
