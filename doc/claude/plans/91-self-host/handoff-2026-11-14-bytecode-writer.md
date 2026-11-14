<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-14 — the bytecode writer in loft, and what is open before an interpreter purely in loft

Branch `tuxedo-pln204-int32`, pushed.  Not rebased on main `5330b3690` (PR #2482 merged).
The register and build order: [rust-dependencies.md](rust-dependencies.md).  Owner's rules
for this arc: the new interpreter must not depend on Rust code; work on it goes in loft or
C, never a `.rs` file; functionality before speed, speed after compatibility.

## What exists

| Piece | Files | Measured by | Result at this handoff |
|---|---|---|---|
| The runner (all 366 operators) | `selfhost/src/state*.loft`, `fill.loft` (generated), `fill_run.loft`, `natives.loft`, `selfhost/crt/` | `python3 selfhost/bench/run_image_survey.py` | 1186 of 1194 agree, 1 stores, 6 stopped, 1 fault, 0 differ |
| The bytecode writer (step 1) | `bgen.loft`, `bgen_set.loft`, `bgen_loop.loft`, `bgen_tuple.loft`, `bgen_text.loft` | `python3 selfhost/bench/bgen_survey.py` | 8506 of 8611 functions byte-identical, 104 not written, 1 differs |
| The image's code stream and tables (step 2, begun) | `bgen_image.loft`, `native_table.loft` | the same survey's second line; one program: `bgen_check --stream` | 1139 of 1194 whole streams identical; operator, shared-body and entry tables equal for every program |
| The runner on loft-written code | `run_image_run.loft` with `LOFT_RUN_PROGRAM=<program image>` | `run_image_survey.py --written` | 1135 of 1194 agree, 0 differ, 53 unwritten |
| Shared decisions | `binds.loft`, `own.loft`, `frame.loft`, `ret_adopt.loft`, `char_walk.loft` | the byte gate | — |
| Speed | — | `selfhost/bench/fill_speed.sh` | fib 1.68×, mix 1.32×, collatz 1.23× of `loft --interpret` (box not quiet) |

The scratch helpers of the session are not in the tree: `dd.sh <program> <function>` (emit
both images under `LOFT_NO_TEXT_BORROW=1`, dump one function's bytes beside the image's) is
`bgen_check <store> <run> --dump <function>` after two `loft --emit-image` /
`--emit-run-image` calls.

## Open work before an interpreter purely in loft

In build order ([rust-dependencies.md](rust-dependencies.md) § the build order).

**Step 1 — the bytecode writer.**
1. The 104 functions not written (the survey lists them): narrow integers held in a link's
   or a tuple's slot (`NarrowSlot`, `tuple_links::member_slot`); a yield that hands a record
   or function value over (`yield_handed_temps`) and a generator of tuples; a tuple member
   kind `put_member` does not store; `OpCreateStack` of a tuple member; a child record moved
   within its slot (`slot_self_step`); a call through a function value with a text-link
   parameter (`store_text_mask`); the functions with an empty body.
2. The text-view rewrite (`mark_text_views`: `opt::text_borrow`, `opt::store_text_borrow`,
   `opt::hoist`'s borrowed temps).  Until it is ported the gate compares under
   `LOFT_NO_TEXT_BORROW=1`.  loft-f0 takes this port (after `opt::range`); the writer then
   calls it.
3. The one differing function: a text constant holding a NUL byte reaches the writer empty —
   the image reader ends a text at its first NUL (also the runner's one fault).
4. What the writer does not emit at all, because the runner does not read it yet: the line of
   each position, the variable and type side tables (`State::line_numbers`, `vars`, `types`,
   `store_spans`) — the debug data `stack_trace()` needs (step 2).

**Step 2 — the image in memory.**  Done in loft: code, starts, natives (plain programs),
operators and layouts, shared bodies, main.  Open, each a port of one `ir_store::run_image_*`
function compared with the image's table in `bgen_check --stream`:
5. `consts` (the constant vectors' stores and the long texts: `build_const_vectors`).
6. `fnrefs` (`fnref_visible_params`, `fnref_text_buffers`, the drop cascade).
7. `gens` (`gen_facts`: zone sizes, text and owned locals, tuple reference offsets).
8. `workers` (`par` worker shapes and tuple row plans) and `store_insts`.
9. `sidecars` (persisted root types: layout identity, paged, box and range load plans).
10. `types` (the store schema — today the program image's), `oldest` / `bridge` / `source` / `cwd`.
11. The operator NUMBERS themselves (`Data::op_code`'s rule) and the natives of a program
    with `#c` functions of its own (one corpus program has 236 natives, the table holds 232).
12. The debug data of item 4, and `stack_trace()` answered from it.

**Step 3 — the loft front end's IR.**
13. The writer is fed `--emit-image`'s IR.  The loft lexer, parser, scopes, slot assignment
    and IR passes (`frontend.loft`, `el*.loft`) must produce that IR — strand 1a in
    [README.md](README.md).  The elaborator did not even parse with this branch's release
    binary when the writer once imported it (`el_ex_null.loft:915`), so its state on this
    branch is unknown and is the first thing to measure (`selfhost/compare/`).
14. The facts the image carries for the writer that the front end must then produce: each
    variable's slot and its flags (`stack_allocated`, `skip_free`, `lazy_buffer`,
    `view_elided`, `linked_narrow`, witnesses), `no_prefill`, `width32`, `known_type`,
    `op_code`.

**Step 4 — one program: source in, run.**
15. `selfhost/front/loft.loft`: source → front end → `bgen_image` → runner, no image file.
16. Remove the older IR interpreter (`run.loft`, `lower.loft`).  `bgen.loft` still calls
    `lower::main_vector_of` and `lower::hash_type_of`; they move to a neutral file first.

**Step 5 — what the runner does not carry.**
17. Reflection (`type_named`), a program's own `#c` function called by symbol
    (`c_labs_bool`), `stack_trace` (item 12), the paged load by a key holding a NUL.

**Step 6 — the C generator in loft** (loft-f0, strand 12): the runner's own sources compiled
to C by loft, then the driver that calls the C compiler.

**Step 7 — the harness and the oracle** as loft programs.

**Speed** (after compatibility): the runner is 1.2–1.7× of `loft --interpret` on the three
kernels; the cure is in the emitted C and in the generator of `fill.loft`.

## Agreed with the other sessions

- **loft-f0** (C generator): shared code, one port per decision, verified against Rust, the
  other side calls it.  Mine: `binds`, `own`, the bind arms, `frame`, `native_table`, the
  lowering-side rewrite decisions, the small IR helpers.  Theirs: the `opt_*` analyses
  (names, non_sentinel, iteration, range, then text_borrow / hoist), `loop_shape`, `cgen_*`.
  Promised pure moves, in order: `irx.loft` (unspan, base, peel_link, var_of, int_of,
  call_name, call_args, a pre-order walk); `rotation.loft`; `bind_arms.loft`
  (`re_bind`, `first_arm`, Gen-free); `codes.loft` (`coded`, the no-prefill twin, the stack
  variant, the text-destination predicates).  `char_walk.loft` is done.
  When merging `laptop-c-perf`: keep exactly one `body_find` / `body_call` in
  `selfhost/crt/src/crt.loft`; `Definition.width32` is on that branch (regenerate with
  `make ir-schema-regen`).
- **loft3-ac** (PRs): PR #2482 is merged; fixes for it go to `tuxedo-pr2482-next`.  loft#2488
  (a `return` inside a match arm beside a wildcard arm left a view's copy unowned) is fixed
  there at `a81457a79`; once this branch holds it, `bgen.loft`'s `NdFnRef` arm may answer
  `t[0] ?? ir::TyVoid` again.  Their advice: run `scripts/own_sweep.py --cell <file> --oracle
  check-records` over `selfhost/src/bgen*.loft`.
  loft#2486 (an early `return v;` of a match-bound vector) and loft#2487 (a lambda in a loop
  capturing a vector) are loft defects the writer's sources may hold: re-run both surveys
  after merging main.

## Not done, and why it matters

- `make ci`, clippy, `make wasm`, `make surface-gen` were not run on this branch's tip.
- `bgen.loft` is over 2,500 lines (limit 5,000): the first binds and the call writer are the
  next split.
- The Rust changes pushed earlier in the session (run-image long-text entry and `store_insts`
  in `src/ir_store.rs`, the `par` worker-row fix, the `LOFT_HOT_INLINE` wrapper in
  `src/c/mod.rs`) stand; whether to revert them is the owner's call.
- A library's cached parse can be read by an image emitted under another
  `LOFT_NO_TEXT_BORROW` setting: one program (`F-Surface/2331`) compared unequal once and
  equal on every re-emission.  Not run down.
