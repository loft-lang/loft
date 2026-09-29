<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Standard library reference — the record

Limitations [STDLIB.md](STDLIB.md) once stated and the library no longer has, oldest
first.  The page describes the library as it is now; this file keeps the record, each
entry naming the issue that closed and the guard that asserts the current behaviour.  A
record doc: its dates are the point (DOC_CONTRACT rule 32).  `rule_tags.py claims --issues`
(the `stale-claims` nightly, @PLN176) names a limitation still on the page whose issue has
closed, and the entry it turns into belongs here.

## 2026-08-03 — a text carrying a NUL iterated short (loft#755)

`for c in s` stopped at an embedded NUL while `len`, indexing and `byte_at` all saw the
whole text.  Iteration yields every code point, the NUL position reading as `null`.
Guard: `tests/scripts/text-nul-iteration-755.loft`.

## 2026-08-03 — `text_from_bytes` and `byte_at` were reported missing for two releases (loft#748)

Both existed; the generated reference filed them under Environment, so a keyword sweep of
the Text page came back empty and was read as a language gap.  The lesson kept from it:
check an instrument against something it should find before trusting it to report an
absence — `grep default/*.loft` answers in one call.  Guard: `tests/scripts/748-chr-code-point.loft`.

## 2026-08-07 — a text-keyed `spatial` was accepted and then answered null (loft#799)

`spatial<Word[w]>` with a `text` key compiled and every point lookup answered `null`.  It
is refused at the declaration, naming `trie<Word[w]>`.  Guard: `tests/parse_errors.rs`.

## 2026-08-07 — `store_bind_lazy` answered `true` for a kind it could not serve (loft#802)

A `sorted` / `index` / `spatial` bound to a paged IMAGE was accepted and then answered
`null` at every lookup.  The binding is refused there, and `false` is worth checking.
Guard: `tests/scripts/802-lazy-refusal-visible.loft`.

## 2026-08-07 — a spatial box query answered points outside the box (loft#800)

`xs[(x1,y1)..(x2,y2)]` answered the raw Z-order interval between the corners, a strict
superset.  It answers exactly what is inside the box.  Guard:
`tests/scripts/800-spatial-box-containment.loft`.

## 2026-08-10 — `content()` answered `""` for a non-UTF-8 file (loft#829)

Indistinguishable from an empty file, so a *write bytes, read them back, compare* gate
passed vacuously on binary data with both sides `""`.  `content()` is nullable: a missing
file, a directory and non-UTF-8 bytes are each `null`.  Guard: `tests/binary_io_matrix.rs`.

## 2026-08-17 — `reduce` with a collection accumulator read back empty (loft#956)

The refusal of a collection accumulator asked the INIT what type it was; a bare `[]` had
none and walked past it, and the fold read back empty on `--interpret` and was an internal
compiler error on `--native`.  The accumulator's type is read off the fold's first
parameter, so `[]` and a typed init are the same fold.  Guard:
`tests/scripts/956-reduce-untyped-accumulator.loft`.

## 2026-08-20 — the open spatial walk was the Morton tail (loft#1002)

`xs[(x,y)..:n]` answered only records at or past the query in Z-order, so it under-delivered
near the end of the curve.  It answers `n` from any origin.  Guard:
`tests/scripts/48b-spatial-slice.loft`.

## 2026-09-28 — four Binary Files rows named routines that do not exist

`little_endian(self: File)`, `big_endian(self: File)`, `write_bin(self: File, v)` and
`read(self: File, v)` sat in the signature table with no declaration in any `default/*.loft`;
the real forms are `f#format = LittleEndian` / `BigEndian`, `f += value` and `f#read as T`.
Found by `rule_tags.py sections`, which resolves every signature row against the stdlib
source, on the day it was written.  Guard: `tests/reference/skill-files.loft`.



## 2026-09-29 — the hash growth trigger was 0.75 until 2026-09-21

(Until 2026-09-21 the trigger was 0.75: tables were
half this size, a miss walked five buckets where it now walks one, and a lookup at this
size was 10 % slower on a hit and 35 % on a miss — `bench/portal/analysis/keyed.md`.)

## 2026-09-29 — the JSON "Legacy text-based API (transitional)" heading

(This section was headed
"Legacy text-based API (transitional)" and announced a withdrawal in 0.9.0 — that was
written on 2026-04-14 for the 0.8.4 RC and no roadmap row, decision record or
`#superseded` marker ever scheduled it.  What P54 actually withdrew was `json_items` and
its peers, which are gone.)

## Open work rows that shipped or were deferred

Moved out of [STDLIB.md § Open work](STDLIB.md#open-work); shipped 2026-05-18 unless the row says otherwise.

| Item | Where it bit | Shape | Effort |
|---|---|---|---|
| ~~Path helpers — stdlib `path` module~~ — shipped 2026-05-18 as text methods | scan.loft, viewer, lib/markdown each rolled their own `dir_of` / `basename` / `resolve_relative` | Pure-loft as four text methods in `default/03_text.loft`: `p.dir()`, `p.basename()`, `p.join(other)`, `p.resolve(target)`.  Module-prefix style (`path::dir(p)`) couldn't ship — loft uses self-type method dispatch, not module namespaces.  `file().path` `./<name>` normalisation deferred to a follow-up. | **Shipped (@PLN42 phase 10.9)** |
| ~~`text.split(text)`~~ — shipped 2026-05-18 as `text.split_text(text)` | scan.loft's link extractor walks char-by-char to find `](` (only `text.split(char)` exists today) | Renamed from `split` to `split_text` because loft doesn't allow fn overloading by non-self parameter type — `split(text, character)` and `split(text, text)` collide on the name `split`.  Underlying overloading limitation deserves its own follow-up (file when a second consumer hits it). | **Shipped (@PLN42 phase 10.4)** |
| ~~`text.starts_with_at(pos, prefix)`~~ — shipped 2026-05-18 | scan.loft's @PLAN matcher does `line[i+1]=='P' && line[i+2]=='L' && …` instead of `line.starts_with_at(i, "PLAN")` | Sugar over the existing slice + comparison.  Pure-loft body in `default/03_text.loft`; works in both backends. | **Shipped (@PLN42 phase 10.5)** |
| ~~`hash.contains(key) -> boolean`~~ — **deferred (not XS)** | scan.loft uses `vector<text>` + linear `set_contains` for valid_pids/valid_plans because `hash<T[K]>` isn't ergonomic as a "set of text" | The XS pitch assumed a simple sugar method.  Reality: `hash<T[K]>` keys are typed per-instance, so a generic `contains()` needs parser-level typed-dispatch — M+ work, not XS.  Use `h[key] != null` or `if h[key] { … }` idiom (established pattern in `tests/scripts/32-collections-regressions.loft`).  The deeper "set of text without wrapper struct" gap is a bigger language feature; defer until a second consumer asks for the sugar. | **Wontfix unless a 2nd consumer demands it** (@PLN42 phase 10.6 deferral) |
| ~~`text::escape_html(s)`~~ — shipped 2026-05-18 | viewer's main.loft rolled its own `escape(s)` for HTML output | Pure-loft.  Escapes the standard 5 entities (`&`, `<`, `>`, `"`, `'`) safely for both element bodies and attribute values.  **Drained out of the default stdlib into `lib/html/` (lib_plans/12 Phase 3.6, 2026-05-27) — now opt-in via `use html;`.** | **Shipped (@PLN42 phase 10.7); moved to `lib/html`** |

## 2026-09-29 — reflection over a nullable field skipped it

(Nullable fields used to be
  skipped entirely — the loop simply ran fewer times than the struct has fields,
  including for nullable fields holding real values.  Fixed; guarded by
  `tests/scripts/pln23-field-iter-nullable.loft`.)

## 2026-09-29 — a lexical filter refused `..` in a relative path (loft#712)

That is a change (loft#712). A lexical filter used to refuse any relative path
containing `..`, and reported the refusal as a **null size** — indistinguishable
from a missing or empty file, so a reader doing `if f#size < HEADER` turned it
into "the file is truncated" and reported a *data* error for what was a *path*
decision. It was not containment either: the same bytes by absolute path were
served, and a `..` that normalised back inside the root was refused too. loft has
no filesystem sandbox — admission is decided at load time and carries no runtime
checks ([SANDBOX.md](SANDBOX.md)) — so the resolved path is the whole answer and
the filesystem gives it.
