# A keyed collection written into a field — what the Rust front end does (@PLN91)

What to mirror where a struct literal's field, or a whole-field assignment, is given a `hash`,
`sorted` or `index` value: the port's refusals "a field write of this type: TyHash / TySorted /
TyIndex", "a field value its type converts: vector<E> to hash<E>" and "a place assignment its
type converts: TyVector to TyHash".  The probe is
[probes/keyed-field-write.loft](probes/keyed-field-write.loft): 17 cases, every value
hand-computed, clean on the interpreter and the C backend.

## The write, by what the value is

`r` is the record, `off` the field's offset, `kt` the collection's type number, `st` the
record's own type number, `fi` the field's index in the struct.

| the value | written as | temporaries |
|---|---|---|
| a KEYED value that is a fresh call's result (`h: mk_h(3)`) | `OpReplaceKeyed(n_mk_h(3), OpGetField(r, off, kt), kt + 32768)` | none; the callee takes no hidden buffer |
| a keyed value in a PLACE — a local, a parameter, another record's field | `OpReplaceKeyed(<place>, OpGetField(r, off, kt), kt)` | none |
| a plain VECTOR, call or local (`h: mk_v(3)`) | `OpFillKeyed(r, <vector>, kt, st, fi)` | the vector call's `__ref_N: vector<ref(E)>`, minted on BOTH passes |
| a keyed literal (`h: [E {…}, E {…}]`) | per element `OpNewRecord(r, st, fi)`, the element's field writes, `OpFinishRecord(r, _elm_1, st, fi)` | `_elm_1: ref(E)["<r>"]` |
| nothing (the field is omitted) | `OpSetInt4(r, off, 0)` a second time | none |

- `+ 32768` (`0x8000`) is the FREE-SOURCE bit: set where the source is a call that gives away
  its store (`call_gives_away_its_store`), never for a place — a place is deep-copied, and the
  probe's `hash_local`, `hash_param` and `hash_field` show the copy (a later append to the field
  is not seen through the source).  `OpFillKeyed` never carries the bit.
- The same `kt + 32768` binds a keyed LOCAL from a call: `h = mk_h(3)` is
  `OpReplaceKeyed(n_mk_h(3), h, kt + 32768)`.

## In a literal

Every keyed field is first zeroed with `OpSetInt4(r, off, 0)`, IN ITS WRITTEN POSITION, directly
before its write: `HBox { n: 1, h: mk_h(3) }` is `OpSetInt(n)`, `OpSetInt4(h)`, `OpReplaceKeyed`;
`First { h: mk_h(2), n: 6, t: "tx" }` is `OpSetInt4(h)`, `OpReplaceKeyed`, `OpSetInt(n)`,
`OpSetText(t)`.  A plain VECTOR field differs: its `OpSetInt4` is hoisted to the front of the
literal, before the fields written ahead of it.

## As a whole assignment (`b.h = <value>`)

| the value | written as |
|---|---|
| keyed, a fresh call | `OpReplaceKeyed(n_mk_h(3), OpGetField(b, off, kt), kt + 32768)` |
| keyed, a place | `OpReplaceKeyed(h, OpGetField(b, off, kt), kt)` |
| a plain vector | `OpClearKeyed(OpGetField(b, off, kt), kt)`, then `OpFillKeyed(b, <vector>, kt, st, fi)` |

No zeroing first: `OpReplaceKeyed` removes what the field held.  The vector case clears first
because `=` replaces; in a literal there is nothing to clear.

## Leave alone

Two collections of one record type in ONE struct are a linked group (`@FR-Col-Group`): a
literal that fills a member is its own rule — [linked-group-literal.md](linked-group-literal.md).
