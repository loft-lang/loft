<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 1b — handoff, 2026-10-30 (night)

Branch `91-self-host`, everything committed and pushed; the suite pin is loft-suite
`91-self-host` with the new cells.  No PR is open.  The previous handoff is
[handoff-2026-10-30.md](handoff-2026-10-30.md).

## What this session built (owner: "implement our plans for the tree", then "build the ! too")

- **R-2s, out-of-line fields, on by default** (@C159; `formal/layout.md` `(L-OutOfLine)`,
  `types.md` `(T-Recursive)`, `binding.md` `(B-OutOfLine)`, `formatting.md`
  `(F-Render-Child)`).  A struct or struct-enum field whose type contains its host is a child
  record in the host's store; `LOFT_NO_OUT_OF_LINE=1` opts out.  Interpreter, `--native` and C.
  Views across a null write are copied out (`ViewCause::Released`); `&t.left` links the child.
  Copy, release, equality and rendering loop over chains (100 000 links).
- **Step 10, the port**: `selfhost/src/ir.loft`'s Node/TypeT children are children; the image
  layer (`data_store.rs`, `ir_store.rs`, `ir_read.rs`, `ir_node.rs`) reads and writes them.
- **R-2's `e!`** (@C160, `types.md` `(N-Present)`): `e ?? panic("<e> is null")`, postfix,
  glued to its operand; the formatter keeps it glued.  All 288 `?? panic` of the ports are `!`.
- User docs: LOFT.md § Recursive types and § `e!`, the Structs and Enums chapters, the
  loft-write skill, CHANGELOG, CAVEATS, DIAGNOSTICS (`redundant-present`).
- Receipts: `ir_schema_roundtrip` ok; `tests/selfhost.rs` ok; compare.py on the 13 kernels:
  compile layers EQUAL on 13, run layers EQUAL on the 8 the loft interpreter runs.

## Open, this branch's to close before it merges

1. **D-fmt-7** (`formatting.md`): the text parse of a long out-of-line chain recurses per
   level — `crate::json`'s reader, its `Parsed` drop, `walk_parsed_into`, C `loft_pw_walk`.
2. The remaining safety items of R-2s: S5 (an out-of-line field as a collection key —
   `database/search.rs:220` panics; refuse at compile time), S6 (binary `f#write`/`f#read` —
   `database/io.rs:308`, `:508` panic; refuse or write the arms), S3's strict read check,
   S7 cells (a `par` worker building a tree; an image upgrade), the debugger/LSP display.
3. A full test run on a box whose toolchain works: here `rust-objcopy` cannot find
   `libLLVM.dylib` (it is in the toolchain's `lib/`, not `lib/rustlib/<host>/lib/`), and
   every native test comparing output fails on the warning.
4. `LOFT.md` is at 997 lines: split it before adding anything.

## Known, not this branch's

- A guard on a FIELD path does not narrow it (`if t.left { f(t.left) }` warns); a local does.
  The same holds for `reference<T>?` fields.
- A prepend (`h = List { v: i, next: h }`) copies the list: the move of a fresh same-store
  value is the planned cure (CAVEATS.md).

## Next

The four items above, then R-1 / R-3 (designs, the owner's), E-1, loft#2260, F-1.
