# @PLN91 — a single-element alternation in a slice pattern (prepared subject)

**The family:** "a slice pattern element of this form: PGroup" (14 sites in 6 programs at try
e27ec2076, 3 of them held by it alone: 35f-alternation, generic-typevar-name-usable,
P-Alt-Same/the-pattern-capture-…).  The pattern is `[(A | B)]`, each branch one element wide:
`[(Ident { n } | Str { n })]`, `[(Ident | Str | Num)]`, `[(a: Num | b: Word)]`,
`[(a: Num | Word), ..r]`, `[(Rect { w: v } | Rect { h: v }), ..] if v > 5`.  The port carries
the MULTI-element alternation (`slice_alt`) and refuses this one in the fixed-element loop
(el_ex_match.loft ~3593/3603).

**The rules:** `@FR-P-Alt-Same` (a capture every branch binds takes the join of its types),
`@FR-P-Alt-Diff` (one bound by only some branches is `τ?`, null when another branch matched),
`@FR-G-Pat-Prec` (`a:V | b:W` captures the matched element).

**Judged right** on the interpreter (LOFT_POISON) and --native-c (leak check), values
hand-computed; a wrong expected value fails the run.  Probe:
[probes/slice-single-alternation.loft](probes/slice-single-alternation.loft), 11 cases.  On
a15f52718 every case is refused at the group.

## Rust's form (`parse_slice_alternation_element`, match_repetition.rs:1256)

With `E` = the element read `OpGetField(OpGetVector(xs, 16, i), 0, 100)` and `T(E)` its tag
`OpConvIntFromEnum(OpGetEnum(E, 0))`, both re-read at every use:

```
[(Ident { n } | Str { n })]       cond:  if <len == 1> (if OpEqInt(T(E), 1) true else OpEqInt(T(E), 2)) else false
                                  _mv_n_1: text = if OpEqInt(T(E), 1) OpGetText(E, 4)
                                                  else if OpEqInt(T(E), 2) OpGetText(E, 4) else OpConvTextFromNull()
[(Word { w } | Num { v })]        _mv_w_1: text?    = if T(E) == Word OpGetText(E, 4) else OpConvTextFromNull()
                                  _mv_v_1: integer? = if T(E) == Num  OpGetInt(E, 8)  else OpConvIntFromNull()
[(a: Num | b: Word)]              _mv_a_1: ref(Tok)["xs"]? = if T(E) == Num E else OpNullRefSentinel()     (a view of xs)
[(a: Num | Word), ..r]            cond's length test is `OpLeInt(1, len)`; the rest is the port's usual rest materialise
```

- The condition is the first branch's tag test, then `v_if(cond, true, next tag test)` per
  further branch, all under the slice's length test.
- One `_mv_<name>_N` per capture name, in first-appearance order across the branches.  The read
  is a fold from the LAST branch backwards (`acc = if T(E)==disc read else acc`), starting
  from the type's null, so it reads in branch order.
- Bound in every branch: the field's type.  Bound in only some: `τ?`.
- A heap capture other than text is a view: skip-free, deps on the subject (`["xs"]`).

## What to code

In the fixed-element loop, an element that is a `PGroup` whose every sequence is ONE element
wide: push the tag disjunction into the element's condition and the capture reads into the
binds, as above.  The capture unification is the multi-element path's own (`slice_alt`'s
P-Alt-Same / P-Alt-Diff), so reuse it.  A guard after the slice (`P-Guard`) and a wildcard
field (`f: _`) need nothing more.

A capture named like the matched SUBJECT, or like a local read after the match, is no trap
any more: the Rust parser now ends every alternation capture with its arm (loft#2591,
f406c3312, `@FR-B-Scope`).  The port's names follow the same rule.

## The sites (try e27ec2076)

```
kr-try/tests/scripts/35f-alternation.loft:20, 30, 35, 45                                    (sole)
kr-try/tests/scripts/generic-typevar-name-usable.loft:18                                    (sole)
kr-suite/rules/types/P-Alt-Same/the-pattern-capture-and-range-rules-answer-what-they-state.loft:33, 34   (sole)
kr-suite/rules/grammar/G-Pat-Prec/an-alternation-branch-may-capture-the-element-it-matched.loft:29–36
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft:107
kr-suite/rules/matching/P-Guard/a-guard-is-asked-of-each-listed-pattern.loft:31
```
