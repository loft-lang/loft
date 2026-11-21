# @PLN91 — `f#read` with no cast reads the type its destination expects (prepared subject)

**The family:** "a variable attribute" (refusal reason `#read`, `#errors`, `#next`; 3 programs held
alone).  This page takes `#read`: 2 sole programs, 113-fread-inferred-size and 115-snapshot-
roundtrip.  The port's `file_read` (el_ex_null ~521) carries `f#read as T` and `f#read(n) as T`.
It refuses a read with NO cast by name.  Both sole sites read with no cast, and take the type
from where the value goes:
```
113:91   s.a = f#read;                                  (a field of a struct)
115:55   magic: u32 = (f#read & 0x7FFFFFFF) as u32;     (through an operator, into a typed local)
```

**The behaviour, as measured** (no formal rule names `#read`'s typing; formal/layout.md ~518 notes
only that `f#read` infers a record count from the same width): a value read with no cast is read
AS the type its position expects, by that type's width and store row, exactly as `f#read as
<that type>` would.

## Rust's form, measured on try afce67d53

```
113:91   s.a = f#read;                 (s.a an integer field)
  [91] OpSetInt4(s, 0, {#reading file: integer
          _read_9: integer = OpConvIntFromNull();
          OpReadFile(f, OpCreateStack(_read_9), 4, 102);      <- 4 bytes, the field type's row
          _read_9 })
  [92] OpSetByte(s, 6, 0, {#reading file: integer(0, 255)     <- the next field, a byte: its own width and row
          _read_10: integer(0, 255) = OpConvIntFromNull(); …

115:55   magic: u32 = (f#read & 0x7FFFFFFF) as u32;
  [55] magic: integer(0, 4294967295) = OpLandInt({#reading file: integer(0, 4294967295)
          _read_1: integer(0, 4294967295) = OpConvIntFromNull();
          OpReadFile(f, OpCreateStack(_read_1), 4, 100);
          _read_1 }, 2147483647)
```

The block is the same `#reading file` block `f#read as T` builds.  Only T comes from the
expected type, not from a cast.  In 115 the expected `u32` reaches the read through the `&`
and the outer `as u32`.

## What to code

In `file_read`, when there is no cast, take T from the expected type (`e.expect`, the
destination's type, passed down through an operator), and build the same block as for `as T`.
Refuse by name only where there is no expected type.

## Probes (the two sole programs; each writes a file, reads it back and asserts the values)

```
kr-try/tests/scripts/113-fread-inferred-size.loft:91 (and :92, a byte field)
kr-try/tests/scripts/115-snapshot-roundtrip.loft:55, :57
```

Not on this page: `x#errors` (1995-a-parsed-record-has-every-field-written, sole, and 14 sites in
other programs) and `c#next` / `ch#next` on a text iteration (02-text, 03-text).  Both are refused
by the same name at el_ex_null ~473 / ~672.
