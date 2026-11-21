# @PLN91 — a vector match bound to a LOCAL whose arm answers its rest capture (prepared subject)

**The family:** "a vector match answering a record or a vector" (12 programs at try 42c658e24,
3 held by it alone).  The family has two clusters.  This page takes the larger one: a vector
match bound to a local (`r = match v { [_, ..t] => t, _ => [] }`), whose arm answers a REST
capture.  That is 9 sites, among them the sole program P-Rest/a-rest-is-a-fresh-capture-
wherever-it-stands.  The other cluster, an arm answering a head ELEMENT record (`[a, ..] => a`
in a `-> S?` function: 1468, 1414, 1466, 1992, 1934), is not on this page.

The port refuses at `el_ex_match.loft` ~3178, where `buffer_arms` returns null.  `buffer_arms`
carries one shape only: a slice match at a vector FUNCTION's tail whose first arm answers its rest
capture, with the return buffer renamed onto the capture.  A match bound to a local is none of
that.

**The rule:** `@FR-P-Rest`: a rest capture is a FRESH vector, materialised from the subject.
`@FR-B-Copy`: a local bound from the match owns what it holds, so the capture it answers is copied
in, never aliased.

## Rust's form, measured on try 42c658e24

```
P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft:188
  v = [1, 2, 3];  r = match v { [_, ..t] => t, _ => [] };  r[0] = 9;
  [188] r: vector<integer>["__vdb_4"] = null;                     <- the local declared, the bind sunk per arm
  {#vector_match
    if OpLeInt(1, OpLengthVector(v)) {#slice_binding
      { !! INSERT  OpDatabase(__vdb_2, 20)                         <- t#1, the rest, materialised fresh
                   t#1: vector<integer>["__vdb_2"] = OpGetField(__vdb_2, 0, 7);  OpSetInt4(__vdb_2, 0, 0)
                   { _rest_lo_1 = 1;  _rest_hi_1 = OpMinInt(OpLengthVector(v), 0);  _rest_idx_1 = null }
                   loop {#Slice materialise  … OpNewRecord(t#1, 7, 65535) … OpFinishRecord(t#1, …) } };
      { !! INSERT  OpDatabase(__vdb_4, 20)                         <- r its own store,
                   r = OpGetField(__vdb_4, 0, 7);  OpSetInt4(__vdb_4, 0, 0)
                   OpAppendVector(r, t#1, 0) } }                   <- the rest COPIED in
    else r: vector<integer>["__vdb_4"] = {#Vector  OpDatabase(__vdb_3, 20);   <- the `_` arm: bound to the
              _vec_1 = OpGetField(__vdb_3, 0, 7);  OpSetInt4(__vdb_3, 0, 0);  _vec_1 }   literal's own store
  }
  [189] OpSetInt(OpGetVector(r, 8, 0), 0, 9);                     <- r is independent of v
```

## What to code

For a vector match bound to a local (`r = match … { … }`) whose result is a vector, sink the
bind into each arm, as Rust does:
- a rest-capture arm materialises the capture (the port already has `Slice materialise`), then
  gives the local a store of its own (`OpDatabase(__vdb_N, <vec tp>)`, the field, the header)
  and copies in, `OpAppendVector(r, <capture>, 0)`;
- a literal arm binds the local to the literal's own `__vdb_N`;
- the local is declared null ahead of the match, typed `vector<…>[<the copy's store>]`.

## Probes

```
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft:89, :188   (sole)
kr-suite/rules/binding/P-Cap-Fresh/a-rest-or-a-repetition-is-a-fresh-store-and-deeply-so.loft:49, :59, :70, :83, :94, :163
kr-suite/rules/binding/P-Cap-View/a-pattern-capture-is-a-view-and-a-rest-is-fresh.loft:84
```
