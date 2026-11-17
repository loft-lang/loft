# @PLN91 — the scope pass in loft: the exit sweep for texts and references

The loft scope pass (`selfhost/src/scope.loft`) lays out functions of scalar locals only; every
other function is refused by name.  This plan ports the first slice of the frees: the sweep that
releases a block's texts and references where the block ends, and before a return.  Owner:
the tester (front-end-instruments.md's follow-up), behind the porter's tips in priority.

## The instrument — the scope gate

`selfhost/bench/scope_survey.py` compares the loft pass with the Rust one definition by
definition, over gate E's corpus, in under two minutes.  Its input is the Rust front end's
image before the scope pass (`LOFT_EMIT_PARSED`); its oracle is the image right after it:

`LOFT_EMIT_SCOPED=<out.store>` — write the program's image as `scopes::check` left it, before
the IR passes, and exit without running it (`src/main.rs`).  `--emit-image` is no oracle for the
scope pass: it is taken after the IR passes, whose inlining rewrites library calls.

A definition is `agree`, `unscoped` (the refusal names the kinds of local that need a free) or
`differ`; differences are ranked by the shape of their first differing line, with each
variable's scope number kept.  `--write-baseline` / `--check` pin and guard the agreeing
definitions (`selfhost/bench/scope_survey.baseline.tsv`).

**Measured at suite fac3652** (2026-11-17, after arguments were given scope 0): 14 of 2190
programs agree; 15991 of 35109 definitions agree, 17703 unscoped, 1415 differ.  The unscoped
ones by kind: reference alone 4020, collection + reference + text 3215, collection + reference
2776, reference + text 2503, text alone 2138 — this slice's kinds stand before 8661.

## The rules, as the Rust pass applies them

Read off six probes (a text local, a reference local, both, a nested block, an early return, a
loop), each diffed between the image before and after the pass, and checked against the Rust
sites named.

1. **A block frees its own locals where it ends**, in the reverse of the order they entered its
   scope: `OpFreeText` for a text, `OpFreeRef` for a reference.  A local belongs to the block it
   is first assigned in — the numbering the loft pass already does (`number`).  The order is the
   pass's `var_order` stack, popped (`free_vars.rs`, `get_free_vars`).
2. **The work texts are the body's**: the parser declares `__work_N` at the body's top, so they
   enter scope 1 first and are freed last, after the body's own locals, even where only a nested
   block writes them.
3. **A loop body's block frees its locals at its own end**, so once per pass.
4. **A return frees every enclosing scope down to the body**, innermost first, before it leaves.
   A returned value that reads a local about to be freed is bound first to `__ret_N`, a new local
   of the current scope (`free_vars.rs`, `ret_temp_counter`); an argument or a constant is
   returned as it stands.  The body's tail is such a return (`tail_return`, already ported).
5. **A frees list goes at the block's end**, after its last statement when the block is void;
   a block that answers a value keeps the value last.

## The slice — what is ported, what stays refused

Ported: rules 1–5 for text and reference locals (`TyText`, `TyReference`, a reference enum),
for functions that answer a scalar or nothing.

Still refused, by name, so nothing is approximated: a function answering a text or a reference
(the buffer delivery, the conditional `OpFreeRefIfDistinct`), a collection local, a `&`
variable local, a closure record, and a block whose value is a text or a reference.

## Prediction

Of the 8661 definitions whose kinds are only text and reference, those answering a scalar or
nothing and holding no closure: about half — **4000 ± 1500 move from unscoped to agree**; the
differences the gate then shows are the rules' missing cases, each a probe and a rule here.

## Built

**Step 1 + 2 (2026-11-17)** — `sweep` in `scope.loft`: rules 1–5, the block-end frees and the
return frees with `__ret_N`, for text and reference locals.  Refused by name where the slice
stops: a text or reference returned past a local to free, a block answering a value with a local
to free, a break or continue leaving one, a return inside an expression, a yield or parallel
block with one live.  The six probes match the Rust pass line for line.

Measured at suite fac3652, whole corpus, `--check` against the baseline: 0 definitions that
agreed stopped agreeing.  Programs agree 14 → 112; definitions agree 15991 → **18452 (+2461)**,
unscoped 17703 → 11525, differ 1415 → 5132.  The prediction (+4000 ± 1500) missed low: about a
third of the newly laid-out definitions differ — the rules' missing cases, ranked by the gate:
`OpFreeRefIfDistinct` where Rust frees a reference that may be the one handed out (245), the
closure records' drop functions (≈310), the conditional drop hook `t_1H_OpDrop` of a rebound
record (43), the reference's `OpDatabase` placement (35).  Each is the next probe and rule here.

Two parser defects met on the way: loft#2544 (fixed, an `is` test of a later-declared unit
variant), loft#2545 (filed: an `if` with a forward call and `[]` arms types void on pass 1 —
why `temp_variable` restates `el_vars::fresh_variable` rather than importing it).

## Steps, each measured by the gate

1. Lift the refusal for text and reference locals in functions of the slice; insert the block-end
   frees (rules 1–3, 5).  Measure.
2. The return frees and `__ret_N` (rule 4).  Measure.
3. Re-run the front layer: programs that now scope run end to end or name what stops them.
