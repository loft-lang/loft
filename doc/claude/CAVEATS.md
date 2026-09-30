
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Caveats

Real edge cases that bite loft programmers today.  Each entry either has
a decided fix (with a milestone) or is an accepted trade-off we intend
to keep.  Entries that merely document shipped diagnostics or internal
compiler details belong in CHANGELOG.md / LOFT.md / SLOTS.md, not here.

**Maintenance rule:** when an entry is fixed, delete it; when it becomes
a design-accepted fact, move it to LOFT_DESIGN.md § Design decisions.

---

## Accepted trade-offs (not scheduled for change)

Closed-by-decision entries live in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  Short pointers kept
here for cross-reference; don't re-argue these in active caveat
tables.

- **Scalar `v[i]` negative-index asymmetry (intentional; documented 2026-07).**  `i ≥ len`
  → `null`, but a negative `i ∈ [-len, -1]` counts **from the end** (`v[-1]` = last element,
  like slices @P384); only `i < -len` → `null`.  So a null-guard (`if v[i]`, `v[i] ?? d`)
  catches an over-range index but NOT a `-1` sentinel / underflow — that reads a real
  element from the end.  Was documented only for slices; now in LOFT.md § indexing +
  loft-write.  Guard a possibly-negative index with `if i >= 0` first.
- **`OpDrop` releases at the OWNER's death, and three things it does not do.** Copying a
  droppable into a struct field, an enum payload or a collection element MOVES it: the source
  stops releasing, and the container's death releases what it holds. The full contract is
  [INTERFACES.md § `OpDrop`](INTERFACES.md); what surprises people is what it deliberately
  leaves out. **Taking a value back OUT does not release it** — `v.remove(i)` and
  `v[i] = other` leak the element that goes away, by decision
  ([DESIGN_DECISIONS.md § C111](DESIGN_DECISIONS.md)), so release it yourself before you
  replace it. **A keyed collection does not release its records**, because a `hash`/`sorted`
  shares them with the collection it is indexed from. **Moving one droppable into TWO
  containers releases it twice**, because loft has no move checker. Two ordering rules go
  with it: a scope end runs AFTER the function body, so a resource whose owner is closed
  inside that body must be released explicitly first, and the release must be idempotent
  because exhaustion and the scope end both call it. When testing one, note that a same-scope
  test proves little — the source dies last there anyway, so a mistake only shows once the
  container OUTLIVES it (a return).
- **The fit test after a narrow store is ADJACENCY-SENSITIVE, and that is the trade
  (`@FR-E-Uncomp-Seen`, @PLN152).**  `x += 10; if !x { … }` on a `u8` / `i8` / `u16` / `i16` /
  `u32` / `limit(lo, hi)` slot answers whether the store fit; put ONE statement between them
  and `!x` goes back to meaning *is this null*, which on such a type is always false.  The
  boundary is not stylistic: past the next statement the only thing that could carry the
  status is the slot, and writing it there would cost every element of a `vector<u8>` the
  byte the narrow type was declared for (0.362 MB against 5.909 MB over 200 000 elements).
  It does not fail silently — `redundant-null-negation` reports the moved test — and it
  reaches only a place whose read is a plain fetch, so `w[bump()]` and a keyed `ks[k].v` (whose
  read is already nullable) keep their existing meaning.  The value stored is identical either
  way.
- **C3** — WASM `par()` runs sequentially.
  See [DESIGN_DECISIONS.md § C3](DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially).
- **C38** — Closure capture was copy-at-definition.
  See [DESIGN_DECISIONS.md § C38](DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition).
  Plan-22 (shipped 2026-05-13) supersedes the copy semantics: scalar captures use
  heap-owned cells (auto-Reference encoding); `Type::Reference` captures use 12B
  `Parts::DbRef` into the live original.  Pure read-only captures of non-Reference
  scalars still behave as value copies.
  Design history (closed by @PLAN22):
  [plans/finished/22-mutable-closures/](plans/finished/22-mutable-closures).
  Regression guard: `tests/scripts/56-closures.loft::test_capture_timing`.

- **@PLN35 Phase 7 — streaming `match` over an `iterator<T>` is EAGER (materialise-then-match).**
  `match some_iter { … }` pulls the whole coroutine into a buffer `vector<T>` (behind the Cursor
  seam), then runs the normal vector-match — for ANY element type (loft#1737).  Consequences:
  (1) the source must be FINITE — the pull stops at `max_lookahead` elements (`LOFT_MAX_LOOKAHEAD`,
  default one million) with a defined error naming the `match` (`(P-IterBound)`, loft#1678); (2) a
  side-effecting `next()` is UB-by-contract (the pull order is the buffer order); (3) the whole
  stream is buffered, so it is not memory-lazy.  A truly LAZY per-read pull + a per-match
  `max_lookahead` bound was scoped and DEFERRED: it needs `read_slice_elem`/`cursor_len` to pull
  incrementally AND the 11 `len`-bounds reframed to `has(pos)` (else `cursor_len` still exhausts) —
  a large refactor whose only payoff (bounded-pattern matching over an infinite source) has no
  consumer yet.  Build it when one appears; today collect explicitly for an unbounded source.
  Guard `tests/scripts/35p-iterator-match.loft`.

- **A NUL character has no representation of its own (loft#755, loft#748).**  `character`'s
  null IS code point 0, and loft's null *text* IS the one-byte NUL string, so a NUL is
  reachable in a text but never distinguishable from "no character":
  * `text_from_bytes([65, 0, 66])` is a real 3-character text — `len`/`size` say 3,
    `byte_at` returns `65, 0, 66`, `find`/slicing work.
  * `s[1]` on that text answers **null**, and `for c in s` yields **null** at that
    position: the same answer, which is the guarantee — iteration yields exactly
    `len(s)` characters and each equals `s[i]`.  Before loft#755 the loop terminated
    on the character VALUE, so it stopped at the NUL and silently dropped the rest
    while every other accessor read past it.
  * `chr(0)` answers `""` for the same reason, and `text_from_bytes([0])` — a lone NUL —
    is the null text, for which `size` still says 1 but iteration correctly yields nothing.

  So a NUL survives a round trip through **bytes** (`byte_at` / `text_from_bytes`) but not
  through **characters**.  A decoder that must preserve NULs should walk
  `for i in 0..size(s) { s.byte_at(i) }` and keep the data as `vector<u8>`.  Giving
  `character` a null distinct from 0 would fix this properly; it is a representation change
  across both backends with no consumer asking for it yet.
  Guard `tests/scripts/text-nul-iteration-755.loft`.

---

## Recursion depth is capped, and the cap halts the run

A call chain deeper than `State::MAX_CALL_DEPTH` (10 000 frames) halts the program with a
`call stack overflow` report on `--interpret` and `--native`, in every build; on wasm the
host engine's stack traps earlier (WASM.md § How deep a program can recurse).  It is the
one runtime halt left that is not `panic` or `assert`, and it breaks the rule that nothing
stops a running program (DESIGN_DECISIONS.md C80).

**What replaces it — [@PLN177](plans/177-growable-stack.md):** the stack grows in blocks, so
stack size never limits a program.  A production build has no depth limit; a development or
test build keeps a call-depth limit that halts with a full report of the recursion.

**Until then:** a recursion that can go deeper than 10 000 frames is written as a loop
over an explicit work list (a `vector` used as a stack).

---

## Native build — same-symbol cross-package `#native` collision (fix deferred → @PLN26)

Two native packages (`[native] crate`) that export the **same `#native` symbol**
can't be disambiguated under `--native`: the C-ABI link resolves first-`.so`-wins,
and the interpreter's bridge registry resolves last-loaded-wins (a *pre-existing
both-backend* hazard).  Today `--native` **rejects a reachable call** to such a
symbol with a clear `compile_error!` ("rename one with `#native \"<unique>\"`") —
so you never silently call the wrong fn; two packages sharing an *unused* symbol
still build.  `--interpret` keeps its existing (silent, last-loaded) behavior.

- **Workaround:** rename one of the colliding `#native` symbols (when you own the
  package); the error names the symbol.
- **Deferred fix** (canonical home: [NATIVE.md § Open work](NATIVE.md) —
  @PLN26 and its successor loft#388 are both closed/parked): per-package symbol
  namespacing so same-symbol packages coexist — touches the cdylib export, the
  interpreter registry, and codegen in lockstep; only needed to *call* a symbol
  two un-renameable packages both export (uncommon).
- **Repro / guards:** `tests/lib/collide_a` + `collide_b` (both export
  `collide_shared`), `collide_main` (calls both → rejected) / `collide_unused`
  (unused → builds); in-crate `native_symbol_collision_across_packages_detected`.

---

## Native build — no loft-source position on runtime faults (@PLN28)

The @PLN28 pc → source-position map (`Definition.source_spans`, populated at
bytecode codegen) is **interpreter-only**: it keys on the bytecode `pc`, which
`--native` has no equivalent of, and the native `raise` helpers pass
`position: None` (`generation/calls.rs`).  So the *mechanism* gap is real.

**But it is mostly not observable today** (verified 2026-08-09, both backends):

- `panic("…")` renders an identical `--> file:line:col` + caret on **both**
  backends, so the loudest runtime fault a user actually meets is not affected.
- Faults that C80 degrades — divide-by-zero, out-of-bounds read, out-of-bounds
  write — print **no** runtime diagnostic on **either** backend. They yield the
  sentinel and the program continues, so there is no caret to lose.

Compile-time diagnostics (parser / type / suggestion) are identical on both. What
remains exposed is any future non-recoverable `raise` path that reports a position:
that one would render on `--interpret` and not on `--native`.

- **Workaround:** none needed for `panic` or for C80-degraded faults. If a
  reporting `raise` is added, reproduce it under `--interpret` for the caret.
- **Canonical home:** [NATIVE.md](NATIVE.md); a native source map would need
  codegen to thread `Position` into the generated Rust, out of @PLN28's scope.

---

## The mixed interpret↔native boundary — closed 2026-06-27 (kept for the mode rule)

The 2026-06-26 wave of `sev:high` mixed-mode issues is fully closed —
[#460](https://github.com/loft-lang/loft/issues/460) (entry-package cdylib
dispatch; fixed via #464, guard
`tests/n3_use_native.rs::entry_package_is_never_auto_native_compiled`),
[#461](https://github.com/loft-lang/loft/issues/461) (struct-arg marshalling
across the interpret→native call; fixed via #466, guard
`moros_glb_cli_end_to_end`), and [#462](https://github.com/loft-lang/loft/issues/462)
including its borrowed-view-of-a-local residual leak (fixed via #466, guard
`tests/leak_cases/clean/p462_cond_reassign_retbuf.loft`).  No open `sev:high`
issue remains.

What stays worth knowing: mixed-mode programs (an interpreted caller into a
native shared-store library) exercise the marshalling boundary that uniform
modes never touch, so when debugging a suspected boundary issue, compare
against a uniform run — whole-program `--native`, or `--interpret` with
`LOFT_NO_NATIVE_LIBS=1` — to isolate which side owns the fault.

---

## Surfaced 2026-08-20 while fixing something else — all five CLOSED (re-measured 2026-09-02)

**All five are gone**, re-verified on 2026-09-02 by running each repro on this tree rather
than reading the row — which is the whole reason this document says to re-verify a caveat.
`#1032` (a generic returning `iterator<T>` walks) and `#1034` (a declared `(text?, integer)`
local is accepted) are simply fixed. The other three left something behind.

**#1033 repaid the re-verification twice.** It had been CLOSED, and it still reproduced — no
longer as the refusal it was filed as, but as a `null` answer with no diagnostic, so it had
moved from `sev:low` to `silent-wrong` while nobody was looking. Its title was wrong too: a
plain `fn f(v: vector<integer>)` was affected identically and a generic over a STRUCT element
was fine, so the axis was never the generic. It was the `par` element STRIDE for a nested
vector, computed from the inner element's type — fixed the same day, guard
`tests/scripts/1033-a-par-worker-gets-the-right-nested-vector.loft`.

**#1030 / #1009 / #1296 — settled, and since C127 the answer is a rule rather than a table.**
A width type's out-of-range `+=` no longer keeps `260`. It takes **the type's DEFAULT** —
zero where the declared range holds it, the bound nearest zero otherwise — and it does so in
every slot alike. The two full-integer TEMPLATES are the exception and keep C85's in-band
sentinel, answering **`null`**: `integer` and `i32` reserve a code at the BOTTOM, outside the
range they report, which is why `i32 = -2147483648` is refused and why that code is a real
absence rather than a number.

For two cycles the answer was a table, because whether a DECLARED range left a code spare
inside its width decided it: `limit(-100, 100) size(1)` answered null and
`limit(-128, 127) size(1)` answered `0`, two declarations one value apart, with the
arithmetic `hi - lo + 1 < 2^(8*size)` doing the deciding and no author having done it. C127
retired that — a spare code is not a null — so `u8`, `i8`, `u16`, `i16`, `u32` and every
`limit(lo, hi)` now answer alike whatever room their range leaves.

The default is a legal value of the type, so nothing holds a non-`τ` value, but it cannot be
told from a computed one. Two spellings can tell: `τ?` always answers null, because a
nullable narrow alias sacrifices an edge value to reserve one; and `if !x { … }` as the very
next statement after the step reads the fit failure without storing anything
(`@FR-E-Uncomp-Seen`). Since C127 the step also says so — `advice[narrow-fallback]` names the
default it took.

**#1031 — `u32` is one short of its name, by construction.** The local and the field agree
now. `u32 = 4294967295` is still refused: `default/01_code.loft` declares `u32` as
`integer limit(0, 4294967294) size(4)`, keeping the top code back — and per the table above
that reserved code is the one no non-null read tests for, so it buys the refusal without
buying a null — and since C127 no declared range buys one, whichever end its spare code
sits at. The 2026-08 changelog line "`u32` finally holding every `u32`" overstated the
fix, and is corrected.

The lesson the rows leave behind: a closed issue is a claim about a build, and the build has
moved. The rows below are kept for their repros; every issue is closed.

| | issue | shape |
|---|---|---|
| a generic function inside a `par` worker answers **`null`** | [#1033](https://github.com/loft-lang/loft/issues/1033) | `silent-wrong` |

**Four of the five are gone because they are FIXED** — re-verified on 2026-09-02 by running
each repro on this tree, which is the reason this table says to re-run rather than trust a
note. `#1030` (`integer limit(0,255) += 10` → `0`, not `260`), `#1031` (a `u32` local and
field now agree), `#1032` (a generic returning `iterator<T>` walks), `#1034` (a declared
`(text?, integer)` local is accepted).

**#1033 stayed, and it CHANGED SHAPE under the same re-verification**, which is why it is
worth the row: it no longer refuses the program, it compiles and answers `null` with no
diagnostic, so it moved from `sev:low` to `silent-wrong`. Both controls pass — the generic
without `par` answers `6`, and `par` with a plain function answers `9` — so it is the pairing
and neither half alone. Reopened.

**Two entries that were here are gone because they are FIXED**, both confirmed by
re-running their repros on both backends before filing anything — which is the reason
to re-verify a caveat rather than file it from a note:

- a tuple's text element assigned from its sibling (`k = ("a","b"); k.1 = k.0`) no
  longer fails `--native` with E0382;
- the Cluster F residual — a discharged `a?` in a TEXT tuple answering the null
  sentinel on the interpreter — is correct on all four carriers on both backends.
  Closed by loft#1026, whose own account (*"`parse_block` has two mutually exclusive
  text-return promotions … the monomorph promoter was replicating half of it"*) is the
  machinery [STABILITY_REDFLAGS](STABILITY_REDFLAGS.md)'s investigation had isolated
  from the other side.

---

## Scheduled — 0.8.5

### ~~P137~~ — `loft --html` Brick Buster: runtime `unreachable` panic — DONE

Shipped on `quality`.  Root cause: `Instant::now()` in
`Stores::new()` panics on `wasm32-unknown-unknown` (the `--html`
target).  Fix: guard switched from `#[cfg(not(feature = "wasm"))]`
to `#[cfg(not(target_arch = "wasm32"))]`; `host_time_now()` returns
0 in that mode; `n_ticks` gated identically.  The headline browser
demo and Moros editor share the same WASM path, both unblocked.
Regression guards: `tests/html_wasm.rs` (4 tests behind a
process-wide serial mutex covering hello-world, ticks, two
allocator paths).  Detail in PROBLEMS.md #137.

### ~~P135 / C58~~ — Canvas Y direction — DONE

Shipped on `quality`.  The three-way flip cascade (upload row-reverse,
TEX_VERT_2D `1 - aPos.y`, ortho `-2/H`) collapsed to one: the ortho
is the only compensating flip, matching the GL convention.  Canvases
and PNG textures now share the same orientation in GL.  Locked as a
language-level invariant in [OPENGL.md § Canvas coordinate
convention](lib_plans/58-graphics/README.md).  Regression guard: 2×2 atlas corner check in
`tests/scripts/snap_smoke.sh`.

### P135 / C58 (historical) — Canvas Y direction is not locked in
Three compensating flips (upload row-reverse, UV, 2D projection) that
don't cancel on non-square atlases.  **Decision:** canonical `(0, 0) =
screen-top-left`, `y` grows down — matches HTML canvas, PNG files, and
how users think about 2D drawing.  The 3D pipeline's internal OpenGL
texture-coordinate math stays internal.  Lock this as a language-level
guarantee in LOFT.md so future backends cannot drift.  Rebake Brick
Buster's atlas (the only loft program with a non-trivial layout).
**Test:** extend `snap_smoke.sh` with a 2×2 atlas corner check.

---

## Scheduled — 0.9.0

### ~~P142~~ — `vector<T>` field panics when T is from an imported file — FIXED 2026-04-17

Plain `use` now imports all `pub` definitions via `import_all`, so
`vector<T>` / `hash<T>` / `index<T>` / `sorted<T>` content types
resolve correctly across files.  The original reproducer (four-file
Moros-style `types.loft` + `palette.loft` + `spawn.loft` + `map.loft`
layout with cross-file `vector<StructType>` fields) exits 0 and reads
back expected values.

**Gap:** no dedicated Rust-level regression guard exists yet for the
multi-file `vector<T>` case.  The adjacent P143 guard
(`tests/lib/p143_*.loft` + `tests/issues.rs::p143_default_struct_return_from_nested_vector_use`)
exercises overlapping code paths but does not specifically cover the
original P142 panic shape.  Worth adding a one-file-per-struct guard
before 1.0 to keep the fix from silently regressing.

### ~~C54~~ — `integer` representation — DONE 2026-04-20

Shipped on branch `int_migrate`.  `integer` is i64 end-to-end
(stack, struct fields, arithmetic) across all three backends.
The `long` keyword and the `l` literal suffix were removed; writing
`long` now fails with "Undefined type long", and an `l`-suffixed
literal no longer parses.  Use `integer` (i64) and plain literals.

**Post-migration caveats — scheduled for 0.9.0 but kept open**:

- **Binary-format writers need explicit width casts.**  Post-2c
  `f += 2` on a `LittleEndian`/`BigEndian` file writes 8 bytes;
  pre-2c wrote 4.  Every `f += <scalar_integer>` that targets a
  u32/u16/u8 binary field must add `as i32` / `as u16` / `as u8`
  explicitly.  Regression guard: `lib/graphics/src/glb.loft` was
  the flagship fix (`74aefb4`) — its test
  `moros_glb_cli_end_to_end` now gates this behaviour.  A parse-time
  lint warns on every un-cast `f += <integer>` and lists the width
  aliases; silence an intentional 8-byte write with `as integer`
  (lint in `src/parser/objects.rs`, guarded by
  `binary_write_bare_integer_warns` + the two `binary_write_*_cast_silent`
  tests in `tests/parse_errors.rs`).
- **~~Cross-crate cdylib FFI stays on i32 vector&lt;integer&gt;
  elements.~~ RESOLVED 2026-05-21 (@P310).**  This bullet (and the
  "obsolete claim" follow-up below) described the bug @P310 finally
  fixed: `vector_elem_rust_type(Type::Integer) => "i32"` emitted a
  4-byte FFI pointer for `vector<integer>`, but post-2c the storage
  stride is 8 bytes, so the pointer disagreed with both the storage
  AND the (now `*const i64`) graphics wrappers — E0308 under
  `--native`/`--check`.  Fixed by keying `vector_elem_rust_type` off
  `IntegerSpec::vector_narrow_width()` (storage stride): plain
  `vector<integer>` → `i64`, narrow aliases keep their forced width.
- **The duplicate `Op*Long` opcodes are gone** (post-2c round 10d —
  see `default/01_code.loft`); the current opcode count is 246.  Only
  `OpConstLongText` (a text op, unrelated) still carries "Long" in its
  name.
- **`Type::Long` enum variant removed** (@PLAN01 phase 4, 2026-04-21).
  All integer-family values flow through `Type::Integer(IntegerSpec)`
  with i64 arithmetic on the stack and per-field storage width via
  `IntegerSpec.forced_size`.  See
  `doc/claude/plans/finished/01-integer-i64/04-deprecate-long.md`.
- **Memory footprint doubled for `integer` fields** (4 → 8
  bytes).  Narrow fields (`u8 / u16 / i8 / i16 / i32`) stay
  compact via `Parts::{Byte, Short, Int}` so pixel buffers,
  bit-packed protocols, and RGBA data are unaffected.
- **Stale derived artefacts after the migration (discovered
  2026-04-21).**  Neither `cargo test` nor `make ci` rebuilds
  `target/wasm32-unknown-unknown/release/libloft.rlib` or
  `tests/lib/native_pkg/native/target/release/libloft_native_test.so`.
  A developer who runs `cargo test --release` against a
  post-migration source tree but pre-migration artefacts will see
  **5 html_wasm failures + 6 native_loader failures** that look
  like real regressions:
    * `--html` rustc errors cite `codegen_runtime.rs:1244` (old
      `cr_rand_int` position) vs. current line 1409.
    * `native_loader::scalar_before_vec` reports
      `offset_sum: expected 106, got 103` because the .so still
      reads elements as 4-byte i32 from the now-8-byte-stride
      memory.
  Rebuild commands are in `DEVELOPMENT.md § Common pitfalls`.
  This class of tripwire is a general post-migration risk, not a
  language bug.
- **~~Cdylib FFI wrapper claim in this file was obsolete.~~ RESOLVED
  2026-05-21 (@P310).**  The half-stride discrepancy this bullet
  flagged is fixed: `vector_elem_rust_type` now emits `*const i64`
  for `vector<integer>` (matching `Type::size()` = 8 and the stride
  `vector_append` uses).  The graphics wrappers already declared
  `*const i64`; `lib/moros_render` takes no `vector<integer>` FFI
  arg (only graphics does), so no production cdylib was on `*const
  i32` for such an arg — verified by grep (no `*const i32` vector
  externs).  The test fixture (`tests/lib/native_pkg/native`) and
  graphics now agree with codegen.  Guards: `generation::p310_vector_elem_tests`
  + `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`.

Regression guard for the overall migration:
`tests/scripts/20-binary.loft`, `21-binary-ops.loft`,
`89-sizeof.loft`; `tests/docs/13-file.loft`, `17-libraries.loft`;
`tests/exit_codes.rs::moros_glb_cli_end_to_end`.

### ~~C60~~ — Hash iteration in key order — DONE 2026-04-13

Shipped on branch `quality`.  `for e in h { ... }` walks the hash in
ascending key order, yielding `reference<T>` — same shape as
`sorted`/`index`.  Implementation: the parser substitutes the iterated
expression with a `hash_sorted(h, tp)` call that builds a u32-stride
rec-nr scratch in the hash's own store (allocation co-location lets
the yielded `DbRef{store, rec, pos=8}` resolve directly to live hash
records).  Iteration routes through the existing `Ordered` (`on=3`)
bytecode — no new opcodes, no new runtime mode.

Commits: pieces 1 (`e50fffe`), 2 (`8d4d573`), edit A (`2e20ba2`),
edit E (`63226b8`), piece 3 (`2145a8d`), native (`0b85cd2`), Step 9
`#remove` diagnostic (`705338e`), docs Step 4 (`363ed12`).  Six
acceptance tests green in `tests/issues.rs::c60_hash_iter_*`.

The original design is in [CAVEATS-history.md](CAVEATS-history.md).

### ~~C61.local~~ — Outer-local shadow — DONE
`x = 5; for x in …` now rejected on pass 1 via the `was_loop_var`
flag on `Variable` — a slot that exists in `names` but has never
served as a loop variable is unambiguously a plain local, so the
shadow is flagged with a rename-or-drop hint.  Same-typed shadow
only (the existing type-mismatch check handles the different-typed
class with a clearer message).  Sequential same-name loops stay
legal because the prior slot carries `was_loop_var = true`.

Unblocked by PROBLEMS.md #139's `OpReserveFrame` fix, which made the
stdlib rename sweep possible without tripping the slot-allocator
TOS assertion.

**Tests:** `tests/parse_errors.rs::c61_local_shadow_rejected`,
`c61_local_shadow_renamed_ok`, `c61_local_dropped_outer_ok`, plus
the flipped-to-reject `shadow_same_type_ok`.
**Files cleaned up:** `lib/graphics/src/mesh.loft` (dropped dead
`row = 0; col = 0` inits), `lib/parser.loft` (renamed `p` / `f` →
`param` / `fld`), `tests/docs/01-keywords.loft` (renamed `for a`
→ `for i`), `tests/scripts/05-enums.loft` (two loops renamed),
`tests/scripts/39-diagnostics-passing.loft` (flipped the
once-permissive test), `lib/graphics/examples/25-brick-buster.loft`
(renamed `br_rt` → `br_pti`).

### ~~P91~~ — Default-from-earlier-parameter — DONE
Implemented via **call-site substitution** rather than function
prologue (the simpler approach worked).  `parse_arguments` injects
earlier arguments into `self.vars` before parsing each default, then
rewrites the parsed `Value` tree so `Var(slot)` references become
`Var(arg_index)` — a stable, portable form.  At call sites,
`Parser::substitute_param_refs` walks the default tree and replaces
each `Var(N)` with the caller's actual `list[N]` (already substituted
if earlier args also had defaults).

**Tests:** `tests/issues.rs::p91_default_references_earlier_param`,
`p91_default_identity_of_earlier_param`,
`p91_default_overridden_by_caller`,
`p91_chained_defaults_reference_earlier_args`.

### ~~P54~~ — typed `JsonValue` tree — SHIPPED
The decided fix is live: `default/06_json.loft` defines the
`JsonValue` enum (`JObject` / `JArray` / `JString` / `JNumber` /
`JBool` / `JNull`) and `json_parse(text) -> JsonValue` is the one
entry point, working on both backends.  The old text-based surface
(`json_items` etc.) is gone — calling it is an "Unknown function"
error.  The Q1 residual — diagnostics on the one-stage auto-wrap
`Struct.parse(text)` — CLOSED 2026-08-20: both spellings report, and
they differ in which half of the answer they give (the one-stage form
names the position, `line 1:33 path:addr.zip`; the staged form names
the types, `Addr.zip: expected JNumber, got JString`).

### ~~C7 / P22~~ — `spatial<T>` diagnostic — DONE
`spatial<T[x,y]>` / `spatial<T[x,y,z]>` (@PLN48) shipped as a working keyed
collection on both backends (interpreter + `--native`) — the old "planned
for 1.1+" diagnostic is gone.  Two diagnostics remain: a bare `spatial<T>`
with no coordinate key fields (*"spatial<T[x, y]> needs coordinate key
fields, e.g. spatial<Mob[x, y]>"*), and more than 3 axes (*"spatial<T[…] >
supports at most 3 coordinate axes, got N"* — `MAX_AXES = 3`).  See
[DATABASE_INDEXES.md § Spatial Index](DATABASE_INDEXES.md#spatial-index-srcradix_treers)
for the full operation set (construct/append/iterate/`len()`/range slices).
**Tests:** `tests/parse_errors.rs::spatial_needs_coordinate_keys`,
`::spatial_rejects_more_than_three_axes` (the old `spatial_not_implemented*`
tests no longer exist).  Guard scripts:
`tests/scripts/48-spatial-construct-free.loft`,
`tests/scripts/48b-spatial-slice.loft`.

### ~~P344~~ — a reused loop-variable name must keep a consistent type
**Fixed (loft#915).** Two `for` loops in one function may reuse a name at
different element types: `for i in [1,2,3] {…}` then `for i in ["a","b"] {…}`
compiles.  Each loop binds its OWN variable, so the second inherits no type, dep
or storage from the first — which is also what makes loft#690's corruption
(reading B's records through A's layout) unreachable by construction rather than
by diagnostic.  Since loft#1600 the loop variable also ENDS with its loop
(`(B-Scope)`): `i` after the loop is `local-out-of-scope`.

What is still rejected is a loop variable landing on a plain function local
(`x = 5; for x in …` → *"loop variable 'x' shadows a local named 'x'"*) and
nested same-name loops.  Guards:
`tests/scripts/915-loop-variable-per-loop.loft` (13 cells, both backends),
`tests/parse_errors.rs::shadow_different_type`, `tests/scripts/36-parse-errors.loft`.

---

## Verification log

Last retested: **2026-04-12** against commit `2aaba5a` (main branch).

| Caveat | Milestone | Decision |
|--------|-----------|----------|
| C3     | 1.1+      | Accepted — WASM threading deferred (Web Worker pool cost > benefit today) |
| ~~C7/P22~~ | — | **Done** — `spatial<T[x,y]>`/`<T[x,y,z]>` shipped as a working keyed collection (@PLN48); residual diagnostics are missing-key-fields and >3-axes |
| C38    | —         | Updated — @PLAN22 (2026-05-13) adds by-body mutation classification; Reference captures always via DbRef; scalars via heap cell.  Pure read-only scalar captures remain value-copy. |
| ~~C54~~ | — | **Done** 2026-04-20 — `integer` is i64 end-to-end; `long` is a historical alias.  See CAVEATS.md § C54 long-form for post-migration footguns |
| ~~C58/P135~~ | — | **Done** — canonical `(0, 0) = screen-top-left`; upload no longer pre-flips rows; convention locked in lib_plans/58-graphics/README.md.  Regression: 2×2 atlas corner check in `tests/scripts/snap_smoke.sh` / `make test-gl-golden` |
| ~~C60~~ | — | **Done** 2026-04-13 — `for kv in hash` yields a `HashEntry` with `.key` / `.value` in insertion/deletion-aware order via the internal ordered index.  See CAVEATS.md § C60 long-form |
| ~~C61.local~~ | — | **Done** — pass-1 reject via `was_loop_var`; stdlib docs cleaned up; unblocked by #139 |
| ~~P54~~ | — | **Done** — first-class `JsonValue` enum + `json_parse` shipped (`default/06_json.loft`); old text-based JSON surface withdrawn.  Residual: Q1 auto-wrap diagnostics (QUALITY-history.md § Open work) |
| ~~P344~~ | — | **Done** (loft#915) — each `for` loop binds its own variable, so two loops in one function may reuse a name at different element types.  A loop variable landing on a plain local is still rejected.  Regression: `tests/scripts/915-loop-variable-per-loop.loft` |
| ~~P91~~ | — | **Done** — call-site substitution of `Var(arg_index)` in stored default tree; 4 regression tests |
| ~~P137~~ | — | **Done** — `Instant::now()` / `n_ticks` gated on `target_arch = "wasm32"`; `host_time_now()` returns 0 on wasm32-without-wasm-feature.  Regression: 4 guards in `tests/html_wasm.rs` behind a serial mutex |

---

## Moved out of this document

- **C12** (null + `??` instead of exceptions) → design fact, see LOFT.md
- **C45** (zone-2 slot reuse text-only) → internal allocator detail, see SLOTS.md
- **C56, C57** (clean diagnostics for stdlib-name clash / nested file-scope decls)
  → shipped in 0.8.4, see CHANGELOG.md
- **C51, C53, C55, C61-nested** → fixed and deleted
- **P55** (thread-local `http_status`) → design reject, not an open item
- **P90** (per-call HashMap lookup) → premature optimisation, see PERFORMANCE.md
- **C60 original design** (the step plan hash key-order iteration shipped from) → CAVEATS-history.md

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — full bug tracker (severity, fix paths)
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries
- [LOFT_DESIGN.md](LOFT.md) § Design decisions — accepted language-level trade-offs
