# @PLN91 — calling the function value a call answers, `f(g)(x)` (prepared subject)

**The family:** "a call that is not of a function by its bare name" (42 programs at try
40e69d190, 3 held by it alone).  This page takes its `SCall` part: a call whose callee is
itself a CALL that answers a function value (`pass2512(fn(x: integer) { … })(2)`).  That
covers 2 of the 3 sole programs.  The port refuses at `el_ex_object.loft` ~1718, just after
its arm for an ELEMENT callee (`fs[1](4)`, ~1703).

**The rule:** `@FR-L-CapKeep` / `@FR-L-CapOwn`: a function value answered by a call is called
through a hidden fn-ref temporary that owns nothing.  The closure record built for the
argument is released once, by its own owner, and never through the temporary.

## Rust's form, measured on try 40e69d190

```
L-CapKeep/2512-a-lambda-the-callee-hands-back-is-released-once.loft:49
  fn pass2512(f: fn(integer)) -> fn(integer) { f }
  pass2512(fn(x: integer) { seen[x] += 1; })(2);
     [49] {#fn_call_tmp
        ___fn_ref_tmp_1: function([integer], void, deps { items: [] }) = n_pass2512({#fn_ref_with_closure
            ___clos_2: ref(__closure_…_1) = null;  OpDatabase(___clos_2, 105);
            OpSetDbRef(___clos_2, 0, seen);  FnRef(887, ___clos_2) });
        fn_ref[4](2);                                    <- a CallRef through the temp (var 4)
     }
  variable table:  ___fn_ref_tmp_1  fn  skipfree        ___clos_2  ref(888)  OWNS
```

The same form appears inside a loop body (L-CapOwn/…confined-to-a-loop-pass…:229), with one
`fn_call_tmp` block per pass.

## What to code

At el_ex_object ~1703, the element-callee arm already does exactly this: `create_unique(vars,
"__fn_ref_tmp", <fn type, no deps>)`, `set_defined`, `call_ref` through it, `skip_free`, and a
`fn_call_tmp` block of `Set(tmp, <callee value>)` then the CallRef.  Let it take an `SCall`
callee too: elaborate the call, and if its type's base is `TyFunction`, run the same arm.  The
callee's argument (the inline closure) is elaborated as any fn-ref argument is.

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/closures/L-CapKeep/2512-a-lambda-the-callee-hands-back-is-released-once.loft:49
kr-suite/rules/closures/L-CapOwn/a-closure-confined-to-a-loop-pass-borrows-what-it-captured.loft:229
```

Not on this page: the third sole program's `SIf` site (a-leaf-early-exit-is-decided-toward-its-
access.loft:42), and the family's 113 sites of "a local of type TyUnknown" (not measured here).
