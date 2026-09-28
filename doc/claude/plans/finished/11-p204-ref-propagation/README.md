<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN11 — @P204: tail-expression return discarded

## Status — DONE 2026-05-02 (PR #197)

Closes **@P204** — refresh the unspan walker in
`src/generation/pre_eval.rs::detect_ref_tail_capture` so the
tail-call rewrite fires on Span-wrapped IR.  3-line fix.

PR-readiness gate: @P204's two failing native tests
(`85_yield_resume`, `87_store_leaks`) blocked PR-readiness; both
green post-fix.  P-issue closure record:
[PROBLEMS.md § 204](../../../PROBLEMS.md#204-tail-expression-return-of-inner-helper-call-discarded-closed-2026-05-02).

## What broke

When a struct-returning function ended with `return inner_call()`,
the parser injected a hidden `__ref_*` placeholder argument that
the inner call wrote its result into.  Native codegen's tail-
expression handling SKIPPED the `__ref_*` threading: it emitted
`n_inner(cell, args)` as a void statement followed by
`return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }` (the null
sentinel).  The caller's `OpCopyRecord(_src, var_q, ...)` panicked
on the null `_src` at `src/database/allocation.rs:347`.

## Why the existing infrastructure didn't fire

The infrastructure for tail-call capture was already in place at
`src/generation/pre_eval.rs:816-861`.  Designed for exactly this
pattern: when a Block result is heap-typed (Reference / Vector /
Enum-with-payload), the last non-Line operator is `Return(Null)`,
and walking backward through cleanup ops reaches a tail Call whose
return type matches the block's heap shape — capture the call's
result into `let __native_tail_ret: DbRef = call(...);` and rewrite
the `Return(Null)` to `return __native_tail_ret;`.

The walker's pattern-match looked like:

```rust
match &operators[i] {
    Value::Line(_) => {}
    Value::Call(d_nr, _) => { ... }
    _ => return None,
}
```

But `operators[i]` could be `Value::Span(box (pos, inner_value))`
— a position-tagging wrapper added by the parser.  The walker's
match arms didn't handle Span; it bailed on the `_` arm even
when the unspanned value WAS a Call.  The infrastructure compiled
but never executed in production.

Lesson for future code reviews of similar walker patterns: check
for `Value::Span` handling explicitly.  "Code that compiled but
never executed" is the failure mode.

## Fix shape (option A')

| Option considered | Outcome |
|---|---|
| A — extend `collect_hidden_ref_args` | Wrong call site — the bug was in `detect_ref_tail_capture`, a different walker |
| **A' — fix `detect_ref_tail_capture` walker** | **Chosen.**  Call `op.unspan()` at each match site.  3 lines. |
| B — new tail-position parse pass | Unnecessary — the existing walker IS the tail-position pass |
| C — codegen workaround | Unnecessary — walker fix routes through the existing emit-time capture path |

## Net delta

- **3 lines changed** in `src/generation/pre_eval.rs` (3 `.unspan()`
  calls added at match sites).
- **1 new regression test** —
  `p204_tail_expression_return_passes_under_native` in
  `tests/codegen_emitter.rs`.
- **2 @EXPECT_FAIL markers removed** from
  `tests/scripts/repro_p204.loft` and `tests/scripts/repro_p205.loft`
  (the latter unrelated to @P204 but a leftover from @PLAN09 phase
  07; un-marking is correct since @P205 closed too).

Total: ~5 lines added, 3 lines removed.

### Cost vs estimate

Plan estimate: 3-12 hours (the dominant uncertainty was the
parser-side architecture).  Actual: ~30 minutes including survey
+ investigation.  Why faster: the existing infrastructure was 90%
correct; only 3 lines needed to change.  The estimate-doubling
rule from @PLAN09's 05a Findings doesn't apply when the fix is at
a known-but-broken site (vs. new emitter / new infrastructure).

## See also

- [PROBLEMS.md § 204](../../../PROBLEMS.md#204-tail-expression-return-of-inner-helper-call-discarded-closed-2026-05-02)
  — symptom + reproducer + closure narrative
- [`../09-native-runtime-rewrite/`](../09-native-runtime-rewrite) —
  parent native-runtime arc; explicitly out-of-scope for @PLAN09
  because the fix is parser/IR-side, not codegen-emitter
- `src/generation/pre_eval.rs::detect_ref_tail_capture` — the
  walker; check for Span handling whenever extending
- `tests/scripts/repro_p204.loft` — minimal reproducer (now passes
  under both backends)
- `tests/codegen_emitter.rs::p204_tail_expression_return_passes_under_native`
  — regression pin
