<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# <N> — Regex captures, find-all and replace: results as spans, text as the caller's view

> **Identity.** This file takes the number the tracker returns: file the plan issue in
> `loft-lang/plans` (one `status:*` + `subject:library` label), then rename this file to
> `<N>-regex-captures-as-spans.md` and set the `@PLN<N>` below.  The `_PROPOSED` prefix
> exists only because the number is minted by the tracker, never by scanning this directory
> (loft-plan-workflow skill § A plan's identity).

Plan issue: `@PLN<N>` (to file).  Library: [`loft-lang/loft-libs-core/regex`](https://github.com/loft-lang/loft-libs-core/tree/main/regex)
(0.4.3).  Driver: [@PLN179 finding 011](179-scripts-in-loft/findings/011-regex-has-no-capture-groups.md)
— four of the eight ports cite it, and it is why those ports read two to three times longer
than their Python originals.  Value **F** (friction-free scripting) · effort **M**, in S
steps that each finish inside the PR cadence.

## Status (REQUIRED)

Open — design only.  Nothing built.  The shipped library answers `matches` / `search` /
`split_on` (a byte offset, a boolean, a lazy split); captures, `find_all` and `replace` are
its README's roadmap and the register's longest-open clarity reason.  The legacy design in
[lib_plans/57-regex](../lib_plans/57-regex/README.md) (a `Regex` handle and a `Match` that
CARRIES its texts) is superseded by this one on the copy question below; its feature table
and its "why a library, not a language feature" argument stand.

## Goal (REQUIRED)

A loft script pulls a value out of a text the way Python does (`m.group(1)`, `re.findall`,
`re.sub`), with no `use` line, with no text copied on `--native` where the Rust version
copies none, with every offset a byte position on a code-point boundary, and with the
interpreter as the copying oracle that proves the native answer.

## The invariant (design-protocol step 1)

> **A regex result is a SPAN of its subject — a byte start and end on code-point
> boundaries — and nothing else.  The library answers spans; the text a span names is the
> caller's own slice of the subject, `line[s.start..s.end]`, which the existing slice rules
> make a view on `--native` and a copy on the interpreter.**

Under this one rule a case nobody tested behaves like the tested ones for the same reason:
there is no second representation.  A result holds no text, so there is nothing to copy at
any boundary — not at the native bridge (integers cross it), not at the library's cdylib ABI
(a record of integers crosses it), not at a `yield`, a store or a return.  The only text that
exists is the one the consumer slices, in the consumer's own frame, where
[`(Slice-Value)`](../formal/collections.md), [`(R-TextBorrow)`](../formal/rewrites.md) and
[`(R-DecodeView)`](../formal/rewrites.md) (@C141) already decide view-or-copy.

**Why not a `Match` that carries its texts** (the 57-regex design, and what Python hands
back).  Three facts, each checkable, decide it:

1. **A record's `text` field is owned bytes.**  `Match { text: line[a..b] }` copies on the
   interpreter (`(B-Copy)`), and on native it is a `String` unless `(R-DecodeView)` proves the
   record confined to the frame that holds the subject.
2. **The library's `pub fn` is compiled into its own cdylib** (`native-auto/`, prebuilt when
   published): a record answered across that ABI is written into the caller's store field by
   field (`LoftStore::set_text` — a copy per text field), and the exported API is the rewrite
   boundary (LIBRARY_AUTHORING.md § contract, rule 11).  So a `Match` built inside the
   library can never be a span in the consumer, however good the rewrite.
3. **A generator declines every span rule** (`(R-DecodeView)` (3), `(R-TextBorrow)`), so the
   `iterator<Match>` shape copies on native too.

Spans sidestep all three: no text, no boundary, no proof.  The Rust reference (`&str` into the
haystack) is reached by the consumer's slice, not by the library's answer — which is exactly
how the stdlib's own `find` → `s[a..b]` already works.  That is the "loft way" this plan
leans on, and it is why the compiler step below (P3) is one general slice rule and not a
regex-shaped representation.

**Re-assertion sites (step 2).**  The invariant is stated at TWO sites, both loud when
omitted: the native bridge, which answers every span through ONE function (`n_spans`, below —
there is no second span-producing native), and the loft surface, which turns a span into
text through ONE method (`Span.of`).  The code-point-boundary half is asserted by nobody in
loft: it is the Rust `regex` crate's own guarantee over `&str` (every `Match` is on a `char`
boundary), probed in P0 and pinned by cells, never re-derived.

## The surface

All methods are `self: text` and so become auto-use triggers (`src/triggers.rs` derives them;
`[triggers] enabled` is the library's whole declaration): a script writes
`line.capture("…")` with no `use`, as it writes `line.matches("…")` today.

```loft
pub struct Span  { start: integer, end: integer }          // bytes; end exclusive; a miss is null
pub struct Match { whole: Span, groups: vector<Span?>, names: vector<text> }

pub fn capture(self: text, pattern: text) -> Match?         // first match, with its groups
pub fn capture_all(self: text, pattern: text) -> vector<Match>   // every non-overlapping match
pub fn search_all(self: text, pattern: text) -> vector<Span>     // every match, groups not kept
pub fn replace_on(self: text, pattern: text, template: text, limit: integer = 0) -> text
pub fn of(self: Span, subject: text) -> text               // subject[self.start..self.end]
pub fn group(self: Match, subject: text, i: integer) -> text?    // null for an unmatched group
pub fn named(self: Match, subject: text, name: text) -> text?
```

```loft
// finding 011's three shapes, as they read with this surface
m = line.capture("^(\\w+)=(.+)$");
if m != null {
  key = m.group(line, 1)?;                 // "fn" — a view on native, a copy interpreted
  value = m.group(line, 2)?;
}
for h in header.capture_all("HTTP/(?<maj>\\d)\\.(?<min>\\d)") {
  version(h.named(header, "maj")?, h.named(header, "min")?);
}
out = "";                                  // re.sub with a callback: the walk idiom
last = 0;
for m in s.capture_all("\\b[a-z]+\\b") {     // every word upper-cased, nothing copied twice
  out += s[last..m.whole.start] + m.whole.of(s).to_uppercase();
  last = m.whole.end;
}
out += s[last..];
```

**Names.**  The library's rule stands: the pattern-taking twin of a stdlib text verb takes a
distinct name, because a bare method call resolves the stdlib first and the wrong one still
compiles (`@RGX-001`).  The stdlib's text methods today: `find rfind replace split
split_text contains starts_with ends_with join trim* to_*case char_slice byte_at len size
is_*`.  So `replace_on` beside `split_on`, `capture` / `capture_all` / `search_all` beside
`search`.  `Span.of` and `Match.group` live on the library's own types, so no future stdlib
text method can take them.  `at` was declined for the subject-indexing method: it is the
convention name for a type's own index method (`x.at(i)`), the likeliest future stdlib
addition on `text`.

**The subject is an argument, not a field.**  `m.group(line, 1)` is one word longer than
Python's `m.group(1)`, and that word is the whole design: it is what keeps the subject out
of the record.  A `Match` that remembered its subject would be the copy this plan refuses.

**Broken patterns.**  `@RGX-003` stands for the value channel (a miss, never a fault), and
gains the @C80 channel the stdlib uses for an unguarded `÷ 0`: the first use of a pattern
that does not compile logs ONE warning naming the pattern and the parser's message.  The
cache already remembers a failed pattern as `None`; it remembers the message too, so the
cost is one log line per broken pattern per process.

**`search` becomes honest** (`@RGX-002`'s second half): its declared `integer` return
answers null on a miss.  It is redeclared `integer?` in the same release, which is the
@PLN102 rule for a not-found case and a signature change `api_compatible_with` records.

## Code points

- **Offsets are bytes**, the unit of `size`, `s[a..b]`, `find` and `byte_at` — never
  `len`'s character count.  Every span starts and ends on a code-point boundary because the
  engine matches `&str` by `char`: `.` is one code point (four bytes on `"😀"`), a class is
  Unicode (`\w` takes `é`, `\p{L}` the letters, `\d` the Unicode digits), `\b` is the Unicode
  word boundary, `(?i)` is simple case folding, and a pattern that could match inside a
  code point is refused by the compiler (an invalid pattern, logged as above).  So
  `s[sp.start..sp.end]` can never cut a character, where `"héllo"[-4..]` can.
- **No normalisation.**  `é` and `e` + U+0301 are different texts to the engine, as they are
  to `==`.  A cell pins it.
- **A character count of a prefix** (a column, a caret) is a stdlib question, not a regex
  one: `len(line[..sp.start])` copies on both backends today (and loft#1728 pinned
  `len(v[a..b])`).  The one-line home is `text.char_index(byte: integer) -> integer`, the
  inverse `char_slice` lacks; it is listed under open questions and never built here.

## How it stays out of the way (the built-in feel), and what it must not do

| today | after |
|---|---|
| `line.matches(p)` loads the library from a bare method call | unchanged; the four new verbs derive their triggers the same way |
| `search` answers null from an `integer` | `integer?` |
| a broken pattern is a silent miss | a miss, plus one warning per pattern |
| `split_on` re-slices `rest = input[pos..n]` per piece, so `^` and `\b` see a fresh start at every boundary | rebuilt on the engine's `find_at(input, pos)`: the context is the whole subject (P0 probe d measures the difference) |
| pattern literals are text; an interpolated value becomes syntax | P5 (optional): `p: Pattern = "^{name}="` through the typed-format target (`(F-Target)`, @PLN124) — literal runs are syntax, holes are escaped, a `Pattern` type trigger loads the library; a `text` pattern stays accepted |

Not done here, each a decision rather than an omission: a `Regex` handle (the cache makes a
compile step a worse API), a callback `replace` (closures do not cross a placed library,
PLACEMENT.md rule 3 — the walk idiom above is four lines and copies nothing), a lazy
`iterator<Match>` (a generator declines every span rule; `capture_all` answers a vector of
integers, which is small), a pure-loft engine (57-regex phase 1, still future).

## Composition matrix — Stage A

The cells, each hand-computed, `--interpret` and `--native`, graduated to
`tests/scripts/<N>-regex-spans.loft` and the library's `tests/worked-examples.loft`
(`@RGX-005` …):

| axis | values |
|---|---|
| subject | ASCII · multibyte before the match · multibyte inside it · empty · match at byte 0 · match at the end · an empty match (`a*` on `"b"`) |
| pattern | no group · one group · an optional group that did not take part (`null` span) · nested groups · alternation choosing a group · a named group · `(?i)` · `\p{L}` · `^` and `\b` against a second match (the `find_at` cell) · a broken pattern |
| count | 0 · 1 · many non-overlapping (`aa` on `"aaa"` is one) |
| consumer | the slice bound to a local and read as a value · appended to · returned · stored in a record · in a loop · in a generator (P3's decline cells) |
| limit | `replace_on` with 0 · 1 · more than the matches |

`python3 scripts/matrix_axes.py file <guard>` says which values the cells reach.

## Sub-arcs

| Item | Source | Verify (goes RED if the phase is wrong) | Status |
|---|---|---|---|
| **P0** probe the four claims (XS) | § P0 | each probe has a predicted outcome written before it runs; a claim that survives its probe is a claim, one that falls reroutes the design | Open |
| **P1** spans: `Span`, `Match`, `capture`, `capture_all`, `search_all`, `Span.of`, `group`, `named`, ONE native `n_spans` (S) | § The surface | the matrix above, both backends; the trigger test with no `use`; `tests/dump_ignored_tests` re-twinned over `capture` and finding 011 re-measured (`scripts/script_recheck.sh`) — the port's lines drop or the phase failed | Open |
| **P2** `replace_on` with `$1` / `${name}` templates and `limit`; `split_on` on `find_at` (S) | § The surface | the Python twin of `opl_points`' `%hex%` unescape (walk idiom) and of a `re.sub` template; the `^`/`\b` boundary cells that today answer differently | Open |
| **P3** `(R-SliceBorrow)`: a text local bound to a slice of an unwritten text is a `&str` (S, compiler, native only) | § P3 | emission pins (`tests/slice_borrow.rs`, the shape of `tests/text_borrow.rs`); the consumer axis cells under `LOFT_NO_SLICE_BORROW=1`, `LOFT_HOIST_VERIFY=1`, `LOFT_POISON=1`; `make rewrite-census` gains the rule's row; a planted defect that admits a written subject must read stale bytes under the verify switch | Open |
| **P4** the broken-pattern warning (XS) | § Broken patterns | a cell asserting the one warning on stderr and its absence on the second use; `@RGX-003` re-measured | Open |
| **P5** `Pattern` typed-format target (S, optional) | § built-in feel | a hole holding `.` matches only a literal dot; the type trigger loads the library with no `use`; `(F-Target-Pos)`: an argument position does not target | Open |
| **P6** `search -> integer?`, the `regex_` spellings removed per COMPATIBILITY.md, `api_compatible_with` bumped (XS) | § `search` | the library's own CI under `LOFT_DENY_WARNINGS=1`; `make revalidate-libs` for the consumers that call `search` | Open |

P0 → P1 → P2 ship as one library release (0.5.0); P3 and P4 are independent of each other
and of the release; P5 waits for a port that interpolates into a pattern; P6 rides the
release.

### P0 — the probes, with their predictions

Each is a throwaway under the scratchpad, `--interpret` and `--native-emit`, run before a
line of P1 is written.  The prediction is the point: a probe that cannot surprise is not one.

- **(a) A record with a text field answered by a dependency's `pub fn` crosses the cdylib
  ABI as a copy.**  Emit a consumer of a one-function path library answering
  `Row { t: self[1..3] }`; read the call site.  *Predicted:* a `set_text` into the caller's
  store.  If it is inlined instead, fact 2 is wrong and a text-carrying `Match` is back on the
  table for the inlined case only.
- **(b) `t = line[a..b]` bound to a local is a `String` today on native.**  Emit
  `fn f(line: text) -> integer { t = line[2..5]; size(t) }`.  *Predicted:* `sub_text(...)`
  into an owned `String`.  This is P3's gap; if a `&str` already appears, P3 is a no-op and
  is struck.
- **(c) Every offset the engine answers is a code-point boundary.**  `"héllo wörld".search("w.r")`
  → 7, end 12; `"😀x".search(".")` → 0..4; `(?-u:\xFF)` → invalid.  Through the shipped
  `search` and `split_on`, both backends.  *Predicted:* holds; the `(?-u)` cell is the one
  that could surprise.
- **(d) `split_on` loses `^` and `\b` context at a piece boundary.**  `"ab ab".split_on("^ab")`
  → *predicted today* `["", " ", ""]` (two matches: the slice restarts `^`), *correct*
  `["", " ab"]`.  Red today is the expected outcome and P2's receipt.

### P3 — `(R-SliceBorrow)`, the one compiler step

```
  (R-SliceBorrow) a text LOCAL t bound ONCE to a value slice s[a..b] of a text s that is a
                  parameter or a local of this frame, where (1) s is not written, rebound,
                  linked or released on any path from the bind to the last read of t;
                  (2) t is read only as a text VALUE ((R-TextBorrow)'s reading: an operand,
                  a `text` argument the callee does not store, the source of a format or a
                  bind into another slot, `size`, a comparison, a walk of its characters);
                  (3) t is never written, linked, returned, stored, captured, yielded or
                  handed to a `&text` parameter; (4) a bind inside a loop reaches only reads
                  of the same iteration; (5) the function is not a generator —
                  is a `&str` into s: no `String` is built.  Declines, keeping the copy,
                  when any of (1)–(5) is unproved.  Native only; the interpreter copies and
                  is the oracle.  Switch `LOFT_NO_SLICE_BORROW`; `LOFT_HOIST_VERIFY=1`
                  re-reads the slice as the owned text at t's last read and compares.
```

It is `(R-WorkView)` with the work buffer replaced by a parameter or local, and
`(R-TextBorrow)`'s value-only reading — the same five conditions, so the analysis it needs
exists (`hoist::borrowed_text_walks` decides (1)–(3) for a walk).  Reach is every script that
slices: finding 002's tokenisers, every `find` → `s[a..b]`, and every regex consumer here.
Decided in the formal register as a rule with its own `@FR-` tag, its code sites citing it,
and `make rule-coverage` counting it.

## Open design questions

1. **`text.char_index(byte) -> integer`** — the inverse of `char_slice`, so a span's column
   is one call and no copy.  Stdlib, one line, its own guard; recommended, filed as its own
   XS issue rather than widened into this plan.
2. **Should `capture` on a pattern with no group at all answer a `Match` or a `Span`?**  A
   `Match` with an empty `groups` (one type, one code path); `search` is the span-only verb.
3. **The warning's channel on the browser target** (no stderr): the library's wasm bridge
   routes it through `log_warn`, which the host already captures — verify on the `--html`
   target in the parity gate (loft-ship skill).

## See also

- [179-scripts-in-loft](179-scripts-in-loft/README.md) — the driver; findings 002, 011, 015.
- [lib_plans/57-regex](../lib_plans/57-regex/README.md) — the legacy design (feature table,
  the library-not-syntax argument); superseded on representation.
- [formal/rewrites.md](../formal/rewrites.md) `(R-TextBorrow)`, `(R-WorkView)`,
  `(R-DecodeView)`; [formal/collections.md](../formal/collections.md) `(Slice-Value)`;
  [formal/formatting.md](../formal/formatting.md) `(F-Target)`.
- [DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md) @C141,
  [DESIGN_DECISIONS_VALUES.md](../DESIGN_DECISIONS_VALUES.md) @C139, @C80.
- [LIBRARY_AUTHORING.md](../LIBRARY_AUTHORING.md) § The library contract (rules 7, 10, 11);
  [PLACEMENT.md](../PLACEMENT.md); [COMPATIBILITY.md](../COMPATIBILITY.md).
- [STDLIB.md § Bytes and code points](../STDLIB.md).
