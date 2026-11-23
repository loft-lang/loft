# @PLN91 — contract, state and reasons to PARITY: the worklist (owner's priority, 2026-11-23)

**The rule:** parity first, before any refusal-led subject.  When the contract, state and reasons
layers all agree, the gate's `agree` itself is redefined: a program or definition agrees only when
its IR, its contract facts, its state and the rules behind its flags ALL align (owner: "merge
everything into the agree test, so an agree is only valid if everything aligns").  Until then the
gate's result line carries all four layers (kr-tools `contract_line.py`).

**Measured on b9b0b1052** (the tester's dump `kr-tools/cr-b9b0/`: `state.tsv`, `contract.tsv`,
`reasons.tsv`).  1404 of 2133 agree on the IR.
- contract: 1393 alike; `buffer_exits` is not derived anywhere.
- state: 1365 alike, 83 items in 39 programs.
- reasons: 1396 alike, 16 flags set by a different rule.

Every item below is decided by one Rust routine and its port counterpart (the
corresponding-routine ruling).  Ordered by programs cleared.

| # | layer: item | programs / items | Rust routine → port counterpart | owner |
|---|---|---|---|---|
| P1 | contract `buffer_exits` | all 1404 | `callee_buffer_exits` + `BufferExits::{exit_kind, answers, foreign}` → el_contract (prints `?` now): parity-buffer-exits.md | porter |
| P2 | contract `rebinds`: port reports `__closure` | 10 / 22 lambdas | Rust's rebinds fact (the visible parameters rebound with an owned store) leaves out `__closure`; the port's counterpart must too | porter |
| P3 | state `inline_refs` on a keyed `??` holder `__ncc_N` | 9 / 24 | Rust marks it `skip_free` and decides its null init by its deps (`keyed_null_mints`); the port's codegen reads the inline mark, so the null-init decision moves onto the deps first | porter |
| P4 | state `user_named` on `__place_N`, port-only | 8 / 26 | Rust marks a mention only where the variable IS the name written (`parse_var_inner`); the port marks `record_compound`'s held place (el_ex_place ~1004) | porter |
| P5 | `unbox_tuple_from_dbref`'s work ref: state `deps` Rust-only + reasons `skip_free` (the port also unregisters) | 5 / 7+7 | Rust marks it `skip_free`, keeps it a work ref, gives it the tuple as dep; the port adds `unregister_work_ref` and drops the dep.  Follow Rust's routine (the porter's Rust suspect #3 found no wrong program) | porter |
| P6 | state `skip_free` on a lambda `__ref_N`, Rust-only | 5 / 7 | R4: `RetPromotion::Bind { substitute }` and the capture-copy roads mint, substitute and retire (`unregister_work_ref`) | porter |
| P7 | reasons `user_named` on a parameter: Rust also by `parse_var_inner` | 4 / 9 | a parameter MENTIONED by a field append or tuple-member write is marked again at the mention; the port marks only `mark_declared_parameters` | porter |
| P8 | state `mutated_capture=__ncc_1`, port-only | 3 / 6 | R2's follow-up: port `collect_mutated_captures` over the capture list | porter |
| P9 | state `user_appended` on `__closure`, Rust-only | 2 / 2 | R3: the loft#1840 mark at `lhs_root_var` of the LOWERED place | porter |
| P10 | contract `borrowed_view` / `adopts_fresh` on 164 `mk_box` | 1 / 2 | a struct-enum local copied into `__retbuf` (variant store ≠ enum store) views nothing and adopts nothing | porter |
| P11 | singles | 8 / 10 | `copy_bound` d/c (a-struct-literal-inside-sizeof…), `_vec_1`/`_vec_4` inline_refs (2044, 501), `caller_hidden_buf` (a-non-escaping-vector-local…), `value_const` w (a-view-of-foreign-data…), `user_named` named1327, `deps` line (a-slice-that-escapes…), `skip_free` `__vdb_2`/`_vec_1` (`l1`), `__vdb_1` (a-nullable-vector-local-at-the-tail…) | porter; tester traces each Rust setter on request |

**After P1–P11:** contract, state and reasons all at 0 differences.  Then the agree merge
(tester, with the porter: `elab_gate.py`'s per-program and per-definition verdict requires all
four layers), and a GitHub baseline regenerated on the new definition.
