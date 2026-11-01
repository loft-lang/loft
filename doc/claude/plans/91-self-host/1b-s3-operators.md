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
| `OpRemoveVector` | `loft_vec_remove_linked` / `loft_vec_remove_at` / `loft_vec_remove` by element type, as the C generator picks | `kernels/vectors.loft` `removed` |
| `OpSizeVector` | the length times the stride the parser filled in | `vectors.loft` `removed` |
| natives `t_4text_starts_with`, `t_4text_ends_with` | loft's own (natives 3, 4) | `small_ops.loft` `affixed` |
| a loop in value position (`@FR-R-TailSelf`'s jump), `break` / `continue` as a value | lowered as statements: the loop leaves only by `return` and lands no value (one a `break` leaves is refused), a jump typed `never` lands none either | `small_ops.loft` `tail_sum` (100 000 deep) |
| `OpHashRemove`, `OpSetKeyed` (a hash) | `loft_hash_remove` (a view member only unlinks), `loft_set_keyed` with the key parts from the key stack (`key_parts`, shared with `OpGetRecord`); other keyed kinds wait for S4 | `hashes.loft` `dropped` |
| scalars: `<<` / `>>`, `OpMathFunc*` and `OpMathFunc2*` (float, single), `pow`, `PI` / `E`, `OpCastIntFromText` / `OpCastFloatFromText`, the float and single `Nullable` division twins, a plain enum's null and truth, a null reference, `t.byte_at(i)` | loft's own operator or function in each case (the twins are the plain `/` and `%`; an enum's absent discriminants 0 and 255 read false, `OpConvEnumFromNull`'s type operand is no value) | `small_ops.loft` `scalars` |
| `OpGetInt4Raw`, `OpSetInt4Raw`, `OpSizeHash`, `OpReverseVector` | `loft_get_int4_raw` / `loft_set_int4_raw` (width 8), `loft_hash_table_bytes`, `loft_vec_reverse` | `scalars`; the census |
| `OpKeepVectorRange`, `OpAppendTextBytes`, `OpTextCharacter` | `loft_keep_vector_range` (size 0 for a linked element), `loft_append_text_bytes` (its growth step brought to `(Col-Capacity)`), the character as its nullable twin (the raising one only logs) | `small_ops.loft` `texted` |
| natives `t_4text_find`, `t_4text_char_slice`, `t_4text_to_lowercase`, `t_4text_to_uppercase` | loft's own; a compiled stdlib body's text buffer (a `&text` after the visible arguments) gets the answer too | `texted` |
| a struct enum's parent store (`OpDatabase` of a `PtEnum` type) | minted at its largest variant (`enum_parent_size`), the variant written after | the census (19 programs) |
| natives `n_text_from_byte_range`, `n_text_from_bytes` | `loft_text_from_byte_range` over the store's vector (the whole range for the second); a compiled caller's text buffer gets the answer | `vectors.loft` `decoded` |

**Measured** (2026-10-31): the census went from 297 to 549 programs passed over these rows,
none wrong, crashed or leaking, and every passing program equal to the Rust interpreter in
every layer.

**Design → code.**  Transcribed directly, with three findings fixed where they lived rather than
routed around: `loft_crt.h` lacked `loft_keyed.h`; the C backend leaked the text temporaries a
`return` inside a loop skipped (loft#2313); and the port's `char_walk` read freed memory because a
vector bound from a `match` over an owned subject was returned as a view of it (loft#2314) — both
fixed in the compiler.  Later: a match's arm gate peeled only the arm (loft#2315), a native
`if` condition answering `boolean?` was not tested as one (loft#2316), and a `_dest`-only text
native assigned to a `text?` local read back empty on the interpreter (loft#2317, silent) — each
fixed where it lived.
