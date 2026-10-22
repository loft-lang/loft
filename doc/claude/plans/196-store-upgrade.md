<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 196 — Store upgrade: a changed definition converts its data, or says why it cannot

Tracker: [@PLN196](https://github.com/loft-lang/plans/issues/196).

## Status

In progress.  It continues the deferred E2 of
[@PLN97](97-layout-contract/README.md) (closed), which shipped what this plan stands on: the
layout identity and the `.dschema` sidecar (`src/schema_sidecar.rs`), the refusal on load
(`Stores::layout_verdict_ok`, `src/database/allocation.rs`), and `loft layout accept|check`
(`run_layout_command`, `src/main.rs`).

Before this plan every reader refused a store whose layout changed, even by one added field,
and nothing converted it; `loft layout check` refused nothing and printed "auto-defaulted on
load" for a change every loader in fact refuses.

**In progress (2026-10-20).**

- **Step 0 — measured.**  `LayoutDesc::from_dump` (`src/database/layout_parse.rs`) reads a
  dump back into the description it was rendered from, exactly or not at all.  Over the
  corpus (1763 programs parsed, 1353 with types) the dump of every program reads back into
  its live description, name-keyed (`every_corpus_layout_reads_back_from_its_dump`).  The
  facts the dump does NOT carry, counted per program: a field's nullability in 388, a
  reference's target in 34, an index group in 50, a hash element's linkedness in 17 — the
  version-2 sidecar's list, now measured.  Defects the probe found, each fixed in this arc:
  the descriptor rendered a `trie` differently from the dump; an enum line named a variant
  rather than its type, so two programs with same-named variants swapped shared one identity
  and `store_load` misread (loft#2096); a method on an enum was stored as one of its variants,
  so `"label" as E` parsed and adding a method refused every stored `E` (loft#2100); and a
  stored `reference<T>` kept the slot number of the program that wrote it, so a load into
  another slot read another store (loft#2099).
- **Step 5 — expression defaults.**  A field default may read `$` and build a value (a format
  string, a method, a vector, a struct, a call); it is lowered into a function of the record
  (`(F-Default-Field)`, formal/calls.md).  `loft layout upgrade` compiles the reading program
  (`host::Program::for_upgrade`) and runs each added field's default on every upgraded record
  through a synthesised `__fill_<T>__<f>`; a constant default is still written directly.
  Guards: `tests/store_upgrade.rs` (construction and upgrade, both backends; the two-step
  rename) and `loft-suite rules/calls/F-Default-Field`.
- **Version-2 sidecar.**  Adds `root=` and `slot=` headers and a facts section after the dump
  (`LayoutDesc::render_facts` / `apply_facts`); the dump and its hash are unchanged, a
  version-1 sidecar is still read, and an older build refuses a version-2 one rather than
  reading it.
- **Step 2 — the map and the check.**  `UpgradeMap` (`src/database/upgrade_map.rs`) is
  derived from two descriptions and nothing else; `loft layout check` prints it and exits 1 on
  a refusal (owner question 2, answered provisionally as yes).  The verdict table is
  `tests/store_upgrade.rs`.
- **Steps 1, 3, 7 — the converter and `loft layout upgrade`.**  `Stores::upgrade_store`
  (`src/database/upgrade.rs`) appends the old layout's types to the table for the length of
  the upgrade, checks that they re-render the old dump, and builds the new store through
  `record_new`/`record_finish`; `loft layout upgrade <file> <store>…` writes beside and swaps.
- **Step 8 — the callers.**  Every whole-image loader converts an older layout in memory
  (`store_load`, `store_load_untrusted`, `store_load_url`, `store_load_url_trusted`; the
  pinned one checks its hash first), `store_persist_bind` upgrades its file in place keeping
  one `<path>.old`, and the live build swap hands the world over as a store image that
  `swap_world` loads through the same path.  The swap's verdict is reached when the next
  instance is compiled (owner, 2026-10-21): the rebuild checks the build against the running
  world's layout (`LOFT_CHECK_WORLD`, `Stores::world_upgrade_plan`), so a refused change
  fails the build and the swap is never offered.  A computed default runs in the program: the
  converter lists the records that owe one (`Stores::owed_*`) and the parser wraps each of
  those calls in a synthesised `__upgrade_drain`.  The paged loaders (`store_load_key*`)
  still refuse a changed layout.  Guards: `tests/store_upgrade.rs` (every upgrade cell also
  read by `store_load` on the OLD file), `tests/store_persist_loft.rs`
  `*_converts_or_refuses_a_changed_layout_both_backends`, and
  `tests/engine_host_kernel.rs` `s5_native_swap_under_running_world` (a world grown by a
  computed field), `cell_a_rebuild_checks_the_running_world_before_it_compiles`.  The
  browser swap (S6) does the same in the page: `swap_check(files)` compiles B against the
  parked A's world before anything is disposed (a refused build leaves A serving),
  `swap_export` hands a world that owns its store over as image + sidecar, and B's
  `swap_world` reads it through the converter (`Stores::load_world_image`); guard
  `tests/engine_host_connector.rs` `s6_browser_swap_under_living_page` (a grown world with a
  computed field, a refused build naming the field).
- **Step 9 — skipped versions.**  The sidecar records the writing release (`version=`, from
  the entry program's `loft.toml`; a native build carries it as a constant).  The floor is
  the manifest's existing `data_compatible_with`, and `data_bridge` names the release a
  refusal sends the user to (`Release::admits`, `src/database/release.rs`); every loader
  asks it where a sidecar becomes a verdict (`verdict_for_sidecar_text`), and the converter
  asks it before anything is read.  A routine `upgrade(from: Release, old: O, item: T)` (an
  overload set, one per `T`) runs on each record of `T` converted from a store older than
  the program, after its carried fields and defaults: the parser gives each a wrapper
  `__upgrade_<T>`, registers the routines before every wrapped load, and the drain calls
  them; `loft layout upgrade` calls them through the host.  `old` is filled by name into a
  scratch store released after the drain.  `Release` is a stdlib struct ordered by number,
  compared against a text with `operator compare`.  Rules: `(L-Release)`,
  `(L-Upgrade-Routine)`, `(L-Upgrade-Old)` in formal/layout.md.  A live rebuild's check
  (`world_upgrade_plan`) takes the same decision from the release the running world
  records — the exact old version — so a build it admits is a swap that converts.  Guards:
  the `cell_a_*` step-9 cells in `tests/store_upgrade.rs`.  Two readings of the design that
  differ from its wording (owner, 2026-10-21: no preference, provided in-process
  replacements are right): (1) the routine runs for every store older than the program, not
  only those older than "the previous release" — the program has no record of which release
  that was, and the routine's own `from` branches decide; (2) a field of `old` the stored
  record lacks takes its default (what a constructor writes for an omitted field) rather than a refusal, because a refusal would refuse every store
  written after that field was dropped (a v3 store opened by v4 with a routine that reads
  `label` for `from < 2.0`).  A value an intermediate release computed and the next one drops — in one record or across
  records (`$.inner.x`) — cannot be seen in the store; `loft layout accept` records what each
  computed field reads and the release that started it (`--sources--` in the baseline), and
  `loft layout check` refuses the drop unless a routine for that struct or a floor at that
  release covers it (`(L-Upgrade-Source)`, the `layout_check_*` cells).  The baseline moved to `layout.lock` beside `loft.toml`: at `.loft/layout.lock` it sat in
  the cache directory every project ignores, so it could never be committed (the old path is
  still read).  A library release records it — `loft package` runs `accept` on the entry
  before the tarball (owner, 2026-10-21: written at release time), and an APPLICATION
  releases through the same command (owner, 2026-10-22): its manifest is the same `[package]`
  shape, so `loft package` records the baseline of the types its entry program reaches.  A
  stored type only a side program declares is not recorded.  Every compile of the project compares its types with that
  baseline and warns at a stored type a store of the baseline's release could not be upgraded
  to (`layout-upgrade-refused`, `Parser::check_layout_baseline`; owner, 2026-10-22: a warning,
  not an error), the one answer `loft layout check` prints (`layout_schema::baseline_report`,
  which asks `Stores::baseline_upgrade`, routines included); `loft package` refuses to
  release while one stands.  The baseline and the check cover the types whose values outlive
  the program (`Data::surviving`, owner 2026-10-22): what a store or `swap_world` call hands
  over — a reload happens only at the host's main loop, so the world is what survives it, never
  a frame — every `pub` type of the project, which a library's user may store, and every type
  one of them holds; a store call on an untyped `reference` makes every type count.

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
| field added | the new definition's default: a constant is written directly (`Stores::write_absent_value`); an expression is evaluated over the upgraded record, so it may read `$` (below) |
| field removed | not copied; its owned heap is not copied |
| number widened (narrow int to wider, single to double float) | converted; a null stays null (`narrow_is_null` / `write_narrow_value`, `@FR-L-Null`) |
| number narrowed, or a field changes kind (text to integer, struct to scalar) | **refused** — names the field and both types |
| enum variant added | nothing to do |
| enum variant removed | a value of that variant becomes null (GOALS.md § What survives an edit) |
| a removed field and an added one in the same type | **refused** — "looks like a rename": the two-step form below |
| rename or conversion, step 1: the new field's default reads the old field (`name: text = $.label`); both exist | the new field is filled from its default, evaluated over the record |
| rename or conversion, step 2: the old field is removed | a plain removal |
| keyed collection kept, key changed | rebuilt by insertion under the new key |
| index added over records already in the store (a group member, `@FR-Col-Group`) | built by insertion (`link_record_siblings`, `src/database/structures.rs`) |
| an insert that displaces a record (a duplicate under a new or changed key) | **refused** — the whole upgrade, with the count and the first key (`Col-Group-Dup` would release the record) |
| index removed | the collection is not copied; the records it shared stay with the remaining members |
| linked layout flips (an index over `T` added or removed) | each element written at the new stride; never a raw block copy |
| a reference inside the store | re-pointed through the old-to-new record map |
| a reference into another store | **refused** (as `type_is_compactable` refuses it for compaction) |

**An added field is filled by its default, and a default may be an expression.**  The
language already lets a default read the record (`name_length: integer = len($.name)`,
LOFT.md), and runs defaults in declaration order after every field the record already has.
An upgraded record is a record nobody wrote the new field of — the question a constructor asks
for an omitted field — so the upgrade asks the same default, with the same ordering: the
carried fields are written first, then each added field's default runs, reading `$` as it
stands.  No new syntax:

```loft
// v1 — what the stored data was written with
struct Item { id: integer, label: text, price: integer }

// v2 — the new fields are filled by expressions over the stored record
struct Item {
  id: integer,
  label: text,
  price: integer,
  name: text = $.label,                      // a rename
  cents: integer = $.price * 100,            // a unit change
  title: text = "{$.id}: {$.label}"          // a combination
}

// v3 — after the stores are upgraded, the old fields go
struct Item { id: integer, name: text, cents: integer, title: text }
```

The default is type-checked as any default is (`name: integer = $.label` is a compile error; a
nullable expression must be discharged), so an upgrade cannot write an ill-typed value.  A
default with an effect (`= now()`) runs once per upgraded record, as it runs once per
construction.  Between v2 and v3, old code that still builds `Item { label: "x" }` gets
`name = "x"` too.

**Why a rename still takes two upgrades.**  The expression is checked against the NEW
struct, so `$.label` compiles only while `label` is still declared.  A rename therefore cannot
happen in one upgrade: step 1 adds the new name, filled by its default, while both exist; step
2, later, removes the old name.  A drop and an add in one edit stay refused ("looks like a
rename"), and the cure the refusal names is the step-1 default.  No single upgrade both loses a
name and creates one, so a wrong guess about what a drop and an add mean cannot destroy data.

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
7. Fabricated values — added fields take only the declared default (a constant, or the
   expression the program declares); GOALS.md "leniency is the feature" admits a default or a
   null, never a value nobody declared.
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
| 5 | Expression defaults fill added fields: the converter calls the default the constructor calls, per upgraded record, after the carried fields; a rename is two upgrades, one-step refused. | A rename, a unit change and a combination cell (hand-computed per record, both backends); step 1 then step 2 ends with the values under the new name; the one-step pair is refused with a reason that names the step-1 default; a constant default still takes the direct write. |
| 6 | References re-pointed; cross-store refused. | A reference cell reads the same record after the upgrade; the cross-store cell refuses. |
| 7 | Every save writes the version-2 sidecar; a binary build prints its description; `loft layout upgrade <store>`: new file beside, atomic swap, old kept. | A refusal and an interrupted run both leave the old store byte-identical. |
| 8 | **Callers**: `store_load` converting an older layout in memory (open question 3), the whole-program swap carrying the world through the converter instead of the JSON snapshot ([`08-live-build-swap.md`](18-engine-host/08-live-build-swap.md) loses references, hashes and sorted collections there), and the wasm build. | The swap scenarios keep a world with a hash and a reference; the converter builds for `wasm32` and passes the matrix there. |

| 9 | **Skipped versions.** The sidecar records the writing release; a store from the previous release takes the one-step conversion, an older one the programmer's `upgrade(from, old, item)` routine after it; see § Skipped versions. | A v1 store opened by v3, where v2 moved a field and v3 dropped it, arrives with the moved value through the routine, both backends, in the program and in a separate upgrade module; without a routine the drop is refused, naming 1.x; a store below the minimum is refused naming the bridge release, byte for byte untouched. |

Steps 0 and 1 settle the design: if the two producers cannot agree, or the
description-driven reader cannot reproduce `copy_claims`, the plan changes before step 2.

## Skipped versions

A conversion pairs two layouts: the one recorded beside the store and the program's own.  That
is enough for a live swap, which always comes from the build just before, and for a store
saved by the previous release.  It is not enough for a user who skips releases.

```loft
// v1
struct Item { id: integer, label: text }
// v2 moves the value: count is computed from label
struct Item { id: integer, label: text, count: integer = len($.label) }
// v3 drops label, which v2 made safe to drop
struct Item { id: integer, count: integer }
```

A v1 store opened by v3 sees `label` removed and `count` added, with v3's default: the moved
value is lost, and nothing says so.  The one-step rename refusal catches this only when the
two fields share a type.  This is data safety, not a swap question, so it outranks the
browser swap.

**The design** (owner, 2026-10-21):

- **The version is the identity.**  The sidecar records the release that wrote the store
  (`loft.toml`'s `version`) beside its layout.  No history of old layouts is kept: the store
  carries its own layout, and the version says which steps it still needs.
- **The normal case is one step.**  A store from the previous release converts by the
  upgrade map with the new defaults, as steps 1–8 build it: the fast path, in the program.
- **Older stores go through ONE loft routine** the programmer writes, which takes the version
  the store came from and runs the steps that version still needs.  It is called after the
  fields carried by name are in place.  It reads only the old fields it declares, as a small
  struct the converter fills by name from the stored record, through the layout the file
  records — typed, so it compiles like any other code, natively too:

  ```loft
  struct Item { id: integer, count: integer }       // release 3.0

  struct ItemBefore2 { label: text }                // the old fields the routine reads
  fn upgrade(from: version, old: ItemBefore2, item: Item) {
    if from < "2.0" { item.count = len(old.label) } // 2.0 moved label into count
  }
  ```

  A field the routine declares that the stored record lacks refuses the load with its name.
- **Where it lives is the programmer's choice.**  In the program, it ships with every build;
  in a separate upgrade module, the normal build carries no code for a one-time conversion,
  and the module is used to bring old stores forward (`loft layout upgrade`).
- **A store older than the previous release with no routine for it** converts directly only
  when nothing is dropped; a conversion that would drop a field is refused, naming the
  version that wrote it.
- **The oldest accepted version** is the programmer's to set in `loft.toml`, never raised
  automatically; raising it is how branches of the routine are retired.  A store older than
  it is refused, and the refusal names the release that bridges the gap: raising the minimum
  records the last release that still read the older versions, so the message reads, for a
  store written by 1.4 into a program whose minimum is 3.0: *this store was written by 1.4;
  open it once with 2.3 (or any release from 1.4 to 2.3), then with this one*.  A refused
  store is left byte for byte as it was.

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

**In the server-backed IDE (`loft --serve`).**  A save never changes the running game (owner,
2026-10-22: code is reviewed before it runs, and a warning does not stop a program).  Save writes
the file, compiles it against the running game's world — a change the next build could not carry
the world across is a `layout-upgrade-refused` warning at the world's type — and starts the
rebuild in the background.  **⇄ Apply** (or Ctrl+Shift+Enter) makes it active: the ready build
takes the world over through the converter; a failed build applies nothing and says why, and the
old build keeps serving.  Held function edits land at the same Apply.  Guard: `tests/serve.rs`
`serve_apply_holds_saved_edits_and_carries_the_world` — a held edit that changes nothing until
Apply, a world grown by a field and an index crossing with its counter, keyed items and new index
intact, and a refused change warned on save and failing the rebuild while the game keeps serving.

## Open questions (owner)

1. ~~The rename source syntax~~ — answered (owner, 2026-10-20): no new syntax; an added
   field's default, which may be an expression over `$`, fills it on upgrade.
2. ~~Does `loft layout check` exit non-zero~~ — answered (owner, 2026-10-22): a refused
   upgrade is a compiler WARNING (`layout-upgrade-refused`) at the stored type, on every
   compile of the project, so it shows in the IDE while the program still runs; `loft layout
   check` exits 1 on it, a library's CI denies it, and `loft package` refuses to release over
   it.  Not an error: legality stays a matter of the source, and a refactor that drops the
   field before writing the routine still runs its tests.
3. ~~Upgrade on load~~ — answered (owner, 2026-10-21): `store_load` converts an older layout
   IN MEMORY.  The call answers true with every added field filled by its default, the file
   on disk keeps its old layout until the program writes it, and a change the upgrade map
   refuses still answers false with the reason.  `store_persist_bind` on an older file
   upgrades the FILE in place (owner, 2026-10-21): the converted store is written beside it
   and swapped in, the original is kept as `<path>.old`, and only then is the store bound —
   after every added field holds its default, so no file on disk ever lacks one.  Upgrading
   the same file again replaces that `.old`: one upgrade back stays on disk.
4. ~~Which structures count as long-lived~~ — answered (owner, 2026-10-21): saved stores, the
   swap world, and every global store of a running program.  A hot reload of a changed struct
   converts the live data in place with the same converter; a change the map refuses keeps
   the old program running and says why.

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
