
# Code Rules

Rules for all Rust and loft code in this project.

---

## Contents
- [Naming](#naming)
- [Functions](#functions)
- [Doc Comments](#doc-comments)
- [Test Suite (`tests/docs/`, `tests/scripts/`)](#test-suite-testsdocs)
- [Clippy and Formatting](#clippy-and-formatting)
- [Null Sentinels](#null-sentinels)

---

## Naming

- Names of functions, variables, arguments, and fields must be self-documenting — short but unambiguous.
- User functions in loft are stored with an `n_` prefix: `data.def_nr("n_foo")`, not `data.def_nr("foo")`.
- Native stdlib functions follow the scheme `n_<func>` (global) or `t_<LEN><Type>_<method>` (method, LEN = type name length). Example: `t_4text_trim` for `text.trim()`.
- Internal-only functions use an `i_` prefix (e.g. `i_parse_errors`). Registered in `Parser::new()`, not in `default/*.loft`. Invisible to user code — no namespace collision.
- Operators use `OpCamelCase` in loft source → bare `snake_case` in Rust (`fill.rs`), without any prefix. Exception: `OpReturn` → `op_return`, because `return` is a Rust keyword.

**Adding an `fn Op*` to `default/*.loft` means `make fill` — and the symptom of
forgetting is not a mismatch.** `fill.rs`'s `OPERATORS` is a POSITIONAL array, so a
new opcode shifts every operator declared after it by one. The interpreter then
dispatches each of those to its neighbour's implementation, and what you see is a
SIGSEGV in a program that has nothing to do with your change — adding
`OpLengthTrie` broke `for p in spatial<…>`. `tests/issues.rs::fill_rs_up_to_date`
catches it, so run the suite before believing an ad-hoc run; if you are already
staring at the crash, the fastest oracle is the INSTALLED `loft`, which answers
correctly on the same file.

## Functions

- One algorithm per function. Extract helpers to avoid duplication.
- Group fields that always travel together into a struct.
- No functions longer than ~50 lines; split if the cognitive complexity warning fires.

## Doc Comments

- Describe *why to use* the function (preconditions, trade-offs, when to use), not *what* it does and not *why it was written*. Link to a doc or issue for the design reason. See [DOC_QUALITY.md](DOC_QUALITY.md).
- Inline comments only where the algorithm is non-obvious. Avoid restating what the code says.

## Test Suite (`tests/docs/`, `tests/scripts/`)

- Each `.loft` file is a living language example as well as a test.
- Every section should have a `// comment` explaining what it exercises and why.
- Tests use `assert(condition, "message")` — the message is the failure label.
- `@NAME: title` and `@TITLE: description` headers are required for documentation generation.

## Clippy and Formatting

- No clippy warnings. The crate root sets `#![warn(clippy::pedantic)]`.
- `cognitive_complexity` (from `clippy::nursery`, not included in `pedantic`) is used selectively; suppress it per-function with `#[allow(clippy::cognitive_complexity)]` only for functions that are structurally complex by necessity (e.g. per-opcode match arms).
- Use `#[allow(clippy::...)]` only when the linter false-positives; always include a comment explaining why.
- Code is formatted with `rustfmt`. No manual formatting overrides.

## Null Sentinels

- Integer null: `i64::MIN` for a full-width `integer`. A narrow alias spends an EDGE of its
  own range, and which edge follows the sign: an unsigned one gives up its top value (`u8?`
  is `0..=254`), a signed one its bottom (`i8?` is `-127..=127`). Float null: `f64::NAN`.
  Reference null: `store_nr == 0 && rec == 0`.
- **The edge is spent only in the `τ?` form, and only where the range fills the width.** A
  non-null `u8` uses all 256 codes, so `255` is a real `u8`; an `i32?` and an `integer
  limit(0,255)?` have a spare code outside their range and give up nothing.
  `IntegerSpec::usable_min` / `usable_max` is the one home that answers which spec spends
  what.
- **The reservation is a property of the TYPE**, so it holds wherever a value is KEPT — local,
  field, element, parameter, return alike (`formal/types.md` `@FR-N-Reserve`). An expression
  in FLIGHT is not a slot and spends nothing: `e as u8?` yields `255` and `(e as u8?) ?? d`
  keeps it, because neither ever holds a `u8?`. `expressions::target_holds_null` is the one
  home for which is which — an element write on a non-null `vector<u8>` presents its target
  as `u8?` for the out-of-bounds MISS, and that is not a nullable slot. loft#1249 records two
  cures that got this backwards and what each broke.
- All arithmetic operations must propagate null (if either operand is null, result is null).
- Never use `0` as a sentinel for integers or references in new code.
- **A not-found answer must not be usable as an index, an offset, a length or a count.**
  Diagnose the miss where it is produced, or route it into a message that names the type and
  the field. `u16::MAX` flowing out of a lookup has cost this project a 59.6 GiB allocation
  (loft#796, used as an offset) and an unattributable `index out of bounds: … the index is
  65535` (loft#977, used as a type-table index).
- **When one fact has several resolvers, they must agree on what a miss looks like.** Ask what
  EVERY resolver answers, not just the one that crashed: in loft#977 the same missed field
  gave `field_type` → `u16::MAX` (out of range, so it panicked) and `field_nr` → `0` and
  `field_ref` → the record base (both in range, so they were silently wrong). The loud one
  fired first and hid the other two; one more entry in the type table and there would have
  been no panic at all, only writes landing in the wrong place.

---

## Dependencies

Prefer the standard library and existing project code over adding new Cargo dependencies.

### Decision rule

Before adding a dependency:
1. **Check if existing code covers it.** Loft already has JSON text parsing (`src/database/structures.rs`) and JSON serialisation (`src/database/format.rs`). New JSON functionality belongs in `src/database/json.rs`, not `serde_json`.
2. **Estimate the implementation size.** If the needed functionality is ≤ ~100 lines of straightforward Rust, write it. If it requires thousands of lines of platform APIs (TLS stacks, image codecs, memory-mapped I/O), a dependency is justified.
3. **Feature-gate optional dependencies.** Any dependency that adds compile weight or is unused for core interpreter work must be behind a Cargo feature (following the `png`, `mmap`, `random`, and planned `http` pattern).
4. **Prefer crates with minimal transitive dependencies.** `ureq` and `png` have no required transitive deps. Avoid crates that pull in async runtimes, proc-macro infrastructure, or heavy frameworks.
5. **Never add a dependency to replace < 100 lines of existing-style code.** Adding `serde_json` for seven JSON field-extraction functions that fit in ~80 lines is the wrong trade-off.
6. **`serde` is forbidden, permanently.** Never add `serde` / `serde_derive` / `serde_json` / `bincode`, and never `#[derive(Serialize, Deserialize)]`. Serialise by hand. See [§ serde is forbidden](#serde-is-forbidden--never-add-it) below.

### Approved dependencies

| Crate | Feature | Justification |
|---|---|---|
| `png` | `png` | PNG codec; ~5 000 lines of DEFLATE + filter logic; not worth writing |
| `mmap-storage` | `mmap` | Memory-mapped file I/O; OS-specific unsafe APIs per platform; not worth writing |
| `rand_core` + `rand_pcg` | `random` | PCG PRNG; cryptographic-quality randomness in ~300 lines; acceptable scope |
| `dirs` | (always) | Platform home-dir lookup; 3 lines of OS APIs per platform; worth the abstraction |
| `stdext` | dev-only | Test utilities; zero production footprint |
| `ureq` | `http` (planned H4) | Blocking HTTP client + TLS; ~3 000 lines of platform APIs; not worth writing |

### Not approved

| Crate | Reason |
|---|---|
| **`serde` / `serde_derive` / `serde_json` / `bincode`** | **Forbidden, permanently — see the rule below. Do not add `serde` as a dependency or `#[derive(Serialize, Deserialize)]` anywhere in native code.** |
| `tokio` / async runtimes | Loft is synchronous; no async use case exists |
| `clap` | CLI arg parsing; 10 lines of `std::env::args()` suffices |
| `log` / `tracing` | Loft has its own `logger.rs` tailored to the runtime model |

### serde is forbidden — never add it

**`serde` (and `serde_derive` / `serde_json` / `bincode`) must never be
a dependency of the interpreter, and `#[derive(Serialize,
Deserialize)]` must never appear on any project type.** This is a hard
rule, not a per-case judgement.

Why, concretely:

- **It does not fit the IR.** The compiler's core types
  (`Value`, `Type`, `Block`, `Definition`, `Data`) carry `&'static str`
  fields (`Block.name`, `Definition.synthetic`) and a non-derivable
  `OnceLock` index. serde-derive injects a `'de: 'static` bound from
  the `&'static str` fields that propagates up the entire recursive
  graph and fails to compile; the `OnceLock` needs `#[serde(skip)]` +
  manual rebuild. The derive fights the data model at every turn.
- **It is unnecessary.** Everything loft needs to serialise (the
  startup `Data`/bytecode cache, store images, the JSON surface) is
  better done by **hand-rolled, length-prefixed little-endian
  encoding** — the approach already used by the store engine
  (`src/store.rs`), the database JSON parser/formatter
  (`src/database/`), and the retired bytecode cache. Hand encoding is
  smaller, has zero proc-macro build cost, lets us skip rebuildable
  fields (HashMap indices) instead of annotating them, and keeps the
  on-disk format explicit and versioned.
- **It is dead weight.** serde pulls in proc-macro infrastructure that
  bloats compile time for a capability the project already covers.

The lone historical exception was `serde-wasm-bindgen`, required by the
`wasm-bindgen` ecosystem at the WASM boundary — and even there loft
passes **plain JSON text strings**, using no derive macros. If a future
WASM need forces `serde` transitively, it stays strictly behind the
`wasm` feature and never reaches native code or the IR types. Native
builds must have zero serde in the tree.

When you need to serialise a Rust type: write `to_bytes` / `from_bytes`
by hand (see `src/store.rs` and the `src/cache.rs` startup-cache
encoder for the pattern), keyed/versioned so a format change is
detected rather than misread.

## Shell scripts

Three shapes that read as correct and are not, each measured in this repo's own scripts:

- **`"$A$'\n'$B"` does not insert a newline.** Inside double quotes `$'\n'` is the five
  characters `$ ' \ n '`, so the last row of `$A` fuses with the first row of `$B` into one
  unmatchable line and BOTH leave the list — positionally, so whichever row sits last is the
  one that silently stops being an exception (`scripts/sync-fixtures.sh` reported permanent
  drift on a file it had declared exempt one list above). Close the quote first:
  `"$A"$'\n'"$B"`.
- **`PIPESTATUS` is bash-only, and `/bin/sh` is `dash` on Debian-family boxes.** A
  `sh -c '… | head; echo ${PIPESTATUS[0]}'` check compares an empty string and passes while
  measuring nothing. Use `#!/usr/bin/env bash` with `set -o pipefail`, or build the pipe
  yourself.
- **Never wait on a process NAME whose text is inside the waiting script, and never `pkill`
  by name on a shared box.** `until ! pgrep -f "make ci"` never exits: the poller's own
  `bash -c '…'` command line contains `make ci`. `pkill -f "make ci"` matched — and killed — a
  sibling checkout's run. Wait on an artefact or a pid file instead (`make ci` records its run
  in `.ci-running`), stop a run through the tool that started it
  (`scripts/find_problems.sh --stop`), and before acting on a "concurrent run" claim read
  the candidates' `readlink /proc/<pid>/cwd`.

  ⚠ **The `[m]ake` bracket works, and that is exactly why it is not a rule you can rely on.**
  Measured 2026-09-07: a bracketed waiter exits on the first poll, an unbracketed one loops
  forever. What defeats the bracket is the PLAIN string appearing anywhere else on an ancestor's
  command line — a second command in the same invocation, an `echo`, a log path — because then
  the ancestor's argv carries the unbracketed text for the bracketed regex to match. That is
  invisible and easy: the measurement above read "bracket self-matches" on its first attempt
  because both forms were in one command. A waiter that is correct today breaks when someone
  adds an `echo` beside it, so **wait on a recorded PID** — `scripts/ci-run.sh status`, or the
  pid `find_problems.sh --bg` prints — which has no pattern to defeat.

  ⚠ **An ALTERNATION defeats the bracket one branch at a time, and that is how it actually
  bites.** Measured 2026-09-07: `pgrep -f "wasm-pack|[m]ake wasm"` reported "still building" for
  eight minutes after `make wasm` had finished, because only the SECOND branch was bracketed —
  the waiter's own argv contains the literal `wasm-pack`, so the first branch matched the
  waiter. Every branch needs its own bracket, which is precisely the sort of detail that is
  right when written and wrong after one edit.

  **The check with no pattern at all is to ask the ARTEFACT, not the process.** `ls
  --time-style=+%H:%M:%S <output>` against `date` answers *is it done?* directly, cannot
  self-match, and is what finally caught that eight-minute stall.

  `pgrep -x` is not the way round it either, and it is WORSE rather than merely inadequate. It
  matches `comm`, which the kernel truncates to `TASK_COMM_LEN` — 16 bytes including the NUL —
  so a name past 15 characters can never match and the check silently always passes. Measured
  from both ends: `pgrep -x` on a 23-character name finds nothing while the same `-x` against
  its truncated `comm` matches, and `pgrep` itself warns — on a tty, which is exactly where a
  scripted waiter is not.

  **The failure MODES are what rank the two.** `-f` self-matching fails LOUD: the loop blocks
  and you go looking. `-x` past 15 characters fails silent and closed — a liveness check that
  reports "finished" the whole time. Reaching for `-x` after being burned by `-f` is reaching
  for the next pattern instead of stopping using patterns, which is the actual lesson: use the
  recorded pid.

- ⚠ **`git merge-base --is-ancestor <sha> HEAD` is not the test for "do I have this CHANGE".**
  It answers about COMMITS, and the moment any checkout cherry-picks, the same change exists
  under a different sha and every ancestry test reads MISSING for the rest of time.  The test
  is `git show <sha> | git patch-id --stable` on both sides, compared.  Measured 2026-09-07: a
  peer reported that `tuxedo-1361-tuple-copy` lacked `cbbea3cd2` (loft#1407's use-after-free
  fix) and therefore carried a live `(H-View)` UAF; I re-ran their check, got the same
  NOT-AN-ANCESTOR, and confirmed it.  Both wrong — the tree had it as `c0c2a9cb`, and both
  shas give patch-id `d935f9a4ec628af42b5d8f8b277d251a18edb63d`.  What caught it was the
  cherry-pick producing an EMPTY patch, not either agent's reasoning.  Across checkouts that
  continuously pick from each other, "do I have this commit" is almost never the question
  being asked.  **And the second-order lesson costs more than the first: verifying a peer by
  re-running the peer's own method can only ever confirm them.  Verification means changing
  the instrument.**

- ⚠ **Never kill a `make ci` mid-compile — and if you must, clear the incremental dir in the
  same breath.**  A killed compile leaves `target/debug/incremental` inconsistent, and the NEXT
  build fails at link with undefined `core::ptr::drop_glue::<…>` and `anon.<hex>.llvm.<hex>`
  referenced from `.rcgu.o` files.  Those names appear in NO source file, so the failure reads
  like a link or codegen defect and cannot be grepped for.  The cure is `cargo clean -p loft`
  and nothing smaller: hand-globbing `target/debug/deps/loft-*` does NOT match `libloft.rlib`,
  so the stale rlib survives and the next gate fails identically with DIFFERENT symbols
  (`loft::vector::get_vector`, `loft::keys::explain_enabled::ON`, now referenced from inside
  the rlib).  Measured 2026-09-07: three gates, two of them wasted, and a 66 GiB clean.  A
  second gate is cheaper than a corrupted target dir, so prefer letting one finish.

- ⚠ **`CI-RESULT: FAILED` with a `FAIL [` count of ZERO is never the code under test.**  It is
  the toolchain, the box, or the target dir — a link failure, contention, a corrupt incremental
  cache.  Read the count BEFORE reading a single line of the error text; the verdict line's
  captured `error[` is frequently a package you never touched.  Measured 2026-09-07: this
  distinction was re-derived from error text three times in one evening because the count was
  only reachable by grepping.  `scripts/ci-run.sh` now names the failing TEST and how many.

---

## See also
- [TESTING.md](TESTING.md) — Test framework, LogConfig debug-logging presets, suite files
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [DEVELOPMENT.md](DEVELOPMENT.md) — Contribution workflow and validation against CODE.md
