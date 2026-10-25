<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 10a — Remaining-bugs design (post-mid-pass review)

**Status:** Closed 2026-05-18.  All three M-shaped items the
design survey identified (@P275 / @P276 / @P283) shipped on the
same day after their fix sites turned out to be cheaper than the
estimates (actual ~6 h combined vs the 11-22 h survey budget).
Three findings during shipping:
  - **@P275's** real bug was a missing `emit_const_vectors` call
    in `output_native` (the design's "extract_literal_values
    recognizer" theory was wrong); side-fix in
    `src/generation/calls.rs` substring substitution.
  - **@P276's** wrap path already existed for Var/TupleGet/Call
    arg shapes; just needed the `Value::Block` case added.
  - **@P283's** root cause was simpler AND more general — NOT
    self-aliasing-specific.  ANY text-returning fn whose body
    used `assign_text`'s work-text path on a RefVar(Text)
    buffer hit it.  Fix mirrors the existing B7
    OpAppendCharacter→OpAppendStackCharacter dispatch to the
    full op cluster (`refvar_text_stack_variant` in
    `src/generation/ops/mod.rs`).

Phase 10.16 follow-up confirmed @P278 (parser bug, deferred to
parser-typer cleanup plan) is the lingering reason
`tools/viewer/src/main.loft::problem_row_summary` stays
extracted — the closed @P283 was a sibling crash, not the
underlying parser issue.

## Why this doc exists

Phase 10 shipped 9 sub-steps quickly (10.1-10.5, 10.7-10.9,
10.16) and deferred 3 (10.6, 10.10, 10.11) when their estimates
proved wrong on first contact.  The 4 remaining items
(10.12-10.15) are all M-shaped per the original phase doc —
each could easily run into the same defer-on-investigation
trap.  This design doc fixes that by SURVEYING the actual fix
sites BEFORE planning, so the implementation budget reflects
reality, and bugs that share a fix site get bundled.

## Summary table — what's left

| Bug | Reproducer status | Fix site (read) | Cluster | Revised estimate | Recommended action |
|---|---|---|---|---|---|
| ~~@P275~~ | **CLOSED 2026-05-18** | `src/generation/mod.rs::output_native` (missing `emit_const_vectors` call) + substitution rename in `src/generation/calls.rs` | const-emit | actual: ~3 h | **Shipped.**  Root cause was DIFFERENT from the initial hypothesis — the gate (`def.const_ref.is_some()`) WAS firing correctly; the real bug was that `output_native` (default `--native` path) simply didn't call `emit_const_vectors` at all.  Only `output_native_reachable` (used by `--ship`) did.  Bonus: a substring-of-its-own-output bug in `src/generation/calls.rs` (`s.const_refs` → `stores.const_refs` accumulating `stor` prefixes when nested) surfaced and was fixed via the proven `_runtime` suffix trick.  See 10.12 row in parent plan. |
| ~~@P276~~ | **CLOSED 2026-05-18** | `src/generation/calls.rs::substitute_template_body` — added a `Value::Block(b)` arm with `b.result == Type::Character` to the existing character-arg wrap path | char-typing | actual: ~1 h | **Shipped.**  Root cause was simpler than the design hypothesised — same wrap pattern already existed for Var/TupleGet/Call; just needed the Block case added.  No pre-eval changes needed. |
| @P277 | NOT yet repro'd minimally — only triggers in scan.loft's specific layout | `src/parser/operators.rs` (or wherever `+= [literal]` re-types LHS) | typer | M+ (likely 1-2 days) | **Defer** — cost dominated by reproducer hunt |
| @P281 | NOT yet repro'd minimally — workaround was added preventatively | `src/parser/mod.rs` pass-1 fn-return-type table | typer | M+ (architectural — touches pass-1) | **Defer** — needs design pass of its own |
| ~~@P283~~ | **CLOSED 2026-05-18** | `src/state/codegen.rs::generate_call` + `src/generation/dispatch.rs::output_call_inner` (Stack-variant op dispatch for RefVar(Text) targets) + `output_set::refvar_text_clone` (avoid `&*var.to_string()` precedence trap on RefVar(Text) Var reads) | text-handling | actual: ~2 h | **Shipped.**  Root cause was simpler AND more general than this design hypothesised — NOT specifically the slice / self-aliasing.  ANY text-returning function whose body emits `OpAppendText` / `OpClearText` / `OpFormat*` / `OpAppendCharacter` on a `RefVar(Text)` work-buffer arg (which `text_return` always promotes) hit the bug.  The fix mirrors the existing B7 `OpAppendCharacter`→`OpAppendStackCharacter` dispatch to the full op cluster on BOTH backends.  Side-finding: the design's "interp lifetime" theory was wrong — the SIGSEGV wasn't from a dangling slice but from `string_mut` (`src/state/text.rs:319`) treating a 12-byte DbRef refvar slot as a `String`. |

**Net plan:** ship @P275 + @P276 + @P283 (the three with confirmed reproducers and identified fix sites) as the remainder of phase 10.  Defer @P277 + @P281 to a follow-up plan focused on parser/typer architecture.

## Per-bug investigation findings

### @P275 — Const vector crashes native (broader than text)

**Status of original story.** The original PROBLEMS.md row
named "Module-scope `const vector<text>`" — investigation
shows ANY const vector crashes native, not just text.  The
text-specific framing was a red herring; the underlying gap
is that `emit_const_vectors` short-circuits for ALL const
vectors in my test runs.

**Evidence.**

```loft
const NUMS: vector<integer> = [10, 20, 30];
fn main() { for n in NUMS { println("{n}"); } }
```

  - Interp: `10\n20\n30\n` ✓
  - Native: `index out of bounds: the len is 0 but the
    index is 572` at `stores.const_refs[(572_i64) as usize]`.

**Where the bug lives.**

```rust
// src/generation/mod.rs:1158
fn emit_const_vectors(&self, w: &mut dyn Write, till: u32) {
    let have_any = (0..till).any(|d| {
        self.data.def(d).def_type == DefType::Constant
            && self.data.def(d).const_ref.is_some()  // ← gate
    });
    if !have_any { return Ok(()); }
    ...
}
```

The `def.const_ref` field is only set by
`compile::build_const_vectors` (src/compile.rs:130).  The
native-emit pipeline at `src/main.rs:2149` runs AFTER
`compile::byte_code` (line 1826) which DOES call
`build_const_vectors` — so `const_ref` should be `Some`.

**But it isn't.**  The likely cause: `extract_literal_values`
(src/compile.rs:143) doesn't recognise the IR shape that the
current parser emits for `const NAME = [...]`.  It looks for
`OpSetInt` / `OpSetFloat` / `OpSetSingle` / `OpSetText` calls;
the actual IR may use `OpAppendVector` or another shape.

**Recommended fix.**

  1. **Diagnose the IR shape.** Dump `data.def(d_nr).code`
     for the const NUMS via a temporary println in
     `build_const_vectors`.  See what ops are actually present.
  2. **Update the recognizer** to match.  Likely 5-10 lines
     in `extract_literal_values` to handle the
     OpAppendVector / OpNewRecord / OpFinishRecord triplet.
  3. **Test:** `tests/scripts/<NN>-const-vector.loft` —
     `const NUMS = [10, 20, 30]; const TAGS = ["a","b","c"];
     for v in NUMS { ... }; for t in TAGS { ... }` — run
     under both backends, assert byte-identical output.

**Effort:** M (3-6 h).  The recognizer + test is small; the
investigation step (dump IR, identify shape) is the bulk.

### @P276 — `s[i] ?? '<char>'` chain-compare type mismatch

**Status of original story.** Confirmed.  The reproducer:

```loft
fn main() {
    s = "abc";
    i = 0;
    if (s[i] ?? ' ') == 'a' { println("ok"); }
}
```

  - Interp: works
  - Native: rustc E0308 — `_v_v1 == char::from(0)` mixes `i32`
    and `char`.

**Where the bug lives.** The `??` operator's emit path for
a character-typed slice receiver.  Likely in
`src/generation/calls.rs` (Op handler) or
`src/generation/text.rs`.

**Pre-fix needed.** Locate the exact emit site.  The
generated Rust for `s[i] ?? '\0'` is approximately:

```rust
let _v_v1 = OpTextCharacterNullable(s, i);  // returns i32
if _v_v1 == char::from(0) { ... }            // char on RHS, i32 on LHS
```

The fix is to make ONE side cast match the other.  Either:
  - `_v_v1 == char::from(0) as i32` (cast char to i32), or
  - `(char::from_u32(_v_v1 as u32).unwrap_or(...)) == char::from(0)`
    (cast i32 to char)

The existing code partly does the first form but not
consistently — the @P276 row in PROBLEMS.md notes that the
emit produces it in some places (`as i32`) but not others.

**Recommended fix.**

  1. Grep `src/generation/` for `char::from(0)` to find every
     emit site.
  2. Audit each — every `_v_v1 == char::from(0)` needs the
     `as i32` cast on the RHS to match the LHS i32.
  3. **Test:** `tests/scripts/<NN>-null-coalesce-char.loft` —
     `if (s[i] ?? ' ') == 'a' { ... }` in various positions
     (assignment, condition, format-string interpolation).

**Effort:** M (4-8 h).  Cluster simplifies if the same emit
site also affects @P283 (text handling).

### @P277 — Local sorted re-types to vector

**Status of original story.** No minimal reproducer.  The
viewer / scan.loft pattern that triggers it is:

```loft
distinct: sorted<TagSlot[name]> = [];
distinct += [TagSlot{name: "x"}];   // ← re-types to vector<TagSlot>
```

But I couldn't isolate this in 5 min when investigating the
neighbouring @P278.  The workaround (wrap in a struct field
— `struct Sets { tags: sorted<...> }`) is in scan.loft and
works.

**Where the bug lives (suspected).** `src/parser/operators.rs`
or `src/parser/expressions.rs` — wherever `+=` literal-vector
append rewrites the LHS type.  The `[T{...}]` literal is
typed as `vector<T>` and the `+=` re-types the LHS.

**Why I'm deferring.** Two reasons:

  1. Without a minimal repro, I can't validate a fix worked.
  2. The fix is in the typer's reassignment path — touching
     it risks breaking other working patterns.  Needs a
     proper design pass that surveys all `+=` sites.

**Recommended action:** **Defer to a follow-up parser plan.**
The workaround is correct.  Re-open when the typer arc lands
naturally (whoever lands @P281's two-pass forward-resolution
will be in this code anyway).

**Effort if forced:** M+ (1-2 days) — most of the cost is
finding a minimal repro and validating the fix doesn't
regress unrelated patterns.

### @P281 — Two-pass forward fn return resolution

**Status of original story.** No minimal reproducer.  The
workaround in scan.loft was applied PREVENTATIVELY when I
moved leaf helpers to the top of the file to dodge "Expect
token ;" errors during validator development — but the
underlying loft bug has never been isolated to a minimal
repro that triggers without the rest of the scanner.

**Where the bug lives (suspected).** `src/parser/mod.rs` pass-1
— the symbol table that records function return types.  Pass
1 doesn't propagate return types to callers, so a caller
defined BEFORE the callee in the same file may type-check
against `unknown` in pass 1, producing strange parse errors
in pass 2.

**Why I'm deferring.**  This is an ARCHITECTURAL fix — the
two-pass parser's pass-1 symbol-table design.  Touching it
without a comprehensive understanding of pass-1 invariants
risks breaking many things.

**Recommended action:** **Defer to a focused parser plan.**
Open `plans/future/<NN>-parser-two-pass-fn-resolution/` with
its own design doc.  Phase 10 ships without 10.15.

**Effort if forced:** L (multi-day) — touches pass-1
architecture; needs design + extensive regression testing
across the existing 633 issues tests.

### @P283 — Format-string + self-slice param crashes both backends

**Status of original story.** Filed today as a sibling
discovery during 10.10's investigation.  Confirmed minimal
reproducer:

```loft
fn render(rb: text, id: text) -> text {
    full_pref = "**@P" + id + "** — ";
    plen = full_pref.len();
    olen = rb.len();
    if rb.starts_with(full_pref) {
        rb = rb[plen..olen];
    }
    "[{id}] {rb}"   // ← format-string interpolation
}
```

  - Interp: SIGSEGV at op=116 (mid-OpAppendStackText)
  - Native: rustc E0368 — `var___work_2 += &*(...)` where
    `var___work_2` is `&mut String` (needs deref before `+=`)

**Where the bug lives.** Two sites:

  1. `src/state/text.rs::OpAppendStackText` — for the interp
     SIGSEGV.  Likely the slice `rb[plen..olen]` of a text
     parameter creates a stack-Str that aliases freed
     memory once the param's storage is reused.
  2. `src/generation/text.rs::append_text` — for the native
     E0368.  When the work buffer is `&mut String` (passed
     by reference for text-return functions) and the source
     is a sliced text-param, the emitted `+=` doesn't deref.

**Pre-fix needed.**  Identify the exact emit shape that
produces `var___work_2 += &*(OpGetTextSub(...))` when
`var___work_2` is `&mut String`.  The `&*` deref pattern
should be wrapping the LHS, not the RHS — `(*var___work_2)
+= &*(...)`.

**Recommended fix.**

  1. **Native side:** in `src/generation/text.rs::append_text`,
     when the destination var is `RefVar(Text)` (i.e. `&mut
     String`), emit `(*var___work) += ...` instead of
     `var___work += ...`.  Likely a single check + emit-shape
     change.
  2. **Interp side:** in `src/state/text.rs::OpAppendStackText`,
     ensure the source slice's lifetime extends across the
     append.  May need to allocate a fresh String for the
     slice copy before the append (slower but correct).
  3. **Test:** `tests/scripts/<NN>-format-string-self-slice.loft`
     using the @P283 reproducer.  Both backends produce
     `[259] body` byte-identically.

**Effort:** M (4-8 h).  Native fix is likely 5-10 lines;
interp fix needs more care (lifetime management).  Test is
small.

## Cluster analysis

The five remaining bugs split into three clusters:

### Cluster A — const-emit (1 bug)

  - @P275

Self-contained.  Fix lives in `src/generation/mod.rs` +
`src/compile.rs`.  No interaction with other bugs.

### Cluster B — text/character handling (2 bugs)

  - @P276 (`??` chain-compare on character)
  - @P283 (format-string of sliced text-param)

Both touch `src/generation/text.rs` (or its neighbours in
`src/generation/calls.rs`).  Investigating one will load
the same file context for the other — schedule together.

### Cluster C — typer architecture (2 bugs, both deferred)

  - @P277 (sorted local re-types to vector)
  - @P281 (two-pass forward fn return)

Both touch `src/parser/` typer logic.  Both lack minimal
reproducers.  Both need design passes of their own.  Defer
both to a separate parser-typer plan.

## Recommended sequencing

Phase 10 closes with three more sub-steps in order of
investigation maturity:

| Sub-step | Bug | Cluster | Why this order |
|---|---|---|---|
| 10.12 | @P275 | A (const-emit) | Self-contained; smallest investigation surface |
| 10.13 | @P276 | B (text/char) | Loads `src/generation/` cluster context |
| 10.10b | @P283 | B (text/char) | Same cluster as 10.13; landed in same session if possible |

Then phase 10 closes.  The deferred items move to:

  - @P277 + @P281 → new plan `plans/future/<NN>-parser-typer-cleanup/`
  - @P278 (original parser parse-error, unreproducible)
    → stays as-is in PROBLEMS.md until someone hits it
    again with a clean repro
  - @P279 (typer unknown(0)) → same status — workaround in
    scan.loft is correct, re-open if a 2nd consumer hits

## Phase-10 closure conditions

Phase 10 closes when:

  - @P275, @P276, @P283 all closed in PROBLEMS.md
  - Their workarounds removed from `tools/indexer/src/scan.loft`
    (extension of 10.16)
  - 0.8.5 changelog's "Smaller language wins" section
    reflects the closures
  - This phase doc moves to "Status: closed" with the full
    9 shipped + 5 deferred sub-steps recorded

## What I'm NOT going to do anymore

  - **Investigate without a minimal reproducer.**  Every
    remaining bug needs a confirmed minimal repro before I
    write fix code.  No "let me see if this fixes it" loops.
  - **Treat M-shaped items as quick wins.**  Each M needs
    its own session: read the fix site, identify the emit /
    parse path, write the test, commit.  No bundling 4 M-
    items into one session.
  - **Defer items mid-investigation.**  Either ship the fix
    or make the deferral the FIRST commit (with the
    investigation captured).  No half-investigated state.

## Effort budget

  - 10.12 (@P275): 3-6 h
  - 10.13 (@P276): 4-8 h
  - 10.10b (@P283): 4-8 h
  - **Total: 11-22 h** of focused work, distributed across
    2-3 sessions.

If any of the three slips by more than 50% past its upper
estimate, defer it to the parser-typer plan and close phase 10
with the rest.  No grinding.

## Cross-references

- [Phase 10 plan README](10-language-harvest.md) — the
  parent plan; this design doc is referenced as 10a in its
  closeout.
- [PROBLEMS.md](../../PROBLEMS.md) — canonical home for
  @P275-@P283.
- [DEVELOPMENT.md § Schedule-to-fix lives in the active
  plan](../../DEVELOPMENT.md#schedule-to-fix-lives-in-the-active-plan)
  — the workflow rule that this design doc embodies.
