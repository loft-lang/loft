# @PLN91 — an early `return` of a tuple of scalars (prepared subject)

**The family:** "a `return` with a value of this type: TyTuple" (4 sites in 4 programs at try
e27ec2076, 3 of them held by it alone): `if i == 0 { return (a, b) }`,
`if d == 0 { return (q + 1, r); }`, `if z { return (null, 3); }`.  Every corpus site has scalar
members.  The refusal is the guard at el_ex_null.loft:2436 (`!scalar(result)`).

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check), values
hand-computed; a wrong expected value fails the run.  Probe:
[probes/early-tuple-return.loft](probes/early-tuple-return.loft), 12 cases, each checked by
its `chk_` caller.  On c70896b1a the scalar cases are refused at the guard.  `text_member` and
`record_member` are refused earlier, by their own families.

## Rust's form: the tail's form, at the `return`

```
if d == 0 { return (7, 8); } (d, d)          if OpEqInt(d, 0) {#block:never return (7i32, 8i32); } else null;  (d, d);
(integer?, integer): return (null, 3)        return (OpConvIntFromNull(), 3i32);
(u8, integer):       return (200 as u8, 1)   return (200i32, 1i32);
t = (d * 2, d * 3); return t                 return t;
return tail_only(d)                          return n_tail_only(d);
```

No buffer and no hold.  `parse_return` converts the value with the `convert_store` the tail
uses (control.rs:14425, "the statement path and the tail path must agree").  The port's tail
of the same tuples already agrees, the null member and the narrow constant included.

## What to code

At el_ex_null.loft:2436, let a result that is a tuple whose members are all scalars (nullable
or not) through the guard.  Its value is converted to the declared tuple as the function's tail
converts it, then `NdReturn`.  Leave the text and record members to their own families.

## The sites (try e27ec2076)

```
kr-try/tests/scripts/a-tail-recursive-small-record-function-is-a-loop.loft:27      (sole)
kr-try/tests/scripts/a-window-walk-guards-its-bounds-once-and-calls-plain-helpers.loft:31   (sole)
kr-suite/rules/rewrites/R-Range/a-call-that-moves-its-arguments-by-a-literal-steps-them.loft:20   (sole)
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft:53
```
