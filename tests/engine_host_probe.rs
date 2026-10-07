// @PLN18 phase 01 — entry probes for the kernel's dispatch mechanic.
//
// Probe 1 (this test): ONE persistent State can dispatch a loft fn repeatedly
// (re-entry safety of execute_argv on a long-lived State) — the kernel calls
// handlers thousands of times per second on one State, so this must hold.
//
// Findings recorded in plans/18-engine-host/ (phase 01 design):
// - loft has NO mutable globals (top-level bindings are constants), so the
//   kernel's world-state model is STORE-ANCHORED: the kernel keeps the world's
//   DbRef and hands it to handlers via a kernel-registered native
//   (`State::static_fn`, the `fn(&mut Stores, &mut DbRef)` ABI) — probe 2,
//   with the kernel skeleton.
use loft::compile;
use loft::file_access as fa;
use loft::parser::Parser;
use loft::state::State;

/// Disk-backed scratch for test fixtures.  `std::env::temp_dir()` is a
/// RAM-backed tmpfs on dev boxes (small quota, shared across sessions), and
/// loft's cache-next-to-source rule would put every `--native` fixture's
/// binary cache there too — the disk-quota stall class.  `target/` lives on
/// disk and is cleaned with the build tree.
fn test_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = fa::create_dir_all(&dir);
    dir
}

#[test]
fn persistent_state_repeated_dispatch() {
    let outp = test_tmp().join(format!("eh_probe_out_{}.txt", std::process::id()));
    let src = format!(
        r#"
fn on_call(args: vector<text>) {{
  n = (args[0] ?? "0") as integer;
  f = file("{}");
  f.write("call {{n}} sum={{n * (n + 1) / 2}}");
}}
"#,
        // Windows temp paths carry backslashes, which a loft string literal
        // reads as ESCAPES (\U, \T…) — forward slashes are valid on every
        // platform's filesystem API.
        outp.to_string_lossy().replace('\\', "/")
    );
    let tmp = test_tmp().join(format!("eh_probe_{}.loft", std::process::id()));
    fa::write(&tmp, &src).unwrap();

    let mut parser = Parser::new();
    parser.parse_dir("default", true, false).expect("stdlib");
    parser.parse(&tmp.to_string_lossy(), false);
    loft::scopes::check(&mut parser.data, &mut parser.database);
    let mut data = parser.data;
    let mut state = State::new(parser.database);
    compile::byte_code(&mut state, &mut data);

    // Re-entry: many dispatches on the SAME State must stay sound (the kernel's
    // per-event call pattern).  The last call's file tells us the final args
    // arrived intact through the whole sequence.
    for i in 1..=50 {
        state.execute_argv("on_call", &data, &[i.to_string()]);
    }
    let out = fa::read_to_string(&outp).expect("report file");
    assert_eq!(
        out.trim(),
        "call 50 sum=1275",
        "50th dispatch intact: {out}"
    );
    let _ = fa::remove_file(&tmp);
    let _ = fa::remove_file(&outp);
}
