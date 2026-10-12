<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Lazy stores — a collection that fetches what it is asked for

A collection can be bound to a source it does not hold. A lookup that **misses**
fetches exactly that one record, keeps it, and every later lookup for the same key
is an ordinary resident read. There is no cache to manage and no loading step to
write: the collection **is** the working set.

```loft
persons: hash<Person[id]> = [];
store_bind_lazy(persons, "sqlite:people.db");

p = persons[42];        // a miss — one SELECT, one row, one record
q = persons[42];        // a hit — never leaves the process
```

Two kinds of source bind the same way:

| source | spelling | what is remote |
|---|---|---|
| a store IMAGE | `"parts.store"`, `"https://host/parts.store"` | loft's own bytes — see [REMOTE_STORES.md](REMOTE_STORES.md) |
| a DATABASE | `"sqlite:people.db"` | a foreign relational schema — rows, columns, SQL types |

The API is in [STDLIB.md](STDLIB.md) (`store_bind_lazy`, `store_lazy_query`,
`store_lazy_range`, `store_lazy_error`, `store_lazy_faults`, `store_lazy_clear`).
This document is the model behind it: what is guaranteed, what a query is derived
from, and what is refused.

## The invariant

> **A record is present WHOLE or not at all, and the collection is the only thing
> that knows which — so a lookup that misses is the one place the outside world is
> consulted.**

## The resident set is the cache

The collection holds exactly the records fetched so far. A `hash<Person[id]>`
already maps key → record, which is already "what have I fetched", and it starts
empty and grows with what is touched. So the cache needs no structure of its own.

That matters for more than tidiness. A separate `(type, key) → rec` map can
diverge from the store — a record freed while its map entry survives — and it is
a second place identity can be decided. With no such map:

1. Look in the loft collection. **Hit → done: no query, ordinary loft speed.**
2. **Miss → consult the source**, materialise the record, insert it.
3. Every later lookup hits.

**Identity falls out of the collection.** After the first fetch, every path to
person 42 finds the one record the hash holds — a keyed lookup, an explicit query,
and a walk from that person's employer all reach the same record, and `is_same`
holds with no identity map anywhere.

That rule has to be absolute rather than conventional, because two identities
coexist at this boundary: loft's is a `DbRef` (store, rec, pos) and the database's
is the primary key. The same row materialised twice would be two records with
equal `id`s and different `DbRef`s. So **every arrival path asks the collection
before materialising**, and no path may materialise a record privately.

## The fault is the collection's miss path, not the address

`Store::addr` looked like the better hook: all 14 typed getters funnel through it
behind `valid()`, and the native `#rust` bodies call the same accessors, so one
site would serve both backends. It is not used, and the reason is worth keeping:

- **`valid()` returns `true` unconditionally in release.** Every check inside it
  is a `debug_assert!`, so it compiles out. A release `get_int` is
  `if rec != 0 { *addr } else { MIN }`. Adding a residency branch there taxes
  every read in every program, lazy or not.
- **Nothing would reach it.** A reference at this boundary is a KEY, not a
  pointer, so following `person.employer` is a LOOKUP on the companies collection
  — the miss path again — not a dereference of a dangling rec.

So residency is **per record, all-or-nothing**, and "not resident" needs no
representation in the store: it is absence from the collection, which `find`
already reports as `rec: 0`.

The unit that buys this is the relational row. A record is fetched whole, so a
wide column (a blob) cannot be left behind — do not map one.

**The traversal is the join.** Following a reference is another indexed lookup, so
no join has to be derived from a layout. The price is round trips: one query per
hop.

## What a query is derived from

Nothing is enumerated ahead of time. The **collection kind is the query shape**,
and the descriptor ([`database/descriptor.rs`](../../src/database/descriptor.rs))
already records the kind, the key fields and the sort direction:

| the collection, and the operation | the query |
|---|---|
| `hash<T[k]>` — `xs[k]` | `WHERE k = ?` |
| `sorted<T[k]>` — a key range | `WHERE k BETWEEN ? AND ?`, in key order |
| `index<T[a, b]>` — a range | `WHERE a = ? AND b BETWEEN ? AND ?` + `ORDER BY` |

The pieces join exactly:

| SQL part | descriptor source |
|---|---|
| table | `LayoutDesc.names[elem]`, lowercased |
| columns | the elem's `LayoutNode::Record(fields)` → each `LayoutField.name` |
| `WHERE` | `Iterated::Hash { keys }` → each `Key` |
| `ORDER BY` | `Iterated::Sorted`/`Index` keys carry `(u16, bool)` — the bool IS the direction |

**A key maps to a column by POSITION.** `Key { type_nr, position }` and
`LayoutField { name, position, content }` both carry a byte offset into the
record, so the key field is the field whose `position` matches. Nothing is
declared twice and nothing is matched by guessing a name.

One thing the descriptor carries that a reader of the type would not expect:

- **A key is an index into the FULL field list**, synthetic fields included, so
  the key list and the column list are numbered in the same space. Do not re-base
  keys on the filtered columns.

### Every identifier is quoted, and that has a price

Every loft identifier is a legal SQL identifier, and some are RESERVED words.
`from` is an ordinary loft field name and the natural one for a history row, and a
query naming it unquoted parses on no engine. Nothing distinguishes a reserved
word by shape, so the derivation **quotes everything** — which removes the class
instead of one word of it, with no list to carry and none to keep current:

```sql
SELECT "person_id", "from", "to" FROM "spell" WHERE "person_id" = ?
```

**SQLite makes that dangerous by default.** It accepts a double-quoted name that
resolves to no identifier as a STRING LITERAL. Against a table with a `name`
column, `SELECT "naam" FROM "person"` does not fail — it returns the text `naam`,
once per row, and a renamed column would be materialised into the record as its
own name. A wrong answer that looks like data is the worst class there is. The
connection turns the misfeature off (`SQLITE_DBCONFIG_DQS_DML` / `_DDL`), which
makes an unresolvable name raise. SQLite older than 3.29 does not know the option,
which is part of why the schema check below is a requirement and not a guard.

The quote CHARACTER is a dialect fact — `"x"` is an identifier in standard SQL and
a string literal in MySQL — so it is declared, not guessed: `Quoting::Double` by
default, `Backtick` for MySQL/MariaDB, `Bare` for a caller who wants the query to
read as they wrote it and accepts a refusal for a name that cannot be written
unquoted. Placeholders are the same kind of fact: `?` by default, `$1`-numbered
for PostgreSQL.

### The mapping is the override, and there is one builder

The database owns the schema. loft binds to tables that already exist and did not
come from loft's types — it is not a schema manager, so there is no DDL and no
migration. What the descriptor cannot supply is declared instead: the table when
it is not the type's name (`persoon` for `Person`), a column when it is not the
field's name, and the dialect facts above.

`Mapping` holds those. **An empty mapping IS the derivation**, so the default and
the override feed one query builder rather than two paths that can drift apart. A
mapping is checked where it is WRITTEN: naming a type or field that does not exist
is refused at construction, not at query time, because a typo would otherwise fall
back to the default and query a column nobody meant.

## The schema is interrogated once, before any answer

A missing column announces itself. **A missing index does not**: every lookup
silently becomes a table scan, and the feature degrades from lazy to catastrophic
without a single wrong answer. So the index half is measured rather than inferred
— `EXPLAIN QUERY PLAN` on the derived query says `SEARCH` when an index serves it
and `SCAN` when nothing does, which asks the engine about THIS query instead of
reconstructing its judgement.

The check runs on the **first fetch**, not at bind: a bind takes a reference, and
a reference carries no type. That is still before any answer a program could
believe. It is decided once per binding and remembered, so it costs two probe
queries for the life of the binding.

A schema that cannot serve the collection is refused through the fault channel,
naming what was wrong.

## A failed fetch cannot raise, and a later success must not erase it

loft has no runtime errors (C80). If an unreachable database answered `null`, then
"no such person" and "the network is down" would be indistinguishable **and
unstable across runs** — the worst class, because it looks like data. So the
failure lives on a store-level channel a program can ask:

- `store_lazy_error(c)` — why a fetch could not reach the source, or `""`
  when healthy. The FIRST failure's reason, kept, because it names the original
  cause; a later one reaches stderr but not this call.
- `store_lazy_faults(c)` — how many fetches could not reach it. After a traversal
  this answers "how incomplete am I". One per FETCH, so a key missed twice counts
  twice — a miss is not remembered.
- `store_lazy_clear(c)` — acknowledge those faults, and the only thing that clears
  them. Neither a genuine absence nor a later success is an acknowledgement.

- `store_lazy_fail(c, why)` — the WRITING end, for a driver written in loft.

**Faults are STICKY.** An earlier version cleared on a later success, which
recreated the same bug in a subtler place: a traversal whose first lookup could
not reach the source and whose second could is MISSING data, and it reported
healthy. Reachability says nothing about what an earlier failure already lost, so
only `store_lazy_clear` clears — and a genuine absence does not clear it either.

## A source core has no driver for: `fn lazy_fetch` (@PLN133 S8)

Core drives one database in Rust (`sqlite:`). The loft library drives four behind
one `SqlDb` interface. Restating the other three in Rust is N drivers now and +1
forever, with the loft versions left to drift — so a collection bound to a scheme
core does NOT drive (`postgres:` · `postgresql:` · `pg:` · `mysql:` · `mariadb:` ·
`maria:` · `duckdb:`) calls **loft** on a miss:

```loft
fn lazy_fetch(coll: hash<Person[id]>, source: text,
              key_int: integer, key_text: text) -> integer {
  // …open `source`, query it, and insert into `coll`…
  return 1;                      // 1 inserted · 0 absent
}
```

It receives the COLLECTION, so what it inserts lands where the lookup is looking,
and the lookup is then re-run — the collection stays the only authority on what is
resident, exactly as for a Rust source. The third answer, *the source is down*,
goes through `store_lazy_fail(coll, why)`: it carries a reason, and answering `0`
for it would make an unreachable source read as an empty table.

### One driver per ELEMENT TYPE (@PLN133 S9)

**A driver serves the type its collection parameter names, and only that type.**
A program with several lazily-bound types declares one driver each. loft refuses a
redefinition, so the extras take a suffix — and the suffix is a label for a
reader, carrying nothing: what a driver serves is read off its parameter.

```loft
fn lazy_fetch(coll: hash<Person[id]>, …) -> integer { … }         // Person
fn lazy_fetch_orders(coll: hash<Order[id]>, …) -> integer { … }   // Order
fn lazy_fetch_seats(coll: index<Ticket[id]>, …) -> integer { … }  // Ticket
```

Three rules follow, and each exists because its absence was a wrong ANSWER rather
than an error:

- **A collection whose type no driver serves reaches NO driver**, and reports
  *"`postgres://…` needs a loft driver for Order"*. Before this, a program's
  single driver was called for every lazily-bound collection whatever it was
  declared for — so a `Person` was inserted into a `hash<Order[id]>` and read
  back through `Order`'s offsets.
- **Two drivers for one element type are refused, naming both.** Picking one
  silently is the same class in a new place.
- **A helper may share the prefix.** `lazy_fetch_row(n: integer)` is an ordinary
  function: past the exact name `lazy_fetch`, a candidate must also take a keyed
  collection first. Anyone writing a driver names its helpers after it, and
  treating those as malformed drivers would refuse the working driver beside them.

### A driver WINS over the source core drives itself (@PLN133 S9)

Core drives `sqlite:` in Rust. Declaring a driver for an element type moves THAT
type's reads onto loft; every type with no driver keeps the Rust source. So a
program adopts the loft path one collection at a time rather than all at once,
and a program that declares nothing is unchanged.

**This is the permanent arrangement, not a migration step.** @PLN133 asked
whether core's Rust sqlite path could then be DELETED, and the answer is no:
deleting it makes a driver mandatory for `sqlite:`, a driver names a concrete
element type so it cannot be generic, and `store_bind_lazy(persons,
"sqlite:people.db")` needing no user code is a shipped promise. What the
precedence rule buys is not deletion but a stopped clock — core's Rust never
gains a fifth backend, and every new one is a loft driver.

The two are meant to be indistinguishable, and that is measured rather than
asserted: the same lookups down each path give the same values, the same
identity, the same residency counts and the same number of trips to the source
(`tests/fixtures/sqldb/s9_two_paths.loft`, both backends).

**One cost to know before reaching for it.** A driver has nowhere to keep a
connection — loft has no process-level state a library can hold — so it connects
per missed row where core caches a handle per target. On a local sqlite file that
is ~2× per fetch (67 µs → 140 µs, measured). On a client-server backend it is a
connect and an auth per row, which is a different order of problem, and those are
exactly the backends core has no Rust driver for.

**A fault inside the driver is CONTAINED.** For an ordinary call, propagating a
fault is right; for a fetch it is not, because C80 says a failed fetch reports
through `store_lazy_error` and the lookup answers null. So a buggy driver leaves
the lookup answering null with a reason, the outer frame intact, and the program
running.

**Both backends, by different routes.** The interpreter runs the driver through
the ordinary call machinery; `--native` cannot — `OpGetRecord` lives in libloft
and cannot see a generated function — so generated `init()` installs a pointer to
it. The answers are byte-identical, which is what the gate compares.

**A contained fault releases what the driver held.** A raise short-circuits the
dispatch loop, so the scope-exit frees never run — harmless for a program about
to exit, and not harmless here, because the program continues. So the stores a
driver creates are remembered for the length of the call and freed if it faults.
An insert copies into the collection's store, so a driver's own new stores are
only ever its locals: what it inserted before faulting survives intact.

**A binding with no driver is REFUSED and says so** — *"`postgres://…` needs a
loft driver for Order"* — rather than being read as a `.store` image, which is
what it used to be. It names the TYPE, because that is what has no driver;
naming only the source would send a reader to a connection string that is fine.

**A refused driver SET reports the same reason on both backends.** The
interpreter re-asks at every miss and reports what it found; `--native` cannot
ask, so generated `init()` installs the refusal as data. Without that the same
program named a different mistake depending on which backend ran it, and the one
naming the real mistake was the one you did not get if you compiled.

## `len` and iteration answer "what have I got"

`len(persons)` is the RESIDENT count, and `for p in persons` walks what has been
touched. This is history-dependent on purpose: after an explicit query, iteration
walks those rows plus whatever else was touched.

The source's row count is a DIFFERENT question. A program that wants it should ask
the source, not a collection pretending to know. Reading `len` as a population
count is asking the wrong object.

This is why a range is an explicit call rather than a slice that fetches: a slice
that silently consulted the source would make `len` and iteration mean two
different things depending on how the collection was reached. See
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) § C104.

## A traversal sees one world

A store is a consistent image; a live source moves. Two faults at different points
in a traversal seeing different worlds breaks the invariant directly, so the
binding pins the source. For a file image that means pinning at bind and REFUSING
a fetch when the image drifted, reported through the fault channel. A database
pins a transaction instead — the one case where consistency can be provided rather
than checked.

This is about the source mutating under a reader. It has nothing to do with
modelling time in the data: a row carrying its own `from`/`to` columns is an
ordinary record, and loft never interprets a date.

## Bounding the working set

Faults only ever add, so the working set needs a way down. `persons = []` is it,
and three things about it are pinned by test:

- It **reclaims** — 2000 entries went from `records 4011, data 97%` to
  `records 3, data 6%`.
- The **binding survives**, because it is keyed by the collection root, which the
  assignment reuses. The next lookup re-faults; emptying does not silently unbind.
- A **held reference survives with its value**, because the deps system keeps a
  referenced record alive while the rest is reclaimed.

One inconsistency worth knowing: an explicit single-entry removal (`coll[k] = null`)
makes a held reference read `null`, while clearing the whole collection leaves it
valid. That is a fact about loft, not about lazy stores.

## What is refused, and why

Each of these answers `store_lazy_error` rather than a wrong record. **A refusal
that only printed is the same silence in a different place**: the lookup answers
`null` either way, and `""` from `store_lazy_error` is documented to mean
"reachable, genuinely no such key" — so a refused binding reported itself
healthy, forever (loft#802). Every refusal below is therefore readable in-band,
and the two chokepoints that guarantee it are `Stores::refuse_paged` (the
loader's, turned into a fault by `fetch_from_file`) and `Stores::bind_lazy` (the
static ones, answered at the bind).

- **A collection kind an IMAGE cannot page — at the BIND.** `store_bind_lazy`
  answers `false` for a `sorted` / `index` bound to a `.store` file or URL: the
  paged reader serves a `hash`, a `trie` (@PLN134) and a `spatial` (@PLN136), that
  is a static property of the pair, and refusing at the call that is wrong beats
  refusing at an arbitrary later lookup. The refused kinds load WHOLE
  (`store_load`, `store_load_url_trusted`), which carries every kind. A DATABASE
  source is not judged here — a `trie` gets `sorted`'s SQL shape and is served.
  A `spatial` faults a POINT as the degenerate bounding box, and the fetch routes on
  the collection's KIND rather than the key's shape: a spatial key arrives as one
  value per axis, and a one-axis one is indistinguishable from a hash's integer key.
- **A source that is reachable and is not a store image.** Every image begins with a
  four-byte signature, and `PageSource::open` reads it — so an empty file, a truncated
  download, a directory, or an HTTP `200` serving an error page is refused with a reason
  instead of binding and answering `null` for every key. Before loft#994 that check did
  not exist: `open` read the file's SIZE, and a size is not a format, so a non-image
  never reached a refusal site at all and failed deep inside the load, which has only
  `false` to return. The URL case is the one that matters — a missing file and a 404 were
  always reported, so what was silent was specifically "reachable, wrong bytes". Note the
  consequence for a DAMAGED image: one whose first four bytes are gone is refused, where
  it used to answer correctly if the pages the keys needed happened to be intact. A
  merely truncated image still reads.
- **Writes.** Read-only. A write to a lazily-backed record is refused loudly;
  silently diverging from the source of truth is the failure this design exists to
  avoid.
- **A `spatial` collection.** Morton order over coordinates has no SQL shape that
  means the same thing, and a bounding-box scan would look like a lazy fetch while
  reading the table.
- **A narrow integer field** (`i32`, `u8`, `size(2)`) — four encodings and their
  null sentinels, plus a different setter again when nullable.
- **A nested struct, a vector, or a stored pointer as a field** — that is another
  table's rows, not a column.
- **A range asked of a `hash`, or of a composite key.** A `hash` has no order to
  range over; a composite key needs `store_lazy_query`, because two bare numbers
  cannot say which value pins the leading column.

## The one place the invariant is escaped by design

`output_c_direct_call` ([`generation/mod.rs`](../../src/generation/mod.rs)) hoists a
RAW base pointer for a `vector<T>` argument and hands C `(pointer, count)`. Its own
contract says that pointer is valid FOR THE CALL and no longer. Every element C
touches after that skips `valid()` entirely, and C **writes back** through it.

Under laziness a write-back into a non-resident region lands nowhere, or on the
wrong bytes. **A lazily-bound store's vector must be forced resident before it
crosses to `#c`, or the crossing must be refused.** This belongs in the contract
rather than in a guard, because it is an escape by design rather than an omission.

## Why this is not the paged image sibling

The two look alike from a distance, and the resemblance misleads.

| | paged image (`store_load_key`, HTTP range) | relational rows |
|---|---|---|
| what is remote | loft's OWN image — same bytes, same layout | a foreign schema |
| unit of fetch | a page (byte range), by offset | a row, by key |
| how a lookup is answered | **loft walks its own hash**, fetching the pages it touches | **the database's index answers it** |
| forming the request | `Range: bytes=X-Y` — no schema needed | SQL, derived from the descriptor |
| the far end | a static file with `Range` support | parses SQL, plans, uses indexes |
| semantics | faithful by construction — it IS the image | **reconstructed** from a foreign schema |

The last row is the one that costs. Over a paged image `len`, iteration and
ordering are simply correct, because the bytes are loft's. Here each has to be
re-derived from a schema that owes loft nothing — which is why the honesty rules
above exist and have no analogue over there. A static image also cannot drift or
mutate under its reader; a live database does both.

## Who executes the query

Deriving the SQL is the easy half. Who runs it is settled by one fact about the
interpreter: **it cannot make a synchronous loft call from inside a lookup.**
`State::fn_call` pushes a `CallFrame` and REDIRECTS the instruction pointer, then
the opcode handler returns and execution continues into the callee. So
`get_record` cannot do "call a loft fetch function, take its result, carry on" —
there is no nested interpreter, and making one means re-running the lookup after a
callback returns, which is a bytecode-level control change.

That rules out the shape most people reach for first — *the binding names a loft
function, and a miss calls it* — on a fact rather than a preference.

So the source is a **Rust-side interface**, and the SQL driver sits behind loft's
existing `#c` machinery, called from Rust: `c_call::resolve` and the per-arity
trampolines are already Rust APIs, so core drives a C library with no crate, no
rustc and no re-entrancy. Core needs to send a derived string and read back a row
— a handful of C entry points — and WHICH library provides them is configuration,
the same shape `[c] optional-libs` uses.

The cost, stated: two implementations of "a source" rather than one, and a narrow
C surface owned by core. Re-open the trade only if a third source appears, because
two is not yet a pattern.

## Testing it

The database tests need `libsqlite3` at RUNTIME (no headers — the symbols are
resolved through `c_call::resolve`). They self-skip when it is absent, so:

- CI installs `libsqlite3-0` and sets **`LOFT_REQUIRE_SQLITE=1`**, which turns
  that skip into a failure. A green Linux run therefore means these tests RAN.
- Without the variable, the skip is recorded in the environmental-skip ledger
  (`LOFT_SKIP_LEDGER`) and surfaced as a CI annotation, so reduced coverage is
  never invisible.

`tests/lazy_sql_source.rs` serialises its tests on one mutex, because
`c_call::register` REPLACES the declared-library list with the running program's
own — an empty list for a script with no `#c` bindings. Without the lock, a test
that merely runs a loft program wipes the sqlite declaration a neighbouring test
is standing on.

## Open work

Most of the table below is superseded in shape by
[@PLN133](https://github.com/loft-lang/plans/issues/133) — **closed 2026-08-08** —
which unified this read path with the `#c` database clients: one connection string
selecting one driver, and one table definition (derived from the type, or read
back from the database) that a writer creates when absent and follows when
present. Its gate writes rows through the derived `INSERT`, binds lazily to the
same string and reads them back, byte-identically on four database backends and
both loft backends. Under it the mapping's loft-source spelling, the narrow-int
refusal and the sqlite-only limit are one question rather than four. The rows
below are the state as BUILT here, in core's own read path.

| item | why it waits |
|---|---|
| **A composite range** | `store_lazy_range(c, lo, hi)` cannot say which value pins a composite key's leading column. `store_lazy_query` covers it verbatim until there is a call shape that carries the pinned prefix. |
| **A DECLARED collection-valued field** | `store_lazy_query(firm.people, "company_id = {firm.id}")` already IS the owner-parameterised query, per collection. What is open is the field knowing its own foreign key so no call is written at all — and that needs a way to declare it. |
| **The `T: DbKeyed` bound** | An interface bound would make "this type has a key" a compile error rather than a runtime refusal, and the accessor would also name WHICH field is the key (a struct with `id`, `company_id` and `year` has three integer fields and no way to tell). Single-column keys work with today's generics; composite keys need associated types, which [@PLN125](https://github.com/loft-lang/plans/issues/125) arc A SHIPPED — `type Key: …` on the interface is now writable, so this is buildable rather than blocked. |
| **The mapping's loft-source spelling** | The mapping VALUE exists and feeds the one builder. How an author WRITES it is a surface question, and its answer depends on the bound above. |
| **A bounded working set** | Selective eviction (keep N, drop least-recently-used) needs a policy owner: loft has no notion of "recently used" on a collection, and adding one is a per-record cost paid by every collection. The blunt form is enough for a working set dropped at a known boundary. |

## See also

- [STDLIB.md](STDLIB.md) — the calls and their exact contracts.
- [REMOTE_STORES.md](REMOTE_STORES.md) — the paged image sibling.
- [DATABASE.md](DATABASE.md) § Adding or changing a collection kind — why a
  per-kind omission does not read as a missing feature.
- [`src/database/sql_query.rs`](../../src/database/sql_query.rs) — the derivation
  and `Mapping`; [`sql_source.rs`](../../src/database/sql_source.rs) — the
  connection and the schema probes; [`lazy.rs`](../../src/database/lazy.rs) — the
  seam and the materialiser.
- [plans/129-lazy-database-stores](plans/129-lazy-database-stores/README.md) — how
  it was built, and what each step answered.
