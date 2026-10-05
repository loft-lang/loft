// @generated — DO NOT EDIT BY HAND.
//
// The interpreter's bytecode-operator dispatch table, generated from the
// `#rust"..."` operator annotations in default/*.loft by
// `src/create.rs::generate_code_into` — each `fn op_*(s: &mut State)` body is
// that operator's `#rust` template (written in `s: &mut State` vocabulary).
//
// Regenerate after changing ANY `#rust` operator template:
//     make fill        (runs the ignored `regen_fill_rs` test, which calls
//                       `create::generate_code_to(.., "src/fill.rs")`)
// Byte-for-byte equality of this file with that regeneration is enforced by
// `tests/issues.rs::fill_rs_up_to_date` and `::n9_generated_fill_matches_src`,
// so hand-edits fail CI — edit the `#rust` template in default/*.loft instead.
//
// The SAME templates feed native code generation (`src/generation/`): there the
// `s.<method>` calls below are rewritten to their `stores.*` / `*_runtime`
// equivalents by `src/generation/calls.rs::substitute_template_body`.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::inline_always)]
#![allow(unused_parens)]

use crate::codegen_runtime;
use crate::hash;
use crate::keys::{DbRef, Str};
use crate::ops;
use crate::state::{Hot, Regs, State};
use crate::tree;
use crate::vector;

pub static OPERATORS: &[fn(&mut State)] = &[
    goto_word::<false>,
    goto_false_word::<false>,
    const_true::<false>,
    const_false::<false>,
    var_bool::<false>,
    const_int::<false>,
    var_int::<false>,
    put_int::<false>,
    conv_float_from_int::<false>,
    add_int::<false>,
    min_int::<false>,
    mul_int::<false>,
    div_int::<false>,
    rem_int::<false>,
    land_int::<false>,
    eq_int::<false>,
    lt_int::<false>,
    le_int::<false>,
    const_float::<false>,
    var_float::<false>,
    put_float::<false>,
    conv_bool_from_float::<false>,
    add_float::<false>,
    min_float::<false>,
    mul_float::<false>,
    div_float::<false>,
    div_float_nullable::<false>,
    lt_float::<false>,
    int_v_v::<false>,
    int_v_c::<false>,
    cmp_int_v_v::<false>,
    cmp_int_v_c::<false>,
    int_v_v_put::<false>,
    int_v_c_put::<false>,
    cmp_int_v_v_jump::<false>,
    cmp_int_v_c_jump::<false>,
    goto::<false>,
    goto_false::<false>,
    call::<false>,
    op_return::<false>,
    free_stack::<false>,
    reserve_frame::<false>,
    put_bool::<false>,
    not::<false>,
    const_short::<false>,
    const_tiny::<false>,
    var_character::<false>,
    var_narrow::<false>,
    put_narrow::<false>,
    put_character::<false>,
    conv_int_from_null::<false>,
    conv_bool_from_null::<false>,
    conv_character_from_int::<false>,
    abs_int::<false>,
    min_single_int::<false>,
    bit_not_single_int::<false>,
    conv_single_from_int::<false>,
    conv_bool_from_int::<false>,
    add_int_nullable::<false>,
    min_int_nullable::<false>,
    mul_int_nullable::<false>,
    div_int_nullable::<false>,
    rem_int_nullable::<false>,
    lor_int::<false>,
    eor_int::<false>,
    s_left_int::<false>,
    s_right_int::<false>,
    ne_int::<false>,
    format_int::<false>,
    format_stack_int::<false>,
    const_single::<false>,
    var_single::<false>,
    put_single::<false>,
    abs_single::<false>,
    min_single_single::<false>,
    cast_int_from_single::<false>,
    conv_float_from_single::<false>,
    conv_bool_from_single::<false>,
    add_single::<false>,
    min_single::<false>,
    mul_single::<false>,
    div_single::<false>,
    eq_single::<false>,
    ne_single::<false>,
    lt_single::<false>,
    le_single::<false>,
    conv_float_from_null::<false>,
    abs_float::<false>,
    math_func_float::<false>,
    math_func2_float::<false>,
    pow_float::<false>,
    min_single_float::<false>,
    cast_single_from_float::<false>,
    cast_int_from_float::<false>,
    rem_float::<false>,
    eq_float::<false>,
    ne_float::<false>,
    le_float::<false>,
    format_float::<false>,
    format_stack_float::<false>,
    var_text::<false>,
    arg_text::<false>,
    const_text::<false>,
    conv_text_from_null::<false>,
    length_text::<false>,
    length_character::<false>,
    conv_bool_from_text::<false>,
    init_text::<false>,
    append_text::<false>,
    put_text::<false>,
    get_text_sub::<false>,
    text_character::<false>,
    text_character_nullable::<false>,
    conv_bool_from_character::<false>,
    clear_text::<false>,
    free_text::<false>,
    eq_text::<false>,
    ne_text::<false>,
    lt_text::<false>,
    le_text::<false>,
    format_text::<false>,
    format_stack_text::<false>,
    append_character::<false>,
    text_compare::<false>,
    cast_character_from_int::<false>,
    conv_int_from_character::<false>,
    var_enum::<false>,
    const_enum::<false>,
    put_enum::<false>,
    conv_bool_from_enum::<false>,
    conv_int_from_enum::<false>,
    conv_bool_from_ref::<false>,
    conv_ref_from_null::<false>,
    init_ref::<false>,
    null_ref_sentinel::<false>,
    init_ref_sentinel::<false>,
    free_ref::<false>,
    free_ref_if_distinct::<false>,
    free_ref_or_hand_up::<false>,
    free_ref_unless_entry::<false>,
    free_scratch::<false>,
    var_ref::<false>,
    put_ref::<false>,
    eq_ref::<false>,
    ne_ref::<false>,
    get_ref::<false>,
    set_ref::<false>,
    set_db_ref::<false>,
    get_db_ref::<false>,
    get_field::<false>,
    get_int::<false>,
    get_character::<false>,
    get_single::<false>,
    get_float::<false>,
    get_byte::<false>,
    get_byte_nullable::<false>,
    get_enum::<false>,
    set_enum::<false>,
    get_boolean::<false>,
    set_boolean::<false>,
    get_short::<false>,
    get_text::<false>,
    set_int::<false>,
    set_character::<false>,
    set_single::<false>,
    set_float::<false>,
    set_byte::<false>,
    set_byte_nullable::<false>,
    set_short::<false>,
    get_int4::<false>,
    set_int4::<false>,
    get_int4_raw::<false>,
    set_int4_raw::<false>,
    get_int4_full::<false>,
    get_short_raw::<false>,
    set_short_raw::<false>,
    get_short_spare::<false>,
    get_short_full::<false>,
    set_text::<false>,
    var_vector::<false>,
    length_vector::<false>,
    vector_is_null::<false>,
    ref_is_null::<false>,
    distinct_store::<false>,
    ref_alias::<false>,
    clear_vector::<false>,
    get_vector::<false>,
    vector_ref::<false>,
    get_vector_nullable::<false>,
    vector_ref_nullable::<false>,
    append_text_bytes::<false>,
    new_record::<false>,
    finish_record::<false>,
    append_vector::<false>,
    slice_vector::<false>,
    slice_view::<false>,
    keep_range::<false>,
    push_int::<false>,
    push_int4::<false>,
    push_float::<false>,
    push_single::<false>,
    push_boolean::<false>,
    push_enum::<false>,
    push_character::<false>,
    push_byte::<false>,
    claim_child_rec::<false>,
    ref_from_child_rec::<false>,
    get_record::<false>,
    hash_add::<false>,
    hash_find::<false>,
    length_hash::<false>,
    eq_bool::<false>,
    ne_bool::<false>,
    iterate::<false>,
    step::<false>,
    append_copy::<false>,
    place_record::<false>,
    move_record::<false>,
    move_field::<false>,
    move_vector::<false>,
    free_record_in::<false>,
    copy_ref_or_null::<false>,
    bind_or_copy::<false>,
    index_group::<false>,
    link_record::<false>,
    set_keyed::<false>,
    static_call::<false>,
    create_stack::<false>,
    init_create_stack::<false>,
    get_stack_text::<false>,
    get_stack_ref::<false>,
    set_stack_ref::<false>,
    set_stack_fn_ref::<false>,
    get_stack_fn_ref::<false>,
    drop_fn_ref::<false>,
    fn_ref_closure::<false>,
    append_stack_text::<false>,
    append_stack_character::<false>,
    clear_stack_text::<false>,
    call_ref::<false>,
    coroutine_next::<false>,
    coroutine_yield::<false>,
    var_fn_ref::<false>,
    put_fn_ref::<false>,
    const_ref::<false>,
    const_store_text::<false>,
    call_ref_store::<false>,
    bind_fn_ref_result::<false>,
    text_walk_step::<false>,
    text_null_jump::<false>,
    text_end_jump::<false>,
    vec_get_int::<false>,
    vec_get_int_nullable::<false>,
    vec_set_int::<false>,
    vec_end_jump::<false>,
    cast_text_from_bool::<false>,
    range_default::<false>,
    conv_character_from_null::<false>,
    const_long_text::<false>,
    cast_int_from_text::<false>,
    cast_single_from_text::<false>,
    cast_float_from_text::<false>,
    conv_single_from_null::<false>,
    rem_single::<false>,
    div_single_nullable::<false>,
    rem_single_nullable::<false>,
    math_func_single::<false>,
    math_func2_single::<false>,
    pow_single::<false>,
    format_single::<false>,
    format_stack_single::<false>,
    math_pi_float::<false>,
    math_e_float::<false>,
    rem_float_nullable::<false>,
    size_text::<false>,
    cast_text_from_enum::<false>,
    cast_enum_from_text::<false>,
    cast_enum_from_int::<false>,
    conv_enum_from_null::<false>,
    database::<false>,
    format_database::<false>,
    format_stack_database::<false>,
    store_tag::<false>,
    free_ref_tag::<false>,
    sizeof_ref::<false>,
    eq_content::<false>,
    ne_content::<false>,
    tag_fault::<false>,
    size_vector::<false>,
    size_struct::<false>,
    size_scalar::<false>,
    length_sorted::<false>,
    cast_vector_from_text::<false>,
    remove_vector::<false>,
    keep_vector_range::<false>,
    insert_vector::<false>,
    adopt_vector::<false>,
    replace_vector::<false>,
    validate::<false>,
    hash_remove::<false>,
    reserve_hash::<false>,
    size_hash::<false>,
    length_index::<false>,
    panic::<false>,
    print::<false>,
    remove::<false>,
    clear::<false>,
    copy_record::<false>,
    replace_keyed::<false>,
    clear_keyed::<false>,
    fill_keyed::<false>,
    length_spatial::<false>,
    length_trie::<false>,
    fn_ref_detach_shared::<false>,
    parallel_begin::<false>,
    parallel_arm::<false>,
    parallel_join::<false>,
    pre_alloc_vector::<false>,
    reserve_vector::<false>,
    get_file::<false>,
    get_dir::<false>,
    get_file_text::<false>,
    write_file::<false>,
    read_file::<false>,
    seek_file::<false>,
    size_file::<false>,
    delete::<false>,
    move_file::<false>,
    truncate_file::<false>,
    sync_file::<false>,
    deliver::<false>,
    expose::<false>,
    release::<false>,
    mkdir::<false>,
    mkdir_all::<false>,
    rmdir::<false>,
    reverse_vector::<false>,
    sort_vector::<false>,
    coroutine_create::<false>,
    coroutine_return::<false>,
    coroutine_exhausted::<false>,
    coroutine_retain::<false>,
];

/// [`OPERATORS`] with the direct stack path compiled in (`@FR-R-FastTable`): valid
/// only while `State::fast_stack` holds, which is fixed for a run.
pub static OPERATORS_FAST: &[fn(&mut State)] = &[
    goto_word::<true>,
    goto_false_word::<true>,
    const_true::<true>,
    const_false::<true>,
    var_bool::<true>,
    const_int::<true>,
    var_int::<true>,
    put_int::<true>,
    conv_float_from_int::<true>,
    add_int::<true>,
    min_int::<true>,
    mul_int::<true>,
    div_int::<true>,
    rem_int::<true>,
    land_int::<true>,
    eq_int::<true>,
    lt_int::<true>,
    le_int::<true>,
    const_float::<true>,
    var_float::<true>,
    put_float::<true>,
    conv_bool_from_float::<true>,
    add_float::<true>,
    min_float::<true>,
    mul_float::<true>,
    div_float::<true>,
    div_float_nullable::<true>,
    lt_float::<true>,
    int_v_v::<true>,
    int_v_c::<true>,
    cmp_int_v_v::<true>,
    cmp_int_v_c::<true>,
    int_v_v_put::<true>,
    int_v_c_put::<true>,
    cmp_int_v_v_jump::<true>,
    cmp_int_v_c_jump::<true>,
    goto::<true>,
    goto_false::<true>,
    call::<true>,
    op_return::<true>,
    free_stack::<true>,
    reserve_frame::<true>,
    put_bool::<true>,
    not::<true>,
    const_short::<true>,
    const_tiny::<true>,
    var_character::<true>,
    var_narrow::<true>,
    put_narrow::<true>,
    put_character::<true>,
    conv_int_from_null::<true>,
    conv_bool_from_null::<true>,
    conv_character_from_int::<true>,
    abs_int::<true>,
    min_single_int::<true>,
    bit_not_single_int::<true>,
    conv_single_from_int::<true>,
    conv_bool_from_int::<true>,
    add_int_nullable::<true>,
    min_int_nullable::<true>,
    mul_int_nullable::<true>,
    div_int_nullable::<true>,
    rem_int_nullable::<true>,
    lor_int::<true>,
    eor_int::<true>,
    s_left_int::<true>,
    s_right_int::<true>,
    ne_int::<true>,
    format_int::<true>,
    format_stack_int::<true>,
    const_single::<true>,
    var_single::<true>,
    put_single::<true>,
    abs_single::<true>,
    min_single_single::<true>,
    cast_int_from_single::<true>,
    conv_float_from_single::<true>,
    conv_bool_from_single::<true>,
    add_single::<true>,
    min_single::<true>,
    mul_single::<true>,
    div_single::<true>,
    eq_single::<true>,
    ne_single::<true>,
    lt_single::<true>,
    le_single::<true>,
    conv_float_from_null::<true>,
    abs_float::<true>,
    math_func_float::<true>,
    math_func2_float::<true>,
    pow_float::<true>,
    min_single_float::<true>,
    cast_single_from_float::<true>,
    cast_int_from_float::<true>,
    rem_float::<true>,
    eq_float::<true>,
    ne_float::<true>,
    le_float::<true>,
    format_float::<true>,
    format_stack_float::<true>,
    var_text::<true>,
    arg_text::<true>,
    const_text::<true>,
    conv_text_from_null::<true>,
    length_text::<true>,
    length_character::<true>,
    conv_bool_from_text::<true>,
    init_text::<true>,
    append_text::<true>,
    put_text::<true>,
    get_text_sub::<true>,
    text_character::<true>,
    text_character_nullable::<true>,
    conv_bool_from_character::<true>,
    clear_text::<true>,
    free_text::<true>,
    eq_text::<true>,
    ne_text::<true>,
    lt_text::<true>,
    le_text::<true>,
    format_text::<true>,
    format_stack_text::<true>,
    append_character::<true>,
    text_compare::<true>,
    cast_character_from_int::<true>,
    conv_int_from_character::<true>,
    var_enum::<true>,
    const_enum::<true>,
    put_enum::<true>,
    conv_bool_from_enum::<true>,
    conv_int_from_enum::<true>,
    conv_bool_from_ref::<true>,
    conv_ref_from_null::<true>,
    init_ref::<true>,
    null_ref_sentinel::<true>,
    init_ref_sentinel::<true>,
    free_ref::<true>,
    free_ref_if_distinct::<true>,
    free_ref_or_hand_up::<true>,
    free_ref_unless_entry::<true>,
    free_scratch::<true>,
    var_ref::<true>,
    put_ref::<true>,
    eq_ref::<true>,
    ne_ref::<true>,
    get_ref::<true>,
    set_ref::<true>,
    set_db_ref::<true>,
    get_db_ref::<true>,
    get_field::<true>,
    get_int::<true>,
    get_character::<true>,
    get_single::<true>,
    get_float::<true>,
    get_byte::<true>,
    get_byte_nullable::<true>,
    get_enum::<true>,
    set_enum::<true>,
    get_boolean::<true>,
    set_boolean::<true>,
    get_short::<true>,
    get_text::<true>,
    set_int::<true>,
    set_character::<true>,
    set_single::<true>,
    set_float::<true>,
    set_byte::<true>,
    set_byte_nullable::<true>,
    set_short::<true>,
    get_int4::<true>,
    set_int4::<true>,
    get_int4_raw::<true>,
    set_int4_raw::<true>,
    get_int4_full::<true>,
    get_short_raw::<true>,
    set_short_raw::<true>,
    get_short_spare::<true>,
    get_short_full::<true>,
    set_text::<true>,
    var_vector::<true>,
    length_vector::<true>,
    vector_is_null::<true>,
    ref_is_null::<true>,
    distinct_store::<true>,
    ref_alias::<true>,
    clear_vector::<true>,
    get_vector::<true>,
    vector_ref::<true>,
    get_vector_nullable::<true>,
    vector_ref_nullable::<true>,
    append_text_bytes::<true>,
    new_record::<true>,
    finish_record::<true>,
    append_vector::<true>,
    slice_vector::<true>,
    slice_view::<true>,
    keep_range::<true>,
    push_int::<true>,
    push_int4::<true>,
    push_float::<true>,
    push_single::<true>,
    push_boolean::<true>,
    push_enum::<true>,
    push_character::<true>,
    push_byte::<true>,
    claim_child_rec::<true>,
    ref_from_child_rec::<true>,
    get_record::<true>,
    hash_add::<true>,
    hash_find::<true>,
    length_hash::<true>,
    eq_bool::<true>,
    ne_bool::<true>,
    iterate::<true>,
    step::<true>,
    append_copy::<true>,
    place_record::<true>,
    move_record::<true>,
    move_field::<true>,
    move_vector::<true>,
    free_record_in::<true>,
    copy_ref_or_null::<true>,
    bind_or_copy::<true>,
    index_group::<true>,
    link_record::<true>,
    set_keyed::<true>,
    static_call::<true>,
    create_stack::<true>,
    init_create_stack::<true>,
    get_stack_text::<true>,
    get_stack_ref::<true>,
    set_stack_ref::<true>,
    set_stack_fn_ref::<true>,
    get_stack_fn_ref::<true>,
    drop_fn_ref::<true>,
    fn_ref_closure::<true>,
    append_stack_text::<true>,
    append_stack_character::<true>,
    clear_stack_text::<true>,
    call_ref::<true>,
    coroutine_next::<true>,
    coroutine_yield::<true>,
    var_fn_ref::<true>,
    put_fn_ref::<true>,
    const_ref::<true>,
    const_store_text::<true>,
    call_ref_store::<true>,
    bind_fn_ref_result::<true>,
    text_walk_step::<true>,
    text_null_jump::<true>,
    text_end_jump::<true>,
    vec_get_int::<true>,
    vec_get_int_nullable::<true>,
    vec_set_int::<true>,
    vec_end_jump::<true>,
    cast_text_from_bool::<true>,
    range_default::<true>,
    conv_character_from_null::<true>,
    const_long_text::<true>,
    cast_int_from_text::<true>,
    cast_single_from_text::<true>,
    cast_float_from_text::<true>,
    conv_single_from_null::<true>,
    rem_single::<true>,
    div_single_nullable::<true>,
    rem_single_nullable::<true>,
    math_func_single::<true>,
    math_func2_single::<true>,
    pow_single::<true>,
    format_single::<true>,
    format_stack_single::<true>,
    math_pi_float::<true>,
    math_e_float::<true>,
    rem_float_nullable::<true>,
    size_text::<true>,
    cast_text_from_enum::<true>,
    cast_enum_from_text::<true>,
    cast_enum_from_int::<true>,
    conv_enum_from_null::<true>,
    database::<true>,
    format_database::<true>,
    format_stack_database::<true>,
    store_tag::<true>,
    free_ref_tag::<true>,
    sizeof_ref::<true>,
    eq_content::<true>,
    ne_content::<true>,
    tag_fault::<true>,
    size_vector::<true>,
    size_struct::<true>,
    size_scalar::<true>,
    length_sorted::<true>,
    cast_vector_from_text::<true>,
    remove_vector::<true>,
    keep_vector_range::<true>,
    insert_vector::<true>,
    adopt_vector::<true>,
    replace_vector::<true>,
    validate::<true>,
    hash_remove::<true>,
    reserve_hash::<true>,
    size_hash::<true>,
    length_index::<true>,
    panic::<true>,
    print::<true>,
    remove::<true>,
    clear::<true>,
    copy_record::<true>,
    replace_keyed::<true>,
    clear_keyed::<true>,
    fill_keyed::<true>,
    length_spatial::<true>,
    length_trie::<true>,
    fn_ref_detach_shared::<true>,
    parallel_begin::<true>,
    parallel_arm::<true>,
    parallel_join::<true>,
    pre_alloc_vector::<true>,
    reserve_vector::<true>,
    get_file::<true>,
    get_dir::<true>,
    get_file_text::<true>,
    write_file::<true>,
    read_file::<true>,
    seek_file::<true>,
    size_file::<true>,
    delete::<true>,
    move_file::<true>,
    truncate_file::<true>,
    sync_file::<true>,
    deliver::<true>,
    expose::<true>,
    release::<true>,
    mkdir::<true>,
    mkdir_all::<true>,
    rmdir::<true>,
    reverse_vector::<true>,
    sort_vector::<true>,
    coroutine_create::<true>,
    coroutine_return::<true>,
    coroutine_exhausted::<true>,
    coroutine_retain::<true>,
];

/// [`OPERATORS_FAST`] with the bytecode position and the stack top passed in and
/// returned in registers ([`Regs`]), so no op reads back what the previous op stored.
pub static OPERATORS_REG: &[fn(&mut State, Regs) -> Regs] = &[
    goto_word_r,
    goto_false_word_r,
    const_true_r,
    const_false_r,
    var_bool_r,
    const_int_r,
    var_int_r,
    put_int_r,
    conv_float_from_int_r,
    add_int_r,
    min_int_r,
    mul_int_r,
    div_int_r,
    rem_int_r,
    land_int_r,
    eq_int_r,
    lt_int_r,
    le_int_r,
    const_float_r,
    var_float_r,
    put_float_r,
    conv_bool_from_float_r,
    add_float_r,
    min_float_r,
    mul_float_r,
    div_float_r,
    div_float_nullable_r,
    lt_float_r,
    int_v_v_r,
    int_v_c_r,
    cmp_int_v_v_r,
    cmp_int_v_c_r,
    int_v_v_put_r,
    int_v_c_put_r,
    cmp_int_v_v_jump_r,
    cmp_int_v_c_jump_r,
    goto_r,
    goto_false_r,
    call_r,
    op_return_r,
    free_stack_r,
    reserve_frame_r,
    put_bool_r,
    not_r,
    const_short_r,
    const_tiny_r,
    var_character_r,
    var_narrow_r,
    put_narrow_r,
    put_character_r,
    conv_int_from_null_r,
    conv_bool_from_null_r,
    conv_character_from_int_r,
    abs_int_r,
    min_single_int_r,
    bit_not_single_int_r,
    conv_single_from_int_r,
    conv_bool_from_int_r,
    add_int_nullable_r,
    min_int_nullable_r,
    mul_int_nullable_r,
    div_int_nullable_r,
    rem_int_nullable_r,
    lor_int_r,
    eor_int_r,
    s_left_int_r,
    s_right_int_r,
    ne_int_r,
    format_int_r,
    format_stack_int_r,
    const_single_r,
    var_single_r,
    put_single_r,
    abs_single_r,
    min_single_single_r,
    cast_int_from_single_r,
    conv_float_from_single_r,
    conv_bool_from_single_r,
    add_single_r,
    min_single_r,
    mul_single_r,
    div_single_r,
    eq_single_r,
    ne_single_r,
    lt_single_r,
    le_single_r,
    conv_float_from_null_r,
    abs_float_r,
    math_func_float_r,
    math_func2_float_r,
    pow_float_r,
    min_single_float_r,
    cast_single_from_float_r,
    cast_int_from_float_r,
    rem_float_r,
    eq_float_r,
    ne_float_r,
    le_float_r,
    format_float_r,
    format_stack_float_r,
    var_text_r,
    arg_text_r,
    const_text_r,
    conv_text_from_null_r,
    length_text_r,
    length_character_r,
    conv_bool_from_text_r,
    init_text_r,
    append_text_r,
    put_text_r,
    get_text_sub_r,
    text_character_r,
    text_character_nullable_r,
    conv_bool_from_character_r,
    clear_text_r,
    free_text_r,
    eq_text_r,
    ne_text_r,
    lt_text_r,
    le_text_r,
    format_text_r,
    format_stack_text_r,
    append_character_r,
    text_compare_r,
    cast_character_from_int_r,
    conv_int_from_character_r,
    var_enum_r,
    const_enum_r,
    put_enum_r,
    conv_bool_from_enum_r,
    conv_int_from_enum_r,
    conv_bool_from_ref_r,
    conv_ref_from_null_r,
    init_ref_r,
    null_ref_sentinel_r,
    init_ref_sentinel_r,
    free_ref_r,
    free_ref_if_distinct_r,
    free_ref_or_hand_up_r,
    free_ref_unless_entry_r,
    free_scratch_r,
    var_ref_r,
    put_ref_r,
    eq_ref_r,
    ne_ref_r,
    get_ref_r,
    set_ref_r,
    set_db_ref_r,
    get_db_ref_r,
    get_field_r,
    get_int_r,
    get_character_r,
    get_single_r,
    get_float_r,
    get_byte_r,
    get_byte_nullable_r,
    get_enum_r,
    set_enum_r,
    get_boolean_r,
    set_boolean_r,
    get_short_r,
    get_text_r,
    set_int_r,
    set_character_r,
    set_single_r,
    set_float_r,
    set_byte_r,
    set_byte_nullable_r,
    set_short_r,
    get_int4_r,
    set_int4_r,
    get_int4_raw_r,
    set_int4_raw_r,
    get_int4_full_r,
    get_short_raw_r,
    set_short_raw_r,
    get_short_spare_r,
    get_short_full_r,
    set_text_r,
    var_vector_r,
    length_vector_r,
    vector_is_null_r,
    ref_is_null_r,
    distinct_store_r,
    ref_alias_r,
    clear_vector_r,
    get_vector_r,
    vector_ref_r,
    get_vector_nullable_r,
    vector_ref_nullable_r,
    append_text_bytes_r,
    new_record_r,
    finish_record_r,
    append_vector_r,
    slice_vector_r,
    slice_view_r,
    keep_range_r,
    push_int_r,
    push_int4_r,
    push_float_r,
    push_single_r,
    push_boolean_r,
    push_enum_r,
    push_character_r,
    push_byte_r,
    claim_child_rec_r,
    ref_from_child_rec_r,
    get_record_r,
    hash_add_r,
    hash_find_r,
    length_hash_r,
    eq_bool_r,
    ne_bool_r,
    iterate_r,
    step_r,
    append_copy_r,
    place_record_r,
    move_record_r,
    move_field_r,
    move_vector_r,
    free_record_in_r,
    copy_ref_or_null_r,
    bind_or_copy_r,
    index_group_r,
    link_record_r,
    set_keyed_r,
    static_call_r,
    create_stack_r,
    init_create_stack_r,
    get_stack_text_r,
    get_stack_ref_r,
    set_stack_ref_r,
    set_stack_fn_ref_r,
    get_stack_fn_ref_r,
    drop_fn_ref_r,
    fn_ref_closure_r,
    append_stack_text_r,
    append_stack_character_r,
    clear_stack_text_r,
    call_ref_r,
    coroutine_next_r,
    coroutine_yield_r,
    var_fn_ref_r,
    put_fn_ref_r,
    const_ref_r,
    const_store_text_r,
    call_ref_store_r,
    bind_fn_ref_result_r,
    text_walk_step_r,
    text_null_jump_r,
    text_end_jump_r,
    vec_get_int_r,
    vec_get_int_nullable_r,
    vec_set_int_r,
    vec_end_jump_r,
    cast_text_from_bool_r,
    range_default_r,
    conv_character_from_null_r,
    const_long_text_r,
    cast_int_from_text_r,
    cast_single_from_text_r,
    cast_float_from_text_r,
    conv_single_from_null_r,
    rem_single_r,
    div_single_nullable_r,
    rem_single_nullable_r,
    math_func_single_r,
    math_func2_single_r,
    pow_single_r,
    format_single_r,
    format_stack_single_r,
    math_pi_float_r,
    math_e_float_r,
    rem_float_nullable_r,
    size_text_r,
    cast_text_from_enum_r,
    cast_enum_from_text_r,
    cast_enum_from_int_r,
    conv_enum_from_null_r,
    database_r,
    format_database_r,
    format_stack_database_r,
    store_tag_r,
    free_ref_tag_r,
    sizeof_ref_r,
    eq_content_r,
    ne_content_r,
    tag_fault_r,
    size_vector_r,
    size_struct_r,
    size_scalar_r,
    length_sorted_r,
    cast_vector_from_text_r,
    remove_vector_r,
    keep_vector_range_r,
    insert_vector_r,
    adopt_vector_r,
    replace_vector_r,
    validate_r,
    hash_remove_r,
    reserve_hash_r,
    size_hash_r,
    length_index_r,
    panic_r,
    print_r,
    remove_r,
    clear_r,
    copy_record_r,
    replace_keyed_r,
    clear_keyed_r,
    fill_keyed_r,
    length_spatial_r,
    length_trie_r,
    fn_ref_detach_shared_r,
    parallel_begin_r,
    parallel_arm_r,
    parallel_join_r,
    pre_alloc_vector_r,
    reserve_vector_r,
    get_file_r,
    get_dir_r,
    get_file_text_r,
    write_file_r,
    read_file_r,
    seek_file_r,
    size_file_r,
    delete_r,
    move_file_r,
    truncate_file_r,
    sync_file_r,
    deliver_r,
    expose_r,
    release_r,
    mkdir_r,
    mkdir_all_r,
    rmdir_r,
    reverse_vector_r,
    sort_vector_r,
    coroutine_create_r,
    coroutine_return_r,
    coroutine_exhausted_r,
    coroutine_retain_r,
];

/// The loft name of each [`OPERATORS`] slot, in slot order — the operator declarations
/// of the `default/` this binary was generated from.
pub const OPERATOR_NAMES: &[&str] = &[
    "OpGotoWord",
    "OpGotoFalseWord",
    "OpConstTrue",
    "OpConstFalse",
    "OpVarBool",
    "OpConstInt",
    "OpVarInt",
    "OpPutInt",
    "OpConvFloatFromInt",
    "OpAddInt",
    "OpMinInt",
    "OpMulInt",
    "OpDivInt",
    "OpRemInt",
    "OpLandInt",
    "OpEqInt",
    "OpLtInt",
    "OpLeInt",
    "OpConstFloat",
    "OpVarFloat",
    "OpPutFloat",
    "OpConvBoolFromFloat",
    "OpAddFloat",
    "OpMinFloat",
    "OpMulFloat",
    "OpDivFloat",
    "OpDivFloatNullable",
    "OpLtFloat",
    "OpIntVV",
    "OpIntVC",
    "OpCmpIntVV",
    "OpCmpIntVC",
    "OpIntVVPut",
    "OpIntVCPut",
    "OpCmpIntVVJump",
    "OpCmpIntVCJump",
    "OpGoto",
    "OpGotoFalse",
    "OpCall",
    "OpReturn",
    "OpFreeStack",
    "OpReserveFrame",
    "OpPutBool",
    "OpNot",
    "OpConstShort",
    "OpConstTiny",
    "OpVarCharacter",
    "OpVarNarrow",
    "OpPutNarrow",
    "OpPutCharacter",
    "OpConvIntFromNull",
    "OpConvBoolFromNull",
    "OpConvCharacterFromInt",
    "OpAbsInt",
    "OpMinSingleInt",
    "OpBitNotSingleInt",
    "OpConvSingleFromInt",
    "OpConvBoolFromInt",
    "OpAddIntNullable",
    "OpMinIntNullable",
    "OpMulIntNullable",
    "OpDivIntNullable",
    "OpRemIntNullable",
    "OpLorInt",
    "OpEorInt",
    "OpSLeftInt",
    "OpSRightInt",
    "OpNeInt",
    "OpFormatInt",
    "OpFormatStackInt",
    "OpConstSingle",
    "OpVarSingle",
    "OpPutSingle",
    "OpAbsSingle",
    "OpMinSingleSingle",
    "OpCastIntFromSingle",
    "OpConvFloatFromSingle",
    "OpConvBoolFromSingle",
    "OpAddSingle",
    "OpMinSingle",
    "OpMulSingle",
    "OpDivSingle",
    "OpEqSingle",
    "OpNeSingle",
    "OpLtSingle",
    "OpLeSingle",
    "OpConvFloatFromNull",
    "OpAbsFloat",
    "OpMathFuncFloat",
    "OpMathFunc2Float",
    "OpPowFloat",
    "OpMinSingleFloat",
    "OpCastSingleFromFloat",
    "OpCastIntFromFloat",
    "OpRemFloat",
    "OpEqFloat",
    "OpNeFloat",
    "OpLeFloat",
    "OpFormatFloat",
    "OpFormatStackFloat",
    "OpVarText",
    "OpArgText",
    "OpConstText",
    "OpConvTextFromNull",
    "OpLengthText",
    "OpLengthCharacter",
    "OpConvBoolFromText",
    "OpInitText",
    "OpAppendText",
    "OpPutText",
    "OpGetTextSub",
    "OpTextCharacter",
    "OpTextCharacterNullable",
    "OpConvBoolFromCharacter",
    "OpClearText",
    "OpFreeText",
    "OpEqText",
    "OpNeText",
    "OpLtText",
    "OpLeText",
    "OpFormatText",
    "OpFormatStackText",
    "OpAppendCharacter",
    "OpTextCompare",
    "OpCastCharacterFromInt",
    "OpConvIntFromCharacter",
    "OpVarEnum",
    "OpConstEnum",
    "OpPutEnum",
    "OpConvBoolFromEnum",
    "OpConvIntFromEnum",
    "OpConvBoolFromRef",
    "OpConvRefFromNull",
    "OpInitRef",
    "OpNullRefSentinel",
    "OpInitRefSentinel",
    "OpFreeRef",
    "OpFreeRefIfDistinct",
    "OpFreeRefOrHandUp",
    "OpFreeRefUnlessEntry",
    "OpFreeScratch",
    "OpVarRef",
    "OpPutRef",
    "OpEqRef",
    "OpNeRef",
    "OpGetRef",
    "OpSetRef",
    "OpSetDbRef",
    "OpGetDbRef",
    "OpGetField",
    "OpGetInt",
    "OpGetCharacter",
    "OpGetSingle",
    "OpGetFloat",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetEnum",
    "OpSetEnum",
    "OpGetBoolean",
    "OpSetBoolean",
    "OpGetShort",
    "OpGetText",
    "OpSetInt",
    "OpSetCharacter",
    "OpSetSingle",
    "OpSetFloat",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetShort",
    "OpGetInt4",
    "OpSetInt4",
    "OpGetInt4Raw",
    "OpSetInt4Raw",
    "OpGetInt4Full",
    "OpGetShortRaw",
    "OpSetShortRaw",
    "OpGetShortSpare",
    "OpGetShortFull",
    "OpSetText",
    "OpVarVector",
    "OpLengthVector",
    "OpVectorIsNull",
    "OpRefIsNull",
    "OpDistinctStore",
    "OpRefAlias",
    "OpClearVector",
    "OpGetVector",
    "OpVectorRef",
    "OpGetVectorNullable",
    "OpVectorRefNullable",
    "OpAppendTextBytes",
    "OpNewRecord",
    "OpFinishRecord",
    "OpAppendVector",
    "OpSliceVector",
    "OpSliceView",
    "OpKeepRange",
    "OpPushInt",
    "OpPushInt4",
    "OpPushFloat",
    "OpPushSingle",
    "OpPushBoolean",
    "OpPushEnum",
    "OpPushCharacter",
    "OpPushByte",
    "OpClaimChildRec",
    "OpRefFromChildRec",
    "OpGetRecord",
    "OpHashAdd",
    "OpHashFind",
    "OpLengthHash",
    "OpEqBool",
    "OpNeBool",
    "OpIterate",
    "OpStep",
    "OpAppendCopy",
    "OpPlaceRecord",
    "OpMoveRecord",
    "OpMoveField",
    "OpMoveVector",
    "OpFreeRecordIn",
    "OpCopyRefOrNull",
    "OpBindOrCopy",
    "OpIndexGroup",
    "OpLinkRecord",
    "OpSetKeyed",
    "OpStaticCall",
    "OpCreateStack",
    "OpInitCreateStack",
    "OpGetStackText",
    "OpGetStackRef",
    "OpSetStackRef",
    "OpSetStackFnRef",
    "OpGetStackFnRef",
    "OpDropFnRef",
    "OpFnRefClosure",
    "OpAppendStackText",
    "OpAppendStackCharacter",
    "OpClearStackText",
    "OpCallRef",
    "OpCoroutineNext",
    "OpCoroutineYield",
    "OpVarFnRef",
    "OpPutFnRef",
    "OpConstRef",
    "OpConstStoreText",
    "OpCallRefStore",
    "OpBindFnRefResult",
    "OpTextWalkStep",
    "OpTextNullJump",
    "OpTextEndJump",
    "OpVecGetInt",
    "OpVecGetIntNullable",
    "OpVecSetInt",
    "OpVecEndJump",
    "OpCastTextFromBool",
    "OpRangeDefault",
    "OpConvCharacterFromNull",
    "OpConstLongText",
    "OpCastIntFromText",
    "OpCastSingleFromText",
    "OpCastFloatFromText",
    "OpConvSingleFromNull",
    "OpRemSingle",
    "OpDivSingleNullable",
    "OpRemSingleNullable",
    "OpMathFuncSingle",
    "OpMathFunc2Single",
    "OpPowSingle",
    "OpFormatSingle",
    "OpFormatStackSingle",
    "OpMathPiFloat",
    "OpMathEFloat",
    "OpRemFloatNullable",
    "OpSizeText",
    "OpCastTextFromEnum",
    "OpCastEnumFromText",
    "OpCastEnumFromInt",
    "OpConvEnumFromNull",
    "OpDatabase",
    "OpFormatDatabase",
    "OpFormatStackDatabase",
    "OpStoreTag",
    "OpFreeRefTag",
    "OpSizeofRef",
    "OpEqContent",
    "OpNeContent",
    "OpTagFault",
    "OpSizeVector",
    "OpSizeStruct",
    "OpSizeScalar",
    "OpLengthSorted",
    "OpCastVectorFromText",
    "OpRemoveVector",
    "OpKeepVectorRange",
    "OpInsertVector",
    "OpAdoptVector",
    "OpReplaceVector",
    "OpValidate",
    "OpHashRemove",
    "OpReserveHash",
    "OpSizeHash",
    "OpLengthIndex",
    "OpPanic",
    "OpPrint",
    "OpRemove",
    "OpClear",
    "OpCopyRecord",
    "OpReplaceKeyed",
    "OpClearKeyed",
    "OpFillKeyed",
    "OpLengthSpatial",
    "OpLengthTrie",
    "OpFnRefDetachShared",
    "OpParallelBegin",
    "OpParallelArm",
    "OpParallelJoin",
    "OpPreAllocVector",
    "OpReserveVector",
    "OpGetFile",
    "OpGetDir",
    "OpGetFileText",
    "OpWriteFile",
    "OpReadFile",
    "OpSeekFile",
    "OpSizeFile",
    "OpDelete",
    "OpMoveFile",
    "OpTruncateFile",
    "OpSyncFile",
    "OpDeliver",
    "OpExpose",
    "OpRelease",
    "OpMkdir",
    "OpMkdirAll",
    "OpRmdir",
    "OpReverseVector",
    "OpSortVector",
    "OpCoroutineCreate",
    "OpCoroutineReturn",
    "OpCoroutineExhausted",
    "OpCoroutineRetain",
];

/// How many operators are `#hot` (slots `0..OP_HOT`, all one-byte opcodes): the lean
/// loop runs them inline.  `Data::op_code` numbers a declaration from this and
/// [`OP_NORMAL`], the order the tables above are laid out in.
pub const OP_HOT: u16 = 36;
/// How many operators are neither `#hot` nor `#cold` (the slots after the hot ones); the
/// `#cold` operators follow them, into the two-byte opcodes.
pub const OP_NORMAL: u16 = 219;

#[inline(always)]
fn goto<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_step = operands.get::<i8>(0);
    s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
}

fn goto_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    goto::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn goto_word<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_step = operands.get::<i32>(0);
    s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
}

fn goto_word_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    goto_word::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn goto_word_h(s: &mut Hot) {
    let operands = s.operands(4);
    let v_step = operands.get::<i32>(0);
    s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
}

#[inline(always)]
fn goto_false<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_step = operands.get::<i8>(0);
    let v_if_false = s.get_stack_m::<F, u8>();
    if v_if_false != 1 {
        s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
    }
}

fn goto_false_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    goto_false::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn goto_false_word<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_step = operands.get::<i32>(0);
    let v_if_false = s.get_stack_m::<F, u8>();
    if v_if_false != 1 {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

fn goto_false_word_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    goto_false_word::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn goto_false_word_h(s: &mut Hot) {
    let operands = s.operands(4);
    let v_step = operands.get::<i32>(0);
    let v_if_false = s.get_stack::<u8>();
    if v_if_false != 1 {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

#[inline(always)]
fn call<const F: bool>(s: &mut State) {
    let operands = s.operands(18);
    let v_d_nr = operands.get::<i64>(0);
    let v_args_size = operands.get::<u16>(8);
    let v_to = operands.get::<i64>(10);
    s.fn_call(v_d_nr as u32, v_args_size, v_to);
}

fn call_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    call::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn op_return<const F: bool>(s: &mut State) {
    let operands = s.operands(5);
    let v_ret = operands.get::<u16>(0);
    let v_value = operands.get::<u8>(2);
    let v_discard = operands.get::<u16>(3);
    s.fn_return(v_ret, v_value, v_discard);
}

fn op_return_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    op_return::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_stack<const F: bool>(s: &mut State) {
    let operands = s.operands(3);
    let v_value = operands.get::<u8>(0);
    let v_discard = operands.get::<u16>(1);
    s.free_stack(v_value, v_discard);
}

fn free_stack_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_stack::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn reserve_frame<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_size = operands.get::<u16>(0);
    s.reserve_frame(v_size);
}

fn reserve_frame_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    reserve_frame::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_true<const F: bool>(s: &mut State) {
    let new_value = true;
    s.put_stack_m::<F, _>(new_value);
}

fn const_true_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_true::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_true_h(s: &mut Hot) {
    let new_value = true;
    s.put_stack(new_value);
}

#[inline(always)]
fn const_false<const F: bool>(s: &mut State) {
    let new_value = false;
    s.put_stack_m::<F, _>(new_value);
}

fn const_false_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_false::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_false_h(s: &mut Hot) {
    let new_value = false;
    s.put_stack(new_value);
}

#[inline(always)]
fn cast_text_from_bool<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = if v_v1 == 1 {
        "true"
    } else if v_v1 == 255 {
        "null"
    } else {
        "false"
    };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_text_from_bool_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_text_from_bool::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_bool<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, u8>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_bool_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_bool::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_bool_h(s: &mut Hot) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var::<u8>(v_pos);
    s.put_stack(new_value);
}

#[inline(always)]
fn put_bool<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_var = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, u8>();
    s.put_var_m::<F, _>(v_var, v_value);
}

fn put_bool_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_bool::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn not<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = v_v1 != 1;
    s.put_stack_m::<F, _>(new_value);
}

fn not_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    not::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn range_default<const F: bool>(s: &mut State) {
    let operands = s.operands(24);
    let v_lo = operands.get::<i64>(0);
    let v_hi = operands.get::<i64>(8);
    let v_dflt = operands.get::<i64>(16);
    let v_val = s.get_stack_m::<F, i64>();
    let new_value = {
        let _rv = v_val;
        if _rv == i64::MIN {
            if v_dflt == i64::MIN { _rv } else { v_dflt }
        } else if _rv >= v_lo && _rv <= v_hi {
            _rv
        } else {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::RangeDefaulted {
                value: _rv,
                lo: v_lo,
                hi: v_hi,
            });
            v_dflt
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn range_default_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    range_default::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_int<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_val = operands.get::<i64>(0);
    let new_value = v_val;
    s.put_stack_m::<F, _>(new_value);
}

fn const_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_int_h(s: &mut Hot) {
    let operands = s.operands(8);
    let v_val = operands.get::<i64>(0);
    let new_value = v_val;
    s.put_stack(new_value);
}

#[inline(always)]
fn const_short<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_val = operands.get::<i16>(0);
    let new_value = i64::from(v_val);
    s.put_stack_m::<F, _>(new_value);
}

fn const_short_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_short::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_tiny<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_val = operands.get::<i8>(0);
    let new_value = i64::from(v_val);
    s.put_stack_m::<F, _>(new_value);
}

fn const_tiny_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_tiny::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_int<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, i64>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_int_h(s: &mut Hot) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var::<i64>(v_pos);
    s.put_stack(new_value);
}

#[inline(always)]
fn var_character<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, char>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_int<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, i64>();
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_int_h(s: &mut Hot) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack::<i64>();
    s.put_var(v_pos, v_value);
}

#[inline(always)]
fn var_narrow<const F: bool>(s: &mut State) {
    let operands = s.operands(7);
    let v_pos = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_kind = operands.get::<u8>(6);
    let new_value = s.var_narrow(v_pos, v_min, v_kind);
    s.put_stack_m::<F, _>(new_value);
}

fn var_narrow_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_narrow::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_narrow<const F: bool>(s: &mut State) {
    let operands = s.operands(7);
    let v_pos = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_kind = operands.get::<u8>(6);
    let v_value = s.get_stack_m::<F, i64>();
    s.put_narrow(v_pos, v_min, v_kind, v_value);
}

fn put_narrow_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_narrow::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_character<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = char::from_u32(s.get_stack_m::<F, u32>()).unwrap_or('\0');
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_int_from_null<const F: bool>(s: &mut State) {
    let new_value = i64::MIN;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_int_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_int_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_null<const F: bool>(s: &mut State) {
    let new_value = 255u8;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_character_from_null<const F: bool>(s: &mut State) {
    let new_value = char::from(0);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_character_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_character_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_character_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v1 == i64::MIN {
        char::from(0)
    } else if let Some(c) = char::from_u32((v_v1) as u32) {
        c
    } else {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
        char::from(0)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn conv_character_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_character_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_long_text<const F: bool>(s: &mut State) {
    let operands = s.operands(16);
    let v_start = operands.get::<i64>(0);
    let v_size = operands.get::<i64>(8);
    s.string_from_texts(v_start, v_size);
}

fn const_long_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_long_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_int_from_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = match v_v1.str().parse::<i64>() {
        Ok(v) if v == i64::MIN => {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
            i64::MIN
        }
        Ok(v) => v,
        Err(_) => i64::MIN,
    };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_int_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_int_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_single_from_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f32::NAN);
    s.put_stack_m::<F, _>(new_value);
}

fn cast_single_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_single_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_float_from_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f64::NAN);
    s.put_stack_m::<F, _>(new_value);
}

fn cast_float_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_float_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn abs_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_abs_int(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn abs_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    abs_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_single_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_negate_int(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn min_single_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_single_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn bit_not_single_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = !v_v1;
    s.put_stack_m::<F, _>(new_value);
}

fn bit_not_single_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    bit_not_single_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_float_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_conv_float_from_int(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_float_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_float_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_float_from_int_h(s: &mut Hot) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_conv_float_from_int(v_v1);
    s.put_stack(new_value);
}

#[inline(always)]
fn conv_single_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_conv_single_from_int(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_single_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_single_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_conv_bool_from_int(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn add_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_add_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn add_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    add_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn add_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_add_int(v_v1, v_v2);
    s.put_stack(new_value);
}

#[inline(always)]
fn min_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_min_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn min_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_min_int(v_v1, v_v2);
    s.put_stack(new_value);
}

#[inline(always)]
fn mul_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_mul_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn mul_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mul_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mul_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_mul_int(v_v1, v_v2);
    s.put_stack(new_value);
}

#[inline(always)]
fn div_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v2 == 0 {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        i64::MIN
    } else {
        ops::op_div_int(v_v1, v_v2)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = if v_v2 == 0 {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        i64::MIN
    } else {
        ops::op_div_int(v_v1, v_v2)
    };
    s.put_stack(new_value);
}

#[inline(always)]
fn rem_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v2 == 0 {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        i64::MIN
    } else {
        ops::op_rem_int(v_v1, v_v2)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn rem_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = if v_v2 == 0 {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        i64::MIN
    } else {
        ops::op_rem_int(v_v1, v_v2)
    };
    s.put_stack(new_value);
}

#[inline(always)]
fn add_int_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_add_int_nullable(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn add_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    add_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_int_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_min_int_nullable(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn min_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mul_int_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_mul_int_nullable(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn mul_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mul_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_int_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = {
        let r = ops::op_div_int_nullable(v_v1, v_v2);
        ops::note_format_fault(1, r == i64::MIN && v_v1 != i64::MIN && v_v2 != i64::MIN);
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn rem_int_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = {
        let r = ops::op_rem_int_nullable(v_v1, v_v2);
        ops::note_format_fault(2, r == i64::MIN && v_v1 != i64::MIN && v_v2 != i64::MIN);
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn land_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_logical_and_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn land_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    land_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn land_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_logical_and_int(v_v1, v_v2);
    s.put_stack(new_value);
}

#[inline(always)]
fn lor_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_logical_or_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn lor_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    lor_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eor_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = ops::op_exclusive_or_int(v_v1, v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn eor_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eor_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn s_left_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v1 == i64::MIN || v_v2 == i64::MIN {
        i64::MIN
    } else if !(0..64).contains(&v_v2) {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::ShiftOutOfRange);
        i64::MIN
    } else {
        let r = ops::op_shift_left_int(v_v1, v_v2);
        if r == i64::MIN {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::ShiftOutOfRange);
            i64::MIN
        } else {
            r
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn s_left_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    s_left_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn s_right_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v1 == i64::MIN || v_v2 == i64::MIN {
        i64::MIN
    } else if !(0..64).contains(&v_v2) {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::ShiftOutOfRange);
        i64::MIN
    } else {
        ops::op_shift_right_int(v_v1, v_v2)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn s_right_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    s_right_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = v_v1 == v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn eq_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn ne_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = v_v1 != v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn ne_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = v_v1 < v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn lt_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    lt_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 < v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn le_int<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = v_v1 <= v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn le_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    le_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn le_int_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 <= v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn format_int<const F: bool>(s: &mut State) {
    s.format_int();
}

fn format_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_stack_int<const F: bool>(s: &mut State) {
    s.format_stack_int();
}

fn format_stack_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_stack_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_single<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_val = operands.get::<f32>(0);
    let new_value = v_val;
    s.put_stack_m::<F, _>(new_value);
}

fn const_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_single<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, f32>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_single<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, f32>();
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_single_from_null<const F: bool>(s: &mut State) {
    let new_value = f32::NAN;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_single_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_single_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn abs_single<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1.abs();
    s.put_stack_m::<F, _>(new_value);
}

fn abs_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    abs_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_single_single<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = -v_v1;
    s.put_stack_m::<F, _>(new_value);
}

fn min_single_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_single_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_int_from_single<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        let f = f64::from(v_v1);
        if f.is_nan() {
            i64::MIN
        } else if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&f) {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
            i64::MIN
        } else {
            let r = ops::op_cast_int_from_single(v_v1);
            if r == i64::MIN {
                s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
                i64::MIN
            } else {
                r
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_int_from_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_int_from_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_float_from_single<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = f64::from(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_float_from_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_float_from_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_single<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = !v_v1.is_nan();
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn add_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1 + v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn add_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    add_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1 - v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn min_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mul_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1 * v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn mul_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mul_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 / v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn rem_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 % v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_single_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        let r = v_v1 / v_v2;
        ops::note_format_fault(1, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_single_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_single_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn rem_single_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        let r = v_v1 % v_v2;
        ops::note_format_fault(2, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_single_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_single_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_func_single<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_fn_id = operands.get::<i8>(0);
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = match v_fn_id {
        0 => v_v1.cos(),
        1 => v_v1.sin(),
        2 => v_v1.tan(),
        3 => v_v1.acos(),
        4 => v_v1.asin(),
        5 => v_v1.atan(),
        6 => v_v1.ceil(),
        7 => v_v1.floor(),
        8 => v_v1.round(),
        9 => v_v1.sqrt(),
        _ => f32::NAN,
    };
    s.put_stack_m::<F, _>(new_value);
}

fn math_func_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_func_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_func2_single<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_fn_id = operands.get::<i8>(0);
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => {
            let __b = v_v2;
            if __b.to_bits() == 10.0_f32.to_bits() {
                v_v1.log10()
            } else if __b.to_bits() == 2.0_f32.to_bits() {
                v_v1.log2()
            } else {
                v_v1.log(__b)
            }
        }
        _ => f32::NAN,
    };
    s.put_stack_m::<F, _>(new_value);
}

fn math_func2_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_func2_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn pow_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn pow_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    pow_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = (v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn eq_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = !((v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1));
    s.put_stack_m::<F, _>(new_value);
}

fn ne_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = {
        let _a = v_v1;
        let _b = v_v2;
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let _lt = !_b.is_nan() && !(_a >= _b);
        _lt
    };
    s.put_stack_m::<F, _>(new_value);
}

fn lt_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    lt_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn le_single<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, f32>();
    let new_value = v_v1.is_nan() || (!v_v2.is_nan() && v_v1 <= v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn le_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    le_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_single<const F: bool>(s: &mut State) {
    s.format_single();
}

fn format_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_stack_single<const F: bool>(s: &mut State) {
    s.format_stack_single();
}

fn format_stack_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_stack_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_float<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_val = operands.get::<f64>(0);
    let new_value = v_val;
    s.put_stack_m::<F, _>(new_value);
}

fn const_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_float_h(s: &mut Hot) {
    let operands = s.operands(8);
    let v_val = operands.get::<f64>(0);
    let new_value = v_val;
    s.put_stack(new_value);
}

#[inline(always)]
fn var_float<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, f64>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_float_h(s: &mut Hot) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var::<f64>(v_pos);
    s.put_stack(new_value);
}

#[inline(always)]
fn put_float<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, f64>();
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_float_h(s: &mut Hot) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack::<f64>();
    s.put_var(v_pos, v_value);
}

#[inline(always)]
fn conv_float_from_null<const F: bool>(s: &mut State) {
    let new_value = f64::NAN;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_float_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_float_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn abs_float<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1.abs();
    s.put_stack_m::<F, _>(new_value);
}

fn abs_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    abs_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_pi_float<const F: bool>(s: &mut State) {
    let new_value = std::f64::consts::PI;
    s.put_stack_m::<F, _>(new_value);
}

fn math_pi_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_pi_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_e_float<const F: bool>(s: &mut State) {
    let new_value = std::f64::consts::E;
    s.put_stack_m::<F, _>(new_value);
}

fn math_e_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_e_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_func_float<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_fn_id = operands.get::<i8>(0);
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = match v_fn_id {
        0 => v_v1.cos(),
        1 => v_v1.sin(),
        2 => v_v1.tan(),
        3 => v_v1.acos(),
        4 => v_v1.asin(),
        5 => v_v1.atan(),
        6 => v_v1.ceil(),
        7 => v_v1.floor(),
        8 => v_v1.round(),
        9 => v_v1.sqrt(),
        _ => f64::NAN,
    };
    s.put_stack_m::<F, _>(new_value);
}

fn math_func_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_func_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn math_func2_float<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_fn_id = operands.get::<i8>(0);
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => {
            let __b = v_v2;
            if __b.to_bits() == 10.0_f64.to_bits() {
                v_v1.log10()
            } else if __b.to_bits() == 2.0_f64.to_bits() {
                v_v1.log2()
            } else {
                v_v1.log(__b)
            }
        }
        _ => f64::NAN,
    };
    s.put_stack_m::<F, _>(new_value);
}

fn math_func2_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    math_func2_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn pow_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn pow_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    pow_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_single_float<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = -v_v1;
    s.put_stack_m::<F, _>(new_value);
}

fn min_single_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_single_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_single_from_float<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1 as f32;
    s.put_stack_m::<F, _>(new_value);
}

fn cast_single_from_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_single_from_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_int_from_float<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = if v_v1.is_nan() {
        i64::MIN
    } else if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&v_v1) {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
        i64::MIN
    } else {
        let r = ops::op_cast_int_from_float(v_v1);
        if r == i64::MIN {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
            i64::MIN
        } else {
            r
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_int_from_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_int_from_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_float<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = !v_v1.is_nan();
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_float_h(s: &mut Hot) {
    let v_v1 = s.get_stack::<f64>();
    let new_value = !v_v1.is_nan();
    s.put_stack(new_value);
}

#[inline(always)]
fn add_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1 + v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn add_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    add_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn add_float_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 + v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn min_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1 - v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn min_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    min_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn min_float_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 - v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn mul_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1 * v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn mul_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mul_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mul_float_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 * v_v2;
    s.put_stack(new_value);
}

#[inline(always)]
fn div_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 / v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_float_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 / v_v2
    };
    s.put_stack(new_value);
}

#[inline(always)]
fn rem_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 % v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_float_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = {
        let r = v_v1 / v_v2;
        ops::note_format_fault(1, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn div_float_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    div_float_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn div_float_nullable_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        let r = v_v1 / v_v2;
        ops::note_format_fault(1, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack(new_value);
}

#[inline(always)]
fn rem_float_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = {
        let r = v_v1 % v_v2;
        ops::note_format_fault(2, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn rem_float_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rem_float_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = (v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn eq_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = !((v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1));
    s.put_stack_m::<F, _>(new_value);
}

fn ne_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = {
        let _a = v_v1;
        let _b = v_v2;
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let _lt = !_b.is_nan() && !(_a >= _b);
        _lt
    };
    s.put_stack_m::<F, _>(new_value);
}

fn lt_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    lt_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_float_h(s: &mut Hot) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        let _a = v_v1;
        let _b = v_v2;
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let _lt = !_b.is_nan() && !(_a >= _b);
        _lt
    };
    s.put_stack(new_value);
}

#[inline(always)]
fn le_float<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, f64>();
    let new_value = v_v1.is_nan() || (!v_v2.is_nan() && v_v1 <= v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn le_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    le_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_float<const F: bool>(s: &mut State) {
    s.format_float();
}

fn format_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_stack_float<const F: bool>(s: &mut State) {
    s.format_stack_float();
}

fn format_stack_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_stack_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_text<const F: bool>(s: &mut State) {
    s.var_text();
}

fn var_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn arg_text<const F: bool>(s: &mut State) {
    s.arg_text();
}

fn arg_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    arg_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_text<const F: bool>(s: &mut State) {
    s.string_from_code();
}

fn const_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_text_from_null<const F: bool>(s: &mut State) {
    let new_value = Str::new(crate::state::STRING_NULL);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_text_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_text_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = {
        let __t = v_v1.str();
        if __t == crate::state::STRING_NULL {
            0
        } else {
            __t.chars().count() as i64
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn length_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = {
        let __t = v_v1.str();
        if __t == crate::state::STRING_NULL {
            0
        } else {
            __t.len() as i64
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn size_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_character<const F: bool>(s: &mut State) {
    s.length_character();
}

fn length_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_text<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str() != crate::state::STRING_NULL;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn init_text<const F: bool>(s: &mut State) {
    s.init_text();
}

fn init_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    init_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_text<const F: bool>(s: &mut State) {
    s.append_text();
}

fn append_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_text<const F: bool>(s: &mut State) {
    s.put_text();
}

fn put_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_text_sub<const F: bool>(s: &mut State) {
    s.get_text_sub();
}

fn get_text_sub_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_text_sub::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn text_character<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.string();
    let new_value = s.text_char_or_raise(v_v1.str(), v_v2);
    s.put_stack_m::<F, _>(new_value);
}

fn text_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn text_character_nullable<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, i64>();
    let v_v1 = s.string();
    let new_value = {
        let ch = ops::text_character(v_v1.str(), v_v2);
        ops::note_format_fault(
            3,
            ch == char::from(0) && v_v2 != i64::MIN && !v_v1.str().is_empty(),
        );
        ch
    };
    s.put_stack_m::<F, _>(new_value);
}

fn text_character_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_character_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_character<const F: bool>(s: &mut State) {
    let v_v1 = char::from_u32(s.get_stack_m::<F, u32>()).unwrap_or('\0');
    let new_value = ops::op_conv_bool_from_character(v_v1);
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn clear_text<const F: bool>(s: &mut State) {
    s.clear_text();
}

fn clear_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    clear_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_text<const F: bool>(s: &mut State) {
    s.free_text();
}

fn free_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_text<const F: bool>(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_eq_text(v_v1.str(), v_v2.str());
    s.put_stack_m::<F, _>(new_value);
}

fn eq_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_text<const F: bool>(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_ne_text(v_v1.str(), v_v2.str());
    s.put_stack_m::<F, _>(new_value);
}

fn ne_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn lt_text<const F: bool>(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_lt_text(v_v1.str(), v_v2.str());
    s.put_stack_m::<F, _>(new_value);
}

fn lt_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    lt_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn le_text<const F: bool>(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_le_text(v_v1.str(), v_v2.str());
    s.put_stack_m::<F, _>(new_value);
}

fn le_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    le_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_text<const F: bool>(s: &mut State) {
    s.format_text();
}

fn format_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_stack_text<const F: bool>(s: &mut State) {
    s.format_stack_text();
}

fn format_stack_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_stack_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_character<const F: bool>(s: &mut State) {
    s.append_character();
}

fn append_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn text_compare<const F: bool>(s: &mut State) {
    s.text_compare();
}

fn text_compare_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_compare::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_character_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i32>();
    let new_value = if let Some(c) = char::from_u32(v_v1 as u32) {
        c
    } else {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
        char::from(0)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_character_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_character_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_int_from_character<const F: bool>(s: &mut State) {
    let v_v1 = char::from_u32(s.get_stack_m::<F, u32>()).unwrap_or('\0');
    let new_value = if v_v1 == char::from(0) {
        i64::MIN
    } else {
        i64::from(v_v1 as u32)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn conv_int_from_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_int_from_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, u8>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_val = operands.get::<u8>(0);
    let new_value = v_val;
    s.put_stack_m::<F, _>(new_value);
}

fn const_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, u8>();
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_enum<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = v_v1 != 255 && v_v1 != 0;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_text_from_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_enum_tp = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = Str::new(s.database.enum_val(v_enum_tp, v_v1));
    s.put_stack_m::<F, _>(new_value);
}

fn cast_text_from_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_text_from_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_enum_from_text<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_enum_tp = operands.get::<u16>(0);
    let v_v1 = s.string();
    let new_value = s.database.to_enum(v_enum_tp, v_v1.str());
    s.put_stack_m::<F, _>(new_value);
}

fn cast_enum_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_enum_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_int_from_enum<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = if v_v1 == 255 {
        i64::MIN
    } else {
        i64::from(v_v1)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn conv_int_from_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_int_from_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_enum_from_int<const F: bool>(s: &mut State) {
    let v_v1 = s.get_stack_m::<F, i64>();
    let new_value = if v_v1 == i64::MIN { 255 } else { v_v1 as u8 };
    s.put_stack_m::<F, _>(new_value);
}

fn cast_enum_from_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_enum_from_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_enum_from_null<const F: bool>(s: &mut State) {
    let new_value = 255u8;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_enum_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_enum_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn database<const F: bool>(s: &mut State) {
    s.database();
}

fn database_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    database::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_database<const F: bool>(s: &mut State) {
    s.format_database();
}

fn format_database_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_database::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn format_stack_database<const F: bool>(s: &mut State) {
    s.format_stack_database();
}

fn format_stack_database_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    format_stack_database::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_bool_from_ref<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, DbRef>();
    let new_value = v_val.rec != 0;
    s.put_stack_m::<F, _>(new_value);
}

fn conv_bool_from_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_bool_from_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn conv_ref_from_null<const F: bool>(s: &mut State) {
    let new_value = s.database.null();
    s.put_stack_m::<F, _>(new_value);
}

fn conv_ref_from_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    conv_ref_from_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn init_ref<const F: bool>(s: &mut State) {
    s.init_ref();
}

fn init_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    init_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn null_ref_sentinel<const F: bool>(s: &mut State) {
    let new_value = DbRef::NULL;
    s.put_stack_m::<F, _>(new_value);
}

fn null_ref_sentinel_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    null_ref_sentinel::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn init_ref_sentinel<const F: bool>(s: &mut State) {
    s.init_ref_sentinel();
}

fn init_ref_sentinel_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    init_ref_sentinel::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_ref<const F: bool>(s: &mut State) {
    s.free_ref();
}

fn free_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn store_tag<const F: bool>(s: &mut State) {
    s.store_tag();
}

fn store_tag_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    store_tag::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_ref_tag<const F: bool>(s: &mut State) {
    s.free_ref_tag();
}

fn free_ref_tag_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_ref_tag::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_ref_if_distinct<const F: bool>(s: &mut State) {
    let v_witness = s.get_stack_m::<F, DbRef>();
    let v_placeholder = s.get_stack_m::<F, DbRef>();
    s.database.free_displaced(&v_placeholder, &v_witness);
}

fn free_ref_if_distinct_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_ref_if_distinct::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_ref_or_hand_up<const F: bool>(s: &mut State) {
    let v_witness = s.get_stack_m::<F, DbRef>();
    let v_placeholder = s.get_stack_m::<F, DbRef>();
    if v_placeholder.store_nr == v_witness.store_nr {
        s.hand_up_returned(v_witness);
    } else {
        s.database.free(&v_placeholder);
    }
}

fn free_ref_or_hand_up_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_ref_or_hand_up::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_ref_unless_entry<const F: bool>(s: &mut State) {
    let v_entry = s.get_stack_m::<F, DbRef>();
    let v_witness = s.get_stack_m::<F, DbRef>();
    let v_placeholder = s.get_stack_m::<F, DbRef>();
    if v_placeholder.store_nr != v_entry.store_nr {
        s.database.free_displaced(&v_placeholder, &v_witness);
    }
}

fn free_ref_unless_entry_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_ref_unless_entry::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_scratch<const F: bool>(s: &mut State) {
    let v_scratch = s.get_stack_m::<F, DbRef>();
    s.database.free_iteration_scratch(&v_scratch);
}

fn free_scratch_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_scratch::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn sizeof_ref<const F: bool>(s: &mut State) {
    s.sizeof_ref();
}

fn sizeof_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    sizeof_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = {
        let r = s.get_var_m::<F, DbRef>(v_pos);
        s.database.valid(&r);
        r
    };
    s.put_stack_m::<F, _>(new_value);
}

fn var_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let v_value = s.get_stack_m::<F, DbRef>();
    s.put_var_m::<F, _>(v_pos, v_value);
}

fn put_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_ref<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec == 0 && v_v2.rec == 0
    } else {
        v_v1 == v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn eq_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_ref<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec != 0 || v_v2.rec != 0
    } else {
        v_v1 != v_v2
    };
    s.put_stack_m::<F, _>(new_value);
}

fn ne_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_content<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_v2 = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.eq_content(&v_v1, &v_v2, v_tp);
    s.put_stack_m::<F, _>(new_value);
}

fn eq_content_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_content::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_content<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_v2 = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = !s.database.eq_content(&v_v1, &v_v2, v_tp);
    s.put_stack_m::<F, _>(new_value);
}

fn ne_content_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_content::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.get_ref(&v_v1, u32::from(v_fld));
    s.put_stack_m::<F, _>(new_value);
}

fn get_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_u32_raw(db.rec, db.pos + u32::from(v_fld), v.rec);
        }
    }
}

fn set_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_db_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, DbRef>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let r = v_val;
        if db.rec != 0 {
            let off = db.pos + u32::from(v_fld);
            let store = s.database.store_mut(&db);
            store.set_u32_raw(db.rec, off, u32::from(r.store_nr));
            store.set_u32_raw(db.rec, off + 4, r.rec);
            store.set_u32_raw(db.rec, off + 8, r.pos);
        }
    }
}

fn set_db_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_db_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_db_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            DbRef::NULL
        } else {
            let store = s.database.store(&db);
            let off = db.pos + u32::from(v_fld);
            let store_nr = store.get_u32_raw(db.rec, off) as u16;
            let rec = store.get_u32_raw(db.rec, off + 4);
            let pos = store.get_u32_raw(db.rec, off + 8);
            DbRef { store_nr, rec, pos }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_db_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_db_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_field<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = DbRef {
        store_nr: v_v1.store_nr,
        rec: v_v1.rec,
        pos: v_v1.pos + u32::from(v_fld),
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_field_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_field::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_int<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            s.database
                .store(&db)
                .get_int(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_character<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            char::from(0)
        } else {
            char::from_u32(
                s.database
                    .store(&db)
                    .get_u32_raw(db.rec, db.pos + u32::from(v_fld)),
            )
            .unwrap_or(char::from(0))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_single<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            f32::NAN
        } else {
            s.database
                .store(&db)
                .get_single(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_float<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            f64::NAN
        } else {
            s.database
                .store(&db)
                .get_float(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_byte<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            i64::from(
                s.database
                    .store(&db)
                    .get_byte(db.rec, db.pos + u32::from(v_fld), (v_min)),
            )
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_byte_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_byte::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_byte_nullable<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .get_byte(db.rec, db.pos + u32::from(v_fld), (v_min));
            if r == (v_min) + 255 {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_byte_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_byte_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            0u8
        } else {
            let r = s
                .database
                .store(&db)
                .get_byte(db.rec, db.pos + u32::from(v_fld), 0);
            if r < 0 { 255u8 } else { r as u8 }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_enum<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, u8>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_byte(db.rec, db.pos + u32::from(v_fld), 0, i32::from(v));
        }
    }
}

fn set_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_boolean<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            255u8
        } else {
            let r = s
                .database
                .store(&db)
                .get_byte(db.rec, db.pos + u32::from(v_fld), 0);
            if r < 0 { 255u8 } else { r as u8 }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_boolean_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_boolean::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_boolean<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, u8>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_byte(db.rec, db.pos + u32::from(v_fld), 0, i32::from(v));
        }
    }
}

fn set_boolean_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_boolean::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_short<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .get_short(db.rec, db.pos + u32::from(v_fld), (v_min));
            if r == i32::MIN {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_short_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_short::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_text<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            Str::new(crate::state::STRING_NULL)
        } else {
            let store = s.database.store(&db);
            Str::new(store.get_str(store.get_u32_raw(db.rec, db.pos + u32::from(v_fld))))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_int<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_int(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn set_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_character<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = char::from_u32(s.get_stack_m::<F, u32>()).unwrap_or('\0');
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_u32_raw(db.rec, db.pos + u32::from(v_fld), v as u32);
        }
    }
}

fn set_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_single<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, f32>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_single(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn set_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_float<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, f64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_float(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn set_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_byte<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database.store_mut(&db).set_byte(
                db.rec,
                db.pos + u32::from(v_fld),
                (v_min),
                v as i32,
            );
        }
    }
}

fn set_byte_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_byte::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_byte_nullable<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .set_byte_nullable(&db, db.pos + u32::from(v_fld), (v_min), v);
        }
    }
}

fn set_byte_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_byte_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_short<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .set_short_nullable(&db, db.pos + u32::from(v_fld), (v_min), v);
        }
    }
}

fn set_short_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_short::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_int4<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .get_i32_raw(db.rec, db.pos + u32::from(v_fld));
            if r == i32::MIN {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_int4_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_int4::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_int4<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_i32_raw(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn set_int4_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_int4::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_int4_raw<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .get_u32_raw(db.rec, db.pos + u32::from(v_fld));
            if r == u32::MAX {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_int4_raw_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_int4_raw::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_int4_raw<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            u32::MAX
        } else {
            v_val as u32
        };
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_u32_raw(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn set_int4_raw_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_int4_raw::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_int4_full<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            i64::from(
                s.database
                    .store(&db)
                    .get_u32_raw(db.rec, db.pos + u32::from(v_fld)),
            )
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_int4_full_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_int4_full::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_short_raw<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .get_i16_raw(db.rec, db.pos + u32::from(v_fld), (v_min));
            if r == i32::MIN {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_short_raw_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_short_raw::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_short_raw<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_val = s.get_stack_m::<F, i64>();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_i16_raw(db.rec, db.pos + u32::from(v_fld), (v_min), v);
        }
    }
}

fn set_short_raw_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_short_raw::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_short_spare<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s
                .database
                .store(&db)
                .read::<u16>(db.rec, db.pos + u32::from(v_fld));
            crate::narrow::dec_short_spare(r, (v_min))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_short_spare_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_short_spare::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_short_full<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_fld = operands.get::<u16>(0);
    let v_min = operands.get::<i32>(2);
    let v_v1 = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            i64::from(s.database.store(&db).get_short_full(
                db.rec,
                db.pos + u32::from(v_fld),
                (v_min),
            ))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_short_full_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_short_full::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_text<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fld = operands.get::<u16>(0);
    let v_val = s.string();
    let v_v1 = s.get_stack_m::<F, DbRef>();
    {
        let db = v_v1;
        let s_val = v_val.str().to_string();
        if db.rec != 0 {
            let store = s.database.store_mut(&db);
            let s_pos = store.set_str(&s_val);
            store.set_u32_raw(db.rec, db.pos + u32::from(v_fld), s_pos);
        }
    }
}

fn set_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, DbRef>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn tag_fault<const F: bool>(s: &mut State) {
    let operands = s.operands(1);
    let v_kind = operands.get::<u8>(0);
    {
        let _ = v_kind;
        ops::arm_format_fault();
    }
}

fn tag_fault_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    tag_fault::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_vector<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(vector::length_vector(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vector_is_null<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = vector::is_absent_collection(&v_r, &s.database.allocations);
    s.put_stack_m::<F, _>(new_value);
}

fn vector_is_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vector_is_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ref_is_null<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = v_r.store_nr == u16::MAX;
    s.put_stack_m::<F, _>(new_value);
}

fn ref_is_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ref_is_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn distinct_store<const F: bool>(s: &mut State) {
    let v_b = s.get_stack_m::<F, DbRef>();
    let v_a = s.get_stack_m::<F, DbRef>();
    let new_value = v_a.store_nr != v_b.store_nr;
    s.put_stack_m::<F, _>(new_value);
}

fn distinct_store_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    distinct_store::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ref_alias<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = v_r;
    s.put_stack_m::<F, _>(new_value);
}

fn ref_alias_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ref_alias::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_stride = operands.get::<u16>(0);
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value =
        i64::from(vector::length_vector(&v_r, &s.database.allocations)) * i64::from(v_stride);
    s.put_stack_m::<F, _>(new_value);
}

fn size_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_struct<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_sz = operands.get::<u16>(0);
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let _ = v_r;
        i64::from(v_sz)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn size_struct_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_struct::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_scalar<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_sz = operands.get::<u16>(0);
    let v_v = s.get_stack_m::<F, i64>();
    let new_value = {
        let _ = v_v;
        i64::from(v_sz)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn size_scalar_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_scalar::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_sorted<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(vector::length_vector(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_sorted_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_sorted::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn clear_vector<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.clear_vector_release(&v_r);
}

fn clear_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    clear_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_size = operands.get::<u16>(0);
    let v_index = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let __vr = v_r;
        let __vi = v_index;
        s.vec_get_or_raise(&__vr, u32::from(v_size), __vi)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vector_ref<const F: bool>(s: &mut State) {
    let v_index = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let __vr = v_r;
        let __vi = v_index;
        s.vec_ref_or_raise(&__vr, __vi)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn vector_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vector_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_vector_nullable<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_size = operands.get::<u16>(0);
    let v_index = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let el = vector::get_vector(&v_r, u32::from(v_size), v_index, &s.database.allocations);
        ops::note_format_fault(3, el.rec == 0 && v_index != i64::MIN && !v_r.is_null());
        el
    };
    s.put_stack_m::<F, _>(new_value);
}

fn get_vector_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_vector_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vector_ref_nullable<const F: bool>(s: &mut State) {
    let v_index = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = {
        let el = vector::get_vector(&v_r, 4, v_index, &s.database.allocations);
        ops::note_format_fault(3, el.rec == 0 && v_index != i64::MIN && !v_r.is_null());
        s.database.get_ref(&el, 0)
    };
    s.put_stack_m::<F, _>(new_value);
}

fn vector_ref_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vector_ref_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cast_vector_from_text<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_val = s.string();
    let new_value = s.db_from_text(v_val.str(), v_db_tp);
    s.put_stack_m::<F, _>(new_value);
}

fn cast_vector_from_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cast_vector_from_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn remove_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_index = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.remove_vector_at(&v_r, v_tp, v_index);
    s.put_stack_m::<F, _>(new_value);
}

fn remove_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    remove_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn keep_vector_range<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_hi = s.get_stack_m::<F, i64>();
    let v_lo = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.keep_vector_range(&v_r, v_tp, v_lo, v_hi)
}

fn keep_vector_range_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    keep_vector_range::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_text_bytes<const F: bool>(s: &mut State) {
    let v_hi = s.get_stack_m::<F, i64>();
    let v_lo = s.get_stack_m::<F, i64>();
    let v_t = s.string();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_text_bytes(&v_r, v_t.str(), v_lo, v_hi)
}

fn append_text_bytes_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_text_bytes::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn insert_vector<const F: bool>(s: &mut State) {
    s.insert_vector();
}

fn insert_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    insert_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn new_record<const F: bool>(s: &mut State) {
    s.new_record();
}

fn new_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    new_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn finish_record<const F: bool>(s: &mut State) {
    s.finish_record();
}

fn finish_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    finish_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_other = s.get_stack_m::<F, DbRef>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_add(&v_r, &v_other, v_tp);
}

fn append_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn slice_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_hi = s.get_stack_m::<F, i64>();
    let v_lo = s.get_stack_m::<F, i64>();
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_slice(&v_r, &v_src, v_lo, v_hi, v_tp);
}

fn slice_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    slice_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn slice_view<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_hi = s.get_stack_m::<F, i64>();
    let v_lo = s.get_stack_m::<F, i64>();
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_slice_view(&v_r, &v_src, v_lo, v_hi, v_tp);
}

fn slice_view_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    slice_view::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn keep_range<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_hi = s.get_stack_m::<F, i64>();
    let v_lo = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_keep_range(&v_r, v_lo, v_hi, v_tp);
}

fn keep_range_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    keep_range::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn adopt_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_adopt(&v_r, &v_src, v_tp);
}

fn adopt_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    adopt_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_int<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_i64(&v_r, v_val);
}

fn push_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_int4<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    {
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        s.database.append_i32(&v_r, v);
    }
}

fn push_int4_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_int4::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_float<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, f64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_f64(&v_r, v_val);
}

fn push_float_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_float::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_single<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, f32>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_f32(&v_r, v_val);
}

fn push_single_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_single::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_boolean<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, u8>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_byte(&v_r, i32::from(v_val));
}

fn push_boolean_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_boolean::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_enum<const F: bool>(s: &mut State) {
    let v_val = s.get_stack_m::<F, u8>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_byte(&v_r, i32::from(v_val));
}

fn push_enum_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_enum::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_character<const F: bool>(s: &mut State) {
    let v_val = char::from_u32(s.get_stack_m::<F, u32>()).unwrap_or('\0');
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_u32(&v_r, v_val as u32);
}

fn push_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn push_byte<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_min = operands.get::<i32>(0);
    let v_val = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.append_byte_min(&v_r, v_min, v_val as i32);
}

fn push_byte_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    push_byte::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn replace_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_other = s.get_stack_m::<F, DbRef>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.vector_replace(&v_r, &v_other, v_tp);
}

fn replace_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    replace_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn claim_child_rec<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_field = s.get_stack_m::<F, DbRef>();
    s.database.claim_child_rec(&v_field, &v_src, v_tp);
}

fn claim_child_rec_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    claim_child_rec::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ref_from_child_rec<const F: bool>(s: &mut State) {
    let v_field = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.ref_from_child_rec(&v_field);
    s.put_stack_m::<F, _>(new_value);
}

fn ref_from_child_rec_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ref_from_child_rec::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_record<const F: bool>(s: &mut State) {
    s.get_record();
}

fn get_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn validate<const F: bool>(s: &mut State) {
    s.validate();
}

fn validate_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    validate::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn hash_add<const F: bool>(s: &mut State) {
    s.hash_add();
}

fn hash_add_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    hash_add::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn hash_find<const F: bool>(s: &mut State) {
    s.hash_find();
}

fn hash_find_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    hash_find::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn hash_remove<const F: bool>(s: &mut State) {
    s.hash_remove();
}

fn hash_remove_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    hash_remove::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_hash<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(hash::count(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_hash_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_hash::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn reserve_hash<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_count = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    s.database.reserve_hash(&v_r, v_count, v_db_tp);
}

fn reserve_hash_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    reserve_hash::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_hash<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(hash::table_bytes(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn size_hash_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_hash::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_index<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_fields = operands.get::<u16>(0);
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(tree::count(&v_r, v_fields, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_index_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_index::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn eq_bool<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, u8>();
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = v_v1 == v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn eq_bool_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    eq_bool::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn ne_bool<const F: bool>(s: &mut State) {
    let v_v2 = s.get_stack_m::<F, u8>();
    let v_v1 = s.get_stack_m::<F, u8>();
    let new_value = v_v1 != v_v2;
    s.put_stack_m::<F, _>(new_value);
}

fn ne_bool_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    ne_bool::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn panic<const F: bool>(s: &mut State) {
    let v_message = s.string();
    panic!("{}", v_message.str());
}

fn panic_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    panic::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn print<const F: bool>(s: &mut State) {
    let v_v1 = s.string();
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    crate::loft_host_print(v_v1.str().as_ptr(), v_v1.str().len());
    #[cfg(all(not(feature = "wasm"), not(target_arch = "wasm32")))]
    if !crate::rpc::print_or_capture(v_v1.str()) {
        crate::codegen_runtime::host_print(v_v1.str());
    }
    #[cfg(all(not(feature = "wasm"), target_arch = "wasm32", target_os = "wasi"))]
    crate::codegen_runtime::host_print(v_v1.str());
    #[cfg(feature = "wasm")]
    crate::wasm::output_push(v_v1.str());
}

fn print_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    print::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn iterate<const F: bool>(s: &mut State) {
    s.iterate();
}

fn iterate_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    iterate::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn step<const F: bool>(s: &mut State) {
    s.step();
}

fn step_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    step::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn remove<const F: bool>(s: &mut State) {
    s.remove();
}

fn remove_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    remove::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn clear<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_data = s.get_stack_m::<F, DbRef>();
    s.database.remove_claims(&v_data, v_tp);
}

fn clear_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    clear::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_copy<const F: bool>(s: &mut State) {
    s.append_copy();
}

fn append_copy_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_copy::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn copy_record<const F: bool>(s: &mut State) {
    s.copy_record();
}

fn copy_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    copy_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn place_record<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_host = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.place_record_prefilled(&v_host, v_tp);
    s.put_stack_m::<F, _>(new_value);
}

fn place_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    place_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn move_record<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_dest = s.get_stack_m::<F, DbRef>();
    let v_data = s.get_stack_m::<F, DbRef>();
    s.database.move_record_out(&v_data, &v_dest, v_tp);
}

fn move_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    move_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn move_field<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_dest = s.get_stack_m::<F, DbRef>();
    let v_data = s.get_stack_m::<F, DbRef>();
    s.database.move_field_out(&v_data, &v_dest, v_tp);
}

fn move_field_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    move_field::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn move_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_dest = s.get_stack_m::<F, DbRef>();
    s.database.move_vector(&v_dest, &v_src, v_tp);
}

fn move_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    move_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn free_record_in<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_rec = s.get_stack_m::<F, DbRef>();
    s.database.free_record_in(&v_rec, v_tp);
}

fn free_record_in_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    free_record_in::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn copy_ref_or_null<const F: bool>(s: &mut State) {
    s.copy_ref_or_null();
}

fn copy_ref_or_null_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    copy_ref_or_null::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn bind_or_copy<const F: bool>(s: &mut State) {
    s.bind_or_copy();
}

fn bind_or_copy_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    bind_or_copy::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn replace_keyed<const F: bool>(s: &mut State) {
    s.replace_keyed();
}

fn replace_keyed_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    replace_keyed::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn clear_keyed<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_tp = operands.get::<u16>(0);
    let v_dest = s.get_stack_m::<F, DbRef>();
    s.database.remove_claims_keyed(&v_dest, v_tp);
}

fn clear_keyed_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    clear_keyed::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn index_group<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_tp = operands.get::<u16>(0);
    let v_parent_tp = operands.get::<u16>(2);
    let v_fld = operands.get::<u16>(4);
    let v_view = s.get_stack_m::<F, DbRef>();
    let v_primary = s.get_stack_m::<F, DbRef>();
    s.database
        .index_group_records(&v_primary, &v_view, v_tp, v_parent_tp, v_fld);
}

fn index_group_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    index_group::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn link_record<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_parent_tp = operands.get::<u16>(0);
    let v_fld = operands.get::<u16>(2);
    let v_rec = s.get_stack_m::<F, DbRef>();
    let v_data = s.get_stack_m::<F, DbRef>();
    s.database
        .link_record_siblings(&v_data, &v_rec, v_parent_tp, v_fld);
}

fn link_record_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    link_record::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn fill_keyed<const F: bool>(s: &mut State) {
    let operands = s.operands(6);
    let v_tp = operands.get::<u16>(0);
    let v_parent_tp = operands.get::<u16>(2);
    let v_field = operands.get::<u16>(4);
    let v_src = s.get_stack_m::<F, DbRef>();
    let v_parent = s.get_stack_m::<F, DbRef>();
    s.database
        .fill_keyed_from_vector(&v_parent, &v_src, v_tp, v_parent_tp, v_field);
}

fn fill_keyed_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    fill_keyed::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_keyed<const F: bool>(s: &mut State) {
    s.set_keyed();
}

fn set_keyed_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_keyed::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_spatial<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(codegen_runtime::spatial_len(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_spatial_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_spatial::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn length_trie<const F: bool>(s: &mut State) {
    let v_r = s.get_stack_m::<F, DbRef>();
    let new_value = i64::from(codegen_runtime::trie_len(&v_r, &s.database.allocations));
    s.put_stack_m::<F, _>(new_value);
}

fn length_trie_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    length_trie::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn static_call<const F: bool>(s: &mut State) {
    s.static_call();
}

fn static_call_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    static_call::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn create_stack<const F: bool>(s: &mut State) {
    s.create_stack();
}

fn create_stack_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    create_stack::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn init_create_stack<const F: bool>(s: &mut State) {
    s.init_create_stack();
}

fn init_create_stack_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    init_create_stack::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_stack_text<const F: bool>(s: &mut State) {
    s.get_stack_text();
}

fn get_stack_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_stack_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_stack_ref<const F: bool>(s: &mut State) {
    s.get_stack_ref();
}

fn get_stack_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_stack_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_stack_ref<const F: bool>(s: &mut State) {
    s.set_stack_ref();
}

fn set_stack_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_stack_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn set_stack_fn_ref<const F: bool>(s: &mut State) {
    s.set_stack_fn_ref();
}

fn set_stack_fn_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    set_stack_fn_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_stack_fn_ref<const F: bool>(s: &mut State) {
    s.get_stack_fn_ref();
}

fn get_stack_fn_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_stack_fn_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn drop_fn_ref<const F: bool>(s: &mut State) {
    s.drop_fn_ref();
}

fn drop_fn_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    drop_fn_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn fn_ref_detach_shared<const F: bool>(s: &mut State) {
    s.fn_ref_detach_shared();
}

fn fn_ref_detach_shared_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    fn_ref_detach_shared::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn fn_ref_closure<const F: bool>(s: &mut State) {
    s.fn_ref_closure();
}

fn fn_ref_closure_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    fn_ref_closure::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_stack_text<const F: bool>(s: &mut State) {
    s.append_stack_text();
}

fn append_stack_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_stack_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn append_stack_character<const F: bool>(s: &mut State) {
    s.append_stack_character();
}

fn append_stack_character_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    append_stack_character::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn clear_stack_text<const F: bool>(s: &mut State) {
    s.clear_stack_text();
}

fn clear_stack_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    clear_stack_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn parallel_begin<const F: bool>(s: &mut State) {
    s.parallel_begin();
}

fn parallel_begin_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    parallel_begin::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn parallel_arm<const F: bool>(s: &mut State) {
    s.parallel_arm();
}

fn parallel_arm_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    parallel_arm::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn parallel_join<const F: bool>(s: &mut State) {
    s.parallel_join();
}

fn parallel_join_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    parallel_join::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn pre_alloc_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_capacity = operands.get::<u16>(0);
    let v_elem_size = operands.get::<u16>(2);
    let v_r = s.get_stack_m::<F, DbRef>();
    vector::pre_alloc_vector(
        &v_r,
        u32::from(v_capacity),
        u32::from(v_elem_size),
        &mut s.database.allocations,
    );
}

fn pre_alloc_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    pre_alloc_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn reserve_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_elem_size = operands.get::<u16>(0);
    let v_count = s.get_stack_m::<F, i64>();
    let v_r = s.get_stack_m::<F, DbRef>();
    vector::reserve_vector(
        &v_r,
        v_count,
        u32::from(v_elem_size),
        &mut s.database.allocations,
    );
}

fn reserve_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    reserve_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_file<const F: bool>(s: &mut State) {
    let v_file = s.get_stack_m::<F, DbRef>();
    let new_value = s.database.get_file(&v_file);
    s.put_stack_m::<F, _>(new_value);
}

fn get_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_dir<const F: bool>(s: &mut State) {
    let v_result = s.get_stack_m::<F, DbRef>();
    let v_path = s.string();
    let new_value = s.database.get_dir(v_path.str(), &v_result);
    s.put_stack_m::<F, _>(new_value);
}

fn get_dir_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_dir::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn get_file_text<const F: bool>(s: &mut State) {
    s.get_file_text();
}

fn get_file_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    get_file_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn write_file<const F: bool>(s: &mut State) {
    s.write_file();
}

fn write_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    write_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn read_file<const F: bool>(s: &mut State) {
    s.read_file();
}

fn read_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    read_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn seek_file<const F: bool>(s: &mut State) {
    s.seek_file();
}

fn seek_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    seek_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn size_file<const F: bool>(s: &mut State) {
    s.size_file();
}

fn size_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    size_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn delete<const F: bool>(s: &mut State) {
    let v_path = s.string();
    let new_value = s.database.fs_delete_at(v_path.str());
    s.put_stack_m::<F, _>(new_value);
}

fn delete_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    delete::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn move_file<const F: bool>(s: &mut State) {
    let v_to = s.string();
    let v_from = s.string();
    let new_value = s.database.fs_move_at(v_from.str(), v_to.str());
    s.put_stack_m::<F, _>(new_value);
}

fn move_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    move_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn truncate_file<const F: bool>(s: &mut State) {
    s.truncate_file();
}

fn truncate_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    truncate_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn sync_file<const F: bool>(s: &mut State) {
    s.sync_file();
}

fn sync_file_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    sync_file::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn deliver<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, DbRef>();
    let v_tag = s.get_stack_m::<F, i64>();
    s.database.deliver_reconstruct(v_tag, v_val, v_db_tp);
}

fn deliver_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    deliver::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn expose<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, DbRef>();
    let v_tag = s.get_stack_m::<F, i64>();
    s.database.expose_value(v_tag, v_val, v_db_tp);
}

fn expose_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    expose::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn release<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_val = s.get_stack_m::<F, DbRef>();
    let v_tag = s.get_stack_m::<F, i64>();
    s.database.release_value(v_tag, v_val, v_db_tp);
}

fn release_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    release::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn call_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(4);
    let v_fn_var = operands.get::<u16>(0);
    let v_arg_size = operands.get::<u16>(2);
    s.fn_call_ref(v_fn_var, v_arg_size);
}

fn call_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    call_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mkdir<const F: bool>(s: &mut State) {
    let v_path = s.string();
    let new_value = s.database.fs_mkdir_at(v_path.str());
    s.put_stack_m::<F, _>(new_value);
}

fn mkdir_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mkdir::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn mkdir_all<const F: bool>(s: &mut State) {
    let v_path = s.string();
    let new_value = s.database.fs_mkdir_all_at(v_path.str());
    s.put_stack_m::<F, _>(new_value);
}

fn mkdir_all_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    mkdir_all::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn rmdir<const F: bool>(s: &mut State) {
    let v_path = s.string();
    let new_value = s.database.fs_rmdir_at(v_path.str());
    s.put_stack_m::<F, _>(new_value);
}

fn rmdir_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    rmdir::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn reverse_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_size = operands.get::<u16>(0);
    let v_r = s.get_stack_m::<F, DbRef>();
    vector::reverse_vector(&v_r, u32::from(v_size), &mut s.database.allocations);
}

fn reverse_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    reverse_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn sort_vector<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_db_tp = operands.get::<u16>(0);
    let v_r = s.get_stack_m::<F, DbRef>();
    {
        let t = v_db_tp;
        if s.database.is_text_type(t) {
            vector::sort_text_vector(&v_r, &mut s.database.allocations);
        } else {
            let elem_size = s.database.size(t);
            let is_float = t == 2 || t == 3;
            vector::sort_vector(&v_r, elem_size, is_float, &mut s.database.allocations);
        }
    }
}

fn sort_vector_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    sort_vector::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_create<const F: bool>(s: &mut State) {
    let operands = s.operands(18);
    let v_d_nr = operands.get::<i64>(0);
    let v_args_size = operands.get::<u16>(8);
    let v_to = operands.get::<i64>(10);
    s.coroutine_create(v_d_nr as u32, u32::from(v_args_size), v_to as u32);
}

fn coroutine_create_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_create::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_next<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_value_size = operands.get::<u16>(0);
    s.coroutine_next(u32::from(v_value_size));
}

fn coroutine_next_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_next::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_return<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_value_size = operands.get::<u16>(0);
    s.coroutine_return(u32::from(v_value_size));
}

fn coroutine_return_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_return::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_yield<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_value_size = operands.get::<u16>(0);
    s.coroutine_yield(u32::from(v_value_size));
}

fn coroutine_yield_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_yield::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_exhausted<const F: bool>(s: &mut State) {
    let v_gen = s.get_stack_m::<F, DbRef>();
    let new_value = s.coroutine_exhausted(&v_gen);
    s.put_stack_m::<F, _>(new_value);
}

fn coroutine_exhausted_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_exhausted::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn var_fn_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    let new_value = s.get_var_m::<F, [std::mem::MaybeUninit<u8>; 20]>(v_pos);
    s.put_stack_m::<F, _>(new_value);
}

fn var_fn_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    var_fn_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn put_fn_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(2);
    let v_pos = operands.get::<u16>(0);
    {
        let v = s.get_stack_m::<F, [std::mem::MaybeUninit<u8>; 20]>();
        s.put_var_m::<F, _>(v_pos, v);
    }
}

fn put_fn_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    put_fn_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_ref<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_d_nr = operands.get::<i64>(0);
    let new_value = s.const_ref_at(v_d_nr as usize);
    s.put_stack_m::<F, _>(new_value);
}

fn const_ref_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_ref::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn const_store_text<const F: bool>(s: &mut State) {
    let operands = s.operands(16);
    let v_rec = operands.get::<i64>(0);
    let v_pos = operands.get::<i64>(8);
    s.string_from_const_store(v_rec as u32, v_pos as u32)
}

fn const_store_text_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    const_store_text::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn call_ref_store<const F: bool>(s: &mut State) {
    let operands = s.operands(12);
    let v_fn_var = operands.get::<u16>(0);
    let v_arg_size = operands.get::<u16>(2);
    let v_mask = operands.get::<i64>(4);
    s.fn_call_ref_store(v_fn_var, v_arg_size, v_mask as u64);
}

fn call_ref_store_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    call_ref_store::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn bind_fn_ref_result<const F: bool>(s: &mut State) {
    s.bind_fn_ref_result();
}

fn bind_fn_ref_result_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    bind_fn_ref_result::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn coroutine_retain<const F: bool>(s: &mut State) {
    let v_gen = s.get_stack_m::<F, DbRef>();
    let new_value = s.coroutine_retain(v_gen);
    s.put_stack_m::<F, _>(new_value);
}

fn coroutine_retain_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    coroutine_retain::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn int_v_v<const F: bool>(s: &mut State) {
    let operands = s.operands(5);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let new_value = ops::fused_int(
        v_kind,
        s.get_var_m::<F, i64>(v_a),
        s.get_var_m::<F, i64>(v_b),
    );
    s.put_stack_m::<F, _>(new_value);
}

fn int_v_v_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    int_v_v::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn int_v_v_h(s: &mut Hot) {
    let operands = s.operands(5);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let new_value = ops::fused_int(v_kind, s.get_var::<i64>(v_a), s.get_var::<i64>(v_b));
    s.put_stack(new_value);
}

#[inline(always)]
fn int_v_c<const F: bool>(s: &mut State) {
    let operands = s.operands(11);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let new_value = ops::fused_int(v_kind, s.get_var_m::<F, i64>(v_a), v_c);
    s.put_stack_m::<F, _>(new_value);
}

fn int_v_c_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    int_v_c::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn int_v_c_h(s: &mut Hot) {
    let operands = s.operands(11);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let new_value = ops::fused_int(v_kind, s.get_var::<i64>(v_a), v_c);
    s.put_stack(new_value);
}

#[inline(always)]
fn cmp_int_v_v<const F: bool>(s: &mut State) {
    let operands = s.operands(5);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let new_value = ops::fused_cmp(
        v_kind,
        s.get_var_m::<F, i64>(v_a),
        s.get_var_m::<F, i64>(v_b),
    );
    s.put_stack_m::<F, _>(new_value);
}

fn cmp_int_v_v_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cmp_int_v_v::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cmp_int_v_v_h(s: &mut Hot) {
    let operands = s.operands(5);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let new_value = ops::fused_cmp(v_kind, s.get_var::<i64>(v_a), s.get_var::<i64>(v_b));
    s.put_stack(new_value);
}

#[inline(always)]
fn cmp_int_v_c<const F: bool>(s: &mut State) {
    let operands = s.operands(11);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let new_value = ops::fused_cmp(v_kind, s.get_var_m::<F, i64>(v_a), v_c);
    s.put_stack_m::<F, _>(new_value);
}

fn cmp_int_v_c_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cmp_int_v_c::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cmp_int_v_c_h(s: &mut Hot) {
    let operands = s.operands(11);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let new_value = ops::fused_cmp(v_kind, s.get_var::<i64>(v_a), v_c);
    s.put_stack(new_value);
}

#[inline(always)]
fn int_v_v_put<const F: bool>(s: &mut State) {
    let operands = s.operands(7);
    let v_kind = operands.get::<u8>(0);
    let v_dst = operands.get::<u16>(1);
    let v_a = operands.get::<u16>(3);
    let v_b = operands.get::<u16>(5);
    {
        let r = ops::fused_int(
            v_kind,
            s.get_var_m::<F, i64>(v_a),
            s.get_var_m::<F, i64>(v_b),
        );
        s.put_var_m::<F, _>(v_dst, r);
    }
}

fn int_v_v_put_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    int_v_v_put::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn int_v_v_put_h(s: &mut Hot) {
    let operands = s.operands(7);
    let v_kind = operands.get::<u8>(0);
    let v_dst = operands.get::<u16>(1);
    let v_a = operands.get::<u16>(3);
    let v_b = operands.get::<u16>(5);
    {
        let r = ops::fused_int(v_kind, s.get_var::<i64>(v_a), s.get_var::<i64>(v_b));
        s.put_var(v_dst, r);
    }
}

#[inline(always)]
fn int_v_c_put<const F: bool>(s: &mut State) {
    let operands = s.operands(13);
    let v_kind = operands.get::<u8>(0);
    let v_dst = operands.get::<u16>(1);
    let v_a = operands.get::<u16>(3);
    let v_c = operands.get::<i64>(5);
    {
        let r = ops::fused_int(v_kind, s.get_var_m::<F, i64>(v_a), v_c);
        s.put_var_m::<F, _>(v_dst, r);
    }
}

fn int_v_c_put_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    int_v_c_put::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn int_v_c_put_h(s: &mut Hot) {
    let operands = s.operands(13);
    let v_kind = operands.get::<u8>(0);
    let v_dst = operands.get::<u16>(1);
    let v_a = operands.get::<u16>(3);
    let v_c = operands.get::<i64>(5);
    {
        let r = ops::fused_int(v_kind, s.get_var::<i64>(v_a), v_c);
        s.put_var(v_dst, r);
    }
}

#[inline(always)]
fn cmp_int_v_v_jump<const F: bool>(s: &mut State) {
    let operands = s.operands(9);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let v_step = operands.get::<i32>(5);
    if !ops::fused_cmp(
        v_kind,
        s.get_var_m::<F, i64>(v_a),
        s.get_var_m::<F, i64>(v_b),
    ) {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

fn cmp_int_v_v_jump_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cmp_int_v_v_jump::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cmp_int_v_v_jump_h(s: &mut Hot) {
    let operands = s.operands(9);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_b = operands.get::<u16>(3);
    let v_step = operands.get::<i32>(5);
    if !ops::fused_cmp(v_kind, s.get_var::<i64>(v_a), s.get_var::<i64>(v_b)) {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

#[inline(always)]
fn cmp_int_v_c_jump<const F: bool>(s: &mut State) {
    let operands = s.operands(15);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let v_step = operands.get::<i32>(11);
    if !ops::fused_cmp(v_kind, s.get_var_m::<F, i64>(v_a), v_c) {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

fn cmp_int_v_c_jump_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    cmp_int_v_c_jump::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn cmp_int_v_c_jump_h(s: &mut Hot) {
    let operands = s.operands(15);
    let v_kind = operands.get::<u8>(0);
    let v_a = operands.get::<u16>(1);
    let v_c = operands.get::<i64>(3);
    let v_step = operands.get::<i32>(11);
    if !ops::fused_cmp(v_kind, s.get_var::<i64>(v_a), v_c) {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

#[inline(always)]
fn text_walk_step<const F: bool>(s: &mut State) {
    s.text_walk_step();
}

fn text_walk_step_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_walk_step::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn text_null_jump<const F: bool>(s: &mut State) {
    s.text_null_jump();
}

fn text_null_jump_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_null_jump::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn text_end_jump<const F: bool>(s: &mut State) {
    s.text_end_jump();
}

fn text_end_jump_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    text_end_jump::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vec_get_int<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_vec = operands.get::<u16>(0);
    let v_size = operands.get::<u16>(2);
    let v_idx = operands.get::<u16>(4);
    let v_fld = operands.get::<u16>(6);
    let new_value = {
        let r = s.get_var_m::<F, DbRef>(v_vec);
        let i = s.get_var_m::<F, i64>(v_idx);
        let db = s.vec_get_or_raise(&r, u32::from(v_size), i);
        if db.rec == 0 {
            i64::MIN
        } else {
            s.database
                .store(&db)
                .get_int(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn vec_get_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vec_get_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vec_get_int_nullable<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_vec = operands.get::<u16>(0);
    let v_size = operands.get::<u16>(2);
    let v_idx = operands.get::<u16>(4);
    let v_fld = operands.get::<u16>(6);
    let new_value = {
        let r = s.get_var_m::<F, DbRef>(v_vec);
        let i = s.get_var_m::<F, i64>(v_idx);
        let db = vector::get_vector(&r, u32::from(v_size), i, &s.database.allocations);
        ops::note_format_fault(3, db.rec == 0 && i != i64::MIN && !r.is_null());
        if db.rec == 0 {
            i64::MIN
        } else {
            s.database
                .store(&db)
                .get_int(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack_m::<F, _>(new_value);
}

fn vec_get_int_nullable_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vec_get_int_nullable::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vec_set_int<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_vec = operands.get::<u16>(0);
    let v_size = operands.get::<u16>(2);
    let v_idx = operands.get::<u16>(4);
    let v_fld = operands.get::<u16>(6);
    let v_val = s.get_stack_m::<F, i64>();
    {
        let v = v_val;
        let r = s.get_var_m::<F, DbRef>(v_vec);
        let i = s.get_var_m::<F, i64>(v_idx);
        let db = s.vec_get_or_raise(&r, u32::from(v_size), i);
        if db.rec != 0 {
            s.database
                .store_mut(&db)
                .set_int(db.rec, db.pos + u32::from(v_fld), v);
        }
    }
}

fn vec_set_int_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vec_set_int::<true>(s);
    s.regs_out()
}

#[inline(always)]
fn vec_end_jump<const F: bool>(s: &mut State) {
    let operands = s.operands(8);
    let v_vec = operands.get::<u16>(0);
    let v_idx = operands.get::<u16>(2);
    let v_step = operands.get::<i32>(4);
    if i64::from(vector::length_vector(
        &s.get_var_m::<F, DbRef>(v_vec),
        &s.database.allocations,
    )) > s.get_var_m::<F, i64>(v_idx)
    {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

fn vec_end_jump_r(s: &mut State, r: Regs) -> Regs {
    s.regs_in(r);
    vec_end_jump::<true>(s);
    s.regs_out()
}

/// The lean loop's dispatch: a `#hot` operator runs inline on the loop's registers
/// ([`Hot`]), every other one through [`OPERATORS_REG`].
#[expect(
    clippy::too_many_lines,
    reason = "one arm per #hot operator: the table IS the dispatch, and splitting it adds a call the inline arms exist to avoid"
)]
#[inline(always)]
pub(crate) fn dispatch_lean(s: &mut State, opcode: u16, r: Regs) -> Regs {
    match opcode {
        0 => {
            let mut h = Hot::new(s, r);
            goto_word_h(&mut h);
            h.finish()
        }
        1 => {
            let mut h = Hot::new(s, r);
            goto_false_word_h(&mut h);
            h.finish()
        }
        2 => {
            let mut h = Hot::new(s, r);
            const_true_h(&mut h);
            h.finish()
        }
        3 => {
            let mut h = Hot::new(s, r);
            const_false_h(&mut h);
            h.finish()
        }
        4 => {
            let mut h = Hot::new(s, r);
            var_bool_h(&mut h);
            h.finish()
        }
        5 => {
            let mut h = Hot::new(s, r);
            const_int_h(&mut h);
            h.finish()
        }
        6 => {
            let mut h = Hot::new(s, r);
            var_int_h(&mut h);
            h.finish()
        }
        7 => {
            let mut h = Hot::new(s, r);
            put_int_h(&mut h);
            h.finish()
        }
        8 => {
            let mut h = Hot::new(s, r);
            conv_float_from_int_h(&mut h);
            h.finish()
        }
        9 => {
            let mut h = Hot::new(s, r);
            add_int_h(&mut h);
            h.finish()
        }
        10 => {
            let mut h = Hot::new(s, r);
            min_int_h(&mut h);
            h.finish()
        }
        11 => {
            let mut h = Hot::new(s, r);
            mul_int_h(&mut h);
            h.finish()
        }
        12 => {
            let mut h = Hot::new(s, r);
            div_int_h(&mut h);
            h.finish()
        }
        13 => {
            let mut h = Hot::new(s, r);
            rem_int_h(&mut h);
            h.finish()
        }
        14 => {
            let mut h = Hot::new(s, r);
            land_int_h(&mut h);
            h.finish()
        }
        15 => {
            let mut h = Hot::new(s, r);
            eq_int_h(&mut h);
            h.finish()
        }
        16 => {
            let mut h = Hot::new(s, r);
            lt_int_h(&mut h);
            h.finish()
        }
        17 => {
            let mut h = Hot::new(s, r);
            le_int_h(&mut h);
            h.finish()
        }
        18 => {
            let mut h = Hot::new(s, r);
            const_float_h(&mut h);
            h.finish()
        }
        19 => {
            let mut h = Hot::new(s, r);
            var_float_h(&mut h);
            h.finish()
        }
        20 => {
            let mut h = Hot::new(s, r);
            put_float_h(&mut h);
            h.finish()
        }
        21 => {
            let mut h = Hot::new(s, r);
            conv_bool_from_float_h(&mut h);
            h.finish()
        }
        22 => {
            let mut h = Hot::new(s, r);
            add_float_h(&mut h);
            h.finish()
        }
        23 => {
            let mut h = Hot::new(s, r);
            min_float_h(&mut h);
            h.finish()
        }
        24 => {
            let mut h = Hot::new(s, r);
            mul_float_h(&mut h);
            h.finish()
        }
        25 => {
            let mut h = Hot::new(s, r);
            div_float_h(&mut h);
            h.finish()
        }
        26 => {
            let mut h = Hot::new(s, r);
            div_float_nullable_h(&mut h);
            h.finish()
        }
        27 => {
            let mut h = Hot::new(s, r);
            lt_float_h(&mut h);
            h.finish()
        }
        28 => {
            let mut h = Hot::new(s, r);
            int_v_v_h(&mut h);
            h.finish()
        }
        29 => {
            let mut h = Hot::new(s, r);
            int_v_c_h(&mut h);
            h.finish()
        }
        30 => {
            let mut h = Hot::new(s, r);
            cmp_int_v_v_h(&mut h);
            h.finish()
        }
        31 => {
            let mut h = Hot::new(s, r);
            cmp_int_v_c_h(&mut h);
            h.finish()
        }
        32 => {
            let mut h = Hot::new(s, r);
            int_v_v_put_h(&mut h);
            h.finish()
        }
        33 => {
            let mut h = Hot::new(s, r);
            int_v_c_put_h(&mut h);
            h.finish()
        }
        34 => {
            let mut h = Hot::new(s, r);
            cmp_int_v_v_jump_h(&mut h);
            h.finish()
        }
        35 => {
            let mut h = Hot::new(s, r);
            cmp_int_v_c_jump_h(&mut h);
            h.finish()
        }
        _ => OPERATORS_REG[usize::from(opcode)](s, r),
    }
}
