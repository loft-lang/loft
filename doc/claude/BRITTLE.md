# Brittle routines in loft

A survey of code paths that are more likely than the rest of the tree to
produce silent-wrong-result or intermittent-failure bugs.  Each entry
lists the symptom pattern you would see, the reason the code is
fragile, and the concrete path(s) to harden it.

The goal is not to fix every item right now — it is to make the risk
visible so new work in these areas gets extra review and regression
coverage.

Conventions used below:
- **Signal** — what a failure looks like from the user's side (crash
  message, wrong output, flaky test).
- **Risk** — the specific reason this code is brittle beyond usual.
- **Mitigation** — tests/asserts/refactors that would make the
  failure visible instead of silent.
- **Recent bugs** — already-fixed issues that exercised this area
  (for context on how it breaks in practice).

---

## 1. `src/store.rs` — word-addressed raw heap

**Files:** `src/store.rs` (~1300 lines), most of `src/database/allocation.rs`.

**What it does:** The entire loft runtime state lives in `Store`
instances — a manually managed word-addressed arena backed by a single
`*mut u8`.  `addr_mut<T>(rec, fld)` computes a byte offset into that
buffer and returns `&mut T`.  Record sizes are encoded as a signed i32
header (positive = claimed, negative = free); a red-black free-block
tree lives *inside* the free blocks themselves (fields FL_LEFT /
FL_RIGHT / FL_COLOR are u32 offsets into unrelated free blocks).

**Signal:**
- `Allocating a used store` panic
- `Database N not correctly freed` in debug builds
- SIGSEGV / `free(): invalid next size` / `malloc(): unaligned tcache
  chunk`
- "value X should be Y" where X is 0xDEADBEEF-ish garbage

**Risk:**
- 257 unsafe blocks and raw pointer dereferences in-tree, the majority
  here.  One mis-counted offset corrupts unrelated records.
- The free-tree rebalance (`fl_rotate_left`, `fl_fix_up`) mutates
  pointers inside free blocks that must become live-data pointers
  after `claim`, and vice versa — any code path that allocates while
  a free-tree iteration is in flight is a potential use-after-free.
- `unsafe impl Send for Store` + `clone_locked_for_worker` share
  pointers across threads.  The guard is entirely per-worker
  `locked = true` discipline.

**Mitigation:**
- `addr_mut` already asserts `!self.locked` in both debug and release
  (S22 fix).
- `LOFT_STORES=warn` prints every allocation / free for post-mortem.
- Best defence is still the regression tests under `tests/issues.rs`
  (P117 / P120 / P121 / P122 / P123 suites) — every leak / UAF that
  got past debug asserts produced a new test there.

**Recent bugs:** P117 (struct-text param leak), P120 (field overwrite
leak), P121 (tuple heap corruption), P122/P123 (loop-store exhaustion).

---

## 2. Hard-coded vector-record layout (`+4` count, `+8` data)

**Files:** `src/vector.rs`, `src/state/io.rs:505`, `src/native.rs`
`populate_frame_variables`, `src/lib/graphics/native/src/lib.rs:908`.

**What it does:** A `vector<T>` record is stored as
`[size i32 | length i32 | element_0 | element_1 | ...]`, so the length
lives at byte offset 4 and the first element at byte offset 8.  Many
call sites write those offsets as raw integer literals rather than
looking them up.

**Signal:** Wrong element counts, vectors that "have length 3" but
their last 2 elements are garbage.

**Risk:** Changing the vector header layout (e.g. adding a capacity
word for amortised growth) requires finding and editing every literal
`+ 4` and `+ 8`.  grep is sufficient today because the pattern is
unique, but nothing *enforces* that someone won't add another raw-byte
path for a new collection type and copy the pattern wrong.

**Mitigation:**
- Introduce `vector::LENGTH_OFFSET = 4` / `DATA_OFFSET = 8` constants
  in `src/vector.rs` and replace all call sites.  One-time, low risk.
- Or, unify all vector reads/writes through the handful of helpers in
  `src/vector.rs` (`length_vector`, `vector_append`, etc.) and delete
  the ad-hoc `set_int(rec, 4, ...)` calls at other paths.

**Recent bugs:** Indirectly P89 (hard-coded StackFrame offsets — same
pattern, different struct; already fixed by schema lookup).

---

## 3. `src/state/mod.rs` raw-pointer thread plumbing

**Files:** `src/state/mod.rs::{execute_at, execute_at_raw, execute_at_ref, static_call}`,
`src/parallel.rs::WorkerProgram::new_state`.

**What it does:** To make `stack_trace()` resolve inside parallel
workers, the main-thread `*const Data` pointer is passed through
`ParallelCtx` into each worker, and each worker copies
`stack_trace_lib_nr`, `data_ptr`, `fn_positions`, and `line_numbers`
into its own `State`.  Four fields, three launchers
(`execute_at{,_raw,_ref}`), multiple construction sites for
`WorkerProgram`.

**Signal:** `stack_trace()` returns empty vec in a worker (silent);
or worse, a `d_nr` from a stale worker pointer indexes into another
thread's `Data` and produces random frame names.

**Risk:**
- Adding a fifth piece of state (e.g. `types` table for printf
  formatting in workers) means editing all three `execute_at_*`
  functions, `new_state`, and every `WorkerProgram { ... }` literal
  — currently 3 in `src/native.rs`, 2 in `src/state/mod.rs`.
- `safe` impls on `WorkerProgram` are manual (`unsafe impl Send`,
  `unsafe impl Sync`).
- The `thread::scope` lifetime guarantee that makes the raw pointers
  safe is not actually enforced by the type system — if someone moves
  worker spawning out of `thread::scope` the pointers dangle.

**Mitigation:**
- Collapse the five fields into a single `WorkerContext` struct so
  `new_state` takes one argument.
- Replace the raw `*const Data` with a reference-counted snapshot or
  an `Arc<Data>` clone — `Data` is append-only after parsing, so a
  single `Arc` shared across all workers is both safe and cheap.

**Recent bugs:** P92 (stack_trace empty in workers).  The fix added
three new fields that each have to be propagated at five sites.

---

## 4. `src/generation/emit.rs` `Str::new(...)` wrap heuristic

**Files:** `src/generation/emit.rs::{Value::Return, output_block::wrap_result}`.

**What it does:** Native codegen represents loft `text` values as
`Str` (owned) or `&str` (borrowed).  Return expressions and block-tail
expressions have to decide whether to wrap the inner expression in
`Str::new(...)` to produce a `Str`.  The current heuristic excludes
user-defined text-returning functions (empty `rust`/`native` fields,
non-`Op` prefix) because those already return `Str`, but *does* wrap
`Op*`-prefixed template calls because those produce `&str`.

**Signal:**
- `expected &str, found Str` or `expected Str, found &str` compile
  errors in generated Rust, often with a long sequence of nested
  Str::new wrappers.
- Bounded-generic T-stubs that silently return STRING_NULL instead
  of their real text value.

**Risk:**
- The "this is an Op template" test is a prefix match on the function
  name.  Renaming an op from `OpGetTextSub` to `GetTextSub` would
  silently move it from the `&str` branch to the `Str` branch.
- `patch_hoisted_returns` now runs for `Type::Text` bodies only when
  the *function name* starts with `t_` (the T-stub convention).  Any
  non-t_-prefixed text function whose IR matches the
  `[Call, OpFreeText, Return(Null)]` pattern will silently drop its
  Call value.

**Mitigation:**
- Replace the "starts with Op" heuristic with an explicit
  `is_template` flag on `Definition`.
- Replace the "starts with t_" gate with a structural check — detect
  the `[expr, OpFreeText(work), Return(Null)]` shape and patch
  wherever it appears.  Or add the flag at definition time.

**Recent bugs:** P86n (native interfaces).  Required three separate
fixes to emit text correctly for bounded-generic interface methods;
each was scoped narrowly to avoid breaking six unrelated test files.

---

## 5. Two-pass parser type resolution

**Files:** `src/parser/definitions.rs` (parse_enum, parse_struct,
parse_typedef, parse_constant, parse_function, parse_interface).

**What it does:** The parser walks every source file twice.  First
pass registers names as `DefType::Unknown` placeholders.  Second pass
resolves references.  In between, `typedef::fill_all` computes field
positions and type sizes; any order-dependence there can desync the
two passes.

**Signal:**
- "Cannot change returned type on X to Y twice was Z" crashes
  (P85b/C56, fixed)
- Parameter count / size mismatches that only appear when one file is
  parsed before another
- "Unknown type" on a name that *is* defined, just further down the
  file

**Risk:**
- `def_nr(name)` falls back from the current source to source 0
  (stdlib).  A user-defined name that collides with stdlib
  short-circuits into the wrong def unless every caller handles the
  cross-source case.  C56 fixed this for 4 definition-introducing
  sites; there are ~10 more `def_nr` call sites in `parser/` that
  assume "if I get a non-MAX result it is mine".
- First-pass side-effects (e.g. `add_def(Unknown)`) depend on being
  balanced by matching second-pass resolution.  An early `return false`
  on a diagnostic can orphan an Unknown that later passes trip over.

**Mitigation:**
- Replace `def_nr(name)` with two explicit helpers
  `local_def_nr(name)` (current source only) and
  `cross_source_def_nr(name)` (fallback) so every call site states
  its intent.
- Add an after-second-pass sanity walk that panics on any remaining
  `DefType::Unknown` — a cheap shield for orphan-from-diagnostic.

**Recent bugs:** P85b (C56), P85c (C57 — nested file-scope keywords).

---

## 6. `src/parser/mod.rs::substitute_type_in_value`

**Files:** `src/parser/mod.rs` lines 1031-1110.

**What it does:** Specialises a bounded-generic template for a
concrete type.  Clones the template's IR, walks it, and rewrites
type-variable references to the concrete type.  Along the way it
fixes up specific opcodes whose embedded arguments become wrong after
substitution — today: `OpGetVector` with `elm_size=0|12` (P136),
`__work_1` trailing arg on text-returning interface methods (I9-text).

**Signal:**
- Bounded-generic function that "works for integers" produces garbage
  for structs, or crashes with `length=0` on a vector of structs.
- Calls to interface methods inside the template silently truncate
  their arg list.

**Risk:**
- The fixups are pattern-matched on `def(d).name`.  Any codegen
  change that renames or splits an op (e.g. `OpGetVector` →
  `OpGetVectorByRef`) bypasses the fixup silently.
- `matches!(&new_args[1], Value::Int(0 | 12))` hard-codes the two
  sentinel element sizes produced by the template compiler.  Add a
  `Type::Long` parameter and the new sentinel (`16`?) is not
  recognised — silent wrong result.

**Mitigation:**
- Detect `needs_element_size` structurally: if `def(d).name ==
  "OpGetVector"` and `new_args[1]` is `Value::Int(_)`, always recompute
  it — never gate on the specific literal value.
- Or, teach the template compiler to emit a sentinel `Value::TypeElementSize(tv_nr)`
  that `substitute_type_in_value` replaces with the concrete size.

**Recent bugs:** P136 (two partial fixes before the final one).

---

## 7. `src/scopes.rs::free_vars` dependency tracking

**Files:** `src/scopes.rs::{free_vars, get_free_vars, scan_set}`.

**What it does:** Decides for each local variable at each scope exit
whether to emit `OpFreeText` / `OpFreeRef` (owned) or skip (borrowed).
The decision is based on the `dep` list on `Type::Reference(_, dep)`,
`Type::Vector(_, dep)`, and `Type::Enum(_, true, dep)` — a list of
attribute indices that name the parameters this variable's storage
depends on.

**Signal:**
- `Database N not correctly freed` in debug builds
- Double-free crashes: `Allocating a used store`
- Memory growth in long-running loops (store pool exhaustion)

**Risk:**
- `dep` is an attribute-index list, not a variable-number list.  Calls
  like `dep_has_var(deps)` translate at read time.  A template
  specialisation that loses the dep (e.g. `Type::Reference(T_nr, []
  )` during clone) produces an owned-looking variable that free_vars
  frees — after the caller already freed it.
- `set_skip_free` is set on a variable in codegen to prevent double-
  free; the flag has to propagate from `stack.function` into
  `data.definitions[def_nr].variables` after all codegen completes
  (S34-style timing).  If the propagation step is skipped for a new
  codegen path, `validate_slots` flags a false conflict and rejects
  the program.

**Mitigation:**
- Track deps as variable numbers everywhere; drop the attribute-index
  mixed representation.  Touches every dep-reading call site but
  eliminates a large class of P136-like bugs.
- Add a post-codegen invariant check: every variable whose type has
  non-empty deps should have a matching `OpFreeRef` emission OR a
  corresponding `skip_free` flag.

**Recent bugs:** P136 (bounded generic for-loop UAF),
 S34 (`skip_free` propagation), P117, P120.

---

## 8. `src/fill.rs` — 233 opcodes, auto-generated, hand-maintained

**Files:** `src/fill.rs` (~6000 lines), `tests/issues.rs::regen_fill_rs`
(ignored maintenance test).

**What it does:** One function per opcode dispatched by `execute`.
The file is regenerated from `#rust"..."` annotations in
`default/*.loft` by running the `regen_fill_rs` test.  Each opcode
pops args from the stack, calls into `src/ops.rs` helpers, and
pushes the result.

**Signal:**
- `pos >= TOS` debug_assert failures
- "stack not correctly cleared" frame-size mismatches
- Silent wrong results when an opcode handler pops the wrong number of
  bytes

**Risk:**
- The file is autogenerated **by an ignored test** (`regen_fill_rs`
  is `#[ignore]` because it regenerates a tracked source file).  A
  change to a `#rust` annotation in `default/` that is not followed
  by a manual `cargo test regen_fill_rs -- --ignored` silently leaves
  fill.rs stale.
- The codegen that reads char-typed bytes from the stack via
  `*s.get_stack::<char>()` was UB and produced release-mode infinite
  loops (P132) — any similar unsafe read of an enum-like type could
  recur.
- Templates that read a text arg via `Str` (12 bytes, 2x i64) vs a
  text return via `String` (24 bytes, 3x i64) use different pops;
  changing a return type from `text` to `&str` in a `#native` binding
  without updating the pop width produces frame-size drift.

**Mitigation:**
- Add a `make fill` target that runs the regen test non-ignored and
  diffs against the committed file; fail CI on mismatch.  This makes
  "you forgot to regenerate" impossible.
- For new native ops, add a runtime frame-size assertion on the first
  call — cheap and catches pop-width drift at startup.

**Recent bugs:** P132 (char sentinel UAF), P129 (duplicate extern
crate), multiple historical fill-regeneration oversights.

---

## 9. `src/parser/collections.rs` `par(...)` + coroutine desugaring

**Files:** `src/parser/collections.rs::{parse_parallel_for_loop,
parse_for_iter_setup}`, `src/parser/builtins.rs` (parallel workers).

**What it does:** Desugars `for e in vec par(r = worker(e), threads) { body }`
into native calls to `n_parallel_for_int/_ref/_text` plus an index
loop over the result vector.  Also compiles the normal `for` into one
of 6+ variants depending on the collection type.

**Signal:**
- "Too few parameters on n_xxx" codegen panic
- "Cannot iterate a hash directly"
- Workers that produce wrong values for structs-with-text-field
- Silent deadlock or `pos >= TOS` when yield is inside par()

**Risk:**
- Many boolean flags (`is_coroutine_iter`, `is_parallel_body`,
  `needs_hidden_text`) that are set during parse and checked during
  codegen.  Missing one toggle in a new construct is a silent-wrong-
  result bug.
- The flat-namespace interaction with `const vector<T>` recursive
  calls + `for` loop (documented in loft-write CAVEATS) is an ambient
  footgun, not a codegen bug — but the parser doesn't warn.

**Mitigation:**
- Emit a compile error (not runtime) when `yield` appears inside a
  `par(...)` body — already tracked as S23; half-implemented.
- Add a golden-IR test suite: `.loft` → dumped IR comparing against
  committed expected JSON for the top 20 `for` / `par` patterns.
  Catches silent IR changes.

**Recent bugs:** P136 (struct vector in bounded generic),
S23 (yield in par body).

---

## 10. `src/lexer.rs` link / revert / cont

**Files:** `src/lexer.rs::{link, revert, cont, peek}`.

**What it does:** Supports arbitrary-depth speculative lookahead.
`link()` returns a `Link` guard; multiple live links cause `cont()`
to remember the token stream in `memory`.  `revert(link)` rewinds
to that position.  Ref counts via `Rc<RefCell<u32>>`.

**Signal:**
- Tokens read "out of order" when speculative parsing is involved
  (e.g. trying `::` qualified names)
- Expected-token diagnostics pointing at the wrong column
- Memory growth during a long file parse

**Risk:**
- `cont()` clears `memory` only when the ref count is 0.  A Link
  dropped without explicit revert while more links are live leaves
  memory growing unboundedly — an OOM waiting for a sufficiently
  pathological source file.
- `revert(link)` takes `Link` by value, drops it, then calls
  `cont()`.  If a user of the link type ever `mem::forget`s it, the
  ref count never returns to 0 and memory is kept forever.

**Mitigation:**
- Add a debug-only `Drop` check on `Link` that logs ref count.
- Bound the `memory` Vec with a cap; panic in debug on overflow
  (catches the forgotten-drop case).

**Recent bugs:** None known — this code is old and battle-tested, but
the invariants are subtle enough that a new recursive-descent
construct adding speculative parse could regress it silently.

---

## Cross-cutting observations

1. **Offset literals `+ 4`, `+ 8`, `+ 12`** appear as a bug-attractor
   across at least 4 different subsystems (stack trace frames, vector
   headers, DbRef struct size, tuple element packing).  Centralising
   them in typed `const` values in their owning module would catch
   most future layout-drift bugs at compile time.

2. **Silent truthy-match on function-name prefixes** ("Op", "n_",
   "t_") is the single most common fragile pattern in codegen and
   parser fixups.  Replacing each with an explicit flag on
   `Definition` would eliminate P86n-class bugs.

3. **Two-pass parser state desync** is the common root for a
   surprising fraction of "Unknown type" / "Cannot redefine X" /
   "Too few parameters" crashes.  A single "definitions finalized"
   checkpoint that asserts no Unknowns and no negative-size attributes
   would convert ~half the current diagnostic zoo into one clear
   error at a known location.

4. **A type predicate asked more than one question.** A `matches!` over `Type`
   variants reads as one fact and is usually serving two: *what is this value*
   and *what may this frame do with it*. `vectors::is_keyed` answered both — the
   COLLECTION KIND (where a `&` link must be peeled, since `(B-Ref-Uniform)`
   makes the link a route and never a shape) and STORE OWNERSHIP (where it must
   not, since a `&` parameter's collection belongs to the caller) — across 76
   call sites. Correcting it for the kind silently changed every ownership answer
   with it, and the two failures wore different clothes: an `unreachable!` in
   `gen_keyed_null`, then a SILENT WRONG ANSWER in a caller two frames down when
   the `op == "="` keyed replace fired through a `&` (loft#1445).

   This is the sibling of the missing-kind class (loft#1291, loft#1292,
   loft#1433) and the harder one to see: a predicate that is too NARROW
   announces itself where it is asked, as an ICE or a lost case, while a
   predicate that is too BROAD announces itself somewhere else entirely.

   Two cures, and the second is the one that lasts. **Find the sites by
   measurement**: give the predicate a temporary env-gated form that computes
   BOTH answers, returns the old one and backtraces when they disagree — one
   run of a boundary matrix then names every call site that would move, which
   inspection over 76 of them cannot. **Then give each question its own NAME**
   (`keyed_kind` / `owns_keyed_store`), rather than leaving a documented choice
   between `base()` and `peel_link()` at each site: a new call site should have
   to type a name that says which question it is asking, because a table in a
   commit message is one reviewer away from being re-conflated.

5. **Golden-IR regression tests** — we have golden PNGs for visual
   output (Brick Buster) and dump files for bytecode execution, but
   nothing pins the *shape* of the IR produced by the top parser
   constructs.  A small suite would catch silent IR drift that
   currently only shows up when codegen or scope analysis then trips
   over it.

## See also

- [PROBLEMS.md](PROBLEMS.md) — currently-open issues (most of the
  brittle surfaces above have at least one entry).
- [CODE.md](CODE.md) — naming / safety rules that apply across all of
  the above.
