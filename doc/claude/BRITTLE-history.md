<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# BRITTLE history

Entries moved out of [BRITTLE.md](BRITTLE.md).

Closed by the rustc writer's retirement (@PLN225, `@C170`): `src/generation/` left the tree.

## 4. `src/generation/emit.rs` `Str::new(...)` wrap heuristic

**Files:** `src/generation/emit.rs::{Value::Return, output_block::wrap_result}`.

**What it does:** Native codegen represents loft `text` values as
`Str` (owned) or `&str` (borrowed).  Return expressions and block-tail
expressions have to decide whether to wrap the inner expression in
`Str::new(...)` to produce a `Str`.  The current heuristic excludes
user-defined text-returning functions (empty `rust`/`native` fields,
non-`Op` prefix) because those already return `Str`, but *does* wrap
`Op*`-prefixed template calls because those produce `&str`.

**Signal:**
- `expected &str, found Str` or `expected Str, found &str` compile
  errors in generated Rust, often with a long sequence of nested
  Str::new wrappers.
- Bounded-generic T-stubs that silently return STRING_NULL instead
  of their real text value.

**Risk:**
- The "this is an Op template" test is a prefix match on the function
  name.  Renaming an op from `OpGetTextSub` to `GetTextSub` would
  silently move it from the `&str` branch to the `Str` branch.
- `patch_hoisted_returns` now runs for `Type::Text` bodies only when
  the *function name* starts with `t_` (the T-stub convention).  Any
  non-t_-prefixed text function whose IR matches the
  `[Call, OpFreeText, Return(Null)]` pattern will silently drop its
  Call value.

**Mitigation:**
- Replace the "starts with Op" heuristic with an explicit
  `is_template` flag on `Definition`.
- Replace the "starts with t_" gate with a structural check — detect
  the `[expr, OpFreeText(work), Return(Null)]` shape and patch
  wherever it appears.  Or add the flag at definition time.

**Recent bugs:** P86n (native interfaces).  Required three separate
fixes to emit text correctly for bounded-generic interface methods;
each was scoped narrowly to avoid breaking six unrelated test files.
