# @PLN91 parity — the contract fact `buffer_exits` (prepared subject)

**Why first:** the owner's priority is contract, state and reasons to PARITY.  `buffer_exits` is
the one contract fact the port does not derive at all: `not derived yet: buffer_exits` on every
agreeing program (1404 at b9b0b1052).  `selfhost/src/el_contract.loft` prints `?` for it, and
the gate counts that apart, never as a guessed value.

**What it says** (src/parser/contract.rs ~26): for each program function with a hidden return
buffer,
- `fills`: every exit answers the buffer;
- `exits`: some exit answers a store the caller never owns (a global constant, a parameter, a
  field of either), which the call site copies in after the call;
- `-`: the callee may not be placed.

This is `(R-Place)`'s callee clause, "the buffer IS the place": a caller that ignores the answer
loses nothing exactly when the callee fills the buffer it is handed.

## The Rust routines, and the port's counterparts to write

Each routine is one counterpart, under the corresponding-routine ruling.  They run over the
FINISHED definition (after both passes), as `contract_report` does: the definition's code, its
variable table and its hidden return-buffer attribute.

| Rust (src/parser/expressions.rs) | what it decides |
|---|---|
| `callee_buffer_exits(d_nr, active) -> Option<bool>` (~3494) | `None` if the callee is on the `active` stack (a cycle declines), or if it mints into an ARGUMENT (`OpDatabase(<argument>, tp)` where `tp` is no vector wrapper, `Database::is_vector_wrapper`), or if it has no hidden buffer or no `Block` body.  Otherwise it collects every `Set(v, val)` into `assigns[v]`, and every `Return(e)` plus the body's last operator (when that is no `Return`) into `exits`.  It pushes `d_nr` on `active` and asks `BufferExits::exit_kind` of each exit.  `placed` holds while every exit has a kind; `foreign` is any exit's `true`.  The answer is `Some(foreign)` when placed and (not foreign, or `LOFT_NO_PLACE_FOREIGN_EXIT` unset), else `None` |
| `callee_fills_its_buffer(d_nr, active)` (~3484) | `callee_buffer_exits(…) == Some(false)`, the strict form a CHAIN exit asks |
| `BufferExits::exit_kind(v) -> Option<bool>` (~14230) | `Some(false)` if `answers(v)`.  Else by shape: `Return(e)` → `exit_kind(e)`; `Block`/`Insert` → its last item's kind; `If(_, a, b)` → both kinds required, ORed; anything else → `Some(true)` if `foreign(v)`, else `None` |
| `BufferExits::answers(v)` | `Var(buf)`; a local ALL of whose assignments answer (guarded by `seen`, a cycle answers false); `Return`/`Block`/`Insert` by their tail; `If` both arms; `OpGetField(x, 0, …)` where `x` answers; a CHAIN call `n_*(…, x)` whose LAST argument answers AND whose callee `callee_fills_its_buffer` (on the same `active` stack) |
| `BufferExits::foreign(v)` | `Var(buf)` false; an argument true; a local all of whose assignments are foreign (`seen`-guarded); `OpConstRef` true; `OpGetField(x, …)` where `x` is foreign |

## What to code

Port the four routines into `el_contract.loft` as one counterpart each, under Rust's names, and
answer `buffer_exits` from them instead of `?`.
- The predicate `is_vector_wrapper(tp)` on the store schema needs its own counterpart; find
  the port's schema routine that already answers it, or port `Database::is_vector_wrapper`.
- Read `LOFT_NO_PLACE_FOREIGN_EXIT` as Rust does (`keys::place_foreign_exit_enabled`).
- Keep the walk order: `any_node` visits `Set` and `Return` in IR order, and the `seen` and
  `active` stacks make it cycle-safe.

Done means `not derived yet: buffer_exits` disappears from the gate's contract line, and the
fact agrees on every agreeing program (the contract count is then over one more fact).

## Probes

Any agreeing program with a record- or vector-returning function, by the gate's own contract
line.  The tester's contract dump (kr-tools `cr-<tip>/contract.tsv`, a wrapper over
`elab_gate.py`'s `contract_compare`) lists each function whose fact differs, once the port
derives it.
