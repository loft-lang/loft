<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 10 — Language enhancements harvested from the dogfood pass

**Status:** Closed 2026-05-18.  Phase 10 ships in 0.8.5.

Shipped: 10.1-10.5, 10.7-10.9, 10.12, 10.13, 10.10b, 10.16
(13 sub-steps).  Deferred to a follow-up parser-typer plan
(`plans/future/<NN>-parser-typer-cleanup/` — opened separately):
10.10 (@P278), 10.11 (@P279), 10.14 (@P277), 10.15 (@P281).
10.6 reverted to "wontfix unless 2nd consumer demands" (the
`hash.contains(key)` deferral, see STDLIB.md `## Open work`).

## Goal

The bookend of the @PLN42 dogfood cycle.  Phases 0-9 built
the tracker indexer + viewer integration; phase 07's
loft-native scanner port surfaced a cluster of small loft +
stdlib gaps.  This phase harvests them — turns "we worked
around X because the language doesn't have Y" into "the
language has Y now."

Per [CLAUDE.md § Development cadence](../../../../CLAUDE.md#development-cadence--the-dogfood-loop):

> Build a real consumer → harvest the language lessons →
> fix the language → ship the lessons as a release.

The viewer + indexer were the consumers (phases 04 + 07).
The lessons got catalogued in their canonical homes (P-issues
in PROBLEMS.md, stdlib gaps in STDLIB.md `## Open work`).
This phase implements the cluster that's small enough to ship
in one focused arc.

## Scope — what's IN this phase

The full dogfood-cycle harvest: every P-issue and stdlib gap
that the phase-04 + phase-07 work surfaced.  Mixed effort
(XS through M) — the phase is intentionally L-sized because
its purpose is the BUNDLE.  Sub-steps are independently
shippable; the phase closes when all 8 P-issues + 8 stdlib
items + workaround-removal pass land.

### Compiler / parser / lexer / native codegen (8 P-issues)

| ID | Item | Effort | Workaround it lifts |
|---|---|---|---|
| @P275 | Module-scope `const vector<text>` crashes native (`stores.const_refs[NNN]` reads zero-length slice) | M | scan.loft uses `fn source_roots() -> vector<text> { [...] }` instead of `const SOURCE_ROOTS: vector<text> = [...]` |
| @P276 | `s[i] ?? '<char>'` chain-compare emits rustc E0308 (`i32` vs `char`) | M | scan.loft removes all `??` guards on character slices; relies on surrounding `i < n` bound checks to keep runtime safe |
| @P277 | Local `sorted<T[K]> = []` + `+= [T{}]` re-types to `vector<T>` | M | scan.loft wraps two sorted sets in `struct DistinctSets { tags, links }` so the field's declared type sticks across `+=` |
| @P278 | Parser mis-parses `if X.method(local_var) { … }` with self-slice reassignment in scope | S | viewer hoists `problem_row_summary` into a helper fn so the method-call argument lives outside the self-slice scope |
| @P279 | Type-inference produces `unknown(0)` for conditional reassignment of text branches | S | scan_link_line uses explicit `a_esc: text = json_escape(anchor);` annotation |
| @P280 | Lexer rejects `\0` / `\xNN` / `\u{NNNN}` escapes | XS | scan.loft uses `' '` (space) as the "won't appear" sentinel; documented brittleness |
| @P281 | Two-pass forward-resolution loses fn return types in pass-1 | M | scan.loft moved leaf helpers (`is_digit_leaf`, `basename_leaf`) to top of file; original names kept as one-line aliases |
| @P282 | Compiler warnings printed to stdout under `--native` | XS | Every `make index-loft` invocation pipes through `2>/dev/null \| grep -vE '^warning\|...' \| grep -v '^$'` |

### Compiler tool polish (1 builtin)

| Item | Effort | Workaround it lifts |
|---|---|---|
| `args() -> vector<text>` builtin | XS | scan.loft uses env var `LOFT_INDEX_BUCKETED` as a CLI-arg workaround; viewer doesn't support args at all |

### Stdlib gaps (5 items)

| Item | Effort | Workaround it lifts |
|---|---|---|
| `vector.sort()` + `vector.sort_by(fn)` | S | scan.loft uses `sorted<TagSlot[name]>` as a sort proxy in 3 places; viewer's plan-bucket sort + activity-feed date sort do similar.  Replaces a struct + dedup-on-insert + iteration pattern with a single method call |
| `text.split(text)` overload | XS | scan_link_line walks char-by-char to find `](` because only `text.split(char)` exists today |
| `text.starts_with_at(pos, prefix)` | XS | scan.loft's @PLAN matcher does `line[i+1]=='P' && line[i+2]=='L' && …` per-char instead of `line.starts_with_at(i, "PLAN")` |
| `hash.contains(key)` + key iteration | XS | scan.loft uses `vector<text>` + linear `set_contains` for valid_pids/valid_plans because `hash<T[K]>` isn't ergonomic as a "set of text" |
| `text::escape_html()` | XS | viewer's main.loft rolled its own `escape(s)` — well-known function that belongs in stdlib |

### Stdlib path module (XS-S)

| Item | Effort | Workaround it lifts |
|---|---|---|
| `path::dir(p)`, `path::basename(p)`, `path::join(parts...)`, `path::resolve(base, target)` | XS-S | scan.loft, the viewer, and lib/markdown each rolled their own `dir_of` / `basename` / `resolve_relative`.  Bonus: `file()` API normalises the `./<name>` prefix at root |

## Scope — what's NOT in this phase

These belong in their own arcs (out of scope for the
0.8.5-bound harvest):

  - **JSON emission helpers** (to_json + JsonBuilder)
    — S-M; deserves its own design doc + commit; tracked
    as a STDLIB.md `## Open work` row
  - **`lib/process/`** (subprocess primitive) — full library
    plan in [`lib_plans/67-process/`](../../lib_plans/67-process/README.md).
    Architectural unlock; ships in its own multi-commit arc.
  - **`lib/fs_watch/`** (file-event watcher) — full library
    plan in [`lib_plans/68-fs-watch/`](../../lib_plans/68-fs-watch/README.md).
    Needs Rust host-bridge work; ships in its own arc.
  - **`lib/cache/`** (mtime-invalidated read-through cache)
    — full library plan in [`lib_plans/69-cache/`](../../lib_plans/69-cache/README.md).
    Could fold into 10.x as a stretch if the path module
    work pulls it in for free, but defaults out.

## Sub-steps

Each sub-step ships as one focused commit + tests.  Order
chosen to maximise per-commit independence (no cross-commit
dependencies).  XS items first as warmups; the M items
(native codegen quirks, two-pass parser arc) come later
when the small wins have warmed the muscle memory.

| # | Sub-step | Files | Test |
|---|---|---|---|
| 10.1 | `\0` / `\xNN` / `\u{NNNN}` escapes in lexer (@P280) | `src/lexer.rs::escape_seq` | tests/lexer.rs unit tests for each escape; round-trip a `'\0'` literal through interp + native |
| 10.2 | Compiler warnings → stderr under `--native` (@P282) | `src/main.rs` (or wherever native run wires its output) | tests/error_messages — assert `2>/dev/null` strips warnings cleanly |
| 10.3 | `args() -> vector<text>` builtin | `default/02_images.loft` (alongside `env_variable`); host bridge in `src/native.rs` | tests/scripts/<NN>-args.loft round-trips a known argv |
| 10.4 | `text.split(text)` overload | `default/03_text.loft`; runtime in `src/state/text.rs` | tests/scripts smoke + edge: empty separator, separator-at-end, no-match |
| 10.5 | `text.starts_with_at(pos, prefix)` method | `default/03_text.loft`; runtime | unit + smoke; verify against `s[pos..pos+len(prefix)] == prefix` equivalence |
| ~~10.6~~ | `hash.contains(key)` method | **Deferred — XS estimate wrong** | The XS pitch assumed a simple `pub fn contains(both: hash, key)` could ship as sugar.  Reality: loft hash keys are typed per-instance (`hash<T[K]>`) and a generic `contains()` method needs parser-level typed-dispatch support — that's M+ work, not XS.  The existing idiom `h[key] != null` (or its truthy form `if h[key] { … }`) already works cleanly across all key types and is the established pattern in `tests/scripts/32-collections-regressions.loft`.  The deeper "set of text without wrapper struct" pain (the original scan.loft motivation) is a bigger language feature that deserves its own design pass.  Skipping 10.6; the STDLIB.md row reverts from "Open work" to "wontfix unless a future consumer demands the sugar." |
| 10.7 | `text::escape_html(s)` method | `default/03_text.loft` | unit test against `&` `<` `>` `"` `'` |
| 10.8 | `vector.sort()` + `vector.sort_by(fn)` | `default/01_code.loft`; runtime | tests/scripts: sort integers, sort texts, sort_by length; native + interp parity |
| 10.9 | stdlib `path` module | `default/<NN>_path.loft` (new file, loaded after `02_images.loft`); runtime helpers in `src/state/io.rs` | tests/scripts: dir/basename/join/resolve edge cases |
| ~~10.10~~ | `if X.method(local)` mis-parses with self-slice (@P278) — **deferred 2026-05-18, @P278 unreproducible; new @P283 found in same neighbourhood** | Investigation: tried the viewer's `problem_row_summary` workaround pattern + several variants.  The original @P278 parse error ("Expect token {") doesn't reproduce anymore — likely transient or shape-specific to a mid-edit state.  HOWEVER, a different runtime bug fires reliably in the same neighbourhood: format-string interpolation of a self-slice-reassigned text PARAMETER crashes both backends.  Filed as **@P283** in PROBLEMS.md with full reproducer at `/tmp/p_followups/p283_format_string_self_slice_param.loft`.  Likely fix sites: `src/state/text.rs` (interp OpAppendStackText for sliced text-params) and `src/generation/text.rs` (native `+=` shape on `&mut String` work buffers).  Workaround (viewer's `problem_row_summary` helper) covers BOTH @P278 and @P283 — extract slicing into a helper that takes the param by value. |
| ~~10.11~~ | Conditional-reassignment `unknown(0)` typer (@P279) — **deferred 2026-05-18, S→M+** | Investigation showed the bug fires reliably in scan.loft's `scan_link_line` context (strip the `a_esc: text =` annotation → "Cannot assign unknown(0) to field LinkRef.anchor of type text") but does NOT reproduce in any minimal isolated example I tried (struct + while + conditional reassignment + esc fn — runs cleanly).  The trigger is some interaction across the outer `while i+5 < n` loop + multiple `if ... { continue; }` early-exits + the `target_raw` parallel reassignment + the struct-field `not null` constraint.  Isolating + fixing needs a focused parser/typer investigation session; the workaround (explicit `a_esc: text =` annotation in scan.loft) is correct and self-explaining.  Re-open when a 2nd consumer hits the same shape, OR when the parser pass that lands 10.15 (two-pass forward-resolution) touches the same code paths. |
| ~~10.12~~ | Const vector native codegen crash (@P275) — **shipped 2026-05-18**.  Two-bug fix per [10a § @P275](10a-remaining-bugs-design.md#p275--const-vector-crashes-native-broader-than-text): (1) `output_native` (default `--native` path) was missing the `emit_const_vectors` call that `output_native_reachable` (`--native-release`) had — added the call so `db.const_refs` is populated before `n_main`; (2) the substitution `s.const_refs` → `stores.const_refs` accumulated `stor` prefixes when `OpConstRef` nested inside another opcode template (substring-of-its-own-output bug) — switched to method accessor + `_runtime` suffix (`s.const_ref_at(` → `stores.const_ref_at_runtime(`), same trick used for `s.raise(` → `stores.raise_runtime(`.  Pinned by `tests/scripts/109-const-vector.loft`.  Discovered separately during testing: @P284 — `for f in vector<float>` doesn't terminate (interp + native both, pre-existing) — filed. |
| ~~10.13~~ | `s[i] ?? '<char>'` chain-compare type mismatch (@P276) — **shipped 2026-05-18**.  Root cause: pre-eval lifted the inner `??` block into a `let _pre_N = { … };` whose body's last expression yielded `i32` (because the inner `_ncc_N: i32` var holds the character that way per `Variable` `rust_type` context).  The outer `OpConvIntFromCharacter` template then compared `_v_v1: i32` against `char::from(0)` → rustc E0308.  Fix: extended `src/generation/calls.rs::substitute_template_body`'s character-arg wrap path (Var/TupleGet/Call already covered) to also cover `Value::Block` whose `result == Type::Character`, wrapping with `ops::to_char(…)`.  Pinned by `tests/scripts/110-null-coalesce-char.loft`. |
| ~~10.10b~~ | @P283 — format-string + self-slice-reassigned text param crashes both backends — **shipped 2026-05-18**.  Root cause was simpler AND more general than the design hypothesised — NOT specifically the slice / self-aliasing.  The work-text buffer used by `assign_text`'s self-reference path (`src/parser/operators.rs:106-113`) gets promoted to a `RefVar(Text)` hidden arg by `text_return` in any text-returning function, but the codegen for the `OpAppendText` / `OpClearText` / `OpFormat{Text,Int,Float,Single,Database}` / `OpAppendCharacter` ops it emits did NOT dispatch to the matching `Stack` variants when the destination was `RefVar(Text)`.  Interp's `op_append_text` (op=116) called `string_mut` on what was actually a 12-byte refvar DbRef slot → SIGSEGV.  Native's `append_text` emitted `var += &*(…)` on `&mut String` → rustc E0368.  Fix mirrored the existing B7 OpAppendCharacter→OpAppendStackCharacter dispatch to cover the full op cluster: a `stack_variant` table in `src/state/codegen.rs::generate_call` (bytecode emit — interp) AND `src/generation/dispatch.rs::output_call_inner` (native emit), checking `parameters.first() == Var(v)` with `Type::RefVar(Type::Text)` and rewriting to the Stack variant.  Plus a `refvar_text_clone` arm in `output_set` for the `OpSet(local_String, Var(refvar_text_arg))` shape — `output_code_inner` for RefVar(Text) Var emits `&*var_X`, then the `.to_string()` wrap parses as `&*(var_X.to_string())` per Rust precedence → `&str` not `String` → E0308.  Emit `var_X.to_string()` directly instead.  Pinned by `tests/scripts/111-format-string-self-slice.loft`.  Actual effort: ~2 h (vs M 4-8 h estimate). |
| ~~10.14~~ | Local `sorted<T[K]>` re-types to vector on `+= [T{}]` (@P277) — **deferred to follow-up plan** | No minimal reproducer found; touches typer reassignment paths.  Workaround (struct-wrapper) in scan.loft works.  See [10a-remaining-bugs-design.md § @P277](10a-remaining-bugs-design.md#p277--local-sorted-re-types-to-vector). | Moves to `plans/future/<NN>-parser-typer-cleanup/`. |
| ~~10.15~~ | Two-pass forward-resolution of fn return types (@P281) — **deferred to follow-up plan** | Architectural change touching pass-1 symbol table; needs design pass.  Workaround (extract leaf helpers to top of file) works.  See [10a-remaining-bugs-design.md § @P281](10a-remaining-bugs-design.md#p281--two-pass-forward-fn-return-resolution). | Same target plan as 10.14. |
| ~~10.16~~ | Workaround removal pass — **shipped in two waves**.  Wave 1 (commit `e31bab27`, 2026-05-15): scan.loft hand-rolled `basename_leaf` / `basename_of` / `dir_of` / `resolve_link_path` deleted (use stdlib `text.basename()` / `dir()` / `resolve()` from 10.9); −63 +14 lines.  Wave 2 (2026-05-18, after @P283 closed): attempted to inline viewer's `problem_row_summary` back into `render_problems_row` since the @P283 codegen crash is gone — but the inline form re-triggers @P278 (parser rejects `.method()` chain after self-slice-style local reassignment in the same scope), which is still open.  Helper kept; its comment updated to attribute its existence to @P278 alone (no longer also @P283).  @P278's PROBLEMS.md row updated with the now-confirmed minimal reproducer (the inlining attempt).  scan.loft's @P281 workaround (forward-resolution leaf-helper hoisting) also stays — @P281 still open.  No further workarounds remain to remove. |

Sub-step 10.16 is the cleanup that proves the harvest worked
— removes the in-code workaround comments now that the
language has caught up.

### Sub-steps for newly discovered issues during this phase

Per [CLAUDE.md § Development cadence](../../../../CLAUDE.md#development-cadence--the-dogfood-loop)
and [DEVELOPMENT.md § Inserting Discovered Enhancements](../../DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan):

> **Always add the FIX SCHEDULING for newly-found issues to
> the current plan's sub-step list before picking up the
> next phase.**

Two-part discipline:

  1. **Design / details / reproducer**: live in the
     issue's canonical home (P-issue in PROBLEMS.md, row
     in STDLIB.md `## Open work`, slot in lib_plans/).
     That's where readers go to understand the bug.

  2. **Plan to fix**: a sub-step row in this table —
     `<step #>` + `<one-line summary referencing the
     canonical home>` + `<files to touch>` + `<test name>`.
     That's where readers go to see "is anyone going to
     actually fix this?"

The sub-step row doesn't duplicate the design — it points
at the canonical home (`@P281`, `STDLIB.md § Open work`)
and commits this plan to landing the fix.

Pattern: when a sub-step (10.N) surfaces a sibling issue —
a related codegen quirk uncovered by the test, an additional
stdlib gap noticed while writing the regression file, a
parser quirk that the new lexer escape exposes — file the
issue in its canonical home (P-issue / `## Open work`
row / lib_plans slot per DEVELOPMENT.md routing), THEN
append a sub-step row 10.<N+1> here that schedules the
fix.  The table grows in flight.

Reasoning: when a workaround site is fresh in working
memory, the cheapest moment to schedule a related fix is
right now, not "later from a list of P-issues that may
languish."  The sub-step row is the COMMITMENT that the
canonical-home entry will actually close.

Examples of what would qualify:

  - 10.12 (const vector native crash) reveals a sibling
    crash for `const hash<T[K]>` → file as @P28X in
    PROBLEMS.md, schedule as 10.12b here.
  - 10.4 (`text.split(text)`) surfaces that
    `text.split(text, max_count)` would have prevented a
    different workaround → add row to STDLIB.md `## Open
    work`, schedule as 10.4b here.
  - 10.15 (forward fn resolution) uncovers a similar
    forward-struct-resolution issue → file as @P28X,
    schedule as 10.15b.

Items too big to inline as a sub-step (L effort, full
design pass needed) get a `lib_plans/future/` slot
created in the canonical-home routing AND get a row in
this table that says "track via `[lib_plans/future/<NN>/](path)`
— close this sub-step when that plan ships its first
phase".  The schedule ALWAYS lives in the active plan;
the design lives wherever it makes sense.

## Acceptance

- 10.1-10.9 each ship with a passing test.
- 10.10 removes every `// loft gap: ...` comment from
  `tools/indexer/src/scan.loft` and `tools/viewer/src/main.loft`
  whose referenced gap is now fixed (those comments cite the
  P-issue / STDLIB.md row that this phase closes).
- @P280 + @P282 close in PROBLEMS.md.
- 5 stdlib gaps + path module remove their rows from
  STDLIB.md `## Open work`.
- CHANGELOG.md 0.8.5 entry's "Smaller language wins" section
  gains the harvested items as bullet points.
- Both backends (interp + native) produce identical output
  for every new method / builtin.

## Risks

| Risk | Mitigation |
|---|---|
| Sub-step 10.2 (warnings to stderr) breaks downstream consumers that grep loft's stdout for warnings | grep `tools/` for any warning-parsing pattern; update or bridge.  scan.loft's existing `2>/dev/null \| grep -vE '^warning'` becomes a no-op (still works, just doesn't need to filter anything) |
| `vector.sort()` (10.8) needs comparable-T trait awareness | Restrict to types with built-in ordering first; `sort_by(fn)` covers user types.  No T-bounded surface in this commit |
| `text.split(text)` (10.4) zero-length separator semantics could surprise (does `"abc".split("")` return `["a","b","c"]` or error?) | Match Rust's `str::split` semantics; document the answer |
| Phase grows beyond M effort | Drop sub-steps to a follow-up commit if the phase gets stuck.  10.1-10.7 are XS each; 10.8-10.9 are S each.  Hard ceiling: anything past 3 days of focused work splits |

## Why bundle these vs ship individually

Each item is small enough to ship alone.  Bundling matters because:

  - **Release narrative**: 0.8.5's "Smaller language wins"
    section gets one cluster of bullets instead of a series
    of point releases.  The release narrative reads as a
    coherent harvest from the dogfood pass.
  - **Workaround-removal pass (10.10)** only makes sense
    once ALL the gaps it cites are closed.  Bundling means
    one cleanup commit instead of N intermediate states
    where some workarounds remain.
  - **Test infrastructure reuse**: each sub-step adds one
    or two `tests/scripts/<NN>-<feature>.loft` files; co-
    located commits share the test harness setup.

## Cross-references

- [CLAUDE.md § Development cadence](../../../../CLAUDE.md#development-cadence--the-dogfood-loop)
  — the project model that makes this phase the natural
  bookend of the dogfood cycle.
- [DEVELOPMENT.md § Inserting Discovered Enhancements](../../DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)
  — the workflow rule that routed individual items to
  P-issues + STDLIB.md `## Open work` instead of a parallel
  catalog; this phase implements that catalog's XS/S items.
- [Phase 07 — loft-native scanner](07-loft-native-scanner.md)
  — the dogfood exercise that surfaced these.  Its "Loft
  gaps surfaced" section is the seed.
- [PROBLEMS.md @P280 + @P282](../../PROBLEMS.md) — the two
  P-issues this phase closes.
- [STDLIB.md `## Open work`](../../STDLIB.md#open-work) —
  the stdlib gap rows this phase implements.
- [CHANGELOG.md 0.8.5 draft](../../../../CHANGELOG.md) —
  where the harvested items land in the release narrative.
