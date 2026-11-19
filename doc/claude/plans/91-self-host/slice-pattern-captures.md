# @PLN91 — captures in a slice pattern (prepared subject)

**The family:** "a slice pattern element of this form" (52 sites at try 63fb36e57; the sole
refusal of 5 programs, met by 18). This page is its main sub-shape, `PCap`: 29 sites in 7
programs. A capture NAMES an element or the rest: `[x: _, y]`, `[x: 5, y]`,
`[h, xs: integer*, a, b]`, `[0, xs: integer*] => xs`. `PGroup` (`g:(Kw { k } Op { o })?`,
`(a: Num | b: Word)`) and `PLit` are other shapes and aren't part of this page.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 200
passes. Hand-computed: `[h, xs: integer*, a, b]` over `[1,2,3,4,5]` → `1 [2,3] 4 5`, over
`[1,2,3]` → `1 [] 2 3`, over `[1]` → `none`; `[x: _, y]` over `[3,4]` → 34, over `[1]` →
-1; `[x: 5, y]` over `[5,2]` → 7, over `[4,2]` → -1; `[0, xs: integer*] => xs` over
`[0,7,8]` → `[7,8]`. The rest is a COPY: `xs += [9]` inside the arm leaves the subject's
length at 3 (`a-rest-is-a-fresh-capture-wherever-it-stands`).

## Rust's form

```
[x: _, y]        (a named wildcard: a plain element binding, no test)
    if OpEqInt(OpLengthVector(v), 2) {#slice_binding
        x#1: integer = OpGetInt(OpGetVector(v, 8, 0), 0);
        y#1: integer = OpGetInt(OpGetVector(v, 8, 1), 0);
        <arm> } else <default>

[x: 5, y]        (a named literal: tested through a field_sub block, THEN bound)
    if if OpEqInt(OpLengthVector(v), 2) {#field_sub:boolean
            _fp_subj_1: integer = OpGetInt(OpGetVector(v, 8, 0), 0);
            OpEqInt(_fp_subj_1, 5) } else false
       {#slice_binding  x#1 = OpGetInt(OpGetVector(v, 8, 0), 0); y#1 = …(v, 8, 1)…; <arm> }
    else <default>

[h, xs: integer*, a, b]    (a named rest between fixed elements)
    if OpLeInt(3, OpLengthVector(v)) {#slice_binding
        h#1 = OpGetInt(OpGetVector(v, 8, 0), 0);
        a#1 = OpGetInt(OpGetVector(v, 8, -2), 0);       <- after the rest: NEGATIVE indices
        b#1 = OpGetInt(OpGetVector(v, 8, -1), 0);
        { !! INSERT                                      <- the rest, materialised LAST
          OpDatabase(__vdb_1, 20)
          xs#1: vector<integer>["__vdb_1"] = OpGetField(__vdb_1, 0, 7)
          OpSetInt4(__vdb_1, 0, 0)
          { !! INSERT  _rest_lo_1 = 1;  _rest_hi_1 = OpMinInt(OpLengthVector(v), 2);
                       _rest_idx_1 = OpConvIntFromNull() }
          loop {#Slice materialise
            _slice_elm_1: integer = {#Vector Index  OpGetInt(OpGetVectorNullable(v, 8, {#Iter range
                _rest_idx_1 = if OpNot(OpConvBoolFromInt(_rest_idx_1)) _rest_lo_1 else OpAddInt(_rest_idx_1, 1);
                if OpLeInt(_rest_hi_1, _rest_idx_1) break(0) else null;
                _rest_idx_1 }), 0) };
            _elm_1: ref(integer)["__vdb_1"] = OpNewRecord(xs#1, 7, 65535);
            OpSetInt(_elm_1, 0, _slice_elm_1);
            OpFinishRecord(xs#1, _elm_1, 7, 65535) } };
        <arm> }
```

**What to code:** a capture binds the element at its position, as `x#N`, inside the arm's
`#slice_binding` block. Positions before the rest are non-negative and those after it are
negative from the end. A named LITERAL (or any testing sub-pattern) is first tested in a
`#field_sub` block over a `_fp_subj_N` read, joined to the length test by `if … else false`,
and the element is read again in the binding. A named rest `xs: T*` is a fresh store
(`__vdb_N`, the element's `main_vector` record) filled by the `#Slice materialise` loop over
`_rest_lo_N = <first rest position>`, `_rest_hi_N = len - <elements after it>`,
`_rest_idx_N`. It's emitted AFTER the fixed bindings, which is why `a`, `b` precede it in the IR.
The length test is `OpEqInt(len, n)` without a rest and `OpLeInt(n, len)` with one.

## The sites (try 63fb36e57)

```
kr-suite/rules/binding/P-Cap-View/a-pattern-capture-is-a-view-and-a-rest-is-fresh.loft::n_main:67
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_alt:57
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_count:65
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_grab:66
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_opt:52
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_tail:67
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_test_the_capture_is_fresh_and_owned:81
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_both_ends:29
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_gap:31
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_last:26
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_last_plus:27
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_lit:25
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_lit_then_name:32
kr-suite/rules/matching/P-Rep-Scalar/a-scalar-repetition-is-a-typed-rest.loft::n_words:33
kr-suite/rules/matching/P-Rep-Scalar/a-typed-rest-leaves-its-tail-where-a-greedy-run-takes-it.loft::n_rest_int:31
kr-suite/rules/matching/P-Rep-Scalar/a-typed-rest-leaves-its-tail-where-a-greedy-run-takes-it.loft::n_rest_text:32
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_typed:176
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_bool:32
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_char:31
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_f:29
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_int:27
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_text:30
kr-suite/rules/types/P-Cap-Ty/a-pattern-capture-takes-the-type-its-pattern-matched.loft::n_s_tok:34
kr-try/tests/scripts/35b-sequence-capture.loft::n_head_score:52
kr-try/tests/scripts/35b-sequence-capture.loft::n_lit_then:42
kr-try/tests/scripts/35b-sequence-capture.loft::n_middle_store:60
kr-try/tests/scripts/35b-sequence-capture.loft::n_namepat_reuse:83
kr-try/tests/scripts/35b-sequence-capture.loft::n_ship_metrics:56
kr-try/tests/scripts/35b-sequence-capture.loft::n_wild_cap:46
```
