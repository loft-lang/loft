# A slice of a vector that is no local — what the Rust front end does (@PLN91)

What to mirror for `<vector>[lo..hi]` where the port refuses "a slice of a vector that is no
local of plain scalars".  The probe is [probes/vector-slice.loft](probes/vector-slice.loft):
25 cases, every value hand-computed, clean on the interpreter (also under `LOFT_POISON=1`) and
the C backend.

## What is already carried, and the three gaps

The port agrees on every CONTEXT of a slice — bound to a local, walked by a `for`, handed as
an argument, measured with `len`, appended with `+=` — as long as the subject is a local, a
parameter, a field of scalars, or an element of a vector of vectors.  It refuses three things,
and each is one rule below:

| gap | probe cases | met in the corpus as |
|---|---|---|
| the subject is a CALL's result | `call_bind`, `call_walk`, `call_arg`, `call_len`, `call_append`, `call_recs_bind`, `native_bind`, `open_lo`, `open_hi`, `negative`, `call_once` | `sv_mk(c)[1..3]`, `src(log)[1..4]`, `j.keys()[1..3]`, `"…".split('-')[1..4]` |
| the subject is a FIELD whose elements are records | `field_recs_bind`, `field_recs_len` | `len(bl.operators[0..2])` (C-Never 2444) |
| the ELEMENTS are vectors | `nested_bind` | `t = [["a{n}"], ["b"], ["c"]]; ts = t[0..2]` (script 1338) |

(The refusal is met on 22 corpus lines in 16 programs.  A nullable local sliced through `?[…]`
is not on this page.)

## 1. A call as the subject: held once in a work ref

The call is evaluated ONCE into a work ref of its own, and that work ref is the subject
everywhere a local would be (`OpLengthVector`, the element reads, `OpSliceView`):

    __ref_2: vector<integer> = null;
    __ref_2 = {#inline_container: vector<integer>["__ref_p2_1"]
      __ref_p2_1: vector<integer>["__ref_1"] = n_nums(5, __ref_1);
      __ref_p2_1;
    };
    _slice_len_1 = OpLengthVector(__ref_2);
    …the bounds, `_next_1`, `_index_1` as for a local…

- `__ref_1` is the call's hidden buffer, declared at the function's top as any call's is.
  `__ref_2` is declared `= null` IN the statement, and the `__ref_p2_1` inside the block is a
  second-pass work ref.
- Where the `= null` and the assignment stand, by context:

| context | `__ref_2 = null` | the `inline_container` assignment |
|---|---|---|
| `s = f()[a..b]` | after the destination's `OpDatabase` / `OpGetField` / `OpSetInt4` | directly after it |
| `for x in f()[a..b]` | before the `For block` | first in the block's preamble, before `_slice_len_1` |
| `g(f()[a..b])`, `len(f()[a..b])` | before the statement that holds the call | first in the inner preamble of the `iter_vec` block, after that block's own `OpDatabase` triple |
| `y += f()[a..b]` | in the statement, after `y` is built | directly after it; then `OpSliceVector(y, __ref_2, lo, hi, 0)` |

- A NATIVE call (`"…".split('-')[1..4]`, `j.keys()[1..3]`) is held the same way.
- The result's type borrows the block's work ref where the elements are not plain scalars:
  `s: vector<ref(R)["__ref_p2_1"]>["__vdb_1"]`, `c: vector<text["__ref_p2_1"]>["__vdb_1"]`;
  of plain scalars it is `vector<integer>["__vdb_1"]`.
- `call_once` is the check that the call is not evaluated again for the length or per
  element: its callee counts its calls.

## 2. A field of records: read again, copied element by element

A field place is never held: `OpGetField(b, off, vt)` is written out again in the length and
in every element read.  With record elements the copy is the `Slice materialise` loop the port
already writes for a LOCAL of records (`local_recs_bind` agrees) — the subject expression is
the only difference:

    _slice_len_1 = OpLengthVector(OpGetField(b, off, vt));
    …
    loop {#Slice materialise
      _slice_elm_1: ref(R)["b"] = {#Vector Index: ref(R)
        OpGetVectorNullable(OpGetField(b, off, vt), <element size>, {#Iter range … });
      };
      _elm_1: ref(R)["__vdb_1"] = OpNewRecord(s, vt, 65535);
      OpCopyRecord(_slice_elm_1, OpGetField(_elm_1, 0, rt), rt);
      OpFinishRecord(s, _elm_1, vt, 65535);
    };

The result borrows the RECORD LOCAL through its elements: `s: vector<ref(R)["b"]>["__vdb_1"]`.
As an argument (`len(b.rs[1..4])`) the loop stands inside the `iter_vec` block, after the
inner preamble, and writes into `___iter_vec_1`.

## 3. Elements that are vectors

The same loop; three things differ from the record form (`nested_bind`):

    _slice_elm_1: vector<integer>["t"] = {#Vector Index: vector<integer>["__ref_1"]
      OpGetField(OpGetVectorNullable(t, 4, {#Iter range … }), 0, <inner vector type>);
    };
    _elm_2: vector<integer>["__vdb_2"] = OpNewRecord(ts, vt, 65535);
    OpCopyRecord(_slice_elm_1, _elm_2, <inner vector type>);
    OpFinishRecord(ts, _elm_2, vt, 65535);

- the element is read THROUGH `OpGetField(…, 0, <inner vector type>)`, and the `Vector Index`
  block keeps the element type's own dep (`["__ref_1"]`) while `_slice_elm_1` and the result
  borrow the subject (`["t"]`: `ts: vector<vector<integer>["t"]>["__vdb_2"]`);
- `OpCopyRecord` copies into the new element itself, typed as the INNER vector — no
  `OpGetField(_elm, 0, …)` around the destination;
- `_elm_N` takes the next free number (`_elm_2` where a literal above used `_elm_1`).

## Text elements

Carried for a local already; with a call subject the one addition to rule 1 is the element
holder's initialiser before the loop: `_slice_elm_1: text["__ref_p2_1"] = "";` then
`OpSetText(_elm_1, 0, _slice_elm_1)` in place of the copy (`native_bind`).

## Met on the way: a vector literal of vector-returning calls

`vv = [nums(2), nums(5)]` is carried by the port without a refusal and DIFFERS: Rust types the
local `vector<vector<integer>["__ref_1"]>["__vdb_1"]` (the element type borrows the first
call's buffer), the port `vector<vector<integer>>["__vdb_1"]`.  No corpus program agrees
through it today, so the gate does not show it; a literal of locals or of literals agrees.
