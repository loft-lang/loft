# The hoisted default — what the Rust front end does (@PLN91)

What to mirror when a field's or a parameter's default is no constant.  Facts read from the
Rust front end's output; the probe that challenges each of them is
[probes/hoisted-default.loft](probes/hoisted-default.loft) (one function per case, every value
hand-computed; `selfhost/front/elab_probe.py one` diffs it against Rust).

## When a default becomes a function

`parse_stored_default` (`src/parser/definitions.rs`) parses the default where it stands, in the
struct's context, which has no frame.  It is replayed at every construction site as it is
unless `default_replayable_in_place` says no — a WHITELIST, so an unrecognised shape is hoisted.

| replayed in place (no function) | hoisted into a function |
|---|---|
| a literal; arithmetic and calls over literals | any block other than the two beside — an `if` with block arms, a formatted string, a vector literal |
| a read of `$` (it is `Var(0)`, rewritten to the record at the site) | any variable that is not `$` |
| an enum unit variant | for a struct-typed field, every build shape (a struct literal, a call) |
| for a field that is NOT struct-typed: an `Object` block, an `Insert` | at a PARAMETER: every block and every `Insert`, `= []` included |

Two more triggers: pass 2 hoists whatever pass 1 minted, and pass 1 hoists a checked narrowing
(a forward reference is asked as the integer it will be).

## The function

| default | definition | hidden parameter | result |
|---|---|---|---|
| `vector<T> = […]` | `n___dflt_<Struct>_<field>` | `__vdb_1: vector<T>` | `vector<T>` |
| `text = "w{1 + 1}"` | `n___dflt_…` | `__work_1: &text` | `text["__work_1"]` |
| a struct literal | `n___dflt_…` | `__retbuf: <Struct>` | the struct |
| a call answering a struct | `n___dflt_…` | `__ref_1: <Struct>` | `<Struct>["__ref_1"]`; the body is the call with `__ref_1` |
| a tuple that needs a temporary | `n___dflt_…` | `__retbuf: __tuple<…>` | the tuple |
| a keyed collection | `n___dflt_…` | none | the collection |
| reads `$` AND builds a value | `t_<LEN><Struct>___dflt_<Struct>_<field>` | `self: <Struct>` first, then the row's hidden one | as the row |
| a parameter default reading EARLIER parameters | `n___dflt_<fn>_<param>` | those parameters first, then the row's hidden one | as the row |

- Minted on PASS 1 where the default stands, and numbered there: a field's directly after its
  struct, a parameter's BEFORE the function it belongs to.
- Pass 2 finds it under either key (`n_…` or the receiver's `t_…`).
- In a generic struct there is ONE definition for every instance.  An empty-vector default
  there (`more: vector<T> = []`) is not hoisted: the site writes only the header.

## The construction site

| field | written as |
|---|---|
| vector | `OpSetInt4(r, off, 0)`, then `OpAppendVector(OpGetField(r, off, <vector type>), <dflt>(__ref_N), <n>)` — `__ref_N: vector<T>` |
| text | `OpSetText(r, off, <dflt>({#default ref … __work_cN = ""; OpCreateStack(__work_cN) }))` — one `__work_cN: text`, no `__ref_N` |
| record, by literal or call | `OpCopyRecord(<dflt>(__ref_N), OpGetField(r, off, <struct>), <struct>)` — `__ref_N: ref(<Struct>)` |
| tuple with a temporary | `{#tuple_field_set OpCopyRecord(<dflt>(__ref_N), OpGetField(r, off, <tuple>), <tuple>) }` |
| tuple without one | `{#tuple_field_set` the member writes `}` |
| keyed collection | `OpSetInt4(r, off, 0)`, then `OpReplaceKeyed(<dflt>(), OpGetField(r, off, <element>), <n>)` — no temporary |
| enum from an `if` over `$` | `OpSetEnum(r, off, <method>(r))` — no temporary |
| a method default | the record is the first argument: `<method>(r, …)` |
| inline defaults | the value itself: `OpSetInt(r, off, OpAddInt(n_seed(), 1))`, `OpSetInt(r, off, OpMulInt(OpGetInt(r, 0), 2))`, `OpSetEnum(r, off, 2u8)` |

- ORDER: `OpDatabase`, the fields the literal GIVES in the order written, then the defaulted
  fields in DECLARATION order.  A given vector field's `OpSetInt4(r, off, 0)` comes first,
  before the other given fields.
- MINTING: every `__ref_N` and `__work_cN` of a defaulted field is minted on PASS 2 only, in
  declaration order of the fields, at the literal's statement.  Pass 1 leaves only the record's
  variable.  Head null-inits run in reverse.

## A call site

`fn par(a, b: integer = seed() * 2, v: vector<integer> = [])`: `par(1)` is
`n_par(1, OpMulInt(n_seed(), 2), n___dflt_par_v(__ref_1))`.  The scalar is inlined; the vector
is the call, with a `__ref_N: vector<integer>` minted on pass 2, one per call site.  A default
that reads an earlier parameter is handed the caller's actual argument:
`n_grow(x, n___dflt_grow_v(x, __ref_1), n___dflt_grow_t(x, {#default ref … }))`.
