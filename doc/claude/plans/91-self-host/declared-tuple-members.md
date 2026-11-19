# @PLN91 — a tuple bound against a declared tuple type (prepared subject)

**The family:** "a value its declared type converts: TyTuple to TyTuple", 17 sites in 7 programs
at try f5320890c. A tuple literal bound to a local whose declared tuple type differs member by
member: a nullable record member (`t: (integer, SNTup?) = (n, null)`, `(SNTup?, integer) =
(mk_tup(n), n)`), a nullable vector (`(integer, vector<integer>?) = (n, [n, n])`), a KEYED
member from a vector literal (`(hash<H[k]>, integer) = ([H {…}, …], 3)`), and numeric widening.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with
1000 passes. Expected values computed by hand: `absent(5)` → 105, `first(5)` → 15, `vec(5)` → 7,
`keyed()` → 5, loop 106995. The keyed case double-freed on main (loft#2576: the scope pass
released the literal's accumulator before the tuple whose release disarms it). It is fixed on
loft30-gate 74bca91de, in the SCOPE pass, so the parsed IR below is unchanged by it.

## Rust's form: each member converted to its DECLARED type where it stands

```
(integer, R?) = (n, null)
    t: (integer, ref(R)?) = (n, OpNullRefSentinel());                  <- `null` as the record null sentinel
(R?, integer) = (mk(n), n)
    t: (ref(R)?, integer) = (n_mk(n, __ref_1), n);                      <- the call into its buffer, typed `R?`
(integer, vector<integer>?) = (n, [n, n])
    t: (integer, vector<integer>["__vdb_1"]?) = (n, {#Vector: vector<integer>["__vdb_1"]
        OpDatabase(__vdb_1, 20);  _vec_1 = OpGetField(__vdb_1, 0, 7);  OpSetInt4(__vdb_1, 0, 0);
        OpPreAllocVector(_vec_1, 2, 8);  OpPushInt(_vec_1, n);  OpPushInt(_vec_1, n);  _vec_1 });
                                                                         <- the member TYPE carries the backing: ["__vdb_1"]?
(hash<H[k]>, integer) = ([H { k: "a", n: 1 }, H { k: "b", n: 2 }], 3)
    t: (hash<H,["k"]>, integer) = ({#Vector: hash<H,["k"]>
        __kvb_1: hash<H,["k"]> = null;  OpDatabase(__kvb_1, 109);
        _elm_1: ref(H)["__kvb_1"] = OpNewRecord(__kvb_1, 109, 65535);
        OpSetText(_elm_1, 8, "a");  OpSetInt(_elm_1, 0, 1);              <- fields in DECLARATION order (k at 8, then n at 0)
        OpFinishRecord(__kvb_1, _elm_1, 109, 65535);  … the second …;  __kvb_1 }, 3);
```

A read of a nullable member under `??` is the ordinary discharge of `t.N`: `__ncc_1: ref(R)["t"] =
t.1; if OpConvBoolFromRef(__ncc_1) …`; a vector member's `?? []` tests `OpVectorIsNull`.

**What to code:** bind a tuple literal against the DECLARED tuple type member by member, each
member converted as a value of its declared type would be in any other position. That means
`null` to the record sentinel, a call into its buffer, a vector literal into a `__vdb_N` with the
member type `vector<…>["__vdb_N"]?`, and a vector literal for a keyed member into a `__kvb_N`
accumulator inside a `{#Vector}` block. No conversion op wraps the tuple. Numeric members widen
as a scalar would. The 7 programs' other spellings are in the site list.

## The sites (try f5320890c)

```
kr-suite/rules/binding/B-Ref-Uniform/1673-a-tuple-link-is-read-and-written-whole-like-a-tuple.loft::n_test_c5_a_hash_member_is_replaced
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_test_every_member_kind
kr-suite/rules/calls/F-Ret/153-p4-a-nullable-parameter-and-return-deliver-like-their-dense-twins.loft::n_test_p4_more_sites
kr-suite/rules/layout/L-Null/a-nullable-is-laid-out-and-owned-like-its-dense-twin.loft::n_opt_absent
kr-suite/rules/layout/L-Null/a-nullable-is-laid-out-and-owned-like-its-dense-twin.loft::n_opt_first
kr-suite/rules/layout/L-Null/a-nullable-is-laid-out-and-owned-like-its-dense-twin.loft::n_opt_second
kr-suite/rules/layout/L-Null/a-nullable-is-laid-out-and-owned-like-its-dense-twin.loft::n_opt_vector
kr-suite/rules/types/C-Tuple/a-tuple-converts-member-by-member.loft::n_test_a_tuple_variable_converts_each_member
kr-suite/rules/types/C-Var/a-tuple-member-is-typed-by-its-declaration-a-variant-one-included.loft::n_test_a_tuple_member_is_parsed_against_its_declared_type
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_absent
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_at_one
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_declared_nested
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_declared_null
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_declared_two
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_nested_nullable
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_null
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_two_null
```
