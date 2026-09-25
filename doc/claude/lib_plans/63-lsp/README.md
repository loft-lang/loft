<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Language Server + Debug Adapter — design

**Status: FINISHED (closed 2026-09-25).**  loft-lsp and loft-dap are built through every
LSP.1–LSP.3 step and every DAP advanced tool; the last one, `pause`, is gated by
`tests/dap_transport.rs::pause_stops_a_running_program_and_continue_resumes_it`.  The
follow-ups named below as "Remaining" (the strict-index codeAction, extract-function, the rename
tail, per-worker `par` threads, the dedicated marketplace plugins) moved to **@PLN171**; the
browser-hosted server belongs to **@PLN62**.

Loft's editor-integration story is built around two protocol-agnostic
servers that any modern IDE knows how to consume:

| Server | Protocol | Drives |
|---|---|---|
| `loft-lsp` | LSP (JSON-RPC over stdio) | code intelligence — diagnostics, completion, hover, go-to-def, rename, refactoring, semantic highlighting |
| `loft-dap` | DAP (JSON-RPC over stdio) | interpreter-mode debugging — launch, breakpoints, step, locals, watch |

One server unlocks **first-class support across every editor that
speaks the protocol**: VSCode, Eclipse (LSP4E + DSP4E), JetBrains
(LSP4IJ), Helix, Neovim, Sublime, the future browser IDE (W2), Emacs
(`eglot` + `dape`).  Per-IDE work shrinks to a thin Java / TS / Lua
plugin that just registers the `.loft` content type and points at the
binary.

For native-mode debugging (`loft --native` produces an ELF / Mach-O /
PE binary), see [NATIVE_DEBUG.md](../../plans/34-native-debug/README.md) — that path is
GDB / LLDB-driven and complementary; the source map is shared.

---

## Architecture

```
┌──────────────────┐       LSP / DAP        ┌──────────────────┐
│  IDE / editor    │  ◀──── JSON-RPC ────▶  │  loft-lsp        │
│  (VSCode, …)     │                        │  loft-dap        │
└──────────────────┘                        └────────┬─────────┘
                                                     │ in-proc
                                            ┌────────▼─────────┐
                                            │  loft (rlib)     │
                                            │   parser         │
                                            │   typedef        │
                                            │   scopes         │
                                            │   state          │
                                            └──────────────────┘
```

`loft-lsp` and `loft-dap` are new binaries in this repo (or eventually
in `loft-tools`) that link the existing `loft` rlib for parser /
typecheck / runtime access.  They translate JSON-RPC requests into
calls against `Parser`, `Data`, `State`.  No new compiler — the
existing one is the whole intelligence layer.

The thin per-IDE plugins (`loft-vscode`, `loft-eclipse`, `loft-jetbrains`)
contain only:
- `package.json` / `plugin.xml` declaring the `.loft` content type,
- a 50–200-line shim that spawns the right binary on activation.

---

## LSP.1 — MVP language server (0.8.6)

**Goal:** every LSP-capable editor gets diagnostics, document outline,
and hover on day one.  Smallest unit of work that delivers visible
value across all editors.

### Surface

| Method | Behaviour |
|---|---|
| `initialize` | Advertise capabilities: `textDocumentSync = 1` (full sync), `documentSymbolProvider = true`, `hoverProvider = true`, `diagnosticProvider`. |
| `textDocument/didOpen` | Parse the file, run typecheck, publish diagnostics. |
| `textDocument/didChange` | Re-parse from full text (incremental sync deferred to LSP.2). |
| `textDocument/didSave` | No-op (didChange already triggered a parse). |
| `textDocument/publishDiagnostics` | Emit `(range, severity, message, code)` for every error / warning the parser produces. |
| `textDocument/documentSymbol` | Walk `Data.definitions` for the file; emit a `DocumentSymbol[]` tree (struct → fields, fn → params).  Drives the IDE's Outline view. |
| `textDocument/hover` | At cursor `(line, col)`, look up the symbol and return its type, signature, and `///` doc-comment if present. |
| `shutdown` / `exit` | Cleanup; `loft-lsp` is per-workspace, not per-file. |

### Loft-side prerequisites

Three accessors that don't exist yet:

1. **`Parser::parse_text(text: &str, path: &Path) -> (Data, Vec<Diagnostic>)`**
   Today the parser writes errors to stderr and exits.  LSP needs them
   as a structured list.  The `Diagnostic` shape:
   ```rust
   pub struct Diagnostic {
       pub range: (Position, Position),  // (line, col) start + end
       pub severity: Severity,           // Error | Warning | Info | Hint
       pub message: String,
       pub code: Option<&'static str>,   // stable ID, e.g. "E0023" or "W-unused"
       pub related: Vec<(Range, String)>, // optional secondary spans
   }
   ```
   Source line/column are already tracked by the lexer (`Lexer::line` /
   `Lexer::col`); they need to be stamped into the `Diagnostic` rather
   than only into the eprintln string.
2. **`Data::symbol_at(file: &Path, pos: Position) -> Option<Symbol>`**
   Resolves a cursor position to a definition, function call, or
   variable reference.  Implemented as a lookup over per-file
   position indices built during parse.
3. **`Data::file_symbols(file: &Path) -> Vec<Symbol>`**
   All top-level definitions in the file, ordered by source position.
   Drives `documentSymbol`.

The first accessor is the heaviest — it requires plumbing diagnostic
positions through every `Self::error(...)` call site (~60 sites in the
parser).  None of the changes are deep; they're the kind of mechanical
sweep the comment-hygiene pass already established.

### Performance budget

LSP servers re-parse on every keystroke.  For a 1000-line file the
target is **sub-100 ms in release mode**.  Today a release-mode loft
binary parses and typechecks the entire stdlib + a 1k-line user file
in ~80 ms; on incremental edits to one file the budget should hold.
For 10k-line files, full re-parse becomes sluggish (~400 ms); LSP.2
introduces incremental sync to mitigate.

### Risks

- **Parser is two-pass and has hidden global state.**  Pass-1 registers
  every definition, pass-2 fills function bodies.  `Data::definitions`
  grows monotonically; re-parsing the same file from scratch should
  produce the same `Data` but only if pass-1 is idempotent across calls.
  Verify with a regression test that re-parses 10× and asserts identical
  symbol tables.
- **Single file vs. workspace.**  Loft programs are usually multi-file
  (`use foo;` references).  LSP.1 parses one file at a time; if the
  user has unsaved changes in `bar.loft` and types `use bar;` in
  `foo.loft`, the cross-file resolution is stale.  Acceptable for MVP;
  workspace-aware parsing is part of LSP.2.
- **Re-entrancy.**  An IDE may send `didChange` while the previous
  parse is still running.  Solution: serialise per-file via a per-URI
  `Mutex<ParserState>`, with the latest `didChange` superseding any
  in-flight one.

### Build order — small, safe, incremental

Each step is independently landable, has its own verify gate, and the server is a
**usable, shippable LSP the moment S3 lands** (diagnostics alone earns its keep on every
editor). The safety spine: build the transport *in isolation* first; grow the compiler
surface only as **additive accessors** returning structured data — never a rewrite of the
parse path; and test each step at the **protocol level** (a scripted harness, not a live
editor) so nothing silently regresses.

- **S0 — protocol harness (the instrument, first).** A driver that pipes
  `Content-Length`-framed JSON-RPC into `loft-lsp`'s stdin and asserts its stdout, so every
  step is CI-tested without an editor. *Gate:* it drives a handshake **and can fail** (feed a
  bad reply, see it caught) — a harness that can't fail proves nothing.
- **S1 — transport skeleton, no compiler.** `initialize` (advertise only
  `textDocumentSync=full`) → `initialized` → `shutdown`/`exit`. *Gate:* harness completes the
  handshake + clean exit; VS Code connects. Framing/encoding bugs live here — nail it before
  any feature touches it.
- **S2 — structured diagnostics — DONE, lighter than planned.** The plan assumed a ~60-site
  sweep to add positions; in fact loft ALREADY carries positioned, coded diagnostics (@I75;
  @PLN102 arc-E) — every `diagnostic!` site records `DiagEntry { level, line, col, message,
  code }`. So S2 was just the *recipe*: a fresh stdlib-loaded parser (`parse_dir("default")`)
  → `parse_source(buf)` → `diagnostics.entries()`. Gate: `tests/lsp_diagnostics.rs` (clean →
  none; syntax error correctly positioned; unknown symbol reported with its message). Two
  dogfood findings from building the consumer: **(a)** a parser **cannot be re-parsed** on a
  warm base — loft registers every definition *per source* (that is how files read each other on
  `use`), so a second `parse_source` re-registers and conflicts (*"Cannot redefine 'main'"*). So
  loft-lsp uses a **fresh parser per parse** — the correct model, not a stopgap (~80 ms, within
  budget). Incremental re-parse / warm-reuse is incompatible with the source model, not a perf
  step to chase. (An initial attempt to "reset" diagnostics for reuse was the wrong tree — the
  redefine conflict is upstream of diagnostics.)
  **(b) — FIXED.** Deferred/semantic errors (e.g. "Unknown function") used to report at the
  *resolution point* (the cursor's drifted resting place at the enclosing item's terminator),
  not the reference site — so they landed on the wrong line. Root cause: a call is
  type-checked in `parser::call()` *after* its arguments are fully parsed, by which time
  `self.lexer` has advanced past the statement; the seven `diagnostic!(self.lexer, …)` "Unknown
  function" sites read that drifted cursor. Fix: thread the identifier's own `name_pos`
  (already captured by `parse_var`) through `parse_call` → `dispatch_call` → `call()` and emit
  via `diagnostic_at!(self.lexer, name_pos, …)`. The caret now sits on the offending name
  itself (`nope` at line 2 col 3), not the paren or the `}`. Chokepoint fix — the whole
  "Unknown function" family (plain, `len`/`size` arity, method-hint, generic-type) moved at
  once. Regression: `tests/lsp_diagnostics.rs::unknown_symbol_is_reported_at_the_reference_site`
  pins `(2, 3)`; four existing `parse_errors`/`issues` position assertions were corrected from
  the old drifted columns to the name columns. Syntax errors were already exact.
- **S3 — publish diagnostics — DONE.** The document lifecycle (`didOpen` / `didChange` /
  `didClose`) drives a fresh-parse per edit and pushes `textDocument/publishDiagnostics`; the
  editor shows squiggles live and clears them when the buffer goes valid (empty list on a clean
  parse, and on close). The compiler coupling is a pure library accessor —
  `loft::lsp::diagnose(text, name, stdlib_dir)` (new `src/lsp.rs`): fresh stdlib-loaded parser →
  buffer → `Diagnostics`, encapsulating the parser-cannot-be-reparsed rule. The binary owns only
  the wire protocol + stdlib-path resolution (`resolve_stdlib_dir()`, exe-relative like the
  `loft` CLI, so it works from any editor CWD). Mapping: loft's 1-based `DiagEntry` → LSP 0-based
  `Diagnostic`; the single-point position is widened to underline the whole identifier
  (`token_len_at`, read from the buffer) so the squiggle covers the token, not a zero-width caret;
  `Level` → severity (Error/Fatal 1, Warning 2). Advertises `textDocumentSync {openClose, change:1}`.
  *Gate:* `tests/lsp_transport.rs::diagnostics_publish_on_open_then_clear_on_fix` drives the real
  spawned binary — asserts the pushed notification, uri, count, severity, message, and the exact
  range (`start (1,2) → end (1,6)` on `nope`), then edits clean and asserts the empty clear. The
  clean-buffer-clears assertion doubles as proof the stdlib resolved from the spawned binary.
  **Diagnostics-only is real value across every LSP editor — this is the shippable milestone.**
- **S4 — outline — DONE.** `textDocument/documentSymbol` lists the buffer's top-level defs
  (fn / method / struct / enum / typedef / constant / interface) in source order. The prereq #3
  accessor shipped as `loft::lsp::outline(text, name, stdlib_dir) -> Vec<Symbol>` (not a `Data`
  method — the fresh-parse recipe belongs with `diagnose`): enumerate `0..data.definitions()`,
  keep `def.source == MAIN_SOURCE` and non-`synthetic`, and read the kind + decoded name from the
  shared `api_surface::classify` (made `pub` — one home for the `n_`/`t_<LEN><Type>_`/`Op` name
  decoding) plus `def.position`. Finding: the parser records a def's `position` at the BODY start
  (past the name), so the LSP maps `range`/`selectionRange` to the name located on its declaration
  line (`name_range`, from the buffer) — the Outline entry jumps to the name, not the `{`. Kind →
  LSP `SymbolKind` (Struct 23, Function 12, Enum 10, …); advertises `documentSymbolProvider`.
  *Gate:* `tests/lsp_outline.rs` (3 — kinds+order, excludes stdlib/variants/synthetics, empty) on
  the lib accessor, and `tests/lsp_transport.rs::document_symbol_lists_the_outline` drives the real
  binary and asserts the reply list + the `selectionRange` landing on `Point` (line 0, chars 7..12).
- **S5 — hover — DONE (name-resolution scope).** `textDocument/hover` shows the resolved
  symbol's signature + its `///` doc as markdown. Shipped as `loft::lsp::symbol_at(text, name,
  stdlib_dir, line, col) -> Option<Hover>`. Two findings drove the design:
  - **Resolution.** Prereq #2 envisioned a per-file position index. S5 ships the lighter
    *name-based* resolver instead: the identifier under the cursor is looked up as `n_<word>`
    (free fn) then `<word>` (type/struct/enum/typedef/constant), each falling back to the stdlib
    — so hovering a call site (`area(2,3)`) resolves to the definition, and hovering `print`
    resolves into the stdlib. Not resolved: **methods** (`t_<LEN><Type>_…` need the receiver
    type) and **locals** (need scope) — those still want the position index, deferred to when
    S6/precise-resolution demands it.
  - **Docs (the "other way to get definition info").** loft keeps NO doc field — the lexer
    discards comments — but docs live as `///` lines in the `.loft` *source*, and every
    `Definition` carries `position.{file,line}` into real source (a stdlib symbol → e.g.
    `default/04_stacktrace.loft:41`). So hover reads the `///` block above the declaration from
    the definition's own source — the open buffer for local defs, the file on disk for
    stdlib/library ones (`stdlib_dir`-relative). This is the convention `gendoc` already relies
    on; the signature itself reuses `api_surface::signature_of` (made `pub`) for one type
    spelling. *Gates:* `tests/lsp_hover.rs` (5 — user-fn-at-call-site + doc, stdlib type + doc
    read cross-file from source, struct sig, off-word → None, unknown → None) and
    `tests/lsp_transport.rs::hover_shows_signature_and_doc` on the real binary.
- **S6 — go-to-definition — DONE.** `textDocument/definition` reuses `symbol_at` and emits an
  LSP `Location`: a LOCAL symbol jumps within the open document, a STDLIB / library symbol jumps
  into its source file (a `file://` uri under `default/`, canonicalized). To make the jump land on
  the NAME (matching the outline), `symbol_at` was extended to return the name-precise position +
  the resolved name — it already reads the def's source for docs, so it now also locates the name
  on the declaration line (`name_col_on_line`, shared with the `///` extraction as `read_def_source`
  / `doc_block_above`). Advertises `definitionProvider`. *Gate:*
  `tests/lsp_transport.rs::go_to_definition_jumps_to_local_and_stdlib_defs` on the real binary —
  local jump (same uri, name range), stdlib jump (`file://…/default/…`), blank → null. The S0
  positive control moved off `textDocument/definition` (now implemented) to `textDocument/completion`.

S0–S6 complete **LSP.1**. LSP.2 (below) and `loft-dap` (a DAP adapter over the **existing**
@PLN16 debugger engine — the engine is done, this is a protocol shim, not a new debugger)
layer on the same spine. Every step ships behind three checks: a unit test (compiler side),
a harness test (protocol side), and a real-editor smoke.

### Shipped ahead of LSP.2

Two LSP.2-surface items already landed on the S-spine, plus a cross-cutting perf win:

- **`textDocument/formatting` — DONE.** Wraps the same `loft fmt` formatter
  (`tools/fmt/whole.loft`) via `loft::lsp::Formatter` (compiled once, cached); returns one
  whole-document `TextEdit`, none when already tidy. Advertises `documentFormattingProvider`.
  Gate: `tests/lsp_transport.rs::formatting_returns_a_whole_document_edit_and_noops_when_tidy`.
- **Stdlib startup-cache warm-load — DONE (perf).** The parse accessors (`diagnose` / `outline` /
  `symbol_at`) re-parsed `default/` on every request — ~50 ms, the dominant per-edit cost. They
  now route through `load_stdlib()`, which `startup_cache::warm_load_stdlib`es the precompiled
  `Data` bundle (~4.8 ms, **~10×**) when present, else cold-parses + `save_stdlib_cache`. Verified
  the bundle round-trips `Definition.position`, so stdlib hover / go-to-def are unaffected. The
  binary defaults `LOFT_STDLIB_CACHE` on (honors an explicit override). This is the "integrate
  with the data we already keep" win — the CLI's stdlib cache, now shared by the LSP.
- **Agent/shell frontend — DONE.** The SAME `loft::lsp` accessors, exposed as one-shot `loft` CLI
  subcommands so scripts and coding agents reach the code intelligence without a live editor (a
  THIRD frontend beside the LSP server and the future browser IDE): `loft symbols <file>`
  (outline), `loft def <name> [file]` (signature + `///` doc + location by NAME — a free fn / type
  / const PLUS every `Type.name` method, so `def len` lists `text.len` / `vector.len` / …, the
  method resolution the cursor-based hover can't do), `loft hover <file> <ln> <col>`.
  Human-readable by default, `--json` for structure (mirrors `loft api`). New lib pieces:
  `loft::lsp::lookup()` + a shared `hover_of_def()`. Gate: `tests/lsp_cli.rs` drives the real
  binary. This is dogfood: it replaces the `grep default/*.loft` + read loop for "what's the
  signature of X".
- **Workspace reverse index + find-references — DONE** (LSP.2 prereq #1, first cut).
  `textDocument/references` returns every occurrence of the symbol under the cursor across the
  workspace. `loft::lsp::WorkspaceIndex::build(root)` LEXES every `.loft` file (loft's own lexer,
  so comments / strings / keywords are excluded and `x.len` yields a `len` token) into a
  `name → [Reference]` reverse index; `references_overlaid` overlays open buffers (a file's live
  refs replace its stale disk refs) so unsaved edits count. The binary builds it lazily from the
  `initialize` root, caches it, advertises `referencesProvider`. Also `loft refs <name> [root]`
  (dogfood — workspace-wide occurrences from the shell). **Scope:** name-keyed / lexical, not
  type-resolved — `references("len")` returns every `len` token across types (a precise,
  comment-aware finder, not scope-aware). *Gates:* `tests/lsp_refs.rs` (cross-file + comment
  exclusion + overlay) and `tests/lsp_transport.rs::find_references_spans_the_workspace`.
  Follow-ups: type/scope-aware precision (resolve each occurrence to confirm the binding), and
  index invalidation on `didSave`.
- **Rename — DONE.** `textDocument/rename` turns the references into a `WorkspaceEdit`
  (`{uri: [TextEdit]}`), and `textDocument/prepareRename` returns the identifier's span +
  placeholder (or null on a non-renamable target). **SAFE by default** (`loft::lsp::plan_rename`):
  refuses a new name that isn't a valid identifier; refuses when the old name resolves to a
  **standard-library** symbol (`lookup(old)` non-empty → `print`/`len`/… are un-renamable); and
  refuses when any reference lives in a stdlib file — so a rename can never corrupt `default/`.
  `prepare_rename` applies the stdlib gate too, so the editor won't even offer a rename box on a
  stdlib name. Advertises `renameProvider {prepareProvider}`. Scope inherits find-references'
  lexical nature — best for GLOBAL user symbols with a workspace-unique name; a local/reused name
  is renamed everywhere it appears (the type/scope-precision follow-up addresses this). *Gates:*
  `tests/lsp_rename.rs` (valid-identifier guard, rename-user + refuse-stdlib/invalid/no-refs,
  prepareRename gates stdlib) and `tests/lsp_transport.rs::rename_produces_a_cross_file_workspace_edit`
  (real binary: prepareRename span, cross-file WorkspaceEdit, invalid-name → error).

### Tag integration — tracker knowledge in the IDE (loft dogfood)

Surface loft's own `@`-tracker system (issues / features / plans) inline, reading the
ALREADY-generated `index/tags.json` + `index/features.json` (`make index`; queried by
`scripts/idx`). Lights up when the workspace is the loft repo (or any tree using the tag
convention + `make index`); **inert elsewhere** (the index files are absent) — which fits the
audience exactly: agents/devs working ON loft. The index is generated so it can lag until the
next `make index` — advisory, like any index-backed feature. Steps:

- **T1 — hover on a tag — DONE.** A cursor on an `@`-tag token (`@P259` / `@PLN63` / `@F7` /
  `@I81` / `@GH247`) hovers its tracker info instead of a symbol. `@F`/`@I` pull the title +
  first-paragraph description from `features.json`; `@P`/`@GH`/`@PLN` show the first indexed
  reference's context; a deterministic issue URL per family (`@GH`→loft, `@PLN`→plans,
  `@F`/`@I`→features). Rendered as markdown with a clickable `[issue]` link.
- **T2 — document links — DONE.** `textDocument/documentLink` returns a link per tag that has an
  issue URL (`@GH`/`@PLN`/`@F`/`@I`), at the tag's range → ctrl-clickable. Advertises
  `documentLinkProvider`.

Prereq (DONE): `loft::lsp::TagIndex` reads `index/tags.json` (required) + `features.json`
(optional) from `<workspace_root>/index` (captured from `initialize`'s `rootUri`), parsed once +
cached; `tag_at` / `tags_in` detect tokens; `render_tag_markdown` formats. Also a **`loft tag
<@TAG>`** CLI (dogfood — the same lookup from the shell; walks up to `index/`). No new index — it
consumes the tracker index the repo already builds (`make index`). *Gates:* `tests/lsp_tags.rs`
(synthetic index — CI has no generated `tags.json`: feature/URL/summary lookup + token detection),
`tests/lsp_transport.rs::tag_hover_and_document_link` (temp workspace, real binary),
`tests/lsp_cli.rs`-adjacent CLI dogfood.

Remaining tag steps:

- **T3 — broken-tag diagnostics — DONE.** A buffer tag the scanner flagged as broken (a
  `@P`/`@PLAN` reference to no valid issue/plan) publishes a Warning at the tag, folded into the
  diagnostics push on open/change. **Consumes the index's `broken` array verbatim** rather than
  re-deriving validity: the scanner (`scan.loft::tag_is_broken`) reads PROBLEMS.md + the plan dirs
  + the freeze-banner `@P→#` map, so re-implementing it offline would duplicate it and risk false
  positives — the LSP inherits the scanner's exact verdict (`TagIndex::is_broken`). Zero false
  positives; the trade-off is that a freshly-typed broken tag not yet indexed only shows after the
  next `make index` (the index is the source of truth, consistent with `idx broken`). *Gates:*
  `tests/lsp_tags.rs::broken_tags_come_from_the_index_verdict` +
  `tests/lsp_transport.rs::broken_tag_publishes_a_warning` (a NON-broken tag draws no warning).
- **T4 — tag completion — DONE.** Typing a partial `@`-tag completes to known tracker tags:
  `TagIndex::complete(prefix)` enumerates the referenced tags in `tags.json` (never a broken-only
  one) plus the `@F<n>`/`@I<n>` feature/infra tags from `features.json`, as Reference-kind items;
  `lsp::tag_completion_prefix` detects the `@`-token at the cursor (routing to tags, not symbols),
  and `@` is a completion trigger char. Inert outside a loft repo. *Gates:* `lsp_tags`
  (`complete` offers `@F1` + a referenced problem tag, excludes broken; prefix detection) +
  `lsp_transport::tag_completion_offers_tracker_tags`.

Follow-up (DONE): the `TagIndex` reloads when `index/tags.json`'s mtime changes, so a mid-session
`make index` is picked up — `ensure_tag_index` re-stats per request and re-parses only on a change
(*Gate:* `lsp_transport::tag_index_reloads_when_the_index_changes`).

---

## LSP.2 — full editing surface (0.9.0)

**Goal:** parity with what JDT delivers for Java in Eclipse — every
operation a working developer expects from a "real language" IDE.

### Surface

| Method | Behaviour |
|---|---|
| `textDocument/completion` | Context-aware suggestions: members of `expr.`, params of a call, in-scope identifiers, keywords.  Sorted by relevance (in-scope first, then stdlib, then alphabetic). |
| `textDocument/definition` | Jump to the symbol's declaration.  Resolves through `use` chains. |
| `textDocument/references` | Find every read / write of the symbol across the workspace. |
| `textDocument/rename` | Rename in-place across the workspace; `prepareRename` first to validate the target is a renamable identifier (not a keyword / native fn). |
| `textDocument/semanticTokens` | Type-aware token classification: function vs. method vs. constant vs. field, mutable vs. const, locals vs. parameters.  Supersedes the SH.1 TextMate grammar's structural-only highlighting. |
| `textDocument/codeAction` | Quick-fixes: "add missing field", "rename to camelCase", "import `bar`".  Each diagnostic with a known fix produces an action. |
| `textDocument/codeAction` (`refactor.extract` — **extract function**) | Turn a SELECTION of statements into a new function: data-flow over the selection computes which locals are read-before-write (→ parameters) and which are written-then-used-after (→ return value(s); loft tuples cover multiple outputs), synthesize the fn (name + signature + body) and replace the selection with a call. **Protocol side is trivial** (a `CodeAction` carrying a `WorkspaceEdit`); the cost is the intra-function data-flow ENGINE, which reads the parser's per-fn variable/scope tables (`Definition.variables: Function`). loft specifics: honor the deps/ownership model on extracted params; handle `self` when inside a method; refuse across `#rust`/`#native` bodies. **DONE — the E0–E6 spine shipped ([EXTRACT.md](EXTRACT.md)): `lsp::extract_function` + the codeAction, behaviour-preserving (apply → re-parse → run-identical gates) for straight-line + single-loop over scalars/owned-heap incl. methods; refuses `&`-refs (E5) + escaping control flow (E6). Follow-ups: expression extraction, read-only refs, unique naming.** |
| `textDocument/inlayHint` | Inline type annotations: parameter names at call sites, inferred types of `let`-style locals. |
| `textDocument/formatting` | **DONE** (shipped ahead of the rest of LSP.2) — runs the SAME formatter the `loft fmt` CLI uses (`tools/fmt/whole.loft`, compiled once via `loft::lsp::Formatter`), returns one whole-document `TextEdit` (none when already tidy). Gate: `tests/lsp_transport.rs::formatting_returns_a_whole_document_edit_and_noops_when_tidy`. (The old `loft --format` Rust formatter was removed; `loft fmt` is the entry point.) |

### Loft-side prerequisites

1. **Workspace symbol index.**  A `Data` per file is fine for LSP.1;
   LSP.2 needs a `Workspace` aggregate with cross-file resolution and
   incremental update on `didChange`.  Naturally a HashMap keyed by
   `(file, def_nr)` plus reverse indices keyed by name and by
   `Symbol → Vec<Reference>`.  **First cut DONE** — `loft::lsp::WorkspaceIndex`
   (name-keyed reverse index of identifier occurrences).  Remaining: the
   type/scope-resolved layer (step F) + `didSave` incremental update.
2. **Completion engine.**  At cursor `(file, line, col)` resolve the
   syntactic context (after `expr.`, inside fn-call args, top-level)
   and return a ranked candidate list.  ~MH effort — the first
   completion that's *helpful* not *noisy* takes work.  (Step C; the
   ranking primitive `suggest_similar` + `member_access` already exist.)
3. **Fix-it catalogue.**  Most diagnostics already know the fix
   ("add `&` here", "type was `text`, expected `integer`").  Surface
   each as a `WorkspaceEdit` the IDE can apply.  (Steps A+B; the fixes
   are computed today, only emitted as message prose — step A structures them.)

### Build order — remaining features (REUSE-first)

The inspection (2026-07) confirmed loft already does the analysis these features
need — they are mostly WIRING existing outputs to LSP, reusing the `WorkspaceEdit`
machinery from rename.  The reusable substrate, cited once:

- **`diagnostics::suggest_similar(name, &candidates)`** (pub, Levenshtein ≤2) — the
  nearest-name primitive behind every "did you mean 'X'"; plus `Parser::suggest_function_name`
  / `suggest_field_name` (pub) and `find_method_receivers`.  Feeds completion + codeAction.
- **`api_surface::classify(data, d) -> (kind, name)`** (pub) — the fn/method/struct/enum/…
  classification = semantic-token kinds + completion-item kinds.
- **`Data::variables() -> &Function`, `Function::var_type(var_nr) -> &Type`, `Data::type_name_str`**
  — per-local resolved types for inlayHint (the same spelling hover uses).
- **`Parser.member_access: HashMap<(type_def, member), Vec<String>>`** — the parser already
  records member accesses → `expr.` completion + scope/type resolution.
- **`DiagEntry { level, message, file, line, col, code }`** — `code` is the stable handle; the
  suggestion is currently only in the message PROSE (the one real gap).

Steps, in dependency order:

- **A — structured suggestion on `DiagEntry` — DONE.** Added an additive
  `suggestion: Option<String>` to `DiagEntry`, set via `Diagnostics::suggest_last` (a one-line
  follow-up after the diagnostic — no new macro).  Wired at the unknown-fn (`parser/mod.rs::call`,
  both sites), unknown-variable (`parser/objects.rs`), and unknown-field (`parser/fields.rs`)
  "did you mean 'X'" sites; the prose is unchanged (purely additive, 0 regressions across
  parse_errors/issues).  The `code` stays the frozen handle; the suggestion is now machine-readable.
  *Gate:* `tests/lsp_diagnostics.rs::unknown_call_carries_a_structured_suggestion`.  (Remaining
  "did you mean" sites — type/variant/vector — are additive, wire as needed.)
- **B — `textDocument/codeAction` — DONE.** The published LSP `Diagnostic` round-trips the
  suggestion on its `data` field; a `codeAction` echoing that diagnostic back yields a
  `CodeAction {title: "Change to \`X\`", kind: "quickfix", edit}` whose `WorkspaceEdit` replaces
  the token's range with the suggestion — no re-parse (the editor supplies the diagnostic).
  Advertises `codeActionProvider`.  *Gate:*
  `tests/lsp_transport.rs::code_action_offers_the_did_you_mean_fix` (real binary — `data.suggestion`
  published, then the quick-fix edit).  **Follow-up (`#superseded` steer) DONE:** the @PLN102 arc-C
  steer now carries the successor `Y` as a `suggestion` and — for FREE-FUNCTION calls — emits ON the
  call name (threaded the name position into `call_nr`/`call_with_named`, `diagnostic_at!` instead of
  the drifted cursor: `steer.loft:5:21`→`5:7`), so the quick-fix replaces `old_add` with `new_add`.
  Method / operator paths keep the cursor caret + no suggestion (a quick-fix there could replace the
  wrong token); method-name precision (via the fields.rs member position) is a follow-up.  *Gates:*
  `lsp_diagnostics::superseded_steer_carries_a_structured_suggestion_on_the_call_name` +
  `lsp_transport::code_action_offers_the_superseded_steer_fix`.  **Remaining:** the strict-index lint
  (`for i in 0..len(s){s[i]}` → `for c in s`) is a STRUCTURAL rewrite (a wider range + synthesized
  edit, not a token swap), so it stays a separate follow-up.
- **C — `textDocument/completion` — DONE (first cut).** `loft::lsp::complete` resolves context
  from the buffer at the cursor: after `expr.` → the receiver type's members (struct fields / enum
  variants / methods, deduped so a virtual-field-method shows once as a method); otherwise the
  in-scope GLOBAL names (`Data.definitions` via `classify`, methods excluded) + keywords
  (`lexer::keywords()`, now `pub`), filtered by the typed prefix (an empty prefix restricts to the
  user's own defs so the stdlib isn't dumped).  Receiver resolution: a TYPE name directly, or a
  VARIABLE's type via a function's `variables()` table (`name_exists`/`var`/`var_type` +
  `type_def_nr`).  **Scope-precise via @PLN115:** the receiver variable resolves in the function
  ENCLOSING the cursor (the top-level fn with the greatest declaration line at or before it), so a
  same-named `v` typed differently in two functions offers the right members in each; falls back to
  the any-function scan.  `classify`→LSP `CompletionItemKind`.  Advertises
  `completionProvider {triggerCharacters: ["."]}`.  *Gates:* `tests/lsp_completion.rs` (prefix→
  globals+keywords, `p.`→fields+method no-dup, `Shape.`→variants, scope-precise receiver) +
  `tests/lsp_transport.rs::completion_offers_members_after_a_dot`.  The S0 positive control moved
  to `textDocument/signatureHelp`.  **Follow-ups DONE:** the enclosing function's in-scope LOCALS +
  PARAMETERS are offered too (kind Variable, @PLN115 scope-precise — the call-argument names the
  global scan can't see), and prefix matching gained fuzzy ranking (a case-exact prefix ranks above
  a case-insensitive one; a 4+-char prefix falls back to a Levenshtein-≤2 match, so a typo'd `prnt`
  still offers `print`). *Gates:* `lsp_completion` (in-scope locals offered + scope-precise; fuzzy
  typo match + short-prefix miss).
- **D — `textDocument/semanticTokens/full` — DONE.** `loft::lsp::semantic_tokens` lexes the
  buffer and classifies each identifier by `token_kind` (`n_<name>`→function, `<name>`→struct/
  enum/type/constant/interface, keyword) — the same name-lookup references/completion use.  The
  binary delta-encodes to the LSP flat int array; `semantic_token_types()` is the declared legend
  (keyword/function/method/struct/enum/type/variable/interface).  Note: loft's structural keywords
  (`fn`/`struct`/`if`/…) lex as *tokens*, not identifiers, so they stay the grammar's job — which
  is correct: semantic tokens add the TYPE-aware layer (is `Point` a struct? is `area` a
  function?) the grammar can't.  Advertises `semanticTokensProvider {legend, full}`.  *Gates:*
  `tests/lsp_semantic.rs` (Point→struct, area→function, integer→type, sorted) +
  `tests/lsp_transport.rs::semantic_tokens_full_returns_encoded_tokens`.  **Sharpened via
  @PLN115:** each token is matched against the resolution index by position first, so LOCALS
  (`variable`), METHODS (`method`), and FIELDS (a new `property` legend entry) are now classified,
  then falls back to the name lookup for globals/types/keywords.
- **E — `textDocument/inlayHint` — DONE (via [@PLN115](../../plans/115-resolution-index/README.md) S7).**
  Was deferred because the parser's variable-table positions (`Variable.source`) were not
  reliably populated in the fresh-parse path (only the first local per fn got one).
  The resolution index fixed that: a local binding's declaration is its earliest recorded
  occurrence (a `name =` write), so `lsp::inlay_hints` reads `var_type` → `type_name_str`
  there and emits `: <type>` for EVERY assignment-local (params + loop/lambda binders skipped).
  Server advertises `inlayHintProvider`.
- **F — type/scope-aware precision — v1 DONE (local scoping).** The first, highest-value slice:
  distinguish a GLOBAL symbol (a top-level def or method — workspace-wide) from a LOCAL (a
  variable / parameter — confined to its enclosing function block), and scope find-references /
  rename accordingly.  `loft::lsp::reference_scope` classifies via `is_global_symbol` (a
  free-fn / type / const / **method** name) — anything else the buffer names is a local; the
  enclosing block comes from `enclosing_block` (a lexer brace-depth scan, so braces in strings /
  comments don't miscount).  `scoped_refs` then narrows a local's references to that block in the
  same file.  Result: renaming a local `x` in one function no longer touches a same-named `x` in
  another — the precision win.  Globals/methods keep the workspace-wide behavior.  Achieved by
  SOURCE-scan, no parser instrumentation.  *Gates:* `tests/lsp_scope.rs` (locals→their function,
  globals/stdlib→Global; `scoped_refs` file+range filter) +
  `tests/lsp_transport.rs::rename_a_local_scopes_to_its_function` (real binary).
  **F (v2+) — DONE via [@PLN115 parse-time resolution index](../../plans/115-resolution-index/README.md)
  (S1–S7 + consumers, all shipped).** The parser now RECORDS `(position → resolved binding)` —
  `Local`/`Global`/`Field`/`Method` — gated (byte-identical off), and every LSP feature reads it:
  - **references/rename of a LOCAL** — exact binding, a same-named field (`p.x`) excluded (S4).
  - **method find-references** — `text.len` across the workspace, excluding `vector.len` (keyed
    on the receiver type) — the per-occurrence resolution v1 lacked.
  - **completion `expr.` receiver** — scope-precise to the enclosing function.
  - **semanticTokens** — locals (`variable`), methods (`method`), fields (`property`) classified.
  - **go-to-definition + hover** — resolve locals + methods (into the stdlib) + fields, precisely.
  - **inlayHint (E)** — the originally-blocked feature, now shipping.

  loft is flat-scoped per function, so "shadowing / block scope" folded to per-occurrence
  binding identity (no intra-fn shadowing exists). **@PLN115 tail DONE:** declarations made
  outside `parse_var` are now recorded for **parameters**, **`for` binders**, and **`fn(e: T)`
  lambdas** (an `Occurrence.declaration` flag + a `pending_param_positions` side-table; the
  `for` binder's `var_nr` from `parse_for_iter_setup`), so their references/rename take S4's
  precise path — a same-named field is excluded, and a rename edits the signature/binder. The
  byte-identical-off gate held at every site. Remaining (safe F-v1 fallback): short `|x|`
  lambdas, `for` destructure / `#fields`, a param with a lambda default, constants; and S6's
  exotic member paths.

Smaller follow-ups (all DONE): workspace-index invalidation on `didSave`; `TagIndex` mtime refresh;
**T4** tag completion; step-C in-scope locals + fuzzy ranking; the `#superseded`-steer codeAction
quick-fix (step B).  Remaining follow-up: the strict-index lint's codeAction — a STRUCTURAL rewrite
(`for i in 0..len(s){s[i]}` → `for c in s`), not a token swap, so it needs a wider range + synthesized
edit.  **Extract-function** (`refactor.extract`, the table row above) stays L-effort and separate —
it needs the data-flow engine, not just wiring; its **design is [EXTRACT.md](EXTRACT.md)** (the
E0–E6 spine + the parser code points the engine reads).

### Incremental parsing

LSP.2 introduces partial re-parse: on `didChange` with small ranges,
re-parse only the affected function body.  Loft's parser is
top-down recursive-descent without global state inside `parse_function`,
so re-parsing one function in isolation is feasible.  ~M effort.
Skip until LSP.1 measurements show real users hitting the 10k-line wall.

### Risks

- **Rename across `#native` boundaries.**  A user can't rename a function
  whose Rust body lives in `#rust "..."` annotations without breaking
  the binding.  The fix-it should refuse with a clear message.
- **Rename that touches imported libraries.**  The workspace includes
  vendored / installed packages; renaming a stdlib function would be
  catastrophic.  Restrict rename to definitions whose source file is
  inside the project root.
- **Performance.**  Workspace-wide find-references on a 50k-line
  project must complete in under 1 s; otherwise developers stop trusting
  the feature.  Pre-build a `Symbol → Vec<Reference>` index during
  initial parse.

---

## LSP.3 — `loft-dap` debug adapter (0.9.0)

**Goal:** interactive interpreter-mode debugging in any DAP-aware
editor.  Set a breakpoint in `.loft` source, run, hit the breakpoint,
inspect locals, step.

**Full design: [DAP.md](DAP.md)** — the DAP⇆RPC map, envelope mechanics
(`seq`/`request_seq`, handshake ordering, `variablesReference`), worked message
translations, the code-point table over `rpc::handle`, and the D0–D6 gates. The
surface table + spine below are its summary.

### Surface

This table is the **as-built** v1 surface (D0–D6, [DAP.md](DAP.md)). The debuggee runs
**in-process** inside the adapter — no child process, no port (Decision 1); every request
is a translation over the `--rpc` engine's one dispatch chokepoint.

| Request | Behaviour (as built) |
|---|---|
| `initialize` | Capabilities: `supportsConfigurationDoneRequest`, `supportsConditionalBreakpoints`, `supportsEvaluateForHovers`, `supportsTerminateRequest`, `supportsSetVariable`. **NOT** `supportsHitConditionalBreakpoints` (no engine hit-count) nor `supportsStepBack` (see [DAP_ADVANCED.md](DAP_ADVANCED.md)). |
| `launch {program, stopOnEntry?}` | Load the program in-process (RPC `launch`, no run); the run is deferred to `configurationDone`. `stopOnEntry` installs a function breakpoint at `main`'s entry. |
| `setBreakpoints {source, breakpoints:[{line, condition?}]}` | RPC `setBreakpoints`; each line comes back `verified` (breakable in the loaded program) or not. Conditions pass straight through to the engine's resolve loop. |
| `configurationDone` | The deferred launch — RPC `run` (entry `main`); a hit → `stopped`, else `terminated`. |
| `threads` | The single synthetic thread `{id:1, name:"main"}` (one-per-worker `par` is a follow-up). |
| `stackTrace` | **The full runtime call stack** ([DAP_ADVANCED.md](DAP_ADVANCED.md) SF, built), innermost first, each frame at its parked / call-site line (the `replmain_*` wrapper filtered). |
| `scopes` | A `Locals` scope for the requested frame; the top frame's is VE-expandable, a caller frame's locals are leaves (eval is top-frame-scoped). |
| `variables` | The frame's locals from the last stop (a stale reference → empty), **with structured expansion** ([DAP_ADVANCED.md](DAP_ADVANCED.md) VE, built): a struct/vector value drills into its children; a scalar is a leaf. |
| `next` / `stepIn` / `stepOut` | RPC `stepOver`/`stepIn`/`stepOut` → `stopped{reason:"step"}`. |
| `continue` | RPC `continue` → the next stop or `terminated`. A `pause` during it stops the run where it is (`stopped{reason:"pause"}`). |
| `dataBreakpointInfo` / `setDataBreakpoints` | **Data breakpoints** ([DAP_ADVANCED.md](DAP_ADVANCED.md) DB, built): watch a scalar local (or a nested field/element) via the engine's watchpoints; a change stops with `reason:"data breakpoint"`. Set at a stop only. |
| `stepBack` / `reverseContinue` | **Reverse execution** ([DAP_ADVANCED.md](DAP_ADVANCED.md) RX, built): a bounded snapshot ring restores the prior state byte-identically. Interpreter-only; I/O not reversed; depth `LOFT_REVERSE_DEPTH` (default 200). |
| `evaluate {expression, frameId, context}` | RPC `eval` in the frame's scope (identifier / field-access / call). |
| `setVariable` / `setExpression` | RPC `setValue` → the refreshed frame value. |
| `disconnect` / `terminate` | RPC `disconnect`; `terminated`, loop ends. |

**Advanced tools** ([DAP_ADVANCED.md](DAP_ADVANCED.md), each a small-step spine grounded in
probe findings) — **all built**: structured **v**ariable **e**xpansion (VE), multi-**f**rame
**s**tack (SF), **d**ata **b**reakpoints via watchpoints (DB), and **r**everse e**x**ecution
(RX, a bounded snapshot ring), and `pause` (an async interrupt, 2026-09-25).  Multi-worker `par`
threads remain an honest refusal — one synthetic thread.

### What is already built — loft-dap is a TRANSLATION, not a new debugger

The @PLN16 debug **engine** AND its wire protocol are shipped, so loft-dap adds almost
no debug logic — it translates DAP ⇆ the existing loft-debug protocol:

- **The engine** — breakpoints, conditions, stepping (in/over/out/continue), frame
  capture + `eval` / `setValue`, watchpoints, undo/redo — is a host-agnostic
  `ReplSession` / `State` API ([@PLN16](../../plans/16-debugger/README.md) A–F, M1–M5).
- **The wire protocol** — a complete NDJSON request/response + event stream that is the
  *sole* serialization of every debug capability — is LANDED (`loft debug --rpc`,
  `src/rpc.rs`; [16.P PROTOCOL.md](../../plans/16-debugger/PROTOCOL.md)), with
  `rpc::handle(session, line) -> (responses, disconnect)` as the one dispatch chokepoint
  and `tests/rpc.rs` proving launch → setBreakpoints → run → stopped → eval → continue →
  output → terminated end-to-end. The protocol was *designed as the DAP shape on
  purpose* (request/response + async `stopped`/`output`/`terminated` events), so DAP is a
  translation of it, never a redesign.

**The DAP ⇆ loft-debug map is nearly 1:1:** DAP `setBreakpoints`/`stackTrace`/
`evaluate`/`continue`/`disconnect` are the same-named RPC requests; DAP `next`/`stepIn`/
`stepOut` → RPC `stepOver`/`stepIn`/`stepOut`; DAP `setVariable` → RPC `setValue`; the
RPC `stopped`/`output`/`terminated` events → the DAP events of the same names. loft-dap
holds a `ReplSession` in-process (like `run_rpc` does) and reuses `rpc::handle` — so **no
child interpreter process and no port** (correcting the `launch` sketch below): the
debuggee runs in the adapter, exactly as `loft debug --rpc` runs it.

The DAP-specific work loft-dap actually adds: the `Content-Length` wire framing (reuse
loft-lsp's), the DAP handshake (`initialize`/`initialized`/`configurationDone`), the DAP
envelope (`seq` / `request_seq` / `type`), and synthesizing DAP's `threads → stackTrace →
scopes → variables` drill-down (with `variablesReference`s) from the RPC's flat frame.

### Build order — small, safe, incremental (mirrors the LSP.1 spine)

Each step is independently landable, has its own protocol-level verify gate (a scripted
harness, not a live editor), and grows the DAP surface as a thin translation over
`rpc::handle`. loft-dap is a new binary in this repo (`src/bin/loft-dap.rs`) that links
the `loft` rlib — the loft-lsp shape.

- **D0 — DAP protocol harness (the instrument, first).** A driver that pipes
  `Content-Length`-framed DAP JSON into `loft-dap`'s stdin and asserts its stdout /
  events, so every step is CI-tested without an editor (mirror `tests/lsp_transport.rs`'s
  `Session`). *Gate:* it drives an `initialize` handshake **and can fail** (feed a bad
  reply, see it caught) — a harness that can't fail proves nothing.
- **D1 — transport skeleton + handshake, no engine.** `initialize` (advertise
  `supportsConfigurationDoneRequest`, `supportsConditionalBreakpoints`,
  `supportsEvaluateForHovers`; NOT `supportsStepBack`) → `initialized` event →
  `configurationDone` → `disconnect`. Reuse loft-lsp's framing; `seq`/`request_seq`
  bookkeeping lives here. *Gate:* harness completes the handshake + clean exit; the DAP
  envelope round-trips. Framing / seq bugs die here before any feature touches them.
- **D2 — launch + run + terminated + output.** `launch {program, stopOnEntry?}` →
  `ReplSession` launch + `run` (via `rpc::handle`) → a `stopped{reason:"entry"}` when
  `stopOnEntry`, else run to end → `terminated` + `exited`. The program's `print` output
  (RPC `output` events, captured off the protocol channel) → DAP `output` events. *Gate:*
  launch a trivial program → `terminated`; with `stopOnEntry` → `stopped`. **First
  end-to-end slice: a real program under the adapter.**
- **D3 — breakpoints + stopped.** `setBreakpoints {source, breakpoints:[{line,
  condition?}]}` → RPC `setBreakpoints` → `{breakpoints:[{verified, line}]}`; a hit →
  `stopped{reason:"breakpoint", threadId:1}`. *Gate:* set a line breakpoint, run, assert
  the stop fires at the right line (harness + real binary). **Breakpoint-stop is real
  value across every DAP editor — the shippable milestone.**
- **D4 — the inspection drill-down (threads / stackTrace / scopes / variables).**
  `threads` → the single synthesized thread; `stackTrace` → RPC `stackTrace` frames → DAP
  `StackFrame[]` (frameId); `scopes {frameId}` → `Locals` (+ `Arguments`) with a
  `variablesReference`; `variables {variablesReference}` → the frame's locals
  (name/value/type, already in the stopped frame). *Gate:* at a breakpoint, walk
  threads→stackTrace→scopes→variables and assert the locals-panel content.
- **D5 — stepping + continue.** `next`/`stepIn`/`stepOut` → RPC
  `stepOver`/`stepIn`/`stepOut` → `stopped{reason:"step"}`; `continue` → RPC `continue`.
  (`pause`: BUILT 2026-09-25 — see DAP.md § pause.) *Gate:* step over/in/out and assert each
  stop lands on the expected line.
- **D6 — evaluate + setVariable.** `evaluate {expression, frameId, context}` → RPC
  `eval` → `{result, type}` (identifier / field-access / call, per the RPC); `setVariable`
  / `setExpression` → RPC `setValue` → the updated frame. Conditional breakpoints already
  ride D3's `condition`. *Gate:* evaluate at a breakpoint; set a variable and assert the
  change is reflected.

D0–D6 complete the loft-dap MVP (LSP.3). The Neovim / VS Code launch config
(`dap.adapters.loft = { command = 'loft-dap' }`) then lights it up with zero adapter code
(below). Every step ships behind the same three checks as LSP.1: a unit test (the RPC
driver is already unit-tested), a harness test (DAP protocol side), and a real-editor smoke.

### Loft-side prerequisites — MET by @PLN16 (the reason loft-dap is thin)

All four are already shipped in the debug engine; loft-dap consumes them via
`rpc::handle`, adding none of them itself:

1. **Pause / step API — DONE.**  Not a global per-opcode `PauseFlag`: the engine uses
   [`interpret_set`](../../plans/16-debugger/README.md) — only the functions on the path
   to a breakpoint run interpreted (where the loop's breakpoint check lives), the rest
   stay native/full-speed — so the cost is scoped to a debug session, not the whole
   interpreter. `State::debug_step(Into/Over/Out/Continue)` drives it.
2. **Source-line breakpoint resolution — DONE.**  `set_breakpoint_file_line` maps
   `(file, line)` to the bytecode offsets and registers the pause; the RPC
   `setBreakpoints` request already exposes it (idempotent per-file, DAP semantics).
3. **Variable formatter — DONE.**  The stopped-frame `locals` are rendered to own-format
   `value` strings (a bare heap local read live/in-place via `show_json`, D2), carried in
   the `stopped` event and `stackTrace` — loft-dap forwards them.
4. **Conditional-breakpoint evaluator — DONE.**  `setBreakpoints`'s `condition` runs
   through the engine's driver-side resolve loop (@PLN16 rich-breakpoints, LANDED); DAP
   passes the condition straight through.

### Multi-worker support

`par(...)` and `parallel { ... }` spawn workers that have their own
`Stores` instances.  DAP `threads` returns one entry per active
worker; `stackTrace` operates per worker.  Pausing one worker pauses
all (synchronous-stop semantics) so the user sees a consistent picture.

### Risks

- **Pause-flag overhead — already solved (no new gate).**  The original worry — a global
  per-opcode flag costing ~1 s in a tight loop — does not apply: the shipped engine scopes
  the breakpoint check to the `interpret_set` (only the functions needed to reach the
  breakpoint run interpreted; everything else stays native), so a non-debug run pays
  nothing and no `#[cfg(feature = "dap")]` split is needed.
- **Breakpoint timing.**  A breakpoint set "before" the function is
  parsed (e.g. on a library file the program hasn't reached yet) needs
  to be applied retroactively.  Solution: keep a `pending_breakpoints`
  list, replay it at every parse.
- **Reverse stepping.**  DAP supports `stepBack` / `reverseContinue`
  via `supportsStepBack`.  Loft can't replay a tree-walking interpreter
  cheaply; v1 does not advertise this capability.
- **Debugger-induced state changes.**  `evaluate` could mutate state
  (e.g. `evaluate("x = 5")`).  v1 evaluates in read-only mode; mutations
  require explicit user opt-in.

---

## Eclipse plugin (1.0.0 — IDE.ECLIPSE)

A ~200-line Java OSGi bundle that registers `.loft` with the LSP4E
generic editor and the DSP4E launcher.  No Loft-specific Java code
beyond the bindings.

### Files

```
loft-eclipse/
├── plugin.xml          (manifest: content type, launch config, …)
├── META-INF/MANIFEST.MF
├── src/
│   └── org/loft/eclipse/
│       ├── LoftLanguageServer.java   (extends LSP4E ProcessStreamConnectionProvider)
│       ├── LoftDebugAdapter.java     (extends DSP4E DebugAdapterDescriptorFactory)
│       └── LoftActivator.java        (OSGi bundle activator)
└── icons/
    └── loft.png        (the file-type icon)
```

### `plugin.xml` skeleton

```xml
<plugin>
  <extension point="org.eclipse.core.contenttype.contentTypes">
    <content-type id="org.loft.contentType"
                  name="Loft Source"
                  base-type="org.eclipse.core.runtime.text"
                  file-extensions="loft" />
  </extension>

  <extension point="org.eclipse.lsp4e.languageServer">
    <server id="org.loft.languageServer"
            class="org.loft.eclipse.LoftLanguageServer"
            label="Loft Language Server" />
    <contentTypeMapping contentType="org.loft.contentType"
                        id="org.loft.languageServer" />
  </extension>

  <extension point="org.eclipse.debug.core.launchConfigurationTypes">
    <launchConfigurationType id="org.loft.debug"
                             name="Loft Program"
                             delegate="org.eclipse.lsp4e.debug.launcher.DSPLaunchDelegate" />
  </extension>

  <extension point="org.eclipse.lsp4e.debug.debugAdapterDescriptorFactories">
    <factory class="org.loft.eclipse.LoftDebugAdapter"
             launchConfigurationType="org.loft.debug" />
  </extension>
</plugin>
```

### `LoftLanguageServer.java` skeleton

```java
public class LoftLanguageServer extends ProcessStreamConnectionProvider {
  public LoftLanguageServer() {
    var loft = findLoftLsp(); // PATH or bundled binary
    setCommands(List.of(loft.toString()));
    setWorkingDirectory(System.getProperty("user.dir"));
  }
  private Path findLoftLsp() {
    // 1. $LOFT_LSP env var if set
    // 2. ~/.loft/bin/loft-lsp if installed via `loft install`
    // 3. PATH lookup for `loft-lsp`
    // Falls back with an actionable error message.
  }
}
```

### Marketplace listing

Eclipse Marketplace requires a hosted P2 update site.  Use the standard
Tycho build (`mvn tycho`) inside `loft-eclipse/`.  CI builds the update
site and uploads to GitHub Pages alongside the rest of the docs; the
Marketplace listing points at that URL.  ~1 day of one-off setup, then
zero ongoing cost — every release rebuilds the update site as part of
`make gallery`.

### Optional polish

| Feature | Effort | Status for IDE.ECLIPSE v1 |
|---|---|---|
| Project wizard ("New → Loft Project") | S | Skipped; users use Generic Project |
| Run-config UI (vs. plain DSP4E launch) | S | Skipped; default DSP4E launcher is fine |
| Custom debug perspective layout | S | Skipped; default Debug perspective works |
| Outline view icon set | XS | Skipped; LSP `documentSymbol` maps to default icons |
| Keybindings (F3 go-to-def, etc.) | XS | LSP4E provides these out of the box |

---

## JetBrains plugin (1.0.0 — IDE.JETBRAINS)

LSP4IJ ([JetBrains/lsp4ij](https://github.com/redhat-developer/lsp4ij))
is the JetBrains-side analogue of LSP4E.  Plugin shape mirrors the
Eclipse one: `plugin.xml`, a `LanguageServerFactory`, and pointer at
`loft-lsp`.

The JetBrains marketplace handles all platforms — IntelliJ Community
/ Ultimate, RustRover, PyCharm, GoLand, WebStorm, etc.  One plugin
listing covers them all.

`loft-dap` is wired through LSP4IJ's `DAPRunConfiguration`.  Same
shape as the Eclipse path.

---

## Neovim (IDE.NEOVIM) — SHIPPED (LSP)

Lives in **`editors/nvim/`** (a runtimepath plugin, alongside `editors/vscode` /
`editors/intellij`): `lua/loft.lua` (the module), `ftdetect/loft.lua` (the `.loft`
filetype), `syntax/loft.vim` (structural highlighting — keywords lex as tokens, so
semantic tokens can't colour them), and a setup `README.md`.

**The LSP side is dependency-free** — it uses Neovim's built-in `vim.lsp` (0.8+),
so `require('loft').setup()` is all it takes; no `nvim-lspconfig`. It registers the
filetype, starts `loft-lsp` on each `.loft` buffer (root = nearest `loft.toml` /
`.git`), and sets buffer-local maps (`gd`/`K`/`grn`/`grr`/`gra`/`gO`/`<leader>f`),
inlay hints (0.10+), and omni-completion. Smoke-tested headless on Neovim 0.9.5:
the server attaches and a `textDocument/hover` round-trips (`fn area(p: Point) ->
integer`).

The debug adapter is auto-registered when `nvim-dap` AND `loft-dap` are both
present — inert until [loft-dap](#lsp3--loft-dap-debug-adapter-090) lands, at which
point `setup()` wires `dap.adapters.loft` + a "Run current file" config with no
extra work.

```lua
-- init.lua — point Neovim at the plugin, then:
vim.opt.runtimepath:append("/abs/path/to/loft/editors/nvim")
require("loft").setup()   -- { loft_lsp = "…" } if loft-lsp isn't on $PATH
```

Full setup / keymaps / troubleshooting: **[editors/nvim/README.md](../../../../editors/nvim/README.md)**.

---

## Sequencing across milestones

| Milestone | LSP work | DAP work | IDE plugins |
|---|---|---|---|
| 0.8.5 | (SH.1, SH.2 — TextMate grammar + VSCode bare-bones extension) | — | — |
| 0.8.6 | LSP.1 — diagnostics + outline + hover | — | (none — LSP.1 lights up VSCode + Eclipse + Neovim immediately via the existing LSP4E / nvim-lspconfig integrations) |
| 0.9.0 | LSP.2 — completion + def + refs + rename | LSP.3 — DAP MVP | — |
| 1.0.0 | (polish only) | (polish only) | IDE.ECLIPSE / IDE.JETBRAINS / IDE.NEOVIM dedicated marketplace plugins |
| 1.1+ | (ongoing — formatter, inlay hints, semantic refactors) | (call hierarchy, conditional breakpoints v2) | (Sublime, Helix, Emacs `eglot` snippets) |

---

## Open work (routed in)

- **INSP.J — JSON output mode for `loft introspect` — DONE.** `loft introspect
  --json <file>` emits ONE machine-readable JSON object over the included sections
  (a string field per section — `bytecode`/`rust`/`slots`/`types`, `ownership` if
  requested — in canonical order), via loft's OWN `json` serializer (own-your-
  dependencies).  A tool / the LSP reads a section by key instead of splitting on
  `=== header ===` boundaries; `--json` short-circuits the text/`*-out`/`--diff`
  paths.  Faithful envelope over `emit_all`'s existing emitters — no new analysis
  (the tabular slots/types stay text for now; per-function structured arrays are a
  later step).  *Gates:* `tests/introspect.rs::{json_mode_emits_a_parseable_section_object,
  json_mode_respects_section_selection}` (parsed back with loft's own `json::parse`).
  Routed here from **@PLN12** on its close (machine-readable introspection is an
  editor/IDE concern; the LSP is its natural consumer).

---

## Cross-references

- [NATIVE_DEBUG.md](../../plans/34-native-debug/README.md) — GDB / LLDB integration for
  `--native`-compiled binaries; shares the source map with LSP.3.
- [WEB_IDE.md](../62-web-ide/README.md) — W2–W6 browser IDE; uses `loft-lsp`
  compiled to WASM as its language-intelligence layer.
- [DX.md](../../plans/36-developer-experience/README.md) — SH.1 / SH.2 / DX.1 / DX.3 / DX.4 — the 0.8.5
  developer-experience predecessors.
- [STACKTRACE.md](../../STACKTRACE.md) — TR1.3 `vector<StackFrame>` API
  that LSP.3 reuses for `stackTrace`.
- [Plan-14 viewer-LSP-bridge](../66-viewer-lsp-bridge/README.md)
  — the CLIENT side that consumes `loft-lsp` (this plan)
  alongside rust-analyzer + jdtls.  Plan-14 phase 03 lights
  up `.loft` files in the viewer once LSP.1 here ships.
