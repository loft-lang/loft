# @PLN183 — documentation in the REPL and the IDE: P0 design + probes

**Status: P0 — design and probes, for review before any code.**  The plan itself (goal, surfaces,
phases) is the tracker issue, [loft-lang/plans#183](https://github.com/loft-lang/plans/issues/183);
this directory holds P0's deliverables:

| file | what |
|---|---|
| `grouping.tsv` + `probe_grouping.py` | probe 1 — every catalogue entry placed in exactly one overview group |
| `constructs.tsv` + `probe_constructs.py` | probe 2 — 28 constructs resolved to their entries by hand, each answer checked against the entry's own code |

Both probes exit 1 on a regression and were made to fail once (an entry dropped, an entry doubled;
a recorded gap that no longer matches).  Run them from the repository root:
`python3 doc/claude/plans/183-docs-in-repl-and-ide/probe_grouping.py`.

## The probes' answers

**Probe 1 — the grouping.**  All 123 entries placed, none twice:

| group | entries | | group | entries |
|---|---|---|---|---|
| Values and types | 14 | | Text | 4 |
| Absence and arithmetic safety | 4 | | Files, stores and data | 8 |
| Operators | 2 | | Concurrency | 2 |
| Collections | 6 | | Standard services | 6 |
| Control flow and patterns | 8 | | Modules, libraries and packages | 6 |
| Functions, methods and generics | 15 | | Tools: run, test, debug, build | 13 |
| **Inside loft (maintainers)** | **35** — every `@I` | | | |

Three findings the grouping surfaced:

- **The 35 `@I` entries are about how loft is built, not what a programmer can use.**  They get a
  group of their own that the overview does not open by default.
- **F120 (`lexer`) and F121 (`parser`) are libraries**, not language features, and neither is in
  the registry (44 packages).  The library overview reads the registry, so today they would appear
  only as features.  Their home is the library side; the catalogue entry can point there.
- **"Operators" holds two entries**, and neither covers what @PLN182 built (below).

**Probe 2 — 28 constructs.**  20 resolve to one entry that shows the construct; 6 are gaps in the
catalogue, recorded so the probe goes red when one closes:

| construct | entry | gap |
|---|---|---|
| `operator compare(self: P, …)` — an operator on a program's type | — | **no entry**: @PLN182 P1–P5b shipped without one |
| `s[1..3]` — a text slice | — | **no entry** |
| `|x| x * 2` — the shorthand lambda | F22 | F22 shows only `fn(x: integer) -> integer { … }` |
| `fn(integer) -> integer` — a function type | F23 | F23 never spells the type (F22 does) |
| `v[1..3]` — a vector slice | F6 | the title promises slicing; the body shows none |
| `x += 1` | F37 | used in 16 entries' examples, shown in none of the operator entries |

**Text search cannot be the lookup.**  The same probe counts the entries a plain search of the
bodies' code offers for each construct: `??` is in 13, ` is ` in 32, `+=` in 22.  Every entry
uses the constructs it does not document, so only the parse can say which construct is under the
cursor, and only a declared key can say which entry documents it.

## Design

### 0. The overview's shape

**Features: a `group:` per entry, carried by the catalogue.**  Each loft-lang/features issue gets
one `group:<key>` label (`group:values`, `group:absence`, … `group:inside`, the keys in
`grouping.tsv`); `make features-gen` copies it into `index/features.json`.  A label, not a body
field, because the catalogue already classifies by label (`kind:feature` / `kind:infra`), and a
label is what the issue list filters on.  Named `group:`, not `subject:`, because `subject:` in
loft-lang/plans names the subsystem a PLAN owns, a different vocabulary.  The generator fails an
entry with no group or two — probe 1, moved into `features-check`.

**Libraries: the registry's own categories.**  The cached signed index already carries, per
package, a `description` and `categories`, and per version the `api` — every `pub` signature with
its one-line doc.  The library overview is: every package, grouped by its first category, with its
description, its latest version, and the version this project uses (its lock entry), marked
installed or installable.  Drilling in lists the `api` items grouped by the type they take as
`self`, then the free functions, then the guide's title when one is installed.  The standard
library is the first entry — `std`, its 32 sections as gendoc already splits them.

**A type's capabilities**, for a type in scope (stdlib, a library's, the program's own): its
methods; the operators and `[]` forms it supports, each with the definition behind it (@PLN182's
`operator` definitions, or the built-in); the interfaces it meets.  Computed from the parsed
program, which the REPL session and the language server each already hold.

### The view in each IDE: one server response, no client code

The language server renders the overview as a small **Markdown site** — one file per page,
linked to each other — into `$LOFT_HOME/doc/<loft version>/`, and asks the client to open its
root with `window/showDocument` (LSP 3.16).  The trigger is a **code action** of kind
`source.loft.overview`, "loft: what can I use here?", because the code-action menu is the one UI
every client already shows (the VS Code light bulb and Source Action, IntelliJ's Alt-Enter
through LSP4IJ, Eclipse quick assist through LSP4E, Neovim's `vim.lsp.buf.code_action()`), plus
the same thing as `workspace/executeCommand loft.overview` for a palette entry.  A type's
capability page is the same action on a type name.

- One implementation: the server writes Markdown, every client already opens a file.
- Drilling down is a Markdown link to the next file: followed in VS Code's preview, `gf` in Neovim.
- The site is regenerated when the loft version or the lock file changes, never during a keystroke.
- **To measure in P3, per client:** that each opens a `showDocument` the server sends.  Not
  measured here for any of the four.  The fallback that needs no `showDocument` is the same pages
  as `workspace/symbol` results.

### 1. Lookup keys: the parse names the construct, the entry declares its keys

Two halves, each with one home:

- **The parse's vocabulary** — a fixed, closed set of construct names the parser and the LSP's
  token walk produce: `op:??`, `op:?`, `op:as`, `kw:match`, `kw:is`, `kw:yield`, `index:vector`,
  `index:hash`, `slice:vector`, `slice:text`, `param:&`, `lambda:short`, `type:fn`,
  `def:operator` … — an enum in code, so a new construct is a compile error until it is named.
  `[]` resolves by the operand's TYPE (the parse knows it), never by text.
- **The entry's keys** — each feature issue lists the construct names it documents, in a
  `Keys:` line the generator copies into `features.json` (the owner edits the issue, as for any
  catalogue fact).

The guard: every vocabulary name maps to exactly one entry, and every key an entry lists is in
the vocabulary.  Probe 2's table is the first 28 rows of that map; its two `no entry` rows are
vocabulary names that would fail it today.

### 2. Offline

The REPL and the language server never reach the network to answer.

- **The feature catalogue ships inside the build**, like the stdlib: `index/features.json` is
  240 KB, embedded and read lazily.  The language server reads it today from the repository's
  own `index/` (`src/lsp.rs`, for maintainer tags), so a programmer outside this repository
  gets nothing.
- **A library's surface comes from the cached index** (`$LOFT_HOME/registry/index.json`, the
  `api` field), present after any `loft install`.  Its guide comes from the installed copy of
  the version this project uses.
- **Missing is said, never guessed:** a library whose index entry predates `api`, or whose guide
  is not installed, says so and names the command (`loft install time`) — the distinction
  loft#1850 drew for the web pages.

### 3. Rendering: one renderer, three back-ends

An entry is parsed once into blocks — title, summary (the first paragraph of `What it is` /
`What it does`), signatures, the first example, the page link — and written by three back-ends:
plain text (the REPL, wrapped to the terminal), Markdown (hover and the overview site), HTML
(gendoc).  A hover keeps the summary, the signature and the link, and drops the example past a
fixed length.  P1 switches gendoc to it with the pages byte-identical, which is what makes the
three unable to drift.

### 4. Runnable examples

An entry's example is the first fence under `## Example` — the one `tests/docs/features/*.loft`
already runs, so it is known to work (88 entries have one; no `@I` entry does).  The REPL runs it
in a **scratch session**, not the user's: an example defines its own `struct` and `fn`, and in
the user's session those would collide with the user's own names (F-OneBody).  `:doc … load`
takes it into the session on purpose.

### 5. The guard

One test walks the whole catalogue and every library `api` item and asks each surface for it:
the REPL `:doc`, the overview site, the hover.  Each answer must be the renderer's output for
that entry.  With P1's single renderer the guard checks the ROUTING (every entry reachable,
every construct resolved, nothing doubled), and the renderer's own test checks the text.

### REPL commands

`:doc` alone opens the overview (features, then libraries); `:doc <query>` looks up a construct, a
feature (`@F2`, `??`), a stdlib or library function, or a library.  `:api <library> [filter]`
and `:ops <Type>` are the two drill-downs named directly.  `:features` and `:libs` are the two
halves of `:doc` on their own.  Every page ends with its web page's address.

## What P0 asks of the owner

1. **The six catalogue gaps** (probe 2): an entry for operators on a program's own type, written
   with @PLN182; an entry or a section for text slicing; and four body fixes (F22, F23, F6, F37).
2. **The `group:` labels and `Keys:` lines** on the catalogue issues — the grouping in
   `grouping.tsv` is the proposal.
3. **F120 / F121** — keep them as features, or move them to the library side.
