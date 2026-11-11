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
`LOFT_REFREE_FAILS=1` makes the interpreter (and `--native`) fail the run at a re-free
instead, naming the op that freed the store and the one freeing it again; it becomes the
default once no suite cell re-frees.  A
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
```

The goal is **MISS = 0 and STATIC = 0** over the suite under every switch.  A STATIC row is
either a false finding or a fault this run did not reach: run the cell with the switch that
reaches it before calling it false.

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
