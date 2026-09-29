// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy: `LOFT_LOG` presets and traces

LOFT_LOG presets, the struct/database dumps in the trace, and IR / execution-trace inspection of one function.  Part of the debugging guide: [DEBUG.md](DEBUG.md).

---

## Preset Guide

| Preset | What it shows | When to use |
|--------|---------------|-------------|
| `minimal` | Bytecode execution trace (opcode + stack state per step) | Stack corruption, wrong opcode, wrong result |
| `ref_debug` | Reference allocation and free events | Double-free, use-after-free, wrong store_nr |
| `full` | IR tree + bytecode + execution | Everything at once; output is very large |
| `static` | IR tree and bytecode only (no execution) | Codegen bugs, wrong IR, wrong opcode selection |
| `crash_tail:N` | Last N lines before panic | Crash triage when full output is too large |
| `locks` | Every store-lock / store-unlock event with store_nr + rec | "Write to locked store at rec=N fld=M" panics — pinpoints which op acquired the lock.  `LOFT_LOCK_BT=1` (an env switch, not a preset) adds, at a REFUSED write, the store number and the Rust backtrace of the writer — a runtime path (a bridge marshal, a loop buffer's reset) looks like the program's own statement from the loft call chain alone |
| `type_timeline:<varname>` | Every type-mutation event for a specific named variable (old → new + origin + the SOURCE LINE that wrote it; set `LOFT_TIMELINE_BT=1` for the stack behind it) | "Why is var X type T at this point?" — flip / change_var_type / depend / substitute_type traces.  A dep list is REPLACED, not merged (`Type::depending`), so "who wrote this dep last" is usually the whole question.  ⚠ Matches by NAME across every parsed function including the stdlib — check `v_nr=` before believing an origin (below) |
| `ir:<fn_name>` | IR tree dump for the named function only (no bytecode, no execution trace) | "What IR did the parser emit for fn X?" — focused codegen-bug diagnosis |
| `slots:<fn_name>` | Slot-allocation summary for the named function — each var's final slot OR a reason why it was skipped | "Why is var X at slot 65535?" — `Incorrect var X[65535]` codegen panics |
| `captures:<fn_name>` | Capture-pipeline summary for the named function + its lambdas — scalars_to_box, mutated_captures, closure_record attrs with auto-Reference status | "Why is closure-record attr X stored inline vs share-by-DbRef?" — closure-encoding diagnosis |

⚠ **`type_timeline` matches by NAME, across every function the run parses — the whole
stdlib included — so a common name makes it answer about a DIFFERENT variable.**  It filters
on the name alone (`type_timeline_target()`), and one run reaching for a receiver named `v`
was told its dep was lost by `make_independent` at `objects.rs:573`, the `(B-Copy)` arm, with
a plausible before/after type beside it; a print inside that arm showed it never fires for
that variable at all.  The trace was not wrong about what it saw — it saw another `v`.

Every line carries `v_nr=N`, so **read the `v_nr` and confirm the function before believing
an origin**, and when the name is common prefer a print at the suspected site over the trace.
The general form is worth carrying past this instrument: *a tool that cannot say which
function it is talking about will confidently answer about a different one* — the same shape
as a guard whose fixture is below the quantum, and the reason two plausible-looking readings
can cancel into what reads as a deliberate design.

`LOFT_VAR_TABLE=<fn substring>` is the companion to the IR dump, and NOT a `LOFT_LOG`
preset — it prints after `scopes::check` on every path:

```
[vartable] n_f — returned Enum(650, true, Deps { items: [1] })
[vartable]   0   n            int         scope=0  arg def OWNS
[vartable]   1   __retbuf     enum(650)   scope=4  def
[vartable]   3   e            enum(650)   scope=0  arg def OWNS
[vartable]   5   _mv_items_1  vec<int>    scope=4  def deps=[__retbuf(1)]
```

Reach for it whenever a borrow points somewhere that makes no sense.  The IR dump
names variables but never NUMBERS them, so a body reading `e` and a type dep printing
`__retbuf` read as one consistent story — which is what made loft#666 look like a
rename for two sessions.  Here the index is beside the name and each dep is resolved
to `name(index)`, so a code/table desync is visible instead of inferred.  The flags
answer the ownership question in the same line: `arg`, `def`, `skipfree`, `inlineref`,
and `OWNS` (the `Function::owns_store` verdict — the ONE predicate every consumer
reads; an element or a match binding must NOT show it).

`LOFT_TRACE_WORKREF=1` is the var table's other half — the ORDER the `__ref_N` names were
claimed in, one line per mint, naming the site that asked:

```
[workref] fn=bad -> v2 __ref_1 arg=no  tp=Vector(Reference(707, …)) at src/parser/mod.rs:8977:40
[workref] fn=bad -> v5 __ref_3 arg=yes tp=Reference(707, …)         at src/parser/objects.rs:3051:31
```

Reach for it when the var table shows the right names in the wrong ROLES.  The table is the
end state; a collision is about sequence, and the two parser passes mint different ones —
which is what showed a call's out-param buffer claiming, on pass 2, the name pass 1 had
promoted to the return-buffer argument (loft#872).  The table alone said only that one
variable was both.

**`arg=` is the field that separates a collision from the intended reuse, and read it before
reading anything else.**  A `__ref_N` name IS function scratch, so pass 2 re-resolving it to
the same scratch slot is exactly right and happens constantly — sweeping all 844
`tests/scripts` for *one variable minted from two different sites in one function* reports
**138 hits across 24 files**, nearly all of them that.  `arg=yes` on a `__ref_N` can only mean
`ref_return` promoted it to the return buffer on pass 1, so a DIFFERENT site is now being
handed the buffer: the same 844-script sweep filtered on it reports **0**, and **7** with
loft#1078's `LOFT_NO_P2_OBJECT_WORKREF=1` opt-out set.  Six of those seven were scripts
already in the suite and already passing, which is how the class stayed invisible.  The field
was added because the trace could not answer the question its own loft#872 example poses.

Its companion is `LOFT_TRACE_RR=1`, which prints `ref_return`'s
`(ls, ls_types, returned)` plus the per-candidate promotion verdict, with the PASS.

One flag is there for a different reason.  **`amplink`** marks a binding the author
spelled with `&` at a struct-typed projection (`c = &v[0]`, `c = &o.inner`).  Such a
projection is already a view, so both spellings emit *byte-identical* IR — this column
is the only place the `&` is still visible after parsing, and the only way to check
that a decision keyed on it is looking at the binding you think it is (@PLN130 F9,
loft#779).

---

## Database / Struct Debug Dumps in the Trace

Every opcode that produces or consumes a `DbRef` (struct, enum, or vector) shows a
compact inline dump of the pointed-to record in the execution trace.  The format is:

```
   8:[44] VarRef(var[32]=l) -> #3.1 { name: "diagonal", start: #2.1 { x: 1.5, y: 2.5 }, end_p: #1.1 { x: 10, y: 20 } }[44]
  65:[68] GetField(v1=ref(3,1,8)[56], fld=0) -> #3.1 { }[56]
```

**Reference prefix** `#store.record` — e.g. `#3.1` means store 3, record 1.
This tells you which allocation each struct lives in, making it easy to track
aliasing and double-free issues across opcodes.

**Depth limit** — nested structs expand up to depth 2 by default.  Deeper records
are shown as `{...}`:

```
#3.1 { inner: #5.7 { val: 42, nested: #6.2 {...} } }
```

**Element limit** — vectors show up to 8 elements by default, then `...N more`:

```
#4.3 [ #2.1 { x: 0 }, #2.2 { x: 1 }, ...6 more ]
```

**Depth limit at a vector** — if the depth limit is reached at a vector, shows
the element count instead of expanding: `#4.3 [10 items...]`

**Null fields are hidden** — fields holding the null sentinel are omitted, so a
freshly allocated struct with only one field set shows only that field.  This keeps
traces compact even for large structs.

### Tuning the dump limits

```bash
LOFT_DUMP_DEPTH=3    # expand up to 3 levels of nesting (default 2)
LOFT_DUMP_ELEMENTS=4 # show at most 4 vector elements (default 8)
```

These are read from the environment at runtime; no recompile needed.

### Accessing dumps directly via `cargo run`

When `LOFT_LOG` is set, `cargo run --bin loft` routes execution through
`execute_log` and writes the full trace (including struct dumps) to stderr:

```bash
LOFT_LOG=full  cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_LOG=minimal cargo run --bin loft -- myprog.loft 2>trace.txt
LOFT_DUMP_DEPTH=3 LOFT_LOG=full cargo run --bin loft -- myprog.loft 2>trace.txt
```

Without `LOFT_LOG`, the program runs without any trace output (production mode).

### Implementation

| File | Role |
|------|------|
| `src/database/mod.rs` | `DumpDb` struct — stores, depth/element limits, compact flag |
| `src/database/format.rs` | `Stores::dump_compact()`, `DumpDb::write()`, `write_struct()`, `write_list()` |
| `src/state/debug.rs` | `dump_limits()`, `dump_result()`, `dump_stack()` — calls `dump_compact()` for inline trace |
| `src/main.rs` | Routes `LOFT_LOG`-enabled runs through `execute_log` instead of `execute_argv` |

---

## Inspecting a Specific Function

### IR inspection with `LOFT_IR`

Set `LOFT_IR` to a function name (substring match) to see the parsed IR tree:

```bash
LOFT_IR=distance loft myprog.loft
```

Output:
```
=== IR: n_distance ===
{#block(1):integer
  [7] OpAddInt(OpMulInt(OpGetInt(p(0), 0), OpGetInt(p(0), 0)),
               OpMulInt(OpGetInt(p(0), 4), OpGetInt(p(0), 4)));
}#block(1):integer
===
```

The IR shows how the parser translated the loft source into internal operations.
Each `Op*` is a bytecode operator; `p(0)` is a variable reference; field offsets
(0, 4) correspond to struct field positions in bytes.

Use `LOFT_IR=*` to dump all user functions.

### Execution trace with `LOFT_LOG`

Set `LOFT_LOG` to trace bytecode execution step by step:

```bash
LOFT_LOG=full loft myprog.loft 2>trace.txt
```

Output (excerpt):
```
Execute main:
    0:[8] ReserveFrame(size=4)
    5:[48] Database(var[36], db_tp=48)
   10:[48] VarRef(var[36]=p) -> #1.1 { }[48]
   13:[60] ConstInt(val=3) -> 3[60]
   18:[64] SetInt(v1=ref(1,1,8)[48], fld=0, val=3[60])
   32:[48] VarRef(var[36]=p) -> #1.1 { x: 3, y: 4 }[48]
   35:[60] Call(d_nr=499, args_size=12, fn=n_distance)
 3487:[64] VarRef(var[48]=p) -> #1.1 { x: 3, y: 4 }[64]
 3490:[76] GetInt(v1=ref(1,1,8)[64], fld=0) -> 3[64]
 3499:[72] MulInt(v1=3[64], v2=3[68]) -> 9[64]
 3513:[72] AddInt(v1=9[64], v2=16[68]) -> 25[64]
 3514:[68] Return(ret=3566[60], value=4, discard=20) -> 25[48]
```

**Reading the trace:**
- `[48]` is the stack position in bytes
- `#1.1 { x: 3, y: 4 }` is an inline struct dump (store 1, record 1)
- `-> 25[64]` shows the result value and where it was pushed on the stack
- `Call(..., fn=n_distance)` shows function entry with the internal name
- `Return(...)` shows the function exit with the returned value

### Filtering by function name

Use `LOFT_LOG=fn:distance` to only trace execution inside `distance`:

```bash
LOFT_LOG=fn:distance loft myprog.loft 2>trace.txt
```

### Combining IR and trace

Both can be used together to see the IR at compile time and the execution at runtime:

```bash
LOFT_IR=distance LOFT_LOG=full loft myprog.loft 2>trace.txt
```

### Quick reference

| Variable | Value | What it shows |
|----------|-------|---------------|
| `LOFT_IR` | `distance` | IR tree for functions matching "distance" |
| `LOFT_IR` | `*` | IR tree for all user functions |
| `LOFT_LOG` | `full` | IR + bytecode + execution trace for all functions |
| `LOFT_LOG` | `minimal` | Execution trace only |
| `LOFT_LOG` | `static` | IR + bytecode only (no execution) |
| `LOFT_LOG` | `fn:distance` | Execution trace for `distance` only |
| `LOFT_LOG` | `crash_tail:50` | Last 50 execution steps before a crash |
| `LOFT_DUMP_DEPTH` | `3` | Struct nesting depth in dumps (default 2) |
| `LOFT_DUMP_ELEMENTS` | `4` | Max vector elements in dumps (default 8) |
| `LOFT_TRACE_ASSERTS` | `/tmp/ran.txt` | Appends `file:line` for every `assert` that EXECUTES — both backends, every process.  Diff against the `assert(` sites in the source to find the ones a suite contains and never runs, and read a whole file tracing at a constant offset as a wrong injected LINE ([GUARDS.md](GUARDS.md#the-set-a-suite-runs-is-not-the-set-it-contains-loft_trace_asserts)) |
