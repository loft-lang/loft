# @PLN91 — a record-answering lambda whose body is `<capture> ?? <literal>` (prepared subject)

**The family:** "a lambda answering a `??` over its capture" (5 sites, 2 programs held by it
alone):
```
O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once:137   capn: S1659? = …;  g = fn(i: integer) -> S1659? { capn ?? S1659 { v: 4.0 } }
a-function-value-fills-its-callers-record:48                      from_table = fn(id: integer) -> Sty { styles[id] ?? Sty { size: 0.0, weight: 0, italic: false } }
```
(158-guard-free :120/:121 and O-Latest/1324 :102 are the shared sites.)  The port refuses at
el_expressions.loft ~440, when `capture_tail_copy` does not take the tail and `ncc_over_capture`
holds.  Its comment names the blocker: Rust's body borrows the lambda's placeholder for the
capture, a variable the port's lambdas do not keep.

**The rule:** `@FR-O-Buffer`, the view half: the capture belongs to the frame that built the
closure.  So a lambda answering a record copies it into its buffer, whichever arm of the `??`
answers: the capture behind its null test, or the literal.

## Rust's forms, measured on try bc97fb998

```
1659:137   -> S1659?  (a NULLABLE result: the buffer stays __retbuf)
  i: integer;  __retbuf: S1659 hidden;  __closure: __closure_test_a_coalesced_capture_0      -> S1659["__retbuf"]?
  {#block: ref(S1659)?                                         <- the body: no capture dep
    __ncc_1: ref(S1659)["__closure"] = null;  __ref_p2_1: ref(S1659) = null;
    {#ncc: ref(S1659)["__retbuf"]
      __ncc_1 = OpGetDbRef(__closure, 0);                       <- the capture, read through the closure record
      if OpConvBoolFromRef(__ncc_1) {#materialized_view_return  __retbuf = null; OpDatabase(__retbuf, 101);
                                                                 OpCopyRecord(__ncc_1, __retbuf, 101); __retbuf }
      else {#materialized_view_return  __retbuf = null; OpDatabase(__retbuf, 101);
              OpCopyRecord({#Object __ref_p2_1 … OpSetFloat(__ref_p2_1, 0, 4f64) … __ref_p2_1 }, __retbuf, 101); __retbuf } } }

a-function-value:48   -> Sty  (a DENSE result: the buffer renamed onto a work ref)
  id: integer;  __ref_1: Sty hidden;  __closure: __closure_main_0      -> Sty["__ref_1"]
  {#block: ref(Sty)["styles"]                                  <- the body borrows the CAPTURE's name
    __ref_p2_1 = null;  __ncc_1: ref(Sty)["__closure"] = null;
    {#ncc: ref(Sty)["__ref_1"]
      __ncc_1 = OpGetVectorNullable(OpGetDbRef(__closure, 0), 24, id);   <- an element of the captured vector
      if OpConvBoolFromRef(__ncc_1) {#materialized_view_return … OpCopyRecord(__ncc_1, __ref_1, 100) … }
      else {#materialized_view_return … OpCopyRecord({#Object __ref_p2_1 … }, __ref_1, 100) … } } }
```

## Each form judged

- **Each arm materialised into the buffer (`OpCopyRecord` behind `OpConvBoolFromRef` for the
  capture, the literal built apart and copied):** a RULE, `@FR-O-Buffer`.  `LOFT_LEAK_CHECK=1`
  passes on both programs (interpreter, exit 0).
- **The buffer's name: `__retbuf` for a nullable result, a work ref `__ref_1` for a dense one,** as
  the dispatcher page found for struct enums.  An ACCIDENT of internal naming (@C168).  The
  dense case's name is minted on the first pass (see head-element-match-over-a-local.md);
  carry it as measured.
- **The dense body typed with the capture's name (`ref(Sty)["styles"]`):** the blocker the port
  names.  It is a dep on a name, so internal, but the port needs the capture's placeholder to
  write it.  Not a behaviour: carry it once the placeholder is kept.

## What to code

Start with 1659, the nullable result: there is no capture dep, and the buffer stays `__retbuf`.
At ~440, build the `#ncc` block over `OpGetDbRef(__closure, 0)` and materialise each arm into
the buffer, as above, instead of refusing.  Then the dense case: keep the lambda's placeholder
variable for the capture so that the body can be typed `[<capture>]`, and rename the buffer
onto its first-pass work ref.

## Probes (the two sole programs; both leak-checked)

```
kr-suite/rules/ownership/O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once.loft:137
kr-try/tests/scripts/a-function-value-fills-its-callers-record.loft:48
```
