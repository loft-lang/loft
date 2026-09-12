
# Lifetime — Dependency Tracking and Scope-Based Freeing

How the `dep` field on `Type::Text`, `Type::Reference`, `Type::Vector`, and other
heap-owning types interacts with scope exit to decide what gets freed.

---

## Dep field and scope exit freeing

The `dep` field on `Type` controls ownership and freeing.  See `src/data.rs`
(Type enum doc) and `src/scopes.rs` (module doc) for the core semantics.

## Scope exit — `get_free_vars`

When a scope ends (block exit, function return, loop iteration boundary), the scope
analysis emits free operations for variables registered in that scope.

### Step 1: Collect variables — `variables(to_scope)` — `src/scopes.rs:503-533`

Walk the scope stack from the current scope back to `to_scope`.  Collect all variables
whose `var_scope` is in this range.  Variables are returned in **reverse insertion
order** (most-recently-created first) to satisfy the LIFO invariant on store freeing.

### Step 2: Skip the return variable

The variable being returned (`ret_var`, found by `returned_var(expr)`) is never freed —
it escapes the scope.

### Step 3: Emit free ops per variable

For each variable `v` in the collected set:

#### Tuples (`Type::Tuple`)
```
→ (T1.3/T1.4: per-element free — not yet fully implemented)
→ skip to next variable
```

#### Text (`Type::Text(_)`)
```rust
if matches!(function.tp(v), Type::Text(_)) {
    ls.push(call("OpFreeText", v, data));   // ALWAYS emitted
}
```

**Text is always freed** at scope exit, regardless of its dep list.  This is because
text occupies stack-frame memory (a `Str` struct = pointer + length).  The stack frame
is about to be reclaimed, so the text buffer must be released.

The dep list on Text is used for **type compatibility checking**, not for free decisions.

#### References, Vectors, Struct-Enums (`Type::Reference(_, dep)` etc.)
```rust
let emit = dep.is_empty()                    // (1) I am the owner
         && !tp.depend().contains(&v)         // (2) not escaping via return type
         && !function.is_skip_free(v);        // (3) not marked skip_free
if emit {
    ls.push(call("OpFreeRef", v, data));
}
```

Three conditions must ALL be true to emit `OpFreeRef`:

1. **`dep.is_empty()`** — the variable owns its store allocation.  If deps are
   non-empty, some other variable owns the underlying store record and will free it.

2. **`!tp.depend().contains(&v)`** — the return type's dep list does not mention this
   variable.  If it does, the value escapes via the return expression and must not be
   freed here.

3. **`!function.is_skip_free(v)`** — the variable is not marked `skip_free`.  This
   flag is set by `clean_work_refs` for work-ref temporaries that are re-purposed
   after use (A14), and by `set_skip_free` for borrowed references like par-loop
   result variables.

##### The two escape routes

Condition 2 covers the route the type system can see, and it is not the only one.
A callee hands a heap value to its caller two ways: a **`return`**, and a **write
through a `&` parameter** — a `&T` is a place in the caller's frame, so assigning
it publishes into storage the caller owns and goes on using.

The second route is invisible to a dep check, because the escaping store is not
named by the return type at all.  It is settled at runtime instead: `scan_set`
pairs the `__ref_N` work-ref buffer with the `&` parameter it was published
through (`paired_witness`), and the scope exit emits
`OpFreeRefIfDistinct(__ref_N, *f)` rather than a plain free.  A runtime store-nr
comparison is what makes it path-local — a publish inside an `if` frees the
buffer on the branch that did not publish, with no per-path set to thread.

Why runtime rather than static: the callee decides.  One that returns the buffer
it was handed (`fn mk(n) -> Box { b = Box{..}; return b }`, and `file()` itself)
makes the buffer the caller's record; one that builds fresh in the return
expression leaves the buffer orphaned and this frame's to free.  A local target
never faces the question — `gen_set_first_ref_call_copy` deep-copies, so the two
stores are always distinct — which is why the fault was specific to `&`
parameters (loft#759).

Both routes have a static checker: `use_analysis::return_source_freed` for the
return, `ref_param_publish_freed` for the `&` write.  Both surface through
`loft introspect --show-ownership`, and both flag the same shape — a delivered
store killed by a plain `OpFreeRef`.

###### What is published must be a store, not a view (loft#775)

The pairing above settles *which* store the write publishes.  It assumes the
right-hand side IS one.  A **field read** is not: `out = ld.wl_world` hands over a
pointer INTO the record `ld` holds, and `ld` is a local this frame frees on the way
out.  The caller went on reading it and the next allocation landed on top — a
four-chunk world became a one-chunk world with its edit clock, which is monotonic,
running BACKWARDS, across a call that never touched its argument.

The return route already had the answer: `materialize_view_return` copies a viewed
record into an owned buffer at the return, which is why `return ld.wl_world` was
safe the whole time.  `Parser::assign_refvar_reference` makes that same move for the
`&` write — `{ w = null; OpDatabase(w, kt); OpCopyRecord(view, w, kt); w }`, marked
`skip_free` because the caller owns the copy now, exactly as it owns a fresh record
built in place (`o = Obj{…}`, @PLN87 P2.2).

It keys on the IR **shape** (an `OpGetField` right-hand side), not on the
right-hand side's deps.  Deps accumulate while a body parses, so a deps test mints
the work-ref on pass 2 only and shifts every later `__ref_N` — the cross-pass
divergence the H5 two-pass contract exists to catch.

**Reading it takes all three lenses.**  `--interpret` reported the store as unfreed
and gave the right value; `LOFT_POISON=1` faulted; `--native` answered silently with
whatever had been allocated since.  Only the poison lens named it, and only once the
probe RECYCLED stores — a freed record reads correctly until something claims its
bytes, so a probe without a churn loop produces a vacuous pass.  That is why the
filed report could say "does not reproduce standalone" about a live corruption.

#### Function (`Type::Function`) — freed at scope exit (verified 2026-05-12)

`Type::Function` does not appear in the `get_free_vars` match by name, but the
closure DbRef embedded at offset+4 of the 16-byte fn-ref slot is freed at
scope exit by two mechanisms that cover every shipped capture pattern:

- **Local var (D1)**: closure record dies with the stack frame via the
  standard local-cleanup path; the captured payload (text / reference / etc.)
  is freed transitively.
- **Struct field (D3)**: @P213's `Parts::ChildRec` cascade walks the host
  struct's fields at scope exit and frees the closure record co-located in
  the host store.

**Verified clean** by `tests/leak.rs::p15_phase03_closure_text_capture_*_no_leak`
— two 100-iteration tight loops calling capturing closures (text capture in
both D1 and D3 destinations) with `state.check_store_leaks()` asserting zero
leaked stores after.

The legacy "Implementation path" section below describes a `get_free_vars`
`Type::Function` arm that was contemplated but never landed because the
`Parts::ChildRec` + standard-local-cleanup combination already covers the
surface.  The cross-scope closure work it discusses (Steps 1–4) DID land
through @P213/P215/P227 by 2026-05-05 — read it as historical context, not
as current open work.

Plan-15 phase 06 will trim the legacy "Implementation path" section once
the matrix is complete; the C2 cells in `tests/closure_matrix.rs` plus the
leak guards above are the canonical regression record for this surface.

---

## Summary: Text vs Reference vs Function freeing

| | Text (`Type::Text(dep)`) | Reference (`Type::Reference(d, dep)`) | Function (`Type::Function(p, r, dep)`) |
|---|---|---|---|
| **Storage** | Stack frame (`Str` = ptr + len) | Store heap (12-byte `DbRef`) | Stack frame (16B: 4B d_nr + 12B closure DbRef) |
| **dep list used for freeing?** | No — always freed | Yes — only freed when `dep.is_empty()` | Indirectly — `tp.depend()` keeps closure-record dep visible to the standard `Reference` free path |
| **dep list purpose** | Type compatibility / format string tracking | Ownership tracking | Protects closure work var via `tp.depend()` |
| **Free opcode** | `OpFreeText` | `OpFreeRef` | `OpFreeRef` on the closure-record DbRef (D1: stack-frame exit; D3: `Parts::ChildRec` cascade) |
| **skip_free flag** | Not checked | Checked | N/A |
| **Return-value exemption** | By `ret_var` identity only | By `ret_var` identity AND `tp.depend().contains(&v)` | Via `tp.depend()` on declared return type |

---

## Removing from a collection — unlink is not the same as free

Two operations, deliberately separate (`src/database/search.rs`):

- **`Stores::remove`** — UNLINK only. It must stay that way, because a SECONDARY
  index (a sibling field's `other_indexes`) shares its records with the primary
  collection; freeing there corrupts the primary. `dedup_keyed` makes exactly
  this split when a new insert displaces an existing key.
- **`Stores::remove_owned`** — unlink AND release. This is what a user-level
  removal (`c[key] = null`, `e#remove`) routes through, on both backends.

Nothing paired the unlink with a free on the removal side, so **every removal
leaked**: a constant population of 300 records grew claimed bytes 0.10 → 0.56 MB
over six insert-then-remove-all cycles, and re-inserting after removing
everything grew the store instead of reusing it. It applied to every collection
kind and every element shape — flat records leaked least, which is why it looked
like "only records that own a vector", and a long-lived store grew without
bound. No compaction could have reclaimed any of it: the blocks were still
marked LIVE.

**The order is per-kind and it is load-bearing.**

| element lives | order | why |
|---|---|---|
| **inline** (`vector`, `sorted`) | claims → unlink | the successor is shifted on top of it, so the fields must be read while they are still its own |
| **own block** (`hash`, `index`, `radix`) | unlink → claims → free block | an `index`'s red-black links live in a FIELD, so before the unlink they look to `remove_claims` like owned children — free first and it follows them into the live siblings and takes the subtree with it |

Getting that backwards is not subtle in its symptom: the whole collection reads
back as `Item not found` from `tree::remove_iter`.

`Array` / `Ordered` are excluded from the block free — they hold separate
records too, but their removal path reads `rec` as an inline index and was wrong
before this, so widening to them would compound a different bug. They are not
reachable from loft syntax today.

Guard: `tests/scripts/removal-frees-what-the-element-owned.loft` (five
collection shapes, measured by record count so allocator rounding cannot mask
it, plus a reuse check that the freed blocks are taken back rather than
re-grown).

---

## Assigned in an `if`/`else` branch — who establishes the slot

`scan_if` (`src/scopes.rs`) has to decide where a variable that a branch assigns
LIVES, because a branch scope exits while the value has to outlive it. It sorts
them two ways, and the split is by **storage**, not by how the variable is used:

- **Backed by a heap store** — `needs_pre_init` is true. The slot is established
  at the PARENT scope with a `Set(v, Null)` emitted before the `If`, so the store
  exists before either branch runs and the parent scope owns and frees it.
- **A plain value** — registered at the parent scope directly (`small_both`) with
  no pre-init, because there is nothing to allocate.

Putting a store-backed variable through the second path hoists the OWNERSHIP
without ever creating the store, and the scope-exit free then meets a
stack-record ref where it expects an owned heap one — `BUG (#306)`, or a SIGSEGV
once the branch has written through it.

So `needs_pre_init` must list **every** store-backed type. It listed text,
reference, vector and boxed enum but not the KEYED collections, so a `hash` /
`sorted` / `index` / `spatial` declared under one name in two arms of a single
`if`/`else` chain crashed the interpreter. `vector` was in the list and behaved,
which made the fault read as type-specific rather than as the omission it was.
`--native` derives this separately and was always correct — a divergence worth
remembering when a fault appears on one backend only.

Guard: `tests/scripts/keyed-collection-in-both-if-arms.loft` (every keyed kind,
two- and three-arm chains, an arm that never appends, and a repeat loop so a
per-pass store leak shows as growth).

---

## View returns — dep merge and materialisation (#306)

A function may return a *view*: a struct read out of a vector (`table[idx]`)
aliases the vector's store rather than owning one.  Two rules keep the caller
from freeing a store it does not own:

1. **View of a parameter → dep merge.**  `ref_return` walks the returned
   local's type deps *transitively* and merges every reachable parameter into
   the declared return deps (`fn pick(t, i) -> M` whose body returns
   `chosen = t[i]; chosen` declares `M["chosen", "t"]`).  The call site maps
   the dep to its argument, so the result is a borrow there and is not freed.
2. **View of a local → materialise.**  When a transitive dep resolves to a
   non-parameter local (`pool = build(); pool[i]`), the owner dies at function
   exit, so deps cannot help the caller.  `block_result` / `parse_return`
   rewrite the return value into a deep copy of the view into a fresh
   work-ref, which NRVO-promotes into the caller-provided buffer
   (`materialize_view_return`, `src/parser/control.rs`).

**A `?` on the return does not change rule 1 (loft#974).**  `fn get(b: Bag, k: text)
-> Item? { b.items[k] }` is the same view of the same parameter as its non-nullable
twin, and the borrow belongs to the STORAGE while the `?` belongs to the VALUE — but
the whole promotion pass was gated on `ret_promo_base()`, which peels `Optional`
for a collection only.  A nullable struct return therefore reached
`classify_ret_promotion` never (`LOFT_TRACE_RETPROMO` printed no line at all), declared
`optional(reference(Item, deps {}))`, and the caller read that empty dep list as OWNED:
`OpFreeRef(it)` at scope exit released a store the CALLER still owned, and the next
unrelated allocation took the recycled slot.  Silent on both backends — `2, 0, 0` where
the inline spelling reads `2, 2, 2`, and correct again under `LOFT_NO_SLOT_REUSE=1`,
which is why no poison or UAF sweep saw it.

The two questions are now asked separately.  `Type::ret_dep_shape()` answers the
SIGNATURE one — which shape carries the deps, looking through `?` for every heap kind —
and marks the nullable struct/enum case `SignatureOnly`: `ref_return` records the borrow
and makes **no placement decision**, because that shape already has loft#896's
`__nullable<S>` delivery and a second one leaks a record per call (measured — widening
`ret_promo_base` instead also re-typed the return non-nullable and diverged the
backends on a missing key).  Native's first-bind then ALIASES such a return rather than
deep-copying it (`generation/dispatch.rs`), which is what the interpreter already
emitted (a bare `PutRef`) — without that the copy is a store the IR never frees.

**A branch join carries what EITHER arm borrows (loft#978).**  `it = if fresh { Item {
… } } else { b.items["one"]? }` delivers a fresh record on one path and a view into `b`
on the other, and which one ran is a run-time fact — so the local's type has to admit it
can alias `b`.  It recorded the opposite, and the cause was a HINT read as a lifetime
fact: `parse_if` parses the `else` block with the THEN arm's type as its expected type,
and a block adopted that expected type whole, deps included.  A fresh-record then-arm
therefore published an empty dep list for the else arm's view; an empty dep list is the
OWNED reading everywhere; scope exit freed the container's record.  The tell was that the
defect was ARM-ORDER sensitive — writing the view first read correctly, because then it
was the fresh arm being handed someone else's deps.

Two rules, and they are separate because they answer separate questions:

- **An expected type supplies the SHAPE, never the borrow.**  `Type::with_deps_of` keeps
  the block's own tail deps when it adopts one.  Applied to the `else` arm alone: it is
  the only block handed a sibling EXPRESSION as its expected type, and every other
  caller's is a DECLARED type whose deps are attribute indices — grafting frame vars onto
  those is the cross-space read loft#666 was made of.
- **A join UNIONS its arms' borrows** (`Type::joined_deps`, at the `if`/`else`, the
  `else if` chain, and all six `match` arm sites — the rule is the construct's, not one
  construct's).  Union rather than pick: it can only keep a store alive longer than one
  arm needed, never free one another arm still holds.

What an arm contributes is filtered first (`Parser::arm_join_type`): a dep naming a store
the arm itself MINTS is that arm's ownership marker, not a borrow — `[]` lowers to
`OpDatabase(__vdb_N, …)` and types as a dep on it — and importing one tells the return
machinery the joined value views a local.  The minted set comes from the same
`collect_defs` walk the ownership classifier reads, so the two cannot drift.

**Still open:** an escaping join — a *return* whose arms disagree — has no static answer
that is right for both, and taking the borrow leaks the record the fresh arm mints
(loft#981; a plain `?? Item { … }` accessor has always done this).  That one wants the
runtime adopt-or-materialise decision `OpBindOrCopy` already makes at bind sites.  A
nullable struct-ENUM return still has no arm in the promotion chain at all.

Var-to-var **re**assignment of same-struct references deep-copies (matching
first assignment, `generate_set` reassignment arm) — the parser strips the
LHS deps on that shape assuming a copy, so aliasing there let a later
`OpFreeRef` release a store the variable never owned.

**Ownership-transition free (#316).**  A var whose latest assignment was
OWNED (`chosen = m_none()`) and which is then reassigned with a BORROW
(`chosen = pool[i] ?? m_none()`) would orphan the owned store: the merged
static type already carries deps, so codegen's dep-empty pre-Set free never
fires.  `scan_set` (`src/scopes.rs`) tracks each Reference var's
latest-assignment ownership through the IR (path-sensitive across `if`
branches, conservative across loop bodies) and emits an explicit
`OpFreeRef(v)` before the borrowing Set.  Only provably-owned shapes are
tracked (a call with no visible-attr return dep; a same-struct var copy);
everything else is Unknown and never freed this way.

**Self-read transition residual (#328).**  When the borrowing RHS reads the
variable itself (`x = x.next` — the linked-list walk), the pre-Set free is
skipped (it would invalidate what the RHS dereferences), so the owned store
survives until scope exit: ONE bounded store per owned→self-read-borrow
transition, also when the reassign sits inside a loop (the depth guard).
Correctness is unaffected.  Walks that start from a borrow (e.g. iterating a
structure owned elsewhere) do not pay it.

**An adopted buffer takes no displacement free (loft#1522).**  A local that ADOPTS a
construction work-ref's store — `y: S? = S { n: 3 }` builds the literal in the function-scoped
`__ref_p2_N` and the binding then aliases it, two names for one store — must not release that
store when a later assignment displaces it.  @P378(a) already paired the SCOPE-EXIT free with
the buffer (`OpFreeRefIfDistinct(y, buffer)`, declined while they alias, so the buffer keeps
its store across iterations and frees it once); the displacement free was never paired, and
`Function::owns_displaced_store` reads @FR-O-Proxy's empty dep list as ownership.  Inside a
loop the rebind therefore released the buffer's store while the buffer kept naming it, and the
next pass re-minted through `OpDatabase` — which reuses the slot's store IN PLACE, over
whatever record the allocator had since put there.

`Function::mark_buffer_witnessed` records the pairing where `witness_buffer` records it, so the
two cannot drift, and `owns_displaced_store` — the ONE fact both backends read (@FR-O-NoDiverge)
— vetoes on it.  That is @FR-O-Complete's own stated direction where a single static site cannot
separate the paths: *a leak is recoverable, a premature free is not.*  Nothing leaks, because
the store the rebind stops releasing belongs to a work-ref that frees it at function exit.

The veto is safe only because the loft#1200 runtime flag covers the case it declines — and that
flag had a hole of its own: `local_owns` is keyed by the ORIGINAL var, built from `orig_code`
before the scan, while a second local of the SAME NAME in a sibling scope (`for … { y: S? = … }`
twice, which is ordinary) is split into a var of its own by the time `scan_set` runs.  Looking
it up by the current var found nothing for that half, so its rebinds got no guarded free at all.
Both lookups read `ov` now.  The pairing's bisect step is `LOFT_NO_BUFFER_VETO=1`.

Two runtime backstops make any future wrong-free loud instead of corrupting:
`free_named` refuses to free the eval-stack store (slot 0,
`Stores::stack_store_at_zero`), and the slot allocator panics with a
"store table exhausted" diagnosis at 65535 live stores instead of wrapping
the u16 watermark back onto the stack store.

---

## The return-value exemption in detail

When a scope exits with a return expression, two mechanisms prevent premature freeing:

### 1. `ret_var` — identity match

`returned_var(expr)` walks the return expression to find the last `Value::Var(v)`.
That variable is skipped entirely in `get_free_vars`.  Works for both Text and
Reference.

### 2. `tp.depend().contains(&v)` — dependency match

The return **type** (`tp`) carries a dep list.  If variable `v` appears in that list,
the return value borrows from `v`, so `v` must outlive the scope.  This only applies
to References (not Text, which is always freed).

Example:
```loft
fn get_name(p: Point) -> text {
    p.name    // returns text; p (Reference) is in return type's deps
}
```
Here `p` has `Type::Reference(Point_dnr, [])` (owned), but the return type is
`Type::Text([v_p])` where `v_p` is p's variable number.  The check
`tp.depend().contains(&v_p)` prevents `OpFreeRef` for `p`, keeping the store record
alive until the caller reads the text.

---

## Text from structs — deps keep the entire ownership chain alive

When a text field is read from a struct — or from any sub-reference reachable from
that struct — the resulting text type inherits every variable in the access chain as
dependencies.  This is critical: without it, any struct in the chain could be freed
while the text still points into store memory.

### How field access builds the dep chain — `src/parser/fields.rs:130-137`

```rust
// Normal (non-constant) field access:
let dep = t.depend();                    // existing deps on the struct expression
t = self.data.attr_type(dnr, fnr);       // field's declared type (e.g. Text([]))
for on in dep { t = t.depending(on); }  // inherit parent's deps
if let Value::Var(nr) = code {
    t = t.depending(*nr);                // add the struct variable itself
}
```

For a text field on struct variable `p`, this produces `Type::Text([v_p])` — the
text depends on variable `p`.

### Why this matters for freeing

Consider:
```loft
fn get_name(p: Point) -> text {
    p.name    // Type::Text([v_p])
}
```

At scope exit, the free logic processes each variable:

1. **`name` (the text result)**: Text is always freed via `OpFreeText` — the dep list
   is not checked.  But `name` is the `ret_var` (the variable being returned), so it
   is **skipped entirely**.

2. **`p` (the struct)**: `p` has `Type::Reference(Point_dnr, [])` — owned, empty deps.
   Normally this would be freed.  But the return type `tp` is `Type::Text([v_p])`, and
   the check `tp.depend().contains(&v_p)` finds `p` in the return type's dep list.
   So `OpFreeRef` is **suppressed** for `p`.

The struct stays alive long enough for the caller to read the text from the return
value.  The caller's scope then frees both the text and (eventually) the struct.

### The dependency chain protects against premature free

```
p.name → Type::Text([v_p])
                      ↑
                      └── "this text was read from p"
                          → at scope exit, p must not be freed if this text escapes
```

Without the dep, `p` would have `dep.is_empty() == true` and no mention in the
return type's dep list, so `OpFreeRef` would fire — deallocating the store record
while the caller still expects to read the returned text from it.

### Sub-references and intermediate variables extend the chain

The same mechanism applies to any depth of struct nesting.  When field access traverses
sub-references, each step inherits the parent's deps and adds the parent variable, so
the final text carries the entire ownership chain:

```loft
fn get_city(company: Company) -> text {
    addr = company.address;   // Type::Reference(Address_dnr, [v_company])
    addr.city                 // Type::Text([v_addr, v_company])
}
```

The return type `Text([v_addr, v_company])` contains both `v_addr` and `v_company`.
At scope exit:
- `v_addr` is found in `tp.depend()` → `OpFreeRef` suppressed for `addr`
- `v_company` is found in `tp.depend()` → `OpFreeRef` suppressed for `company`

The entire chain from text back to the root struct is kept alive.

### Deeper nesting — every sub-reference is protected

This extends to arbitrary depth.  Each field access step in `parse_field`
(`src/parser/fields.rs:130-137`) inherits deps from the parent expression and adds the
parent variable, so deps accumulate transitively:

```loft
fn get_street(org: Organization) -> text {
    hq = org.headquarters;          // Reference(Office_dnr, [v_org])
    loc = hq.location;              // Reference(Location_dnr, [v_hq, v_org])
    loc.street                      // Text([v_loc, v_hq, v_org])
}
```

At scope exit the return type `Text([v_loc, v_hq, v_org])` protects all three
references from being freed:

```
loc.street → Text([v_loc, v_hq, v_org])
                    ↑      ↑     ↑
                    │      │     └── org must stay alive (root owner)
                    │      └──────── hq must stay alive (intermediate)
                    └─────────────── loc must stay alive (direct parent)
```

Without this transitive dep chain, freeing `org` at scope exit would deallocate the
store record that `hq` points into, and freeing `hq` would deallocate the record that
`loc` points into — both while the returned text still references store memory.

The same principle applies to non-text sub-references too: a borrowed Reference
(`dep` non-empty) is never freed by `get_free_vars`, and its presence in the return
type's dep list prevents its parent from being freed.  Text is simply the most visible
case because text is always freed unless it is the `ret_var` itself — making the dep
chain on the return type the only thing keeping the structs alive.

### Text-to-text: no dep added

When reading a text *variable* (not a struct field), no self-dep is added
(`src/parser/objects.rs:115-119`).  This is correct because text lives on the stack
frame — there is no separate store allocation to protect.  The text variable IS the
value, so the `ret_var` identity check is sufficient.

---

## Closures are structs — the same lifetime rules apply

A closure record is a store-allocated struct.  When a lambda captures variables, the
compiler synthesizes an anonymous struct `__closure_N` with fields matching each
captured variable.  At runtime, the closure record is a `DbRef` pointing to a store
allocation — identical to any other struct.

This means text read from a closure record should follow the exact same dep chain
rules as text read from a regular struct: the text must depend on the closure record
variable, and that dep must keep the closure store allocation alive.

### Closure record allocation — `src/parser/vectors.rs:628-712`

- Anonymous struct `__closure_N` with fields matching each captured variable
- Allocated at lambda **definition time** via `OpDatabase`
- Each captured value is **copied** into the record's fields (set_field)
- The record's `DbRef` is embedded in the 16-byte fn-ref slot
- Work variable `__clos_N` has type `Type::Reference(closure_d_nr, [])` — owned

### Adopted vs borrowed captures — who frees the store behind a capture (#682)

A reference / collection capture is stored as a 12-byte `DbRef`, so exactly one owner
must reclaim it.  `free_named`'s cascade (`src/database/allocation.rs`) frees the
record's `DbRef` fields when the record dies, and `get_free_vars`' `captured_ref`
suppresses the defining frame's own `OpFreeRef` for a captured reference.  Those two
are a PAIR: the suppression is what hands the store over, and it is what makes an
escaping factory closure sound (#323).

The pairing only holds where a frame free existed to suppress.  A captured
**parameter** never enters the scope-exit sweep at all (`variables()`: "never return
function arguments"), and a **projection local** (`ch = w.chunks[1]`, a `for` element)
is `owns == false` — so for both, the cascade was a second free of a live store.  It
destroyed the caller's value, and because a freed-but-unreused store still reads
correctly, the crash surfaced thousands of ops later in whatever function next
touched it.

The two cases are now distinct in the schema.  Both are 12 bytes at align 4, so
nothing about layout, reads or writes changes — only the free decision:

| Marker (`Deps`) | Field storage type | Cascade frees it? |
|---|---|---|
| `share_sentinel()` | `dbref` | yes — the record ADOPTED the store |
| `borrowed_share_sentinel()` | `dbref_borrow` | no — someone else owns it |

**The verdict is not knowable at parse time**, which is why it is not decided in
`synthesize_closure_record`.  `ch = pick(w, 1)` parses as "borrows `w`" from the
callee's declared return, and only `scopes::check`'s call-result rewrite
(`make_independent`, the `!adopts_fresh_store` arm) turns it into OWNED once it knows
the return ABI deep-copies into a fresh store.  Reading the parse-time dep leaked
that copy.  So the decision is `scopes::mark_borrowed_captures`, run after every dep
rewrite has settled; `--native` reads the marked attribute type directly, while the
interpreter's already-registered schema is synced by
`typedef::sync_capture_ownership` from `compile::byte_code_from`.

A `__cell_<T>` capture (plan-22's boxed mutated scalar / text) is ALWAYS adopted: the
cell is minted for that closure alone, so the record is its only possible owner
however the original binding was reached.

Both `dbref` shapes are registered **together**, from either entry point: type
numbers are positional and `--native` replays the registration sequence to rebuild
the schema, so a shape present in only some programs would shift every id after it.

### A mutated scalar PARAMETER is boxed through a shadow local (#685)

`flip_scalars_to_box_types` cannot flip an argument in place: its slot receives the
caller's scalar, and a 12-byte cell `DbRef` type there would change the call ABI. It
used to skip arguments outright — but `box_captured_names_for_outer_scalars` did not,
so the closure record got a 12-byte cell field fed from an 8-byte stack slot, and
`emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of 8 and corrupted the fn-ref
being built beside it. **The two halves of "is this capture boxed?" must be decided by
one fact.**

They are, by making the argument case *be* the local case:
`promote_boxed_scalar_arg` mints `__bx_<name>` with the argument's own type,
`set_promoted_from` + `remap_name` point the name at it before the body parses, and
`parse_code`'s promoted-argument preamble seeds it at function entry. The emitted IR is
then byte-identical to the working local shape — which is why every boxable type is
covered with no per-type work, and why the caller's value stays untouched (a scalar
parameter is by value; the argument slot is never written).

Two constraints worth knowing before touching this:

- The shadow is marked **`defined` at creation**, so the body's first write does not
  ALSO prepend an allocation (`maybe_prepend_cell_alloc`). A second `OpDatabase` would
  replace the seeded cell and silently lose the argument's value.
- `RefVar` arguments are excluded. A user `&T` out-parameter's writes must reach the
  caller (a private cell would swallow them), and a mutable text local the compiler
  already promoted to a hidden `&text` out-parameter is itself the working path — a
  first attempt without this exclusion refused code that worked.

The seed shares `boxed_cell_alloc_and_set` with the first-assignment path — one home
for "a boxed scalar comes into existence", because the seed is the only assignment a
parameter's cell is guaranteed to have (the mutation may live entirely inside the
closure).

### Which mutated captures take a CELL, and which stay inline (#687)

A mutated capture is normally boxed into a shared `__cell_<T>` so the closure's writes and
the enclosing body's reads hit one location.  The exception is a binding that **already
carries its own indirection**: a text local that is the function's RETURN SOURCE, which the
return machinery promotes to a hidden `&text` out-parameter so the caller supplies the
buffer.  That binding cannot also be a cell — and does not need to be, because the record
stores the value inline and the existing per-call write-back propagates the closure's
changes.  Two indirections for one binding is a crash: the record holds a cell DbRef while
the binding is a `&text` stack pointer.

**`RefVar` is the fact.**  Plan-22 02d-vii used to stand in for it with "skip text boxing
when the parent returns text", which was wrong in both directions — too wide (it also
skipped a text local the function does NOT return, which boxes cleanly) and no help for a
PARAMETER, which has no indirection to reuse and so was refused outright.

**Where the decision lives.**  Two halves have to agree — the record's attribute type
(`box_captured_names_for_outer_scalars`, at the LAMBDA's epilogue) and the binding's own
type (`flip_scalars_to_box_types`, pass 2) — and the epilogue is too early to be right: a
to-be-returned text local is still a plain `Text` there and only becomes `RefVar(Text)`
later in the body.  So the epilogue boxes PROVISIONALLY (the common case, and the attribute
must say something because pass 1 freezes the record's storage), and
`Parser::finalize_capture_storage` corrects it at the parent's **pass-1 body end** — the
first moment the fact is final, and still before `fill_all` lays the record out.

One text-returning function can need both answers at once (`keep` returned, `side` not),
which is why no per-function condition can work.

### A capture of a FORWARD-declared type is typed and laid out in pass 2 (#686)

A capture's type is the type of an EXPRESSION (`ch = w.chunks[1]`), so it cannot be
deferred by name the way a written-down `f: World` field can.  With `World` declared later
in the file, pass 1 freezes the record's attribute as `Unknown(0)` — the "no type known"
sentinel.  Two things then went wrong, and the first hid the second:

1. `copy_unknown_fields` read the `0` as a stub def number and set the field to
   `data.def(0).returned` — in practice `text`.  A LYING fact: the field looked resolved,
   so the closure body type-checked against a type the program never mentions
   (`Unknown field text.cells`).  Guarded now (`was != 0`, matching the `Vector` arm).
2. With no invented type, the field was unsized — and `fill_database`'s field loop SKIPS an
   attribute it cannot size while still registering the struct, so `finish` sized the
   record with `position == u16::MAX` and `finish_type` never revisits a sized type.  The
   closure read and wrote its capture at **offset 65535**, intermittently fatal.

**The invariant: a struct is laid out only once its fields are sized.**  `fill_all` skips a
def whose fields are not all known yet (`layout_blocked`); the loop is keyed on
`known_type == u16::MAX`, so this defers rather than drops.  `parse_lambda`'s
`resolve_forward_captures` then re-types the attribute from `capture_context` at LAMBDA
ENTRY — not at the synthesis epilogue, which runs after the body — and lays that one record
out via `Stores::lay_out_record`.

Why the on-demand layout is required: the body bakes field offsets into its IR as it
parses, so the end-of-pass `finish()` is too late, and a full `finish()` mid-parse
re-appends keyed-index bookkeeping.  This is the same deferral `lay_out_synth` already makes
for a forward-referenced synth enum.

⚠ **The second fault was INTERMITTENT** — a positionless field only crashes when the bytes
at offset 65535 happen to be fatal.  Two byte-identical probe files disagreed during the
investigation, and single runs "confirmed" three different stories before a repeat-run
harness (N≥12) replaced them.  Any probe in this area needs one.

### A field whose type another MODULE declares (#797)

The same invariant, reached by a different route, and with a wider blast radius: the
struct here is one a package author wrote down, not a synthesised closure record.

A package entry that `use`s a module before declaring the types that module names
suspends itself at the `use`.  The module is then parsed to completion — layout
included — while every such type is still a `DefType::Unknown` stub.  `fill_database`
skips a field whose `type_elm` is `u32::MAX`, so the field gets no slot, and the stub is
upgraded IN PLACE moments later when the entry resumes and declares the type.  The
DECLARATION therefore ends up correct and the LAYOUT keeps the hole — the two disagree
for the rest of the run, and `position()` answered `u16::MAX` for a field the program
could name.  Same offset-65535 write as #686, but into the neighbouring records: the
symptom depended on what happened to sit next to the record, so one run gave a SIGSEGV
inside an unrelated walk, another an allocation that reached 59.6 GiB, and a third a
clean pass.

Three sites had to agree for the deferral to hold:

- **`layout_blocked`** — the guard, now covering `Unknown(stub)` as well as `Unknown(0)`,
  and TRANSITIVE, because an inline field stores its content's bytes so a host whose
  field type is waiting cannot be laid out either.
- **The sweep at the top of `fill_all`** — re-runs `copy_unknown_fields` over everything
  still unlaid, so each `fill_all` picks up whatever the files parsed since have
  declared.  Without it the deferral never ends: `actual_types_deferred` only sweeps the
  file it is finishing, and nothing was asking again.
- **`copy_unknown_fields` peels `Optional`** — `S?` and `S` name the same forward
  reference.  `Type::Optional` is the wrapper this area keeps forgetting: `rewrite_type_opt`
  and the native `init()` generator's field-hoist match were missing it too, so a nullable
  forward field failed where a plain one worked — the generator emitted
  `db.field(t_host, "f", t_content)` ahead of `let t_content`, i.e. the library did not
  compile.  Adding an arm per site is how that gets missed a fourth time; each of the
  three peels the marker and asks about the base.

Guard: `forward_module_type_gets_a_slot` (`tests/issues.rs`), driving
`tests/multilib/fwd797_layout.loft` over the `fwd797` fixture.  It asserts SIZES as well
as values — a read follows whatever offsets the layout ended up with, so reading a field
back cannot by itself prove the field has storage.

The resolution half — a type named in a function BODY rather than in a field DECLARATION —
was loft#801 and is closed too: the body sites now leave the same forward-reference stub,
and `parse_file` drains its suspended parents on a plain Error instead of abandoning them.
See [COMPILER.md § How a forward reference actually resolves](COMPILER.md).

One member of the family is still open, and it is an ENUM:
[loft#803](https://github.com/loft-lang/loft/issues/803).  A struct field whose enum is
declared later is laid out against an unregistered enum, so the field takes zero width and
the field AFTER it loses its position — this same corruption, one field along.  Do not
patch it from here: the issue records three fixes that each looked right and each broke
something measurable (a silently wrong variant, a layout that never lifts, renumbered
native type ids).  The order in which an enum gets its runtime type and its variant
discriminants is the actual question.

### Inside the lambda: `__closure` is a struct parameter

When parsing the lambda body, the compiler adds a hidden `__closure` parameter:

```rust
// src/parser/vectors.rs:391-399
let closure_tp = Type::Reference(closure_rec, vec![]);
self.data.add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
let v_nr = self.create_var("__closure", &closure_tp);
self.vars.become_argument(v_nr);
self.closure_param = v_nr;
```

`__closure` is `Type::Reference(closure_rec, [])` — an owned Reference that is a
function argument (scope 0, never freed by `get_free_vars`).

### Reading captured variables = reading struct fields

When the lambda body references a captured variable like `prefix`, the parser redirects
it to a field read from the closure record.  There are two code paths:

**Path 1** — known closure variable (`src/parser/objects.rs:91-98`):
```rust
let closure_d_nr = self.data.def(self.context).closure_record;
let fnr = self.data.attr(closure_d_nr, name);
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

**Path 2** — capture_context variable (`src/parser/objects.rs:172-175`):
```rust
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

### The analogy to regular struct field access

Compare the closure field read above with normal field access
(`src/parser/fields.rs:130-137`):

```rust
let dep = t.depend();
t = self.data.attr_type(dnr, fnr);       // same — get field's declared type
for on in dep { t = t.depending(on); }  // inherit parent's deps
if let Value::Var(nr) = code {
    t = t.depending(*nr);                // add the struct variable as dep
}
```

Normal field access adds the struct variable (and its deps) to the result type.
For a text field on struct `p`, this produces `Type::Text([v_p])` — the text depends
on `p`, which prevents `p` from being freed while the text escapes.

Both closure field-read paths add `self.closure_param` as a dependency after reading
the field type, matching the pattern for normal struct field access.  This produces
`Type::Text([v___closure])` for a text field read from a closure, matching the
`Type::Text([v_p])` produced by `p.name` for a regular struct.  The dep keeps the
closure record alive while derived text is in use.

---

## The 16-byte fn-ref slot layout

A `Type::Function` variable occupies 16 bytes on the stack:

```
bytes  0.. 4: d_nr       (i32, function definition number)
bytes  4..16: closure     (DbRef, 12 bytes; null sentinel if no closure)
```

### Key codegen paths

| Component | Location | Role |
|-----------|----------|------|
| `gen_set_first_at_tos` | `codegen.rs:843-847` | Delegates to `gen_fn_ref_value` for Function vars |
| `gen_fn_ref_value` | `codegen.rs:466-490` | Ensures every if-else branch produces 16B |
| `OpVarFnRef` | `02_files.loft:350` | Push 16B fn-ref from frame variable |
| `OpPutFnRef` | `02_files.loft:354` | Pop 16B fn-ref into frame variable |
| `OpNullRefSentinel` | `01_code.loft:733` | Pads non-capturing lambdas (4B d_nr → 16B) |
| `fn_call_ref` | `state/mod.rs:221-249` | Reads d_nr at offset 0, closure at offset+4 |

### fn-ref type carries closure dep — `vectors.rs:666-669`

```rust
// A5.6-text: fn-ref depends on closure work var `w` so that
// get_free_vars does not emit OpFreeRef for the closure record
// before the fn-ref escapes the defining scope.
let fn_type = Type::Function(visible_params, Box::new(ret_tp), vec![w]);
```

### Return type dep propagation — `vectors.rs:701-711`

When the enclosing function returns a fn-ref, the closure dep `w` is propagated to
the declared return type so `get_free_vars` at the Return statement sees
`tp.depend()` containing `w`:

```rust
if matches!(self.data.def(self.context).returned, Type::Function(_, _, _)) {
    self.data.definitions[self.context as usize].returned =
        self.data.definitions[self.context as usize].returned.depending(w);
}
```

---

## Current status of closure freeing

### Same-scope closures: WORKING

All same-scope closure tests pass.  `___clos_N` and the fn-ref live in the same
function scope; `get_free_vars` doesn't run between closure allocation and fn-ref use.

**Passing tests** (tests/expressions.rs):
- `closure_capture_integer` (line 317)
- `closure_capture_after_change` (line 323)
- `closure_capture_multiple` (line 335)
- `closure_capture_text_integer_return` (line 355)
- `closure_capture_text_return` (line 364)
- `closure_capture_struct_ref` (line 375)
- `closure_capture_vector_elem` (line 390)
- `closure_capture_text_loop` (line 406)

### Cross-scope closures: WORKING

**`closure_capture_text`** (tests/expressions.rs:343) now passes.

Four bugs were fixed:

1. **Free suppression** — `get_free_vars` used only the block result type (`tp`) for
   the dep check, but the block result type doesn't carry the closure dep that was
   propagated to the function's declared return type.  Fix: also check
   `data.def(self.d_nr).returned.depend()`.

2. **Work-buffer propagation** — the declared `fn(text) -> text` return type didn't
   encode the lambda's work-buffer deps.  `try_fn_ref_call` created zero work buffers,
   so the lambda's `__work_1` parameter received garbage.  Fix: `emit_lambda_code`
   replaces the inner return type with the lambda's actual return type.

3. **fn-ref null pre-init** — `gen_set_first_at_tos` emitted only `NullRefSentinel`
   (12 bytes) for Function variables, but fn-ref slots are 16 bytes.  `PutFnRef`
   overwrote 4 bytes of the next variable.  Fix: emit `ConstInt(i32::MIN)` +
   `NullRefSentinel` for a full 16-byte null slot.

4. **Caller-side closure free** — `get_free_vars` had no `Type::Function` branch.
   The closure DbRef at offset+4 leaked when fn-ref variables went out of scope.
   Fix: add a `Type::Function` arm with a codegen special case that reads the
   closure via `OpVarRef(var_pos - 4)` before `OpFreeRef`.  Same-scope fn-refs
   carry `dep=[w]` (the closure work var), so the free is suppressed — `___clos_N`
   already handles it.

### Caller-side closure free: native codegen path

The interpreter frees closure records via the codegen special case described
above.  Native codegen handles closure-record drop via Rust's RAII at
stack-frame exit (no explicit `OpFreeRef` emission needed); @PLAN15 phase
03–05 leak guards (`tests/leak.rs::p15_phase03_*` / `_phase04_*` /
`_phase05_*`) confirmed both paths produce clean store state under
100-iteration tight loops for text / Reference / nested-closure captures
across D1 + D3.  Cross-mode equivalence (`tests/closure_matrix.rs`)
catches any future native-vs-interp divergence in observable output.

---

## Historical implementation path (closed)

The three bugs that originally motivated `Type::Function` work in
`get_free_vars` — cross-scope closure freeing, caller-side cleanup,
and capturing-into-struct-field — closed through @P213 (struct-field
layout via `Parts::ChildRec`, 2026-05-04), @P215 (nested-closure
name resolution, 2026-05-05), and @P227 (text-returning fn-ref
calls, 2026-05-05).  Plan-15 phases 03–05 (2026-05-12) confirmed
no residual leak via `tests/leak.rs` 100-iteration tight loops
across capture types (text / Reference / nested) and destinations
(local / struct field).

The detailed step-by-step "Implementation path" that previously
lived here described the contemplated `OpFreeClosureRef` opcode +
`get_free_vars` extension — neither shipped because the
`Parts::ChildRec` cascade plus standard local-cleanup already
covers the surface.  Removed during @PLAN15 phase 06 closeout
(2026-05-12); see git history if you need the original analysis.

---

## `OpFreeText` runtime — `src/state/text.rs:270`

```rust
pub fn free_text(&mut self) {
    let pos = *self.code::<u16>();           // stack slot
    let s = self.string_mut(pos);
    s.clear();
    s.shrink_to(0);                          // release heap allocation
}
```

Clears and deallocates the string buffer at the given stack position.  In debug
builds, fills freed memory with `'*'` and checks for double-free.

## `OpFreeRef` runtime — `src/state/io.rs:414-437`

```rust
pub fn free_ref(&mut self) {
    let db = *self.ref_at(pos);              // read DbRef from stack
    self.database.free(&db);                 // return store slot to free list
}
```

Returns the store record to the free list.  The store allocator uses a bitmap
(`free_bits`) for slot reclamation (S29).

---

## Inline-lift safety — the `OpCopyRecord | 0x8000` invariant

Struct-returning calls that appear inline in an expression (format-string
interpolation, chained accessor, assertion, tuple element) are transformed by
scope analysis into `__lift_N = callee(...)` followed by
`OpCopyRecord(src, to=__lift_N, tp)`.  The top bit of `tp` (`0x8000`) is the
**free-source** flag: after copying the returned record into the destination,
the source store is freed.

The flag is necessary for **owned returns** — if the callee freshly allocated
its return (e.g. `fn f() -> T { T { .. } }`), nothing else would free that
store and issue #120 reintroduces.

The flag is **unsafe for borrowed-view returns** — if the callee returned a
view into one of its arguments (e.g. `fn f(c) -> Inner { c.items[0] }`), the
source store is the caller's own data.  Freeing it corrupts the caller.

### The gate

`src/state/codegen.rs` emits the flag only when the callee's declared return
type carries an **empty** `dep` chain (= owned).  If the chain is non-empty
(= view into some arg), the flag is cleared.

Two emission sites:

- `gen_set_first_ref_call_copy` (~`codegen.rs:1284`) — first-assignment from a call
- `generate_set` reassignment path (~`codegen.rs:918`) — re-assignment into an existing ref slot

Both sites test:
```rust
let is_borrowed_view = !stack.data.def(fn_nr).returned.depend().is_empty();
let tp_with_free = if is_borrowed_view {
    i32::from(tp_nr)                 // no free
} else {
    i32::from(tp_nr) | 0x8000        // safe to free (owned)
};
```

### Feeding the gate — dep merging from return expressions

For the gate to work, `def.returned.depend()` must reflect whether the
function's body ever returns a view.  Two parser helpers merge per-return
deps into the declared return type:

| Helper | File | Fires on |
|---|---|---|
| `text_return(ls)` | `parser/control.rs:2264` | `Type::Text` returns, in both `parse_return` (mid-body) and `block_result` (tail) |
| `ref_return(ls)` | `parser/control.rs:2351` | `Type::Reference` / `Type::Enum(_, true, _)` returns, in both `parse_return` (mid-body) and `block_result` (tail) |

The Vector arm of `ref_return` fires only from `block_result` (tail), not
from `parse_return` (mid-body) — promoting mid-body Vector deps would
promote globals and locals to hidden ref args and break callers.

For a mixed-return callee
```loft
fn first_or_empty(c: Container, idx: integer) -> Inner {
    if idx >= 0 && idx < len(c.items) {
        return c.items[idx];   // view
    }
    Inner { n: 0 }             // owned
}
```
the mid-body `return c.items[idx]` carries `Reference(Inner, [c])`.
`parse_return` calls `ref_return([c])`, which merges `c` into
`def.returned`'s dep chain via the `attr_names` idempotency path (since `c`
is already an attribute — no new hidden arg created).  The declared return
becomes `Reference(Inner, [c])`; the gate fires at the call site; `0x8000`
clears; the caller's store is untouched.

### Lock bracket — second line of defence

Both gated emission sites wrap the `OpCopyRecord` in `n_set_store_lock(arg,
true)` / `(arg, false)` for every ref-typed arg to the call.  The runtime
`copy_record` handler at `state/io.rs:1001` skips the source-free when the
source store is locked.  This is a belt-and-suspenders guard for the case
where the dep-chain inference is incomplete.

### Known trade-offs

1. **Owned-fallback leak on mixed-return callees.**  After the dep merge,
   the gate clears `0x8000` for every call to a mixed-return callee.  The
   owned fallback branch's fresh store is no longer freed and leaks.
   Magnitude: one small struct per fallback call; the fallback is typically
   an error path.  Future: promote Reference returns to a caller-provided
   scratch buffer (analogous to the `__ref_1` vector mechanism) to close
   this.

2. **WASM feature unconditionally clears `0x8000`** at
   `gen_set_first_ref_call_copy`.  Safe (no corruption) but leaks
   callee-fresh stores under WASM.  Separate audit.

3. **Vector mid-body returns** are not merged.  If a Vector-returning
   function has `return GLOBAL_CONST;` or `return local_vec;` in a branch,
   `ref_return`'s promotion logic would add hidden ref args that break
   callers.  No Vector SIGSEGV variant has been observed; a future phase
   could filter `ls` to function-parameter vars only.

### History

P181 surfaced the corruption; Phase 1 (2026-04-18) added the gate;
Phase 1b (2026-04-18) added the `parse_return` dep merge; Phase 2
(2026-04-18) audited all `OpCopyRecord` emission sites and confirmed
the invariant holds.  See
`doc/claude/plans/finished/00-inline-lift-safety/` for the full initiative record.

## Diagnostic: `LOFT_LOG=scope_debug`

Set `LOFT_LOG=scope_debug` to trace free decisions at compile time:

```
[scope_debug] freeing 'p' (var=3, scope=2)
[scope_debug] NOT freeing 'name' (var=5, scope=2): dep_empty=false in_ret=false skip_free=false
[scope_debug] ORPHANED Reference 'x' (var=7): its scope=4 is not in the chain to to_scope=2
```

The orphan check catches variables whose scope was never entered in the current chain —
a condition that should not occur after the A5.6 block-pre-registration fix.

## Diagnostic: `introspect --show-ownership` + the free-before-dependent-read overlay

`loft introspect --show-ownership <prog>` renders each binding's resolved ownership
(`Owned` / `Borrowed(base)` / `Join(base)`, with `Owned (backing=…)` for a buffer). This is
the STATIC half of the @PLN103 lifetime inspector (the runtime half is `LOFT_STORES=timeline`).

The verdicts alone are **temporal-agnostic** — they say *who owns what*, not *when a store is
freed relative to its reads* — so a correctly-placed free and a use-after-free render
identically. The overlay closes that gap: `use_analysis::free_before_dependent_read` walks the
committed IR and, along each straight-line path, flags an `OpFreeRef(S)` followed by a
DEREFERENCE of any (transitive) view of `S`:

```
  ⚠ UAF: `arg` is read AFTER `OpFreeRef(__vdb_1)` — `arg` views the freed store
     (backing=__vdb_1); free-before-dependent-read
```

Precision rests on two axes: it follows **transitive** view chains (a nested `match arg {…}`
derefs `arg` through an intermediate that only transitively views the store) and counts only
**dereference** reads — a bare `Var` in a `return`/move is a safe delivery the retbuf/ownership
machinery handles, not a UAF. Blind spot: a bug whose root is a *missing* dep (a dropped borrow)
cannot be seen by a dep-based check. Gate: `tests/introspect.rs`.
