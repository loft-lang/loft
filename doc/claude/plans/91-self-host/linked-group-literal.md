# A literal that fills a linked collection group — what the Rust front end does (@PLN91)

Two or more collections of ONE record type in one struct are a linked group
(`@FR-Col-Group`): they hold the same records, each member a route to them.  What to mirror
where a struct literal names a member: the port's refusal "a literal that fills a member of a
linked collection group".  The probe is
[probes/linked-group-literal.loft](probes/linked-group-literal.loft): 12 cases, every value
hand-computed, clean on the interpreter and the C backend.

`r` is the record, `st` its type number, `fi` the index of the member the literal writes.

## The rule

1. **Every member's header is zeroed first, as one block, in offset order** —
   `OpSetInt4(r, <off>, 0)` per member, named by the literal or not, BEFORE any other field of
   the literal (a scalar written ahead of the members comes after the block).
2. **A member given a literal, keyed or vector**, is built through the RECORD, element by
   element: `_elm_1 = OpNewRecord(r, st, fi)`, the element's field writes,
   `OpFinishRecord(r, _elm_1, st, fi)`.  `fi` is the member the literal names; the finish is
   what files the record in every member.  It is the same form a keyed field has outside a
   group — only a group's VECTOR member differs from a plain vector field, which is built
   through its handle with `65535` for the field.
3. **A member given `[]`, and a member omitted, write nothing more**: the block of rule 1 is
   their whole write.  (Outside a group an omitted keyed field is zeroed a second time.)
4. **A VECTOR member given a vector VALUE** is filled as a vector field is, and then the
   group is indexed, once per keyed VIEW member:
   - a local: `OpAppendVector(OpGetField(r, off, <vector type>), v, <element type>)`
   - a call: `__ref_N = OpGetField(r, off, <vector type>); n_mk_v(3, __ref_N)` — `__ref_N`
     minted on BOTH passes
   - then `OpIndexGroup(OpGetField(r, off, <vector type>), OpGetField(r, <view off>, <vector type>), <view type>, st, fi)`
5. Fields that are not members are written in their written order, around the member writes.

## Where the port stands

It carries rule 1, the vector member's literal and its value forms, and an all-omitted
group.  It refuses the keyed member's literal (rule 2) — the form it already writes for a
keyed field outside a group.

## Leave refused

A KEYED member given a keyed VALUE (`VH { rows: [], by_k: mk_h(2) }`, `b.by_k = mk_h(2)`): the
Rust front end writes `OpReplaceKeyed` into the member and indexes nothing, so the group's
other members stay empty while the member's own lookups find the records.  That is a defect
of the Rust front end, not a form to mirror.
