# @PLN91 — an append whose item reads the vector it is appended to (prepared subject)

**The family:** "an appended item that reads the vector it is appended to, staged or
snapshotted", 6 sites in 5 programs at try c268bf4b9, and the ONLY refusal in 3: `v += [last(v) + i]`,
`rows += ["{s}:{len(rows)}"]`, `g += [a_mk(g[0]?, 50)]`, `b += [10 + len(b)]`, `docs += [from(docs[0])]`.

**The rule:** `(E-Asgn-Compound)` / `(I-Comp)`. The right-hand side is reduced, left to right and
with its effects, BEFORE anything is appended, so every item sees the vector as it was when the
statement began.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand: `v += [last(v) + i]` for i in 0..5 → `[0,1,3,6,10]`;
`["a:0","b:1","c:2"]`; `g += [a_mk(g[0]?, 50)]` then `[a_mk(g[1]?, 7)]` → 51, 58, `g[0]` still 1;
`v = [3]; v += [len(v), len(v) * 10]` → `[3,1,10]` (BOTH items see length 1).

## Rust: `snapshot_read_destination` (vectors.rs:6009), routes in order

1. pass 1, or a KEYED destination → nothing (a keyed literal is built through its destination).
2. a scalar/text element whose part reads the destination under ANOTHER name (a local viewing
   it, `reads_destination_by_another_name`) → `stage_literal_parts`.
3. no part reads the destination → nothing.
4. (literal, scalar or record element) a part hands the destination to a call that is neither
   primitive nor `#pure`, at a non-`const` parameter (`writes`), or a RECORD part that is a user
   call naming the destination (`record_call_reads`), or a place-prefix read → `stage_literal_parts`
   (when every record part naming the destination is a call: `stageable`).
5. a literal into a bare unlinked VARIABLE with a SCALAR element and every reading part
   hoistable (only primitive / `#pure` calls) → one `_build_val_N` temp per part that mentions it.
6. otherwise → the whole-vector snapshot (the rest of the function).

## The forms (tip e43620208)

```
route 4, scalar, a user call over the destination      v += [last(v) + i]
    __ref_1: integer = null;                                  <- head
    __ref_1 = OpAddInt(n_last(v), i);  OpPreAllocVector(v, 1, 8);  OpPushInt(v, __ref_1);
route 5, primitive reads, two items                     v += [len(v), len(v) * 10]
    _build_val_1: integer = t_6vector_len(v);  _build_val_2: integer = OpMulInt(t_6vector_len(v), 10);
    OpPreAllocVector(v, 2, 8);  OpPushInt(v, _build_val_1);  OpPushInt(v, _build_val_2);
route 4, a record call naming the destination           g += [a_mk(g[0]?, 50)]
    __ref_p2_2: ref(A) = n_a_mk({#ncc __ncc_1 = OpGetVectorNullable(g, 16, 0); … else {#Object __ref_p2_1 …}}, 50, __ref_1);
                                                         <- into a buffer of its OWN (__ref_1), not the element
    OpPreAllocVector(g, 1, 16);  _elm_2: ref(A)["g"] = OpNewRecord(g, 102, 65535);
    OpCopyRecord(__ref_p2_2, _elm_2, 16484);  OpFinishRecord(g, _elm_2, 102, 65535);
```

Compare the plain call append (keyed-append-of-a-record.md): there the element IS the callee's
buffer (`n_mk(i, _elm_1)`) with `OpDistinctStore`. Here the callee reads `g`, so building into a
new element of `g` would let it see a grown `g`. The call goes to its own buffer, and the element
is copied from it (`COPY_FRESH_DEST`, no source free).

**What to code:** run this test where the append is lowered: does any part read the destination?
If so, take the route in Rust's order above. Routes 4 and 5 cover the five corpus sites; route 6
(the whole snapshot) is not among them. Keep the head mints (`__ref_N` for a staged scalar,
`__ref_p2_N` / `__ref_N` per record call, in source order).

## The sites (try c268bf4b9)

```
kr-suite/rules/rewrites/R-ForwardWalk/a-walk-over-a-field-reads-its-length-where-the-body-can-change-it.loft::n_vcross
kr-try/tests/scripts/157-group-push.loft::n_g5
kr-try/tests/scripts/158-push-window.loft::n_b1
kr-try/tests/scripts/158-push-window.loft::n_d3
kr-try/tests/scripts/a-builder-rebinds-into-its-own-return-buffer.loft::n_main
kr-try/tests/scripts/a-local-built-for-the-returned-literal-is-built-in-its-buffers-store.loft::n_add_line
```
