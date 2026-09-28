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
  - [5. Variable Liveness](#5-variable-liveness-srcvariablesrs)
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

---

## Quality Requirements

Before any change is merged, it must satisfy all of the following.

### Tests first
Every feature requires:
- At least one positive test (correct program uses the feature successfully).
- At least one negative test (a diagnostic test in `tests/parse_errors.rs` or
  `tests/immutability.rs` that confirms the right error is produced for misuse).
- For collection or runtime features: a large-N stress test (N ≥ 100) covering
  add, lookup, remove, and iteration.
- A corresponding example in the relevant `tests/docs/*.loft` file.

### Clean build
`make clippy` must pass without warnings. This runs `cargo fmt`, `cargo clippy`, and
the documentation generator.

### Full test suite
`make test` and `make loft-test` must both pass. No test may be moved to `#[ignore]`
without a corresponding entry in `PROBLEMS.md` with severity and workaround.

### Error quality
New parse or type errors must:
- State *what* is wrong and *where* in the source (file + line).
- Include the token or expression that triggered the error.
- Suggest the fix when the fix is unambiguous.
- Not panic. Use the diagnostic system (`src/diagnostics.rs`); prefer `Warning` or
  `Error` over `Fatal` unless the compiler cannot continue.

#### Diagnostic positioning — current state and improvement plan

The diagnostic system offers three methods with different position accuracy:

| Method | Position source | When to use |
|--------|----------------|-------------|
| `diagnostic!(lexer, level, msg)` | Current `lexer.position` | Simple errors where the lexer is at or near the problem |
| `specific!(lexer, &lex_result, level, msg)` | Saved `LexResult.position` | When you have the exact token that caused the error |
| `lexer.pos_diagnostic(level, &pos, msg)` | Explicit `Position` | When using a stored definition site (e.g. from `Definition.position`) |

**Known positioning issues (improvement opportunities):**

The following diagnostics point to a suboptimal location. Each can be improved
by saving the relevant token position before the parser advances past it.

1. **Operator errors point one token ahead** (`parser/mod.rs:527, 762, 771`).
   `call_op` and `validate_convert` use `lexer.peek()` to get the error position,
   which points to the *next* token, not the operator or operand that failed.
   **Fix:** Save the operator token's `LexResult` before calling `call_op`, and
   pass it to `specific!()` in the error branch.

2. **Division-by-zero warning points past the zero** (`parser/expressions.rs:1036`).
   The warning fires after the right-hand operand has been consumed.
   **Fix:** Save the position of the operator token (`*`, `/`, `%`) before
   parsing the RHS. Use that position in the diagnostic.

3. **Match exhaustiveness error points at closing `}`** (`parser/control.rs:719`).
   The error fires after the entire match body has been parsed.
   **Fix:** Save `lexer.at()` at the `match` keyword (before parsing arms)
   and use `lexer.to(saved)` before emitting the exhaustiveness error.

4. **"Not all code paths return" warning points at end of block** (`parser/control.rs:175`).
   **Fix:** Save the position of the `->` return-type annotation and use it
   for the diagnostic, since that is where the return-type promise was made.

5. **Unused-definition warning uses `{:?}` debug format** (`data.rs:1424-1434`).
   The position is correct but rendered via Rust `Debug` formatting instead of
   the `pos_diagnostic()` API.
   **Fix:** Call `lexer.pos_diagnostic()` with `def.position` instead of embedding
   `{:?}` in a format string.

**General pattern for improving a diagnostic position:**

```rust
// BEFORE (position drifts past the error):
let rhs_type = self.expression(&mut rhs);
if rhs_type != expected {
    diagnostic!(self.lexer, Level::Error, "type mismatch");
    //                                    ^ points at token AFTER rhs
}

// AFTER (position pinned to the operator):
let op_pos = self.lexer.at();          // save BEFORE parsing rhs
let rhs_type = self.expression(&mut rhs);
if rhs_type != expected {
    self.lexer.to(op_pos);             // restore to operator
    diagnostic!(self.lexer, Level::Error, "type mismatch");
    //                                    ^ points at the operator
}
```

When adding a new diagnostic:
1. Decide what the user should see underlined: the token, the name, or the
   whole expression.
2. Save `lexer.at()` *before* the parser advances past that point.
3. Use `lexer.to(saved)` just before the `diagnostic!()` call, or use
   `specific!()` / `pos_diagnostic()` with the saved position.
4. Add a test in `tests/parse_errors.rs` that asserts the exact `file:line:col`.

#### Diagnostic message quality — improvement plan

The compiler emits ~200 diagnostics across 9 files. The overall quality is
reasonable but several categories of improvement would make the experience
significantly better for new users.

**Step 1 — Convert panics to diagnostics (7 locations)**

These `panic!` / `unreachable!` calls crash the compiler instead of showing a
useful error. Each should become a `diagnostic!` + `return`:

| File | Line | Current message | Suggested replacement |
|------|------|-----------------|-----------------------|
| `parser/mod.rs` | 644 | `panic!("Get not implemented on '{}'")` | `Error: Field access not supported on type '{}'` |
| `parser/mod.rs` | 718 | `panic!(...)` | `Error: Internal: unexpected type in field access` |
| `parser/mod.rs` | 795 | `panic!("Incorrect Dynamic function {}")` | `Error: Cannot call '{}' dynamically` |
| `parser/mod.rs` | 965 | `panic!("Unexpected reference type {}")` | `Error: Unexpected reference type '{}'` |
| `parser/expressions.rs` | 2268 | `unreachable!("Value::Iter with non-Block")` | `Error: Internal: malformed iterator` |
| `parser/collections.rs` | 56 | `panic!("Incorrect Iter")` | `Error: Internal: malformed iterator expression` |
| `parser/control.rs` | 1157 | `unreachable!("ref_return called with...")` | `Error: Internal: unexpected return type in ref_return` |

Panics marked "Internal:" are compiler bugs, not user errors — but they should
still produce a diagnostic instead of a stack trace.

**Step 2 — Downgrade over-aggressive Fatal levels (3 locations)**

`Level::Fatal` stops all further parsing. These cases are recoverable:

| File | Line | Message | Change to |
|------|------|---------|-----------|
| `parser/mod.rs` | 1071 | `"Syntax error"` | `Level::Error` — add context: `"Unexpected token '{token}'"` |
| `parser/mod.rs` | 1067 | `"use statements must appear before definitions"` | `Level::Error` — parse can continue |
| `data.rs` | 1035 | `"Cannot redefine {type} {name}"` | `Level::Error` — skip the duplicate, continue |

The remaining Fatal (`lexer.rs:465` unterminated character) is appropriate.

**Step 3 — Add context to generic messages (5 locations)**

These messages leave the user guessing. Each needs the relevant identifier:

| File | Line | Current | Improved |
|------|------|---------|----------|
| `parser/mod.rs` | 1071 | `"Syntax error"` | `"Unexpected '{token}' — expected a declaration or statement"` |
| `parser/expressions.rs` | 2442 | `"Cannot iterate"` | `"Cannot iterate over type '{}' — expected vector, sorted, index, text, or range"` |
| `parser/control.rs` | 322 | `"expect variant name or '_' in match arm"` | `"Expected variant name or '_', got '{token}'"` — include the unexpected token |
| `parser/control.rs` | 510 | `"expect variant name after '\|'"` | `"Expected variant name after '\|', got '{token}'"` |
| `lexer.rs` | 838 | `"Expect token {token}"` | Already parameterised; verify `{token}` is actually filled (it is — the `token` parameter is the expected token string) |

**Step 4 — Add fix suggestions to common errors (6 candidates)**

Messages that describe the problem but don't suggest the fix:

| Current message | Suggested addition |
|-----------------|-------------------|
| `"Parameter '{}' has & but is never modified"` | Already has: `"; remove the &"` — good |
| `"Variable cannot change type from {} to {}"` | Add: `"; use a new variable name or cast with 'as'"` |
| `"Cannot modify const parameter '{}'"` | Add: `"; remove 'const' from the parameter or use a local copy"` |
| `"match on {} is not exhaustive — missing: {}"` | Add: `"; add the missing variants or a '_ =>' wildcard"` |
| `"Cannot add elements to '{}' while it is being iterated"` | Add: `"; collect additions in a separate variable and append after the loop"` |

**Step 5 — Improve error recovery after token failures**

Currently `lexer.token("X")` emits `"Expect token X"` and continues parsing
at the same position. The next parse attempt sees the same unexpected token,
producing a cascade of confusing errors.

Recovery strategy per construct:

| Construct | Recovery action after failed token |
|-----------|-----------------------------------|
| Missing `;` | Continue — the parser already handles implicit semicolons in most cases |
| Missing `)` | Skip tokens until `)` or `{` or `;` is found |
| Missing `}` | Skip tokens until `}` at the same brace depth |
| Missing `=>` in match | Skip tokens until `=>` or `,` or `}` |
| Missing `,` in args | Continue — the next identifier starts the next parameter |

Implementation: add a `recover_to(tokens: &[&str])` method to `Lexer` that
advances until one of the given tokens is found (or EOF). Call it after
each `token()` failure in contexts where cascading is likely.

**Step 6 — Normalise message style**

Convention (already followed by ~95% of messages):
- Start with an uppercase letter.
- Name identifiers in single quotes: `'variable_name'`.
- Name types without quotes: `integer`, `vector<text>`.
- Use `—` (em dash) to separate the problem from the suggestion.
- End without a period.

The two exceptions to fix:
- `parser/control.rs:510` — `"expect variant name after '|'"` → `"Expect..."`
- `parser/expressions.rs:765` — `"set_file_size is not defined"` → `"'set_file_size' is not defined"`

**Step 7 — Add missing diagnostic coverage**

| Gap | Where to add | Priority |
|-----|-------------|----------|
| Calling a function with wrong argument count | `parser/control.rs` `parse_method` / `parse_call` — some paths lack the count | Medium |
| Assigning to a struct field with wrong type | `parser/expressions.rs` `towards_set` — some field-type mismatches are silent | Medium |
| Using an undefined variable in an expression | `parser/expressions.rs` — currently returns `Type::Null` silently in some paths | Low |
| Integer literal out of range for target type | `parser/expressions.rs` — `i32` overflow not checked at parse time | Low |

**Verification checklist for each step:**

Every diagnostic change must:
- [ ] Have a test in `tests/parse_errors.rs` asserting the exact message and position
- [ ] Not break any existing tests (`cargo test`)
- [ ] Not introduce new `panic!` paths
- [ ] Follow the style convention above

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

### Currently out of scope (planned for a future major version)

- Closure capture for lambdas (planned 1.1; lambdas without capture shipped in 0.8.2).
- REPL / interactive mode (planned 1.1).
- Generics beyond the built-in collection types.
- Async/await.
- FFI / C interop.

Do not implement these ahead of schedule. If the roadmap changes, update `PLANNING.md`
first and get agreement before touching code.

---

## Backwards Compatibility

Loft has not reached 1.0. Before 1.0, **breaking changes are allowed** but must be
deliberate and documented.

### Pre-1.0 rules
- Any syntax change that makes a valid existing program fail to compile must be noted
  in `PLANNING.md` under the relevant milestone.
- Any change that alters the **runtime output** of a correct program is a breaking
  change even if the program still compiles — treat it the same way.
- All `.loft` files in `tests/docs/` and `tests/scripts/` serve as the backwards-
  compatibility corpus. They must pass after every change.

### At and after 1.0
- No syntax that currently compiles and runs correctly may be removed without a
  deprecation cycle of at least one minor version.
- The binary serialisation format of `Stores::write_data` / `read_data` must be
  versioned. Add a schema version tag before 1.0 ships (`PLANNING.md` item).
- The bytecode format is internal and may change between minor versions, but the
  change must be accompanied by a version bump in the bytecode header so that stale
  `.lbc` files are rejected with a clear error.

### Operator numbering
`OPERATORS` in `src/fill.rs` is a generated file — the opcode numbers are implicit
array indices. Never insert a new opcode into the middle of the array; always append
at the end or within a logical group at the end of that group. Inserting in the middle
renumbers all subsequent opcodes and silently corrupts any previously generated
bytecode.

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
- The `in_format_expr` flag was added in 2026 to allow `\"` inside `{...}` format
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
| `src/parser/mod.rs` | Top-level loop, type-parsing (`parse_type`), type coercion (`convert`, `cast`), two-pass coordination |
| `src/parser/definitions.rs` | `fn`, `struct`, `enum`, `enum_fn` (polymorphic dispatch synthesis) |
| `src/parser/expressions.rs` | Binary operators (precedence climbing), assignments, format strings, struct literals, postfix `.field` and `[index]` |
| `src/parser/collections.rs` | Iterators, `for` loops, `parallel_for`, comprehensions |
| `src/parser/control.rs` | `if`/`else`, `loop`, `break`, `continue`, `return`, function calls |
| `src/parser/builtins.rs` | Parallel worker helpers |

**When to modify.** For any new syntax — new control flow, new collection operations,
new built-in expressions, new literal forms.

**Key patterns:**

- **`first_pass` guard.** Functions that generate IR must check `if self.first_pass { return early; }`. Pass 1 must only register names; if it generates IR it will be discarded and replaced in pass 2.
- **`vars.claim(name, type)`** — declares a new variable and returns its slot number `v_nr`.
- **`vars.get(name)`** — looks up a previously declared variable.
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
- `parse_operators(precedence)` uses precedence climbing. New binary operators must
  be given a precedence level and added to the dispatch table in `expressions.rs`.
- The `first_pass` boolean is a field on the parser struct (`self.first_pass`). There
  is no type safety enforcing which operations are legal in which pass. Be vigilant.

---

### 3. Type Resolution (`src/typedef.rs`)

**What it does.** After pass 1, replaces all `Type::Unknown(id)` placeholders with
concrete types, registers each struct/enum layout in `Stores`, and computes field
byte offsets. Runs between the two parser passes.

**Functions:**
- `actual_types()` — resolves forward references.
- `fill_database()` — registers each type with the `Stores` schema.
- `fill_all()` — calls `database.finish()` to seal the schema.

**When to modify.** When adding a new composite type (e.g. a new collection variant).
The new `Parts` variant must be registered here.

**Caveats.**
- Type cycles (struct A contains B contains A) are detected by `has_value_cycle()`
  in `fill_all()` and produce a compile error. Only value-typed struct fields are
  checked; `reference<T>` fields (pointers) are fine and do not cause cycles.
- `fill_all()` seals the schema — no types may be added after this point. The runtime
  assumes the schema is immutable once execution begins.

---

### 4. Scope Analysis (`src/scopes.rs`)

**What it does.** Walks the IR tree and:
1. Assigns a scope number to each variable.
2. Inserts `OpFreeText` / `OpFreeRef` at scope exit for owned types (Text,
   Reference, Vector).
3. Pre-initialises variables that are first assigned inside a branch (so the stack
   slot is valid regardless of which branch executes).

**Entry point:** `check(data)` — iterates all functions.

**When to modify.** When adding a new owned type (a type that must be freed when it
goes out of scope), or when changing how branches or loops interact with variable
lifetimes.

**Caveats.**
- This is the most fragile subsystem. The `var_mapping` table (which maps a variable
  slot to a "copy" slot when a name is reused across sibling scopes) is hard to
  reason about. Read `SLOTS.md` before touching it.
- Pre-init for reference types first assigned inside a branch is handled by
  `find_first_ref_vars` (with `deps_ready` guard for borrowed types).  This was
  fixed in 2026-03-13 (PROBLEMS #2).  When adding a new owned type, write a test
  that assigns it first inside an `if` branch to verify pre-init works.
- Bugs in scope analysis manifest at runtime (wrong `store_nr`, double-free, garbage
  pointers) — not at compile time. They are hard to diagnose. Add a new test to
  `tests/slot_assign.rs` for any new variable interaction you add.
- The `ref_debug` LOFT_LOG preset is the primary tool for diagnosing scope bugs.
  Enable it by setting `LOFT_LOG=ref_debug` in the environment before running a test.

---

### 5. Variable Liveness (`src/variables.rs`)

**What it does.** Computes, for each variable in each function, the sequence numbers
of its first definition (`first_def`) and last use (`last_use`). These intervals are
used by `validate_slots` to detect stack slot conflicts (two overlapping variables
assigned the same slot).

**Key functions:**
- `compute_intervals(val, function, ...)` — walk IR; populate `first_def`/`last_use`.
- `validate_slots(function, data, def_nr)` — post-codegen conflict checker (panics in debug builds on conflict).
- `size(tp, context)` — bytes needed for a type on the stack.

**When to modify.** When adding a new type that occupies stack space, update `size()`
to return the correct byte count. If the new type is owned, ensure that `OpFree*`
sequences extend the `last_use` of the variable correctly (see how `free_text_nr` and
`free_ref_nr` are handled).

**Caveats.**
- `first_def` and `last_use` are global per function — they span the entire function,
  not just the lexical scope. This is a conservative approximation. Two variables
  may appear to overlap even if they are never simultaneously live. The exemption for
  same-name/same-slot pairs in `find_conflict` handles the common case.
- `size()` duplicates type-size knowledge that also lives in `calc.rs`. If you add a
  new type, update both. A mismatch causes silent stack corruption.

---

### 6. Bytecode Stack Tracker (`src/stack.rs`)

**What it does.** During bytecode emission, maintains an accurate byte count of the
current operand stack depth. Ensures that `OpFreeStack` amounts are correct and that
`break` targets are reached with the right depth.

**Key methods:**
- `operator(d_nr)` — update position after opcode emission.
- `add_loop(code_pos)` / `end_loop(state)` — manage loop frames and patch `break` gotos.
- `add_break(code_pos, loop_nr)` — register a pending break.

**When to modify.** When adding a new opcode that pushes or pops stack values,
`operator()` must return the correct stack delta for the new opcode.

**Caveats.**
- Stack depth mismatches (too many pops, too few pops) cause silent corruption at
  runtime. There is no runtime stack-underflow check in production builds.
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
| `src/state/codegen.rs` | Compiles one function's IR tree recursively (`value_code`) |
| `src/state/mod.rs` | Stack frame primitives (`put_code`, `put_word`, `get_stack`, `patch`) |
| `src/state/text.rs` | String/text formatting at runtime |
| `src/state/io.rs` | File I/O and record operations at runtime |
| `src/state/debug.rs` | IR dump, bytecode disassembly, debug metadata |

**Key functions:**
- `State::def_code(d_nr, data)` — compile one function.
- `State::value_code(val, stack, data)` — recursive IR-to-opcode emitter.
- `State::patch(pos, offset)` — back-patch a forward jump target after the target address is known.

**When to modify.** When adding a new IR node variant or a new opcode. Add a branch
in `value_code` for the new IR variant and emit the corresponding opcode bytes.

**Caveats.**
- Forward jump patching (`patch`) is manual. After emitting a `goto_false` opcode,
  record `code_pos()`, emit the branch body, then call `patch(saved_pos, code_pos())`
  to fill in the jump distance. Forgetting the patch leaves a garbage jump target.
- The four `HashMap` debug tables (`stack`, `vars`, `calls`, `types` in `State`) are
  populated during codegen for use by the test suite. If you add a new opcode family,
  populate the relevant debug table so that `debug.rs` can disassemble it.
- `src/fill.rs` is **generated** by `src/create.rs` / `src/generation.rs`. Do not
  edit it by hand. To add an opcode, add it to the IR and let the generator produce
  the implementation, or add a `#rust` annotation in the relevant `default/*.loft`
  file.

---

### 8. Operator Dispatch & Execution (`src/fill.rs`)

**What it does.** `OPERATORS` is an array of 248 `fn(&mut State)` function pointers,
indexed by opcode byte. `State::execute()` reads one opcode byte per iteration and
calls `OPERATORS[opcode](state)`. Each function reads inline operands with
`state.code::<T>()` and pops/pushes the stack.

**This file is generated.** Edit `src/generation.rs` or `src/create.rs` and
re-run `make gtest` to regenerate. Direct edits to `src/fill.rs` will be overwritten.

**When to add an opcode.** When a new operation cannot be expressed as a call to an
existing opcode sequence and the performance or expressibility benefit justifies a new
opcode. Prefer reusing existing opcodes where possible — every new opcode increases
the maintenance surface of the generator, the disassembler, and the stack tracker.

**Caveats.**
- Opcodes must be appended, never inserted in the middle. Insertion renumbers all
  subsequent opcodes and silently breaks existing bytecode.
- Each `op_*` function must leave the stack in a predictable state. The conventions
  are: pop all inputs, push exactly one output (or none for side effects). Violating
  this corrupts every subsequent operation.
- There is currently no runtime stack-depth assertion. A wrong implementation will
  corrupt the stack silently. Use the `minimal` LOFT_LOG preset to trace execution
  and verify by hand that the stack depth is consistent.

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
| `src/database/allocation.rs` | `allocate` (new Store), `database` (top-level record), `clone_for_worker` |
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
| `src/radix_tree.rs` | `spatial<T>` — radix tree (incomplete) |

**When to modify.** When adding a new collection type, add a new `Parts` variant in
`src/database/mod.rs`, register it in `typedef.rs`, implement the core operations
(`add`, `find`, `remove`, `iterate`, `copy`), and add corresponding opcodes.

**Caveats.**
- The `Store::claim` allocator has a fast path (LLRB free-space tree, O(log F) where F
  is the number of tracked free blocks) and a linear-scan fallback for tiny blocks or
  first-time allocation. The tree tracks blocks with size >= 2 words; single-word free
  blocks are found only by the scan.
- `Stores::free()` decrements `max` without checking LIFO order (Issue 27 root
  cause analysis). Stores must be freed in exact reverse allocation order. Any
  feature that allocates stores conditionally or in non-LIFO order must enforce LIFO
  explicitly.
- The Red-Black tree uses negative values to encode back-links (parent pointers).
  This is a non-obvious encoding documented in `src/tree.rs`. Do not mistake a
  negative left-child for a null or error.
- The hash table load threshold is 75% (`(length * 2 / 3) + 1 >= room`). Beyond
  this point probe chains grow and cache behaviour degrades.

---

### 10. Standard Library (`src/native.rs` + `default/*.loft`)

**What it does.** Provides all built-in functions available to loft programs without
an explicit `use` import. Native functions are implemented in Rust and registered in
the `FUNCTIONS` table in `src/native.rs`. Higher-level library functions are written
in loft itself in `default/*.loft` and are compiled together with every user program.

**Naming convention in `FUNCTIONS`:**
- `n_<name>` — global function (e.g. `n_assert`, `n_log_info`).
- `t_<LEN><Type>_<method>` — method on a built-in type (`t_4text_starts_with`).
  `LEN` is the byte length of `Type` padded to a multiple of 4.

**When to modify.** When adding a new built-in that requires access to `State`
internals (the stack, the store, or the I/O handles) that cannot be expressed in
loft. Otherwise, prefer writing the function in `default/*.loft`.

**Caveats.**
- Native function signatures must match the loft type signature exactly. A mismatch
  in the number or type of arguments causes a stack corruption at runtime, not a
  compile error.
- The `FUNCTIONS` table is scanned linearly on lookup. It is not sorted. Keep names
  unique; duplicates cause the first entry to shadow the second silently.
- Functions in `default/*.loft` are loaded in alphabetical file order. If function A
  calls function B and A's file sorts before B's file, pass 1 may not have registered
  B yet when A is parsed in pass 1. This is safe because pass 1 is lenient, but keep
  it in mind if forward-reference errors appear.

---

## Known Caveats by Subsystem

| Subsystem | Caveat | Workaround |
|-----------|--------|------------|
| Lexer | `\"` inside `{...}` format expression requires `in_format_expr` flag (fixed in 2026) | Fixed; test in `tests/format_strings.rs` |
| Parser | `first_pass: bool` has no type safety — wrong-pass code is a silent bug | Always check `self.first_pass` at the top of IR-generating branches |
| Type resolution | Type cycles detected at compile time by `has_value_cycle()` | `reference<T>` fields break cycles; value-type fields are checked |
| Scope analysis | Pre-init for refs in branches is handled but fragile | Test new owned types inside `if` branches; `find_first_ref_vars` + `deps_ready` gate |
| Variable liveness | Global first_def/last_use is conservative; may flag false conflicts | Write a `tests/slot_assign.rs` test to verify |
| Stack tracker | No runtime depth assertion; mismatches corrupt silently | Use `LOFT_LOG=minimal` and verify depth by hand |
| Bytecode gen | Forward jumps must be manually patched; missing patch = garbage target | Always pair a `goto_false` emit with a `patch` call |
| `fill.rs` | Generated file — direct edits are overwritten on next `make gtest` | Edit `src/generation.rs` only |
| Opcode numbering | Insertion renumbers all later opcodes | Append only; never insert |
| `Store` allocator | LLRB free tree O(log F); linear scan fallback for tiny blocks | Fast path covers most allocations; scan only for blocks < 2 words |
| `Stores::free()` | Does not enforce LIFO order (root cause of Issue 27) | Always free stores in exact reverse allocation order |
| Red-Black tree | Negative values encode back-links — easy to misread | Read the layout comment in `src/tree.rs` before modifying |
| Hash table | Open-addressing with 75% load threshold | Rehash doubles capacity; probe chains stay short |
| `spatial<T>` | Radix tree iteration and removal are stubs | Do not use; emits a compile-time error |
| Library imports | `use lib::*` and `use lib::Name` are supported (T1-2) | — |

---

## Debugging Strategy

See [claude/DEBUG.md](claude/DEBUG.md) for the full debugging guide: LOFT_LOG presets,
diagnosing parse errors, runtime crashes, validate_slots panics, scope analysis bugs,
and using the test framework for quick iteration.

---

## See also

- [claude/DEBUG.md](claude/DEBUG.md) — Debugging guide: LOFT_LOG presets, diagnosing crashes, scope bugs, slot panics
- [PROMPTS.md](PROMPTS.md) — Working effectively with Claude and when to use each prompt in `prompts.txt`
- [claude/PLANNING.md](claude/PLANNING.md) — Priority-ordered enhancement backlog and version milestones
- [claude/PACKAGES.md](claude/PACKAGES.md) — Package format, registry, external libraries, native extensions
- [claude/PROBLEMS.md](claude/PROBLEMS.md) — Known bugs with severity, workarounds, and fix paths
- [claude/COMPILER.md](claude/COMPILER.md) — Deep dive into the lexer, parser, IR, and bytecode pipeline
- [claude/DESIGN.md](claude/DESIGN.md) — Algorithm analysis for every major subsystem
- [claude/TESTING.md](claude/TESTING.md) — Test framework, running tests, debugging `.loft` script failures
- [claude/SLOTS.md](claude/SLOTS.md) — Variable scoping and slot assignment details
- [claude/INCONSISTENCIES.md](claude/INCONSISTENCIES.md) — Language quirks and known semantic asymmetries
