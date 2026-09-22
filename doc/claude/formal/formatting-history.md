# formal/formatting-history.md — the deviation register for [formatting.md](formatting.md)

> **The rules are next door.**  [formatting.md](formatting.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0**. `D-fmt-2` and `D-fmt-3` were closed on 2026-08-29 by the composition fix that
`formatting.md` now states as a rule; this file carried them as open until 2026-09-01 because
the commit that split it out of the rules doc replayed the list from before that fix.
`D-fmt-4` was opened and closed the same day by the `@FR-E-NullArg` walk, and `D-fmt-5` on
2026-09-01 by the reference review of the Formatting chapter.  `D-fmt-6` opened and closed on
2026-09-22 (below).

⚠ **This doc read `OPEN: 0` for its whole life, and the walk that first asked found four
defects.** The line was never a measurement: it said *"a rules doc adds no code deviation"*,
which is a claim about the doc's GENRE rather than about the code, so no oracle stood under it.
The four are `D-fmt-1`, `D-fmt-2`, `D-fmt-3` and `D-fmt-4`; what they have in common is that a
neighbouring spelling of each was already correct, which is what a differential oracle is
blindest to — both backends agreed, and agreed on the wrong answer.

⚠ **And the walk after it found eight more (`D-fmt-5`), in the same subject, three days
later.** Both walks started from `@FR-F-Spec`; what the second one added was a CHAPTER to
read against, and every one of the eight lives in a sentence the chapter states and no cell
ran — the pad on a float, the case of `X`, a width before `b`. The lesson is not that the
first walk was careless: a rule walk asks *does the code do what the rule says*, and a
reference walk asks *does the code do what we PROMISED*, which is a larger set because the
prose promises more than the rules state.

### D-fmt-6 — OPENED AND CLOSED (2026-09-22): a hole that opened with whitespace was refused

`(F-Interp)` says a hole holds an expression parsed at full language level, where whitespace
around it means nothing, and LOFT.md writes `"{ build("p{n}q") }"` in its own reference.  Both
backends refused every such hole as *"Formatter error"*, followed by a cascade.  A hole's first
token is read in the string's mode, where whitespace is a token because a spec's fill may be a
space; `parse_string` switched the hole to code only after that token, so a space after the `{`
reached the expression parser.  Closed by skipping whitespace at that switch (`Lexer::whitespace`,
which nothing called before).  A space after the `:` is still the fill.  Found while closing
D-tup-15 (loft#1590), whose look-ahead reads holes the same way.  Guard
`tests/scripts/a-format-hole-may-open-with-whitespace.loft`.

### D-fmt-1 — OPENED AND CLOSED (2026-08-29): four ways a spec did not reach its renderer

Found by walking `@FR-F-Spec`; all four fixed in the same pass, guards in
`tests/scripts/a-format-spec-is-honoured-not-dropped.loft`,
`a-json-spec-spelling-is-one-decision.loft` and
`a-format-spec-the-renderer-cannot-execute-is-refused.loft`.

1. **A tagged null ignored its alignment.** `{a / b:<12}` right-aligned `null(/0)` while
   `{n:<12}` left-aligned a bare `null`, so the alignment a hole got depended on whether its
   null carried a fault cause. Six lines existed in THREE copies — `ops::format_long_with_tag`
   and the interpreter's `State::format_int` / `format_stack_int` — each passing a literal `1`
   where `format_long`'s bare-null path resolves `dir`. The two interpreter copies now call the
   shared one, so the question has a single place to be answered.
2. **`{p:J}` and `{p:json}` were not `{p:j}`.** Whether a WIDTH expression starts here and which
   radix a letter names are one question, and they were answered from two lists: the radix reader
   lower-cased and accepted `json`, the skip list named only `j`. The width expression therefore
   ate the letter — an "Unknown variable 'json'" where no such variable exists, and where one
   does, that variable's value silently taken as the width with the rendering falling back to the
   compact loft form. `Parser::radix_for` is now the one home both readings consult.
3. **A width that is not a number was accepted.** `{n:0>5}` parses as `0 > 5`; the BOOLEAN
   reached the width, rendering with no padding on `--interpret` and reaching rustc as
   `E0308 expected i64, found bool` on `--native`. This is the residual of the fix that made
   `string_states` order-independent — its own note says an out-of-order flag "was simply left in
   the stream for the WIDTH expression to find", and a `0` fill is left there for the same reason,
   because the fill branch can only claim a token and a digit lexes as an Integer. F-Spec-Fill
   above now says why a digit cannot be a fill; the width slot now requires an integer.
4. **`{n:e}` and `{n:j}` aborted the interpreter.** `ops::format_long` implements four radixes
   and ends in `panic!("Unknown radix")`; the spec reader answers two more. An ordinary source
   program reached that panic. Both are refused at the type dispatch now.

### D-fmt-4 — OPENED AND CLOSED (2026-08-29): a null character carried a fault cause, on one backend

- Was: a null `character` in a format hole rendered `null(<tag>)` when the fault tag was armed
  — `"hi"[9]` read `null(oob)` — where `(F-Render)` says a null character renders as NOTHING,
  and says why: iterating text past its end must append no garbage. Only the INTERPRETER did
  it (`State::append_character`); `--native` rendered nothing, so the two backends disagreed
  on an ordinary text overrun and `@FR-D-op-1` makes that a bug in whichever one disobeys.
- The intent was good and is worth restating: it showed *what* produced the missing character
  instead of an empty space. But a diagnostic that exists on one backend is not a language
  feature, and no test pinned it (the four `fmt43_*` cases in `tests/runtime_warnings.rs` are
  all INTEGER holes, and all still pass).
- Fixed toward the rule and toward native: `append_character` drops the tag rather than
  rendering it. The tag is still TAKEN, so it cannot leak into a later hole of the same
  string. Guard `tests/scripts/a-fault-tag-names-the-fault-that-happened.loft`.
- The cause itself is now written by the op that FAULTS (`Stores::note_format_fault`) rather
  than armed from the op's shape, so `null(<reason>)` names what happened — loft#1169. A hole
  may hold several fault-prone ops while only the outermost is armed, so a peer that inherits
  a null LEAVES the tag alone: the cause travels with the null from wherever it was born.
- **Reversing this is a change to `(F-Render)`, not to the op.** If a character hole should
  carry its fault cause, the rule's character row says so and BOTH backends implement it.

### D-fmt-5 — OPENED AND CLOSED (2026-09-01): the spec halves that were only asked of an integer

Found by the reference review of chapter 30 (Formatting), which is the first time the
chapter's own subject was swept rather than read. Every entry has the same shape as
`D-fmt-1`: a neighbouring spelling was already correct, both backends agreed, and the
agreement was on the wrong answer. Guards in
`tests/scripts/a-format-pad-reaches-the-float-renderers.loft`,
`a-width-may-precede-any-base-letter.loft`, the eight new cells in
`a-format-spec-the-renderer-cannot-execute-is-refused.loft`, and — for the one property an
assertion cannot carry — `tests/format_width_is_bounded.rs`.

1. **A width at or below zero allocated without bound.** `format_text` counted its pad
   characters into a `usize`, so `-1` became 18_446_744_073_709_551_615 of them.
   `println("{-1:0}")` — a one-line program with nothing unusual in it — asked the allocator
   for the whole field in one call and the process was OOM-killed. No renderer spells a
   negative width; they reach one by subtracting a sign or a `0x` marker they emitted first,
   from a width that may be zero. **A time bound does not bound memory**: under
   `LOFT_TIMEOUT` this reads as a hang, and the 2 GiB test ceiling is a STORE budget that a
   Rust `String` is outside of. Fixed at the one place that pads (`width.max(0)`), which
   covers every caller that subtracts.
2. **A precision reached renderers that have no fractional digits.** Only the `float` and
   `single` arms call `append_data_fp`, which is what splits a dotted `W.P`; everywhere else
   the `f64` stayed in a slot the opcode reads as an i64 width — ~4.6e18 pad characters, so
   `{n:8.2}` was (1) above, and `--native` handed rustc `E0308 expected i64, found f64` about
   loft's internals. The bare `{n:.4}` was quieter and worse: it left the precision in the
   width slot and rendered a four-wide field. Refused now, per type.
3. **A radix reached renderers with no arm for it.** `x`, `b`, `o` and `e` were dropped in
   silence on a float, a single, a character and a vector. The L9 escalation had always
   stated the rule — *"a specifier that can never have any effect on the value type is always
   a bug"* — and asked it of `text` and `boolean` only. The refusal is now stated as *which
   radixes does this renderer have an arm for*, which every type answers.
4. **`X` rendered lower-case hex.** `x` and `X` both mapped to radix 16 and the renderer
   wrote `{val:x}` — so the spelling the compiler's own *"use `x`, `X`, `b`, `o` or `d`"*
   diagnostic advertises answered `ff`, and nothing said the case had been ignored. The spec's
   radix field is already a render MODE wearing a radix's name (`-1` is JSON, `1` is
   scientific), so upper-case hex took a value of its own rather than a second argument on
   four opcodes.
5. **A float and a single dropped the pad token and the fill character.** The four float
   opcodes had no slot for the token, so `ops::format_float` filled with a hard-coded space:
   `{f:08.2}` padded with spaces and `{f:*^11}` ignored its fill, both silently, while the
   same specs worked one type over.
6. **A zero pad was inserted INTO a radix marker.** `{255:#06x}` rendered `000xff`, which is
   not a spelling of 255 in any base and cannot be pasted back into a program. The decimal arm
   carried the sign half of this alone; `format_prefixed` is now the one home for both.
7. **A width could not precede `b` or `o`.** The lexer took `x` into a numeric literal only
   directly after a leading `0` but took `b` and `o` anywhere in a digit run, so `{10:6b}`
   scanned `6b`, failed to parse it, and reported *"Problem parsing number"* — a diagnostic
   about a literal, pointing at a format spec, for a program with no literal wrong in it.
   Every width-and-binary and width-and-octal spec was unspellable while the hex twin one
   character away worked.
8. **A null integer took a zero pad and a null float did not.** `{n:08}` rendered `0000null`
   on the integer path, which reads as a numeric value and is a rendering of nothing. F-Spec
   already draws the line for the sign (*"`null` … takes none"*); the pad is the same
   question, and the two numeric renderers answered it differently. One home now, cited from
   all three.

### D-fmt-2 — CLOSED (2026-08-29, loft#1165): a `character` hole drops its whole spec

`{c:>5}` renders `x`. `Type::Character` emits `OpAppendCharacter`, which takes the accumulator
and the value and has nowhere to put width, alignment or fill. Violates F-Spec-Exec: it neither
honours the spec nor refuses it. Closed by rendering into a scratch text with the padding removed and then formatting THAT
text with it, which reaches every such type at once instead of widening one op signature per
family. The decision F-Render forces is written into the rules: a null character renders as
nothing, so `{nc:>3}` is a full field of pad characters.

### D-fmt-3 — CLOSED (2026-08-29, loft#1166): a vector/struct hole drops width and alignment

`{v:>12}` renders `[1,2]`. The record arms pass `OutputState::db_format()`, which is two bits
(`#` and the JSON radix); width, alignment and fill are not passed at all. Violates F-Spec-Exec
for the same reason and wants `OpFormatDatabase`'s signature widened on both backends.

- **Conformance is differential** — formatting is enforced across the two backends by the @PLN89
  oracle (D-op-1) plus the dedicated `tests/scripts/14-formatting.loft` / `tests/docs/30-formatting.loft`
  suites, which pin the exact rendered strings and run on both backends. A divergence in a rendered
  value, a null form, a struct/vector Display, or a format-spec result is caught there.
- **Rendering `null` as the word `null` is the contract, not a gap** — the in-band sentinels
  (`i64::MIN`, `NaN`, `STRING_NULL`, `0xFF`) are an operational decision (operational.md E-Null);
  their uniform `"null"` text (character-0 excepted) is intended, so it is a rule here, not a
  deviation.

## Carried by formatting.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [formatting.md](formatting.md) now states only what is open.

### the two days this register read OPEN: 2 after D-fmt-2/3 closed

⚠ This section read **OPEN: 2** for two days after both entries were closed. `D-fmt-2` and
`D-fmt-3` were fixed by the commit that wrote the "a spec tunes ⟦v⟧ … for EVERY type"
paragraph into the rules above, and loft#1165 / loft#1166 were closed with it — but the
commit that split this register out of the rules doc replayed the deviation list from before
that fix, and a clean prose merge keeps both halves. **An `OPEN: n` is a claim to
re-measure, and the cheapest way is to run the entry's own repro**: `{c:>5}` and `{v:>12}`
both padded, which is what a deviation says they do not.

### the status line formal/README.md's area table carried until 2026-09-04

**rules written (2026-07-05), 0 own** — arbitrary-expression interpolation, `{{`/`}}` escape, per-type render (null → `"null"`, char-0 → nothing), the width/align/pad/precision/radix specs, and fault-safe interpolation (`{a/b}` → `null(/0)`, never a halt); one rendering sink → backend parity; plus `F-Target` (@PLN124, 2026-08-09) — the same template builds a VALUE when checked against a type defining `lit`/`hole_*`; conformance via the oracle

