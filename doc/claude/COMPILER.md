
# Compiler Pipeline

This document covers how loft source code is turned into executable bytecode: the lexer, the two-pass parser, the IR, type resolution, scope analysis, and bytecode generation.

---

## Contents
- [Pipeline overview](#pipeline-overview)
- [Lexer (`src/lexer.rs`)](#lexer-srclexerrs)
- [Parser (`src/parser/`)](#parser-srcparser)
- [IR — The `Value` tree (`src/data.rs`)](#ir--the-value-tree-srcdatars)
- [Type resolution (`src/typedef.rs`)](#type-resolution-srctypedefrs)
- [Scope analysis (`src/scopes.rs`)](#scope-analysis-srcscopesrs)
- [Rust code generation (`src/generation/`)](#rust-code-generation-srcgenerationrs)
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

## Parser (`src/parser/`)

### `Parser` struct

```rust
pub struct Parser {
    pub data: Data,           // all definitions (functions, types, structs, enums)
    pub database: Stores,     // runtime type schema (field positions, sizes)
    pub lexer: Lexer,
    pub diagnostics: Diagnostics,
    first_pass: bool,         // true during first pass, false during second
    context: u32,             // definition number of the function being parsed
    vars: Function,           // variable table for the current function
    in_loop: bool,            // whether break/continue are valid
    default: bool,            // true when parsing the default/ library
    file: u32,
    line: u32,
}
```

### Two-pass design

Every source file is parsed **twice**:

**First pass** (`first_pass = true`):
- Registers all type, enum, struct, and function definitions.
- Assigns variable slots (but types may still be `Unknown`).
- Lenient: unknown types and unresolved names do not cause errors.
- Claims working text variables for string assembly expressions.
- Records which stores (via `database`) are mutated by each function.
- After the first pass, `typedef::actual_types` resolves unknown types and `typedef::fill_all` computes field offsets in `Stores`.

**Second pass** (`first_pass = false`):
- Generates the full `Value` IR tree for each function body.
- All type names, variable types, and function signatures must be known.
- Emits errors for type mismatches, unknown variables, and call failures.

The two-pass approach allows forward references — a struct or function can be used before it is defined.

**Pass 1 erroring means pass 2 never runs, so a `!first_pass` gate is a REACHABILITY
decision, not just a timing one.**  Every driver (`parse`, `parse_str`, `parse_snippet`)
returns as soon as pass 1 reaches `Level::Error`; `reached_second_pass()` exists to tell a
caller that happened.  A check gated to pass 2 is therefore silent for every program whose
pass 1 fails — and that is exactly the population that most needs it, because a pass-1 error
is often the CONSEQUENCE of the thing the pass-2 check would have named.  loft#825 is the
worked example: the loop-variable type conflict ran on pass 2, so a body error caused by the
stale loop-variable type won on pass 1 and the user was told to rename a variable that was
not the problem, with a cure that reproduced the error under each new name.

When you gate a diagnostic, ask which pass its TRIGGER can fire on, not only which pass has
the information.  Where both types (or both facts) it compares are already resolved on pass
1, run it on both: pass 1 can then only stay silent where pass 2 still speaks — it cannot
contradict it, and a pass-1 Error cannot double up, because it skips pass 2.  Where the fact
genuinely does not exist until pass 2 (layouts, `Span`s, monomorphs), keep the gate; see
[§ Reading a verdict off the IR](#reading-a-verdict-off-the-ir--read-the-type-not-the-shape).

Both of those run **per file**, at the end of each `parse_file`, and each sweeps only the
definitions that file added.  A `use` suspends the current file to parse the dependency to
completion, so a module can be laid out while a type it names is still an unresolved stub
belonging to the file further up the chain.  `fill_all` therefore defers any def whose
fields are not all known (`layout_blocked`) and re-asks with `copy_unknown_fields` on every
subsequent call — a layout, once registered, is never revisited, so getting it right the
first time is the only chance.  See [LIFETIME.md § A field whose type another MODULE
declares](LIFETIME.md) for what the missing deferral cost.

### How a forward reference actually resolves — the stub, and who adopts it

Worth knowing before touching name resolution, because the mechanism is not a lookup.

A type name the parser cannot resolve yet becomes a real definition: an
`add_def(name, …, DefType::Unknown)` **stub**.  Three things then act on it.

1. **The declaration adopts it.**  `parse_struct`, `parse_enum` and `parse_typedef` each
   look the name up before registering, and when they find a stub they upgrade it IN
   PLACE — same def number, now a real type.  Every `Type::Unknown(stub)` already stored
   on someone's attribute therefore resolves for free.
2. **`use` carries it across files.**  `use inner;` imports the module's names into the
   importer, stubs included.  That is the whole reason a module can name a type the
   IMPORTER declares: the module leaves a stub, the import makes it visible under the
   importer's source, and the importer's own declaration adopts it.  Both files end up
   sharing one def.  (There is no cross-source *lookup* — `Data::def_nr` is keyed on
   `(name, source)` with only a source-0 fallback.)
3. **`resolve_deferred_unknowns` settles the rest** once every file has been parsed:
   rewrite the references if the stub resolved, report `Undefined type` if it did not.

The consequence to remember: **only a spelling that LEAVES a stub can be forward-
referenced.**  Written types go through `parse_type`, which leaves one.  Expressions did
not, so `r: Roofs = Roofs { … }` compiled and the identical `r = Roofs { … }` did not
(loft#801).  Two sites in `parse_var` now leave the same stub — the `Name { … }`
construction branch and the bare-name branch — recorded in `speculative_type_refs` so an
unadopted one stays quiet and lets the construction site report with the author's own
spelling.  A bare name qualifying a VALUE (`Colour.Green`) is included too, since
loft#803 established the discriminants that made its value right.

**A stub is a DEFERRAL, not a resolved type.**  Handing one to a consumer as
`Type::Null` is how `Colour.Green` reported `Unknown type null — did you mean 'JNull'?`
— two names the author never wrote, in PASS 1, aborting before the pass that could have
resolved it.  The field access already has a quiet `Type::Unknown(_)` path for exactly
this, and it keeps the error rather than losing it: a stub nobody adopts is still
`Unknown` in pass 2, where the same site reports "Field of unknown variable".

#### Registration is keyed on the DEF, never on a def-number range

Adoption is what breaks range-scoped work, and it does so silently.
`actual_types_deferred` registers types over `start_def..` — "everything this file just
added".  An adopted stub is not in that range: the module left the stub, so its def
number is BELOW the resuming importer's `start_def`, while the def itself is the
importer's enum.  It therefore never registered, `known_type` stayed `u16::MAX`, and
`Stores::enum_val` answers **`unknown`** for exactly that — a wrong value, not an error
(loft#803).  The layout fault beside it is the same cause: an unregistered enum has zero
width, so the field AFTER an enum-typed field lost its position (the loft#797 shape).

So `fill_all` scans **from 0** for any enum still lacking a db type, because the
condition is a fact about the def, not about where its number falls.  That in turn
requires registration to be idempotent, and its two halves are not idempotent in the
same way — which is what defeated the three earlier attempts on loft#803:

| half | idempotence | why |
|---|---|---|
| `Stores::enumerate` (mint the type) | **once** | it PUSHES; a second call mints a second `Colour` and renumbers every type id after it, so the generated `init()` referenced a `t189` it had not declared |
| `Stores::value` (the db variant list) | **add-if-absent** | it appends blindly; a second pass gives the enum `Red, Green, Red, Green` and shifts what each discriminant names |
| `set_attr_value` (the def's own discriminants) | **every pass** | `parse_enum` rebuilds the def's attributes each pass, so a stamp that ran only at mint leaves pass 2 — the pass that generates code — with variants carrying no discriminant |

`register_enum_db` returns early to a stamp-only path when `known_type` is already set,
which also skips the `__nullable<` name disambiguation — that keys on "the bare name is
already a db type", which is true of the def's own second visit and would rename it.

⚠ **Key that early return on the DEF, never on the NAME.** A db name is not unique
across defs: the stdlib declares `enum Format` (`02_files.loft`) and a program may
declare its own. Making registration idempotent by looking the name up in
`Stores::names` handed the user's `Format` the STDLIB type, so `Format.Number`
(discriminant 2) read back `LittleEndian` — in a program containing no forward
reference at all. `enumerate` shadowing the name is exactly what keeps two same-named
enums apart, so the mint must stay unconditional. Guarded by the name-collision cell in
`tests/scripts/803-forward-enum-value.loft`.

**Every CONSUMER of a pass-1 placeholder needs its own pass-2 report.** The site that creates
a stub reads as the risky half, so review lands there; the risk is spread across every reader,
and a reader that simply does nothing writes no code to notice. loft#934: a CamelCase name
became a speculative type stub, and `Zzz {…}`, `Zzz.x`, `y: Zzz` and `sizeof(Zzz)` each
reported it on pass 2 — four reporters, so the mechanism looked covered — while a bare VALUE
use had none: on pass 2 the stub EXISTS, so the "Unknown variable" branch a lowercase name
reaches was never entered, and `fn f() -> integer { Zzz }` compiled and returned uninitialised
memory. When adding a placeholder, list its consumers and ask what each prints when the
placeholder is never adopted; "nothing" is the bug.

### The H5 two-pass contract — the lazy-append law

`assert_pass2_def_attr_stable` (`src/parser/mod.rs`) pins the cross-pass contract. It is a
plain `assert!`, so it runs in EVERY build — an ordinary `cargo build --bin loft` aborts on
it, which is what a user sees when a program trips it:

> **Pass-1 facts are frozen — every pass-1 def number and attr index must be
> identical after pass 2.  Pass 2 may only APPEND name-keyed synthetic facts
> that pass 1 could not know.**

The four legal lazy-append forms (all verified long-latent on an
`origin/main` control build when the 2026-07 calibration first checked them):

| Form | Producer | Why pass 1 cannot mint it |
|---|---|---|
| `vector<T>` / `main_vector<T>` wrapper defs | the reduce/map/filter desugar machinery (`Data::vector_def`) | the family early-returns on pass 1 (unresolved lambda/forward types); `map`'s OUTPUT wrapper is the lambda's return type |
| generic-instantiation defs (`t_<LEN><Type>_<fn>` with an `n_<fn>` `DefType::Generic` template) | `try_generic_instantiation` | pass 1 only predicts the return type — instantiating there would capture the template's still-being-built body IR |
| the `__closure` hidden attr | `parse_lambda*` (added `!first_pass` from pass 1's closure record) | captures are only known after the body parses |
| a trailing `__work_N` attr | a `text_return` work-buffer promotion the pass-1 classify could not yet see | body facts (e.g. a self-slice-reassigned text param) resolve on pass 2 |

Everything else stays fatal — notably `__ref_N` / `__retbuf` growth, the
`ref_return` drift class the assert was built for, and any non-synthetic
pass-2-only def or attr.  The appends are safe precisely because they are
name-keyed and trailing: call sites are re-parsed in pass 2 against the final
attr list, and no pass-1 number moves.

**The contract presumes both passes parse the SAME text, and the lexer is what makes that true
(loft#1648).**  Each pass `switch`es to every source again, and a file read twice from disk can
change in between: a registry package another process is still extracting into the shared
`~/.loft/registry/`, a concurrent `loft install`, an editor save.  Parsed from two texts, the
passes mint different definitions and H5 refuses a program nothing is wrong with. That was the
flaky *"pass-2-only definition `Coord`"* suite start, `Coord` being a struct of the `graphics`
package.  So `Lexer::switch` keeps the bytes of the first read of each path
(`parse_snapshot`) and answers every later switch to it in the same parse from them;
`Parser::parse_main` clears the snapshot at the start of each parse (`Lexer::begin_parse`), so the
next parse reads the disk as it is then.  Text scans that run beside the parse (the auto-`use`
pre-scan, the declared-name count) read through `Lexer::source_text`, so they see the same bytes.
Guard: `lexer::test::one_parse_reads_a_file_once_and_the_next_parse_reads_it_again`.

The snapshot is taken AFTER `reserve_late_return_buffers`, which is what lets a buffer be
reserved between the passes for a return type pass 1 could not classify (#675). That is also
the limit of what this check can see: it compares COUNTS. A fix aimed at the count can leave
the two passes agreeing on how many attributes there are and disagreeing about which VARIABLE
is the argument — which is the next contract.

### Argument geometry — the attribute list and the slot list are one list

`check_argument_geometry` (`src/state/codegen.rs`) pins the contract *inside* one function:

> **A call site lowers its argument list from the definition's ATTRIBUTES; the callee places
> its argument slots in `Function::arguments()` order, which is variable-NUMBER order.  The
> two are the same list, in the same order.**

Nothing made them agree. A return-buffer promotion that retires one argument variable and
makes a later-numbered one the argument in its place breaks the correspondence silently: the
call site writes the closure into the return buffer's slot, the body reads its `__closure`
from the buffer's, and the program answers a zeroed record or a captured integer of `0` — on
the interpreter only, because `--native` builds its argument list from the attributes alone
and never consults the variable order (loft#1188, and the collection leg of loft#1178).

The condition is a **type** disagreement at a position, not a name one. Two hidden scratch
buffers of the same type are interchangeable — the caller pushes two empty `&text`s and the
callee fills whichever it was handed — and the stdlib's `text::resolve` carries exactly that
pair in swapped order and answers correctly on both backends. A type disagreement cannot be
benign in the same way: the slot holds a value of another shape, so every read of it reads
another argument's bytes. Only a definition whose argument count equals its attribute count is
compared, because a `#rust` / `#native` body has attributes and no variable table and a
promoted `text` parameter is redirected to a shadow local.

### Reading a verdict off the IR — read the TYPE, not the shape

The corollary for any predicate that decides something by inspecting IR: **the two passes
see different IR for the same source**, so a verdict read off the shape can differ between
them — and by H5 above, a definition that first appears in pass 2 is fatal. In practice
the branch that mints it simply never runs on pass 1, and the lexer revert that was going
to feed it finds nothing to lower, so the failure surfaces as a syntax error pointing at
the end of an unrelated construct.

Two differences bite (both loft#699/#701):

- **No struct layouts on pass 1.** Field offsets are the `u16::MAX` placeholder, so a
  nested struct literal is an offset-less `Insert` in pass 1 and an `Object` block in
  pass 2. Asking "is this a nested struct literal?" of the IR answered no, then yes.
  Ask the field's **declared type** instead — both passes agree on that.
- **No `Span` wrappers on pass 1.** Positions are recorded in pass 2, so a tree that was
  a bare `Call` becomes `Span(pos, Call)`. A walker written as a `match` with a catch-all
  `other => other` arm silently returns the whole tree unrewritten: that is how
  `remap_var_nr` left a parameter default holding the callee's raw slot numbers on pass 2
  only. Walk with the exhaustive walker (`visit_constant_vars`), which returns `false` for
  a shape it cannot rewrite — the caller then treats the unrewritten indices as
  "not replayable" and takes the sound path, so the hole cannot re-form silently.

The same asymmetry applies to what gets STORED: a constant's initialiser was kept from
pass 1, so every field offset in it was the placeholder — which is why a struct-element
vector constant pre-built at the wrong stride (loft#702). A vector constant now adopts
the pass-2 initialiser.

### Synthesised-identity stability — the counter-coupling hazard

A recurring bug class spans the parser **and** the native backend. Both synthesise
named things — temporary variables (parser) and hoisted sub-expressions (native
pre-eval) — and both historically derived a synthesised thing's **identity from a
mutable counter that two independent traversals had to read in lock-step**. When the
two traversals disagree about the counter's state, the identities desync silently.

Three instances seen so far:

| Bug | Synthesised thing | Identity source | The two parties | Failure |
|---|---|---|---|---|
| #282 | par materialise temp var | `Vars::unique`: `self.unique += 1` → `_{name}_{N}` | pass-1 walk vs pass-2 walk, matched by **name equality** (`add_variable` reuses by name) | a temp created only in pass 2 shifts the counter → names desync → wrong-type reuse → wrong store stride |
| result-var (#2) | comprehension/result temp | same `Vars::unique` counter | same two passes | same desync, different construct |
| #272 | hoisted pre-eval `_pre_{N}` | `collect`'s `self.counter` | `collect_pre_evals` walk vs `output_code_with_subst` walk, matched by **regenerated-string equality** | a node kind that bypasses the structural recogniser (`Op*`) drifts → re-inline → side-effecting operand double-evaluated → empty read |

**Shared root.** Identity is *re-derived from a counter by two traversals* that are
coupled only by the fragile bet that the counter is in the same state both times.
`try_subst_pre_eval` (`src/generation/pre_eval.rs`) makes this literal: to recognise a
node it rewinds `self.counter` and **re-runs codegen, then string-compares the output** —
so a node's identity is "the exact text it emits at counter K," a property of the *walk*,
not of the *node*.

**Design invariant.** *Synthesised identity is a pure function of the IR node's intrinsic
position — minted once, stored, and read. No identity is recomputed from mutable
traversal state; no two traversals coordinate by regenerating output and comparing.* This
collapses the re-assertion sites (every counter read) down to one mint per identity.

The invariant has two independent realisations:

- **Parser (mechanism A).** Key the temp name on an intrinsic discriminator the synthesis
  site already holds — loop number, comprehension id, node span — not on `self.unique`.
  The two-pass contract becomes "same key ⇒ same name," which is *order-independent*: a
  pass-2-only temp lands on the same name it would have in pass 1. The #282 fix did this
  locally (`_par_{name}_l{loop_nr}`); the general form replaces `Vars::unique(name)` with a
  key-derived `Vars::derived(name, key)` at every synthesis site.
- **Native (mechanism B).** Collapse `collect_pre_evals` and `output_code_with_subst` into
  a **single walk** that decides-and-applies hoists as it goes (emit `let _pre_N = …;` to a
  prelude buffer, drop the name inline). With one walk there is no second traversal to
  disagree, the counter is read once per node, and the regenerate-and-string-compare
  machinery deletes entirely — `Op*` stops being special because there is no recogniser to
  bypass. See [NATIVE.md § Open work](NATIVE.md#open-work) ("one-walk pre-eval").

A heavier third realisation — an explicit `NodeId` numbered once on the IR, making
identity intrinsic everywhere — is held until more counter-coupling surfaces.

### Entry points

```rust
// Parse a file (two full passes)
parser.parse("path/to/file.loft", is_default);

// Parse all .loft files in a directory, alphabetically
parser.parse_dir("default", true, debug);

// Parse from an in-memory string (used in tests)
parser.parse_str(text, "filename", logging);
```

`parse_dir` recurses into subdirectories and calls `scopes::check` after each file.

### `parse_file` — top-level loop

```rust
fn parse_file(&mut self) {
    // 1. Process `use` declarations first, switching the lexer to the
    //    included file and returning when it's done.
    while self.lexer.has_token("use") { ... }

    // 2. Parse top-level definitions in a loop:
    loop {
        self.lexer.has_token("pub");   // optional pub modifier
        if !parse_enum()
        && !parse_typedef()
        && !parse_function()
        && !parse_struct()
        && !parse_constant() { break; }
    }

    // 3. Resolve types and fill the Stores schema.
    typedef::actual_types(...);
    typedef::fill_all(...);
    database.finish();

    // 4. Synthesise polymorphic dispatch helpers.
    self.enum_fn();
}
```

### `use` resolution — `lib_path`

When `use foo;` is encountered, the parser looks for `foo.loft` in the following order:

1. `lib/foo.loft` (project-local library)
2. `foo.loft` (current directory)
3. `<current_dir>/lib/foo.loft`
4. `<base_dir>/lib/foo.loft` (when inside `tests/`)
5. Directories from the `LOFT_LIB` environment variable
6. `<current_dir>/foo.loft`
7. `<base_dir>/foo.loft`

### Operator precedence

Binary operators are parsed using a recursive-descent precedence climber. `OPERATORS` lists levels from lowest to highest precedence:

```rust
static OPERATORS: &[&[&str]] = &[
    &["||", "or"],                           // 0 — lowest
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["as"],                                 // 9 — highest
];
```

`parse_operators(precedence)` handles one level; it calls `parse_operators(precedence+1)` for the right operand. At the top of the recursion, `parse_part` handles postfix `.field` and `[index]` access, and `parse_single` handles atoms.

The postfix chain does not open on a `Void` subject. `if` is an expression, so a bare
`if c { … }` STATEMENT reaches the chain like any value, and without that guard it consumed
the `[` that opened the NEXT line — a function whose tail was `[1, 2]` had the literal parsed
as an index on the `if`, and the diagnostic named keyed collections the program never used.
`[` indexes a value and a `Void` expression produced none; `if c { [1,2] } else { [3,4] }[0]`
keeps its index because its type is a vector, and `for` / `while` are statements that never
reach the chain.

### `parse_single` — atom parsing

Handles the innermost syntactic unit:

| Token | Result |
|---|---|
| `!` / `-` | Unary not / negate |
| `(` expr `)` | Grouped expression |
| `{` block `}` | Inline block |
| `[` ... `]` | Vector literal |
| `if` | Inline if-expression |
| `fn` identifier | Compile-time function reference → `Value::Int(d_nr)` (see below) |
| identifier | Variable, function call, type constructor, or method |
| `$` | Current record reference (inside struct field defaults) |
| integer / float / single | Literal |
| string | Format-string expression |
| character | Character literal as integer |
| `true` / `false` / `null` | Literal boolean / null |

**Method call with same-type variable (`parse_single` Issue 1 fix):** When an identifier
resolves to a Reference-typed variable and the current parse context is an assignment
target of the same Reference type (i.e. `d = c.method()` where both `d` and `c` are the
same struct), `parse_single` calls `vars.make_independent(d, c)` (records that `d` is a
fresh copy of `c`'s slot) and returns `Value::Var(c)` directly. It does **not** emit
`OpCopyRecord(c, d, tp)` as the method self-argument, which was the root cause of Issue 1
(garbage `store_nr` crash). `generate_set` handles direct-assignment `d = c` via a
`ConvRefFromNull + Database + CopyRecord` sequence in its own branch.

### Function parsing — `parse_function`

```
'fn' name '(' [args] ')' ['->' return_type] ( ';' | '{' body '}' )
```

- First pass: registers the definition via `data.add_fn` or `data.add_op`.
- Second pass: looks up existing definition with `data.get_fn`, parses body, stores the code in `data.definitions[context].code`.
- Functions ending with `;` have no body (declaration of an external/built-in operation).
- After the body, `parse_rust` optionally reads `#rust "..."` annotations for the code generator.

**Important — internal function naming:**
- `add_fn` stores user-defined functions under the key `"n_<name>"` (e.g. `fn helper` → `"n_helper"`), not under `"helper"`.
- `add_op` (used only for default-library operators) stores under the plain name.
- `def_nr("helper")` therefore returns `u32::MAX` even if `fn helper` exists — the name `"helper"` is not in `def_names`.
- Consequence for type resolution: if a user writes `v: helper` (function name used as a type), `parse_type("helper")` sees `u32::MAX`, creates a `DefType::Unknown` entry for `"helper"` on the first pass, and `actual_types` emits "Undefined type helper" after the first pass. This is the correct/expected error path — no second-pass diagnostic needed.

### Struct parsing — `parse_struct`

```
'struct' Name '{' field* '}'
```

Each field: `name ':' type ['=' default] ['limit' min '..' max] [CHECK(...)]`

- Field types with `default(expr)` or `virtual(expr)` are handled via `parse_field_default`.
- `$` in a default expression is replaced by `Value::Var(0)` (the current record reference) at struct-init time.
- Trailing commas are allowed.

### Enum parsing — `parse_enum`

```
'enum' Name '{' variant* '}'
```

Two forms of variant:
- Plain: `Name` — a simple value.
- Struct-enum: `Name '{' field* '}'` — a variant with fields (polymorphic record).

After parsing, `enum_fn` synthesises dynamic dispatch wrappers so that functions defined on specific variants can be called polymorphically.

**`enum_fn` / `enum_numbers` — text-buffer forwarding (2026-03-13):**
`enum_fn` runs at the END of the **first pass**, immediately after all variant struct
types are registered. At that point `text_return` has already added `RefVar(Text)`
attributes to each variant function (because `text_return` runs during `parse_code` →
`block_result` for the function body, which is second-pass-only, so the attributes ARE
present by the time `enum_fn` runs in the *first* pass when types are complete).

To forward text-buffer arguments from the dispatcher to each variant:
1. `enum_fn` iterates `args[1..]` (all attributes beyond `self`) and creates a
   corresponding dispatcher argument for each; for `RefVar(Text)` attributes the
   variable is registered with `become_argument`.
2. `extra_call_args` and `extra_call_types` are collected from the dispatcher's own
   variable table for each such attribute.
3. `enum_numbers` is called with these extras; each variant's call IR becomes
   `Call(describe_Variant, [Var(0), Var(dispatcher_buf)])` instead of
   `Call(describe_Variant, [Var(0)])`.

**`generate_call` — `RefVar` forwarding special case (2026-03-13):**
When compiling a mutable argument whose type is `RefVar(_)` and the parameter is
`Var(v)` with `v` also typed `RefVar(_)`, emit only `OpVarRef(var_pos)` (reads the raw
`DbRef`) instead of the usual `generate_var` path which adds `OpGetStackText` after
`OpVarRef`.  The dereference (`OpGetStackText`) must be suppressed when the callee
expects a `DbRef` pointer, not the `str` content.

### ~~`parse_append_vector` — `RefVar(Vector)` gap~~ **FIXED (Issue 4)**

Previously, `v += items` inside a `&vector<T>` parameter was silently discarded.
The fix is `assign_refvar_vector` in `parse_assign` (see the `parse_assign` section
above). The old `parse_append_vector` path (used for non-RefVar vectors) is unchanged.

### Type parsing — `parse_type`

Converts a type identifier into a `Type` enum value. Handles:
- Built-in types: `integer`, `float`, `single`, `boolean`, `text`, `character`, `reference`
- Generic containers: `vector<T>`, `sorted<T[key]>`, `index<T[key]>`, `hash<T[key]>`
- User-defined structs and enums by name lookup
- `&T` reference types

### Type conversion and casting — `convert` and `cast`

Before emitting a binary operation or assignment, the parser checks if the actual type is compatible with the expected type:

1. **`convert`** — implicit, lossless conversion (e.g. widening an integer range, converting null, unwrapping a `RefVar`). Looks for `OpConv*` operators.
2. **`cast`** — explicit `as` conversion (e.g. text to enum, int to enum). Looks for `OpCast*` operators.
3. **`can_convert`** — pure check used for error reporting without code modification.

`can_convert` must accept everything `convert` accepts, including the pairs `convert`
accepts by emitting NOTHING.  It is not `convert`'s subset — it is `validate_convert`'s
only input, so a pair the two disagree on becomes a refusal with no cause behind it.  A
bare-collection parameter (`len(self: hash)`) takes any parameterised hash with no
conversion op at all, so `convert` returning `false` there is normal and `can_convert` is
what actually decides.  When you add an arm to one, add the mirror to the other: loft#824
was a `RefVar` argument that `convert` peeled and `can_convert` did not, which turned
`len(h)` on a `&hash<Row[id]>` into *"expected hash, got &hash<Row,["id"]>"*.

**Dispatch reads THROUGH a `&`; layout reads the `&` itself.**  `Data::type_def_nr`
answers two different questions and only one answer suits both.  For `RefVar(τ)` it says
`reference` — correct for storage (the slot holds a pointer) and wrong for the receiver a
method hangs off, since no type named `reference` declares one.  `Data::find_fn` peels the
wrapper before it looks, so `&vector<T>` resolves `t_6vector_len` exactly as `vector<T>`
does; `parse_field` has always peeled it for the method spelling.  The same peel feeds the
type-directed `len` / `size` builtins in `Parser::call` (one `recv` binding, not one peel
per arm).  See [LOFT.md § Methods and function calls](LOFT.md).

### String and format expression parsing — `parse_string`

When the lexer emits a `CString` followed by format mode:

```
"prefix {expr [:format_spec]} suffix"
```

The parser builds an `Insert` or append sequence:
- The prefix string literal is emitted.
- The format expression is parsed as a normal expression via `expression()`.
- A format specifier (width, radix, alignment, padding) is parsed by `string_states` and `get_radix`.
- The corresponding `OpFormat*` operator is called.
- The suffix string literal is emitted.
- The whole thing is assembled into text using append operations.

`expression()` internally calls `known_var_or_type` which emits an "Unknown variable" error
if the expression variable has not yet been assigned (i.e., `is_defined == false`). This is
the diagnostic path for PROBLEMS #10 — using `{cd}` before `cd = val` in the source.  The
`is_defined` flag is now correctly maintained: it is set only when the `=` token is
confirmed in `parse_assign`, not speculatively beforehand.

Loop-counter variables like `e#count` and `e#first` (lazily created in `iter_op`) are
explicitly marked defined when first referenced, so they are always valid inside a loop body.

### Variable tracking — `Function` / `vars`

`self.vars` (a `Function` from `src/variables/`) tracks the variable table for the function being compiled:

- `create_var(name, type)` — allocates a new slot.
- `unique(prefix, type)` — allocates an anonymous working variable.
- `change_var_type(nr, type)` — updates the inferred type of a variable.
- `become_argument(nr)` — marks a slot as a function parameter.
- `work_texts()` — returns slots claimed for text assembly.
- `test_used(lexer, data)` — emits warnings for unused variables.

### Vector literal parsing — `parse_vector` / `vector_db`

`parse_vector` (called when `[` is encountered) builds vector literal and append IR. It internally tracks the "owner variable" slot `vec` as a `u16`:

- If parsing an append to a struct field (`is_field = true`), `vec = u16::MAX` (sentinel meaning "no owning variable").
- If parsing a plain variable append (`v += [...]`), `vec = variable_slot_number`.
- Otherwise a temporary slot is created via `create_unique`.

`vector_db` (called from `build_vector_list`) emits the `OpDatabase` op that allocates a store for new struct-valued vector elements. It must guard against `vec == u16::MAX` before calling `is_argument(vec)`:

```rust
fn vector_db(&mut self, assign_tp: &Type, vec: u16) -> Vec<Value> {
    if self.first_pass || vec == u16::MAX || self.vars.is_argument(vec) {
        Vec::new()  // skip: field context, first pass, or function argument
    } else { ... }
}
```

Without the `vec == u16::MAX` guard, calling `is_argument(u16::MAX)` would panic with an out-of-bounds index (since `u16::MAX = 65535` far exceeds the variable table size). This was a bug that triggered whenever a `vector<Struct>` field was appended to using a struct literal, e.g. `q.list += [Num{v:1}, Num{v:2}]`.

---

### Runtime safety checks in the second pass

During the second pass, `parse_assign` and `parse_function` enforce two additional
safety invariants beyond type-checking:

**For-loop mutation guard (`parse_assign`, `variables/`):**
When parsing `v += items`, if the type is a collection (`Vector`, `Sorted`, `Index`, or
`Spatial`) and `v` resolves to a `Value::Var(v_nr)`, the parser calls `vars.is_iterated_var(v_nr)`.
This walks the `current_loop` chain in `variables/` comparing against each loop's `coll_var`
(original collection variable, set via `set_coll_var()` in `parse_for`). If the variable is
currently being iterated, a compile error is emitted:

```
Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop
```

The check only fires for `Value::Var` LHS, not field access (`Value::Field`), so `db.items += x`
is not blocked. `v#remove` is explicitly allowed, filtered loop or not — it is implemented via
`OpRemove` which adjusts the iterator position before removing.

**Empty-body stubs (`parse_function`, `def_code` in `state.rs`):**
A function whose body is an empty block `{ }` AND whose first parameter is named `self` is
treated as an intentional polymorphic stub. Two effects:
- `parse_function` skips the `test_used` call that would emit "Parameter self is never read".
- `def_code` detects the empty `Value::Block` case, performs normal argument claiming (so that
  owned references like Text/Reference get their lifecycle managed correctly), then emits only
  `OpReturn` — the stub silently returns null for its declared return type.

Detection requires the first parameter to be named `self` to avoid false positives on ordinary
empty helper functions like `fn setup() { }`.

---

### `parse_assign` — assignment and mutating operators

```
expr [ '=' | '+=' | '-=' | '*=' | '%=' | '/=' ] expr
```

For a simple `=`:
- If the left side is a variable, the right side type is used to refine the variable's type (`change_var_type`).
- If the left side is a field, `set_field` emits the appropriate `OpSet*` call.

**`vars.defined` placement (PROBLEMS #10 fix, 2026-03-15):** `vars.defined(v_nr)` is
called *inside* the `has_token("=")` block, only after the `=` token has been confirmed.
Before this fix, the call preceded the token check, so any bare `Value::Var` seen as a
candidate LHS (including `{cd}` inside a format expression) was incorrectly marked
defined, hiding the "use before assignment" error and causing a panic in the
byte-code generator when the variable's stack slot was still `u16::MAX`.

For `+=` on text: delegates to `assign_text` which manages the string-assembly working variable.

For `+=` on `&vector<T>` parameters: handled by `assign_refvar_vector`. When the LHS variable has type `RefVar(Vector)` and the operator is `+=`, and the RHS is not a `Value::Insert` or `Value::Block` (bracket-form literals/comprehensions), it emits `OpAppendVector(Var(v_nr), rhs_expr, rec_tp)`. Bracket-form `[elem]` and vector comprehensions produce `Value::Insert` / `Value::Block` on the RHS; those fall through to the existing `parse_block` expansion path which uses `OpFinishRecord` and already handles ref-params correctly.

The key implementation detail: a `&vector<T>` parameter is passed via `OpCreateStack`, which stores the caller's actual vector `DbRef` in field 0 of the temp record. `generate_var` for `RefVar(Vector)` emits `OpVarRef + OpGetStackRef(0)` — this correctly retrieves the caller's vector. `OpAppendVector` then appends to that vector in place, so the caller sees the change.

`find_written_vars` already recognises `OpAppendVector` as a write (via a pre-existing check on the opcode name), so the "Parameter 'v' has & but is never modified; remove the &" error is suppressed correctly.

---

### Function references — `parse_fn_ref`

The `fn <name>` atom expression (parsed by `parse_fn_ref`) produces a compile-time
integer containing the definition number of the named function:

```loft
fn double_score(r: const Score) -> integer { r.value * 2 }

// User-facing syntax — the parser rewrites this into an internal parallel_for call:
for a in items par(b=double_score(a), 4) { results += [b] }
```

The `fn` reference is lowered to `Value::Int(d_nr)` where `d_nr` is the definition index.
At bytecode generation this becomes `ConstInt(d_nr)`. The internal `parallel_for` native
function receives this integer and uses it to dispatch the worker. Users must not call
`parallel_for` directly; use the `par(...)` for-loop clause instead.

**Callable fn-ref variables (T1-1):** A variable or parameter of type `Type::Function` can
also be called directly via normal call syntax. `parse_call` checks whether the callee name
resolves to a local variable of `Type::Function`; if so, it emits `Value::CallRef(var_nr,
args)` instead of `Value::Call`. `generate_call_ref` in `state.rs` pushes the arguments,
computes `fn_var_dist = stack.position - var.stack_pos`, and emits
`OpCallRef | fn_var_dist: u16 | arg_size: u16` (op_code 252, declared in
`default/02_files.loft`). At runtime, `fn_call_ref` reads the `d_nr` from the variable,
looks it up in `fn_positions: Vec<u32>` (populated at the start of each execute call), and
dispatches via `fn_call`.

**Named arguments:** `parse_call` uses `lexer.peek_named_arg()` (two-token lookahead
via `link`/`revert`) to detect `name: value` syntax. Named args are resolved to
positional slots in `call_with_named`; `add_defaults` fills any remaining gaps.

`fn(T) -> R` is a valid parameter type parsed by `parse_fn_type`:
```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
```

`generate_var` handles `Type::Function` by emitting `OpVarInt` (fn refs are stored as `i32`
d_nr). `find_fn` in `data.rs` returns early to the `n_` global lookup when
`type_def_nr` returns `u32::MAX`, preventing a panic on `Function`-typed method dispatch.

### Reverse collection iteration — `rev(sorted_col)`

`parse_in_range()` recognises `rev(<expr>)` with no `..` when the expression type is
`Sorted` or `Index`.  It sets `Parser::reverse_iterator = true` and consumes the closing `)`.
`fill_iter()` checks this flag and ORs bit 64 into the `on` byte of the OpIterate/OpStep
instruction pair.  The flag is reset after both `fill_iter` calls inside `iterator()`, and
also on the first-pass early return (so it does not persist across parse passes).

At runtime, `state::step()` for type-2 (sorted) detects `on & 64` and calls
`vector::vector_step_rev()` instead of `vector_step()`. `vector_step_rev` treats any
position `>= length` (the value produced by `iterate()` for the "not started" sentinel)
as "start at the last element", then decrements on each call, and returns `i32::MAX`
when the beginning has been passed.

**Type-3 (`ordered`) is the same protocol in BYTE OFFSETS, and it is a separate pair of
functions for that reason alone.** An `ordered` holds 4-byte record-id slots, so its
cursor walks `8, 12, 16, …` rather than `0, 1, 2, …`: `vector::vector_next` /
`vector::vector_prev` are the byte-offset twins of `vector_step` / `vector_step_rev`,
and `vector::step_ordered` picks between them on the same `on & 64`.

The unit is the trap. `vector::ordered_range_cursors` is the ONE place that derives a
range's `(start, finish)` for this arm, and it answers byte offsets — both backends had
their own copy answering slot INDICES, which the stepper then read as byte offsets
(loft#904: every element of a bounded range read zero, and its reverse segfaulted). It
mirrors the type-2 index arithmetic cell for cell, because a collection's KIND must not
change what a range means and a program gets one layout or the other purely by whether
some other struct mentions a keyed collection over the element type.

`finish == 0` means NO bound in either direction — it is below every valid position, so
it never trips. That is what the unbounded form returns, and it is what keeps type-4 (a
hash / spatial / trie iteration scratch, which shares `step_ordered` and is always
unbounded) walking to its end.

### Vector comprehension machinery — `build_comprehension_code`

`[for elm in v { body }]` in an array context compiles through `parse_vector_for` →
`build_comprehension_code`. The same infrastructure is used by `map`, `filter`, and `reduce`
(T1-3, done 2026-03-15).

**Key helper functions:**

| Function | Purpose |
|---|---|
| `for_type(in_type)` | Returns the loop-variable type for an iterable (`vector<T>` → `T`) |
| `iterator(code, in_type, it, iter_var, pre_var)` | Modifies `code` in place to become the iterator-init expression (`v_set(iter_var, -1)` for vectors); returns the per-step expression that reads the next element |
| `unique_elm_var(parent_tp, assign_tp, vec)` | Creates a `Reference`-typed temp variable used as the record slot for each appended element |
| `vector_db(elem_type, vec_var)` | Returns IR ops that allocate a new database store and initialize `vec_var` to point to it |
| `build_comprehension_code(vec, elm, in_t, in_type, var_tp, for_var, for_next, pre_var, fill, create_iter, if_step, body, val, is_var, is_field, block, tp)` | Builds the full loop IR: optionally calls `vector_db`, then `fill`, then `create_iter`, then a `v_loop` containing `for_next + null-break + optional-if_step + body + OpNewRecord + set_field + OpFinishRecord` |

**Key parameters to `build_comprehension_code`:**
- `vec` — result vector variable number
- `elm` — reference variable that receives each newly allocated slot
- `in_t` — element type of the result vector (mutable; updated by the function)
- `in_type` — type of the input collection (with `depending(vec_copy_var)` already applied)
- `var_tp` — type of the loop variable (from `for_type`)
- `for_next` — `v_set(for_var, iter_next)` — assigns next element to loop variable
- `if_step` — optional filter condition: `Value::Null` = no filter; non-Null = skip element when false
- `body` — expression whose value is appended to the result vector each iteration
- `is_var=false, is_field=false, block=true` — standalone result vector (include `vector_db`, push `Value::Var(result_vec)` at end). After `vector_db`, `tp` is updated to carry the db dependency so scopes does not emit a double `OpFreeRef` for the result variable.

**Iteration pattern for `Type::Vector` (from `iterator()`):**
- `create_iter` = `v_set(iter_var, -1)` — initialize index to -1
- `iter_next` = `v_block([v_set(iter_var, iter_var + 1), OpGetVector(vec, size, iter_var)])` — increment then read
- Null check in loop: convert element to bool → if false → `Value::Break(0)`

### Parallel for-loop — `parse_parallel_for_loop`

The `par(b=<worker_call>, <threads>)` clause on a `for` loop runs a worker function on
every element of a vector in parallel and delivers results in the original order:

```loft
for a in items par(b=my_func(a), 4) { sum += b; }   // global fn
for a in items par(b=a.my_method(), 4) { sum += b; } // method
```

The parser intercepts a `for … in … par(…) { … }` pattern in `parse_for`. When the
`par(` token is found after the range expression, it calls `parse_parallel_for_loop`,
which:

1. Parses the worker call expression (either `fn(elem)` or `elem.method()`) via
   `parse_parallel_worker` to extract `(fn_d_nr, return_type)`.
2. Infers `elem_size` from the element type's Stores byte size.
3. Infers `return_size` from the primitive return type (1 for bool, 4 for single,
   8 for float and full-width integer).
4. Rewrites the loop into:
   - `par_results = parallel_for(input, elem_size, return_size, threads, fn_d_nr)`
   - A conventional for-loop over the result vector that binds `b` to each element.

The worker function must take a single `const` reference argument of the element type
and return one primitive value (integer, float, single, or boolean). Text and
reference return types are not yet supported.

The native function `n_parallel_for` in `native.rs` calls `run_parallel_raw` in
`parallel.rs`, which spawns threads using Rayon and collects results in order.

---

## IR — The `Value` tree (`src/data.rs`)

The parser produces a tree of `Value` nodes that represents a function body.

### `Value` enum

IR node variants (full definition in [INTERMEDIATE.md](INTERMEDIATE.md)):
- Literals: `Null`, `Int(i32)`, `Long(i64)`, `Float(f64)`, `Single(f32)`, `Boolean(bool)`, `Text(String)`, `Enum(u8, u16)`
- Variables: `Var(u16)` (read), `Set(u16, Box<Value>)` (write)
- Calls: `Call(u32, Vec<Value>)` — definition nr + args; `CallRef(u16, Vec<Value>)` — fn-ref variable nr + args (see [Function references](#function-references--parse_fn_ref))
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
[DESIGN_DECISIONS § C66](DESIGN_DECISIONS.md#c66--no-runtime-exceptions-in-production-loft-programs-never-abort-on-user-attributable-edge-cases),
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
- [TESTING.md](TESTING.md) — Test framework, LogConfig debug-logging presets
- [../DEVELOPERS.md](../DEVELOPERS.md) — How to add features: pipeline walkthrough, caveats per subsystem, debugging strategy
