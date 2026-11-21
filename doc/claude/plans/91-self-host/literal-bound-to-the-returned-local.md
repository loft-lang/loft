# @PLN91 — a record literal bound to the local a function returns, its fields reading a parameter (prepared subject)

**The family:** "a parameter rebuilt in place by a literal whose collection field reads it" (3
sites, 2 programs held by it alone):
```
164-adopt-first-bind.loft:134     fn grow(p: P) -> P { o = P { pts: p.pts, n: p.n + 1 }; o.pts += [7.0]; o }
O-Derived/1701-…-survives-the-next-allocation.loft:43
                                  fn rebuild(d0: Der, rel: single) -> Der { d = Der { step: d0.step, tiles: Tiles { … d0.tiles.proto … }, margins: d0.margins, src: d0.src }; check(d, …); d }
```
(O-Derived/1683 :52, `r = Flat { v: o.v }; r`, is the third site.)  The local (`o`, `d`) is the
one the function returns, so the first pass renames it onto the hidden return buffer (`o: P
hidden`).  The port then reads the bind as a PARAMETER rebuilt in place, and refuses because a
collection field of the literal reads a parameter (`p.pts`, `d0.tiles.proto`).

**The rule:** `@FR-B-Copy`: each collection field of the literal is a copy of the parameter's
field.  The parameter is left as it was, and the answer owns its collections.

## Rust's form, measured on try e420bc11c

```
164:134   struct P { pts: vector<float>, n: integer = 0 }
  p: P;  o: P hidden                    -> P["o"]
  {#block: ref(P)["__retbuf"]
    __ref_p2_1: ref(P) = null;  __ref_1: integer = null;      <- __ref_1 declared, never used
    [134] o: ref(P) = {#Object: ref(P)["__ref_p2_1"]          <- built APART, in its own store…
            __ref_p2_1 = null;  OpDatabase(__ref_p2_1, 102);
            OpSetInt4(__ref_p2_1, 8, 0);                      <- the vector field's header
            OpAppendVector(OpGetField(__ref_p2_1, 8, 100), OpGetField(p, 8, 100), 3);   <- p.pts COPIED in
            OpSetInt(__ref_p2_1, 0, OpAddInt(OpGetInt(p, 0), 1));
            __ref_p2_1 };                                     <- …and the hidden local bound to it
    OpPushFloat(OpGetField(o, 8, 100), 7f64);
    [134] o }

1701:43   the same: d: Der hidden;  d = {#Object __ref_p2_2 … OpAppendVector(<field>, <d0's field>, 0) …
          OpCopyRecord(OpGetField(d0, 32, 101), OpGetField(__ref_p2_2, 32, 101), 101) … __ref_p2_2 };
          the nested `Tiles { … }` literal written into the outer record's inline field
```
The literal is not built into the buffer.  It is built in its own `__ref_p2_N`, and the
renamed local is BOUND to it, exactly as for a plain local.

## Each form judged

- **Each collection field copied from the parameter (`OpAppendVector` / `OpCopyRecord`):** a
  RULE, `@FR-B-Copy`.
- **The renamed (hidden) local bound to a literal built apart, not built in place:** measured
  safe, not an observable fault.  `LOFT_LEAK_CHECK=1` runs pass with no store left behind: 164
  on the interpreter and `--native-c`, 1701 on the interpreter.  The representation is Rust's
  (C122: the contract is semantics); carry it.
- **`__ref_1: integer = null` declared and never used:** an ACCIDENT of internal numbers
  (@C168); carry it.

## What to code

At the refusal, tell the renamed return local from a real parameter.  A hidden local the first
pass renamed onto the buffer (`argument` set only because of the rename) takes the plain local's
bind: the literal built apart in `__ref_p2_N`, its collection fields copied from the parameter,
the local bound to it.  A true parameter rebuilt in place stays refused.

## Probes (the two sole programs; both leak-checked)

```
kr-try/tests/scripts/164-adopt-first-bind.loft:134
kr-suite/rules/ownership/O-Derived/1701-a-carried-record-passed-through-an-inline-rebuild-survives-the-next-allocation.loft:43
```
