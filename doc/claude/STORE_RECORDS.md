<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STORE_RECORDS.md — every store a function touches, from creation to its end

loft frees heap stores at compile time.  Whether a program leaks, frees a store twice or
reads one after its free is therefore decided before it runs, and should be **checked**
before it runs.  This doc defines the abstraction that makes that check possible: a
**store record** per store, attached to each function's IR, naming where the store is
created, every use of it, and how each path ends it.  It also states the rule every
store-lifetime fix follows: [§ Every new defect](#every-new-defect-updates-the-records-and-the-checker).

The ownership model these records describe is [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md); its
formal rules are [formal/ownership.md](formal/ownership.md).  The checker that reads the
records grows out of the existing oracle in `src/ownership_cfg.rs`.

## Why a record, and not more checks over `deps`

Today "who owns this store" is spread over several facts that must agree: the `deps` list on
each variable's type, the function signature (built over two parser passes), `skip_free`,
`owns_store`, and the scope passes under `src/scopes/` that each derive part of it.  The
code generator reads those facts to place the frees.  A checker that reads the same facts
agrees with every defect in them: `deps=[v]` on a local that holds a fresh store reads as
"a view, nothing to free" to both.

So the records follow one rule above all:

> **Independence.**  A store record is derived from the IR and from the callees' own
> records, never from a fact the code generator decided from (`deps`, `skip_free`,
> `owns_store`, `return_adopts_fresh_store`).  Where the two disagree, one of them is wrong,
> and the checker says so.

## The record

One record per store a function can hold.  It is built from the function's FINAL IR, after
the scope pass has placed the frees, so it describes exactly what both backends translate.

| Part | What it holds |
|---|---|
| **Origin** | how the store comes into being — see below |
| **Holders** | every variable whose value points into the store: its owner and every view |
| **Uses** | every IR node that reads or writes through a holder |
| **Ends** | per path, what ends this function's responsibility for the store — see below |

**Origins:**

- **Mint** — an `OpDatabase` or record construction into a variable of this function.
- **Call, fresh** — a call to a function whose return summary says it hands over a store
  of its own (read from the callee's records, never from the caller's `deps`).
- **Call, join** — a callee whose answer is a fresh store on some runs and a view of an
  argument on others; the record carries the condition its end is guarded by.
- **Copy** — a deep copy into a new store.
- **Foreign** — a parameter's store, a constant or a global: held here, never ended here.
  A free of a foreign store is a fault.

**Ends:**

- **Free**, in any of the spellings `OpSets::frees` names, and the source of a copy that frees
  it (`OpSets::freed_source_arg`: `OpCopyRecord`, `OpReplaceKeyed` and `OpSetChildRec` under
  `COPY_FREE_SOURCE`), except a source in the destination's own store.
- **Transfer**: returned to the caller, stored into another store (which then owns it),
  captured by a closure, or handed to a call that consumes it.
- **Guarded free**: a free that is a no-op when this path's store is already ended, as in
  the free each arm of a value branch emits.  It is a separate kind, so a deliberate guarded
  re-free is never counted as a double free, and an unguarded one always is.

**Return summary.**  A function's records answer what its result is to a caller: a fresh
store, a view of parameter *i*, or a join of the two.  The summaries are the least fixpoint
over the call graph, so a recursive function gets the same answer whichever order the
parser reads it in.  A summary comes from the body, never from which parser pass ran.

## The checks

One walk over the control-flow graph `src/ownership_cfg.rs` already builds:

| Fault | Reported when |
|---|---|
| **leak** | a path from a store's origin reaches the function's exit with no end |
| **double end** | a path passes two unguarded ends of one store |
| **re-free** | a double end whose second end is a free the runtime refuses (`@FR-H-FreeTwice`) |
| **use after end** | a use of a store is reachable from one of its ends |
| **wrong end** | a free of a foreign store, or of a view |

The runtime detects the same four, on the paths a run takes: the leak check
(`LOFT_LEAK_CHECK`), `LOFT_STRICT_STORES` for a use after free (it never reuses a slot), and
the refused-free reports (#306, #405).  A runtime fault the checker did not report is a hole
in the records or the checks.

**A re-free is a defect, not a tolerated case (owner).**  Both runtimes refuse a free of an
already-free slot (`Stores::free_named`, and `loft_rt.h` for the C output), but the slot
number is all they check: when anything allocates between the two frees it takes that slot,
and the second free releases the new owner's store.  The C backend ships what the compiler
emits, so the compiler emits each free once on every path wherever it can.  The records
report a re-free as its own kind, so the list of compiler sites still emitting one stays
visible, and `LOFT_DOUBLE_FREE` counts the ones a run reaches on either backend.
The interpreter, `--native` and the C output (`loft_rt.h` `loft_refree_fail`) fail the run
at a re-free, the interpreter giving the position of the free and of the one freeing it again; `LOFT_REFREE_FAILS=1` names each free's function, line and
variable too (a name table the compile otherwise skips), and `LOFT_ALLOW_REFREE=1` lets a
re-free pass on every backend (a `-DLOFT_DOUBLE_FREE` C build counts instead).  No suite cell
re-frees, under `LOFT_STRICT_STORES` either.  Every backend resets a variable to null after its
free, so a stale variable freed again is a free of null (`Codegen::generate_call` on the
interpreter); `LOFT_NO_FREE_RESET=1` limits the interpreter's reset to the variables that
took a pre-build free and to lift temps.  A compiler temporary reset to null frees nothing:
the compiler writes that null only after the temp's store was released or handed over
(`bind_lowering::re_bind`); `LOFT_TEMP_NULL_RESET_FREES=1` restores the free.  A `match`
subject temp borrows what its subject borrows (`parse_vector_match`; an iterator subject's
buffer views its `__vdb`); `LOFT_NO_MATCH_SUBJ_BORROWS=1` leaves it owning that store.  A slice
materialised where a vector is wanted (`a = v[..-1] + [9]`, `iterator_as_vector`) answers
the borrows of its hidden local; `LOFT_NO_SLICE_LEAF_BORROWS=1` types it dep-less.  A
literal handed to a callee that returns its parameter (`st = id(P { … })`) is paired with the
binding that adopts it (`adopted_work_refs`); `LOFT_NO_CALL_DELIVERED_PAIR=1` leaves it
unpaired.  A vector local that owns what it names through an append's guarded backing gets no
second owner for a call's fresh result (`own_fresh_binds`);
`LOFT_FRESH_BIND_OWNER_BESIDE_GUARDED=1` gives it one.  A work-ref whose deps name only
other bindings is a view and takes no scope-exit free (an update over a value branch,
`S { l: 7, ..(if c { p } else { q }) }`); `LOFT_WORK_REF_VIEW_FREES=1` frees it.  An append
whose copy frees a construction's store resets the construction's work-ref after it
(`v += [S { op: 0, ..n }]`); `LOFT_NO_APPEND_SOURCE_DISARM=1` leaves it naming the store.  A
nullable slot write's stash of a projection (`a.1` into a returned tuple) borrows the
projection's root (`emit_nullable_slot_write`); `LOFT_NO_STASH_VIEW_DEP=1` types it as an owner.
A tuple held from a record's unbox (a call's tuple returned or appended) borrows the work-ref
the unbox binds (`unboxed_member_types`); `LOFT_NO_UNBOX_MEMBER_DEPS=1` leaves its members owning.  A `(R-HostStore)` result is let
go after a field adopts it as a detach (`OpNullRefSentinel`), not a free of the store the node
was built in; the records see the free as a use after end of the node
(`LOFT_HOST_LETGO_FREES=1` restores it).  A whole write through a `&` link to a plain parameter
releases nothing it displaces (`Function::link_write_releases`, `@FR-F-ParamRebind`); the
records follow a link installed on one variable as a write to that variable, and a release of
a parameter's store is a wrong end (`LOFT_LINK_PARAM_RELEASES=1` restores it).
A tuple member's free retires every work-ref an assignment made that member's claimant, where
it still names the store (a loop drops the per-assignment claimant, `tuple_claimants`);
`LOFT_NO_CLAIMANT_DISARM=1` leaves them naming it.  A collection a fn-ref call delivers into
a local that another assignment binds to a view gets a lift temporary as its owner, and the
local views it (`scan_set`); `LOFT_NO_CALLREF_LIFT=1` makes the local its owner again, whose
displaced free then releases the view's temporary store.  A nullable value stored into a
declared dense local keeps what it borrows (`dd: S = o.opt` views `o`);
`LOFT_NO_REPR_KEEPS_DEPS=1` types the local as the bare declaration, an owner.  An inline
`??` is lifted into an owner only where its subject temp owns nothing (loft#879's leak);
`LOFT_NCC_LIFT_BESIDE_OWNER=1` lifts beside a subject temp that owns the store too.  The
interpreter's reassignment copy goes into a fresh store wherever the displaced store waits
for its stashed free; `LOFT_NO_STASH_FRESH=1` writes into it, and a null answer then frees it
twice.  A captured local that only names its backings' stores stands its frame-end release
down (`rebound_captures`); `LOFT_NO_BACKED_CAPTURE_STAND_DOWN=1` keeps it beside theirs.  The
first bind of a record local assigned again later, from a value branch of whole variables, is
written out per arm as a reassignment is (`scan_set`'s `writes_out`);
`LOFT_NO_MULTI_FIRST_WRITES_OUT=1` keeps the per-arm lift the binding borrows.  A
reassignment copied into a fresh store keeps the local's old store protected across the copy
when the value reads the local (`ReBind::protects_old`, one answer for the code generator and
the records, beside `ReBind::copies_into_fresh`); `LOFT_NO_FRESH_PROTECTS_OLD=1` leaves it out,
and the copy's source-free and the stashed free both release it.  A literal buffer adopted by
an owning local that is assigned again takes no scope-exit free (the local releases its store
on every path) — a local renamed onto the return buffer included, which releases at exit
through its entry witness; `LOFT_NO_REBOUND_ADOPTER_DISARM=1` keeps the pairing.  Such a renamed
buffer, rebound from a call that reads it, copies the call's answer where the copy lands in a
fresh store (`re_arm`); `LOFT_NO_BUFFER_FRESH_COPY=1` binds it plainly, aliasing what the call
answered.  A branch arm delivering a vector local into the return buffer frees only the
local's OWNING deps (`materialize_vector_arms_collect`); `LOFT_ARM_FREES_VIEW_DEPS=1` frees a
view dep too.  A tuple bound from a value branch — a `match` block ending in its `if` chain
included — joins its arms member by member, a `null` member blocking nothing
(`branch_tuple_call_mints`); `LOFT_NO_MATCH_TUPLE_JOIN=1` reads only a bare `if`, and
`LOFT_NO_NULL_ARM_TUPLE_JOIN=1` lets a `null` arm leave the member unjoined.  A `&` link
argument (`OpCreateStack(u)`) is protected across a copy's source-free as the store it reaches
(`protectable_ref_args`); `LOFT_NO_LINK_ARG_PROTECT=1` skips it, and the set reads complete while
protecting nothing.  A tuple argument's heap members are a callee's to hand back too: a tuple
literal protects each member's root, a member built inside the literal hoisted to give it a name
(`lift_tuple_call_members`), and any other tuple leaves the set incomplete; a call's result borrows
what the tuple's members borrow (`resolve_deps`); and the records read a member of a tuple
parameter as the caller's store.  `LOFT_NO_TUPLE_ARG_COVER=1` skips the tuple (the copy's
source-free releases the member its holder frees again), `LOFT_NO_TUPLE_ARG_DEPS=1` reads it as
borrowing nothing (a vector result adopts the member's store), and `LOFT_NO_TUPLE_ARG_NAME=1`
leaves the member unnamed (a callee's mint leaks).  An explicit `return p` of a `&` parameter records the borrow as the tail
`{ p }` does; `LOFT_NO_RETURN_LINK_BORROW=1` declares the return fresh.  A generic instance
whose return leaves are its parameters through a mutual recursion over another generic
(`pick` → `pick_again` → `pick`) declares their borrow, as its concrete twin does
(`ViewWalk::template_call_views`); `LOFT_NO_TEMPLATE_CALL_VIEW=1` declares it fresh.  A
reassignment from a call that reads the local and answers a borrowed view copies where the answer
can never be the local's own store (`re_arm`); `LOFT_NO_FRESH_COPY_OF_VIEW=1` keeps the plain
pass-through.  A fresh-returning generic instance copies a NULLABLE parameter it hands back too
(`copy_param_returns_of_fresh_instance`); `LOFT_NO_NULLABLE_PARAM_RETURN_COPY=1` hands it back raw.
An owning record local rebound from a value that reads it and answers a view (`x = x.next ?? y`)
copies into a fresh store (`re_arm`, and the C writer's `reading_view_copy`, one test:
`bind_lowering::answers_a_view_to_copy`); `LOFT_NO_READING_VIEW_COPY=1` binds the view.  A diverging
coalesce bound in its buffer's own scope (`q = mq(4) ?? return`) frees `q` only where it no longer
names the buffer; `LOFT_NO_COALESCE_SAME_SCOPE_WITNESS=1` frees both.  A null test of a temp's
field (`Holder.parse(t).rows == null`) reads it in place, the temp's lift owning the store;
`LOFT_NO_NULL_TEST_PROJECTION_VIEW=1` captures it in a work-ref that frees it again.  The records
read an operator answering a heap value from a text (`OpCastVectorFromText`) as a mint.  A literal
buffer adopted by a renamed return buffer that is assigned again takes no free, its guard against
the return included (`pair_literal_buffers`); `LOFT_NO_GUARDED_BUFFER_DISARM=1` keeps the guarded one.
On rustc `--native`, a call arm inside `output_set`'s `_old_<name>` stash leaves the displaced
store to that stash (`Output::old_stash_var`); `LOFT_NATIVE_INNER_DISPLACED_FREE=1` frees it in
the arm too.  The records read IR, which both spellings share, so they cannot see this one.  A
record update built in a temporary (`Unit { hp: …, ..u }` as a value) gives the temporary a store
of its own and copies the base in, which on a return buffer is the caller's store, refilled
(`parse_object_update`); `LOFT_NO_UPDATE_TEMP_COPY=1` binds a whole-variable base instead, which
named onto the buffer freed the caller's store (the records: `store-wrong-end`).  A call
join whose base is a TUPLE argument is witnessed by the tuple's one heap member
(`use_analysis::join_witness_member`, read by every writer); a tuple with several has no single
witness and is copied.  `LOFT_NO_TUPLE_JOIN_WITNESS=1` compares against the tuple slot, which
holds no store: the records report the re-free (`store-re-free`, "a join's witness is a tuple").  A
fn-ref whose closure record a return releases inside its arms (`free_record_in_omitting_arms`)
takes no release of its own at that return (`if pick { return a; } return b;`);
`LOFT_NO_ARM_RELEASED_FNREF_STAND_DOWN=1` releases both.  A `match` of lambdas carries every
arm's closure record in its deps, as the `if` twin does (`arm_join_type`);
`LOFT_NO_MATCH_ARM_RECORD_DEP=1` keeps only the first arm's, and the fn-ref's release then pairs
with that record alone.  A collection
coalesce read inline (`(g ?? nr_h(i))[i]`) lifts its fallback call into a `__lift_N` inside the
arm (`Scopes::lift_owned_coalesce_arm`), handed off where the outer call consumes it;
`LOFT_NO_COALESCE_ARM_LIFT=1` leaves the call inline, and a keyed fallback leaks.  Where
the fn-ref lift makes a local a view (`v = h(k)`), an append backing the local's guarded mint
marked never-free releases its own store again (`scan_set`); `LOFT_NO_LIFT_BACKING_FREE=1` keeps
it never-free, and the C output leaks it.  A whole write through a `&` link to a plain heap
parameter takes the direct rebind's entry witness (`Parser::rebind_param_through_link`);
`LOFT_NO_LINK_PARAM_REBIND=1` leaves the written record to nobody.  A record of another type
than the return is never promoted onto the return buffer (`classify_ret_promotion_inner`: the
container a projection tail reads stays an inner work-ref); `LOFT_NO_RET_TYPE_MATCH=1` promotes
it, and the caller's buffer is freed under a record of the wrong type.  A vector field replaced from a
projection whose type carries no dep (a destructured tuple member, `(s.n, s.t, s.v) = trio(…)`)
copies it into the staging temp as a typed projection does (`__p154_rhs`);
`LOFT_NO_UNTYPED_PROJECTION_COPY=1` stages the projection itself, which the temp then frees.
A generic instance returning a COLLECTION through a local bound whole from its parameter
(`s = x; s`) declares the borrow its concrete twin does, since a collection's whole bind shares
the store (`collection_return_views_whole_binds`); `LOFT_NO_COLLECTION_LOCAL_VIEW=1` declares it
fresh, and the caller frees its argument's store twice.  A keyed work-ref whose null init mints
a store owns it whatever its deps name (`get_free_vars`, `keyed_owner`);
`LOFT_NO_KEYED_WORK_REF_OWNS=1` reads it as a view, and the store leaks.  Last-use reclaim,
moving a backing's null init down to its build, drops the free of that backing from an early
return ahead of the build, where the store does not exist yet (`lastuse_reclaim`);
`LOFT_RECLAIM_KEEPS_EARLY_FREES=1` keeps it, and the return frees whatever the slot held.

A closure record HOLDS what it captures: each capture slot, keyed by the record's store and
offset, keeps the captured value, and the captured stores are handed on only when a value
naming the record leaves (a return, a store into another owner).  Which record owns a capture
is a per-run fact (`@FR-L-CapOne`) the code generator settles at each return by emptying the
slots of the records that stay, so the records read that choice instead of guessing it.  A
record built inside another store (a fn field) takes its captures over at the capture.  Where
the frame frees a capture only on the paths a record does not hold it, the records keep the
correlation: the records at least one of which names a record wherever the store is live, and
those null there.

The closure records' releases, each switch bringing back one second free (doc strings in
`keys.rs`):

- `LOFT_NO_HELD_FN_SKIP_FREE` — a captured fn-ref's record released by the cascade and the frame.
- `LOFT_NO_BORROWED_DISPLACED_STRIP` — a borrowing record keeps its parse-time displaced release.
- `LOFT_NO_NULL_SLOT_GENERATION` — two builds over one null local never group; both own it.
- `LOFT_NO_EVERY_BUILD_BACKING` — only the last build's backing is handed to a record.
- `LOFT_NO_STAYING_JOIN_CAPTURE_BORROWS` — a staying record owns a join-bound capture rebound later.
- `LOFT_NO_PASS_END_BACKING_DETACH` — a pass-end record release leaves the frame backing in place.
- `LOFT_NO_ARM_WRITTEN_JOIN` — a written-out branch is not read as a join of its arms' stores.
- `LOFT_NO_RECORD_SNAPSHOT_RELEASES` — a build releases a displaced slot the snapshot also releases.
- `LOFT_NO_RECORD_RELEASE_DETACH` — a record's release leaves a local naming its capture's store.
- `LOFT_NO_FDISP_KEEP_TEMP` — a displaced fn-ref snapshot reads as a value use after the scope pass.
- `LOFT_NO_LOOP_BUFFER_CAPTURE_BORROWS` — a staying record owns a capture rebound from a call buffer.
- `LOFT_NO_VIEW_TEMP_TRANSITION_GUARD` — a rebind's displaced free releases a frame temporary's store.
- `LOFT_NO_FKEEP_REBUILD_DETACH` — a rebuild's release leaves the `__fkeep_N` snapshot naming the record.  Two closure records
built over a loop-reassigned capture between the same two assignments group by the store
they adopt, so one owns it (`capture_adoption`, `@FR-L-CapOne`);
`LOFT_NO_LOOP_CAPTURE_GROUP=1` lets each own it, and both cascades free it.  A call that
delivers into the caller's hidden buffer gets no second owner from `own_fresh_binds`;
`LOFT_FRESH_BIND_BESIDE_BUFFER=1` gives it one.  A yielded tuple unboxed from a call's record
hands that record's work-ref to the consumer (`yield_handed_temps`), and one unboxed from an
existing value copies its record members (`yield_owned_value`); `LOFT_NO_UNBOX_YIELD_HANDOVER=1`
does neither.  The records model a handing-over `yield` (its stores are the consumer's, its
handed temps forgotten) instead of giving up on a generator.  A `&` link to a collection
(`c = &h`) never takes a free (every whole-value write goes through it);
`LOFT_NO_LINK_NEVER_FREE=1` lets it free as an owner.  A local bound once to a projection of
a lifted call result (`own = mk(n).bounds`) views the lift; `LOFT_NO_LIFT_PROJECTION_VIEW=1`
keeps it owning.  A tagged-slot read off a call's record (`x = fresh_h(k).e`) borrows the
work-ref that holds the record (`read_through_enum_slot`); `LOFT_NO_SLOT_READ_VIEW=1` types it
as an owner.
The records follow a tuple local's members as variables of their own (`t.i`), so a member's
release and a second name's release of the same store meet.  At a
scope's end the sweep releases one store once where two names hold it
(`scopes::get_free_vars`): a fn-ref and the local closure record it was built over
(`LOFT_NO_FNREF_STAND_DOWN=1` frees both), a record a capturing record's cascade adopts
(`LOFT_NO_CAPTURED_RECORD_STAND_DOWN=1`), and a captured vector local beside its `__vdb_N`
backing (`LOFT_NO_CAPTURED_VIEW_STAND_DOWN=1`).  A
**guarded** free (one that asks first whether the store is live, as `OpStoreLive` does) is
not a re-free.

## Measuring: the sweep

`scripts/own_sweep.py` runs language-suite cells with the checker on and the runtime
detectors armed, once per lowering switch asked for, and gives each run a verdict:
**MISS** (the runtime found a fault, the checker did not), **CAUGHT**, **STATIC** (the
checker reported something the runtime did not see) or **CLEAN**.  A `LOFT_NO_*` switch from
[BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md) turns an ownership lowering off, so it
re-creates a class of defect on today's build.

```bash
scripts/own_sweep.py --suite ../loft-suite                 # the suite, no switch
scripts/own_sweep.py --suite ../loft-suite $(grep -o 'LOFT_NO_[A-Z0-9_]*' \
    doc/claude/BOTH_BACKEND_SWITCHES.md | sort -u | sed 's/^/--switch /') --out sweep.tsv
scripts/own_sweep.py --cell prog.loft                      # one program
scripts/own_sweep.py --out after.tsv --compare before.tsv  # each cell whose verdict changed
```

The goal is **MISS = 0 and STATIC = 0** over the suite under every switch.  A STATIC row is
either a false finding or a fault this run did not reach: run the cell with the switch that
reaches it before calling it false.

Each STATIC finding resolved is recorded as **gap** (a records error, cured in
`src/store_records.rs`) or **real** (a compiler defect the checker finds first, cured in the
three places below), in its commit and as a tally per batch.  Gaps per STATIC is
the checker's false-finding rate; reals per STATIC is how often it finds a defect first.  Both
are meant to improve over time.

## Every new defect updates the records and the checker

**Owner's rule.**  Every new store-lifetime defect — a leak, a double free, a use after free
or a wrong free — is fixed in three places, in the same change:

1. **The compiler**, as before: the cure, at the chokepoint the formal rules name.
2. **The store records**: the construct the defect flowed through gets a correct record (a
   new origin, use or end kind, or the shape the record builder did not follow).
3. **The checker**: with the cure switched off (its `LOFT_NO_*` switch, or the build `make
   falsify` uses), the checker reports the defect.  `scripts/own_sweep.py --cell <cell>`
   with that switch reads CAUGHT, not MISS.

A fix that makes the runtime clean while the checker stays silent on the defect is
incomplete: the next defect of the same shape is again found only by a run.  This holds for
a defect found by any route: a failing run, the sweep, an issue, a consumer's report.

## What the IR does not say yet: how an assignment is lowered

An IR `Set(v, value)` does not say what happens to stores.  The code generator decides it:
`gen_set_first_at_tos` and the reassignment path in `src/state/codegen.rs` choose among about
30 named lowerings (`LOFT_TRACE_BIND` prints the one each assignment takes): bind `v` to the
value, deep-copy the value into a fresh store of `v`'s own and free the source, adopt the
store a callee minted, move, and more.  The choice reads the facts the records must not
trust (`deps`, `return_adopts_fresh_store`), and the Rust-native (`src/generation/`) and C
(`src/c/`) backends repeat it in their own code.

So the IR is not yet a full description of a function's stores, and a record built from it
alone mis-reads every assignment whose lowering copies.  Measured: `x = f(e)` where `f`
answers its argument on some paths reads as "`x` views `e`'s store" in the IR, while the
emitted code copies into a fresh store, so the records report a double free that never
happens.

The store abstraction therefore has two halves: the records, and **the lowering choice moved
into the IR** — each assignment says, in the IR, whether it binds, copies, adopts or moves,
decided once and read by all three code generators and by the records.  Until then the
records can only read the choice the interpreter's code generator made.

## Phases

1. **Records beside the checker.**  Build the records as an observer (the shipped code stays
   byte-identical) and print them with `LOFT_STORE_TABLE=<fn>`; `LOFT_OWN_ORACLE=check-records`
   reports their findings, and `scripts/own_sweep.py --oracle check-records` measures them.
   Built: `src/store_records.rs`.
2. **The lowering in the IR.**  Each assignment carries its lowering kind, decided in one place
   before code generation; the three code generators read it instead of deciding it, and
   `LOFT_TRACE_BIND`'s arms become the kinds' names.
3. **Checks on the records.**  Re-base the leak, double-end, use-after-end and wrong-end
   checks on the records; drive the sweep's MISS and STATIC counts to 0, one shape at a time,
   each with its suite cell.
4. **Gate.**  The suite runner and CI fail on a checker finding, as they do on a runtime one.
5. **Frees from the records.**  The code generator places frees from the records, and the
   `deps`-derived facts become one input to them rather than a second answer.
