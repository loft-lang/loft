# @PLN91 — `f#read(n) as vector<T>` of a `single` or a narrow integer (prepared subject)

**The family:** "a file read of a vector with no size, or of this element type" (6 sites; 3
programs held by it alone):
```
158-scalar-file-read.loft:114            chunk = f#read(4) as vector<u8>;                    (TyInteger, sole)
899-unbound-file-read-only-vector:25     n = len(f#read(8) as vector<single>);               (TySingle, sole)
13-file.loft:149                         floats = f#read(n * sizeof(single)) as vector<single>;  (TySingle, sole)
899-unbound-file-read-temp :41 :50 :80   … as vector<single>, and :80 `as vector<P>` (a record element)
```
The port's `file_read` (el_ex_null ~536) has the road: a fresh `_read_N` local and `OpReadFile`
by the vector's store row.  But it guards the road with four conditions:
- a size is given;
- `plain_scalar(inner)` holds;
- the element is not a plain enum;
- **`main_vector<{nm}>` is already defined.**

`single` and `u8` pass the first three: `plain_scalar` takes `TySingle` and every
`TyInteger`, and `elem_name` names both.

**The likely cause is the fourth condition, which Rust does not have.** A `main_vector<T>` is
the store definition a vector LOCAL's `__vdb` needs.  The read's `_read_N` is set to null and
filled by `OpReadFile`; it gets no `__vdb`.  None of the three sole programs declares a
`vector<single>` or `vector<u8>` local any other way, so no `main_vector` exists to satisfy the
check.  (Not measured by patching the port: confirm by removing the check first.)

## Rust's form

`Parser::file_op` (src/parser/objects.rs ~2045): with `as T`, the row is `self.get_type(&tp)`,
the vector type's own definition.  No store definition is made or required:
```
158:114   chunk = f#read(4) as vector<u8>;
  [114] chunk: vector<integer(0, 255)> = {#reading file: vector<integer(0, 255)>
           _read_1: vector<integer(0, 255)> = null;
           OpReadFile(f, OpCreateStack(_read_1), 4, 102);              <- 102 = vector<u8>'s type
           _read_1 }

13:149    floats = f#read(n * sizeof(single)) as vector<single>;
  [149] floats: vector<single> = {#reading file: vector<single>
           _read_5: vector<single> = null;
           OpReadFile(f, OpCreateStack(_read_5), OpMulInt(n, 4), 103);  <- 103 = vector<single>'s type
           _read_5 }
```
(Measured on try 4f83c4f88.)

## What to code

In `file_read`'s vector road, drop the `main_vector<{nm}>` condition.  Take the row as the
vector type's own definition (Rust's `get_type`), declaring `vector<T>` if it does not exist yet,
as a type mention does.  Keep the size condition and the plain-scalar condition.  The record
element (`as vector<P>`, 899-temp :80) stays out of this page.

## Probes (the three sole programs; each writes a file, reads it back and asserts)

```
kr-try/tests/scripts/158-scalar-file-read.loft:114
kr-try/tests/scripts/899-unbound-file-read-only-vector.loft:25
kr-try/tests/docs/13-file.loft:149
```
