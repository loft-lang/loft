<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `word_count` — why the interpreter is 5× Python on it

**Analysed and priced 2026-10-02, nothing built** (branch `laptop-superinstructions`, head
`9e169fd36`, every interpreter rule of the eliminations arc in).  `bench/08_word_count` is the
one row of the original suite (01–08) where Python is still far ahead.  On the others the
interpreter is 1.1–1.8× Python's time, and on `sum_loop` it is 1.25× faster.  The full-suite
run that showed it: `bench/stats.py --lanes interp,native,rust,python --samples 3` on the
pinned-layout build.

| lane | per word | |
|---|---:|---|
| loft interpreter | 291 ns | 88 ms for 300,000 words |
| Python | 57 ns | the interpreter is **5.16×** this |
| loft native | — | **3.23×** Rust; Rust is `HashMap<&str, i64>` with the default SipHash |

## Where the 291 ns go

Measured by removing one thing at a time from `tally`: each variant was timed on the pinned
build with `--n 20`, and the answers checked equal (hash `7f690e`) wherever the variant still
computes the count.  Python's side was measured the same way (`dict.get` + store, and the loop
with a `len` test instead).

| part of one word | interpreter | Python | the variants it comes from |
|---|---:|---:|---|
| the `?? ""` discharge of the element read | **~70 ns** | ~0 | `w = words[i] ?? ""` → `w = words[i]`: 87.3 → 66.1 ms, same hash |
| the hash lookup and the count update | **~100 ns** | ~18 ns | with the lookup and without: 66.1 → 37.5 ms (and 87.3 → 56.8 with the discharge kept) |
| the element read itself | ~26 ns | — | `words[i]` → the constant `"the"`: 37.5 → 29.6 ms |
| loop, dispatch, `w`'s own copy, the `len` test | ~99 ns | ~37 ns | the loop with a constant word |

A flat `perf record` of the whole bench agrees: dispatch 17 %, text copies and allocation
(`append_text`, `malloc`/`free`, `finish_grow`, `free_text`) about 25 %, the lookup about
22 %.

## The three causes, each with its lever

### 1. The text discharge copies the word twice — interpreter only, ~24 % of the bench

`w = words[i] ?? ""` lowers to `__ncc_1 = OpGetText(read); if OpConvBoolFromText(__ncc_1)
w = __ncc_1 else w = ""`.  The interpreter's text locals own a `String`.  So the word is
appended into the temporary, appended again into `w`, and both are freed: two allocations and
two copies per word.  Native emits both as `&str` (the text borrow, `round-3.md`), so this is
an interpreter-only cost.

**Lever (S, exact):** remove the temporary in the IR.  `w = read; if !OpConvBoolFromText(w)
w = ""` is the same statement when the target is a plain text local that the read does not
mention.  Both forms evaluate the read first and assign `w` exactly once more on the null
path.  One copy remains.  The rewrite belongs beside `(R-SameRead)` and `(R-InRange)` in
`formal/rewrites.md` (C125: remove the temporary rather than make it cheaper), runs in the IR
phase for both backends, and native is unaffected (its temporary is already a borrow).
Priced by the variant without `??`: −24 %.  That variant drops the null fallback, so it
over-prices the lever by the one `ConvBoolFromText` + jump kept.  The shape is common:
`v[i] ?? ""` binding a local appears 20 times in `lib/`, `tools/` (the formatter, the viewer)
and the libraries' checkouts.

### 2. The keyed lookup — both backends, ~100 ns against Python's 18

Three costs, in the order they are cheap to remove:

* **The key's content list is rebuilt on every lookup — interpreter only, ~7 %.**
  `State::stack_keys` calls `Stores::get_keys(db_tp)`.  That function walks the type's key
  fields through `key_field` / `key_contents_for_field` and collects a fresh `Vec<u16>` on
  every call (`Map<FlatMap<FilterMap>>` and `Vec<u16>::from_iter` are 4.6 % of the profile on
  their own).  It then allocates a second `Vec<Content>` for the key.  The list is a fact of
  the schema: compute it once per type when the schema is final, and pop the key into a
  fixed-size array for `no_keys ≤ 4`.  **S**, no format change.
* **The text key is hashed on every lookup — ~12 %.**  `hash::find` + `key_hash` +
  `SipHasher13::write`.  Python caches a string's hash inside the string object, and these
  words are interned constants, so its lookup is close to a pointer compare.  Loft text has
  no object to cache a hash in, so this part of the gap is structural.  It is not where native
  loses to Rust: Rust hashes with SipHash too.
* **Native goes through the generic lookup.**  The emitted `tally` calls `OpGetRecord(cell,
  …, &[Content::Str(Str::new(…))])`, the `Content`-slice path, for a text key.  The typed
  keyed path of `keyed.md` (L1–L7) does not reach it.  That is native's 3.2×, a medium-sized
  piece of work belonging to `keyed.md`.

### 3. The loop around it — ~99 ns against Python's 37

About 30 ops a word, and `w = "the"` still allocates.  The text local owns its `String`, so
every assignment copies, even from a constant, and the loop's end frees it.  Native borrows.
Giving the interpreter a borrowed text local is the same notion `vector-build.md` § T1 found
has no home ("a text variable is a `&str`" is spelled inline at six sites).  It is a
refactor before it is a rewrite, so it is not priced here.

## Order, and what it buys

1. The discharge rewrite (cause 1): S, exact, both backends' IR, −24 % here and on every
   `?? ""` text bind.
2. The key list cached per type, and the key popped into an array (cause 2, first bullet): S,
   interpreter, ~−7 %.

Together they take the row from about 87 to about 60 ms: from 5.2× Python to about 3.5×.
What is left after that is the hash (structural) and the owning text local (§ T1's refactor).
Native's own lever is the typed text-key lookup, in `keyed.md`.
