<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 196 — Store upgrade: a changed definition converts its data, or says why it cannot

Tracker: [@PLN196](https://github.com/loft-lang/plans/issues/196).

## Status

Future — design, no implementation.  It continues the deferred E2 of
[@PLN97](97-layout-contract/README.md) (closed), which shipped what this plan stands on: the
layout identity and the `.dschema` sidecar (`src/schema_sidecar.rs`), the refusal on load
(`Stores::layout_verdict_ok`, `src/database/allocation.rs`), and `loft layout accept|check`
(`run_layout_command`, `src/main.rs`).

Today every reader refuses a store whose layout changed, even by one added field, and nothing
converts it.  `loft layout check` compares type NAMES against `.loft/layout.lock`, refuses
nothing (exit 0 on every change), writes an outline of `// TODO backfill` comments, and prints
"auto-defaulted on load" for a change every loader in fact refuses.

## Goal

The programmer changes a script's definitions; a tool validates the change against the older
definitions and either upgrades the data **store to store**, with no text in between, or names
the reason it cannot.  One routine serves stores on disk, stores read from a URL, long-lived
structures in memory (hot reload, a whole-program swap) and WebAssembly.

- **Effort:** H  ·  **Design:** ~ (steps 0–1 are the probes that settle it)

## The invariant

> **An upgrade reads old bytes only through the OLD layout, writes only through the NEW one,
> and every value either arrives under its own name, takes the new definition's default, or
> the whole upgrade is refused before the old data is touched.**

This is the "migrate" half of [`(L-Sound)`](../formal/layout.md) ("reject-or-migrate, never
read the bytes raw"), whose "reject" half @PLN97 shipped.

**One home: the upgrade map.**  For each (old type, new type) pair it records, per new field,
where the value comes from (an old field, a widening, the default) and, per old field, whether
it is dropped; per keyed collection, how it is rebuilt; and a list of refusals, each with its
reason.  It is derived once, from the old layout description and the new type table.  The
validation tool prints it, the converter executes it, and every later caller (load path, live
swap, wasm) runs the same converter.  Re-assertion sites: one.  A caller cannot re-decide a
field's fate.

## Where the old layout comes from

The new layout is always the running program's type table.  The old one has four sources, and
all of them fill **one name-keyed description** (type numbers differ between builds; names do
not):

| source | used for | today |
|---|---|---|
| the live type table (`Stores.types`) | structures in memory, hot reload, a whole-program swap, wasm | complete |
| the `.dschema` sidecar beside a store (file or URL) | stores saved by an older program | most of it; see below |
| the older source tree, compiled without running | old data with no schema saved beside it | complete: the same type table, from the parser |
| a binary build (interpreter bundle or `--native` program) | old data whose source is not at hand | not yet: the build carries its type table and prints its description on request |

**What today's sidecar already records** (`Stores::layout_dump` / `render_layout_parts`,
`src/database/types.rs`): every type by name and size; each struct field's name, position and
type; enum variants in order; narrow integers' start and null value; `vector<T>` against
`array<T>` (the linked layout); each keyed collection's kind, element type and key fields (by
field number, which the element type's line resolves to a name); the host endianness; and
full-width nullability on the `schema=` line when written with `Data`.

**What a version-2 sidecar must add:**

1. whether a type is linked when the store holds only keyed collections over it (the
   `vector`/`array` tell is absent then, and a hash entry's shape depends on it);
2. index group membership (`other_indexes`, `src/database/mod.rs`): which collections share
   their records (`@FR-Col-Group`);
3. the target type of a stored reference (`dbref12` names none);
4. the `schema=` line always, so nullability is never guessed.

Defaults are not needed: an added field takes the NEW definition's default.  A version-1
sidecar still serves a store without the cases above; for one with them, the tool refuses and
says to rewrite the sidecar once with the old program.

**The old layout travels with the data.**  A running program publishes the description of
every long-lived structure it holds: each store it saves (the version-2 sidecar), the world it
hands over in a whole-program swap, the state it keeps across a hot reload.  So an upgrade
never needs the older source tree: whoever holds old data also holds the description it was
written with.  Only a store saved before version 2, with one of the missing facts, needs the
old source tree or the old binary once.

**One description, four producers, one guard.**  [`LayoutDesc`](../../../src/database/descriptor.rs)
is the structured twin of the dump, built from live `Stores` (its keyed collections are opaque
`Iterated` nodes today, so it grows).  The guard: for every corpus program, the description
built from the live table equals the one parsed back from the sidecar it writes, the one the
source compiles to, and the one its `--native` build prints.  That is the render/parse round
trip `tests/layout_descriptor.rs` already runs for the text, extended to the parsed form and
to every producer.

## Upgrade rules

| change | upgrade |
|---|---|
| type unchanged (equal dump line, closure included) | raw block copy (the fast path) |
| field added | the new definition's default (`Stores::write_absent_value`) |
| field removed | not copied; its owned heap is not copied |
| number widened (narrow int to wider, single to double float) | converted; a null stays null (`narrow_is_null` / `write_narrow_value`, `@FR-L-Null`) |
| number narrowed, or a field changes kind (text to integer, struct to scalar) | **refused** — names the field and both types |
| enum variant added | nothing to do |
| enum variant removed | a value of that variant becomes null (GOALS.md § What survives an edit) |
| a removed field and an added one in the same type | **refused** — "looks like a rename": the two-step form below |
| rename, step 1: the new field declares the old one as its source; both exist | the new field is filled from the old |
| rename, step 2: the old field is removed | a plain removal |
| keyed collection kept, key changed | rebuilt by insertion under the new key |
| index added over records already in the store (a group member, `@FR-Col-Group`) | built by insertion (`link_record_siblings`, `src/database/structures.rs`) |
| an insert that displaces a record (a duplicate under a new or changed key) | **refused** — the whole upgrade, with the count and the first key (`Col-Group-Dup` would release the record) |
| index removed | the collection is not copied; the records it shared stay with the remaining members |
| linked layout flips (an index over `T` added or removed) | each element written at the new stride; never a raw block copy |
| a reference inside the store | re-pointed through the old-to-new record map |
| a reference into another store | **refused** (as `type_is_compactable` refuses it for compaction) |

**Why a rename takes two upgrades.**  A field may take its value only from a field that exists
in **both** the old and the new definition.  So a rename cannot happen in one upgrade: step 1
adds the new name, filled from the old, while both exist; step 2, later, removes the old name.
No single upgrade both loses a name and creates one, and a wrong guess about what a drop and an
add mean cannot destroy data.

## Failure paths

1. Old bytes read under the new layout — the converter reads old records only through the
   description; a raw copy only where the per-type dump lines are equal.
2. Silent loss: a duplicate key, a narrowing, a one-step rename — refused before anything is
   written.
3. A crash mid-upgrade — the new store is written beside the old and swapped in by rename; the
   old one is kept.
4. The wrong old definition — a version-2 sidecar's hash must match the store's; a mismatch is
   refused before any read.
5. Dangling references after records are renumbered — a fix-up pass through the record map.
6. Orphan records in the new store — the converted store passes the store leak check.
7. Fabricated values — added fields take only the declared default; GOALS.md "leniency is the
   feature" admits a default or a null, never an invented value.
8. A refusal at run time — none: the tool refuses, and `store_load` keeps answering false with
   the reason (C80).

## Composition matrix — Stage A

Axes, each cell a pair of scripts plus a store, values hand-computed and checked for value,
length and leaks:

- **field kind:** narrow and wide integer, float, text, enum, nested struct, inline vector,
  linked array, hash, sorted, index, spatial, trie, in-store reference, cross-store reference;
- **edit:** unchanged, add, remove, widen, narrow, kind change, rename in one step, rename in
  two steps, key change, index added, index removed, variant added, variant removed;
- **position:** top-level field, nested struct field, field of a vector element;
- **contents:** empty collection, one element, many (an empty container hides append against
  replace);
- **linked:** `T` with and without a keyed collection elsewhere in the program;
- **source of the old layout:** live table, sidecar version 2, sidecar version 1.

The new program reads the converted store on `--interpret` and on `--native`.

**The linked flip, both directions.**  A keyed collection over `T` anywhere in the program,
even in a struct the store never holds, makes `T` linked: every `vector<T>` becomes
`array<T>`, its elements moved out of the vector into records of their own, and the vector
holds 4-byte record ids instead of inline elements.  Today both directions are refused on
load (the dump line changes `vector<T>` to `array<T>`, measured), so these are upgrade cells,
not a live misread:

- **index gained** (`vector<T>` to `array<T>`): one record claimed per element, the element
  converted into it, the id list written in the old order;
- **index lost** (`array<T>` to `vector<T>`): each record's payload folded back inline at
  `T`'s stride, order kept;
- **not every lost index unlinks `T`**: a stored `reference<T>` keeps `T` linked
  (`@FR-H-LongRef`), so the map reads linkedness from the new type table, never from "an
  index was removed";
- **a reference to such an element** names an inline position on one side and a record on
  the other, so it goes through the old-to-new record map (step 6);
- **the cause is elsewhere**: the struct that holds the vector did not change, so the map's
  report names the collection that flipped it (`T is now stored by reference because
  X.by_k indexes it`).

Crossed with empty, one and many elements, and with an element that owns heap (text, a
nested vector).  `hash<T>` alone is the exception: its entries change shape with `T`'s
linkedness, but today's reader decodes either shape and writes after the load come out right
in both directions (measured), so a hash-only flip needs no conversion of its own.

## Steps

| # | step | verify (what goes red if it is wrong) |
|---|---|---|
| 0 | **Probe.** Build the name-keyed description from the live table, from a sidecar, and from the source compiled without running, for every corpus program. | Two producers differ for some program, or the round trip loses a fact the rules table needs. |
| 1 | **Identity conversion** through the description-driven reader, old = new. | Its output differs from `copy_claims` (`src/database/allocation.rs`) on any corpus store: a digest per store. Also its speed against `compact_slot`, the bar (about 0.6 µs per record). |
| 2 | **Field-level diff and refusal classifier** in `loft layout check`; the false "auto-defaulted" note goes. | A table of script pairs, each with its expected verdict and reason text. No data involved. |
| 3 | Fields added, removed, widened; enum variants. | The matrix cells for those edits, both backends. |
| 4 | Keyed collections: key change, index added and removed, the linked flip; duplicate refusal. | The keyed cells: counts, order, lookups; the duplicate cell must refuse with the old store untouched. |
| 5 | Two-step rename; one-step refused. | Step 1 then step 2 ends with the values under the new name; the one-step pair is refused with the rename reason. |
| 6 | References re-pointed; cross-store refused. | A reference cell reads the same record after the upgrade; the cross-store cell refuses. |
| 7 | Every save writes the version-2 sidecar; a binary build prints its description; `loft layout upgrade <store>`: new file beside, atomic swap, old kept. | A refusal and an interrupted run both leave the old store byte-identical. |
| 8 | **Callers**: `store_load` offering the upgrade, the whole-program swap carrying the world through the converter instead of the JSON snapshot ([`08-live-build-swap.md`](18-engine-host/08-live-build-swap.md) loses references, hashes and sorted collections there), and the wasm build. | The swap scenarios keep a world with a hash and a reference; the converter builds for `wasm32` and passes the matrix there. |

Steps 0 and 1 settle the design: if the two producers cannot agree, or the
description-driven reader cannot reproduce `copy_claims`, the plan changes before step 2.

## Acceptance — the setup the claim is made in

The plan is done when this works end to end, because it is where the owner will claim it: an
IDE in the browser, linked to a running game or simulation, with a local optimisation service
building in the background.  The programmer adds a field and an index to a struct the running
world holds; the service rebuilds; the world crosses into the new build through the converter
with its references, hashes and sorted collections intact (today's JSON snapshot restores them
to defaults); and a change the map refuses shows its reason in the IDE while the game keeps
running the old build.  It rests on the background rebuild and the browser swap of
[08-live-build-swap.md](18-engine-host/08-live-build-swap.md) (S4, S6), and on an IDE in the
browser: the server-backed one ([@PLN16](https://github.com/loft-lang/plans/issues/16) M5e)
first, and the serverless one ([@PLN62](https://github.com/loft-lang/plans/issues/62)) once it
runs a full game or a long simulation, which is its goal.  The converter builds for `wasm32`
(step 8), so it serves both.

## Open questions (owner)

1. **The rename source syntax** for step 1 (for example a field annotation naming its source
   field).  It is valid only while the source field still exists, which is what enforces the
   two steps.
2. **Does `loft layout check` exit non-zero** on a refused upgrade, so a project's CI gates on
   it?
3. **Upgrade on load:** offered automatically by `store_load`, or only through
   `loft layout upgrade`?
4. **Which structures count as long-lived** and publish a description: saved stores and the
   swap world for certain; also every global store of a running program, so a hot reload of a
   changed struct can upgrade it in place?

## Cross-arc dependencies

- [@PLN97](97-layout-contract/README.md): the identity, the sidecar, the CLI; this plan is its
  deferred E2.
- [18-engine-host 08](18-engine-host/08-live-build-swap.md): the whole-program swap names
  "converting one store format to another" as its missing after-part; step 8 supplies it.
- [@PLN43](https://github.com/loft-lang/plans/issues/43) durable stores (parked): a write-ahead
  log would need the same upgrade for its records.

## See also

- [formal/layout.md](../formal/layout.md) `(L-Sound)`;
  [formal/collections.md](../formal/collections.md) `(Col-Group)`, `(Col-Group-Dup)`.
- [DATABASE.md § Working-set store loader](../DATABASE.md) — the refusal this plan turns into an
  upgrade.
- [GOALS.md § Why a language, not a store bolted onto an existing one](../GOALS.md) — the table
  of what survives an edit.
- [23-db-clients/OBJECT_MAPPING.md](23-db-clients/OBJECT_MAPPING.md) § S7c (@PLN23): the SQL
  side's "shape, never content" migration, the nearest existing design.
