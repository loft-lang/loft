<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# INTERFACES-history.md — the implementation record

The timeline behind [INTERFACES.md](INTERFACES.md): the implementation steps I1–I10 as they were planned and built.

## Implementation steps

Each step is independently compilable and testable. Steps I1–I6 are the core;
I7–I10 add usability and standard library support.

---

### I1 — Lexer: add `interface` keyword

**File:** `src/lexer.rs`

Add `"interface"` to the `KEYWORDS` static slice. After this step,
`interface` is tokenised as `Token("interface")` instead of
`Identifier("interface")`, making it available as a reserved keyword for
the parser.

**Test:** parsing a file that uses `interface` as an identifier should produce
a keyword-conflict error (same as `struct`, `fn`, etc.).

---

### I2 — Data: add `DefType::Interface` and `Definition.bound`

**Files:** `src/data.rs`

**2a.** Add a new variant to `DefType`:

```rust
pub enum DefType {
    // ... existing variants ...
    /// An interface declaration: a named set of required method signatures.
    /// Child definitions (via parent links) are the required method stubs.
    Interface,
}
```

**2b.** Add a `bound` field to `Definition`:

```rust
pub struct Definition {
    // ... existing fields ...
    /// For Generic functions: the def_nrs of all required interfaces (empty = no bounds).
    pub bounds: Vec<u32>,
}
```

Initialise `bounds` to `vec![]` in `Definition`'s constructor. Using a `Vec`
from the start means multiple bounds (`<T: A + B>`) requires no data model
change later — only the parser needs extending.

**Conflict detection:** if two bounds in `bounds` declare a method with the
same name but different signatures, emit an error at the `fn` declaration site:
`"interfaces A and B both declare method foo with conflicting signatures"`.
This is checked once when the bounds are resolved in the second pass.

**Test:** `Definition` constructs with `bounds = vec![]` without affecting
existing behaviour. A generic function with two bounds stores two entries.

---

### I3 — Parser first pass: parse interface declarations

**File:** `src/parser/definitions.rs`

Add a `parse_interface(&mut self) -> bool` method, called from
`parse_file`'s top-level loop alongside `parse_struct`, `parse_enum`, etc.

```
interface Ident { fn_signature* }

fn_signature = "fn" Ident "(" param_list ")" [ "->" type ] ";"
               // no body — ends with ";" or "}"
```

First pass actions:
1. Consume `interface`.
2. Read the interface name (must be `CamelCase`; emit error otherwise).
3. Call `data.add_def(name, pos, DefType::Interface)` to register it.
4. Parse each method signature. For each:
   - Call `data.add_def(method_name, pos, DefType::Function)` with
     `parent = interface_def_nr`.
   - Store parameter types and return type in the `Definition.attributes`
     or `Definition.returned` fields (same layout as a regular function stub).
   - `Self` in parameter/return types is stored as `Type::Unknown(interface_def_nr)`
     as a placeholder; it is resolved to the concrete type at instantiation.
5. Skip the body (no second-pass IR generation for interfaces).

**Test:** a file with a valid interface declaration parses without error.
An interface with a duplicate name emits the existing "already defined" diagnostic.

---

### I4 — Parser first pass: parse `<T: Bound>` syntax

**File:** `src/parser/definitions.rs`, inside `parse_function`.

The existing generic parsing detects `<T>` at the function name:

```rust
// Current code (simplified):
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    lexer.token(">");
    is_generic = true;
}
```

Extend this to optionally read `: A + B + ...` after the type variable:

```rust
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    let mut bound_names: Vec<String> = vec![];
    if lexer.has_token(":") {
        bound_names.push(lexer.identifier());       // first bound
        while lexer.has_token("+") {
            bound_names.push(lexer.identifier());   // additional bounds
        }
    }
    lexer.token(">");
    is_generic = true;
    // ...
    if !bound_names.is_empty() {
        self.pending_bounds = bound_names;   // resolved in second pass
    }
}
```

In the second pass, resolve each name in `pending_bounds` via
`data.def_nr(&name)` and push the result into `definition.bounds`. If any
name does not resolve to a `DefType::Interface`, emit "unknown interface".
After all bounds are resolved, run conflict detection (see I2).

**Test:** `fn foo<T: Ordered>(...) { ... }` stores one bound.
`fn foo<T: Ordered + Printable>(...) { ... }` stores two bounds.
`fn foo<T>(...) { ... }` stores zero bounds. Unknown interface name errors.

---

### I5 — Type resolution: validate interface bodies

**File:** `src/typedef.rs`, inside `actual_types` or a new `check_interfaces`.

After type resolution, iterate over all `DefType::Interface` definitions.
For each required method (child definitions with the interface as parent):

- Resolve all `Type::Unknown(interface_def_nr)` (the `Self` placeholder) to
  a sentinel that the satisfaction checker in I6 will substitute.
- Validate that all other types in the signature are known and concrete.
- Emit errors for unresolved types in interface bodies.

No bytecode is generated for interface definitions themselves.

**Test:** an interface with an unknown type in a method signature emits a
clear "unknown type" error. An interface with all valid types passes silently.

---

### I6 — Satisfaction checking at instantiation

**File:** `src/parser/definitions.rs`, inside the generic specialisation logic.

Currently, when a generic function `fn foo<T>(...)` is called with a concrete
`T = Point`, the compiler looks for or creates a specialised copy named
`foo_Point`. Extend this to also run a satisfaction check:

```rust
fn check_satisfaction(
    data: &Data,
    concrete_type: u32,   // def_nr of the concrete struct/enum
    bound: u32,           // def_nr of one required interface
    call_pos: &Position,
    diagnostics: &mut Diagnostics,
) {
    // Collect required method signatures from the interface's children.
    for child in data.children_of(bound) {
        let concrete_fn = data.find_method(child.name, concrete_type);
        if concrete_fn == u32::MAX {
            diagnostics.error(
                call_pos,
                &format!(
                    "{} does not satisfy interface {}: missing fn {}",
                    data.def(concrete_type).name,
                    data.def(bound).name,
                    child.name,
                )
            );
        } else {
            // Check return type and param types match (with Self → concrete_type).
        }
    }
}

// Call once per bound, per (concrete_type, generic_fn) pair:
for &bound in &definition.bounds {
    check_satisfaction(data, concrete_type, bound, call_pos, diagnostics);
}
```

Cache results per `(concrete_type, generic_fn)` pair to avoid re-checking on
every call. The cache key covers all bounds together; if any bound fails the
whole instantiation fails.

**Test:** calling `max_of([Priority{...}])` where `Priority` has `less_than`
compiles cleanly. Calling `max_of([Thing{...}])` where `Thing` lacks
`less_than` emits the "does not satisfy" error.

---

### I7 — Allow bounded method calls on T

**File:** `src/parser/control.rs` or `src/parser/objects.rs` — wherever
method calls on generic `T` currently emit the
`"generic type T: method call requires a concrete type"` error.

When `x.method(args)` is encountered and `x` is of generic type `T`:

1. Collect `definition.bounds` for the enclosing generic function.
2. Search each bound's children for a method named `method`. Stop at the
   first match.
3. If found in any bound: allow the call. The method resolves to the concrete
   implementation when the specialised copy is compiled.
4. If not found in any bound: emit the existing "method call requires a
   concrete type" error, listing all bounds that were searched.

**Test:** inside `fn find_max_and_log<T: Ordered + Printable>`, both
`result < item` and `item.to_text()` compile. A method not in either bound
still errors.

---

### I8 — Operator interfaces

**File:** `src/parser/operators.rs` — wherever operators on generic `T`
currently emit the `"operator '+' requires a concrete type"` error.

When an operator expression is encountered with a `T`-typed operand, the
procedure is:

1. Map the operator token to its `OpCamelCase` name
   (e.g. `+` → `"OpAdd"`, unary `-` → `"OpNeg"`).
2. Search **each bound** in `definition.bounds` for a child method named
   `"OpAdd"` (or the relevant name). Stop at the first match.
3. If not found in any bound: emit the existing "operator requires concrete type" error.
4. If found: validate the operand types against the interface signature:
   - For binary operators: the right-hand operand must match the declared
     second-parameter type (either `Self` → `T`, or a concrete type like
     `float`). Emit a type-mismatch error if they differ.
   - For unary operators: no second operand to check.
5. Determine the result type from the interface method's declared return type:
   - `Self` in return position → `T` (the generic type variable).
   - Any concrete type (e.g. `boolean`, `float`) → that concrete type.
   Emit the operator as a `Call(op_nr, args)` IR node with this result type.

This result-type propagation is the key addition over the boolean allow/deny
check: the generic body's type checker needs to know whether `x / y` produces
`T` or `float` in order to type-check subsequent expressions.

This requires the operator-name mapping (operator token → `OpCamelCase`)
which already exists in `src/parser/operators.rs`. It only needs to be
made accessible at the check site.

**Covered cases:**
- `T + T -> T` — same-type binary, Self return
- `T < T -> boolean` — same-type binary, concrete return
- `T * float -> T` — mixed-type binary, Self return
- `T / T -> float` — same-type binary, concrete return
- `-T -> T` — unary, Self return
- `T += T` — desugars to `T = T + T` before this stage; handled by OpAdd

**Test:** inside `fn sum_of<T: Addable>`, `total = total + item` compiles
and the result has type `T`. Inside `fn average<T: Averageable>`,
`total / len(v)` compiles and the result has type `float`. Inside
`fn id<T>` (no bound), `total = total + item` still errors.

---

### I9 — Standard library interfaces

**File:** `default/01_code.loft`

Add interface declarations at the top of the file, before the operator
definitions they describe:

```loft
pub interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}

pub interface Equatable {
    fn OpEq(self: Self, other: Self) -> boolean
    fn OpNe(self: Self, other: Self) -> boolean
}

pub interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}

pub interface Printable {
    fn to_text(self: Self) -> text
}
```

Convert the currently-native `sum_of`, `min_of`, `max_of`, `any_of`, `all_of`
from native Rust implementations to bounded generic loft functions where
feasible. Those that require operator access (`sum_of`, `min_of`, `max_of`)
depend on I8 landing first.

**Test:** existing tests for these stdlib functions pass unchanged.
A new test shows a user-defined type satisfying `Ordered` and being passed
to `max_of`.

---

### I10 — Diagnostics

**Files:** `src/diagnostics.rs`, satisfaction check in I6.

Polish the error messages from the satisfaction check:

```
error[I01]: type `Priority` does not satisfy interface `Ordered`
  --> example.loft:14:5
   |
14 |     max_of(priorities)
   |     ^^^^^^ `Ordered` required by this bound on `T`
   |
   = missing: fn OpLt(self: Priority, other: Priority) -> boolean
   = missing: fn OpGt(self: Priority, other: Priority) -> boolean
   = help: add `fn OpGt(self: Priority, other: Priority) -> boolean { ... }`
```

Also add a diagnostic for using an interface name as a type
(`x: Ordered = ...`) with a clear "interfaces cannot be used as types" message.

**Test:** a deliberately unsatisfied call produces the formatted multi-line
error. Using an interface as a variable type produces the specific message.
