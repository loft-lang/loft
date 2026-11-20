# @PLN91 — a lambda capturing a local whose type pass 1 cannot know (prepared subject)

**The family:** "a lambda that captures a local of its function" (32 programs at try 0ca590df5,
4 held by it alone).  This page takes its `of this type TyUnknown` part: one sole program,
three sites (`marked`, `summed`, `both`).  The captured local's type comes from a struct
declared BELOW the function (`count = len(w.items)` with `w: Holder`, `Holder` declared
later), so on pass 1 it is `TyUnknown`, and the port refuses the capture.  Every other
difference in the program follows from that refusal: the three `__lambda_*_0` bodies are
missing from the port's report (the gate lists them as DIFFER), and so is the test that calls
them.

**The rule:** `@FR-L-CapScalar` / `@FR-L-CapHeap`: a scalar capture is copied into the closure
record and a heap one is shared through it by reference, so the record's layout follows the
captures' types.  Where pass 1 cannot type a capture yet, the record is laid out on pass 2, once
the type is known.  It is not
refused, and the record is never laid out half-typed (loft#2496: a `vector<boolean>` capture
beside it was stored as a reference to definition 0 and read as unresolved).

## Rust's form, measured on try 0ca590df5

```
2496-a-forward-typed-capture-beside-a-boolean-vector-has-its-storage.loft:26
  fn marked(w: Holder) -> integer {
    count = len(w.items);  marks: vector<boolean> = [false, false, false, false];
    each(w.items, fn(n: integer) { if n < count { marks[n] = true; } });  …
     ___clos_1: ref(__closure_marked_0) = null;                          <- head
     [24] count: integer = t_6vector_len(OpGetField(w, 0, 7));
     [26] n_each(OpGetField(w, 0, 7), {#fn_ref_with_closure
            ___clos_1 = null;  OpDatabase(___clos_1, 103);
            OpSetInt(___clos_1, 0, count);                               <- count: integer at 0
            OpSetDbRef(___clos_1, 8, marks);                             <- marks: a reference at 8
            …
  fn __lambda_marked_0(n: integer, __closure: __closure_marked_0) -> void
     [27] if OpLtInt(n, OpGetInt(__closure, 0)) {
     [28]   OpSetBoolean(OpGetVector(OpGetDbRef(__closure, 8), 1, n), 0, true); }
```

## What to code

Where the port refuses a capture `of this type TyUnknown`: on pass 1, register the capture
without a field type (the record's fields stay open), and lay out `__closure_<fn>_<n>` on
pass 2 from the captures' pass-2 types, in capture order.  The record's known type and its
field offsets (`count` at 0, `marks` at 8) then follow from the types, as for any closure.
Check that a `vector<boolean>` capture is laid out as a reference to its vector and not left
unresolved (the loft#2496 case).

## Probe (the sole program, hand-computed: of `[0, 2, 7]`, two values are below the length 3)

```
kr-try/tests/scripts/2496-a-forward-typed-capture-beside-a-boolean-vector-has-its-storage.loft:26, :45, :59
```
