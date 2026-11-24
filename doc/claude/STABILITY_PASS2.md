<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_PASS2.md — relocations: every routine moves to its data structure

> The DEFERRED rows below are ORDERED in
> [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) — the single tracking view
> across all stability docs (the privacy-pass rows fold into its #9, the
> codec row into #7).

Pass 2 of [STABILITY_METHOD.md](STABILITY_METHOD.md), started 2026-06-11 in
the post-release low-churn window.  Scope set by the user:

- **Detect routines that operate on foreign data structures and move them to
  the module that owns the structure.**  Ownership map: `Value`/`Type`/
  `Data`/`Definition` → `src/data.rs`; `Stores` → `src/database/`; `Store` →
  `src/store.rs`; `DbRef` → `src/keys.rs`; `Function` (variable table) →
  `src/variables/`; `State` → `src/state/`.
- **Remove duplicates where already possible.**
- **Tree-walking routines are reimplemented cleanly: ONE walker on the owner
  taking a closure** (`Value::for_each_child` / `any_node`), replacing the
  hand-rolled recursive descents.  A new `Value` variant then extends ONE
  exhaustive match and every traversal inherits the edge.
- **Making the structures internally private is NOT in scope yet** — moves
  first; the privacy pass comes after.

Per-move gates: full suite + both backends; behavior-affecting unifications
(coverage-drifted duplicates) get a note on which direction widened.

## Status legend

`☐ todo` · `▶ partial` · `✅ done` · `➖ nothing foreign found`

## The walker (the keystone)

`Value::for_each_child(&mut FnMut(&Value))` — the ONE place that knows
`Value`'s tree shape (exhaustive match, no wildcard) — plus
`Value::any_node` (pre-order boolean search, Span-transparent),
`Value::walk` (pre-order visitor for collectors), the mutable twins
`for_each_child_mut` / `map_nodes` (kept adjacent so the two matches
cannot drift), the shared predicates (`reads_var`, `base_var`), and the
`Type` twins (`Type::for_each_child` / `any_node` / `contains_def`).
Hand-rolled walkers convert to closures over these; each conversion notes
whether the unified coverage WIDENED the old walker's (drifted default arms
are the #313/#330 disease — widening is deliberate and fail-safe direction
is checked per consumer).

## Work list

| File | Foreign routines / duplicates detected (survey 2026-06-11) | Action | Status |
|---|---|---|---|
| `src/scopes.rs` | ~22 hand-rolled `&Value` walkers: `value_reads_var` (pub, 3 ext. consumers), `contains_free`, `contains_alloc`(+`_unconditional`), `is_top_free`, `is_var_null_init`, `returned_var`, `expr_ends_in_return`, `escapes_value`, `guard_escapes`, `confine_reassign_safe`, `holder_retained`, `has_free_before_alloc`, `recover_backer`, `assigns_local`, `store_dead_after_block`, `binding_source`, `walk_par_safe_value`, `walk_classified`, `walk_shallow_parent_write`, `reclaim_safe`; `needs_pre_init`/`is_value_return_type` (`&Type`) | `reads_var` → `Value::reads_var`; exists-anywhere walkers → `any_node`/`walk` closures (done); the rest STAY with documented reasons — positional (`expr_ends_in_return`, `returned_var`, `escapes_value`), dominance-state (`confine_reassign_safe`, `store_dead_after_block`, `contains_alloc_unconditional`), receiver-position exception (`holder_retained`), sequenced flow (`store_liveness_walk`), shape tests (`is_var_null_init`, `binding_source`, `free_op_var`-alikes), Type policy (`needs_pre_init`, `is_value_return_type`), orchestrators (`reclaim_safe`, `has_free_before_alloc` — #260 pass-3 deletion), mutator (`prepend_to_scope` — `map_children` family) | ✅ waves 1+4 |
| `src/parser/mod.rs` | `base_var_of`, `find_capturing_fn_ref` (`&Value`); `type_carries_closure` (pub), `type_contains_tv`, `is_*_vector_element_target` ×2, `type_element_size` (`&Type`); `is_addressable` | `base_var_of` → `Value::base_var` (w1); `type_contains_tv` → `Type::contains_def` (w3); `find_capturing_fn_ref` stays (shape-targeted, #313 layout); `type_carries_closure` stays (prunes at the pointer marker — a layout walk, not a pure descent); `type_element_size` (vector-element storage policy), `is_addressable` (accessor-chain shape) stay | ✅ waves 1–5 |
| `src/parser/control.rs` | `value_mentions_var` (dup of pre_eval's), `base_host_var` (dup of base_var_of), `tail_has_tuple_leaf`, `is_block_divergent`, `definitely_returns` (pub), `find_branch_terminal_var`, `tail_var`, `collect_hidden_ref_args`; `match_arm_types_unify` (`&Type`) | dups died into `Value::reads_var`/`Value::base_var`; tail/terminal walkers audited wave 4 — ALL positional (`tail_has_tuple_leaf`, `is_block_divergent`, `definitely_returns`, `find_branch_terminal_var`, `tail_var`, `collect_hidden_ref_args`), stay | ✅ waves 1+4 |
| `src/variables/validate.rs` | `slot_kind` (verbatim dup of slots_v2), `short_type` | dup dies — import `slots_v2::slot_kind` | ✅ wave 1 |
| `src/parser/collections.rs` | `is_capturing_fnref`, `worker_returns_capturing_closure` (`&Value`); `narrow_route_for` (`&Type`) | walkers STAY — tail/return-POSITIONAL semantics, not exists-anywhere (a tail combinator is a different keystone); `narrow_route_for` stays — narrow-int routing policy | ✅ waves 2+5: audited |
| `src/parser/expressions.rs` | `leaf_tuple_lhs`, `inline_ref_set_in` (`&Value`) | `inline_ref_set_in` → `any_node` closure (done); `leaf_tuple_lhs` stays — positional LHS-shape extractor | ✅ wave 2 |
| `src/parser/definitions.rs` | `type_contains_def` (`&Type`) | died into `Type::contains_def` | ✅ wave 3 |
| `src/parser/vectors.rs` | `cell_struct_name` (pub), `cell_value_type` (`&Type`) | STAY — cell-promotion policy (plan-22 closure cells); every caller is inside vectors.rs, the policy's home | ✅ wave 5: audited |
| `src/parser/objects.rs` | construction emission reaches `database.position` (fine — asks the home); `replace_record_ref` Value rewriter | mutable keystone added (`for_each_child_mut` + `map_nodes`); `replace_record_ref` is now a 5-line closure (in-place; preserves `Block.var_size` the old rebuild zeroed — fresh-parsed default exprs carry 0 anyway, suite-verified) | ✅ wave 5 |
| `src/parser/operators.rs`, `fields.rs`, `builtins.rs` | call_to_set GET→SET table (stays — op-layer fact); `code_references_var` (a FOURTH reads-var copy, drifted: missed If/Loop/CallRef/Iter), nested `contains_break`; `lit_int`/`lit_nonzero`/`is_struct_returning_call` shape tests | `code_references_var` died into `Value::reads_var` (wider = more protective work-text wrapping, the safe direction — the doc comment itself described the bug when it under-detected); `contains_break` → `any_node`; shape tests stay | ✅ wave 5 |
| `src/typedef.rs` | `has_value_cycle` (Data/attrs walker); heavy direct `Definition` field mutation (it IS the type-resolution pass — co-owner by design) | `has_value_cycle` → `Data::has_value_cycle`; field writes stay (resolution is its job) | ✅ wave 5 |
| `src/variables/mod.rs` | `size`/`align` (pub, `&Type`) — THE slot-size table; `work_refs*` build from Type | DECIDED: stay in `variables/` — `size(tp, Context)` is the variable table's slot model (`Context` is a variables concept); data.rs `byte_width` covers the STORE layout side; the two are different facts, not a dup | ✅ wave 5: decided |
| `src/state/codegen.rs` | `emit_typed_null`, `known_type`, `add_const` (`&Type`); `ir_contains_var` (debug assert walker); Definition `.code/.variables` writes (it IS the bytecode owner of those fields) | `ir_contains_var` → `any_node` (wave 2 — old version had no Span arm, assertion strengthened); Type helpers STAY — bytecode-layer constant/null mapping policy | ✅ waves 2+5 |
| `src/ir_store.rs` / `ir_schema.rs` / `ir_read.rs` / `ir_node.rs` / `data_store.rs` | the Value/Type CODECS (serialize the whole IR) | DEFERRED → own design slot: codecs encode per-variant FIELDS (not just child edges), so `for_each_child` alone can't drive them; F9's "derive codec from one schema" is the real fix and deserves its own plan | ➖ deferred (design) |
| `src/btree.rs`, `src/radix_tree.rs`, `src/vector.rs`, `src/hash.rs` | operate on `Store`/`DbRef` raw layouts — they ARE the collection algorithms over the store (co-owners by design, like fill.rs) | no move; privacy pass gives them a defined interface later | ➖ |
| `src/parallel.rs`, `src/extensions.rs`, `src/native.rs`, `src/wasm_gl.rs`, `src/data_store.rs` | 60+ direct `stores.allocations[]` touches | DEFERRED → the privacy pass IS this batch (accessor surface on `Stores`: worker-slot, swap-back, lock APIs; design with THREADING.md in hand) | ➖ deferred (privacy pass) |
| `src/state/io.rs`, `state/debug.rs`, `repl.rs` | direct `stores.types[].parts` reads | DEFERRED → same privacy-pass accessor batch | ➖ deferred (privacy pass) |
| `src/compile.rs`, `src/state/mod.rs` | minor Definition field reads | DEFERRED → privacy pass | ➖ deferred (privacy pass) |
| `src/main.rs`, `src/lexer.rs`, `src/keys.rs`, `src/store.rs`, `src/database/*` | own their structures | — | ➖ |

## Type walker note

Wave 3 added the `Type` keystone: `Type::for_each_child` (exhaustive over
RefVar/Vector/Rewritten/Iterator/Function/Tuple children; def-nr heads are
leaves) + `Type::any_node` + `Type::contains_def`.  Remaining `&Type`
classifiers (`type_carries_closure` — prunes at the pointer marker, so it
keeps a hand-rolled match even after moving; `has_value_cycle` — a `Data`
graph walk, not a type-tree walk) move in later waves.

## The `IrNode` keystone — and what one missed walker cost

`IrNode::for_each_child` (`src/ir_node.rs`) is the backing-agnostic twin of
`Value::for_each_child`: exhaustive over `ValueType`, no wildcard, so a new
variant forces a decision instead of landing in a per-walker default arm.
Every walker that must be TOTAL (reachability, liveness, "find every X") is a
closure over one of the two.

Note this is **not** the `ir_node.rs` deferral in the work list above.  That
row defers the *codecs* — they encode per-variant FIELDS, which child edges
alone cannot drive.  A child walk over the same handle is a separate, smaller
fact, and it was missing.

The reachability walkers (`collect_calls`, `collect_fn_ref_literals`, `collect_int_fn_refs`,
in `src/opt/names.rs`) are closures over the keystone: a callee reached only from a node kind
a hand-rolled whitelist forgot is otherwise pruned while its call site stays ([#815](https://github.com/loft-lang/loft/issues/815)).

Two lessons for the remaining waves:

- **A wave-5 "audited" row is only as wide as what the audit enumerated.**  A row
  that names only a file's `&Type` classifiers and concludes "stay" says nothing
  about the `&Value`/`IrNode` walkers in the same file.  Enumerate by SHAPE (every
  recursive descent over the IR), not by the names already in the row.
- **The extraction/recursion split is what makes a conversion safe.**  Each
  walker keeps its special cases as `match` arms that only EXTRACT (a worker
  fn-nr riding as an integer literal, a fn-ref `Set` target), then delegates
  recursion to the keystone.  Nothing about the tree's shape is restated.
  `collect_int_fn_refs` keeps exactly one deliberate exception — it does not
  descend into `Call` args, where the fn-ref is the call's RESULT and the
  arguments are ordinary integers that must not be read as def numbers.

### The third lesson — a keystone protects only the walkers that adopt it

A 2026-08-20 re-survey found the same missing variant set in a walker written
*after* the two lessons above were recorded.
`Parser::rewrite_generic_type_defaults` (`src/parser/mod.rs`) answers a generic
template's deferred `TV_*` markers once `T` is concrete, so it must be TOTAL — a
marker it does not reach ships to the monomorph as the placeholder. It descends ten
`Value` variants and ends `other => other`; `Tuple`, `TuplePut`, `Parallel`,
`ParFor`, `Iter`, `BreakWith` and `Yield` are all absent, which is nearly the #815
list again.

Two things follow for this doc:

- **The work list is a snapshot, not a standing guard.** Every row here was
  surveyed in June 2026. A walker added later is not in any row, so "all rows
  audited" says nothing about it. New total walkers need the same conversion at the
  time they are written.
- **A reachability guard cannot catch it.** It asserts a property of the REACHABLE set.
  A rewrite walk is a different traversal with the same failure mode, and no guard
  states the general property *"a walker that must be total delegates recursion to
  the keystone"*.

Detail and the remedy: [STABILITY_REDFLAGS § Cluster F](STABILITY_REDFLAGS.md);
ordered as row F in [STABILITY_ROADMAP](STABILITY_ROADMAP.md).

## Move log

The waves' log, and the rows of files that have left the tree:
[STABILITY_PASS2-history.md](STABILITY_PASS2-history.md).
