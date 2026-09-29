<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 179 — Scripts in loft: the repo's own tooling, ported and proven against what it replaces

Plan issue: [@PLN179](https://github.com/loft-lang/plans/issues/179).
Value: **G** (the dogfood loop turned on the repo's own tooling) · effort **L**, in
tranches of S–M that each finish inside the PR cadence.

## Status (REQUIRED)

Open — design ready, nothing ported.  Two loft scripts already exist under `scripts/`
(`build-gallery-examples.loft`, `build-playground-examples.loft`); nothing in the Makefile,
CI or the hooks runs either, so they are the first two subjects for the twin harness rather
than a head start.  Long-term and low urgency by the owner's own framing (2026-09-29): the
plan exists so the ports accumulate as a measured body of examples, not so the population
is cleared fast.

## Goal (REQUIRED)

Every script this repository runs is a loft program, and each port was proven against the
Python or bash script it replaced by running both on the same inputs and comparing the four
channels byte-for-byte — so the pairs are the comparison material `00-vs-python.html` never
had: the same tool, in the language it was written in and in loft, both still runnable.

## The census, as measured 2026-09-29

| what | count |
|---|---|
| script files tracked (`.py` + `.sh`) | 231 — 98 Python (25.3k lines), 133 bash (16.9k lines) |
| of which under `scripts/` | 127 files, 31.7k lines; 5 over 800 lines, 22 in 300–800 |
| called from the Makefile / CI workflows / edit hooks | 57 / 41 / 2 (`doc_lint.py`, `check_doc_drift.sh`) |
| `python3` steps in `.github/workflows` | 24 |
| files that shell out to `gh` / `git` / `cargo` / `rustc` / `jq` / `curl` | 58 / 42 / 37 / 26 / 12 / 9 |
| Python files using `re` / `json` / TOML / `argparse` / `subprocess` | 48 / 31 / 37 / 24 / 40 |
| **files needing neither a subprocess nor a regex** | **5** (813 lines): `doc_review.py`, `api_lint.py`, `install_claude_hooks.py`, `registry_schema_gate.sh`, `browser/coop_server.py` |
| start-up, `println("hi")`, warm, this box | loft (release) 75–80 ms · `python3 -c` 13 ms |

The last row is the number a hook or a per-file CI step pays on every call; where it goes
(the `default/*.loft` load, after @PLN52) is measured in strand 1, not assumed.

**What loft has today for a script:** file I/O and `list_dir`, `json_parse`, `arguments()`,
`env_variable(s)`, stdin as `host_input`, `eprintln`, `assert`/`panic` (a non-zero exit),
`now()`; the registry's `arguments` (CLI flags + `--help`), `regex`, `time`, `crypto`
(hashes), `markdown`, `html`, `web`; and `lib/git` — git as a typed library over a closed
query vocabulary (`src/git_query.rs`, @I117, @PLN119 arc F).

**What it lacks, and the doctrine that decides how each gap closes:** no TOML reader, no
explicit `exit(code)`, no typed access to `cargo`, `gh`, `rustc`, `jq`, `curl` — and no
general `run(cmd, args)`, by decision ([@PLN119 § Why not a subprocess
primitive](119-out-of-process-libraries/README.md)): an external command lives INSIDE a
vetted library that names the questions and builds the argv itself.  So this plan never asks
for a subprocess primitive.  Each tool the scripts lean on gets the `lib/git` treatment or
disappears: `jq` is `json_parse`; `curl` and `gh` are HTTP against a JSON API, which the
`web` library already speaks, so a pure-loft `github` library needs no native at all;
`rustc` is only ever reached through `cargo` or loft's own build phase; `cargo` is the one
tool that earns a native in the `git_query.rs` shape (`metadata`, `build`, `test`, each a
closed query).

## The one invariant, and the harness that checks it

> **A port is admissible only when the original and the port, run on the same inputs, leave
> the same world: identical stdout, stderr, exit status and written files.**

That is the `make falsify` / `behavior_golden` shape (four channels apart, never a summary
verdict) applied to a script.  The **twin harness** — `make script-twin NAME=<x>` — runs
both sides from `tests/comparisons/scripts/<name>/` on the fixtures kept there, into a
scratch directory each, and diffs the four channels.  Two rules keep it honest:

- **No network and no live tool in a twin.**  A script that asks `gh`, `cargo` or `git`
  gets its answers from a RECORDING (`LOFT_TOOL_RECORD=<dir>` in the typed library; the
  Python side reads the same directory through a ten-line shim), so both sides consume the
  same bytes and the twin runs in CI.  A recording is refreshed by hand and committed; a
  twin that needs the live tool is not a twin.
- **The harness must be able to fail.**  Its own first test is a deliberately divergent
  pair (one channel off, each of the four in turn); a twin that cannot go red proves
  nothing (`loft-plan-workflow` § Cutting a phase, lower bound).

## Sub-arcs

Each strand is cut so the old path and the new one run at once and are compared exactly
(upper bound), and so it can go red on its own (lower bound).

### Strand 1 — Census + ratchet (XS–S, the first loft script)

`scripts/script_census.loft` — in loft, because it needs only file reading and text
search: per script its language, lines, callers (Makefile / CI / hook / none), the external
tools it names, its I/O contract (prints only · exit-code gate · writes files) and the
tranche it falls in.  `make script-census` prints the table; a `--count` scalar is the
**ratchet**: the Python+bash line count under `scripts/` that must not grow.  Also measures
the start-up row above with `LOFT_PROFILE` and files the answer.  Validation: the table
agrees with the hand counts in this file (they were made by grep; a disagreement is a
finding, whichever side is wrong).

### Strand 2 — The twin harness (S)

`scripts/script_twin.sh` (bash, deliberately: it must run while no loft binary exists, and
it is the last script this plan ports) + `tests/comparisons/scripts/README.md` + the
divergent self-test.  Gate: the self-test goes red on each channel; the two existing loft
scripts run under it against the JS files they generate (the committed file is the
"original" side).  Nothing else depends on this strand, and it can fail alone.

### Strand 3 — The subprocess-free tranche (S–M)

The five scripts above, largest first (`doc_review.py` 370, `api_lint.py` 296), skipping
`coop_server.py` (a server; strand 5 decides its fate).  Expected gaps, closed here per the
dogfood loop before the next tranche starts: a **`toml` library** in `loft-libs-core` (37
scripts read TOML; loft parses `loft.toml` in Rust today and exposes nothing), **`exit(code)`**
(or the documented rule that `assert` is exit 1 and a gate wants a code — decide once, in
`STDLIB.md`), and whatever the `regex` library turns out not to cover.  Each gap is a fix or
a library with its own test, never a workaround in the port.

### Strand 4 — Typed tool interfaces (M, one tool per phase, one consumer each)

| tool | route | first consumer |
|---|---|---|
| `jq` | `json_parse` — nothing to build | the smallest `jq`-only bash script |
| `curl` / `gh` | `github` library, pure loft over `web` (issues, labels, PRs, workflow runs, releases — the vocabulary the 58 scripts actually use, measured by strand 1) | `work-issues.sh` (`make work`) |
| `git` | `lib/git`, extended query by query | the first `git`-reading report script |
| `cargo` | a native in the `git_query.rs` shape: `metadata`, `build`, `test` as closed queries | `check-rlib` or `rewrite_census.py` |
| `rustc` | never direct — through `cargo` or `loft --native` | — |

Every interface lands with its recording mode (strand 2's rule) and with its first consumer
ported and twinned; an interface nobody calls is green by construction and is not a phase.

### Strand 5 — Tranches by caller class (M–L, repeating)

1. **Makefile-only reports** — a divergence costs nothing, so they go first and in bulk.
2. **CI gates** — in SHADOW: the workflow runs both, `script_twin` compares, a divergence
   fails the leg; after N green runs on unrelated PRs the Python side is removed.
3. **Hooks** (`doc_lint.py`, `check_doc_drift.sh`, `session-start.sh`) — last, because a
   hook runs before any build exists; it needs an installed release binary (@PLN142) and a
   fallback to the original while none is present.  Start-up cost matters most here.
4. **Never, unless the owner says so:** `registry-sign.sh` / `registry_maintain.sh` (signing
   and the registry's own maintenance — a port changes a security surface), `release-checklist.py`
   until the `github` library has carried a full release cycle, and `coop_server.py`.

### Strand 6 — The comparison yield (S, continuous)

When a port replaces its original, the original does not vanish: it moves to
`tests/comparisons/scripts/<name>/orig.py` (or `.sh`) beside `port.loft` and the fixtures,
so the twin keeps running and the pair stays a **claim as a program** in the
`tests/comparisons` sense.  A generator (loft) renders the pairs as a page beside
`00-vs-python.html`: per script the two sources, lines, the start-up and run-time row from
the twin, and the constructs the port needed — the language comparison the owner asked for,
made of tools that are used every day rather than of samples written to be compared.

## Phase ordering

1 → 2 → 3 → (4 and 5 interleave: an interface, then the tranche it unblocks) → 6 runs from
the first replaced script onward.  Strand 3 is the first tranche that ships value; 1 and 2
are the instruments and are cheap.

## Open design questions

1. **Replace or accompany?**  Recommendation: replace after the shadow period, with the
   original kept under `tests/comparisons/scripts/` (strand 6).  Two live copies of one
   tool drift; one copy plus a gated twin does not.
2. **Where does the `github` vocabulary stop?**  `gh` is used for issues, labels, PRs,
   workflow runs, releases and `api` calls; strand 1 measures which, and the library carries
   only those.  A script needing an endpoint outside it extends the library, never calls
   `web` directly from the script.
3. **The ratchet as a gate.**  Recommendation: from strand 1, `make ci` fails when a NEW
   `.py`/`.sh` appears under `scripts/` without a `# why-not-loft: <gap>` line naming the
   gap that stopped it being loft — the line is a finding for strand 3/4, and the
   population stops growing while it is being ported.

## Cross-arc dependencies

- @PLN119 — the typed-library doctrine and `lib/git` as the shape for `cargo`.
- @PLN142 — a user-local install, which is what a hook needs before a build exists.
- @PLN102 arc E — `behavior_golden`'s four-channel comparison; the twin is its per-script form.
- @PLN52 — stdlib fast start; the 75 ms start-up row is re-measured against it in strand 1.
- @PLN91 — the self-hosting epic; this plan is the tooling floor under it, not one of its strands.

## See also

- [`tests/comparisons/README.md`](../../../tests/comparisons/README.md) — the claim-as-a-program doctrine strand 6 extends.
- [`119-out-of-process-libraries/README.md`](119-out-of-process-libraries/README.md) — § Why not a subprocess primitive.
- [`STDLIB.md`](../STDLIB.md) § Environment · [`LIBRARY_AUTHORING.md`](../LIBRARY_AUTHORING.md) — where `toml` and `github` are built.
- [`CI_BUDGET.md`](../CI_BUDGET.md) — the shadow runs of strand 5 must stay inside the 20-minute PR rule.
