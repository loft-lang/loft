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

## Closed after the first write of this handoff

- **D-fmt-7**: a long chain reads back from text (a parse deeper than 2 000 levels runs on a
  sized-stack thread); C compares, copies, releases and renders one as a loop.
- **S5** keys were already refused; **S6** binary `f#write`/`f#read` of a tree is refused by
  name; **S3** the strict read checks a child; **S7** an added or removed child field upgrades
  a stored type, and `par` and store-image cells are in the suite.
- **Step 8**: the debugger edits through a child (`t.left.key = 9`); the LSP shows a child
  field as its declared type.
- `LOFT.md` split: the Operators subject is `LOFT_OPERATORS.md`.
- **The full run**: `find_problems.sh --full`, then the 25 binaries that failed under contention
  rerun alone on this branch and on its start commit, both with `LOFT_NATIVE_KEEP_SYMBOLS=1`.
  Nothing fails here that does not fail there: `lib_process` (`spawns_sh` twin case) fails on
  both, and `native_source_key` / `p254_cache_poisoning`'s symbol test fail on the toolchain —
  `-Cstrip=symbols` runs `rust-objcopy`, which cannot load `libLLVM.dylib` on this box (it is
  in the toolchain's `lib/`, not `lib/rustlib/<host>/lib/`), so no binary is stripped and the
  warning lands in compared output.  Rerun those two where the toolchain works before merge.

## Known, not this branch's

- A guard on a FIELD path does not narrow it (`if t.left { f(t.left) }` warns); a local does.
  The same holds for `reference<T>?` fields.
- A prepend (`h = List { v: i, next: h }`) copies the list: the move of a fresh same-store
  value is the planned cure (CAVEATS.md).

## Next

E-1 is built (00c234374, D-heap-52), and the gate reds the branch carried are repaired
(7f974619a: clippy on rustc 1.99, the `@FR-N-Shape` ratchet, the `time` census rows).  Next:
loft#2260, then E-4 and F-1; R-1 / R-3 remain designs for the owner.
