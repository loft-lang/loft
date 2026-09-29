# Interfaces — running at scope end (`OpDrop`)

How a library type runs a user-written cleanup at scope end.  Part of [INTERFACES.md](INTERFACES.md), which holds the syntax, semantics and standard interfaces.

## Running at scope end — `OpDrop`

A type that defines `OpDrop` runs it when a scope that OWNS one lets it die —
the shape RAII gives a transaction, a file handle or a C resource:

```loft
struct Tx { tag: text, done: boolean }

fn commit(self: Tx) -> boolean { …; self.done = true; return true; }

fn OpDrop(self: Tx) {
  if !self.done { rollback(self.tag); }
}

fn work() {
  t = begin("orders");
  …                       // no commit on this path
}                         // <- rollback runs here
```

**The rule:**

> A drop runs when the value's OWNER dies. Taking a value back out of its owner
> does not run one.

For a value held in a plain binding, the owner is that binding, and the drop runs
exactly where the binding's own `OpFree*` runs — the same scope exit, the same
early-exit paths. loft already computes that. The ownership model decides, per
binding, whether this scope owns a value and whether it dies here, which is what
emits `OpFreeRef`; a returned or borrowed value is already excluded, and the
early `return` / `break` / return-out-of-a-loop cases are already handled there
(loft#731 exists because a hand-rolled version of exactly those went wrong). So
the hook DERIVES from the borrow model rather than sitting beside it, and there
is one answer to "when does this run", not two that can drift.

### Putting one in a container

Copying a droppable into a struct field, an enum payload or a collection element
is a **move**: the container becomes the owner. The value you copied FROM no
longer drops, and the container's death releases what it holds.

```loft
struct Handle { id: integer }
fn OpDrop(self: Handle) { close(self.id); }

struct Session { h: Handle }

fn open_session() -> Session {
  h = acquire();
  return Session { h: h };   // `h` no longer drops — the Session owns it now
}

fn work() {
  s = open_session();
  …
}                            // <- close() runs here, once
```

What moves is the value and the RESPONSIBILITY to release it: `h` is spent after
that line, and reading it is `error[read-after-move]` — read it as `s.h` instead. Without that move
the resource was released twice over: once by the source at its own scope end,
and never by the container. That is invisible while both die in the same scope,
and it is a use-after-free the moment the container outlives the source, which is
what `open_session` above does.

**A reassignment releases what it displaces.** `s = S { h: acquire() }; s = S { h: acquire() }`
runs the hook on the first record's handle at the second assignment — after the new value
has been computed, so `s = grow(s)` still finds the old resource live while `grow` runs —
and so does a rebind from a call, to `null`, or inside a loop of a local declared outside
it. The hook used to run at scope end only, and the first handle was never closed
(loft#1362).

**Whether a line copies or moves is read off that line.** A value the function OWNS
— a local it bound to a fresh value — MOVES when it is placed: bound (`h2 = h`),
written into a field, appended, or returned. The new structure releases it, and the
old name is spent: reading it afterwards is `error[read-after-move]`, which names the
line the value moved on. Anything else placed into a new structure is a COPY — a
parameter (the caller still owns it), a member of a container (`s.h`, `v[i]`), a
captured variable — and a copy of a droppable is `error[copy-of-droppable]` unless
the type says how a copy gets its own lease, below. Passing a value as an argument,
a `&` link and a view (`x = s.h`) make no second structure and are always legal.
The rules are `formal/heap.md` `(H-Move)`, `(H-Spent)` and `(H-Copy-Refuse)`.

### Copying one — `OpCopy`

A type that CAN have two live copies — a reference-counted buffer, a read-only handle
that can be reopened — declares `fn OpCopy(self: T)`. A copy then copies the bytes
and runs `OpCopy` on the NEW structure, which takes its own lease there; each copy is
released once, at its own death.

```loft
struct Buf { id: integer }
fn OpDrop(self: Buf) { release(self.id); }
fn OpCopy(self: Buf) { retain(self.id); }   // the copy holds a reference of its own

fn keep(b: Buf) {
  mine = b;          // a copy: `OpCopy` runs on `mine`
}                    // <- `release` runs for `mine`; the caller's `b` releases later
```

Like `OpDrop`, it takes only `self` and answers nothing. A struct whose members
declare `OpCopy` gets a synthesized cascade: its own `OpCopy` first, then its
members'. A copy is refused if ANY droppable inside the type lacks `OpCopy`. A move,
a vector's growth, a view and an argument run no hook — and neither does a copy the
compiler skips altogether together with its release, which it may: never rely on the
hook running for a particular copy, only on each copy that exists holding a lease.

A container releases in this order:

1. its own `OpDrop`, if it has one — a wrapper may still need what it wraps, the
   way a connection says goodbye over the socket it is about to close;
2. then its fields, in reverse declaration order, each through its own type;
3. a collection field element by element, in element order.

Nesting needs no special case: each type releases its own members, so a struct
inside a struct inside a vector releases once, at the outermost owner's death.

What follows from all of this, and is worth knowing before you reach for it:

- **A drop cannot fail.** loft has no runtime errors (C80) and a rollback at
  scope end can fail for real — the connection dropped, the server went away —
  with no caller left to tell. So `OpDrop` may not return, and the compiler
  refuses one that tries. **Anything whose failure matters stays an explicit
  call**: `tx.commit()` answers, the closing brace does not. That asymmetry is
  the design.
- **A drop reaches the world, not its caller's data.** It receives only `self`,
  and construction COPIES the data — `Tx { journal: j }` holds a copy of `j` — so
  a drop cannot write back into a caller's loft-side collection. Its effect is
  I/O, or a resource it owns (a `#c` handle). That is exactly the intended use:
  libpq's `PQexec("ROLLBACK")` at a closing brace.
- **Order within a scope is reverse-declaration**, matching the existing free
  order, or a statement would outlive the transaction it belongs to.
- **A binding written inside an `if` block is hoisted to the function scope** —
  that is where loft frees it, so that is where it drops. A `for` body is a scope
  of its own, so a droppable made per iteration drops per iteration.
- **A value that was never created never drops.** The free is null-tolerant and
  a drop is not, so the call is guarded by the same liveness test the free
  performs internally.
- **In a library, the hook may be private** — and usually should be. Nothing
  calls `OpDrop` by name, so `pub` only widens the surface. The hook is looked up
  through the source that declares the TYPE, which is what makes a private one
  reachable: a library's symbols are module-scoped (@PLN102 C97), and both askers
  run after parsing, when the current source is the main program.

**Three things a drop does NOT do.** Each is a deliberate boundary, not an
omission:

- **Taking a value OUT of its owner does not release it.** `v.remove(i)`,
  `v[i] = other` and an overwritten FIELD `o.s = other` do not release the value that
  goes away: it leaks, and the program is otherwise correct. (A reassigned LOCAL is
  different — its record has one owner, the local, and the release runs there.) Releasing there would mean the runtime's free
  cascade calling back into your loft code, inside the one operation the heap
  invariant rests on, for a hook that by contract can neither fail nor answer. If
  you churn a collection of live resources, release the old element yourself
  before you replace it. The reasoning is
  [DESIGN_DECISIONS.md § C111](DESIGN_DECISIONS.md).
- **A keyed collection does not release its records.** A `hash` / `sorted` /
  `index` shares its records with the collection it is indexed from, so releasing
  through one would release somebody else's element. Keep droppables in a plain
  `vector`, or release them explicitly.
- **A value moves once.** `a = C { h: h }; b = C { h: h }` moves `h` into `a`, and
  the second line reads a spent name: `error[read-after-move]`. Build the second
  container from its own value, or give it a copy the type can lease (`OpCopy`).

**When to reach for it.** A drop pays for itself when the value owns something
the program cannot see — a `#c` handle, a lock, a file — and the release is
unconditional. It does not pay for loft-side data, which the ownership model
already frees. And it is a poor fit for anything whose release ORDER matters
against an explicit call: a scope end runs after the function body, so a cursor
whose connection is shut inside that body is still live when the shut happens.
@PLN138's `lazy_fetch` closes its cursor explicitly for exactly that reason.

Shipped as @PLN125 arc B, with the owner rule and the container cascade added by
@PLN139 (loft#849). `tests/scripts/pln125-b-drop.loft` is the hook's behaviour
matrix, with two unrelated consumers (a transaction and a lease) because a hook
with one user is a hook whose invariant is untested;
`tests/scripts/139-drop-cascade.loft` is the container half. The design reasoning
is [plans/23-db-clients/LIFETIME_AND_PROCEDURES.md](plans/23-db-clients/LIFETIME_AND_PROCEDURES.md)
and [plans/139-drop-cascade.md](plans/139-drop-cascade.md).
