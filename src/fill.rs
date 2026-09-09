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
#![allow(unused_parens)]

use crate::codegen_runtime;
use crate::hash;
use crate::keys::{DbRef, Str};
use crate::ops;
use crate::state::State;
use crate::tree;
use crate::vector;

pub const OPERATORS: &[fn(&mut State)] = &[
    goto,
    goto_word,
    goto_false,
    goto_false_word,
    call,
    op_return,
    free_stack,
    reserve_frame,
    const_true,
    const_false,
    cast_text_from_bool,
    var_bool,
    put_bool,
    not,
    range_default,
    const_int,
    const_short,
    const_tiny,
    var_int,
    var_character,
    put_int,
    put_character,
    conv_int_from_null,
    conv_bool_from_null,
    conv_character_from_null,
    conv_character_from_int,
    const_long_text,
    cast_int_from_text,
    cast_single_from_text,
    cast_float_from_text,
    abs_int,
    min_single_int,
    bit_not_single_int,
    conv_float_from_int,
    conv_single_from_int,
    conv_bool_from_int,
    add_int,
    min_int,
    mul_int,
    div_int,
    rem_int,
    add_int_nullable,
    min_int_nullable,
    mul_int_nullable,
    div_int_nullable,
    rem_int_nullable,
    land_int,
    lor_int,
    eor_int,
    s_left_int,
    s_right_int,
    eq_int,
    ne_int,
    lt_int,
    le_int,
    format_int,
    format_stack_int,
    const_single,
    var_single,
    put_single,
    conv_single_from_null,
    abs_single,
    min_single_single,
    cast_int_from_single,
    conv_float_from_single,
    conv_bool_from_single,
    add_single,
    min_single,
    mul_single,
    div_single,
    rem_single,
    div_single_nullable,
    rem_single_nullable,
    math_func_single,
    math_func2_single,
    pow_single,
    eq_single,
    ne_single,
    lt_single,
    le_single,
    format_single,
    format_stack_single,
    const_float,
    var_float,
    put_float,
    conv_float_from_null,
    abs_float,
    math_pi_float,
    math_e_float,
    math_func_float,
    math_func2_float,
    pow_float,
    min_single_float,
    cast_single_from_float,
    cast_int_from_float,
    conv_bool_from_float,
    add_float,
    min_float,
    mul_float,
    div_float,
    rem_float,
    div_float_nullable,
    rem_float_nullable,
    eq_float,
    ne_float,
    lt_float,
    le_float,
    format_float,
    format_stack_float,
    var_text,
    arg_text,
    const_text,
    conv_text_from_null,
    length_text,
    size_text,
    length_character,
    conv_bool_from_text,
    init_text,
    append_text,
    put_text,
    get_text_sub,
    text_character,
    text_character_nullable,
    conv_bool_from_character,
    clear_text,
    free_text,
    eq_text,
    ne_text,
    lt_text,
    le_text,
    format_text,
    format_stack_text,
    append_character,
    text_compare,
    cast_character_from_int,
    conv_int_from_character,
    var_enum,
    const_enum,
    put_enum,
    conv_bool_from_enum,
    cast_text_from_enum,
    cast_enum_from_text,
    conv_int_from_enum,
    cast_enum_from_int,
    conv_enum_from_null,
    database,
    format_database,
    format_stack_database,
    conv_bool_from_ref,
    conv_ref_from_null,
    init_ref,
    null_ref_sentinel,
    init_ref_sentinel,
    free_ref,
    store_tag,
    free_ref_tag,
    free_ref_if_distinct,
    free_ref_or_hand_up,
    free_scratch,
    sizeof_ref,
    var_ref,
    put_ref,
    eq_ref,
    ne_ref,
    get_ref,
    set_ref,
    set_db_ref,
    get_db_ref,
    get_field,
    get_int,
    get_character,
    get_single,
    get_float,
    get_byte,
    get_byte_nullable,
    get_enum,
    set_enum,
    get_boolean,
    set_boolean,
    get_short,
    get_text,
    set_int,
    set_character,
    set_single,
    set_float,
    set_byte,
    set_byte_nullable,
    set_short,
    get_int4,
    set_int4,
    get_int4_raw,
    set_int4_raw,
    get_int4_full,
    get_short_raw,
    set_short_raw,
    get_short_full,
    set_text,
    var_vector,
    tag_fault,
    length_vector,
    vector_is_null,
    ref_is_null,
    distinct_store,
    ref_alias,
    size_vector,
    size_struct,
    size_scalar,
    length_sorted,
    clear_vector,
    get_vector,
    vector_ref,
    get_vector_nullable,
    vector_ref_nullable,
    cast_vector_from_text,
    remove_vector,
    insert_vector,
    new_record,
    finish_record,
    append_vector,
    push_int,
    push_int4,
    push_float,
    push_single,
    push_boolean,
    push_enum,
    push_character,
    replace_vector,
    claim_child_rec,
    ref_from_child_rec,
    get_record,
    validate,
    hash_add,
    hash_find,
    hash_remove,
    length_hash,
    reserve_hash,
    size_hash,
    length_index,
    eq_bool,
    ne_bool,
    panic,
    print,
    iterate,
    step,
    remove,
    clear,
    append_copy,
    copy_record,
    copy_ref_or_null,
    bind_or_copy,
    replace_keyed,
    clear_keyed,
    index_group,
    link_record,
    fill_keyed,
    set_keyed,
    length_spatial,
    length_trie,
    static_call,
    create_stack,
    init_create_stack,
    get_stack_text,
    get_stack_ref,
    set_stack_ref,
    set_stack_fn_ref,
    get_stack_fn_ref,
    append_stack_text,
    append_stack_character,
    clear_stack_text,
    parallel_begin,
    parallel_arm,
    parallel_join,
    pre_alloc_vector,
    reserve_vector,
    get_file,
    get_dir,
    get_file_text,
    write_file,
    read_file,
    seek_file,
    size_file,
    delete,
    move_file,
    truncate_file,
    sync_file,
    deliver,
    expose,
    release,
    call_ref,
    mkdir,
    mkdir_all,
    rmdir,
    reverse_vector,
    sort_vector,
    coroutine_create,
    coroutine_next,
    coroutine_return,
    coroutine_yield,
    coroutine_exhausted,
    var_fn_ref,
    put_fn_ref,
    const_ref,
    const_store_text,
];

fn goto(s: &mut State) {
    let v_step = s.code::<i8>();
    s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
}

fn goto_word(s: &mut State) {
    let v_step = s.code::<i32>();
    s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
}

fn goto_false(s: &mut State) {
    let v_step = s.code::<i8>();
    let v_if_false = s.get_stack::<u8>();
    if v_if_false != 1 {
        s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
    }
}

fn goto_false_word(s: &mut State) {
    let v_step = s.code::<i32>();
    let v_if_false = s.get_stack::<u8>();
    if v_if_false != 1 {
        s.code_pos = (i64::from(s.code_pos) + i64::from(v_step)) as u32;
    }
}

fn call(s: &mut State) {
    let v_d_nr = s.code::<i64>();
    let v_args_size = s.code::<u16>();
    let v_to = s.code::<i64>();
    s.fn_call(v_d_nr as u32, v_args_size, v_to);
}

fn op_return(s: &mut State) {
    let v_ret = s.code::<u16>();
    let v_value = s.code::<u8>();
    let v_discard = s.code::<u16>();
    s.fn_return(v_ret, v_value, v_discard);
}

fn free_stack(s: &mut State) {
    let v_value = s.code::<u8>();
    let v_discard = s.code::<u16>();
    s.free_stack(v_value, v_discard);
}

fn reserve_frame(s: &mut State) {
    let v_size = s.code::<u16>();
    s.reserve_frame(v_size);
}

fn const_true(s: &mut State) {
    let new_value = true;
    s.put_stack(new_value);
}

fn const_false(s: &mut State) {
    let new_value = false;
    s.put_stack(new_value);
}

fn cast_text_from_bool(s: &mut State) {
    let v_v1 = s.get_stack::<u8>();
    let new_value = if v_v1 == 1 {
        "true"
    } else if v_v1 == 255 {
        "null"
    } else {
        "false"
    };
    s.put_stack(new_value);
}

fn var_bool(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<u8>(v_pos);
    s.put_stack(new_value);
}

fn put_bool(s: &mut State) {
    let v_var = s.code::<u16>();
    let v_value = s.get_stack::<u8>();
    s.put_var(v_var, v_value);
}

fn not(s: &mut State) {
    let v_v1 = s.get_stack::<u8>();
    let new_value = v_v1 != 1;
    s.put_stack(new_value);
}

fn range_default(s: &mut State) {
    let v_lo = s.code::<i64>();
    let v_hi = s.code::<i64>();
    let v_dflt = s.code::<i64>();
    let v_val = s.get_stack::<i64>();
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
    s.put_stack(new_value);
}

fn const_int(s: &mut State) {
    let v_val = s.code::<i64>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn const_short(s: &mut State) {
    let v_val = s.code::<i16>();
    let new_value = i64::from(v_val);
    s.put_stack(new_value);
}

fn const_tiny(s: &mut State) {
    let v_val = s.code::<i8>();
    let new_value = i64::from(v_val);
    s.put_stack(new_value);
}

fn var_int(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<i64>(v_pos);
    s.put_stack(new_value);
}

fn var_character(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<char>(v_pos);
    s.put_stack(new_value);
}

fn put_int(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = s.get_stack::<i64>();
    s.put_var(v_pos, v_value);
}

fn put_character(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = char::from_u32(s.get_stack::<u32>()).unwrap_or('\0');
    s.put_var(v_pos, v_value);
}

fn conv_int_from_null(s: &mut State) {
    let new_value = i64::MIN;
    s.put_stack(new_value);
}

fn conv_bool_from_null(s: &mut State) {
    let new_value = 255u8;
    s.put_stack(new_value);
}

fn conv_character_from_null(s: &mut State) {
    let new_value = char::from(0);
    s.put_stack(new_value);
}

fn conv_character_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = if v_v1 == i64::MIN {
        char::from(0)
    } else if let Some(c) = char::from_u32((v_v1) as u32) {
        c
    } else {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
        char::from(0)
    };
    s.put_stack(new_value);
}

fn const_long_text(s: &mut State) {
    let v_start = s.code::<i64>();
    let v_size = s.code::<i64>();
    s.string_from_texts(v_start, v_size);
}

fn cast_int_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = match v_v1.str().parse::<i64>() {
        Ok(v) if v == i64::MIN => {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
            i64::MIN
        }
        Ok(v) => v,
        Err(_) => i64::MIN,
    };
    s.put_stack(new_value);
}

fn cast_single_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f32::NAN);
    s.put_stack(new_value);
}

fn cast_float_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f64::NAN);
    s.put_stack(new_value);
}

fn abs_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_abs_int(v_v1);
    s.put_stack(new_value);
}

fn min_single_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_negate_int(v_v1);
    s.put_stack(new_value);
}

fn bit_not_single_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = !v_v1;
    s.put_stack(new_value);
}

fn conv_float_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_conv_float_from_int(v_v1);
    s.put_stack(new_value);
}

fn conv_single_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_conv_single_from_int(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_conv_bool_from_int(v_v1);
    s.put_stack(new_value);
}

fn add_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_add_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn min_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_min_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn mul_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_mul_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn div_int(s: &mut State) {
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

fn rem_int(s: &mut State) {
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

fn add_int_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_add_int_nullable(v_v1, v_v2);
    s.put_stack(new_value);
}

fn min_int_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_min_int_nullable(v_v1, v_v2);
    s.put_stack(new_value);
}

fn mul_int_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_mul_int_nullable(v_v1, v_v2);
    s.put_stack(new_value);
}

fn div_int_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = {
        let r = ops::op_div_int_nullable(v_v1, v_v2);
        ops::note_format_fault(1, r == i64::MIN && v_v1 != i64::MIN && v_v2 != i64::MIN);
        r
    };
    s.put_stack(new_value);
}

fn rem_int_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = {
        let r = ops::op_rem_int_nullable(v_v1, v_v2);
        ops::note_format_fault(2, r == i64::MIN && v_v1 != i64::MIN && v_v2 != i64::MIN);
        r
    };
    s.put_stack(new_value);
}

fn land_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_logical_and_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn lor_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_logical_or_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn eor_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = ops::op_exclusive_or_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn s_left_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
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
    s.put_stack(new_value);
}

fn s_right_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = if v_v1 == i64::MIN || v_v2 == i64::MIN {
        i64::MIN
    } else if !(0..64).contains(&v_v2) {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::ShiftOutOfRange);
        i64::MIN
    } else {
        ops::op_shift_right_int(v_v1, v_v2)
    };
    s.put_stack(new_value);
}

fn eq_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

fn ne_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 != v_v2;
    s.put_stack(new_value);
}

fn lt_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 < v_v2;
    s.put_stack(new_value);
}

fn le_int(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<i64>();
    let new_value = v_v1 <= v_v2;
    s.put_stack(new_value);
}

fn format_int(s: &mut State) {
    s.format_int();
}

fn format_stack_int(s: &mut State) {
    s.format_stack_int();
}

fn const_single(s: &mut State) {
    let v_val = s.code::<f32>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn var_single(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<f32>(v_pos);
    s.put_stack(new_value);
}

fn put_single(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = s.get_stack::<f32>();
    s.put_var(v_pos, v_value);
}

fn conv_single_from_null(s: &mut State) {
    let new_value = f32::NAN;
    s.put_stack(new_value);
}

fn abs_single(s: &mut State) {
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1.abs();
    s.put_stack(new_value);
}

fn min_single_single(s: &mut State) {
    let v_v1 = s.get_stack::<f32>();
    let new_value = -v_v1;
    s.put_stack(new_value);
}

fn cast_int_from_single(s: &mut State) {
    let v_v1 = s.get_stack::<f32>();
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
    s.put_stack(new_value);
}

fn conv_float_from_single(s: &mut State) {
    let v_v1 = s.get_stack::<f32>();
    let new_value = f64::from(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_single(s: &mut State) {
    let v_v1 = s.get_stack::<f32>();
    let new_value = !v_v1.is_nan();
    s.put_stack(new_value);
}

fn add_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1 + v_v2;
    s.put_stack(new_value);
}

fn min_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1 - v_v2;
    s.put_stack(new_value);
}

fn mul_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1 * v_v2;
    s.put_stack(new_value);
}

fn div_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 / v_v2
    };
    s.put_stack(new_value);
}

fn rem_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 % v_v2
    };
    s.put_stack(new_value);
}

fn div_single_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = {
        let r = v_v1 / v_v2;
        ops::note_format_fault(1, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack(new_value);
}

fn rem_single_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = {
        let r = v_v1 % v_v2;
        ops::note_format_fault(2, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack(new_value);
}

fn math_func_single(s: &mut State) {
    let v_fn_id = s.code::<i8>();
    let v_v1 = s.get_stack::<f32>();
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
    s.put_stack(new_value);
}

fn math_func2_single(s: &mut State) {
    let v_fn_id = s.code::<i8>();
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => v_v1.log(v_v2),
        _ => f32::NAN,
    };
    s.put_stack(new_value);
}

fn pow_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack(new_value);
}

fn eq_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = (v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1);
    s.put_stack(new_value);
}

fn ne_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = !((v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1));
    s.put_stack(new_value);
}

fn lt_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value =
        (v_v1.is_nan() && !v_v2.is_nan()) || (!v_v1.is_nan() && !v_v2.is_nan() && v_v1 < v_v2);
    s.put_stack(new_value);
}

fn le_single(s: &mut State) {
    let v_v2 = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<f32>();
    let new_value = v_v1.is_nan() || (!v_v2.is_nan() && v_v1 <= v_v2);
    s.put_stack(new_value);
}

fn format_single(s: &mut State) {
    s.format_single();
}

fn format_stack_single(s: &mut State) {
    s.format_stack_single();
}

fn const_float(s: &mut State) {
    let v_val = s.code::<f64>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn var_float(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<f64>(v_pos);
    s.put_stack(new_value);
}

fn put_float(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = s.get_stack::<f64>();
    s.put_var(v_pos, v_value);
}

fn conv_float_from_null(s: &mut State) {
    let new_value = f64::NAN;
    s.put_stack(new_value);
}

fn abs_float(s: &mut State) {
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1.abs();
    s.put_stack(new_value);
}

fn math_pi_float(s: &mut State) {
    let new_value = std::f64::consts::PI;
    s.put_stack(new_value);
}

fn math_e_float(s: &mut State) {
    let new_value = std::f64::consts::E;
    s.put_stack(new_value);
}

fn math_func_float(s: &mut State) {
    let v_fn_id = s.code::<i8>();
    let v_v1 = s.get_stack::<f64>();
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
    s.put_stack(new_value);
}

fn math_func2_float(s: &mut State) {
    let v_fn_id = s.code::<i8>();
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => v_v1.log(v_v2),
        _ => f64::NAN,
    };
    s.put_stack(new_value);
}

fn pow_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack(new_value);
}

fn min_single_float(s: &mut State) {
    let v_v1 = s.get_stack::<f64>();
    let new_value = -v_v1;
    s.put_stack(new_value);
}

fn cast_single_from_float(s: &mut State) {
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 as f32;
    s.put_stack(new_value);
}

fn cast_int_from_float(s: &mut State) {
    let v_v1 = s.get_stack::<f64>();
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
    s.put_stack(new_value);
}

fn conv_bool_from_float(s: &mut State) {
    let v_v1 = s.get_stack::<f64>();
    let new_value = !v_v1.is_nan();
    s.put_stack(new_value);
}

fn add_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 + v_v2;
    s.put_stack(new_value);
}

fn min_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 - v_v2;
    s.put_stack(new_value);
}

fn mul_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1 * v_v2;
    s.put_stack(new_value);
}

fn div_float(s: &mut State) {
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

fn rem_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        if v_v2 == 0.0 {
            s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::DivideByZero);
        }
        v_v1 % v_v2
    };
    s.put_stack(new_value);
}

fn div_float_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        let r = v_v1 / v_v2;
        ops::note_format_fault(1, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack(new_value);
}

fn rem_float_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = {
        let r = v_v1 % v_v2;
        ops::note_format_fault(2, r.is_nan() && !v_v1.is_nan() && !v_v2.is_nan());
        r
    };
    s.put_stack(new_value);
}

fn eq_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = (v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1);
    s.put_stack(new_value);
}

fn ne_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = !((v_v1.is_nan() && v_v2.is_nan()) || (v_v1 <= v_v2 && v_v2 <= v_v1));
    s.put_stack(new_value);
}

fn lt_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value =
        (v_v1.is_nan() && !v_v2.is_nan()) || (!v_v1.is_nan() && !v_v2.is_nan() && v_v1 < v_v2);
    s.put_stack(new_value);
}

fn le_float(s: &mut State) {
    let v_v2 = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<f64>();
    let new_value = v_v1.is_nan() || (!v_v2.is_nan() && v_v1 <= v_v2);
    s.put_stack(new_value);
}

fn format_float(s: &mut State) {
    s.format_float();
}

fn format_stack_float(s: &mut State) {
    s.format_stack_float();
}

fn var_text(s: &mut State) {
    s.var_text();
}

fn arg_text(s: &mut State) {
    s.arg_text();
}

fn const_text(s: &mut State) {
    s.string_from_code();
}

fn conv_text_from_null(s: &mut State) {
    let new_value = Str::new(crate::state::STRING_NULL);
    s.put_stack(new_value);
}

fn length_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().chars().count() as i64;
    s.put_stack(new_value);
}

fn size_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().len() as i64;
    s.put_stack(new_value);
}

fn length_character(s: &mut State) {
    s.length_character();
}

fn conv_bool_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str() != crate::state::STRING_NULL;
    s.put_stack(new_value);
}

fn init_text(s: &mut State) {
    s.init_text();
}

fn append_text(s: &mut State) {
    s.append_text();
}

fn put_text(s: &mut State) {
    s.put_text();
}

fn get_text_sub(s: &mut State) {
    s.get_text_sub();
}

fn text_character(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.string();
    let new_value = s.text_char_or_raise(v_v1.str(), v_v2);
    s.put_stack(new_value);
}

fn text_character_nullable(s: &mut State) {
    let v_v2 = s.get_stack::<i64>();
    let v_v1 = s.string();
    let new_value = {
        let ch = ops::text_character(v_v1.str(), v_v2);
        ops::note_format_fault(
            3,
            ch == char::from(0) && v_v2 != i64::MIN && !v_v1.str().is_empty(),
        );
        ch
    };
    s.put_stack(new_value);
}

fn conv_bool_from_character(s: &mut State) {
    let v_v1 = char::from_u32(s.get_stack::<u32>()).unwrap_or('\0');
    let new_value = ops::op_conv_bool_from_character(v_v1);
    s.put_stack(new_value);
}

fn clear_text(s: &mut State) {
    s.clear_text();
}

fn free_text(s: &mut State) {
    s.free_text();
}

fn eq_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_eq_text(v_v1.str(), v_v2.str());
    s.put_stack(new_value);
}

fn ne_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_ne_text(v_v1.str(), v_v2.str());
    s.put_stack(new_value);
}

fn lt_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_lt_text(v_v1.str(), v_v2.str());
    s.put_stack(new_value);
}

fn le_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = ops::op_le_text(v_v1.str(), v_v2.str());
    s.put_stack(new_value);
}

fn format_text(s: &mut State) {
    s.format_text();
}

fn format_stack_text(s: &mut State) {
    s.format_stack_text();
}

fn append_character(s: &mut State) {
    s.append_character();
}

fn text_compare(s: &mut State) {
    s.text_compare();
}

fn cast_character_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i32>();
    let new_value = if let Some(c) = char::from_u32(v_v1 as u32) {
        c
    } else {
        s.raise_recoverable(crate::runtime_error::RuntimeErrorKind::CastOutOfRange);
        char::from(0)
    };
    s.put_stack(new_value);
}

fn conv_int_from_character(s: &mut State) {
    let v_v1 = char::from_u32(s.get_stack::<u32>()).unwrap_or('\0');
    let new_value = if v_v1 == char::from(0) {
        i64::MIN
    } else {
        i64::from(v_v1 as u32)
    };
    s.put_stack(new_value);
}

fn var_enum(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<u8>(v_pos);
    s.put_stack(new_value);
}

fn const_enum(s: &mut State) {
    let v_val = s.code::<u8>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn put_enum(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = s.get_stack::<u8>();
    s.put_var(v_pos, v_value);
}

fn conv_bool_from_enum(s: &mut State) {
    let v_v1 = s.get_stack::<u8>();
    let new_value = v_v1 != 255 && v_v1 != 0;
    s.put_stack(new_value);
}

fn cast_text_from_enum(s: &mut State) {
    let v_enum_tp = s.code::<u16>();
    let v_v1 = s.get_stack::<u8>();
    let new_value = Str::new(s.database.enum_val(v_enum_tp, v_v1));
    s.put_stack(new_value);
}

fn cast_enum_from_text(s: &mut State) {
    let v_enum_tp = s.code::<u16>();
    let v_v1 = s.string();
    let new_value = s.database.to_enum(v_enum_tp, v_v1.str());
    s.put_stack(new_value);
}

fn conv_int_from_enum(s: &mut State) {
    let v_v1 = s.get_stack::<u8>();
    let new_value = if v_v1 == 255 {
        i64::MIN
    } else {
        i64::from(v_v1)
    };
    s.put_stack(new_value);
}

fn cast_enum_from_int(s: &mut State) {
    let v_v1 = s.get_stack::<i64>();
    let new_value = if v_v1 == i64::MIN { 255 } else { v_v1 as u8 };
    s.put_stack(new_value);
}

fn conv_enum_from_null(s: &mut State) {
    let new_value = 255u8;
    s.put_stack(new_value);
}

fn database(s: &mut State) {
    s.database();
}

fn format_database(s: &mut State) {
    s.format_database();
}

fn format_stack_database(s: &mut State) {
    s.format_stack_database();
}

fn conv_bool_from_ref(s: &mut State) {
    let v_val = s.get_stack::<DbRef>();
    let new_value = v_val.rec != 0;
    s.put_stack(new_value);
}

fn conv_ref_from_null(s: &mut State) {
    let new_value = s.database.null();
    s.put_stack(new_value);
}

fn init_ref(s: &mut State) {
    s.init_ref();
}

fn null_ref_sentinel(s: &mut State) {
    let new_value = DbRef::NULL;
    s.put_stack(new_value);
}

fn init_ref_sentinel(s: &mut State) {
    s.init_ref_sentinel();
}

fn free_ref(s: &mut State) {
    s.free_ref();
}

fn store_tag(s: &mut State) {
    s.store_tag();
}

fn free_ref_tag(s: &mut State) {
    s.free_ref_tag();
}

fn free_ref_if_distinct(s: &mut State) {
    let v_witness = s.get_stack::<DbRef>();
    let v_placeholder = s.get_stack::<DbRef>();
    s.database.free_displaced(&v_placeholder, &v_witness);
}

fn free_ref_or_hand_up(s: &mut State) {
    let v_witness = s.get_stack::<DbRef>();
    let v_placeholder = s.get_stack::<DbRef>();
    if v_placeholder.store_nr == v_witness.store_nr {
        s.hand_up_returned(v_witness);
    } else {
        s.database.free(&v_placeholder);
    }
}

fn free_scratch(s: &mut State) {
    let v_scratch = s.get_stack::<DbRef>();
    s.database.free_iteration_scratch(&v_scratch);
}

fn sizeof_ref(s: &mut State) {
    s.sizeof_ref();
}

fn var_ref(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = {
        let r = s.get_var::<DbRef>(v_pos);
        s.database.valid(&r);
        r
    };
    s.put_stack(new_value);
}

fn put_ref(s: &mut State) {
    let v_pos = s.code::<u16>();
    let v_value = s.get_stack::<DbRef>();
    s.put_var(v_pos, v_value);
}

fn eq_ref(s: &mut State) {
    let v_v2 = s.get_stack::<DbRef>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec == 0 && v_v2.rec == 0
    } else {
        v_v1 == v_v2
    };
    s.put_stack(new_value);
}

fn ne_ref(s: &mut State) {
    let v_v2 = s.get_stack::<DbRef>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec != 0 || v_v2.rec != 0
    } else {
        v_v1 != v_v2
    };
    s.put_stack(new_value);
}

fn get_ref(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = s.database.get_ref(&v_v1, u32::from(v_fld));
    s.put_stack(new_value);
}

fn set_ref(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<DbRef>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn set_db_ref(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<DbRef>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn get_db_ref(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_field(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = DbRef {
        store_nr: v_v1.store_nr,
        rec: v_v1.rec,
        pos: v_v1.pos + u32::from(v_fld),
    };
    s.put_stack(new_value);
}

fn get_int(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_character(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_single(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_float(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_byte(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            i64::from(s.database.store(&db).get_byte(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
            ))
        }
    };
    s.put_stack(new_value);
}

fn get_byte_nullable(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r =
                s.database
                    .store(&db)
                    .get_byte(db.rec, db.pos + u32::from(v_fld), i32::from(v_min));
            if r == i32::from(v_min) + 255 {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack(new_value);
}

fn get_enum(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn set_enum(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<u8>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn get_boolean(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn set_boolean(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<u8>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn get_short(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s.database.store(&db).get_short(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
            );
            if r == i32::MIN {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack(new_value);
}

fn get_text(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            Str::new(crate::state::STRING_NULL)
        } else {
            let store = s.database.store(&db);
            Str::new(store.get_str(store.get_u32_raw(db.rec, db.pos + u32::from(v_fld))))
        }
    };
    s.put_stack(new_value);
}

fn set_int(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn set_character(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = char::from_u32(s.get_stack::<u32>()).unwrap_or('\0');
    let v_v1 = s.get_stack::<DbRef>();
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

fn set_single(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<f32>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn set_float(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<f64>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn set_byte(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
    {
        let db = v_v1;
        let v = v_val;
        if db.rec != 0 {
            s.database.store_mut(&db).set_byte(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
                v as i32,
            );
        }
    }
}

fn set_byte_nullable(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .set_byte_nullable(&db, db.pos + u32::from(v_fld), i32::from(v_min), v);
        }
    }
}

fn set_short(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database
                .set_short_nullable(&db, db.pos + u32::from(v_fld), i32::from(v_min), v);
        }
    }
}

fn get_int4(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn set_int4(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn get_int4_raw(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn set_int4_raw(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
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

fn get_int4_full(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_v1 = s.get_stack::<DbRef>();
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
    s.put_stack(new_value);
}

fn get_short_raw(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            let r = s.database.store(&db).get_i16_raw(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
            );
            if r == i32::MIN {
                i64::MIN
            } else {
                i64::from(r)
            }
        }
    };
    s.put_stack(new_value);
}

fn set_short_raw(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_val = s.get_stack::<i64>();
    let v_v1 = s.get_stack::<DbRef>();
    {
        let db = v_v1;
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        if db.rec != 0 {
            s.database.store_mut(&db).set_i16_raw(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
                v,
            );
        }
    }
}

fn get_short_full(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_min = s.code::<i16>();
    let v_v1 = s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i64::MIN
        } else {
            i64::from(s.database.store(&db).get_short_full(
                db.rec,
                db.pos + u32::from(v_fld),
                i32::from(v_min),
            ))
        }
    };
    s.put_stack(new_value);
}

fn set_text(s: &mut State) {
    let v_fld = s.code::<u16>();
    let v_val = s.string();
    let v_v1 = s.get_stack::<DbRef>();
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

fn var_vector(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<DbRef>(v_pos);
    s.put_stack(new_value);
}

fn tag_fault(s: &mut State) {
    let v_kind = s.code::<u8>();
    {
        let _ = v_kind;
        ops::arm_format_fault();
    }
}

fn length_vector(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(vector::length_vector(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn vector_is_null(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = vector::is_absent_collection(&v_r, &s.database.allocations);
    s.put_stack(new_value);
}

fn ref_is_null(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = v_r.store_nr == u16::MAX;
    s.put_stack(new_value);
}

fn distinct_store(s: &mut State) {
    let v_b = s.get_stack::<DbRef>();
    let v_a = s.get_stack::<DbRef>();
    let new_value = v_a.store_nr != v_b.store_nr;
    s.put_stack(new_value);
}

fn ref_alias(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = v_r;
    s.put_stack(new_value);
}

fn size_vector(s: &mut State) {
    let v_stride = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
    let new_value =
        i64::from(vector::length_vector(&v_r, &s.database.allocations)) * i64::from(v_stride);
    s.put_stack(new_value);
}

fn size_struct(s: &mut State) {
    let v_sz = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = {
        let _ = v_r;
        i64::from(v_sz)
    };
    s.put_stack(new_value);
}

fn size_scalar(s: &mut State) {
    let v_sz = s.code::<u16>();
    let v_v = s.get_stack::<i64>();
    let new_value = {
        let _ = v_v;
        i64::from(v_sz)
    };
    s.put_stack(new_value);
}

fn length_sorted(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(vector::length_vector(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn clear_vector(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    vector::clear_vector(&v_r, &mut s.database.allocations);
}

fn get_vector(s: &mut State) {
    let v_size = s.code::<u16>();
    let v_index = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = {
        let __vr = v_r;
        let __vi = v_index;
        s.vec_get_or_raise(&__vr, u32::from(v_size), __vi)
    };
    s.put_stack(new_value);
}

fn vector_ref(s: &mut State) {
    let v_index = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = {
        let __vr = v_r;
        let __vi = v_index;
        s.vec_ref_or_raise(&__vr, __vi)
    };
    s.put_stack(new_value);
}

fn get_vector_nullable(s: &mut State) {
    let v_size = s.code::<u16>();
    let v_index = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = {
        let el = vector::get_vector(&v_r, u32::from(v_size), v_index, &s.database.allocations);
        ops::note_format_fault(3, el.rec == 0 && v_index != i64::MIN && !v_r.is_null());
        el
    };
    s.put_stack(new_value);
}

fn vector_ref_nullable(s: &mut State) {
    let v_index = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = {
        let el = vector::get_vector(&v_r, 4, v_index, &s.database.allocations);
        ops::note_format_fault(3, el.rec == 0 && v_index != i64::MIN && !v_r.is_null());
        s.database.get_ref(&el, 0)
    };
    s.put_stack(new_value);
}

fn cast_vector_from_text(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_val = s.string();
    let new_value = s.db_from_text(v_val.str(), v_db_tp);
    s.put_stack(new_value);
}

fn remove_vector(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_index = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = s.database.remove_vector_at(&v_r, v_tp, v_index);
    s.put_stack(new_value);
}

fn insert_vector(s: &mut State) {
    s.insert_vector();
}

fn new_record(s: &mut State) {
    s.new_record();
}

fn finish_record(s: &mut State) {
    s.finish_record();
}

fn append_vector(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_other = s.get_stack::<DbRef>();
    let v_r = s.get_stack::<DbRef>();
    s.database.vector_add(&v_r, &v_other, v_tp);
}

fn push_int(s: &mut State) {
    let v_val = *s.get_stack::<i64>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_i64(&v_r, v_val);
}

fn push_int4(s: &mut State) {
    let v_val = *s.get_stack::<i64>();
    let v_r = *s.get_stack::<DbRef>();
    {
        let v = if v_val == i64::MIN {
            i32::MIN
        } else {
            v_val as i32
        };
        s.database.append_i32(&v_r, v);
    }
}

fn push_float(s: &mut State) {
    let v_val = *s.get_stack::<f64>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_f64(&v_r, v_val);
}

fn push_single(s: &mut State) {
    let v_val = *s.get_stack::<f32>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_f32(&v_r, v_val);
}

fn push_boolean(s: &mut State) {
    let v_val = *s.get_stack::<u8>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_byte(&v_r, i32::from(v_val));
}

fn push_enum(s: &mut State) {
    let v_val = *s.get_stack::<u8>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_byte(&v_r, i32::from(v_val));
}

fn push_character(s: &mut State) {
    let v_val = char::from_u32(*s.get_stack::<u32>()).unwrap_or('\0');
    let v_r = *s.get_stack::<DbRef>();
    s.database.append_u32(&v_r, v_val as u32);
}

fn replace_vector(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_other = s.get_stack::<DbRef>();
    let v_r = s.get_stack::<DbRef>();
    s.database.vector_replace(&v_r, &v_other, v_tp);
}

fn claim_child_rec(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_src = s.get_stack::<DbRef>();
    let v_field = s.get_stack::<DbRef>();
    s.database.claim_child_rec(&v_field, &v_src, v_tp);
}

fn ref_from_child_rec(s: &mut State) {
    let v_field = s.get_stack::<DbRef>();
    let new_value = s.database.ref_from_child_rec(&v_field);
    s.put_stack(new_value);
}

fn get_record(s: &mut State) {
    s.get_record();
}

fn validate(s: &mut State) {
    s.validate();
}

fn hash_add(s: &mut State) {
    s.hash_add();
}

fn hash_find(s: &mut State) {
    s.hash_find();
}

fn hash_remove(s: &mut State) {
    s.hash_remove();
}

fn length_hash(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(hash::count(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn reserve_hash(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_count = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    s.database.reserve_hash(&v_r, v_count, v_db_tp);
}

fn size_hash(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(hash::table_bytes(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn length_index(s: &mut State) {
    let v_fields = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(tree::count(&v_r, v_fields, &s.database.allocations));
    s.put_stack(new_value);
}

fn eq_bool(s: &mut State) {
    let v_v2 = s.get_stack::<u8>();
    let v_v1 = s.get_stack::<u8>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

fn ne_bool(s: &mut State) {
    let v_v2 = s.get_stack::<u8>();
    let v_v1 = s.get_stack::<u8>();
    let new_value = v_v1 != v_v2;
    s.put_stack(new_value);
}

fn panic(s: &mut State) {
    let v_message = s.string();
    panic!("{}", v_message.str());
}

fn print(s: &mut State) {
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

fn iterate(s: &mut State) {
    s.iterate();
}

fn step(s: &mut State) {
    s.step();
}

fn remove(s: &mut State) {
    s.remove();
}

fn clear(s: &mut State) {
    s.clear();
}

fn append_copy(s: &mut State) {
    s.append_copy();
}

fn copy_record(s: &mut State) {
    s.copy_record();
}

fn copy_ref_or_null(s: &mut State) {
    s.copy_ref_or_null();
}

fn bind_or_copy(s: &mut State) {
    s.bind_or_copy();
}

fn replace_keyed(s: &mut State) {
    s.replace_keyed();
}

fn clear_keyed(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_dest = s.get_stack::<DbRef>();
    s.database.remove_claims_keyed(&v_dest, v_tp);
}

fn index_group(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_view = s.get_stack::<DbRef>();
    let v_primary = s.get_stack::<DbRef>();
    s.database.index_group_records(&v_primary, &v_view, v_tp);
}

fn link_record(s: &mut State) {
    let v_parent_tp = s.code::<u16>();
    let v_fld = s.code::<u16>();
    let v_rec = s.get_stack::<DbRef>();
    let v_data = s.get_stack::<DbRef>();
    s.database
        .link_record_siblings(&v_data, &v_rec, v_parent_tp, v_fld);
}

fn fill_keyed(s: &mut State) {
    let v_tp = s.code::<u16>();
    let v_parent_tp = s.code::<u16>();
    let v_field = s.code::<u16>();
    let v_src = s.get_stack::<DbRef>();
    let v_parent = s.get_stack::<DbRef>();
    s.database
        .fill_keyed_from_vector(&v_parent, &v_src, v_tp, v_parent_tp, v_field);
}

fn set_keyed(s: &mut State) {
    s.set_keyed();
}

fn length_spatial(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(codegen_runtime::spatial_len(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn length_trie(s: &mut State) {
    let v_r = s.get_stack::<DbRef>();
    let new_value = i64::from(codegen_runtime::trie_len(&v_r, &s.database.allocations));
    s.put_stack(new_value);
}

fn static_call(s: &mut State) {
    s.static_call();
}

fn create_stack(s: &mut State) {
    s.create_stack();
}

fn init_create_stack(s: &mut State) {
    s.init_create_stack();
}

fn get_stack_text(s: &mut State) {
    s.get_stack_text();
}

fn get_stack_ref(s: &mut State) {
    s.get_stack_ref();
}

fn set_stack_ref(s: &mut State) {
    s.set_stack_ref();
}

fn set_stack_fn_ref(s: &mut State) {
    s.set_stack_fn_ref();
}

fn get_stack_fn_ref(s: &mut State) {
    s.get_stack_fn_ref();
}

fn append_stack_text(s: &mut State) {
    s.append_stack_text();
}

fn append_stack_character(s: &mut State) {
    s.append_stack_character();
}

fn clear_stack_text(s: &mut State) {
    s.clear_stack_text();
}

fn parallel_begin(s: &mut State) {
    s.parallel_begin();
}

fn parallel_arm(s: &mut State) {
    s.parallel_arm();
}

fn parallel_join(s: &mut State) {
    s.parallel_join();
}

fn pre_alloc_vector(s: &mut State) {
    let v_capacity = s.code::<u16>();
    let v_elem_size = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
    vector::pre_alloc_vector(
        &v_r,
        u32::from(v_capacity),
        u32::from(v_elem_size),
        &mut s.database.allocations,
    );
}

fn reserve_vector(s: &mut State) {
    let v_elem_size = s.code::<u16>();
    let v_count = s.get_stack::<i64>();
    let v_r = s.get_stack::<DbRef>();
    vector::reserve_vector(
        &v_r,
        v_count,
        u32::from(v_elem_size),
        &mut s.database.allocations,
    );
}

fn get_file(s: &mut State) {
    let v_file = s.get_stack::<DbRef>();
    let new_value = s.database.get_file(&v_file);
    s.put_stack(new_value);
}

fn get_dir(s: &mut State) {
    let v_result = s.get_stack::<DbRef>();
    let v_path = s.string();
    let new_value = s.database.get_dir(v_path.str(), &v_result);
    s.put_stack(new_value);
}

fn get_file_text(s: &mut State) {
    s.get_file_text();
}

fn write_file(s: &mut State) {
    s.write_file();
}

fn read_file(s: &mut State) {
    s.read_file();
}

fn seek_file(s: &mut State) {
    s.seek_file();
}

fn size_file(s: &mut State) {
    s.size_file();
}

fn delete(s: &mut State) {
    let v_path = s.string();
    let new_value = codegen_runtime::fs_delete(&s.database.resolve_path(v_path.str()));
    s.put_stack(new_value);
}

fn move_file(s: &mut State) {
    let v_to = s.string();
    let v_from = s.string();
    let new_value = codegen_runtime::fs_move(
        &s.database.resolve_path(v_from.str()),
        &s.database.resolve_path(v_to.str()),
    );
    s.put_stack(new_value);
}

fn truncate_file(s: &mut State) {
    s.truncate_file();
}

fn sync_file(s: &mut State) {
    s.sync_file();
}

fn deliver(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_val = s.get_stack::<DbRef>();
    let v_tag = s.get_stack::<i64>();
    s.database.deliver_reconstruct(v_tag, v_val, v_db_tp);
}

fn expose(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_val = s.get_stack::<DbRef>();
    let v_tag = s.get_stack::<i64>();
    s.database.expose_value(v_tag, v_val, v_db_tp);
}

fn release(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_val = s.get_stack::<DbRef>();
    let v_tag = s.get_stack::<i64>();
    s.database.release_value(v_tag, v_val, v_db_tp);
}

fn call_ref(s: &mut State) {
    let v_fn_var = s.code::<u16>();
    let v_arg_size = s.code::<u16>();
    s.fn_call_ref(v_fn_var, v_arg_size);
}

fn mkdir(s: &mut State) {
    let v_path = s.string();
    let new_value = codegen_runtime::fs_mkdir(&s.database.resolve_path(v_path.str()));
    s.put_stack(new_value);
}

fn mkdir_all(s: &mut State) {
    let v_path = s.string();
    let new_value = codegen_runtime::fs_mkdir_all(&s.database.resolve_path(v_path.str()));
    s.put_stack(new_value);
}

fn rmdir(s: &mut State) {
    let v_path = s.string();
    let new_value = codegen_runtime::fs_rmdir(&s.database.resolve_path(v_path.str()));
    s.put_stack(new_value);
}

fn reverse_vector(s: &mut State) {
    let v_size = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
    vector::reverse_vector(&v_r, u32::from(v_size), &mut s.database.allocations);
}

fn sort_vector(s: &mut State) {
    let v_db_tp = s.code::<u16>();
    let v_r = s.get_stack::<DbRef>();
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

fn coroutine_create(s: &mut State) {
    let v_d_nr = s.code::<i64>();
    let v_args_size = s.code::<u16>();
    let v_to = s.code::<i64>();
    s.coroutine_create(v_d_nr as u32, u32::from(v_args_size), v_to as u32);
}

fn coroutine_next(s: &mut State) {
    let v_value_size = s.code::<u16>();
    s.coroutine_next(u32::from(v_value_size & 0xFF));
}

fn coroutine_return(s: &mut State) {
    let v_value_size = s.code::<u16>();
    s.coroutine_return(u32::from(v_value_size));
}

fn coroutine_yield(s: &mut State) {
    let v_value_size = s.code::<u16>();
    s.coroutine_yield(u32::from(v_value_size));
}

fn coroutine_exhausted(s: &mut State) {
    let v_gen = s.get_stack::<DbRef>();
    let new_value = s.coroutine_exhausted(&v_gen);
    s.put_stack(new_value);
}

fn var_fn_ref(s: &mut State) {
    let v_pos = s.code::<u16>();
    let new_value = s.get_var::<[std::mem::MaybeUninit<u8>; 20]>(v_pos);
    s.put_stack(new_value);
}

fn put_fn_ref(s: &mut State) {
    let v_pos = s.code::<u16>();
    {
        let v = s.get_stack::<[std::mem::MaybeUninit<u8>; 20]>();
        s.put_var(v_pos, v);
    }
}

fn const_ref(s: &mut State) {
    let v_d_nr = s.code::<i64>();
    let new_value = s.const_ref_at(v_d_nr as usize);
    s.put_stack(new_value);
}

fn const_store_text(s: &mut State) {
    let v_rec = s.code::<i64>();
    let v_pos = s.code::<i64>();
    s.string_from_const_store(v_rec as u32, v_pos as u32)
}
