# @PLN91 — a `for` over a struct with a user `next` operator (prepared subject)

**The family:** "a `for` over this type: TyReference": 3 programs.  81-iterator-protocol is
held by it alone; Op-Mark/1950b and a-library-type-answers-its-operators-through-a-bare-use are
not.  A struct with `operator next(self: S) -> T?` is walked by `for x in s`.  The port's `for`
(el_ex_match ~456) refuses a source whose type is a record reference.

**The rule:** the iterator protocol (`@FR-I-Next` / `@FR-I-Done` over a user `next`): each
step calls `next` on the walked object.  The loop ends at the first null the step answers.

## Rust's form, measured on try 0273bd941

```
81   struct Counter { current: integer, limit: integer }   operator next(self: Counter) -> integer? { … }
     c = new_counter(3);  total = 0;  for x in c { total = total + x; }
  [27] {#For block: void
         ___iter_obj_1: ref(Counter)["c"] = c;                <- the walked object, a view of the source
         loop {#For loop
           x: integer = t_7Counter_next(___iter_obj_1);        <- the operator's definition, called directly
           if OpEqInt(x, OpConvIntFromNull()) {#break break(0) } else null;
           {#block  total = OpAddInt(total, x) } } }

     for _ in c2 { count = count + 1; }                        <- a `_` binding: a fresh `___N` local
  [33] … ___iter_obj_2: ref(Counter)["c2"] = c2;
         loop { ___1: integer = t_7Counter_next(___iter_obj_2);  if OpEqInt(___1, OpConvIntFromNull()) break … }
```
The `___iter_obj_N` numbering runs per function (`_1`, `_2`, … in source order).  The null test
is the element type's null (`OpConvIntFromNull` for an integer, `OpConvTextFromNull` through
`OpEqText` for a text).

## Each form judged

- **The walked object in `___iter_obj_N` (a view of the source), `next` called each step, a break
  at the first null:** a RULE, the iterator protocol.
- **A `_` binding as a fresh `___N`:** the port's existing `_` road; nothing new.
- **NOT JUDGED (1950b, a text `next`):** the loop binding is typed with the variable its BODY
  writes it into as its dep.  `for line in l { last = line; … }` gives `line: text["last"]`, and
  `{ all += line }` gives `line: text["all"]`.  Its value comes from the `__work_c1` the call
  is handed, not from `last`.  Outside a loop the same text local carries no such dep (probe:
  `x = mk(l); last = x;` gives `x: text?`).  The direction looks reversed, so judge it under
  the scope oracle (scope_survey) before carrying it.  81 does not need it.

## What to code

In the `for` source dispatch (el_ex_match ~456): when the source is a record reference whose
struct declares `operator next` (Rust: `find_fn(u16::MAX, "next", <type>)`, src/parser/collections.rs
~926; `@FR-Op-Mark`: only an `operator next` drives a `for`, never a plain `fn next`), declare `___iter_obj_N` as a view
of the source.  Then emit the loop: bind the loop variable to the call, break on the element
type's null, then the body.  Start with 81 (integer).

## Probe (the sole program; every assert hand-computed)

```
kr-try/tests/scripts/81-iterator-protocol.loft:27, :32, :37   (0+1+2 = 3; an empty counter, 0 rounds; one round, 0)
```
