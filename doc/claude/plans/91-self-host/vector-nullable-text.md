# @PLN91 — a vector of nullable text (prepared subject)

**The family:** "a vector of this element type: TyOptional" has 52 sites in 28 programs at try
e43620208. Its element kinds: `text?` 13 sites, the narrow integers (`u8?` 10, `u16?`, `u32?`, `i8?`,
`i16?`), enums (`Color?`, `C?`), `single?`, plus 19 literals that hold a `null`. This page is the
`text?` part, the largest and the one shelved as "needs `vector<text?>` / `main_vector<text?>`
schema rows". It needs ONE new schema row, not a new row type.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native:
`v: vector<text?> = ["a", nt, "c"]` with `nt: text? = null`, then `v += [null]`, `v += ["e"]` →
`len=5`, two nulls met by a `for`, the others concatenated `ace`, `v[1] ?? "-"` → `-`, `v[0] ?? "-"` → `a`.

## Rust's form

```
  __vdb_1: ref(main_vector<text?>) = null;
  v: vector<text?>["__vdb_1"] = OpGetField(__vdb_1, 0, 44);          <- the element record type is vector<text>'s (44)
  _elm_1 = OpNewRecord(v, 44, 65535); OpSetText(_elm_1, 0, "a");  OpFinishRecord(v, _elm_1, 44, 65535);
  … OpSetText(_elm_1, 0, nt) …                                      <- a `text?` local stored as it is
  _elm_2 … OpSetText(_elm_2, 0, OpConvTextFromNull()) …              <- a literal `null` element: the text null sentinel
  for:   _vector_1: vector<text?>["v"] = v;  x = OpGetText(OpGetVectorNullable(_vector_1, 4, x#index), 0);
         x == null  →  OpEqText(x, OpConvTextFromNull())
  `??`:  __ncc_1: text["v"] = OpGetText(OpGetVectorNullable(v, 4, 1), 0);
definitions:
  #875 vector<text?>       type=6 parent=<text> at=3:21 known=65535   <- no schema row of its own
  #876 main_vector<text?>  type=5 at=3:21 known=100                    <- ONE new schema row (the wrapper)
```

**What to code:** accept `text?` as a vector element by storing it exactly as `text`. Use the same
element record type and width (4), with a `null` or an absent `text?` written as the text null
sentinel (`OpConvTextFromNull()`). Reads are `OpGetText(OpGetVectorNullable(…))`, typed `text?`, and
a null test is `OpEqText(x, OpConvTextFromNull())`. Register `vector<text?>` with no schema row
(`known=65535`, parent `text`) and `main_vector<text?>` with a NEW row at the literal that first
names the type. The narrow-integer and enum element kinds are the same family's other parts, each
with its own null sentinel; they are not prepared here.

## The `text?` sites (try e43620208)

```
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_b_put  line 197
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_c_copy_first  line 218
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_c_put  line 209
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_r_after  line 234
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_r_before  line 228
kr-suite/rules/heap/H-TextReplace/a-text-field-written-through-a-parameter-releases-what-it-replaces-unless-a-text-parameter-is-read-after.loft::n_r_loop  line 240
kr-suite/rules/iteration/I-For/a-walk-over-a-link-parameter-is-a-walk-over-the-vector.loft::n_test_more_element_kinds_a_field_and_a_loop  line 119
kr-suite/rules/layout/L-Null-Text/1795-an-absent-text-has-no-characters.loft::n_test_an_absent_text_has_no_characters_in_every_position  line 41
kr-suite/rules/types/N-Idem/1771-a-nullable-type-argument-collapses-t-optional.loft::n_main  line 38
kr-suite/rules/types/N-Shape/2317-a-destination-passed-text-native-fills-a-nullable-text-local.loft::n_element  line 47
kr-try/tests/scripts/158-text-borrow.loft::n_main  line 235
kr-try/tests/scripts/25-nullable-sequences.loft::n_element_null  line 133
kr-try/tests/scripts/25-nullable-vector-scalar-element.loft::n_check_text  line 44
```
