# @PLN91 — a narrow-integer vector field: written from a vector place, and its rows' order (prepared subject)

**The family:** "a vector field of this element type written from a vector place: TyInteger"
(10 sites in 9 programs at try f9a55db69, 3 of them held by it alone: R-Base/a-byte-read-…,
R-PushFill ×2).  The pattern is a struct literal whose `vector<u8>` (or `i8`, `u16`, `i16`) field
is given a vector local, a parameter or a call's result: `ES { m: m, sf: sf }`,
`return D { v: bs, next: a + n }`, `Rb { data: d, out: [], tag: 5 }`.  The refusal is
el_ex_place.loft:4571.

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check),
values hand-computed; a wrong expected value fails the run.  Probe:
[probes/vector-field-narrow.loft](probes/vector-field-narrow.loft), 10 cases.  On f9a55db69,
8 cases are refused, the `vector<integer>` control agrees, and `from_call` (`S { b: bytes(4) }`)
DIFFERS: by the row order of part 2, not by the call.

## 1. The field write: the element's content row

```
S { b: m, tag: 1 }  (b: vector<u8>)     OpAppendVector(OpGetField(__retbuf, 8, 100), m, 30)       <- `byte` (30)
S8 { b: m }         (b: vector<i8>)     OpAppendVector(OpGetField(…, 0, 102), m, 101)             <- byte<-128,false>
S { b: m }          (b: vector<integer>) … , m, 0)                                                 <- the control
```

**What to code:** the same one-line cause as vector-copy-narrow.md (09a179622).  `rec` is looked
up by NAME (`schema_type(e, "integer(0, 255)")`), which no row has; only the four-byte widths
take their rows from `four_byte_field_rows`.  A `TyInteger` element's row is
`el_schema::content_type(e, inner)` (el_ex_place.loft ~4562).

## 2. The rows' order: a struct before its narrow fields' rows (latent)

Rust lays out each struct's row FIRST, then the rows its narrow vector fields need (the
element's, then its `vector<…>`), struct by struct in declaration order.  The port makes every
1- and 2-byte element row before ANY struct.  The four-byte rows already agree.  Cells, one
`fn main() { a = A { }; … }` each:

```
struct A { b: vector<i8> }                    rust: A byte<-128> vector<byte<-128>> main_vector<…>
                                              port: byte<-128> vector<byte<-128>> A main_vector<…>
struct A { b: vector<u8> }                    rust: A vector<byte> …          port: vector<byte> A …
struct Z { n: integer }  struct A { b: vector<i8> }
                                              rust: Z A byte<-128> …          port: byte<-128> vector<…> Z A …
struct A { b: vector<i8>, c: vector<u16> }    rust: A byte<-128> vector<byte<-128>> short_raw<0> vector<short_raw<0>> …
struct A { b: vector<i8> }  struct B { c: vector<i8> }
                                              rust: A byte<-128> vector<byte<-128>> B …   (B's rows exist already)
struct A { b: vector<i32> }                   agrees
```

A row that already exists when the struct is laid out keeps its place: in the probe's split,
a function `bytes(n) -> vector<u8>` made `vector<byte>` before `S` in Rust too
(`100:vector<byte> 101:S 102:S8 103:byte<-128,false> …`).  That is the probe's `from_call`
differ, where the port gives `S` 107.  It is latent wherever a struct has a narrow vector field.
Of the 21 corpus programs with one, 14 are refused today, 4 agree (another use made the rows
first) and 1 differs.

**What to code:** where the port registers a struct field's narrow element rows (the
narrow-row registration from c70896b1a / e27ec2076, `int4_rows` and `vector_def` in
el_ex_base), a struct field's rows are made after the struct's own row, in field order.  The
four-byte path, which already agrees, is the model.

## The sites (try f9a55db69)

```
kr-suite/rules/rewrites/R-Base/a-byte-read-through-a-held-header-takes-its-base-at-the-read.loft:26     (sole)
kr-suite/rules/rewrites/R-PushFill/a-loop-pushing-to-several-vectors-fills-each-on-its-own.loft:43      (sole)
kr-suite/rules/rewrites/R-PushFill/vectors-built-side-by-side-each-push-through-a-window.loft:50        (sole)
kr-try/tests/scripts/174-foreign-view.loft:64
kr-try/tests/scripts/a-byte-range-of-a-vector-copied-one-at-a-time-is-one-append.loft:63
kr-try/tests/scripts/a-decoded-text-is-read-where-it-lies.loft:55
kr-try/tests/scripts/a-chain-exit-hands-the-placed-buffer-through.loft:299
kr-try/tests/scripts/a-decoded-value-is-read-at-its-position.loft:92
kr-suite/rules/rewrites/R-DecodeView/a-decoded-view-answers-what-its-copy-would-across-every-rule-it-meets.loft:178, 611
```
