// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Developer Guide — Adding Features to Loft

This guide is for contributors who want to add new language features, fix bugs, or
extend the standard library. It covers the process for proposing and implementing
changes, quality gates that must be cleared, how to avoid scope creep, and how the
existing pipeline works so you can locate the right place to make a change.

---

## Contents
- [Language Goals — the north star](#language-goals--the-north-star)
- [Quality Requirements](#quality-requirements)
- [Scope Gate — What Belongs in Loft](#scope-gate--what-belongs-in-loft)
- [Backwards Compatibility](#backwards-compatibility)
- [Feature Proposal Process](#feature-proposal-process)
- [How the Pipeline Works](#how-the-pipeline-works)
  - [1. Lexer](#1-lexer-srclexerrs)
  - [2. Two-Pass Parser](#2-two-pass-parser-srcparser)
  - [3. Type Resolution](#3-type-resolution-srctypedefrs)
  - [4. Scope Analysis](#4-scope-analysis-srcscopesrs)
  - [5. Variable Liveness](#5-variable-liveness-srcvariables)
  - [6. Bytecode Stack Tracker](#6-bytecode-stack-tracker-srcstackrs)
  - [7. Bytecode Generation](#7-bytecode-generation-srccompilers--srcstate)
  - [8. Operator Dispatch & Execution](#8-operator-dispatch--execution-srcfillrs)
  - [9. Data Store & Type Schema](#9-data-store--type-schema-srcdatabase)
  - [10. Standard Library (native functions)](#10-standard-library-srcnativers--defaultloft)
- [Known Caveats by Subsystem](#known-caveats-by-subsystem)
- [Debugging Strategy](#debugging-strategy) *(see [claude/DEBUG.md](claude/DEBUG.md) for full guide)*

---

## Language Goals — the north star

Every proposed change must serve at least one of these four goals. If it does not,
reject it before implementation starts.

1. **Correct.** Programs produce the right answer or a clear error — never silent
   wrong results. This includes null safety, overflow detection, type safety, and
   memory management. Correctness wins over every other goal.

2. **Prototype-friendly.** A new developer must be able to express an idea with
   minimal ceremony: concise syntax, clear error messages, a runtime that reports
   problems and exits meaningfully. Any friction that slows down the *first hour* of
   use is a correctness-class defect for the language itself.

3. **Performant at scale.** Allocation, collection lookups, and parallel execution
   must stay efficient as data grows. New features must not regress existing
   benchmarks by more than 5%.

4. **Architecturally clean.** The compiler and runtime must remain free of technical
   debt that makes the next feature hard to add. Code quality is a first-class concern.

The project's full goal set, and what each goal is measured by, is
[GOALS.md](claude/GOALS.md).

---

## Quality Requirements

Before any change is merged, it must satisfy all of the following.

### Tests first
Every feature requires:
- At least one positive test (correct program uses the feature successfully).
- At least one negative test (a diagnostic test in `tests/parse_errors.rs`, or a
  `tests/scripts/*.loft` file with `@EXPECT_ERROR`, that confirms the right error is
  produced for misuse).
- For collection or runtime features: a large-N stress test (N ≥ 100) covering
  add, lookup, remove, and iteration.
- A corresponding example in the relevant `tests/docs/*.loft` file.

### Clean build
`make clippy` must pass without warnings. This runs `cargo fmt --check`,
`cargo clippy --tests -- -D warnings`, and `cargo check --no-default-features`.

### Full test suite
`make ci` (fmt → clippy → test) must pass; `make loft-test` runs every
`tests/docs/*.loft` file. No test may be moved to `#[ignore]` without a GitHub issue that
records its severity and workaround ([ISSUE_TRACKING.md](claude/ISSUE_TRACKING.md)).

### Error quality
New parse or type errors must:
- State *what* is wrong and *where* in the source (file + line).
- Include the token or expression that triggered the error.
- Suggest the fix when the fix is unambiguous.
- Not panic. Use the diagnostic system (`src/diagnostics.rs`); prefer `Warning` or
  `Error` over `Fatal` unless the compiler cannot continue.

#### Diagnostic positioning

The diagnostic system (`src/diagnostics.rs`) offers these entry points:

| Method | Position source | When to use |
|--------|----------------|-------------|
| `diagnostic!(lexer, level, msg)` | Current `lexer.position` | The lexer is still at the problem |
| `specific!(lexer, &lex_result, level, msg)` | Saved `LexResult.position` | You hold the exact token that caused the error |
| `diagnostic_at!(lexer, &pos, level, msg)` / `lexer.pos_diagnostic(level, &pos, msg)` | Explicit `Position` | Detection happens after the construct is parsed — use the construct's own saved position |

`diagnostic!` and `diagnostic_at!` also take a code — `diagnostic!(lexer, level, code =
"kebab-slug", msg)` — the stable name of the diagnostic; a new code lands with its row in
[DIAGNOSTICS.md](claude/DIAGNOSTICS.md) ([§ Adding a code](claude/DIAGNOSTICS.md#adding-a-code)).

The cursor runs one token ahead of what the parser has decided about, so a check that runs
once a construct is complete points at the next token unless it is given a saved position.
Save `lexer.at()` (or the node's `Position`) *before* the parser advances past what the
user should see underlined, and report with `diagnostic_at!`.  A seek with
`lexer.to(saved)` moves the reporting position for later diagnostics too; DIAGNOSTICS.md
§ Adding a code, items 4–6, has the rules.

One known position to improve: an operator type error (`"x" - 3`) points at the right-hand
operand, not the operator.

Add a test in `tests/parse_errors.rs` that asserts the message and its exact
`file:line:col`.

#### Diagnostic message style

- Start with an uppercase letter.
- Name identifiers in single quotes: `'variable_name'`.
- Name types without quotes: `integer`, `vector<text>`.
- Use `—` (em dash) to separate the problem from the suggestion.
- End without a period.
- Suggest the fix when it is unambiguous (`"...; add the missing variants or a '_ =>' wildcard"`).

Messages that do not follow the style yet: `"expect variant name ..."` in
`src/parser/control.rs` and `src/parser/operators.rs` (lowercase), `"set_file_size is not
defined"` in `src/parser/operators.rs` (unquoted name), and `"Cannot iterate; expected
vector, sorted, index, text, or range"` in `src/parser/fields.rs` (does not name the type
it got).

Parse recovery after a failed token uses `Lexer::recover_to(&[...])`, which skips to
one of the given tokens so one mistake does not cascade into many errors.

Every diagnostic change must:
- [ ] Have a test in `tests/parse_errors.rs` asserting the exact message and position
- [ ] Not break any existing tests
- [ ] Not introduce new `panic!` paths
- [ ] Follow the style above

### Documentation
All user-visible syntax or standard library additions must be documented in the
appropriate `tests/docs/*.loft` file. Changes to compiler internals must be reflected
in the relevant `doc/claude/*.md` document.

---

## Scope Gate — What Belongs in Loft

Loft is a **systems scripting language** with a focus on data processing, file I/O,
structured logging, and parallel computation. The following questions determine
whether a feature belongs in the language itself versus a library:

| Question | In core language if… | In library otherwise |
|----------|----------------------|----------------------|
| Does it require new syntax? | Yes — syntax must be in the core | No — a function call is enough |
| Does every program need it? | Yes — it is in the default library | No — it is in an optional library |
| Does it require runtime support (new opcodes)? | Yes | No |
| Is it about I/O, collections, or types? | Core concern | May belong in a library |

### Scope creep red flags

Stop and reconsider if you find yourself:
- Adding a second syntax form for something that already works (convenience additions
  accumulate and become maintenance burden).
- Importing a new Cargo dependency for a single feature (every dependency is a
  forever commitment to version management).
- Writing more than ~200 lines of new Rust code for a feature that has no test yet.
- Changing the IR (`src/data.rs`) just because it would be slightly more convenient
  — IR changes cascade through the parser, codegen, scope analysis, and debug dumps.
- Adding a flag or configuration option instead of making the right behaviour the
  default.

### Out of scope

- Async/await.  Loft has synchronous generators (`yield`,
  [COROUTINE.md](claude/COROUTINE.md)); async generators are out of scope
  (COROUTINE.md § Relationship to Rust's `gen` / `async gen`).

Closures that capture, the REPL (`loft repl`), user generics (`fn f<T>(…)`,
`tests/docs/25-generics.loft`) and C interop (`#c` bindings,
[PACKAGES_C_BINDING.md](claude/PACKAGES_C_BINDING.md)) exist.  Features the project
declined are in [DESIGN_DECISIONS.md](claude/DESIGN_DECISIONS.md); check it before
proposing one, and get agreement before touching code.

---

## Backwards Compatibility

The policy is [COMPATIBILITY.md](claude/COMPATIBILITY.md).  In short:

### Before contract 1
- Loft is at contract 0: breaking changes are allowed, but deliberate.  A new refusal
  that reaches a consumer's code is recorded with its cure in COMPATIBILITY.md
  § The cures consumers have needed before contract 1, and the change is listed in
  `CHANGELOG.md`.
- Any change that alters the **runtime output** of a correct program is a breaking
  change even if the program still compiles — treat it the same way.
- All `.loft` files in `tests/docs/` and `tests/scripts/` serve as the backwards-
  compatibility corpus. They must pass after every change.

### From contract 1 (the 1.0 freeze)
- A program that works keeps working, identically, on every later loft.  Evolution is
  additive: nothing a working program depends on is removed.
- The set of programs that raise an error may only shrink; error *text* is not frozen.
  A diagnostic's stable name is its code.

### Operator numbering
Opcode numbers are assigned in the order the operators are declared in `default/*.loft`,
and are the indices into `OPERATORS` in the generated `src/fill.rs`.  Adding an operator
anywhere but at the end renumbers every later opcode.  After adding one, regenerate
`src/fill.rs` (`make fill`) and `index/target_surface.json` (`make surface-gen`); CI
fails on either when it is stale.  Cached bytecode is keyed by the loft build, so an old
cache is never reused.

---

## Feature Proposal Process

1. **Write the user-facing spec first.** Add a loft code example to the relevant
   `tests/docs/*.loft` file (or create a new one). Write what the program should do
   in a `// comment`. This forces you to think about syntax and error cases before
   touching Rust.

2. **Identify the change scope.** Read the pipeline overview below and mark which
   subsystems need to change. If more than four subsystems are involved, the feature
   is large — split it.

3. **Write the negative test.** Add a test to `tests/parse_errors.rs` for at least
   one misuse scenario before writing any implementation. This confirms the error path
   will work.

4. **Implement in pipeline order.** Start from the earliest stage that needs to
   change (usually the parser) and work downstream. Do not skip stages.

5. **Run `make test` after each stage.** A green test suite at each checkpoint is
   cheaper than debugging cross-stage interactions later.

6. **Update documentation last.** Once tests pass, update the relevant `doc/claude/`
   file and add or expand the `tests/docs/*.loft` example.

---

## How the Pipeline Works

This section describes each subsystem: what it does, what files are involved, and
the most common mistakes made when modifying it.

### 1. Lexer (`src/lexer.rs`)

**What it does.** Converts raw UTF-8 source text into a stream of `LexItem` tokens:
integers, floats, strings, identifiers, and punctuation. Supports backtracking via
`link()` / `revert(link)` so the parser can speculatively try grammar alternatives.

**Key methods:**
- `cont()` — advance to the next token.
- `has_token(s)` / `has_identifier()` / `has_integer()` — conditional advance.
- `peek()` / `peek_token()` — look ahead without consuming.
- `link()` / `revert(link)` — save and restore position for speculative parsing.

**When to modify.** Only when new literal syntax is needed (new numeric suffixes, new
escape sequences, new string-interpolation forms). Most feature additions do not touch
the lexer.

**Caveats.**
- The `in_format_expr` flag allows `\"` inside `{...}` format
  expressions. If you add new string contexts, mirror this pattern — a boolean flag
  on `Lexer` that changes how inner tokens are classified.
- `link()` buffers tokens in memory. Do not link across large spans of source;
  revert immediately after the speculative parse succeeds or fails.
- Keyword detection is case-sensitive and exact — a typo in a keyword string silently
  makes it an identifier.

---

### 2. Two-Pass Parser (`src/parser/`)

**What it does.** Converts the token stream into a `Value` IR tree in two passes.
Pass 1 (first_pass=true) registers all top-level names — struct definitions, enum
definitions, function signatures — without generating full IR. Pass 2 generates
complete, fully-typed IR using the name registry built in pass 1.

**Files:**

| File | Responsibility |
|------|----------------|
| `src/parser/mod.rs` | Top-level loop, type coercion (`convert`, `cast`), the operator precedence table (`OPERATORS`), two-pass coordination |
| `src/parser/definitions.rs` | `fn`, `struct`, `enum`, type parsing (`parse_type`), `enum_fn` (polymorphic dispatch synthesis) |
| `src/parser/expressions.rs` | Expressions, assignments, format strings |
| `src/parser/operators.rs` | Binary operators (precedence climbing, `parse_operators`), operator dispatch and coercion |
| `src/parser/fields.rs` | Postfix `.field` and `[index]` |
| `src/parser/objects.rs` | Variable references (`parse_var`), file operations, enum variant values |
| `src/parser/vectors.rs` | Appends (`parse_append_vector`), lambdas and captures (`parse_lambda`), fn-refs |
| `src/parser/collections.rs` | Iterators, `for` loops, `par(...)` loops, comprehensions |
| `src/parser/control.rs` | `if`/`else`, `match`, `break`, `continue`, `return`, function calls |
| `src/parser/dispatch.rs` | Choosing among overloads of one name |
| `src/parser/builtins.rs` | Parallel worker helpers |

**When to modify.** For any new syntax — new control flow, new collection operations,
new built-in expressions, new literal forms.

**Key patterns:**

- **`first_pass` guard.** Functions that generate IR must check `if self.first_pass { return early; }`. Pass 1 must only register names; IR it generates is discarded and replaced in pass 2.
- **`vars.add_variable(name, type, lexer)`** — declares a new variable and returns its number `v_nr`.
- **`vars.var(name)`** — looks up a declared variable by name.
- **`Value::Call(fn_nr, args)`** — call a known function by definition number.
- **`Value::Block(block)`** — a scoped sequence of statements (variables declared inside are freed at block exit).
- **`Value::Insert(stmts)`** — an inline sequence without a new scope (used for generated code).

**Caveats.**
- Pass 1 is lenient: it allows unknown types and unresolved names. Pass 2 must be
  strict. If you see a correct program fail in pass 2 with "Unknown variable" or
  "Unknown function", the name was not registered in pass 1.
- `convert()` and `cast()` apply implicit conversions (e.g. `integer` → `long`).
  They must be called on every expression result that flows into a typed context.
  Forgetting them causes type mismatches at runtime without a compile-time diagnostic.
- `parse_operators(…, precedence)` uses precedence climbing. New binary operators must
  be given a precedence level in the `OPERATORS` table in `src/parser/mod.rs`.
- The `first_pass` boolean is a field on the parser struct (`self.first_pass`). There
  is no type safety enforcing which operations are legal in which pass. Be vigilant.

---

### 3. Type Resolution (`src/typedef.rs`)

**What it does.** After pass 1, replaces all `Type::Unknown(id)` placeholders with
concrete types, registers each struct/enum layout in `Stores`, and computes field
byte offsets. Runs between the two parser passes.

**Functions:**
- `actual_types_deferred()` — resolves forward references.
- `fill_database()` — registers each type with the `Stores` schema.
- `fill_all()` — reports type cycles and lays the records out (`Stores::finish`).

**When to modify.** When adding a new composite type (e.g. a new collection variant).
The new `Parts` variant must be registered here.

**Caveats.**
- Type cycles (struct A contains B contains A) are detected by `has_value_cycle()`
  in `fill_all()` and produce a compile error. Only value-typed struct fields are
  checked; `reference<T>` fields (pointers) are fine and do not cause cycles.
- Pass 2 can still lay out a type: a struct whose field names a generic declared
  below it (`lay_out_late`), or a synthetic type the parser creates
  (`register_and_lay_out_synth`).  The runtime assumes the schema is immutable once
  execution begins.

---

### 4. Scope Analysis (`src/scopes.rs`)

**What it does.** Walks the IR tree and:
1. Assigns a scope number to each variable.
2. Inserts `OpFreeText` / `OpFreeRef` at scope exit for owned types (Text,
   Reference, Vector).
3. Pre-initialises variables that are first assigned inside a branch (so the stack
   slot is valid regardless of which branch executes).

**Entry point:** `check(data, database)` — iterates all functions.

**When to modify.** When adding a new owned type (a type that must be freed when it
goes out of scope), or when changing how branches or loops interact with variable
lifetimes.

**Caveats.**
- This is the most fragile subsystem. The `var_mapping` table (which maps a variable
  slot to a "copy" slot when a name is reused across sibling scopes) is hard to
  reason about. Read `SLOTS.md` before touching it.
- Pre-init for reference types first assigned inside a branch is handled by
  `find_first_ref_vars` (with `deps_ready` guard for borrowed types).  When adding a
  new owned type, write a test that assigns it first inside an `if` branch to verify
  pre-init works.
- Bugs in scope analysis manifest at runtime (wrong `store_nr`, double-free, garbage
  pointers) — not at compile time. They are hard to diagnose. Add a new test to
  `tests/slots.rs` for any new variable interaction you add.
- The `ref_debug` LOFT_LOG preset is the primary tool for diagnosing scope bugs.
  Enable it by setting `LOFT_LOG=ref_debug` in the environment before running a test.

---

### 5. Variable Liveness (`src/variables/`)

**What it does.** Computes, for each variable in each function, the sequence numbers
of its first definition (`first_def`) and last use (`last_use`). These intervals are
used by `validate_slots` to detect stack slot conflicts (two overlapping variables
assigned the same slot).

**Key functions:**
- `compute_intervals(val, function, ...)` (`intervals.rs`) — walk IR; populate `first_def`/`last_use`.
- `validate_slots(function, data, def_nr, scope_blind)` (`validate.rs`) — post-codegen conflict checker; runs in debug-assertion and test builds and panics on a conflict.
- `size(tp, context)` (`mod.rs`) — bytes needed for a type on the stack.

**When to modify.** When adding a new type that occupies stack space, update `size()`
to return the correct byte count. If the new type is owned, ensure that `OpFree*`
sequences extend the `last_use` of the variable correctly (see how `free_text_nr` and
`free_ref_nr` are handled).

**Caveats.**
- `first_def` and `last_use` are global per function — they span the entire function,
  not just the lexical scope. This is a conservative approximation. Two variables
  may appear to overlap even if they are never simultaneously live. The exemption for
  same-name/same-slot pairs in `find_conflict` handles the common case.
- `size()` must agree with `Stack::size_code` in `src/stack.rs` (§ 6). If you add a
  new type, update both. A mismatch causes silent stack corruption.

---

### 6. Bytecode Stack Tracker (`src/stack.rs`)

**What it does.** During bytecode emission, maintains an accurate byte count of the
current operand stack depth. Ensures that `OpFreeStack` amounts are correct and that
`break` targets are reached with the right depth.

**Key methods:**
- `add_op(name, state)` / `operator(d_nr)` — emit an opcode / update the position after it.
- `add_loop(code_pos)` / `end_loop(state)` — manage loop frames and patch `break` gotos.
- `add_break(code_pos, loop_nr)` — register a pending break.

**When to modify.** When adding a new opcode that pushes or pops stack values,
`operator()` must return the correct stack delta for the new opcode.

**Caveats.**
- Stack depth mismatches (too many pops, too few pops) cause silent corruption at
  runtime. A release build has no stack-underflow check; `LOFT_VERIFY_STACK=1`
  (`--interpret`) reports a read of a stack slot that nothing wrote.
- `size_code(val)` computes how many bytes a `Value` IR node produces — it partially
  overlaps with `variables.rs::size()`. Both must agree for new types.

---

### 7. Bytecode Generation (`src/compile.rs` + `src/state/`)

**What it does.** Compiles the `Value` IR tree for each function into a flat byte
stream. Each IR node maps to one or more opcodes with inline operands.

**Files:**

| File | Responsibility |
|------|----------------|
| `src/compile.rs` | Iterates functions; calls `def_code` for each |
| `src/state/codegen.rs` | Compiles one function's IR tree recursively (`generate`) |
| `src/state/mod.rs` | Code-stream primitives (`code_add`, `code_put`, the `code_pos` field) and the stack accessors (`get_stack`, `get_var`) |
| `src/state/text.rs` | String/text formatting at runtime |
| `src/state/io.rs` | File I/O and record operations at runtime |
| `src/state/debug.rs` | IR dump, bytecode disassembly, debug metadata |

**Key functions:**
- `State::def_code(d_nr, data, …)` — compile one function.
- `State::generate(val, stack, top)` — recursive IR-to-opcode emitter.
- `State::code_put(pos, value)` — back-patch a forward jump target after the target address is known.

**When to modify.** When adding a new IR node variant or a new opcode. Add a branch
in `generate` for the new IR variant and emit the corresponding opcode bytes.

**Caveats.**
- Forward jump patching is manual. After emitting `OpGotoFalseWord`, record
  `self.code_pos`, write a placeholder (`code_add(0i32)`), emit the branch body, then
  call `code_put(saved_pos, distance)` to fill in the jump distance. Forgetting the
  patch leaves a garbage jump target.
- The four `HashMap` debug tables (`stack`, `vars`, `calls`, `types` in `State`) are
  populated during codegen for use by the test suite. If you add a new opcode family,
  populate the relevant debug table so that `debug.rs` can disassemble it.
- `src/fill.rs` is **generated** by `src/create.rs` from the `#rust "..."` operator
  templates in `default/*.loft`. Do not edit it by hand. To add an opcode, declare the
  operator with its `#rust` template in `default/*.loft`, then run `make fill`.

---

### 8. Operator Dispatch & Execution (`src/fill.rs`)

**What it does.** `OPERATORS` is an array of `fn(&mut State)` function pointers, one
per opcode. The dispatch loop in `src/state/mod.rs` reads one opcode per iteration and
calls `OPERATORS[opcode](state)`: opcodes 0–254 are one byte, and byte 255 is an escape
followed by a second byte, for opcode `255 + byte`. Each function reads inline operands
with `state.code::<T>()` and pops/pushes the stack.

**This file is generated** from the `#rust "..."` operator templates in
`default/*.loft` by `src/create.rs`; the same templates feed the `--native` generator
(`src/generation/`). Edit the template and run `make fill`. Tests fail when
`src/fill.rs` differs from the regeneration (`tests/issues.rs::fill_rs_up_to_date`).

**When to add an opcode.** When a new operation cannot be expressed as a call to an
existing opcode sequence and the performance or expressibility benefit justifies a new
opcode. Prefer reusing existing opcodes where possible — every new opcode increases
the maintenance surface of the generator, the disassembler, and the stack tracker.

**Caveats.**
- Adding an opcode renumbers every later one: regenerate `src/fill.rs` and
  `index/target_surface.json` (see [Operator numbering](#operator-numbering)).
- Each `op_*` function must leave the stack in a predictable state. The conventions
  are: pop all inputs, push exactly one output (or none for side effects). Violating
  this corrupts every subsequent operation.
- A release build has no runtime stack-depth assertion. A wrong implementation will
  corrupt the stack silently. Use the `minimal` LOFT_LOG preset to trace execution
  and verify by hand that the stack depth is consistent, and `LOFT_VERIFY_STACK=1`
  to catch a read of an unwritten slot.

---

### 9. Data Store & Type Schema (`src/database/`)

**What it does.** Owns the runtime data: struct instances, collections, and the stack
frames themselves. `Stores` is the multi-store manager that maps type numbers to
layouts and allocates memory via the word-addressed `Store` heap allocator.

**Files:**

| File | Responsibility |
|------|----------------|
| `src/store.rs` | Word-addressed heap: `claim`, `delete`, `resize`, typed accessors |
| `src/database/mod.rs` | Constructor, parse-key helpers |
| `src/database/types.rs` | Type registration: `structure`, `field`, `enumerate`, `value`, `finish` |
| `src/database/allocation.rs` | `database` (a new store and its top-level record), `free`, the free-slot bitmap |
| `src/database/search.rs` | `find` / iteration helpers |
| `src/database/structures.rs` | Record construction |
| `src/database/io.rs` | Binary serialisation (`read_data`, `write_data`) |
| `src/database/format.rs` | Debug display |

**Collection implementations:**

| File | Collection |
|------|-----------|
| `src/vector.rs` | `vector<T>` — dynamic array |
| `src/tree.rs` | `sorted<T>` and `index<T>` — Red-Black tree |
| `src/hash.rs` | `hash<T>` — open-addressing hash table |
| `src/radix_tree.rs` + `src/radix_db.rs` | `spatial<T[x,y]>` — PATRICIA radix tree over a Morton (Z-order) key |

**When to modify.** When adding a new collection type, add a new `Parts` variant in
`src/database/mod.rs`, register it in `typedef.rs`, implement the core operations
(`add`, `find`, `remove`, `iterate`, `copy`), and add corresponding opcodes.

**Caveats.**
- `Store::claim` takes from the tail while the store has freed nothing, then from
  the smallest fitting block in an LLRB free-space tree (O(log F), F = tracked free
  blocks), coalescing adjacent free blocks before it grows the store.  A linear scan
  handles single-word blocks.
- `Stores::free()` releases a store's slot through a free-slot bitmap, so stores may
  be freed in any order (`@FR-H-FreeAny`).
- The Red-Black tree uses negative links to encode threads to an in-order
  neighbour. This is a non-obvious encoding documented in `src/tree.rs`. Do not
  mistake a negative link for a null or error.
- The hash table is rebuilt at half full (`is_full` in `src/hash.rs`;
  `LOFT_NO_HALF_LOAD=1` restores the three-quarters threshold).

---

### 10. Standard Library (`src/native.rs` + `default/*.loft`)

**What it does.** Provides all built-in functions available to loft programs without
an explicit `use` import. Native functions are implemented in Rust and registered in
the `FUNCTIONS` table in `src/native.rs`. Higher-level library functions are written
in loft itself in `default/*.loft` and are compiled together with every user program.

**Naming convention in `FUNCTIONS`:**
- `n_<name>` — global function (e.g. `n_assert`, `n_log_info`).
- `t_<LEN><Type>_<method>` — method on a built-in type (`t_4text_starts_with`).
  `LEN` is the number of characters in `Type` (`t_9character_…`).

**When to modify.** When adding a new built-in that requires access to `State`
internals (the stack, the store, or the I/O handles) that cannot be expressed in
loft. Otherwise, prefer writing the function in `default/*.loft`.

**Caveats.**
- Native function signatures must match the loft type signature exactly. A mismatch
  in the number or type of arguments causes a stack corruption at runtime, not a
  compile error.
- `native::init` registers the `FUNCTIONS` table into a name map at start-up. Keep
  names unique: a duplicate name silently replaces the earlier entry.
- Functions in `default/*.loft` are loaded in file-name order (`01_code.loft` first). If function A
  calls function B and A's file sorts before B's file, pass 1 may not have registered
  B yet when A is parsed in pass 1. This is safe because pass 1 is lenient, but keep
  it in mind if forward-reference errors appear.

---

## Known Caveats by Subsystem

| Subsystem | Caveat | Workaround |
|-----------|--------|------------|
| Lexer | `\"` inside a `{...}` format expression relies on the `in_format_expr` flag | Mirror the flag for a new string context |
| Parser | `first_pass: bool` has no type safety — wrong-pass code is a silent bug | Always check `self.first_pass` at the top of IR-generating branches |
| Type resolution | Type cycles detected at compile time by `has_value_cycle()` | `reference<T>` fields break cycles; value-type fields are checked |
| Scope analysis | Pre-init for refs in branches is handled but fragile | Test new owned types inside `if` branches; `find_first_ref_vars` + `deps_ready` gate |
| Variable liveness | Global first_def/last_use is conservative; may flag false conflicts | Write a `tests/slots.rs` test to verify |
| Stack tracker | No runtime depth assertion in release builds; mismatches corrupt silently | Use `LOFT_LOG=minimal` and verify depth by hand; `LOFT_VERIFY_STACK=1` |
| Bytecode gen | Forward jumps must be manually patched; missing patch = garbage target | Always pair an `OpGotoFalseWord` emit with a `code_put` call |
| `fill.rs` | Generated file — direct edits fail `fill_rs_up_to_date` and are overwritten by `make fill` | Edit the `#rust` template in `default/*.loft` |
| Opcode numbering | Adding an opcode renumbers all later opcodes | `make fill` and `make surface-gen` |
| `Store` allocator | LLRB free tree O(log F); linear scan for single-word blocks | Tail and tree paths cover most allocations |
| Red-Black tree | Negative links encode threads — easy to misread | Read the layout comment in `src/tree.rs` before modifying |
| Hash table | Open addressing, rebuilt at half full | Rehash doubles capacity; probe chains stay short |

---

## Debugging Strategy

`loft debug prog.loft:12` stops at line 12: read and change the live frame, then
step — use it instead of adding `println` calls.  See [claude/DEBUG.md](claude/DEBUG.md)
for the full debugging guide: LOFT_LOG presets, diagnosing parse errors, runtime
crashes, validate_slots panics, scope analysis bugs, and using the test framework for
quick iteration.

---

## See also

- [claude/DEBUG.md](claude/DEBUG.md) — Debugging guide: LOFT_LOG presets, diagnosing crashes, scope bugs, slot panics
- [PROMPTS.md](PROMPTS.md) — Working effectively with Claude
- [claude/PLANNING.md](claude/PLANNING.md) — Priority-ordered enhancement backlog and version milestones
- [claude/PACKAGES.md](claude/PACKAGES.md) — Package format, registry, external libraries, native extensions
- [claude/PROBLEMS.md](claude/PROBLEMS.md) — Archive of closed problem entries; open bugs are GitHub issues ([claude/ISSUE_TRACKING.md](claude/ISSUE_TRACKING.md))
- [claude/COMPILER.md](claude/COMPILER.md) — Deep dive into the lexer, parser, IR, and bytecode pipeline
- [claude/DESIGN.md](claude/DESIGN.md) — Algorithm analysis for every major subsystem
- [claude/TESTING.md](claude/TESTING.md) — Test framework; [claude/RUNNING_TESTS.md](claude/RUNNING_TESTS.md) — running tests, debugging `.loft` script failures
- [claude/SLOTS.md](claude/SLOTS.md) — Variable scoping and slot assignment details
- [claude/INCONSISTENCIES.md](claude/INCONSISTENCIES.md) — Language quirks and known semantic asymmetries
