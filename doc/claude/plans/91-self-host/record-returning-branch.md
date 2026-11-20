# @PLN91 — a dense record returned from a branch whose arms view, unwrap or recurse (prepared subject)

**The family:** "a function returning a record from something other than a literal" (25
programs at try cec149aab, 6 held by it alone).  This page takes its `if` part: 7 sites, 4 of
them in programs the family holds alone, and all four are DENSE returns (`-> D`, not `-> D?`).
The refusal is `el_expressions.loft` ~696, after every `if`-tail road declines.

**The rule:** `@FR-O-Buffer` / `@FR-B-View`: a dense record result answered by a branch
is the branch itself.  Each arm is lowered as it would be standing alone (a literal in a work
ref of its own, a view of a parameter as the view, a call with its own buffer), and the
function's return type carries every arm's deps.  Nothing is copied into `__retbuf` in the IR:
the caller-side delivery is the backends'.  (A NULLABLE result is another road,
`materialize_if_tail`: every view arm is copied into `__retbuf`, see H-CopySelf `pick`.)

## Rust's form (measured on try cec149aab)

```
2376 pick(h: H, c: boolean) -> D { if c { h.ds[0]! } else { D { items: [A { n: 55 }], tag: 5 } } }
     -> D["h"]
     if c {#block {#ncr  _ncr_1: ref(D)["h"] = OpGetVectorNullable(OpGetField(h, 0, 104), 16, 0);
                         if !_ncr_1 n_panic(…); _ncr_1 }}
     else {#block {#Object __ref_p2_1 … }}               <- the literal in a work ref, not __retbuf
978  i978_tail(b: I978Bag, fresh: boolean) -> I978Item
       { if fresh { I978Item { … } } else { b.items["one"] ?? I978Item { … } } }
     -> I978Item["b"]
     if fresh {#Object __ref_p2_1} else { … if OpConvBoolFromRef(__ncc_1) __ncc_1 else {#Object __ref_p2_2} }
F-Ret td(v: Nd) -> Nd { match v { NLeaf { n } => { if n == 7 { return mk(8); } v }, NBlk { ops } => { …; NBlk { ops: o2 } } … } }
     -> ref(Nd)["??"]
     if <tag 1> {#match_arm {#block if … {#block:never return n_mk(8, __ref_1)}; v }} else …
O-Derived classify(n: Node) -> Own   (arms: a literal, a join of two recursive calls, a block ending in `r = classify(…); r`)
     -> ref(Own)["??"]
     if <tag 1> {#match_arm {#Object __ref_p2_1}} else … {#block … r = n_classify({#ncc …}); r }
```

`["??"]` is the deps of a result whose arm is a call this pass has not typed yet (the recursion).

## Where the port declines

`if_tail_params` (el_expressions ~2798) is the road for "every arm hands back a parameter, a
literal in a work ref of its own, or the sentinel".  Its arms are: `NdIf`, a parameter `NdVar`,
the sentinel, a field or vector element of a parameter, an `Object` block, a `join-arm-owner`
block, a `scalar_match` block, and `call_arm_params`' single-call block.  The four functions
each need an arm kind it does not read:

| function | arm it cannot read | what the arm answers |
|---|---|---|
| 2376 `pick` | a plain `block` holding an `ncr` block (`h.ds[0]!`) | `_ncr_1`, bound to an element of parameter `h` |
| 978 `i978_tail` | `if OpConvBoolFromRef(__ncc_1) __ncc_1 else {#Object}` | `__ncc_1`, bound to a keyed lookup into parameter `b` |
| F-Ret `td` | `#match_arm` blocks (only `scalar_match` is looked through), a plain `block` ending in `v` after an early `return` | parameter `v`, a literal |
| O-Derived `classify` | `#match_arm`, a plain `block` ending in local `r` bound from `n_classify(…)` | a call's answer |

## What to code

Teach `if_tail_params` to read through, each to the value it delivers:
1. a plain `block` (and a `match_arm` / `match` wrapper, as `scalar_match` already is): its
   last operator, an early-`return` statement before it being no arm;
2. an `ncr` block (`x!`) and a `??` temp (`__ncc_N`): the temp's bind, so an element or field
   of a parameter answers that parameter;
3. a tail local bound once from a call in the arm's own block (`r = f(…); r`): the call, as
   `call_arm_params` reads a call arm.

Then derive the return type's deps as Rust does: the union of the parameters the arms borrow,
or `["??"]` where an arm's call is one pass 2 has not typed yet.

## Probes (the four sole programs; each is a test with hand-computed answers)

```
kr-suite/rules/ownership/O-Deps/2376-an-element-read-off-a-borrowing-calls-field-names-no-callee-dep.loft:40   n_pick
kr-try/tests/scripts/978-branch-join-carries-both-arms-borrows.loft:132                                        n_i978_tail
kr-suite/rules/calls/F-Ret/a-recursive-call-after-an-early-fresh-return-may-still-answer-its-argument.loft:52  n_td
kr-suite/rules/ownership/O-Derived/a-call-argument-a-recursive-join-answers-is-not-freed-before-the-return.loft:84   n_classify
```

`probe.py FILE --tools <try> --show <fn>` prints Rust's form and the port's diff per function.
