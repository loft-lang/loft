<!-- reference for the loft-write skill: read when a value may be null, or a cast or text parse is involved -->
# loft-write: null, `?` and casts

The null-and-casts half of the [loft-write skill](../SKILL.md).  Read it when a slot may hold
null, when you discharge a `τ?`, when you parse text into a number or an enum, or when you
cast to a variant.

## Nullability defaults

**Every scalar and vector is NON-NULL by default** (@PLN25 DN1).  A plain `integer` / `float` /
`single` / `boolean` / `character` / `text` (and `vector<T>`) field, local, or return does not
intend `null`: storing `null` or an undischarged `τ?` into it is a WARNING (`(N-Store)` — the
slot then holds null, and a library's deny-warnings CI fails on it), and a bare text parse into
it is refused (`text-parse-may-fail`, below).  To ALLOW null, append `?` to the type
(`integer?`, `text?`, `vector<T>?`).  Don't write the `not null` modifier: it means what the
default already is.

A plain `vector<T>` field defaults to `[]` (empty, non-null) — declare it plainly; write
`vector<T>?` only when null must be distinct from empty.

## Uncomputable arithmetic answers null and the program keeps running

Divide / modulo by zero and integer overflow (`i64::MAX + 1`) are uncomputable (@C80, the
spreadsheet model), so they yield `null` and execution CONTINUES — no trap, no halt in every
build.  Any arithmetic landing exactly on `i64::MIN` also reads as `null` (the in-band
sentinel).  Defend a site with `?? <fallback>` to turn the null into a value; a CONSTANT zero
divisor warns at compile time; a runtime one, like an index out of range, logs one Warn line
unless the site is defended (`??`, a null check) — the value is null either way.  A function
returning `integer` may `return null` (it yields `i64::MIN`, detectable with `!result`).

## Postfix `?` — the type's own default

**Prefer `x?` when the fallback is just the type's zero (@PLN116).**  Postfix `?` discharges
`T?` to `T` using **the type's own** default, so `?? 0` / `?? 0.0` / `?? ""` /
`?? Pixel{r:0,g:0,b:0}` boilerplate collapses to one character: `row.label?` on a `text?`
answers `""`.  `??` supplies *the default you give*; `?` supplies *the default the type gives*
— that pairing is the mnemonic.

<!-- from tests/reference/skill-null-and-casts.loft -->
```loft
first = points[i]?;         // Point?  → Point{} (every field defaulted)
count = (a / b)?;           // integer? → 0      (parens — see below)
```

Defaults: scalar → `0`/`0.0`/`false`/`'\0'`, `text` → `""`, collection → empty, record →
every field defaulted, nullable `U?` → `null`, bare enum → its **first-declared** variant.

Three things to know before reaching for it:

- **`?` binds TIGHTEST, `??` binds LOOSEST** — opposite ends.  On a *binary* result that flips
  the parens: `a / b ?? 0` discharges the division, but `a / b?` is `a / (b?)` and evaluates to
  **null**.  Write `(a / b)?`.  So `?` is a clear win on a **primary** (`row.label?`, `v[i]?`,
  `x?` — no parens) and a wash on arithmetic.
- **On an enum it is positional.**  `c?` is the first-*declared* variant, so reordering the
  enum silently changes it.  Use `?? Colour.Red` when the variant matters.
- **Two types have no default, so `x?` is a COMPILE error** (never a runtime one): a bare
  reference, and a record with a bare enum field lacking `= <variant>`.  The error names the
  fixes; `??` or `match` is the fallback.

## A null test that leaves narrows

After `if v == null { return …; }` or `assert(v != null, …)`, `v` reads as present to the end
of the block (until it is assigned again); no `?? d` needed.  The same holds for an index bound,
a non-zero divisor and a non-negative `sqrt` operand proven by an `if … { return }` or an
`assert`.

## Reading a maybe-null element `v[i]`

Pick by contract: `v[i] ?? sentinel` when a non-null default is meaningful (changes behaviour on
OOB); `if i < len(v) { v[i] }` when the access is index-bounded (works on a struct field
directly — no capture needed); `x = v[i]; if x != null { … }` when the consumer's contract is
"missing → null".  A negative `i` is not caught by any of these — it counts from the end (the
skill's § Vectors).

## Parsing text is fallible: `text as integer` types `integer?`

`as float` → `float?`, `as single` → `single?`.  `"42" as integer` is a **parse**, not a numeric
cast — a non-numeric string yields `null`, so the compiler types the result nullable and
**rejects storing it into a non-null slot**.  Discharge at the parse site:
`n = s as integer ?? 0` (default), `n = s as integer?` (keep it nullable), or `match`.  This is
the same reachable-fault rule as `÷0` / out-of-bounds indexing (@PLN25 `(N-Parse)`).  A
*numeric* `as` (`3.14 as integer`, `x as u8`) is not a parse and does not add `?` on its own (a
narrowing numeric cast that can miss uses `as u8?`).

**At a text parse, put `?` after the CHECKED cast:**

<!-- from tests/reference/skill-null-and-casts.loft -->
```loft
n = (s as integer?)?;    // integer — checked cast, then default ⇒ 0 on a bad parse
m = s as integer ?? 0;   // integer — the assert-or-default form
o = s as integer?;       // integer? — the nullable CAST (that `?` belongs to the type)
```

and a bare `as` on text is an assertion, so the `?` on it is refused:

<!-- from tests/reference/skill-casts-refused.loft -->
```loft
n = (s as integer)?;     // COMPILE ERROR — a bare `as` on text is an assertion
```

A bare `text as integer` is an *assertion* the compiler refuses (a parse can't be proven), so
`(s as integer)?` fails on the inner cast — not because `?` is unavailable.  Write the checked
cast first.

## A cast to a variant is non-null

`s as Circle` answers a `Circle` (@C131); on a miss its fields are defaulted and it WARNS
(`variant-cast-default`) unless checked first (`if s is Circle { … }`) or spelled `as Circle?`
(null on a miss).  A miss the compiler can prove is an error.  Likewise `t as Color` (text →
plain enum) answers the FIRST variant on a miss and warns (`enum-parse-default`); write
`as Color?` or `t as Color ?? Blue`.
