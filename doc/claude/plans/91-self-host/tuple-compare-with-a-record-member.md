# @PLN91 — a tuple comparison with a record member (prepared subject)

**Status: DONE** — measured at 0a1a8b43c: its refusal no longer occurs.  Kept as the record of the subject.

**The family:** "a tuple comparison with a record member" (11 sites).  Two programs are held by
it alone: T-Cons/2055-a-tuple-comparison-with-a-struct-member-is-its-functions-value (9 sites)
and Op-Eq/a-user-op-eq-on-an-inner-type-changes-no-outer-comparison (:77).
```
2055:22   fn lit_right(p: P) -> boolean { (p, 1) == (P { hidden: 41 }, 1) }
2055:26   fn text_member(p: P) -> boolean { (p, "x") == (P { hidden: 41 }, "x") }
2055:28   fn not_equal(p: P) -> boolean { (p, 1) != (P { hidden: 41 }, 1) }
```
The port's `tuple_compare` (el_compare.loft ~41) refuses any operand whose tuple has a record
member.  Its comment says why: the member is held uncopied in `___cmp_l_N`, with a literal built
straight into the operand.

**The rule:** `@FR-E-Eq`: a struct compares by CONTENT.  A tuple compares member by member,
each member by its own type's operator (`Parser::tuple_compare`, src/parser/compare.rs ~109).
A comparison only reads, so an operand's record member may be a VIEW.

## Rust's form, measured on try c3f3e1328

```
2055:22   (p, 1) == (P { hidden: 41 }, 1)
  __ref_p2_1: ref(P) = null;                                         <- the literal's store, declared at the head
  [22] {#tuple_compare: boolean
         ___cmp_l_1: (ref(P)["p"], integer) = (p, 1);                <- the parameter held as a view, uncopied
         ___cmp_r_1: (ref(P), integer) = ({#Object __ref_p2_1 … OpDatabase(__ref_p2_1, 100); OpSetInt(__ref_p2_1, 0, 41); __ref_p2_1 }, 1);
         if OpEqContent(___cmp_l_1.0, ___cmp_r_1.0, 100) OpEqInt(___cmp_l_1.1, ___cmp_r_1.1) else false }

2055:26   (p, "x") == (…, "x")
         ___cmp_l_1: (ref(P)["p"], text["p"]) = (p, "x");            <- ONE dep list for the whole tuple
         … if OpEqContent(…0, …0, 100) OpEqText(…1, …1) else false

2055:28   !=
         if OpNeContent(___cmp_l_1.0, ___cmp_r_1.0, 100) true else OpNeInt(___cmp_l_1.1, ___cmp_r_1.1)
```
A record member compares with `OpEqContent` / `OpNeContent(<a>, <b>, <the struct's known type>)`.
`==` chains `if <eq> <next> else false`; `!=` chains `if <ne> true else <next>`.

## Each form judged

- **A record member compared by content (`OpEqContent` / `OpNeContent`, the known type):** a RULE,
  `@FR-E-Eq`.
- **The left operand's record member held as a view (`["p"]`, uncopied):** a RULE.  A comparison
  only reads.
- **The literal operand built in its own `__ref_p2_N`:** a RULE, a fresh record.
  `LOFT_LEAK_CHECK=1` passes 2055 on the interpreter and `--native-c` (exit 0), so the
  literal's store is released.
- **One dep list for the whole tuple** (`text["p"]` on the literal `"x"`): an ACCIDENT of tuple
  typing (a tuple carries one deps list); carry it.

## What to code

In `tuple_compare`, take a record member: compare it with `OpEqContent` / `OpNeContent(<l>.i,
<r>.i, <known type>)` in the chain.  Bind the operands as now, a record member uncopied and a
literal built apart, and lift the refusal at ~41.  The ordering operators (`<` …) over a record
member are not in these programs: keep refusing them by name until one is measured.

## Probes (the two sole programs)

```
kr-suite/rules/tuples/T-Cons/2055-a-tuple-comparison-with-a-struct-member-is-its-functions-value.loft:22–29, :41
kr-suite/rules/operators/Op-Eq/a-user-op-eq-on-an-inner-type-changes-no-outer-comparison.loft:77
```
