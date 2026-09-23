
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Stack Trace Introspection

> **Status: phases 1–4 completed in 0.8.3.  Phases 5–6 (local variable
> inspection via `stack_trace_full()`) deferred to 1.1+.**

`stack_trace()` returns a snapshot of the current call stack as a
`vector<StackFrame>`, giving loft programs structured access to function names,
source locations, and live argument/variable values at the point of the call.

---

## Types and API

All types are declared in `default/04_stacktrace.loft`.  See that file for the
canonical definitions of `ArgValue`, `ArgInfo`, `VarInfo`, and `StackFrame`.

```loft
pub fn stack_trace() -> vector<StackFrame>;
```

Returns the call stack as a vector of frames ordered **outermost first**:
index 0 is the entry point; the last element is the direct caller.  The vector
is fully materialised at the moment of the call.  Each frame's `variables`
field contains live local variables at that frame's call site.

---

## Usage Example

```loft
fn inspect_arg(v: ArgValue) -> text {
    match v {
        NullVal                       => "null",
        BoolVal   { b }               => "{b}",
        IntVal    { n }               => "{n}",
        LongVal   { n }               => "{n}",
        FloatVal  { f }               => "{f}",
        SingleVal { f }               => "{f}",
        CharVal   { c }               => "'{c}'",
        TextVal   { t }               => "\"{t}\"",
        RefVal    { store, rec, pos } => "ref({store},{rec},{pos})",
        FnVal     { d_nr }            => "fn#{d_nr}",
        OtherVal  { description }     => "<{description}>",
    }
}

fn assert_positive(n: integer) {
    if n <= 0 {
        for frame in stack_trace() {
            println("{frame.file}:{frame.line}  {frame.function}");
            for arg in frame.arguments {
                println("  {arg.name}: {arg.type_name} = {inspect_arg(arg.value)}");
            }
        }
        assert(false, "n must be positive");
    }
}
```

---

## Implementation summary

| Component | File | What it does |
|---|---|---|
| Type declarations | `default/04_stacktrace.loft` | `ArgValue`, `ArgInfo`, `VarInfo`, `StackFrame` |
| Shadow call-frame vector | `src/state/mod.rs` | `CallFrame` struct, `call_stack: Vec<CallFrame>` on `State` |
| Native binding | `src/native.rs` | `n_stack_trace` — materialises `vector<StackFrame>` from snapshot |
| Variable snapshots | `src/database/mod.rs` | `VarSnapshot`, `VarValueSnapshot`, `variables_snapshot` field |
| Frame variable iterator | `src/state/debug.rs` | `iter_frame_variables`, `iter_frame_variables_at`, `peek_at` |
| Diagnostic dump | `src/state/debug.rs` | `dump_frame_variables` — per-opcode variable dump |
| Tests | `tests/frame_vars.rs` | 10 tests for the introspection framework |

---

## Safety concerns

| ID | Concern | Resolution |
|---|---|---|
| SC-ST-1 | Text null sentinel is `Str::new(STRING_NULL)`, not a null pointer | Correct null check in materialisation |
| SC-ST-2 | `Str` may borrow a `String` buffer; shallow copy dangles | Always `str.str().to_owned()` |
| SC-ST-3 | Re-entrant calls during materialisation mutate `call_stack` | `call_stack.clone()` snapshot first |
| SC-ST-4 | `fn_call`'s `_size` is local-var space, not args size | Extended with explicit `d_nr`, `args_size`, `local_size` |
| SC-ST-5 | No bounds guard on argument reads | Per-parameter `offset + size <= args_size` guard |
| SC-ST-6 | `RefVal` coordinates dangle after source frame returns | Documented as point-in-time snapshot |
| SC-ST-7 | Trace in worker stores cannot cross thread boundaries | Documented restriction |

---

## Known Limitations

| ID | Limitation |
|---|---|
| ST-1 | `RefVal` exposes raw DbRef coordinates; dereferencing requires native code |
| ST-2 | `RefVal` coordinates are point-in-time; may describe freed records if trace is retained |
| ST-3 | Native calls (via `library`) do not appear as frames |
| ST-4 | `line` is `0` for compiler-synthesised call sites |
| ST-5 | `stack_trace()` in a `par(...)` worker returns only the worker's frames |
| ST-6 | Under `--native` each frame's `variables` is empty: a compiled frame has no reader for its locals, so only the interpreter reports them |

---

## Remaining work: Phases 5–6 (1.1+)

Phase 5 adds a `debug_symbols` feature flag and `LocalVarMeta` records on
`Definition` for each local variable's name, type, stack position, and
bytecode live range.  Phase 6 implements `stack_trace_full()` which populates
the `variables` field using these debug records.  Both phases are gated behind
`#[cfg(feature = "debug_symbols")]` so release builds are unaffected.

---

## See also

- [INTERMEDIATE.md](INTERMEDIATE.md) — `State` layout, `fn_call`/`fn_return`, `Str` vs `String`
- [INTERNALS.md](INTERNALS.md) — native function registry, `library` call convention
- [LOGGER.md](LOGGER.md) — runtime logging (complement to stack traces)
- [THREADING.md](THREADING.md) — parallel execution model (ST-5 context)
