<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN206 P7 — one debugger over both runtimes

**Goal.**  A program debugged with `loft debug` runs its libraries compiled (their hosted C
units, as a plain run does).  When the debugger needs a routine — a breakpoint in it, a step
into it, a pause while it runs — that routine continues in the interpreter, and the routines it
calls stay compiled.

## The invariant

**A routine's activation runs either compiled or interpreted, and nothing the program can
observe tells which.**  The debugger chooses per activation; a switch from compiled to
interpreted hands over the routine's whole state at a point both forms share.

What makes it possible is P1–P6: one store model, so a record a compiled body made is a record
the interpreter reads, and a unit exports each library function beside the bytecode the
interpreter keeps for it (the verify's twin already runs that bytecode on the same stores).

## The sites that must agree

1. **The call site** (`State::static_call`): a call into a compiled routine the debugger wants
   becomes an ordinary call of its bytecode (`fn_call`).  A static call pushes its arguments as
   an ordinary call does (one stepped slot per attribute, hidden ones included), so the frame is
   the same frame.
2. **What the debugger wants** (`Debugger::wants`): a routine whose code holds a breakpoint, or
   whose callees can reach one (`opt::names::reachable_functions`), or any routine while a step
   into is under way.  The reachability clause is load-bearing: inside a compiled routine a call
   to another compiled routine is a direct C call the debugger never sees, so without it a
   breakpoint two compiled calls deep is silently missed.
3. **The session** (`ReplSession`): a debug session adopts each `use`d library's unit as a plain
   run does, with the same blocks (`native_adopt::scope_library`, the export set, `hosted_split`,
   `cached_or_build_shared_cdylib`, `probe_and_mark_exports`); a library that does not build
   interprets, as before.
4. **The pause point** (P7b, P7c): a loop head, named in both forms by the function's key and
   the loop's scope number — the unit's `loft_ho_commit(key, scope)`, the interpreter's
   `State::loop_sites[(def, scope)]` (recorded by `gen_loop` and `gen_rotated_loop`).

## Failure paths

- A breakpoint in a routine reached only through compiled calls never fires (site 2's clause).
- A routed call whose argument layout differs from an ordinary call's reads its parameters
  shifted (site 1): guarded by stopping inside a routed routine and reading every parameter.
- A routine the session interprets because its library did not build is debugged as before:
  routing is a no-op for a function that is not compiled.
- A generator: a static call to a compiled generator answers a handle; routing it would need the
  interpreter's coroutine creation, so a generator is not routed, and its body has no
  safepoint: a pause waits for it to yield.
- Mid-routine (P7c): a local the unit holds in a shape the hand-over cannot write (a tuple, a
  text borrowed from the unit's own memory) makes that site refuse; the routine runs on to the
  next site that can hand over, or to its return.

## Phases

| Phase | What | Red when |
|---|---|---|
| **P7a** | the session adopts units; a call the debugger wants runs interpreted; its callees stay compiled | a breakpoint inside a compiled library function does not stop, a parameter reads wrong there, or a callee the routine makes does not run compiled |
| **P7b** | a pause while a compiled routine runs reaches it at its next loop head, where it offers its locals and asks to be taken over | a pause during a long compiled loop does not stop, or stops elsewhere than in that routine |
| **P7c** | the routine continues in the interpreter: its locals written into the interpreter's frame by name, the loop head's bytecode position entered, its answer returned to the caller as the call's | a routine continued this way answers differently from the same routine run whole, or leaks or double-frees a store |

P7a is complete on its own: everything a plain run gets from the units, the debugger gets, and
every stop the interpreter-only debugger made it still makes.  P7b and P7c are the "interrupt a
running C program" half.

## Measured

- **P7a done.**  `ReplSession::load_program` adopts the units (`native_artifact::adopt_for_session`)
  and wires their bridges; `State::debug_route` turns a static call the debugger wants into an
  ordinary call.  Guards in `tests/rpc.rs`: a breakpoint two compiled calls deep stops, reads its
  parameter, and the routine's callee runs compiled (bridge hits, `SHARED_DISPATCH_HITS`);
  stepping into compiled routines makes the all-interpreted debugger's stops.
- **P7b and P7c done.**  A pause during a long compiled loop stops in that routine, interpreted,
  and the run continues to the answer the routine gives run whole: an integer sum, and a routine
  holding a vector it grows and a text it appends to (`rpc_a_pause_hands_a_running_compiled_routine_to_the_interpreter`,
  `rpc_a_handed_over_routine_keeps_its_vector_and_text`).  The safepoints cost about 4% on a
  loop whose body is one multiply and modulo; `LOFT_NO_HANDOVER=1` builds without them (its own
  unit key).

## P7b and P7c: the hand-over

A C routine cannot pause in place — the debugger pauses by returning to its driver — so a pause
while a compiled routine runs is a hand-over: the **outermost** compiled routine (the one the
interpreter called) continues interpreted from its next loop head, and the interpreter's own
pause then stops it there.  A routine it is in a call to finishes compiled: its frame is live
under the call.

1. **The safepoint** (`src/c/body.rs` `loop_core`, `expand_safepoints`).  At each plain loop
   head of a hosted unit's function, `loft_pause_asked()`: the host's pause flag
   (`debugger::INTERRUPT`, its address in `HostedTable::pause`) is set, no refusal is latched
   for this bridge call (`loft_ho_off`), and this frame is the bridge's direct callee
   (`LOFT_HDEPTH == loft_br_hdepth + 1`; a unit without `CALL_CHAIN` counts its depth in
   `loft_hdepth`).  None is emitted where the loop has C-only state in flight — a write window,
   a held file cursor, a range copy — nor in a function split into parts.
2. **The offer.**  Each declared local the unit holds as a plain value is offered by name and
   kind (`loft_ho_put`: integer, float, text, reference, boolean or enum, single); a sticky sum
   is offered as its checked value.  `loft_ho_commit(key, scope)` asks the host
   (`HostedTable::handover`).
3. **The verdict** (`State::accept_handover`, `src/state/handover.rs`).  The routine must be the
   one the static call entered (`bridge_call_d`), its loop must have a head in the interpreter's
   code, and every local `frame_view` shows held at that pc must be among the offer in a shape
   its slot takes; a store the routine holds that the frame does not refuses too (no one would
   free it).  A refusal latches `loft_ho_off` for the rest of the bridge call, and
   `LOFT_TRACE_HANDOVER=1` says why.
4. **The unwind.**  On acceptance the routine returns at once, without its own releases (the
   frame owns its stores now); the dispatcher answers nothing; `static_call` calls
   `resume_handover`, which enters the routine's frame as `fn_call` does, at the loop head's pc,
   with the eval stack at that pc's depth and each slot written.  The main loop carries on —
   paused there.

**Known refusals.**  A local only the interpreter keeps (a hoisted length such as `_len_14`)
has no counterpart in the unit, so a loop that hoists one refuses; so does a loop the
interpreter rotated with a seed (no head recorded).  A refused routine finishes compiled, and the
pause stops at the next interpreted statement after it.
