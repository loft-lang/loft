<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Placement — where a library runs

A loft library can run in this process, in a worker process on this machine, or
on another machine, and its consumers cannot tell which. One line of the
library's own manifest decides:

```toml
[library]
placement = "inproc" | "process" | "remote"
```

This doc is the **mechanism and the authoring rules**. The declaration and its
user-facing behaviour are in [PACKAGES_MANIFEST_TABLES.md § `placement`](PACKAGES_MANIFEST_TABLES.md); the plan
that built it, with the design questions and the corrections it made to itself,
is [plans/119-out-of-process-libraries](plans/119-out-of-process-libraries/README.md).

## The invariant everything serves

> **A call to a library is indistinguishable — in type, effect,
> ownership/lifetime, and error behaviour — from the same call in-process.
> Where it runs is deployment policy, not source.**

Its **re-assertion sites** — the places it must independently hold, and therefore
the places that can disagree — are: argument marshal, return marshal, ownership
transfer (who frees), null propagation, effect classification, capability
admission, and the leak check. Each of those facts has ONE home both placements
consult, so the off-diagonal cells cannot disagree.

The gate is that sentence as a test: one unchanged consumer, one unchanged
library, run under every placement, requiring identical stdout, stderr and exit
status (`tests/placement_parity.rs`, `tests/placement_remote.rs`).

## Writing a library that can be placed

Four rules. Each one is also better in-process, which is how you know they are
about the design and not about the boundary.

### 1. A public function must not BE a native

Placement works by **giving** a function a native symbol and replacing the stub
with the dispatcher — so a function that already has one is skipped. A library
whose surface is `pub fn f(…); #native` is marked nowhere, and every call runs in
the caller, where the library's state does not exist.

Make the native private and let the public name be a wrapper:

```loft
fn kernel_send(cid: integer, msg: text) -> boolean;
#native

pub fn send(cid: integer, msg: text) -> boolean { kernel_send(cid, msg) }
```

Consumers are unaffected — same name, same signature.

### 2. Answer a value, not a cursor

An API of "advance, then read a field, then read a field" costs one crossing per
read. One that answers a whole value costs one crossing, whatever it contains.
`engine_host` offers a cursor underneath (`next_event()` plus four getters — five
calls per event) and a value on top:

```loft
pub fn turn(max_events: integer) -> Turn   // { running, tick, events: vector<Event> }
```

One crossing per frame, whatever the event count.

### 3. Closures do not cross

A library whose entry point takes a function — `run(port, on_event, on_tick)` —
can only be driven in-process. Offer a form where the caller owns the loop as
well, and both work.

### 4. A returned VIEW cannot be placed

A heap return is delivered one of three ways, and only two leave the caller
owning what it gets:

| delivery | who frees | placeable |
|---|---|---|
| `Owned` — a fresh store the callee minted | the caller | yes |
| `RetBuf` — materialised into the caller's hidden buffer | the caller already owns it | yes |
| `View` — a borrow of an argument or of the callee's own state | nobody | **no** |

`fn head(v: vector<P>) -> P { v[0] }` is the third. In-process the caller gets a
borrow and frees nothing, which is right; placed, the answer must be copied into
the caller's address space, and a copy nobody frees is a leak per call. Such a
function is not placed and runs in-process, where the borrow means what it says —
a view across a process boundary is not a view anyway.

The verdict is `use_analysis::heap_return_delivery`, which is also what
`--show-ownership` renders. Return a fresh value instead to make it placeable.

**And mark what you only read `const`.** It is a compile-time promise that the
callee cannot write, so the crossing skips carrying the argument home — about a
tenth of a call taking a large vector.

## What crosses

Scalars (every integer width with its sign, `single`, boolean), text, and
**structs and vectors** to any depth — a `text` field, a struct inside a struct,
`vector<struct>`, `vector<text>`, `vector<vector<T>>`, an empty vector, a null
struct. A signature outside that is simply not marked, so it runs in-process
byte-identically; nothing becomes a call that fails later.

Deliberately outside: a polymorphic **enum** and a **keyed collection**
(`hash`/`index`/`sorted`). Both are reference-shaped and so look placeable, but an
enum's payload type is a runtime discriminant and a keyed collection carries an
index whose ordering is the caller's.

**A compound argument is passed BY REFERENCE and stays that way.** `pub fn
bump(p: Point)` that assigns `p.x` changes the CALLER's `p`; a vector parameter
appended to grows the caller's vector. So a compound argument is copied in AND
copied back. Two arguments that are the same value (`f(p, p)`) marshal to ONE
record, because that is what the callee sees in-process.

## The arena

A struct or a vector is a graph of records, not a value that fits in a frame, so
it crosses as itself: an ordinary loft `Store` backed by an mmapped file, with
`copy_claims` — the deep copy an assignment already uses — marshalling into it.
No second wire vocabulary.

It works because of one fact: **nothing inside a store is a `DbRef`**. A text's
bytes, a vector's element block, a child record are all plain `u32` record ids
(`Stores::relocate_ptr_fields` is the exhaustive list). A record graph is
therefore independent of the address it is mapped at **and** of the store number
it is registered under, so each side names the arena with its own index and no
pointer is ever translated.

Three properties worth knowing:

- **Two arenas, one writer each** (arguments and returns). A `Store` caches its
  free list on the Rust side as well as in the mapping, so two processes claiming
  out of one would hand out the same block twice. The exception is forced by
  by-reference arguments — the callee may allocate in the argument arena — so the
  worker re-derives that cache from the mapping (`Store::resync_allocator`).
- **Reset per call**, not freed record by record. A loft borrow cannot outlive
  the call, so nothing the callee kept may still point there.
- **Bound into `Stores` only while a call is crossing**. A store in
  `allocations` at exit IS a leak by definition, and the arena is not the
  program's data; borrowing the slot keeps the gate's leak half exactly as strict
  under `process` as under `inproc` rather than needing an exemption.

## The two transports

Only the transport differs between `process` and `remote`. The marshal, the
layout gate, the delivery three-way, the `const` skip, the copy-back and the fault
handling are shared verbatim — they are properties of the BOUNDARY, not of the
wire. In the code that is one `enum Link` with two arms, and the dispatcher does
not know which it has.

| part | `process` | `remote` |
|---|---|---|
| the frame | shared mapping + futex | one message on a socket |
| the arenas | files both sides map | the same files' live BYTES, sent |

**The handshake spins before it sleeps.** A plain "wake the worker" futex costs
~4 µs — at that price placement is not policy, because moving a library would
change the shape of the code calling it. Each side spins briefly and publishes a
**sleeper count**, so the other pays the `FUTEX_WAKE` syscall only when someone is
genuinely asleep. The naive version is a performance bug, not a simpler variant.

**A remote arena travels as an image, not an encoding** — a store is
self-contained, so a `vector<Order>` goes on the socket as its own bytes. Only
the LIVE prefix travels; `Store::adopt_image` makes the space past it one free
block, because a prefix is not a well-formed store and a zero word in the
receiver's tail reads as a zero-size block.

## The layout gate

The two sides are different programs, and a record in the arena is read as the
RECEIVING program's own type. At install, one round trip per placed function
compares @PLN97's `layout_algo_hash` — which covers everything the type
references, and the host's endianness — computed on each side. A function they
disagree about is **not placed**, and says so.

## Ownership and faults

- A compound argument is the CALLER's throughout; the callee has a borrow, which
  in loft cannot outlive the call. That is why the arena can be reset per call and
  why the callee's writes are copied home.
- A compound answer lands in whatever the caller offered: a materialised
  destination it will free, or — where it offered an empty placeholder — a store
  minted exactly as `OpDatabase` would, which the caller's `OpFreeRef` frees.
- **A failed crossing maps neither arena.** A worker that died mid-write left them
  in whatever state it reached and may have resized one out from under this
  side's mapping, so the error path reads nothing from them — and the dispatcher
  walks every compound argument with loft's own guard-before-dereference check
  (`verify_graph_ok`) before reporting. A fault inside the library is the caller's
  runtime error, which is what makes its error behaviour match an in-process call.
- **A fault is relayed whole, not as prose.** What a fault renders from is its
  detail line, its kind label, its POSITION and the frames it fired under, and a
  `String` error channel carries only the first of those — so a placed fault used
  to print `panic: runtime error: panic: refusing -1` with the library's file,
  line, caret and call chain all gone, and the message doubled. The wire's error
  frame carries all four parts, and `RuntimeError::relayed` rebuilds the fault on
  the caller's side without re-describing it.
- **The call chain spans both processes, so it is joined in one place.** `main()`
  is in the caller and the library's own frames are in the worker; neither side
  can see the whole chain. The library's half crosses with the fault, and
  `State::note_runtime_error_halt` — the single point that already backfills
  frames for every `Stores`-side raise — appends the caller's half to it.

## Cost

Measured end to end against the identical loop with the call removed:

| shape | per call |
|---|---|
| in-process, `--interpret` | ~0.05 µs |
| placed scalar | 0.80 µs |
| placed 2-field struct | 1.35 µs |
| placed 16-element vector | 1.55 µs |
| **remote**, any of the above | ~27 µs (loopback) |

The local numbers barely move with the value's size, and the remote one is
dominated by the round trip rather than the data — both because the crossing is a
copy of the value's own layout and not a per-field encode. So placement is for a
library whose calls do real work, and a poor fit for a getter in a loop.

## Limits

- **Calls to one placed library serialise** — the wire has a single request slot
  and the dispatcher holds a lock across the crossing. Correct, not cautious: two
  threads interleaving two frames in one buffer is a wrong answer, not a slow one.
  A placed library is therefore a poor fit for a hot `par` arm (it works, and is
  gated, under both parent-sharing modes).
- **Linux only** on the calling side (the local handshake is `futex`); elsewhere
  a placed library runs in-process, which by the invariant is the same program.
- **Not under `--native`**, which compiles the library's body into the program
  binary, so its calls never leave the process.
- `LOFT_REQUIRE_PLACEMENT=1` turns either of the last two from a silent fallback
  into an error naming which applied.

## Adding a native to the loft binary

A library whose natives live in the binary (`[native] in_binary = true` —
`lib/git`, `lib/engine_host`) needs **two** registrations, and they are keyed
differently:

| backend | table | keyed by |
|---|---|---|
| interpreter | `src/native.rs` | the native SYMBOL |
| `--native` | `CODEGEN_RUNTIME_FNS` + a typed twin | the loft DEF NAME |

Miss the second and the library is interpreter-only — silently, until something
compiles it. A `&text` out-parameter arrives at the typed twin as `&mut String`.

## Instruments and gates

- `LOFT_TRACE_ARENA=1` — what each side put in the arena and read back out, with
  the process id per line.
- `LOFT_STRICT_STORES=1` — stops the allocator recycling a released slot, which
  turns "how many stores did this run ever need" into a number the two placements
  must agree on. `check_store_leaks` alone cannot see it: it reports what is
  unfreed AT EXIT, and so is blind to a program that allocates one store per call
  and frees it — exactly a placed call's shape, and exactly what runs a store
  table out of slots.
- Gates: `tests/placement_parity.rs` · `tests/placement_remote.rs` ·
  `tests/placement_worker.rs` · `tests/lib_git.rs` ·
  `tests/engine_host_placed.rs`.

## See also

- [PACKAGES_MANIFEST_TABLES.md § `placement`](PACKAGES_MANIFEST_TABLES.md) — declaring it, and what a consumer sees.
- [SANDBOX.md](SANDBOX.md) — the isolation tier this adds: isolated **and**
  direct-data, because the boundary is a page table rather than a marshalling step.
- [DATABASE.md](DATABASE.md) — stores, `DbRef`, the working-set loader.
- [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md) — the deps north-star the crossing holds.
- [formal/layout.md](formal/layout.md) — the layout contract used as the wire.
- [plans/119-out-of-process-libraries](plans/119-out-of-process-libraries/README.md)
  — the plan, its answered design questions, and the corrections it made to itself.
