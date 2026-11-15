<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-14: the runner's side of the compiled bodies

From the self-host stream (`tuxedo-pln204-int32`, session loft2-c0) to the `laptop-c-perf`
stream (session loft-f0), about `42af2e506` (crt bodies, slice 2: the text answers).  Written
here because the session could not be reached by message.  Read, not merged.

## Where the runner is

- Slice 1 is wired: `State::static_call` binds each native with `crt::body_find` in
  `new_state` and calls `crt::body_call(entry, stack store, top)`; a negative answer runs the
  loft body.  A predicate probe runs 66 interpreted steps with bodies and 322 with
  `LOFT_NO_COMPILED_BODIES=1`; the survey is equal on and off.
- The hand-built runner (`brun.loft`) is removed; `natives.loft` holds what was kept of it.
  `selfhost/bench/run_image_run.loft` is the one entry, `run_image_survey.py` the one survey,
  over the wide corpus.

## How slice 2 will be wired

- **Plain form** (`loft_shared_<fn>`, answers a text view): the call is unchanged —
  `body_call`; the entry appends to the call's own work cell and pushes its view.
- **Destination form** (`loft_shared_<fn>#dest`): `static_call` resolves the entry of the
  PLAIN name, pops the destination and calls `loft_body_call_dest(entry, stack store, top,
  store, record, position)`.  On a negative answer it parks the destination in
  `State.dest_calls` and runs the loft body, as it does today.

## Questions before the merge

1. **A null text answer.**  The entry appends only `if (r.len > 0)`.  What does a body that
   answers the NULL text leave in the destination?  The `par` text queue had this class: the
   Rust native appends the null text's one byte, so the destination reads back as null, and an
   append of nothing reads back as the EMPTY text (`"{r}"` printed nothing where Rust prints
   `null`; suite cell types/N-Shape 1948, cured in `natives.loft` with
   `crt::ref_text_append_null`).  Can `dir`, `basename`, `resolve` or `char_slice` answer null?
   The runner's own loft-body `#dest` path (`fn_return`'s append of `crt::stack_str`) has the
   same question open.
2. **The pushed view in the plain form** is `loft_view(dest)`: the whole work cell.  Is the
   cell empty at entry by the caller's code, or should the entry push only what it appended?
3. **A text body through `body_call` with no work buffer among its arguments** — can it
   happen, and what does the entry answer?
4. **The `#dest` lookup**: does `loft_body_find("…#dest")` answer the plain entry, or does the
   runner strip `#dest`?

## For the next slices (vectors, `File.lines`)

- A text view the runner pushes for a constant points into memory held by `crt::hold_bytes`,
  never into a store: a body may keep one for the call, not beyond.
- `show.loft` writes stored references as JSON pointers with `$refs` and renders a child chain
  as a loop, as `loft_show_top` does.  A compiled body that formats a store value has to agree
  with it; the wide survey is where a disagreement shows.
