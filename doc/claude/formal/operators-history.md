<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/operators-history.md — the deviation register for [operators.md](operators.md)

> **The rules are next door.**  [operators.md](operators.md) states what must always be true of
> an operator on a user type; this file is its TIMELINE — every place the code was measured not
> to do it, when, and what closed it.  The rules doc carries the CURRENT state.

OPEN: **0**.

- **D-opr-1** *(opened 2026-10-05, CLOSED 2026-10-05; the 2026.10 review of `time` and
  `server`)* — `(Op-Left)` for a type declared in a library reached through a bare `use lib;`
  (@C98: the `lib::` qualifier only).  Each operator site asked the caller's scope for the
  operator by name, and a qualified import binds none, so every form on such a value was
  refused — with advice to declare an operator the library already declared — while
  `x.method()` answered through the type's attribute table: `d1 < d2` and `d2 - d1` refused,
  `"{span}"` printed the raw record, `for req in srv` refused (`cannot iterate over Server`),
  and an OVERLOADED member failed in both spellings, `d.minus(span)` reaching the first member
  alone.  Both published READMEs (`time` 0.4.0, `server` 0.7.3) used the bare `use`.  Closed by
  `Data::import_operators` — every `use` binds the operator definitions its library publishes,
  under their per-type keys — and `Data::receiver_overload_source`, which chooses a receiver's
  overload set in the source that declares the receiver's type.  Guard
  `tests/scripts/a-library-type-answers-its-operators-through-a-bare-use.loft`.
