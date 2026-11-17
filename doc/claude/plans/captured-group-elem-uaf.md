<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Captured-group element access — use-after-free of the group's backing store

**Status: FIXED — BOTH 35m and 35c (2026-07-19), both backends.** `area:store-lifetime`, `sev:high`.
Two independent roots, two fixes (see below). The nightly miri gate's remaining two red scripts are
both green now.

**35m fix** — the materialisation fix landed: `if_tail_yields_text` now sees through `Block["vector_match"]`
(`control.rs`), so a text-returning vector-match takes the proven per-arm `__acc` accumulator
(`do_if_acc` → `push_text_arms_into`) — each arm byte-copies its text into an OWNED buffer
before the group's backing store is freed, exactly the WORKING reference shape below. One-line
recogniser change reusing the scalar-match delivery; NOT the `scopes.rs` free-move the earlier
"decided direction" proposed (that was found INSUFFICIENT — the returned `&text` aliased the
freed store, so a free-move alone leaves a caller-side UAF).

Verified: `35m` clean under `LOFT_STORE_GUARD` + `LOFT_POISON` (both backends), correct values,
leak-clean; the `--show-ownership` overlay is now SILENT on the whole corpus; full suite green
(the one flake, `wasm_debug_relay`, is pre-existing). Emitted IR now matches the m5 reference
(owned `_mv_name_1:text` copy → `___acc_1 = _mv_name_1` → free `__vdb_1` AFTER).

**35c fix** — a DIFFERENT root (return-source freed before return, `parse` sub-class A): a captured
struct-enum field (`[Kw { word }, ..] => LetS{…}`) binds an owned `_mv_word_1` text whose
`OpFreeText` is appended AFTER the arm chain in the vector-match block. `scopes::collect_return_sources`
took the block's `.last()` op — that trailing free — and so never found the record sources
`__ref_1`/`__ref_2`; unsuppressed, the returned enum store was freed with a plain `OpFreeRef` before
the `return` (`P4-records` `OpFreeRefIfDistinct` never fired because `ret_var` was also lost). Fix:
`collect_return_sources` now skips trailing scope-exit frees (new `last_non_free_result`) to reach
the real block value, so the record sources are found and freed conditionally (`OpFreeRefIfDistinct`,
the c6 shape). Verified: 35c clean under `LOFT_STORE_GUARD` + `LOFT_POISON` on both backends, correct
values, full suite green. The overlay does NOT fire on 35c (its free is not a free-before-view-read;
this root is a return-alias free, which a dep-based static check cannot see — a candidate for a
return-source-free static gate — NOW BUILT, see the Static gate section below).

## Static gate (2026-07-19) — `introspect --show-ownership` now MAKES THIS VISIBLE

The @PLN103 lifetime inspector was blind to this bug: its static ownership verdicts are
temporal-agnostic (identical for a correct free and a use-after-free) and its runtime timeline
tracks leaks, not reads-after-free. It now carries a **free-before-dependent-read overlay**
(`use_analysis::free_before_dependent_read`, rendered under `--show-ownership`): along each
straight-line path, an `OpFreeRef(S)` followed by a DEREFERENCE of any (transitive) view of `S`
prints `⚠ UAF: \`arg\` is read AFTER \`OpFreeRef(__vdb_1)\` …`. Run it:

```
loft introspect --show-ownership tests/scripts/a-repetition-runs-mid-slice-and-over-a-cursor.loft   # ⚠ UAF fires
```

Boundary (verified): fires on 35m + the minimal repro + the nested-match variant; SILENT on the
int-return twin, the owned-vector twin, and 511/512 corpus scripts (the one hit is 35m). Two axes
were load-bearing to get it precise — **transitive** deps (a nested `match arg{…}` derefs `arg`
through an intermediate `_match_subj_2` that only transitively views `__vdb_1`) and **deref-only**
reads (a bare `Var` in a `return`/move is a safe delivery, not a UAF — this is what cleared the
`__ret_N` return-hoist false positives on `85-…`/`562-…`).

**Second overlay — return-source-free (the 35c class).** `use_analysis::return_source_freed`, also
under `--show-ownership`, catches the 35c root the deref overlay cannot see (the return DELIVERS a
reference, it does not dereference in-frame): a plain `OpFreeRef(S)` where `S` is a record the return
value ALIASES on the same path (`⚠ UAF: \`__ref_1\` is a RETURN SOURCE freed by a plain OpFreeRef …`;
the safe form is `OpFreeRefIfDistinct`). It is **path-sensitive** — it tracks the plain-freed set per
branch and flags only a free of the store that IS the return on that path, so a plain free on a
`return null` path is not a false positive (the `497`/`98` shapes). Verified: fires on the buggy 35c
(temp-reverting the `collect_return_sources` fix), SILENT on the fixed tree + the whole corpus/stdlib.

Regression gates: `tests/introspect.rs::ownership_overlay_silent_after_captured_group_fix` guards BOTH
fixes end-to-end (+ `tests/data/uaf_overlay.loft` carrying `bad`/`good`/`parse`); each overlay's own
firing is proved parser-free in `use_analysis::{uaf_overlay_tests, return_source_tests}`.

**This overlay is the fix's static gate:** it fires now; after the materialisation fix it must go
SILENT on 35m (both backends share the verdict), with no new corpus hit. It also proved 35c is a
different root (35c stays silent — its free is correctly placed).

Owns two failing scripts — **NOT the same root** (a 2026-07-19 finding, see the Static gate
below; the earlier "same root" claim is retracted):
- `tests/scripts/a-repetition-runs-mid-slice-and-over-a-cursor.loft` — `get_vector: use-after-free on store N`. THIS
  plan's bug: a free-before-dependent-read (the group's `__vdb_1` freed before `arg[0]` deref).
- `tests/scripts/35c-rest-capture.loft` — `sub-class A (enum + captured field + rest): null`. A
  DIFFERENT root (return-source freed before return), FIXED separately — see the 35c fix in the
  status block above. `parse`'s `OpFreeRef(__vdb_1)` was correctly placed; the bug was the RETURNED
  enum store (`__ref_1`) freed with a plain `OpFreeRef` because `collect_return_sources` was hidden
  from the record sources by a trailing `OpFreeText`. The overlay does not fire on it (a return-alias
  free, not a free-before-view-read).

These are why the nightly **miri** workflow's *Debug-assertions gate* and *LOFT_POISON
arena-UAF gate* have been red (both `#[cfg(debug_assertions)]`-only; the scripts PASS on a
plain release build, so the normal suite never catches them). @PLN35 Phase 6 (match-slice
repetition) introduced the shape.

## Repro (minimal)

```loft
enum Token { Punct { #lexeme sym: text }, Ident { name: text }, Num { value: integer } }
fn id(n: text) -> Token { Ident { name: n } }
fn p(s: text) -> Token { Punct { sym: s } }
// reading an ELEMENT of the captured group `arg` (index or nested rest match) UAFs;
// `len(arg)` is fine; the same access on a NORMAL vector is fine.
fn f(v: vector<Token>) -> text {
  match v { [_, "(", (arg: Ident)*(","), ")"] => match arg[0] { Ident { name } => name, _ => "?" }, _ => "!" }
}
fn main() { f([id("foo"), p("("), id("a"), p(","), id("b"), p(")")]); }
```

Build + run (the free-check is a `debug_assert!`, off in release):
```
RUSTFLAGS='-C debug-assertions=on' CARGO_TARGET_DIR=target-da cargo build --release --bin loft
LOFT_STORE_GUARD=1 target-da/release/loft --interpret f.loft
#   → thread 'main' panicked at src/vector.rs:356: get_vector: use-after-free on store N
```
(`LOFT_POISON=1` under a nextest run is the other gate; same root — freed memory read back.)

## Matrix (re-run 2026-07-19 on the DA `LOFT_STORE_GUARD` binary — the doc's original
matrix was a BLIND instrument; the get_vector panic only fires when the freed slot is
REUSED before the read, so "no panic standalone" ≠ "no premature free")

| arm body | return | standalone panic? | premature free? |
|---|---|---|---|
| `len(arg)` (m1) | `integer` | no | **no** — the whole match hoists to an owned `__ret_1:integer`, so `OpFreeRef(__vdb_1)` lands AFTER `len(arg)` (correct) |
| `len(name)*100+len(arg)` (m3) | `integer` | no (but DID panic inside 35m once a sibling reused the slot) | **no** — same int-hoist |
| `match arg[0] { Ident{name} => name }` (m2) | `text` | **YES** | **yes** |
| `match arg { [Ident{name},..] => name }` (m4) | `text` | **YES** | **yes** |
| the SAME element-text return on a NORMAL owned vector `ws` (m5) | `text` | no | **no** — arm materialises an OWNED `text` copy, frees after (the WORKING reference) |

Real boundary: it is **not** "element deref vs `len`". A **value-return** arm (int from
`len`) is safe because it takes the `__ret_N` hoist (free after the result). A **text-return**
arm that yields a value VIEWING `arg` (element field, nested match) is broken because the
vector-match promotes the `&text` result to the caller out-buffer *aliased to the view* and
frees `__vdb_1` under it. `len(arg)` is safe even though it reads `arg` — the int it produces
is independent of the store.

## Root cause (RE-PINNED 2026-07-19 — the free comes from `push_frees_into_arms`, and a
free-MOVE alone is INSUFFICIENT)

The captured group `arg` is a **view** into a fresh backing store `__vdb_1`
(`arg = OpGetField(__vdb_1, 0, 70)`, dep `["__vdb_1"]`). The failing fn returns text via the
`&text` out-buffer contract (`fn n_f(v, _mv_name_1:&text) -> text["_mv_name_1"]`). Emitted arm:

```
{#slice_binding
  OpDatabase(__vdb_1, 69);  arg = OpGetField(__vdb_1, 0, 70);   ; arg VIEWS __vdb_1
  … materialise loop building arg …
  OpFreeRef(__vdb_1);                                            ; <-- premature free
  if arm_cond { _mv_name_1 = OpGetText(…OpGetVector(arg,16,0)…); _mv_name_1 } else "?";
}                                                                ; reads arg[0] AFTER free → UAF
```

Two facts the original diagnosis missed:
1. **Who frees, and where.** The free is NOT scopes.rs "last direct use" reclaim. `__vdb_1` is
   function-scoped and conditionally allocated per match-arm, so its scope-exit free is
   **distributed into each arm** by `scopes.rs::push_frees_into_arms` (the @PLN35 sub-class B
   path, reached from `insert_free` for a `RefVar(Text)` return whose tail is an `If`). That
   helper inserts the free at index `len-1` — right BEFORE the arm's result value. Its comment
   ("a store null on an arm's path makes the free a no-op, so pushing into every arm is safe")
   is blind to the THEN arm, where `__vdb_1` IS allocated and its view `arg` is read by the
   very result the free precedes.
2. **A free-MOVE is not enough.** The arm's result `_mv_name_1` is the `&text` out-buffer
   *aliased to a view* of `arg` (hence `__vdb_1`). Moving `OpFreeRef(__vdb_1)` to after `arg`'s
   last read still leaves the returned `&text` pointing into the store, which is freed before
   control returns → the caller reads freed memory. So the store cannot be freed in-frame at
   all *unless the text is first copied to an owned value*.

The **value-return** arm (int from `len`) is already correct: `insert_free` hoists the whole
match to an owned `__ret_N:integer` and frees after — the int is independent of the store. The
`&text` return is excluded from that hoist (see the `is_text_result`/`tail_needs_eval` guards
in `insert_free`, ~control-flow at scopes.rs:4771-4864), so it takes `push_frees_into_arms`.

## The WORKING reference (what correct looks like — `loft introspect` it)

The identical element-text return on a NORMAL owned vector (`ws: vector<Token> = […]; match
ws[0] { Ident{name} => name }`) is correct on both backends. Its arm materialises an **owned**
`text` copy, and the free lands after:

```
_mv_name_1(text) = OpGetText(…OpGetVector(ws,16,0)…);   ; OWNED byte copy (not &text)
___acc_1(&text) = _mv_name_1;                            ; out-buffer := the owned copy
OpFreeText(_mv_name_1);  OpFreeRef(__vdb_1);             ; free AFTER the copy
return ___acc_1;
```

The captured-group path must produce THIS shape: an owned-text copy of the arm result before
`__vdb_1` is freed, delivered to the out-buffer.

## Where the fix goes (corrected)

The chokepoint is the **text-return delivery for a vector-match arm**, not a scopes.rs
free-move. The fix must make the arm that returns text VIEWING an arm-local materialised group
copy that text to owned before the group's backing store is freed — i.e. reuse the
normal-match materialisation (the `ViewOfLocal`/owned-text path, `classify_text_return` +
`text_return` promotion in `control.rs`) instead of promoting `_mv_name_1` to a raw `&text`
alias of the view. Candidate loci:
- `text_return` / `classify_text_dep` (`control.rs:7898-8018`) — the promotion that turns the
  match value into the `&text` out-buffer; it does not distinguish "owned built text" from "a
  view into an arm-local store that gets freed".
- `scopes.rs::insert_free` + `push_frees_into_arms` (scopes.rs:4636-4874) — the free
  distribution; whatever path is chosen, the store's free must land AFTER an owned copy exists.
- `materialize_field_projection` / `parse_slice_repetition` (`control.rs` ~4195, ~4649) build
  `arg`; a materialisation that makes `arg`'s ELEMENTS owned-independent would also close it.

**Do NOT ship a free-MOVE-only patch** (the previously "decided direction"): it stops the miri
panic but leaves the returned `&text` aliasing freed memory — a subtler, still-real UAF.
Verify any fix makes the captured-group IR match the WORKING reference above on BOTH backends.

## Verification matrix (required before commit)

A store-lifetime change must pass EVERY mode — do not trust one:
1. `--interpret` and `--native` on the two scripts (both must PASS, same value).
2. Debug-assertions build (`RUSTFLAGS='-C debug-assertions=on' LOFT_STORE_GUARD=1`) — no UAF panic.
3. `LOFT_POISON=1` (the POISON gate) — no null/garbage read.
4. A leak check (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) — the fix must not orphan `__vdb`.
5. Full suite both backends (`./scripts/find_problems.sh --bg`) — no regressions (esp. other
   match/vector-return/borrowed-view cases; watch for a re-run of the `call_op`-style over-reach).
6. Graduate: the two scripts already exist; confirm they move from red-in-miri to green.

## Not this bug (ruled out during the investigation)

Name collision, field-name unification, API stubs, the `call_op` unary-inference bug
(that was the graphics-cluster issue, fixed in loft#592). This is purely the captured-group
view's backing-store lifetime.
