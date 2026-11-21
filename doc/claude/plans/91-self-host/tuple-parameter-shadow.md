# @PLN91 — a by-value tuple parameter with a heap member: the `__tp_<name>` shadow (prepared subject)

**The family:** "a tuple parameter with a heap member": 31 sites in 10 programs, 2 held by it
alone (O-Oracle/2471-a-tuple-argument-is-witnessed-by-its-heap-member,
2472-a-tuple-argument-with-several-heap-members-is-witnessed-by-each).  Every function with a
parameter like `p: (R, integer)` is refused at el_definitions.loft ~1471.  The most sites are in
F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value (14).  The pattern programs
(tuple patterns over `(Pt, integer)`) and 1512 hold the rest.

**The rule:** `@FR-F-ParamTuple`: a by-value tuple parameter with a heap member is the CALLEE's
own value.  It gets an owned shadow, `__tp_<name>`, and every mention of the parameter reads
the shadow.  The shadow is seeded at entry with a COPY of the heap members.  Where the finished
body only READS the shadow, it is seeded with the parameter itself instead, typed as a view
(`passes::MemberWalk::only_read`), so the frame frees nothing the caller owns.
(`LOFT_NO_TUPLE_PARAM_COPY=1` turns the rule off in Rust.)

## Rust's form

- **The shadow, first pass:** src/parser/definitions.rs ~3655–3690.  At the start of PASS 1, for
  each argument that is not `__`-named, is not a `RefVar` and has `tuple_carries_heap(tp)`: a
  variable `__tp_<name>` of the parameter's type, `set_promoted_from(shadow, v)`, the
  const/value-const marks copied, and `remap_name(name, shadow)`, so the name resolves to the
  shadow from then on.
- **The seed:** src/parser/expressions.rs ~10126–10170 (`promoted_arg_seed`, `tuple_param_seed`).
  If `MemberWalk::only_read(body)` holds, with a tuple RESULT counted as an escape, the shadow
  is typed `tuple_view_of(param)`: each non-scalar member `["<param>"]`.  The seed is then the
  plain `__tp_p = p`.  Otherwise the seed is the per-member copy.

Measured on try 4667b05fd:
```
F-ParamTuple:42   fn c_read(t: (P, integer)) -> integer { t.0.x * 10 + t.1 }          (only reads: the VIEW seed)
  __ref_p2_1: ref(P) = null;                                    <- minted for the copy road, unused here
  __tp_t: (ref(P)["t"], integer) = t;
  [42] OpAddInt(OpMulInt(OpGetInt(__tp_t.0, 0), 10), __tp_t.1)

F-ParamTuple:28   fn c_field(t: (P, integer)) { t.0.x = 50; }                          (writes: the COPY seed)
  __ref_p2_1: ref(P) = null;
  __tp_t: (ref(P)["__ref_p2_1"], integer) = ({#tuple_member_move: ref(P)["__ref_p2_1"]
          __ref_p2_1 = null;
          {#tuple_member_copy_fill: void  OpDatabase(__ref_p2_1, 100);  OpCopyRecord(t.0, __ref_p2_1, 100) };
          __ref_p2_1 }, t.1);
  [28] OpSetInt(__tp_t.0, 0, 50)

2471:26   fn pick(p: (R, integer)) -> R { if p.1 > 0 { p.0 } else { R { w: 0 } } }   (only reads; the answer copied out)
  __ref_p2_2 = null;  __ref_p2_1 = null;                        <- __ref_p2_1 the copy road's, unused
  __tp_p: (ref(R)["p"], integer) = p;
  [26] return {#materialized_view_return  __retbuf = null; OpDatabase(__retbuf, 100);
               OpCopyRecord(if OpLtInt(0, __tp_p.1) {#block: ref(R)["__tp_p"]  __tp_p.0 }
                            else {#block: ref(R)  {#Object __ref_p2_2 … OpSetInt(__ref_p2_2, 0, 0) … } }, __retbuf, 100);
               __retbuf }
```

## Each form judged

- **The shadow `__tp_<name>`, every mention remapped to it:** a RULE (`@FR-F-ParamTuple`).
- **The copy seed (`tuple_member_move` / `tuple_member_copy_fill`) where the body writes or lets
  a member escape; the view seed where it only reads:** a RULE.  The callee's writes never
  reach the caller, and a read-only body costs no copy.
- **The copy road's `__ref_p2_N` minted even when the view seed wins (unused in `c_read`, `pick`):**
  an ACCIDENT of internal numbers (@C168).  The copy is built on pass 1 before the walk decides;
  carry it.
- **A member answered through a record result** (2471 `pick`): copied out into `__retbuf` by the
  existing `materialized_view_return`; nothing new.

## What to code

1. On pass 1, add the shadow for each qualifying parameter and remap its name.  Build the copy
   seed's members then, so that `__ref_p2_N` is minted in Rust's order.
2. On pass 2, after the body: if it only reads the shadow (port `MemberWalk::only_read`, src/parser/passes.rs
   ~2453, with a tuple result counted as an escape), type the shadow as the view and seed
   `__tp_p = p`; otherwise seed the copy.

Start with the read-only programs (2471 `pick`/`pick_b`, 2472, `c_read`), then the writers.

## Probes

```
kr-suite/rules/ownership/O-Oracle/2471-a-tuple-argument-is-witnessed-by-its-heap-member.loft:26, :27          (sole)
kr-suite/rules/ownership/O-Oracle/2472-a-tuple-argument-with-several-heap-members-is-witnessed-by-each.loft:23, :24   (sole)
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft:28–46   (the writers; 14 sites)
```
