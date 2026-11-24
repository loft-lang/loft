#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(unused_labels)]
#![allow(unused_braces)]
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]
#![allow(unused_unsafe)]

extern crate loft;
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::ops;
use loft::vector;
use loft::codegen_runtime;
use loft::codegen_runtime::*;
mod external {
    pub fn rand_seed(seed: i64) { loft::codegen_runtime::cr_rand_seed(seed); }
    pub fn rand_int(lo: i32, hi: i32) -> i32 { loft::codegen_runtime::cr_rand_int(lo, hi) }
}

fn init(db: &mut Stores) {
    let e = db.enumerate("FieldValue");
    db.value(e, "FvBool", 10_u16);
    db.value(e, "FvInt", 12_u16);
    db.value(e, "FvLong", 13_u16);
    db.value(e, "FvFloat", 14_u16);
    db.value(e, "FvSingle", 15_u16);
    db.value(e, "FvChar", 16_u16);
    db.value(e, "FvText", 17_u16);
    let s = db.structure("Self", 0); // 8
    let s = db.structure("T", 0); // 9
    let s = db.structure("FvBool", 1); // 10
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 4);
    db.byte(0, false); // type 11
    let s = db.structure("FvInt", 2); // 12
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 0);
    let s = db.structure("FvLong", 3); // 13
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 1);
    let s = db.structure("FvFloat", 4); // 14
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 3);
    let s = db.structure("FvSingle", 5); // 15
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 2);
    let s = db.structure("FvChar", 6); // 16
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 6);
    let s = db.structure("FvText", 7); // 17
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 5);
    let s = db.structure("StructField", 0); // 18
    db.field(s, "name", 5);
    db.field(s, "value", 7);
    let s = db.structure("main_vector<integer>", 0); // 19
    let vec_vector = db.vector(0);
    db.field(s, "vector", vec_vector);
    db.vector(0);
    let s = db.structure("main_vector<T>", 0); // 21
    let vec_vector = db.vector(9);
    db.field(s, "vector", vec_vector);
    db.vector(9);
    db.vector(5);
    let e = db.enumerate("Format");
    db.value(e, "TextFile", u16::MAX);
    db.value(e, "LittleEndian", u16::MAX);
    db.value(e, "BigEndian", u16::MAX);
    db.value(e, "Directory", u16::MAX);
    db.value(e, "NotExists", u16::MAX);
    let e = db.enumerate("FileResult");
    db.value(e, "Ok", u16::MAX);
    db.value(e, "NotFound", u16::MAX);
    db.value(e, "PermissionDenied", u16::MAX);
    db.value(e, "IsDirectory", u16::MAX);
    db.value(e, "NotDirectory", u16::MAX);
    db.value(e, "Other", u16::MAX);
    db.value(e, "ok", u16::MAX);
    let s = db.structure("EnvVariable", 0); // 26
    db.field(s, "name", 5);
    db.field(s, "value", 5);
    let s = db.structure("File", 0); // 27
    db.field(s, "path", 5);
    db.field(s, "size", 1);
    db.field(s, "format", 24);
    db.field(s, "ref", 0);
    db.field(s, "current", 1);
    db.field(s, "next", 1);
    let s = db.structure("main_vector<text>", 0); // 28
    let vec_vector = db.vector(5);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<File>", 0); // 29
    let vec_vector = db.vector(27);
    db.field(s, "vector", vec_vector);
    db.vector(27);
    let e = db.enumerate("ArgValue");
    db.value(e, "NullVal", 32_u16);
    db.value(e, "BoolVal", 33_u16);
    db.value(e, "IntVal", 34_u16);
    db.value(e, "LongVal", 35_u16);
    db.value(e, "FloatVal", 36_u16);
    db.value(e, "SingleVal", 37_u16);
    db.value(e, "CharVal", 38_u16);
    db.value(e, "TextVal", 39_u16);
    db.value(e, "RefVal", 40_u16);
    db.value(e, "FnVal", 41_u16);
    db.value(e, "OtherVal", 42_u16);
    let s = db.structure("NullVal", 1); // 32
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let s = db.structure("BoolVal", 2); // 33
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "b", 4);
    let s = db.structure("IntVal", 3); // 34
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "n", 0);
    let s = db.structure("LongVal", 4); // 35
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "n", 1);
    let s = db.structure("FloatVal", 5); // 36
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "f", 3);
    let s = db.structure("SingleVal", 6); // 37
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "f", 2);
    let s = db.structure("CharVal", 7); // 38
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "c", 6);
    let s = db.structure("TextVal", 8); // 39
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "t", 5);
    let s = db.structure("RefVal", 9); // 40
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "store", 0);
    db.field(s, "rec", 0);
    db.field(s, "pos", 0);
    let s = db.structure("FnVal", 10); // 41
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "d_nr", 0);
    let s = db.structure("OtherVal", 11); // 42
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "description", 5);
    let s = db.structure("ArgInfo", 0); // 43
    db.field(s, "name", 5);
    db.field(s, "type_name", 5);
    db.field(s, "value", 31);
    let s = db.structure("VarInfo", 0); // 44
    db.field(s, "name", 5);
    db.field(s, "type_name", 5);
    db.field(s, "value", 31);
    let s = db.structure("StackFrame", 0); // 45
    db.field(s, "function", 5);
    db.field(s, "file", 5);
    db.field(s, "line", 0);
    let vec_arguments = db.vector(43);
    db.field(s, "arguments", vec_arguments);
    let vec_variables = db.vector(44);
    db.field(s, "variables", vec_variables);
    db.vector(43);
    db.vector(44);
    let s = db.structure("main_vector<ArgInfo>", 0); // 48
    let vec_vector = db.vector(43);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<VarInfo>", 0); // 49
    let vec_vector = db.vector(44);
    db.field(s, "vector", vec_vector);
    let e = db.enumerate("CoroutineStatus");
    db.value(e, "Created", u16::MAX);
    db.value(e, "Suspended", u16::MAX);
    db.value(e, "Running", u16::MAX);
    db.value(e, "Exhausted", u16::MAX);
    let e = db.enumerate("JsonValue");
    db.value(e, "JNull", 52_u16);
    db.value(e, "JBool", 53_u16);
    db.value(e, "JNumber", 54_u16);
    db.value(e, "JString", 55_u16);
    db.value(e, "JArray", 56_u16);
    db.value(e, "JObject", 58_u16);
    db.value(e, "field", u16::MAX);
    db.value(e, "item", u16::MAX);
    db.value(e, "len", u16::MAX);
    db.value(e, "as_text", u16::MAX);
    db.value(e, "as_number", u16::MAX);
    db.value(e, "as_long", u16::MAX);
    db.value(e, "as_bool", u16::MAX);
    db.value(e, "kind", u16::MAX);
    db.value(e, "keys", u16::MAX);
    db.value(e, "fields", u16::MAX);
    db.value(e, "has_field", u16::MAX);
    db.value(e, "to_json", u16::MAX);
    db.value(e, "to_json_pretty", u16::MAX);
    let s = db.structure("JNull", 1); // 52
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let s = db.structure("JBool", 2); // 53
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "value", 4);
    let s = db.structure("JNumber", 3); // 54
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "value", 3);
    let s = db.structure("JString", 4); // 55
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "value", 5);
    let s = db.structure("JArray", 5); // 56
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let vec_items = db.vector(51);
    db.field(s, "items", vec_items);
    db.vector(51);
    let s = db.structure("JsonField", 0); // 59
    db.field(s, "name", 5);
    db.field(s, "value", 51);
    let s = db.structure("JObject", 6); // 58
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let vec_fields = db.vector(59);
    db.field(s, "fields", vec_fields);
    db.vector(59);
    let s = db.structure("main_vector<JsonValue>", 0); // 61
    let vec_vector = db.vector(51);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<JsonField>", 0); // 62
    let vec_vector = db.vector(59);
    db.field(s, "vector", vec_vector);
    let s = db.structure("HexAddress", 0); // 63
    db.field(s, "ha_q", 0);
    db.field(s, "ha_r", 0);
    db.field(s, "ha_cy", 0);
    let s = db.structure("Hex", 0); // 64
    db.field(s, "h_height", 0);
    db.field(s, "h_material", 0);
    db.field(s, "h_item", 0);
    db.field(s, "h_item_rotation", 0);
    db.field(s, "h_wall_n", 0);
    db.field(s, "h_wall_ne", 0);
    db.field(s, "h_wall_se", 0);
    let s = db.structure("Chunk", 0); // 65
    db.field(s, "ck_cx", 0);
    db.field(s, "ck_cy", 0);
    db.field(s, "ck_cz", 0);
    let vec_ck_hexes = db.vector(64);
    db.field(s, "ck_hexes", vec_ck_hexes);
    db.vector(64);
    let s = db.structure("main_vector<Hex>", 0); // 67
    let vec_vector = db.vector(64);
    db.field(s, "vector", vec_vector);
    let s = db.structure("MaterialDef", 0); // 68
    db.field(s, "md_name", 5);
    db.field(s, "md_category", 5);
    db.field(s, "md_stair_kind", 5);
    db.field(s, "md_texture", 0);
    db.field(s, "md_tint_r", 0);
    db.field(s, "md_tint_g", 0);
    db.field(s, "md_tint_b", 0);
    db.field(s, "md_walkable", 0);
    db.field(s, "md_swimmable", 0);
    db.field(s, "md_climbable", 0);
    db.field(s, "md_slippery", 0);
    db.field(s, "md_loud", 0);
    let s = db.structure("WallDef", 0); // 69
    db.field(s, "wd_name", 5);
    db.field(s, "wd_body", 5);
    db.field(s, "wd_base", 5);
    db.field(s, "wd_thickness", 3);
    db.field(s, "wd_texture", 0);
    db.field(s, "wd_tint_r", 0);
    db.field(s, "wd_tint_g", 0);
    db.field(s, "wd_tint_b", 0);
    let s = db.structure("ItemDef", 0); // 70
    db.field(s, "id_name", 5);
    db.field(s, "id_kind", 5);
    db.field(s, "id_model", 0);
    db.field(s, "id_symmetric", 0);
    db.field(s, "id_arc_radius", 3);
    let s = db.structure("SpawnPoint", 0); // 71
    db.field(s, "sp_hex", 63);
    db.field(s, "sp_kind", 5);
    db.field(s, "sp_creature", 0);
    db.field(s, "sp_npc_id", 0);
    db.field(s, "sp_count", 0);
    db.field(s, "sp_facing", 0);
    db.field(s, "sp_condition", 5);
    let s = db.structure("NpcWaypoint", 0); // 72
    db.field(s, "wp_hex", 63);
    db.field(s, "wp_activity", 5);
    db.field(s, "wp_time_start", 0);
    db.field(s, "wp_time_end", 0);
    db.field(s, "wp_facing", 0);
    db.field(s, "wp_note", 5);
    let s = db.structure("NpcRoutine", 0); // 73
    db.field(s, "nr_npc_id", 0);
    db.field(s, "nr_name", 5);
    db.field(s, "nr_creature", 0);
    let vec_nr_waypoints = db.vector(72);
    db.field(s, "nr_waypoints", vec_nr_waypoints);
    db.vector(72);
    let s = db.structure("main_vector<NpcWaypoint>", 0); // 75
    let vec_vector = db.vector(72);
    db.field(s, "vector", vec_vector);
    db.vector(65);
    let s = db.structure("Map", 0); // 77
    db.field(s, "m_name", 5);
    let vec_m_chunks = db.vector(65);
    db.field(s, "m_chunks", vec_m_chunks);
    let vec_m_material_palette = db.vector(68);
    db.field(s, "m_material_palette", vec_m_material_palette);
    let vec_m_wall_palette = db.vector(69);
    db.field(s, "m_wall_palette", vec_m_wall_palette);
    let vec_m_item_palette = db.vector(70);
    db.field(s, "m_item_palette", vec_m_item_palette);
    let vec_m_spawn_points = db.vector(71);
    db.field(s, "m_spawn_points", vec_m_spawn_points);
    let vec_m_npc_routines = db.vector(73);
    db.field(s, "m_npc_routines", vec_m_npc_routines);
    db.vector(68);
    db.vector(69);
    db.vector(70);
    db.vector(71);
    db.vector(73);
    let s = db.structure("main_vector<Chunk>", 0); // 83
    let vec_vector = db.vector(65);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<MaterialDef>", 0); // 84
    let vec_vector = db.vector(68);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<WallDef>", 0); // 85
    let vec_vector = db.vector(69);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<ItemDef>", 0); // 86
    let vec_vector = db.vector(70);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<SpawnPoint>", 0); // 87
    let vec_vector = db.vector(71);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<NpcRoutine>", 0); // 88
    let vec_vector = db.vector(73);
    db.field(s, "vector", vec_vector);
    db.finish();
}

fn i_parse_errors(stores: &mut Stores) -> Str {
  loft::codegen_runtime::i_parse_errors(stores)
}


fn i_parse_error_push(stores: &mut Stores, mut var_msg: &str) {
  loft::codegen_runtime::i_parse_error_push(stores, var_msg)
}


fn __iface_14_OpLt(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> bool {
  todo!("native function __iface_14_OpLt")
}


fn __iface_17_OpEq(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> bool {
  todo!("native function __iface_17_OpEq")
}


fn __iface_19_OpAdd(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> DbRef {
  todo!("native function __iface_19_OpAdd")
}


fn __iface_21_OpMul(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> DbRef {
  todo!("native function __iface_21_OpMul")
}


fn __iface_21_OpMin(stores: &mut Stores, mut var_self: DbRef) -> DbRef {
  todo!("native function __iface_21_OpMin")
}


fn __iface_24_scale(stores: &mut Stores, mut var_self: DbRef, mut var_factor: i32) -> i32 {
  todo!("native function __iface_24_scale")
}


fn __iface_26_to_text(stores: &mut Stores, mut var_self: DbRef) -> Str {
  todo!("native function __iface_26_to_text")
}


fn t_7integer_abs(stores: &mut Stores, mut var_both: i32) -> i32 { //block_1: integer
  return ops::op_abs_int((var_both))
  } /*block_1: integer*/

fn t_4long_abs(stores: &mut Stores, mut var_both: i64) -> i64 { //block_1: long
  return ops::op_abs_long((var_both))
  } /*block_1: long*/

fn t_6single_abs(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return (var_both).abs()
  } /*block_1: single*/

fn t_6single_cos(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (0_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_sin(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (1_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_tan(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (2_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_acos(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (3_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_asin(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (4_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_atan(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (5_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_ceil(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (6_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_floor(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (7_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_round(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (8_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_sqrt(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return match (9_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_atan2(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  return match (0_i32) { 0 => (var_both).atan2((var_v2)), 1 => (var_both).log((var_v2)), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_log(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  return match (1_i32) { 0 => (var_both).atan2((var_v2)), 1 => (var_both).log((var_v2)), _ => f32::NAN }
  } /*block_1: single*/

fn t_6single_pow(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  return (var_both).powf((var_v2))
  } /*block_1: single*/

fn t_5float_abs(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return (var_both).abs()
  } /*block_1: float*/

fn t_5float_cos(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (0_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_sin(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (1_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_tan(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (2_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_acos(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (3_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_asin(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (4_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_atan(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (5_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_ceil(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (6_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_floor(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (7_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_round(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (8_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_sqrt(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return match (9_i32) { 0 => (var_both).cos(), 1 => (var_both).sin(), 2 => (var_both).tan(), 3 => (var_both).acos(), 4 => (var_both).asin(), 5 => (var_both).atan(), 6 => (var_both).ceil(), 7 => (var_both).floor(), 8 => (var_both).round(), 9 => (var_both).sqrt(), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_atan2(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  return match (0_i32) { 0 => (var_both).atan2((var_v2)), 1 => (var_both).log((var_v2)), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_log(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  return match (1_i32) { 0 => (var_both).atan2((var_v2)), 1 => (var_both).log((var_v2)), _ => f64::NAN }
  } /*block_1: float*/

fn t_5float_pow(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  return (var_both).powf((var_v2))
  } /*block_1: float*/

fn t_6single_exp(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return t_6single_pow(stores, (std::f64::consts::E) as f32, var_both)
  } /*block_1: single*/

fn t_5float_exp(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return t_5float_pow(stores, std::f64::consts::E, var_both)
  } /*block_1: float*/

fn t_6single_ln(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return t_6single_log(stores, var_both, (std::f64::consts::E) as f32)
  } /*block_1: single*/

fn t_5float_ln(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return t_5float_log(stores, var_both, std::f64::consts::E)
  } /*block_1: float*/

fn t_6single_log2(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return t_6single_log(stores, var_both, 2_f32)
  } /*block_1: single*/

fn t_5float_log2(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return t_5float_log(stores, var_both, 2_f64)
  } /*block_1: float*/

fn t_6single_log10(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  return t_6single_log(stores, var_both, 10_f32)
  } /*block_1: single*/

fn t_5float_log10(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  return t_5float_log(stores, var_both, 10_f64)
  } /*block_1: float*/

fn t_7integer_min(stores: &mut Stores, mut var_both: i32, mut var_b: i32) -> i32 { //block_1: integer
  if if !(ops::op_conv_bool_from_int((var_both))) {true} else {!(ops::op_conv_bool_from_int((var_b)))} { //block_2: never
    return i32::MIN
    } /*block_2: never*/ else {()};
  return if (var_both) <= (var_b) { //block_3: integer
    var_both
    } /*block_3: integer*/ else { //block_4: integer
    var_b
    } /*block_4: integer*/
  } /*block_1: integer*/

fn t_7integer_max(stores: &mut Stores, mut var_both: i32, mut var_b: i32) -> i32 { //block_1: integer
  if if !(ops::op_conv_bool_from_int((var_both))) {true} else {!(ops::op_conv_bool_from_int((var_b)))} { //block_2: never
    return i32::MIN
    } /*block_2: never*/ else {()};
  return if (var_b) <= (var_both) { //block_3: integer
    var_both
    } /*block_3: integer*/ else { //block_4: integer
    var_b
    } /*block_4: integer*/
  } /*block_1: integer*/

fn t_7integer_clamp(stores: &mut Stores, mut var_both: i32, mut var_lo: i32, mut var_hi: i32) -> i32 { //block_1: integer
  let _pre_0 = t_7integer_max(stores, var_both, var_lo);
  return t_7integer_min(stores, _pre_0, var_hi)
  } /*block_1: integer*/

fn t_4long_min(stores: &mut Stores, mut var_both: i64, mut var_b: i64) -> i64 { //block_1: long
  if if !(ops::op_conv_bool_from_long((var_both))) {true} else {!(ops::op_conv_bool_from_long((var_b)))} { //block_2: never
    return i64::MIN
    } /*block_2: never*/ else {()};
  return if (var_both) <= (var_b) { //block_3: long
    var_both
    } /*block_3: long*/ else { //block_4: long
    var_b
    } /*block_4: long*/
  } /*block_1: long*/

fn t_4long_max(stores: &mut Stores, mut var_both: i64, mut var_b: i64) -> i64 { //block_1: long
  if if !(ops::op_conv_bool_from_long((var_both))) {true} else {!(ops::op_conv_bool_from_long((var_b)))} { //block_2: never
    return i64::MIN
    } /*block_2: never*/ else {()};
  return if (var_b) <= (var_both) { //block_3: long
    var_both
    } /*block_3: long*/ else { //block_4: long
    var_b
    } /*block_4: long*/
  } /*block_1: long*/

fn t_4long_clamp(stores: &mut Stores, mut var_both: i64, mut var_lo: i64, mut var_hi: i64) -> i64 { //block_1: long
  let _pre_1 = t_4long_max(stores, var_both, var_lo);
  return t_4long_min(stores, _pre_1, var_hi)
  } /*block_1: long*/

fn t_6single_min(stores: &mut Stores, mut var_both: f32, mut var_b: f32) -> f32 { //block_1: single
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: never
    return f32::NAN
    } /*block_2: never*/ else {()};
  return if !(var_both).is_nan() && !(var_b).is_nan() && (var_both) <= (var_b) { //block_3: single
    var_both
    } /*block_3: single*/ else { //block_4: single
    var_b
    } /*block_4: single*/
  } /*block_1: single*/

fn t_6single_max(stores: &mut Stores, mut var_both: f32, mut var_b: f32) -> f32 { //block_1: single
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: never
    return f32::NAN
    } /*block_2: never*/ else {()};
  return if !(var_b).is_nan() && !(var_both).is_nan() && (var_b) <= (var_both) { //block_3: single
    var_both
    } /*block_3: single*/ else { //block_4: single
    var_b
    } /*block_4: single*/
  } /*block_1: single*/

fn t_6single_clamp(stores: &mut Stores, mut var_both: f32, mut var_lo: f32, mut var_hi: f32) -> f32 { //block_1: single
  let _pre_2 = t_6single_max(stores, var_both, var_lo);
  return t_6single_min(stores, _pre_2, var_hi)
  } /*block_1: single*/

fn t_5float_min(stores: &mut Stores, mut var_both: f64, mut var_b: f64) -> f64 { //block_1: float
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: never
    return f64::NAN
    } /*block_2: never*/ else {()};
  return if !(var_both).is_nan() && !(var_b).is_nan() && (var_both) <= (var_b) { //block_3: float
    var_both
    } /*block_3: float*/ else { //block_4: float
    var_b
    } /*block_4: float*/
  } /*block_1: float*/

fn t_5float_max(stores: &mut Stores, mut var_both: f64, mut var_b: f64) -> f64 { //block_1: float
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: never
    return f64::NAN
    } /*block_2: never*/ else {()};
  return if !(var_b).is_nan() && !(var_both).is_nan() && (var_b) <= (var_both) { //block_3: float
    var_both
    } /*block_3: float*/ else { //block_4: float
    var_b
    } /*block_4: float*/
  } /*block_1: float*/

fn t_5float_clamp(stores: &mut Stores, mut var_both: f64, mut var_lo: f64, mut var_hi: f64) -> f64 { //block_1: float
  let _pre_3 = t_5float_max(stores, var_both, var_lo);
  return t_5float_min(stores, _pre_3, var_hi)
  } /*block_1: float*/

fn t_4text_len(stores: &mut Stores, mut var_both: &str) -> i32 { //block_1: integer
  return (var_both).len() as i32
  } /*block_1: integer*/

fn t_4text_size(stores: &mut Stores, mut var_both: &str) -> i32 { //block_1: integer
  return (var_both).chars().count() as i32
  } /*block_1: integer*/

fn t_9character_len(stores: &mut Stores, mut var_both: i32) -> i32 { //block_1: integer
  return OpLengthCharacter(stores, var_both)
  } /*block_1: integer*/

fn t_6vector_len(stores: &mut Stores, mut var_both: DbRef) -> i32 { //block_1: integer
  return vector::length_vector(&(var_both), &stores.allocations) as i32
  } /*block_1: integer*/

fn t_6vector_clear(stores: &mut Stores, mut var_both: DbRef) { //block_1: void
  if var_both.rec != 0 { vector::clear_vector(&var_both, &mut stores.allocations); };
  } /*block_1: void*/

fn n_assert<M: std::fmt::Display, F: std::fmt::Display>(_s: &mut Stores, test: bool, msg: M, file: F, line: i32) {
  if !test { panic!("{}:{} {}", file, line, msg); }
}

fn n_panic(stores: &mut Stores, mut var_message: &str, mut var_file: &str, mut var_line: i32) {
}


fn n_log_info(stores: &mut Stores, mut var_message: &str, mut var_file: &str, mut var_line: i32) {
}


fn n_log_warn(stores: &mut Stores, mut var_message: &str, mut var_file: &str, mut var_line: i32) {
}


fn n_log_error(stores: &mut Stores, mut var_message: &str, mut var_file: &str, mut var_line: i32) {
}


fn n_log_fatal(stores: &mut Stores, mut var_message: &str, mut var_file: &str, mut var_line: i32) {
}


fn n_print(stores: &mut Stores, mut var_v1: &str) { //block_1: void
  cr_call_push("print", "/home/lima.guest/loft/default/01_code.loft", 877);
  let _call_guard = codegen_runtime::CallGuard;
  {#[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
crate::loft_host_print((var_v1).as_ptr(), (var_v1).len());
#[cfg(not(target_arch = "wasm32"))]
print!("{}", (var_v1));
#[cfg(feature = "wasm")]
crate::wasm::output_push((var_v1));};
  } /*block_1: void*/

fn n_println(stores: &mut Stores, mut var_v1: &str) { //block_1: void
  cr_call_push("println", "/home/lima.guest/loft/default/01_code.loft", 882);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___work_1: String = "".to_string();
  let _pre_4 = { //Formatted string_2: text["__work_1"]
    var___work_1 = String::with_capacity(16_usize);
    ops::format_text(&mut var___work_1, var_v1, 0_i32, 2, 32);
    var___work_1 += &*("\n");
    &var___work_1
    } /*Formatted string_2: text["__work_1"]*/;
  {#[cfg(all(target_arch = "wasm32", not(feature = "wasm")))]
crate::loft_host_print((_pre_4).as_ptr(), (_pre_4).len());
#[cfg(not(target_arch = "wasm32"))]
print!("{}", (_pre_4));
#[cfg(feature = "wasm")]
crate::wasm::output_push((_pre_4));};
  ;
  } /*block_1: void*/

fn n_parallel_for_int(stores: &mut Stores, mut var_func: &str, mut var_input: DbRef, mut var_element_size: i32, mut var_threads: i32) -> DbRef {
  todo!("native function n_parallel_for_int")
}


fn n_parallel_for(stores: &mut Stores, mut var_input: DbRef, mut var_element_size: i32, mut var_return_size: i32, mut var_threads: i32, mut var_func: i32) -> DbRef {
  todo!("native function n_parallel_for")
}


fn n_parallel_for_light(stores: &mut Stores, mut var_input: DbRef, mut var_element_size: i32, mut var_return_size: i32, mut var_threads: i32, mut var_func: i32) -> DbRef {
  todo!("native function n_parallel_for_light")
}


fn n___add_int(stores: &mut Stores, mut var_a: i32, mut var_b: i32) -> i32 { //block_1: integer
  cr_call_push("__add_int", "/home/lima.guest/loft/default/01_code.loft", 968);
  let _call_guard = codegen_runtime::CallGuard;
  return ops::op_add_int((var_a), (var_b))
  } /*block_1: integer*/

fn n_sum_of(stores: &mut Stores, mut var_v: DbRef) -> i32 { //block_1: integer
  cr_call_push("sum_of", "/home/lima.guest/loft/default/01_code.loft", 971);
  let _call_guard = codegen_runtime::CallGuard;
  return { //reduce_2: integer
    let mut var__reduce_acc_1: i32 = 0_i32;
    let mut var__reduce_vec_2: DbRef = var_v;
    let mut var__reduce_idx_3: i32 = -1_i32;
    'l3: loop { //reduce loop_3
      let mut var__reduce_elm_4: i32 = { //iter next_4: integer
        var__reduce_idx_3 = ops::op_add_int((var__reduce_idx_3), (1_i32));
        {{let db = (vector::get_vector(&(var__reduce_vec_2), (4_i32) as u32, (var__reduce_idx_3), &stores.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}
        } /*iter next_4: integer*/;
      if !(ops::op_conv_bool_from_int((var__reduce_elm_4))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      var__reduce_acc_1 = n___add_int(stores, var__reduce_acc_1, var__reduce_elm_4);
      } /*reduce loop_3*/;
    var__reduce_acc_1
    } /*reduce_2: integer*/
  } /*block_1: integer*/

fn t_1T_OpLt(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> bool {
  todo!("native function t_1T_OpLt")
}


fn t_1T_OpAdd(stores: &mut Stores, mut var_self: DbRef, mut var_other: DbRef) -> DbRef {
  todo!("native function t_1T_OpAdd")
}


fn t_10FileResult_ok(stores: &mut Stores, mut var_self: u8) -> bool { //block_1: boolean
  return (if ((var_self) as u8) == 255 { i32::MIN } else { i32::from(((var_self) as u8)) }) == (if ((1_u8) as u8) == 255 { i32::MIN } else { i32::from(((1_u8) as u8)) })
  } /*block_1: boolean*/

fn t_4File_content(stores: &mut Stores, mut var_self: DbRef, mut var_result: &mut String) -> Str { //block_1: text["result"]
  *var_result = "".to_string();
  let mut var_txt: String = "".to_string();
  OpGetFileText(stores, var_self, &mut var_txt);
  *var_result += &*(&var_txt);
  ;
  return Str::new(&*var_result)
  } /*block_1: text["result"]*/

fn t_4File_lines(stores: &mut Stores, mut var_self: DbRef, mut var_result: DbRef) -> DbRef { //block_1: vector<text>["result"]
  let mut var___work_1: String = "".to_string();
  if var_result.rec != 0 { vector::clear_vector(&var_result, &mut stores.allocations); };
  let _pre_5 = { //default ref_2: ref(reference)["__work_1"]
    var___work_1 = "".to_string();
    &mut var___work_1
    } /*default ref_2: ref(reference)["__work_1"]*/;
  let mut var_c: String = t_4File_content(stores, var_self, _pre_5).to_string();
  let mut var_p: i32 = 0_i32;
  let mut var_prev_cr: bool = false;
  { //For block_3: void
    let mut var_ch__index: i32 = 0_i32;
    let mut var_ch__next: i32 = 0_i32;
    'l4: loop { //For loop_4
      let mut var_ch: i32 = { //for text next_5: character
        var_ch__index = var_ch__next;
        let mut var__for_result_1: i32 = (ops::text_character((&var_c), (var_ch__next))) as u32 as i32;
        var_ch__next = ops::op_add_int((var_ch__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_5: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_ch)))) { //break_6: void
        break;
        } /*break_6: void*/ else {()};
      { //block_7: void
        let mut var__elm_2: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
        let mut var__elm_3: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
        if (if (ops::to_char(var_ch)) == char::from(0) { i32::MIN } else { (ops::to_char(var_ch)) as i32 }) == (if (char::from_u32(10_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(10_u32).unwrap_or('\0')) as i32 }) { //block_8: void
          let mut var_e: i32 = var_ch__index;
          if var_prev_cr { //block_9: void
            {vector::pre_alloc_vector(&(var_result), (1_i32) as u32, (4_i32) as u32, &mut stores.allocations);};
            var__elm_2 = OpNewRecord(stores, var_result, 23_i32, 65535_i32);
            {{let db = (var__elm_2); let s_val = (&*(OpGetTextSub(&var_c, var_p, ops::op_min_int((var_e), (1_i32))))).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
            OpFinishRecord(stores, var_result, var__elm_2, 23_i32, 65535_i32);
            } /*block_9: void*/ else { //block_10: void
            {vector::pre_alloc_vector(&(var_result), (1_i32) as u32, (4_i32) as u32, &mut stores.allocations);};
            var__elm_3 = OpNewRecord(stores, var_result, 23_i32, 65535_i32);
            {{let db = (var__elm_3); let s_val = (&*(OpGetTextSub(&var_c, var_p, var_e))).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
            OpFinishRecord(stores, var_result, var__elm_3, 23_i32, 65535_i32);
            } /*block_10: void*/;
          var_p = var_ch__next;
          } /*block_8: void*/ else {()};
        var_prev_cr = (if (ops::to_char(var_ch)) == char::from(0) { i32::MIN } else { (ops::to_char(var_ch)) as i32 }) == (if (char::from_u32(13_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(13_u32).unwrap_or('\0')) as i32 });
        } /*block_7: void*/;
      } /*For loop_4*/;
    } /*For block_3: void*/;
  let mut var__elm_4: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
  if (var_p) < (t_4text_len(stores, &var_c)) { //block_11: void
    {vector::pre_alloc_vector(&(var_result), (1_i32) as u32, (4_i32) as u32, &mut stores.allocations);};
    var__elm_4 = OpNewRecord(stores, var_result, 23_i32, 65535_i32);
    let _pre_7 = t_4text_len(stores, &var_c);
    let _pre_6 = OpGetTextSub(&var_c, var_p, _pre_7);
    {{let db = (var__elm_4); let s_val = (&*(_pre_6)).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
    OpFinishRecord(stores, var_result, var__elm_4, 23_i32, 65535_i32);
    } /*block_11: void*/ else {()};
  ;
  ;
  return var_result
  } /*block_1: vector<text>["result"]*/

fn t_4text_split(stores: &mut Stores, mut var_self: &str, mut var_separator: i32, mut var_result: DbRef) -> DbRef { //block_1: vector<text>["result"]
  if var_result.rec != 0 { vector::clear_vector(&var_result, &mut stores.allocations); };
  let mut var_p: i32 = 0_i32;
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        let mut var__elm_2: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
        if (if (ops::to_char(var_c)) == char::from(0) { i32::MIN } else { (ops::to_char(var_c)) as i32 }) == (if (ops::to_char(var_separator)) == char::from(0) { i32::MIN } else { (ops::to_char(var_separator)) as i32 }) { //block_7: void
          {vector::pre_alloc_vector(&(var_result), (1_i32) as u32, (4_i32) as u32, &mut stores.allocations);};
          var__elm_2 = OpNewRecord(stores, var_result, 23_i32, 65535_i32);
          {{let db = (var__elm_2); let s_val = (&*(OpGetTextSub(var_self, var_p, var_c__index))).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
          OpFinishRecord(stores, var_result, var__elm_2, 23_i32, 65535_i32);
          var_p = var_c__next;
          } /*block_7: void*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  let mut var__elm_3: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
  if (0_i32) < (t_4text_len(stores, var_self)) { //block_8: void
    {vector::pre_alloc_vector(&(var_result), (1_i32) as u32, (4_i32) as u32, &mut stores.allocations);};
    var__elm_3 = OpNewRecord(stores, var_result, 23_i32, 65535_i32);
    let _pre_7 = t_4text_len(stores, var_self);
    let _pre_6 = OpGetTextSub(var_self, var_p, _pre_7);
    {{let db = (var__elm_3); let s_val = (&*(_pre_6)).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
    OpFinishRecord(stores, var_result, var__elm_3, 23_i32, 65535_i32);
    } /*block_8: void*/ else {()};
  return var_result
  } /*block_1: vector<text>["result"]*/

fn t_6vector_join(stores: &mut Stores, mut var_self: DbRef, mut var_jn_sep: &str, mut var_jn_result: &mut String) -> Str { //block_1: text["jn_result"]
  *var_jn_result = "".to_string();
  { //For block_2: void
    let mut var_jn_part__count: i32 = 0_i32;
    let mut var__vector_1: DbRef = var_self;
    let mut var_jn_part__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_jn_part = { //iter next_4: text
        var_jn_part__index = ops::op_add_int((var_jn_part__index), (1_i32));
        {{let db = (vector::get_vector(&(var__vector_1), (4_i32) as u32, (var_jn_part__index), &stores.allocations)); let store = stores.store(&db); store.get_str(store.get_int(db.rec, db.pos + (0_i32) as u32) as u32)}}
        } /*iter next_4: text*/.to_string();
      if !((&var_jn_part) != loft::state::STRING_NULL) { //break_5: void
        ;
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((var_jn_part__count) == (0_i32)) { //block_7: void
          *var_jn_result += &*(var_jn_sep);
          } /*block_7: void*/ else {()};
        *var_jn_result += &*(&var_jn_part);
        } /*block_6: void*/;
      var_jn_part__count = ops::op_add_int((var_jn_part__count), (1_i32));
      ;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return Str::new(&*var_jn_result)
  } /*block_1: text["jn_result"]*/

fn n_valid_path(stores: &mut Stores, mut var_path: &str) -> bool { //block_1: boolean
  cr_call_push("valid_path", "/home/lima.guest/loft/default/02_files.loft", 160);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var_depth: i32 = 0_i32;
  let mut var_start: i32 = 0_i32;
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_path), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        let mut var_part: String = "".to_string();
        if if (if (ops::to_char(var_c)) == char::from(0) { i32::MIN } else { (ops::to_char(var_c)) as i32 }) == (if (char::from_u32(47_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(47_u32).unwrap_or('\0')) as i32 }) {true} else {(if (ops::to_char(var_c)) == char::from(0) { i32::MIN } else { (ops::to_char(var_c)) as i32 }) == (if (char::from_u32(92_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(92_u32).unwrap_or('\0')) as i32 })} { //block_7: void
          var_part = OpGetTextSub(var_path, var_start, var_c__index).to_string();
          var_start = var_c__next;
          if (&var_part) == ("..") { //block_8: void
            var_depth = ops::op_min_int((var_depth), (1_i32));
            if (var_depth) < (0_i32) { //block_9: never
              let mut var___ret_1: bool = false;
              ;
              return var___ret_1
              } /*block_9: never*/ else {()};
            } /*block_8: void*/ else {if if (&var_part) != (".") {(0_i32) < (t_4text_len(stores, &var_part))} else {false} { //block_10: void
              var_depth = ops::op_add_int((var_depth), (1_i32));
              } /*block_10: void*/ else {()}};
          } /*block_7: void*/ else {()};
        ;
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  if (var_start) < (t_4text_len(stores, var_path)) { //block_11: void
    let _pre_6 = t_4text_len(stores, var_path);
    let mut var_part: String = OpGetTextSub(var_path, var_start, _pre_6).to_string();
    if (&var_part) == ("..") { //block_12: void
      var_depth = ops::op_min_int((var_depth), (1_i32));
      } /*block_12: void*/ else {()};
    ;
    } /*block_11: void*/ else {()};
  return (0_i32) <= (var_depth)
  } /*block_1: boolean*/

fn n_file(stores: &mut Stores, mut var_path: &str, mut var_result: DbRef) -> DbRef { //block_1: ref(File)["result"]
  cr_call_push("file", "/home/lima.guest/loft/default/02_files.loft", 188);
  let _call_guard = codegen_runtime::CallGuard;
  var_result = OpDatabase(stores, var_result, 27_i32);
  {{let db = (var_result); let s_val = (var_path).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (24_i32) as u32, s_pos as i32);}};
  {{let db = (var_result); stores.store_mut(&db).set_int(db.rec, db.pos + (28_i32) as u32, (i32::MIN));}};
  {{let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + (0_i32) as u32, (0_i64));}};
  {{let db = (var_result); stores.store_mut(&db).set_byte(db.rec, db.pos + (32_i32) as u32, 0, i32::from(((0_u8) as u8)));}};
  {{let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + (8_i32) as u32, (0_i64));}};
  {{let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + (16_i32) as u32, (0_i64));}};
  if n_valid_path(stores, var_path) { //block_2: void
    stores.get_file(&(var_result));
    } /*block_2: void*/ else { //block_3: void
    {{let db = (var_result); stores.store_mut(&db).set_byte(db.rec, db.pos + (32_i32) as u32, 0, i32::from(((5_u8) as u8)));}};
    } /*block_3: void*/;
  return var_result
  } /*block_1: ref(File)["result"]*/

fn n_exists(stores: &mut Stores, mut var_path: &str) -> bool { //block_1: boolean
  cr_call_push("exists", "/home/lima.guest/loft/default/02_files.loft", 200);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___lift_1: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  var___lift_1 = n_file(stores, var_path, var___ref_1);
  let _pre_6 = {{let db = (var___lift_1); let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i32) as u32, 0); if r < 0 { 255u8 } else { r as u8 }}};
  let mut var___ret_1: bool = {({if ((_pre_6) as u8) == 255 { i32::MIN } else { i32::from(((_pre_6) as u8)) }}) != (if ((5_u8) as u8) == 255 { i32::MIN } else { i32::from(((5_u8) as u8)) })};
  OpFreeRef(stores, var___lift_1, "var___lift_1"); var___lift_1.store_nr = u16::MAX;
  OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
  return var___ret_1
  } /*block_1: boolean*/

fn t_4File_exists(stores: &mut Stores, mut var_both: DbRef) -> bool { //block_1: boolean
  let _pre_7 = {{let db = (var_both); let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i32) as u32, 0); if r < 0 { 255u8 } else { r as u8 }}};
  return {({if ((_pre_7) as u8) == 255 { i32::MIN } else { i32::from(((_pre_7) as u8)) }}) != (if ((5_u8) as u8) == 255 { i32::MIN } else { i32::from(((5_u8) as u8)) })}
  } /*block_1: boolean*/

fn n_delete(stores: &mut Stores, mut var_path: &str) -> u8 { //block_1: FileResult
  cr_call_push("delete", "/home/lima.guest/loft/default/02_files.loft", 212);
  let _call_guard = codegen_runtime::CallGuard;
  if !(n_exists(stores, var_path)) { //block_2: never
    return 2_u8
    } /*block_2: never*/ else {()};
  if codegen_runtime::fs_delete((var_path)) { //block_3: never
    return 1_u8
    } /*block_3: never*/ else {()};
  return 6_u8
  } /*block_1: FileResult*/

fn n_move(stores: &mut Stores, mut var_from: &str, mut var_to: &str) -> u8 { //block_1: FileResult
  cr_call_push("move", "/home/lima.guest/loft/default/02_files.loft", 221);
  let _call_guard = codegen_runtime::CallGuard;
  if if !(n_valid_path(stores, var_to)) {true} else {!(n_valid_path(stores, var_from))} { //block_2: never
    return 2_u8
    } /*block_2: never*/ else {()};
  if !(n_exists(stores, var_from)) { //block_3: never
    return 2_u8
    } /*block_3: never*/ else {()};
  if n_exists(stores, var_to) { //block_4: never
    return 6_u8
    } /*block_4: never*/ else {()};
  if codegen_runtime::fs_move((var_from), (var_to)) { //block_5: never
    return 1_u8
    } /*block_5: never*/ else {()};
  return 6_u8
  } /*block_1: FileResult*/

fn n_mkdir(stores: &mut Stores, mut var_path: &str) -> u8 { //block_1: FileResult
  cr_call_push("mkdir", "/home/lima.guest/loft/default/02_files.loft", 231);
  let _call_guard = codegen_runtime::CallGuard;
  if !(n_valid_path(stores, var_path)) { //block_2: never
    return 2_u8
    } /*block_2: never*/ else {()};
  if codegen_runtime::fs_mkdir((var_path)) { //block_3: never
    return 1_u8
    } /*block_3: never*/ else {()};
  return 6_u8
  } /*block_1: FileResult*/

fn n_mkdir_all(stores: &mut Stores, mut var_path: &str) -> u8 { //block_1: FileResult
  cr_call_push("mkdir_all", "/home/lima.guest/loft/default/02_files.loft", 239);
  let _call_guard = codegen_runtime::CallGuard;
  if !(n_valid_path(stores, var_path)) { //block_2: never
    return 2_u8
    } /*block_2: never*/ else {()};
  if codegen_runtime::fs_mkdir_all((var_path)) { //block_3: never
    return 1_u8
    } /*block_3: never*/ else {()};
  return 6_u8
  } /*block_1: FileResult*/

fn t_4File_set_file_size(stores: &mut Stores, mut var_self: DbRef, mut var_size: i64) -> u8 { //block_1: FileResult
  let _pre_8 = {{let db = (var_self); let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i32) as u32, 0); if r < 0 { 255u8 } else { r as u8 }}};
  if {({if ((_pre_8) as u8) == 255 { i32::MIN } else { i32::from(((_pre_8) as u8)) }}) == (if ((4_u8) as u8) == 255 { i32::MIN } else { i32::from(((4_u8) as u8)) })} { //block_2: never
    return 4_u8
    } /*block_2: never*/ else {()};
  let _pre_9 = {{let db = (var_self); let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i32) as u32, 0); if r < 0 { 255u8 } else { r as u8 }}};
  if {({if ((_pre_9) as u8) == 255 { i32::MIN } else { i32::from(((_pre_9) as u8)) }}) == (if ((5_u8) as u8) == 255 { i32::MIN } else { i32::from(((5_u8) as u8)) })} { //block_3: never
    return 2_u8
    } /*block_3: never*/ else {()};
  if (var_size) < (0_i64) { //block_4: never
    return 6_u8
    } /*block_4: never*/ else {()};
  if OpTruncateFile(stores, var_self, var_size) { //block_5: never
    return 1_u8
    } /*block_5: never*/ else {()};
  return 6_u8
  } /*block_1: FileResult*/

fn t_4File_files(stores: &mut Stores, mut var_self: DbRef, mut var_result: DbRef) -> DbRef { //block_1: vector<ref(File)>["result"]
  if var_result.rec != 0 { vector::clear_vector(&var_result, &mut stores.allocations); };
  let _pre_10 = {{let db = (var_self); let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i32) as u32, 0); if r < 0 { 255u8 } else { r as u8 }}};
  if {({if ((_pre_10) as u8) == 255 { i32::MIN } else { i32::from(((_pre_10) as u8)) }}) == (if ((4_u8) as u8) == 255 { i32::MIN } else { i32::from(((4_u8) as u8)) })} { //block_2: void
    let _pre_10 = {{let db = (var_self); let store = stores.store(&db); store.get_str(store.get_int(db.rec, db.pos + (24_i32) as u32) as u32)}};
    {stores.get_dir((&*(_pre_10)), &(var_result))};
    } /*block_2: void*/ else {()};
  return var_result
  } /*block_1: vector<ref(File)>["result"]*/

fn t_4File_write(stores: &mut Stores, mut var_self: DbRef, mut var_v: &str) {
}


fn n_env_variables(stores: &mut Stores) -> DbRef {
  todo!("native function n_env_variables")
}


fn n_env_variable(stores: &mut Stores, mut var_name: &str) -> Str {
  todo!("native function n_env_variable")
}


fn t_4text_starts_with(stores: &mut Stores, mut var_self: &str, mut var_value: &str) -> bool {
  todo!("native function t_4text_starts_with")
}


fn t_4text_ends_with(stores: &mut Stores, mut var_self: &str, mut var_value: &str) -> bool {
  todo!("native function t_4text_ends_with")
}


fn t_4text_trim(stores: &mut Stores, mut var_both: &str) -> Str {
  todo!("native function t_4text_trim")
}


fn t_4text_trim_start(stores: &mut Stores, mut var_self: &str) -> Str {
  todo!("native function t_4text_trim_start")
}


fn t_4text_trim_end(stores: &mut Stores, mut var_self: &str) -> Str {
  todo!("native function t_4text_trim_end")
}


fn t_4text_find(stores: &mut Stores, mut var_self: &str, mut var_value: &str) -> i32 {
  todo!("native function t_4text_find")
}


fn t_4text_rfind(stores: &mut Stores, mut var_self: &str, mut var_value: &str) -> i32 {
  todo!("native function t_4text_rfind")
}


fn t_4text_contains(stores: &mut Stores, mut var_self: &str, mut var_value: &str) -> bool {
  todo!("native function t_4text_contains")
}


fn t_4text_replace(stores: &mut Stores, mut var_self: &str, mut var_value: &str, mut var_with: &str) -> Str {
  todo!("native function t_4text_replace")
}


fn t_4text_to_lowercase(stores: &mut Stores, mut var_self: &str) -> Str {
  todo!("native function t_4text_to_lowercase")
}


fn t_4text_to_uppercase(stores: &mut Stores, mut var_self: &str) -> Str {
  todo!("native function t_4text_to_uppercase")
}


fn t_4text_is_lowercase(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_lowercase()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_lowercase(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_lowercase")
}


fn t_4text_is_uppercase(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_uppercase()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_uppercase(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_uppercase")
}


fn t_4text_is_numeric(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_numeric()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_numeric(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_numeric")
}


fn t_4text_is_alphanumeric(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_alphanumeric()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_alphanumeric(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_alphanumeric")
}


fn t_4text_is_alphabetic(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_alphabetic()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_alphabetic(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_alphabetic")
}


fn t_4text_is_whitespace(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_whitespace()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_whitespace(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_whitespace")
}


fn t_4text_is_control(stores: &mut Stores, mut var_self: &str) -> bool { //block_1: boolean
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    'l3: loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = (ops::text_character((var_self), (var_c__next))) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(ops::op_conv_bool_from_character((ops::to_char(var_c)))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((ops::to_char(var_c)).is_control()) { //block_7: never
          return false
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return true
  } /*block_1: boolean*/

fn t_9character_is_control(stores: &mut Stores, mut var_self: i32) -> bool {
  todo!("native function t_9character_is_control")
}


fn n_join(stores: &mut Stores, mut var_parts: DbRef, mut var_sep: &str, mut var_result: &mut String) -> Str { //block_1: text["result"]
  cr_call_push("join", "/home/lima.guest/loft/default/03_text.loft", 101);
  let _call_guard = codegen_runtime::CallGuard;
  *var_result = "".to_string();
  { //For block_2: void
    let mut var_p__count: i32 = 0_i32;
    let mut var__vector_1: DbRef = var_parts;
    let mut var_p__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_p = { //iter next_4: text
        var_p__index = ops::op_add_int((var_p__index), (1_i32));
        {{let db = (vector::get_vector(&(var__vector_1), (4_i32) as u32, (var_p__index), &stores.allocations)); let store = stores.store(&db); store.get_str(store.get_int(db.rec, db.pos + (0_i32) as u32) as u32)}}
        } /*iter next_4: text*/.to_string();
      if !((&var_p) != loft::state::STRING_NULL) { //break_5: void
        ;
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((var_p__count) == (0_i32)) { //block_7: void
          *var_result += &*(var_sep);
          } /*block_7: void*/ else {()};
        *var_result += &*(&var_p);
        } /*block_6: void*/;
      var_p__count = ops::op_add_int((var_p__count), (1_i32));
      ;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return Str::new(&*var_result)
  } /*block_1: text["result"]*/

fn n_arguments(stores: &mut Stores) -> DbRef {
  todo!("native function n_arguments")
}


fn n_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {
  todo!("native function n_directory")
}


fn n_user_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {
  todo!("native function n_user_directory")
}


fn n_program_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {
  todo!("native function n_program_directory")
}


fn n_source_dir(stores: &mut Stores) -> Str {
  todo!("native function n_source_dir")
}


fn n_exhausted(stores: &mut Stores, mut var_gen: DbRef) -> bool { //block_1: boolean
  cr_call_push("exhausted", "/home/lima.guest/loft/default/05_coroutine.loft", 17);
  let _call_guard = codegen_runtime::CallGuard;
  return loft::coroutine_host::coroutine_is_exhausted(var_gen)
  } /*block_1: boolean*/

fn n_json_parse(stores: &mut Stores, mut var_raw: &str) -> DbRef {
  todo!("native function n_json_parse")
}


fn n_json_errors(stores: &mut Stores) -> Str {
  todo!("native function n_json_errors")
}


fn t_9JsonValue_field(stores: &mut Stores, mut var_self: DbRef, mut var_name: &str) -> DbRef {
  todo!("native function t_9JsonValue_field")
}


fn t_9JsonValue_item(stores: &mut Stores, mut var_self: DbRef, mut var_index: i32) -> DbRef {
  todo!("native function t_9JsonValue_item")
}


fn t_9JsonValue_len(stores: &mut Stores, mut var_self: DbRef) -> i32 {
  todo!("native function t_9JsonValue_len")
}


fn t_9JsonValue_as_text(stores: &mut Stores, mut var_self: DbRef) -> Str {
  todo!("native function t_9JsonValue_as_text")
}


fn t_9JsonValue_as_number(stores: &mut Stores, mut var_self: DbRef) -> f64 {
  todo!("native function t_9JsonValue_as_number")
}


fn t_9JsonValue_as_long(stores: &mut Stores, mut var_self: DbRef) -> i64 {
  todo!("native function t_9JsonValue_as_long")
}


fn t_9JsonValue_as_bool(stores: &mut Stores, mut var_self: DbRef) -> bool {
  todo!("native function t_9JsonValue_as_bool")
}


fn t_9JsonValue_kind(stores: &mut Stores, mut var_self: DbRef) -> Str {
  todo!("native function t_9JsonValue_kind")
}


fn t_9JsonValue_keys(stores: &mut Stores, mut var_self: DbRef) -> DbRef {
  todo!("native function t_9JsonValue_keys")
}


fn t_9JsonValue_fields(stores: &mut Stores, mut var_self: DbRef) -> DbRef {
  todo!("native function t_9JsonValue_fields")
}


fn t_9JsonValue_has_field(stores: &mut Stores, mut var_self: DbRef, mut var_name: &str) -> bool {
  todo!("native function t_9JsonValue_has_field")
}


fn t_9JsonValue_to_json(stores: &mut Stores, mut var_self: DbRef) -> Str {
  todo!("native function t_9JsonValue_to_json")
}


fn t_9JsonValue_to_json_pretty(stores: &mut Stores, mut var_self: DbRef) -> Str {
  todo!("native function t_9JsonValue_to_json_pretty")
}


fn n_json_null(stores: &mut Stores) -> DbRef {
  todo!("native function n_json_null")
}


fn n_json_bool(stores: &mut Stores, mut var_v: bool) -> DbRef {
  todo!("native function n_json_bool")
}


fn n_json_number(stores: &mut Stores, mut var_v: f64) -> DbRef {
  todo!("native function n_json_number")
}


fn n_json_string(stores: &mut Stores, mut var_v: &str) -> DbRef {
  todo!("native function n_json_string")
}


fn n_json_array(stores: &mut Stores, mut var_items: DbRef) -> DbRef {
  todo!("native function n_json_array")
}


fn n_json_object(stores: &mut Stores, mut var_fields: DbRef) -> DbRef {
  todo!("native function n_json_object")
}


fn n_struct_from_jsonvalue(stores: &mut Stores, mut var_v: DbRef, mut var_struct_kt: i32) -> DbRef {
  todo!("native function n_struct_from_jsonvalue")
}


fn n_hex_rotation(stores: &mut Stores, mut var_h: DbRef) -> i32 { //block_1: integer
  cr_call_push("hex_rotation", "/home/lima.guest/moros/lib/moros_map/src/types.loft", 26);
  let _call_guard = codegen_runtime::CallGuard;
  return {ops::op_logical_and_int(({{let db = (var_h); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (12_i32) as u32)} }}), (31_i32))}
  } /*block_1: integer*/

fn n_hex_spawn_flag(stores: &mut Stores, mut var_h: DbRef) -> bool { //block_1: boolean
  cr_call_push("hex_spawn_flag", "/home/lima.guest/moros/lib/moros_map/src/types.loft", 29);
  let _call_guard = codegen_runtime::CallGuard;
  return {({ops::op_logical_and_int(({ops::op_shift_right_int(({{let db = (var_h); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (12_i32) as u32)} }}), (5_i32))}), (1_i32))}) == (1_i32)}
  } /*block_1: boolean*/

fn n_hex_waypoint_flag(stores: &mut Stores, mut var_h: DbRef) -> bool { //block_1: boolean
  cr_call_push("hex_waypoint_flag", "/home/lima.guest/moros/lib/moros_map/src/types.loft", 32);
  let _call_guard = codegen_runtime::CallGuard;
  return {({ops::op_logical_and_int(({ops::op_shift_right_int(({{let db = (var_h); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (12_i32) as u32)} }}), (6_i32))}), (1_i32))}) == (1_i32)}
  } /*block_1: boolean*/

fn n_map_empty(stores: &mut Stores) -> DbRef { //block_1: ref(Map)
  cr_call_push("map_empty", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 24);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  return { //Object_2: ref(Map)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 77_i32);
    {{let db = (var___ref_1); let s_val = ("untitled").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + (0_i32) as u32, s_pos as i32);}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_2: ref(Map)["__ref_1"]*/
  } /*block_1: ref(Map)*/

fn n_map_to_json(stores: &mut Stores, mut var_m: DbRef, mut var___work_1: &mut String) -> Str { //block_1: text["__work_1"]
  cr_call_push("map_to_json", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 39);
  let _call_guard = codegen_runtime::CallGuard;
  *var___work_1 = "".to_string();
  return Str::new({ //Formatted string_2: text["__work_1"]
    *var___work_1 = "".to_string();
    OpFormatDatabase(stores, &mut var___work_1, var_m, 77_i32, 2_i32);
    &*var___work_1
    } /*Formatted string_2: text["__work_1"]*/)
  } /*block_1: text["__work_1"]*/

fn n_map_from_json(stores: &mut Stores, mut var_json: &str, mut var_result: DbRef) -> DbRef { //block_1: ref(Map)["result"]
  cr_call_push("map_from_json", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 45);
  let _call_guard = codegen_runtime::CallGuard;
  if (var_json) == ("") { //block_2: never
    return n_map_empty(stores)
    } /*block_2: never*/ else {()};
  var_result = db_from_text(stores, (var_json), (77_u16));
  if (&*(n_json_errors(stores))) != ("") { //block_3: never
    return n_map_empty(stores)
    } /*block_3: never*/ else {()};
  return var_result
  } /*block_1: ref(Map)["result"]*/

fn n_map_has_chunk(stores: &mut Stores, mut var_m: DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32) -> bool { //block_1: boolean
  cr_call_push("map_has_chunk", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 55);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32};
    let mut var_hc_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_hc_c: DbRef = { //iter next_4: ref(Chunk)
        var_hc_c__index = ops::op_add_int((var_hc_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_hc_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_hc_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_hc_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_hc_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_hc_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          return true
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return false
  } /*block_1: boolean*/

fn n_build_chunk(stores: &mut Stores, mut var_cx: i32, mut var_cy: i32, mut var_cz: i32) -> DbRef { //block_1: ref(Chunk)
  cr_call_push("build_chunk", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 69);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var___vdb_1: DbRef = stores.null_named("var___vdb_1");
  var___vdb_1 = OpDatabase(stores, var___vdb_1, 67_i32);
  let mut var_hexes: DbRef = DbRef {store_nr: (var___vdb_1).store_nr, rec: (var___vdb_1).rec, pos: (var___vdb_1).pos + (0_i32) as u32};
  {{let db = (var___vdb_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
  { //For block_2: void
    let mut var____index: i32 = i32::MIN;
    'l3: loop { //For loop_3
      let mut var__: i32 = { //Iter range_4: integer
        var____index = if !(ops::op_conv_bool_from_int((var____index))) {0_i32} else {ops::op_add_int((var____index), (1_i32))};
        if (4_i32) <= (var____index) {break} else {()};
        var____index
        } /*Iter range_4: integer*/;
      { //block_5: void
        {vector::pre_alloc_vector(&(var_hexes), (1_i32) as u32, (28_i32) as u32, &mut stores.allocations);};
        let mut var__elm_1: DbRef = OpNewRecord(stores, var_hexes, 66_i32, 65535_i32);
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
        {{let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
        OpFinishRecord(stores, var_hexes, var__elm_1, 66_i32, 65535_i32);
        } /*block_5: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  { //Object_6: ref(Chunk)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 65_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (var_cx));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (var_cy));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (var_cz));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {stores.vector_add(&(DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + (12_i32) as u32}), &(var_hexes), (64_u16));};
    OpFreeRef(stores, var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX;
    return var___ref_1
    } /*Object_6: ref(Chunk)["__ref_1"]*/
  } /*block_1: ref(Chunk)*/

fn n_map_ensure_chunk(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32) { //block_1: void
  cr_call_push("map_ensure_chunk", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 79);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___lift_1: DbRef = DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_ec_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_ec_c: DbRef = { //iter next_4: ref(Chunk)
        var_ec_c__index = ops::op_add_int((var_ec_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_ec_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_ec_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_ec_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_ec_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_ec_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  let mut var__elm_2: DbRef = OpNewRecord(stores, *var_m, 77_i32, 1_i32);
  var___lift_1 = n_build_chunk(stores, var_cx, var_cy, var_cz);
  OpCopyRecord(stores, var___lift_1, var__elm_2, 65_i32);
  OpFinishRecord(stores, *var_m, var__elm_2, 77_i32, 1_i32);
  OpFreeRef(stores, var___lift_1, "var___lift_1"); var___lift_1.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_map_set_hex(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32, mut var_h: DbRef) { //block_1: void
  cr_call_push("map_set_hex", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 91);
  let _call_guard = codegen_runtime::CallGuard;
  n_map_ensure_chunk(stores, var_m, var_q, var_r, var_cy);
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_hx: i32 = ops::op_rem_int((var_q), (32_i32));
  let mut var_hz: i32 = ops::op_rem_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((var_hx), (32_i32))), (var_hz));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_sh_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_sh_c: DbRef = { //iter next_4: ref(Chunk)
        var_sh_c__index = ops::op_add_int((var_sh_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_sh_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_sh_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_sh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_sh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_sh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          OpCopyRecord(stores, var_h, vector::get_vector(&(DbRef {store_nr: (var_sh_c).store_nr, rec: (var_sh_c).rec, pos: (var_sh_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations), 64_i32);
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  } /*block_1: void*/

fn n_map_paint_material(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32, mut var_material: i32) { //block_1: void
  cr_call_push("map_paint_material", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 107);
  let _call_guard = codegen_runtime::CallGuard;
  n_map_ensure_chunk(stores, var_m, var_q, var_r, var_cy);
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((ops::op_rem_int((var_q), (32_i32))), (32_i32))), (ops::op_rem_int((var_r), (32_i32))));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_pm_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_pm_c: DbRef = { //iter next_4: ref(Chunk)
        var_pm_c__index = ops::op_add_int((var_pm_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_pm_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_pm_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_pm_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_pm_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_pm_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          {{let db = (vector::get_vector(&(DbRef {store_nr: (var_pm_c).store_nr, rec: (var_pm_c).rec, pos: (var_pm_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (var_material));}};
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  } /*block_1: void*/

fn n_map_set_height(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32, mut var_height: i32) { //block_1: void
  cr_call_push("map_set_height", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 121);
  let _call_guard = codegen_runtime::CallGuard;
  n_map_ensure_chunk(stores, var_m, var_q, var_r, var_cy);
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((ops::op_rem_int((var_q), (32_i32))), (32_i32))), (ops::op_rem_int((var_r), (32_i32))));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_msh_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_msh_c: DbRef = { //iter next_4: ref(Chunk)
        var_msh_c__index = ops::op_add_int((var_msh_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_msh_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_msh_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_msh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_msh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_msh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          {{let db = (vector::get_vector(&(DbRef {store_nr: (var_msh_c).store_nr, rec: (var_msh_c).rec, pos: (var_msh_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (var_height));}};
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  } /*block_1: void*/

fn n_map_place_item(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32, mut var_item: i32, mut var_rotation: i32) { //block_1: void
  cr_call_push("map_place_item", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 135);
  let _call_guard = codegen_runtime::CallGuard;
  n_map_ensure_chunk(stores, var_m, var_q, var_r, var_cy);
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((ops::op_rem_int((var_q), (32_i32))), (32_i32))), (ops::op_rem_int((var_r), (32_i32))));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_pi_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_pi_c: DbRef = { //iter next_4: ref(Chunk)
        var_pi_c__index = ops::op_add_int((var_pi_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_pi_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_pi_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_pi_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_pi_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_pi_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          {{let db = (vector::get_vector(&(DbRef {store_nr: (var_pi_c).store_nr, rec: (var_pi_c).rec, pos: (var_pi_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (var_item));}};
          let mut var_flags: i32 = {ops::op_logical_and_int(({{let db = (vector::get_vector(&(DbRef {store_nr: (var_pi_c).store_nr, rec: (var_pi_c).rec, pos: (var_pi_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (12_i32) as u32)} }}), (ops::op_shift_left_int((3_i32), (5_i32))))};
          {{let db = (vector::get_vector(&(DbRef {store_nr: (var_pi_c).store_nr, rec: (var_pi_c).rec, pos: (var_pi_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (ops::op_logical_or_int((ops::op_logical_and_int((var_rotation), (31_i32))), (var_flags))));}};
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  } /*block_1: void*/

fn n_map_set_wall(stores: &mut Stores, mut var_m: &mut DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32, mut var_dir: i32, mut var_wall: i32) { //block_1: void
  cr_call_push("map_set_wall", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 151);
  let _call_guard = codegen_runtime::CallGuard;
  n_map_ensure_chunk(stores, var_m, var_q, var_r, var_cy);
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((ops::op_rem_int((var_q), (32_i32))), (32_i32))), (ops::op_rem_int((var_r), (32_i32))));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (*var_m).store_nr, rec: (*var_m).rec, pos: (*var_m).pos + (4_i32) as u32};
    let mut var_sw_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_sw_c: DbRef = { //iter next_4: ref(Chunk)
        var_sw_c__index = ops::op_add_int((var_sw_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_sw_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_sw_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_sw_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_sw_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_sw_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: never
          if (var_dir) == (0_i32) { //block_8: void
            {{let db = (vector::get_vector(&(DbRef {store_nr: (var_sw_c).store_nr, rec: (var_sw_c).rec, pos: (var_sw_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (var_wall));}};
            } /*block_8: void*/ else {()};
          if (var_dir) == (1_i32) { //block_9: void
            {{let db = (vector::get_vector(&(DbRef {store_nr: (var_sw_c).store_nr, rec: (var_sw_c).rec, pos: (var_sw_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (var_wall));}};
            } /*block_9: void*/ else {()};
          if (var_dir) == (2_i32) { //block_10: void
            {{let db = (vector::get_vector(&(DbRef {store_nr: (var_sw_c).store_nr, rec: (var_sw_c).rec, pos: (var_sw_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (var_wall));}};
            } /*block_10: void*/ else {()};
          return ()
          } /*block_7: never*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  } /*block_1: void*/

fn n_map_get_hex(stores: &mut Stores, mut var_m: DbRef, mut var_q: i32, mut var_r: i32, mut var_cy: i32) -> DbRef { //block_1: ref(Hex)
  cr_call_push("map_get_hex", "/home/lima.guest/moros/lib/moros_map/src/moros_map.loft", 170);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var_cx: i32 = ops::op_div_int((var_q), (32_i32));
  let mut var_cz: i32 = ops::op_div_int((var_r), (32_i32));
  let mut var_hx: i32 = ops::op_rem_int((var_q), (32_i32));
  let mut var_hz: i32 = ops::op_rem_int((var_r), (32_i32));
  let mut var_idx: i32 = ops::op_add_int((ops::op_mul_int((var_hx), (32_i32))), (var_hz));
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32};
    let mut var_gh_c__index: i32 = -1_i32;
    'l3: loop { //For loop_3
      let mut var_gh_c: DbRef = { //iter next_4: ref(Chunk)
        var_gh_c__index = ops::op_add_int((var_gh_c__index), (1_i32));
        vector::get_vector(&(var__vector_1), (16_i32) as u32, (var_gh_c__index), &stores.allocations)
        } /*iter next_4: ref(Chunk)*/;
      if !((var_gh_c).rec != 0) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if if if {({{let db = (var_gh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (var_cx)} {{({{let db = (var_gh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (var_cy)}} else {false} {{({{let db = (var_gh_c); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (var_cz)}} else {false} { //block_7: void
          if if (0_i32) <= (var_idx) {(var_idx) < (t_6vector_len(stores, DbRef {store_nr: (var_gh_c).store_nr, rec: (var_gh_c).rec, pos: (var_gh_c).pos + (12_i32) as u32}))} else {false} { //block_8: never
            OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
            return vector::get_vector(&(DbRef {store_nr: (var_gh_c).store_nr, rec: (var_gh_c).rec, pos: (var_gh_c).pos + (12_i32) as u32}), (28_i32) as u32, (var_idx), &stores.allocations)
            } /*block_8: never*/ else {()};
          } /*block_7: void*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return { //Object_9: ref(Hex)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 64_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_9: ref(Hex)["__ref_1"]*/
  } /*block_1: ref(Hex)*/

fn n_test_ensure_chunk(stores: &mut Stores) { //block_1: void
  cr_call_push("test_ensure_chunk", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 4);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var_m: DbRef = n_map_empty(stores);
  n_map_ensure_chunk(stores, &mut var_m, 0_i32, 0_i32, 0_i32);
  let _pre_11 = (0_i32) < (t_6vector_len(stores, DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32}));
  n_assert(stores, _pre_11, "chunk created", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 7_i32);
  let _pre_12 = (t_6vector_len(stores, DbRef {store_nr: (vector::get_vector(&(DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32}), (16_i32) as u32, (0_i32), &stores.allocations)).store_nr, rec: (vector::get_vector(&(DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32}), (16_i32) as u32, (0_i32), &stores.allocations)).rec, pos: (vector::get_vector(&(DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32}), (16_i32) as u32, (0_i32), &stores.allocations)).pos + (12_i32) as u32})) == (4_i32);
  n_assert(stores, _pre_12, "hexes allocated", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 8_i32);
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_ensure_chunk_idempotent(stores: &mut Stores) { //block_1: void
  cr_call_push("test_ensure_chunk_idempotent", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 11);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var_m: DbRef = n_map_empty(stores);
  n_map_ensure_chunk(stores, &mut var_m, 0_i32, 0_i32, 0_i32);
  n_map_ensure_chunk(stores, &mut var_m, 0_i32, 0_i32, 0_i32);
  let _pre_13 = (t_6vector_len(stores, DbRef {store_nr: (var_m).store_nr, rec: (var_m).rec, pos: (var_m).pos + (4_i32) as u32})) == (1_i32);
  n_assert(stores, _pre_13, "still one chunk", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 15_i32);
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_set_hex(stores: &mut Stores) { //block_1: void
  cr_call_push("test_set_hex", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 18);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var_m: DbRef = n_map_empty(stores);
  let mut var_h: DbRef = stores.null_named("var_h");
  var_h = OpDatabase(stores, var_h, 64_i32);
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (42_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (3_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
  {{let db = (var_h); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
  n_map_set_hex(stores, &mut var_m, 0_i32, 0_i32, 0_i32, var_h);
  let mut var_got: DbRef = stores.null_named("var_got");
    var_got = OpDatabase(stores, var_got, 64_i32);
    { let _src = n_map_get_hex(stores, var_m, 0_i32, 0_i32, 0_i32); OpCopyRecord(stores, _src, var_got, 64_i32); };
  let _pre_14 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (42_i32)};
  n_assert(stores, _pre_14, "height written", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 23_i32);
  let _pre_15 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (3_i32)};
  n_assert(stores, _pre_15, "material written", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 24_i32);
  OpFreeRef(stores, var_got, "var_got"); var_got.store_nr = u16::MAX;
  OpFreeRef(stores, var_h, "var_h"); var_h.store_nr = u16::MAX;
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_paint_material(stores: &mut Stores) { //block_1: void
  cr_call_push("test_paint_material", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 27);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var_m: DbRef = n_map_empty(stores);
  let _pre_16 = { //Object_2: ref(Hex)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 64_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (10_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (1_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_2: ref(Hex)["__ref_1"]*/;
  n_map_set_hex(stores, &mut var_m, 1_i32, 1_i32, 0_i32, _pre_16);
  n_map_paint_material(stores, &mut var_m, 1_i32, 1_i32, 0_i32, 5_i32);
  let mut var_got: DbRef = stores.null_named("var_got");
    var_got = OpDatabase(stores, var_got, 64_i32);
    { let _src = n_map_get_hex(stores, var_m, 1_i32, 1_i32, 0_i32); OpCopyRecord(stores, _src, var_got, 64_i32); };
  let _pre_17 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (5_i32)};
  n_assert(stores, _pre_17, "material changed", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 32_i32);
  let _pre_18 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (10_i32)};
  n_assert(stores, _pre_18, "height preserved", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 33_i32);
  OpFreeRef(stores, var_got, "var_got"); var_got.store_nr = u16::MAX;
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_set_height_op(stores: &mut Stores) { //block_1: void
  cr_call_push("test_set_height_op", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 36);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var_m: DbRef = n_map_empty(stores);
  let _pre_19 = { //Object_2: ref(Hex)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 64_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (2_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_2: ref(Hex)["__ref_1"]*/;
  n_map_set_hex(stores, &mut var_m, 0_i32, 0_i32, 0_i32, _pre_19);
  n_map_set_height(stores, &mut var_m, 0_i32, 0_i32, 0_i32, 100_i32);
  let mut var_got: DbRef = stores.null_named("var_got");
    var_got = OpDatabase(stores, var_got, 64_i32);
    { let _src = n_map_get_hex(stores, var_m, 0_i32, 0_i32, 0_i32); OpCopyRecord(stores, _src, var_got, 64_i32); };
  let _pre_20 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i32) as u32)} }}) == (100_i32)};
  n_assert(stores, _pre_20, "height changed", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 41_i32);
  let _pre_21 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (2_i32)};
  n_assert(stores, _pre_21, "material preserved", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 42_i32);
  OpFreeRef(stores, var_got, "var_got"); var_got.store_nr = u16::MAX;
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_place_item_op(stores: &mut Stores) { //block_1: void
  cr_call_push("test_place_item_op", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 45);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var_m: DbRef = n_map_empty(stores);
  let _pre_22 = { //Object_2: ref(Hex)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 64_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (1_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_2: ref(Hex)["__ref_1"]*/;
  n_map_set_hex(stores, &mut var_m, 0_i32, 0_i32, 0_i32, _pre_22);
  n_map_place_item(stores, &mut var_m, 0_i32, 0_i32, 0_i32, 4_i32, 7_i32);
  let mut var_got: DbRef = stores.null_named("var_got");
    var_got = OpDatabase(stores, var_got, 64_i32);
    { let _src = n_map_get_hex(stores, var_m, 0_i32, 0_i32, 0_i32); OpCopyRecord(stores, _src, var_got, 64_i32); };
  let _pre_23 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (8_i32) as u32)} }}) == (4_i32)};
  n_assert(stores, _pre_23, "item placed", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 50_i32);
  let _pre_24 = (n_hex_rotation(stores, var_got)) == (7_i32);
  n_assert(stores, _pre_24, "rotation set", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 51_i32);
  let _pre_25 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (4_i32) as u32)} }}) == (1_i32)};
  n_assert(stores, _pre_25, "material preserved", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 52_i32);
  OpFreeRef(stores, var_got, "var_got"); var_got.store_nr = u16::MAX;
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
  } /*block_1: void*/

fn n_test_set_wall_op(stores: &mut Stores) { //block_1: void
  cr_call_push("test_set_wall_op", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 55);
  let _call_guard = codegen_runtime::CallGuard;
  let mut var___ref_1: DbRef = stores.null_named("var___ref_1");
  let mut var_m: DbRef = n_map_empty(stores);
  let _pre_26 = { //Object_2: ref(Hex)["__ref_1"]
    var___ref_1 = OpDatabase(stores, var___ref_1, 64_i32);
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (4_i32) as u32, (1_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (0_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (8_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (12_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (16_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (20_i32) as u32, (0_i32));}};
    {{let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + (24_i32) as u32, (0_i32));}};
    var___ref_1
    } /*Object_2: ref(Hex)["__ref_1"]*/;
  n_map_set_hex(stores, &mut var_m, 0_i32, 0_i32, 0_i32, _pre_26);
  n_map_set_wall(stores, &mut var_m, 0_i32, 0_i32, 0_i32, 0_i32, 3_i32);
  n_map_set_wall(stores, &mut var_m, 0_i32, 0_i32, 0_i32, 2_i32, 5_i32);
  let mut var_got: DbRef = stores.null_named("var_got");
    var_got = OpDatabase(stores, var_got, 64_i32);
    { let _src = n_map_get_hex(stores, var_m, 0_i32, 0_i32, 0_i32); OpCopyRecord(stores, _src, var_got, 64_i32); };
  let _pre_27 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (16_i32) as u32)} }}) == (3_i32)};
  n_assert(stores, _pre_27, "wall_n set", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 61_i32);
  let _pre_28 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (24_i32) as u32)} }}) == (5_i32)};
  n_assert(stores, _pre_28, "wall_se set", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 62_i32);
  let _pre_29 = {({{let db = (var_got); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (20_i32) as u32)} }}) == (0_i32)};
  n_assert(stores, _pre_29, "wall_ne untouched", "/home/lima.guest/moros/lib/moros_map/tests/edit.loft", 63_i32);
  OpFreeRef(stores, var_got, "var_got"); var_got.store_nr = u16::MAX;
  OpFreeRef(stores, var_m, "var_m"); var_m.store_nr = u16::MAX;
  OpFreeRef(stores, var___ref_1, "var___ref_1"); var___ref_1.store_nr = u16::MAX;
  } /*block_1: void*/


fn main() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_map_empty(&mut stores);
    eprintln!(">> test_paint_material");
    n_test_paint_material(&mut stores);
    eprintln!(">> done");
}
