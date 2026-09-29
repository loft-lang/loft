<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN133 — one SQL boundary: a universal table definition behind one connection string

## Status

**CLOSED 2026-08-08. The gate passes.** All four probes ran 2026-08-06 and
decided the architecture; S1–S6 and S8 landed 2026-08-07, S7 and S9 on
2026-08-08, S11–S14 the same day. S10 is DECIDED rather than built, and the
decision is that the deletion it asked for must not happen — § S10.

P1 **passes**, so **option B is viable**. P2 **passes for reads and fails for
writes on sqlite**, which is a bounded, documented limit rather than a blocker.

| | |
|---|---|
| S1 reflection: collection kind + keys | **done** — `tests/scripts/pln127-reflect.loft` |
| S2 `TableDef` + `derive` | **done** — `tests/fixtures/sqldb/schema/` |
| S3 `render` per dialect | **done** — hand-written DDL, four dialects |
| S4 `reconcile` | **done** — read and write verdicts, six hand-built pairs |
| S5 the connection string | **done** |
| S6 `introspect` (sqlite) + the round trip, run twice | **done** |
| S7 the backend registry | **done** — `tests/fixtures/sqldb/registry/`, shape (1) |
| S8 core's lazy fault calls loft | **done, both backends** |
| S9 prerequisite: per-type driver dispatch | **done** — and it closed a wrong-value hole S8 had left (below) |
| S9 sqlite down the loft path | **done** — a declared driver WINS, and the two paths are proven indistinguishable |
| S10 delete core's Rust sqlite source | **DECIDED: no — a demotion, and it already shipped in S9** (§ S10) |
| S11 absent → create | **done** — `ensure`, all four backends |
| S12 present → follow | **done** — same call, same four backends |
| S13 `insert_row` from the same `TableDef` | **done** for the statement; the GENERIC struct walk stays @PLN23 S5 (§ S13) |
| S14 the gate, run twice | **done** — 4 database backends × 2 loft backends, byte-identical |

`introspect` now reads all four catalogues, so requirement 3 is no longer a
sqlite claim. Every query was run against a live server of its engine before it
was written down — the rule P3 exists to enforce.

Every measurement below is from the current tree and is cited so it can be
re-checked rather than believed.

**Issue:** [loft-lang/plans#133](https://github.com/loft-lang/plans/issues/133).

## Effort + design

- **Effort:** H
- **Design:** the invariant is named, every load-bearing claim has a probe, and
  the reconcile rules for a foreign schema are now tested against real tables on
  four engines rather than only stated.
- **Last touched:** 2026-08-08

## Goal

Two requirements, and they compose into one design.

1. **One configuration string switches every SQL consumer in the process to a
   different database** — a lazily-bound collection (@PLN129), a client routine
   (@PLN23), and an ORM write (@PLN23 S5–S7) — with no other edit anywhere.
2. **A structure written to the database is IMMEDIATELY usable through lazy
   loading.** Write a graph, bind a collection to the same string, traverse it.
   No export step, no schema hand-off, no second description of the same tables.
3. **A loft structure has ONE universal table definition.** Where the database has
   nothing, loft writes that definition itself so lazy loading is possible at all.
   Where the database already holds data — **ours, or another project's** — loft
   FOLLOWS the structure that is there.

Today the first is impossible in the strongest sense — the lazy path speaks only
sqlite and cannot reach the other three backends at all — and the second has
never been possible, because the writer and the reader derive their SQL from the
same descriptor **in two different languages**.

## The invariant

> **A loft type has exactly one table definition, and that definition is a VALUE —
> derived from the type, or read back from the database, in the SAME shape.
> Creating, following, querying and writing are all functions of that one value,
> so a table loft made and a table loft found are indistinguishable to everything
> downstream. A connection string selects which driver renders it.**

Everything below either tries to falsify that sentence or pays for it.

### `TableDef` is the single currency

The invariant is only worth stating if one value really carries all four jobs. It
does, as three producers and four consumers of one type:

| | |
|---|---|
| `derive(T) -> TableDef` | the UNIVERSAL definition, from the loft type |
| `introspect(conn, name) -> TableDef?` | what the database actually has |
| `reconcile(want, have) -> Binding \| Refusal` | the match, naming exactly what is wrong — and carrying the per-column CONVERSION |
| `render(TableDef, dialect) -> DDL` | `CREATE TABLE` **and its index** |
| `select(TableDef, key)` / `insert(TableDef)` | the statements |

Requirement 3 is then two lines rather than a mode: **absent → `render(derive(T))`;
present → `reconcile(derive(T), introspect())`.** No provenance flag, no "is this
ours" state to keep — a database loft created takes the same path as a foreign
one, and simply reconciles trivially. Removing that state is the point: a
provenance marker would be a second description of the same fact, which is the
failure this design exists to prevent.

**This reverses a recorded decision, and that has to be explicit.** BINDING.md
carries the owner's 2026-08-04 decision that *"the DATABASE owns that schema…
loft is not a schema manager — no DDL, no migrations."* Requirement 3 revises it:
loft DOES emit DDL, but only into absence. The revised rule is **loft never
overwrites a structure it did not find missing**, which keeps the original
decision's substance — the existing database remains the authority — while
allowing the empty case to bootstrap itself.

### The asymmetry that makes this cheap: we CHOOSE, or we CONVERT

`reconcile` does not answer yes/no. It answers **a binding carrying one conversion
per column**, and that single idea removes most of the hard cases:

- **Where loft defines the table, it picks the cleanest and most efficient column
  type** — `BIGINT`, `DOUBLE PRECISION`, `TEXT`, `BOOLEAN`, `NOT NULL` where the
  field is not nullable. The conversion is then identity, and nothing is paid.
- **Where loft follows someone else's table, it accepts what is there and
  converts** — a number kept in a `VARCHAR`, a boolean as `0`/`1`, a float as
  `NUMERIC`. The conversion is a property of the binding, computed once at
  reconcile, not a branch taken per row read.

**This is what dissolves the float problem.** A `double` returned BY VALUE cannot
cross the `#c` boundary (the interpreter's trampolines are integer-class), which is
why the library binds floats as TEXT — and reading it as text and parsing it is a
conversion like any other. So:

- **Measured (P2): reading is exact**, with `printf('%!.17g', …)`. So @PLN128's
  E3 float refusal is **not** a prerequisite for the read path, which is the whole
  of the lazy loader.
- **Measured (P2): writing is not**, on sqlite, for extreme-exponent values —
  its decimal parser can store a literal one ULP off, and more digits do not fix
  it. **E3 therefore pays for the WRITE path on sqlite specifically**, and
  nowhere else. 0 of 3000 values in the ±1e6 range were affected.
- On PostgreSQL this is not even a workaround — the wire protocol returns every
  value as text already, so "parse it" is the native path. **Expected to be exact;
  not measured** (no server), and the gate must check it.
- What replaces the capability requirement is a **contract**: a float written by
  loft and read back by loft must compare EQUAL. A rendering that loses the low
  bits is a wrong value, and P2 shows the obvious spelling (`CAST(v AS TEXT)`)
  loses it 94% of the time.

The same asymmetry retires a second refusal for free: @PLN129 today refuses a
**narrow integer field** (`i32`, `u8`, `size(2)`) because there are four encodings
and their null sentinels to get right. Under a conversion plan that is one
per-column decision made once at reconcile, not four code paths at read time.

### Following a foreign structure — what reconcile must decide

The interesting half of requirement 3 is the table loft did not write. `reconcile`
is where the design earns its keep, and each answer is a rule rather than a
heuristic:

- **Column matching is by NAME, not position.** Position is how loft finds which
  of its own fields is the KEY (`Key.position == LayoutField.position`); the
  column is then that field's name, overridable by the declared mapping. A
  foreign table's column order means nothing and must never be read as meaning.
- **Extra columns in the table are FINE for reading** — `SELECT` names only the
  columns loft wants. They are not fine for writing: an unknown `NOT NULL` column
  with no default makes `INSERT` impossible, so a write must be refused there,
  naming the column. Reading and writing therefore reconcile to different
  verdicts against the same table, and `Binding` has to carry which it permits.
- **A missing column, or an incompatible type, refuses** — @PLN129 already does
  this and reports through arc C's channel.
- **A missing INDEX on a foreign table refuses; it does not silently ALTER.**
  Creating a table where there was none is filling absence. Adding an index to
  someone else's populated table is mutating their schema, can take a long time,
  and can change their write performance. It stays opt-in, and the refusal names
  the index it wanted so a DBA can add it.
- **Our own struct gaining a field is MIGRATION, not reconciliation.** This is
  the one case where "is it ours" genuinely matters, and the uniform rule does not
  absorb it: under reconcile a table missing a column simply refuses. Migration
  (@PLN23 S7) stays an explicit, separate operation. Naming this boundary is what
  keeps the uniform rule honest instead of quietly wrong.

**Requirement 2 is what makes the shared derivation load-bearing rather than
tidy.** If the writer creates `person(id, name)` and the reader derives
`SELECT "id","name" FROM "person" WHERE "id" = ?`, a round trip works only while
the two derivations agree — and nothing checks that they do. Worse, @PLN129's
bind is REFUSED when no index serves the lookup (`EXPLAIN QUERY PLAN` answering
`SCAN`), so a writer that emits a table without the index the collection kind
implies produces a database its own reader cannot bind to. **The collection kind
has to decide both**: the index the writer creates and the query the reader
derives. One derivation is the only way that holds.

## What is actually duplicated — measured, not recalled

| | core (@PLN129) | the loft library (@PLN23) |
|---|---|---|
| sqlite symbols bound | **15**, hand-written typed `extern "C"` in `src/database/sql_source.rs` | **14**, `#c` declarations in `tests/fixtures/sqldb/sqlite/` |
| other backends | **none** | postgres, mariadb, duckdb — one `SqlDb` interface, byte-identical output |
| quoting / placeholders | `Quoting::{Double,Backtick,Bare}`, `Placeholder::{Question,Numbered}` in Rust | per backend, in loft (`$1 $2 …`, `?`, backticks) |
| `double` by value | **yes** — `extern "C" fn(*mut c_void, c_int) -> f64` | **no** — floats bind as TEXT |
| schema → SQL | `sql_query.rs` derives SELECT from `LayoutDesc` | S5–S7 would derive DDL/INSERT from the same |

Twelve sqlite symbols are bound twice. The dialect facts were measured twice,
independently. And the two halves disagree about floats.

## Why it happened — a capability gap, not carelessness

Three C-call mechanisms exist, and they do not agree:

| caller | mechanism | can pass a `double` |
|---|---|---|
| `--native` | rustc-emitted typed `extern "C"` from `CSignature` | yes (refused for parity) |
| the interpreter | `call_at_arity(f, &[u64])` — a 0..=12 ladder, integer class only | **no** |
| core's lazy source | hand-written typed externs | **yes** |

The interpreter's ladder is `u64`-only for a reason that is not laziness: a
signature is known at **runtime**, and Rust cannot build a typed `extern "C"`
from runtime data. A full cross-product of arity × argument class is ~2^13 × 13
function types.

So core needed floats, the public `#c` path could not give them, and core wrote
its own driver. **The SQL duplication is downstream of the C-call capability
gap.** That is the thing to fix first, whichever architecture wins.

## The option that looks cleanest, and why it is wrong

*Let the loft library DECLARE the symbols and the dialect; let core read those
declarations and execute them itself.* One declaration, two executors, no
re-entrancy needed.

**It fails on the protocol.** The four backends are not one protocol with
different symbol names:

- sqlite — `prepare_v2` / `step` / `column_*`
- postgres — `PQexec` returns a result object, then `PQgetvalue`
- mariadb — a statement API over `MYSQL_BIND` **arrays of structs**, which needs
  a hand-written ANSI-C shim to express at all

What unifies them is the `SqlDb` **interface**, and that interface is loft code.
A symbol table is not enough, so core would re-implement the per-backend logic —
the duplication returns one layer down, with the backends now stated twice each.

## The real choice: which side owns the driver

**Option A — drivers move to Rust in core.** The loft library becomes a thin
wrapper over core builtins.

**Option B — core gains the ability to call loft**, and uses the library that
already exists.

**Counting decides it.** Under A, every backend is restated in Rust: N = 4 today,
+1 per backend forever, and the loft versions must either be deleted (losing
@PLN24's flagship consumer and the "no rustc in a library" property) or kept in
sync by hand — silent divergence, the exact `N × silence` this project counts
before writing code. Under B, a backend is written **once**, in loft, where four
of them already are and are already proven byte-identical across both loft
backends.

**Option B, on the counting.** But it rests on one unproven claim, below.

## How far B goes — the derivation moves to loft too

Requirement 2 wants one derivation. Under B it can be **loft code**, and that is
the answer to "rewrite Rust into loft or the other way": `src/database/sql_query.rs`
(415 lines of Rust) becomes a loft module beside the `SqlDb` interface, and serves
both directions —

| from one derivation | consumer |
|---|---|
| `SELECT … WHERE key = ?` | the lazy fault (@PLN129) |
| `CREATE TABLE …` **and its index** | the ORM write (@PLN23 S7) |
| `INSERT … VALUES (…)` | the ORM write |

**This is possible because @PLN127 already put the descriptor in loft's hands.**
`type_of(v)` answers a `TypeInfo` carrying each field's `name`, `type_name`,
`position`, `kind` and `nullable` — and the reflection doc already names the use:
*"It is what a generated `CREATE TABLE` needs for `NOT NULL`."*

**One gap, and it is additive.** Reflection reports a keyed collection as
`KeyedKind` plus its element type "and nothing more" — it does **not** expose
which kind it is (hash / index / sorted / ordered / radix), its **key fields**, or
the sort-direction bit. The derivation needs exactly those: core reads them from
`Iterated::Hash { keys }` and the `(u16, bool)` direction. So moving the
derivation to loft requires extending @PLN127's surface with the collection kind
and its keys, **carrying `position` so the key→column match stays by position and
never by name-guessing**. Additive, and it is the same fact reflection already
publishes for ordinary fields.

Core then keeps only what cannot leave it: detect the miss, hand the collection's
type and key to loft, and materialise the returned row into the store through
`record_new` + `record_finish`.

## The probe that decides it

> **Can `State` run a loft function to completion re-entrantly, from inside an
> opcode handler, and return its value?**

@PLN129 costed this as "a bytecode-level control change… far larger than this
arc", and that was right *for the shape it considered* — a callback from inside
the lookup. The requirement is weaker than that, and the code is already most of
the way there:

`Stores::find_or_fetch` (`src/database/allocation.rs:3997`) already does
**miss → fetch → re-run the lookup**. Re-running is deliberate and documented:
the collection stays the single source of truth. What is missing is only that the
fetch is Rust.

So the change is to lift `find_or_fetch` out of `Stores` into its **two** callers
— `State::get_record` and `codegen_runtime`'s lookup, the count @PLN129
established — so the `&mut Stores` borrow is released around the fetch:

```
found = stores.find(…)
if found.rec == 0 && stores.is_lazy(coll) {
    run the loft fetch          // Stores borrow released here
    found = stores.find(…)      // retry — the rule that already exists
}
```

For `--native` this is trivial: generated code is Rust calling Rust. For the
interpreter it needs `State::run_until_return(d_nr, args)` — push the frame, run
the existing loop until `call_depth` drops back to the entry depth, take the
value. That is a **re-entrant execute loop, not a bytecode change**.

**Write that function and call one trivial loft `fn` from an opcode handler.** If
the eval stack, `raise` unwinding, or a `par` block cannot survive re-entry,
option B is dead and A is the answer. This is cheap and it decides the whole
architecture, so nothing else should be built first.

## Failure paths — written before the code

1. **A backend the build does not have.** Must be "unreachable", never "no such
   row". `c_library_available` (@PLN24 arc G) and `store_lazy_error` (@PLN129 arc
   C) already exist; the string parser must route into them rather than answering
   null.
2. **Dialect drift.** Quoting and placeholders must come from the BACKEND, not
   from core. Otherwise the string switches the driver but not the quoting, and
   mariadb gets ANSI double quotes — measured to answer
   *`error in your SQL syntax … near '"loft_p"'`*. A wrong query, not a wrong
   result, but only because mariadb happened to reject it.
3. **A float loses its low bits on the way through text.** Core does exact `f64`
   today through typed externs; under B a float crosses as text and is parsed,
   which is the accepted model — but only if the rendering carries enough
   significant digits to round-trip IEEE-754 exactly. sqlite's default text
   rendering of a `REAL` does not, so the driver has to ask for more. **The test
   is a round trip that compares EQUAL, per backend**, and it must be written
   before any consumer stores a coordinate. This is the failure that looks like
   data: values that are almost right.
4. **Re-entrancy inside `par`.** A fault taken on a worker thread would run loft
   there. Either the fetch is serialised, or `par` + lazy is refused.
5. **Two notions of "one world".** @PLN129 arc D pins a source per traversal;
   @PLN23 T1–T3 has transactions. Under one driver these must become one concept,
   or a traversal inside a transaction has two answers about what it can see.
6. **The derivation, read and write.** `sql_query.rs` derives SELECT from
   `LayoutDesc`; S5–S7 would derive DDL and INSERT from the same descriptor. One
   home, or a row written and a row read disagree about what the table is. Note
   the two traps already paid for on the read side: keys map to columns **by
   position**, and `LayoutField::is_data` excludes the red-black links
   (`#left_1`/`#right_1`/`#color_1`) that a `sorted`/`index` child table would
   otherwise name as columns. **A loft-side derivation must inherit both**, which
   means reflection has to filter the synthetic fields the way `is_data` does —
   if `type_of` reports `#color_1`, the generated `CREATE TABLE` grows a column no
   reader wants and the round trip breaks on the writer's side.
7. **The writer omits the index the reader requires.** @PLN129 refuses a bind
   whose lookup no index serves, and it is right to: a missing index is a silent
   collapse from lazy to table-scan. So DDL generation must emit the index the
   collection kind implies — `hash` an equality index on its keys, `sorted`/`index`
   an ordered one — or requirement 2 fails on the first bind after a write. This
   is the strongest argument for one derivation: the same fact chooses both.
8. **Visibility — "immediately" has an ordering rule.** @PLN129 arc D PINS the
   source when the collection binds, and for a database that pin is a
   transaction. A collection bound before the write commits will not see it, and
   arc D will REFUSE the fetch as drift rather than answer stale — correct, but
   it turns requirement 2 into a puzzle unless the rule is stated: **bind after
   commit, or make the pin re-establishable on demand.** Whichever is chosen has
   to be written down, because both are defensible and they differ in what a
   long-running traversal sees.
10. **A fault inside the nested fetch must not halt the program — MEASURED, and
   it does today.** P1c ran a loft fetch function that overflowed the call stack:
   the fault propagated and the program halted, and the caller read a garbage
   value. For an ordinary call that is right; for a fetch it violates C80, where a
   failed fetch reports through `store_lazy_error` and the lookup answers null.
   So the nested run must CONTAIN the fault and convert it into arc C's channel —
   otherwise moving the source from Rust to loft turns a class of source bug into
   a program halt, which is a regression against what @PLN129 ships today.
9. **Lazy stays READ-ONLY, and requirement 2 does not change that.** The write
   goes through the ORM/client; the lazy binding remains a read path that refuses
   writes loudly (@PLN129 failure path 4). "Write then read lazily" is two
   operations on one database, not a read-write collection. Making the binding
   writable would reopen the exact divergence-from-source-of-truth failure the
   whole design exists to avoid.

## Probe results — run 2026-08-06

Both gating probes were built as throwaways, run, and the code reverted. The
scripts are in this directory's history only; what matters is the answers.

### P1 — can the interpreter run loft re-entrantly from a lookup? **YES.**

**The mechanism already existed.** `State::execute_at` runs a loft function to
completion from Rust for the `par` workers — push a frame, run the dispatch loop,
read the result. The probe reused the ORDINARY call machinery instead: `fn_call`
pushes the frame and stores the return address, then the loop runs until that
frame pops. So the callee returns through exactly the path every other call uses.

**The borrow dissolves as predicted.** `Stores::find` RETURNS a value, so the
`&mut self.database` borrow ends before the nested call. The split in
`State::get_record` is three lines:

```rust
let first = self.database.find(&data, db_tp, &key);
if first.rec == 0 { /* nested loft call here */ self.database.find(&data, db_tp, &key) }
else { first }
```

| cell | result |
|---|---|
| a zero-arg loft fn called from inside `get_record`'s miss path | ran, returned its value (`6` from `3+2+1`) |
| that fn making its own nested + recursive calls | fine, 3 deep |
| outer locals across the nested run (`integer`, `text`, `vector`) | **all intact** — `a=111`, `b=outer-text`, `v=7,8,9` |
| repeated, including inside a `while` loop | 3 invocations, all clean |
| a resident HIT | no nested call — the miss path is the only entry |
| **the fetch shape**: the nested fn receives the COLLECTION, allocates text and INSERTS into the very collection the outer lookup is walking | **the outer retry finds the inserted record**; a second lookup is a resident hit with no nested call; the pre-existing entry is intact; no leak under `LOFT_STORES=warn` |

**So @PLN129's "far larger than this arc" was right about the shape it
considered and wrong about the one needed.** It assumed a callback *inside* the
lookup; what the design needs is a nested run at the CALLER, and the retry it
requires is already how `find_or_fetch` works.

**P1c found a failure path the prose had not listed.** When the nested function
FAULTS (runaway recursion → call-stack overflow), the fault propagates and
**halts the program**, and the probe read a garbage value off the stack. For an
ordinary call that is correct. For a FETCH it is not: @PLN129's contract is C80 —
a failed fetch reports through `store_lazy_error`, never a halt. So a buggy or
unlucky loft fetch function would turn a lookup into a program halt where the
Rust source reported through the channel.

**That is a requirement on the design, not a defect in the probe:** the nested
call must be run with the fault CONTAINED and converted into arc C's channel.
Recorded as failure path 10.

### P2 — does a float survive a text round trip? **READS yes, WRITES no (sqlite).**

Measured with sqlite 3.45.1, 2000 uniformly random bit-pattern doubles plus 3000
in the ±1e6 range.

| rendering | inexact, of 2000 random doubles |
|---|---|
| `CAST(v AS TEXT)` — the obvious choice | **1887** |
| `printf('%.17g', v)` | **928** |
| `printf('%!.17g', v)` — note the `!` | **1** |

**loft's own half is exact.** `float → "{v}" → as float` round-tripped 12/12
hand-picked hard cases on BOTH backends: loft renders the shortest form that
round-trips.

**`CAST(v AS TEXT)` is the trap**, and it is the spelling anyone would reach for
first: it renders `%!.15g`, so `123456789012345.6` comes back as
`123456789012346.0` — a different number, not a rounding artefact. `%.17g`
*without* the bang looks fine on hand-picked values and still loses 46% of random
ones, which is exactly why this was swept rather than sampled.

**The one remaining failure is at INSERT, not SELECT.** Isolated: sqlite's own
round trip (`v = CAST(printf('%!.17g', v) AS REAL)`) is **exact 2000/2000**, so
the reader is sound. The single mismatch is sqlite's decimal PARSER storing a
value one ULP from the literal it was given — and **more digits does not fix it**
(shortest, 20-digit and 25-digit forms all store the same wrong double).

**Bounded, and the boundary is worth knowing:** 0 of 3000 values in the ±1e6
range were inexact. The failure lives at extreme exponents (the mismatch was
`-5.196972490273514e-183`). So:

- **Reading is exact** with `printf('%!.17g', …)`, which the sqlite driver's
  SELECT must wrap float columns in. Use `printf`, not `format` — `format()` is
  sqlite 3.38+, `printf()` is 3.8.3+.
- **Writing an extreme-exponent float through a decimal literal can lose one
  ULP.** The exact fix is `sqlite3_bind_double`, which is the `#c` float
  capability — so **@PLN128 E3 pays for the WRITE path on sqlite**, and only
  there. It stays out of the critical path for everything else.
- PostgreSQL is unverified without a server; its wire protocol is text already
  and its parser is `strtod`, so it is expected to be exact. **Expected, not
  measured** — the gate must check it.

### P3 — the float round trip on all FOUR backends. **The loss is on the WRITE side.**

Run locally against live servers (PostgreSQL 16.14, MariaDB 10.11.14, duckdb via
`LD_LIBRARY_PATH`), 2000 random bit-pattern doubles each except duckdb (500,
driven through the loft fixture).

| backend | rendering | inexact |
|---|---|---|
| sqlite 3.45 | `CAST(v AS TEXT)` | 1887 / 2000 |
| sqlite 3.45 | `printf('%.17g', v)` | 928 / 2000 |
| sqlite 3.45 | **`printf('%!.17g', v)`** | **1 / 2000** |
| PostgreSQL 16 | `extra_float_digits = 0` | 1887 / 2000 |
| PostgreSQL 16 | **`extra_float_digits = 1` (the modern default) or `3`** | **0 / 2000** |
| MariaDB 10.11 | **plain `v`, or `CAST(v AS CHAR)`** | **0 / 2000** |
| MariaDB 10.11 | `FORMAT(v, 17)` | 1144 / 2000 |
| duckdb | plain `v` / `CAST(v AS VARCHAR)`, **through loft's literal** | **19 / 500 — diagnosed, see below** |

**Each engine renders exactly once told to.** PostgreSQL and MariaDB are exact on
BOTH sides — their parsers are correctly rounded, so a decimal literal makes the
round trip. Two rules fall out, and neither is a default you can lean on:
`extra_float_digits` must be SET by the driver (it defaulted to 0 before PG12, and
a session can change it), and `FORMAT()` is a locale formatter, not a renderer.

**duckdb's own round trip is exact** — `v = CAST(CAST(v AS VARCHAR) AS DOUBLE)`
answered `yes` for every failing case, so the loss is in the hand-off, in
duckdb's parse of the literal loft hands it.

**Diagnosed (2026-08-06) by testing duckdb alone in C**, reading the value back
with `duckdb_value_double` so duckdb's own text rendering is out of the loop and
exactly one stage is under test. Of the 19 failures:

| class | count | what happens |
|---|---|---|
| **the 10²⁵⁶ bug** | **15** | the parsed value is *exactly* `correct / 10^256`, silently |
| 1–2 ULP | 4 | ordinary parser rounding, at ordinary magnitudes |

**The 10²⁵⁶ bug has a sharp, reproducible boundary, and it is the LITERAL FORM
that triggers it — not the value.** With the same doubles written in exponent
notation, duckdb is correct everywhere:

| the value, as `5.754124332515439e<E>` | full decimal expansion | exponent form |
|---|---|---|
| E = 272 (expansion 273 chars) | ok | ok |
| **E = 274 … 293 (expansion 275–294 chars)** | **WRONG — off by 10²⁵⁶** | ok |
| E = 294 and above | ok | ok |

A *window*, not a threshold: it recovers above E = 293. In the window the answer
is a plausible number roughly 10²⁵⁶ too small — the failure class C80 exists to
prevent, arriving as data.

**The mechanism is a TYPE decision, and `typeof` says it outright:**

| the literal | duckdb types it | result |
|---|---|---|
| 294-digit integer | **`HUGEINT`** | **silently truncated** — 5.75e293 does not fit in 128 bits |
| the same digits QUOTED, cast to `DOUBLE` | string → double | correct |
| `5.754124332515439e293` | `DOUBLE` | correct |
| 316-digit integer | `DOUBLE` | correct |

So duckdb reads a long bare integer literal as a 128-bit integer (max ≈ 1.7e38),
overflows it **without an error**, and only gives up on `HUGEINT` for a much
longer digit run — which is exactly why the failure has an upper edge as well as
a lower one. Nothing about the value is out of range for `DOUBLE`; it is the
literal's TYPE that is wrong.

Two spellings avoid it entirely, and either is a fine fix for the driver:
**exponent notation** (typed `DOUBLE`) or **quoting the digits** and casting.

**loft is what walks into it.** `"{v}"` renders a float as a full decimal
expansion with **no exponent**, so `5e-324` is 326 characters and any float above
~1e274 is a 275+ character digit run. Every SQL literal loft builds from a float
is therefore in expansion form by construction.

**An earlier version of this section got this wrong twice, and both errors are
worth keeping.** First it named long literals as the cause without probing —
a hypothesis published as a finding. Then it "disproved" that with literals of
252, 294 and 303 characters that round-tripped fine — but 252 is *below* the
window and the other two were TINY magnitudes, where the long digit run is after
the decimal point and truncating it costs precision rather than magnitude. A
disproof drawn from unrepresentative cells is not a disproof. The mechanism was
right; the reasoning on both passes was not.

### The workaround, verified — duckdb stays

**We are not blocked on upstream.** The fix is entirely on our side of the
boundary, and it needs no change to loft's float rendering. Same 500 doubles,
same loft-produced digits, three ways of writing them:

| how the driver writes the float | wrong |
|---|---|
| `INSERT … VALUES ({v})` — bare literal, today | **19 / 500** |
| `INSERT … VALUES (CAST('{v}' AS DOUBLE))` — **quote loft's own digits** | **0 / 500** |
| exponent notation (`%.17g`) | **0 / 500** |

Quoting fixes **both** classes — the 15 HUGEINT truncations and the 4 ULP cases —
because a quoted value takes the string→double path instead of the literal
tokenizer. Confirmed end to end through the real fixture, not just in C:
`d.db_exec("INSERT INTO p3f VALUES (CAST('{v}' AS DOUBLE))")` gives **0 / 500**.

### …and the drivers already do the right thing — measured before changing them

**The duckdb driver needs NO fix.** It does not inline floats: `bind()` routes
`SQL_FLOAT` to `ddb_bind_varchar(stmt, at, b.as_text())`, which is the
string→double path, and that is **0 / 500**. The 19 failures were reached by a
PROBE that interpolated a bare literal into `db_exec` — not by the driver. All
four backends bind a float as text for the same reason (no `#c` path carries a
`double` by value), so all four are on the safe side of the literal bug.

**The exposure is a caller writing `db_exec("… {v} …")` directly**, which
bypasses `SqlText` entirely — the same hole @PLN124's interpolation hook exists
to close for injection, showing up as a numeric fault instead of a syntactic one.

**sqlite loses one value in seven, and it IS the bind.** Its text→REAL converter
rounds `-5.196972490273514e-183` one ULP wrong where `sqlite3_bind_double` on the
same value is right, measured directly in C. The other three backends parse
correctly and score 7/7.

*This claim was stated, retracted, and is now restored — and the retraction is the
instructive part.* The guard briefly reported sqlite at **7/7**, which looked like
proof that the bind was fine and the READ was the whole story. It was not a
measurement: `floats` is itself a generic function, so under
[loft#791](https://github.com/loft-lang/loft/issues/791) its write loop and its
read loop saw the SAME corrupted vector and agreed with each other. **A guard can
pass by comparing garbage to identical garbage**, and this one did. Fixing #791
made it honest and the real defect reappeared, matching the C measurement exactly.

The per-backend READ expression is still needed, for a separate reason: sqlite
renders a `REAL` as `%!.15g`, so reading naively loses the low bits of values that
were stored correctly. Both effects are real; only one of them is the bind.

**A shim could not fix it in any case.** `tests/fixtures/sqldb/sqlite/loft.toml`
declares sqlite `optional-libs`, and the shim's header states it is *"deliberately
free of any sqlite3 symbol… so it links against nothing"* — which is what lets it
compile on a machine with no libsqlite3 (@PLN24 arc G). A shim calling
`sqlite3_bind_double` would put that hard dependency back.

**So the rule is one rule, and it was always the right one:** a driver BINDS a
float — which all four already do — and a CALLER must never interpolate one into
a raw statement. sqlite's last ULP waits on @PLN128 E3, which is where a
write-path requirement belonged all along.

**And READING needs a per-backend full-precision expression**, which the `SqlDb`
contract does not currently have. That is a real requirement this probe found:
sqlite needs `printf('%!.17g', v)`, PostgreSQL needs `extra_float_digits >= 1`,
MariaDB and duckdb render shortest-round-trip already. A portable `SELECT v` is
lossy on two of four. The regression guard parameterises it the way `create` is
parameterised; a real driver should carry it in the backend.

**What this settles for the design:** a driver must render a float in **exponent
notation**, quote it, or bind it — never emit `"{v}"` as a bare literal. Reading
is exact everywhere once the rendering is specified.

**Two things to take upstream / next door**, neither fixed here:
- **duckdb 1.5.5** (the current release) types a 275–294 digit bare integer
  literal as `HUGEINT` and overflows it silently, giving a value 10²⁵⁶ too small.
  Minimal repro: `SELECT typeof(<294 digits>)` → `HUGEINT`;
  `SELECT CAST(<294 digits> AS DOUBLE)` → wrong; `SELECT CAST('<same digits>' AS
  DOUBLE)` → right. The silent-`HUGEINT`-overflow FAMILY is known upstream
  (duckdb#24081, duckdb#14580, both closed) but for aggregates, not literals;
  no issue was found for this case.
- **loft's float→text has no exponent form at all.** `1e300` is a 301-character
  string in every context, not just SQL, and `5e-324` is 326. That is what walks
  loft into the above, and it is worth deciding on its own merits.

### What the guard found next door — loft#791

Chasing why one cell of the float guard behaved differently per backend led out of
SQL entirely: **a `vector` declared inside a GENERIC function reads correctly at
index 0 and garbage from index 1 onward**, silently for numbers and as a CRASH for
`text`, on both backends. Passing the same vector to a non-generic helper reads
every element perfectly in the same call, and `len()` is right throughout — so the
data is fine and the access inside the generic is not.

Ruled out: the backend (identical corruption on both), generics as such (a local
1-method and a local 14-method interface bound are both correct), and a
library-declared interface as such (a freshly written two-line library does not
reproduce). The trigger is something specific to the `tests/fixtures/sqldb`
library set that is not yet isolated. Filed as
[loft#791](https://github.com/loft-lang/loft/issues/791).

**Worth noting how it presented:** as a database inconsistency — one backend
disagreeing with three on a float cell. It is a language bug. That is the second
time in this probe series that a SQL-shaped symptom had a non-SQL cause.

### P4 — can a fault inside the nested fetch be contained? **YES, but it leaks.**

Containment is save-run-take-restore: run the nested frame while
`database.runtime_error` is none; if the callee raised, TAKE the error, truncate
the call stack to the saved depth, restore `call_depth` / `stack_pos` /
`code_pos`, clear `had_fatal`, and hand the reason back as a value.

Measured with a fetch function that overflows the call stack on its first call
and succeeds on its second:

- the fault was **contained** — `contained fault: call stack overflow`;
- the lookup answered **null**, which is the C80 / arc C shape;
- the outer program **continued** with every local intact;
- a **later** nested call succeeded and delivered its record;
- exit status 0.

**The caveat, and it is the finding: `Warning: 1 stores not freed at program
exit`.** Truncating the call stack abandons whatever the faulting callee had
allocated. A control run of the same probe without the fault leaks nothing, so
the leak is the contained fault's.

Containment therefore needs an unwind that RELEASES what the aborted callee held,
not just a stack truncation — otherwise a traversal over an unreachable source
leaks once per failed fetch, which is exactly the long-running case arc C's
sticky fault counter exists for. Recorded as the first thing S8 must solve.

## What the build settled

### S1 — the two kinds a `sorted` declaration can be

`sorted<T[…]>` does not name one structure. It registers as `Parts::Sorted` — a
red-black tree — unless `T` is **co-located**, which a `hash` / `index` /
`spatial` field elsewhere in the program makes it; then the same declaration
becomes `Parts::Ordered`, a sorted by-value vector searched by bisection. So
reflection reports `KeyedSorted` and `KeyedOrdered` as two kinds rather than one
"ordered somehow": they are different structures, and a consumer that only knew
they were both ordered could not tell which refusals apply to which.

**It is not a property of the declaration**, which is what makes it worth
pinning: the gate needs a second element type to hold the `ordered` cell,
because linking `KeyedItem` to demonstrate it would have changed what the
`sorted<KeyedItem[…]>` cell beside it reports.

### S1 — a key is delivered whole or not at all

The descriptor holds keys as INDICES into the element record's field list, and
those indices are resolved to `(name, byte position)` before they are published.
Two reasons, and the second is the one that decides the shape:

- An index is meaningless to a caller who also has to skip the synthetic fields
  (`#left_1`, `#right_1`, `#color_1`) that `LayoutField::is_data` filters, so a
  raw index would make every consumer re-derive `is_data` to use it.
- **A key that cannot be resolved drops the whole list.** A `__nullable<S>`
  element keys through its `Some` payload, which is not the element node, so its
  keys resolve to nothing. Publishing the resolvable half would derive
  `WHERE k1 = ?` from a two-key collection — a query that reads the WRONG rows
  rather than fewer of them. Empty is a refusal a caller can see; partial is not.

### S4 — "convert" and "identity" turn out to be the SAME operation

The design leaned on an asymmetry: where loft defines the table the conversion is
identity and nothing is paid; where it follows someone else's it accepts what is
there and converts. Building it found that at READ time there is no difference at
all — **every driver hands a value over as text**, because no `#c` path carries a
`double` by value (P3), so "parse a number kept in a `VARCHAR`" and "read a
`BIGINT`" are one code path. The conversion is chosen by what the loft FIELD
wants; the database column decides only whether it is *possible*.

That makes the asymmetry cheaper than costed, and it survives at the two ends
where it is real: at `CREATE`, where loft picks the cleanest column type, and at
the WRITE, where a value is bound.

**What is left is one genuine type refusal**, and naming it is what stops
`reconcile` degenerating into a function that always agrees: a column whose
engine type loft has no name for (`geometry`, `bytea`) is refused for a numeric
or boolean field. Whether an engine's text form of its own exotic type parses
back is a fact about the engine, and guessing it puts a plausible wrong number in
the record.

**The write verdict is the other half, and it is not a subset of the read
verdict.** A table with an extra `NOT NULL` column and no default is perfectly
readable — `SELECT` names only the columns loft wants — and no `INSERT` loft can
build will satisfy it. One `reconcile`, two answers, carried in the binding.

### S6 — the plan's own gate, at the library level, already runs

S6 was scoped as "read-only, changes no existing behaviour", and building it made
the whole of § The gate reachable a step early: `schema_live.loft` writes a
schema into an EMPTY sqlite database, reads it back out of the catalogue,
reconciles the two, and then does it again against a table made by hand with a
scrambled column order, a float kept in a `VARCHAR`, a boolean kept in `TEXT`
and one extra column. Both loft backends, byte-identical output.

**What is NOT yet covered is the half that needs core**: the values, the identity
across two paths, and the query counts. This gate proves the two DERIVATIONS
agree; S8–S14 prove a row arrives.

Two cells are worth keeping in view because they are where an easy version would
have looked right:

- **`declared flag=INTEGER` beside `flag conversion=ConvBoolean`.** sqlite has no
  boolean type, so loft writes one as `INTEGER` and it comes back as an integer;
  the BINDING is what makes it a boolean again. Either half alone reads as
  correct while being wrong.
- **`extra bound=true write=false`.** One table, two verdicts, from one
  `reconcile`.

**`introspect` was sqlite only at S6, and that was a scope statement.** S12
closed it: all four catalogues, each query run against a live server of its
engine first — and the columns half did unify on `information_schema` while the
INDEX half did not, which is the sort of thing only running it tells you. See
§ S11 + S12.

### S8 — the mechanism works, and it is smaller than @PLN129 costed it

A collection bound to a scheme core has no Rust driver for now calls a LOFT
function on a miss, re-entrantly, from inside the lookup:

```loft
fn lazy_fetch(coll: hash<Person[id]>, source: text,
              key_int: integer, key_text: text) -> integer
```

`1` inserted, `0` absent, and the third answer — *the source is down* — goes
through `store_lazy_fail`, because it carries a REASON and answering `0` for it
is exactly what arc C exists to prevent.

**Nothing about the call is new machinery.** `fn_call` pushes the frame and
stores the return address exactly as a `Call` op does, and
`State::run_until_return` runs the dispatch loop until that frame pops — so the
driver returns through the path every other call uses. The `execute_at*` family
could not serve: each RESETS `stack_pos` for a fresh par worker, which is right
there and would discard the caller's frame here.

**The structural change is where the miss/fetch decision lives.** It moved out of
`Stores` into its two callers, because `Stores` cannot run a loft function and
`State` can — the `&mut Stores` borrow has to end between the miss and the fetch,
which is only possible where it is taken. The retry itself is unchanged.

**Failure path 1 is closed as a side effect, and it was worse than the plan
said.** `postgres://…` used to classify as a `.store` image, so a miss reported
something about a paged reader. `LazySource::Loft` is now its own kind, and a
binding with no driver answers *"`postgres://…` needs a loft driver — define `fn
lazy_fetch(…)`"*.

**Gated** by `tests/fixtures/133-lazy-loft-driver.loft` through
`tests/lazy_sql_source.rs::a_lazy_fetch_can_be_a_loft_function`. Not in
`tests/scripts/`: that directory is swept on both backends under a leak gate, and
this program is asymmetric on both counts by design.

#### `--native` reaches the driver by a different route, and answers the same

`OpGetRecord` is compiled into libloft and cannot see a function the generator
wrote, so generated `init()` installs a POINTER to it
(`codegen_runtime::register_lazy_fetch`). Three things that were not obvious:

- **The driver is a reachability ROOT.** Nothing in loft calls `lazy_fetch`; the
  RUNTIME does, on a miss. A walk rooted at `main` never reaches it, so
  `--native` emitted no body for the function it was about to install a pointer
  to — and the failure mode is the quiet one, a program that compiles and simply
  has no driver.
- **The signature has ONE home.** `Data::lazy_fetch_driver` checks it, and both
  backends ask there: the interpreter pushes arguments positionally and the
  generator installs a pointer of a fixed type, so a driver whose shape they
  disagreed about could not exist. A wrong signature is refused rather than
  called — arguments are positional, so calling it reads someone else's bytes as
  its own, which is a wrong VALUE rather than a failure.
- **Containment needed a second mechanism.** The interpreter contains a fault by
  stopping its dispatch loop; native has no loop, and its depth guard called
  `process::exit`. Inside a driver it now UNWINDS and `OpGetRecord` catches it,
  with the panic hook silent for exactly that case — a lookup that correctly
  answered null must not also look like a program falling over.

**The gate is `assert_eq!` on the whole output of both backends**, not a line at
a time: two mechanisms reaching one answer is the claim, and a divergence nobody
predicted shows up only in the comparison.

#### The releasing unwind — done, and what made it safe

P4 measured one leaked store per contained fault and left the fix open. What
closed it was measuring *which* store: a driver that faults having allocated
nothing leaks nothing, and one that allocates first leaks exactly what its
abandoned frames held — on BOTH backends, identically. Native's unwind runs
Rust's drop glue and still leaked, which says the missing free is loft's own
scope-exit code on both sides rather than anything backend-specific.

**The cause is not S8's.** A raise in loft short-circuits the dispatch loop, so
the scope-exit frees the compiler emitted never run. The suite already knows
this — `SCRIPTS_LEAK_ALLOW`'s history records scripts that abort mid-`main` being
exempted for exactly this reason. It is harmless for a program about to exit. S8
is the first case where a raise happens and the program CONTINUES, so it is the
first place the leak accumulates.

**The fix**: while a driver runs, remember every store it creates; on a fault,
free those. Not the abandoned frames' variables — `State::iter_frame_variables_at`
could enumerate them, but only on the interpreter, and the two backends had just
been made to agree.

**What makes it sound is a measurement, not an argument.** The risk was freeing a
store the collection had come to point into — a use-after-free, strictly worse
than the leak. `tests/fixtures/133-lazy-unwind.loft` is nine cells that each try
to leave something REACHABLE behind before faulting: an inserted row, a text
built during the call, a filled vector, a value returned from a second frame, two
rows either side of an allocation, a fault with no allocation, and a successful
fetch. Each is read back late, after fifty more faults, and the run asserts the
collection's heap verifies and nothing is unfreed.

Two break-checks:

- Remove the free → the leak returns, 128 stores across 57 faults. The matrix
  catches its absence.
- Free the candidates on the SUCCESS path too → **nothing changes**, and that is
  the stronger half of the safety argument. Even where every store the driver
  produced is definitely still needed, none of them is a candidate: an insert
  COPIES into the collection's store, so a driver's new stores are only ever its
  own locals. The `!faulted` guard stays because it is right in principle, not
  because the matrix needs it.

### S7 had a design question the plan did not name: `SqlDb` is STATIC dispatch

Requirement 1 is *one string switches every SQL consumer in the process*, and S5
delivers the half that is a parser. The other half — handing back a connection
the caller then uses — had no obvious spelling, because **loft interfaces are
static dispatch**: `SqlDb` is satisfied by four unrelated types, and no function
can return "one of them". `sql.loft`'s own header states this as the reason the
cursor is state ON the connection rather than a second type the connection
returns; it applied to the registry too.

> **Half of that has since changed.** @PLN125 arc A gave interfaces associated
> types, and @PLN138 moved the cursor onto one — so `sql.loft` no longer says
> what is quoted above, and a connection yields as many cursors as a caller
> wants. The registry's half stands: `AnyDb` is still an enum, because choosing
> an implementation at RUN time is a different question from naming a companion
> type at compile time. It gained an `AnyRows` beside it.

Three shapes, and the choice was not free:

1. **A struct-enum over the four backends** (`enum AnyDb { DbSqlite { sq: Sqlite }, … }`).
   This is loft's native polymorphism — but it names all four backends in one
   type, so a program that wants only sqlite links the other three, and adding a
   fifth edits the enum.
2. **The caller matches on `Conn.backend` itself** and holds a concrete type.
   Honest and zero-cost, and it makes requirement 1 a smaller claim than it
   sounds: the string picks the driver, and the caller still has a `match`.
3. **Core does the dispatch**, which is S8 and later — core holds no loft type,
   so it calls a loft function by name and the registry can be inside that
   function.

**(1) is what was built**, and the decision is recorded in
`tests/fixtures/sqldb/registry/src/registry.loft`'s header rather than left to be
inferred: it keeps the promise requirement 1 makes — one string, and no `match`
in any consumer — and the linking cost is the price of a uniform API over four C
libraries. (2) stays defensible and cheaper, and the difference is visible to
every consumer, which is why it belongs in a file.

### S7 — the correction the shape needed, and the two it did not survive

**The method must be on the ENUM, not on the variant.** The obvious reading of
loft's struct-enum polymorphism is `fn db_exec(self: DbSqlite, …)` per variant,
and that dispatches correctly — but it does not satisfy an interface for the
enum, which the compiler says outright: *"'AnyDb' does not satisfy interface
'SqlDb': missing db_exec"*. Fifteen `match self` forwarders is the whole cost,
and none of them decides anything.

**Two neighbouring language defects had to be fixed to write it, and both are
core, not fixture.** Neither was in this plan's code and both reproduce on the
released binary:

- **`Type::is_same` did not peel `Optional`.** Every dep-ignoring rule in it (a
  text's deps, an integer's range, a vector's element buffer) was unreachable for
  a `τ?`, because derived `==` on the wrapper reaches the inner `Deps`. Two
  `text?` differing only in which local they came through read as different
  types, and it presents as a refusal quoting the same name twice: *"cannot
  unify: text? and text?"*. `db_col` is exactly the shape — four backends, one
  of which (`duckdb`) returns through a local. Peeled on BOTH sides only, so a
  `τ?` and a bare `τ` stay different kinds, which is the whole of DN1.
- **A tail branch delivered a BORROW into the return accumulator.** An arm whose
  text is not a bare variable is built into a work buffer and handed back as
  `OpCreateStack(buf)`; `push_text_arms_into` wrapped that reference in the
  delivery, and the enclosing scope frees the buffer on the next statement. The
  interpreter answered `""` — a wrong value, silently, exit 0 — and `--native`
  emitted `*var_acc = ().to_string()`, which is not Rust. The shape is
  `return x ?? "fallback"`, and binding to a local first
  (`y = x ?? "fallback"; return y`) avoided it, **which is what made it look like
  a bug about `??` rather than about delivery**.

**The instructive part is how the second was found.** It was not found by the
registry — the registry does not contain that shape. It was found by the
REGRESSION TEST written for the first fix, whose helper happened to spell
`return got ?? "<null>"`. A guard written for one defect walked into another,
which is the argument for writing the guard rather than checking the fix by hand.

**And one is filed rather than fixed** — [loft#806](https://github.com/loft-lang/loft/issues/806):
a METHOD call coalesced in RETURN position (`return t.m(i) ?? "x"`) SIGSEGVs the
interpreter while `--native` is correct. The caller-retbuf promotion (loft#662)
makes the callee's work buffer a `&text` PARAMETER, and the `#default ref` site
then wraps an already-borrowed variable in `OpCreateStack`, building a reference
to a reference. `src/parser/mod.rs` guards exactly that for non-text references;
the text arm below it does not. Filed because the fix is a decision inside the
promotion path that reaches every text-returning call, not just this one; the
workaround is one intermediate local.

Gated by `tests/fixtures/sqldb/registry_pure.loft` (unconditional — it opens no
library) and `registry_live.loft`, through `tests/native.rs`, on both loft
backends with the whole output compared. The two core fixes have their own
guarantee probes in `tests/scripts/` — `pln133-optional-unify.loft` and
`pln133-text-tail-delivery.loft` — and both were run against a pristine `main`
build first, where each fails in the way its fix describes.

**What S7 settled that the plan had not costed: the connection string is not one
string.** `parse_conn` says which driver a string names; what that driver's own
`db_open` wants is a fact about the DRIVER, and the answers genuinely differ —
sqlite and duckdb take a path, libpq reads a URI itself (so it must arrive WITH
its scheme, which is why `Conn` carries the whole string beside the target), and
mariadb's client takes keywords, so `mysql://ada:secret@db.host/loft` has to be
TRANSLATED. That last one produced a refusal worth keeping: **a port is refused,
not dropped**, because this driver connects on 3306 and nothing in its `db_open`
reads a port — honouring the string would reach a different server than it names,
which is a plausible answer from the wrong place rather than an error.

**And the session setup is why `connect` has to exist rather than merely be
convenient.** P3 measured PostgreSQL returning 1887 of 2000 random doubles
inexact at `extra_float_digits = 0` and 0 of 2000 at 1 or 3. That is a SESSION
setting, so the precision a float reads back at is decided by whoever connected —
nothing downstream can fix it, and nothing downstream can see it is wrong.
`Dialect.setup` had carried those statements since S3 with no one to run them;
`connect` is the one place that can.

### S9's prerequisite: per-type driver dispatch — and the corruption it was hiding

**Done 2026-08-08.** The probe that was meant to measure a LIMIT found a wrong
value instead, which is the more important half of this entry.

**What was there.** `Data::lazy_fetch_driver` looked up `n_lazy_fetch` — one
def_nr — so a program declared exactly one driver and a second was refused with
*"Cannot redefine 'lazy_fetch'"*. That reads as a limit on how many collections a
program may lazily bind. It is not:

> **Nothing checked that the driver a miss reached was declared for THAT
> collection.** A program with two lazily-bound element types ran the FIRST
> type's driver against the second collection.

Measured on both backends, with two `hash` collections bound to two sources:
`w.orders[9]` ran the driver written for `TdcPerson`, which inserted a
`TdcPerson` into a `hash<TdcOrder[id]>`, and reading `.what` back gave
`person-9-postgres://db/people` — one type's field read through another type's
offset. Not an error, not a null: a plausible value, which is the class @PLN129
arc C exists to keep out of the value channel. S8's own shape check was about the
driver's SIGNATURE and never about its subject.

**One mechanism fixes both.** The driver is looked up by the collection's ELEMENT
TYPE, which makes several drivers possible and makes reaching the wrong one
impossible — the limit and the corruption were the same missing fact.

- **What a driver serves is read off its declared collection parameter**, never
  guessed from its name. `Data::lazy_fetch_drivers` answers
  `(element type name, def_nr)` for every driver, and that is the single home both
  backends ask.
- **The key is a NAME, not a number.** The two sides count types in different
  spaces — a parse-time `Definition` and a runtime `Stores::types` entry — and a
  name is the one key both hold without a mapping to keep in step. @PLN133 S8's
  own `LOFT_STRICT_SCHEMA_IDS` exists because that kind of mapping drifts.
- **Membership needs more than a name.** `lazy_fetch` exactly is THE driver name,
  so a wrong shape there is named rather than walked past; `lazy_fetch_<anything>`
  additionally requires a keyed collection as its first parameter. That second
  rule was not fussiness — anyone writing a driver names its helpers after it
  (`lazy_fetch_row`, `lazy_fetch_query`), and under a name-only rule each helper
  was read as a malformed driver and poisoned every lookup in the program,
  including the working driver beside it. The first version did exactly that.
- **Two drivers for one element type are refused, naming both.** Silently picking
  one is the same wrong-value class in a new place.
- **`--native` installs one pointer per driver**, keyed on the same name
  (`register_lazy_fetch("TdcPerson", n_lazy_fetch)`), and every driver is a
  reachability ROOT — a driver left out of the walk is the quiet failure S8
  already documented, arriving once per type instead of once per program.

**A backend divergence had to be closed to gate the refusals**, and it was S8's,
not S9's: the interpreter asks `Data` at every miss and reports the sentence it
wrote, while `--native` cannot ask `Data` at all — it registered nothing and said
*"needs a loft driver"*. The same program named a different mistake depending on
which backend you ran, and the one naming the ACTUAL mistake was the one you did
not get if you compiled. The refusal now travels as data
(`register_lazy_fetch_refusal`), and the "no driver" sentence has one home
(`database::lazy::no_lazy_driver`) so the two cannot word it differently.

**The emission diff is one line.** `loft introspect` over the two-driver corpus
before and after differs only in the registration — one
`register_lazy_fetch(n_lazy_fetch)` becoming two keyed calls. Nothing else in the
IR, the bytecode or the generated Rust moved, which is the whole claim of the
change. Corpus and both captures: `bytecode-comparisons/two-drivers-*`.

Gated by `tests/fixtures/133-lazy-driver-dispatch.loft` (three element types over
`hash` and `index`, a fourth bound with no driver, a helper sharing the prefix,
absent-vs-unreachable) plus the two refusal programs, through
`tests/lazy_sql_source.rs`, both backends with the whole output compared. **The
cell that matters is `orphan`**, and its assertion is a driver-call COUNT rather
than a value: a collection whose type no driver serves must reach none, and a
value check alone would pass on a driver that happened to answer nothing.

### S9 — sqlite down the loft path, and the two paths measured against each other

**Done 2026-08-08.** The step is *"switch sqlite to the loft path"*, and taken
literally it cannot preserve what it must: every @PLN129 test binds `sqlite:` with
NO user code, so routing sqlite to loft wholesale leaves them with no driver.
`store_bind_lazy(persons, "sqlite:people.db")` needing no loading step is the
promise CHANGELOG.md already ships.

So S9 is an OPT-IN with a measurement, and the two halves are separable on
purpose:

- **A declared driver WINS, including over a source core drives in Rust.** A
  program moves its sqlite reads onto loft one element type at a time; every type
  with no driver stays on the Rust source, unchanged. That is what makes the swap
  measurable rather than a flag day — and the whole @PLN129 suite is the control,
  because nothing in it declares a driver.
- **The two paths are proven indistinguishable.**
  `tests/fixtures/sqldb/s9_two_paths.loft` puts two element types of one shape
  over two identical tables in ONE program bound to ONE connection string:
  `S9Rust` has no driver, `S9Loft` has one. Same values, same float, same
  identity, same residency counts, same absence handling — and the trip count,
  which is the only thing a value check cannot see. Both backends, byte-identical.

**Nothing in the driver names a column.** The table, the columns and the `WHERE`
come from `derive(type_of(coll))` — the same `TableDef` a writer would `render`
into `CREATE TABLE`. That is requirement 2's one derivation serving both
directions, and it is what `select_by_key` (new here, the `select(TableDef, key)`
the design table always listed) exists to do. It wraps a float column in the
dialect's read expression, because a portable `SELECT score` is lossy on two
engines of four and silent about it (P3).

#### The cost, attributed rather than assumed

A loft driver has **nowhere to keep a connection**: loft has no process-level
state a library can hold, so the driver connects, queries and disconnects per
missed row, where core's Rust source caches a handle per target. Measured on a
release build, 400 single-key fetches each:

| | per fetch |
|---|---|
| core's Rust source | **67 µs** |
| the loft driver (connect + derive + query + close) | **140 µs** |

**~2.1×, not the order of magnitude the shape suggests** — a local sqlite file
reopens cheaply. The number is worth keeping for what it does NOT cover: for a
client-server backend the same shape is a TCP connect and an auth per row, and
those are exactly the backends core has no Rust driver for, so it is the case
that matters most and the one no measurement here reaches. Connection reuse is
therefore a real requirement of the write side (S13) rather than a nicety, and it
needs somewhere for a library to keep state.

### S10 — the deletion is REFUSED, and the plan is better for it

S10 was written as *delete core's 15 typed externs and `sql_query.rs`*. Building
S9 turned that into a question, and the answer is **no**. It is a decision rather
than a slice, and stating it is the honest close.

**Deleting the Rust path makes a driver MANDATORY**, and a driver names a
concrete element type, so it cannot be generic and cannot come from a library.
`store_bind_lazy(persons, "sqlite:people.db")` needing no user code is a promise
CHANGELOG.md already ships. Under
[COMPATIBILITY.md](../../COMPATIBILITY.md) that settles it on its own: no
functioning program breaks, and this would break every one that binds `sqlite:`
without writing a driver.

The one alternative the plan named — core SYNTHESISES a per-type driver whose
body calls the loft sqldb library — trades the compatibility break for a
dependency break. `tests/fixtures/sqldb` is a fixture, and core cannot require a
fixture. Publishing it as a registry library does not help: core would then need
an INSTALLED library to serve its own built-in connection scheme, which moves a
guaranteed capability behind a download.

**So S10 is a demotion, and S9 already shipped it.** The precedence rule is the
whole answer: *a declared driver WINS, per element type; where there is none,
core's Rust source serves.* What that buys is not deletion but a stopped clock —
core's Rust never gains a fifth backend, every new one is a loft driver, and the
duplication is frozen at what exists instead of growing with the backend count.
That was the plan's actual complaint, measured in its own § What is actually
duplicated: N=4 now and **+1 forever**. The +1 is gone.

The deletion becomes possible the day the derivation ships INSIDE loft's
distribution rather than as a fixture, because only then can core synthesise a
driver without depending on something a user might not have. That is a decision
about what the distribution contains (@PLN112, the registry), and it is named
here so the next person does not re-derive it from scratch.

### S11 + S12 — create-or-follow is ONE call, on four engines

`ensure(d, dial, want)` is the whole absent-or-present decision, and it is one
function because it is one DECISION: splitting it would put the test in every
caller, which is where two callers eventually disagree about what absence means.

The rule it holds: **loft never touches a table it did not find missing.**
Creating a table where there was none fills an absence. Altering one that is
there mutates somebody else's schema, and the fact that loft could derive a
better one is not permission — so a mismatch REFUSES, naming the column or the
index, and `reconcile` was already written to do exactly that.

**The read-back is not ceremony.** `ensure` creates, then reads the table BACK
out of the catalogue and reconciles against what the engine actually stored — not
against the derivation it just sent. An engine rewrites what it was given:
mariadb stores `BOOLEAN` as `tinyint(1)` and reports `tinyint`; sqlite keeps the
affinity of a declared name nobody else would recognise. Reconciling against the
derived definition would ASSERT the round trip instead of testing it, and the
whole point of this function is that a table loft made and a table loft found are
indistinguishable afterwards.

**S12 needed `introspect` to stop being sqlite-only**, which is the work S6
deferred with its scope statement. All four catalogues now, and each query was
RUN against a live server of that engine before it was written down:

| | columns | indexes |
|---|---|---|
| sqlite | `pragma_table_info` | `pragma_index_list` + `pragma_index_xinfo` |
| postgres | `information_schema.columns`, scoped `current_schema()` | `pg_index` ⋈ `pg_attribute` |
| maria | `information_schema.columns`, scoped `DATABASE()` | `information_schema.statistics` |
| duckdb | `information_schema.columns`, scoped `current_schema()` | `duckdb_indexes()` |

The columns half unified and the index half did not, and the table says so rather
than pretending: `information_schema` has **no index view at all**. duckdb is the
sharp end — it answers the `CREATE INDEX` TEXT rather than a row per column, so
its columns are read back out of its own normalised rendering.

Two facts measured on the way, both of which would have been guessed wrong:

- **PostgreSQL's `indkey`/`indoption` are 0-based `int2vector`s.** Subscripting
  `indoption[ord-1]` reads every direction as NULL, which looks like "no index
  is descending" and is not.
- **A whitelist, not sqlite's substring test.** sqlite has AFFINITY — a rule the
  engine applies to any declared name — so following it agrees with the engine.
  The other three have real types, and a substring test there is a guess wearing
  a rule's clothes: PostgreSQL's `point` contains `INT`. An unlisted type answers
  `ColOther`, which `reconcile` refuses BY NAME.

### S13 — the statement, and the half that is not this plan's

`insert_row(want, dial, values)` renders `INSERT INTO "person" ("id","nm",…)
VALUES (?,?,…)` from the same `TableDef` the reader's `SELECT` comes from, with
every value BOUND. That is requirement 2's load-bearing half: a writer that named
its own columns could drift from the reader by one rename, and nothing would
catch it until a row came back wrong.

**What is NOT built, and cannot be here:** the generic walk that takes an
arbitrary loft struct and produces those values. loft's reflection reports TYPES,
not VALUES — `TypeInfo`/`FieldInfo` describe the layout and there is no way to
read field *n* of a record generically — so an ORM writer has to be either
generated or hand-written per type. That is @PLN23 S5's problem plus a language
gap, and it is named here rather than absorbed: this plan's claim is that the
STATEMENT has one derivation, and it does.

The gate therefore supplies its own values in the definition's column order, and
a count that does not match REFUSES — a row written into the wrong columns is a
wrong value that reads back perfectly.

### S14 — the gate, and it passes on four backends where it asked for two

`tests/fixtures/sqldb/round_trip.loft`, driven by
`tests/native.rs::a_structure_written_is_immediately_readable_through_one_connection_string`.
Write three rows through the derived `INSERT`, bind a collection lazily to the
SAME connection string, traverse it, and read back values, identity and the trip
counts laziness predicts.

**Run twice, and the second run is the one that matters.** Run 1 goes into an
empty database, where loft writes the schema. Run 2 goes into a table made by
hand — different column ORDER, the float kept in a `VARCHAR`, and an extra
column loft knows nothing about — where loft must FOLLOW it. Same program, same
assertions. Run 1 passes even if `reconcile` is a stub that always agrees; only
run 2 proves requirement 3.

**Byte-identical across eight cells**, measured 2026-08-08 on this box
(sha256 of stdout `70fbf61a…` for every one):

| | `--interpret` | `--native` |
|---|---|---|
| sqlite | ✅ | ✅ |
| postgres 16.14 | ✅ | ✅ |
| maria 10.11.14 | ✅ | ✅ |
| duckdb 1.5.5 | ✅ | ✅ |

CI reaches sqlite only, as ever; `LOFT_SQLDB_MODE` selects the others and the
four-backend run is a LOCAL measurement, written here because that is where it
was made.

**The trip count is the oracle the values cannot be.** A reader that fetched the
whole table would return exactly these rows; only the number of trips separates a
lazy read from an eager one. Six per program — 42, 7 and the absent 999 in each
run — and the repeat of a resident 42 reaches nothing, which is what says the
working set is the authority rather than a cache beside it.

The float is `0.5`/`0.25`/`0.125` on purpose. P3 measured that sqlite and duckdb
lose the low bits of most doubles on the WRITE side; that is a bounded, recorded
limit of the write path, not something a round-trip gate can fix, so this one
uses values every engine stores exactly and claims exactness only for those.
`uniform.loft`'s `floats` cell is where the full-precision question lives.

#### What the gate cost, in defects found

- **[loft#810](https://github.com/loft-lang/loft/issues/810)** — fixed on the way
  in (a method's return adopted by one half of the compiler and freed by the
  other). It was filed off this plan's cost measurement and turned out to need
  no library at all.
- **[loft#813](https://github.com/loft-lang/loft/issues/813)** — found extracting
  a helper out of `introspect`: a value whose static type is a struct-enum
  VARIANT (`x = AsA { … }` rather than `x: Any = AsA { … }`) is accepted where a
  bounded generic wants the ENUM, and then answers the type's empty value. Silent
  on `--interpret`, a `todo!()` panic on `--native`, a SIGSEGV with two generic
  hops. The fixture is unaffected because it annotates; the extraction was
  reverted, which is why `introspect` still scores 40 on the complexity nudge.

#### A `Dialect?` that is known non-null still warns

`d.db_dialect()` answers `Dialect?`, and an `if dial == null { return }` above the
use does not narrow it — so every consumer here carries *"a nullable `Dialect?` is
stored into parameter 2 … it becomes null there"*. It does not; the fixture runs
with `--no-warnings`, and `s9_two_paths.loft` has carried the same line since S9.
Recorded rather than worked around: the honest discharge would be to fabricate a
`Dialect`, and a `Dialect` has no honest empty value — which is the whole reason
`dialect_named` returns null instead of a default.

#### A store-lifetime crash the probes found — [loft#810](https://github.com/loft-lang/loft/issues/810)

Attributing the cost above walked into a SIGSEGV that has nothing to do with lazy
loading: a function **in a library** that both holds a `vector` local and
builds+returns a record of ANOTHER package's type crashes on the second call, when
the caller binds the result to a loop-body local. `Store::copy` computes
`size * 8 - 4` from a record whose size word reads `0`; in release that wraps
rather than panicking, so it arrives as a segfault.

Six axes were moved one at a time to find the boundary — unroll the loop, drop
the local, drop the vector, return the own package's record, move the function
into the program — and each one alone makes it run. It does not block S9: the
driver derives its `TableDef` per fetch, which is the fresh-argument cell that
passes. Filed rather than fixed because the question is why `record_new` reaches
a zero-size record, which is store/vector work, and the six-way boundary is what
a fix has to keep green.

### Five language defects the build surfaced, all pre-existing on `main`

None of them is in this plan's code, all three reproduce on the released binary,
and each has a workaround the schema package now uses:

- **[loft#792](https://github.com/loft-lang/loft/issues/792)** — a reflected
  record passed as a call ARGUMENT leaks on `--interpret`, and takes the callee's
  freshly built vector with it from the second call on. `f(type_of(x))` leaks;
  `t = type_of(x); f(t)` does not. It is the temporary, not the type: a
  hand-written struct of the same shape leaks nothing.
- **[loft#793](https://github.com/loft-lang/loft/issues/793)** — a LIBRARY
  function returning `T?` answers **null** when the value came from a call.
  Silent, both backends, and it is the reason `dialect_named` answered null for
  every backend it had a dialect for. Measured with a repeat-run harness because
  the first run happened to pass: **1 correct in 20** on `--interpret`, **0 in 6**
  on `--native`, **10 in 10** with the same code in one file, and **20 in 20**
  with the result bound to a local first.
- **[loft#794](https://github.com/loft-lang/loft/issues/794)** — reading BOTH
  loops' `#count` inside a nested loop body aborts the compiler. Loud, so no
  wrong answer, but it has no workaround and the natural spelling of a
  match-by-position probe runs straight into it.
- **[loft#795](https://github.com/loft-lang/loft/issues/795)** — `_` is exempt
  from the one-type-per-name rule every other local obeys, and `--native` then
  emits Rust that will not compile. Three assignments are needed: `FileResult`,
  `boolean`, `FileResult` again. The same code with a NAMED local is refused
  cleanly today, with a message that says what to do — so the fix is to stop
  exempting `_`, which closes the backend divergence and the missing diagnostic
  at once.

**The one worth generalising is #793**, and not for its cause: a single run of
the broken shape passed. A wrong answer that is only *usually* wrong reads as a
flake, and every "I could not reproduce it" in this session came from trusting
one run. The harness — clear the cache, run twenty, count — is what turned it
from an anecdote into a filed defect with a verified workaround.

## Implementation steps — small, safe, each one green

Ordered by **risk, not dependency**: the two unknowns are probed before anything
depends on them, and everything that can land INERT does so first. Every step
below leaves the tree green and nothing half-wired, so a step that goes wrong is
one revert rather than an unpicking.

**The safety net is already built.** @PLN129's tests assert query COUNTS — one
lookup, one query; a repeat lookup, zero — and arc F asserts identity across a
graph traversal. Those are the oracle for every step that replaces the driver: the
values can stay right while the counts go wrong, which is exactly the failure a
swap causes.

### Probes — nothing ships

| # | do | answers |
|---|---|---|
| **P1** | ~~a nested loft call from inside a lookup~~ | **DONE 2026-08-06 — PASSES.** Option B is viable. See § Probe results. |
| **P2** | ~~float through a text round trip~~ | **DONE 2026-08-06 — reads exact, sqlite writes lose one ULP at extreme exponents.** See § Probe results. |
| **P3** | ~~the float round trip on all four backends~~ | **DONE 2026-08-06.** Reads exact everywhere once the rendering is SPECIFIED; writes lose bits on sqlite and duckdb. |
| **P4** | ~~contain a fault inside the nested run~~ | **DONE 2026-08-06 — contained, but it LEAKS.** Needs a releasing unwind, not a stack truncation. |

### Inert — pure values and pure functions, nothing calls them

| # | do | green because |
|---|---|---|
| **S1** | ~~Reflection gains the collection KIND, its key fields and the direction bit, each carrying `position`.~~ **DONE 2026-08-07** — `TypeInfo.collection` (`CollectionKind`) and `TypeInfo.keys` (`vector<KeyInfo>`); gated in `tests/scripts/pln127-reflect.loft` on both backends. | additive; the new fields have no reader yet |
| **S2** | ~~`TableDef` the value, and `derive(T)` from reflection.~~ **DONE 2026-08-07** | a pure function with unit tests |
| **S3** | ~~`render(TableDef, dialect) -> DDL`, including the index the collection kind implies.~~ **DONE 2026-08-07** | unit-tested against hand-written expected DDL per dialect — hand-written, so agreement between two generators cannot pass for correctness |
| **S4** | ~~`reconcile(want, have) -> Binding \| Refusal`, carrying per-column conversions.~~ **DONE 2026-08-07** | pure; tested on hand-built pairs: exact match, missing column, incompatible type, extra column, missing index, a number in a `VARCHAR` |
| **S5** | ~~The connection-string parser: `scheme:rest` → backend name.~~ **DONE 2026-08-07** | pure; nothing routes through it |

S2–S5 live in `tests/fixtures/sqldb/schema/` — a package with no `[c]` section at
all, beside the four backends and the `SqlDb` interface. Gated by
`tests/fixtures/sqldb/schema_pure.loft` through
`tests/native.rs::one_table_definition_derives_reconciles_and_renders`, on both
loft backends, **unconditionally**: it holds no connection, so unlike every other
SQL test in that file it cannot skip. The derivation the reader and the writer
must agree on is the last thing that should have a gate that evaporates.

### Wiring — one consumer at a time, sqlite kept as the control

| # | do | safe because |
|---|---|---|
| **S6** | ~~`introspect(conn, table) -> TableDef?` in the loft library, sqlite only.~~ **DONE 2026-08-07** — with the round trip, run twice, in `tests/fixtures/sqldb/schema_live.loft`. | read-only; changes no existing behaviour |
| **S7** | ~~The backend registry, used by the LIBRARY's own connect. No core change.~~ **DONE 2026-08-08** — `AnyDb`, a struct-enum satisfying `SqlDb` itself; `connect(spec)` parses, checks availability, opens and runs the dialect's session setup. Two core defects had to be fixed to write it; a third is filed. | the library's four backends already pass their tests, and the registry did not move them |
| **S8** | ~~Core's lazy fault calls loft **for non-sqlite backends only**. Core's sqlite path is untouched.~~ **DONE 2026-08-07, both backends.** | every existing @PLN129 test still runs the old path — the suite is the control while the new path is proven beside it |
| **S9a** | ~~Per-type driver dispatch, the prerequisite S9 turned out to need.~~ **DONE 2026-08-08** — a driver is found by the collection's element type, read off its own parameter; several drivers per program, and reaching the wrong one is impossible. It closed a wrong-value hole S8 had left. | the emission diff is one registration line; every existing @PLN129 and S8 test is the control |
| **S9** | ~~Switch sqlite to the loft path too.~~ **DONE 2026-08-08** — a declared driver WINS over the Rust source, per element type, and the two paths are proven indistinguishable on one database in one program. | the count assertions are the oracle, and they held: same values, same identity, three trips and no fourth, both backends |
| **S10** | ~~Delete core's 15 typed externs and `sql_query.rs`.~~ **DECIDED 2026-08-08 — the deletion is REFUSED.** It would make a driver mandatory for `sqlite:`, breaking a shipped promise, and the alternative makes a fixture a dependency of core. S9's precedence rule IS the answer: a demotion, not a deletion. See § S10. | — |

### Create-or-follow, then the write side

| # | do | |
|---|---|---|
| **S11** | ~~Absent → `render(derive(T))`. Only into a table that is not there.~~ **DONE 2026-08-08** — `ensure`, which asks the CATALOGUE rather than leaning on `IF NOT EXISTS`. | a fresh database becomes usable with no setup |
| **S12** | ~~Present → `reconcile`, refusing with the column or index NAMED.~~ **DONE 2026-08-08** — the same call, and it needed `introspect` on all four catalogues. | a foreign database becomes usable with no rewrite |
| **S13** | ~~`insert(TableDef)`~~ **DONE 2026-08-08** — one derivation renders the reader's `SELECT` and the writer's `INSERT`. The ORM's generic struct walk stays @PLN23 S5: reflection reports types, not values. | |
| **S14** | ~~**The gate**, run twice.~~ **DONE 2026-08-08** — 4 database backends × 2 loft backends, byte-identical. | |

**Every step's cross-backend claim is a LOCAL measurement.** CI gates sqlite only;
PostgreSQL, MariaDB and duckdb are run locally and their results written down
where they were measured — see [TEST_ENVIRONMENTS.md § Database backends](../../TEST_ENVIRONMENTS.md).
A step that says "all four agree" without a local run beside it has not been
checked, and that is exactly the gap P3 found in the float rendering.

**What each wiring step must NOT do:** S8 must not touch the sqlite path (that is
what makes it revertible); S9 must not change the counts (that is what makes it
provable); S11 must not touch a table that exists (that is the whole of "the
database is the authority").

## The gate — the round trip, run twice

**MET 2026-08-08**, on four database backends and both loft backends, with
byte-identical output in all eight cells —
`tests/fixtures/sqldb/round_trip.loft` (§ S14 for the measurement). What follows
is the specification it was built against, kept as written so the claim can be
checked against what was asked for rather than against what was built.

Write a struct graph through the ORM to a database, bind a collection lazily to
the **same connection string**, traverse it, and get back what was written:
values, identity across two paths, and the query COUNTS laziness predicts. On
**both loft backends** and on **at least two database backends** — a round trip
that only closes on sqlite has not tested the string.

**Run it twice**, and the second run is the one that matters:

1. Into an **EMPTY** database — loft creates the schema and its index.
2. Into a table created **by hand**, with different column types, a different
   column order and one extra column — loft follows it.

Same loft program, same assertions, both green. The first run passes even if
`reconcile` is a stub that always agrees; only the second proves requirement 3.

The gate cannot pass vacuously: if the two derivations disagree the bind fails, if
the writer omits the index the bind is refused, if the snapshot is pinned wrong
the traversal reads nothing, and if a conversion is missing a float comes back
almost right.

## What would falsify this design

- The re-entrancy probe fails (step 0) — then A, and the cost is four Rust
  drivers plus a decision about what the loft library is for.
- **A float cannot be round-tripped through text on some backend** (step 1). Then
  that backend needs the `#c` float capability after all, and @PLN128 E3 returns
  to being a prerequisite — for that backend only, which is worth knowing before
  it is promised.
- **`reconcile` cannot express some real foreign schema** — a composite primary
  key spread across columns loft models as one field, a table where the "key" is a
  view, a type whose text form is not parseable back. Then requirement 3 holds for
  a NAMED class of schemas rather than universally, and the class has to be
  written down instead of implied.
- A fifth backend appears whose protocol the `SqlDb` interface cannot express —
  then "one driver per string" is a smaller claim than it looks and the interface
  needs widening before this is built on.
- **Reflection cannot carry a keyed collection's keys without becoming a second
  description of the layout.** @PLN127's stated design rule is that reflection
  must not drift into a parallel account of the same bytes. If exposing keys
  cannot be done as a READ of the same descriptor core uses, the derivation stays
  in Rust and requirement 2 is met by core generating the DDL instead — a smaller
  unification, and one that leaves the ORM asking core for its schema.
- **The round trip needs a writable path the ORM has not built.** @PLN23's S5–S7
  are designed, not built. **This one landed on its feet**: the writer's
  STATEMENT is derived here (`insert_row`), and only the generic walk from a
  struct's fields to its values belongs to the ORM — so the gate runs with the
  program supplying values in the definition's own column order. The falsifying
  case would have been a writer that had to name its own columns, and it did not
  arise.

## Cross-plan effect

- **@PLN129** (closed) — S10 deletes code it added; S8 grants it the three
  backends it never had. Its count assertions become this plan's oracle.
- **@PLN23** — its client becomes the ONLY driver, which raises the bar on it: no
  longer a fixture but the thing core depends on. Its S5–S7 ORM shares one
  derivation with the lazy read.
- **@PLN24** — option B preserves its thesis (`#c`, no rustc in a library) and
  makes it load-bearing; option A retires it for databases. Its arc F is already
  closed by @PLN23 S1; its arc E (wasm) is untouched by this.
- **@PLN127** — S1 extends its reflection surface, additively.
- **@PLN128** — its E3 float refusal moves from prerequisite to optimisation, on
  the strength of P2.
- **@PLN125** — associated types would let the per-column conversion be one
  generic method rather than one per kind. Not a blocker.

## See also

- [LAZY_STORES.md](../../LAZY_STORES.md) — the read side as built, including the
  derivation this plan moves and the refusals it retires.
- [plans/23-db-clients/OBJECT_MAPPING.md](../23-db-clients/OBJECT_MAPPING.md) —
  the S1–S7 ladder whose write half this depends on.
- [plans/129-lazy-database-stores/BINDING.md](../129-lazy-database-stores/BINDING.md)
  — the binding contract, whose "no DDL, no migrations" decision requirement 3
  revises.
- [plans/24-c-abi-binding/README.md](../24-c-abi-binding/README.md) — the `#c`
  machinery, the trampoline ladder, and the capability gap this design routes
  around.
- [INTERFACES.md](../../INTERFACES.md) — `SqlDb` is an ordinary bounded generic;
  § Interpolation targets is how its statements are built safely.
