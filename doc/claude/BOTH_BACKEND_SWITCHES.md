<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Both-backend optimisation switches

Which switch turns off an optimisation that the interpreter AND `--native` both take, and
what checks it.  Two families: LOWERING rewrites, made at parse time or in the scope pass, so
both backends run the rewritten IR; and RUNTIME policies inside the store, keyed collections
and the free tree.  A wrong answer on `--native` only belongs to
[NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) instead.

Each entry names its switch (the first bisect step), the falsifiers that apply
(`LOFT_STRICT_STORES=1`, `LOFT_POISON=1`, `LOFT_POISON_CLAIM=1`, `LOFT_KEYED_VERIFY=1`, the
native leak check) and its `LOFT_TRACE_*` switch where one exists.  Because both backends
share the rewrite, agreement between them proves nothing here: hand-computed cell values and
the switch A/B are the evidence.  The store's design is [DATABASE.md](DATABASE.md); the rules
are in [formal/rewrites.md](formal/rewrites.md), [formal/heap.md](formal/heap.md) and
[formal/ownership.md](formal/ownership.md).

## Lowering: format appends, record placement and buffers

**`LOFT_NO_FORMAT_APPEND=1`** (`@FR-R-FormatAppend`, default-ON, parse time, BOTH backends)
makes `out += "…{e}…"` build its work text again — with it off, every literal and hole of
the format is appended to `out` directly, in order, where no hole reads `out` and `out` is
a text variable (a field keeps the work text; html `escape_html` 10.3× → 2.6× of Rust,
zttext `materialise` 24× → 6.9×, `flow_layout_full` 107× → 15×) — and is the first bisect step for a wrong, missing or doubled part of
a text built by appending format strings.  `LOFT_TRACE_FORMAT_APPEND=1` names each append
written through and each kept, with the reason.

**`LOFT_NO_LITERAL_EXIT_BUFFER=1`** (@PLN164 B2, `@FR-R-Place`'s callee clause, parse
time, BOTH backends) makes a mid-body `return S { … }` build its record in a store of its
own again — with it off, every literal exit of a record function writes the `__retbuf`
the caller handed, through the same null-guarded mint the TAIL literal has used since
@PLN157 § V, so a callee with several literal exits answers ONE store; that includes a
buffer a CHAIN renamed (`return no_mk()` beside `return Mk { … }`, `parse_circle`'s
shape), and a literal tail beside chain exits — and is the first bisect step for a wrong
record out of a callee with more than one exit.
**`LOFT_NO_PLACE_RESULT=1`** (@PLN164 B2 units 2–3, `@FR-R-Place` + `@FR-R-MoveLast`, decided
after the scope pass, BOTH backends) keeps a call result minting its own store and deep-copying
into its destination again — with it off, a plain local bound from a callee whose every exit
writes the handed buffer, and whose ONE owning destination on every path is a record-literal
field of an element appended to a PARAMETER's collection (`sc.ops += [Op { paint: pp, … }]`),
gets that buffer CLAIMED IN the parameter's store (`OpPlaceRecord`), the field takes it by
RELOCATION (`OpMoveRecord`: the bytes move, the heap handles keep their claims, the source's
block is released on the spot), and the exit releases the record alone on the one path that
still holds it (`OpFreeRecordIn`) and nothing after a move — the path state is a compile-time
fact the pass writes into the IR, never a runtime stand-in (no zeroed source, no `v = null`,
which the interpreter lowers as a store-level free of what the local held).  It is the first
bisect step for a wrong field, a leak or a double free out of a record built by a call and
stored in an appended element; `LOFT_TRACE_PLACE=1` names each admission and each decline
(a read after the store, a hand-off to a call, a rebind, a second destination or host, a
destination in a local record, a loop, a rejoin of a stored and a held path all keep the copy).
Measured 2026-09-16 on the drawing bench's parse row: one store fewer per line and the copy
gone, and a WASH on every counter (`perf stat`: instructions, cycles and cache misses equal
within noise) because the record relocated there carries no heap on that scene.
**`LOFT_NO_REBIND_PLACE=1`** (`@FR-R-Rebind`, default-ON since 2026-09-24, parse time +
scope pass, BOTH backends) keeps `x = f(x, …)` minting the callee's exit record in a store
of its own and copying it over `x` again — with it off, the call's hidden buffer argument IS
`x`, so the callee's exit literal writes x's record in place (a vector field that views the
same field, `Doc { buf: d.buf, … }`, is `(H-CopySelf)` and costs nothing; a scalar the
literal reads off the parameter is STAGED ahead of the first write; `return d` answers the
buffer itself), and a body whose last statement is an explicit `return` takes the buffer
road at all (zttext `delete_range` 138× → 9.6×) — and is the first bisect step for a wrong
field, a leak or a double free out of a local rebound from a call that takes it.
`LOFT_TRACE_REBIND=1` names each admission and each decline (a text field or a vector read
at ANOTHER field of the parameter, a literal-list vector field, a chain exit, a promoted
buffer, the local handed in twice, a view or witnessed local each decline); the falsifiers
are `LOFT_STRICT_STORES` / `LOFT_POISON` / the native leak check on the cells.
**`LOFT_NO_CONST_VIEW=1`** (`@FR-R-Const`, default-ON since 2026-09-25, parse time + scope
pass, BOTH backends) makes a literal-bodied function build its vector on every call again —
with it off, a zero-parameter function whose whole body is ONE vector literal over literals
(`fn face_rows() -> vector<integer> { [0, 0, 0, 4, …] }`, the idiom the `const` diagnostics
themselves prescribe for what the constant store cannot paste) is given a synthetic constant
twin, pre-built ONCE in `CONST_STORE` like a top-level `NAMES = [ … ]`, and a call whose
result only lands in READ positions — the single bind of a local that is only indexed,
measured, iterated or copied into another such local, or the call itself under an index,
a `len` or an iteration — answers `OpConstRef`, the node a top-level constant's use site
emits; the call's lazy buffer is never minted.  Every other position keeps the call (a
write or append through the local, a hand-off to ANY user call, a store into a field or
element, a return, a `&` link), because the constant store is write-locked and the
program's copy must be its own — and is the first bisect step for a wrong element, a
"write to read-only store" panic or a stale table out of a call of such a function.
`LOFT_TRACE_CONST=1` names each function made a constant (and each that is not, with its
body's shape) and each call admitted or declined; the interpreter and native share the
rewrite, so the switch A/B and the cells' hand-computed values are the falsifier.
**`LOFT_NO_COMPACT=1`** (`@FR-R-Compact`, default-ON since 2026-09-25, scope pass, BOTH
backends) makes a vector rebuilt from a contiguous run of its own elements copy again —
with it off, `t: vector<S> = []; for i in a..b { t += [V[i]?]; } V = t;` (a history
truncated to its cursor, its oldest entries dropped) is ONE guarded op: `if 0 <= lo &&
lo <= hi && hi <= len(V) { OpKeepVectorRange(V, tp, lo, hi) } else { the statements as
written }`, the op releasing every element outside the run where it stands, moving the
run to the front as one block and setting the length, and the fallback arm answering
exactly what the copy answered for any range the guard refuses (a negative start, an end
past the length that pads with default records, a null bound).  RECORD elements read
through `?` only — a scalar in range can hold the null its `??` replaces — and `t` named
nowhere else; the filter, prepend and identity forms of the rule keep their rebuild
(dryopea `truncate_to` 259× → 14×).  It is the first bisect step for a wrong, missing or
stale element after a vector is rebuilt from its own elements; `LOFT_TRACE_COMPACT=1`
names each rebuild admitted and each declined with the reason.
**`LOFT_NO_ELEMENT_IN_PLACE=1`** (@PLN164 C1, `@FR-R-InPlaceLiteral`, parse time, BOTH
backends) makes `v[i] = S { … }` build its record in a temp store and deep-copy it into the
slot again — with it off, the literal writes the slot's fields, as the FIELD destination
`o.f = S { … }` always has, with every field expression STAGED first (the language evaluates
a literal's fields before the assignment stores, so `El { a: o.f.b, b: o.f.a }` reads the
place it is about to overwrite) and an omitted field taking its declared default — and is the
first bisect step for a wrong element, a leak or a double free at an element assigned from a
literal.  A computed index keeps the copy (the receiver is re-derived per field write), as do
a collection-TYPED field and a linked-group member.
**`LOFT_NO_BUFFER_IS_PLACE=1`** (@PLN164 C2, `@FR-R-Place`, `@FR-O-Buffer`, parse time, BOTH
backends) makes `h.v = mkints(n)` mint a buffer store, fill it, clear `h.v` and copy every
element in again — with it off, the call is handed the destination AS its return buffer: the
buffer stays the variable the call site mints and only what it holds changes, so the result's
deps and the free sweep are untouched, and `(O-Buffer)` gains the clause that such a buffer is
never freed — and is the first bisect step for a wrong, empty or stale vector field after an
assignment from a call.  Declined where an argument reaches the destination, where the callee
MINTS into its buffer (a returned vector literal does), and for a struct-enum variant's field.
Measured on the drawing bench's parse row: the consumer spells this nowhere — the emission is
byte-identical under the switch — so the gain is structural.

**`LOFT_NO_CALLEE_DISTURB=1`** (@PLN164 C3, `@FR-B-Disturb`, `@FR-B-Ref-Reshape`, BOTH
backends) makes the disturbance walk read THIS frame's ops only again — with it off, a
container a CALLEE grows or removes from disturbs the caller's live view of it, so
`e = sc.els[0]?; grow(sc, 200); e.a + e.b` materialises the view instead of reading an element
that moved (it answered 4294967401 for 3), the reach closed over the call graph
(`disturbed_params_map`) and the copy notice naming the callee — and is the bisect step for a
wrong field read through a view across a call that appends to or removes from a container.
`LOFT_TRACE_DISTURB=1` names each disturbed place; a hidden `__retbuf` is an argument slot and
is excluded, or every record-returning function that fills a vector field would report one.

## Lowering: branches, locals and counted loops

**A value branch of calls witnesses every arm's buffer (@PLN157 § V-af, BOTH backends,
parse time):** `v = if c { mk(i) } else { mk2(i) }` in a loop binds `v` to whichever arm's
hidden return buffer ran, so every arm's buffer is `v`'s witness — the buffers are
allocated once per activation and `v`'s per-iteration free declines against each, where
before the branch paired nothing and every iteration minted and freed a store (the
consumer's `smooth` −26 %).  **`LOFT_NO_JOIN_BUFFER_WITNESS=1`** restores the unpaired
form and is the first bisect step for a leak, a double free or a stale record out of a
loop that binds a record from a branch of calls; `LOFT_STRICT_STORES=1` and
`LOFT_POISON=1` are the falsifiers.

**Store confinement across sibling blocks (default-ON since 2026-08-21, both backends):** a
local reassigned across sibling `if`/`else if`/`match` arms used to keep EVERY arm's store
alive to scope exit, so the watermark grew with the number of reassignment SITES rather than
with how many of them run — a 16-site function peaked at 20 stores whichever single arm was
taken. `recover_backer` confines each block's store to its block: a flat **5** at 2, 4, 8 and
16 sites. **`LOFT_NO_CONF_RECOVER=1`** emits the pre-confinement form and is the first bisect
step for a wrong answer in a function that reassigns a local across sibling blocks. ⚠ The
soundness condition is `store_dead_after_block`, NOT the flag: a local READ after the blocks
does not confine, because freeing a confined store while the local still holds it returns the
wrong element on the branch NOT taken. QUALITY.md § Cluster III Route 2.

**Owner witness for a mixed-ownership local (loft#1336, default-ON, both backends):** a
heap-record local that OWNS after one assignment (a copy, a minting call) and VIEWS after
another carries a hidden `__own_<name>` naming the store it minted while it still holds it;
the IR releases it by store identity at the rebind or at scope exit, and the local itself is
never-free (`formal/ownership.md` @FR-O-Witness). **`LOFT_NO_OWNER_WITNESS=1`** emits the
pre-witness form: the first bisect step for a leak or a wrong answer in a local that is both
copy-bound and view-bound (a walker `cur: Node? = a; cur = cur.next`), and what the
`LOFT_NO_JOIN_OWN` positive controls set beside their own switch.

**The counted loop's second counter (@PLN157 § V-ab, default-ON, both backends):** a
`for i in a..b` whose start is not a literal runs a hidden `next` counter seeded AT `a`
(tested, yielded into `i#index`, then stepped) instead of a null-encoded counter that asked
`if !i#index { a } else { i#index + 1 }` on every iteration — a null test and a select LLVM
cannot fold, half the cost of a tight loop; a literal start already took P3b's single
counter seeded at `a - 1`, which a computed start cannot use because `a - 1` is the null
sentinel when `a` is the type's minimum.  **`LOFT_NO_NEXT_COUNTER=1`** emits the
null-encoded form again on both backends: the first bisect step for a wrong value out of a
counted loop whose start is a variable or an expression.  Every value the compare and the
step see is unchanged, so the edges are too — including loft#1525, an inclusive range to
the type MAXIMUM that never terminates on either form.

**`LOFT_NO_BYTE_COPY=1`** (`@FR-R-ByteCopy`, default-ON, scope pass, BOTH backends) makes
`for i in lo..hi { buf += [t.byte_at(i) as u8] }` push byte by byte again — with it off, the
loop is one append of the bytes `[lo, hi)` behind `0 <= lo && lo <= hi && hi <= size(t)`,
the loop as written running for a range the guard refuses (cbor `encode_bytes` 94.7× →
43.1×; 2.9 → 0.4 ns a byte on the probe) — and is the first bisect step for a wrong, missing
or extra byte out of such a copy.  `LOFT_TRACE_BYTE_COPY=1` names each site admitted and
each kept.

## Lowering: adopting and minting call buffers

**Adopt at first bind (@PLN164 B1, `@FR-O-Move`, default-ON, both backends, parse time):**
a plain local first-bound from a callee that returns the local it promoted onto its buffer
(`fn mk() -> P { o = P { … }; …; o }`) adopts the store the callee minted — one mint and one
free per call where there were two mints, a deep copy and two frees — paired with the call's
buffer for an identity-guarded free exactly as a literal-returning callee's result is.  The
buffer stays null on purpose: pooled at function entry it is freed by the interpreter's
rebind of the promoted local (plan 51 cluster 3's shape; native guards that free with
`_rb_w_`), which is the plan's B1b.  **`LOFT_NO_ADOPT_FIRST_BIND=1`** restores the copy and
is the first bisect step for a leak, a double free or a wrong field out of a local bound
from such a callee; `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` are the falsifiers.

**Reuse the adopting call's buffer (@PLN164 B1b, `@FR-O-Buffer`, default-ON, both backends,
scopes pass):** the record-buffer pool now takes those buffers too, so such a callee is handed
the CALLER's store after the first call and fills it — one store per call site per activation
instead of one per call (the parse row's `Mark` class).  A callee's promoted buffer local then
holds either the caller's store or one it minted, so the scope pass snapshots the store handed
in (`__rbw_<buf> = OpRefAlias(buf)`) and every free of that local declines on it; and a local
first bound inside an `if` (whose pre-init makes the bind a rebind) adopts at that bind
(`Variable::deferred_first_bind`).  **`LOFT_NO_ADOPT_BUFFER_REUSE=1`** keeps those buffers
null again and mints no snapshot — the first bisect step for a use-after-free or a wrong
field out of a callee that rebinds or returns past a local it promoted onto its buffer.
`LOFT_TRACE_POOL=1` names the gate that keeps each witnessed buffer out of the pool.

**Mint at first use (@PLN164 A0, `@FR-O-LazyBuffer`, default-ON, both backends, scopes
pass):** a hidden return buffer (`__ref_N`) is minted in front of the statement that hands it
to a callee, behind `OpRefIsNull`, instead of at function entry — a scanner tried on every
line and matching one paid its buffers' mint and free on all the others (the drawing parse
row −7 %).  A vector buffer carries `Variable::lazy_buffer`, so its entry init is the null
sentinel on both backends and its guarded `Set(b, Null)` is the mint.
**`LOFT_NO_LAZY_BUFFER=1`** mints at entry again and is the first bisect step for a leak, a
double free or a wrong value at a call that takes a hidden buffer.

**`LOFT_NO_WORK_BUFFER=1`** (`@FR-R-WorkBuffer`, default-ON, after pass 2, BOTH backends)
makes a vector local that never leaves its frame mint its store at the declaration and free
it at the exit again — with it off, `v: vector<τ> = []` (τ a scalar) whose every mention is
an operand of an in-place vector operator (push, append, length, clear, remove, an element
read or written as a scalar; `for x in v` included) or a by-value argument of a loft-bodied
callee whose answer carries no dep on that parameter is a hidden WORK-BUFFER parameter the
caller supplies as the per-site work-ref a return buffer already takes (minted once per
activation, freed at the caller's exit), and the callee CLEARS it where the declaration
stood; a callee handed the null sentinel takes the rebound-parameter road at that site (its
own store, released against the entry witness), so an entry the runtime builds by hand owes
it nothing (the probe 81 → 33 ns a call).  It is the first bisect step for a wrong, stale or
leaked vector inside a function that declares one, or for a frame-shape fault at a hand-built
entry.  `LOFT_TRACE_WORK_BUFFER=1` names each local promoted and each candidate declined with
the reason; `LOFT_WORK_BUFFER_NULL=1` is the positive control for the null road (every
caller-side buffer left null, both backends).  A hand-off to a `&` parameter or to a callee
answering a view, a local copied whole into any other place (a record's field, where the
emitter builds it in place or the move elision builds it straight into the field; the
return buffer, which adopts it; another local) or bound purely as a copy of another vector
(one copy in and no push, the borrow elision's), a local numbered before an argument in a
function whose return type carries deps (the swap would renumber them), a local declared
inside a loop (`@FR-R-LoopBuffer`'s length reset is cheaper than a clear), a local written
and never read (the dead-store lint's), a mention in a return, a literal, a link or a
capture, a record or text element, a body that suspends or forks, an entry point, a generic,
a lambda, a function whose address is taken and a `par` worker keep the mint.

## Lowering: `??` chains and nested literals

**A `??` chain is RIGHT-associated (loft#1612, `@FR-G-Assoc`, `@FR-B-Copy`, default-ON, both
backends, parse time):** `x ?? y ?? d` parses as `x ?? (y ?? d)`, so every operand is an ARM of a
plain `if` and an arm's bind is the copy `(B-Copy)` describes.  Left-associated, `(x ?? y) ?? d`
has a subject that is not a variable, which is hoisted into a `__ncc_N` temp that VIEWS the
operand it chose — so the destination shared that operand's store (a record on the interpreter, a
vector and a call-first chain on both).  Either grouping answers the same operand in the same
order, so no program's value moves with it.  **`LOFT_COALESCE_LEFT_ASSOC=1`** parses the old form
and is the first bisect step for a wrong value, a leak or a double release out of a chain of three
or more operands.  A chain the AUTHOR parenthesised still hands the parser a hoisted subject, and
is right-associated in the SCOPES pass instead (**`LOFT_NO_COALESCE_REASSOC=1`** keeps loft#1591's
destination gate, the bisect step for that spelling).  Beside them,
**`LOFT_NO_CHAIN_ARM_SINK=1`** stops a chain BLOCK being a sunk ARM of a branch — with it off,
`x: H = if c { s.h ?? b } else { mk(3) }` writes the chain out as a statement of its own, but ONLY
where every arm of that chain is owned, since an arm that views a place the program can still
reach owes the destination a copy the written-out `Set` cannot make — and is the bisect step for a
double release, a leak or an aliased record out of a branch with a `??` chain in one of its arms.

**A nested literal is written into its field (@PLN164 C6, `@FR-R-InPlaceLiteral`, default-ON,
both backends, parse time):** `sc.ops += [Op { paint: Paint { … } }]` writes `Paint`'s fields
into the element's `paint` field instead of building a store of its own and copying it —
only where the outer record is fresh (an appended element, a construction temporary), so
nothing can read the place while it is written (parse row −6–7 %).
**`LOFT_NO_NESTED_IN_PLACE=1`** builds and copies again, and is the first bisect step for a
wrong field, a leak or a double free in a record built from a nested literal.
**`LOFT_NO_APPEND_STAGING=1`** (loft#1548, `(E-Asgn-Compound)`) lets an appended literal's
field expressions run between the element's mint and its finish again — with it off, a field
that reads or grows its own container (`s.qs += [Q { b: g(s) }]`) is evaluated first; it is
the bisect step for a lost or overwritten element out of such an append.

## Runtime: repeat fills and self-appends

**`LOFT_NO_BLOCK_REPEAT=1`** (runtime, BOTH backends) makes `[x; n]` fill one element at a
time again — a `copy_block` and a `copy_claims` call each — where with it off
`Stores::fill_from_template` doubles block copies and walks claims only for a heap-owning
template (`lock_curved` −14 %); first bisect step for a wrong element out of a repeat literal or
a constant comprehension.  **`LOFT_NO_FIELD_FILL=1`** (parse time, BOTH backends) keeps a
constant comprehension in a struct FIELD (`Lay { best: [for _ in 0..n { 2.0 }] }`) on its
per-element push loop — with it off it takes the repeat-literal lowering a local's already has,
when the field is a plain vector (`is_plain_vector`; a keyed collection stays on the loop, since
n appends are not n inserts); together with the block repeat `lock_curved` 4.2× → 2.2×.
**`LOFT_NO_SELF_APPEND_BLOCK=1`** (runtime, BOTH backends) makes `v += v` copy through a
byte snapshot taken before the growth again — with it off, a self-append is one block copy
inside the grown record, its source re-read from the field slot after the growth (which is
also what fixed the heap-owning case: the claims walk read the source through a number
captured before the growth relocated it — a freed block).  The doubling fill a canvas is
built with is this shape run to a ladder; `render_marks` −8 %.  First bisect step for a
wrong element out of a self-append.

## Runtime: keyed collections

**`LOFT_NO_FAST_ORDER=1`** (@PLN158 keyed class, runtime, BOTH backends) makes every search
that ORDERS — an `index` descent, a `sorted` / `ordered` binary search — compare through the
general `key_compare` / `compare` again, an exact `index` lookup take the boundary descent,
and an `index` insert look its duplicate up before it descends — with it off, the key is
resolved once per search (`keys::FastOrder`), a full-key lookup stops at the equal node
(`tree::find_exact`), and a refused `tree::add` names the duplicate its own descent met
(`index` fill-and-find −33 %) — and is the first bisect step for a wrong element, order or
lookup out of a `sorted`, `ordered` or `index`.  **`LOFT_NO_ONE_PROBE_INSERT=1`** makes a
`hash` insert look its duplicate up and then file the entry as two hash-and-probe walks again
— with it off, one walk answers both (`hash::probe_for_insert`; a 5,000-key fill −37 %) — and
is the first bisect step for a lost, duplicated or unfindable `hash` entry.
**`LOFT_KEYED_VERIFY=1`** is the falsifier for both: every pre-resolved comparison, every
exact lookup and every one-probe insert is checked against the general form as it is made,
and a disagreement panics naming both answers — run the keyed cells or the script corpus
under it after touching `keys.rs`, `hash.rs`, `tree.rs` or the `sorted` searches.  Two more
levers of the same pass carry no switch because they have no second answer to bisect to: a
miss on a collection with no lazy binding answers at once (`Stores::lazy_bound`, a lookup
loop that misses half the time −20 %), and a whole word fed to the hasher on a word boundary
is one inline round (`SipHasher13::write_u64`, digest pinned by `siphash_std_parity`).
**`LOFT_NO_HALF_LOAD=1`** (runtime, BOTH backends) rebuilds a `hash` table at three quarters
full again instead of at half — a writer's policy no reader assumes, so stores written under
either read under both; with it off a miss walks ~1 bucket where it walked 5 at the old
threshold (removal −25 %, the vector + hash group −22 %, fill −21 %), for ~3.6 bytes an entry
more bucket array — and is the bisect step for a `hash` whose table size matters.
**`LOFT_NO_TYPED_KEYED=1`** (`@FR-R-TypedKeyed`, generation time, `--native` only) emits a
lookup in a `hash` with ONE integer key as the general `OpGetRecord` again — with it off it
is `OpGetHashLong`, the key handed over as the integer it is, with no `Content` built, no
type-row dispatch and the walk compiled for the key's kind (613 → 444 instructions a lookup,
`hash_find` −13 %) — and is the first bisect step for a wrong or missing record out of such
a lookup on native; `LOFT_KEYED_VERIFY=1` answers every typed lookup through the general
entry too, and checks a removal's recognised slot against the entry's own index.
`bench/portal/analysis/keyed.md` has the ledger and what is left.

## Runtime: store claims, clears and prefill

**`LOFT_POISON_CLAIM=1`** (`Store::poison_fill`) fills a freshly CLAIMED payload with
`0xDEADBEEF` instead of zeros — the claim-side twin of `LOFT_POISON`'s poison-on-free, and
the falsifier for *"does this caller rely on zero-init?"*: a handle or length read out of
unwritten space becomes a loud out-of-range value the store's guards refuse, where
`LOFT_NO_ZERO_CLAIM=1` only leaves stale bytes that often look enough like zeros to pass.
Census 2026-09-12: 1 232 of 1 261 `tests/scripts` clean on the interpreter, 29 dependent
(the buffer/delivery family), every @PLN157 cell corpus clean on both backends.

**`LOFT_NO_CLEAR_RELEASE=1`** (`@FR-H-ClearRelease`, runtime, BOTH backends) makes a
vector's entry clear a pure length reset again — with it off, clearing a REUSED
store-root vector whose elements own heap releases what they own first, closing an
unbounded leak in every shape-A return buffer (~357 KB per `fronds` call) — and is the
bisect step for a double free or a wrong value at a cleared vector.  `LOFT_TRACE_CLEAR=1`
names each clear's shape and element verdict.

**`LOFT_NO_STORE_RESET_CLEAR=1`** (@PLN157 § V-ag, runtime, BOTH backends) makes that
release a WALK again — with it off, clearing a store-ROOT vector resets the store in one
step and re-establishes its two records, because the shape `@FR-H-ClearRelease` already
tests says the vector owns the store's whole extent (`fronds` −36 %, the free tree was
28 % of that row) — and is the first bisect step for a wrong value, a leak or a
use-after-free at a recycled vector-returning call.
**`LOFT_NO_RESET_CAPACITY=1`** (@PLN157 § V-ai, runtime, BOTH backends) makes that reset
re-establish the vector at the fresh minimum again — with it off, the vector comes back at
the CAPACITY the previous fill reached (the buffer is reused across calls and the store
already holds the extent), so the growth ladder runs once per buffer instead of once per
call, and the freed rungs that took every later claim off `bump_tail` and into the free
tree are never made (`fronds` −22.5 %, 4.28× → 3.42× on x86-64) — and is the bisect step
for a wrong element or length out of a reused vector-returning call.  `LOFT_TRACE_CLEAR=1`
prints the capacity each reset re-establishes.

**`LOFT_NO_PREFILL_IMAGE=1`** (@PLN164 C4, `@FR-R-Prefill`, runtime, BOTH backends) makes
every record mint that is not proven complete-write prefill its defaults field by field
again — with it off, the prefill is ONE block write of a per-type image captured from the
walk's first run over a zeroed span (the `parse` row −11 %) — and is the first bisect step
for a wrong default, sentinel or variant tag in a minted record.  `LOFT_PREFILL_VERIFY=1`
re-runs the walk after every image write and panics naming the type where the bytes
disagree; `LOFT_TRACE_PREFILL=1` names each capture and each use — and a cell can pass
without ever reaching the image, because on `--native` a literal's mint is a complete
write that never prefills and a callee's buffer is minted once per caller activation.

## Runtime: the free tree

**Free-block footers (@PLN157 § V-v, default-ON, both backends, `@FR-H-FreeFooter`):** a
store's FREE block carries its size at both ends, so a record delete coalesces BACKWARD in
O(1) off the footer (tree-confirmed — claimed data can spell a false footer) instead of
leaving adjacent frees to `claim`'s lazy O(blocks) sweep; the sweep stays armed only for
the untracked one-word case.  **`LOFT_NO_FREE_FOOTER=1`** restores the sweep-only form
whole (every delete arms it) and is the first bisect step for a store-layout fault in a
delete-heavy run.  Measured: `fronds` −7.7 % (the § V-u arena's churn), and the class it
retires is the `coalesce_free` cliff PERFORMANCE.md § V-j P2 first measured at 29.5 % of
a shared-arena row.

**The tail free block is a wilderness (@PLN164, default-ON, both backends,
`@FR-H-Wilderness`):** the free block that ends a store is held beside the free tree, and the
tree's insert, remove and best-fit take treat it as the node it would have been — so every
claim takes the block it always took (a seeded side-by-side unit test pins the layout), and a
claim from the tail or a delete into it costs no tree delete, insert or rebalance (64 % of the
parse row's tree claims took the tail; the row −6 % in cycles).  **`LOFT_NO_WILDERNESS=1`**
keeps the tail in the tree again (read per store at construction) and is the first bisect step
for a store-layout fault or a claim that hands out a live block.

**A best-fit claim carves its node in place (default-ON, both backends, `@FR-H-Carve`):** a
claim from a free-tree block at least twice the request leaves the remainder in the block's
place in the tree (links, color, parent pointer) instead of a delete and an insert — the block
is the smallest that fits, so the remainder keeps its order, and every claim takes the block it
always took (`hash_text_keys` −8 %).  **`LOFT_NO_CARVE_IN_PLACE=1`** deletes and inserts again
(read per store at construction) and is the bisect step for a store-layout fault or a corrupt
free tree after a claim.
