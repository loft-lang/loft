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
typed error.  Calculation faults yield null in EVERY mode (@C80); a `{…}` interpolation never raises.
**Why.** loft's targets — games, scenes, servers — must not stop; a frozen program is worse than a
wrong value, while a developer wants the loud stop.

**Revisit when.** A deployment shape surfaces where log-and-continue is wrong.  Decided
2026-05-11, calculation faults revised by @C80 on 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt).
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
in development, test and production.  An unguarded division or modulo by zero, and an index
out of range, also log ONE Warn line naming the fault (`divide_by_zero`, `index_out_of_bounds`)
wherever the operation sits — a statement, a `return`, an argument, a format-string hole — on
both backends; a site the program guards (`?? d`, a null check) logs nothing, since the program
already inspects it (owner, 2026-10-21).  A divisor that is a constant zero is a compile-time
warning (`divide-by-constant-zero`), and `--dev-soft-halt` names each fault in development.
Tests observe faults through the log.  `panic` and `assert` are not calculations:
they halt in development and tests and log-and-continue in production (@C66).  Startup failures still
stop (@C67).  **Why.** As in a spreadsheet, one bad cell never stops the others: degradation stays
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
(`@FR-H-WriteLocked`, @C80).  **Why.** Re-spelling the place copies a chain exactly where the copy
goes wrong, and re-reads instead of reporting; a store as a condition is the `=`-for-`==` typo; `||`
or `??` would give "or" a third meaning; a result wrapper is declined by @C89.

**Revisit when.** A consumer shows the Warn line is noise in a real program — and even then the
answer is a cheaper guard (`else {}`), never a mode split or a compile-time nag.  Decided
2026-09-28 — [record](DESIGN_DECISIONS-history.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment).
**Holds at:** `@C130` — `./scripts/idx tag:@C130`.
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

## C142 — A null inside a `{…}` hole renders plain `null`, never the reason it is null

**Decision.** A hole whose value is null renders `null` — the rendering `(F-Render)` gives a null
of its type — whether the null was stored, inherited or produced by a fault inside the hole
(`{a / b}` with `b = 0`, `{v[9]}`, `{5.0 % 0.0}`).  No hole kind annotates it with a cause, on
either backend, and nothing switches one on.  The fault itself is unchanged: it yields the null
value and the program continues (@C80, `(E-Uncomp)`), and formatting never traps (@C66).
**Why.** The same null rendered differently by where the fault sat — `"{a/b}"` against
`c = a / b; "{c}"` — and keeping a cause attached to a hole's null cost four defects in a row,
each a cause shown, lost or worn by the wrong hole.  The value is what a hole shows; *why* a
value is null belongs to the program's own inspection (`??`, a test) or the log.

**Revisit when.** Not stated.  Decided 2026-10-08 (owner) — [record](DESIGN_DECISIONS-history.md#c142--a-null-inside-a--hole-renders-plain-null-never-the-reason-it-is-null).
**Holds at:** `@C142` — `./scripts/idx tag:@C142`: the hole's nullable rewrite in the parser
(`src/parser/objects.rs`) and the null test before a type's own `to_text`
(`src/parser/collections.rs`), `formal/formatting.md` `(F-FaultSafe)`, guards
`loft-suite/rules/formatting/F-FaultSafe/a-fault-inside-any-hole-renders-plain-null.loft` and
`loft-suite/rules/formatting/F-FaultSafe/a-null-hole-never-calls-the-users-to-text.loft`.
**Catalogue:** @F38 (arithmetic safety) · `formal/formatting.md` `(F-FaultSafe)`, `formal/operational.md` `(E-Uncomp)` · reads @C66, @C80.

## C148 — A parsed value carries its own parse errors, wherever the whole value goes

**Decision.** `x#errors` is a `vector<text>` with one entry per value that did not fit in the
`Type.parse(…)` call (or the `text as T` cast) that built the value `x` holds — a record, a
struct-enum or a vector — whichever form built it: the one-step `Type.parse(text)` and the
staged `Type.parse(json_parse(text))` both name EVERY mismatch there, and a value that did not
fit leaves its field at the type's default while the values around it are kept (owner,
2026-10-16, loft#2028).  Reading it changes nothing.  The entries are part of that value: they go with it
when the WHOLE value is bound, rebound, passed, returned from a function or copied to another
variable, whichever way the compiler delivers it (a move, an exchange of stores, a copy).  A
value built any other way has none, and neither has a part copied out of a parsed value or a
parsed value stored into a field or an element; a VIEW of a part answers for the whole it lies
in.  A parsed vector that grows keeps its own.  Asking a variable that is neither a record nor
a vector is refused at compile time.
**Why.** One register for the whole program answered for the LAST parse, so `a#errors` reported
on `b` once `b` was parsed, and it was one text the read cleared, so the loop a programmer
writes walked characters and a second read answered nothing.  And a delivery that copies must
answer as one that moves does, or the answer would follow an optimisation.

**Revisit when.** A consumer needs the errors of a value stored inside another.  Decided 2026-10-12
(owner) — [record](DESIGN_DECISIONS-history.md#c148--a-parsed-value-carries-its-own-parse-errors-wherever-the-whole-value-goes).
**Holds at:** `@C148` — `./scripts/idx tag:@C148`: the store's content (`Store::parse_errors`,
emptied by `Store::init`, exchanged by `Store::swap_contents`), `Stores::deliver_parse_errors`
and its vector form at the whole-value writes, the accessor's lowering
(`src/parser/objects.rs`), guards
`loft-suite/attic/scripts/a-record-errors-read-answers-for-its-own-parse.loft` and
`loft-suite/attic/scripts/a-vector-errors-read-answers-for-its-own-parse.loft`.
**Catalogue:** reads C80 (a value read never raises: the errors are asked, not thrown).

## C155 — A tail self-call the frame cannot jump is refused, never left a call

**Decision.** A self-call in tail position is a jump that reuses the frame (`(R-TailSelf)`,
[formal/rewrites.md](formal/rewrites.md)).  Where the frame cannot be reused — an argument that
makes a fresh value or names a local for a store-held parameter, a text or `&` parameter handed
anything but itself, a parameter the body rebinds, a `&` local, a result delivered into the
caller's buffer, a generator, a `parallel` block or a closure in the body — the program is
refused at the call, naming the variable and the cure
(write the recursion as a loop, or bind the call's value to a local before returning it when a
call is meant).
The call is never left standing to stop at `MAX_CALL_DEPTH`.  **Why.** A recursion that works
for small inputs and overflows at ten thousand frames is a wrong answer that arrives late, and
a compile-time refusal is the one-way door loft may still add before the freeze
([COMPATIBILITY.md § The error surface is one-directional](COMPATIBILITY.md)); dropping it
later, as the heap clause lands shape by shape, costs nothing.  The refusal and the pass share
one predicate (`tail_frame::admit_frame`), so what compiles is exactly what jumps.
**Holds at:** `@C155` — the refusal in `src/parser/definitions.rs` (pass two, beside the
per-function lints), `admit_frame` and `declared_params` in `src/tail_frame.rs`; guards:
`tests/error_messages/cases/63_a_tail_self_call_the_frame_cannot_jump_is_refused.loft`, and in
loft-suite `rules/rewrites/R-TailSelf/a-tail-self-call-the-frame-cannot-jump-is-refused.loft`.
**Catalogue:** `formal/rewrites.md` `(R-TailSelf)` · `formal/calls.md` `(F-ParamHeap)`,
`(F-ParamRebind)` · COMPATIBILITY.md § The error surface is one-directional
