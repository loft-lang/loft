# @PLN91 — a `vector<u8>` field written from a vector local (prepared subject)

**The family:** "a vector field of this element type written from a vector place: TyInteger"
(10 sites; 3 programs held by it alone).  A struct literal fills a NARROW-integer vector field
from a vector local.  The three sole sites all write a `vector<u8>` field; two of them also
write a `vector<i32>` field in the same literal:
```
R-Base/a-byte-read-through-a-held-header-takes-its-base-at-the-read:26       Rb { data: d, out: [], tag: 5 }        (data: vector<u8>)
R-PushFill/a-loop-pushing-to-several-vectors-fills-each-on-its-own:43        ES { m: m, sf: sf }                    (vector<u8>, vector<i32>)
R-PushFill/vectors-built-side-by-side-each-push-through-a-window:50          E2 { w: w, mat: m, surf: sf, n: 0 }    (vector<u8>, vector<i32>)
```
The other seven are `bs` byte vectors put into a returned record (`Dec { value: VBytes { value: bs } … }`,
`D { v: bs, … }`) in programs refused elsewhere too.

The port refuses at `el_ex_place.loft` ~4794 when the element's row `rec` or the field's vector
type `vk` is negative.  `rec` comes from `schema_type(e, elem_name(inner))` unless
`four_byte_field_rows` gives the pair; that helper is the `i32` road.  For a `u8` element,
`elem_name` answers `"integer(0, 255)"`, and the likely failure is that no schema type carries
that name.  **Measure which of `rec` and `vk` is negative first.**

**The rule:** `@FR-B-Copy`: a vector local written into a record's field is copied in.  Its
elements are appended at the field's element row, the same narrow store type a push into
that field uses.

## Rust's form, measured on try 629ec5300

```
R-PushFill:43   struct ES { m: vector<u8>, sf: vector<i32> }   … ES { m: m, sf: sf }
  [43] {#Object: ref(ES)["__retbuf"]
         if if OpRefIsNull(__retbuf) false else OpConvBoolFromRef(__retbuf) null else OpDatabase(__retbuf, 104);
         OpSetInt4(__retbuf, 0, 0);  OpSetInt4(__retbuf, 4, 0);
         OpAppendVector(OpGetField(__retbuf, 0, 100), m, 30);       <- vector<u8>: field type 100, element row 30 (byte)
         OpAppendVector(OpGetField(__retbuf, 4, 102), sf, 101);     <- vector<i32>: field type 102, element row 101
         __retbuf }

R-Base:26   struct Rb { data: vector<u8>, … }
  OpAppendVector(OpGetField(__retbuf, 8, 100), d, 30);
```
Row 30 is the byte store type that Rust's `Database::byte(0, false)` names.  It is the row a
`u8` element is stored at, not a schema type of the element's name.

## What to code

At ~4789, for a narrow integer element (`narrow_width(spec) > 0`), take `rec` from the element's
narrow store type: `el_schema`'s `int_part` road, which `store_type` already takes for a narrow
integer (el_schema ~651).  Take `vk` from the field's vector type, as for any element.  Check
the `i32` field beside it: `four_byte_field_rows` should already give `(102, 101)`.

## Each form judged

- **The element row** (row 30, the narrow store type): a RULE, `@FR-B-Copy`.  The elements are
  stored at the field's own width.
- **The work-buffer rank** the porter met next (`pooled_work_buffer`, src/parser/call_args.rs
  ~936): an ACCIDENT.  The pool's key prints `forced_size`, but its rank counts by `==`, which
  ignores it, so `empty(a: vector<integer>, b: vector<i32>)` gives `b` rank 1 and a fresh
  `__ref_N`.  Only an internal number changes (@C168).  Buffers of different widths never share
  a key either way.  It may be carried.
- **`_range_end_N` typed as its bound** (`integer(0, 255)` for a `u8` bound): a RULE.  The bound
  keeps its type.

## Probes (the three sole programs; each asserts its vectors by hand)

```
kr-suite/rules/rewrites/R-Base/a-byte-read-through-a-held-header-takes-its-base-at-the-read.loft:26
kr-suite/rules/rewrites/R-PushFill/a-loop-pushing-to-several-vectors-fills-each-on-its-own.loft:43
kr-suite/rules/rewrites/R-PushFill/vectors-built-side-by-side-each-push-through-a-window.loft:50
```
