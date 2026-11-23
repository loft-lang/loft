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
| S3 | `skip_free` on a `__ref_N`, Rust-only | 11 / 7 | MEASURED (11-23) | see § S3 below |
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

## S3, measured (11-23, on the join's Rust af1def05e)

The 11 items are three cases:
- **2471/2472's `pick`, `pick_b`, `pick2`, `pick3` (4):** the decision-order rule the porter is
  carrying (the shadow holds the copy seed's type during the body).  Rust's selector then takes
  `MaterializeView`, which mints `__ref_1` and retires it.
- **N-Road's `local_at` `__ref_2` (1):** on the porter's list.
- **Six lambdas in four programs** (1659 ×3, a-rebind-exchanges… `r6_0`, 1353 `main_2`,
  a-plain-bind-of-a-fn-ref… `_1`).  `LOFT_TRACE_VARFLAGS=*` in image mode
  (`LOFT_EMIT_PARSED`): set by `Function::unregister_work_ref` (src/variables/mod.rs ~4822).
  "Retire a work-ref the one-buffer binding substituted out of the IR: every use now names the
  return buffer, so the variable names no storage at all."  Callers: control.rs ~12673 and
  ~14115.  The port never mints the ref (its `r6_0` state has no `__ref_1`), and Rust's ref
  is named by no op, has no slot and is never freed.  **Inert**: the same flag on a variable no
  code names.  (A full-interpreter trace names src/state/codegen.rs ~521 instead, which is
  codegen's own `skip_free` sweep.  The image is written before it, so trace the image run.)

**This generalises § 0.**  Every inert class so far is one of two things:
- a flag on a VARIABLE NO PASS-2 CODE NAMES (a capture placeholder; a retired work ref);
- the def-level `mutated_capture` temps.

One filter replaces the per-flag reader list: compare a variable's flags only where an op of
the function names the variable, or the variable is an argument.  Compare `mutated_capture`
only over the lambda's real captures.  Positive control: a work ref that IS named by code must
stay compared, and so must a `_`-named capture written in a lambda.
