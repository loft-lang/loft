# @PLN91 — a record-answering lambda that answers its capture: 1659's other five lambdas (prepared subject)

**The subject:** what O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once needs beyond
lambda-coalesce-over-a-capture.md to agree whole.  The porter carried the `??` lambda
(wip/lambda-ncc-over-a-capture.patch).  That turns 1659 from refused into DIFF, because five
other lambdas then differ.  Measured, they are THREE forms: `rebound_capture_is_a_copy_0` and
`bound_capture_written_through_is_a_copy_0` are byte-identical to
`capture_or_mint_bound_leaves_the_capture_0`, apart from the name and the position.
```
:100, :163, :172   g = fn(i: integer) -> S1659 { if i > 1000 { return S1659 { v: 9.0 }; } return cap; };      (form A)
:111               g = fn(i: integer) -> S1659 { if i % 2 == 0 { cap } else { S1659 { v: 1.0 } } };            (form B)
:124               g = fn(i: integer) -> S1659? { if i % 2 == 0 { capn } else { null } };                       (form C)
```
**What the port does differently** (the porter's measurement): forms A and B rename the buffer
onto `__ref_1`.  Form C answers the capture as a bare view.

**The rule:** `@FR-O-Buffer`, the view half: a capture belongs to the frame that built the
closure.  A lambda answering it copies it into `__retbuf` behind its null test, wherever it
answers it: an early `return`, an `if` arm, or the tail.

## Rust's forms, measured on try 71e086496

The buffer is **`__retbuf` in all three, dense or nullable**.  The capture is read as
`OpGetDbRef(__closure, 0)`.
```
A  (:100)   -> S1659["__retbuf"]            body {#block: ref(S1659)  …}
    __ref_p2_1 = null;
    if OpLtInt(1000, i) {#block: never
       return {#Object __ref_p2_1 … OpSetFloat(__ref_p2_1, 0, 9f64) … __ref_p2_1 } } else null;   <- the literal returned AS BUILT
    return if OpRefIsNull(OpGetDbRef(__closure, 0)) OpNullRefSentinel()
           else {#materialized_view_return  __retbuf = null; OpDatabase(__retbuf, 101);
                   OpCopyRecord(OpGetDbRef(__closure, 0), __retbuf, 101); __retbuf };

B  (:111)   -> S1659["__retbuf"]            body {#block: ref(S1659)  …}
    __ref_p2_1 = null;
    return if OpEqInt(OpRemInt(i, 2), 0)
      {#block: ref(S1659)["__retbuf"]
         if OpRefIsNull(OpGetDbRef(__closure, 0)) OpNullRefSentinel()
         else {#materialized_view_return … OpCopyRecord(OpGetDbRef(__closure, 0), __retbuf, 101) … } }
      else {#materialized_view_return  __retbuf = null; OpDatabase(__retbuf, 101);
              OpCopyRecord({#block: ref(S1659)  {#Object __ref_p2_1 … OpSetFloat(__ref_p2_1, 0, 1f64) … } },   <- the literal in a block
                           __retbuf, 101); __retbuf };

C  (:124)   -> S1659["__retbuf"]?           body {#block: ref(S1659)?  …}      (no `return`, no __ref_p2_N)
    if OpEqInt(OpRemInt(i, 2), 0)
      {#block: ref(S1659)["__retbuf"]  if OpRefIsNull(OpGetDbRef(__closure, 0)) OpNullRefSentinel()
                                        else {#materialized_view_return … OpCopyRecord(OpGetDbRef(__closure, 0), __retbuf, 101) … } }
    else {#block: ref(S1659)["__retbuf"]  OpNullRefSentinel() }
```

## Each form judged

- **The capture copied into `__retbuf` behind `OpRefIsNull`, in every place it is answered:** a
  RULE, `@FR-O-Buffer`.  `LOFT_LEAK_CHECK=1` passes on 1659 (interpreter, exit 0).
- **The buffer stays `__retbuf`:** what Rust does.  The port's rename onto `__ref_1` in A and B is
  the port's defect of naming.
- **A's early `return` of a literal handed back AS BUILT (`__ref_p2_1`), not copied in:** what
  Rust does; the literal is a fresh store already.  Carry it.
- **B's dense tail wrapped as `return if …`, and its literal arm wrapped in a `{#block}` inside
  `OpCopyRecord`:** forms of Rust's tail delivery; carry them as they stand.
- **C, nullable: no `return`, and the `null` arm a `{#block: …["__retbuf"]}` around the sentinel:**
  carry it.

## What to code

Carry the capture materialisation in each position (an early `return`, an `if` arm, the tail),
keeping `__retbuf` and not renaming it.  Work through A (three lambdas at once), then C, then B,
then the `??` from lambda-coalesce-over-a-capture.md.  1659 then agrees whole.

## Probe

```
kr-suite/rules/ownership/O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once.loft:100, :111, :124, :137, :163, :172
```
