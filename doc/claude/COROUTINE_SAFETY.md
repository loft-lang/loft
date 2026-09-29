# Coroutines — safety concerns and mitigations

The SC-CO-n safety register for coroutine frames.  Part of [COROUTINE.md](COROUTINE.md), which holds the syntax, execution model and runtime design.

## Safety Concerns and Mitigations

### SC-CO-1 — Text ownership in the saved stack

**Problem:** `Str { ptr, len }` slots in `stack_bytes` may point to a dynamic
`String` that was freed when the stack was rewound after the yield. On resume,
those pointers dangle.

**Mitigation:** `serialise_text_slots` converts every dynamic text slot to an
owned `String` in `frame.text_owned`, and writes a `Str` pointing to the new
buffer into `stack_bytes`. On resume, the owned `String` addresses are patched
back into `stack_bytes` before the bytes are written to the live stack, keeping
all `Str` pointers valid.

---

### SC-CO-2 — DbRef locals may dangle if stores are freed mid-suspension

**Problem:** A generator may hold a `DbRef` local that refers to a heap record.
If the consumer frees or reallocates that record between iterations, the
suspended frame holds stale coordinates.

**Mitigation:** This is no different from any other loft local holding a
`DbRef`. The generator does not add a new risk. Document as a Known Limitation
(CL-2); the caller must not free records that a suspended generator still
references.

**P2-R5 — Text-specific variant: store-backed `Str` at yield**

When a generator reads a text field from a store record, the resulting `Str`
value is a zero-copy pointer directly into the store's raw allocation
(`{ ptr: store.ptr + rec*8 + 8, len }`).  If this `Str` is live in a local at
a `yield` point, `stack_bytes` encodes the raw pointer.

If the consumer:
1. Deletes the record (`database.free(r)` / `store.delete(rec)`), OR
2. Frees the entire store (`database.free(db_ref)`)

…between the yield and the next resume, and the store word is subsequently
reused for different data, the `Str.ptr` in `stack_bytes` points to unrelated
bytes — **silent data corruption** on resume.

**Invariant (CL-2b):** Any `Str` value derived from a store record field (via
`store.get_str()` or equivalent) must be treated as a borrow of the store's
memory.  If such a `Str` is live at a `yield` point, the caller must not delete
the backing record or free the store before the generator is exhausted or the
local is overwritten.

This is more dangerous than the `DbRef` case (SC-CO-2) because a `Str` looks
like a plain value, not an obvious reference.

**Long-term fix:** CO1.3d's `serialise_text_slots` (P2-R3) will deep-copy
store-derived `Str` values into owned `String` objects at yield time, eliminating
this class entirely.

---

### SC-CO-3 — Re-entrant advance corrupts the live stack

**Problem:** Advancing a `Running` generator overwrites live stack bytes with
saved bytes, corrupting the currently executing frame.

**Mitigation:** `OpCoroutineNext` checks `active_coroutines.contains(&idx)`
before resuming. If the index is already active, execution aborts:

```
runtime error: coroutine advanced re-entrantly; this generator is already running
```

The `contains` check is O(depth) but depth is bounded by nested `yield from`
chains, which are shallow in practice.

---

### SC-CO-4 — `yield` inside a `par(...)` body crosses thread boundaries

**Problem:** Each parallel worker has its own `State` and `coroutines` table.
A `COROUTINE_STORE` DbRef produced inside a worker cannot be advanced by
the enclosing scope's `State`.

**Mitigation:** The compiler must reject `yield` and generator function calls
inside `par(...)` bodies. `iterator<T>` values must not be assigned to
cross-thread shared variables. Until the compiler check is implemented, this
is a hard documented restriction:

> Generator functions and `yield` may not be used inside `par(...)` bodies.
> The `iterator<T>` DbRef must not cross thread boundaries.

---

### SC-CO-5 — Stack serialisation cost is O(depth) per yield

**Problem:** A deeply recursive generator copies a large `stack_bytes` and
`call_frames` on every yield and resume.

**Mitigation:** Document the cost model. Use iterative generators for
performance-critical paths; recursive `yield from` is for correctness-first
code. No optimisation is planned for the initial implementation.

---

### SC-CO-6 — Advancing an exhausted generator must always return null

**Problem:** If an exhausted frame were freed and its slot reused, a subsequent
`next()` on the old DbRef would access the wrong frame.

**Mitigation:** every handle carries a **generation stamp** in `DbRef::pos`, taken
from the frame it was made for, and each entry point (`coroutine_next`,
`coroutine_exhausted`, `free_coroutine`) compares it before touching the slot. A
handle whose frame is gone reads as exhausted and frees nothing, so the slot can
be recycled the moment its generator finishes — which is what makes the
scope-exit free of a handle safe (loft#835). `OpCoroutineNext` on an `Exhausted`
frame pushes null immediately without entering the body.

---

### SC-CO-7 — Absolute `stack_base` stale when caller pushes locals after creation

**Problem:** If `stack_base` were fixed at creation time (position `P`), any
local the caller pushes after creation would live at `P + …`. Resuming the
generator would then write `stack_bytes` starting at `P`, overwriting those
locals.

```loft
gen = count_up(0);  // suppose base captured at P
x   = 42;           // x at P+12 (after DbRef)
for n in gen { say("{n} {x}"); }  // would overwrite x!
```

**Mitigation:** `OpCoroutineNext` always sets `frame.stack_base = self.stack_pos`
before writing any bytes. The frame is placed at the current top of the caller's
stack, above all live caller locals. This is safe because no slot in
`stack_bytes` contains an absolute stack address; `Str` pointers reference
`text_code` or `text_owned` buffers, and `DbRef` values reference the store
heap — neither is affected by relocating the frame base.

---

### SC-CO-8 — Dynamic String objects leaked during yield serialisation

**Problem:** `str.str().to_owned()` creates a new `String` but does not free
the original. The original dynamic allocation has no remaining owner and leaks.

**Mitigation:** `serialise_text_slots` calls `database.free_dynamic_str(ptr)`
on the original allocation immediately after `to_owned()`. The exact API
mirrors `OpFreeText`; the implementation must align with how `text.rs` manages
the scratch/side-table of dynamic `String` objects.

---

### SC-CO-9 — Scalar `coroutine_sp` cannot represent `yield from` nesting

**Problem:** `yield from` makes two coroutines simultaneously active — the
outer (waiting in the `OpYieldFrom` loop) and the inner (executing). A single
`usize` cannot represent both, so the re-entrant check would miss the outer
generator while it is mid-`yield-from`.

**Mitigation:** `active_coroutines: Vec<usize>` holds the indices of all
currently active frames. `OpCoroutineNext` checks membership before resuming.
`OpYield` and `OpCoroutineReturn` pop the last entry.

---

### SC-CO-10 — Yielded `text` value not serialised; its String may be freed

**Problem:** If serialisation covers only the locals region (below the yielded
value), a `Str` in the yielded value still points to a dynamic `String` that
`SC-CO-8`'s mitigation then frees. The consumer receives a dangling pointer.

**Mitigation:** `OpYield` serialises the **entire** region from `stack_base`
to `stack_pos` (locals + yielded value) in one pass. After serialisation, the
yielded value bytes contain `Str` pointers that point into `text_owned` buffers
(not freed originals). Only the locals portion is stored in `frame.stack_bytes`;
the value portion is slid to `stack_base` as the `next()` return value.

---

### SC-CO-11 — `e#remove` inside a generator for-loop must be a compile-time error

**Problem:** Generators do not back a store record; any remove opcode emitted
against a generator iterator would operate on garbage coordinates, potentially
corrupting an unrelated record.

**Mitigation:** The compiler must detect `e#remove` on a generator-typed
iterator (identified at the `for` loop's type-resolution step by the element
type's source — a `COROUTINE_STORE` DbRef) and report:

```
error: `e#remove` is not valid on a generator iterator;
       generators do not back a store — use a collection if removal is needed.
```

---

### SC-CO-12 — `text_owned` offset stored as `u16` silently truncates

**Problem:** A `u16` offset caps at 65535 bytes. A deeply recursive generator
can exceed this (e.g. 3000 nested calls × ~22 bytes/CallFrame ≈ 66 KB).
Truncation silently patches the wrong `Str` slot on resume.

**Mitigation:** Use `u32` for the offset field (`Vec<(u32, String)>`), giving
4 GB headroom — sufficient for any realistic frame size.

---

### Summary of safety concerns

| ID | Concern | Severity | Resolution |
|---|---|---|---|
| SC-CO-1 | Dynamic text slots dangle after stack rewind | High | `text_owned` deep-copy; pointer patch on resume |
| SC-CO-2 | DbRef locals dangle if caller frees records mid-suspension | Medium | Documented (CL-2); caller responsibility |
| SC-CO-3 | Re-entrant advance overwrites live stack | High | `active_coroutines.contains()` check; runtime error |
| SC-CO-4 | `yield` inside `par(...)` crosses thread boundaries | High | Compiler error; documented hard restriction |
| SC-CO-5 | Serialisation cost O(depth) per yield | Low | Documented cost model; no optimisation planned |
| SC-CO-6 | Advancing exhausted generator after slot reuse | Medium | Exhausted frames kept alive; null pushed without frame entry |
| SC-CO-7 | Fixed `stack_base` overwritten by caller locals pushed after creation | High | Set `stack_base = stack_pos` at every resume |
| SC-CO-8 | Original dynamic String leaked after `to_owned()` | High | `database.free_dynamic_str(ptr)` in `serialise_text_slots` |
| SC-CO-9 | Scalar active-coroutine tracker fails for `yield from` nesting | Medium | `active_coroutines: Vec<usize>` replaces scalar |
| SC-CO-10 | Yielded `text` value's `Str` not serialised; freed by SC-CO-8 mitigation | High | Serialise entire `[stack_base..stack_pos]` region in one pass |
| SC-CO-11 | `e#remove` against generator emits store-remove opcode; corrupts unrelated records | Medium | Compile-time error at `for` loop type resolution |
| SC-CO-12 | `text_owned` `u16` offset truncates for frames > 65535 bytes | Low | `u32` offset field |

---

**Implementation phases** (completed, 0.8.3) — the record is in [COROUTINE-history.md](COROUTINE-history.md).
