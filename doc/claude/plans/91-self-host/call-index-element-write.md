# @PLN91 — a write through an element whose index runs a call (prepared subject)

**The family:** "an assignment to a nullable place" (38 programs met, the sole refusal of 5, at
try 7ae757a80). This page is its sub-shape where the place is a vector ELEMENT whose index runs
a call or a block: 19 sites in 10 programs. A computed index makes the element read nullable,
which is why the port refuses it. Rust evaluates such an index exactly once (`(E-Asgn)`,
`(E-Asgn-Compound)`; the @PLN221 slot matrix rows 10–12, loft#2580/#2581/#2582/#2584).
The plain writes `vt[1] = null` (a `vector<text?>` element) and field writes of a nullable
record are other shapes and aren't part of this page.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native.
Hand-computed: `vs[next(c)].x -= 1` over `[V{50}, V{60}]` with `next` answering 1 → `vs[1].x`
59; `h.ws[at(c, 1)].items = [P { k: 7, t: "q" }]` → `h.ws[1].items` is exactly that one
record; `vi[at(c, 2)] += 5` over `[10, 20, 30]` → `[10,20,35]`; the counter `c.n` = 3 (each
index ran once).

## Rust's form

```
vs[next(c)].x -= 1        (a compound write: the ELEMENT bound once to a place view)
    __place_1: ref(V)["vs"] = OpGetVector(vs, 8, n_next(c));
    OpSetInt(__place_1, 0, OpMinInt(OpGetInt(__place_1, 0), 1));

vi[at(c, 2)] += 5          (the same for a scalar element; note the place's TYPE)
    __place_2: vector<integer>["vi"] = OpGetVector(vi, 8, n_at(c, 2));
    OpSetInt(__place_2, 0, OpAddInt(OpGetInt(__place_2, 0), 5));

h.ws[at(c, 1)].items = [P { k: 7, t: "q" }]
                           (a whole collection written with =: the INDEX bound once to an
                            integer temp, the place chain kept inline for every op that rebuilds it)
    __index_1: integer = n_at(c, 1);
    OpClearVector(OpGetField(OpGetVector(OpGetField(h, 0, 106), 4, __index_1), 0, 104));
    _elm_4: ref(P)["h"] = OpNewRecord(OpGetVector(OpGetField(h, 0, 106), 4, __index_1), 103, 0);
    OpSetInt(_elm_4, 0, 7);  …  OpFinishRecord(…)
```

**What to code:** (1) a COMPOUND write (`op=`) whose place addresses an element through an index
that runs a call, or a block that writes, binds the element ONCE to `__place_N` (typed as
above: a record element `ref(T)["<vector>"]`, a scalar one `vector<T>["<vector>"]`) and reads
and writes through it. (2) A whole-COLLECTION `=` through such a place binds the INDEX once to
`__index_N: integer` and keeps the place chain inline: every op that rebuilds the collection
re-addresses it, so a right side that grows the vector is safe. An index that only READS
(`len(v) - 1`, a variable) takes neither form unless the right side calls or blocks
(loft#2584: pass 1's record of the right side's forms). (3) The element read through a
computed index is not refused as a nullable place here: it's the place, not a value.

## The sites (try 7ae757a80)

```
kr-suite/rules/calls/F-Ret/2443-a-record-copied-out-of-a-kept-buffer-releases-it.loft::n_arm:36
kr-suite/rules/collections/Col-Copy/a-vector-field-keeps-its-absence.loft::n_test_a_copy_into_a_field_keeps_the_sources_absence:70
kr-suite/rules/collections/Col-Group/a-write-through-any-member-of-a-linked-group-reaches-every-member.loft::n_test_a_group_element_written_through_the_vector_member_reaches_every_member:503
kr-suite/rules/collections/Col-Len/an-absent-keyed-field-is-empty-to-every-reader.loft::n_test_an_absent_hash_field:85
kr-suite/rules/collections/Col-Len/an-absent-keyed-field-is-empty-to-every-reader.loft::n_test_an_absent_index_field:173
kr-suite/rules/collections/Col-Len/an-absent-keyed-field-is-empty-to-every-reader.loft::n_test_an_absent_sorted_field:129
kr-suite/rules/collections/Col-Len/an-absent-keyed-field-is-empty-to-every-reader.loft::n_test_an_absent_spatial_field:217
kr-suite/rules/collections/Col-Len/an-absent-keyed-field-is-empty-to-every-reader.loft::n_test_an_absent_trie_field:261
kr-suite/rules/operational/E-Asgn/2580-a-whole-collection-written-through-a-called-place-addresses-it-once.loft::n_main:27
kr-suite/rules/operational/E-Asgn/2580-a-whole-collection-written-through-a-called-place-addresses-it-once.loft::n_main:36
kr-suite/rules/operational/E-Asgn/2580-a-whole-collection-written-through-a-called-place-addresses-it-once.loft::n_main:40
kr-suite/rules/operational/E-Asgn-Compound/1819-a-compound-assignment-computes-its-operator-on-both-operand-types.loft::n_main:138
kr-suite/rules/operational/E-Asgn-Compound/1825-a-compound-place-with-a-side-effecting-index-is-addressed-once.loft::n_main:46
kr-suite/rules/operators/Op-Compound/a-compound-of-a-user-operator-reads-its-place-once.loft::n_test_a_field_element_twice:41
kr-suite/rules/operators/Op-Compound/a-compound-of-a-user-operator-reads-its-place-once.loft::n_test_a_field_element_twice:42
kr-suite/rules/operators/Op-Compound/a-compound-of-a-user-operator-reads-its-place-once.loft::n_test_an_element_with_a_counted_index:34
kr-suite/rules/ownership/O-Detach/2397-a-literal-reads-its-field-under-another-name.loft::n_test_an_index_the_statement_cannot_name:108
kr-suite/rules/ownership/O-Detach/2397-a-literal-reads-its-field-under-another-name.loft::n_test_another_index_variable_may_name_the_same_element:101
kr-suite/rules/types/N-Store/1720b-a-nullable-written-to-an-element-is-reported.loft::n_test_elem_record:21
```
