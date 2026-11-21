# @PLN91 — a record function answering a vector match's element binding, over a LOCAL vector (prepared subject)

**The subject:** the case head-element-match-answer.md leaves open, the one the porter measured
and kept refused.  O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft:48 has
`fn first_struct() -> S? { p: vector<S> = [S { a: 7 }, S { a: 8 }]; match p { [a, ..] => a, _ =>
null } }`.  The subject is a vector LOCAL the function builds, not a parameter.  The arms are
delivered exactly as over a parameter.  What differs is the hidden return buffer: Rust RENAMES it
onto a work ref.

**The rule:** `@FR-O-Buffer`: the arms deliver into the hidden buffer, as head-element-match-
answer.md has it.  Over a local subject, the buffer takes a work ref's name (`__ref_N`).

## Rust's form, measured on try 68bb3548b

```
fn first_struct() -> S? { p: vector<S> = [S { a: 7 }, S { a: 8 }]; match p { [a, ..] => a, _ => null } }
  __ref_1: S mutable nullable hidden            <- the hidden RETURN BUFFER, named __ref_1 (not __retbuf)
  -> S["__ref_1"]?
  {#block: ref(S)["__retbuf"]?                  <- the body block's type still says __retbuf
    __vdb_1 = null;  [48] OpDatabase(__vdb_1, 112);  p = OpGetField(__vdb_1, 0, 113);  …the two elements…
    [48] {#vector_match: ref(S)["__ref_1"]
      if OpLeInt(1, OpLengthVector(p)) {#slice_binding: ref(S)["__ref_1"]
        a#1: ref(S)["p"] = OpGetField(OpGetVector(p, 8, 0), 0, 104);
        if OpRefIsNull(a#1) OpNullRefSentinel()
        else {#materialized_view_return  __ref_1 = null;  OpDatabase(__ref_1, 104);
                                         OpCopyRecord(a#1, __ref_1, 104);  __ref_1 } }
      else OpNullRefSentinel() } }
```

Three things to copy as they stand:
- the hidden parameter's NAME is `__ref_1`;
- the result type and the match's type name `__ref_1`;
- the function's body block keeps `["__retbuf"]?`.

## What to code

In the record-answer road of head-element-match-answer (the port's `buffer_arms` sibling): when
the match's subject is a LOCAL rather than a parameter, rename the hidden return buffer onto a
fresh work ref (`__ref_N`, the parameter attribute renamed), type the result and the match by
it, and deliver into it.  Leave the body block's own type naming `__retbuf`.

## Probe

```
kr-suite/rules/ownership/O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft:48   (sole; its head_n,
    a nullable binding, keeps the binding's own type `S["a#1"]?`: head-element-match-answer.md)
```
