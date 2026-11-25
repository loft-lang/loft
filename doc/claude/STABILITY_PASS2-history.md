<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# STABILITY_PASS2 history

Records moved out of [STABILITY_PASS2.md](STABILITY_PASS2.md).

## Work-list rows of the rustc writer (its files left the tree with @PLN225, `@C170`)

| File | Foreign routines / duplicates detected (survey 2026-06-11) | Action | Status |
|---|---|---|---|
| `src/generation/pre_eval.rs` | `value_mentions_var` (dup), `free_op_var`, `needs_pre_eval`, `is_void_value` (pub), `create_stack_var`; `heap_shape_matches` (`&Type`) | dup died into `Value::reads_var`; rest audited wave 4 — `needs_pre_eval` stays (pre-eval policy with a deliberate descend set; converting would change generated-code shape without a bug), `free_op_var`/`is_void_value`/`create_stack_var` are shape tests, `heap_shape_matches` Type policy | ✅ waves 1+4 |
| `src/generation/coroutine.rs` | `contains_yield`, `detect_yield_from` (`&Value`); `persistent_default` (`&Type`) | `contains_yield` → `any_node` (wave 1); `detect_yield_from` positional, `persistent_default` policy — stay | ✅ waves 1+5 |
| `src/generation/mod.rs` | `narrow_int_cast`, `default_native_value` (pub), `is_collection_field` (`&Type`); direct `data.definitions[]`/`known_type` reads; **the three REACHABILITY walkers** `collect_calls` / `collect_fn_ref_literals` / `collect_int_fn_refs` | classifiers STAY — they encode the NATIVE layer's value-mapping policy over Type, not Type's structure (their Spacial omissions are the known N9 native-coverage enhancement, [NATIVE.md § N9](NATIVE.md)); field reads → privacy pass.  The three walkers were MISSED by the wave-5 audit (it listed only the Type classifiers) and are now closures over the keystone — see the note below | ✅ wave 5: audited; walkers converted (loft#815) |
| `src/generation/emit.rs` | nested local `walk` (ncc skip-free finder), `tail_is_return` (`&Value`) | `walk` → `any_node` closure (done); `tail_is_return` stays — positional | ✅ wave 2 |
| `src/generation/ops/parallel.rs` | `closure_shape`, `is_narrow_int_return`, `tuple_elem_read`, `is_by_value_scalar` (`&Type`) | STAY — parallel-marshalling policy over Type (native layer), not Type structure | ✅ wave 5: audited |
| `src/generation/dispatch.rs` | reads `stores.types[].name` directly | accessor DEFERRED → privacy pass; #260's declaration move EXECUTED (the `__vdb` declaration point now derives from the variable table — prologue in `generation/mod.rs::output_function` + `state/codegen.rs::def_code`; `dispatch.rs` gained only the `predeclared` first-Set handling) | ▶ #260 done; accessor deferred |

## The `IrNode` keystone — the loft#815 record

The cost of it missing: `src/generation/mod.rs` kept three hand-rolled
reachability descents (`collect_calls`, `collect_fn_ref_literals`,
`collect_int_fn_refs`), each a whitelist of node kinds ending in `_ => {}`.
None listed `Tuple`.  Native emits only the functions the reachable set names,
so a callee reached ONLY from a tuple element was pruned while its call site
was still emitted — rustc then failed `E0425: cannot find function` and the
library refused to build.  The registry library `hex_way` hit it on
`(0.0 - sin(a) * dir, cos(a) * dir)`, taking down every program in its
dependency cone (loft#815).  `Tuple`, `Parallel`, `BreakWith`, `TuplePut` and
`ParFor` were all absent from all three.

Guards: `tests/issues.rs::i815_callee_of_a_tuple_element_stays_reachable` and
`::i815_reachable_set_is_closed_under_calls` — the second asserts the general
property (the reachable set is closed under the call relation) with an
independent walker, so it catches the next omitted node kind rather than this
one.

These two guards left with the rustc writer's tests (@PLN225 R4); the walkers moved to
`src/opt/names.rs`.

## Move log

### Wave 1 — 2026-06-11 — the keystone + the known duplicates

- **Added** `Value::for_each_child` (exhaustive 34-variant match, no
  wildcard), `Value::any_node` (pre-order, Span-transparent),
  `Value::reads_var`, `Value::base_var` — `src/data.rs`.
- **Died:** `scopes::value_reads_var`, `parser/control::value_mentions_var`,
  `generation/pre_eval::value_mentions_var` → `Value::reads_var`.
  *Widened:* the unified predicate also matches `TuplePut`/`FnRef` work-var/
  `FnRefDnr`/`CallRef` callee/`Iter` var/`ParFor` x_var+r_var, and descends
  `Tuple`/`Parallel`/`BreakWith`/`Iter`/`ParFor` children the mentions
  copies skipped.  Checked fail-safe per consumer: scan_set #316 (wider →
  fewer pre-frees), codegen `rhs_reads_v` (wider → stash mode), objects
  #330 hoist gate (wider → more hoisting), control guard detection (wider →
  condition correctly recognised as a guard), pre_eval
  `target_used_between` (wider → fewer return-collapses).
- **Died:** `parser/mod::base_var_of`, `parser/control::base_host_var`
  (identical bodies) → `Value::base_var`.
- **Died:** `variables/validate::{SlotKind, slot_kind}` (verbatim dup) →
  imports `slots_v2::{SlotKind, slot_kind}`.
- **Converted to `any_node` closures:** `scopes::contains_free`,
  `scopes::contains_alloc`, `coroutine::contains_yield`.  *Widened:* all
  three now descend every edge (CallRef args, If conditions, Iter, ParFor);
  for free/alloc detection wider = more reclaim exclusion (safe); for yield
  detection full coverage is the correct semantics (a missed yield
  miscompiled).  **NOT converted:** `contains_alloc_unconditional` — its
  refusal to descend If/Loop/Parallel is dominance semantics, not drift.
- Gates: full suite 2289 passed / 173 skipped (baseline-equal), clippy
  clean, fmt applied.
- ~~Known benign: imaging fixture cdylib rebuild fails~~ — FIXED in the
  follow-up wave below.

### Wave 2 — 2026-06-11 — pure exists-anywhere walkers; positional walkers audited

- **Converted to `any_node` closures:** `expressions::inline_ref_set_in`
  (drift found: `BreakWith` was a leaf — a `Set` inside its value was
  missed; wider answer = correct null-init insertion point; depth-limit
  removed, deep-nesting test now also asserts the positive case),
  `state/codegen::ir_contains_var` (debug-only first-assignment
  self-reference assert; old walker had NO `Span` arm so Span-wrapped
  self-references escaped — assertion strengthened; release suite
  unaffected, debug smoke on closure-heavy scripts clean),
  `emit::block_contains_ncc_skip_free`'s nested `walk` (now also sees a
  `__ncc_` Set inside `Loop`/call args; wider → scratch-buffer
  materialisation in more cases, the safe direction).
- **Audited, deliberately NOT converted:** `collections::is_capturing_fnref`
  + `worker_returns_capturing_closure`, `expressions::leaf_tuple_lhs`,
  `emit::tail_is_return` — all POSITIONAL (tail/last/return-position)
  semantics, not exists-anywhere; `parser/mod::find_capturing_fn_ref` —
  shape-targeted (walks only the vectors.rs construction wrappers; full
  descent would match unrelated FnRefs and change #313 layout decisions).
- Gates: full suite 2288/2289 + `moros_glb_cli_end_to_end` transient
  cdylib-build race (passes in isolation after clearing `native-auto/`;
  the known concurrent-build truncation), debug smoke on closure-heavy
  scripts, clippy clean, fmt clean.

### Wave 3 — 2026-06-11 — the Type keystone + the contains-def twins

- **Added** `Type::for_each_child` / `Type::any_node` /
  `Type::contains_def` — `src/data.rs`.
- **Died:** `definitions::type_contains_def` + `parser/mod::type_contains_tv`
  (same predicate, drifted arms, both only ever called with type-variable
  numbers) → `Type::contains_def`.  *Widened:* descends Tuple / Function /
  Iterator / RefVar / Rewritten children — `substitute_type` already
  rewrites through Tuple (plan-17), so the GET-side predicate had drifted
  behind the SET side; also matches all def-carrying heads (Routine,
  Sorted/Index/Spacial/Hash) — harmless for the tv callers (def numbers are
  unique) and honest for future ones.
- Gates: full suite 2289 passed / 173 skipped, clippy clean, fmt clean.

### Wave 4 — 2026-06-11 — the scopes.rs cluster + the `walk` visitor keystone

- **Added** `Value::walk` (pre-order visitor, Span-transparent) — for
  collectors, the side-effecting sibling of `any_node`.
- **Converted:** `guard_escapes` (descent inherited; positional
  block-tail/return predicates stay in the closure), `recover_backer`,
  `assigns_local`, `walk_par_safe_value`, `walk_classified`,
  `walk_shallow_parent_write`, `collect_freed_vars` (debug leak check).
  *Widened:* the par-safety walkers (`walk_par_safe_value`,
  `walk_classified`, `walk_shallow_parent_write`) previously treated
  Return / Tuple / Iter / ParFor / TuplePut positions as safe-by-default
  leaves — a par-unsafe call hidden in a `return f()` value escaped the
  scan.  Wider scan = more conservative par classification (the safe
  direction).  `recover_backer`'s wider discovery stays downstream-gated by
  the reclaim soundness checks.
- **Audited, stay (reasons on the work-list rows):** the dominance-state,
  positional, shape-test, sequenced-flow, and policy walkers across
  scopes.rs / control.rs / pre_eval.rs.
- Gates: full suite 2289 passed / 173 skipped (including the formerly
  flaky `moros_glb_cli_end_to_end` — its cdylib build race got a real fix
  in the same push: cross-process build lock + atomic `.so` install in
  `native_lib.rs`), clippy clean, fmt clean.

### Wave 5 — 2026-06-11 — moves, the mutable keystone, the fourth reads-var copy

- **Added** `Value::for_each_child_mut` + `Value::map_nodes` (the mutable
  twins, kept adjacent to the immutable match so the two cannot drift).
- **Moved:** `typedef::has_value_cycle` → `Data::has_value_cycle` (a walk
  over `Data`'s definition graph lives with `Data`).
- **Converted:** `objects::replace_record_ref` (45-line consume-and-rebuild
  rewriter → 5-line `map_nodes` closure; in-place now preserves
  `Block.var_size` the old rebuild zeroed — fresh-parsed default
  expressions carry 0 anyway), `operators::contains_break` → `any_node`.
- **Died:** `operators::code_references_var` — the FOURTH reads-var copy
  (drifted: missed If / Loop / CallRef / Iter descent) → `Value::reads_var`.
  Its own doc comment described the data-loss bug that under-detection
  caused (`assign_text` clear-before-evaluate); wider detection = more
  protective work-text wrapping, the safe direction.
- **Audited, stay (reasons on the rows):** the native-layer Type
  classifiers (generation/mod, ops/parallel, state/codegen), cell-promotion
  policy (vectors.rs), `size`/`align` ownership DECIDED for variables/,
  coroutine + collections leftovers.
- **Deferred rows made explicit:** codecs (F9 design), `Stores` accessor
  batch + Definition field reads (the privacy pass — out of scope per
  user), dispatch.rs accessor (same; carries #260's declaration move).
- Gates: full suite 2289 passed / 173 skipped, clippy clean, fmt clean.

## Pass-2 relocation scope: COMPLETE (2026-06-11)

Every row is now ✅ done, ✅ audited-with-reason, or ➖ explicitly deferred
to its named successor (privacy pass / F9 codec design).  The keystone
family in `src/data.rs` is the single source of tree shape for both `Value`
and `Type`; four drifted copies of the reads-var predicate are one method;
every remaining hand-rolled walker carries a documented reason it is NOT a
plain exists-anywhere search.

### Follow-up wave — 2026-06-11 — the encountered-but-unfixed items, closed

- **Imaging fixture builds again.**  Root cause was double-layered:
  the fixture pinned `loft-ffi-build = "0.1"` (local crate is 0.2.0, so
  crates-io 0.1.0 won — missing `generate_register_from_loft_with_bridges`),
  and bumping to `"0.2"` STILL failed because the published 0.2.0 also
  lacks the function — **the local crate has drifted past its published
  version without a version bump**.  Fix: `[patch.crates-io]` path-patch to
  the in-repo `loft-ffi`/`loft-ffi-build`/`loft-ffi-macros` (a fixture
  should validate the current tree anyway).  ⚠ The next `loft-ffi-build`
  publish must bump to 0.2.1+ and lift the patch — noted in the fixture's
  Cargo.toml.
- **Spacial arms added** to `generation::is_collection_field` and
  `default_native_value`, aligning them with the `heap_dep`/`slot_kind`
  store-backed family.  Unreachable today (`spacial<T>` is parser-rejected,
  "planned for 1.1+") so this is landmine-removal, not a behavior change;
  the full native-coverage work stays under [NATIVE.md § N9](NATIVE.md).
- **Regression coverage for the widened walker shapes:**
  `loft-suite/attic/scripts/293-text-self-ref-wide-shapes.loft` pins text
  self-reference through If / Block / Loop RHS on both backends (the
  shapes `code_references_var` missed); `par_shallow_tests` gains
  `parent_write_in_return_value_detected` + `…_in_tuple_element_detected`
  (the safe-by-default leaves `walk_shallow_parent_write` had).
- Gates: full suite 2291 passed / 173 skipped (2 new unit tests), clippy
  clean, fmt clean, cdylib rebuild log clean of the imaging failure.

### Pass-3 wave 1 — 2026-06-11 — function-level de-duplication

- **Dominance twins unified.**  `confine_reassign_safe` and
  `store_dead_after_block` were ~80 identical lines apart ("mirrors …"
  by its own doc); both are now thin wrappers over ONE `dominance_walk`
  differing only in start value + `invalidate_conditional`.  Drift
  closed: the stronger gate had silently lost the `ParFor` arm its twin
  had (reads inside a parallel body escaped the over-free check; under
  invalidation a `ParFor` assignment now also invalidates — both
  conservative directions).
- **Par-safety pair unified.**  `walk_par_safe_value`+`call_is_par_safe`
  (5b recursive walk) and `walk_classified`+`call_classified`
  (fixed-point pass) differed ONLY in the unknown-user-fn policy — now
  `body_calls_par_safe` + `call_purity_safe` with the policy as a
  closure (visited-set recursion vs classification lookup).
- **`Value::tail` keystone** (Span-transparent, Block/Insert last
  non-`Line` operator) replaces four hand-rolled tail descents with
  drifted arm sets: `scopes::expr_ends_in_return` (missed Span + Line),
  `emit::tail_is_return` (missed `Insert` — scopes wraps tail returns in
  `Insert([frees…, Return])`, the exact shape `codegen::is_divergent`'s
  comment documents), `control::definitely_returns` (missed Span),
  `control::tail_has_tuple_leaf` (no Line skip).
- **Free-op recognizer unified** (pre_eval.rs had FOUR copies in one
  file, two stale): `is_free_op` + `freed_var` closures and the
  line-818 cleanup-skip list now all derive from `free_op_var` — the
  only copy that knew `OpFreeRefIfDistinct` (the #330 alias-witness
  free that `scopes::get_free_vars` emits into the IR).  Real holes
  closed: the @P274 use-after-free guard could miss a hoist past an
  if-distinct free of the expr's own operand, and the native tail-ret
  capture ABORTED (`Op*` fall-through → lost tail result shape) on
  meeting one in its cleanup window.
- **`probe_cur_dir_lib`/`probe_base_dir_lib`** (verbatim dup) →
  `probe_dir_lib`.
- Gates: full suite 2292 passed / 173 skipped, clippy 0 warnings, fmt
  clean.
