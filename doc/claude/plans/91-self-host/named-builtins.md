# @PLN91 — the parser's builtins by name: `deliver`, `expose`, `release`, `type_name(expr)` (prepared subject)

**The family:** "a call of an unknown function" (23 programs at try 0ca590df5, 3 held by it
alone).  These are not library functions: Rust's `parse_call` recognises them BY NAME
(control.rs ~15685–16030) and lowers each to an op or a constant.  The port has no arm for
`deliver`, `expose` or `release`, and parses `type_name` only with a TYPE argument
(syntax.loft ~1652, as `SSizeof`).  So a call of any of them falls through to the refusal
(el_ex_object ~1928).  This page covers 2 of the 3 sole programs.

**The rule:** each is a compiler builtin with a fixed lowering, and none is a user call.

## Rust's lowerings (control.rs)

```
deliver(tag, value)   ~15708   pass 1: Void, no code
                               pass 2: OpDeliver(tag, value, db_tp)   db_tp = get_type(<value's static type>)
expose(tag, value)    ~15729   pass 2: OpExpose(tag, value, db_tp)    (same shape)
release(tag, value)            pass 2: OpRelease(tag, value, db_tp)
type_name(x)          ~16013   pass 2: the TEXT CONSTANT display_type_name(<x's type>), typed text (no deps)
typedef(x)                     pass 2: Int(known_type of x's type), integer 0..65536
```

The value goes in by VALUE (its code, not an `OpCreateStack` slot), so a record's DbRef is
what the op reads, on both backends.

## What to code

Before the unknown-function refusal, add named arms: `deliver` / `expose` / `release` with two
arguments lower as above, with `db_tp` from the second argument's type (the port's
`get_type` / known-type of a record).  `type_name` with an EXPRESSION argument (not just a
type) becomes the text constant of that expression's type name; the expression is
elaborated for its type and its code dropped, as Rust drops it.

## Probes (two of the three sole programs, each with hand-computed answers)

```
kr-try/tests/scripts/pln105-expose-iterate.loft:36, :41, :68                 expose / release / deliver
kr-suite/rules/types/T-Syn/a-struct-literal-inside-sizeof-or-type-name-synthesises-its-type.loft:28   type_name(TPt { x: 1, b: 2 })
(also, not sole: kr-suite/rules/types/T-Syn/an-expression-synthesises-its-own-type.loft:52)
```

Not on this page: the third sole program, 22b-par-fold.loft:31 (`par_fold(range, 0, add_two,
4)`).  Rust lowers it in a parser routine of its own (`parse_par_fold`, control.rs ~15894),
which is a subject by itself.
