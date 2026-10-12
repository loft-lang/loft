<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — scope-bound transactions, and procedures written as text

Two asks, designed before code because the failure paths are where the
invariants live:

1. a **drop at scope end** that ends a transaction or cancels a running
   statement, so the cleanup reads as part of the declaration;
2. **string formatting to define a stored procedure** inside the library.

They look unrelated and share one property: both put a *side effect* somewhere
the reader is not looking — one at a brace, one inside a string. That is the risk
in each, and it is what these designs have to make safe.

## Part 1 — a drop at scope end

### The observation that makes this cheap

loft has no destructors, and adding one sounds like a large feature. It is
smaller than it looks, because **loft already computes the fact a drop needs.**

The ownership model (`deps`, `scopes.rs`) decides, per binding, whether *this*
scope owns a value and whether it dies here — that is what emits `OpFreeRef`,
`OpFreeText` and `OpFreeScratch` today. A returned value is not freed; a borrowed
one is not freed; a value escaping through a field is not freed. Those are the
same questions a drop must answer, already answered, already tested.

So the invariant is not new analysis. It is a call at an existing point:

> **A drop runs exactly where the value's own `OpFree*` runs — the same binding,
> the same scope exit, the same early-exit paths — and never anywhere else.**

That phrasing is deliberate: it makes the feature *derive* from the borrow model
rather than sit beside it, so there is one answer to "when does this run", not
two that can disagree.

It also inherits behaviour that is easy to get wrong. loft#731 exists because a
`return` out of a loop bypasses a loop epilogue, so `scopes.rs` emits the
iteration-scratch free at the FUNCTION's scope exit as well. A drop riding that
machinery gets early `return`, `break` and the loop-epilogue case for free — a
hand-rolled `defer` would have to rediscover every one.

### What it costs, honestly

**A drop cannot fail.** loft's rule is that there are no runtime errors ever
(@C80). A rollback at scope end can fail for real — the connection dropped, the
server went away — and there is no caller left to tell. So a drop is
*best-effort and silent*, and that is a genuine semantic weakening: a transaction
you believed was rolled back may not have been, and nothing raised. The cure is
not to make drop fallible; it is that **anything whose failure matters must be
explicit**:

```loft
tx.commit()      // may fail; you get the answer
// scope end     // best-effort rollback if not committed; you get nothing
```

Commit is a call. Rollback-on-abandon is the drop. That asymmetry is the design,
not an oversight.

**Order within a scope** must be reverse-declaration, matching the existing free
order, or a statement would outlive the transaction it belongs to.

**A drop that touches C is a side effect at a compiler-chosen point.** Today
`#c` calls happen where the program writes them. A drop means libpq's
`PQexec("ROLLBACK")` runs at a closing brace. That is exactly the readability the
ask is after, and exactly what makes it dangerous if the point is ever unclear —
which is why the invariant is "where the free runs", a point the compiler already
prints in the IR.

### The alternative that needs no language change

@PLN23 already specifies:

```loft
conn.transaction(fn(tx) {
  tx.exec("…");
})            // commits on normal completion, rolls back on early exit
```

This gives deterministic, *visible* scoping with no new language feature and no
silent failure — the closure's end is a place the reader can see, and the
wrapper can RETURN a status because it is an ordinary call. Its weakness is
exactly the ask: nesting reads worse, and a handle cannot outlive the closure
even when that would be natural.

**Recommendation: build the closure form first** (it is in the plan, it works
today, and it is the thing a consumer needs), and treat the drop hook as a
language proposal justified by more than transactions — a hook that exists only
for `sql` is a hook with one user. If it lands, it should be `#drop fn` on a
struct, wired to the existing free site, and its first proof should be a
transaction *and* something unrelated, or the invariant is untested.

## Part 1b — transactions: the contract the mapping depends on

Atomicity is not a feature of this design, it is a **precondition of the object
mapping**. A whole-collection write is "replace the child rows for one owner" —
several statements. Without one transaction around them, a crash leaves a
collection half written and the read path cannot tell: it sees rows, they are
wrong, and nothing says so. So:

> **One object-graph write is one transaction.** Not "should be" — the mapping is
> unsound otherwise, and no reader can detect the damage afterwards.

### On the interface

Three methods, because all three backends spell them the same way (`BEGIN` /
`COMMIT` / `ROLLBACK` are ordinary statements on sqlite, MariaDB and
PostgreSQL — this is the rare place where uniform is free):

```loft
fn db_begin(self: Self) -> boolean
fn db_commit(self: Self) -> boolean      // may FAIL, and you get the answer
fn db_rollback(self: Self) -> boolean
```

`db_commit` returns a status because a commit can fail for real — a constraint,
a deadlock, a lost connection — and that is precisely the moment a caller must
find out. This is the asymmetry Part 1 rests on: **commit is a call that answers;
rollback-on-abandon is the drop that does not.**

### What must be refused rather than emulated

- **Nesting.** None of the three nest `BEGIN`. `SAVEPOINT` is the real mechanism
  and is spelled differently enough to be a separate design; until then a
  `db_begin` inside a transaction is an error, not a silent no-op. A silent
  no-op is the dangerous version, because the inner "rollback" would then discard
  the OUTER transaction's work.
- **DDL inside a transaction.** MariaDB commits implicitly on DDL; PostgreSQL and
  sqlite do not. So a migration that mixes DDL and data in one transaction is
  atomic on two backends and not on the third — refused, not papered over.

### Interaction with the procedure emulation

A server-side procedure is one round trip and therefore implicitly atomic for the
statements inside it. The sqlite emulation (Part 2) is several round trips and is
**not**. So a procedure that relies on internal atomicity must be called inside an
explicit transaction, and the emulation is what makes that a rule rather than an
accident of which backend you are on.

## Part 2 — a procedure defined by string formatting

### The distinction the whole design rests on

@PLN23 forbids a string-concat query API: prepared statements only, so SQL
injection is prevented *by construction* rather than by discipline. A procedure
body built from a format string looks like exactly the thing that rule forbids.

It is not, and the difference is not "trust the author" — it is structural:

| | who writes it | where the parameters go |
|---|---|---|
| a **query** | the program, per call, possibly from user input | values, at run time |
| a **procedure body** | the library, once, at deploy time | placeholders the body declares |

A procedure body is *code the library ships*, in the same sense its loft source
is. The values a caller later passes go through the procedure's own parameters —
i.e. through the prepared-statement path — never through the text.

So the rule:

> **Formatting may interpolate SCHEMA — identifiers the library controls — and
> never VALUES. A value reaches SQL only as a placeholder bound at call time.**

### Making that structural rather than advisory

`"{x}"` interpolates anything, so the type system cannot tell a table name from a
user's surname. Left there, the rule is a comment and the first mistake is a
vulnerability.

**The real answer is [INTERPOLATION_HOOK.md](INTERPOLATION_HOOK.md)** — make the
literal/hole boundary survive into a library type, so a value has no path into
SQL syntax at all. Two mechanisms were tried first and both were MEASURED to
fail: a type cannot carry it (formatting renders everything to text) and `const`
cannot either (a const string interpolates runtime data quite happily). The two
gates below are what a library can do *without* the hook, and they remain useful
with it:

1. **A distinct type for what may be interpolated.** `SqlIdent` is constructed by
   a checked function (`ident("orders")` — refuses anything that is not a bare
   identifier) and is the only type the procedure builder accepts in its
   interpolations. Passing a `text` is then a compile error, not a review
   finding.
2. **The body is a declaration, not an expression.** A procedure is registered
   once, at a defined moment (`deploy`), against a connection — never assembled
   inside request handling. A builder that can only run at deploy time cannot see
   request data.

```loft
proc = sql_procedure(
  name: ident("archive_orders"),
  params: [param("cutoff", SqlType.Date)],
  body: "DELETE FROM {tbl} WHERE created < :cutoff",   // {tbl}: SqlIdent
  tbl: ident("orders"),
);
db.deploy(proc);              // once, at deploy time
db.call("archive_orders", [today()]);   // values, bound — never formatted
```

### Failure paths

- **Dialect.** `CREATE PROCEDURE` is not portable: MariaDB, PostgreSQL
  (`CREATE FUNCTION` / `DO`) and sqlite (which has no stored procedures at all)
  disagree on syntax and on whether the feature exists. So `deploy` belongs on the
  BACKEND, not on the uniform interface — sqlite must be able to answer "not
  supported" rather than pretend. This is the same shape as `#c`'s wasm column:
  a defined answer in every column, not the same answer.
- **Idempotence.** Deploying twice must not fail or silently keep an old body.
  `CREATE OR REPLACE` where it exists; drop-then-create where it does not; and
  the body's hash recorded so an unchanged procedure is not rewritten.
- **Migration.** A procedure is schema, so it has the same versioning problem as
  a table, and the object mapping's rules apply to it.
- **`text` bound as a key or identifier** needs a length bound (already open in
  OBJECT_MAPPING.md).

### Re-assertion sites

The identifier/value split would have to be restated by: the procedure builder,
the query builder, the migration writer, and each backend's `deploy`. Four sites,
and omitting it is **silent** — the SQL still runs. So it collapses to one:
`SqlIdent` has a single constructor that validates, and no other path can produce
one. The type is the chokepoint; the rule is not repeated anywhere.

## Status

Design only — nothing here is built. It sits after the S1–S7 ladder in
OBJECT_MAPPING.md, because both parts need a working `sql` interface underneath
and the transaction wrapper is S-level work that the closure form already covers.

Part 1 is a **language** feature; it SHIPPED as **@PLN125 arc B** — spelled
`fn OpDrop(self: T)` rather than the `#drop fn` sketched above, and documented in
[INTERFACES.md § Running at scope end](../../INTERFACES.md). It was tracked as B1–B5,
alongside associated types (arc A) and indexing (arc C) — the remaining gaps
between a library type and a built-in one. B5 requires a **second, unrelated
consumer** before it lands: a transaction alone would leave the invariant tested
by exactly one shape. Parts 1b and 2 stay here, with @PLN23.
