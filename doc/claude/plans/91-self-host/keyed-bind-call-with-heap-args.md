# @PLN91 — a keyed local bound from a call with a record or collection argument (prepared subject)

**The family:** "a keyed local bound from something other than a local" (21 sites in 16
programs at try b94e088a2).  Its largest part, and the 3 programs it holds alone, is ONE shape:
`s = f(args)` where `f` answers a keyed collection and an argument carries a store — a record
literal (`keyed(KRec { k: 2 })`, 157-mint-hoist), a vector literal (`mkset(["q", extra, "q"])`),
a record local (`mk_sorted(w)`, I-For/2573), a keyed local or field the callee may hand back
(`id(keep)`, `h = id(g)`, `kn_tail(kn_a)`, `om_h(b.h)`, `fr2562_param(a)`).  The refusal is
`el_ex_assign.loft:3760`; the comment above it already names the gap (`protectable_ref_args`).

Not this subject: an `if` whose arms are calls (`r = if c { g(n1) } else { g(n2) }`, 6 sites:
Rust's `join_source_frees`), a call through a function value (`x = m(i)`, 3 sites).

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native (leak check),
values hand-computed; a wrong expected value fails the run.  Probe:
[probes/keyed-bind-call-args.loft](probes/keyed-bind-call-args.loft), 12 cases.  Split one
function per case on b94e088a2: `arg_none`, `arg_scalar`, `arg_text_format` AGREE (the
`plain_args` path); the other 9 are refused here.

## Rust's form (expressions.rs ~8560–8740)

```
c = Ctr {}; s = mk_sorted(c)        s: sorted<P,…> = null;
                                    n_protect_store_frees(c);
                                    OpReplaceKeyed(n_mk_sorted(c), s, 32875);        <- kt | 0x8000
                                    n_unprotect_store_frees(c);
s = keyed(KRec { k: 2 })            __ref_p2_1: ref(KRec) = null;                     <- hoisted to the top
                                    s = null;
                                    n_protect_store_frees(__ref_p2_1);              <- the literal's pass-2 work ref
                                    OpReplaceKeyed(n_keyed({#Object … __ref_p2_1 = null; OpDatabase(__ref_p2_1, …) … }), s, 32875);
                                    n_unprotect_store_frees(__ref_p2_1);
s = mkset(["c", "a", "b", "a"])     s = null;
                                    OpReplaceKeyed(n_mkset({#Vector … _vec_1 = OpGetField(__vdb_1, …) … _vec_1 }), s, 32876);
                                    (NO bracket: `_vec_1` is bound to a value inside the argument)
g = mk0(); s = id(g)                n_protect_store_frees(g); OpReplaceKeyed(n_id(g), s, 32873); n_unprotect_store_frees(g);
s = id(b.s)                         n_protect_store_frees(b); OpReplaceKeyed(n_id(OpGetField(b, 0, 105)), s, 32873); n_unprotect_store_frees(b);
in a loop                           the same per pass; `__ref_p2_1` declared once at the function's top
```

## What to code

The `NdCall` arm of the keyed bind (el_ex_assign.loft ~3680–3762), for a loft call whose
arguments are not all scalars or text (and not the `self_only` shape, which stays as it is):

1. **The slots:** `protectable_ref_args(call).0` — each argument whose parameter type carries
   a store: a `NdVar` names itself; a derived view (`b.s`, `w[0]`, `o ?? q`) names its root
   slot (`arg_root_slots`); a `null` / `OpNullRefSentinel()` names nothing; a vector of plain
   scalars is skipped when the callee answers a record; anything else makes the set incomplete.
   Then drop every slot BOUND TO A VALUE inside the call (`bound_to_a_value_in`: a set whose
   value is not `null`): the vector literal's `_vec_1` goes, the record literal's
   `__ref_p2_1` (set only to `null` inside) stays.
2. **The bit:** `kt | 32768` when the callee may not hand back a caller's store OR every
   argument is covered (`call_return_frees_source` = `!may_return_a_borrow || covers_all`).
   Do NOT use `gives_away_its_store` here: it answers false for `id`, where Rust sets the bit
   (`arg_borrowed`: 32873 = 105 | 32768) and lets the bracket decide at run time.
3. **The statement:** `kg + [v = null, protect(slot)…, OpReplaceKeyed(call, v, bit), unprotect(slot)…]`
   in slot order; `v` takes no deps (`make_independent` on each).

Both functions exist already, ported for the bytecode writer over the finished bundle:
`binds.loft:1015` (`protectable_ref_args`, with `own::arg_root_slots`) and `binds.loft:1404` /
`:1422` (`may_return_a_borrow`, `call_return_frees_source`).  The elaborator's answer must be
the same slots and the same bit.

## The sites (try b94e088a2), this shape

```
kr-try/tests/scripts/157-mint-hoist.loft::n_test_mint_hoist:122                          (sole)
kr-try/tests/scripts/a-keyed-bind-from-a-call-on-a-vector-literal-compiles-natively.loft :27, :33   (sole)
kr-suite/rules/iteration/I-For/2573-a-for-evaluates-its-sorted-or-index-source-once.loft::n_main:44  (sole)
kr-suite/rules/calls/F-Ret/2562-…::n_test_fr2562_a_parameters_field_stays_the_callers:101
kr-suite/rules/heap/H-SwapIn/a-rebind-from-a-call-may-exchange-the-stores-….loft:84
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-….loft:286
kr-suite/rules/layout/L-Null/a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft:141, :286
kr-suite/rules/ownership/O-Complete/1142-a-keyed-return-through-a-join-frees-….loft:135
kr-suite/rules/ownership/O-Move/a-keyed-collection-returned-borrowed-is-the-callers-copy.loft:45
```
