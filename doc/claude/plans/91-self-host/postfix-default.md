# @PLN91 — the postfix `?` default of a nullable collection (prepared subject)

**The family:** "a `?` default of this type" (29 sites in 18 programs at try b94e088a2): `e?`
on a nullable vector, hash or other collection (`len(b?)`, `for e in s? { … }`, `got = h?`,
`for x in o.h?`). The record and RefVar sub-shapes (`(n as Bin?)?`, a `&` link) are other
shapes and aren't part of this page.

**The rule:** `(N-Default)` (formal/types.md): `e?` ≡ `e ?? construct_default(τ)`, the TYPE's
default, an empty collection here.

**Judged right** on the interpreter, --native-c and --native, hand-computed from the rule:
`len(null?)` 0, `len([4, 5]?)` 2; a `for` over an absent hash runs no pass (0), over a present
one sums 10 + 20 = 30; `got = h?; got += [E{9, 1}]` gives 1 over an absent hash and 3 over a
present one with 2 entries, and the source keeps 2 (the bind copies).

## Rust's form

`e?` lowers exactly as `e ?? []`: the default arm mints the type's empty store.

```
len(b?)            (b: vector<integer>?)
    __vdb_1: ref(main_vector<integer>) = null;          <- hoisted to the top
    _vec_1: vector<integer> = null;
    t_6vector_len(if OpNot(OpVectorIsNull(b)) b else {#Vector:vector<integer>["__vdb_1"]
        OpDatabase(__vdb_1, 20);
        _vec_1 = OpGetField(__vdb_1, 0, 7);
        OpSetInt4(__vdb_1, 0, 0);
        _vec_1 });

for e in s? { … }  (s: hash<E[k]>?)
    __kvb_1: hash<E,["k"]> = null;                      <- hoisted to the top
    _hash_scratch_1 = n_hash_sorted(if OpNot(OpVectorIsNull(s)) s else {#Vector:hash<E,["k"]>
        __kvb_1 = null; OpDatabase(__kvb_1, 101); __kvb_1 }, 101);
    … the ordinary hash walk over _hash_scratch_1

got = h?           (a keyed bind from it: keyed-local-bind.md's form)
    got: hash<E,["k"]> = null;
    OpReplaceKeyed(if OpNot(OpVectorIsNull(h)) h else {#Vector … __kvb_1 … }, got, 101);
```

**What to code:** a postfix `?` on a nullable collection takes the `?? []` lowering you already
have (`register_default_literal`'s empty default): the null test is `OpNot(OpVectorIsNull(e))`
for every collection kind, the empty arm mints `__vdb_N`+`_vec_N` (a vector) or `__kvb_N` (a
keyed collection), each null-init hoisted to the function's top, and the value then flows into
its consumer (a call, a `for`, a keyed bind) exactly as a `?? []` would.

## The sites (try b94e088a2)

```
kr-suite/rules/binding/B-Copy/c131-a-cast-to-a-variant-answers-the-variant.loft::n_main:28  a `?` default of this type: TyReference
kr-suite/rules/binding/B-Ref-Alias/a-null-written-to-a-collection-reaches-every-name-for-it.loft::n_main:46  a `?` default of this type: TyNull
kr-suite/rules/binding/B-Ref-Alias/a-null-written-to-a-collection-reaches-every-name-for-it.loft::n_main:46  a `?` default of this type: TyVector
kr-suite/rules/binding/B-Ref-Alias/a-null-written-to-a-collection-reaches-every-name-for-it.loft::n_main:58  a `?` default of this type: TyHash
kr-suite/rules/binding/B-Ref-Uniform/a-default-fallback-through-a-link-discharges-its-referent.loft::n_grow_df:22  a `?` default of this type: TyRefVar
kr-suite/rules/binding/B-Ref-Uniform/a-default-fallback-through-a-link-discharges-its-referent.loft::n_int_df:28  a `?` default of this type: TyRefVar
kr-suite/rules/binding/B-Ref-Uniform/a-default-fallback-through-a-link-discharges-its-referent.loft::n_maybe_df:25  a `?` default of this type: TyRefVar
kr-suite/rules/binding/B-Ref-Uniform/a-default-fallback-through-a-link-discharges-its-referent.loft::n_text_df:27  a `?` default of this type: TyRefVar
kr-suite/rules/binding/B-Scope/1156-a-loop-body-local-read-after-the-loop-is-not-freed-per-iteration.loft::n_main:86  a `?` default of this type: TyVector
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_nullable_keyed_member_joins_its_group:276  a `?` default of this type: TyHash
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_nullable_keyed_member_joins_its_group:277  a `?` default of this type: TySorted
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_nullable_keyed_member_joins_its_group:278  a `?` default of this type: TyIndex
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_nullable_keyed_member_joins_its_group:279  a `?` default of this type: TyTrie
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_nullable_keyed_member_joins_its_group:280  a `?` default of this type: TyRadix
kr-suite/rules/heap/H-FreeNull/freeing-null-releases-nothing-on-every-path.loft::n_nulls:64  a `?` default of this type: TyVector
kr-suite/rules/heap/H-Removed/a-cleared-vector-releases-the-records-a-reference-named.loft::n_absent:114  a `?` default of this type: TyVector
kr-suite/rules/iteration/I-NullSrc/a-discharged-null-source-runs-no-round-whatever-it-is.loft::n_test_a_discharged_absent_source:87  a `?` default of this type: TyHash
kr-suite/rules/layout/L-Null-Which/2443-a-struct-enum-element-read-off-a-call-borrows-the-vector.loft::n_params:34  a `?` default of this type: TyVector
kr-suite/rules/operational/E-Default/the-types-default-discharges-each-kind.loft::n_test_insert_into_an_absent_hash:111  a `?` default of this type: TyHash
kr-suite/rules/ownership/O-Proxy/1456-a-key-write-through-a-nullable-collections-view-materialises.loft::n_nullable_keeps_the_element:77  a `?` default of this type: TySorted
kr-suite/rules/rewrites/R-KeyedEntry/a-keyed-insert-writes-the-entry-its-key-names.loft::n_main:115  a `?` default of this type: TyHash
kr-suite/rules/tuples/T-Absent/an-absent-tuple-meets-the-type-its-author-declares.loft::n_c3:158  a `?` default of this type: TyTuple
kr-suite/rules/tuples/T-Absent/the-operators-split-on-an-absent-tuple-and-one-that-is-there.loft::n_test_a_variable_index_hit_and_miss:55  a `?` default of this type: TyTuple
kr-suite/rules/types/D-NoEnumF/a-variant-cast-takes-the-default-its-fields-choose.loft::n_test_a_miss_takes_the_fields_own_default:36  a `?` default of this type: TyReference
kr-suite/rules/types/N-Cast/a-checked-cast-answers-null-on-a-miss-and-a-join-carries-it.loft::n_test_a_discharged_miss_takes_the_default:75  a `?` default of this type: TyReference
kr-suite/rules/types/N-Coal/a-spatial-or-trie-call-result-outlives-its-coalesce.loft::n_test_a_spatial_subject_bound_through_a_default:43  a `?` default of this type: TyRadix
kr-suite/rules/types/N-Coal/a-spatial-or-trie-call-result-outlives-its-coalesce.loft::n_test_a_trie_subject:67  a `?` default of this type: TyTrie
kr-suite/rules/types/N-Domain/1450-a-projection-is-not-more-non-null-than-its-receiver.loft::n_test_a_write_to_an_absent_keyed_local_still_lands:129  a `?` default of this type: TyHash
kr-suite/rules/types/N-Domain/1450-a-projection-is-not-more-non-null-than-its-receiver.loft::n_test_a_write_to_an_absent_vector_local_still_lands:136  a `?` default of this type: TyVector
```
