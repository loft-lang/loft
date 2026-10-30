<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 S3 — history

The history companion of [1b-s3.md](1b-s3.md): designs that were built and later retired, kept
as they stood.

## RETIRED 2026-10-30 — ownership at run time

**Retired by the owner (2026-10-30):** *"your goal is to make loft again inside loft, not to
design a new language with new mechanics, the IR produced should be the same, the operators
should do the same with the same semantics."*  The owner table, the in-flight adoption and the
return sweep below were a mechanism the Rust interpreter does not have; a bind and a free now
lower as the Rust bytecode writer lowers them (1b-s3.md § Binds and frees).  The section as it
stood, with its one amendment (the hidden return buffer, 2026-10-30):

### Ownership at run time — DECIDED (owner, 2026-10-28)

**Ruling:** a record bound from a call is decided at run time by (O-Move), *integrated with the
lifetimes so there is no room for disagreement*.  So the bind and every free read ONE fact.

**The invariant — `(O-Owner)` held at run time:** the interpreter keeps an owner table, one entry
per store of the C runtime: the variable slot that owns it, or the frame holding it in flight (a
returned value not bound yet), or nobody.  Every operator that creates, binds, frees or returns
a record reads and updates that entry, and no store is released except through it.

| event | what the table does |
|---|---|
| `OpDatabase(v)` | `v` owns its store: made again in place.  `v` is the function's hidden return buffer (`hidden_return_buffer_attr`): the caller's buffer, made again in place whoever owns it, because filling it is the call's protocol and the generators write into it; a fresh store the frame owns when the caller passed none.  Otherwise a fresh store, owned by `v` (never a clear of a store `v` only views) |
| `v = x`, `x`'s store held in flight by this frame, or by nobody | ADOPT: `v` owns it |
| `v = x` copying (a record variable, a projection `element_copy` copies, a loft call whose result lives in a store someone owns) | COPY into `v`'s own store (refilled in place when `v` owns one) |
| `v = x` otherwise (a view) | `v` names the store, owns nothing |
| any bind | a store `v` owned and no longer names is freed (`displacing_rebind`) |
| `OpFreeRef(v)`, `OpFreeRefIfDistinct(v, w)` | freed only when THIS FRAME owns it — through any of its variables, since a vector's field variable names its wrapper's store — never a store a caller owns; `v` is reset as the runtime resets it |
| `OpCopyRecord` that frees its source | the source's entry goes with it |
| `Ret` | the returned store, if this frame owned it, is held in flight by the caller; every other store this frame owns or holds is freed (the SWEEP) |

**What this settles.**  A call bind needs no analysis of the callee: a record minted by the
call arrives in flight and is adopted; one that lives in a store somebody owns (an argument, a
field of one) is copied, which is (O-Move)'s borrow clause.  The IR's frees cannot disagree
with that choice, because a free acts only on what the table says the variable owns; and a free
the IR does not place for a store the interpreter owns is the sweep's.  Rebinds and parameters
need no refusal for the same reason.

```loft
c = keep(src);    // the result is src's store, owned by src: copied, c owns the copy
f = file("x");    // file() minted it in its own frame; returned in flight: f adopts it
p = make(1, 2);  p = make(3, 4);   // the second bind frees the first store p owned
```

**The leak check stays a falsifier:** `stores_left` counts the stores live after the run, and
`swept` counts what the sweep released — the frees the IR did not place.  A swept store is no
fault (the table owned it), but the census reports it, as the measure of where the static
lifetimes and the run-time ones differ.

**What a later slice must add to the table:** an operator that stores a reference INTO a record
(`OpSetDbRef`, a vector push of a record) moves ownership from a variable to the container; it
is refused until it does.

