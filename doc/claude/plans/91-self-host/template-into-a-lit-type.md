# @PLN91 — a format string checked against a type that defines `lit` (prepared subject)

**The family:** "a value its declared type converts" (21 programs at try 0ca590df5, 2 held by
it alone).  This page takes its `TyText to TyReference` part: a template bound to a STRUCT type
(`q: Q = "x{a}y{b}{c}z"`).  That covers both sole programs, F-Target/each-hole-kind-calls-its-
own-method (7 sites) and a-nullable-hole-into-a-non-null-parameter-warns (1).  The port has no
template-object road: it types the template as text and refuses the conversion to `Q`.

**The rule:** `@FR-F-Target` (with `@FR-F-Desugar`, `@FR-F-Target-Pos`): a template checked
against a type that defines `fn lit(self, s: text)` does not build text.  It makes a fresh
value of that type and calls `lit(s)` for every NON-EMPTY literal run and `hole_<kind>(v)` for
every hole, in source order.  The kind comes from the hole's type: `u8`, `i32` and `i16` are
`hole_int`, a struct `Name` is `hole_name`, and an acronym name `SQLIdent` is `hole_sql_ident`.
A template with no holes makes one `lit`.  The empty template makes no call, and is still a `Q`.

## Rust's form, measured on try 5fa0629d9

```
F-Target/each-hole-kind-calls-its-own-method.loft:53
  struct Q { const parts: vector<text>, const log: vector<text> }
  fn lit(self: Q, s: text) { … }   fn hole_int(self: Q, v: integer) { … }   …
  a: u8 = 200;  b: i32 = -5;  c: i16 = 7;
  q: Q = "x{a}y{b}{c}z";
     [53] q: ref(Q)["__fmt_1"] = {#Formatted object
        __fmt_1: ref(Q) = null;  OpDatabase(__fmt_1, 103);
        OpSetInt4(__fmt_1, 0, 0);  OpSetInt4(__fmt_1, 0, 0);       <- parts' header, zeroed TWICE
        OpSetInt4(__fmt_1, 4, 0);  OpSetInt4(__fmt_1, 4, 0);       <- log's header, zeroed twice
        t_1Q_lit(__fmt_1, "x");
        t_1Q_hole_int(__fmt_1, a);
        t_1Q_lit(__fmt_1, "y");
        t_1Q_hole_int(__fmt_1, b);
        t_1Q_hole_int(__fmt_1, c);                                <- adjacent holes: no empty lit
        t_1Q_lit(__fmt_1, "z");
        __fmt_1 }
```

Each vector header is zeroed twice in Rust's form (the cause is not measured here).  The
port's form has to match it as it stands.

## Rust's code

- `Parser::interpolation_target` (src/parser/hints.rs ~478): the expected type is a struct with
  a method `lit`; otherwise the template is text, as today.
- `begin_format_object` (src/parser/objects.rs ~4034): the `__fmt_N` record, its `OpDatabase`
  and its field initialisation.
- `format_lit` (~4055): one `lit` call per non-empty run.  A `lit` with the wrong parameter
  count is a diagnostic there, not a refusal.
- The builder closes with `v_block(list, tp, "Formatted object")` (~3877).
- The hole kind is the hole's type mapped to a method name.  The cell asserts every scalar
  kind, a narrow integer, a struct, a plain enum, an acronym-named struct, and a null `text?`
  (nullability is not a kind).

## Probes (the two sole programs; each asserts the call log, so order, kind and value)

```
kr-suite/rules/formatting/F-Target/each-hole-kind-calls-its-own-method.loft:53, :63, :71, :78, :83, :90
kr-suite/rules/formatting/F-Target-Kind/a-nullable-hole-into-a-non-null-parameter-warns.loft:31
```
