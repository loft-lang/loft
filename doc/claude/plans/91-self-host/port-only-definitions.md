# @PLN91 — the definitions only the port registers (forbidden)

**The rule (owner, 2026-11-26):** a definition only the port registers is the port creating a new
language instead of porting one; a try that adds one is `TRYRESULT issues` (refusal-worklist.md).
These are the ones left, measured at try 2be44f5a6 (on 8e161452b), 71 rows (85 at
8e161452b).  Each family names who removes it; a row goes once a try shows it gone.

**Removed by the tester (refused before read, no lowering):** a `[…]` whose destination is a keyed
collection — a refused `&` of a keyed collection keeps its link type (el_ex_vector), a keyed
subject's non-empty `??` default (el_ex_call), a literal whose position expects a keyed collection
that `keyed_tail_literal` declines (el_ex_assign).  The porter's keyed-literal lowering replaces
the last two refusals.

## vector type — 54

| program | definition |
|---|---|
| a-null-written-to-a-collection-reaches-every-name-for-it.loft | `main_vector<Lh>` |
| a-null-written-to-a-collection-reaches-every-name-for-it.loft | `vector<Lh>` |
| a-view-materialises-where-its-container-is-disturbed-whatever-spells-it.loft | `main_vector<Room>` |
| a-view-materialises-where-its-container-is-disturbed-whatever-spells-it.loft | `vector<Room>` |
| 2562-a-returned-keyed-field-of-a-local-record-is-its-own-store.loft | `main_vector<Fr2562E>` |
| 2562-a-returned-keyed-field-of-a-local-record-is-its-own-store.loft | `vector<Fr2562E>` |
| a-lambda-reads-its-captures-inside-a-comprehension-and-leaves-them-alive.loft | `main_vector<CapPt>` |
| a-lambda-reads-its-captures-inside-a-comprehension-and-leaves-them-alive.loft | `vector<CapPt>` |
| a-generic-at-a-tuple-answers-the-boxed-record-and-its-callback-is-its-twins-type.loft | `main_vector<__tuple<integer,text>>` |
| a-template-that-names-itself-at-its-own-variables-compiles-on-its-own.loft | `main_vector<Tree>` |
| a-template-that-names-itself-at-its-own-variables-compiles-on-its-own.loft | `vector<Tree>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `main_vector<Ab1150H>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `main_vector<Ab1150I>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `main_vector<Ab1150P>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `main_vector<Ab1150S>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `main_vector<Ab1150T>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `vector<Ab1150H>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `vector<Ab1150I>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `vector<Ab1150P>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `vector<Ab1150S>` |
| a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft | `vector<Ab1150T>` |
| a-struct-point-pattern-reads-any-declaration-order-and-a-null-field.loft | `main_vector<Q?>` |
| a-struct-point-pattern-reads-any-declaration-order-and-a-null-field.loft | `vector<Q?>` |
| 2500-a-stored-reference-to-a-struct-enum-reads-as-its-record.loft | `main_vector<__typevar_Hit>` |
| 2500-a-stored-reference-to-a-struct-enum-reads-as-its-record.loft | `vector<__typevar_Hit>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnH>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnI>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnP>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnS>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnT>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `vector<JnH>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `vector<JnI>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `vector<JnP>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `vector<JnS>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `vector<JnT>` |
| 164-consumed-keyed-lift-rebind.loft | `main_vector<Lr1H>` |
| 164-consumed-keyed-lift-rebind.loft | `main_vector<Lr1S>` |
| 164-consumed-keyed-lift-rebind.loft | `vector<Lr1H>` |
| 164-consumed-keyed-lift-rebind.loft | `vector<Lr1S>` |
| a-spatial-or-trie-call-result-outlives-its-coalesce.loft | `main_vector<Cp>` |
| a-spatial-or-trie-call-result-outlives-its-coalesce.loft | `main_vector<Cw>` |
| a-spatial-or-trie-call-result-outlives-its-coalesce.loft | `vector<Cp>` |
| a-spatial-or-trie-call-result-outlives-its-coalesce.loft | `vector<Cw>` |
| 1836-a-link-to-a-nullable-place-writes-through.loft | `main_vector<Ek>` |
| 1836-a-link-to-a-nullable-place-writes-through.loft | `vector<Ek>` |
| an-expected-element-type-reaches-every-literal-position.loft | `main_vector<(integer, integer)>` |
| an-expected-element-type-reaches-every-literal-position.loft | `main_vector<vector<integer>>` |
| an-expected-element-type-reaches-every-literal-position.loft | `vector<(integer, integer)>` |
| an-expected-element-type-reaches-every-literal-position.loft | `vector<__tuple<integer,integer>>` |
| an-expected-element-type-reaches-every-literal-position.loft | `vector<vector<integer>>` |
| 2337-an-empty-literal-arm-builds-its-fields-collection.loft | `main_vector<K>` |
| 2337-an-empty-literal-arm-builds-its-fields-collection.loft | `vector<K>` |
| 923-nullable-vector-element-schema.loft | `main_vector<>` |
| 923-nullable-vector-element-schema.loft | `vector<>` |

## generic function instance — 9

| program | definition |
|---|---|
| a-reference-parameter-reaches-the-whole-surface.loft | `i_4text_t_6vector_sort` |
| a-reference-parameter-reaches-the-whole-surface.loft | `i_7integer_t_6vector_sort` |
| a-generic-at-a-tuple-answers-the-boxed-record-and-its-callback-is-its-twins-type.loft | `i_40integer#__tuple<integer,integer(0, 255)>_n_pick` |
| a-generic-over-a-vector-of-tuples-of-its-type-variables.loft | `i_7integer_n_setfirst` |
| c91-every-type-is-equatable.loft | `i_35__tuple<(integer, integer),integer>_n_same` |
| every-type-has-one-layout-whichever-spelling-asks.loft | `i_50__tuple<integer(0, 255),integer,integer(0, 65535)>_n_sz` |
| 1202-a-captured-record-enum-is-not-the-callers-to-take.loft | `i_7integer_t_6vector_map` |
| 100-enhancements.loft | `i_15integer#integer_t_6vector_map` |
| 100-enhancements.loft | `i_7integer_t_6vector_filter` |

## lambda / closure — 3

| program | definition |
|---|---|
| 2435-two-modules-types-of-one-name-key-two-instances.loft | `n___lambda_answers_0#2` |
| 2435-two-modules-types-of-one-name-key-two-instances.loft | `n___lambda_answers_1#2` |
| a-lambda-body-converts-into-its-expected-return.loft | `n___lambda___dflt_takes_f_0` |

## interface stub — 2

| program | definition |
|---|---|
| a-template-that-names-itself-at-its-own-variables-compiles-on-its-own.loft | `__iface_948#1_weight` |
| pln125-a2c-companion.loft | `__iface_911#1_plain_relay` |

## function — 2

| program | definition |
|---|---|
| a-lambda-body-converts-into-its-expected-return.loft | `n___dflt_takes_f` |
| a-library-carries-generics-across-the-boundary.loft | `n_describe` |

## tuple record — 1

| program | definition |
|---|---|
| an-expected-element-type-reaches-every-literal-position.loft | `__tuple<integer,integer>` |

