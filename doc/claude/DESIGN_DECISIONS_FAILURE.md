<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — failure

The entries of the [decision register](DESIGN_DECISIONS.md) about what a running program does when something cannot produce a value, and what stops it.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

## C66 — Production loft programs never abort on user-attributable edge cases (development may halt)

**Decision.** A production run (`--production`, the logger's `production` flag) never aborts:
`panic` and a failed `assert` log, set `had_fatal` and continue; in development they halt with a
typed error.  Calculation faults yield null in EVERY mode (C80); a `{…}` interpolation never raises.
**Why.** loft's targets — games, scenes, servers — must not stop; a frozen program is worse than a
wrong value, while a developer wants the loud stop.

**Revisit when.** A deployment shape surfaces where log-and-continue is wrong.  Decided
2026-05-11, calculation faults revised by C80 on 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt).
**Holds at:** `@C66` — `./scripts/idx tag:@C66`: production continues on both backends
(`tests/panic_halts_both_backends.rs`), the locked-store write too (`tests/locked_writes.rs`).
**Catalogue:** @F38 (arithmetic safety), @F44 (logging — panic/assert).

## C67 — Fail at startup, not at runtime (no programmer-side try/catch for internal bugs)

**Decision.** No `try`/`catch`, no `#panic_safe`, no defensive boilerplate for internal bugs: an
internal bug is refused at compile time (no reachable `todo!()`), a startup fault exits non-zero for
the supervisor, and a steady-state external fault (network, disk) is handled inside the LIBRARY
behind a clean API.  User-domain errors as values stay allowed.  **Why.** Wrap-everything burdens
every programmer and hides crashes from the supervisor.

**Revisit when.** A class of internal bug escapes compile-time analysis, or typed errors grow
more boilerplate than a catch would — the typed-error mechanism may then evolve; try/catch for
internal-bug recovery stays closed.  Decided 2026-05-13 — [record](DESIGN_DECISIONS-history.md#c67--fail-at-startup-not-at-runtime-no-programmer-side-trycatch-for-internal-bugs).
**Holds at:** `@C67` — `./scripts/idx tag:@C67`: `try`/`catch` refused by name; a reachable
unimplemented native refused at compile time and at startup (`tests/exit_codes.rs`).
**Catalogue:** @F44 (logging — panic/assert).

## C80 — The spreadsheet fault model: nothing stops a running calculation

**Decision.** A calculation that cannot produce a value — division by zero, integer overflow, an
out-of-bounds index, a deref of an absent value — yields null and execution continues, identically
in development, test and production.  It is silent except for one warning on an unguarded division
by zero; tests observe faults through the debug log.  `panic` and `assert` are not calculations:
they halt in development and tests and log-and-continue in production (C66).  Startup failures still
stop (C67).  **Why.** As in a spreadsheet, one bad cell never stops the others: degradation stays
local, with no unwinding or cleanup blocks to get wrong.

**No runtime exceptions — ever.**  Nothing a running program does stops it: no new runtime error
kind, no check that halts, and any found is removed.  The only halts — `panic`, `assert`, the
call-depth limit and a write to a locked store (`@FR-H-WriteLocked`) — happen in development and
test builds only; in production the locked write is logged and discarded, and the depth limit
reports the recursion fully ([@PLN177](plans/177-growable-stack.md); until it lands,
[CAVEATS.md](CAVEATS.md#recursion-depth-is-capped-and-the-cap-halts-the-run)).  A fix uses
instead a compile-time refusal, a compile-time warning that a construction needs inspecting, the
programmer's OWN optional inspection of one expression (`?? fallback`, a checked result), or null
with a log line: every fault answers a value the programmer MAY inspect on that expression.

**Revisit when.** Silent null-and-continue loses critical information for a consumer — and even
then the fix is more observability, never a halt.  Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation).
**Holds at:** [formal/operational.md](formal/operational.md) @FR-E-Uncomp, @FR-E-Report.
**Catalogue:** @F38 (arithmetic safety), @F1 (null model), @F44 (logging — panic/assert).

## C130 — A store carries its own failure arm: else after the assignment

**Decision.** An assignment statement may end in `else { … }`, run exactly when the store did not
take: an index out of range, a null index, a write through `nullref` at any step of the chain, a
narrow slot the value did not fit, a lock fault.  The place is spelled once; the store stays a
statement (a store in condition position is refused) and has no success arm.  An unguarded dropped
write logs one Warn line in every build kind — that log is the default resolution, and a missing
`else` is never diagnosed.  `else` supersedes `if !place`, which stays accepted.  **A lock fault is
not a dropped write:** writing a locked store is a program error, not a missing element, so
unguarded it halts a DEVELOPMENT run with the lock report, and a production run logs and discards it
(`@FR-H-WriteLocked`, C80).  **Why.** Re-spelling the place copies a chain exactly where the copy
goes wrong, and re-reads instead of reporting; a store as a condition is the `=`-for-`==` typo; `||`
or `??` would give "or" a third meaning; a result wrapper is declined by C89.

**Revisit when.** A consumer shows the Warn line is noise in a real program — and even then the
answer is a cheaper guard (`else {}`), never a mode split or a compile-time nag.  Decided
2026-09-28 — [record](DESIGN_DECISIONS-history.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment).
**Catalogue:** @F1 (null model), @F38 (arithmetic safety) · [@PLN178](plans/178-store-else.md) · `formal/heap.md` `(H-WriteOOB)` / `(H-WriteNull)` / `(H-WriteLocked)`, `formal/operational.md` `(E-Uncomp-Seen)` / `(E-Report)`.

## C131 — A cast to a variant answers the variant: a provable miss is refused, an unproven one defaults and warns

**Decision.** `s as V` casts an enum value to one of its variants.  A miss the compiler can prove —
the value is known to be another variant — is a compile error.  Otherwise the result is a non-null
`V`: on a miss, `V` with every field at its default (`(D-Rec)`), never null; a variant with no
default refuses the cast.  An unchecked cast WARNS (`variant-cast-default`), as an unguarded
division does, unless it is checked directly — inside `if s is V { … }` — or spelled `s as V?`, the
checked form, which answers null on a miss.  A text parsed `as E` for a plain enum follows the
same rule: a literal naming no variant is refused, and a runtime text naming none answers E's
first-declared variant (`(D-Enum)`) and warns (`enum-parse-default`) unless written `as E?` or
followed by `?? <variant>`.  **Why.** A cast that can answer null while typed
non-null lied to every check after it (`!c` was reported "always false" and printed true); a
default keeps the promised type, and the warning says where the default can be taken.

**Revisit when.** A program needs the other variants' data at the cast site — the answer is `match`
or `is`, not a wider cast.  Decided 2026-09-29 (owner) — [record](DESIGN_DECISIONS-history.md#c131--a-cast-to-a-variant-answers-the-variant-a-provable-miss-is-refused-an-unproven-one-defaults-and-warns).
**Holds at:** `@C131` — `./scripts/idx tag:@C131`.
**Catalogue:** @F5 (checked cast), @F30 (is variant check) · `formal/types.md` `(N-Cast)`, `(D-Rec)`.
