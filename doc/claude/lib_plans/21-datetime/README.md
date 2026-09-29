<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — `time` library + `DateTime` value type

## Status — DONE / SHIPPED 2026-07-08

**`time 0.1.0`** (epoch-ms `integer` API, 2026-05-25) **and `time 0.2.0`** (the first-grade
`value struct DateTime` / `Duration` layer) are both shipped; **0.2.0 is published + signed to the
registry**, with all 24 lib tests green on `--interpret` AND `--native`, and the DateTime surface
verified identical on **wasm** (wasm32-wasip2 / wasmtime) — now gated going forward by the
differential oracle's third backend (`tests/oracle/29`). User-facing API:
[LOFT_DATA.md § Value structs / First-grade custom types](../../LOFT_DATA.md).

The prerequisites the pivoted design needed all shipped this cycle: the per-type
`to_text(self, spec)` format hook + direct operator dispatch + `as` conversions
([@PLN99](../../plans/99-first-grade-types/README.md)) and zero-cost value structs
([@PLN101](../../plans/101-value-structs/README.md)) — so `DateTime` is a distinct nominal type
(`dt + 5` rejects), with chronological operators, custom `{dt:date}`/`{dt:iso}` formatting, `as`
conversions, and **zero heap overhead** inside records / vectors, all with NO built-in
`Type::DateTime`.

**Follow-ups (tracked bugs / cosmetic — NOT blockers):** the lib's `to_text` still carries an
interpolate-each-arm workaround for the open **#534 residual** (a `match`-`&str` fn mixed with a
`String` arm in a 3-arm hook — owned by the ../loft2 stream); the now-unneeded #533 tail-`if`
workaround can be dropped in a 0.2.1 cleanup once someone re-publishes.

---

### Historical (the 2026-06-14 pivot that this plan delivered)

The full-datetime tail no longer builds a distinct built-in `Type::DateTime` — a **`time`-library
struct** gets every property the built-in was for. It shipped and unblocked the trainer app,
needing zero core changes and working identically on interpret / `--native` / wasm.

The **full-datetime tail no longer builds a distinct built-in `Type::DateTime`**
(old arcs A/B/C).  An evaluation against the present code (2026-06-14) showed a
**`time`-library struct** `DateTime { ms: integer }` gets every property the
built-in was for — distinct-type safety, operators, civil math — for free, and
the one gap (custom `{dt:…}` formatting) is better filled by **one general core
feature**: a per-type `to_text(self, spec)` format hook that *every* library
reuses.  This collapses ~25 files of built-in into one small reusable feature +
a pure-library `time` release.  Full rationale + design:
**[DESIGN.md](DESIGN.md)**.  Deferred to the 2026-08 "better PHP / more capable
libraries" cycle; no code lands for 2026-07.

Driver: the `training` app (`../personal/training`,
see its `MIGRATION.md § Loft capability gaps`) is date-indexed
(daily wellness, CTL/ATL EWMA over day sequences, weekly rollups,
HRV-by-day) and its B8–B10 routines are blocked on dates alone.
`BROADENING.md`'s Data/ETL gap list names the same hole.

Decisions (updated 2026-06-14 by the [DESIGN.md](DESIGN.md) pivot):

1. **`DateTime` is a `time`-library struct `DateTime { ms: integer }`**, *not*
   a distinct built-in type.  It is a distinct nominal type (`Type::Reference`),
   so `dt + 5` is a compile error by construction and `dt1 < dt2` works via
   library operator defs — the chronological type safety the built-in was for,
   with zero core type-system changes.  *(Was: a distinct built-in `DateTime`
   value type over `i64`.)*
2. **UTC formatting + a fixed-offset helper** for local-day bucketing.
   No IANA timezone database, no DST handling (documented limitation).
3. **REVERSED — formatting gets a per-type hook in core; everything else is
   library code.**  The old decision said "no per-type format hook exists, so
   `{dt:date}` rendering must be in core."  The pivot *adds* that hook: a general
   `to_text(self, spec)` dispatch from `append_data` for any user struct (the
   natural loft analog of `Display`/`__format__`).  It is the one core change,
   and it is reusable by every library — not a DateTime one-off.
4. **JS-`Date`-aligned epoch + proleptic-Gregorian civil math** (the unit/epoch
   match JS `Date`).  The civil math is **pure-loft library code** (already
   shipped in `time 0.1.0`) — *not* a two-backend core contract with
   `js_sys::Date` delegation.  Dropping that delegation removes a browser-only
   path wasip2 can't use; the ~30-line Rust-free civil math compiles into every
   backend identically.

## Goal

Ship a pure-loft `lib/time` library (operations + `format_*` renderers)
covering every date operation the `training` app needs (table below)
with no workarounds — then, as the plan tail, promote it to a distinct
built-in `DateTime` type with built-in `{dt:…}` formatting.

## Effort + design

- **Effort (post-pivot):** S–M — **one** general core feature (the per-type
  `to_text(self, spec)` format hook) + a pure-library `time` release (struct +
  operators + `to_text` + signature changes).  *(Was H: the old built-in
  `Type::DateTime` rippled through ~25 files — that path is dropped.)*
- **Design:** ✓ resolved — see [DESIGN.md](DESIGN.md) (struct + format hook).
- **Last touched:** 2026-06-14 (pivot to struct + general format hook)

## Design principles

- **`DateTime` = a struct over `i64` epoch-ms, JS-`Date`-aligned.**  Same
  unit/epoch as `now()` and JS `Date`.  Cross-backend parity is *structural*:
  the same epoch-ms renders identically on interpret / `--native` / wasm
  because the civil-calendar mapping is one body of **pure-loft** code (already
  shipped in `time 0.1.0`) that compiles into every backend.
- **No `js_sys` delegation (post-pivot).**  The earlier "thin WASM" plan routed
  wasm through `js_sys::Date` UTC getters; dropped — the ~30-line proleptic-
  Gregorian civil math is tiny, compiles into every target, and gives results
  identical to JS `Date`, so a browser-only path wasip2 can't use buys nothing.
- **Small, bounded surface.**  This is a "basic" type (user's word):
  no `Duration` type, no calendar arithmetic beyond day/week/second
  steps, no locales.  `DateTime − DateTime` yields plain milliseconds;
  the library wraps that as `days_between` etc.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **D** — `lib/time` operations + `format_*` renderers (pure loft, over `integer` epoch-ms) | `lib/time/` (new package) | **BASICS — SHIPPED 2026-05-25.**  Validated on interpreter + `--native` (run, all asserts pass) and wasm32-wasip2 (compiles clean).  Test: `tests/docs/32-time.loft` (CI three-backend) + `lib/time/tests/01-basics.loft` (package smoke). |
| **E** — port the training app's date-dependent routines onto `lib/time` | `../personal/training/loft/` | BASICS — next |
| **A** — distinct `DateTime` variant in the `Type` enum | — | **SUPERSEDED** by [DESIGN.md](DESIGN.md): `DateTime` is a library struct, no core type |
| **B** — native + wasm conversion backends (`js_sys::Date`) | — | **SUPERSEDED**: civil math stays pure-loft library code (shipped); no `js_sys` delegation |
| **C** — built-in `{dt:…}` format specifiers | — | **SUPERSEDED** by the general `to_text(self, spec)` hook (one core feature, all libraries) |
| **F** — general per-type format hook (`to_text(self, spec)` from `append_data`) | `src/parser/collections.rs` | **DEFERRED (post-release)** — the one core change; replaces arcs B+C |

## Phase ordering

**Basics (now — unblocks the trainer app, zero core changes):**

1. **D** — write `lib/time` as pure loft over `integer` epoch-ms:
   civil-calendar conversion (Hinnant `days_from_civil` /
   `civil_from_days`) + the operations and `format_*(…) -> text`
   renderers in the coverage table.  Tests against hand-computed
   goldens; runs on interpret / `--native` / wasm unchanged.
2. **E** — port training B8–B10 (HRV attribution, `hrv_predict`,
   goals/phase/schedule) onto `lib/time`; verify against the Python
   parity oracle.

**Full datetime support (PIVOTED 2026-06-14 — see [DESIGN.md](DESIGN.md); deferred
past the 2026-07 release):**

The old arcs B→C→A (two-backend conversion core, `{dt:…}` core opcodes, a built-in
`Type::DateTime`) are **dropped**.  The full tail is now:

3. **F (core)** — one general per-type format hook: `append_data` dispatches
   `{x}` / `{x:spec}` on any user struct to its `to_text(self, spec: text) ->
   text` method.  Two-part: lift the bounded-generic-only gate on
   `try_bound_to_text_call`, and branch the spec parser
   (`objects.rs:1367-1406`) so a custom type's spec is read as a free-form raw
   string instead of the numeric `get_radix` grammar.  Reusable by every library.
4. **`time` release** — `struct DateTime { ms: integer }` + operators
   (`OpLt/…/OpEq/OpMin`) + `to_text` + constructor/accessor signature changes.
   Pure library work; civil math stays the pure-loft code already shipped.

## Training-app coverage — every API traces to a real use

Derived from grepping `../personal/training/python/*.py` (the parity
oracle).  The plan is "enough for the training app" iff every row has a
home in A–D.

| Training-app need (Python) | Covered by |
|---|---|
| `date.fromisoformat("YYYY-MM-DD")` (33×) | `time::parse(text) -> DateTime` |
| `datetime.fromisoformat(...)` w/ time (4×) | `time::parse` (accepts `YYYY-MM-DD[ T]HH:MM[:SS]`) |
| `datetime.fromtimestamp(epoch)` (5×) | `now()` returns `DateTime`; epoch-ms ctor `time::from_millis(i)` |
| `.isoformat()` (99×) | `time::format_date` / `time::format_iso` (basics); `{dt:date}`/`{dt:iso}` (full) |
| `.strftime("%Y-%m-%d")` / `"%H:%M"` / `"%a"` (4×) | `time::format_date` / `format_time` / `weekday_name` (basics); `{dt:…}` (full) |
| `date.today()` / `datetime.now()` (38×) | `now()` (instant) + `time::today(offset_min)` (local day) |
| `+ timedelta(days=N)` / `weeks=N` (many) | `time::add_days(dt,n)` / `time::add_weeks(dt,n)` |
| `(a - b).days` | `time::days_between(a,b) -> integer` |
| `(a - b).total_seconds()` (7×) | `(a - b)` → ms; `time::seconds_between(a,b)` |
| `d.weekday()` (6×) — week starts Monday | `time::weekday(dt) -> integer` (0=Mon … 6=Sun) |
| `d - timedelta(days=d.weekday())` (week start) | `time::start_of_week(dt)` |
| `isocalendar()` → (year, week) | `time::iso_year(dt)` / `time::iso_week(dt)` |
| local "today" via `zoneinfo` | `time::local_day(dt, offset_min)` / `time::today(offset_min)` — fixed offset, **no DST** |
| ordering / comparison of dates | built-in `==` `<` `>` on `DateTime` (chronological) |

## `DateTime` format tokens — proposal (now `time::to_text` specs, not core)

These are the spec strings the library's `to_text(self, spec)` interprets (the
general format hook, arc F) — **not** built-in core specifiers.  Mirror what JS
`Date.toISOString()` / field getters produce; keep the set small.

| Spec | Renders | JS analogue |
|---|---|---|
| `{dt}` | `2026-05-25 14:30` | — (loft default) |
| `{dt:date}` | `2026-05-25` | `toISOString().slice(0,10)` |
| `{dt:time}` | `14:30` | `getUTCHours`:`getUTCMinutes` |
| `{dt:datetime}` | `2026-05-25 14:30:07` | combined |
| `{dt:iso}` | `2026-05-25T14:30:07Z` | `toISOString()` |
| `{dt:wday}` | `Mon` | `getUTCDay()` → name |

## `lib/time` operations (arc D) — idiomatic loft names

Names chosen for loft, **not** transliterated from Python.

- **Construct / parse:** `parse(text) -> DateTime`,
  `from_millis(integer) -> DateTime`, `from_ymd(y,mo,d) -> DateTime`,
  `today(offset_min) -> DateTime`.
- **Step:** `add_days(dt,n)`, `add_weeks(dt,n)`, `add_seconds(dt,n)`.
- **Difference:** `days_between(a,b) -> integer`,
  `seconds_between(a,b) -> integer`.
- **Fields:** `year`, `month`, `day`, `hour`, `minute`, `second`,
  `weekday(dt) -> integer`, `iso_year`, `iso_week`.
- **Boundaries:** `start_of_day(dt)`, `start_of_week(dt)` (Monday).
- **Local (fixed offset, no DST):** `local_day(dt, offset_min)`.

All pure loft over the `DateTime` i64 + the arc-B conversion (exposed to
loft as the field accessors).  No native code in the library.

## Cross-backend testing — all four backends gated

The basics ship with two test files:
`lib/time/tests/01-basics.loft` (package smoke, travels on extraction)
and `tests/docs/32-time.loft` (monorepo docs).  Verified 2026-05-25,
`time` is **executed + asserted green on every backend**:

| Backend | Gate | Status for `time` |
|---|---|---|
| Interpreter | `wrap.rs::library_suite` (`loft test`) + docs `wrap.rs::dir` | runs + asserts pass |
| `--native` | `native.rs::native_library_suite` (`loft --native test`) + docs `native.rs` | runs + asserts pass |
| browser `feature="wasm"` | `html_wasm.rs::wasm_library_suite` → `--html` + `tools/wasm_repro.mjs` (Node) | **runs + asserts pass** |
| wasm32-wasip2 | `html_wasm.rs::wasm_library_suite` → rustc + wasmtime | **runs + asserts pass** |

**The WASM library gate now exists** — `tests/html_wasm.rs::wasm_library_suite`
(added 2026-05-25), closing the gap that
[`../12-library-extraction/`](../12-library-extraction) § WASM gate
flagged.  It iterates `lib/*/tests/*.loft`, and for each test that
declares a `fn main()` (the wasm entry point) runs it under **Node**
(browser `feature="wasm"` via `--html`) and **wasmtime** (`wasm32-wasip2`)
when those tools are available — self-skipping otherwise, so it's a no-op
on a box with neither.  `loft --native-wasm test` is NOT a building block:
the `test` subcommand runs the interpreter and ignores `--native-wasm`.

Skip-lists (`LIB_PKGS_WASM_SKIP` / `LIB_TESTS_WASM_SKIP`) park backends
that genuinely can't run: `server` (no browser/WASI sockets — a real
platform limit), `imaging` ([@P321c](../../PROBLEMS.md) — store-mutating
`#native` codegen; the right browser fix is a JS bridge over the
browser's own image codec, not a bundled Rust PNG stack), and `world`
([@P334](../../PROBLEMS.md) — traps on both wasm runtimes though it
passes interp + native; surfaced by this gate).

**Post-pivot note:** the `js_sys::Date` browser delegation is dropped (see
[DESIGN.md](DESIGN.md)); civil math stays pure-loft, so the existing
three-backend `time` gate already covers it with no extra parity path.  The
`time::to_text` renderings ride this same gate when the struct lands.

## Open design questions

> **Superseded 2026-06-14 by the [DESIGN.md](DESIGN.md) pivot.**  Q1 is no longer
> "built-in vs tagged integer" — `DateTime` is a **library struct** (distinct
> `Type::Reference`, so `dt + 5` is a compile error by construction without any
> core type).  Q3 operators are library `Op*` defs; Q6 tokens are `time::to_text`
> spec strings, not core specifiers.  Originals kept below for context.

1. **Distinct `Type::DateTime` vs a tagged `Integer`?**  Locked to
   *distinct* by the user, but confirm the storage/codegen route:
   reuse the i64 integer storage path (`i64`, `i64::MIN` null) and add
   only the static-type identity + format-default + operator rules,
   rather than a wholly new storage class.
2. **Literals in source, or constructor-only?**  Likely
   constructor-only (`time::parse("…")` / `now()`); no `DateTime`
   literal syntax in the lexer.  Confirm.
3. **`DateTime − DateTime` → ?**  Proposal: plain `integer`
   milliseconds (no `Duration` type — keeps it "basic"); library wraps
   as `days_between`/`seconds_between`.  `DateTime + integer` — allow
   (ms offset) or force `time::add_*`?  Leaning: forbid bare `+ integer`
   (ambiguous unit), require library steps.
4. **`weekday` as `integer` or a `Weekday` enum in `lib/time`?**
   **RESOLVED in basics:** `integer` 0=Mon…6=Sun (plus `weekday_name`
   for the `%a` need).  An enum can be added later without breaking
   callers.
5. **Library name: `lib/time` vs `lib/date`?**  **RESOLVED: `time`**
   (covers both date and time-of-day); lives in `loft-libs-core`.
6. **`{dt:…}` token vocabulary** (table above) + the bare-`{dt}`
   default (`datetime` to the minute? full `iso`?).  Still open — the
   basics ship `time::format_*` text renderers; the `{dt:…}` specifiers
   are the arc-C full-support tail.
7. **Parse strictness** — **RESOLVED in basics:** accept `YYYY-MM-DD`,
   optional ` `/`T` + `HH:MM[:SS]`, optional `Z`; anything malformed
   returns null.

## Cross-arc dependencies

- **lib_plans/12-library-extraction** — `lib/time` is on the extraction
  roster in the **`loft-libs-core`** chunk (Tier A — pure-loft, no
  `native/` dir, zero compiler-crate coupling).  The training app
  consumes it as a registry dependency (`loft install time`), so it
  must be a published library, not monorepo-internal.  **Boundary
  note (post-pivot):** `DateTime` is now a `lib/time` **struct** — fully
  library code in the `loft-libs-core` chunk, type and operators and all.
  The only core piece is the *general* `to_text(self, spec)` format hook
  (arc F), which is type-agnostic and serves every library, not a
  DateTime primitive.
- **lib_plans/59-lazy-stdlib** — if `lib/time` ever grows a
  native bridge, it inherits the lazy-load trigger pattern; the
  pure-loft design avoids needing it.

## See also

- [DESIGN.md](DESIGN.md) — the struct + general `to_text(self, spec)` format-hook
  design that supersedes the old built-in arcs A/B/C.
- `doc/claude/LOFT_LITERALS.md § String formatting` — the format system the `to_text` hook
  (arc F) extends.
- `doc/claude/BROADENING.md` — Data/ETL gap list naming the date hole.
- `../personal/training/MIGRATION.md` — the driving consumer + parity oracle.
- ROADMAP.md `F` table row `TIME.1`.
