# @PLN91 — a rule (a cursor match) whose arm answers a view of its parameter (prepared subject)

**The family:** "a function returning a record from something other than a literal" (the
`block cursor match` reason).  One program is held by it alone, P-Rule/a-sub-rule-run-releases-
its-answers-and-never-the-subjects (`n_view` :33, `n_mixed` :34).  A rule `fn view(p: P) -> N? {
match p { [Num { v }] => p.nodes[0], _ => null } }` answers an element of its PARAMETER's
field, and `mixed` answers that view in one arm of an `if` and a fresh literal in the other.

The port's record-return selector (el_expressions.loft ~676) takes a cursor-match tail only when
`cursor_arms_answer` holds: every arm is a literal built in its own work ref, or the null
sentinel.  An arm that is a view of a parameter, or an `if` with such an arm, falls through to the
refusal at ~713.

**The rule:** `@FR-O-Buffer`, its view half: a record answer that is a VIEW of a parameter is
handed back as that view, typed with the parameter as its dependency (`N["p"]?`).  It is not
copied into the buffer, and the caller keeps it alive through `p` (the program's `borrowed()`
checks that `d.nodes` is untouched: `"14 7 1"`).

## Rust's forms, measured on try b55f97520

```
:31  fn rec(p: P) -> N? { match p { [Num { v }] => N { v: v }, _ => null } }       (carried: the reference)
   -> N?        … {#slice_binding … {#Object __ref_p2_1 … } } else OpNullRefSentinel()

:33  fn view(p: P) -> N? { match p { [Num { v }] => p.nodes[0], _ => null } }
   __retbuf: N hidden    -> N["p"]?                                    <- typed by the PARAMETER
   {#block: ref(N)["p"]?  [33] {#cursor match: ref(N)["p"]?  … {#vector_match: ref(N)["p"]?
      if <Num at pos> {#slice_binding: ref(N)["p"]
         _mv_v_1 = …;  OpSetInt(p, 0, OpAddInt(_cursor_pos_1, 1));
         OpGetVector(OpGetField(p, 12, 109), 8, 0) }                   <- the view, AS IT IS: no copy
      else OpNullRefSentinel() } } }

:34  fn mixed(p: P) -> N? { match p { [Num { v }] => if v > 4 { N { v: v } } else { p.nodes[0] }, _ => null } }
   -> N["p"]?
   … {#slice_binding: ref(N)["p"]
        if OpLtInt(4, _mv_v_1) {#block: ref(N)  {#Object __ref_p2_1 … OpSetInt(__ref_p2_1, 0, _mv_v_1) … } }
        else {#block: ref(N)["p"]  OpGetVector(OpGetField(p, 12, 109), 8, 0) } }
     else OpNullRefSentinel()
```

Nothing is copied into `__retbuf` in any arm, not even the literal in `mixed`.  The function
result, the body block, the cursor match, the vector match and the slice binding are all typed
`ref(N)["p"]?`.  `mixed`'s inner `if` keeps each arm's own type: `ref(N)` for the literal and
`ref(N)["p"]` for the view.

## What to code

Widen `cursor_arms_answer` to also accept, per arm:
- a view of a parameter: a field or element read rooted at a parameter (`OpGetVector(OpGetField(p,
  …), …)`);
- an `if` whose arms are each one of the accepted kinds (a literal in its own work ref, a view
  of a parameter, the sentinel).

When any arm views a parameter, type the result and every enclosing block as `ref(N)[<that
parameter>]` (with `?` for a nullable result), as Rust's `-> N["p"]?` shows.

## Probe (the sole program; main asserts each answer)

```
kr-suite/rules/matching/P-Rule/a-sub-rule-run-releases-its-answers-and-never-the-subjects.loft:33, :34
   borrowed() == "14 7 1" (the view leaves d.nodes whole), mixed_run() == "13 7"
```
