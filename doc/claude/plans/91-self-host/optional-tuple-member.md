# @PLN91 — a member read of a tuple local bound from a variable index (prepared subject)

**The family:** "a tuple member of this subject" (28 programs at try 1644a5f5f, 3 held by it
alone).  This page takes its `TyOptional` part: 14 sites, including 2 of the 3 sole
programs.  A tuple element read through a VARIABLE index (`b = t[i]`) is bound as an
OPTIONAL tuple (`(integer, text)?`: the index may be out of range).  The port then refuses
the member read `b.1` at `el_ex_assign.loft` ~1877, whose match carries a `TyTuple` subject
but not a `TyOptional` one.

**The rule:** `(T-Absent)`: an index out of range gives a PRESENT tuple of null members, so a
member of the optional tuple local reads its member slot as the dense tuple's does, and is null
where the element was absent.  No unwrap and no check are emitted.

## Rust's form, measured on try 1644a5f5f

```
N-Domain/1478-a-text-tuple-member-fits-its-owned-slot.loft:57
  fn pick78(t: vector<(integer, text)>, i: integer) -> text { b = t[i]; b.1 }
     b: (integer, text["t"])? = {#tuple_unbox
        __ref_1: ref(__tuple<integer,text>)["t"] = OpGetVector(t, 16, i);
        (OpGetInt(__ref_1, 0), OpGetText(__ref_1, 8)) };
     ___tret_1 = b.1;                                     <- the member read, as on a dense tuple
```

The bind (the `tuple_unbox` into an optional local) is already what the port builds, since the
refusal comes only at the member read.

## What to code

At el_ex_assign ~1877: let a `TyOptional` whose inner type is a tuple take the `TyTuple` arm.
The member's type is the member's own, and so are its deps (`text["t"]` keeps `t`).  Check one
site of each member kind: a scalar (`tin.0`) and a text (`tin.1`, `b.1`).

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/types/N-Domain/1478-a-text-tuple-member-fits-its-owned-slot.loft:57, :65   b.1 (text), from t[i]
kr-try/tests/scripts/an-arrived-absence-keeps-a-tuples-text-frees.loft:47                 tin.0, tin.1 from ts[i]
```

All of that file's cells are in `main`, which is refused at line 47 today, so they agree only
together: c1 (a constant index: a dense tuple, the control), e1 (in range), e2 (out of range: a
present tuple of null members), e3 (a copy owns its text), e4 (rebound every loop turn), e5 (a
nested tuple member with text), and c2 (a record member is a view, never freed).
