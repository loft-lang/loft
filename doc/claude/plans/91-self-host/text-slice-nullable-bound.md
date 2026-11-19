# @PLN91 — a text slice with a nullable bound (prepared subject)

**The family:** "a text slice with a nullable bound" (16 sites in 6 programs at try b94e088a2,
3 of them held by this refusal alone): `t[1..maybe(0)]`, `line[at + size(marker)..]` where
`at = find(…)` is `integer?`, `report[pos + len(marker)..len(report)]`, every open and inclusive
form.  The refusal is `el_ex_vector.loft:180`.

**The rule:** `(N-Store)` "a null index reads null" through `(N-Prop)`'s guard: a nullable bound
makes the slice `text?`, null where a bound is (loft#2306).  The operands still run left to
right, each once (`(E-Left)` × `(F-Args)`).

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native (leak check), values
hand-computed; the probe fails when one expected value is changed.  Probe:
[probes/text-slice-nullable-bound.loft](probes/text-slice-nullable-bound.loft), 23 cases, one
axis each.  Split one function per case (`kr-tools/subj/cases.sh FILE maybe,word,at,a1`): 21
cases are refused on b94e088a2; `elem_bound` (a non-null element bound) agrees; `min_ahead`
DIFFERS (see the second change below).

## Rust's form

`text_slice_type` (fields.rs:3302) hands the finished `OpGetTextSub(text, lo, hi)` to the
general `wrap_null_transparent` (calls.rs:1080) with the argument types `[text, lo_t, hi_t]` and
the answer `text` WITHOUT deps; pass 1 answers `text?` and wraps nothing.

```
t[maybe(1)..maybe(3)]       {#null_transparent:text?
                              __ntp_1: integer? = n_maybe(1);           <- each nullable bound held, left to right
                              __ntp_2: integer? = n_maybe(3);
                              if OpConvBoolFromInt(__ntp_1) if OpConvBoolFromInt(__ntp_2)
                                  OpGetTextSub(t, __ntp_1, __ntp_2) else OpConvTextFromNull()
                              else OpConvTextFromNull() }               <- tests nested, the FIRST hold outermost
t[1..=maybe(2)]               __ntp_1: integer? = OpAddInt(n_maybe(2), 1);   <- the inclusive +1 is inside the hold
t[maybe(2)..=3]               … OpGetTextSub(t, __ntp_1, OpAddInt(3, 1))     <- a plain bound stays in the call
t[maybe(3)..] / t[..maybe(0)] … OpGetTextSub(t, __ntp_1, 2147483647) / (t, 0, __ntp_1)  <- the open side's literal stays
d.body[maybe(5)..]            __work_p2_1: text = OpGetText(d, 0);       <- a text that is no variable, AHEAD of a
word(c)[at(c, 2)..]           __work_p2_1: text = n_word(c, …);             nullable bound: held FIRST, in a pass-2
                              __ntp_1: integer? = …;                        work text (`work_text_p2`)
                              … OpGetTextSub(__work_p2_1, __ntp_1, 2147483647)
```

The consumers need nothing new: `== null`, `??`, `len`, a bind (`rest: text = {#null_transparent…}`),
`+=`, a format and a loop body all take the block as they take any `text?`.

## What to code

1. **`text_slice` (el_ex_vector.loft:~170):** drop the refusal; record each bound's type (an
   open bound is `TyInteger`); after `OpGetTextSub` is built (with the inclusive `OpAddInt`
   already in its bound), when either bound type is `TyOptional`: on pass 1 answer
   `TyOptional { TyText }` (no deps) and the plain code; on pass 2 call
   `el_ex_object::null_transparent(e, vars, <the unspanned call>, [x.tp, lo_t, hi_t], TyText{no deps}, <spanned>)`.
   A slice with no nullable bound keeps today's form and deps.
2. **`null_transparent` (el_ex_object.loft:2530), shared with `min`/`max`/`abs`/…:** carry
   `(F-Args)`'s hoist, which it lacks today.  Let `last` = the index of the last `TyOptional`
   argument.  An argument BEFORE `last` that is not `TyOptional` and does not read alike
   anywhere (anything but a variable or a literal: `NdVar`, null, int, long, float, single,
   boolean, text, enum literal, under a span) is held too, in its place in the order: a TEXT in
   `el_vars::work_text_p2(vars)` (a function-scope pass-2 work text: the slice may view it), any
   other type in `create_unique(vars, "_ntp", t)`.  It is held but NOT tested.  This is a
   latent differ today: `min(a1(c), at(c, 2))` answers `t_7integer_min(n_a1(c), __ntp_1)` in
   the port and `__ntp_1 = n_a1(c); __ntp_2 = n_at(c, 2); … min(__ntp_1, __ntp_2)` in Rust
   (case `min_ahead`); `min(k, maybe(3))` with a local `k` agrees (`min_var_ahead`).

Leave alone: a non-null bound (`elem_bound`), and the vector slice's nullable bound — that is
`vector_slice`'s own refusal ("a slice of a vector that is no local of plain scalars"), with
another Rust form (N-Store's `vec_null_*` cases).

## The sites (try b94e088a2)

```
kr-try/tests/scripts/03-text.loft::n_main:115
kr-try/tests/scripts/1011-tooling-contracts.loft::n_field1011:28
kr-try/tests/scripts/124-vector-amortised-growth.loft::n_free_blocks_digits:23
kr-try/tests/scripts/1357-every-text-buffer-a-frame-mints-is-released.loft::n_field:91
kr-suite/rules/operational/E-Left/a-null-transparent-call-evaluates-its-arguments-in-order.loft  (n_test_a_text_slice_reads_its_text_first:27, n_test_in_a_loop:37)
kr-suite/rules/types/N-Store/a-null-slice-bound-reads-null.loft  (10 sites, lines 29–47)
```
