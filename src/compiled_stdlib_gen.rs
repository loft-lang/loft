// @generated — DO NOT EDIT BY HAND.
//
// The standard library's compiled loft bodies (@PLN181, `src/compiled_stdlib.rs`): the
// functions `compiled_stdlib::export_set` picks, as the native backend emits them, with
// one shared-store bridge each.  Regenerate after changing default/*.loft:
//     make compiled-stdlib
// `tests/compiled_stdlib.rs::compiled_stdlib_up_to_date` fails when this file is stale.


use crate as loft;
unsafe extern "C" {
}
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::ops;
use loft::vector;
use loft::hash;
use loft::tree;
use loft::codegen_runtime;
use loft::codegen_runtime::*;
use loft::narrow;
static __C_LIBS: &[(&str, &str)] = &[
];
static __C_LIB_SYMS: &[(&str, &[&str])] = &[
];
fn init(cell: &std::cell::UnsafeCell<Stores>) {
    let db: &mut Stores = unsafe { &mut *cell.get() };
    let t0: u16 = 0;
    let t1: u16 = 1;
    let t2: u16 = 2;
    let t3: u16 = 3;
    let t4: u16 = 4;
    let t5: u16 = 5;
    let t6: u16 = 6;
    let _ = (t0, t1, t2, t3, t4, t5, t6); // suppress unused-let warnings for unreferenced base types
    let t7 = db.vector(t0);
    let _ = t7; // may be unused
    let t8 = db.enumerate("Ordering");
    let t9 = db.enumerate("FieldValue");
    let t10 = db.structure("__typevar_Self", 0);
    let t11 = db.structure("__typevar_T#1", 0);
    let t12 = db.structure("__typevar_T#2", 0);
    let t13 = db.structure("__typevar_T#3", 0);
    let t14 = db.structure("main_vector<__typevar_T#1>", 0);
    let vec_vector = db.vector(t11);
    db.field(t14, "vector", vec_vector);
    db.set_field_nullable(t14, "vector", true);
    let t15 = db.vector(t11);
    let _ = t15; // may be unused
    let t16 = db.structure("__typevar_K#1", 0);
    let t17 = db.structure("main_vector<__typevar_K#1>", 0);
    let vec_vector = db.vector(t16);
    db.field(t17, "vector", vec_vector);
    db.set_field_nullable(t17, "vector", true);
    let t18 = db.vector(t16);
    let _ = t18; // may be unused
    let t19 = db.structure("main_vector<integer>", 0);
    let vec_vector = db.vector(t0);
    db.field(t19, "vector", vec_vector);
    db.set_field_nullable(t19, "vector", true);
    let t20 = db.structure("main_vector<__typevar_T#3>", 0);
    let vec_vector = db.vector(t13);
    db.field(t20, "vector", vec_vector);
    db.set_field_nullable(t20, "vector", true);
    let t21 = db.vector(t13);
    let _ = t21; // may be unused
    let t22 = db.structure("__typevar_K#2", 0);
    let t23 = db.structure("__typevar_U#1", 0);
    let t24 = db.structure("__typevar_T#4", 0);
    let t25 = db.structure("main_vector<__typevar_T#4>", 0);
    let vec_vector = db.vector(t24);
    db.field(t25, "vector", vec_vector);
    db.set_field_nullable(t25, "vector", true);
    let t26 = db.vector(t24);
    let _ = t26; // may be unused
    let t27 = db.structure("__typevar_AssertValue#1", 0);
    let t28 = db.structure("FvBool", 1);
    let byte_enum = db.byte(0, false);
    db.field(t28, "enum", byte_enum);
    db.set_field_nullable(t28, "enum", true);
    db.field(t28, "v", t4);
    let t29 = db.byte(0, false);
    let _ = t29; // may be unused
    let t30 = db.structure("FvInt", 2);
    let byte_enum = db.byte(0, false);
    db.field(t30, "enum", byte_enum);
    db.set_field_nullable(t30, "enum", true);
    db.field(t30, "v", 0);
    let t31 = db.structure("FvLong", 3);
    let byte_enum = db.byte(0, false);
    db.field(t31, "enum", byte_enum);
    db.set_field_nullable(t31, "enum", true);
    db.field(t31, "v", 0);
    let t32 = db.structure("FvFloat", 4);
    let byte_enum = db.byte(0, false);
    db.field(t32, "enum", byte_enum);
    db.set_field_nullable(t32, "enum", true);
    db.field(t32, "v", t3);
    let t33 = db.structure("FvSingle", 5);
    let byte_enum = db.byte(0, false);
    db.field(t33, "enum", byte_enum);
    db.set_field_nullable(t33, "enum", true);
    db.field(t33, "v", t2);
    let t34 = db.structure("FvChar", 6);
    let byte_enum = db.byte(0, false);
    db.field(t34, "enum", byte_enum);
    db.set_field_nullable(t34, "enum", true);
    db.field(t34, "v", t6);
    let t35 = db.structure("FvText", 7);
    let byte_enum = db.byte(0, false);
    db.field(t35, "enum", byte_enum);
    db.set_field_nullable(t35, "enum", true);
    db.field(t35, "v", t5);
    let t36 = db.structure("StructField", 0);
    db.field(t36, "name", t5);
    db.field(t36, "value", t9);
    db.field(t36, "nullable", t4);
    let t37 = db.vector(t23);
    let _ = t37; // may be unused
    let t38 = db.structure("main_vector<__typevar_U#1>", 0);
    let vec_vector = db.vector(t23);
    db.field(t38, "vector", vec_vector);
    db.set_field_nullable(t38, "vector", true);
    let t39 = db.vector(t12);
    let _ = t39; // may be unused
    let t40 = db.structure("main_vector<__typevar_T#2>", 0);
    let vec_vector = db.vector(t12);
    db.field(t40, "vector", vec_vector);
    db.set_field_nullable(t40, "vector", true);
    let t41 = db.vector(t5);
    let _ = t41; // may be unused
    let t42 = db.enumerate("Format");
    let t43 = db.enumerate("FileResult");
    let t44 = db.structure("EnvVariable", 0);
    db.field(t44, "name", t5);
    db.field(t44, "value", t5);
    let t45 = db.structure("File", 0);
    db.field(t45, "path", t5);
    db.field(t45, "size", 0);
    db.field(t45, "format", t42);
    let int_ref = db.int(-2147483647, true);
    db.field(t45, "ref", int_ref);
    db.set_field_nullable(t45, "ref", true);
    db.field(t45, "current", 0);
    db.field(t45, "next", 0);
    let t46 = db.int(-2147483647, true);
    let _ = t46; // may be unused
    let t47 = db.structure("main_vector<text>", 0);
    let vec_vector = db.vector(t5);
    db.field(t47, "vector", vec_vector);
    db.set_field_nullable(t47, "vector", true);
    let t48 = db.structure("main_vector<File>", 0);
    let vec_vector = db.vector(t45);
    db.field(t48, "vector", vec_vector);
    db.set_field_nullable(t48, "vector", true);
    let t49 = db.vector(t45);
    let _ = t49; // may be unused
    let t50 = db.enumerate("ArgValue");
    let t51 = db.structure("NullVal", 1);
    let byte_enum = db.byte(0, false);
    db.field(t51, "enum", byte_enum);
    db.set_field_nullable(t51, "enum", true);
    let t52 = db.structure("BoolVal", 2);
    let byte_enum = db.byte(0, false);
    db.field(t52, "enum", byte_enum);
    db.set_field_nullable(t52, "enum", true);
    db.field(t52, "b", t4);
    let t53 = db.structure("IntVal", 3);
    let byte_enum = db.byte(0, false);
    db.field(t53, "enum", byte_enum);
    db.set_field_nullable(t53, "enum", true);
    db.field(t53, "n", 0);
    let t54 = db.structure("LongVal", 4);
    let byte_enum = db.byte(0, false);
    db.field(t54, "enum", byte_enum);
    db.set_field_nullable(t54, "enum", true);
    db.field(t54, "n", 0);
    let t55 = db.structure("FloatVal", 5);
    let byte_enum = db.byte(0, false);
    db.field(t55, "enum", byte_enum);
    db.set_field_nullable(t55, "enum", true);
    db.field(t55, "f", t3);
    let t56 = db.structure("SingleVal", 6);
    let byte_enum = db.byte(0, false);
    db.field(t56, "enum", byte_enum);
    db.set_field_nullable(t56, "enum", true);
    db.field(t56, "f", t2);
    let t57 = db.structure("CharVal", 7);
    let byte_enum = db.byte(0, false);
    db.field(t57, "enum", byte_enum);
    db.set_field_nullable(t57, "enum", true);
    db.field(t57, "c", t6);
    let t58 = db.structure("TextVal", 8);
    let byte_enum = db.byte(0, false);
    db.field(t58, "enum", byte_enum);
    db.set_field_nullable(t58, "enum", true);
    db.field(t58, "t", t5);
    let t59 = db.structure("RefVal", 9);
    let byte_enum = db.byte(0, false);
    db.field(t59, "enum", byte_enum);
    db.set_field_nullable(t59, "enum", true);
    db.field(t59, "store", 0);
    db.field(t59, "rec", 0);
    db.field(t59, "pos", 0);
    let t60 = db.structure("FnVal", 10);
    let byte_enum = db.byte(0, false);
    db.field(t60, "enum", byte_enum);
    db.set_field_nullable(t60, "enum", true);
    db.field(t60, "d_nr", 0);
    let t61 = db.structure("OtherVal", 11);
    let byte_enum = db.byte(0, false);
    db.field(t61, "enum", byte_enum);
    db.set_field_nullable(t61, "enum", true);
    db.field(t61, "description", t5);
    let t62 = db.structure("ArgInfo", 0);
    db.field(t62, "name", t5);
    db.field(t62, "type_name", t5);
    db.field(t62, "value", t50);
    let t63 = db.structure("VarInfo", 0);
    db.field(t63, "name", t5);
    db.field(t63, "type_name", t5);
    db.field(t63, "value", t50);
    let t64 = db.structure("StackFrame", 0);
    db.field(t64, "function", t5);
    db.field(t64, "file", t5);
    db.field(t64, "line", 0);
    let vec_arguments = db.vector(t62);
    db.field(t64, "arguments", vec_arguments);
    let vec_variables = db.vector(t63);
    db.field(t64, "variables", vec_variables);
    let t65 = db.vector(t62);
    let _ = t65; // may be unused
    let t66 = db.vector(t63);
    let _ = t66; // may be unused
    let t67 = db.structure("main_vector<ArgInfo>", 0);
    let vec_vector = db.vector(t62);
    db.field(t67, "vector", vec_vector);
    db.set_field_nullable(t67, "vector", true);
    let t68 = db.structure("main_vector<VarInfo>", 0);
    let vec_vector = db.vector(t63);
    db.field(t68, "vector", vec_vector);
    db.set_field_nullable(t68, "vector", true);
    let t69 = db.enumerate("CoroutineStatus");
    let t70 = db.enumerate("JsonValue");
    let t71 = db.structure("JNull", 1);
    let byte_enum = db.byte(0, false);
    db.field(t71, "enum", byte_enum);
    db.set_field_nullable(t71, "enum", true);
    let t72 = db.structure("JBool", 2);
    let byte_enum = db.byte(0, false);
    db.field(t72, "enum", byte_enum);
    db.set_field_nullable(t72, "enum", true);
    db.field(t72, "value", t4);
    let t73 = db.structure("JNumber", 3);
    let byte_enum = db.byte(0, false);
    db.field(t73, "enum", byte_enum);
    db.set_field_nullable(t73, "enum", true);
    db.field(t73, "value", t3);
    let t74 = db.structure("JString", 4);
    let byte_enum = db.byte(0, false);
    db.field(t74, "enum", byte_enum);
    db.set_field_nullable(t74, "enum", true);
    db.field(t74, "value", t5);
    let t75 = db.structure("JArray", 5);
    let byte_enum = db.byte(0, false);
    db.field(t75, "enum", byte_enum);
    db.set_field_nullable(t75, "enum", true);
    let vec_items = db.vector(t70);
    db.field(t75, "items", vec_items);
    let t76 = db.vector(t70);
    let _ = t76; // may be unused
    let t77 = db.structure("JObject", 6);
    let byte_enum = db.byte(0, false);
    db.field(t77, "enum", byte_enum);
    db.set_field_nullable(t77, "enum", true);
    let t78 = db.structure("JsonField", 0);
    db.field(t78, "name", t5);
    db.field(t78, "value", t70);
    let vec_fields = db.vector(t78);
    db.field(t77, "fields", vec_fields);
    let t79 = db.vector(t78);
    let _ = t79; // may be unused
    let t80 = db.structure("JInteger", 7);
    let byte_enum = db.byte(0, false);
    db.field(t80, "enum", byte_enum);
    db.set_field_nullable(t80, "enum", true);
    db.field(t80, "value", 0);
    let t81 = db.structure("main_vector<JsonValue>", 0);
    let vec_vector = db.vector(t70);
    db.field(t81, "vector", vec_vector);
    db.set_field_nullable(t81, "vector", true);
    let t82 = db.structure("main_vector<JsonField>", 0);
    let vec_vector = db.vector(t78);
    db.field(t82, "vector", vec_vector);
    db.set_field_nullable(t82, "vector", true);
    let t83 = db.enumerate("TypeKind");
    let t84 = db.enumerate("CollectionKind");
    let t85 = db.structure("FieldInfo", 0);
    db.field(t85, "name", t5);
    db.field(t85, "type_name", t5);
    db.field(t85, "position", 0);
    db.field(t85, "kind", t83);
    db.field(t85, "nullable", t4);
    let t86 = db.structure("KeyInfo", 0);
    db.field(t86, "name", t5);
    db.field(t86, "position", 0);
    db.field(t86, "ascending", t4);
    let t87 = db.structure("VariantInfo", 0);
    db.field(t87, "name", t5);
    db.field(t87, "tag", 0);
    let t88 = db.structure("TypeInfo", 0);
    db.field(t88, "name", t5);
    db.field(t88, "kind", t83);
    db.field(t88, "size", 0);
    let vec_fields = db.vector(t85);
    db.field(t88, "fields", vec_fields);
    let vec_variants = db.vector(t87);
    db.field(t88, "variants", vec_variants);
    db.field(t88, "element", t5);
    db.field(t88, "collection", t84);
    let vec_keys = db.vector(t86);
    db.field(t88, "keys", vec_keys);
    let t89 = db.vector(t85);
    let _ = t89; // may be unused
    let t90 = db.vector(t87);
    let _ = t90; // may be unused
    let t91 = db.vector(t86);
    let _ = t91; // may be unused
    let t92 = db.structure("ValueInfo", 0);
    db.field(t92, "kind", t83);
    db.field(t92, "is_null", t4);
    db.field(t92, "i", 0);
    db.field(t92, "f", t3);
    db.field(t92, "t", t5);
    let t93 = db.structure("main_vector<FieldInfo>", 0);
    let vec_vector = db.vector(t85);
    db.field(t93, "vector", vec_vector);
    db.set_field_nullable(t93, "vector", true);
    let t94 = db.structure("main_vector<VariantInfo>", 0);
    let vec_vector = db.vector(t87);
    db.field(t94, "vector", vec_vector);
    db.set_field_nullable(t94, "vector", true);
    let t95 = db.structure("main_vector<KeyInfo>", 0);
    let vec_vector = db.vector(t86);
    db.field(t95, "vector", vec_vector);
    db.set_field_nullable(t95, "vector", true);
    db.value(t8, "Less", u16::MAX);
    db.value(t8, "Equal", u16::MAX);
    db.value(t8, "Greater", u16::MAX);
    db.value(t8, "then", u16::MAX);
    db.value(t9, "FvBool", t28);
    db.value(t9, "FvInt", t30);
    db.value(t9, "FvLong", t31);
    db.value(t9, "FvFloat", t32);
    db.value(t9, "FvSingle", t33);
    db.value(t9, "FvChar", t34);
    db.value(t9, "FvText", t35);
    db.value(t42, "TextFile", u16::MAX);
    db.value(t42, "LittleEndian", u16::MAX);
    db.value(t42, "BigEndian", u16::MAX);
    db.value(t42, "Directory", u16::MAX);
    db.value(t42, "NotExists", u16::MAX);
    db.value(t43, "Ok", u16::MAX);
    db.value(t43, "NotFound", u16::MAX);
    db.value(t43, "PermissionDenied", u16::MAX);
    db.value(t43, "IsDirectory", u16::MAX);
    db.value(t43, "NotEmpty", u16::MAX);
    db.value(t43, "Other", u16::MAX);
    db.value(t43, "ok", u16::MAX);
    db.value(t50, "NullVal", t51);
    db.value(t50, "BoolVal", t52);
    db.value(t50, "IntVal", t53);
    db.value(t50, "LongVal", t54);
    db.value(t50, "FloatVal", t55);
    db.value(t50, "SingleVal", t56);
    db.value(t50, "CharVal", t57);
    db.value(t50, "TextVal", t58);
    db.value(t50, "RefVal", t59);
    db.value(t50, "FnVal", t60);
    db.value(t50, "OtherVal", t61);
    db.value(t69, "Created", u16::MAX);
    db.value(t69, "Suspended", u16::MAX);
    db.value(t69, "Running", u16::MAX);
    db.value(t69, "Exhausted", u16::MAX);
    db.value(t70, "JNull", t71);
    db.value(t70, "JBool", t72);
    db.value(t70, "JNumber", t73);
    db.value(t70, "JString", t74);
    db.value(t70, "JArray", t75);
    db.value(t70, "JObject", t77);
    db.value(t70, "JInteger", t80);
    db.value(t70, "field", u16::MAX);
    db.value(t70, "item", u16::MAX);
    db.value(t70, "len", u16::MAX);
    db.value(t70, "as_text", u16::MAX);
    db.value(t70, "as_number", u16::MAX);
    db.value(t70, "as_long", u16::MAX);
    db.value(t70, "as_bool", u16::MAX);
    db.value(t70, "kind", u16::MAX);
    db.value(t70, "keys", u16::MAX);
    db.value(t70, "fields", u16::MAX);
    db.value(t70, "has_field", u16::MAX);
    db.value(t70, "to_json", u16::MAX);
    db.value(t70, "to_json_pretty", u16::MAX);
    db.value(t83, "IntegerKind", u16::MAX);
    db.value(t83, "LongKind", u16::MAX);
    db.value(t83, "SingleKind", u16::MAX);
    db.value(t83, "FloatKind", u16::MAX);
    db.value(t83, "BooleanKind", u16::MAX);
    db.value(t83, "TextKind", u16::MAX);
    db.value(t83, "CharacterKind", u16::MAX);
    db.value(t83, "RecordKind", u16::MAX);
    db.value(t83, "EnumKind", u16::MAX);
    db.value(t83, "VariantKind", u16::MAX);
    db.value(t83, "VectorKind", u16::MAX);
    db.value(t83, "KeyedKind", u16::MAX);
    db.value(t83, "RefKind", u16::MAX);
    db.value(t83, "OtherKind", u16::MAX);
    db.value(t84, "NotKeyed", u16::MAX);
    db.value(t84, "KeyedHash", u16::MAX);
    db.value(t84, "KeyedIndex", u16::MAX);
    db.value(t84, "KeyedSorted", u16::MAX);
    db.value(t84, "KeyedOrdered", u16::MAX);
    db.value(t84, "KeyedRadix", u16::MAX);
    db.value(t84, "KeyedTrie", u16::MAX);
    db.verify_schema_ids(&[
        "integer",
        "long",
        "single",
        "float",
        "boolean",
        "text",
        "character",
        "vector<integer>",
        "Ordering",
        "FieldValue",
        "__typevar_Self",
        "__typevar_T#1",
        "__typevar_T#2",
        "__typevar_T#3",
        "main_vector<__typevar_T#1>",
        "vector<__typevar_T#1>",
        "__typevar_K#1",
        "main_vector<__typevar_K#1>",
        "vector<__typevar_K#1>",
        "main_vector<integer>",
        "main_vector<__typevar_T#3>",
        "vector<__typevar_T#3>",
        "__typevar_K#2",
        "__typevar_U#1",
        "__typevar_T#4",
        "main_vector<__typevar_T#4>",
        "vector<__typevar_T#4>",
        "__typevar_AssertValue#1",
        "FvBool",
        "byte",
        "FvInt",
        "FvLong",
        "FvFloat",
        "FvSingle",
        "FvChar",
        "FvText",
        "StructField",
        "vector<__typevar_U#1>",
        "main_vector<__typevar_U#1>",
        "vector<__typevar_T#2>",
        "main_vector<__typevar_T#2>",
        "vector<text>",
        "Format",
        "FileResult",
        "EnvVariable",
        "File",
        "int<-2147483647,true>",
        "main_vector<text>",
        "main_vector<File>",
        "vector<File>",
        "ArgValue",
        "NullVal",
        "BoolVal",
        "IntVal",
        "LongVal",
        "FloatVal",
        "SingleVal",
        "CharVal",
        "TextVal",
        "RefVal",
        "FnVal",
        "OtherVal",
        "ArgInfo",
        "VarInfo",
        "StackFrame",
        "vector<ArgInfo>",
        "vector<VarInfo>",
        "main_vector<ArgInfo>",
        "main_vector<VarInfo>",
        "CoroutineStatus",
        "JsonValue",
        "JNull",
        "JBool",
        "JNumber",
        "JString",
        "JArray",
        "vector<JsonValue>",
        "JObject",
        "JsonField",
        "vector<JsonField>",
        "JInteger",
        "main_vector<JsonValue>",
        "main_vector<JsonField>",
        "TypeKind",
        "CollectionKind",
        "FieldInfo",
        "KeyInfo",
        "VariantInfo",
        "TypeInfo",
        "vector<FieldInfo>",
        "vector<VariantInfo>",
        "vector<KeyInfo>",
        "ValueInfo",
        "main_vector<FieldInfo>",
        "main_vector<VariantInfo>",
        "main_vector<KeyInfo>",
    ]);
    db.finish();
}

#[inline]
fn i_parse_errors(cell: &std::cell::UnsafeCell<Stores>) -> String {
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  loft::codegen_runtime::i_parse_errors(stores)
}


// loft:default/01_code.loft:774
#[inline]
fn t_7integer_min(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i64, mut var_b: i64) -> i64 { //block_1: integer
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/01_code.loft:775
  return if ((((var_self) as i64) <= ((var_b) as i64)) as u8) == 1 { //block_2: integer
    var_self
    } /*block_2: integer*/ else { //block_3: integer
    var_b
    } /*block_3: integer*/
  } /*block_1: integer*/

// loft:default/01_code.loft:779
#[inline]
fn t_7integer_max(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i64, mut var_b: i64) -> i64 { //block_1: integer
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/01_code.loft:780
  return if ((((var_b) as i64) <= ((var_self) as i64)) as u8) == 1 { //block_2: integer
    var_self
    } /*block_2: integer*/ else { //block_3: integer
    var_b
    } /*block_3: integer*/
  } /*block_1: integer*/

// loft:default/01_code.loft:784
#[inline]
fn t_7integer_clamp(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i64, mut var_lo: i64, mut var_hi: i64) -> i64 { //block_1: integer
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/01_code.loft:785
  let _pre_0 = t_7integer_max(cell, var_self, var_lo);
  return t_7integer_min(cell, _pre_0, var_hi)
  } /*block_1: integer*/

// loft:default/01_code.loft:929
#[inline]
fn t_4text_len(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> i64 { //block_1: integer
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/01_code.loft:930
  return {{ let __t = (var_self); if __t == loft::state::STRING_NULL { 0 } else { __t.chars().count() as i64 } }}
  } /*block_1: integer*/

// loft:default/01_code.loft:941
#[inline]
fn t_4text_size(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> i64 { //block_1: integer
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/01_code.loft:942
  return {{ let __t = (var_self); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}
  } /*block_1: integer*/

// loft:default/02_files.loft:158
#[inline]
fn t_4File_content(cell: &std::cell::UnsafeCell<Stores>, mut var_self: DbRef, mut var_result: &mut String) -> Str { //block_1: text["result"]?
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/02_files.loft:161
  let _pre_1 = {{let db = (var_self); if db.rec == 0 { 0u8 } else { let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i64) as u32, 0); if r < 0 { 255u8 } else { r as u8 } }}};
  if (((({{ let _v_v1 = ((_pre_1) as u8); if _v_v1 == 255 { i64::MIN } else { i64::from(_v_v1) } }}) as i64) == (({{ let _v_v1 = ((5_u8) as u8); if _v_v1 == 255 { i64::MIN } else { i64::from(_v_v1) } }}) as i64)) as u8) == 1 { //block_2: never
    return Str::new(loft::state::STRING_NULL)
    } /*block_2: never*/ else {()};
  // loft:default/02_files.loft:163
  let _pre_2 = {{let db = (var_self); if db.rec == 0 { 0u8 } else { let r = stores.store(&db).get_byte(db.rec, db.pos + (32_i64) as u32, 0); if r < 0 { 255u8 } else { r as u8 } }}};
  if (((({{ let _v_v1 = ((_pre_2) as u8); if _v_v1 == 255 { i64::MIN } else { i64::from(_v_v1) } }}) as i64) == (({{ let _v_v1 = ((4_u8) as u8); if _v_v1 == 255 { i64::MIN } else { i64::from(_v_v1) } }}) as i64)) as u8) == 1 { //block_3: never
    return Str::new(loft::state::STRING_NULL)
    } /*block_3: never*/ else {()};
  // loft:default/02_files.loft:164
  *var_result = ("").to_string();
  // loft:default/02_files.loft:165
  let mut var_txt: String = "".to_string();
  // loft:default/02_files.loft:166
  OpGetFileText(cell, var_self, &mut var_txt);
  // loft:default/02_files.loft:167
  (*var_result) += &*(&var_txt);
  // loft:default/02_files.loft:172
  if ((if ((((t_4text_size(cell, &*var_result)) as i64) == ((0_i64) as i64)) as u8) == 1 {({((0_i64) as i64) < (({{let db = (var_self); if db.rec == 0 { i64::MIN } else { stores.store(&db).get_int(db.rec, db.pos + (0_i64) as u32)} }}) as i64)} as u8)} else {({false} as u8)}) as u8) == 1 { //block_4: never
    ;
    return Str::new(loft::state::STRING_NULL)
    } /*block_4: never*/ else {()};
  // loft:default/02_files.loft:175
  ;
  return Str::new(&*var_result)
  } /*block_1: text["result"]?*/

// loft:default/02_files.loft:182
fn t_4File_lines(cell: &std::cell::UnsafeCell<Stores>, mut var_self: DbRef, mut var___retbuf: DbRef) -> DbRef { //block_1: vector<text>["__retbuf"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let mut var___vdb_1: DbRef = DbRef::NULL;
  var___vdb_1 = DbRef::NULL;
  let mut var___work_c1: String = "".to_string();
  if var___retbuf.rec != 0 { stores.clear_vector_release(&var___retbuf); };
  // loft:default/02_files.loft:183
  ();
  let mut var_result: DbRef = { if var___retbuf.store_nr == u16::MAX || var___retbuf.rec == 0 { var___retbuf = OpDatabase(cell, var___retbuf, 47_i32); } else { stores.clear_vector_release(&var___retbuf); } var___retbuf };
  {{ let _v_val = (0_i64); {let db = (var___vdb_1); let v = if _v_val == i64::MIN { i32::MIN } else { _v_val as i32 }; if db.rec != 0 { stores.store_mut(&db).set_i32_raw(db.rec, db.pos + (0_i64) as u32, v); }} }};
  // loft:default/02_files.loft:185
  let mut var_c: String = "".to_string();
  { //ncc_2: void
    let _pre_3 = { //default ref_3: ref(reference)["__work_c1"]
      var___work_c1 = "".to_string();
      &mut var___work_c1
      } /*default ref_3: ref(reference)["__work_c1"]*/;
    let mut var___ncc_1: String = t_4File_content(cell, var_self, _pre_3).to_string();
    if (((&var___ncc_1) != loft::state::STRING_NULL) as u8) == 1 {var_c = var___ncc_1.clone()} else {var_c = "".to_string()};
    } /*ncc_2: void*/;
  ;
  // loft:default/02_files.loft:186
  let mut var_p: i64 = 0_i64;
  // loft:default/02_files.loft:187
  let mut var_prev_cr: u8 = (false) as u8;
  // loft:default/02_files.loft:188
  { //For block_4: void
    let mut var_ch__index: i64 = 0_i64;
    let mut var__for_text_1: String = var_c.clone();
    let mut var_ch__next: i64 = 0_i64;
          if (((((&var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l5: loop { //For loop_5
      let mut var_ch: i32 = { let __tb = (&var__for_text_1).as_bytes(); let __ti = var_ch__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_ch__index = var_ch__next; var_ch__next = var_ch__next + 1; i32::from(__tb[__ti]) } else { //for text next_6: character
        var_ch__index = var_ch__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_ch__next); { let _v_v1 = (&var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_ch__next = ops::op_add_int((var_ch__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_ch__next) as i64) <= ((var_ch__index) as i64)) as u8) == 1 {var_ch__next = ops::op_add_int((var_ch__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_6: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (&var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_ch__index) as i64)) as u8) == 1 { //break_8: void
        break;
        } /*break_8: void*/ else {()};
      { //block_9: void
        // loft:default/02_files.loft:189
        let mut var__elm_1: DbRef = DbRef::NULL;
        let mut var__elm_2: DbRef = DbRef::NULL;
        if (((({{ let _v_v1 = (ops::to_char(var_ch)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } }}) as i64) == ((10_i64) as i64)) as u8) == 1 { //block_10: void
          // loft:default/02_files.loft:190
          let mut var_e: i64 = var_ch__index;
          // loft:default/02_files.loft:191
          if ((var_prev_cr) as u8) == 1 { //block_11: void
            {vector::pre_alloc_vector(&(var_result), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
            var__elm_1 = OpNewRecord(cell, var_result, 41_i32, 65535_i32);
            {{let db = (var__elm_1); let s_val = (&*(OpGetTextSub(&var_c, var_p, ops::op_min_int((var_e), (1_i64))))).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
            OpFinishRecord(cell, var_result, var__elm_1, 41_i32, 65535_i32);
            } /*block_11: void*/ else { //block_12: void
            {vector::pre_alloc_vector(&(var_result), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
            var__elm_2 = OpNewRecord(cell, var_result, 41_i32, 65535_i32);
            {{let db = (var__elm_2); let s_val = (&*(OpGetTextSub(&var_c, var_p, var_e))).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
            OpFinishRecord(cell, var_result, var__elm_2, 41_i32, 65535_i32);
            } /*block_12: void*/;
          // loft:default/02_files.loft:192
          var_p = var_ch__next;
          } /*block_10: void*/ else {()};
        // loft:default/02_files.loft:194
        var_prev_cr = ((({{ let _v_v1 = (ops::to_char(var_ch)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } }}) as i64) == ((13_i64) as i64)) as u8;
        } /*block_9: void*/;
      } /*For loop_5*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_4: void*/;
  // loft:default/02_files.loft:196
  let mut var__elm_3: DbRef = DbRef::NULL;
  if ((((var_p) as i64) < ((t_4text_size(cell, &var_c)) as i64)) as u8) == 1 { //block_13: void
    {vector::pre_alloc_vector(&(var_result), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
    var__elm_3 = OpNewRecord(cell, var_result, 41_i32, 65535_i32);
    let _pre_4 = t_4text_size(cell, &var_c);
    let _pre_3 = OpGetTextSub(&var_c, var_p, _pre_4);
    {{let db = (var__elm_3); let s_val = (&*(_pre_3)).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
    OpFinishRecord(cell, var_result, var__elm_3, 41_i32, 65535_i32);
    } /*block_13: void*/ else {()};
  // loft:default/02_files.loft:197
  { //one_buffer_vec_copy_14: vector<text>["__retbuf"]
    ();
    ();
    ;
    if var___vdb_1.store_nr != u16::MAX { OpFreeRef(cell,var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX; };
    ;
    return var___retbuf
    } /*one_buffer_vec_copy_14: vector<text>["__retbuf"]*/
  } /*block_1: vector<text>["__retbuf"]*/

// loft:default/02_files.loft:206
fn t_4text_split(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str, mut var_separator: i32, mut var___retbuf: DbRef) -> DbRef { //block_1: vector<text>["__retbuf"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let mut var___vdb_1: DbRef = DbRef::NULL;
  var___vdb_1 = DbRef::NULL;
  if var___retbuf.rec != 0 { stores.clear_vector_release(&var___retbuf); };
  // loft:default/02_files.loft:207
  ();
  let mut var_result: DbRef = { if var___retbuf.store_nr == u16::MAX || var___retbuf.rec == 0 { var___retbuf = OpDatabase(cell, var___retbuf, 47_i32); } else { stores.clear_vector_release(&var___retbuf); } var___retbuf };
  {{ let _v_val = (0_i64); {let db = (var___vdb_1); let v = if _v_val == i64::MIN { i32::MIN } else { _v_val as i32 }; if db.rec != 0 { stores.store_mut(&db).set_i32_raw(db.rec, db.pos + (0_i64) as u32, v); }} }};
  // loft:default/02_files.loft:208
  let mut var_p: i64 = 0_i64;
  // loft:default/02_files.loft:209
  { //For block_2: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l3: loop { //For loop_3
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_4: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_6: void
        break;
        } /*break_6: void*/ else {()};
      { //block_7: void
        // loft:default/02_files.loft:210
        let mut var__elm_1: DbRef = DbRef::NULL;
        if (((({{ let _v_v1 = (ops::to_char(var_c)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } }}) as i64) == (({{ let _v_v1 = (ops::to_char(var_separator)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } }}) as i64)) as u8) == 1 { //block_8: void
          // loft:default/02_files.loft:211
          {vector::pre_alloc_vector(&(var_result), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
          var__elm_1 = OpNewRecord(cell, var_result, 41_i32, 65535_i32);
          {{let db = (var__elm_1); let s_val = (&*(OpGetTextSub(var_self, var_p, var_c__index))).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
          OpFinishRecord(cell, var_result, var__elm_1, 41_i32, 65535_i32);
          // loft:default/02_files.loft:212
          var_p = var_c__next;
          } /*block_8: void*/ else {()};
        } /*block_7: void*/;
      } /*For loop_3*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_2: void*/;
  // loft:default/02_files.loft:218
  let mut var__elm_2: DbRef = DbRef::NULL;
  if ((((0_i64) as i64) < ((t_4text_len(cell, var_self)) as i64)) as u8) == 1 { //block_9: void
    {vector::pre_alloc_vector(&(var_result), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
    var__elm_2 = OpNewRecord(cell, var_result, 41_i32, 65535_i32);
    let _pre_4 = t_4text_size(cell, var_self);
    let _pre_3 = OpGetTextSub(var_self, var_p, _pre_4);
    {{let db = (var__elm_2); let s_val = (&*(_pre_3)).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
    OpFinishRecord(cell, var_result, var__elm_2, 41_i32, 65535_i32);
    } /*block_9: void*/ else {()};
  // loft:default/02_files.loft:219
  { //one_buffer_vec_copy_10: vector<text>["__retbuf"]
    ();
    ();
    if var___vdb_1.store_nr != u16::MAX { OpFreeRef(cell,var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX; };
    return var___retbuf
    } /*one_buffer_vec_copy_10: vector<text>["__retbuf"]*/
  } /*block_1: vector<text>["__retbuf"]*/

// loft:default/02_files.loft:229
fn t_4text_split_text(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str, mut var_separator: &str, mut var___retbuf: DbRef) -> DbRef { //block_1: vector<text>["__retbuf"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let mut var___vdb_1: DbRef = DbRef::NULL;
  var___vdb_1 = DbRef::NULL;
  if var___retbuf.rec != 0 { stores.clear_vector_release(&var___retbuf); };
  // loft:default/02_files.loft:230
  ();
  let mut var_out: DbRef = { if var___retbuf.store_nr == u16::MAX || var___retbuf.rec == 0 { var___retbuf = OpDatabase(cell, var___retbuf, 47_i32); } else { stores.clear_vector_release(&var___retbuf); } var___retbuf };
  {{ let _v_val = (0_i64); {let db = (var___vdb_1); let v = if _v_val == i64::MIN { i32::MIN } else { _v_val as i32 }; if db.rec != 0 { stores.store_mut(&db).set_i32_raw(db.rec, db.pos + (0_i64) as u32, v); }} }};
  // loft:default/02_files.loft:231
  let mut var_n: i64 = t_4text_size(cell, var_self);
  // loft:default/02_files.loft:232
  if ((((var_n) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    {stores.vector_replace(&(var___retbuf), &(var_out), (5_u16));};
          if var___vdb_1.store_nr != u16::MAX { OpFreeRef(cell,var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX; };
      return var___retbuf

    } /*block_2: never*/ else {()};
  // loft:default/02_files.loft:233
  let mut var_sl: i64 = t_4text_size(cell, var_separator);
  // loft:default/02_files.loft:234
  let mut var__elm_1: DbRef = DbRef::NULL;
  if ((((var_sl) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_3: never
    // loft:default/02_files.loft:235
    {vector::pre_alloc_vector(&(var_out), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
    var__elm_1 = OpNewRecord(cell, var_out, 41_i32, 65535_i32);
    {{let db = (var__elm_1); let s_val = (var_self).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
    OpFinishRecord(cell, var_out, var__elm_1, 41_i32, 65535_i32);
    // loft:default/02_files.loft:235
    {stores.vector_replace(&(var___retbuf), &(var_out), (5_u16));};
          if var___vdb_1.store_nr != u16::MAX { OpFreeRef(cell,var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX; };
      return var___retbuf

    } /*block_3: never*/ else {()};
  // loft:default/02_files.loft:237
  let mut var_p: i64 = 0_i64;
  // loft:default/02_files.loft:238
  let mut var_i: i64 = 0_i64;
  // loft:default/02_files.loft:239
  'l4: loop { //while_4
    if ((((((ops::op_add_int((var_i), (var_sl))) as i64) <= ((var_n) as i64)) as u8) != 1) as u8) == 1 { //break_5: void
      break;
      } /*break_5: void*/ else {()};
    { //block_6: void
      // loft:default/02_files.loft:240
      let mut var__elm_2: DbRef = DbRef::NULL;
      if ((ops::op_eq_text((&*(OpGetTextSub(var_self, var_i, ops::op_add_int((var_i), (var_sl))))), (var_separator))) as u8) == 1 { //block_7: void
        // loft:default/02_files.loft:241
        {vector::pre_alloc_vector(&(var_out), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
        var__elm_2 = OpNewRecord(cell, var_out, 41_i32, 65535_i32);
        {{let db = (var__elm_2); let s_val = (&*(OpGetTextSub(var_self, var_p, var_i))).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
        OpFinishRecord(cell, var_out, var__elm_2, 41_i32, 65535_i32);
        // loft:default/02_files.loft:242
        var_p = ops::op_add_int((var_i), (var_sl));
        // loft:default/02_files.loft:243
        var_i = var_p;
        } /*block_7: void*/ else { //block_8: void
        var_i = ops::op_add_int((var_i), (1_i64));
        } /*block_8: void*/;
      } /*block_6: void*/;
    } /*while_4*/;
  // loft:default/02_files.loft:246
  {vector::pre_alloc_vector(&(var_out), (1_i64) as u32, (4_i64) as u32, &mut stores.allocations);};
  let mut var__elm_3: DbRef = OpNewRecord(cell, var_out, 41_i32, 65535_i32);
  {{let db = (var__elm_3); let s_val = (&*(OpGetTextSub(var_self, var_p, var_n))).to_string(); if db.rec != 0 { let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_u32_raw(db.rec, db.pos + (0_i64) as u32, s_pos); }}};
  OpFinishRecord(cell, var_out, var__elm_3, 41_i32, 65535_i32);
  // loft:default/02_files.loft:247
  { //one_buffer_vec_copy_9: vector<text>["__retbuf"]
    ();
    ();
    if var___vdb_1.store_nr != u16::MAX { OpFreeRef(cell,var___vdb_1, "var___vdb_1"); var___vdb_1.store_nr = u16::MAX; };
    return var___retbuf
    } /*one_buffer_vec_copy_9: vector<text>["__retbuf"]*/
  } /*block_1: vector<text>["__retbuf"]*/

// loft:default/02_files.loft:1324
fn t_4text_resolve(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str, mut var_target: &str, mut var_t: &mut String, mut var___work_4: &mut String, mut var___work_2: &mut String) -> Str { //block_1: text["base", "t"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let mut var___work_p2_2: String = "".to_string();
  let mut var___work_p2_1: String = "".to_string();
  *var___work_2 = ("").to_string();
  let mut var___work_1: String = "".to_string();
  // loft:default/02_files.loft:1325
  let mut var_base: String = var_self.to_string();
  // loft:default/02_files.loft:1326
  *var_t = (var_target).to_string();
  // loft:default/02_files.loft:1328
  'l2: loop { //while_2
    if (({{ let _ha0 = if ((if ((((2_i64) as i64) <= ((t_4text_size(cell, &*var_t)) as i64)) as u8) == 1 {({(({{ let _ha0 = (stores.text_char_or_raise_runtime((&*var_t), (0_i64))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((46_i64) as i64)} as u8)} else {({false} as u8)}) as u8) == 1 {({(({{ let _ha0 = (stores.text_char_or_raise_runtime((&*var_t), (1_i64))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((47_i64) as i64)} as u8)} else {({false} as u8)}; ((_ha0) as u8) != 1 }}) as u8) == 1 { //break_3: void
      break;
      } /*break_3: void*/ else {()};
    { //block_4: void
      var___work_p2_1.clear();
      let _pre_4 = t_4text_size(cell, &*var_t);
      let _pre_3 = OpGetTextSub(&*var_t, 2_i64, _pre_4);
      var___work_p2_1 += &*(_pre_3);
      *var_t = (&var___work_p2_1).to_string();
      } /*block_4: void*/;
    } /*while_2*/;
  // loft:default/02_files.loft:1330
  'l5: loop { //while_5
    if (({{ let _ha0 = if ((if ((if ((((3_i64) as i64) <= ((t_4text_size(cell, &*var_t)) as i64)) as u8) == 1 {({(({{ let _ha0 = (stores.text_char_or_raise_runtime((&*var_t), (0_i64))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((46_i64) as i64)} as u8)} else {({false} as u8)}) as u8) == 1 {({(({{ let _ha0 = (stores.text_char_or_raise_runtime((&*var_t), (1_i64))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((46_i64) as i64)} as u8)} else {({false} as u8)}) as u8) == 1 {({(({{ let _ha0 = (stores.text_char_or_raise_runtime((&*var_t), (2_i64))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((47_i64) as i64)} as u8)} else {({false} as u8)}; ((_ha0) as u8) != 1 }}) as u8) == 1 { //break_6: void
      break;
      } /*break_6: void*/ else {()};
    { //block_7: void
      // loft:default/02_files.loft:1331
      let mut var_bn: i64 = t_4text_size(cell, &var_base);
      // loft:default/02_files.loft:1332
      let mut var_cut: i64 = var_bn;
      // loft:default/02_files.loft:1333
      let mut var_walked: u8 = (false) as u8;
      // loft:default/02_files.loft:1334
      'l8: loop { //while_8
        if ((((((0_i64) as i64) < ((var_cut) as i64)) as u8) != 1) as u8) == 1 { //break_9: void
          break;
          } /*break_9: void*/ else {()};
        { //block_10: void
          // loft:default/02_files.loft:1335
          if (((({{ let _ha0 = (stores.text_char_or_raise_runtime((&var_base), (ops::op_min_int((var_cut), (1_i64))))) as u32 as i32; { let _v_v1 = (ops::to_char(_ha0)); if _v_v1 == char::from(0) { i64::MIN } else { i64::from(_v_v1 as u32) } } }}) as i64) == ((47_i64) as i64)) as u8) == 1 { //block_11: never
            // loft:default/02_files.loft:1336
            var_cut = ops::op_min_int((var_cut), (1_i64));
            // loft:default/02_files.loft:1337
            var_walked = (true) as u8;
            // loft:default/02_files.loft:1338
            break
            } /*block_11: never*/ else {()};
          // loft:default/02_files.loft:1340
          var_cut = ops::op_min_int((var_cut), (1_i64));
          } /*block_10: void*/;
        } /*while_8*/;
      // loft:default/02_files.loft:1342
      if ((var_walked) as u8) == 1 { //block_12: void
        var___work_1.clear();
        var___work_1 += &*(OpGetTextSub(&var_base, 0_i64, var_cut));
        var_base = var___work_1.clone();
        } /*block_12: void*/ else { //block_13: void
        var_base = "".to_string();
        } /*block_13: void*/;
      // loft:default/02_files.loft:1343
      var___work_p2_2.clear();
      let _pre_4 = t_4text_size(cell, &*var_t);
      let _pre_3 = OpGetTextSub(&*var_t, 3_i64, _pre_4);
      var___work_p2_2 += &*(_pre_3);
      *var_t = (&var___work_p2_2).to_string();
      } /*block_7: void*/;
    } /*while_5*/;
  // loft:default/02_files.loft:1345
  if ((ops::op_eq_text((&var_base), (""))) as u8) == 1 { //block_14: never
    ;
    ;
    ;
    ;
    return Str::new(&*var_t)
    } /*block_14: never*/ else {()};
  // loft:default/02_files.loft:1346
  { //Formatted string_15: text["__work_2"]
    *var___work_2 = ("").to_string();
    ops::format_text(&mut var___work_2, &var_base, 0_i64, 2, 32);
    (*var___work_2) += &*("/");
    ops::format_text(&mut var___work_2, &*var_t, 0_i64, 2, 32);
    ;
    ;
    ;
    ;
    return Str::new(&*var___work_2)
    } /*Formatted string_15: text["__work_2"]*/
  } /*block_1: text["base", "t"]*/

// loft:default/03_text.loft:44
fn t_4text_char_slice(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str, mut var_from: i64, mut var_to: i64, mut var_cs_out: &mut String) -> Str { //block_1: text["cs_n"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  let mut var___work_1: String = "".to_string();
  // loft:default/03_text.loft:45
  let mut var_cs_n: i64 = t_4text_len(cell, var_self);
  // loft:default/03_text.loft:46
  let mut var_cs_lo: i64 = var_from;
  // loft:default/03_text.loft:47
  if ((((var_cs_lo) as i64) < ((0_i64) as i64)) as u8) == 1 { //block_2: void
    var_cs_lo = ops::op_add_int((var_cs_lo), (var_cs_n));
    } /*block_2: void*/ else {()};
  // loft:default/03_text.loft:48
  let mut var_cs_hi: i64 = var_to;
  // loft:default/03_text.loft:49
  if ((((var_cs_hi) as i64) < ((0_i64) as i64)) as u8) == 1 { //block_3: void
    var_cs_hi = ops::op_add_int((var_cs_hi), (var_cs_n));
    } /*block_3: void*/ else {()};
  // loft:default/03_text.loft:50
  var_cs_lo = t_7integer_clamp(cell, var_cs_lo, 0_i64, var_cs_n);
  // loft:default/03_text.loft:51
  var_cs_hi = t_7integer_clamp(cell, var_cs_hi, 0_i64, var_cs_n);
  // loft:default/03_text.loft:52
  if ((((var_cs_hi) as i64) <= ((var_cs_lo) as i64)) as u8) == 1 { //block_4: never
    ;
    return Str::new("")
    } /*block_4: never*/ else {()};
  // loft:default/03_text.loft:53
  *var_cs_out = ("").to_string();
  // loft:default/03_text.loft:54
  let mut var_cs_k: i64 = 0_i64;
  // loft:default/03_text.loft:55
  { //For block_5: void
    let mut var_cs_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_cs_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l6: loop { //For loop_6
      let mut var_cs_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_cs_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_cs_c__index = var_cs_c__next; var_cs_c__next = var_cs_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_7: character
        var_cs_c__index = var_cs_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_cs_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_cs_c__next = ops::op_add_int((var_cs_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_cs_c__next) as i64) <= ((var_cs_c__index) as i64)) as u8) == 1 {var_cs_c__next = ops::op_add_int((var_cs_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_7: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_cs_c__index) as i64)) as u8) == 1 { //break_9: void
        break;
        } /*break_9: void*/ else {()};
      { //block_10: void
        // loft:default/03_text.loft:56
        if ((((var_cs_hi) as i64) <= ((var_cs_k) as i64)) as u8) == 1 { //block_11: never
          break
          } /*block_11: never*/ else {()};
        // loft:default/03_text.loft:57
        if ((((var_cs_lo) as i64) <= ((var_cs_k) as i64)) as u8) == 1 { //block_12: void
          {let c = var_cs_c; if c != 0 { var_cs_out.push(ops::to_char(c)); } };
          } /*block_12: void*/ else {()};
        // loft:default/03_text.loft:58
        var_cs_k = ((var_cs_k).wrapping_add(1_i64));
        } /*block_10: void*/;
      } /*For loop_6*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_5: void*/;
  // loft:default/03_text.loft:60
  ;
  return Str::new(&*var_cs_out)
  } /*block_1: text["cs_n"]*/

// loft:default/03_text.loft:110
fn t_4text_is_lowercase(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:111
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:112
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:113
        if (((((ops::to_char(var_c)).is_lowercase()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:115
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:119
#[inline]
fn t_9character_is_lowercase(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_lowercase")
}


// loft:default/03_text.loft:124
fn t_4text_is_uppercase(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:125
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:126
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:127
        if (((((ops::to_char(var_c)).is_uppercase()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:129
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:133
#[inline]
fn t_9character_is_uppercase(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_uppercase")
}


// loft:default/03_text.loft:138
fn t_4text_is_numeric(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:139
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:140
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:141
        if (((((ops::to_char(var_c)).is_numeric()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:143
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:147
#[inline]
fn t_9character_is_numeric(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_numeric")
}


// loft:default/03_text.loft:152
fn t_4text_is_alphanumeric(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:153
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:154
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:155
        if (((((ops::to_char(var_c)).is_alphanumeric()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:157
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:161
#[inline]
fn t_9character_is_alphanumeric(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_alphanumeric")
}


// loft:default/03_text.loft:166
fn t_4text_is_alphabetic(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:167
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:168
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:169
        if (((((ops::to_char(var_c)).is_alphabetic()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:171
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:175
#[inline]
fn t_9character_is_alphabetic(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_alphabetic")
}


// loft:default/03_text.loft:180
fn t_4text_is_whitespace(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:181
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:182
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:183
        if (((((ops::to_char(var_c)).is_whitespace()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:185
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:189
#[inline]
fn t_9character_is_whitespace(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_whitespace")
}


// loft:default/03_text.loft:194
fn t_4text_is_control(cell: &std::cell::UnsafeCell<Stores>, mut var_self: &str) -> u8 { //block_1: boolean
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:195
  if ((((t_4text_len(cell, var_self)) as i64) == ((0_i64) as i64)) as u8) == 1 { //block_2: never
    return (false) as u8
    } /*block_2: never*/ else {()};
  // loft:default/03_text.loft:196
  { //For block_3: void
    let mut var_c__index: i64 = 0_i64;
    let mut var__for_text_1: &str = var_self;
    let mut var_c__next: i64 = 0_i64;
          if (((((var__for_text_1) != loft::state::STRING_NULL) as u8) != 1) as u8) != 1 { //@FR-R-CharWalk the null test, asked once
'l4: loop { //For loop_4
      let mut var_c: i32 = { let __tb = (var__for_text_1).as_bytes(); let __ti = var_c__next as usize; if __ti < __tb.len() && __tb[__ti].wrapping_sub(1) < 0x7F { var_c__index = var_c__next; var_c__next = var_c__next + 1; i32::from(__tb[__ti]) } else { //for text next_5: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = ({ let _v_v2 = (var_c__next); { let _v_v1 = (var__for_text_1); { let ch = ops::text_character(_v_v1, _v_v2); ops::note_format_fault(3, ch == char::from(0) && _v_v2 != i64::MIN && !_v_v1.is_empty()); ch } } }) as u32 as i32;
        var_c__next = ops::op_add_int((var_c__next), (OpLengthCharacter(cell, var__for_result_1)));
        if ((((var_c__next) as i64) <= ((var_c__index) as i64)) as u8) == 1 {var_c__next = ops::op_add_int((var_c__index), (1_i64))} else {()};
        var__for_result_1
        } /*for text next_5: character*/ } /*@FR-R-CharWalk*/;
      if (((({{ let __t = (var__for_text_1); if __t == loft::state::STRING_NULL { 0 } else { __t.len() as i64 } }}) as i64) <= ((var_c__index) as i64)) as u8) == 1 { //break_7: void
        break;
        } /*break_7: void*/ else {()};
      { //block_8: void
        // loft:default/03_text.loft:197
        if (((((ops::to_char(var_c)).is_control()) as u8) != 1) as u8) == 1 { //block_9: never
          let mut var___ret_1: u8 = (false) as u8;
          ;
          return (var___ret_1) as u8
          } /*block_9: never*/ else {()};
        } /*block_8: void*/;
      } /*For loop_4*/
      } /*@FR-R-CharWalk*/;
    ;
    } /*For block_3: void*/;
  // loft:default/03_text.loft:199
  return (true) as u8
  } /*block_1: boolean*/

// loft:default/03_text.loft:203
#[inline]
fn t_9character_is_control(cell: &std::cell::UnsafeCell<Stores>, mut var_self: i32) -> u8 {
  todo!("native function t_9character_is_control")
}


// loft:default/03_text.loft:210
fn t_6vector_join(cell: &std::cell::UnsafeCell<Stores>, mut var_self: DbRef, mut var_sep: &str, mut var_result: &mut String) -> Str { //block_1: text["result"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:211
  *var_result = ("").to_string();
  // loft:default/03_text.loft:212
  { //For block_2: void
    let mut var_p__count: i64 = 0_i64;
    let mut var__vector_1: DbRef = var_self;
    let __vh_1 = vector::vec_header(&(var__vector_1), &stores.allocations); //@PLN157 § V-n view header for var__vector_1
    let mut var_p__index: i64 = -1_i64;
    { //loft#885 loop-invariant vector headers
      let __vb_2: *const u8 = vector::vec_base(&__vh_1, &stores.allocations); //@PLN157 § V-ak element base of the held header
      let __ts_2: (*const u8, u32) = vector::text_span_of(&__vh_1, &stores.allocations);
      'l3: loop { //For loop_3
      let var_p: &str = { //iter next_4: text
        var_p__index = ops::op_add_int((var_p__index), (1_i64));
        unsafe { vector::text_elem_at::<false>(&__vh_1, __vb_2, __ts_2, &(var__vector_1), (var_p__index) as i64, &stores.allocations) }
        } /*iter next_4: text*/;
      if (((((i64::from(__vh_1.len))) as i64) <= ((var_p__index) as i64)) as u8) == 1 { //break_5: void
        ;
        break;
        } /*break_5: void*/ else {()};
      if ((((var_p__index) as i64) < ((0_i64) as i64)) as u8) == 1 { //break_6: void
        ;
        break;
        } /*break_6: void*/ else {()};
      { //block_7: void
        // loft:default/03_text.loft:213
        if ((((((var_p__count) as i64) == ((0_i64) as i64)) as u8) != 1) as u8) == 1 { //block_8: void
          (*var_result) += &*(var_sep);
          } /*block_8: void*/ else {()};
        // loft:default/03_text.loft:214
        (*var_result) += &*(var_p);
        } /*block_7: void*/;
      var_p__count = ops::op_add_int((var_p__count), (1_i64));
      ;
      } /*For loop_3*/ };
    } /*For block_2: void*/;
  // loft:default/03_text.loft:216
  return Str::new(&*var_result)
  } /*block_1: text["result"]*/

// loft:default/03_text.loft:210
fn t_6vector_join__inv(cell: &std::cell::UnsafeCell<Stores>, mut var_self: DbRef, mut var_sep: &str, mut var_result: &mut String, __ih_0: vector::VecHeader, __ib_0: *const u8) -> Str { //block_1: text["result"]
  let stores: &mut Stores = unsafe { &mut *cell.get() };
  // loft:default/03_text.loft:211
  *var_result = ("").to_string();
  // loft:default/03_text.loft:212
  { //For block_2: void
    let mut var_p__count: i64 = 0_i64;
    let mut var__vector_1: DbRef = var_self;
    let __vh_1 = __ih_0; //@PLN157 § V-n view header for var__vector_1, copied from the held path (§ V-p)
    let __vb_1 = __ib_0; //@FR-R-Base view base for var__vector_1, shared from the held path
    let __ts_1: (*const u8, u32) = vector::text_span_of(&__vh_1, &stores.allocations);
    let mut var_p__index: i64 = -1_i64;
    'l3: loop { //For loop_3
      let var_p: &str = { //iter next_4: text
        var_p__index = ops::op_add_int((var_p__index), (1_i64));
        unsafe { vector::text_elem_at::<false>(&__vh_1, __vb_1, __ts_1, &(var__vector_1), (var_p__index) as i64, &stores.allocations) }
        } /*iter next_4: text*/;
      if (((((i64::from(__vh_1.len))) as i64) <= ((var_p__index) as i64)) as u8) == 1 { //break_5: void
        ;
        break;
        } /*break_5: void*/ else {()};
      if ((((var_p__index) as i64) < ((0_i64) as i64)) as u8) == 1 { //break_6: void
        ;
        break;
        } /*break_6: void*/ else {()};
      { //block_7: void
        // loft:default/03_text.loft:213
        if ((((((var_p__count) as i64) == ((0_i64) as i64)) as u8) != 1) as u8) == 1 { //block_8: void
          (*var_result) += &*(var_sep);
          } /*block_8: void*/ else {()};
        // loft:default/03_text.loft:214
        (*var_result) += &*(var_p);
        } /*block_7: void*/;
      var_p__count = ops::op_add_int((var_p__count), (1_i64));
      ;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  // loft:default/03_text.loft:216
  return Str::new(&*var_result)
  } /*block_1: text["result"]*/


pub extern "C" fn loft_shared_t_4File_lines(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (2, a);
    let p0: DbRef = a[0].dbref;
    let mut p1: DbRef = if 1 < n { a[1].dbref } else { DbRef { store_nr: 0, rec: 0, pos: 0 } };
    let mut p1_fresh = false;
    if p1.rec == 0 && p1.pos == 0 {
        let _tid1 = unsafe { (&*cell.get()) }.name("main_vector<text>");
        assert!(_tid1 != u16::MAX, "shared bridge: type main_vector<text> not registered in the caller store");
        p1 = unsafe { (&mut *cell.get()).null_named("__shared_dest") };
        p1 = OpDatabase(cell, p1, i32::from(_tid1));
        p1_fresh = true;
    }
    unsafe { (*ret).dbref = (t_4File_lines(cell, p0, p1)); }
    if p1_fresh && !loft::keys::bridge_orphan_free_disabled() {
    let __r = unsafe { (*ret).dbref };
    if !(__r.store_nr == p1.store_nr && __r.rec == p1.rec && __r.pos == p1.pos) {
    unsafe { (&mut *cell.get()).free_named(&p1, "__shared_dest_orphan"); }
    }
    }
}

pub extern "C" fn loft_shared_t_4text_split(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (3, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    let p1: i32 = a[1].scalar as i32;
    let mut p2: DbRef = if 2 < n { a[2].dbref } else { DbRef { store_nr: 0, rec: 0, pos: 0 } };
    let mut p2_fresh = false;
    if p2.rec == 0 && p2.pos == 0 {
        let _tid2 = unsafe { (&*cell.get()) }.name("main_vector<text>");
        assert!(_tid2 != u16::MAX, "shared bridge: type main_vector<text> not registered in the caller store");
        p2 = unsafe { (&mut *cell.get()).null_named("__shared_dest") };
        p2 = OpDatabase(cell, p2, i32::from(_tid2));
        p2_fresh = true;
    }
    unsafe { (*ret).dbref = (t_4text_split(cell, p0, p1, p2)); }
    if p2_fresh && !loft::keys::bridge_orphan_free_disabled() {
    let __r = unsafe { (*ret).dbref };
    if !(__r.store_nr == p2.store_nr && __r.rec == p2.rec && __r.pos == p2.pos) {
    unsafe { (&mut *cell.get()).free_named(&p2, "__shared_dest_orphan"); }
    }
    }
}

pub extern "C" fn loft_shared_t_4text_split_text(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (3, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    let p1: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[1].text_ptr, a[1].text_len)) };
    let mut p2: DbRef = if 2 < n { a[2].dbref } else { DbRef { store_nr: 0, rec: 0, pos: 0 } };
    let mut p2_fresh = false;
    if p2.rec == 0 && p2.pos == 0 {
        let _tid2 = unsafe { (&*cell.get()) }.name("main_vector<text>");
        assert!(_tid2 != u16::MAX, "shared bridge: type main_vector<text> not registered in the caller store");
        p2 = unsafe { (&mut *cell.get()).null_named("__shared_dest") };
        p2 = OpDatabase(cell, p2, i32::from(_tid2));
        p2_fresh = true;
    }
    unsafe { (*ret).dbref = (t_4text_split_text(cell, p0, p1, p2)); }
    if p2_fresh && !loft::keys::bridge_orphan_free_disabled() {
    let __r = unsafe { (*ret).dbref };
    if !(__r.store_nr == p2.store_nr && __r.rec == p2.rec && __r.pos == p2.pos) {
    unsafe { (&mut *cell.get()).free_named(&p2, "__shared_dest_orphan"); }
    }
    }
}

pub extern "C" fn loft_shared_t_4text_resolve(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (2, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    let p1: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[1].text_ptr, a[1].text_len)) };
    let mut p2: String = String::new();
    let mut p3: String = String::new();
    let mut p4: String = String::new();
    let __r = (t_4text_resolve(cell, p0, p1, &mut p2, &mut p3, &mut p4));
    let __t = __r.str();
    let __st: &mut Stores = unsafe { &mut *cell.get() };
    if let Some(__d) = __st.bridge_text_dest.take() {
    if !__t.is_empty() {
    __st.store_mut(&__d).addr_mut::<String>(__d.rec, __d.pos).push_str(__t);
    }
    }
    unsafe { (*ret).text_ptr = std::ptr::null(); (*ret).text_len = 0; }
}

pub extern "C" fn loft_shared_t_4text_char_slice(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (3, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    let p1: i64 = a[1].scalar;
    let p2: i64 = a[2].scalar;
    let mut p3: String = String::new();
    let __r = (t_4text_char_slice(cell, p0, p1, p2, &mut p3));
    let __t = __r.str();
    let __st: &mut Stores = unsafe { &mut *cell.get() };
    if let Some(__d) = __st.bridge_text_dest.take() {
    if !__t.is_empty() {
    __st.store_mut(&__d).addr_mut::<String>(__d.rec, __d.pos).push_str(__t);
    }
    }
    unsafe { (*ret).text_ptr = std::ptr::null(); (*ret).text_len = 0; }
}

pub extern "C" fn loft_shared_t_4text_is_lowercase(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_lowercase(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_uppercase(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_uppercase(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_numeric(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_numeric(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_alphanumeric(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_alphanumeric(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_alphabetic(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_alphabetic(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_whitespace(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_whitespace(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_4text_is_control(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (1, a);
    let p0: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[0].text_ptr, a[0].text_len)) };
    unsafe { (*ret).scalar = (t_4text_is_control(cell, p0)) as i64; }
}

pub extern "C" fn loft_shared_t_6vector_join(
    stores: *mut Stores,
    args: *const loft::native_lib::LibArg,
    n: usize,
    ret: *mut loft::native_lib::LibArg,
) {
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    let _ = (2, a);
    let p0: DbRef = a[0].dbref;
    let p1: &str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(a[1].text_ptr, a[1].text_len)) };
    let mut p2: String = String::new();
    let __r = (t_6vector_join(cell, p0, p1, &mut p2));
    let __t = __r.str();
    let __st: &mut Stores = unsafe { &mut *cell.get() };
    if let Some(__d) = __st.bridge_text_dest.take() {
    if !__t.is_empty() {
    __st.store_mut(&__d).addr_mut::<String>(__d.rec, __d.pos).push_str(__t);
    }
    }
    unsafe { (*ret).text_ptr = std::ptr::null(); (*ret).text_len = 0; }
}

pub extern "C" fn loft_type_layout_fp_v1() -> u64 { 2598374647381934984u64 }

/// `(function, bridge symbol, bridge)` for every compiled function, sorted by name.
pub(crate) static BRIDGES: &[(&str, &str, super::Bridge)] = &[
    ("t_4File_lines", "loft_shared_t_4File_lines", loft_shared_t_4File_lines),
    ("t_4text_char_slice", "loft_shared_t_4text_char_slice", loft_shared_t_4text_char_slice),
    ("t_4text_is_alphabetic", "loft_shared_t_4text_is_alphabetic", loft_shared_t_4text_is_alphabetic),
    ("t_4text_is_alphanumeric", "loft_shared_t_4text_is_alphanumeric", loft_shared_t_4text_is_alphanumeric),
    ("t_4text_is_control", "loft_shared_t_4text_is_control", loft_shared_t_4text_is_control),
    ("t_4text_is_lowercase", "loft_shared_t_4text_is_lowercase", loft_shared_t_4text_is_lowercase),
    ("t_4text_is_numeric", "loft_shared_t_4text_is_numeric", loft_shared_t_4text_is_numeric),
    ("t_4text_is_uppercase", "loft_shared_t_4text_is_uppercase", loft_shared_t_4text_is_uppercase),
    ("t_4text_is_whitespace", "loft_shared_t_4text_is_whitespace", loft_shared_t_4text_is_whitespace),
    ("t_4text_resolve", "loft_shared_t_4text_resolve", loft_shared_t_4text_resolve),
    ("t_4text_split", "loft_shared_t_4text_split", loft_shared_t_4text_split),
    ("t_4text_split_text", "loft_shared_t_4text_split_text", loft_shared_t_4text_split_text),
    ("t_6vector_join", "loft_shared_t_6vector_join", loft_shared_t_6vector_join),
];

/// The standard library's type table this was generated against: its length and fingerprint.
pub(crate) const PREFIX_TYPES: usize = 96;
pub(crate) const PREFIX_FINGERPRINT: u64 = 13106178954628645925;
/// The hash of the `default/*.loft` source these bodies were compiled from.
pub(crate) const SOURCE_HASH: u64 = 8494925563378724926;
