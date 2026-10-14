//! Validation for the `loft::host` Rust→loft call API (P1).
//! The load-bearing check: a host `call(f, args)` equals an in-language call of `f`.

use loft::file_access as fa;
use loft::host::{LoftError, Program, Value};

const SRC: &str = r#"
fn add(a: integer, b: integer) -> integer { a + b }
fn greet(name: text) -> text { "hi {name}" }
fn is_big(n: integer) -> boolean { n > 100 }
fn dbl(x: single) -> single { x + x }
fn boom(n: integer) -> integer { assert(n > 0, "must be positive"); n }
fn taint(v: integer) -> integer { v }
fn thru_u8(v: u8) -> integer { v }
fn thru_i8(v: i8) -> integer { v }
fn thru_i16(v: i16) -> integer { v }
fn thru_i32(v: i32) -> integer { v }
fn thru_u32(v: u32) -> integer { v }
fn ret_i8(v: integer) -> i8 { v as i8 }
fn ret_i16(v: integer) -> i16 { v as i16 }
fn ret_i32(v: integer) -> i32 { v as i32 }
fn ret_u8(v: integer) -> u8 { v as u8 }
"#;

fn prog() -> Program {
    Program::from_source(SRC).expect("program compiles")
}

#[test]
fn int_in_int_out() {
    let mut p = prog();
    assert_eq!(
        p.call("add", &[Value::Int(2), Value::Int(3)]).unwrap(),
        Value::Int(5)
    );
    // second call on the same program reuses the frame correctly
    assert_eq!(
        p.call("add", &[Value::Int(40), Value::Int(2)]).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn text_in_text_out() {
    let mut p = prog();
    // exercises the hidden text work-buffer path
    assert_eq!(
        p.call("greet", &[Value::Text("Ada".into())]).unwrap(),
        Value::Text("hi Ada".into())
    );
}

#[test]
fn boolean_and_single_returns() {
    let mut p = prog();
    assert_eq!(
        p.call("is_big", &[Value::Int(500)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        p.call("is_big", &[Value::Int(5)]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        p.call("dbl", &[Value::Float(9.0)]).unwrap(),
        Value::Float(18.0)
    );
}

/// A narrow integer parameter occupies a FULL stack cell, so the host marshal
/// must fill all eight bytes of it.
///
/// The stack steps in 8-byte units, so writing a `u8` argument as one byte does
/// not shorten the frame — the later arguments still land where the callee
/// expects them, and nothing crashes.  What happens instead is that the cell's
/// other seven bytes keep whatever the PREVIOUS call left there, and the callee
/// reads them as part of the number.  Hence `taint` first: it puts a
/// recognisable pattern in that cell, so a regression answers
/// `0x0F0F0F0F0F0F0F01` instead of `1` and says so in its own value.
#[test]
fn a_narrow_argument_does_not_inherit_the_previous_calls_bytes() {
    let mut p = prog();
    assert_eq!(p.call("thru_u8", &[Value::Int(1)]).unwrap(), Value::Int(1));
    assert_eq!(
        p.call("taint", &[Value::Int(0x0F0F_0F0F_0F0F_0F0F)])
            .unwrap(),
        Value::Int(0x0F0F_0F0F_0F0F_0F0F)
    );
    assert_eq!(
        p.call("thru_u8", &[Value::Int(1)]).unwrap(),
        Value::Int(1),
        "the u8 argument inherited the previous call's upper bytes"
    );
}

/// Sign is part of the value, in both directions.  A narrow signed type is
/// sign-extended in its stack cell, so reading or writing that cell at the
/// declared STORAGE width (`u8` = 1 byte) turns every negative into a large
/// positive — `-1` as an `i8` came back as `255`.
#[test]
fn narrow_integers_keep_their_sign_across_a_host_call() {
    let mut p = prog();
    for (func, v) in [
        ("thru_i8", -1),
        ("thru_i8", -128),
        ("thru_i8", 127),
        ("thru_i16", -1),
        ("thru_i16", -32_768),
        ("thru_i32", -1),
        ("thru_i32", -2_147_483_648),
        ("thru_u8", 255),
        ("thru_u32", 4_294_967_294),
    ] {
        assert_eq!(
            p.call(func, &[Value::Int(v)]).unwrap(),
            Value::Int(v),
            "{func}({v}) did not round-trip"
        );
    }
    for (func, v) in [
        ("ret_i8", -1),
        ("ret_i8", -128),
        ("ret_i16", -1),
        ("ret_i16", -32_768),
        ("ret_i32", -1),
        ("ret_i32", -2_147_483_648),
        ("ret_u8", 255),
    ] {
        assert_eq!(
            p.call(func, &[Value::Int(v)]).unwrap(),
            Value::Int(v),
            "{func} returning {v} did not round-trip"
        );
    }
}

#[test]
fn unknown_function() {
    let mut p = prog();
    assert!(matches!(p.call("nope", &[]), Err(LoftError::UnknownFn(_))));
}

#[test]
fn arg_count_mismatch() {
    let mut p = prog();
    assert!(matches!(
        p.call("add", &[Value::Int(1)]),
        Err(LoftError::ArgCount {
            expected: 2,
            got: 1,
            ..
        })
    ));
}

#[test]
fn arg_type_mismatch() {
    let mut p = prog();
    assert!(matches!(
        p.call("add", &[Value::Text("x".into()), Value::Int(2)]),
        Err(LoftError::ArgType { index: 0, .. })
    ));
}

#[test]
fn runtime_error_surfaces() {
    let mut p = prog();
    // assert failure raises → Runtime error, not a panic
    assert!(matches!(
        p.call("boom", &[Value::Int(0)]),
        Err(LoftError::Runtime(_))
    ));
    // and the program is still usable afterwards (error was drained)
    assert_eq!(p.call("boom", &[Value::Int(7)]).unwrap(), Value::Int(7));
}

#[test]
fn formatter_dogfood() {
    // The driver: call the loft-written formatter's `format(text) -> text`.
    let src = fa::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input = "struct P{x:integer,y:integer}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // definitions expand (the canonical rule); at minimum it changed + is non-empty
    assert!(out.contains("struct P {"), "got: {out:?}");
    assert!(out.contains("x: integer,"), "got: {out:?}");
}

#[test]
fn formatter_enum_variant_if_body_is_a_block() {
    // Regression: `if x == Enum.Variant { <stmts> }` — the CamelCase variant name
    // (`Directory`) reached via `.` must NOT be mistaken for a `Directory { … }` struct
    // literal.  When it was, the if-body was rendered as a COMMA container and the last
    // statement got a stray trailing comma (`walk_into(cp, out);,`), which is invalid
    // loft.  The formatter mirrors the compiler here: a dotted name (`Format.Directory`)
    // is an enum-value access, never a struct construction (see objects.rs::parse_object,
    // gated on `Name` being a Struct/EnumValue type + the next token being `{`).
    let src = fa::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input =
        "enum E { A, B }\nfn f(m: E) {\n  if m == E.A {\n    x = 1;\n    print(\"{x}\");\n  }\n}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // No stray comma anywhere (the exact bug symptom).
    assert!(
        !out.contains(";,"),
        "stray comma from struct-lit misclassification: {out:?}"
    );
    // The body is a real multi-statement BLOCK: each statement on its own line, not
    // comma-joined and not collapsed onto one line.
    assert!(
        out.contains("x = 1;\n"),
        "if-body statement 1 on its own line: {out:?}"
    );
    assert!(
        out.contains("print(\"{x}\");"),
        "if-body statement 2 preserved: {out:?}"
    );
    assert!(
        !out.contains("x = 1; print"),
        "multi-statement if-body must not be inlined as a container: {out:?}"
    );
}

#[test]
fn formatter_qualified_variant_before_block_is_not_a_struct_lit() {
    // Regression (tests/fixtures/libs/graphics/src/glb.loft): a CamelCase enum variant reached
    // via `::` (`scene::Point`) — or a `::`-qualified return type (`-> math::Vec3`) — placed right
    // before a BLOCK must render as a block, never a comma container.  The preceding-token
    // heuristic mis-classified it as a struct literal, because the `.`/`->` exclusions do not
    // cover a QUALIFIED name (loft resolves construct-vs-block with types; a formatter can't).  A
    // wrapping body was then rendered as comma-separated "elements", emitting invalid `;,`.  The
    // fix confirms against the body: a struct/variant literal holds `field:` pairs and never a
    // statement, so a `;`-bearing body is a block whatever precedes the brace.
    let src = fa::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    // a long body so a mis-rendered container WRAPS and exposes the `;,` (an inline one hides it).
    let input = "enum Light { Point }\nfn f(lt: Light) -> text {\n  out = \"\";\n  if lt == Light::Point { aaaaaa = 111111; bbbbbb = 222222; cccccc = 333333; out += \"translation data here\"; }\n  out\n}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // The exact corruption symptom: a statement turned into a comma-separated container element.
    assert!(
        !out.contains(";,"),
        "qualified variant before a block must not render as a comma container: {out:?}"
    );
    // The body is a real block: statements keep their `;`, and none becomes a `stmt,` element.
    assert!(
        out.contains("aaaaaa = 111111;"),
        "block statement preserved with its semicolon: {out:?}"
    );
    assert!(
        !out.contains("aaaaaa = 111111,"),
        "statement must not become a container element: {out:?}"
    );
}

#[test]
fn formatter_width_counts_characters_not_bytes() {
    // Regression (@PLN110): the formatter measures DISPLAY width, which is a CHARACTER
    // count — not a byte count. This assert line is 89 characters but 103 UTF-8 bytes
    // (em-dashes are 1 char / 3 bytes); it fits the 100-column budget and must stay on
    // one line. When width was measured with `size(text)` (which became the BYTE count
    // at the flip), the formatter over-measured multi-byte lines and wrapped ones that
    // visually fit — the fix uses `len` (characters) for column/width math.
    let src = fa::read_to_string("tools/fmt/whole.loft").expect("formatter source");
    let mut p = Program::from_source(&src).expect("formatter compiles");
    let input = "fn f(a: integer, b: integer) {\n  assert(a == b, \"mismatch —  —  —  —  —  —  — end padding to reach target length here\");\n}\n";
    let out = p
        .call("format", &[Value::Text(input.into())])
        .unwrap()
        .into_text()
        .unwrap();
    // the assert's args stay on ONE line (char-width 89 < 100); a byte-width measure
    // (103 > 100) would have wrapped them onto separate lines (`assert(\n a == b, …`).
    assert!(
        out.contains("assert(a == b, \"mismatch"),
        "multi-byte line under the char budget must stay one line: {out:?}"
    );
    assert!(
        !out.contains("assert(\n"),
        "args must not wrap — width is character count, not bytes: {out:?}"
    );
}

/// What one host call costs, as a REPORT — run it, read the number, never gate
/// on it (`cargo test --release --test host_call measure_call_cost -- --ignored
/// --nocapture`).
///
/// It exists because this cost is invisible in the tests above and used to be
/// dominated by something none of them could see: publishing the fault-site span
/// table deep-cloned the whole map on every entry, which costs nothing for a
/// program entered once and 4.4 µs of every 4.7 µs call for a program entered in
/// a loop.  A `loft::host` caller does exactly that, and so does every call to a
/// process-placed library (@PLN119), which travels the same path.  Roughly 0.5 µs
/// on this machine after the fix; an answer in microseconds means the snapshot is
/// being rebuilt per call again.
#[test]
#[ignore = "a measurement, not a gate — run with --test host_call measure_call_cost -- --ignored --nocapture"]
fn measure_call_cost() {
    let mut p = prog();
    let n = 100_000;
    let t = std::time::Instant::now();
    for _ in 0..n {
        let _ = p.call("add", &[Value::Int(2), Value::Int(3)]).unwrap();
    }
    let d = t.elapsed();
    println!("host call: {d:?} total, {:?}/call", d / n);
}
