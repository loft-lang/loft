# @PLN91 — the enum-level dispatcher synthesised from per-variant methods (@F20) (prepared subject)

**Status: DONE** — measured at 0a1a8b43c: its refusal no longer occurs.  Kept as the record of the subject.

**The family:** "a variant method its enum answers nowhere (the @F20 dispatcher)": 6 programs,
2 held by it alone (docs/09-enum, D-Scalar/1254-an-empty-stub-returns-its-types-default).  The
others are 05-enums, F-Recv/a-call-reaches-the-method-spelled-for-its-receivers-nullability,
L-Null/1782 and M-Variant/1778.  When an enum's variants each declare a method
(`fn area(self: Circle)`, `fn area(self: Rect)`) and the enum declares none, Rust synthesises
`t_<n><Enum>_<method>`.  It dispatches on the tag to each variant's method.  The port's
`el_dispatch.loft` ~378 refuses wherever that definition would be needed and does not exist:
it synthesises no dispatcher.

**The rule:** @F20 (`Disp-Exhaustive`, @PLN162).  Calling the method on the enum runs the
receiver's variant's method.  An absent receiver answers the TYPED null of the result
(`@FR-M-Variant`, its nullable-subject paragraph).  An uncovered variant is a compile error, so
the synthesised dispatcher always covers every variant.

## Rust's form, measured on try 49eb3a518

`Parser::enum_fn` (src/parser/definitions.rs ~583; FIRST pass only) selects the methods, and
`create_enum_dispatch_fn` (~406) builds each:
```
09-enum   enum Shape { Circle { radius: float }, Rect { width: float, height: float } }
          fn area(self: Circle) -> float { … }   fn area(self: Rect) -> float { … }
#896 t_5Shape_area   at=192:2                        <- numbered after every source definition (n_main is #895)
  self: ref(Shape) mutable nullable
  -> float
  {#dynamic_fn: float
    if OpEqInt(OpConvIntFromEnum(OpGetEnum(self, 0)), 1) {#ret  return t_6Circle_area(self); } else null;
    if OpEqInt(OpConvIntFromEnum(OpGetEnum(self, 0)), 2) {#ret  return t_4Rect_area(self); } else null;
    return OpConvFloatFromNull(); }                       <- the typed null, for an absent receiver

#897 t_5Shape_describe   (text-returning: the arms' hidden buffers are the dispatcher's own)
  self: ref(Shape) mutable nullable;  __work_1: &text mutable nullable;  ___tret_1: &text mutable nullable hidden
  -> text["___tret_1", "__work_1"]
  {#dynamic_fn: text["___tret_1"]
    if <tag == 1> {#ret { !! INSERT  ___tret_1 = t_6Circle_describe(self, __work_1);  return ___tret_1 } } else null;
    if <tag == 2> {#ret { !! INSERT  ___tret_1 = t_4Rect_describe(self, __work_1);  return ___tret_1 } } else null;
    ___tret_1 = OpConvTextFromNull();  return ___tret_1; }

1254   #888 t_6Sh1254_ar1254   the same shape as t_5Shape_area (float, two variants, tags 1 and 2)
```

## Each form judged

- **One `if <tag == k> return <variant method>(self, …)` per variant, in variant order, then the
  typed null:** a RULE (@F20, `@FR-M-Variant`).
- **Hidden buffers** (a text result): the dispatcher takes the hidden attributes of the FIRST
  implementation that has any.  It forwards visible parameters to every arm, and hidden ones
  only to the arms that declare them (definitions.rs ~487–548, loft#1430).  A RULE: an arm writes
  into the caller's buffer.
- **`at=` the file's end and the number after every source definition:** an ACCIDENT of
  internal numbers (@C168).  The dispatcher is made at the end of the first pass; carry it.

## What to code

Synthesise the dispatcher in the port where `el_dispatch.loft` ~378 now refuses.  Select by
`enum_fn`'s rules: the port already ports `overload_set_owns_dispatch`,
`dispatches_past_receiver` and the free-member test.  Add one definition per (enum, method)
after every source definition, with the receiver `self: ref(<Enum>)`, the first implementation's
return type and the hidden attributes as above, and the body above.  Start with 1254 and
09-enum's `area` (float, no buffers), then `describe` (text).

## Probes (the two sole programs)

```
kr-suite/rules/types/D-Scalar/1254-an-empty-stub-returns-its-types-default.loft   (t_6Sh1254_ar1254)
kr-try/tests/docs/09-enum.loft   (t_5Shape_area, t_5Shape_describe; main calls each on a Circle and a Rect)
```
