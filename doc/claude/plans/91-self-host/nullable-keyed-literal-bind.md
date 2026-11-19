# @PLN91 — a nullable keyed collection bound from a literal (prepared subject)

**The family:** "a value its declared type converts: TyVector to TyOptional" has 21 sites at
tip e43620208. Every site is one shape: a local declared as a NULLABLE keyed collection and
bound from a literal, `c: hash<H[k]>? = [H { k: 3, v: 33 }]`. That covers hash, index, sorted,
spatial and trie (`a-nullable-keyed-collection-can-hold-a-v…` holds 14 of the sites).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with
1000 passes: `(c[3] ?? H {}).v` → 33; append then `len(c ?? [])` → 2; `c = null; c == null` →
true; the index forms → 66, and → -5 for a missing key with a literal default.

## Rust's form

```
fn h_one() -> integer { c: hash<H[k]>? = [H { k: 3, v: 33 }]; (c[3] ?? H {}).v }
    __ref_p2_1: ref(H) = null;  __ncc_1: ref(H)["c"] = null;          <- the `??`'s temps, minted first
    c: hash<H,["k"]>? = null;                                          <- declared type kept, NO deps
    if OpVectorIsNull(c) OpDatabase(c, 102) else null;                 <- the GUARDED mint
    _elm_1: ref(H)["c"] = OpNewRecord(c, 102, 65535); OpSetInt(_elm_1, 0, 3); OpSetInt(_elm_1, 8, 33);
    OpFinishRecord(c, _elm_1, 102, 65535);
    OpGetInt({#ncc __ncc_1 = OpGetRecord(c, 102, 1, 3); if OpConvBoolFromRef(__ncc_1) __ncc_1 else {#Object __ref_p2_1 …} }, 8)

append  `c += [H {…}]`  : the same guarded mint again, then the element (`_elm_2`)
`len(c ?? [])`          : t_4hash_len(if OpNot(OpVectorIsNull(c)) c else {#Vector __kvb_1 = null; OpDatabase(__kvb_1, 102); __kvb_1})
                          (`__kvb_1: hash<H,["k"]> = null` is minted at the head, first)
`c = null; c == null`   : if OpRefIsNull(c) null else { OpClearKeyed(c, 102); OpSetInt4(c, 0, -1) };  OpVectorIsNull(c)
index                    : the same, with the index's own database type (103) and `index<I,[("k", true)]>?`
```

The plain bind `c: hash<H[k]> = [...]` has NO mint at all (`c = null;` then the elements). The
`?` is what turns the mint on.

**What to code:** accept the bind of a vector literal into a declared nullable keyed collection
as the declared type itself, with no conversion. Keep the local's type `hash<…>?` with empty deps.
The literal then takes the path you already ported in 1450d for vectors: `build_vector_list`'s
guarded mint (vectors.rs, `guarded_mint`: not first pass, empty deps, not `assign_replaces`, the
assign target is this local), `if OpVectorIsNull(c) OpDatabase(c, <keyed db type>) else null`,
here with the keyed collection's database type. The `c = null` write (3 sites, the `*_then_null`
functions) is a separate form. Rust's state trace prints no route line for it, so its emitter is
not named here; port it after the bind, by the form above.

## The sites (tip e43620208)

```
kr-suite/rules/binding/B-Copy/1319-a-nullable-whole-value-bind-copies-like-its-dense-twin.loft::n_b19_the_keyed_kinds_are_the_control
kr-suite/rules/binding/B-Ref-Alias/a-null-written-to-a-collection-reaches-every-name-for-it.loft::n_main
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_append
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_hash_one
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_i_append
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_idx_then_null
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_index
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_lit_then_null
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_sorted
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_spatial
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_srt_then_null
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_trie
kr-suite/rules/collections/Col-Cons/a-nullable-keyed-collection-can-hold-a-value.loft::n_nk_trie_then_null
kr-suite/rules/layout/L-Null/bind-copies-or-views-the-whole-boundary.loft::n_test_whole_var_nullable_keyed_copies
kr-suite/rules/ownership/O-Latest/1324-a-reassigned-capture-suppresses-the-store-the-record-holds.loft::n_test_a_keyed_capture_keeps_its_collection_and_leaks_nothing
kr-suite/rules/ownership/O-Proxy/1320-a-branch-joined-binding-frees-the-arm-that-minted.loft::n_test_the_borrow_arm_leaves_the_callers_collection_intact
kr-suite/rules/ownership/O-Proxy/1456-a-key-write-through-a-nullable-collections-view-materialises.loft::n_nullable_keeps_the_element
kr-suite/rules/types/N-Shape/1836-a-link-to-a-nullable-place-writes-through.loft::n_kn_lit
kr-suite/rules/types/N-Shape/1836-a-link-to-a-nullable-place-writes-through.loft::n_test_a_link_to_a_nullable_vector_writes_through
loft-keyed-ranges/tests/scripts/1331-a-repointed-literal-accumulator-is-detached.loft::n_test_a_nullable_keyed_local_still_frees_what_it_displaces
loft-keyed-ranges/tests/scripts/1434-a-diagnostic-spells-a-keyed-type-as-the-source-does.loft::n_a_keyed_type_is_named_as_the_source_writes_it
```
