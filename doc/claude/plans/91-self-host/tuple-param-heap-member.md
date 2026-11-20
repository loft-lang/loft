# @PLN91 — a by-value tuple parameter with a record member (prepared subject)

**The family:** "a tuple parameter with a heap member" (32 sites in 10 programs at try
ddab16d1b, 2 of them held by it alone: O-Oracle/2471, O-Oracle/2472).  The pattern is
`fn f(t: (P, integer))`, `(S, S)`, `(integer, S)`, `(S?, integer)`, `(text, P)`: a tuple
parameter, not a `&` one, with a record (or struct-enum) member.  The port refuses it at the
signature (el_definitions.loft ~1462), so the function and its callers drop.  A VECTOR member
is carried already, read through the parameter or copied into the `__tp_<name>` a write
promotes.  This subject extends that path to record members.  The `&(…)` parameter is
ref-tuple-param.md.

**The rule:** `@FR-F-ParamTuple`: a by-value tuple parameter is the callee's own value.  A
write through a member, whether a field, a compound or the member handed on and written, changes
the callee's copy and never the caller's tuple.  A callee that only reads copies nothing.

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check),
values hand-computed; a wrong expected value fails the run.  Probe:
[probes/tuple-param-heap.loft](probes/tuple-param-heap.loft), 15 callees, each checked by its
`chk_` caller.  On ddab16d1b every case with a record member is refused at the signature.
`write_scalar` agrees, and `rebind_member` meets another refusal ("a write to a tuple member
that is no plain scalar").

## Rust's form: a shadow `__tp_<name>`, made on pass 1, every member read through it

`definitions.rs:3656`: on PASS 1 every by-value parameter whose tuple carries a heap member gets
a shadow `__tp_<name>` (`add_variable`, `set_promoted_from`, `remap_name`), so both passes read
the body against it.  On pass 2 the shadow is seeded at entry, in one of two ways:

```
a callee that WRITES through a record member (write_field, compound, text_member, bump):
    __ref_p2_1: ref(P) = null;                                        <- hoisted, one per record member
    __tp_t: (ref(P)["__ref_p2_1"], integer) = ({#tuple_member_move:ref(P)["__ref_p2_1"]
        __ref_p2_1 = null;
        {#tuple_member_copy_fill  OpDatabase(__ref_p2_1, 100);  OpCopyRecord(t.0, __ref_p2_1, 100) };
        __ref_p2_1 }, t.1);                                           <- the record copied, a scalar as is
    t.0.x = 50          ->  OpSetInt(__tp_t.0, 0, 50)
    t.0.x + t.1         ->  OpAddInt(OpGetInt(__tp_t.0, 0), __tp_t.1)
    (text, P): t.0 += "!"   ->  OpClearText(__work_1); OpAppendText(__work_1, __tp_t.0); …; __tp_t.0 = __work_1
    bump: … t           ->  {#synthetic_tuple_return __ref_1 … OpCopyRecord(__tp_t.0, OpGetField(__ref_1, 0, 100), 100); OpSetInt(__ref_1, 8, __tp_t.1) }

a callee that only READS (read_field, read_two, pass_member, …):
    __ref_p2_1: ref(P) = null;                                        <- still declared
    __tp_t: (ref(P)["t"], integer) = t;                               <- an alias, borrowing `t`
    t.0.x + t.1         ->  OpAddInt(OpGetInt(__tp_t.0, 0), __tp_t.1)
```

Which seed applies is `passes::MemberWalk::only_read` (passes.rs:2453).  It decides copy unless
the finished body only reads: a write op, a `&` argument or a writing callee on a place inside a
member (or a local bound from one, to a fixed point), a callee pass 2 has not read yet, the
member yielded or returned in a tuple with a heap member, a closure or a call through a fn-ref
all mean copy.  A whole rebind (`t = …`) and a member rebind (`t.1 = …`) do not.  Every
unknown errs toward copying.

## What to code

At the refusal (el_definitions.loft ~1462): give a record or struct-enum member the treatment a
vector member has.  The `__tp_<name>` shadow is made on pass 1.  On pass 2 its seed is the
`#tuple_member_move` / `#tuple_member_copy_fill` copy per record member when `only_read`
answers false, and the alias `= t` (deps `["t"]`) when it answers true.  Each record member
gets its `__ref_p2_N`, declared in both cases.  Reads and writes of `t.k` go through the
shadow.  The port's vector-member `only_read` walk, if it has one, is the place to extend; if
not, mirror passes.rs:2453's list.

## The sites (try ddab16d1b)

```
kr-suite/rules/ownership/O-Oracle/2471-a-tuple-argument-is-witnessed-by-its-heap-member.loft: 26, 27      (sole)
kr-suite/rules/ownership/O-Oracle/2472-a-tuple-argument-with-several-heap-members-is-witnessed-by-each.loft: 23, 24   (sole)
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft: 28–46, 95 (13 sites)
kr-try/tests/scripts/1512-a-tuple-argument-call-member-releases-once.loft: 60, 62, 63, 64
kr-suite/rules/ownership/O-Move/a-member-of-a-tuple-argument-handed-back-is-the-callers.loft: 28, 29, 30
kr-suite/rules/matching/P-Cap/a-capture-and-a-plain-struct-stand-at-every-pattern-position.loft: 15, 17, 18
kr-suite/rules/binding/B-Ref-Lvalue/a-tuple-member-is-a-place-a-link-names.loft: 172, 173
kr-suite/rules/matching/P-Guard/a-guard-is-asked-of-each-listed-pattern.loft: 29
kr-suite/rules/matching/P-Point/a-struct-point-pattern-reads-any-declaration-order-and-a-null-field.loft: 58
kr-suite/rules/ownership/O-Witness/a-mixed-local-viewing-a-tuple-member-releases-through-its-witness.loft: 58
```
