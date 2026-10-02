
# Compiler Pipeline

This document covers how loft source code is turned into executable bytecode: the lexer, the two-pass parser, the IR, type resolution, scope analysis, and bytecode generation.

---

## Contents
- [Pipeline overview](#pipeline-overview)
- [Lexer (`src/lexer.rs`)](#lexer-srclexerrs)
- Parser (`src/parser/`) — [COMPILER_PARSER.md](COMPILER_PARSER.md)
- [IR — The `Value` tree (`src/data.rs`)](#ir--the-value-tree-srcdatars)
- [Type resolution (`src/typedef.rs`)](#type-resolution-srctypedefrs)
- [Scope analysis (`src/scopes.rs`)](#scope-analysis-srcscopesrs)
- [Rust code generation (`src/generation/`)](#rust-code-generation-srcgeneration)
- [Bytecode generation (`src/compile.rs`, `src/state/`)](#bytecode-generation-srccompilers-srcstate)
- [Default library (`default/*.loft`)](#default-library-defaultloft)
- [Naming conventions enforced by the parser](#naming-conventions-enforced-by-the-parser)
- [Diagnostic system (`src/diagnostics.rs`)](#diagnostic-system-srcdiagnosticsrs)
- [Source file summary](#source-file-summary)

---

## Pipeline overview

```
Source text (.loft)
       │
       ▼
  [ Lexer ]           src/lexer.rs
  tokenises chars into LexItem stream
       │
       ▼
  [ Parser — first pass ]     src/parser/
  defines all names; determines types; claims variables
  lenient: unknowns are allowed, deferred to pass 2
       │
       ▼
  [ typedef::actual_types ]   src/typedef.rs
  resolves all unknown types; fills Stores schema
       │
       ▼
  [ Parser — second pass ]    src/parser/
  generates IR (Value tree) with full type knowledge
       │
       ▼
  [ typedef::fill_all ]       src/typedef.rs
  finalises field positions in Stores
       │
       ▼
  [ enum_fn ]                 src/parser/definitions.rs
  synthesises polymorphic dispatch functions for enums
       │
       ▼
  [ scopes::check ]           src/scopes.rs
  assigns scope numbers to variables; inserts free/drop ops
       │
       ▼
  [ byte_code ]               src/compile.rs
  compiles IR Value trees → flat bytecode in State
       │
       ▼
  [ state.execute ]           src/state/mod.rs
  runs bytecode
```

---

## Lexer (`src/lexer.rs`)

### Core types

```rust
pub enum LexItem {
    Integer(u32, bool),  // value, started_with_zero
    Long(u64),
    Float(f64),
    Single(f32),
    Token(String),       // keyword or punctuation
    Identifier(String),  // any non-keyword identifier
    CString(String),     // string literal content up to next { or "
    Character(u32),      // 'x' character constant
    None,                // end of input / end of line
}
```

`LexResult` bundles a `LexItem` with a `Position` (file, line, column).

### Token and keyword sets

Defined as static slices at the top of the file:

- **TOKENS** — punctuation and multi-character operators:
  `:`, `::`, `.`, `..`, `,`, `{`, `}`, `(`, `)`, `[`, `]`, `;`, `!`, `!=`, `+`, `+=`, `-`, `-=`, `*`, `*=`, `/`, `/=`, `%`, `%=`, `=`, `==`, `<`, `<=`, `>`, `>=`, `&`, `&&`, `|`, `||`, `->`, `=>`, `^`, `<<`, `>>`, `$`, `//`, `#`

- **KEYWORDS** — reserved words that are emitted as `Token`, not `Identifier`:
  `as`, `if`, `in`, `else`, `for`, `continue`, `break`, `return`, `true`, `false`, `null`, `struct`, `fn`, `type`, `enum`, `pub`, `and`, `or`, `use`, `match`, `sizeof`, `debug_assert`, `assert`, `panic`

  Note: `fields` was temporarily in KEYWORDS (L3) but is removed in A10.0.  A10 uses
  `s#fields` postfix syntax, so no keyword reservation is needed.

  **Intrinsic keyword handling:**
  - `sizeof` — handled in `parse_single` via `has_token("sizeof")` → `parse_size`.
  - `assert` / `panic` — handled in `parse_single` via `has_token` → `parse_intrinsic_call`, which parses arguments and delegates to `parse_call_diagnostic` for file/line injection. These names are also defined as `pub fn` in `default/01_code.loft`; `parse_fn_name()` in `definitions.rs` allows keyword tokens as function names when `self.default` is true so that the default library can register their signatures.
  - `debug_assert` — reserved for A2.3; currently produces a parse error if used in user code.
  - `s#fields` — A10 field iteration; `fields` is recognized contextually after `#` in `parse_for`, not as a pre-reserved keyword.

  Names recognized by name in `parse_call` but intentionally left as identifiers (lower collision risk): `log_info`, `log_warn`, `log_error`, `log_fatal`, `parallel_for`, `fields`.

The lexer tries two-character tokens first (e.g. `!=` before `!`). Keywords are detected after the identifier is collected.

### Lexer modes

```rust
pub enum Mode {
    Code,        // normal code: skip whitespace and line endings
    Formatting,  // inside a format string after `{`: preserve spaces
}
```

Mode switches happen inside string scanning. When a `{` is encountered inside a string, the lexer switches to `Formatting` and returns the prefix as `CString`. The parser then reads a format expression. When `}` is encountered in `Formatting` mode, the lexer returns to scanning the rest of the string.

This allows inline format expressions like `"result: {value:>10}"` to be tokenised seamlessly.

### String literals and escape sequences

- `"..."` → `CString` for each segment between `{...}` format expressions.
- `\\`, `\"`, `\'`, `\t`, `\r`, `\n` are supported escape sequences.
- `{{` and `}}` inside strings are literal braces.

### Number literals

| Syntax | Result |
|---|---|
| `123` | `Integer(123, false)` |
| `0xaf` | `Integer(0xaf, false)` |
| `0b1010` | `Integer(10, false)` |
| `0o17` | `Integer(15, false)` |
| `1.5` | `Float(1.5)` |
| `1.5f` | `Single(1.5)` |
| `1e2` | `Float(100.0)` |

Special case: `1..4` tokenises as `Integer(1)`, `Token("..")`, `Integer(4)` — the lexer uses a look-ahead to avoid consuming `..` as part of a float.

**That case emits TWO tokens from ONE scan**, and it is the only place that does. Having read `1..`, the number lexer cannot return both, so it returns the `Integer` as the live token and QUEUES the `..` in the replay buffer. A number that ends at a field dot (`n.v.0.0`, `r.0.x`) does the same with a `.`. Both tokens must reach the buffer — see the invariant under Backtracking below.

### Backtracking with `Link` / `revert`

The lexer supports arbitrary lookahead through a memory buffer:

```rust
let link = lexer.link();    // save current position; start buffering tokens
// ... try parsing something ...
lexer.revert(link);         // restore position; replay buffered tokens
```

`link()` increments a reference count. While any link is alive all consumed tokens are buffered. `Link` implements `Drop` to decrement the count; when the count reaches zero the buffer is discarded.

The parser uses this to speculatively attempt a parse path (e.g. checking whether an identifier is a type name or a variable) and backtrack on failure.

**The invariant: the buffer must hold every token the scan consumed, in the order it read them.** A revert replays from the buffer, so a token the scan produced but did not record is simply gone from the re-read. The one scan that produces two tokens (a number ending at `..` or a field `.`, above) is where that can go wrong: `cont()` records a freshly-scanned token, and the queued follow-up is written into the buffer by the number lexer itself, so the NUMBER has to be inserted in front of it rather than treated as already-buffered. Without that, `i + 1..` replayed as `i`, `+`, `..` and left the `+` with one operand — a parse that only fails when a look-ahead happens to span the number (`lexer::test::link_revert_replays_a_queued_number_split`).

### Key lexer methods

| Method | Purpose |
|---|---|
| `cont()` | Advance to the next token (stored in `peek`) |
| `peek()` | Return the current token without advancing |
| `peek_token(s)` | Return true if current token equals `s` |
| `has_token(s)` | Consume and return true if current token equals `s` |
| `token(s)` | Consume expected token; emit error if not found |
| `has_identifier()` | Consume and return if current item is `Identifier` |
| `has_integer()` | Consume and return if current item is `Integer` |
| `has_cstring()` | Consume and return if current item is `CString` |
| `has_keyword(s)` | Consume if current item is `Identifier(s)` (local keyword) |
| `link()` / `revert(l)` | Save / restore lexer position |
| `switch(filename)` | Open a new file and restart |
| `parse_string(text, name)` | Switch to an in-memory string |

---

**Parser (`src/parser/`)** — see [COMPILER_PARSER.md](COMPILER_PARSER.md).

---

## IR — The `Value` tree (`src/data.rs`)

The parser produces a tree of `Value` nodes that represents a function body.

### `Value` enum

IR node variants (full definition in [INTERMEDIATE.md](INTERMEDIATE.md)):
- Literals: `Null`, `Int(i32)`, `Long(i64)`, `Float(f64)`, `Single(f32)`, `Boolean(bool)`, `Text(String)`, `Enum(u8, u16)`
- Variables: `Var(u16)` (read), `Set(u16, Box<Value>)` (write)
- Calls: `Call(u32, Vec<Value>)` — definition nr + args; `CallRef(u16, Vec<Value>)` — fn-ref variable nr + args (see [Function references](COMPILER_PARSER.md#function-references--parse_fn_ref))
- Control: `Block`, `Insert`, `If`, `Loop`, `Break(u16)`, `Continue(u16)`, `Return`, `Drop`
- Iteration: `Iter(u16, init, step)`, `Keys(Vec<Key>)`

`Block` wraps a `Vec<Value>` (statement list), the result `Type`, a `scope` number, and a name used in bytecode dumps.

### `v_block`, `v_set`, `v_if`, `v_loop` — IR constructors

Convenience functions used throughout the parser:

```rust
v_block(ops, result_type, name) → Value::Block(...)
v_set(var, expr)               → Value::Insert([Value::Set(var, expr)])
v_if(cond, then, else)         → Value::If(...)
```

### `Type` enum

Carries the static type of a `Value`. Key variants:

| Variant | Meaning |
|---|---|
| `Unknown(u32)` | Not yet resolved (first pass, or pending inference) |
| `Null` | The null/absent value |
| `Void` | No return value |
| `Integer(min, max)` | Bounded integer; min/max drive storage size (1/2/4/8 bytes; default `integer` is i64) |
| `Boolean` | True/false |
| `Float` | 64-bit float |
| `Single` | 32-bit float |
| `Character` | Unicode code point (stored as `Int`) |
| `Text(Vec<u16>)` | String; the `Vec<u16>` lists variables this text depends on |
| `Enum(def_nr, is_ref, deps)` | Enum type; `is_ref` true for struct-enum references |
| `Reference(def_nr, deps)` | Record reference (pointer into a Store) |
| `Vector(Box<Type>, deps)` | Dynamic array |
| `Sorted/Index/Hash/Spatial` | Keyed collections |
| `RefVar(Box<Type>)` | Stack reference (`&T` parameter) |
| `Iterator(result, state)` | Iterator type |
| `Function(Vec<Type>, Box<Type>)` | First-class function type (arg types + return type); runtime value is `i32` d_nr; variables of this type are callable via normal call syntax |
| `Rewritten(Box<Type>)` | Marker that text/vector append was rewritten |

The dependency lists (`deps: Vec<u16>`) track which variables a reference-typed value "depends on" for lifetime purposes, used by scope analysis.

#### `Type::depend()` and `Type::depending()`

`depend() -> Vec<u16>` extracts the full dep list from any type, recursing through `RefVar`.

`depending(on: u16) -> Type` returns a copy of the type with `on` prepended to the dep list. Called during expression parsing whenever a compound value borrows storage from a local variable (e.g. a text value built from variable 3 → `Type::Text(vec![3])`).

#### `Type::RefVar`

`Type::RefVar(Box<Type>)` means "stack reference" — a DbRef pointing into the stack allocation of another variable, rather than an independently-owned record in a Store. It is used for `&text` parameters (function arguments that alias a caller's text variable). `depend()` on `RefVar` delegates to the inner type.

#### Text return dependencies

When a function returns `Type::Text`, `text_return()` in `parser.rs` promotes local text variables to function *attributes* of type `RefVar(Text)`, and lists the resulting attribute indices in the return type's dep vec. This means a returned text value keeps the caller's stack alive until the return value is consumed.

### `DefType` — definition categories

```rust
pub enum DefType {
    Unknown,     // not yet resolved
    Function,    // normal function
    Dynamic,     // polymorphic dispatch wrapper
    Enum,        // enum type
    EnumValue,   // one variant of an enum
    Struct,      // struct type
    Vector,      // vector type definition
    Type,        // built-in type (integer, text, …)
    Constant,    // named constant
}
```

### `Data` — the definition table

`Data` holds `Vec<Definition>` for every named entity. A `Definition` stores:
- `name`, `def_type`, `returned` (return type for functions)
- `attributes: Vec<Attribute>` — fields (for structs/enums) or parameters (for functions)
- `code: Value` — the compiled IR body
- `variables: Function` — the variable table
- `known_type: u16` — the corresponding `Stores` database type id
- `rust: String` — optional hand-written Rust body for built-in ops

Key `Data` methods:

| Method | Purpose |
|---|---|
| `def_nr(name)` | Look up definition index by name |
| `find_fn(source, name, type)` | Find function by name and first-argument type |
| `add_fn / add_op` | Register a new function/operator in first pass |
| `get_fn` | Find existing function in second pass |
| `get_possible(prefix, lexer)` | Get all definitions whose name starts with prefix |
| `definitions()` | Current count of definitions |
| `def(nr)` | Borrow a definition by index |

---

## Type resolution (`src/typedef.rs`)

Called after each parse pass inside `parse_file`:

### `actual_types`

Iterates all definitions added since `start_def` and:
- Resolves `Unknown` types to their concrete forms (now that all names are registered).
- For each struct/enum, calls `fill_database` to register fields in `Stores`.
- Ensures that vector-of-struct types are registered in `Stores`.

### `fill_database`

For a struct or enum definition, calls `Stores` methods to build the runtime type schema:
- `db.structure(name, parent)` — creates a record type.
- `db.field(s, name, type_id)` — adds a field.
- `db.enumerate(name)` + `db.value(e, variant, ...)` — creates an enum type.
- Field sizes (1/2/4/8 bytes for integers; 4 for references/vectors; 8 for float) are determined by `Type::size`.

### `fill_all`

Calls `database.finish()` to compute final field byte offsets for all record types.

---

## Function calling convention — the heap-return buffer (@PLN55)

Every BODY-carrying plain fn returning `Reference` / `Vector` /
struct-`Enum` carries one hidden attribute `__retbuf` (typed as the
return type, last position) plus a backing argument var from its pass-1
signature parse — **arity is a pure function of the declaration**.

- **Promotion** (`ref_return`): an NRVO-promotable returned local takes
  over the buffer by ROLE SWAP — the attribute is renamed to the local
  (the attr↔var coupling is by name), the placeholder var is retired
  (`Function::retire_argument`), the local keeps its var number (frame
  position is var-number order).  A non-promoted body simply never
  writes the buffer.
- **Callers** fill every hidden heap attr with a fresh `__ref_N`
  work-ref (`add_defaults`); its null-init preamble binds the NULL
  SENTINEL (no allocation — the self-dep keeps `emit_null_dbref` off the
  `null_named` path).  Results are consumed BY VALUE; cleanup is the
  witness pair `OpFreeRef(x)` + `OpFreeRefIfDistinct(__ref, x)`.
- **Other invokers speak the same ABI**: par worker lanes count dests by
  TYPE and witness-free unadopted dests; entry invocations
  (`execute_argv` — incl. the REPL's capture wrappers) push sentinel
  dests; the cdylib shared bridge resolves dest type ids AT RUNTIME by
  type name in the caller's store.
- **Excluded** (no buffer): native `;` declarations, ops and
  `#rust`-templated fns (Rust-implemented, ABI frozen), generic
  templates (specialisations never promote), lambdas (in-place growth;
  invoked via fn-ref dispatch, no earlier caller can exist).

Design + probe history: `plans/55-return-abi/README.md`.

---

## Scope analysis (`src/scopes.rs`)

`scopes::check(data)` is called after parsing a file. It visits every function's IR tree and:

1. Assigns each variable declaration to a scope number (0 = function arguments, 1 = function body, 2+ = nested blocks).
2. Tracks which scopes are currently open via a scope stack.
3. When a scope closes, inserts `OpFreeText` / `OpFreeRef` cleanup calls for variables that go out of scope.
4. Detects re-use of a variable name across sibling scopes and remaps the second occurrence to a fresh slot via `copy_variable`.

The scope numbers are written back into `Function.variables[i].scope` after the pass.

### Key data structures

- `var_scope: BTreeMap<u16, u16>` — maps variable number → scope number where it was first assigned.
- `var_mapping: HashMap<u16, u16>` — maps an original variable to its locally-copied replacement when a variable from an outer (exited) scope is reused in an inner scope.

### Variable assignment (`scan` on `Value::Set`)

When `Value::Set(v, value)` is processed:

1. If `v` already has a `var_scope` entry from a scope that is **no longer open** (not in the scope stack) and no mapping yet exists → call `copy_variable(v)` to create a fresh slot, and record the mapping.
2. For every variable index `d` in `function.tp(v).depend()` that is not yet in `var_scope` → insert `d` into `var_scope` at the current scope and prepend a null/empty initializer for `d` into the output as a `Value::Insert`.
3. Insert `v` into `var_scope` at the current scope (if not already present).

This ensures dependency variables are always initialised in the same scope as the variable that borrows them.

### Cleanup generation (`get_free_vars` / `free_vars`)

`get_free_vars(function, data, to_scope, tp, ret_var)` produces the `OpFree*` calls for all variables in `var_scope` up to `to_scope`:

```
for each variable v in scope:
    skip if v == ret_var  (it is being returned)

    if type is Text(_)
        → emit OpFreeText(v)

    if type is Reference/Vector/Enum(ref)
       AND dep list is empty        ← variable owns its allocation
       AND v ∉ tp.depend()          ← not needed by the return value
        → emit OpFreeRef(v)
```

`free_vars` then inserts the free ops into the IR:
- If the final expression is a `Value::Block`, free ops are inserted **inside** the block just before the block's last operator (`insert_free`), so cleanup runs before the block's `OpFreeStack`.
- Otherwise, free ops are inserted before or after the expression in the statement list.

### Block returns and `OpFreeStack`

`OpFreeStack(value_bytes, discard_bytes)` collapses a block's stack frame:
- decrements `stack_pos` by `discard_bytes`
- asserts no `text_positions` entries remain in the discarded range (debug builds)
- bitwise-copies `value_bytes` bytes as the block's result

**Constraint**: all `String`-typed (text) variables allocated **inside** a block must be freed with `OpFreeText` before `OpFreeStack` runs. The one exception is the *return variable* (`ret_var`), which is skipped in `get_free_vars`. This works safely only when the text variable was allocated **outside** the block (function scope or an enclosing block scope), so that its stack position falls below the `OpFreeStack` discard range.

If a block allocates a new text variable internally and returns it, the variable's position falls inside the discard range and the debug assertion fires. The fix in such cases is to hoist the text variable's initialisation (`claim_temp`) to the enclosing scope.

### `copy_variable` (`variables/`)

Creates an exact duplicate of a variable (same name, same type including deps) with fresh `scope = u16::MAX` and `stack_pos = u16::MAX`. Used when a variable from an outer scope is assigned again in an inner sibling scope that no longer has the outer scope in its stack.

---

## Rust code generation (`src/generation/`)

`src/generation/` provides the `Output` struct and `rust_type` function used to transpile compiled loft programs to Rust source files. This is used only during development to regenerate `src/fill.rs` and `src/native.rs` from the `#rust "..."` annotations in the default library. It is not involved in the normal interpreter execution path.

### `Output<'a>`

```rust
pub struct Output<'a> {
    pub data: &'a Data,         // read-only view of all definitions
    pub stores: &'a Stores,     // runtime type schema
    pub counter: u32,           // unique label counter for generated identifiers
    pub def_nr: u32,            // definition number currently being emitted
    pub indent: u32,            // current indentation level
    pub declared: HashSet<u16>, // variable slots already declared in this function
}
```

Bundles the read-only compile-time data with the mutable emission state so that individual emit functions receive a single context argument.

### `rust_type(tp, context) -> String`

Maps a loft `Type` to the corresponding Rust type string. The `context` parameter controls the form:

| Context | Effect |
|---|---|
| `Context::Argument` | Stack/argument passing type (e.g. `Str` for text, `i32` for integer) |
| `Context::Variable` | Local variable type (e.g. `String` for text — owned heap allocation) |
| `Context::Reference` | Prefixes the argument type with `&` |

Integer types are mapped to `u8`/`u16`/`i8`/`i16`/`i32` based on the `Integer(min, max)` range. Reference, vector, and collection types all map to `DbRef`.

---

## Bytecode generation (`src/compile.rs`, `src/state/`)

`byte_code(state, data)` iterates all `Function` definitions (excluding operators) and calls `state.def_code(d_nr, data)` for each. This compiles the `Value` IR tree into a flat bytecode representation stored in `State`.

The bytecode is a compact encoding of the `Call`/`Set`/`If`/`Loop` IR nodes. It is optimised for fast interpretation rather than size.

`state.execute("main", data)` runs the named function.

`show_code(writer, state, data)` dumps both the IR tree and the bytecode for each user-defined function to a writer — used for the debug output in `tests/dumps/`.

---

## Default library (`default/*.loft`)

The default library is loaded before any user source. It is parsed with `default: true`, which:
- Allows `OpXxx`-prefixed names (operator definitions).
- Allows `#rust "..."` annotations that supply the Rust implementation string for the code generator (`src/generation/`).
- Allows `#hot` / `#cold` on an operator: its dispatch priority.  A `#hot` operator takes a one-byte opcode and runs inline in the interpreter's lean loop, so its template may use only its operands, the stack accessors, `code_pos` and `raise_recoverable` (anything more fails to compile as hot); a `#cold` one is rare and takes a two-byte opcode.  Changing either is `make fill` (formal/rewrites.md `(R-HotInline)`, `(R-OpPriority)`).
- Registers all built-in types, operators, and standard functions in `Data` and `Stores`.

Files are loaded in alphabetical order:
- `01_code.loft` — all operators and standard functions
- `02_files.loft` — file I/O, `Format`, `EnvVariable`, path helpers
- `03_text.loft` — text utility functions

---

## Naming conventions enforced by the parser

| Category | Convention | Enforcement |
|---|---|---|
| Functions / variables | `lower_case` | `is_lower()` |
| Types / structs / enums / enum values | `CamelCase` | `is_camel()` |
| Constants | `UPPER_CASE` | (noted but not enforced by `is_upper`) |
| Operator definitions | `OpXxx` prefix | `is_op()` |

Violations emit an `Error` diagnostic but do not abort compilation.

---

## Diagnostic system (`src/diagnostics.rs`)

Every loft error — parser, type-check, or runtime — reaches the user as
`file:line:col` + a concrete message + the source line with a caret +
(where useful) a suggestion. The machinery (@PLN28) is three layers:

### Layer 1 — positions & spans

`Level` orders `Debug < Warning < Error < Fatal`. Compile-time messages
are collected on the `Lexer`, merged into `Parser::diagnostics` after
each parse call, and carried as `DiagEntry { level, message, file, line,
col }`.

- `diagnostic!(lexer, level, …)` stamps the **lexer's current cursor**.
- `diagnostic_at!(lexer, &pos, level, …)` stamps a **captured
  `Position`** — used for type errors detected *after* the offending
  node is parsed (the cursor has drifted to the `;`/`)` by then), so the
  caret points at the token the user actually got wrong.

Fault-prone IR nodes additionally carry their position *in the tree* via
`Value::Span(Box<(Position, Value)>)` (`src/data.rs`), wrapping the
runtime-fault-prone constructs (`/` `%`, index `[`, field `.`, `Call`/
`CallRef`). Every second-pass / codegen walker has a one-line `Span`
passthrough arm; sites that pattern-match a specific `Value` shape route
through `Value::unspan()` / `unspan_mut()`. At codegen, a `Span` records
`pc → Position` into `Definition.source_spans` (mirror of `line_numbers`)
so a runtime fault can be mapped back to source. (Nodes whose diagnostics
already capture their own `Position` via `diagnostic_at!` — assignment,
`for`, `return`, struct-literal, narrowing cast — are intentionally *not*
wrapped; see `plans/28-error-messages/01-spans-on-ir.md § Resolution
2026-07-07`.)

### Layer 2 — runtime errors (C66: log-and-continue)

Runtime faults (divide-by-zero, index OOB, null deref, narrowing-cast
overflow, `panic`/`assert`) build a `runtime_error::RuntimeError` and
store it in `Stores::runtime_error` with `had_fatal = true`. Per
[DESIGN_DECISIONS § C66](DESIGN_DECISIONS_FAILURE.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt),
the faulting op then **completes with its sentinel** (null DbRef, char 0,
`i64::MIN`, …) and execution **continues** — loft programs must not abort
on user-attributable edge cases. The stored error carries the source
`Position` (via `source_spans`) for rendering at exit. `--dev-soft-halt`
/ `LOFT_DEV_SOFT_HALT=1` demotes dev-mode raises to the same
log-and-continue so one run surfaces every fault site.

### Layer 3 — renderers

- `DiagEntry::to_string_compact` — single line `Level: message at
  file:line:col`, used by the test harness.
- `diagnostic_render::render_pretty_all` — the user default: header +
  `--> file:line:col` + source line + caret, with cascade dedup.

`LOFT_ERRORS=compact|pretty` (env) or `--errors=compact|pretty` (CLI,
overrides env) switches renderers; the default is `pretty`. The test
harness pins `compact` in `tests/common/`.

**Suggestions** (`suggest_similar` / `suggest_similar_capped`,
`src/diagnostics.rs`) append `— did you mean '<near>'?` to *name-not-
found* diagnostics (variable, function, field, method, type, enum
variant, format capture). Short names (≤3 chars) never suggest; 4+ chars
allow Levenshtein-2 (catches transpositions like `naem`→`name`).

**Invariant:** every error knows its source position. Anything that
`panic!`s in the runtime is an interpreter bug, not a user error (the one
intentional exception is documented at its site in `fill.rs`).

---

## Source file summary

| File | Role |
|---|---|
| `src/lexer.rs` | Tokeniser; link/revert backtracking; string/format mode |
| `src/parser/mod.rs` | `Parser` struct, constructors, `parse`/`parse_dir`/`parse_file`, core helpers |
| `src/parser/definitions.rs` | Enum/struct/typedef/function parsing; `enum_fn` dispatch synthesis |
| `src/parser/expressions.rs` | Expression parsing: operators, assignments, strings, function references |
| `src/parser/collections.rs` | Iterators, `for` loops, `map`/`filter`, parallel-for, vector comprehensions |
| `src/parser/control.rs` | Control flow: `if`, `while`, `return`, `parse_call`, `parse_method` |
| `src/parser/builtins.rs` | Parallel worker parsing helpers |
| `src/data.rs` | `Value`, `Type`, `DefType`, `Data`, `Attribute` definitions |
| `src/typedef.rs` | Type resolution; `Stores` schema population |
| `src/scopes.rs` | Scope assignment; lifetime cleanup insertion |
| `src/variables/` | Per-function variable table (`Function`) |
| `src/compile.rs` | `byte_code` — IR → bytecode; `show_code` |
| `src/state/mod.rs` | `State` struct, constructors, `execute`/`execute_argv`, stack primitives |
| `src/state/text.rs` | String/text operations: allocation, formatting, slicing |
| `src/state/io.rs` | File I/O, database manipulation, vector/hash/record operations |
| `src/state/codegen.rs` | Bytecode generation: `generate`, `generate_set`, all `gen_*` helpers |
| `src/state/debug.rs` | Debug dump: `dump_code`, `dump_op_arg`, `print_code`, log step tracing |
| `src/diagnostics.rs` | Error/warning collection and formatting |
| `src/database/mod.rs` | `Stores` constructor, basic get/put, parse-key helpers |
| `src/database/types.rs` | Type-building methods: `structure`, `field`, `finish`, `sorted`, `hash`, etc. |
| `src/database/allocation.rs` | Store management, claim/free, `copy_claims*`, `clone_for_worker` |
| `src/database/search.rs` | Find/iterate: `find`, `find_vector`, `find_array`, `find_index`, `next` |
| `src/database/structures.rs` | Record construction, parsing, `get_ref`, `get_field`, `vector_add` |
| `src/database/io.rs` | File I/O: `read_data`, `write_data`, `get_file`, `get_dir`, `get_png` |
| `src/database/format.rs` | Display/formatting: `show`, `dump`, `rec`, `path` |
| `src/generation/` | Rust code generator — `Output` struct, `rust_type` mapping, emits `fill.rs` / `native.rs` |
| `src/calc.rs` | Field byte-offset calculator for struct/enum-variant layout |
| `src/stack.rs` | Bytecode-generation stack frame (`Stack`, `Loop`) |
| `src/create.rs` | Drives code generation: `generate_lib` and `generate_code` |
| `default/*.loft` | Built-in operators and standard library |

---

## See also
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs
- [TESTING.md](TESTING.md) — Test framework; [RUNNING_TESTS.md](RUNNING_TESTS.md) — LogConfig debug-logging presets
- [../DEVELOPERS.md](../DEVELOPERS.md) — How to add features: pipeline walkthrough, caveats per subsystem, debugging strategy
