<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The last three toward 2× — designs, each priced by hand first

The C ship routines still over 2.7× after the 2026-11-03 push, against their Rust twins:
hex_shape `wall_chain_walk` (#2379, 3.08× clang / 2.15× gcc), arguments `parse` (#2380, 2.93× /
2.05×) and hex_recover `forms_upto` (1.65× / 2.70× gcc).  Each design below is a widening of a
rule we already have, as @PLN211 arc D asks, and each was priced by editing the emitted C of the
routine's own bench (same answer, instructions per op counted on one core; times only compared
within one run).  The phases are P16–P22 of the plan's table.

## wall_chain_walk — fold the edge-direction search (P16, P17)

**Where the time is.**  Half of clang's time is hex_field's `eg_index` → `eg_dir_from`, reached
from `chain_marks` through `edge_mat(edges, q, r, nq, nr)` with `(nq, nr) = hex_neighbor(q, r,
wd)`.  `eg_dir_from` searches the six directions for the one that leads from `(q, r)` to `(nq,
nr)` — and that direction is `wd`, which the caller just used.  The search folds away once the
caller's `wd` is a constant and everything between is inlined: `nb_q(q, r, i) == nq` becomes a
comparison of two expressions in `q` and `r`'s parity, equal for `i == wd` and false for the
others (`r - 1 == r + 1` and the like), which both C compilers prove.  The twin gets it from
LLVM inlining the whole chain; the C doesn't, for two reasons:

- `chain_marks`' direction loop (`for wd in 0..3`) has a `continue`, and `(R-Peel)` refuses a
  body with a `break` or `continue` — so `wd` is never a constant;
- clang leaves `edge_mat` / `eg_index` / `eg_dir_from` / `nb_q` / `nb_r` / `hex_neighbor` as
  calls inside a large function (the same inlining budget that kept `loft_push_code` out of
  line in `materialise`).

**Priced.**  Instructions per op (twin 29.4 M):

| form | clang | gcc |
|---|---|---|
| as emitted | 111.4 M | 70.4 M |
| the six helpers always-inlined | 94.8 M | 65.9 M |
| and the direction loop peeled by hand | **41.9 M** (1.43× the twin) | 60.2 M (2.05×) |

**P16 — `(R-Peel)`'s exit clause.**  A body with a `continue` of the peeled loop, or a `break`
of it, is admitted: copy `k` ends at label `peel<L>_<k>`, a `continue` jumps there, a `break`
jumps past the last copy to `peel<L>_end`; a `break` or `continue` of an ENCLOSING loop keeps
its label, since the copies are inside that loop's body as the loop was.  The counters after a
`break` are as the loop leaves them at that trip (each copy's own assignment already set them).
Each copy starts from the loop's entry binding state (3f4abdd50).  Guard cells: `continue` in
the first, middle and last trip; `break` in each; a `break` of the enclosing loop from a copy;
a `return` from a copy.  Poison sweep on the result, since a copy is new code for every
lifetime rule.

**P17 — always-inline for a small leaf.**  A function whose body, after its own peels, has no
loop, no call but operators and other such leaves, and at most N IR nodes (N set by the census
of how many functions it marks, starting at 40), is emitted `LOFT_HOT_INLINE` — and so are its
wrappers.  Both compilers then see the chain whole.  Measured as: the instruction counts above
on `wall_chain_walk`, and a full table on both compilers (an always-inline that grows a hot
caller can cost elsewhere; that run decides N).  A REPORT of the functions marked, by size,
lands with it.

**Expected.**  ≈ 1.5× clang, ≈ 2.0× gcc on `wall_chain_walk`; `edgeset_count` and every other
caller of `edge_mat` over a peeled direction loop the same way.

## parse — texts read and kept as views (P18, P19, P20)

**Where the time is** (clang, the full bench): the walks over `self.options` comparing each
option's `long_name` (`find_long_opt`, `ambiguous_matches`) — per option a checked element
read, a store lookup and a record test in `loft_get_text`, the string-record check in
`loft_store_str`, then `memcmp`; `classify` copying each argument word into a token record
(`Pos { word: arg }`), freed after `parse` reads it — `loft_text_free`, `loft_refill_text`,
`memmove` and `loft_tp_take` together ≈ 13 %; `find_short_opt` building `"{fch}"` per option to
compare it.  The twin's `classify` borrows: its tokens hold `&str` slices of `argv`.

**Priced.**  The text field read through the held header in both `find_long_opt` walks
(`loft_vh_text_at(h, size, i, fld)`): clang 168.4 M → 151.3 M instructions (−10 %), gcc −1 %.
Twin 47.2 M.  The other two are priced as the first step of their phases.

**P18 — a text field of a held element read at its address.**  The narrow clause of
`(R-Header)` for `OpGetText`: the field's string record read at `base + i·size + fld` and
resolved in the held vector's store, no reference built, no record test; under the counter
clause (or a walk whose index is tested before the field is read) no index test either.  The
walk's element read moves after its length test so that an index past the end is never
formed.  Guard cells: a text field read in a walk, past the end of a shorter vector, null.

**P19 — `(R-Transient)`'s view clause for a text the caller keeps alive.**  In a transient
producer (`classify`, whose `vector<Tok>` `parse` reads and drops), a record's text field set
from a text that outlives the transient value — an argument of the producer, a field of one —
is held as a VIEW (`loft_str`) in the arena record, not a copy, under the transient's own
lives check (the source is not written while the value lives, which the caller's frame
guarantees for an argument).  Price first: `classify`'s tokens as views in the hand form; the
copy-and-free share says ≈ −13 % on clang.

**P20 — `"{c}" == t` compared without building the text.**  An equality between a format
string whose only part is one interpolated `character` and a text compares the character's
UTF-8 bytes with the text — no work buffer, no formatting.  An IR-phase rewrite (both backends,
@PLN211), in the parser's format lowering.  Price first; `find_short_opt` is ≈ 2.5 % of clang's
time, so this one is for its generality, not this routine.

**Expected.**  P18 + P19 ≈ −23 % on clang → ≈ 2.25×; the rest of the gap is `classify`'s own
text work (prefix tests, the `=` search, slices), which the twin does on `&str` — the next
widening after these three is measured, not guessed.

## forms_upto — the candidate built outside the store, the dedup in place (P21, P22)

**Where the time is.**  Per candidate the `Form` `ef` is rebuilt in the store — two vector clears,
two reservations, a push window per vector (`loft_vec_reserve_more` 9 %, `loft_vec_clear` 6 %,
`loft_ph_of` and the puts ≈ 10 %) — then read back by `form_admissible` and `form_canon`; per
admissible form the dedup `(texts[si] ?? "") == ct` copies each stored text twice (a
`loft_text_own` and a `loft_text_set` for the `??`), compares, frees both.

**Priced.**  The dedup read as a view (`e = loft_get_text(...); if null e = ""; eq(e, ct)`):
306 → 270 µs per op (−12 %), same hash.

**P21 — `(R-Transient)` with a literal root.**  A local built by a record literal (here
`form_new` inlined as a destination) and only read — passed to reader callees, its fields
read — is a transient root like a producer's result: built in the arena (its vectors as
fixed C arrays when their length is a literal, here three), the readers called through their
`__tr` twins.  Today `site_plan` only takes a root bound from a producer CALL, so the inlined
literal is never a candidate.  Price first: the hand form of `ef` as an arena struct with two
`int64_t[3]` and `form_admissible` / `form_canon` reading it; the store share says ≈ −25 %.

**P22 — a text `??` read as a view where its consumer only reads.**  `(R-TextBorrow)`'s
discharge clause widened: `v[i] ?? lit` (or `?? ""`) over a text element is the view it picks
when its value is consumed by a reading operator (`==`, `!=`, `<`…, `len`, `starts_with`, an
append into ANOTHER text) in the same expression — never bound to a variable, never handed to a
call that could write `v`.  The consumer check is the part the current clause lacks: without it
a view handed to a call that pushes to `v` would dangle.

**Expected.**  P21 + P22 ≈ −35 % → ≈ 1.8× gcc.

## Order

P16 (smallest, widest: every peel with an early exit) → P17 (needs the full-table decision of N)
→ P21 → P22 → P18 → P19 → P20.  Each phase is gated like the rest of the plan: its guard cells
falsified by plants, the C sweep plain and POISON against the pinned suite, the C test binaries,
both clippy variants, and the full table on both compilers before its row is called done.
