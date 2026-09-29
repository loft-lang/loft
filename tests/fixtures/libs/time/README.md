<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# time — date/time operations for loft

```sh
loft install time
```

Pure-loft date / time arithmetic on millisecond-since-epoch
integers — the same model as JavaScript's `Date`.  Identical results
on the interpreter, `--native`, and WASM.

Two layers, use whichever fits: a **`value struct DateTime` / `Duration`** for
first-grade, type-safe, zero-cost dates (0.2.0), on top of the original
`integer`-epoch functions (unchanged — the 0.1.0 surface still works).

## DateTime / Duration (0.2.0)

`DateTime` and `Duration` are zero-cost `value struct`s over the same epoch-ms
`integer` — so they add **no** heap allocation as a record field or vector
element (a `vector<DateTime>` costs the same as `vector<integer>`), yet they are
a distinct type: `dt + 5` is a compile error, only `dt + hours(2)` steps a time.

```loft
use time;

d1 = time::datetime(2026, 7, 8, 12, 0, 0);   // or time::date(y, mo, d)
d2 = "2026-07-08T13:30:00" as DateTime;        // total best-effort parse
span = d2 - d1;                                 // -> Duration
later = d1 + time::hours(2);                     // typed stepping
if d1 < d2 { }                                   // all six comparisons
n = d1.year(); mm = d1.month(); w = d1.weekday_name();
s = "{d1:date}";   // 2026-07-08   ·  "{d1:iso}" · "{d1:time}" · "{d1}" · "{span}"
```

- **Construct**: `date(y,mo,d)`, `datetime(y,mo,d,h,mi,s)`, `ms as DateTime`,
  `"…" as DateTime`, `now() as DateTime`.
- **Read (methods)**: `year month day hour minute second weekday iso_year
  iso_week weekday_name month_name to_millis`.
- **Operators**: `< <= > >= == !=`; `dt - dt -> Duration`; `dt + Duration ->
  DateTime`; `dt.minus(Duration)`.
- **Duration**: `milliseconds seconds minutes hours days weeks`; `+ - *`,
  comparisons, `negate`, `total_millis/seconds/minutes/hours/days`.
- **Format**: `{dt}` `{dt:date}` `{dt:time}` `{dt:seconds}` `{dt:iso}`
  `{dt:wday}` `{dt:month}` · `{dur}` → `[-]H:MM:SS`.
- **Nullability**: a `value struct` has no null, so a fallible parse cannot
  return `DateTime` *and* signal failure.  Keep failure at the integer level —
  `parse(s)` is null when the text is not *shaped* like a date — then wrap:
  `ms = parse(s); if !ms { … } else { dt = ms as DateTime }`.  `"…" as DateTime`
  is a total best-effort parse for the path where a bad date need not be caught.
  ⚠ Neither door rejects an *impossible* date: out-of-range fields roll over
  (`"2026-02-30"` → 2026-03-02), so a form that must reject one round-trips it
  through `format_date` — see `@TIM-002` below.

## The integer-epoch API (0.1.0, still current)

- **Construct / parse**: `from_ymd`, `from_millis`, `parse`, `today`.
- **Field extraction (UTC)**: `year`, `month`, `day`, `hour`, `minute`,
  `second`, `weekday` (0=Mon..6=Sun), `iso_year`, `iso_week`.
- **Step**: `add_days`, `add_weeks`, `add_seconds`.
- **Difference**: `days_between`, `seconds_between`.
- **Boundaries**: `start_of_day`, `start_of_week` (Monday).
- **Local time**: fixed-offset (minutes) — `to_local`, `local_day`,
  `today(offset_minutes)`.  No DST, no tz database.

## The five traps a signature does not carry

A time here is a bare `integer`, so the compiler cannot tell milliseconds from
days, an instant from an offset-shifted one, or a calendar year from an ISO
one.  Each row below links to a test that demonstrates the correct call and is
run by CI — the code is the documentation, so it cannot go stale.

| the question | the trap | worked example |
|---|---|---|
| stepping a time | the unit is **milliseconds**: `t - 3` moves three thousandths of a second, not three days.  `add_days` / `add_weeks` / `add_seconds` are the unit doors; `DateTime` + `Duration` puts the unit in the type | [`@TIM-001`](tests/03-worked-examples.loft) |
| reading a date a user typed | `parse`'s null means *not shaped like a date*, never *not a real date* — `"2026-13-45"` answers 2027-02-14, non-null, and `as DateTime` turns prose into the epoch | [`@TIM-002`](tests/03-worked-examples.loft) |
| how long ago was this | `days_between` counts **midnights crossed**, so a 26-hour span reads 2 where a 46-hour span reads 1; `seconds_between` is elapsed time and truncates toward zero | [`@TIM-003`](tests/03-worked-examples.loft) |
| a user's local day | `to_local` shifts the **instant** and `local_day` answers a bucket **key** — neither may be compared with a real timestamp | [`@TIM-004`](tests/03-worked-examples.loft) |
| a weekly report key | an ISO week number is only meaningful beside `iso_year`; pairing it with `year()` invents `2021-W53` and splits a real week in half | [`@TIM-005`](tests/03-worked-examples.loft) |

## Why this package exists

Every game needs frame timing, run-length tracking, and date display
(scores, save-file timestamps, daily-challenge boundaries).  Bundling
those primitives in a small, dependency-free, identically-behaving
package means a game can drop it in without pulling in a graphics or
windowing stack.

The proleptic Gregorian calendar implementation is Howard Hinnant's
days_from_civil / civil_from_days — field-for-field match with
`js_sys::Date`, so WASM consumers see the same values as native.

## Tests

```sh
cd time && loft --interpret --tests tests
```

`tests/01-basics.loft` covers the integer-epoch API (construction, field
extraction across the year, leap-year boundaries, week math, ISO week
numbering, local-day bucketing under several offsets).
`tests/02-datetime.loft` covers the `DateTime` / `Duration` value types
(construction, fields, operators, typed arithmetic, formatting, conversions,
and zero-cost use as vector elements).  `tests/03-worked-examples.loft` is the
five worked examples in the table above — one test per contract, each written
the way a caller would write the call.  All goldens are hand-computed and
verified identical on `--interpret` and `--native`.
