# @PLN91 — the tester's backlog under the behaviour contract (state, contract, reasons)

**What changed:** the contract the port is held to has three layers beyond refusals and the IR
(1a-parser.md § The behaviour contract):
- each function's image STATE (variable flags, `LOFT_REPORT_STATE`);
- the facts a caller acts on (`[contract]`);
- the Rust rule behind each flag (`[varflag]` reasons).

So the backlog is cut from the differences in those layers, grouped by the Rust DECISION behind
them, and no longer only from refusals.  Measured on the porter's tip 7fe954324 (suite 0fe1a93,
tester try `try-7fe954324`, state file `state.tsv`): 1401 of 2132 agree, 0 differ; contract
1390 of 1401 alike; state 1310 of 1401 (568 items); reasons 1391 of 1401.  Group the items with
`kr-tools/state_groups.py STATE.TSV`.

## 0. First: 490 of the 568 state items are flags no Rust reader can see

For each flag, the Rust readers were read:

| class | items | programs | why it is inert |
|---|---|---|---|
| `user_named` on a name not starting with `_` | 417 | 60 | its ONE reader is `is_compiler_generated` (src/variables/mod.rs ~6220: `name.starts_with('_') && !user_named`) |
| `mutated_capture` entries | 42 | 17 | all 42 are compiler temps (`_ncr_1`, `__ncc_1`, `__ref_1`, `_elm_1` …).  The readers intersect the list with the lambda's real captures (vectors.rs ~3062, ~9803), or match it against the caller's parameter names (closures.rs ~457: at worst a conservative "written") |
| `deps` / `user_appended` on a lambda's capture placeholder | 29 | 9 | the pass-1 placeholder (§ Rust accidents) is never referenced by pass-2 code; `user_appended`'s one reader, the copy-write lint (use_analysis.rs ~255), also requires `copy_bound` |
| `user_appended` on `__closure` | 2 | 2 | the same lint: `__closure` is never `copy_bound` |

With these set aside, state agrees on **1366 of 1401** programs and **78 items** remain.  Almost
all of the 490 come from one Rust accident, the capture placeholder.  Porting it would copy an
accident whose flags nothing reads; leaving it means it dominates every state count.

**Recommendation:** compare a flag only where a reader can see it: `user_named` on `_`-names,
and `mutated_capture` on the lambda's real captures.  Compare a capture placeholder's deps and
`user_appended` not at all.  That is the contract's own rule ("does not grow unless a
mismatch shows a need"), applied in the other direction.  It is the porter's and owner's call,
because elab_gate.py is the porter's tool.  If accepted: a reader-scope filter in
`state_compare`, citing the readers above, and a positive control (a `_`-named capture written
in a lambda must still be compared).

## The subjects, by the Rust decision behind them (the 78 items left)

Each subject names the Rust setter to trace with `LOFT_TRACE_VARFLAGS=*` (its `file:line`).
"porter" = already on the porter's list (handoff 2026-11-23); "new" = prepared here.

| # | subject | items / progs | owner | the Rust decision |
|---|---|---|---|---|
| S1 | `user_named` on `__place_N`, port-only | 26 / 8 | porter (lead 1) | Rust marks a mention only where the variable IS the name written (objects.rs `parse_var_inner`); the port marks `record_compound`'s held place (el_ex_place ~1004).  READABLE: `__place_1` is a `_`-name, so `is_compiler_generated` answers false on the port |
| S2 | `inline_refs` on a keyed `??` holder (`__ncc_N`), port-only | 24 / 7 | porter | Rust marks it `skip_free` and decides its null init by its deps (`keyed_null_mints`); the port reads the inline mark for codegen, so the null-init decision must move onto the deps first |
| S3 | `skip_free` on a lambda's `__ref_N`, Rust-only | 11 / 7 | new, to measure | all in lambdas answering a record through a fn-ref (a-rebind-exchanges…, 1353, a-plain-bind-of-a-fn-ref-result…).  Porter's notes point at `RetPromotion::Bind { substitute }` unregistering the ref and `suppress_collapsed_workrefs`' second caller (control.rs ~5025, unported) |
| S4 | `deps` on a tuple element's `__ref_N`, Rust-only | 7 / 5 | porter (suspect) | `unbox_tuple_from_dbref`'s work ref (`deps=rs`, `deps=t`, `deps=v`): the work ref borrows the tuple it unboxes |
| S5 | contract `rebinds` | 10 progs | new, to measure | the parameters rebound with a store the body owns.  The contract lines are not dumped per function yet: needs a gate pass with the contract difference written out |
| S6 | contract `buffer_exits` | all 1401 not derived | porter (unported) | the placement analysis `callee_buffer_exits`.  The largest unported fact |
| S7 | singles | 9 / 8 | mixed | `copy_bound` d/c in a sizeof/type-name literal (a-struct-literal-inside-sizeof…); `caller_hidden_buf` `__ref_1` (a-non-escaping-vector-local…); `value_const` w (a-view-of-foreign-data…, porter); `skip_free` `__vdb_2`/`_vec_1` (a-default-of-a-method-chain…, `l1`, porter); `__vdb_1` (a-nullable-vector-local-at-the-tail…); `inline_refs` `_vec_1` (2044) / `_vec_4` (501) |
| S8 | reasons: `user_named` 13, `skip_free` 7 programs | — | new, to measure | the same flag set by a different rule.  Needs the `LOFT_ELAB_REASON_FILE` pass |

**Next, the tester:** one gate pass on the tip's tools that writes the reason file and the
contract differences (S5, S8).  Then S3 traced to its Rust setter and prepared as a page,
S5 and S8 grouped the same way.  The refusal-led backlog is exhausted: forward-typed-capture,
keyed-branch-arms, reversed-range-forms and store-else-on-an-append all have no site left at
7fe954324.  The next refusal-led subjects come from the tip's ranking: lambda families such as
"a lambda answering a record or a collection beside a shared capture" (35 sites) and "a lambda
that captures a local of its function: TyFunction" (28).
