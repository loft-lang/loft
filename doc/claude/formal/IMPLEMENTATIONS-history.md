# formal/IMPLEMENTATIONS-history.md — the walks behind the census in [IMPLEMENTATIONS.md](IMPLEMENTATIONS.md)

> **The census is next door.**  [IMPLEMENTATIONS.md](IMPLEMENTATIONS.md) states the checklist and its
> verdicts; this file keeps the dated walks that produced three of them — what was measured, when,
> and what closed it (DOC_QUALITY § Maintainer docs 4).  Sections moved as written on 2026-09-29.

## A traverser's audit is blind to a classifier's omission (2026-09-08)

Three defects this cycle — loft#1444, loft#1474, loft#1477 — are one shape: **one question,
several decoders, and the defect is in whichever decoder the failing route consults.**  All
three sit in the return/closure neighbourhood, and the instrument that should have found them
could not, for a reason worth writing down.

`ir_walker_audit.py`'s `walkers` and `reach` measure DESCENT: does this walker enter every
child-bearing shape.  `Value::FnRef` is a LEAF in `Value::for_each_child` — correctly, it
carries no child expression — so a walker with no `FnRef` arm is invisible to both modes.
loft#1477 is exactly that: `scopes::collect_return_sources` classifies which VALUES a return
delivers, and a capturing lambda is not a `Var`, so `return fn() { … }` contributed nothing to
the delivered set and the frame freed what it had just handed to the caller.  `reach` listed
the function and named nine missing variants; `FnRef` was not among them and could not be.

**The distinction:** a TRAVERSER must descend into every child-bearing shape, and the audit
checks that.  A value CLASSIFIER must recognise every value-bearing LEAF — the complement —
and nothing checks it.

⚠ **The obvious screen for the complement is not an instrument yet.**  "Walkers that name `Var`
but omit `FnRef`/`FnRefDnr`/`TupleGet`/`Enum`" returns **238 functions**, because most of them
ask a question those leaves cannot answer.  Recorded so the next person does not build on the
number: sharpening it needs a way to say which walkers ask *which values does this deliver or
own*, and that is not written down anywhere.

## The inline-edge question — one notion, five spellings, one of them known (2026-09-10)

*Does this FIELD embed the host record's own bytes?*  The question `Data::has_value_cycle` asks
to decide whether a struct contains itself, and the row it belongs to above is **one notion,
many SPELLINGS** — with the sharpest version of that row's problem, because four of the five
spellings share no token with the one the matcher named.

The rules had already enumerated the set and the code had not read them.
`formal/layout.md` `(L-Null-Tag)` lists the INLINE positions — *"a `vector`/keyed element, an
embedded field, a tuple member"* — and `(L-Null-Which)` cites `has_value_cycle` BY NAME for
reading the same `u16::MAX` share marker.  The walk matched `Type::Reference(child, deps)` bare.

| spelling | reaches the walk as | seen? | symptom |
|---|---|---|---|
| `next: Node` | `Reference(Node)` | ✅ | — |
| `next: Node?` | `Optional(Reference(Node))` — the rewrite to the tagged `__nullable<Node>` runs AFTER this pass | ❌ | `type layout: … field 'next' has no position (u16::MAX)` |
| `p: (integer, Node)` | `Tuple([Integer, Reference(Node)])` | ❌ | **ICE** — `attempt to add with overflow`, the `u16` offset accumulator |
| `e: E`, `enum E { Branch { n: … } }` | `Enum(E)`, whose payloads are the enum's CHILDREN and not its attributes | ❌ | layout dump |
| `next: reference<Node>` / `reference<Node>?` | `Reference(Node, {u16::MAX})` | ✅ | correctly NOT an edge |

**What makes this one worth its own section is the severity gradient.**  A missed spelling here
does not cost a diagnostic — it costs a type that has no finite size, so the reader meets the
record builder rather than the compiler.  The cheapest surface is a layout dump with no cure in
it; the dearest is an internal compiler error.  A blind spot in a *reporter* is normally benign,
and this reporter is the only thing standing between the user and an unrepresentable type.

**And the spelling nobody takes is the one that worked.**  `next: Node?` is how a linked list is
written; `next: Node` is a type nobody writes on purpose.  All three existing guards
(`type_cycle_self`, `type_cycle_indirect`, `36b-pass1-parse-errors.loft`) assert the bare
spelling, so the covered route was the unused one.  The sharpest cell is an INDIRECT cycle with
a single optional hop (`A{b:B} B{a:A?}`) — the walk gets all the way around but for one step and
reports nothing, which is what says the defect is the edge decoder and not the traversal.

Cure per the row: **one body, exhaustive by construction.**  `Data::inline_field_defs` matches
every `Type` former and is the walk's only edge question, so a new former forces a decision
there; the walk asks ENUMS as well as structs, whose variants are children.  Nine guards, each
measured failing first, against seven controls — the controls are the load-bearing half, because
following an enum edge is what could start reporting a cycle for every program that puts an enum
in a struct.

## The key-owner question — one notion, six homes, three of them short (2026-08-29)

*Which field list do a keyed collection's key NUMBERS index?*  `Stores::key_owner` is the
declared home and its doc says why: a synth `__nullable<S>` element keeps S's keys inside the
`Some` variant's inline payload, so indexing the enum's own field list finds none of them.  Every
other element answers itself, which makes the short spelling **correct on every dense program**
— the normal appearance of this defect.

| site | direction | asked `key_owner`? |
|---|---|---|
| `Stores::hash` | name → number | ✅ (inline loop) |
| `Stores::create_key` — `sorted`, `index` | name → number | ✅ |
| `typedef::key_bearing_def` — the DEF-level twin the parser and `fill_database` use | name → def | ✅ |
| `Stores::field_name` — `spatial`, `trie` | name → number | ❌ |
| `Stores::key_name` — the `sorted` → `ordered` group rename | number → name | ❌ |
| `generation::bare_field_name` — the bare `init()` stream | number → name | ❌ |

Three short, and each failed differently because the DIRECTION differs.  Name → number failed
LOUDLY (*"`nm` is not a field of `__nullable<W>`"* — a refusal for a program the interpreter had
no trouble with).  Number → name failed SILENTLY and worse: the key list is part of the type
NAME, a type name is the intern key, so a `?` or an empty list is not a cosmetic difference — it
MINTS a second collection type, and every runtime id past it sits one above the compile-time id
baked into the emitted ops.  `verify_schema_ids` caught it (loft#739's guard, doing its job).

⚠ **The inverse direction is a distinct question and has to be counted separately.**  A census of
"who calls `key_owner`" finds the name → number sites and none of the number → name ones, because
the latter do not look like key resolution at the call site — they look like rendering.  Both
belong to one notion: `create_key` and `key_name` are inverses of each other and disagreed.

**A fourth spelling, of the neighbouring question.**  *Where does an `index`'s red-black
bookkeeping live?* had three homes — `Stores::fields`, `Stores::find_index` and
`Stores::build_index_sorted_vec` — each recomputing `8 + fields[left_field].position`.  The two
copies read the element's own field list, so the tree descended from `u16::MAX` for a nullable
element.  They now call `fields`, which resolves through the new `Stores::index_owner`, the same
helper the APPEND uses — so where the links are written and where the walk starts cannot drift.

**The rules gap this sits in is the one already recorded above.**  `Col-Hash` / `Col-Sorted` /
`Col-Index` / `Col-Spatial` / `Col-Trie` each define one kind and no rule names the keyed FAMILY;
nothing in `formal/` states the linked-GROUP contract at all — *two or more collections over one
element type in one struct are several routes to a single record set* — even though loft#843,
loft#901 and loft#927 are all fixes to it and two more landed this week (a view beside a
`vector<S?>`, and group formation ceasing to depend on declaration order).  An edge the rules
cannot express is a rule that wants extending: `Col-Group` is the missing one.
