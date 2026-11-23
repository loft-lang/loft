# @PLN91 — contract, state and reasons to PARITY: the worklist (owner's priority, 2026-11-23)

**The rule:** parity first, before any refusal-led subject.  When the contract, state and reasons
layers all agree, the gate's `agree` itself is redefined: a program or definition agrees only when
its IR, its contract facts, its state and the rules behind its flags ALL align (owner: "merge
everything into the agree test, so an agree is only valid if everything aligns").  Until then the
gate's result line carries all four layers (kr-tools `contract_line.py`).

**Measured on d45974644** (the cut, main merged; every try and tip cycle now writes
`state.tsv` and `reasons.tsv` beside its `gate.txt`).  1405 of 2135 agree on the IR.
- contract: 1404 alike; `buffer_exits` is not derived anywhere.
- state: 1381 alike, 45 items (b9b0b1052: 83).
- reasons: 1397 alike, 16 flags set by a different rule — exactly P5's 7 and P7's 9, so P5
  and P7 together clear the reasons layer.

Done since b9b0b1052, each gated CLEAN: P2 (7cc755a88/11f4fc6b3), P4, P6 = R4, P8 = R2's
follow-up (318bb0aed), P3's refusal lift (bfcd797ec, on the cut), P9 = R3 (302e582e1,
post-cut).

Every item below is decided by one Rust routine and its port counterpart (the
corresponding-routine ruling).  Ordered by programs cleared.

| # | layer: item | programs / items | Rust routine → port counterpart | owner |
|---|---|---|---|---|
| P1 | contract `buffer_exits` | all 1404 | `callee_buffer_exits` + `BufferExits::{exit_kind, answers, foreign}` → el_contract (prints `?` now): parity-buffer-exits.md | porter |
| P3 | state `inline_refs` on a keyed `??` holder `__ncc_N` (the refusal that hid w5 is lifted; these 9 are in other programs) | 9 | Rust marks it `skip_free` and decides its null init by its deps (`keyed_null_mints`); the port's codegen reads the inline mark, so the null-init decision moves onto the deps first | porter |
| P5 | `unbox_tuple_from_dbref`'s work ref: state `deps` Rust-only + reasons `skip_free` (the port also unregisters) | 6 deps + 7 reasons: `__ref_12` an-arrived-absence-keeps-a-tuples-text-frees, `__ref_13` 1478, and the port's own cells a-vector-of-… (`__ref_1`, `__ref_2`), nested-tuple…, tuples-across… (`__ref_1`, `__ref_2`) | Rust's `skip_free` reason is `unbox_tuple_from_dbref` alone: it marks the work ref `skip_free`, keeps it registered, and gives it the tuple as its dep.  The port's reason is `unbox_tuple_from_dbref,unregister_work_ref` and its dep is missing.  Drop the `unregister_work_ref` and keep the dep (the porter's Rust suspect #3 found no wrong program, so Rust's routine stands) | porter |
| P7 | reasons `user_named` on a parameter: Rust also by `parse_var_inner` | 9 in 3 programs: F-ParamScalar a-compound-write-to-a-tuple-parameters-member… (`t` in appended, plain, number_beside_text, both), R-KeyedEntry a-group-member-takes-the-plain-insert… (`g` fill_g, `h` fill_p), R-KeyedEntry a-keyed-insert-writes-the-entry-its-key… (`s` bump, bump_one, put3) | Rust's `parse_var_inner` marks `user_named` on EVERY mention of an existing variable spelled by its own name (src/parser/objects.rs ~1043: `if self.vars.name(index_var) == name { mark_user_named }`), parameters included, so a parameter's reasons are `mark_declared_parameters,parse_var_inner`.  The port's name route marks a plain mention, but its tuple-member compound write (`t.0 += "!"`) and its append into a parameter's keyed field (`g.by_k += [G { … }]`, the R-KeyedEntry insert) reach the parameter without passing it: mark it there, under the same reason | porter |
| P10 | contract `borrowed_view` / `adopts_fresh` on 164 `mk_box` | 1 / 2 | a struct-enum local copied into `__retbuf` (variant store ≠ enum store) views nothing and adopts nothing | porter |
| P11 | singles | 8 / 10 | `copy_bound` d/c (a-struct-literal-inside-sizeof…), `_vec_1`/`_vec_4` inline_refs (2044, 501), `caller_hidden_buf` (a-non-escaping-vector-local…), `value_const` w (a-view-of-foreign-data…), `user_named` named1327, `deps` line (a-slice-that-escapes…), `skip_free` `__vdb_2`/`_vec_1` (`l1`), `__vdb_1` (a-nullable-vector-local-at-the-tail…) | porter; tester traces each Rust setter on request |

**After P1–P11:** contract, state and reasons all at 0 differences.  Then the agree merge
(tester, with the porter: `elab_gate.py`'s per-program and per-definition verdict requires all
four layers), and a GitHub baseline regenerated on the new definition.
