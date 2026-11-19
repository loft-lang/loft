# @PLN91 — an early `return` of a vector the function does not own (prepared subject)

**The family:** "a `return` with a value of this type" (91 sites at try 63fb36e57; the sole
refusal of 5 programs). Its largest sub-shape is `TyVector` (29 sites in 13 programs): a
`return` from inside a branch, not the tail, of a vector the function doesn't own. That's a
parameter's field (`return s.vals`, `return p.items`, `return o.o_in.i_v`), a constant
(`return NUMS`, `return NAMES`), or a call's field (`return wrap(c).inner.v`).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 300
passes, for the field forms. Hand-computed: `a = from_src(Src { vals: [4, 7] }, -1); a += [9]`
→ `a` is `[4,7,9]` and the source stays `[4,7]`; `proj(5, -1)` → `[5,6]`; the tails `[k]` /
`[k, k]` → `[3]` / `[2,2]`. The caller always gets its own copy.

**One ORACLE DEFECT, not to mirror:** an early `return` of a CONSTANT hands out the
constant's own store. `fn f(k) -> vector<integer> { if k < 0 { return NUMS; } [k] }` then
`b = f(-1); b += [9]` stops with "write to a constant" (interpreter) or "write to a locked
store" (C, rustc), on main as well. The TAIL spelling `fn f() -> vector<integer> { NUMS }`
copies, and so does the bind `b = NUMS`. I'm fixing it on loft30-gate. Port the COPY form below
for a constant too, which is what Rust will emit after the fix.

## Rust's form

```
return s.vals        (a parameter's vector field: copied into the return buffer)
    OpClearVector(__retbuf);
    OpAppendVector(__retbuf, OpGetField(s, 0, 7), 0);
    return __retbuf;

return wrap(c).inner.v     (a field of a call's record: the holding record copied into the buffer,
                            the field read off the copy)
    return OpGetField({#inline ref copy:ref(In)["__retbuf"]
        __retbuf: vector<integer> = null;
        OpDatabase(__retbuf, 101);                                <- 101 = In, the record holding .v
        OpCopyRecord(OpGetField(n_wrap(c, __ref_1), 0, 101), __retbuf, 101);
        __retbuf }, 0, 7);

return NUMS          (a constant: TODAY `return OpConstRef(874);` with no buffer, the defect;
                      after the fix, and in the tail spelling today:)
    OpClearVector(__retbuf);
    OpAppendVector(__retbuf, OpConstRef(874), 0);
    return __retbuf;
```

The function declares the hidden `__retbuf: vector<integer>` attribute, and the body opens
with `{ !! INSERT OpClearVector(__retbuf) }`. Its tail is the `#one_buffer_vec_copy` block, as
for any vector tail. Both functions answer `-> vector<integer>["??"]`: an unresolved return dep
that Rust prints for every one of these (also in your `first(h)` note), not something to mirror
in the dump comparison.

**What to code:** an early `return e` of a vector `e` the function doesn't own goes through
the same `__retbuf` copy as the tail: clear, `OpAppendVector(__retbuf, e, <elem type>)`, then
`return __retbuf`. A field of a CALL's record copies the holding record into the buffer and
reads the field off it (`#inline ref copy`). A constant takes the clear-and-append form.

## The sites (try 63fb36e57)

```
kr-suite/rules/calls/F-Return/a-function-answers-the-value-its-return-or-its-tail-names.loft::n_vec_if:102
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_append_record:161
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_under_return:541
kr-suite/rules/iteration/I-Comp/two-discard-comprehensions-each-bind-their-own-variable.loft::n_mixed_types:21
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c1:39
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c2:42
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c3:45
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c4:49
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c5:52
kr-suite/rules/ownership/O-Buffer/1618-a-returned-nullable-vector-local-is-delivered.loft::n_c7:58
kr-suite/rules/rewrites/R-Place/a-literal-vector-field-is-built-in-its-field.loft::n_glob:63
kr-suite/rules/rewrites/R-Place/a-literal-vector-field-is-built-in-its-field.loft::n_pfield:70
kr-suite/rules/rewrites/R-Place/a-literal-vector-field-is-built-in-its-field.loft::n_proj:53
kr-suite/rules/types/N-Shape/2233-a-coalesce-tail-delivers-into-the-callers-buffer-a-lambdas-too.loft::n___lambda_test_an_explicit_return_0:205
kr-suite/rules/types/N-Shape/2233-a-coalesce-tail-delivers-into-the-callers-buffer-a-lambdas-too.loft::n_wr:72
kr-try/tests/scripts/164-buffer-is-the-place.loft::n_mkproj:76
kr-try/tests/scripts/174-a-slice-is-a-vector-wherever-a-vector-is-expected.loft::n_tail2:18
kr-try/tests/scripts/445-generic-tree-walk.loft::n_gw_walk:42
kr-try/tests/scripts/445-generic-tree-walk.loft::n_gw_walk_dfs:126
kr-try/tests/scripts/445-generic-tree-walk.loft::t_6GwNode_gw_kids:27
kr-try/tests/scripts/445-generic-tree-walk.loft::t_6SwNode_children:77
kr-try/tests/scripts/899-unbound-file-read-temp.loft::n_r899_read:41
kr-try/tests/scripts/981-split-ownership-return.loft::n_i981_take:87
kr-try/tests/scripts/an-appended-call-result-arrives-from-every-exit.loft::n_const_first:38
kr-try/tests/scripts/an-appended-call-result-arrives-from-every-exit.loft::n_from_src:54
kr-try/tests/scripts/an-appended-call-result-arrives-from-every-exit.loft::n_wrote_first:31
kr-try/tests/scripts/a-placed-call-copies-an-answer-from-elsewhere.loft::n_from_src:65
kr-try/tests/scripts/a-placed-call-copies-an-answer-from-elsewhere.loft::n_names:40
kr-try/tests/scripts/a-placed-call-copies-an-answer-from-elsewhere.loft::n_wrote_first:52
```
