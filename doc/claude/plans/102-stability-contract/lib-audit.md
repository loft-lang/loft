<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Phase 2 lib-side pre-freeze audit (the worklist)

> **Status: prep drafted 2026-07-10.** This is the triaged worklist for the **lib side of
> the pre-freeze audit** ([COMPATIBILITY.md § Before the flip](../../COMPATIBILITY.md)) — the
> dedicated Phase 2 pass on the stdlib surface, which freezes forever at contract 1. It exists
> so Phase 2 opens against a concrete list, not a blank page. Sourced from a 4-agent survey of
> `default/*.loft` + `STDLIB.md` + `INCONSISTENCIES.md`, calibrated by CODE.md's naming rules.
>
> **How to use it.** Each item is a decision: **fix now** (while contract 0 allows), or
> **consciously accept** (record a [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) entry + a
> golden test so the freeze is a decision, not an oversight). **Severity: High** = a silent
> wrong result a program cannot observe → *must* be fixed or explicitly accepted before the
> flip. Medium = surprising-but-safe. Low = cosmetic. Nothing here is committed; this is the
> agenda for the Phase 2 review with the owner. The tables below carry only a *terse* fix column;
> each item is expanded to the full working format (below) when it is actually worked.

### The disposition — lean toward improving, not toward freezing what we have

The absolute promise is about *after* the freeze. *Before* it, the mandate is the opposite of
conservative: **actively improve.** Freezing an illogical choice does not just cost us — it
**pushes that illogical choice onto everyone who ever touches loft, forever.** Measured against
that, the conversion cost of a change now is small: we pay it once, on a corpus we control, to
spare every future user a permanent wart.

So the two dispositions are not symmetric. **"Consciously accept" is reserved for choices that
are genuinely *deliberate* and defensible** (a distinct `character` scalar; the spreadsheet
div-by-zero model) — it is **not** a way to dodge the conversion work. When an item is simply
*wrong or illogical*, the default is **fix it now**, and the size of its conversion set is a
cost to weigh, not a veto. The willingness to improve is exactly what this window is for; once
it closes it never reopens.

### Working format for each item — alternatives + the conversion set

Because every choice here is **permanent**, no item is a snap fix. Each is worked as a small
design decision and must carry two things the terse tables above cannot:

1. **The alternatives, well presented.** State the **current** behavior, then **each viable
   alternative** with its trade-offs — not just one "suggested fix." The permanent choice is
   made *between laid-out options*, deliberately, with the owner. A one-line "fix: rename X" is a
   prompt for that presentation, not the decision.
2. **The conversion set.** A change to the stdlib is not free even at contract 0: every existing
   program on the *current* API — the `tests/scripts/` + `tests/docs/` corpus, `tests/*.rs`
   `code!` cases, `lib/*` consumers, the examples, and STDLIB.md's snippets — must be **converted**
   to the new form as part of the change. So each worked item **enumerates the programs it
   touches** and the conversion is done with it. This is why the audit runs *before* the freeze
   and *before* the ecosystem grows — the conversion set only ever gets bigger. (It is also why a
   change made under a *golden-behavior corpus* is safe: the corpus diff shows exactly which
   programs the change moves, so nothing converts silently.)

**Worked example (the format, on H2 — `len(text)`):**

> **Current.** `len(text)` returns **bytes**; `size(text)` returns **characters**; `for c in s`
> and `s[i]` iterate/index by **byte**. On non-ASCII this silently miscounts.
>
> **Alternatives.**
> - **A — swap the meanings:** `len(text)` → characters (matches vector's "natural count" and
>   every mainstream language), add `byte_len(text)` for bytes; keep `size` as a byte alias or
>   retire it. *Best learnability; largest conversion set (every current `len`-on-text call that
>   assumed bytes flips meaning).*
> - **B — keep `len`=bytes, rename the char form:** make `size`→`char_len`, leave `len`=bytes.
>   *Smallest conversion set; but keeps the surprising `len`=bytes forever.*
> - **C — freeze as-is, add a safe char-indexed accessor** (`char_at`, `char_count`) and a loud
>   DESIGN_DECISIONS note. *Zero conversion; keeps the trap, only softens it.*
>
> **Conversion set (for A).** grep `\blen\(` / `.len()` on text-typed receivers across
> `default/*.loft`, `tests/scripts/`, `tests/docs/`, `lib/*`, STDLIB.md; each ASCII-only site is
> a no-op, each non-ASCII site is a real fix. Estimate before choosing — the size of the set is
> part of the decision.
>
> **Recommendation.** A (learnability wins for a permanent surface), *if* the conversion set is
> tractable; else B. Decide with the owner.

---

## The keystone: the null-sentinel model is the deepest freeze risk (decide first)

The single largest source of **High / silent-wrong** findings is not a naming wart — it is
that **`null` is encoded as an in-band sentinel value** (`i64::MIN` for `integer`, `NaN` for
`float`/`single`). Freezing this model freezes its collisions. It is one decision with many
faces, so it heads the list:

| Face | What breaks, silently | Ref |
|---|---|---|
| `1 << 63`, `&`/`\|`/`^` landing on `0x8000_0000_0000_0000` → reads as **null** | bit/flag/mask/hash code silently corrupts; the value `-9223372036854775808` is unrepresentable | `ops.rs` shift/bitwise; `01_code.loft:132` |
| **integer overflow** (`i64::MAX + 1`) → silent `i64::MIN` = null, **no diagnostic** — while int div-by-zero *does* raise | two "uncomputable" paths, one tagged one silent | `ops.rs:42,225`; `01_code.loft:173` |
| `null == null` is **true for integer** (raw `==`) but **false for float** (NaN short-circuit) | generic/ported code sees type-dependent null identity | `01_code.loft:199` vs `:344,548` |
| `find`/`rfind` return the `i64::MIN` sentinel but are typed **`-> integer`** (not nullable) | `s[0..s.find(x)]` compiles and faults/garbages when absent | `03_text.loft:44,49` |
| `min_of`/`max_of` return **null on empty** but are documented/typed `-> integer` | a program treats the result as non-null and gets null | `01_code.loft:1532,1539`; `STDLIB.md:212` |
| no `INT_MIN` constant (the true min is `i64::MIN+1`, not exposed); `u32` max is `2³²−2`; nullable `u8` loses `255` | consumers hardcode wrong magic bounds | `01_code.loft:38,901` |

**This is the one item that most deserves a real decision rather than a freeze-as-is** — it is
the boundary between @PLN102 and @PLN25 (the null model). Options: carry an explicit nullness
bit for `integer` (retire the sentinel); or keep the sentinel but *guard + document* every
collision, expose the reserved value as a constant, and give `find`/`min_of`/etc. honest
nullable return types. **Recommendation: settle this before any other lib item** — it changes
several signatures below, and signatures are the least-forgivable thing to freeze wrong.

**✅ Keystone status (chose "guard + document", not retire the sentinel):** rows above resolved by
the [null-model keystone](keystone-null-model.md) — row (null == null type-dependent) by **step 2**
(uniform scalar null comparison); row (`1 << 63` bit-collision) by **step 3a** (shift collisions
report); row (`find`/`rfind` typed non-null) and row (`min_of`/`max_of` typed non-null) by **step 4**
(now `integer?` / `T?`, zero caller breaks). Still open (deferred, not blocking freeze): the silent
integer-**overflow** row (@C85 decided edge — stays silent) and the missing `INT_MIN`/bounds
**constant** exposure (step 5 golden-residual / a separate additive lib item).

---

## The must-fix set (High / silent-wrong) — resolve or consciously-accept before the flip

Beyond the null-sentinel cluster above:

| # | Item | Why permanent-if-frozen | Fix | Ref |
|---|---|---|---|---|
| H1 | **Float `==`/`!=` are approximate (fixed epsilon) but `<`/`<=` are exact** | `a<b` **and** `a==b` both true near the boundary; sub-epsilon values collapse non-transitively; a float `hash`/`sorted` key hashes on exact bits but compares fuzzy | make `==` exact IEEE; expose `approx_eq(a,b,eps)` as a named fn | `01_code.loft:344,548` |
| H2 | ~~**`len(text)` = BYTES, `size(text)` = chars** (inverted from every mainstream lang)~~ **DONE via [@PLN110](../110-len-size-semantics/README.md)** — flipped: `len(text)`=character count, `size(text)`=byte footprint (and `size` is now a real allocation-local memory primitive on every structure). Byte indexing/`find`/slices stay byte-addressed and pair with `size`; a `for i in 0..len(s){s[i]}` units error is caught by the default-on strict-index lint. `len(character)` kept = UTF-8 byte width (owner call). Validated both backends: full suite 2998/2998, both top-risk consumers correct (`glb.loft` binary chunk length; `lib/markdown` multi-byte render). Republish of the affected registry libs is the only user-gated tail. | ~~silently wrong on non-ASCII~~ | `01_code.loft` (`len`/`size`), `03_text.loft` |
| H3 | ~~**`File.write()` returns void**~~ **DONE** — now returns `FileResult` (`Ok`/`Other`), like delete/move/mkdir; a failed write is observable (`f.write(s).ok()`); discarding callers unaffected. Runtime `Stores::write_file` returns bool; loft `write` wraps it. Both backends; test scripts/561. | ~~unobservable~~ | `02_files.loft` `write` |
| H4 | ~~`content()`/`read_bytes()`/`list_dir()` return `""`/`[]` for a missing file~~ **DONE** — now `text?`/`vector<u8>?`/`vector<text>?`; a MISSING file/dir reads NULL, an EMPTY-present one reads `""`/`[]` (the honest-`τ?` @C90 pattern). Discharge with `?? ""`/`?? []`. Fixed `vector<T>? == null` (vec_null peels Optional). Both backends; tests scripts/430,562. | ~~silent~~ | `02_files.loft` |
| H5 | ✅ **FIXED ([@PLN109](../109-json-lexer-unification/README.md))** — was: JSON numbers f64-backed, integers > 2⁵³ silently rounded. Now an integer-shaped number preserves its exact i64 as `JInteger` / `Parsed::Int`; `{"id":9007199254740993}` decodes exact on both backends, typed and generic. | was: silent precision loss on IDs / ns-timestamps | `crate::json` driven by loft's own lexer; `JsonValue::JInteger` + `JV_DISCR_INT` | `06_json.loft:29` |
| H6 | ✅ **FIXED (2026-07-16)** — the 7 all-quantified text classifiers now return `false` on `""` (was vacuously `true`) | closed the `if input.is_numeric()` accepts-empty footgun; empty→false matches Python's `"".isdigit()`/`.isalpha()`/`.isspace()` | DONE — an `if self.len()==0 { return false }` guard on each of the 7; regression in `tests/scripts/03-text.loft`; verified both backends; ZERO in-tree callers relied on the vacuous-true | `03_text.loft:74–146` |
| H7 | ~~**`text as integer/single/float` silently yields null / NaN**~~ **DONE — already closed by @PLN25 N-Parse** (verified 2026-07-17, both backends): a text parse `as integer` types **`integer?`** (`as float`→`float?`, `as single`→`single?`), and storing it non-null is a **compile error** (`n: integer = "abc" as integer` → *"a text parse `as integer` may fail — use `integer?` … or `?? <default>`"*). The blessed checked path IS `s as integer?` / `s as integer ?? d`; `"1.2.3" as float` → null (discharged), never a silent NaN. Exactly the proposed fix, already shipped. | ~~silent~~ | `fill.rs:466,472,478` |
| H8 | ~~**`sorted[a..b]` is a KEY-range query sharing vector's positional-slice syntax**~~ **CONSCIOUSLY ACCEPTED ([@C99](../../DESIGN_DECISIONS.md), 2026-07-17)** — sorted's subscript is *uniformly* KEY-addressed: `s[k]` single-lookup, `s[k]=null` removal, and `s[a..b]` key-range are ONE model (verified `s[20]`→key20, `s[1]`→null, so the SINGLE subscript is already key-addressed, not positional). The audit's "reject positional-shaped slices" is incoherent — it can't reject the range while keeping the key-addressed single lookup, and it breaks `spatial`'s deliberate proximity syntax. The cross-type footgun (visually like vector's positional slice) is inherent to key-addressing and is documented (LOFT.md Gotcha) + golden-locked (`sorted_subscript_is_key_addressed_not_positional`, keys far from positions), not removed. | ~~trap~~ deliberate + consistent | @C99; `LOFT.md` |
| H9 | ~~**`FileResult` advertises error variants that never fire**~~ **DONE (2026-07-17, both backends)** — the classifier is now real: `delete`/`move`/`mkdir`/`mkdir_all` map the OS error to `NotFound` / `PermissionDenied` / `IsDirectory` / `Other` (shared chokepoint `codegen_runtime::fs_classify` → integer code → the loft `fs_result` mapper; verified `delete(dir)`→IsDirectory + read-only-dir→PermissionDenied, both backends). The one variant that could **not** be produced portably — `NotDirectory` (needs `ErrorKind::NotADirectory`, unstable before Rust 1.83, and `codegen_runtime` is recompiled by the user's rustc for `--native`) — was **removed** rather than frozen as a lie. Codes are decoupled from the enum's declaration order (explicit `match`). Regression extended in `tests/scripts/42-file-result.loft`. | ~~frozen lie~~ | `02_files.loft` |

---

## Cross-cutting systemic themes (fix the pattern, not just instances)

1. **Method-vs-free placement is inconsistent and cheap to make uniform** (INC#8). Aggregates
   (`sum_of`/`min_of`/`max_of`/`sum`) are free-only so `v.sum_of()` errors while `v.len()`
   works; `index.len()` fails as a method though every sibling has it; `trim` is `both:` but
   `trim_start`/`trim_end` are `self:`. **Fix: declare the whole aggregate + text + collection
   families `both:` uniformly** — additive, erases a permanent "which can I dot?" scar.
2. **The error surface is heterogeneous — and it is frozen too.** "Something went wrong" is
   variously `File{format:NotExists}`, `""`, `[]`, `boolean`, `FileResult`, `0` sentinel,
   `i64::MIN`, a global `json_errors()` string, or a `raise`. **Fix: pick one convention per
   surface** (fault vs nullable-return) before it calcifies.
3. **Empty-collection / empty-input edge behavior is undecided per-op.** `min_of([])`=null vs
   `sum_of([])`=0; `v[0]` raises vs `h[key]` returns null; `split("")`=`[]`; `is_numeric("")`=false (H6 FIXED).
   **Fix: a single stated rule for "operation over empty."**
4. **Naming conventions drift.** The `_of` suffix is on `min_of`/`max_of`/`sum_of` but not
   `sum`; `dir` vs `directory` split within one module; `float`/`single` inverted from C-family;
   `long` names a type that doesn't exist (`as_long`); `spacial` is a misspelling frozen as a
   keyword; the `both:` mechanism-keyword leaks in as the param **name** (`abs(both:)`), and
   the docs say `abs(v:)`. **Fix: one naming pass** (recommendations per item below).

---

## Per-module worklist

### Collections (vector / sorted / index / hash / spacial)

| Sev | Item | Fix | Ref |
|---|---|---|---|
| High | `min_of`/`max_of` typed `-> integer` but return null on empty | pin `-> τ?` (or raise) + fix doc | `:1532,1539` |
| High | ~~`sorted[a..b]` positional-shaped key-range slice (INC#2, H8)~~ ✅ ACCEPTED (@C99) — uniform key-addressing, deliberate | see H8 | — |
| Med | **`spacial` → `spatial`** misspelled keyword (from #550, just merged) | rename now, contract 0 | `:1281,1157` |
| Med | compiler-desugar internals are `pub` (`hash_sorted`/`radix_sorted`/`spacial_range`…) | make non-pub or `__`-reserve | `:1136–1157` |
| Med | `clear()` only on vector (op `OpClearKeyed` already exists) | add `clear(both:)` to all keyed types | `:1026,1266` |
| Med | `sum_of` integer-only while `min_of`/`max_of` generic | make `sum_of` generic over `Addable` | `:1529` |
| Med | no membership predicate (`contains`/`has`/`contains_key`) — `c[k]!=null` conflates absent/null | add `contains`/`has` | — |
| Med | `index.len()` fails as method (free-only special-case) | real `both:` `len` | `parser/mod.rs:2842` |
| Med | aggregates free-only (`v.sum_of()` errors) | declare `both:` | `:1529–1551` |
| Med | `v[i]` OOB raises but `h[key]` absent returns null | one contract per surface (accept + DD) | `:1040` |
| Med | `+=` vs `[key]=` dup-key behavior undocumented | verify + document both | `:1272` |
| Low | no `keys()`/`values()` on keyed types | add (additive) | — |
| Low | no `is_empty`/`first`/`last` | add `is_empty` (trivial); `first`/`last`→null on empty | — |
| Low | `sum`/`sum_of` confusable near-synonyms; `_of` suffix inconsistent | collapse to one family | `:1547` |
| Low | `map`/`filter`/comprehension vector-only (INC#2); `#index` invalid on index | accept + DD entry | — |
| Low | sort direction on the struct type drives every query (INC#12) | accept + DD, or add per-query direction | `LOFT.md:217` |
| Med | an **inline slice** `v[a..b]` is typed `iterator`, so `v[a..b].len()` errors ("Unknown type iterator") while free `len(v[a..b])` works; once assigned (`s = v[a..b]`, materialised to a vector) BOTH work | give an inline slice the vector method surface (materialise for method dispatch), or reject the free form too for consistency | verified 2026-07-11 |

### Text & character

| Sev | Item | Fix | Ref |
|---|---|---|---|
| High | ~~`len`=bytes/`size`=chars + byte-indexing vs char-iteration (H2)~~ ✅ DONE (@PLN110) — `len`=chars / `size`=bytes; units error lint default-on | see H2 | `01_code.loft` |
| High | ~~classifiers vacuously `true` on `""`~~ ✅ FIXED (H6) — empty → false | see H6 | `03_text.loft:74` |
| High | ~~`text as int/float` silent null/NaN (H7)~~ ✅ DONE (@PLN25 N-Parse) — typed `τ?` + forced discharge | see H7 | `fill.rs:466` |
| Med | `split(character)` vs `split_text(text)` — permanent two-name split | unify on `text` needle before freeze | `02_files.loft:139,167` |
| Med | `join` = both string-join and path-join (receiver-dispatched) | rename path form `path_join` | `03_text.loft:148` / `02_files.loft:706` |
| Med | no `character` case conversion (`to_lowercase` text-only, but `is_lowercase` has char form) | add char overloads | `03_text.loft:62` |
| Med | no `text.lines()` (only `File.lines()`, which is CRLF-aware) | add `lines(self:text)` | `02_files.loft:111` |
| Med | `byte_at` OOB returns `0` = a real NUL byte; `txt[i]` raises but `byte_at` doesn't | consistent OOB policy; nullable `char_at` | `03_text.loft:176` |
| Med | `text_from_bytes` maps invalid UTF-8 → `""` = empty input | null on invalid, or `_checked` variant | `03_text.loft:184` |
| Med | `is_lowercase("hello world")` = false (space fails); users expect "no uppercase" | rename/clarify predicate | `03_text.loft:70` |
| Low | no `repeat`, `pad_start/end`, `is_ascii*`, `to_digit`, `strip_prefix/suffix`, `replace_first`, `is_empty`, `contains(char)`, `find(from:)` | add the high-value ones (additive; but names freeze) | — |
| Low | `find`/`rfind` return `-> integer` with hidden i64::MIN null (part of keystone) | nullable return type | `03_text.loft:44` |
| Low | `dir` (abbrev) vs `basename` (full word) path vocab mismatch; `rfind` abbrev | one register | `02_files.loft:676,691` |
| Low | path helpers slash-only, operate on any `text`, no `Path` type | decide `Path` wrapper or document POSIX-only | `02_files.loft:658` |
| Low | `'a'+'b'` arithmetic vs `"a"+"b"` concat; `txt[i]`→char, `txt[i..j]`→text (INC#9) | accept consciously + DD | — |

### Numeric / math / operators

| Sev | Item | Fix | Ref |
|---|---|---|---|
| High | float epsilon `==` vs exact `<` (H1); null-sentinel cluster (keystone) | see H1 + keystone | `:344,548` |
| Med | **`both` mechanism-keyword leaks as the param NAME** (`abs(both:-5)`); docs say `abs(v:)` → documented named call fails | rename first param to `v`/`a`, keep `both:` semantics | `:151,566` |
| Med | `float`=64-bit / `single`=32-bit inverted from C-family; no `double` alias | add `double` alias for `float` | `:11,13` |
| Med | no `pow(integer,integer)` (coerces to float, inexact past 2⁵³) | add checked integer `pow` | `:338,463` |
| Med | no numeric constants (`INT_MAX`/`INT_MIN`/`TAU`/`INFINITY`/`NAN`/float `MAX/MIN/EPSILON`) | ship a constant set | `:374` |
| Med | `min`/`max`/`clamp` got `τ?` nullable overloads; `abs`/`sqrt`/`floor`/… did not | add `τ?` overloads across the math set (uniform null-lifting) | `:575` |
| — | domain-error math (`sqrt(-1)`, `acos(2)`) → **null** (NaN = the float null, @C90) | **ACCEPT: null, NO error** — governed by the existing [@C80](../../DESIGN_DECISIONS.md) spreadsheet model (undefined → null, never a runtime error); already conforms on both backends. No new decision needed. | `ops.rs` |
| Med | div-by-zero result depends on syntactic context (`x/0` null vs `(x/0)??d` sees Inf) | accept + DD + golden, or make paths agree | `:531–546` |
| Med | shift ops: `<<` masks in release / debug-asserts; `>>` no range check + sign-extends | define one rule for out-of-range/negative shifts | `ops.rs:408–428` |
| Med | integer `/` truncates, `%` takes dividend sign; no `rem_euclid`/`div_floor` | add floored variants | `ops.rs:255` |
| Med | `min`/`max`/`clamp` numeric-only though text/character are `Ordered` | add generic `Ordered` binary overloads | `:566` |
| Med | `u32` max is `2³²−2` not `2³²−1` (siblings get full range); `@P293` ≥2³¹ reads negative | reconcile or accept + document | `:38` |
| Low | aggregates free-only (cross-cutting theme 1) | `both:` | `:1529` |
| Low | no `sign`/`signum`, `trunc`/`fract`, `hypot`/`cbrt`/hyperbolic, `to_radians/degrees`, `is_nan/is_finite` | add (one-line ops; names freeze) | — |
| Low | no `u64` (crypto/hashing want it) | decide in-scope for 1.0 or accept + DD | `:38` |
| Low | `clamp(v,lo,hi)` with `lo>hi` silently returns `hi` | document precondition / debug-assert | `:608` |
| Low | `round` half-away-from-zero vs `as integer` truncate-toward-zero disagree on negatives | document rounding contract | `:307` |
| Low | `i32` has no `limit` clause (siblings do); `u32` missing from STDLIB.md table | add for symmetry | `:28` |

### File / I/O, JSON, coroutine, stacktrace

| Sev | Item | Fix | Ref |
|---|---|---|---|
| High | `write()` void → silent failure (H3); `content`/`read_bytes` conflate missing/empty (H4) | see H3/H4 | `:545,97` |
| High | JSON f64 integer precision (H5) → **routed to @PLN109** (retire the hand-rolled scanner, use loft's lexer); ~~`FileResult` phantom variants (H9)~~ ✅ H9 DONE — variants classified (`fs_classify`), `NotDirectory` removed | see H5/H9 | `06_json.loft:21` |
| Med | time units split: `now()` ms / `mtime()` s / `ticks()` µs — silent `/1000` to compare | unit-suffixed peers or loud doc | `:583,336,587` |
| Med | JSON errors via process-global `json_errors()` string side-channel (frozen surface) | reconsider the error channel | `06_json.loft:50` |
| Med | no `copy(from,to)`, no `append` (write truncates), no `rmdir` | add the three (additive; but the gap+idioms calcify) | `:269,545` |
| Med | metadata split path-vs-handle with no parity (no `size(path)`) | fill parity | `:336` |
| Med | `CoroutineStatus` enum has no accessor — 3 of 4 variants unreachable | add `status(gen)` or drop variants | `05_coroutine.loft:9` |
| Med | `Format` enum conflates open-mode (endianness) with entry-kind (Directory/NotExists) | split the enums | `02_files.loft:29` |
| Med | `stacktrace` public structs leak internals (`RefVal{store,rec,pos}`, `FnVal{d_nr}`), single-letter fields | hide/rename before freeze | `04_stacktrace.loft:16` |
| Med | `File` struct exposes internal bookkeeping publicly (`ref`,`current`,`next`) | hide non-`path`/`size`/`format` | `02_files.loft:53` |
| Med | `move` collapses "destination exists" to `Other` (no `AlreadyExists`) | add variant or document | `:272` |
| Med | `as_long`/`long` names a type that doesn't exist | rename `as_integer` | `06_json.loft:93` |
| Med | `dir` vs `directory` abbreviation split (`source_dir` the odd one) | pick one | `03_text.loft:210` |
| Med | text-read (`content`/`read_bytes`) vs write (`write`/`write_bytes`) verb asymmetry | add `read()`/`bytes()` alias | `:97` |
| Low | `path_sep()` exists but path helpers hardcode `/` | reconcile | `:192` |
| Low | `file()` maps out-of-project paths → `NotExists` (sandbox default) | document as DD | `:229` |
| Low | write defaults: overwrite + implicit `TextFile` | document/decide | `:545` |
| Low | `mtime` sentinel `0` ambiguous with epoch second 0 | note | `:336` |
| Low | `stack_trace_full` referenced but does not exist; JSON has no builder / `json_number(integer)` | resolve doc/impl split; add | — |
| Low | `f#read as Struct` leaks one record per read (known) | resource wart | `STDLIB.md:459` |

---

## INCONSISTENCIES.md re-examination (the 4 lib entries)

These are already "documented + regression-guarded" as acknowledged design points — but the
freeze forces a *fresh* decision, because "documented" ≠ "the right permanent choice":

- **#2** (vector richer than sorted/index/hash; `#index` invalid on index; **sorted key-range
  slice** = H8). The slice trap is the must-fix; the coverage asymmetry is a fix-or-accept.
- **#8** (method-vs-free placement) → cross-cutting theme 1: make the families `both:` uniform.
- **#9** (`txt[i]`→char, `txt[i..j]`→text) → keep consciously; soften with `char_at`.
- **#12** (sort direction on the struct type) → accept + DD, or add a per-query direction.

---

## How Phase 2 runs

For **every** item, work it in the full format above: **present the alternatives** (current +
each option + trade-offs), decide with the owner, then **enumerate and convert its program set**
in the same change. The order:

1. **Decide the null-sentinel keystone first** — it changes signatures (H-cluster, `find`,
   `min_of`), and signatures are the least-forgivable freeze. Coordinate with @PLN25. Its
   conversion set is the widest, so pricing it first informs everything downstream.
2. **Walk the must-fix set (H1–H9)** — each is fix-now or accept-with-golden-test, each with its
   alternatives laid out and its conversion set enumerated *before* the choice (the set's size is
   part of the decision).
3. **Apply the cross-cutting fixes as sweeps**, not per-instance (method-vs-free → uniform
   `both:`; the naming pass; the empty-op rule; the error-surface convention) — a sweep converts
   its whole program set at once.
4. **Walk the per-module Medium/Low tables**, deciding fix-or-accept per row.
5. **Every conscious acceptance → a [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) entry + a
   golden test**; **every fix → its conversion set migrated in the same change**, while
   `CONTRACT_VERSION` is still 0. Land the **golden-behavior corpus first** so each conversion's
   diff is visible (nothing moves silently).
6. Only when the list is cleared is the `0 → 1` flip earned (with the language side — Phase 1 —
   also settled).

**Caveat (named, not hidden):** this worklist is the *known* surface. A freeze-forever audit's
real residual is the wart no survey imagined — so Phase 2 should also *use* the stdlib against a
real consumer (the dogfood loop) to surface what a read cannot, and add each to this list before
the flip.

## See also

- [COMPATIBILITY.md](../../COMPATIBILITY.md) — the promise + § Before the flip (this is its lib half).
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) — the language-side warts ledger.
- [STDLIB.md](../../STDLIB.md) — the documented surface being audited.
- [CODE.md](../../CODE.md) — the stdlib naming convention the naming findings are judged against.
