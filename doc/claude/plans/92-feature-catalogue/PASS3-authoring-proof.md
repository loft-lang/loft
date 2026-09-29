<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN92 Pass 3 — authoring proof (go/no-go evidence)

**Purpose.** @PLN92's whole justification is gated on one question ([README §
Success criterion](README.md#success-criterion--gated-on-doc-improvement)): does
authoring self-contained per-feature docs produce *significantly better* language
documentation? This is the cheap falsification test — three features authored in full
(`## What it is` / `## How it aids you` / `## Example`), examples run cross-backend, put
beside today's LOFT.md.

**Method.** Authored **@F17** (named args), **@F29** (match), **@F22** (closures). Each
example was run on **both** backends (`--interpret` + `--native`) and produced identical
output. The three drafts are **now live** in
[`loft-lang/features`](https://github.com/loft-lang/features) issues #17, #29, #22 —
subjects as their own headers, user-facing prose, no technicalities.

## Verdict

**Significantly better for the *terse* half of the catalogue; a modest reformatting for
the already-detailed half.** The uniform wins are real and apply to every feature:

- an explicit **value** statement — LOFT.md usually says *what*, rarely *why you'd want it*;
- a **complete, tested** example — today's are often un-runnable fragments;
- consistent structure across all features; single source (docs + `tests/docs` generate from the issue).

But the format does **not** make docs *deeper* — depth still comes from writing. For
already-rich sections (match, closures) the gain is polish + a value line + a tested
example; to match their depth the authored issue must be as long (self-contained allows
it — that's *more writing, not less content*). So: **uniform, incremental improvement,
biggest where docs are thinnest.**

Roughly half the catalogue is terse like @F17 — whose current LOFT.md example has **no
function body and prints nothing** — so the aggregate lift is real, but it is an
**authoring grind proportional to 56 features**, not a free structural upgrade.

**Go/no-go:** worth it *iff* the goal is consistency + explicit value + tested examples +
single-source across all 56. If deeper docs were expected from the *format alone*, it
won't deliver that — depth is writing, feature by feature.

## Per-feature comparison to today's LOFT.md

| Feature | LOFT.md today | Authored delta |
|---|---|---|
| **@F17** named args | 3 sentences + a fragment (`connect` has **no body**, prints nothing) | **large** — value statement + complete runnable+verified example + self-contained |
| **@F29** match | already detailed (enum destructure, guards, exhaustiveness nuance) | **modest** — value statement + one tested example + uniform shape; *less* comprehensive unless lengthened to match |
| **@F22** closures | already detailed (per-type capture rules, cross-scope, limitations) | **modest** — same; the win is polish + a tested example |

## The three authored drafts

Live in issues #17 / #29 / #22. Reproduced here as the recorded evidence.

### @F17 — Named arguments + default parameter values

#### What it is

Pass a function's arguments by name — `connect(host: "db", port: 5432)` — instead of only
in order, and give parameters default values so callers can leave them out.

#### How it aids you

Call sites read like their own documentation, and you can set just the one option you
care about without listing everything before it. Adding a new optional setting later
never breaks the calls people already wrote.

#### Example

```loft
fn connect(host: text, port: integer = 8080, tls: boolean = true) -> text {
    "{host}:{port} tls={tls}"
}
fn main() {
    println(connect("example.com"));              // example.com:8080 tls=true
    println(connect("example.com", tls: false));  // example.com:8080 tls=false
    println(connect(host: "db", port: 5432));     // db:5432 tls=true
}
```

### @F29 — Pattern matching (match)

#### What it is

`match` picks one branch by comparing a value against a list of patterns, and gives back
that branch's result. A pattern can be a specific value, a range, several options at
once, or a catch-all. Every possibility has to be covered.

#### How it aids you

One clear choice replaces a long chain of `if` / `else if`, and nothing can slip through
uncovered. The chosen result drops straight into a variable instead of being set piece by
piece across branches.

#### Example

```loft
fn grade(score: integer) -> text {
    match score {
        null     => "absent",
        90..=100 => "A",
        80..90   => "B",
        _        => "other"
    }
}
fn main() {
    println(grade(95));  // A
    println(grade(85));  // B
    println(grade(50));  // other
}
```

### @F22 — Closures & lambdas

#### What it is

A closure is a small anonymous function you can keep in a variable, hand to another
function, or return. It remembers the values it uses from around it, taken at the moment
you create it.

#### How it aids you

Write a short piece of behaviour right where you need it — a callback, a quick rule, or a
function that builds another function — without naming a separate function for each.
Because it captures its values when it's made, it keeps behaving the same even if those
values change afterwards.

#### Example

```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }
}
fn main() {
    add5 = make_adder(5);
    println("add5(10) = {add5(10)}");      // add5(10) = 15
    greeting = "Hello";
    greet = fn(name: text) -> text { "{greeting}, {name}" };
    greeting = "Bye";
    println(greet("world"));               // Hello, world
}
```

## Result — GO taken (2026-07-01)

GO, and the full catalogue is now authored: **79/80 issues** (`@F` 55/56 + `@I` 24/24;
`@F43` random deferred as a library). Each `@F` `## Example` was run byte-identical on
`--interpret` and `--native` before push. 9 authored by hand, 70 across 6 parallel
author-agents; spot-checked for the `## What` / `## How` / `## Example` shape + plain
non-technical prose across the null / functions / math / IR areas — consistent and clean.

**The decisive ROI evidence was a by-product of authoring, not of the format.** Because
every `## Example` is *run*, authoring surfaced reference errors that un-runnable
fragments had hidden for a long time:

- **`sizeof(integer)` documented as `4`** (LOFT_DATA.md § Sizeof) — it is **8**. Fixed.
- **`n: integer = null`** shown as a valid annotation (loft-write skill) — the language
  **rejects** it (`use … 'as'`); the idiom is `null as integer`. Fixed.
- A third suspected gap — an enum struct-variant inside a vector literal — was a **false
  alarm**: an agent's probe used non-loft qualified syntax (`Shape.Circle { … }`); the
  canonical **bare** form (`Circle { … }`) works in every position (assignment, vector
  literal, call arg). No language gap — the verification loop self-corrected the
  mis-report.

That is the improvement made concrete: the tested-example format enforces *correctness*,
not just consistency + a value line — the very thing the [Success criterion](README.md#success-criterion--gated-on-doc-improvement)
gated on.

**Remaining for @PLN92** (not part of this proof): strand-3 sync automation (render the
in-project mirror + extract each `## Example` → `tests/docs/@F<n>.loft` + the two CI
guards), strand-4 coverage gate, strand-5 hygiene.

## See also

- [README.md § Success criterion](README.md#success-criterion--gated-on-doc-improvement) — the ROI gate this tests.
- `loft-lang/features` #17 / #29 / #22 — the live authored issues.
- `@PLN92` — the tracker issue.
