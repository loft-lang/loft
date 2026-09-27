// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests that replicate slot-assignment bugs found in the wrap-test failures.
//! All three tests now pass — the two-zone slot redesign (0.8.3) fixed all B-class bugs.
//!
//! Three bug classes reproduced here (all fixed):
//!
//! * **B-dir** (S17, CAVEATS.md C4): A `text` variable was pre-assigned a slot *below*
//!   the actual TOS at codegen time → `[generate_set]` panic.  Fixed by two-zone design:
//!   large variables are placed after the zone-1 frame is pre-claimed, so their slot
//!   always matches the actual stack position.
//!
//! * **B-binary** (S18, CAVEATS.md C5): A `ref` variable's pre-assigned slot was
//!   overridden downward by codegen, then reused by a subsequent variable, creating a
//!   live-interval overlap → `validate_slots` panic.  Fixed by the same two-zone design.
//!
//! * **B-stress**: A vector variable was pre-assigned 4 bytes below the actual TOS.
//!   Codegen moved it upward, leaving the adjacent index variable inside the vector
//!   slot → `validate_slots` panic.  Fixed by two-zone design.

extern crate loft;

mod testing;

// ── B-dir: Text variable placed below actual TOS in nested scopes ─────────────

/// Replicates the pattern in `t_6Parser_type_def` (lib/parser.loft):
/// a `text` variable `f` is defined inside a `for field` loop that is nested inside
/// an `if`-block inside a `for param` loop.
///
/// Previously caused `[generate_set] Text variable 'f' … pre-assigned slot N < TOS M`.
/// Fixed by the two-zone slot redesign (S17, 0.8.3).
#[test]
fn text_below_tos_nested_loops() {
    code!(
        "fn parse_generic(data: text, define: boolean) -> text {
    id = \"\";
    params = [];
    flds = [];
    if data[0] == '<' {
        id += \"<\";
        for param in 0..16 {
            p = param;
            params +=[p];
            id += \"{p}\";
            if data[1] == '[' {
                id += \"[\";
                for field in 0..16 {
                    desc = data[field] == '-';
                    f = \"\";
                    if desc { f = \"-\"; }
                    f += data[field..field+1];
                    id += f;
                    flds +=[f];
                    if !data[field+1] { field# break; }
                    id += \",\";
                }
                id += \"]\";
            }
            if !data[param+2] { param# break; }
            id += \",\";
        }
        id += \">\";
    }
    if define { id += \"!\"; }
    id
}

fn test() {
    result = parse_generic(\"<integer[-id,name]>\", false);
    assert(result != \"\", \"got: {result}\");
}"
    );
}

// ── B-binary: ref variable codegen override causes subsequent var to conflict ──

/// Replicates the pattern in `tests/scripts/12-binary.loft`:
/// many sequential `{f = file(…); …}` blocks, each creating a short-lived `File`
/// reference.
///
/// Previously caused a `validate_slots` panic — `'_read_N'` and `'f'` sharing a slot.
/// Fixed by the two-zone slot redesign (S18, 0.8.3).
#[test]
fn sequential_file_blocks_read_conflict() {
    code!(
        "fn test() {
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian; f += 0 as u8; f += 255 as u8; }
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian;
   assert(f#read(1) as u8 == 0, \"u8-0\"); assert(f#read(1) as u8 == 255, \"u8-255\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian; f += 0x0203 as u16; }
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian;
   assert(f#read(2) as u16 == 0x0203, \"u16-be\"); }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(2) as u16 == 0x0302, \"u16-le\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian; f += 0x11223344 as i32; }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(4) as i32 == 0x11223344, \"i32-le-rt\"); }
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian;
   assert(f#read(4) as i32 == 0x44332211, \"i32-le-as-be\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian; f += 0x11223344 as i32; }
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian;
   assert(f#read(4) as i32 == 0x11223344, \"i32-be-rt\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   f += 0x0102030405060708 as integer; assert(f#size == 8, \"long-sz\"); }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(8) as integer == 0x0102030405060708, \"long-rt\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian; f += 1.5f; }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(4) as single == 1.5f, \"single-rt\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   f += 3.14; assert(f#size == 8, \"float-sz\"); }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(8) as float == 3.14, \"float-rt\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   f += \"Hello\"; assert(f#size == 5, \"text-sz\"); }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(5) as text == \"Hello\", \"text-rt\"); rest = f#read(100) as text; assert(rest == \"\", \"eof\"); }
  delete(\"slots_test_a.bin\");
  {f = file(\"slots_test_b.bin\"); f#format = LittleEndian; f +=[1, 2]; }
  {f = file(\"slots_test_b.bin\"); f#format = LittleEndian;
   f#read(8) as integer; assert(f#read(8) as integer == 2, \"vec-2nd\"); }
  delete(\"slots_test_b.bin\");
  {f = file(\"slots_test_a.bin\"); f#format = BigEndian;
   f += 0 as u8; f += 1 as u8; f += 0x0203 as u16; f += 0x04050607 as i32;
   f += 0x08090a0b0c0d0e0f as integer; f += \"Hello world!\"; }
  {f = file(\"slots_test_a.bin\"); f#format = LittleEndian;
   assert(f#read(4) as i32 == 0x03020100, \"mixed-4\");
   f#next = 16;
   assert(f#read(5) as text == \"Hello\", \"mixed-seek\");
   rest = f#read(100) as text; assert(rest == \" world!\", \"mixed-tail\"); }
  delete(\"slots_test_a.bin\");
}"
    );
}

// ── B-stress: x#index lands inside _vector_8 slot after codegen shifts vector ──

/// Replicates the pattern in `tests/scripts/16-stress.loft`:
/// after a fill-and-clear cycle loop (which contains a `cnt` variable whose scope
/// exit is underestimated), `sv` is pre-assigned to the slot where `cnt` lived.
/// Because the actual TOS is 4 bytes higher at that point, codegen moves `sv`
/// (and subsequently `_vector_8`, the iteration copy) upward by 4 bytes.
/// The `x#index` variable for `for x in sv` is pre-assigned immediately after the
/// *pre-assigned* `_vector_8`, but it stays there while `_vector_8` was moved into
/// that range → `validate_slots` panic.
///
/// Expected failure: `Variables '_vector_8' … and 'x#index' … share a stack slot`.
#[test]
fn vector_iteration_index_inside_vec_slot() {
    code!(
        "fn test() {
  const N = 100;
  v =[for i in 0..N { i }];
  sum = 0;
  for x in v { sum += x; }
  assert(sum == 4950, \"sum {sum}\");
  for x in v { x#remove; }
  assert(len(v) == 0, \"empty after remove len={len(v)}\");
  for cycle in 0..3 {
    v +=[for i in 0..50 { i }];
    cnt = 0;
    for _ in v { cnt += 1; }
    assert(cnt == 50, \"cycle {cycle} cnt {cnt}\");
    for x in v { x#remove; }
  }
  assert(len(v) == 0, \"empty after 3 cycles len={len(v)}\");
  sv =[42];
  assert(sv[0] == 42, \"sv read\");
  for x in sv { x#remove; }
  assert(len(sv) == 0, \"sv empty len={len(sv)}\");
  sv +=[99];
  assert(sv[0] == 99, \"sv reinserted\");
  for x in sv { x#remove; }
}"
    );
}
