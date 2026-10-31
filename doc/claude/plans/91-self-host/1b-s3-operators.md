<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the store operators the census ranked, one by one

A child document of [1b-s3.md](1b-s3.md).  Each operator below is carried through the C
runtime's own routine (the shim marshals, `crt` wraps, `lower.loft` lowers, `run.loft` runs),
guarded by a kernel with values by hand, and measured: the census before and after, and the
strict comparison over every program the census passes (out, alloc, leak, binds, slots).

| operator | how | guard |
|---|---|---|
| `OpAppendCopy` (`[x; n]`) | `loft_vec_append_copy` on the element type; a linked element refused | `kernels/repeats.loft` `rep` |
| `OpVectorIsNull` | `crt::vec_absent` — an empty vector is present | `repeats.loft` `absent` |
| `OpDistinctStore` | the two references' store numbers differ | the census |
| `OpSliceView`, `OpSliceVector` | `loft_vec_slice_view` / `loft_vec_slice`, scalar elements only | `repeats.loft` `slices` |
| `OpAddIntNullable` … `OpMulIntNullable` | `Add`, `Sub`, `Mul`: the same values, and the C runtime reports no overflow for either twin | `kernels/small_ops.loft` `ops_or` |
| `OpClearKeyed` | `loft_remove_claims` of any type; a linked group's view `loft_remove_claims_mode` | `kernels/hashes.loft` `keyed` |
| `OpReplaceKeyed` | `loft_replace_keyed`, its swap bit from `types::swap_safe` | `hashes.loft` `keyed` |
| `OpCastTextFromEnum` | the enum's names as the function's text constants (`EnumName`) | `small_ops.loft` `enum_named` |
| `OpTextCharacterNullable`, `OpLengthCharacter` | the character holding a byte; its UTF-8 width | `small_ops.loft` `text_walk` |
| `OpPlaceRecord` (`@FR-R-Place`) | `loft_place_record` with the type's size and default image; `foreign` as the C backend decides it, from the host variable's root type | `kernels/places.loft` `moved` |
| `OpMoveVector`, `OpMoveField`, `OpMoveRecord`, `OpReplaceVector`, `OpFreeRecordIn` | the runtime's own move, replace and release routines; a linked element type takes the `_array` twin | `places.loft` `moved`; the census |
| `OpTextWalkStep` (`@FR-R-CharWalk`) | the bytecode writer's fused text-walk step: `char_walk` matches the parser's `for c in T` lowering per loop, the loop variable's set is one `TextWalkStep` (`walk-step` in the bind table) | `small_ops.loft` `text_walk`; the binds layer |
| `Value::Insert` as a value | its items statements, the last the value (`NdInsert` in `expr` and `kind_of`) | O-Move/1884 |
| native `n_vector_sum_int` | `crt::vector_sum_int`, the C runtime's kernel (native 2) | `repeats.loft` `summed` |
| the stack-top return `[value, frees…, return null]` | `tail_capture`, as the C writer reads it: the value taken, the frees run, then `Ret` | O-Move/1884 c6 (a wrapper forwarding a call bound after a preamble) |
| `OpConstRef` (vector constants) | `consts.loft` ports `build_const_vectors` with its readers (`extract_literal_values`, `const_eval`, `fold_text_block`): each constant's literal body built into a store of its own at load, before the trace's `M` mark as Rust builds it at compile time, then pinned and read-only (`crt::lock_constant`); `OpConstRef(d)` answers `(store, 1, 8)`, a constant the builder cannot read is refused by name | `kernels/constants.loft` `consts` |

**Measured** (2026-10-31): the census went from 297 to 459 programs passed over these rows,
none wrong, crashed or leaking, and every passing program equal to the Rust interpreter in
every layer.

**Design → code.**  Transcribed directly, with three findings fixed where they lived rather than
routed around: `loft_crt.h` lacked `loft_keyed.h`; the C backend leaked the text temporaries a
`return` inside a loop skipped (loft#2313); and the port's `char_walk` read freed memory because a
vector bound from a `match` over an owned subject was returned as a view of it (loft#2314) — both
fixed in the compiler.
