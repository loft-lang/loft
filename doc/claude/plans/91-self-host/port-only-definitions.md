# @PLN91 — the definitions only the port registers (forbidden)

**The rule (owner, 2026-11-26):** a definition only the port registers is the port creating a new
language instead of porting one; a try that adds one is `TRYRESULT issues` (refusal-worklist.md).
Left at try 5cdc4acc0 (the porter's keyed-literal lowering on bc8b9e94a): 34 rows, in
16 programs (85 at 8e161452b).  A row goes once a try shows it gone.

**Removed:** a `[…]` bound for a keyed collection (tester's refusals, then the porter's lowering:
`keyed_literal` takes every keyed hint — an arm, an empty `[]` arm, a `??` default, a `-> τ?`); every
generic function instance (tuple identity from its members, binding through tuple members, the
in-place `sort` / `map` / `filter` Rust builds).

**Known causes of what is left:**
- a write or append through a link to a keyed collection that is not refused at the `&`
  (1836, a-null-written-…): the link road;
- a declared element type not handed into the literal (an-expected-element-type-…, which also
  holds the one tuple record);
- a field the port dropped at a refusal, which makes a struct look like a type variable's
  placeholder (2500's `__typevar_Hit`); 923's nameless `vector<>`;
- lambdas, interface stubs and functions: not yet measured.

| program | definitions |
|---|---|
| an-expected-element-type-reaches-every-literal-position.loft | `__tuple<integer,integer>`, `main_vector<(integer, integer)>`, `main_vector<vector<integer>>`, `vector<(integer, integer)>`, `vector<__tuple<integer,integer>>`, `vector<vector<integer>>` |
| a-template-that-names-itself-at-its-own-variables-compiles-on-its-own.loft | `__iface_948#1_weight`, `main_vector<Tree>`, `vector<Tree>` |
| 1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft | `main_vector<JnH>`, `vector<JnH>` |
| 1836-a-link-to-a-nullable-place-writes-through.loft | `main_vector<Ek>`, `vector<Ek>` |
| 2337-an-empty-literal-arm-builds-its-fields-collection.loft | `main_vector<K>`, `vector<K>` |
| 2435-two-modules-types-of-one-name-key-two-instances.loft | `n___lambda_answers_0#2`, `n___lambda_answers_1#2` |
| 2500-a-stored-reference-to-a-struct-enum-reads-as-its-record.loft | `main_vector<__typevar_Hit>`, `vector<__typevar_Hit>` |
| 923-nullable-vector-element-schema.loft | `main_vector<>`, `vector<>` |
| a-lambda-body-converts-into-its-expected-return.loft | `n___dflt_takes_f`, `n___lambda___dflt_takes_f_0` |
| a-lambda-reads-its-captures-inside-a-comprehension-and-leaves-them-alive.loft | `main_vector<CapPt>`, `vector<CapPt>` |
| a-null-written-to-a-collection-reaches-every-name-for-it.loft | `main_vector<Lh>`, `vector<Lh>` |
| a-struct-point-pattern-reads-any-declaration-order-and-a-null-field.loft | `main_vector<Q?>`, `vector<Q?>` |
| a-view-materialises-where-its-container-is-disturbed-whatever-spells-it.loft | `main_vector<Room>`, `vector<Room>` |
| a-generic-at-a-tuple-answers-the-boxed-record-and-its-callback-is-its-twins-type.loft | `main_vector<__tuple<integer,text>>` |
| a-library-carries-generics-across-the-boundary.loft | `n_describe` |
| pln125-a2c-companion.loft | `__iface_911#1_plain_relay` |
