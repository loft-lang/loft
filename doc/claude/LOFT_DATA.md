# Loft Collections, Structs and Calls

Vectors, key-based collections, structs and record initialization, methods and call spellings, assertions, `sizeof` and random numbers.  Part of the language reference: [LOFT.md](LOFT.md).

---

## Vectors

```
v = [1, 2, 3]               // create with literal
v: vector<integer> = []     // empty vector with type annotation
buf: vector<single> = []    // empty vector of f32
v += [4]                    // append one element
v += [5, 6]                 // append multiple elements
for x in v { }             // iterate
v[i]                        // index; i>=len -> null, negative i counts from the end (see below)
v[start..end]               // slice range (end exclusive)
v[start..=end]              // slice range (end inclusive)
v[start..]                  // open-ended slice to end
v[..end]                    // open-start slice from 0 to end (exclusive)
[elem; 16]                  // repeat initializer: 16 copies of elem
[for n in 1..7 { n * 2 }]  // vector comprehension (builds [2, 4, 6, 8, 10, 12])
[for n in 1..10 if n % 2 == 0 { n }]  // comprehension with filter
```

**A vector can also be taken apart by `match`** — `[first, ..rest]`, `[a, .., z]` and the other
slice patterns are in [§ Slice patterns](LOFT_CONTROL.md#slice-patterns-matching-a-vector).

**The vector built-ins are stdlib methods on `vector`** — `reverse`, `reserve`, `insert`,
`sort`, `filter`, `map`, `reduce`: `reverse(v)` and `v.reverse()` are one call (as are
`filter(v, f)` / `v.filter(f)` and the rest), and a program defines its own `reverse` for its own types beside it
(`fn reverse(self: Roster)`), one definition per receiver type; one for `vector` itself is
refused, as for any stdlib method.  They are calls in every respect: `insert`'s index and
element are values before the vector grows, so `v.insert(0, v[1])` inserts the element that
was at index 1, and the element converts as any element store does (`2` into a
`vector<float>` is `2.0`; `300` into a `vector<u8>` is refused).  `sort` takes any element
with a `<` (`Ordered`): a struct defining `op <` sorts by it, stably, and a null sorts first.
`reduce` folds into any accumulator — a number, a `text`, a struct, a vector — and takes any
function value, a fn-ref variable or a capturing lambda included.

**Slices are iterators, and a vector wherever a vector is expected.**  `v[lo..hi]`
can be used in `for x in v[lo..hi] { … }` and wherever an iterator is accepted
without building anything; wherever a `vector<T>` is expected — a local
(`sub = v[lo..hi]`), a function argument (`f(v[lo..hi])`), a return, a struct
field's value, an element of a literal — it is the fresh vector that local
would hold, independent of `v`.  Only a KEYED range slice (`sorted`, `index`,
`trie`) stays a `for`-only iterator.

**A slice of read-only bytes is a view, and read-only itself.**  A `vector<u8>` that
came from `file_map` (or from a library's buffer) is read-only, and `sub = v[lo..hi]`
on it copies nothing: `sub` reads the same bytes in place, for as long as it is bound,
however `v` is rebound or dropped meanwhile.  Reading it is what reading any vector is.
Writing it (`sub[0] = 1`, `sub += [2]`) is refused with the message a write into the
mapping gets, and the cure is the same: bind it (`w = sub`), which copies, and write `w`.
An empty slice is an ordinary empty vector.

**Negative slice bounds count from the end (@P384).**  `v[2..-1]` is
"element 2 up to (not including) the last": on `[10, 20, 30, 40, 50]`
it yields `30, 40`.  `v[-2..]` yields the last two elements.  A
negative bound is shorthand for `len(v) + bound`, so `v[0..len(v) - 1]`
and `v[0..-1]` are the same slice.

**Scalar `v[i]` follows the same negative rule — mind the null-guard footgun.**
The full picture: `i ∈ [0, len)` → the element; `i ≥ len` → `null`; a **negative**
`i ∈ [-len, -1]` counts **from the end** (`v[-1]` is the last element, `v[-len]` the
first — the same rule as negative slice bounds above); `i < -len` → `null`.  Because a
negative index in range yields a real element, a *computed* index that can go negative
does **not** null-guard: `if v[i] { … }` and `v[i] ?? d` only catch `i ≥ len`, not a
`-1` "not-found" sentinel or a subtraction underflow.  Test `if i >= 0` first (or `?? d`
only after a `>= 0` check) when `i` may be negative.

**Struct elements: reads are views, writes are copies — never swap
in-place via a temp (#338).**  For a `vector<STRUCT>`, `tmp = v[j]` yields
a dep-tracked LINK to slot `j`'s record (writes through `tmp` mutate
`v[j]`); but `v[j] = v[k]` COPIES `k`'s record bytes into slot `j`'s
storage.  The classic swap therefore silently corrupts:

<!-- from tests/reference/vector-views.loft -->
```loft
tmp = v[j];     // a view of slot j
v[j] = v[k];    // copies k's record INTO slot j — tmp now reads k's record
v[k] = tmp;     // writes k's record back: j's record is LOST, k's DUPLICATED
```

Swap through scalar temps per field, rebuild into a fresh vector
(selection sort instead of in-place insertion sort), or copy the record
explicitly through a fresh struct literal before overwriting the slot.

**A view lasts only as long as the place it names, and loft tells you when it
stops.**  `tmp = v[j]` (and a struct-typed field read, `w = o.inner`) is a link
into the container, so it stays valid only while that place does.  Where the
compiler can see the place will not survive the binding, it gives `tmp` its own
copy instead — taken at the bind, so it holds the value you bound — and says so:

```
advice: in `f`, `c` was copied out of `bx` because `bx` is reassigned while `c`
  is in use — a view names a place inside `bx`, and giving `bx` a new value
  leaves nothing for it to point at. Writes through `c` no longer reach `bx`.
```

Four things end the place: **removing** from the container (`v.remove(i)`
renumbers the rest), **growing** it (`v += [x]`, an insert, a keyed add — a
container that outgrows its room moves every element), **writing a key field**
through the view on a keyed collection, and **reassigning the container itself**
(`bx = T{…}`).  Each counts where it happens: in this function, or in a function it
calls with the container.  After the
copy, writes through the binding no longer reach the container — which is why it
is reported and not silent.  To keep writing through, re-read the view after the
change (`c = bx.v[0]`).

The copy happens only when you still USE the view after the change — *"while `c`
is in use"* is meant literally.  Finish with the view first and it keeps writing
through, so moving the last use above the removal is a second way out:

<!-- from tests/reference/vector-views.loft -->
```loft
c = v[0];  c.n = 99;  v.remove(2);   // no copy — `c` is done before `v` changes
```

whereas a view still in use after the change is copied first, and the late write lands in the copy:

<!-- from tests/reference/vector-views.loft -->
```loft
c = v[0];  v.remove(2);  c.n = 99;   // copied — `c` is used after `v` changed
```

Overwriting a place is not ending it: `o.inner = Box{…}` writes into the place
`o.inner` already occupies, so a view of it sees the new value and still writes
through.

**Write `&` and you get an error instead of a copy.**  `c = &v[0]` says *"I want a
live link"* — an ownership decision, not a hint — so loft will not quietly hand you a
copy.  Where it cannot honour the link, it refuses the program instead.  All four
things that end a place do this: removing from the container, growing it, writing a
KEY field through the reference, and replacing the container itself.

```
error: cannot remove from `v` while `c` references an element of it — a removal
  renumbers the remaining elements, so a write through `c` would no longer reach
  the element it names. Move the removal after the last use of `c`, or bind
  without `&` to work on a copy
```

The same refusal covers a **call**: passing an element of a container and the
container itself to a function that removes from it (`shift(v[2], v)`) is rejected,
whether or not the parameter is spelled `&` — a struct parameter names the caller's
element either way, so the write would be lost either way.  Pass the INDEX instead
and read the element again after the removal:

<!-- from tests/reference/vector-views.loft -->
```loft
fn shift(idx: integer, all: &vector<Box>) { all.remove(0); all[idx - 1].n = 99; }
```

For a keyed collection the remedy is to re-insert rather than to reorder, because the
key write IS the thing that cannot be honoured — changing a key would leave the element
reachable by no key at all:

<!-- from tests/reference/vector-views-refused.loft -->
```loft
c = &s[30];  c.key = 5;      // refused
```

A keyed record moves by ASSIGNMENT under the new key, the old one removed.  The subscript of
an assignment names the key: a copy of the record is placed under it, with its key field set
to the subscript, whatever key the record carried (`Col-Assign`, loft#1716):

<!-- from tests/reference/vector-views.loft -->
```loft
s[5] = s[30];  s[30] = null; // the subscript names the key: a copy under 5, then 30 removed
```

**Why an error rather than some defined behaviour.**  loft may always drop an error
later, but never add one after the language freezes — so refusing keeps the door open.
If loft one day gains the machinery to honour these references properly, every program
that compiles today still compiles and the refused ones start working.  Had it shipped
the silent copy instead, that copy would be the contract forever.

Full rule + the reasoning:
[OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not).

**Empty vectors** require a type annotation so the compiler knows the element type.
Use `v: vector<T> = []` instead of the older `[for _ in 0..0 { default }]` pattern.

To remove elements while iterating, use `v#remove` inside a filtered loop (see [For loops](LOFT_CONTROL.md#for-loops)).

---

## Key-based collections (hash / index / sorted)

All three keyed collection types support single-element removal by assigning `null` to a subscript:

<!-- from tests/reference/keyed-collections.loft -->
```loft
h[key] = null;          // hash: remove element whose key field equals key
idx[nr, name] = null;   // index: remove element by compound key
s[key] = null;          // sorted: remove element by key field
```

Removing a key that is not present is a **no-op** (safe, no error).

`sorted` and `index` collections support forward and reverse iteration:
<!-- from tests/reference/keyed-collections.loft -->
```loft
for v in sorted_col { fwd += v.n; }         // forward — visits elements in key order
for v in rev(sorted_col) { bwd += v.n; }    // reverse — visits elements in reverse key order
```

Lookup also returns `null` when an element is absent:
<!-- from tests/reference/keyed-collections.loft -->
```loft
if h[key] { found = true; }
elem = idx[42, "foo"];    // null if not present
```

**All keyed-collection subscripting is by KEY, never by position (C99).** On a
`sorted` / `index` / `hash`, `coll[k]` looks up the element whose key **equals**
`k` — not the element at position `k` — so `s[20]` finds key 20 and `s[1]`
returns `null` when there is no key 1 (it is not position 1). On an ordered
`sorted` / `spatial`, a **range** subscript is likewise key-addressed: `s[lo..hi]`
is a **key-range** query (`s[15..35]` selects the elements whose key is in
`[15, 35)`, not positions 15..35), and `spatial` uses the same form for proximity
(`xs[(x,y)..(x2,y2)]`). This is the same key-addressing as `s[k]` and
`s[k] = null` — it only *looks* like a vector's positional slice `v[1..3]`.
**Porting a positional `v[1..3]` to a sorted changes its meaning**; use the key
range you want, or iterate and count positions yourself.

**Gotcha (INC#2) — vector has a richer literal/build API than the keyed
collections.** All four collection types share `+=`, `for` iteration
(hash iterates via its internal ordered index), and subscript removal
(`h[key] = null`), but **comprehensions** (`[for x in c if p { … }]`)
produce only a `vector<T>`; there is no `sorted<T>` / `index<T>` / `hash<T>`
comprehension form — instead, build a vector literal and assign it to a
keyed-collection field, which does implicit conversion.  Inside a `for`
loop, `#index` is valid on vector and sorted but a compile error on index
collections.  When porting code between collection types, treat these gaps
as structural differences rather than bugs — they are intentional and not
planned to close.

**`spatial<T[x,y]>` / `spatial<T[x,y,z]>` (1–3 coordinate axes, @PLN48) is a
related keyed collection** backed by a Morton/Z-order radix tree.  It shares
`+=` append, `for` iteration (visited in the tree's natural Morton order —
no sort, unlike `hash`), `.len()`, and the full point subscript: `xs[x, y]`
reads the record at exactly that point (`null` when empty), `xs[x, y] = mob`
inserts-or-replaces, and `xs[x, y] = null` removes — the same three roles
`h[k]` has on a `hash`.  Note the point subscript takes the coordinates as
separate subscripts (`xs[3, 6]`), where the range forms below parenthesise
them.  Proximity queries use range-slice
syntax instead of new keywords or methods: `xs[(x1,y1)..(x2,y2)]` is the
bounding box and gives exactly what is inside it, while
`xs[(x,y)..]` and `xs[(x,y)..:n]` walk OUTWARD from that point, nearest
first — two cursors seeded either side of the query, so `..:n` answers `n`
records from any origin and a query past every record still answers its
neighbours.  The walk is
APPROXIMATE: it orders by Morton distance, which jumps at quadrant
boundaries, so a truly-near point can arrive a little late.  Reach for a
symmetric box, `xs[(x-r, y-r)..(x+r, y+r)]`, when the answer must be exact.
See [STDLIB.md § Keyed collections](STDLIB.md#keyed-collections-hash--index--sorted)
for the full syntax table.

**`trie<T[field]>` keys on ONE text field** and shares the same radix tree as
`spatial` — only the key oracle above it differs, so `spatial` is coordinates and
`trie` is bytes.  It shares `+=` append, `for` iteration (in key order, no sort),
`.len()`, and the exact subscript `t[key]` (`null` when absent — never a
neighbour).

What it offers that `sorted<T[text]>` cannot is the **prefix**:

```
for w in words["kerk"..]    { … }   // every key beginning with "kerk", in key order
for w in words["kerk"..:20] { … }   // the first 20 of them
```

The prefix IS the query.  A `sorted` range needs a successor string
(`words["kerk".."kerl"]`) that the caller has to construct, gets wrong at a byte
boundary, and which answers a key INTERVAL rather than a prefix — so `t[a..b]` is
refused, and it names `sorted` as the kind that answers an interval.  Key order is
BYTE order and the terminator sorts before any byte, which is why `kerk` precedes
`kerkstraat` precedes `kerkweg`.  Exactly one key field: a trie orders one key's
bytes, so several keys have no order to share (use `sorted<T[a, b]>` for that).

### Two collections over one element type are ONE record set

Declare two collections over the same element struct in one struct and you get **two routes to
a single set of records**, not two collections. Filling either fills both:

<!-- from tests/reference/two-collections.loft -->
```loft
struct Tile { k: integer, n: text }
struct Level { tiles: vector<Tile>, by_key: hash<Tile[k]> }
```

An append through one member is visible through the other:

<!-- from tests/reference/two-collections.loft -->
```loft
lvl = Level { };
lvl.tiles += [Tile { k: 7, n: "gate" }];
// len(lvl.by_key) is 1 — the hash is a VIEW of the same record
```

That is the point of the feature: the keyed view stays in step with the list with no code to
keep it in step, and it works whichever member you spell the insert through.

**When it happens — one line:** two or more collections over the same element type, in the same
struct **or struct-enum variant**, **at least one of them keyed** (`hash` / `sorted` / `index` /
`spatial` / `trie`). Field order does not matter, and neither does whether the vector's element
is nullable (`vector<Tile?>`). Two plain vectors over one element type are **independent** — a group needs a
keyed member.

**When you want them apart**, say so in the TYPES — there is no per-field opt-out:

| written this way | result |
|---|---|
| a second element **struct** (`vector<Chosen>` beside `hash<Tile[k]>`), fields identical | **independent** |
| the two collections in **different structs** | **independent** |
| both as **locals** rather than struct fields | **independent** |
| `type Chosen = Tile;` then `vector<Chosen>` | ⚠ **still one group** — an alias names the same type |

The second struct is the escape, and it costs a field copy to move between them:

<!-- from tests/reference/two-collections.loft -->
```loft
struct Chosen { k: integer, n: text }
fn to_chosen(t: Tile) -> Chosen { Chosen { k: t.k, n: t.n } }

struct Picks { by_key: hash<Tile[k]>, picked: vector<Chosen> }
```

⚠ **Nothing at the declaration tells you which you got** — a group that did not form looks
exactly like an empty one, so `len(view) == 0` is the symptom to recognise. Details, teardown
and the record-ownership rules: [DATABASE.md § Clearing one member of a linked group](DATABASE.md).

---

## Structs and record initialization

Named form (recommended; type is explicit):
```
point = Point { x: 1.0, y: 2.0 }
```

Anonymous form (type is inferred from context):
```
point = { x: 1.0, y: 2.0 }
```

Fields not specified get their `= expr` default, or the zero value for their type.
Nullable fields default to `null`. A **bare (non-`Optional`) enum field is the one exception**: it
has no zero value (an enum's 0 is `null`, which a non-null field may not hold), so omitting it is a
compile error — provide it, give it `= <variant>`, or type it `E?` (@PLN116).

**A nullable STRUCT-enum (`Shape?`) works** as a local, a parameter, a return, a struct FIELD and
a `vector<Shape?>` element — `= null`, `== null`, truthiness, `??`, `match` and reassignment in
both directions all behave (loft#1065, loft#1071). How absence is STORED follows the slot, as the
null model says it should: a handle carries the reference sentinel, while an inline slot is a
four-byte record pointer where absence is `0`.

Iterating one works too: in `for e in v { e == null }` the loop binding is a sub-reference to the
element slot, and the test reads that slot's word rather than a handle's sentinel.

Field access uses `.`:
```
point.x
arg.long.len()
```

### Shared field names

Field names are type-scoped, not globally unique.  Different structs and enum
variants can share a field name — the compiler resolves the correct field by
the type of the receiver:

<!-- from tests/reference/shared-fields.loft -->
```loft
struct Point { x: float, y: float }
struct Rect { x: float, y: float, w: float, h: float }
```

Each type's `x` is its own field, at its own offset:

<!-- from tests/reference/shared-fields.loft -->
```loft
p = Point { x: 1.0, y: 2.0 };
r = Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 };
assert(p.x == 1.0, "Point's x");
assert(r.x == 10.0, "Rect's x (different offset, same name)");
```

This also works between struct-enum variants:
<!-- from tests/reference/shared-fields.loft -->
```loft
enum Shape {
  Circle { radius: float, label: text },
  Square { side: float, label: text }
}
```

and a shared name reads on either variant:

<!-- from tests/reference/shared-fields.loft -->
```loft
c = Circle { radius: 5.0, label: "big" };
s = Square { side: 3.0, label: "small" };
assert(c.label == "big", "the Circle's label");
assert(s.label == "small", "the Square's label");
```

Verified: works in vectors (`pts[0].x`), function parameters, and across
struct/enum boundaries.  See `tests/scripts/23-field-overlap-structs.loft`
and `tests/scripts/24-field-overlap-enum-struct.loft`.

### Value structs (`value struct`)

A `value struct` is a struct with **value (copy) semantics** and **zero heap overhead** as a
field of another record or as a vector element (@PLN101).  Reading one out of a field or element
yields an independent copy — mutating the copy never writes back through the source:

<!-- from tests/reference/value-structs.loft -->
```loft
value struct Point { x: integer, y: integer }

struct Path { points: vector<Point> }
```

An element bound from the vector is a copy:

<!-- from tests/reference/value-structs.loft -->
```loft
p = Path { points: [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }] };
q = p.points[0];   // q is a COPY of element 0
q.x = 99;          // p.points[0].x is still 1 — no aliasing
```

- **Copy, not alias.** A plain `struct` binding is a *view* (a later `p.points[0].x = 9`
  is seen through it); a `value struct` binding is a snapshot.  This is the whole difference —
  everything else (operators via `OpXxx` methods, `{v}` / `{v:spec}` formatting, `as`
  conversions) works exactly as for a reference `struct`.
- **Zero-cost inside records and vectors.**  Fields and elements are stored inline (no per-field
  or per-element allocation), and a read-only use (e.g. `for pt in p.points { s = s + pt.x }`)
  is left as a zero-cost view — a `vector<value struct>` allocates the same as the raw scalar
  layout, flat in the element count.  A standalone local may own its own store (negligible; a
  loop reuses the slot).
- **Non-null.**  A value struct is inline bytes with no null sentinel: `value struct?` is a
  compile error, and every value struct must be initialised (there is no null value struct).
- **Method params stay zero-copy.**  `self` / `both` on a value-struct method is passed by
  reference (no copy); the copy only happens when you *bind* a value struct out of a view.

Use a `value struct` for small wrapper types that must be as cheap as the raw field they wrap
(e.g. `DateTime { ms: integer }`, `Point`, `Color`) — where the copy semantics of a scalar are
what you want.  Use a reference `struct` when you want shared/aliased mutation or nullability.

### First-grade custom types — operators, formatting, `as` (@PLN99)

A user `struct` (or `value struct`) can behave exactly like a built-in across three surfaces —
this is what makes a wrapper type (`DateTime`, `Money`, `Colour`, a `Decimal`, a URL) ergonomic
rather than a bag of functions.  Mark the type and these functions `pub` to use them across a
`use` boundary.

- **Operators** — define `fn OpLt(self: T, other: T) -> boolean` (and `OpLe/OpGt/OpGe/OpEq/OpNe`),
  `fn OpAdd/OpMin/OpMul(self: T, …) -> …`, etc., and `a < b` / `a - b` dispatch them **directly**
  (not only inside `<T: Ordered>`).  `OpMin` is the `-` operator (subtraction), `OpAdd` is `+`.
  **Operators key on `(OpName, receiver type)`** — you cannot overload the *same* operator by the
  *second* operand's type (e.g. one `OpMin(T, T)` and one `OpMin(T, U)` collide); give the second
  form a named method instead.  A type with no such op errors as before (`dt + 5` stays a compile
  error — distinct-type safety is free).
- **Scope end** — define `fn OpDrop(self: T)` and it runs when the value's OWNER dies: the
  binding's own scope exit, the early-`return`/`break` paths, reverse-declaration order within a
  scope (@PLN125 arc B), and a REASSIGNMENT of the binding, which releases the record it
  displaces once the new value is computed.  Copying a droppable into a struct field, an enum payload or a
  collection element MOVES it — the source stops dropping, and the container's death releases
  what it holds, its own hook first and then its members (@PLN139).  A plain whole-value copy
  (`t = s`, the variable an `if` arm yields, `t = s; return t`) moves it the same way: the
  copy releases, the source does not; a copy off a PARAMETER leaves the caller as the owner
  ([INTERFACES.md § `OpDrop`](INTERFACES.md)).  Taking a value back OUT
  (`v.remove(i)`, `v[i] = other`) does not release it.  A drop **cannot fail** (C80 — no caller
  is left to tell), so it may not return and anything whose failure matters stays an explicit
  call (`tx.commit()` answers, the closing brace does not).  It receives only `self`, whose data
  is COPIED at construction, so its effect reaches the world (I/O, a `#c` handle it owns) rather
  than a caller's collection.  Full contract, including what it deliberately does NOT do:
  [INTERFACES.md § `OpDrop`](INTERFACES.md).
- **Indexing** — define `fn OpIndex(self: T, i: τ) -> υ` and `x[i]` dispatches it, so a matrix, a
  bitset, a row or a ring buffer reads as `x[i]` rather than `x.at(i)` (@PLN125 arc C).  The index
  type is whatever the method declares — a row addressed by column NAME takes a `text`.  An
  interface requires it as `op [] (self: Self, i: τ) -> υ`.  `OpIndex` READS: `x[i] = …` is refused
  (a type that must be written through offers a setter, `x.set(i, v)`).
- **Formatting** — define `fn to_text(self: T, spec: text) -> text`.  Then `"{x}"` calls it with
  `spec == ""` and `"{x:anything}"` passes `"anything"` raw — the type owns its whole spec
  vocabulary (the Python `__format__` model; core learns no date/money tokens).  *Known issue
  (#533): today the body must not be a bare tail `if` — bind the result to a local and return it
  (`r = if … else …; r`), else the branch mis-selects.*
- **Conversions** — define `fn OpConvTFromS(v: S) -> T` (e.g. `OpConvDateTimeFromText`), and
  `s as T` dispatches it: `"2026-07-08" as DateTime`, `"#ff0000" as Colour`, `"1.5" as Decimal`.
  With no matching conversion, `as T` is a clean compile error (not a silent mis-cast).

**When to reach for this — and which library types still should.**  A type earns
the full treatment when it hits all four axes `DateTime` does: **(1)** it is one
value or a small fixed bundle (so `value struct`'s zero-cost copy fits), **(2)**
it has arithmetic or ordering meaning (→ operators), **(3)** it has a canonical
text form (→ `to_text`), **(4)** it converts to/from a primitive or text (→ `as`).
`DateTime` / `Duration` (the `time` lib) are the shipped exemplars.  Prime
un-upgraded candidates in the current libraries, highest-leverage first:

- **`Colour`** — today a bare packed `integer` in `graphics` / `imaging` with
  hand-rolled `rgb()` / `color_r()` free functions (the exact pre-`DateTime`
  shape).  A `value struct Colour { packed: integer }` with blend/scale
  operators, a `{c:#}` → `#RRGGBB` `to_text`, and `Colour ↔ integer` / `↔ text`
  conversions packs **flat** in pixel buffers (no heap cost) and is the clearest
  next dogfood — it exercises every part of the machinery, as `DateTime` did.
- **`Vec2` / `Vec3` / `Rect` / `Point`** — today plain **heap** structs with
  `add3` / `scale3` / `dot3` free functions; as zero-cost value structs with
  `+` / `*` / `dot` operators they allocate flat in a vector, so the win lands
  exactly where you make thousands of them (meshes, particles, physics).
- **`Version`** (registry semver — hand-rolled parse + compare), **`Angle`**, and
  a units family (`ByteSize`, `Money` / `Decimal`) round out the list.

Guards: `tests/scripts/511-first-grade-operators.loft`, `tests/scripts/512-first-grade-format.loft`, `tests/scripts/513-first-grade-conversions.loft`.

---

## Methods and function calls

Functions whose first parameter is named `self` can be called with dot syntax:
```
text.starts_with("prefix")
text.to_uppercase()
```

Otherwise they are called as free functions:
```
len(collection)
round(PI * 1000.0)
```

**Gotcha (INC#8) — a `self` function takes both spellings; a plain one takes
one.**  A function whose first parameter is `self` answers `x.f(…)` AND `f(x, …)`
— the second resolves by the receiver's type — and a package's method is imported
by name like any function (`use lib::(f)`).  A plain first-parameter name makes a
free function, and its method spelling is refused by name.  **A `self` method is
not a fn-ref value**, so it cannot be handed to `map`/`filter` or to a parameter of
function type — the refusal names the cure, wrap it in a lambda, `map(v, |q| { q.m(…) })`
(guard `tests/error_messages/cases/56_method_is_not_a_fn_ref.loft`).  In
the standard library `len(v)`, `abs(n)`, `text.starts_with(s)` are `self`
functions and callable either way; `sum_of(v)` and `print(s)` are free-only.  When
in doubt, try the free form first — it works for both kinds.

**A `&` parameter calls like the value it references.**  `&` is how an argument is
PASSED, not a different type, so inside `fn f(v: &vector<integer>)` the name `v` is
the vector and every call form it supports works on it — `len(v)`, `v.len()`,
`size(v)`.  The same holds for `&text`, the keyed collections, a `&Struct` and a
`&integer`.  There is nothing to unwrap first (loft#824):

<!-- from tests/reference/ref-params.loft -->
```loft
fn total(v: &vector<integer>) -> integer {
  v += [9];        // the append reaches the caller's vector
  len(v)           // …and the length is the vector's, not a reference's
}
```

Note the trade the `&` asks for: it earns its place only when the function writes
through it.  A helper that just reads is told *"Parameter 'v' has & but is never
modified; remove the &"* — drop the `&` and the by-value signature reads the same.

### Both call spellings, one definition

A `self` function is callable as a method and as a free function:

<!-- from tests/reference/methods.loft -->
```loft
pub fn exists(self: Doc) -> boolean {
  self.format != "missing"
}
```

Both spellings call it:

<!-- from tests/reference/methods.loft -->
```loft
// Can be called as:
a = f.exists();      // method syntax
b = exists(f);       // free function syntax
```

Both spellings exist on purpose: `v.sin()` for a programmer whose fingers learned
the Rust convention, `sin(v)` for one who never did — neither is forced on the
other, and they reach the same function.

**`both` is deprecated.**  A first parameter named `both` used to be the spelling
for "method and free function"; `self` now does all of it, including an import by
name (`use lib::(f)` brings in the methods `lib` declares under that name), so `both`
names nothing more.  It still compiles and means exactly `self`,
with the warning `both-receiver-deprecated` — rename the parameter to `self`.

**One name, one body per type.**  A method and a free function with the same name
whose first parameter has the same type are refused, whichever is declared first:

```
struct Pt { x: integer }
fn doit(self: Pt) -> integer { self.x + 1 }
fn doit(p: Pt) -> integer { p.x + 2 }   // error: Cannot redefine 'doit' … declare it once as a `self` method
```

Otherwise `p.doit()` and `doit(p)` would run different code — and before the
refusal the free one was silently unreachable, because `doit(p)` resolves to the
method.  The one `self` function already takes both spellings.  A free function
on a different type (`fn doit(q: Qt)`) is an ordinary overload and stays legal.
The rule is `formal/calls.md (F-OneBody)`.

### Named arguments

Any parameter can be passed by name using `name: value` syntax.  Positional arguments
come first; once a named argument appears, all subsequent must be named.  Parameters
not provided must have a default value.

```
fn connect(host: text, port: integer = 8080, tls: boolean = true) -> text
connect("example.com")                         // all defaults
connect("example.com", tls: false)             // skip port
connect(host: "example.com", port: 443)        // all named
```

Both spellings of a call take names, including the method one — `cfg.render(dry: true)`
and `render(cfg, dry: true)` are the same call.  The receiver is argument 0, so naming
it (`render(self: cfg)`) is the one thing that does not work: it is already provided.

A default is an **expression**, evaluated at the call rather than stored as a constant.
It runs once per call, not at all when the caller supplies the argument, and it may read
a parameter declared before it:

```
fn window(rows: integer, height: integer = rows * 10) -> integer { height }
window(4)      // 40 — the default reads `rows`
window(4, 7)   // 7  — the default is not evaluated
```

A default is **not part of the function's type**.  Adding one to an existing function
keeps every direct call working, but a fn-ref of type `fn(integer) -> integer` stops
matching the moment a second parameter arrives however optional it is — so growing the
signature of something handed out as a VALUE is a breaking change ([INTERFACES.md](INTERFACES.md)).

Catalogue: @F17.

---

## Assertions

```
assert(condition)
assert(condition, "message")
```

Panics at runtime if the condition is false.

Catalogue: @F44; guard: `tests/scripts/1147-assert-eq-reports-both-sides-at-the-call-site.loft`.

---

## Sizeof

```
sizeof(integer)    // 8
sizeof(u8)         // 1 (packed field size)
sizeof(u16)        // 2
sizeof(MyStruct)   // sum of packed field sizes
sizeof(my_var)     // size of the variable's type
```

`sizeof(TYPE)` returns the packed byte size used when the type is stored as a struct
field or vector element. For range-constrained integer types (`u8`, `u16`, etc.) this
is the packed size (1 or 2 bytes), not the stack slot size. For polymorphic enums and
references, the size is computed at runtime from the actual variant.

Catalogue: @F45; guard: `tests/scripts/89-sizeof.loft`.

---

## Random numbers

Three functions for pseudo-random integer generation. All use a thread-local PCG64 generator.

```grammar
rand_seed(seed: integer)                   // seed the generator
rand(lo: integer, hi: integer) -> integer  // uniform in [lo, hi]; null if lo > hi
rand_indices(n: integer) -> vector<integer>// shuffled [0..n-1]
```

`rand_seed` makes sequences reproducible:

<!-- from library:random -->
```loft
rand_seed(42);
a = rand(1, 100);  // same value every run with seed 42
```

`rand_indices` is the idiomatic way to randomly visit all elements of a collection:

<!-- from library:random -->
```loft
rand_seed(7);
items = ["a", "b", "c"];
for i in rand_indices(len(items)) { println(items[i]) }
```
