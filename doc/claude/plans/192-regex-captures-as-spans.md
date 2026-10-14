<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 192 — Regex captures, find-all and replace: a `Match` written so the compiler can keep it a view

Plan issue: [@PLN192](https://github.com/loft-lang/plans/issues/192) (`status:future`, `subject:libs`).  Library: [`loft-lang/loft-libs-core/regex`](https://github.com/loft-lang/loft-libs-core/tree/main/regex)
(0.4.3).  Driver: [@PLN179 finding 011](179-scripts-in-loft/findings/011-regex-has-no-capture-groups.md)
— four of the eight ports cite it, and it is why those ports read two to three times longer
than their Python originals.  Value **U** (ease of use — friction-free scripting) · effort **M**, in S
steps that each finish inside the PR cadence.

## Status (REQUIRED)

Open — design probed, nothing built.  The shipped library answers `matches` / `search` /
`split_on`; captures, `find_all` and `replace` are its README's roadmap and the register's
longest-open clarity reason.  The P0 probes below have run (both backends, the scratch
clone of the library): three claims held, one fell and rerouted the representation (§ The
invariant), and `split_on` was found to answer wrongly at a piece boundary (§ P0 d).  The
legacy design in [lib_plans/57-regex](../lib_plans/57-regex/README.md) stands on its
feature table and its "a library, not syntax" argument; this plan replaces its `Regex`
handle and decides what a `Match` holds.

## Goal (REQUIRED)

A loft script pulls a value out of a text the way Python does (`m.group(1)`, `re.findall`,
`re.sub`), with no `use` line, with every offset a byte position on a code-point boundary,
written in the one shape under which the compiler keeps every text it answers a VIEW of the
subject on `--native` — no copy where the Rust version copies none — while the interpreter
copies and is the oracle.

## The invariant (design-protocol step 1)

> **A `Match` is a record whose every text field is constructed as a slice of the subject
> and read only as a value, so that `(R-DecodeView)` (@C141) — the rule under which a
> decoded text is a span of the bytes it came from — admits it without a regex-shaped
> clause.  Its offsets are byte positions on code-point boundaries, the engine's guarantee,
> asserted at ONE native.  Nothing in the surface chooses a representation; the compiler
> proves where a view is the same value (@C125), and where the proof fails the field is an
> owned copy that answers the same.**

Under this rule a case nobody tested behaves like the tested ones for the same reason: the
library writes the plain program (a record, a slice, a field read) and the span-or-copy
decision has one home, the rewrite register.  The regex library adds no representation of
its own — which is what "the loft way to represent a read-only slice" means here: @PLN174
gave vectors a view (`OpSliceView`, a foreign span read-only under the store contract), and
@C141 gives a decoded text the same, decided by proof, never by a type the programmer holds.

**What the probes decided** (§ P0, run before this was written):

- A library's `pub fn` is emitted INLINE into the consumer's `--native` program, before and
  after its `native-auto` cdylib exists.  So the construction of a `Match` and every read of
  it sit in one compilation unit, which is the region `(R-DecodeView)` needs.  (The
  prediction was the opposite — a record copied across a cdylib ABI — and a spans-only
  surface built on that prediction was dropped with it.)
- Today that record's text field is `OpGetTextSub(subject, a, b).to_string()` followed by a
  `set_str` into the record's store: one owned copy per field, on native as interpreted.
  `(R-DecodeView)` instance 1 is BUILT, on branch `157-native-4x` (pushed, not yet on
  `main`; § P4 says where).  Its source is a byte vector and the regex subject is a `text`,
  and its view type is an ENUM, where `Match` is a struct.  So the zero-copy half of the goal
  is the clause this plan contributes (§ P4), and never something this library can deliver
  alone.
- Every offset the engine answers is a code-point boundary (`.` on `"😀x"` spans bytes
  0..4; `(?-u:\xFF)` is refused), so a span can never cut a character where `"héllo"[-4..]`
  can.

**Re-assertion sites (step 2).**  Two, both loud when omitted: ONE native answers every
span (`n_spans`; a second span-producing native is the defect), and ONE loft function builds
a `Match` from spans (`match_at`), so the slice-of-subject shape the rule admits is spelled
once.  The boundary guarantee is the Rust `regex` crate's over `&str`, probed, pinned by
cells, re-derived nowhere.

## The surface

All methods are `self: text` and so become auto-use triggers (`src/triggers.rs` derives them
from `pub fn f(self: T, …)`; `[triggers] enabled` is the library's whole declaration): a
script writes `line.capture("…")` with no `use`, as it writes `line.matches("…")` today.
A library struct's fields must be declared `pub` to be read by a consumer (measured in P0 a).

```loft
pub struct Group { pub text: text?, pub start: integer, pub end: integer }   // null: did not take part
pub struct Match {
  pub text: text,             // the whole match — subject[start..end]
  pub start: integer,         // bytes, on code-point boundaries
  pub end: integer,           // exclusive
  pub groups: vector<Group>,  // group 1 .. n; group 0 is the match itself
  pub names: vector<text>,    // "" for an unnamed group
}

pub fn capture(self: text, pattern: text) -> Match?             // first match with its groups
pub fn capture_all(self: text, pattern: text) -> vector<Match>  // every non-overlapping match
pub fn replace_on(self: text, pattern: text, template: text, limit: integer = 0) -> text
pub fn group(self: Match, i: integer) -> text?                  // self.groups[i - 1]?.text
pub fn named(self: Match, name: text) -> text?
```

```loft
// finding 011's three shapes, as they read with this surface
m = line.capture("^(\\w+)=(.+)$");
if m != null {
  key = m.group(1)?;                       // a view on native once (R-DecodeView) admits it
  value = m.group(2)?;
}
for h in header.capture_all("HTTP/(?<maj>\\d)\\.(?<min>\\d)") {
  version(h.named("maj")?, h.named("min")?);
}
out = "";                                  // re.sub with a callback: the walk idiom
last = 0;
for m in s.capture_all("\\b[a-z]+\\b") {
  out += s[last..m.start] + m.text.to_uppercase();
  last = m.end;
}
out += s[last..];
```

**Built as the rule admits it.**  `match_at` constructs every text field as a slice of its
`self` parameter — `Match { text: self[s..e], … }`, `Group { text: self[gs..ge], … }` — and
returns the record to the caller that passed `self`, which is `(R-DecodeView)`'s confinement
clause (2) verbatim: *a return hands R to a caller only where that call passed B as an
argument*.  `capture_all` builds a `vector<Match>` the same way.  A consumer that keeps a
`Match` past its subject (stores it in a longer-lived record, returns it from the frame that
owns the subject) is the rule's decline case: the field is an owned copy and the program
answers the same — the cell that proves it is in the matrix.

**Names.**  The library's rule stands: the pattern-taking twin of a stdlib text verb takes a
distinct name, because a bare method call resolves the stdlib first and the wrong one still
compiles (`@RGX-001`).  The stdlib's text methods today: `find rfind replace split
split_text contains starts_with ends_with join trim* to_*case char_slice byte_at len size
is_*`.  So `replace_on` beside `split_on`, `capture` / `capture_all` beside `search`.
`group` and `named` live on `Match`, so no future stdlib text method can take them.

**Broken patterns.**  `@RGX-003` stands for the value channel (a miss, never a fault), and
gains the @C80 channel the stdlib uses for an unguarded `÷ 0`: the first use of a pattern
that does not compile logs ONE warning naming the pattern and the parser's message.  The
cache already remembers a failed pattern as `None`; it remembers the message too.

**`search` becomes honest** (`@RGX-002`'s second half): its declared `integer` return
answers null on a miss.  It is redeclared `integer?` in the same release, which is the
@PLN102 rule for a not-found case and a signature change `api_compatible_with` records.

## Code points

- **Offsets are bytes**, the unit of `size`, `s[a..b]`, `find` and `byte_at` — never
  `len`'s character count.  Every span starts and ends on a code-point boundary because the
  engine matches `&str` by `char`: `.` is one code point, a class is Unicode (`\w` takes `é`,
  `\p{L}` the letters, `\d` the Unicode digits), `\b` is the Unicode word boundary, `(?i)`
  is simple case folding, and a pattern that could match inside a code point is an invalid
  pattern (logged as above).  Measured: § P0 c.
- **No normalisation.**  A precomposed `é` and `e` + U+0301 are different texts to the
  engine, as they are to `==`; a cell pins it with `len` beside the match.
- **A character count of a prefix** (a column, a caret) is a stdlib question, not a regex
  one: `len(line[..m.start])` copies on both backends today (loft#1728 pinned
  `len(v[a..b])`).  The one-line home is `text.char_index(byte: integer) -> integer`, the
  inverse `char_slice` lacks; listed under open questions, never built here.

## How it stays out of the way (the built-in feel), and what it must not do

| today | after |
|---|---|
| `line.matches(p)` loads the library from a bare method call | unchanged; the new verbs derive their triggers the same way |
| `search` answers null from an `integer` | `integer?` |
| a broken pattern is a silent miss | a miss, plus one warning per pattern |
| `split_on` re-slices `rest = input[pos..n]` per piece, so `^` and `\b` match again at every boundary: `"abab".split_on("^ab")` answers three pieces | rebuilt on the engine's `find_at(input, pos)`: one match, `["", "ab"]` (§ P0 d) |
| pattern literals are text; an interpolated value becomes syntax | P6 (optional): `p: Pattern = "^{name}="` through the typed-format target (`(F-Target)`, @PLN124) — literal runs are syntax, holes are escaped, a `Pattern` type trigger loads the library; a `text` pattern stays accepted |

Not done here, each a decision rather than an omission: a `Regex` handle (the cache makes a
compile step a worse API); a callback `replace` (closures do not cross a placed library,
PLACEMENT.md rule 3 — the walk idiom above is four lines); a lazy `iterator<Match>` (a
generator declines every span rule; `capture_all` answers a vector); a pure-loft engine
(57-regex phase 1, still future); a `Match` that remembers its subject (a text field holding
the whole subject is the one copy no rule removes).

## Composition matrix — Stage A

The cells, each hand-computed, `--interpret` and `--native`, graduated to
`tests/scripts/192-regex-captures.loft` and the library's `tests/worked-examples.loft`
(`@RGX-005` …):

| axis | values |
|---|---|
| subject | ASCII · multibyte before the match · multibyte inside it · empty · match at byte 0 · match at the end · an empty match (`a*` on `"b"`) |
| pattern | no group · one group · an optional group that did not take part (`null` text) · nested groups · alternation choosing a group · a named group · `(?i)` · `\p{L}` · `^` and `\b` against a second match · a broken pattern |
| count | 0 · 1 · many non-overlapping (`aa` on `"aaa"` is one) |
| holder (P4's cells) | the `Match` read in the frame that passed the subject · stored into a longer-lived vector (declines) · returned from the frame that owns the subject (declines) · the subject appended to after the capture (declines, value right) · a generator |
| limit | `replace_on` with 0 · 1 · more than the matches |

`python3 scripts/matrix_axes.py file <guard>` says which values the cells reach.

## Sub-arcs

| Item | Source | Verify (goes RED if the phase is wrong) | Status |
|---|---|---|---|
| **P0** probe the four claims (XS) | § P0 | a prediction written before each run | **Done** — a, b held in the value channel; a's representation claim fell; d found the defect |
| **P1** `Group`, `Match`, `capture`, `capture_all`, `group`, `named`; ONE native `n_spans` (S, library) | § The surface | the matrix, both backends; the trigger test with no `use`; `tests/dump_ignored_tests` re-twinned over `capture` and finding 011 re-measured (`scripts/script_recheck.sh`) — the port's lines drop or the phase failed; the twin's performance column records the copy P4 will remove | Open |
| **P2** `replace_on` with `$1` / `${name}` templates and `limit`; `split_on` on `find_at` (S, library) | § The surface | the Python twin of a `re.sub` template; the `^` / `\b` cells of P0 d, red today, green after | Open |
| **P3** `(R-SliceBorrow)`: a text local bound to a slice of an unwritten text is a `&str` (S, compiler, native) | § P3 | emission pins in the shape of `tests/text_borrow.rs`; cells under `LOFT_NO_SLICE_BORROW=1`, `LOFT_HOIST_VERIFY=1`, `LOFT_POISON=1`; `make rewrite-census` gains the row; a planted admission of a written subject reads stale bytes under the verify switch | Open |
| **P4** `(R-DecodeView)`'s text-source clause: B may be a `text` parameter; a `vector<R>` of admitted records is a holder (M, compiler, native) | § P4 | `LOFT_TRACE_DECODE_VIEW` names no decline on `capture`'s twin and its emission holds no `to_string` / `set_str` for a text field; the holder-axis cells; `make perf-check` on the two ports' rows | Open — the rule's instance 1 is built on `157-native-4x` (§ P4 names the commits, files and the three things this clause must decide); this plan contributes the clause and the regex corpus as its rows |
| **P5** the broken-pattern warning (XS, library) | § Broken patterns | a cell asserting the one warning on stderr and its absence on the second use; `@RGX-003` re-measured | Open |
| **P6** `Pattern` typed-format target (S, library, optional) | § built-in feel | a hole holding `.` matches only a literal dot; the type trigger loads the library with no `use`; `(F-Target-Pos)`: an argument position does not target | Open |
| **P7** `search -> integer?`, the `regex_` spellings removed per COMPATIBILITY.md, `api_compatible_with` bumped (XS, library) | § `search` | the library's own CI under `LOFT_DENY_WARNINGS=1`; `make revalidate-libs` for the consumers of `search` | Open |

P1 → P2 → P5 → P7 ship as one library release (0.5.0), each a commit the testbed gates
alone (LIBRARY_AUTHORING.md § The testbed).  P3 is independent and pays for every script
that slices (finding 002's tokenisers, every `find` → `s[a..b]`).  P4 waits on its rule.  P6
waits for a port that interpolates into a pattern.

### P0 — the probes, the predictions, the results

Throwaways under the scratchpad, `--interpret`, `--native` and `--native-emit`, the library
from a clone by `--lib`, the prediction written before each ran.

- **(a) A record with a text field answered by a dependency's `pub fn` crosses the cdylib
  ABI as a copy.**  A path library `rowlib` answering `Row { t: self[1..3], n: 1 }`, the
  consumer emitted before and after a `--native` run built `rowlib/native-auto/`.
  *Predicted:* a `set_text` across an ABI call.  *Measured:* `t_4text_head` is emitted in
  the consumer's own Rust both times, called directly from `n_main`; the field is
  `OpGetTextSub(var_self, 1, 3).to_string()` + `set_str` — the copy is real, the ABI is not.
  **Fell**: the representation question is the rewrite register's, not the library's.
- **(b) `t = line[a..b]` bound to a local is a `String` today on native.**  *Predicted and
  measured:* `let mut var_t: String = OpGetTextSub(var_line, 2, 5).to_string();` — P3's gap,
  one `.to_string()` the proof removes.
- **(c) Every offset the engine answers is a code-point boundary.**  *Predicted and measured,
  both backends:* `"héllo wörld".search("w.r")` → 7; `"😀x".search(".")` → 0 and `"x"` at 4;
  `"abc".search("(?-u:\xFF)")` → null; `"é".matches("^.$")` → true.
- **(d) `split_on` loses `^` and `\b` context at a piece boundary.**  *Predicted and
  measured, both backends:* `"abab".split_on("^ab")` → `[][][]` and `"abab".split_on("\\bab")`
  → `[][][]`, where one match (`[][ab]`) is correct.  A library defect, fixed in P2 with the
  cells as its receipt.

### P3 — `(R-SliceBorrow)`, the small compiler step

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

It is `(R-WorkView)` with the work buffer replaced by a parameter or local, under
`(R-TextBorrow)`'s value-only reading — the same five conditions, so the analysis exists
(`hoist::borrowed_text_walks` decides (1)–(3) for a walk).  Its counterpart for a `match`
statement — a text a pattern binds, copied into a `String` — is what is left on pluginabi
`check_request` after `(R-DecodeView)` (its key bindings, priced −1.5 % of that row in
`157-native-4x`'s `bench/portal/analysis/worklist.md` § 0); `arguments/parse`'s text copies
are about 10 % of its row.  Nothing for either is built yet.  Decided in the formal register
as a rule with its own `@FR-` tag, its code sites citing it, `make rule-coverage` counting it.

### P4 — the clause this plan owes `(R-DecodeView)`

The rule's source B is "a byte vector"; a regex subject is a `text`.  The clause: *B may be a
`text` parameter or local, and a span of it is the subject's bytes at `[a, b)` — already
valid UTF-8 on a code-point boundary, so condition (5) holds by construction and no
`text_from_bytes` test is paid.*  And a holder clause: *a `vector<R>` whose every element is
an admitted R, built in the confined region, is itself a holder of the region.*  Both are
measured on `capture` and `capture_all`; neither changes the invariant the rule already
states (a span answers what the owned copy would, because B outlives it unchanged).

**Where instance 1 is.**  Branch `157-native-4x` (pushed to `origin`, not yet on `main`):

- Commits: `f00897790` (the build), `41b419bf8` (its docs), `cbd50ff8d` (the span helpers
  read the frame once a call), `aed8e7da4` (why a library function's `native` mark does not
  keep it out of a region); the row's price and result `1185b06a5`, `d9252256a`.
- Admission and planning: `src/generation/decode_view.rs` (`view_types`, `root_frame`,
  `region_of`, `plan`).  Emission, the `<f>__dv` twins: `src/generation/decode_view_emit.rs`.
- Runtime: `src/codegen_runtime.rs`, the `dv_*` helpers (`DvFrame`, `dv_span_text`,
  `dv_text`, `dv_put_span`, `dv_clear`), and the `Stores::dv_frame` field in
  `src/database/mod.rs`.
- Rule text: `doc/claude/formal/rewrites.md` § `(R-DecodeView)`; switches
  `LOFT_NO_DECODE_VIEW` and `LOFT_TRACE_DECODE_VIEW` (`doc/claude/NATIVE_SWITCHES.md`).
- Guard `tests/scripts/a-decoded-text-is-read-where-it-lies.loft`; pin `tests/decode_view.rs`
  (the expected roots and twins).
- Measured on pluginabi `check_request`: 7.55× → 5.07× its Rust twin
  (`bench/portal/analysis/over-9x.md`, `bench/portal/analysis/worklist.md` § 0).
- Joining the two branches: `157-native-4x` also extends `src/same_read.rs` with
  `(R-SameRead)`'s get-or-insert clause (`057baaaed`), a file this branch also carries.

**What the build decided for this clause:**

1. **A view type is an ENUM today.**  The span word — `i64::MIN | (lo + 1) << 32 | n` — is
   kept in the record's last 8 bytes, and only an enum of 16 bytes or more has those spare,
   padded to its largest variant (`view_types`).  `Match` and `Group` are structs and have
   no spare word, so the clause must decide where a struct keeps its span: a design
   question, not only a new source.
2. **One span per record.**  One spare word, so a variant with two text fields declines.
   `Match` and `Group`, one text field each, fit.
3. **The frame is a byte-vector parameter.**  A root is a function whose single byte-vector
   parameter, not passed by `&`, is the frame; it sets `stores.dv_frame` for the call
   through the `DvFrame` guard (`root_frame`).  A `text` subject is a new kind of frame, and
   it may skip `dv_span_text`'s UTF-8 test, as stated above.
4. **A vector of view records is already a holder.**  An element of a view type works (guard
   cell c15, `mixed`), so `capture_all`'s `vector<Match>` should take the existing path
   once `Match` is a view type.
5. **What makes a root decline:** returning a decoded value from the root; a span variant's
   record created on its own; a native function, a function reference, `yield` or `par`
   reached by a decoded value; any read of a byte-string field.  A library's `pub fn` is
   emitted inline into the consumer (§ P0 a), which is why the region's `loft_fn` test does
   not look at `def.native`.

## Open design questions

1. **`text.char_index(byte) -> integer`** — the inverse of `char_slice`, so a span's column
   is one call and no copy.  Stdlib, one line, its own guard; recommended, filed as its own
   XS issue rather than widened into this plan.
2. **Does an installed, prebuilt library inline the same way a `--lib` path library does?**
   P0 a measured the path form; the registry form is the same package directory, so the
   prediction is yes — one cell on the library's testbed before P1 closes.
3. **The warning's channel on the browser target** (no stderr): the library's wasm bridge
   routes it through `log_warn`, which the host already captures — verified on the `--html`
   target in the parity gate (loft-ship skill).

## See also

- [179-scripts-in-loft](179-scripts-in-loft/README.md) — the driver; findings 002, 011, 015.
- [lib_plans/57-regex](../lib_plans/57-regex/README.md) — the legacy design (feature table,
  the library-not-syntax argument).
- [formal/rewrites.md](../formal/rewrites.md) `(R-DecodeView)`, `(R-TextBorrow)`,
  `(R-WorkView)`; [formal/collections.md](../formal/collections.md) `(Slice-Value)`;
  [formal/formatting.md](../formal/formatting.md) `(F-Target)`.
- [DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md) @C141, @C125;
  [DESIGN_DECISIONS_VALUES.md](../DESIGN_DECISIONS_VALUES.md) @C139;
  [DESIGN_DECISIONS_FAILURE.md](../DESIGN_DECISIONS_FAILURE.md) @C80;
  [174-foreign-stores](174-foreign-stores/README.md).
- [LIBRARY_AUTHORING.md](../LIBRARY_AUTHORING.md) § The library contract (rules 7, 10, 11);
  [PLACEMENT.md](../PLACEMENT.md); [COMPATIBILITY.md](../COMPATIBILITY.md);
  [STDLIB.md § Bytes and code points](../STDLIB.md).
