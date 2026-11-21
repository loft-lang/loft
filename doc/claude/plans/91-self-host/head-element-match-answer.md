# @PLN91 — a record function answering a vector match's element binding (prepared subject)

**The family:** "a vector match answering a record or a vector" (12 programs, 3 held by it
alone), its RECORD cluster.  A slice match at a record function's tail whose arm answers an
ELEMENT it bound (`[a, ..] => a`).  The sole programs are 1468-a-nullable-call-result-binds-
without-an-ownerless-copy (`head_1468`, `head_dense_1468`) and O-Deps/1414-a-nullable-element-
binding-borrows-its-subject (`first_struct`).  The cluster also covers 1466, 1992, 1934 and P-Rest's
:89.  The port refuses at `el_ex_match.loft` ~3178, where `buffer_arms` (rest-capture tails only)
answers null.  The vector cluster of this family is rest-capture-match-into-a-local.md.

**The rule:** `@FR-O-Buffer` / `@FR-B-View`: the element binding `a` is a VIEW of the subject's
element.  A function answering a record delivers into its hidden buffer, so each arm copies its
value in (`materialized_view_return`), and an absent element answers the null sentinel.

## Rust's forms, measured on try 57cd49288

```
1468:62   fn head_1468(p: vector<S1468>) -> S1468? { match p { [a, ..] => a, _ => null } }
  __retbuf: S1468 hidden        -> S1468["__retbuf"]?
  [62] {#vector_match
    if OpLeInt(1, OpLengthVector(p)) {#slice_binding
      a#1: ref(S1468)["p"] = OpGetField(OpGetVector(p, 8, 0), 0, 100);      <- the element, a view
      if OpRefIsNull(a#1) OpNullRefSentinel()
      else {#materialized_view_return  __retbuf = null;  OpDatabase(__retbuf, 100);
                                       OpCopyRecord(a#1, __retbuf, 100);  __retbuf } }
    else OpNullRefSentinel() }

1468:65   fn head_dense_1468(p) -> S1468 { match p { [a, ..] => a, _ => S1468 { a: 0 } } }
  __ref_p2_1: ref(S1468) = null;  -> S1468["__retbuf"]
  … the element arm exactly as above, its null check INCLUDED, though the result is dense …
  else {#materialized_view_return  __retbuf = null;  OpDatabase(__retbuf, 100);
          OpCopyRecord({#Object __ref_p2_1 … OpSetInt(__ref_p2_1, 0, 0) … __ref_p2_1 }, __retbuf, 100);  __retbuf }

O-Deps/1414   fn first_struct() -> S? { p: vector<S> = [S { a: 7 }, S { a: 8 }]; match p { [a, ..] => a, … } }
  -> S["__ref_1"]?              <- the buffer is named __ref_1 here, not __retbuf
  … if OpRefIsNull(a#1) OpNullRefSentinel() else {#materialized_view_return  … OpCopyRecord(a#1, __ref_1, 104) … }
```

## What to code

Let `buffer_arms` (or a sibling of it) take a record result.  For each arm:
- an element binding answers `if OpRefIsNull(a) OpNullRefSentinel() else {#materialized_view_return
  buf = null; OpDatabase(buf, <rec tp>); OpCopyRecord(a, buf, <rec tp>); buf }`.  Keep the null
  check in the DENSE case too, as Rust does;
- a record literal is built in its own `__ref_p2_N` and copied into the buffer the same way;
- `null` answers `OpNullRefSentinel()`.

The match answers the buffer, typed `S[<buffer>]` (with `?` for a nullable result).  **The
buffer's name follows the subject** (measured by the porter): over a PARAMETER, Rust keeps
`__retbuf`; over a LOCAL vector (1414's `first_struct`), it renames the return buffer onto a work
ref, `__ref_1`.  And the slice block and the match are typed as the buffer for a DENSE binding
(`S["__retbuf"]`), but keep the binding's own type for a nullable one (`S["a#1"]?`, 1414's
`head_n`).

## Probes (the sole programs, each with hand-computed answers)

```
kr-try/tests/scripts/1468-a-nullable-call-result-binds-without-an-ownerless-copy.loft:62, :65
kr-suite/rules/ownership/O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft:48
```
