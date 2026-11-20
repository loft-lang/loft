# @PLN91 — a `&` tuple parameter (prepared subject)

**The family:** "a `&` parameter of a tuple" (13 sites in 13 programs at try e27ec2076, 2 of
them held by this refusal alone: tests/scripts/50-tuples, tests/docs/28-tuples).  Every site
is a signature: `fn swap(pair: &(integer, integer))`, `fn bump_pair(p: &(text, integer))`,
`fn via(p: &(u8, integer))`, `fn bump(t: &(integer, Entity))`.  The port refuses at line 0, so
the whole function and its callers drop.

**The rule:** `@FR-T-Ref-Rep` (`data::ref_tuple_is_record`): a `&(…)` whose every member a
struct field can hold (no `τ?`, no function, no nested tuple) names the `__tuple<…>` RECORD,
the representation a `&S` has.  A link to a tuple local, a vector element and a struct field
are then one mechanism (loft#1883).

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check), values
hand-computed; a wrong expected value fails the run.  Probe:
[probes/ref-tuple-param.loft](probes/ref-tuple-param.loft), 10 callees, each checked by its
`chk_` caller.  On e27ec2076 every case is refused at the signature.

## Rust's form

```
fn swap(p: &(integer, integer))     p: &ref(__tuple<integer,integer>)                 <- ref_var_type (passes.rs:358)
  t = p.0; p.0 = p.1; p.1 = t           t = OpGetInt(p, 0); OpSetInt(p, 0, OpGetInt(p, 8)); OpSetInt(p, 8, t)
fn whole(p) { p = (9, 8) }          {#ref_tuple_write  {#synthetic_tuple_return … __ref_1 … };  OpCopyRecord(__ref_1, p, 100) }
fn text_member(p: &(text, integer)) p: &ref(__tuple<text,integer>)    the record's own layout: integer at 0, text at 8
  p.0 += "!"                            _field_1 = OpGetText(p, 8); OpAppendText(_field_1, "!"); OpSetTextReplace(p, 8, _field_1)
fn forwarded(p) { fwd_inner(p) }    n_fwd_inner(p)                                    <- the link handed on as it is

the CALLER                          __ref_1: ref(__tuple<integer,integer>) = null;   <- hoisted
  x = (3, 7)                            x: ref(__tuple<integer,integer>)["__ref_1"] = {#synthetic_tuple_return
                                          __ref_1 = null; OpDatabase(__ref_1, 100); OpSetInt(__ref_1, 0, 3); OpSetInt(__ref_1, 8, 7); __ref_1 }
  swap(x)                               n_swap(OpCreateStack(x))
  x.0 * 10 + x.1                        OpAddInt(OpMulInt(OpGetInt(x, 0), 10), OpGetInt(x, 8))
```

## What to code

1. **The parameter type** (the refusal's site): a `&(…)` with every member recordable is
   `TyRefVar { TyReference { __tuple<…> } }`, its record made by the `__tuple<…>` builder the
   port already has (`el_types` tuple def, `boxed_tuple_return`).  A member access through it
   is a field read or write at the record's offset, as through a `&S`.  A member the record
   cannot hold stays refused (Rust refuses it too, `ref_tuple_record_element_ok`).
2. **The caller's tuple local is built as the record.**  Pass 1 records each tuple local
   handed to such a parameter (`ref_linked_tuple_locals`, convert.rs:1106).  A callee declared
   below its caller is judged after pass 1 (`record_forward_ref_tuple_links`, passes.rs:396).
   On pass 2 the local's `= (…)` bind builds the record through the return path's builder
   (expressions.rs:5785): a pass-2 work ref `__ref_N`, the `#synthetic_tuple_return` block, the
   local typed `ref(__tuple<…>)["__ref_N"]`.  Its reads are field reads.  The argument is
   `OpCreateStack(x)`.  A tuple local never handed to a `&` keeps its stack form.
3. **A whole write through the link** (`p = (9, 8)`): `ref_tuple_whole_write`
   (expressions.rs:4399) builds a fresh record and `OpCopyRecord`s it over the link's.

Leave alone: a `b = &a` link to a tuple local.  It records the local the same way (expressions.rs:6092)
but is its own family ("a `&` or `const` declaration, or one without a value").

## The sites (try e27ec2076)

```
kr-try/tests/scripts/50-tuples.loft:15                     swap(pair: &(integer, integer))   (sole)
kr-try/tests/docs/28-tuples.loft:46                        swap_ints(pair: &(integer, integer))   (sole)
…/one-place-named-through-ref-parameters-and-links-is-one-place.loft:124   tu(a: &(…), b: &(…))
…/a-stored-tuple-is-a-place-a-link-names.loft:22           swap(p: &(integer, integer))
…/a-tuple-member-is-a-place-a-link-names.loft:174          via_param(t: &(P, integer))
…/1673-a-tuple-link-is-read-and-written-whole-like-a-tuple.loft:34   arg_s(p: &(integer, integer))
…/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-….loft:79   bump_pair(p: &(text, integer))
…/a-link-is-rebound-and-hoisted-around-as-its-place-allows.loft:121       link_step(p: &(integer, integer), k)
…/1674-a-tuple-literal-copies-every-heap-member.loft:69    swap_through_link(p: &(vector<text>, vector<text>))
…/a-link-to-a-narrow-tuple-member-writes-the-member.loft:52               via(p: &(u8, integer))
…/a-link-to-a-text-tuple-member-writes-the-member.loft:40                 via(p: &(text, integer))
…/a-destructure-unpacks-a-reference-tuple.loft:43          scalar_pair(p: &(integer, integer))
…/a-tuple-member-is-typed-by-its-declaration-a-variant-one-….loft:116   bump(t: &(integer, Entity))
```
