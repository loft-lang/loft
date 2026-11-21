# @PLN91 — a text cast to a record or a struct enum, `s as R?` (prepared subject)

**The family:** "a checked narrowing cast" (7 sites).  This page takes the TEXT-source form, which
F-Render-Child/a-child-renders-and-parses-as-its-value holds alone (3 sites):
```
:29   f = "{e:j}" as Expr?;      (Expr a struct enum)
:44   back = s as List?;         (struct List { v: integer, next: List? })
:54   e = s as Expr?;
```
D-NoEnumF/a-checked-parse-answers-null-for-an-enum-field (:27, :34: `` `{…}` as CpBare? ``) is
the same form, not sole.  The port's cast (el_ex_match ~1834) carries only integer narrowing
under `as τ?`, so a text source with a record target falls through to the refusal.

**The rule:** a text cast to a heap type ALLOCATES: it interns the text into a new store
(`Parser::cast_allocates`, src/parser/convert.rs ~1473).  It is the same lenient parse that
`T.parse(text)` runs, and it answers null where the text does not parse.

## Rust's form, measured on try 709915c85

src/parser/convert.rs ~1559: for a `Text` source and a `Reference` or struct-enum target, the
code becomes `OpCastVectorFromText(<text>, <target's known type>)`.
```
:44   back = s as List?;
  [44] back: ref(List) = OpCastVectorFromText(s, 103);            <- 103 = List's known type

:29   f = "{e:j}" as Expr?;
  [29] f: ref(Expr) = OpCastVectorFromText({#Formatted string: text["__work_3"]
                        __work_3 = "";  OpFormatDatabase(__work_3, e, 100, 2);  __work_3 }, 100);   <- 100 = Expr
```
It is the same op and arguments the port's `type_parse` emits for `T.parse(x)`
(el_ex_object.loft ~2762).  The local takes the plain record reference, `ref(List)`.

## Each form judged

- **`OpCastVectorFromText(<text>, <R's known type>)`, a store of its own:** a RULE
  (`cast_allocates`).  A text source and a heap target can only be answered by interning.
- **The local typed `ref(R)`:** a record reference is its own null.  Carry it as measured.

## What to code

In the cast (el_ex_match ~1834, before the refusal): when the source is a text and the target
(under the `?`) is a struct reference or a struct enum, emit `OpCastVectorFromText(<the text>,
<the target's known_type>)`, typed as the target.  Reuse `type_parse`'s emission.  Leave
1752's form out (`(o.inner as S?) ?? …`, a RECORD field cast to its nullable type: a view, not
a parse); measure it on its own.

## Probe (the sole program; every assert round-trips a value)

```
kr-suite/rules/formatting/F-Render-Child/a-child-renders-and-parses-as-its-value.loft:29, :44, :54
```
