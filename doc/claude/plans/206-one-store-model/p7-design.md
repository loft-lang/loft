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
4. **The pause point** (P7b, P7c): the unit's statement sites (`loft_frame_site`), where the
   unit's state and the interpreter's line table meet.

## Failure paths

- A breakpoint in a routine reached only through compiled calls never fires (site 2's clause).
- A routed call whose argument layout differs from an ordinary call's reads its parameters
  shifted (site 1): guarded by stopping inside a routed routine and reading every parameter.
- A routine the session interprets because its library did not build is debugged as before:
  routing is a no-op for a function that is not compiled.
- A generator: a static call to a compiled generator answers a handle; routing it would need the
  interpreter's coroutine creation, so a generator is not routed (its body is stepped as compiled
  until P7c).
- Mid-routine (P7c): a local the unit holds in a shape the hand-over cannot write (a tuple, a
  text borrowed from the unit's own memory) makes that site refuse; the routine runs on to the
  next site that can hand over, or to its return.

## Phases

| Phase | What | Red when |
|---|---|---|
| **P7a** | the session adopts units; a call the debugger wants runs interpreted; its callees stay compiled | a breakpoint inside a compiled library function does not stop, a parameter reads wrong there, or a callee the routine makes does not run compiled |
| **P7b** | a pause while a compiled routine runs stops at the unit's next statement site and shows where it is: the compiled frames with their lines, marked compiled | a pause during a long compiled loop does not stop, or names the wrong line |
| **P7c** | the stopped routine continues in the interpreter: its locals written into the interpreter's frame by variable, its statement's bytecode position entered, its answer returned to its compiled caller | a routine continued this way answers differently from the same routine run whole, or leaks or double-frees a store |

P7a is complete on its own: everything a plain run gets from the units, the debugger gets, and
every stop the interpreter-only debugger made it still makes.  P7b and P7c are the "interrupt a
running C program" half.
