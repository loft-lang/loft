# Standard Library: Parallel, Reflection, Environment, Time, Random

`par`, reflection, command-line arguments, environment, memory diagnostics, time and random numbers.  Part of the standard library reference: [STDLIB.md](STDLIB.md).

---

## Parallel

The public parallel API is the `par(...)` for-loop clause. The internal functions `parallel_for` and `parallel_for_int` are not part of the user API.

Function references (a function's bare name, type `fn(T) -> R`) are first-class callable values — they can be stored in variables, passed as parameters, and called directly (`f(args)`), not only as `par(...)` worker arguments. See [LOFT_LITERALS.md](LOFT.md) § Literals for the full syntax.

### `par(...)` Parallel For-Loop

```grammar
for a in <vector> par(b=<worker_call>, <threads>) {
    // body — b holds the worker result for this element
}
```

Two worker call forms:

| Form | Example | Description |
|---|---|---|
| Form 1 | `func(a)` | Global function called with the loop element |
| Form 2 | `a.method()` | Method on the element type |

Supported return types: `integer`, `float`, `single`, `boolean`, inline `enum`, `text`.
Extra context arguments are forwarded: `par(b=scale(a, mult), N)`.
Input must be a `vector<T>`.

<!-- from tests/reference/par-loop.loft -->
```loft
struct Score { value: integer }
struct Scores { items: vector<Score> }
```
<!-- from tests/reference/par-loop.loft -->
```loft
fn double_score(r: const Score) -> integer { r.value * 2 }
fn get_value(self: const Score) -> integer { self.value }
```

Both forms deliver each worker's value to the body, in the original order:

<!-- from tests/reference/par-loop.loft -->
```loft
q = make_scores();   // vector of Score

// Form 1: global function
sum = 0;
for a in q.items par(b=double_score(a), 4) {
    sum += b;
}

// Form 2: method
total = 0;
for a in q.items par(b=a.get_value(), 1) {
    total += b;
```

**Worker function rules:**
- Accept a single `const` reference as the first parameter.
- Do not name the first parameter `self` (this makes it a method, looked up differently).
- Workers receive a read-only store snapshot; writing to input data panics.
- No nested parallelism.

**Limitations:**
- Float/integer result accumulation in the loop body: if `b` is float/integer, using it in arithmetic with a pre-declared float/integer variable can trigger a first-pass type-inference conflict. Workaround: use `b` only in boolean comparisons or cast (`sum += b as integer`).
- Implementation: `src/parallel.rs`; see [THREADING.md](THREADING.md) for internals.

---

## Reflection

The declared shape of a type, as data — what a generic serialiser, an ORM mapping
or a schema check needs.  Declared in `default/07_reflect.loft`.

| Function | Description |
|----------|-------------|
| `type_of(x) -> TypeInfo` | The declared shape of `x`'s type. **The argument is read for its TYPE and is not evaluated** (the contract C's `sizeof` has), so pass a variable, a field or a parameter rather than an expression with a side effect. |
| `type_named(name: text) -> TypeInfo?` | The declared shape of the type called `name`, or **null** when this program has no such type. Use when the name is a runtime value — a config file, a database catalogue, a command line — so there is nothing to call `type_of` on. |

`TypeInfo` carries `name`, `kind`, `size` (bytes per record), `fields`,
`variants`, `element`, `collection` and `keys`; each `FieldInfo` carries `name`,
`type_name`, `position`, `kind` and `nullable`.  Match on `kind` first: only a
record and a struct-enum variant have `fields`, only an enum has `variants`, and
only a vector or a keyed collection names an `element`.  Empty is the honest
answer for a kind that has no such thing.

<!-- from tests/reference/reflection.loft -->
```loft
t = type_of(row);
println("{t.name} ({t.size} bytes)");
for f in t.fields { println("  {f.name}: {f.type_name} @{f.position}") }
```

`TypeKind` is `IntegerKind` · `LongKind` · `SingleKind` · `FloatKind` ·
`BooleanKind` · `TextKind` · `CharacterKind` · `RecordKind` · `EnumKind` ·
`VariantKind` · `VectorKind` · `KeyedKind` · `RefKind` · `OtherKind`.  A kind
this loft version has no name for is reported as `OtherKind`, never guessed at.

### A keyed collection: which one, and on which fields

`KeyedKind` says a type is walked by cursor and stops there, which is not enough
to derive a query FROM.  `collection` says which of the five it is —
`KeyedHash` · `KeyedIndex` · `KeyedSorted` · `KeyedOrdered` · `KeyedRadix`, and
`NotKeyed` for every other type — and `keys` lists its key fields **in key
order**, each a `KeyInfo` of `name`, `position` and `ascending`.

<!-- from tests/reference/reflection.loft -->
```loft
t = type_of(people);                       // hash<Person[id]>
if t.collection == KeyedHash {
  for k in t.keys { println("key {k.name} @{k.position}") }
}
```

- **`position` is the ELEMENT record's byte offset** — the same number that
  element type's `FieldInfo.position` carries, so a key joins to a field by
  value.  A name is a display fact; the position is what the collection keys on.
- **`ascending` is `true` for a kind with no order of its own** (`KeyedHash`,
  `KeyedRadix`), because there is no descending answer to give.  Match
  `collection` first where "ascending" and "unordered" differ: only the three
  ordered kinds can serve a range.
- **`KeyedSorted` vs `KeyedOrdered`** is the same declaration over a different
  structure: `sorted<T[…]>` is a tree, and becomes a sorted by-value vector
  (`KeyedOrdered`) when `T` is co-located by a `hash` / `index` / `spatial`
  field elsewhere.  Both are ordered.
- **`keys` is delivered whole or empty.**  A query built from half a composite
  key reads the wrong rows, so a key the descriptor cannot resolve to an element
  field drops the whole list — which is also what a `__nullable<S>` element
  reports, because its keys live in the `Some` payload rather than in the
  element node.

Two limits worth knowing before you reach for it:

- **Not inside a generic.**  A generic body is parsed once against its type
  variable, so `type_of(v)` there answers `__typevar_T` — the same reason
  `"{v:j}"` in a generic body renders `{}`.  Call it where the concrete type is
  known.
- **Storage where the declaration cannot be recovered.**  A narrow `i32` field
  reports `IntegerKind`; its width is in `size`, not in a separate kind.
  `boolean` and `character` report what was declared.

Read-only, and it describes a TYPE.  `nullable` is reported even though it is not
a layout fact — a nullable field occupies the same bytes and spells absence with
a sentinel — because the compiler records it for you; it is what a generated
`CREATE TABLE` needs for `NOT NULL`.  Whether a field is `const` is NOT reported:
it constrains loft code rather than data.  WRITING a value by field name is
deliberately still out of scope (@PLN127).

### Reading a value: `field_value`

`type_of` says a record has a `text` at byte 16; `field_value(x, position)` reads
it.  Together they are what a generic serialiser, an ORM write or a value diff
needs — neither half is enough alone.

| Function | Description |
|----------|-------------|
| `field_value(x, position: integer) -> ValueInfo` | What `x` holds at that byte position, tagged with the kind the type's own descriptor gives it. |
| `field_value(x, path: vector<integer>) -> ValueInfo` | The same, reached through a chain of INLINE record fields — `field_value(doc, [8, 0])` is `doc.origin.x`. A one-element path answers exactly what the bare position does. |

**A nested record needs the path form, and the offsets must not be added up.**
`type_of` reports a nested record's fields at positions relative to THAT record,
and a `ValueInfo` for a record field carries no handle to call `field_value` on
again — so one number cannot name `origin.x`. Summing them does not work either,
and that is the point rather than a limitation: the check that makes this
ordinary loft rather than a pointer is that a position must **begin a field of
the type it is read against**, and `origin`'s offset plus `x`'s begins nothing in
the owner. The walk therefore happens inside, one descriptor step per element,
each checked the same way.

Every step but the last must land on an inline record. A step onto a scalar, a
vector, a keyed collection or a stored reference answers `OtherKind` and reads
nothing — a stored reference names a record with its own identity, and following
it would be a pointer chase rather than a field read. An empty path answers
`OtherKind` for the same reason a bad position does.

`ValueInfo` carries `kind`, `is_null`, and three payloads — `i` (integer, long,
boolean as 1/0, character as its code point), `f` (float, single) and `t` (text).
Match `kind` to know which one carries the answer.

<!-- from tests/reference/reflection.loft -->
```loft
t = type_of(row);
for f in t.fields {
  v = field_value(row, f.position);
  if v.is_null { println("{f.name} is null") }
  else if v.kind == TextKind { println("{f.name}={v.t}") }
  else { println("{f.name}={v.i}") }
}
```

- **The position is CHECKED against the descriptor, never trusted.**  Only a
  field the type says BEGINS there is read.  A hand-made offset, a stale one, one
  belonging to a different type, the interior of a wider field, or an `index`
  element's `#left_1` bookkeeping all answer `OtherKind` with no read at all — so
  no argument reads bytes the layout did not name.
- **`kind` is the descriptor's answer, not yours.**  Passing the position of an
  `integer` field does not make the reading an integer; it makes it whatever that
  field is.  That is why the kind is reported rather than requested.
- **Three answers stay distinct**: `OtherKind` (nothing begins there), a
  non-scalar kind (a nested record, a vector, a keyed collection, a stored
  reference — nothing to read out), and `is_null` (the scalar holds loft's null).
  `is_null` is `false` for the first two, because a field that was never read is
  not a field that read as null.
- **`null` is not `""`, `0` or `false`.**  A null `text?` answers `is_null`; an
  empty `text` answers a zero-length string.  An ORM that confused them would
  write the wrong row, which is why the flag rides beside the payload rather than
  inside it.
- **A narrow field is read at ITS width**, with its own sentinel — an `i32` field
  reports `IntegerKind` and does not take its neighbour's bytes.
- **REFUSED inside a generic**, and refused rather than quietly unsupported.  A
  generic body is parsed once against its type variable, so there is no concrete
  type; left to answer, every call there reports `OtherKind`, which for an ORM is
  an EMPTY ROW rather than an error.  It is a compile error naming the type
  variable — call it one frame out and pass the values in.

The two halves have one consumer each in the tree:
`tests/scripts/pln127-reflect-consumer.loft` generates `CREATE TABLE` from a loft
struct through the type half alone, and `tests/fixtures/sqldb/round_trip.loft`
builds an `INSERT`'s values through this one (@PLN23 S5) — walking the table
definition's columns, each of which carries the byte position of the field that
fills it.

### The other way round: `for f in s#fields`

`s#fields` is SYNTAX, not a function: the loop is unrolled at compile time into
one block per field, and `f` is a `StructField` carrying `name`, `value` and
`nullable`.

<!-- from tests/reference/reflection.loft -->
```loft
for f in m#fields {
  if f.value is FvText { v } { println("{f.name} = {v}") }
  if f.value is FvInt  { v } { println("{f.name} = {v}") }
}
```

`value` is a `FieldValue` struct-enum — `FvBool` · `FvInt` · `FvLong` ·
`FvFloat` · `FvSingle` · `FvChar` · `FvText` — so the payload arrives already
typed and no position arithmetic is involved.

**Which of the two to reach for.**  `#fields` gives you per-field CODE with typed
payloads and no runtime cost; `type_of` + `field_value` give you per-field DATA
you can carry, index and compare.  Use `#fields` to *write* something per field;
use reflection when the field list itself is the value — when it has to line up
with a table definition, a wire format, or another type's fields.

Three things to know:

- **Only scalar fields are visited.**  A nested record, a vector or a keyed
  collection has no `FieldValue` payload and is skipped.  `type_of(x).fields`
  reports those, so the two lists agree on scalars and reflection is the longer
  one.
- **`nullable` says a sentinel is possible.**  The payload variants are typed
  non-null and `τ?` shares `τ`'s runtime layout, so a null field's value arrives
  inside `FvText` / `FvInt` / … as loft's null.  The flag is what tells you that
  can happen; it mirrors `FieldInfo.nullable`.
- **It is an unroll, so it costs code.**  Two work-refs per field per loop, live
  until scope exit.  Collect several answers in ONE loop rather than writing
  three loops over the same struct.

---

## Environment

Functions for interacting with the host operating system.

### Command-Line Arguments

| Function | Description |
|----------|-------------|
| `arguments() -> vector<text>` | The arguments after the program's path, in order — `loft p.loft --count x` answers `["--count", "x"]`; the program name is NOT in it (measured on all three backends 2026-09-30; this row used to say it was). |

### Environment Variables

| Function | Description |
|----------|-------------|
| `env_variable(name: text) -> text` | Returns the value of the environment variable `name`, or **`""` if it is not set** — measured on both backends. |
| `env_variables() -> vector<EnvVariable>` | Returns all environment variables as a vector of `EnvVariable` records (fields: `name`, `value`). |

> **An unset variable answers `""`, not null**, so `env_variable("X") ?? "default"`
> **never fires** — the `??` is dead code and a test written on it passes whatever
> the program does. Treating empty as absent is the caller's job:
> `v = env_variable("X"); if v == "" { v = "default" }`. (The return type is `text`,
> not `text?`, which is why this is consistent rather than a bug — but it reads as
> one, and it silently voided a real test in @PLN23.)

### Paths

| Function | Description |
|----------|-------------|
| `directory(v: &text = "") -> text` | Returns the current working directory, optionally with `v` appended as a subpath. Use to construct absolute paths relative to where the program was launched. |
| `user_directory(v: &text = "") -> text` | Returns the current user's home directory, optionally with `v` appended. |
| `program_directory(v: &text = "") -> text` | Returns the directory containing the running executable, optionally with `v` appended. |

### Memory diagnostics

| Function | Description |
|----------|-------------|
| `store_memory() -> text` | Returns a multi-line snapshot of all LIVE heap stores' internal utilisation — total capacity vs actual claimed data vs free space, record + free-block counts, **mergeable adjacent-free pairs** (free neighbours that should have coalesced), **`tail%` / `inner%`** (see below), and the largest stores by capacity with their type name and creation site (`bc:<pos>` — a bytecode position on the interpreter, mapping to source via `LOFT_LOG=static`; `0` on `--native`). Use to watch memory growth / fragmentation in a running program. See also `LOFT_STORES=log\|warn` (alloc/free trace). |
| `store_reclaim(collection) -> integer` | Give back the free space at the END of a store-rooted collection's store, and answer with the BYTES handed back (`0` when there was nothing). For a collection bound with `store_persist_bind` that is the **file** shrinking; otherwise it is memory returned to the allocator. Records never move, so every reference stays valid. It keeps an eighth of the live content as slack — the store stays in use, and one trimmed to the byte would pay a 2.33× re-grow on its next claim — so a store already at that size answers `0`, and asking twice is free. Returns `0` and changes nothing for a store that is read-only, shares another store's memory, or carries a `store_durable_seal` sidecar. |
| `store_release(collection) -> integer` | Say "everything I have written so far is finished": start writing it out to the bound file and stop holding it in memory, answering the BYTES dropped from the resident set (`0` when there was nothing to drop, when the collection is not bound to a file, or when the TARGET cannot honour the hint — the drop is `madvise` (MADV_DONTNEED), so a build without `mmap` and any non-unix target answer `0` by construction; the call is a residency hint, and a program running where it cannot be honoured is not a program that is wrong). For a GENERATOR streaming a large collection into a store bound with `store_persist_bind` — measured on a 20 000-record build, one call per record: peak memory **44.3 MB → 2.2 MB (20×) at no cost in wall clock**. Content is untouched and every reference stays valid; reading a released record re-reads it from the file at the cost of one page fault, so this is a hint that can cost speed and never an answer. **It pays when records are written in KEY ORDER and not returned to** — a build that keeps many records open at once scatters the arena with free blocks (3 691 against 10) and the allocator then keeps re-reading them, giving 1.0× instead of 20×. Not `store_reclaim`, which changes the file's LENGTH; this never does. Not a durability barrier — it asks for writeback to *start*, and `store_durable_seal` is what promises it landed. |

**`tail%` and `inner%` say WHERE a store's free space sits**, which is the
difference between free space you get back and free space you do not.

- **`tail`** — above the last record. This is what `store_reclaim` returns, less
  the eighth it leaves behind. A persisted store's image already ends at the last
  record, so the tail is arena capacity, not file bytes — until the store is
  BOUND, where it is both. ⚠ **On a bound store, MID-RUN, that tail is why the
  FILE SIZE compares nothing**: capacity grows by 7/3 and never shrinks by
  itself, so between the bind and the release the file is a rung on a ladder —
  two points a rung apart differ by 133% holding identical records, and one
  holding twice the data can be byte-identical. Call `store_reclaim` before
  reading a size in the middle of a run. The file a program LEAVES BEHIND needs
  no such call: releasing the collection hands the tail back, so the finished
  file follows its content (loft#752).
- **`inner`** — between records. It is reusable for future allocation, but it
  *is* written to the file, because the image has to span up to the last record.
  `store_reclaim` does not touch it — **loading the store does**, automatically:
  a collection read back by `store_load`, or bound to an existing file, is
  rebuilt dense when its interior is worth it (@PLN123 arc B, see
  [DATABASE.md](DATABASE.md)). So a high `inner` is a number you act on with the
  next load, not with this call.

A store built once reads `inner 0%`. One whose live set fell well below its peak
reads a high `inner` — 71% in the case behind loft#713 — and that is the part
only relocation could recover. Coalescing does not touch it: forcing a sweep took
2,700 free blocks to 6 and left `inner` unchanged at 45%, because merging free
blocks never moves a live one.

**Reading the two before calling `store_reclaim`** is the whole workflow: `tail%`
is what you would get, `inner%` is what you would not.

**You say when, and that is deliberate.** Only a live set that drops far below
its peak *and stays there* has anything to give back — whether a drop is
permanent is something the program knows and the runtime cannot infer. A
measured steady-state churn (40 cycles of +300/−300 records over a 2,000-record
hash) ends at 0.29 MB / 56% used if left alone; calling `store_reclaim` every
cycle ends denser, at 0.17 MB / 93% used, but moves **9.5 MB** of grow-and-shrink
traffic to get there — 55× the store's own size, to save 0.11 MB. Called once
after a permanent drop it costs one walk; called on a cycle it pays for a re-grow
every time.

---

## Time

Two clocks, answering two different questions. Both return loft's 64-bit `integer`, which a
millisecond epoch stamp needs. See [tests/docs/22-time.loft](../../tests/docs/22-time.loft) for the
chapter and `tests/scripts/the-reference-clock-units-are-the-ones-it-names.loft` for the
guard that pins the units against each other.

| Function | Description |
|----------|-------------|
| `now() -> integer` | Wall-clock time as **milliseconds** since the Unix epoch (1970-01-01T00:00:00 UTC). For timestamps and anything compared against a calendar. It reads the SYSTEM clock, so an NTP correction or a manual change can step it in either direction — never subtract two `now()` values to time something. |
| `ticks() -> integer` | **Microseconds** elapsed since program start, from a monotonic clock. Never steps backward whatever happens to the system clock, which is the guarantee `now()` does not carry. For benchmarks and frame timing. |

There is no calendar in the standard library — no year/month/weekday, no formatting. That
is the **`time` package**'s subject (`loft install time`, then `use time;`), and it works on
the same millisecond-since-epoch integer `now()` returns, so nothing is converted:
`format_iso(now())`, `weekday_name(now())`, `add_days(now(), 30)`.

---

## Random — `use random;`

These live in the **`random` package**, not the always-loaded stdlib. Add
`use random;` (or call them qualified, `random::…`) or the names do not resolve;
loft names the package for you when they are missing.

A fast PCG64 generator, seeded with a fixed default at startup. Call `rand_seed` before use
when reproducibility matters.

| Function | Description |
|----------|-------------|
| `rand(lo: integer, hi: integer) -> integer` | Returns a uniformly distributed random integer in `[lo, hi]` (inclusive). Returns null if `lo > hi` or either bound is null. |
| `rand_seed(seed: integer)` | Seeds the thread-local RNG. Same seed always produces the same sequence. |
| `rand_indices(n: integer) -> vector<integer>` | Returns a vector of `n` integers `[0, 1, ..., n-1]` in a random order. Empty when `n ≤ 0`. Useful for random iteration or sampling without replacement. |

**Example — pick 3 distinct items at random:**
<!-- from library:random -->
```loft
use random;
fn main() {
  rand_seed(42);
  items = ["a", "b", "c", "d", "e"];
  order = rand_indices(len(items));
  for i in 0..3 { println(items[order[i]]) }
}
```
