<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-16 — rebased on main; gate E's differs after the move, and loft#2515

Branch `tuxedo-pln204-int32`, rebased on `origin/main` d191761c2 (#2513).  Main held this branch
up to 64361031e by content; only the four later commits were replayed.  Goal (owner, /goal): the
full loft interpreter, the in-between data of the two implementations compared at every layer.

## Where it stands

- Main moved the suite pin to 508580956 (`tests/loft-suite.pin`).  **Every survey baseline was
  written at the old pin** (`selfhost/front/elab_gate.baseline.tsv`, `selfhost/bench/*.baseline.tsv`)
  and fails `require_pin` until re-written: gate E first (it needs 0 differ), then the writer and
  runner surveys, one heavy job at a time.
- Gate E at the new pin, before the uncommitted ports below: 733 of 2125 agree, 12 differ.  The
  differs were new Rust behaviour from main plus programs a cleared refusal newly exposed.
- With the ports below: **741 of 2125 agree, 1381 refused, 3 differ** (verdict files diffed run
  against run: every moved program is one a port named).

## Ported this session (Rust function → loft)

- `literal_exits_into_buffer` (committed, 601cddf7c); skipped for a `null` tail, as Rust's
  delivery arms are never reached there (uncommitted).
- `coalesce_with` mints `__ncc_N` for a subject of unknown type on pass 1 (committed).
- Pattern groups `(2 | 3)`, `null` arms, a nullable scalar subject peeled, text struct bindings,
  root captures `w: V { … }` (`root_capture`, with the variant proved for the arm body).
- Closure cells (`el_cell.loft`: `cell_struct_name`, `synthesize_cell_structs`,
  `flip_scalars_to_box_types`, `auto_deref_boxed_scalar`, `maybe_prepend_cell_alloc`); a map of
  the Rust sites is in the session scratchpad only — re-derive from `vectors.rs` 3019–3200 and
  9481–9600 if needed.
- loft#2499: a `match` that is `never` beside a `void` arm is `void`; the sibling's null goes
  behind ANY then arm typed `never` (both arms never included); DN1's `if` widening by
  `branch_yields_null`.
- loft#2481: an `ncr` block over a loft call answering a record drops the call's borrows, on
  both passes.
- A record item that is no literal in a vector FIELD reuses `record_item_copy`.
- `@FR-O-Detach` for a parameter rebind reads a VIEW of the parameter too (`views_of`); a
  literal reading one is built apart.
- A local declared as a variant (`ice: KIce = KIce { … }`) is built in place.
- A dense record `if` keeps a user-named local's dep the arm binds (`{ a: W = …; a }` borrows
  `a`); the minted-store filter drops only compiler work refs.  Making the `if` filter nothing
  at all was measured: 723 agree, 22 differ — the filter is right for literal arms.
- A variant literal rebuilding a local of its enum is built in place (the `(H-Drop)` build-apart
  asks the literal's own record only); a local declared as a variant is built in place.
- `@FR-B-Scope`'s split (loft#1700, `rebind_after_block`): the open blocks and each binding's
  block on `Vars`; a bind after the name's block ended, at a type the old binding refuses or that
  differs from it only in absence, is `name#<line * 100000 + column>`; a bind in ANOTHER loop is
  left to the per-loop split.
- `branch_yields_null` as Rust's: a bare `null` counts on the second pass only, a forward call's
  block and a value behind a halt answer nothing.

## Open, in order

1. The three differs: `139-drop-cascade`, now at `c14` — Rust parses a vector field's items
   before it builds each element, so the items' call buffers (`__ref_3`, `__ref_4`) number before
   the copies' `__ref_p2_4`/`_5`; the port interleaves; F-ParamRebind (an artifact of Rust's abandoned in-place attempt,
   `__vdb_1`); `a-lifted-call-result-adopts-
   like-a-named-bind` — `mkcv`'s `return v` (a view of a local) keeps `__retbuf` in Rust, where
   the port's views-local path renames the buffer onto a work ref.  Copying into `__retbuf` on
   that path instead was MEASURED and broke four agreeing programs (H-Move 1628 and 2004, N-Road,
   `a-transient-twin-writes-only-what-it-built`): the rename is right for them, so what decides
   `mkcv` is another clause of `classify_ret_promotion_inner` — read it before porting.  Also a
   text return through a call keeps the call's borrow (`text["z"]`) on the body block.
2. Refused by name now, to port: the fit fusion (`E-Uncomp-Seen`, `src/parser/fit.rs`, @PLN152
   step 5 — the refusal also reads `!place` inside `&&` / `||`), and loft#1145's per-loop split
   (`body_local_binding`, `name#bN`; `157-record-ptr`).  `2177-a-malformed-image-…` registers
   `vector<u8>` (known 100) ahead of the program's structs in Rust and after them in the port.
3. Re-write the baselines at the new pin once gate E has no differ.
4. loft#2515 (filed, compiler half fixed in 72a943412, guard cell loft-suite 728fcfd): the
   static checker still MISSES it with `LOFT_NO_RET_TYPE_MATCH=1` (`own_sweep.py`: uaf,re-free).
   `src/store_records.rs` is 6545 lines, over the 5000-line ceiling, so the check waits on that
   file's split.  The cell's `@falsified-at` receipt is pending (`make falsify`).
5. CI on the branch (run 43775747754): the scripts ratchet is fixed (bc2257796 — its message
   says 23 scripts; the count is 72); `html_wasm rust_writer::html_debug_tier_runs_on_wasm`
   took 349.5 s against the 300 s budget.  Not caused by this branch's changes as far as read;
   time it locally on a quiet box and find where the time goes.

## Method notes

- One program against the kept tools: `elab_gate.one(loft, defs, elab, std, path, tmp, env, i)`
  with `LOFT_ELAB_KEEP=1`; the oracle's report is `defs <image> program <std>`.
- A loft panic's frame list needs `loft --interpret`; bare `loft` compiles through rustc first
  (20+ minutes for the elaborator).
- A gate lists its corpus before it builds: never add or delete a file in the pinned suite
  worktree during a run.  New cells go in `../loft-suite-91` (branch `91-self-host`).
- The definitions table emptied mid-elaboration was loft#2515 in the compiled elaborator itself
  (`value_of` reads `def(e, cd).attributes[0]!.typedef[0]`): a store fault in the port is a
  language-defect candidate before it is a port defect.
