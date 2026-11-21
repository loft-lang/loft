# @PLN91 — a comprehension over `rev(…)` (prepared subject)

**The family:** "an expression form", reason `SIterWrap`.  One program is held by it alone,
I-Comp/2345-a-comprehension-over-rev-stops-at-the-first-element.  Its first refusals are :22
(`a = [for x in rev(v) { x }]`) and :37 (`t = [for r in rev(rs) { r.s }]`).

The port's `for` STATEMENT carries `rev(src)`: el_ex_match ~374 unwraps the `SIterWrap`, sets
`rev`, and walks the inner source.  `comprehension_into` (~1194) does not unwrap it.  It
elaborates its source as an expression (`el_ex_assign::expression`, ~1243), and the expression
dispatch refuses `SIterWrap` as a form (el_ex_assign ~714).

**The rule:** `@FR-I-Comp`: `[for x in src { e }]` pushes `e` for each element the walk of
`src` yields, in the walk's order.  No formal rule names `rev(…)`'s walk; as measured, `rev(v)`
walks a vector from its last element down to its first.

## Rust's form, measured on try be25967a4

```
2345:22   v = [10, 20, 30];  a = [for x in rev(v) { x }];          (assert "[30,20,10]")
  [22] OpDatabase(__vdb_2, 20);  a: vector<integer>["__vdb_2"] = OpGetField(__vdb_2, 0, 7);  OpSetInt4(__vdb_2, 0, 0);
       _vector_1: vector<integer>["v"] = v;
       x#index: integer = OpLengthVector(_vector_1);                  <- starts at the LENGTH
       loop {#For comprehension
         x: integer = {#iter next
           x#index = {#rev step  if OpLeInt(1, x#index) OpMinInt(x#index, 1) else -2147483648 };
           OpGetInt(OpGetVectorNullable(_vector_1, 8, x#index), 0) };
         if OpLeInt(OpLengthVector(_vector_1), x#index) {#break break(0) } else null;   <- the forward exit, kept
         if OpLtInt(x#index, 0) {#break break(0) } else null;                           <- the reversed exit
         _comp_1: integer = {#block x };
         OpPushInt(a, _comp_1) }
```
The walk is the `for` statement's reversed walk over a vector local's `_vector_N`, as the
comprehension builds it for a forward walk.  The index starts at the length.  The `#rev step`
replaces the forward step.  Both exits are emitted, the forward one first.

## What to code

In `comprehension_into`, unwrap `SIterWrap { kind: "rev", e: inner }` as the `for` statement
does (~374): walk `inner`, and build the walk's index start, step and exits by the reversed
form above.  Refuse `unordered(…)` by name, as the statement does.  Try the reversed walk the
`for` statement already builds before writing a new one: the shapes should be the same.

## Probe (the sole program; every assert hand-computed)

```
kr-suite/rules/iteration/I-Comp/2345-a-comprehension-over-rev-stops-at-the-first-element.loft
   :22 [30,20,10]  :24 x#index [2,1,0]  :26 a filter [60,40]  :28 rev of a slice [20,10]  :31 an empty source
   :37 records ["b","a"]  :39 texts ["zz","yy","xx"]  :41 a literal vector of vectors [1,2]
```
Each later line is a cell of its own, so the program agrees whole only when every one is carried.
