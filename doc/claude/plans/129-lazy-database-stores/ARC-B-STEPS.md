<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN129 arc B — the implementation sequence

**Complete.** A `sqlite:<path>` binding faults on a miss, derives its own SELECT, materialises
the row into the collection and passes arc F's graph gate on both backends. What each step
ANSWERED, where it differed from what it expected, and the three findings that changed the design
are at the bottom (§ What the steps answered).

Step 8's implicit half is **declined**, not pending —
[DESIGN_DECISIONS.md § @C104](../../DESIGN_DECISIONS.md). The remaining tails are open work on the
built system and live in [LAZY_STORES.md § Open work](../../LAZY_STORES.md).

How B / B2 / B3 / B4 get built in steps that each land GREEN and each answer one question.
Companion to [README.md](README.md) (the model), [QUERIES.md](QUERIES.md) (what a binding can
ask) and [BINDING.md](BINDING.md) (the schema contract). Those three say what to build; this
says in what order, and what each step is allowed to leave undone.

## Why a sequence rather than a branch

Arc A shipped as one arc because the risk was one mechanism. Arc B is not that shape: it is a
derivation, a C surface, a materialiser and a schema check, and only the derivation is
cheap. Built as one change it would be a long-lived branch whose first green run is also its
last — the shape that hides which half is wrong.

So every step below is **independently landable**: the tree is green after it, nothing is
half-wired, and the step before it is still doing its job. Where a step cannot be verified on
its own, that is called out as the reason it is bundled with the next one.

**The order is chosen by risk, not by dependency.** The derivation is settled and testable
without a database; the C surface is the unknown. So the derivation lands first to get it out
of the way, and the unknown is probed before anything depends on it — step 0 exists for
exactly that.

## What is already answered

Two of the README's open questions are settled by arc A having shipped, and the steps below
assume them:

- **"How is *not resident* represented?"** — it is not a third record state. A miss is
  `find` answering `rec: 0`, and the fetch INSERTS through the ordinary path. There is no new
  header state in `valid()`, and the hot path is unchanged. This was the most load-bearing
  unknown and the file source retired it.
- **"Does `--native` route reads through the same accessors?"** — yes, measured; N = 1 on
  both backends.

What remains open is genuinely arc B's: who executes, and what the row→record path costs.

---

## Step 0 — probe the C surface before designing against it

**Question:** can core drive a C library through `c_call::resolve` from inside a store
accessor — no rustc, no re-entrancy, no loft frame?

**Do:** a throwaway probe that resolves one symbol from the sqlite fixture library and calls
it from a Rust unit test. Nothing in `src/database/`. Assert it returns a value and that a
second call in the same process reuses the handle.

**Why first:** every later step assumes this works. QUERIES.md § arc B settles the *shape* by
reasoning about `State::fn_call`; it does not demonstrate the call. If this probe fails, the
sequence below is wrong from step 4 onward and the alternative (teaching the bytecode to
resume a lookup) has to be re-opened — which is a far bigger change and better known now
than at step 6.

**Landable:** nothing ships. The probe is deleted or graduated into step 4's tests.

---

## Step 1 — give the source a seam, with one implementation

**Question:** can today's file fetch move behind an interface without changing what it does?

**Do:** extract the `load_key` / `load_key_text` calls in `fetch_missing` behind a
`LazySource` enum with a single `File` variant. Same functions, same order, same errors.

**Verify:** every arc A/C/D/E/F test unchanged and green. This step's whole claim is that it
changes nothing — so a test that had to be edited is the signal it changed something.

**Does NOT:** add a second variant. A seam with one implementation is the cheap half; adding
the second is step 6, and keeping them apart is what makes step 6's diff readable.

---

## Step 2 — the derivation, pure and unwired

**Question:** does the descriptor carry enough to write the SELECT?

**Do:** `derive_select(desc, collection_type, shape) -> Option<Sql>` in its own module.
Table from `names[elem]`; columns from the elem's `Record(fields)`; `WHERE` from the
`Iterated` keys, each matched to its column **by position** (`Key.position` ==
`LayoutField.position`), never by name-guessing; `ORDER BY` from the `(u16, bool)`
direction bit. Shapes: equality (`Hash`, `Radix`), range (`Sorted`, `Ordered`, `Index`).

**Verify:** unit tests over the two worked examples in QUERIES.md § the derivation — the
`Person` equality SELECT and the `Position` composite range with its `ORDER BY … ASC` — plus
the cases the doc does not spell: a composite hash key, a descending direction bit, and a type
whose descriptor has no `Record` node (must return `None`, not a malformed query).

**Landable:** nothing calls it. A pure function with tests is green by construction.

**Does NOT:** quote or escape identifiers — that is step 3, because it belongs with the
mapping that introduces illegal names.

---

## Step 3 — the declared mapping, feeding the same builder

**Question:** where does `Person.name → persoon.naam` live, and does the override path stay
one builder rather than two?

**Do:** a mapping value (type→table, field→column) that `derive_select` consults, defaulting
to the descriptor when absent. Identifier quoting lands here, driven by the mapping's own
rules rather than by guessing a dialect.

**Verify:** the same worked examples with and without a mapping; a loft field name that is not
a legal SQL identifier; and a mapping naming a field the type does not have (refused at
construction, not at query time).

**Does NOT:** decide how a mapping is WRITTEN in loft source. That is a surface question and
belongs with B3's compile-time half; this step is the value and its consumer.

---

## Step 4 — a database source that executes

**Question:** can the seam from step 1 take a second implementation that runs the string from
step 2?

**Do:** the narrow C surface QUERIES.md names — connect, execute, next row, column value,
close — behind a `LazySource::Sql`, driven by `c_call::resolve` per step 0. Which library
provides them is configuration, the same shape `[c] optional-libs` uses.

**Verify:** against the sqlite fixture, because it needs no server and therefore keeps the
test honest on a machine with no database. A row comes back and its columns are readable.

**Does NOT:** materialise a record yet. This step ends at "the row is in Rust's hands", which
is the boundary where the next question starts.

---

## Step 5 — row → record, through the path that already exists

**Question:** does a fetched row become a record that the collection owns, with identity
intact?

**Do:** materialise via the same claim/insert the file source uses, so a SQL arrival and a
file arrival end in the same place. The collection is asked first and the fetch re-runs the
lookup afterwards — arc A's rule, unchanged.

**Verify:** the identity assertion arc F already makes, now over SQL — two paths to one
person give `is_same`. And the NULL crossing: SQL `NULL`, `''` and a value must stay three
distinct answers, which is the distinction @PLN23's uniform fixture exists to preserve.

**Bundled with step 6 if it cannot be tested alone** — a materialiser with no caller has no
observable behaviour, and a test that reaches into it privately would assert the
implementation rather than the contract.

---

## Step 6 — wire the miss path; arc B is live for equality

**Question:** does a keyed lookup on a database-bound collection fetch exactly one row?

**Do:** `fetch_missing` selects the source by binding kind. Everything before this step is
inert; this is the switch.

**Verify:** the counting assertions, which are the ones that matter — one lookup, one query;
a second lookup of the same key, zero queries. Falsify by making every lookup re-fetch and
confirm the counts go red while the values stay right, exactly as arc F did for the file
source.

**Landable:** B's headline claim is true from here. B2/B3/B4 extend it; nothing after this is
required for a keyed lookup to work.

---

## Step 7 — the bind-time schema and index check

**Question:** is the bind refused when the schema cannot serve it?

**Do:** at bind, verify the columns exist with compatible types and that the index a
`hash`/`index` collection assumes is present. Refuse otherwise, reported through arc C's
existing channel.

**Why here and not earlier:** BINDING.md calls the missing index the main risk surface — "not
an error, a silent collapse from lazy to table-scan". It cannot be checked before step 4
(there is no connection to ask) and must not be later than the first real consumer, because a
consumer that runs green over a table scan will be believed.

**Verify:** a table missing a column, a column of an incompatible type, and a `hash` binding
over a table with no index on its key — each refused, each naming what was wrong.

---

## Step 8 — the second query shape: ranges and composites

**Question:** does a `sorted`/`index` slice become ONE query rather than N lookups?

**Do:** wire the range shape derived in step 2 to the slice operation.

**Verify:** the batching claim from QUERIES.md — a range slice issues one query returning many
rows, and the N+1 shape it replaces is visible in the counts.

---

## Step 9 — B2, the explicit escape hatch

**Do:** run a given query, materialise the rows INTO the collection. The generalisation of
`store_load_keys`.

**Verify:** the requirement that ties the model together — a `LIKE` query and a walk from a
company reach the SAME person records. A result set that is a detached list is the failure,
and it is what the test has to be able to catch.

---

## Step 10 — B4, collection-valued fields

**Do:** `company.people` as an owner-parameterised query.

**Verify:** one query per owner, and the records land in the shared collection like every
other arrival path.

---

## Step 11 — arc F over SQL: the gate

**Do:** the persons/companies graph traversal from arc F, against a database rather than a
file.

**Verify:** the same counts arc F proved over a file — one hop, one person and one company; a
second person at the same company leaves the company count at 1 and `c1 == c2`. Until this
passes, arc B is a hypothesis in exactly the way the README says arc F exists to settle.

---

## What could still make this sequence wrong

Named now so a later step can be recognised as a symptom rather than a surprise.

- **Step 0 fails.** The C call cannot be driven from a store accessor, and the source cannot
  be Rust-side. Everything from step 4 changes shape; steps 1–3 survive.
- **The row→record path needs a type it cannot get.** The descriptor gives layout, not SQL
  types. If a column's type cannot be mapped without asking the database, step 7's check
  moves earlier and becomes a prerequisite rather than a guard.
- **The transaction pin (arc D's database half) turns out to be per-connection.** D pins a
  file by identity; a database pins a transaction. If the C surface cannot hold one open
  across faults, consistency degrades to detection, and that is a design change to arc D
  rather than a step here.

## What the steps answered

Written after building them, because the answers are what the next reader needs.

| step | answer |
|---|---|
| **0** — the C surface | **Yes.** Core resolves sqlite through `c_call::resolve` and calls it from Rust with no rustc, no loft frame, no re-entrancy. Typed `extern "C"` pointers rather than the u64 trampoline ladder — core knows the signature at compile time, so it gets the ABI by construction the way `--native` does. Graduated into a test rather than deleted: the answer can regress, and it would surface as a fetch that mysteriously finds nothing. |
| **1** — the seam | `LazySource` + `Fetched`. No test moved, which was the claim. The three outcomes it returns (`Inserted` / `Absent` / `Unreachable`) are the distinction arc C already made. |
| **2** — the derivation | The descriptor carries enough. It also carried two things this plan did not know — below. |
| **3** — the mapping | `Mapping` holds table/column overrides plus the two dialect facts core must spell (quoting, placeholders). An empty mapping IS the derivation, so there is one builder rather than two paths. |
| **4** — the source | `SqlConn` connects read-only, runs the string, returns rows with `NULL` / `''` / value kept distinct. |
| **5+6** — row→record, wired | `record_new` + `hash::add` — the same pair `coll += [x]` uses, so a SQL arrival and an ordinary insert end in the same place. |
| **7** — the schema check | On the FIRST FETCH, not at bind: a bind takes a reference and a reference carries no type. Still before any answer a program could believe. Two probe queries once per binding — the count test asserts 5 rather than 9, which is what proves it does not repeat. |
| **8** — ranges | `store_lazy_range(coll, lo, hi)` — one query, many rows, and the count test asserts 1 rather than 5. It also turned out to make the ORDERED kinds work for free: materialising through `record_finish` rather than `hash::add` dispatches per kind, so a keyed lookup on a `sorted`/`index` collection stopped being refused. What it does NOT do is below. |
| **9** (B2) — the explicit query | `store_lazy_query(coll, condition)`. Only the WHERE comes from the caller; the table and columns are still derived, which is what makes a row arriving this way the same as one arriving by key. A row already resident is SKIPPED, so `LIKE` and a keyed lookup reach one record. |
| **11** — the gate | Passes over SQL on both backends with the counts arc F proved over a file. |

**A new stdlib builtin needs a second edit, and RUNNING it will not tell you.** `store_lazy_query`'s
`#rust` body serves `--native` only; the interpreter dispatches through `src/native.rs`'s
`FUNCTIONS` table. With the entry missing, the interpreter returned a plausible number — 8322 —
from an uninitialised slot: the count was wrong, the collection did not grow, and `--native` was
correct all along.

**The repo already guards it, and checking that was worth more than the anecdote.**
`tests/issues.rs::native_rs_functions_up_to_date` fails with *"src/native.rs is missing 1
function(s) from default/*.loft: n_store_lazy_query"* — verified by removing the entry again on
purpose. So the omission is caught by the suite and only slipped past because the binary was run
by hand first. The transferable part is the ordering, not a thing to remember: **run the gate
before believing an ad-hoc run**, because the gate names what the run only garbles.

### Three findings that changed the design

1. **The `Position` worked example was not valid SQL.** `from` and `to` are ordinary loft field
   names for a history row and reserved words in every engine. Nothing distinguishes a reserved
   word by shape, so the derivation quotes everything — no list to carry and none to keep current.
2. **Quoting has a price on SQLite, and it is the worst kind.** A double-quoted name that resolves
   to no identifier is accepted as a STRING LITERAL: `SELECT "naam" FROM "person"` returns the text
   `naam` once per row. A renamed column would have been materialised into the record. The
   connection turns it off (`SQLITE_DBCONFIG_DQS_DML`/`_DDL`); an SQLite older than 3.29 does not
   know the option, so **step 7 is a requirement rather than a guard.**
3. **An `index` element record carries its own red-black links** (`#left_1`, `#right_1`,
   `#color_1`) and `#color_1` is an ordinary boolean — so a column filter written on field TYPE
   selected a column no table has. `LayoutField::is_data` now has one home, shared with
   `read_via_descriptor` and the browser delivery.

### What the steps did NOT settle

The tails below live in [LAZY_STORES.md § Open work](../../LAZY_STORES.md), which is where a
reader of the built system will look for them. What belongs here is why each one was left rather
than finished, because that reasoning is about the sequence:

- **The IMPLICIT range** — step 8 as written wires the range shape "to the slice operation", so
  that `xs[lo..hi]` fetches. It ships as an EXPLICIT call instead, and that is now a **decision**
  rather than a tail ([@C104](../../DESIGN_DECISIONS.md)): failure path 2 settled that a lazily-bound
  collection answers *"what have I got"*, so a slice that silently consults the source makes `len`
  and iteration mean two different things depending on how the collection was reached. The
  batching claim, which is what step 8 exists for, is fully delivered by the explicit form: one
  query, many rows, asserted by count.
- **A composite range** and **B4's DECLARED form** are both blocked on a SURFACE nobody has
  designed — a call shape that carries a pinned key prefix, and a way for a field to declare its
  own foreign key. Building a runtime for a declaration nobody has designed would settle the
  surface by accident, which is the opposite of how the rest of this arc was built. B4's SHAPE
  ships and needed no new surface: `store_lazy_query(firm.people, "company_id = {firm.id}")` IS
  the owner-parameterised query, per collection, so two firms' fields hold their own rows.

  Making that work surfaced a real defect next door, filed rather than patched over:
  `store_verify` on a struct-FIELD collection reads the hash root as the WRAPPER struct and
  reports a corruption that is not there ([loft#790](https://github.com/loft-lang/loft/issues/790)).
  It reproduces with ordinary inserts and no laziness anywhere, on both backends and on the
  installed build — and the control that proved that is what kept it from being read as this
  arc's bug.
- **B3's compile-time half** — the `T: DbKeyed` bound and the mapping's loft-source spelling — was
  a stated dependency on @PLN125's associated types for composite keys. Arc A has since shipped
  them, so the dependency is discharged and the work is unblocked. The mapping VALUE is built and
  feeds the one builder.

### What is deliberately refused, and why that is not a gap in the invariant

Each of these answers `store_lazy_error` rather than a wrong record:

- a `spatial` collection — Morton order over coordinates has no SQL shape that means the same
  thing, and a bounding-box scan would look like a lazy fetch while reading the table;
- a narrow integer field (`i32`, `u8`, `size(2)`) — four encodings and their null sentinels, plus a
  different setter again when nullable, so it waits for a cell that can be checked;
- a nested struct, a vector or a stored pointer as a field — another table's rows, not a column;
- a range asked of a `hash`, or of a composite key.

## See also

- [QUERIES.md](QUERIES.md) § arc B — who executes, and why the source stays Rust-side.
- [BINDING.md](BINDING.md) — what must be declared and what is checked when.
- [README.md](README.md) § Sub-arcs — the per-arc status this sequence implements.
