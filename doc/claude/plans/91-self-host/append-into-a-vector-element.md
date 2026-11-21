# @PLN91 — `+=` into an element of a vector of vectors (prepared subject)

**The family:** "an append to a collection that is not a field of a record" (26 programs at try
775c31471, 3 held by it alone).  This page takes its `OpGetVector of TyVector` part: an append
whose target is an ELEMENT of a vector of vectors (`vv[0] += [9]`, `h.ws[i] += [9]`), which
covers 2 of the 3 sole programs.  The port refuses at `el_ex_place.loft` ~1594.

**The rule:** an element of a vector of vectors is a place (`@FR-B-View`): `+=` appends to the
inner vector where it lies, as onto a field.  A compound write evaluates its place once, so an
index with a side effect runs once (`@FR-E-Asgn-Compound`, loft#2581).

## Rust's form, measured on try 775c31471

```
growing-or-emptying-a-vector-ends-the-places-inside-it.loft:120
  vv[0] += [9];
     [120] OpPushInt(OpGetVector(vv, 4, 0), 9);           <- each item pushed onto the element read

2581-a-compound-write-through-a-block-index-evaluates-the-block-once.loft:27
  h.ws[{ f.calls += 1; 1 }] += [9];
     [27] __place_4: vector<vector<integer>>["h"] = OpGetVector(OpGetField(h, 4, 100), 4,
                                                   {#block OpSetInt(f, 0, OpAddInt(OpGetInt(f, 0), 1)); 1 });
          OpPushInt(__place_4, 9);                        <- the place held once, then pushed onto
```

The element size `4` is the inner vector's handle.  The pushes are the same pushes a vector
field takes for a literal of plain items.

## What to code

At el_ex_place ~1594: an append whose target is `OpGetVector` of a vector-of-vectors takes
the field append's road, with the element read as the receiver.  Where the index is not a plain
variable or constant (a block, a call), hold the element once in a `__place_N`, as the port
already does for `keep[i]?.k += add` (el_ex_assign ~2954), and push onto that.

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/binding/B-Disturb/growing-or-emptying-a-vector-ends-the-places-inside-it.loft:120   vv[0] += [9]
kr-suite/rules/operational/E-Asgn-Compound/2581-a-compound-write-through-a-block-index-evaluates-the-block-once.loft:27   h.ws[{…}] += [9]
```

Not on this page: the third sole program's `p.v += [7]` inside a lambda over a captured heap
parameter (`OpGetDbRef of TyVoid`, a-captured-heap-parameter-is-shared-with-the-caller…:20).
