
# Design — Major Algorithms in loft

This document catalogues every significant algorithm in the project, ordered by execution pipeline stage. For each algorithm it records: goals, participating functions with locations, code-size estimate, reducibility, code-quality assessment, debuggability, and enhancement opportunities.

Complexity estimates use O-notation over the number of IR nodes (N), variables (V), operators (K), or bytes in the store (B).

---

## Contents
- [1. Lexer / Tokeniser](#1-lexer--tokeniser)
- [2. Two-Pass Recursive-Descent Parser](#2-two-pass-recursive-descent-parser)
- [3. Type Resolution](#3-type-resolution)
- [4. Scope Analysis & Lifetime Management](#4-scope-analysis--lifetime-management)
- [5. Variable Liveness & Live Intervals](#5-variable-liveness--live-intervals)
- [6. Bytecode Stack Tracker](#6-bytecode-stack-tracker)
- [7. Bytecode Generation](#7-bytecode-generation)
- [8. Operator Dispatch & Execution](#8-operator-dispatch--execution)
- [9. Field Layout Calculator](#9-field-layout-calculator)
- [10. Word-Addressed Heap Allocator (Store)](#10-word-addressed-heap-allocator-store)
- [11. Type Schema & Multi-Store Manager (Stores)](#11-type-schema--multi-store-manager-stores)
- [12. B+tree (Index Collections)](#12-btree-index-collections)
- [13. Open-Addressing Hash Table](#13-open-addressing-hash-table)
- [14. Dynamic Arrays (Vector)](#14-dynamic-arrays-vector)
- [15. Radix Tree (Spatial Index)](#15-radix-tree-spatial-index)
- [16. Rust Code Generator](#16-rust-code-generator)
- [17. Text Formatting & String Utilities](#17-text-formatting--string-utilities)
- [18. PNG Image Decoder](#18-png-image-decoder)
- [19. HTML Documentation Generator](#19-html-documentation-generator)
- [20. CLI Entry Point & Default Library Loader](#20-cli-entry-point--default-library-loader)
- [Summary Table](#summary-table)

---

## 1. Lexer / Tokeniser

**Goal.** Convert a raw UTF-8 character stream into a `LexItem` token stream with source-position tracking. Support backtracking so the parser can speculatively try grammar alternatives without re-reading the source file.

**Functions.**

| Function | File | Lines |
|---|---|---|
| `Lexer::cont()` | `src/lexer.rs` | ~800 |
| `Lexer::peek()` / `peek_token()` | `src/lexer.rs` | ~820 |
| `Lexer::has_token()` / `has_identifier()` / `has_integer()` | `src/lexer.rs` | ~840 |
| `Lexer::link()` | `src/lexer.rs` | ~742 |
| `Lexer::revert(link)` | `src/lexer.rs` | ~756 |
| `hex_parse()` / `bin_parse()` / `oct_parse()` | `src/lexer.rs` | ~140–180 |

**Complexity.** O(C) per token (C = characters consumed per token). The link/revert mechanism is O(buffered-tokens) space. Total file scan: O(file_size). ~1 000 lines.

**Reducibility.** The file is already well-scoped. The link/revert buffering logic is an unusual pattern; it could be replaced by a `Peekable<Iterator>` + explicit save/restore of the read position in the source string, saving ~60 lines. Not worth it — the current model is correct and stable.

**Code quality.** Good. Single responsibility; small token-type helpers; naming is clear. The `link/revert` names are non-standard but documented. Acceptable.

**Debuggability.** Easy. Tokens are data values; a `--lex` trace flag would print them trivially. Bugs are local and manifest immediately as parse errors.

**Enhancement opportunities.**
- Add a `--lex` dump mode for tracing tokenisation.
- Intern identifier strings into a symbol table to avoid repeated heap allocation and speed up later comparisons.
- Allow the lexer to emit span information (byte range, not just start position) for better error underlining.

---

## 2. Two-Pass Recursive-Descent Parser

**Goal.** Parse loft source into a `Value` IR tree in two passes: pass 1 registers all top-level definitions (types, functions, constants) without full type-checking; pass 2 generates complete, fully-typed IR. Forward references are resolved by pass 1 registration before pass 2 runs.

**Functions.**

| Function | File | Role |
|---|---|---|
| `parse_file()` | `src/parser/mod.rs` | Top-level loop; calls typedef after pass 1 |
| `parse_function()` | `src/parser/definitions.rs` | `fn` declaration → body IR |
| `parse_struct()` | `src/parser/definitions.rs` | `struct` declaration |
| `parse_enum()` | `src/parser/definitions.rs` | `enum` declaration |
| `parse_type()` | `src/parser/mod.rs` | Identifier → `Type` enum |
| `parse_operators(precedence)` | `src/parser/expressions.rs` | Precedence-climbing binary operators |
| `parse_part()` | `src/parser/expressions.rs` | Postfix `.field` and `[index]` |
| `parse_single()` | `src/parser/expressions.rs` | Atoms: literals, vars, calls, conditionals |
| `parse_assign()` | `src/parser/expressions.rs` | Assignment and compound operators |
| `parse_string()` | `src/parser/expressions.rs` | Format-string expressions |
| `convert()` / `cast()` / `can_convert()` | `src/parser/mod.rs` | Type coercion |
| `object_init()` | `src/parser/expressions.rs` | Struct literal initialisation |
| `replace_record_ref()` | `src/parser/expressions.rs` | Substitute `Var(0)` placeholder |

**Complexity.** O(N) where N = IR nodes. Each token is visited O(1) times across the two passes combined. Precedence climbing is O(N·levels) = O(10N).

**Code quality.** Good. As of 2026-03-15 the parser has been split into six focused sub-modules (`src/parser/`): `mod.rs` (struct + core helpers), `definitions.rs` (types/enums/functions), `expressions.rs` (operators/assignments/format strings), `collections.rs` (iterators/for-loops/parallel-for), `control.rs` (control flow/calls), `builtins.rs` (parallel worker helpers). Pass selection via a boolean field (`self.first_pass`) is implicit — callers must remember what pass they are in. A dedicated `ParseContext` struct separating pass-1 and pass-2 state would be cleaner.

**Debuggability.** Medium. IR trees can be printed (via the log system). Type mismatch errors are well-reported. The main difficulty is tracing why a particular branch of `parse_single` was taken; a `--parse-trace` mode would help.

**Enhancement opportunities.**
- Replace the `first_pass: bool` field with a typed `enum ParsePass { TypeRegistration, IrGeneration }`.
- Emit structured diagnostics with source spans rather than string messages, enabling IDE integration.
- Add incremental re-parsing hooks for a future language server.

---

## 3. Type Resolution

**Goal.** After pass 1, replace all `Type::Unknown(id)` placeholders with concrete types, register struct/enum layouts in `Stores`, and compute field byte offsets. Must run before pass 2 begins.

**Functions.**

| Function | File | Role |
|---|---|---|
| `actual_types()` | `src/typedef.rs` | Resolve `Unknown` → concrete; call `fill_database` |
| `fill_database()` | `src/typedef.rs` | Register each struct/enum with `Stores` |
| `fill_all()` | `src/typedef.rs` | Call `database.finish()` to seal schema |

**Complexity.** O(T²) worst case if types have chains of forward references, but practically O(T) for T types. ~246 lines.

**Reducibility.** Already small. The three-step `actual_types → fill_database → fill_all` separation is clear. No obvious reduction.

**Code quality.** Good. Short file; each function has one purpose. The coupling to `calc.rs` via `typedef::fill_database` calling `calc::calculate_positions` is appropriate.

**Debuggability.** Easy. Type registration errors surface as clear panics or diagnostics. The schema can be dumped via `Stores` debug printing.

**Enhancement opportunities.**
- Report cycles in type definitions (struct A contains B contains A) rather than panicking.
- Emit a schema dump in a standardised format (e.g. JSON) for tooling.

---

## 4. Scope Analysis & Lifetime Management

**Goal.** Assign a scope number to every IR node; detect when owned resources (Text, Reference, Vector) go out of scope and insert `OpFreeText` / `OpFreeRef` cleanup operations. Pre-initialise variables first assigned inside branches so that the stack layout is safe regardless of which branch executes.

**Functions.**

| Function | File | Role |
|---|---|---|
| `check(data)` | `src/scopes.rs` | Entry point; iterates all functions |
| `scan(val, function, data)` | `src/scopes.rs` | Recursive IR tree traversal |
| `scan_set()` | `src/scopes.rs` | Handle `Value::Set`; insert dep initialisers |
| `scan_if()` | `src/scopes.rs` | Handle `Value::If`; pre-emit `Set(v, Null)` for branch-first vars |
| `enter_scope()` / `exit_scope()` | `src/scopes.rs` | Manage scope stack |
| `get_free_vars()` | `src/scopes.rs` | Produce `OpFree*` calls for out-of-scope vars |
| `free_vars()` | `src/scopes.rs` | Insert free ops into IR |
| `copy_variable()` | `src/scopes.rs` | Fresh slot when a name is reused across sibling scopes |

**Complexity.** O(N + V) per function: one IR traversal pass, one free-variable scan per scope exit. Total across all functions: O(N_total + V_total). ~486 lines.

**Reducibility.** Moderate. The pre-init logic for if/else (Option A sub-3) adds non-trivial complexity in `scan_if`. When slot assignment (Steps 3+4 of [SLOTS.md](SLOTS.md)) is complete, some of this pre-init complexity can be simplified. The `var_mapping` table (for `copy_variable`) is hard to reason about; a clearer ownership model would help.

**Code quality.** Fair. The algorithm is correct for owned types but borrowed-ref pre-init is still incomplete (known runtime crash). The interplay of `var_scope`, `var_mapping`, and scope stack makes invariants hard to state. Adding explicit pre/post-condition comments would improve this significantly.

**Debuggability.** Hard. Bugs manifest at runtime (wrong `store_nr`, double-free) rather than compile time. The `ref_debug` LOFT_LOG mode helps but the gap between where a variable is freed and where the bug surfaces is large.

**Enhancement opportunities.**
- Model liveness explicitly (as computed by `compute_intervals`) during scope analysis, eliminating the separate liveness pass.
- Replace `var_mapping` with a proper SSA-like renaming pass, which would also simplify slot assignment.
- Finish borrowed-ref pre-init (Steps 3+4 of [SLOTS.md](SLOTS.md)).

---

## 5. Variable Liveness & Live Intervals

**Goal.** Walk the IR in execution order and record, for each variable, the sequence number of its first definition (`first_def`) and last use (`last_use`). These intervals are later used to detect stack slot conflicts.

**Functions.**

| Function | File | Role |
|---|---|---|
| `compute_intervals(val, function, free_text_nr, free_ref_nr, seq)` | `src/variables/` | Walk IR; populate `first_def`/`last_use` |
| `validate_slots(function, data, def_nr)` | `src/variables/` | Post-codegen conflict checker |
| `find_conflict(vars)` | `src/variables/` | O(V²) overlap scan |
| `size(tp, context)` | `src/variables/` | Bytes needed for a type on the stack |

**Complexity.** `compute_intervals`: O(N). `validate_slots` / `find_conflict`: O(V²) — acceptable because V is small per function (< 100 typically). Total file: ~1 166 lines.

**Reducibility.** The O(V²) scan in `find_conflict` could be replaced by an interval-graph sweep (sort by `first_def`, scan with active-interval set) for O(V log V). For current function sizes this is not a bottleneck. The `size()` function duplicates type-size knowledge from `calc.rs`; consolidating them would reduce drift risk.

**Code quality.** Good for `compute_intervals`. The critical ordering (process `Set` value expression before recording `first_def`) is documented in comments. `validate_slots` is defensive (panics on conflict) — appropriate for a correctness check. The `size()` duplication is a minor quality issue.

**Debuggability.** Medium. When a conflict is detected `validate_slots` dumps all variable details, which is helpful. The hard part is understanding *why* two intervals overlap — the IR printed by `LOFT_LOG` helps but the mapping from IR node to sequence number is not displayed.

**Enhancement opportunities.**
- Print the sequence-number annotation alongside the IR dump for easier correlation.
- Replace O(V²) scan with an interval-graph sweep.
- Unify `size()` with `calc.rs` to eliminate dual sources of type-size truth.
- Promote `validate_slots` from a panic to a diagnostic so the test suite can report multiple conflicts per function.

---

## 6. Bytecode Stack Tracker

**Goal.** During bytecode emission, maintain an accurate count of the current operand stack depth in bytes. This ensures that `OpFreeStack` amounts are correct and that `break` targets are reached with the right stack depth.

**Functions.**

| Function | File | Role |
|---|---|---|
| `Stack::new()` | `src/stack.rs` | Construct stack frame for one function |
| `Stack::operator(d_nr)` | `src/stack.rs` | Update position after opcode emission |
| `Stack::size_code(val)` | `src/stack.rs` | Bytes a `Value` node produces |
| `Stack::add_loop(code_pos)` | `src/stack.rs` | Push loop frame |
| `Stack::end_loop(state)` | `src/stack.rs` | Pop loop frame; patch break gotos |
| `Stack::add_break(code_pos, loop_nr)` | `src/stack.rs` | Register pending break goto |
| `Stack::get_loop(loop_nr)` | `src/stack.rs` | Get loop start position |
| `Stack::loop_position(loop_nr)` | `src/stack.rs` | Stack depth at loop entry |

**Complexity.** O(1) per operator emission. Loop break patching: O(breaks_per_loop). ~175 lines.

**Reducibility.** Already small. Nothing obvious to remove.

**Code quality.** Good. Clear struct layout; methods have single responsibilities. The `size_code` function duplicates some IR-node type logic from the parser — a shared utility would be cleaner.

**Debuggability.** Medium. When a stack-depth mismatch occurs (e.g. `OpFreeStack` over-releases), the error manifests as a memory corruption or wrong value at runtime, not at codegen time. Adding a runtime assertion that checks `stack_pos == expected_pos` at function entry/exit would help.

**Enhancement opportunities.**
- Add a codegen-time assertion that stack depth returns to baseline after each statement.
- Unify `size_code` with the variable `size()` function in `variables/`.

---

## 7. Bytecode Generation

**Goal.** Compile the `Value` IR tree for each function into a flat bytecode stream. Map each IR node to one or more opcodes with inline operands; manage variable positions; emit control-flow jumps and patch forward references.

**Functions.**

| Function | File | Role |
|---|---|---|
| `byte_code(state, data)` | `src/compile.rs` | Iterate functions; call `def_code` for each |
| `State::def_code(d_nr, data)` | `src/state/codegen.rs` | Compile one function's IR → bytecode |
| `State::value_code(val, stack, data)` | `src/state/codegen.rs` | Recursive IR-node → opcode emitter |
| `State::code_pos()` | `src/state/mod.rs` | Current bytecode write position |
| `State::put_code(byte)` / `put_word(u16)` / `put_long(u32)` | `src/state/mod.rs` | Emit raw bytes |
| `State::patch(pos, offset)` | `src/state/codegen.rs` | Back-patch a forward jump target |

**Complexity.** O(N) IR nodes → O(K) opcodes where K ≈ 2–5 per IR node.

**Code quality.** Good. As of 2026-03-15 `state/` has been split into five sub-modules: `mod.rs` (struct + stack primitives), `codegen.rs` (bytecode generation), `text.rs` (string/text ops), `io.rs` (file I/O + record ops), `debug.rs` (dump/trace). The `HashMap`-based debug metadata (`stack`, `vars`, `calls`, `types`) is useful for testing but adds noise to the core code path.

**Debuggability.** Medium. The `static` LOFT_LOG preset shows IR + bytecode without execution, which is the right level for codegen bugs. Forward-reference patching errors (wrong jump target) are hard to spot without a bytecode disassembler.

**Enhancement opportunities.**
- Add a bytecode disassembler (opcode → human-readable name + operands) for debugging.
- Replace the four `HashMap` debug tables with a single `DebugInfo` struct.
- Consider SSA form as an intermediate step between `Value` IR and bytecode to simplify optimization passes.

---

## 8. Operator Dispatch & Execution

**Goal.** Execute the 233-opcode bytecode by dispatching each opcode byte through a static function-pointer array `OPERATORS[opcode]`. Each operator reads inline operands from the bytecode stream and reads/writes the operand stack.

**Functions.**

| Function | File | Role |
|---|---|---|
| `OPERATORS` array | `src/fill.rs` | 233 `fn(&mut State)` pointers |
| `op_*` functions (×233) | `src/fill.rs` | Individual operator implementations |
| `State::execute()` | `src/state/mod.rs` | Main interpreter loop |
| `State::get_stack::<T>()` / `put_stack::<T>()` | `src/state/mod.rs` | Typed stack pop/push |
| `State::code::<T>()` | `src/state/mod.rs` | Read inline operand from bytecode |

**Complexity.** O(I) instructions executed. Each operator is O(1) or O(field_count) for struct operations. `fill.rs` is ~1 891 lines (generated).

**Reducibility.** `fill.rs` is machine-generated from `#rust` annotations via `generate_code()`. Its size is inherent in the opcode count. The generator could be made smarter (e.g. generating a disassembly table alongside), but the output file itself is not hand-maintained and should not be reduced manually.

**Code quality.** Good for a generated file. The naming convention (`op_snake_case`) is consistent. The `op_return` special case is documented. Because the file is generated, code quality is enforced at the generator level (`create.rs`).

**Debuggability.** Good. The `minimal` and `ref_debug` LOFT_LOG presets provide execution traces. The `crash_tail:N` preset captures the last N lines before a panic. The main gap is a human-readable disassembler.

**Enhancement opportunities.**
- Generate a disassembler table alongside `OPERATORS`.
- Consider a threaded-code or direct-threading dispatch for performance (though throughput is not currently a bottleneck).
- Add a coverage counter per opcode for testing completeness.

---

## 9. Field Layout Calculator

**Goal.** Compute the byte offset of each field in a struct or enum-variant record, packing fields efficiently with alignment gaps filled by smaller fields.

**Functions.**

| Function | File | Role |
|---|---|---|
| `calculate_positions(fields, sub, size, alignment)` | `src/calc.rs` | Main layout algorithm |

**Complexity.** O(F²) where F = number of fields (F typically < 20). The gap-filling inner loop scans the gap map for each field. ~78 lines.

**Reducibility.** Already minimal. The `BTreeMap<pos, size>` gap map is a clean data structure for the problem. Could be replaced with a simple greedy first-fit, shaving ~10 lines at the cost of slight suboptimality. Not worth it.

**Code quality.** Good. Tiny, single-purpose file. The `sub=true` discriminant reservation is a minor special case that could be generalised to "reserve first N bytes" but that adds abstraction without current benefit.

**Debuggability.** Easy. The algorithm is deterministic and can be unit-tested with small structs. Bugs would be layout mismatches visible as field read/write corruption.

**Enhancement opportunities.**
- Add a struct layout dump (field name, type, offset, size) to the documentation generator output.
- Enforce alignment padding so total `size` is always a multiple of `alignment` (required for arrays of structs). Currently may be omitted for the last field.

---

## 10. Word-Addressed Heap Allocator (Store)

**Goal.** Provide a fast, in-process heap for all runtime allocations (stack frames, struct instances, vectors, collection nodes) using 8-byte word addressing, first-fit allocation, and adjacent-block coalescing on free.

**Functions.**

| Function | File | Role |
|---|---|---|
| `Store::claim(size)` | `src/store.rs` | First-fit allocate `size` words |
| `Store::delete(rec)` | `src/store.rs` | Free record; coalesce neighbours |
| `Store::resize(rec, size)` | `src/store.rs` | Grow or shrink in-place / relocate |
| `Store::validate()` | `src/store.rs` | Debug: verify header consistency |
| `Store::get_int(rec, pos)` / `set_int()` | `src/store.rs` | 4-byte typed accessors |
| `Store::get_long(rec, pos)` / `set_long()` | `src/store.rs` | 8-byte typed accessors |
| `Store::get_str(rec, pos)` / `set_str()` | `src/store.rs` | String pointer+length accessors |
| `Store::buffer(rec)` | `src/store.rs` | Raw mutable byte slice (for PNG decode) |

**Complexity.** `claim`: O(log F) fast path via LLRB free-space tree (F = tracked free
blocks), O(B) linear-scan fallback for tiny blocks (< 2 words) or first allocation.
`delete`: O(log F) (tree insert). `resize`: O(B) if relocation needed.

**Reducibility.** Moderate. The LLRB tree + linear scan dual-path is appropriately sized.
The doubling growth strategy is standard. The typed accessor pairs (`get_int`/`set_int`
× 4 types) are repetitive — a macro would halve them (~40 lines).

**Code quality.** Good. Clear header convention (positive = live, negative = free).
LLRB free-space tree provides O(log F) allocation for most cases. `validate()` and
`fl_validate()` debug functions catch corruption and tree invariant violations.

**Debuggability.** Medium. Memory corruption in the store often manifests far from the allocation site. The `validate()` function helps catch corruption early. Adding a canary word at record boundaries would help detect overflows.
- Add boundary canaries (debug mode) for overflow detection.
- Replace the typed accessor proliferation with a single generic `get::<T>` / `set::<T>` using Rust generics.
- Add allocation statistics (total allocated, fragmentation ratio) for profiling.

---

## 11. Type Schema & Multi-Store Manager (Stores)

**Goal.** Maintain the type registry (struct layouts, enum variants, field metadata) and own the set of runtime stores. Provide a unified API for type registration, store allocation, and record allocation that the interpreter uses.

**Functions.**

| Function | File | Role |
|---|---|---|
| `Stores::structure(name)` | `src/database/types.rs` | Register a struct type |
| `Stores::field(type_nr, name, field_type)` | `src/database/types.rs` | Add field to struct |
| `Stores::enumerate(name)` | `src/database/types.rs` | Register an enum type |
| `Stores::value(enum_nr, discriminant, name)` | `src/database/types.rs` | Add enum variant |
| `Stores::finish()` | `src/database/types.rs` | Seal schema; compute sizes/alignment |
| `Stores::allocate()` | `src/database/allocation.rs` | Create a new `Store` |
| `Stores::database(size)` | `src/database/allocation.rs` | Allocate a top-level record |
| `Stores::byte(min, nullable)` / `Stores::short(...)` | `src/database/types.rs` | Register compact integer types |
| `Stores::read_data()` / `write_data()` | `src/database/io.rs` | Binary serialisation |

**Complexity.** Schema registration: O(T·F) for T types with F fields each. Runtime allocation: delegates to `Store::claim` = O(log F) fast path.

**Code quality.** Good. As of 2026-03-15 `database/` has been split into seven sub-modules: `mod.rs` (constructor + parse-key helpers), `types.rs` (type-building), `allocation.rs` (claim/free/clone), `search.rs` (find/iterate), `structures.rs` (record construction), `io.rs` (file I/O), `format.rs` (display). The `Parts` enum with 14 variants is the natural extension point for new collection types.

**Debuggability.** Medium. The type schema dump (printed by the test framework) is useful. Runtime allocation bugs are hard to trace because `Stores` delegates to `Store`. A higher-level allocation log (which function allocated which record) would help.

**Enhancement opportunities.**
- Add schema version tagging to `write_data` / `read_data` for forward compatibility.
- Provide a human-readable schema dump (type, fields, offsets) as a debug command.
- Add store-level allocation histograms for memory profiling.

---

## 12. B+tree (Index Collections)

**Goal.** Provide an ordered, balanced key structure for `index` collections. Supports ordered iteration, predecessor/successor queries, and O(log N) insert/find/remove; `len` is O(1).

**Functions.**

| Function | File | Role |
|---|---|---|
| `find` / `find_exact` | `src/btree.rs` | O(log N) boundary and point lookup |
| `add` | `src/btree.rs` | Insert; answers the record already carrying the key |
| `remove` | `src/btree.rs` | Delete a record, found by its own key |
| `count` | `src/btree.rs` | O(1) length from the header |
| `next_record` / `previous_record` | `src/btree.rs` | In-order traversal by hint, verified |

**Complexity.** O(log N) for find, insert, remove. O(N) for full traversal.

**Layout.** Nodes of 16 slots in one arena record of the collection's store; a slot holds an 8-byte key part (the first key field, order-preserving) and a record number. The element record carries no tree fields. Layout: [DATABASE_INDEXES.md](DATABASE_INDEXES.md).

**Debuggability.** `tests/btree.rs` checks the tree against a sorted model for every key kind, including walks under concurrent removal.

---

## 13. Open-Addressing Hash Table

**Goal.** Provide O(1) average-case key lookup and insert for `Hash` and `Index` collection types using linear probing with an 87.5% load-factor trigger for rehashing.

**Functions.**

| Function | File | Role |
|---|---|---|
| `add(hash, rec, stores, keys)` | `src/hash.rs` | Insert record; rehash if overloaded |
| `find(hash_ref, stores, keys, key)` | `src/hash.rs` | Lookup by key |
| `hash_set(claim, rec, stores, keys)` | `src/hash.rs` | Place record in table |
| `hash_free_pos(claim, rec, stores, keys)` | `src/hash.rs` | Find first free slot via linear probe |

**Complexity.** O(1) average case; O(N) worst case (all keys collide). Rehash: O(N). Load factor threshold: 87.5% (14/16). ~190 lines.

**Reducibility.** Already compact. The rehash logic (allocate 2× table, re-insert all) is standard.

**Code quality.** Good. Small file; linear probing is simple to reason about. The 87.5% threshold is high — typical practice is 75%. High load increases probe length and degrades cache behaviour.

**Debuggability.** Medium. Hash collisions and probe chains are invisible without instrumentation. A `dump_hash()` debug function showing occupancy per slot would help.

**Enhancement opportunities.**
- Lower the load-factor threshold to 75% (12/16) for better average probe length.
- Add Robin Hood or backward-shift deletion so that `remove()` works correctly without tombstones.
- Add collision statistics (max probe length, average probe length) in debug builds.

---

## 14. Dynamic Arrays (Vector)

**Goal.** Provide resizable arrays of value-typed elements (`Vector`) and reference-typed elements (`Array`). Support O(1) amortised append, O(N) remove, and O(1) index access.

**Functions.**

| Function | File | Role |
|---|---|---|
| `new(elem_type)` | `src/vector.rs` | Allocate empty vector record |
| `append(vec, elem, stores)` | `src/vector.rs` | Append element; grow if needed |
| `remove(vec, index, stores)` | `src/vector.rs` | Remove at index; shift tail |
| `get(vec, index, stores)` | `src/vector.rs` | Read element by index |
| `finish(vec, stores)` | `src/vector.rs` | Finalise allocation |
| `validate(vec, stores)` | `src/vector.rs` | Debug: verify count consistency |

**Complexity.** Append: O(1) amortised. Remove: O(N). Get: O(1). ~433 lines.

**Reducibility.** Moderate. The `By-Value` vs `By-Reference` duality (Vector vs Array) doubles code paths. A trait or macro abstraction would halve the duplication.

**Code quality.** Good. The distinction between `Vector` (value) and `Array` (reference) is important for correctness and is respected throughout. The `validate()` function is good defensive practice.

**Debuggability.** Easy. Index out-of-bounds is immediately detectable; element type mismatches are caught by the store's typed accessors.

**Enhancement opportunities.**
- Unify Vector and Array code paths with a generic element-access trait.
- Add `insert(vec, index, elem)` as a complement to `remove`.
- Add a `reserve(vec, capacity)` to avoid repeated reallocations when the final size is known.

---

## 15. Radix Tree (Spatial Index)

**Goal.** Provide a compact bit-indexed tree for the `Spatial` collection type, supporting O(log N) insert and range/nearest-neighbour queries via bit-by-bit key decomposition.

**Functions.**

| Function | File | Role |
|---|---|---|
| `rtree_init(store, initial)` | `src/radix_tree.rs` | Allocate tree + bits records |
| `rtree_first(store, tree)` | `src/radix_tree.rs` | Leftmost leaf iterator |
| `rtree_last(store, tree)` | `src/radix_tree.rs` | Rightmost leaf iterator |
| `rtree_find(store, tree, key)` | `src/radix_tree.rs` | Find by bit-predicate key |
| `rtree_insert(store, tree, rec, key)` | `src/radix_tree.rs` | Insert via bit-by-bit key |
| `rtree_validate(store, tree, key)` | `src/radix_tree.rs` | Debug count check |
| `rtree_optimize(store, tree)` | `src/radix_tree.rs` | Stub — not implemented |

**Status.** Partially implemented. `next()` iteration and `remove()` are stubs. `#![allow(dead_code)]` gate indicates it is not yet integrated. ~288 lines.

**Complexity.** O(K) per operation where K = key bit length. Currently functional only for insert and point queries.

**Reducibility.** Not applicable until the algorithm is complete.

**Code quality.** Incomplete — not yet production-quality. The `RadixIter` path-recording approach is clean. The companion bits record (path-compression skip counts) is a non-obvious two-record design.

**Debuggability.** Hard — the path-compression encoding is not documented clearly enough to verify correctness by inspection.

**Enhancement opportunities.**
- Implement `rtree_next()` to make iteration complete.
- Implement `rtree_remove()`.
- Document the record layout and bit-compression scheme with a diagram.
- Consider whether a k-d tree would better serve 2D spatial queries (more natural for Image coordinates).

---

## 16. Rust Code Generator

**Goal.** Emit two Rust source files from the compiled loft default library: `fill.rs` (233 operator implementations) and `native.rs` (standard library function stubs). These are the files actually compiled into the interpreter binary.

**Functions.**

| Function | File | Role |
|---|---|---|
| `generate_lib(data)` | `src/create.rs` | Write `tests/generated/text.rs` (library functions) |
| `generate_code(data)` | `src/create.rs` | Write `tests/generated/fill.rs` (operators) |
| `operator_name(operator)` | `src/create.rs` | `OpCamelCase` → `op_snake_case` |
| `rust_type(tp, context)` | `src/create.rs` | loft `Type` → Rust type string |
| `replace_attributes(body)` | `src/create.rs` | `@param` → `v_param` substitution |

**Complexity.** O(D) where D = number of definitions. File I/O bound. ~150 + 1 075 lines (create.rs + generation/).

**Reducibility.** Moderate. `generation/` is large because it handles all operator categories. A template-based approach (Tera, Askama) would reduce the string-building boilerplate by ~200 lines.

**Code quality.** Good. The `operator_name` conversion is clean and covers the `return` keyword edge case. The `@param` substitution is a simple but fragile string-replace; it would break if a parameter name appeared inside a string literal.

**Debuggability.** Easy. The generated files can be inspected directly; `cargo fmt` normalises them. Errors in generation produce malformed Rust which the compiler catches immediately.

**Enhancement opportunities.**
- Replace string-building with a Rust code generation library (e.g. `quote!` macro or template engine).
- Fix the `@param` substitution to be token-aware rather than string-replace (avoids collisions with string literals).
- Generate a disassembly table in `fill.rs` mapping opcode numbers to human-readable names.
- Add generation of type-check assertions that verify stack state before/after each operator (debug build only).

---

## 17. Text Formatting & String Utilities

**Goal.** Provide UTF-8-safe string slicing (with negative indexing and code-point boundary adjustment), and numeric formatting for all primitive types with radix, width, alignment, and null-sentinel handling.

**Functions.**

| Function | File | Role |
|---|---|---|
| `text_character(val, from)` | `src/ops.rs` | Get char at byte position |
| `sub_text(val, from, till)` | `src/ops.rs` | Zero-copy substring |
| `fix_from(from, s)` / `fix_till(till, from, s)` | `src/ops.rs` | Boundary alignment |
| `format_text(s, val, width, dir, token)` | `src/ops.rs` | Pad string to width |
| `format_int(s, val, radix, width, token, plus, note)` | `src/ops.rs` | Format i32 |
| `format_long(s, val, radix, width, ...)` | `src/ops.rs` | Format i64 |
| `format_float(s, val, width, precision)` | `src/ops.rs` | Format f64 |
| `format_single(s, val, width, precision)` | `src/ops.rs` | Format f32 |
| `op_add_int` / `op_min_int` / ... (arithmetic) | `src/ops.rs` | Null-sentinel arithmetic |

**Complexity.** O(len) for formatting; O(C) per character slice where C = bytes from boundary. ~315 lines.

**Reducibility.** The `format_int` / `format_long` and `format_float` / `format_single` pairs are near-identical. A generic `format_num<T>` would halve them (~30 lines). The null-sentinel arithmetic functions are repetitive; a macro would reduce boilerplate.

**Code quality.** Good. UTF-8 boundary handling is explicit and correct. Null propagation is consistent. The file mixes formatting (presentation) with arithmetic (semantics) — splitting them into `text_format.rs` and `arithmetic.rs` would improve single-responsibility.

**Debuggability.** Easy. Formatting errors produce wrong output, not crashes. Null propagation can be unit-tested in isolation.

**Enhancement opportunities.**
- Use Rust generics or macros to unify `format_int`/`format_long` and `format_float`/`format_single`.
- Split into `text_format.rs` and `arithmetic.rs`.
- Add a `format_character(s, val, width, dir)` for symmetric character formatting.

---

## 18. PNG Image Decoder

**Goal.** Decode a PNG file directly into a `Store` word-addressed buffer, returning the record offset for use as a loft `Image` value. Avoids a copy by writing the decoded pixels directly into a pre-claimed store allocation.

**Functions.**

| Function | File | Role |
|---|---|---|
| `read(file_path, store)` | `src/png_store.rs` | Decode PNG → `(img_rec, width, height)` |

**Complexity.** O(W×H) pixels. Dominated by PNG decode time. ~89 lines.

**Reducibility.** Already minimal. Single-purpose file.

**Code quality.** Good. The zero-copy decode via `store.buffer()` is an efficient approach. Error handling propagates `io::Result`. The only concern is that the store record size is computed as `output_buffer_size / 8 + 1` without checking alignment requirements.

**Debuggability.** Easy. PNG decode errors are well-reported by the `png` crate. Buffer size mismatches would manifest as out-of-bounds panics.

**Enhancement opportunities.**
- Add support for multi-frame (animated) PNGs.
- Validate that the claimed store size exactly covers the output buffer (assert in debug mode).
- Add JPEG / WebP support via the `image` crate for broader format coverage.

---

## 19. HTML Documentation Generator

**Goal.** Parse loft source files for `//`-prefixed doc comments and `## Section` headers, then render them as navigable HTML pages.

**Functions.**

| Function | File | Role |
|---|---|---|
| `parse_loft()` | `src/gendoc.rs` | Extract doc sections from loft source |
| `parse_section()` | `src/gendoc.rs` | Identify `## Title` markers |
| `emit_html()` | `src/documentation.rs` | Render sections to HTML |

**Complexity.** O(file_size) per file. ~626 + 743 lines.

**Reducibility.** Moderate. A Markdown-to-HTML library (e.g. `pulldown-cmark`) could replace the custom parser and HTML emitter, shrinking the combined 1 369 lines to ~200 while gaining standard Markdown feature support.

**Code quality.** Adequate. The two-file split (parsing vs rendering) is correct. Custom HTML generation is fragile — any user-supplied string could break the output if not escaped. Review for XSS-equivalent issues (invalid HTML from unescaped content).

**Debuggability.** Easy. Bugs produce malformed HTML visible in any browser's inspector.

**Enhancement opportunities.**
- Replace custom Markdown parser with `pulldown-cmark`.
- Add HTML entity escaping for all user-supplied strings.
- Add a table-of-contents sidebar based on `## Section` headers.
- Support `//!` module-level doc comments for file-level descriptions.

---

## 20. CLI Entry Point & Default Library Loader

**Goal.** Provide the `loft` binary CLI, auto-detect the project root, load the default library in alphabetical order, parse the user file, run scope analysis and bytecode generation, and execute `main`.

**Functions.**

| Function | File | Role |
|---|---|---|
| `main()` | `src/main.rs` | CLI arg parsing and full pipeline |
| `project_dir()` | `src/main.rs` | Auto-detect project root from executable path |

**Complexity.** Pipeline orchestration: O(1) calls to O(N) sub-algorithms. ~50–80 lines.

**Reducibility.** Already minimal. The nine-step pipeline in `main()` maps cleanly to the nine phases of the system.

**Code quality.** Good. Clear linear flow; each phase is a single function call. The `--path` override for project dir is a useful escape hatch. The auto-detection based on executable path is fragile for symlinks or unusual install layouts.

**Debuggability.** Easy. All phases have their own diagnostics; errors propagate and are printed before exit.

**Enhancement opportunities.**
- Add a `--check` flag that runs only parse + scope analysis (no execution) for CI.
- Add a `--dump-ir` flag to print the IR tree without compiling.
- Handle symlinked executables correctly in `project_dir()`.
- Support reading from stdin (`loft -`) for piped use.

---

## Summary Table

| # | Algorithm | File(s) | LOC (approx) | Complexity | Split recommended | Quality | Debuggability |
|---|---|---|---|---|---|---|---|
| 1 | Lexer | lexer.rs | 1 116 | O(file) | No | Good | Easy |
| 2 | Two-pass parser | ~~parser.rs~~ → **src/parser/** (6 modules) | 7 873 | O(N) | ~~**Yes**~~ **Done 2026-03-15** | Good | Medium |
| 3 | Type resolution | typedef.rs | 246 | O(T) | No | Good | Easy |
| 4 | Scope analysis | scopes.rs | ~486 | O(N+V) | No | Fair | Hard |
| 5 | Live intervals | variables/ | 1 166 | O(N+V²) | No | Good | Medium |
| 6 | Stack tracker | stack.rs | 175 | O(1)/op | No | Good | Medium |
| 7 | Bytecode generation | compile.rs + **src/state/** (5 modules) | 3 888 | O(N) | No | Good | Medium |
| 8 | Operator dispatch | fill.rs | 1 799 | O(I) | No (generated) | Good | Good |
| 9 | Field layout | calc.rs | 78 | O(F²) | No | Good | Easy |
| 10 | Heap allocator | store.rs | 1 126 | O(log F) claim | No | Good | Medium |
| 11 | Type schema + stores | ~~database.rs~~ → **src/database/** (7 modules) | 3 931 | O(T·F) | ~~**Yes**~~ **Done 2026-03-15** | Good | Medium |
| 12 | B+tree | btree.rs | 1 651 | O(log N) | No | Good | Medium |
| 13 | Hash table | hash.rs | 190 | O(1) avg | No | Good | Medium |
| 14 | Dynamic arrays | vector.rs | 433 | O(1) / O(N) | No | Good | Easy |
| 15 | Radix tree | radix_tree.rs | 288 | O(K) | No | Incomplete | Hard |
| 16 | Rust code generator | create.rs + generation/ | 1 225 | O(D) | No | Good | Easy |
| 17 | Text formatting | ops.rs | 315 | O(len) | Marginal | Good | Easy |
| 18 | PNG decoder | png_store.rs | 89 | O(W×H) | No | Good | Easy |
| 19 | Documentation gen | gendoc.rs + documentation.rs | 1 369 | O(file) | **Yes** (use crate) | Fair | Easy |
| 20 | CLI / loader | main.rs | ~80 | O(1) | No | Good | Easy |

**Highest-value remaining improvements** (splits done 2026-03-15):
1. ~~Split `parser.rs` / `state.rs` / `database.rs`~~ **Done** — `src/parser/` (6), `src/state/` (5), `src/database/` (7) modules.
2. Complete radix tree (`rtree_next`, `rtree_remove`) and finish borrowed-ref pre-init in `scopes.rs`.
3. `assign_slots()` compile-time pass in `variables/` (A6) — eliminates runtime `claim()` and removes slot conflicts in long functions.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog with effort/impact estimates
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [../DEVELOPERS.md](../DEVELOPERS.md) — Feature proposal process, known caveats per subsystem, debugging strategy
