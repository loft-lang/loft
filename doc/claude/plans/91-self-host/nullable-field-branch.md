# @PLN91 — a branch whose arms are nullable struct fields: each arm reads its own tag (prepared subject)

**The family:** "`if` arms of different types" (32 programs at try 0ca590df5, 2 held by it
alone).  This page takes one of the two sole programs, L-Null-Which/2572-a-join-of-a-nullable-
field-with-null-reads-its-tag.  It returns a nullable struct field from a branch: `struct H { a:
R?, b: R? }`, `fn chain_end(h: H, k) -> R? { if k == 0 { h.a } else if k == 1 { h.b } else
{ null } }`.  The port refuses `chain_end` ("`if` arms of different types: TyEnum, TyOptional"),
and two more functions in the same file already DIFFER per function, latent because the
program is refused: `by_match` and `match_null_first`.  One rule covers all three.

**The rule:** `@FR-L-Null-Which`: a nullable struct field is held inline as a `__nullable<R>`
wrapper (tag at 0, payload at 8).  A branch arm that answers such a field reads ITS OWN tag:
`if tag != 0 { <the payload> } else { null }`.  So the branch is an `R?` that VIEWS the owner
(`R["h"]?`), with nothing copied.

## Rust's form, measured on try fda5c52ba

```
fn chain_end(h: H, k: integer) -> R? { if k == 0 { h.a } else if k == 1 { h.b } else { null } }
  __retbuf: R hidden      -> R["h"]?
  [22] if OpEqInt(k, 0) {#block: ref(R)["h"]?
          if OpNot(OpEqInt(OpConvIntFromEnum(OpGetEnum(OpGetField(h, 0, 100), 0)), 0))
             OpGetField(OpGetField(h, 0, 100), 8, …) else …          <- h.a's own tag, per arm
       } else if OpEqInt(k, 1) {#block: ref(R)["h"]?
          … the same on OpGetField(h, 24, 100) …                    <- h.b's own tag
       } else {#block: ref(__nullable<R>)["h"]  OpNullRefSentinel() }
  the join is ref(R)["h"]?; the null arm's own type stays __nullable<R>
```

**The port's form for `by_match`** (a `match` with arms `h.a`, `h.b`, `null`) is different. It
types the whole match as the wrapper, `__ref_1: ref(__nullable<R>)["h"] = {#scalar_match …
OpGetField(h, 0, 100) … OpGetField(h, 24, 100) …}`.  It then reads the tag ONCE after the join
(`nullable_slot_read`), and copies the payload into a renamed buffer (`materialized_view_return`,
`__ref_2`, `-> R["__ref_2"]?`).  Rust keeps `__retbuf`, gives `-> R["h"]?`, unwraps per arm
inside the `scalar_match`, and copies nothing.

## What to code

When a branch arm (an `if` arm or a match arm) answers a nullable struct field (`__nullable<R>`)
and the branch's expected type is `R?`: unwrap AT THE ARM, as `if tag != 0 { OpGetField(field,
8, R) } else { null }`, typed `ref(R)[owner]?`.  The join is then `R?` with the owner's deps.
A `null` arm answers `OpNullRefSentinel()`.  Do not read the tag after the join, and do not copy
into a return buffer.  The same unwrap makes `chain_end`'s `else if` join (no refusal) and turns
`by_match` / `match_null_first` back to agreeing.

## Probe (the sole program; `main` asserts each function on fields absent and present)

```
kr-suite/rules/layout/L-Null-Which/2572-a-join-of-a-nullable-field-with-null-reads-its-tag.loft
   n_chain_end (refused :22), n_by_match, n_match_null_first (both DIFFER)
```

Not on this page: the family's other sole program, 2562's `if n > 5 { b.fr_h } else
{ [Fr2562E { … }] }` (a keyed field beside a vector literal).
