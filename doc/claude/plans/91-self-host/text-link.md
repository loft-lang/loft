# @PLN91 — a `&` link to a text, a local or a field (prepared subject)

**The family:** "a `&` reference to something other than a scalar or a vector place" (24
programs at try 0ca590df5, 3 held by it alone).  This page takes its `TyText` part, a `&` link
to a TEXT, which covers 2 of the 3 sole programs:
- a text field (`n = &q.m`, `f = &o.a`);
- a nullable text local (`pc = &c` with `c: text?`).

The same 1372 program also links a nullable record (`pd = &d`) and a nullable vector (`pe =
&e`).  The port refuses with `TyText`, `TyReference` and `TyVector`.

**The rule:** `@FR-B-Ref-Alias` / `@FR-B-Ref-Repoint`: `&` makes the binding a LINK to the
place, and reads and writes go through it.  A text link has a KIND, the kind of place it names:
a text VARIABLE (a frame slot) or a text FIELD or element (a slot in a store).  The kind is part
of the link's type.

## Rust's form, measured on try 67b6c2065

```
B-Ref-Lvalue/167-a-text-link-into-a-store.loft:70        a FIELD link (struct Q { m: text? })
  q = Q { m: null };  n = &q.m;  assert(n == null && q.m == null, …);  n = "set";
     [70] n: &text["q"] = OpGetField(q, 0);                       <- the field's place in q's store
     [71] … OpEqText(OpGetText(OpVarRef(n), 0), OpConvTextFromNull()) …   <- read through the link
     [72] OpSetTextReplace(OpVarRef(n), 0, "set");                <- write through the link

B-Ref-Intro/1372-a-reference-links-a-nullable-slot.loft:72   a VARIABLE link
  c: text? = "a";  pc = &c;  pc = "z";  assert(c == "z", …);
     [72] pc: &text["c"]? = OpCreateStack(c);                     <- the frame slot of c
     [73] pc: &text["c"]? = "z";                                  <- a Set through the link
```

The field link's type is `&text["q"]` (it borrows the OWNER), and the variable link's is
`&text["c"]?` (nullable, as `c` is).

## What to code

Where the port refuses `&` of a `TyText` place: for a field, take `OpGetField(owner, pos)`, typed
`&text[owner]`; reads become `OpGetText(OpVarRef(link), 0)` and writes
`OpSetTextReplace(OpVarRef(link), 0, value)`.  For a variable, take `OpCreateStack(var)`, typed
`&text[var]` with the variable's nullability; a write is a `Set` of the link.  Then measure
1372's `pd = &d` (a nullable record) and `pe = &e` (a nullable vector), which the same program
needs.

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/binding/B-Ref-Lvalue/167-a-text-link-into-a-store.loft:70, :103      field links
kr-suite/rules/binding/B-Ref-Intro/1372-a-reference-links-a-nullable-slot.loft:72, :77, :82   variable links (text?, S?, vector?)
```

Not on this page: the third sole program's `child: &nodes[i - 1]`, a reference STORED in a keyed
collection's record (a-stored-reference-into-a-keyed-collection-holds-w…:36).
