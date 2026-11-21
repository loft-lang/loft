# @PLN91 — a `-> vector<u8>` function whose tail is a number literal (prepared subject)

**The family:** "a vector answered at another element than the result names", reason
`integer for integer(0, 255)`: 3 programs held by it alone.  A function returning `vector<u8>`
answers a literal of plain numbers:
```
a-slice-handed-to-a-const-parameter-needs-no-store-of-its-own.loft:21   fn frame() -> vector<u8> { [104, 101, 108, …, 100] }
a-decoded-text-is-read-where-it-lies.loft:224                           fn op_frame() -> vector<u8> { [4, 3, 2, 2, 111, …] }
a-decoded-value-is-read-at-its-position.loft:291                        fn f1() -> vector<u8> { [4, 5, 2, 2, 111, …] }
```
The same form appears, not sole, in 2 more programs: a-value-read-and-dropped:186 and
R-DecodeView's `ping`.  T-Chk-Vec's float cases (`-> vector<float> { [1, 2] }`) refuse the same
way, as `integer for float`.  The port builds the tail literal at its own element (`integer`),
and `vector_return` (el_ex_deliver.loft ~48) then refuses the element mismatch.

**The rule:** `@FR-T-Chk-Vec`: `Γ ⊢ [e₁ … eₙ] ⇐ vector<τ>  ⟸  ∀i. Γ ⊢ eᵢ ⇐ τ`.  A literal in a
position that expects `vector<τ>` is checked AT `τ`: each item is built as a `τ`, never built
as an integer and converted afterwards.  The function's tail is such a position, and its result
names `τ`.

## Rust's form, measured on try e5266ff25

```
fn frame() -> vector<u8> { [104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100] }
  __vdb_1: vector<integer(0, 255)> hidden                 <- the returned vector, on the hidden buffer
  -> vector<integer(0, 255)>
  {#block: vector<integer(0, 255)>
    { !! INSERT  OpClearVector(__vdb_1) };
    {#Vector: vector<integer(0, 255)>["__vdb_1"]
      OpDatabase(__vdb_1, 101);                             <- the u8 vector's store type
      _vec_1: vector<integer(0, 255)>["__vdb_1"] = OpGetField(__vdb_1, 0, 100);
      OpSetInt4(__vdb_1, 0, 0);
      OpPreAllocVector(_vec_1, 11, 1);                      <- 11 items of ONE byte
      OpPushByte(_vec_1, 0, 104);  OpPushByte(_vec_1, 0, 101);  …  OpPushByte(_vec_1, 0, 100);
      _vec_1 } }
```

## Each form judged

- **Each item built as the result's element** (`OpPushByte`, a 1-byte pre-allocation): a RULE,
  `@FR-T-Chk-Vec`.
- **The literal delivered onto the hidden return buffer** (`__vdb_1`, `OpClearVector` first): the
  port's existing vector-return road.  Nothing new.

## What to code

Hand the result's element type down to a tail vector literal, as `hinted_argument` does for an
argument: build it with `el_ex_pattern::vector_value(e, vars, node, elem: <the result's
element>)`.  Then `vector_return`'s mismatch check no longer fires for it.  Check the float form
of T-Chk-Vec (`-> vector<float> { [1, 2] }`) against Rust before you rely on the same road: it
reaches the same refusal.

## Probes (the three sole programs)

```
kr-try/tests/scripts/a-slice-handed-to-a-const-parameter-needs-no-store-of-its-own.loft:21
kr-try/tests/scripts/a-decoded-text-is-read-where-it-lies.loft:224
kr-try/tests/scripts/a-decoded-value-is-read-at-its-position.loft:291
```
Not on this page: F-Return/2555 (`integer? for integer`), a `[h]` whose `h` is nullable through
an early `return`.
