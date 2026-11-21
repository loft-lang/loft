// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN47 — binary file I/O type matrix (write ↔ read round-trip).
//!
//! Each cell writes a known value of one loft type to a temp file in one
//! binary format, reopens the file, reads it back, and asserts the value
//! survived.  Every cell runs under BOTH the interpreter and `--native`;
//! the [`cross_mode`] harness asserts byte-identical stdout, so a width /
//! endianness / backend divergence — the silent-corruption class this
//! plan exists to catch — is a recorded failure even when each backend
//! succeeds in isolation.
//!
//! Axes (see `doc/claude/plans/47-binary-io-validation/00-matrix.md`):
//! value type W0–W11 × format F1 LittleEndian / F2 BigEndian / F3 TextFile
//! × access pattern A1 append / A2 offset / A3 truncate / A4 sync.
//!
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
//!
//! ```bash
//! cargo test --release --test binary_io_matrix -- --ignored
//! ```
//!
//! Cells use program-relative paths (no `#cwd`) so their scratch `.bin`
//! files resolve beside the generated snippet in the temp dir, not in the
//! repo, and `delete()` them at both ends.  The struct cells (W9/W11) do NOT use
//! the leak-free harness: `q = f#read as Struct` inherits loft's
//! pre-existing block-temp ownership leak (`x = { …; struct_temp }` leaks
//! one record — reproducible with no file I/O at all; see the plan's
//! § "known limitation"), which is an ownership-model bug tracked
//! separately, not a binary-I/O defect.  Their values still round-trip
//! byte-identically across backends, which is what these cells lock in.

mod common;

use common::cross_mode::{run_cross_mode_leak_free, run_cross_mode_rejected};

// ── W0 — integer (i64, 8 bytes) ─────────────────────────────────────────────

cross_mode!(
    w0_integer_le,
    r#"
    fn test() {
        delete("biom_w0_le.bin");
       { f = file("biom_w0_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (0x0102030405060708 as integer); f.sync(); }
       { f = file("biom_w0_le.bin"); f#format = LittleEndian;
         v = f#read as integer;
         assert(v == 0x0102030405060708, "w0 le value");
         assert(f#size == 8, "w0 wrote 8 bytes");
         print("w0_integer_le {v}\n"); }
        delete("biom_w0_le.bin");
    }
    "#
);

cross_mode!(
    w0_integer_be,
    r#"
    fn test() {
        delete("biom_w0_be.bin");
       { f = file("biom_w0_be.bin"); f#format = BigEndian; f.set_file_size(0);
         f += (0x0102030405060708 as integer); f.sync(); }
       { f = file("biom_w0_be.bin"); f#format = BigEndian;
         v = f#read as integer;
         assert(v == 0x0102030405060708, "w0 be value");
         print("w0_integer_be {v}\n"); }
        delete("biom_w0_be.bin");
    }
    "#
);

// ── W1 — i32 / u32 (4 bytes) ────────────────────────────────────────────────

cross_mode!(
    w1_i32_le,
    r#"
    fn test() {
        delete("biom_w1i_le.bin");
       { f = file("biom_w1i_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (0x11223344 as i32); f.sync(); }
       { f = file("biom_w1i_le.bin"); f#format = LittleEndian;
         v: i32 = f#read as i32;
         assert(v == 0x11223344, "w1 i32 le");
         assert(f#size == 4, "w1 wrote 4 bytes");
         print("w1_i32_le {v}\n"); }
        delete("biom_w1i_le.bin");
    }
    "#
);

cross_mode!(
    w1_u32_le,
    r#"
    fn test() {
        delete("biom_w1u_le.bin");
       { f = file("biom_w1u_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (0x1ECEC0DE as u32); f.sync(); }
       { f = file("biom_w1u_le.bin"); f#format = LittleEndian;
         v: u32 = f#read as u32;
         assert(v == (0x1ECEC0DE as u32), "w1 u32 le");
         print("w1_u32_le {v}\n"); }
        delete("biom_w1u_le.bin");
    }
    "#
);

cross_mode!(
    w1_i32_be,
    r#"
    fn test() {
        delete("biom_w1i_be.bin");
       { f = file("biom_w1i_be.bin"); f#format = BigEndian; f.set_file_size(0);
         f += (0x11223344 as i32); f.sync(); }
       { f = file("biom_w1i_be.bin"); f#format = BigEndian;
         v: i32 = f#read as i32;
         assert(v == 0x11223344, "w1 i32 be");
         print("w1_i32_be {v}\n"); }
        delete("biom_w1i_be.bin");
    }
    "#
);

// ── W2 — i16 / u16 (2 bytes) ────────────────────────────────────────────────

cross_mode!(
    w2_i16_le,
    r#"
    fn test() {
        delete("biom_w2_le.bin");
       { f = file("biom_w2_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (-12345 as i16); f.sync(); }
       { f = file("biom_w2_le.bin"); f#format = LittleEndian;
         v: i16 = f#read as i16;
         assert(v == -12345, "w2 i16 le");
         assert(f#size == 2, "w2 wrote 2 bytes");
         print("w2_i16_le {v}\n"); }
        delete("biom_w2_le.bin");
    }
    "#
);

cross_mode!(
    w2_u16_be,
    r#"
    fn test() {
        delete("biom_w2_be.bin");
       { f = file("biom_w2_be.bin"); f#format = BigEndian; f.set_file_size(0);
         f += (0xBEEF as u16); f.sync(); }
       { f = file("biom_w2_be.bin"); f#format = BigEndian;
         v: u16 = f#read as u16;
         assert(v == (0xBEEF as u16), "w2 u16 be");
         print("w2_u16_be {v}\n"); }
        delete("biom_w2_be.bin");
    }
    "#
);

// ── W3 — u8 / boolean (1 byte) ──────────────────────────────────────────────

cross_mode!(
    w3_u8_le,
    r#"
    fn test() {
        delete("biom_w3u_le.bin");
       { f = file("biom_w3u_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (255 as u8); f.sync(); }
       { f = file("biom_w3u_le.bin"); f#format = LittleEndian;
         v: u8 = f#read as u8;
         assert(v == 255, "w3 u8 le");
         assert(f#size == 1, "w3 wrote 1 byte");
         print("w3_u8_le {v}\n"); }
        delete("biom_w3u_le.bin");
    }
    "#
);

cross_mode!(
    w3_boolean_le,
    r#"
    fn test() {
        delete("biom_w3b_le.bin");
       { f = file("biom_w3b_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         bt = true; f += bt; bf = false; f += bf; f.sync(); }
       { f = file("biom_w3b_le.bin"); f#format = LittleEndian;
         r1 = f#read as boolean; r2 = f#read as boolean;
         assert(r1 == true, "w3 bool true"); assert(r2 == false, "w3 bool false");
         assert(f#size == 2, "w3 two booleans = 2 bytes");
         print("w3_boolean_le {r1},{r2}\n"); }
        delete("biom_w3b_le.bin");
    }
    "#
);

cross_mode!(
    w3_boolean_be,
    r#"
    fn test() {
        delete("biom_w3b_be.bin");
       { f = file("biom_w3b_be.bin"); f#format = BigEndian; f.set_file_size(0);
         bt = true; f += bt; f.sync(); }
       { f = file("biom_w3b_be.bin"); f#format = BigEndian;
         r = f#read as boolean;
         assert(r == true, "w3 bool be");
         print("w3_boolean_be {r}\n"); }
        delete("biom_w3b_be.bin");
    }
    "#
);

// ── W4 — character (4-byte codepoint) ───────────────────────────────────────

cross_mode!(
    w4_character_le,
    r#"
    fn test() {
        delete("biom_w4_le.bin");
       { f = file("biom_w4_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         ch = 'A'; f += ch; f.sync(); }
       { f = file("biom_w4_le.bin"); f#format = LittleEndian;
         r = f#read as character;
         assert(r == 'A', "w4 char le");
         assert(f#size == 4, "w4 wrote 4 bytes");
         print("w4_character_le {r}\n"); }
        delete("biom_w4_le.bin");
    }
    "#
);

cross_mode!(
    w4_character_be,
    r#"
    fn test() {
        delete("biom_w4_be.bin");
       { f = file("biom_w4_be.bin"); f#format = BigEndian; f.set_file_size(0);
         ch = 'Z'; f += ch; f.sync(); }
       { f = file("biom_w4_be.bin"); f#format = BigEndian;
         r = f#read as character;
         assert(r == 'Z', "w4 char be");
         print("w4_character_be {r}\n"); }
        delete("biom_w4_be.bin");
    }
    "#
);

// ── W5 — float (f64, 8 bytes) ───────────────────────────────────────────────

cross_mode!(
    w5_float_le,
    r#"
    fn test() {
        delete("biom_w5_le.bin");
       { f = file("biom_w5_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += 3.140625; f.sync(); }
       { f = file("biom_w5_le.bin"); f#format = LittleEndian;
         v = f#read as float;
         assert(v == 3.140625, "w5 float le");
         assert(f#size == 8, "w5 wrote 8 bytes");
         print("w5_float_le {v}\n"); }
        delete("biom_w5_le.bin");
    }
    "#
);

cross_mode!(
    w5_float_be,
    r#"
    fn test() {
        delete("biom_w5_be.bin");
       { f = file("biom_w5_be.bin"); f#format = BigEndian; f.set_file_size(0);
         f += 3.140625; f.sync(); }
       { f = file("biom_w5_be.bin"); f#format = BigEndian;
         v = f#read as float;
         assert(v == 3.140625, "w5 float be");
         print("w5_float_be {v}\n"); }
        delete("biom_w5_be.bin");
    }
    "#
);

// ── W6 — single (f32, 4 bytes) ──────────────────────────────────────────────

cross_mode!(
    w6_single_le,
    r#"
    fn test() {
        delete("biom_w6_le.bin");
       { f = file("biom_w6_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         s = 1.25 as single; f += s; f.sync(); }
       { f = file("biom_w6_le.bin"); f#format = LittleEndian;
         v: single = f#read as single;
         assert(v == (1.25 as single), "w6 single le");
         assert(f#size == 4, "w6 wrote 4 bytes");
         print("w6_single_le {v}\n"); }
        delete("biom_w6_le.bin");
    }
    "#
);

cross_mode!(
    w6_single_be,
    r#"
    fn test() {
        delete("biom_w6_be.bin");
       { f = file("biom_w6_be.bin"); f#format = BigEndian; f.set_file_size(0);
         s = 1.25 as single; f += s; f.sync(); }
       { f = file("biom_w6_be.bin"); f#format = BigEndian;
         v: single = f#read as single;
         assert(v == (1.25 as single), "w6 single be");
         print("w6_single_be {v}\n"); }
        delete("biom_w6_be.bin");
    }
    "#
);

// ── Access patterns (A2 offset / A3 truncate / A4 sync) on integer ──────────

cross_mode!(
    a2_offset_overwrite,
    r#"
    fn test() {
        delete("biom_a2.bin");
       { f = file("biom_a2.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (11 as i32); f += (22 as i32); f.sync(); }
       { f = file("biom_a2.bin"); f#format = LittleEndian;
         f#next = 0; f += (99 as i32); f.sync(); }
       { f = file("biom_a2.bin"); f#format = LittleEndian;
         a = f#read as i32; b = f#read as i32;
         assert(a == 99, "a2 overwritten slot 0"); assert(b == 22, "a2 slot 1 intact");
         print("a2_offset_overwrite {a},{b}\n"); }
        delete("biom_a2.bin");
    }
    "#
);

cross_mode!(
    a3_truncate_replace,
    r#"
    fn test() {
        delete("biom_a3.bin");
       { f = file("biom_a3.bin"); f#format = LittleEndian; f.set_file_size(0);
         f += (1 as i32); f += (2 as i32); f += (3 as i32); f.sync(); }
       { f = file("biom_a3.bin"); f#format = LittleEndian;
         f.set_file_size(0); f += (7 as i32); f.sync(); }
       { f = file("biom_a3.bin"); f#format = LittleEndian;
         a = f#read as i32;
         assert(a == 7, "a3 truncated then replaced"); assert(f#size == 4, "a3 size after truncate");
         print("a3_truncate_replace {a}\n"); }
        delete("biom_a3.bin");
    }
    "#
);

// ── W7 — text (F1 LittleEndian + F3 TextFile), explicit-count read ──────────

cross_mode!(
    w7_text_le,
    r#"
    fn test() {
        delete("biom_w7_le.bin");
       { f = file("biom_w7_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         t = "hello"; f += t; f.sync(); }
       { f = file("biom_w7_le.bin"); f#format = LittleEndian;
         s = f#read(5) as text;
         assert(s == "hello", "w7 text le");
         print("w7_text_le {s}\n"); }
        delete("biom_w7_le.bin");
    }
    "#
);

cross_mode!(
    w7_text_textfile,
    r#"
    fn test() {
        delete("biom_w7_tf.bin");
       { f = file("biom_w7_tf.bin"); f#format = TextFile; f.set_file_size(0);
         f += "line"; f.sync(); }
       { f = file("biom_w7_tf.bin"); f#format = TextFile;
         s = f#read(4) as text;
         assert(s == "line", "w7 text textfile");
         print("w7_text_textfile {s}\n"); }
        delete("biom_w7_tf.bin");
    }
    "#
);

// A text read with `f#read(n) as text` is UTF-8 or nothing: bytes that are not valid UTF-8
// leave the target empty, as `text_from_bytes` answers for them — the interpreter built a
// text out of them unchecked.  The read after it, at the next position, shows the empty
// answer is about those bytes alone.
cross_mode!(
    w7_text_not_utf8_is_empty,
    r#"
    fn test() {
        delete("biom_w7_bad.bin");
        write_bytes("biom_w7_bad.bin", [97 as u8, 255 as u8, 98 as u8, 99 as u8, 100 as u8]);
       { f = file("biom_w7_bad.bin"); f#format = LittleEndian;
         s = f#read(3) as text;
         assert(size(s) == 0, "a\\xFFb is not a text: {s}");
         t = f#read(2) as text;
         assert(t == "cd", "the next read is unaffected: {t}");
         print("w7_text_not_utf8 empty\n"); }
        delete("biom_w7_bad.bin");
    }
    "#
);

// ── W8 — vector<scalar>, `f#read(N)` counts N BYTES (element_count * width) ──

cross_mode!(
    w8_vector_integer_le,
    r#"
    fn test() {
        delete("biom_w8i_le.bin");
       { f = file("biom_w8i_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         v: vector<integer> = [10, 20, 30]; f += v; f.sync(); }
       { f = file("biom_w8i_le.bin"); f#format = LittleEndian;
         w: vector<integer> = f#read(24) as vector<integer>;
         assert(len(w) == 3, "w8 int len"); assert(w[0] == 10, "w8 int 0");
         assert(w[2] == 30, "w8 int 2");
         print("w8_vector_integer_le {len(w)} {w[0]} {w[2]}\n"); }
        delete("biom_w8i_le.bin");
    }
    "#
);

cross_mode!(
    w8_vector_integer_be,
    r#"
    fn test() {
        delete("biom_w8i_be.bin");
       { f = file("biom_w8i_be.bin"); f#format = BigEndian; f.set_file_size(0);
         v: vector<integer> = [100, 200]; f += v; f.sync(); }
       { f = file("biom_w8i_be.bin"); f#format = BigEndian;
         w: vector<integer> = f#read(16) as vector<integer>;
         assert(len(w) == 2, "w8 be len"); assert(w[1] == 200, "w8 be 1");
         print("w8_vector_integer_be {len(w)} {w[1]}\n"); }
        delete("biom_w8i_be.bin");
    }
    "#
);

cross_mode!(
    w8_vector_single_be,
    r#"
    fn test() {
        delete("biom_w8s_be.bin");
       { f = file("biom_w8s_be.bin"); f#format = BigEndian; f.set_file_size(0);
         v: vector<single> = [1.5 as single, 2.5 as single, 3.5 as single, 4.5 as single];
         f += v; f.sync(); }
       { f = file("biom_w8s_be.bin"); f#format = BigEndian;
         w: vector<single> = f#read(16) as vector<single>;
         assert(len(w) == 4, "w8 single len"); assert(w[3] == (4.5 as single), "w8 single 3");
         print("w8_vector_single_be {len(w)} {w[3]}\n"); }
        delete("biom_w8s_be.bin");
    }
    "#
);

cross_mode!(
    w8_vector_u8_le,
    r#"
    fn test() {
        delete("biom_w8b_le.bin");
       { f = file("biom_w8b_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         v: vector<u8> = [1 as u8, 2 as u8, 255 as u8]; f += v; f.sync(); }
       { f = file("biom_w8b_le.bin"); f#format = LittleEndian;
         w: vector<u8> = f#read(3) as vector<u8>;
         assert(len(w) == 3, "w8 u8 len"); assert(w[2] == 255, "w8 u8 2");
         print("w8_vector_u8_le {len(w)} {w[2]}\n"); }
        delete("biom_w8b_le.bin");
    }
    "#
);

// ── W9 — struct of scalars (per-field walk, both endians) ───────────────────

cross_mode!(
    w9_struct_le,
    r#"
    struct Point { x: integer, y: integer, tag: u8 }
    fn test() {
        delete("biom_w9_le.bin");
       { f = file("biom_w9_le.bin"); f#format = LittleEndian; f.set_file_size(0);
         p = Point { x: 42, y: -7, tag: 200 }; f += p; f.sync(); }
       { f = file("biom_w9_le.bin"); f#format = LittleEndian;
         q = f#read as Point;
         assert(q.x == 42, "w9 x"); assert(q.y == -7, "w9 y"); assert(q.tag == 200, "w9 tag");
         print("w9_struct_le {q.x},{q.y},{q.tag}\n"); }
        delete("biom_w9_le.bin");
    }
    "#
);

cross_mode!(
    w9_struct_be,
    r#"
    struct Point { x: integer, y: integer, tag: u8 }
    fn test() {
        delete("biom_w9_be.bin");
       { f = file("biom_w9_be.bin"); f#format = BigEndian; f.set_file_size(0);
         p = Point { x: 42, y: -7, tag: 200 }; f += p; f.sync(); }
       { f = file("biom_w9_be.bin"); f#format = BigEndian;
         q = f#read as Point;
         assert(q.x == 42, "w9 be x"); assert(q.y == -7, "w9 be y"); assert(q.tag == 200, "w9 be tag");
         print("w9_struct_be {q.x},{q.y},{q.tag}\n"); }
        delete("biom_w9_be.bin");
    }
    "#
);

cross_mode!(
    w9_struct_mixed_widths_be,
    r#"
    struct Rec { a: i32, b: i16, c: u8, d: float, e: single, f: integer }
    fn test() {
        delete("biom_w9m.bin");
       { fl = file("biom_w9m.bin"); fl#format = BigEndian; fl.set_file_size(0);
         r = Rec { a: 100000, b: -3000, c: 250, d: 3.5, e: 1.25 as single, f: 9000000000 };
         fl += r; fl.sync(); }
       { fl = file("biom_w9m.bin"); fl#format = BigEndian;
         q = fl#read as Rec;
         assert(q.a == 100000, "w9m a"); assert(q.b == -3000, "w9m b"); assert(q.c == 250, "w9m c");
         assert(q.d == 3.5, "w9m d"); assert(q.e == (1.25 as single), "w9m e");
         assert(q.f == 9000000000, "w9m f");
         print("w9_struct_mixed {q.a},{q.b},{q.c},{q.d},{q.e},{q.f}\n"); }
        delete("biom_w9m.bin");
    }
    "#
);

// ── W11 — nested plain struct (fixed-width; rides the W9 per-field walk) ─────

cross_mode!(
    w11_nested_struct_le,
    r#"
    struct Inner { a: integer, b: integer }
    struct Outer { tag: u8, inner: Inner }
    fn test() {
        delete("biom_w11.bin");
       { f = file("biom_w11.bin"); f#format = LittleEndian; f.set_file_size(0);
         p = Outer { tag: 3, inner: Inner { a: 11, b: 22 } }; f += p; f.sync(); }
       { f = file("biom_w11.bin"); f#format = LittleEndian;
         q = f#read as Outer;
         assert(q.tag == 3, "w11 tag"); assert(q.inner.a == 11, "w11 a"); assert(q.inner.b == 22, "w11 b");
         print("w11_nested {q.tag},{q.inner.a},{q.inner.b}\n"); }
        delete("biom_w11.bin");
    }
    "#
);

// ── W10 — variable-width struct field: rejected on BOTH backends ─────────────

/// A `text` field has no fixed byte width, so the fixed-width `f += s` /
/// `f#read as S` walk cannot round-trip it — reject at compile time on both
/// backends rather than crash at runtime (the pre-fix behaviour).
#[test]
fn w10_struct_text_field_rejected() {
    run_cross_mode_rejected(
        "w10_struct_text_field_rejected",
        r#"
    struct Named { id: integer, name: text }
    fn test() {
        f = file("biom_w10t.bin"); f#format = LittleEndian;
        p = Named { id: 5, name: "hi" }; f += p;
    }
    "#,
        "variable-width field 'name'",
    );
}

/// A `vector` field is likewise variable-width — same rejection.
#[test]
fn w10_struct_vector_field_rejected() {
    run_cross_mode_rejected(
        "w10_struct_vector_field_rejected",
        r#"
    struct Bag { id: integer, items: vector<integer> }
    fn test() {
        f = file("biom_w10v.bin"); f#format = LittleEndian;
        _ = f#read(8) as Bag;
    }
    "#,
        "variable-width field 'items'",
    );
}

// ── Leak-free guarantee for the leak-free types (scalars / text / vector) ────
//
// Struct reads are excluded here — they inherit the pre-existing block-temp
// ownership leak (see the module doc); their round-trip correctness is locked
// by the cross-mode cells above.

#[test]
fn leak_free_scalar_roundtrip() {
    run_cross_mode_leak_free(
        "leak_free_scalar_roundtrip",
        r#"
    fn test() {
        delete("biom_lf_s.bin");
        for i in 0..8 {
          { f = file("biom_lf_s.bin"); f#format = LittleEndian; f.set_file_size(0);
            f += (i as i32); f.sync(); }
          { f = file("biom_lf_s.bin"); f#format = LittleEndian;
            v: i32 = f#read as i32; assert(v == i, "lf scalar"); }
        }
        delete("biom_lf_s.bin");
        print("leak_free_scalar ok\n");
    }
    "#,
    );
}

#[test]
fn leak_free_vector_roundtrip() {
    run_cross_mode_leak_free(
        "leak_free_vector_roundtrip",
        r#"
    fn test() {
        delete("biom_lf_v.bin");
        for i in 0..8 {
          { f = file("biom_lf_v.bin"); f#format = LittleEndian; f.set_file_size(0);
            v: vector<integer> = [i, i + 1, i + 2]; f += v; f.sync(); }
          { f = file("biom_lf_v.bin"); f#format = LittleEndian;
            w: vector<integer> = f#read(24) as vector<integer>;
            assert(len(w) == 3, "lf vec len"); assert(w[0] == i, "lf vec 0"); }
        }
        delete("biom_lf_v.bin");
        print("leak_free_vector ok\n");
    }
    "#,
    );
}

// ── loft#829 — `content()` says null for bytes it cannot read as text ────────
//
// `file(p).content()` used to answer `""` for a non-UTF-8 file: the call succeeded,
// and its answer was indistinguishable from *"the file is empty"*.  That inverts
// gates rather than breaking them — "write bytes, read them back, compare lengths"
// passes vacuously on binary data, because both sides are `""`.  The contract is
// now null for *no text here*, the same answer a MISSING file already gives
// (@PLN102 H4); `""` is reserved for a file that really is empty.
//
// The empty-file cell is what makes this a boundary instead of a blanket: null and
// `""` have to stay apart, or the fix trades one indistinguishable pair for another.
// Every cell is `cross_mode!`, because the read had TWO homes — the stderr warning
// lived in the interpreter's `State::get_file_text` alone, so `--native` read a
// binary file in complete silence.  One shared `read_file_text_into` now serves both.

cross_mode!(
    c829_non_utf8_content_is_null,
    r#"
    fn test() {
        delete("biom_829_bad.bin");
        write_bytes("biom_829_bad.bin", [65 as u8, 255 as u8, 66 as u8]);
        c = file("biom_829_bad.bin").content();
        assert(c == null, "non-UTF-8 content() is null, not \"\"");
        // The file is intact and readable — the null is about text, not about loss.
        assert(file("biom_829_bad.bin").size == 3, "the 3 bytes are still on disk");
        b = read_bytes("biom_829_bad.bin") ?? [];
        assert(len(b) == 3, "read_bytes reads all 3 bytes");
        assert(b[1] == 255, "read_bytes is byte-exact: {b[1]}");
        print("c829 non-utf8 null\n");
        delete("biom_829_bad.bin");
    }
    "#
);

cross_mode!(
    c829_empty_file_content_is_empty_text,
    r#"
    fn test() {
        delete("biom_829_empty.bin");
        write_bytes("biom_829_empty.bin", []);
        c = file("biom_829_empty.bin").content();
        assert(c != null, "an EMPTY file still reads as text, not null");
        assert(size(c ?? "x") == 0, "and that text is empty");
        print("c829 empty is empty text\n");
        delete("biom_829_empty.bin");
    }
    "#
);

cross_mode!(
    c829_valid_utf8_still_reads,
    r#"
    fn test() {
        delete("biom_829_ok.bin");
        // 'h', then the two bytes of 'é' — 3 bytes, 2 characters.  A NUL byte is
        // valid UTF-8 and must not be mistaken for the unreadable case.
        write_bytes("biom_829_ok.bin", [104 as u8, 195 as u8, 169 as u8, 0 as u8]);
        c = file("biom_829_ok.bin").content() ?? "";
        assert(size(c) == 4, "byte length survives: {size(c)}");
        assert(len(c) == 3, "character length survives: {len(c)}");
        print("c829 utf8 reads\n");
        delete("biom_829_ok.bin");
    }
    "#
);

cross_mode!(
    c829_directory_content_is_null,
    r#"
    fn test() {
        mkdir("biom_829_dir");
        c = file("biom_829_dir").content();
        assert(c == null, "a directory holds no text, so content() is null");
        print("c829 dir null\n");
        delete("biom_829_dir");
    }
    "#
);

// ─── loft#970 — a read whose result ends at the store's end ──────────────────
//
// `read_bytes` panicked *"Store access out of bounds … the reference is corrupt"* as soon
// as the vector it allocates ended exactly at the store's last byte.  The reference was
// fine; the WIDTH was one word too many.  `Store::buffer` read the record's size word —
// which counts itself, since `claim_block` sets `claimed_end = pos + size_word` — and then
// spanned that many bytes starting 8 bytes in, so the slice it handed back always ran one
// word past the record.  `slice::from_raw_parts_mut` beyond an allocation is UB whether or
// not anything reads it, so this was latent from the start and merely invisible: the
// overrun only leaves the STORE when the record happens to be the last one in it.
//
// That is why the reporter measured a boundary in store SLACK rather than in file size —
// 1352 bytes fine, 1353 fatal in a five-line program; 328/329 in one that had allocated a
// vector first.  It surfaced when @PLN950's always-on bound replaced a `debug_assert!` that
// loft's own library build compiles out, which is exactly what that change was for.
//
// The sizes below are chosen to LAND on the boundary rather than to be round: a cell at a
// comfortable size passes on the broken binary and would pin nothing.  Both are asserted
// byte-exact, because a fix that returned a short buffer would satisfy a length check that
// only asked for "not a panic".

cross_mode!(
    c970_read_bytes_at_the_store_boundary,
    r#"
    fn test() {
        // 1353 is the first size that faulted in the reporter's five-line program; 328/329
        // straddle the boundary in one that allocates first, which is this shape.
        for n in [328, 329, 1352, 1353, 2000] {
            p = "biom_970_{n}.bin";
            delete(p);
            blob: vector<u8> = [];
            i = 0;
            while i < n { blob += [(i & 255) as u8]; i = i + 1; }
            assert(write_bytes(p, blob), "wrote {n}");
            back = read_bytes(p) ?? [];
            assert(len(back) == n, "read_bytes({n}) gave {len(back)}");
            // Byte-exact at both ends: the last byte is the one a short buffer loses.
            assert(back[0] == blob[0], "first byte of {n}");
            assert(back[n - 1] == blob[n - 1], "last byte of {n}");
            delete(p);
        }
        print("c970 boundary reads ok\n");
    }
    "#
);
