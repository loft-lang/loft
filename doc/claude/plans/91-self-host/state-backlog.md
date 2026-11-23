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

**Owner ruling (2026-11-23): the comparison stays STRICT equality** ("that leads to far easier
comparison of code.  If you dislike the rust implementation that one has to move first").  So
nothing is filtered out of the gate.  A flag that is inert because Rust's implementation is
accidental is a RUST change: fix it in Rust first, prove the change moves no IR (gate E on the
Rust side unchanged) and no suite cell (both backends), and the port follows.  The classes
above become Rust work:

**And (owner, same day): "There should always be a corresponding routine between the two
implementations for a decision that can be compared."**  Strict equality is compared ROUTINE
against routine.  Each decision is made in one Rust function and in the port function that
mirrors it, and the reasons layer names that function.  A port that reaches Rust's answer by a
different routine is a defect even when the output agrees.  For each change below, the Rust
routine and its port counterpart:
- R1: Rust's `parse_var_inner` capture arm; the port's capture-read road (el_ex_name) mirrors
  it once the placeholder is gone.
- R2: Rust's `collect_mutated_captures` (+ `walk_for_mutations`).  The port's per-capture
  `writes_a_name` is NOT its counterpart, so the port ports the fixed routine.
- R3: Rust's loft#1840 mark (expressions.rs ~11065, `lhs_root_var` over the LOWERED place) and
  its port counterpart (el_ex_assign `assignment_inner`, `append_place` over the SYNTAX): see § R3.
- R4: Rust's `unregister_work_ref` and its two callers.  The port needs the counterpart (the
  porter carried it for `pick` in 71006afc3).

| # | Rust change | removes | site |
|---|---|---|---|
| R1 | the pass-1 capture placeholder does not survive into pass 2's variable table (or is not made) | 446 items: `user_named` 417, `deps`/`user_appended` 29 | objects.rs `parse_var_inner`'s capture arm (~1261: `create_var` + `mark_user_named`) |
| R2 | `collect_mutated_captures` lists only the lambda's captures, not every written name in its variable table | 42 items, 17 programs | its candidate list (1a-parser.md § Rust suspects); its readers already intersect with the captures |
| R3 | NOT a Rust change (measured 11-23): Rust's mark is right.  The port's counterpart must root at the LOWERED place | 2 items | see § R3 below |
| R4 | NOT a Rust change (measured 11-23): the port carries Rust's mint-substitute-retire | the 6 lambda `__ref_1 skip_free` items | see § R4 below |

Order: R2 (smallest, readers unaffected), then R1 (the largest gain), then R3 and R4.  For each,
first probe the claim that nothing observable moves:
- emit the image before and after and diff it with gate E on the Rust side;
- run the suite cells of the area on both backends.

The port then drops nothing: it already has no placeholder, lists only captures, and mints no
retired ref.  R4 is the one exception that may go the other way.  Rust's mint-then-retire is
its reasoning for `pick` (the decision-order rule), so for R4 judge per caller whether the
port carries the mint instead.

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
  code names.  (A full-interpreter run prints a second line from codegen's own `skip_free` sweep,
  src/state/codegen.rs ~521, under the spelling `n___lambda_…`; the image's rule is the
  `__lambda_…` line.  DEBUG_TRACE.md § A function's contract and state.)

**This generalises § 0.**  Every inert class so far is one of two things:
- a flag on a VARIABLE NO PASS-2 CODE NAMES (a capture placeholder; a retired work ref);
- the def-level `mutated_capture` temps.

Under the strict-equality ruling this is no gate filter: it is the list of Rust changes R1–R4 above.

## R1, designed (11-23; built after R2 lands)

The placeholder is load-bearing on PASS 1, so it is retired, not removed:
- **Why pass 1 needs it:** a capture read has no closure record yet, so `parse_var_inner`'s
  capture arm (objects.rs ~1261) makes a variable of the capture's name, which types the read
  and is the target of a write.  `collect_mutated_captures` finds the pass-1 writes through it,
  and that decides each capture's storage (auto-Reference for a mutated one).
- **Pass 2:** reads go through the closure record, and no pass-2 code names the placeholder.
  But the variable table persists across the passes, so it reaches the image with its flags:
  `user_named`, a self-dep (`deps=n` on a boxed capture), `user_appended`, and `value_const`
  (`capture_const`).

The change:
1. Mark the variable at its creation as a pass-1 capture placeholder (an explicit mark, not a
   name test).
2. Once the lambda's pass 2 has parsed its body, clear those four flags on each marked variable
   no pass-2 code names.
3. The variable keeps its index (no renumbering), but carries no flag, so the state report has
   nothing on it to compare.

It is a retire like `unregister_work_ref`.  The decision leaves the comparison and the port
needs no counterpart, since it makes no placeholder.  Prediction: the 446 items (`user_named`
417, `deps` 20, `user_appended` 9) gone, nothing new, the IR identical, and the closure suite
cells green on both backends.

## R3, measured (11-23): a port item, not a Rust change

`f = fn(tag: text, r: boolean) -> boolean { log += tag; r }` (E-And/a-decided-left-operand…,
E-Or/a-true-left-operand…).  Rust's loft#1840 mark records the program's own `+=` on the
variable its place is rooted at, `lhs_root_var(&to)` over the LOWERED place.
- **Pass 1:** the root is the placeholder `log` (retired by R1).
- **Pass 2:** the capture is read through the closure record, so the root is `__closure`, and
  `[varflag] user_appended __closure … @ expressions.rs:11065` follows.

That is Rust's rule applied consistently: the append lands in the closure record's content.
The port's counterpart (el_ex_assign.loft ~903, `assignment_inner`) roots the mark at the
variable the SYNTAX names (`append_place`).  It reaches `__closure` for some capture appends,
which the porter carried, but not for this text append through a capture.  By the
corresponding-routine ruling the port's mark roots at its lowered place, as Rust's does.  So
this is the porter's, and nothing changes in Rust.

## R4, measured (11-23): the port carries the mint-substitute-retire

`LOFT_TRACE_RETPROMO=1` gives Rust's verdict per lambda (image run):
- **`r6_0` (a-rebind-exchanges…), 1353 `main_2`, a-plain-bind-of-a-fn-ref… `_1`:** the tail's
  work ref `__ref_1` is classified `SkipInnerRef` on pass 1 and `Bind { … }` with a buffer on
  pass 2.  So `RetPromotion::Bind { substitute }` (control.rs ~14115) substitutes the call's
  work ref by the return buffer and retires it (`unregister_work_ref`: `skip_free`, no
  null-init, no free).
- **1659's three lambdas** (`coalesced_capture_0`, `capture_or_mint_inline_0`,
  `nullable_capture_or_null_0`): a return from a block that materialises a view, which mints
  and retires `__ref_1`.  This is the same rule as 2471's `pick` (the porter's 71006afc3,
  `branch_view_return`).

In both, the port delivers straight into `__retbuf` and never mints the ref: a different routine
reaching Rust's answer.  Rust's reasoning holds (a call's answer gets a work ref, and the
one-buffer binding substitutes it), so under the corresponding-routine ruling the PORT carries
it.  It mints the call's work ref, substitutes it by the buffer and retires it, at its
counterparts of the Bind-substitute caller and of the capture-copy roads
(`capture_tail_copy`/`capture_if_copy`).  No Rust change.
