# @PLN91 — a record local rebound from literals and returned (prepared subject)

**The family:** "a literal of a droppable built apart for a parameter it rebinds", 10 sites in 5
programs at try ca4065330, and the ONLY refusal in 3. A function builds a record holding a record
field into a local, may rebind the local from another literal (or another local), and returns it:
`s = Lhold { h: lmk(1) }; s = Lhold { h: lmk(2) }; return s;` (1564's a1–a8, 1573's g3, l6, dc9,
mk_named).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with
1000 passes and no leak. Expected values computed by hand: twice → `2/2/[2,4]`; an early `return s`
before the rebind → `3/3`; after it → `4/[4,8]`; `s = t` from another local → `12/12`; loop total
7500.

## Rust's form

```
fn twice() -> Lhold { s = Lhold { h: lmk(1), n: 1 }; s = Lhold { h: lmk(2), n: 2 }; return s; }
  s: Lhold mutable nullable hidden                    <- the return buffer RENAMED onto `s` (MergeAttr a: 0 at the return)
  -> Lhold["s"]
  {#block: ref(Lhold)["__retbuf"]
    __ref_2: ref(Lh) = null;  __ref_1: ref(Lh) = null;          <- one buffer per call of the record field's builder
    if (if OpRefIsNull(s) false else OpConvBoolFromRef(s)) null else OpDatabase(s, 101);   <- mint into `s` only where absent
    OpCopyRecord(n_lmk(1, __ref_1), OpGetField(s, 0, 100), 100);  <- the embedded record: the call into its own buffer, copied in
    OpSetInt(s, 16, 1);                                            <- then the scalar
    <the same three statements for the second literal, __ref_2>     <- the rebind writes the SAME record again: no free, no new store
    return s; }
```

Routes (`LOFT_STATE_TRACE`): `classify_reference_delivery=BuildIntoBuffer(__ref_p2_1,__retbuf)` on
the function, `parse_object` per literal, `classify_ret_promotion=MergeAttr { a: 0 }` at the
return.

**What to code:** where `classify_reference_delivery` answers `BuildIntoBuffer` and the returned
local is rebound only from record literals, every literal assigned to the local is built INTO the
renamed buffer `s`. Each one gets the guarded mint (`if (if OpRefIsNull(s) false else
OpConvBoolFromRef(s)) null else OpDatabase(s, <db>)`), then its fields in declaration order: an
embedded record field from a call is the call into a fresh `__ref_N` copied in by `OpCopyRecord(…,
OpGetField(s, <off>, <type>), <type>)`, a scalar by its setter. The early-return and `s = t`
variants are in the site list. Trace them with `elab_trace.py --points route` before porting:
`s = t` from a local takes a copy, not this build.

## The sites (try ca4065330)

```
kr-suite/rules/heap/H-Copy-Refuse/a-move-of-an-owned-local-releases-once.loft::n_l6
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a1_f
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a2_f
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a5_f
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a6_f
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a7_f
kr-suite/rules/heap/H-Drop/1564-a-local-releases-every-record-it-held-across-passes-and-returns.loft::n_a8_f
kr-suite/rules/heap/H-Drop/1573-a-rebuilt-local-releases-what-it-displaces-after-the-new-value.loft::n_g3_f
kr-suite/rules/ownership/O-Latest/a-whole-value-copy-of-a-droppable-releases-once.loft::n_dc9
kr-try/tests/scripts/1512-a-tuple-argument-call-member-releases-once.loft::n_mk_named
```
