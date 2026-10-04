// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B — the database source, from the C surface up.
//
// The first test here is step 0's probe, GRADUATED rather than deleted: it is
// the question every later step rests on — can core drive a C library through
// `c_call::resolve` from Rust, with no rustc, no loft frame and no re-entrancy?
// Kept because the answer can regress: a change to symbol resolution would
// otherwise surface as a lazy fetch that mysteriously finds nothing.

#![cfg(all(feature = "native-extensions", unix))]

mod common;
extern crate loft;

use common::cached_default;

use std::ffi::{CStr, CString, c_char, c_int, c_void};

/// The library name to `dlopen`, spelled the way each platform spells it — the
/// versioned `.so.0` on Linux, the plain `.dylib` macOS ships in its shared
/// cache.  One name for both would not fail; it would SKIP, which is the outcome
/// a whole platform's coverage disappears into.
const SQLITE_LIB: &str = if cfg!(target_os = "macos") {
    "libsqlite3.dylib"
} else {
    "libsqlite3.so.0"
};

fn declare_sqlite() {
    loft::c_call::set_declared_libraries(vec![loft::data::CLibrary {
        name: SQLITE_LIB.to_string(),
        pkg_dir: String::new(),
        optional: true,
    }]);
}

/// Serialises every test in this file, because they share one process-wide fact.
///
/// `c_call::register` REPLACES the declared-library list with the program's own
/// — an EMPTY list for a script that declares no `#c` bindings, which is every
/// script here.  So a test that merely RUNS a loft program wipes the sqlite
/// declaration its neighbour is standing on, and the neighbour's `resolve` then
/// answers `None` for a library that is installed.  The per-source query
/// counters are the same kind of fact.
///
/// Under nextest (a process per test) that cannot happen and this lock costs
/// nothing.  Under `cargo test`, which runs a binary's tests as threads in ONE
/// process, it is the difference between a suite and a coin toss.
static SQLITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The gate every test here opens with: take the process to ourselves, declare
/// the library, and say whether this machine can run the test at all.
///
/// A developer box with no sqlite SKIPS — the courtesy the rest of the suite
/// already extends to a missing node or chrome.  But a skip and a pass are
/// indistinguishable in a summary, and these tests are the ONLY coverage the
/// lazy database source has: let them self-skip unremarked and a runner image
/// that drops the library retires the whole suite without turning anything red.
///
/// So the skip is never silent.  It is recorded in the environmental-skip
/// ledger, which CI drains into annotations and a job summary; and where the
/// library is expected rather than hoped for, `LOFT_REQUIRE_SQLITE=1` turns the
/// skip into a failure.  CI installs sqlite and sets it, so a green CI run means
/// these tests RAN.
fn sqlite_guard(test: &str) -> Option<std::sync::MutexGuard<'static, ()>> {
    let held = SQLITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    declare_sqlite();
    if loft::c_call::resolve("sqlite3_open_v2").is_some() {
        return Some(held);
    }
    assert!(
        std::env::var("LOFT_REQUIRE_SQLITE").as_deref() != Ok("1"),
        "LOFT_REQUIRE_SQLITE=1 but {SQLITE_LIB} did not resolve.  These tests are the \
         only coverage the lazy database source has, so skipping them here would report \
         a green run for a suite that never executed."
    );
    eprintln!("SKIP: {SQLITE_LIB} not installed");
    common::record_env_skips(
        "lazy_sql_source",
        "no-libsqlite3",
        &[(
            test.to_string(),
            format!("{SQLITE_LIB} did not resolve — the lazy database source went untested"),
        )],
    );
    None
}

#[test]
fn probe_core_can_drive_sqlite_through_resolve() {
    let Some(_sqlite) = sqlite_guard("probe_core_can_drive_sqlite_through_resolve") else {
        return;
    };
    let open = loft::c_call::resolve("sqlite3_open").expect("sqlite3_open");
    let exec = loft::c_call::resolve("sqlite3_exec").expect("sqlite3_exec");
    let prepare = loft::c_call::resolve("sqlite3_prepare_v2").expect("sqlite3_prepare_v2");
    let step = loft::c_call::resolve("sqlite3_step").expect("sqlite3_step");
    let col_text = loft::c_call::resolve("sqlite3_column_text").expect("sqlite3_column_text");
    let finalize = loft::c_call::resolve("sqlite3_finalize").expect("sqlite3_finalize");
    let close = loft::c_call::resolve("sqlite3_close").expect("sqlite3_close");

    // Typed, not the u64 trampoline ladder: core knows the signature at compile
    // time, so it can have the ABI by construction the way `--native` does.
    type Open = extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
    type Exec = extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_void,
        *const c_void,
        *mut *mut c_char,
    ) -> c_int;
    type Prepare = extern "C" fn(
        *mut c_void,
        *const c_char,
        c_int,
        *mut *mut c_void,
        *mut *const c_char,
    ) -> c_int;
    type Step = extern "C" fn(*mut c_void) -> c_int;
    type ColText = extern "C" fn(*mut c_void, c_int) -> *const c_char;
    type Fin = extern "C" fn(*mut c_void) -> c_int;
    type Close = extern "C" fn(*mut c_void) -> c_int;

    let open: Open = unsafe { std::mem::transmute(open) };
    let exec: Exec = unsafe { std::mem::transmute(exec) };
    let prepare: Prepare = unsafe { std::mem::transmute(prepare) };
    let step: Step = unsafe { std::mem::transmute(step) };
    let col_text: ColText = unsafe { std::mem::transmute(col_text) };
    let finalize: Fin = unsafe { std::mem::transmute(finalize) };
    let close: Close = unsafe { std::mem::transmute(close) };

    let path = CString::new(":memory:").unwrap();
    let mut db: *mut c_void = std::ptr::null_mut();
    assert_eq!(open(path.as_ptr(), &raw mut db), 0, "sqlite3_open");
    assert!(!db.is_null());

    let ddl = CString::new(
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (42,'grace')",
    )
    .unwrap();
    assert_eq!(
        exec(
            db,
            ddl.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut()
        ),
        0,
        "sqlite3_exec"
    );

    // The shape a fault needs: one derived SELECT, one row, one column value.
    let sql = CString::new("SELECT name FROM person WHERE id = 42").unwrap();
    let mut st: *mut c_void = std::ptr::null_mut();
    assert_eq!(
        prepare(db, sql.as_ptr(), -1, &raw mut st, std::ptr::null_mut()),
        0,
        "sqlite3_prepare_v2"
    );
    assert_eq!(step(st), 100, "SQLITE_ROW");
    let name = col_text(st, 0);
    assert!(!name.is_null());
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .to_string();
    assert_eq!(name, "grace");
    finalize(st);

    // A SECOND resolve in the same process must reuse the loaded handle rather
    // than re-open: `resolve` searches what is already loaded first.
    let again = loft::c_call::resolve("sqlite3_open").expect("second resolve");
    assert_eq!(again, open as *const ());

    close(db);
}

// ── Step 4: the source that executes ──────────────────────────────────────────

use loft::database::sql_source::{Cell, Driver, SqlConn, driver_of, queries_run};

/// A scratch database seeded through sqlite's own CLI-free path: the connection
/// core opens is READ-ONLY, so the fixture is written with the raw symbols the
/// probe above already proved work.
fn seed(path: &std::path::Path, ddl: &str) {
    let exec = loft::c_call::resolve("sqlite3_exec").expect("sqlite3_exec");
    let open = loft::c_call::resolve("sqlite3_open").expect("sqlite3_open");
    let close = loft::c_call::resolve("sqlite3_close").expect("sqlite3_close");
    type Open = extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
    type Exec = extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_void,
        *const c_void,
        *mut *mut c_char,
    ) -> c_int;
    type Close = extern "C" fn(*mut c_void) -> c_int;
    let open: Open = unsafe { std::mem::transmute(open) };
    let exec: Exec = unsafe { std::mem::transmute(exec) };
    let close: Close = unsafe { std::mem::transmute(close) };

    let p = CString::new(path.to_string_lossy().as_ref()).unwrap();
    let mut db: *mut c_void = std::ptr::null_mut();
    assert_eq!(open(p.as_ptr(), &raw mut db), 0);
    let sql = CString::new(ddl).unwrap();
    assert_eq!(
        exec(
            db,
            sql.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut()
        ),
        0,
        "seed failed"
    );
    close(db);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_pln129_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join("people.db")
}

#[test]
fn a_source_string_names_its_driver() {
    assert_eq!(
        driver_of("sqlite:people.db"),
        Some((Driver::Sqlite, "people.db"))
    );
    // An image path is not a database, and must stay the file source's business.
    assert_eq!(driver_of("people.store"), None);
    assert_eq!(driver_of("https://example.com/people.store"), None);
}

#[test]
fn the_derived_query_runs_and_its_columns_come_back() {
    let Some(_sqlite) = sqlite_guard("the_derived_query_runs_and_its_columns_come_back") else {
        return;
    };
    let path = scratch("exec");
    seed(
        &path,
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (1,'ada'),(42,'grace'),(7,NULL),(9,'')",
    );

    let conn = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("open");
    // Exactly what `derive_select` emits for `hash<Person[id]>`.
    let rows = conn
        .query(
            "SELECT \"id\", \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(42)],
        )
        .expect("query");
    assert_eq!(rows.len(), 1, "one key, one row");
    assert_eq!(rows[0][0], Some(Cell::Int(42)));
    assert_eq!(rows[0][1], Some(Cell::Text("grace".to_string())));

    // The NULL crossing: SQL NULL, '' and a value are THREE answers, and a
    // binding that collapses any two of them is the bug @PLN23's fixture exists
    // to catch.
    let rows = conn
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(7)],
        )
        .expect("query");
    assert_eq!(rows[0][0], None, "SQL NULL is not the empty string");
    let rows = conn
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(9)],
        )
        .expect("query");
    assert_eq!(rows[0][0], Some(Cell::Text(String::new())));

    // A key that is not there is an ABSENCE — no rows, no error.
    let rows = conn
        .query(
            "SELECT \"id\", \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(999)],
        )
        .expect("query");
    assert!(rows.is_empty());

    // A text key binds as text, not as a number that happens to parse.
    let rows = conn
        .query(
            "SELECT \"id\" FROM \"person\" WHERE \"name\" = ?",
            &[Cell::Text("ada".to_string())],
        )
        .expect("query");
    assert_eq!(rows[0][0], Some(Cell::Int(1)));

    // A second open of the same target REUSES the handle: the same open database, not a
    // new one.  A connection per fault would cost more than the eager load laziness
    // replaces.
    let again = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("reopen is cached");
    assert!(
        again.same_connection(&conn),
        "a second open reuses the handle"
    );
    let rows = again
        .query(
            "SELECT \"name\" FROM \"person\" WHERE \"id\" = ?",
            &[Cell::Int(42)],
        )
        .expect("the cached handle reads");
    assert_eq!(rows[0][0], Some(Cell::Text("grace".to_string())));
}

#[test]
fn the_connection_is_read_only_and_a_bad_query_says_why() {
    let Some(_sqlite) = sqlite_guard("the_connection_is_read_only_and_a_bad_query_says_why") else {
        return;
    };
    let path = scratch("readonly");
    seed(
        &path,
        "CREATE TABLE person(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO person VALUES (42,'grace')",
    );
    let conn = SqlConn::open(Driver::Sqlite, &path.to_string_lossy()).expect("open");

    // v1 is read-only, and the HANDLE enforces it — not a convention a later
    // caller can forget (README failure path 4).
    let err = conn
        .query("INSERT INTO person VALUES (1,'ada')", &[])
        .expect_err("a write must be refused");
    assert!(
        err.contains("readonly") || err.contains("read-only"),
        "{err}"
    );

    // A schema that drifted surfaces as the engine's own text rather than as an
    // absence — the distinction arc C is built on.
    //
    // And this cell is the reason the connection turns SQLite's double-quoted
    // string literals OFF. Measured with them on, against a table holding one
    // row: this query answered `[Some(Text("naam"))]` — no error, the COLUMN NAME
    // returned as data, for every row. The derived query quotes every identifier,
    // so a renamed column would have materialised its own name into the record
    // (failure path 9, in its cruellest form).
    let err = conn
        .query("SELECT \"naam\" FROM \"person\"", &[])
        .expect_err("a missing column must be an error, never a string literal");
    assert!(err.contains("naam"), "{err}");

    // An unreachable database is unreachable, not empty.
    let missing = SqlConn::open(Driver::Sqlite, "/nonexistent/dir/nope.db");
    assert!(missing.is_err());
}

// ── Step 6: the miss path is wired, and the COUNTS are the assertion ───────────

/// Run one loft program in-process, the way the test harness does, so the query
/// counter can be read afterwards.
fn run_loft(src: &str) {
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(src, "lazy_sql_count", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "the program did not compile: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    if let Some(err) = state.database.runtime_error.take() {
        panic!("{}", err.message);
    }
}

#[test]
fn a_keyed_lookup_costs_exactly_one_query_and_a_hit_costs_none() {
    let Some(_sqlite) = sqlite_guard("a_keyed_lookup_costs_exactly_one_query_and_a_hit_costs_none")
    else {
        return;
    };
    let path = scratch("counts");
    seed(
        &path,
        "CREATE TABLE cperson(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO cperson VALUES (1,'ada'),(42,'grace'),(7,'alan')",
    );
    let src = format!(
        r#"
struct CPerson {{ id: integer, name: text }}

fn test() {{
  people: hash<CPerson[id]> = [];
  assert(store_bind_lazy(people, "sqlite:{}"), "bind");
  p = people[42];
  assert(p.name == "grace", "first fetch: {{p.name}}");
  assert(people.len() == 1, "one record touched, one resident");
  q = people[42];
  assert(p == q, "one record, however it is reached");
  assert(people.len() == 1, "a hit adds nothing");
  r = people[7];
  assert(r.name == "alan", "second fetch");
  assert(people.len() == 2, "two touched");
  assert(people[999] == null, "absent is absent");
  assert(store_lazy_error(people) == "", "an absence is not a failure");
}}
"#,
        path.to_string_lossy()
    );

    let target = path.to_string_lossy().to_string();
    let before = queries_run(&target);
    run_loft(&src);
    let spent = queries_run(&target) - before;
    // Three lookups reach the source — 42, 7, and the absent 999 — and the
    // SECOND lookup of 42 must not, because it hit the working set. This is the
    // assertion that separates a lazy read from an eager one: the values above
    // would be identical either way.
    //
    // Plus TWO for step 7's schema check (`PRAGMA table_info` and `EXPLAIN QUERY
    // PLAN`), and the fact that the total is 5 rather than 9 is the assertion
    // that it runs ONCE per binding: a check repeated per fault would triple the
    // cost of the feature to re-learn something that cannot change.
    assert_eq!(
        spent, 5,
        "3 lookups that reach the source + 2 one-off schema probes"
    );
}

// ── Step 11: arc F's gate, over SQL ───────────────────────────────────────────

/// @PLN129 arc B step 11 — the graph traversal arc F proved against a file,
/// now against a database.
///
/// Two assertions carry it and neither is a value. **Identity**: two persons at
/// the same company must reach ONE company record. **Count**: `c=1` after the
/// second hop is what proves the hop HIT the working set — every value here
/// would pass under an eager load, and only the counts would not.
#[test]
fn the_graph_traverses_lazily_over_sql_both_backends() {
    let Some(_sqlite) = sqlite_guard("the_graph_traverses_lazily_over_sql_both_backends") else {
        return;
    };
    let dir = scratch("graph");
    let persons = dir.with_file_name("persons.db");
    let companies = dir.with_file_name("companies.db");
    seed(
        &companies,
        "CREATE TABLE sqcompany(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO sqcompany VALUES (7,'Acme'),(9,'Globex'),(11,'Initech')",
    );
    seed(
        &persons,
        "CREATE TABLE sqpersong(id INTEGER PRIMARY KEY, name TEXT, employer INTEGER); \
         INSERT INTO sqpersong VALUES (1,'ada',7),(2,'grace',7),(3,'alan',9),(4,'edsger',11)",
    );

    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/129-lazy-sql-graph.loft");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&script)
            .env("LOFT_SQL_PERSONS", format!("sqlite:{}", persons.display()))
            .env(
                "LOFT_SQL_COMPANIES",
                format!("sqlite:{}", companies.display()),
            )
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(out.status.success(), "{backend}: {stdout}");

        // The control: both collections start empty, so the counts mean something.
        assert!(stdout.contains("start p=0 c=0"), "{backend}: {stdout}");
        assert!(
            stdout.contains("hop1 ada@Acme p=1 c=1"),
            "{backend}: one hop fetches one person and one company: {stdout}"
        );
        assert!(
            stdout.contains("hop2 grace@Acme p=2 c=1"),
            "{backend}: a second person at the SAME company must NOT re-fetch it: {stdout}"
        );
        assert!(
            stdout.contains("identity=true"),
            "{backend}: two paths to one company must give ONE record — if this \
             fails the design is wrong: {stdout}"
        );
        assert!(
            stdout.contains("hop3 alan@Globex p=3 c=2"),
            "{backend}: a different company IS fetched: {stdout}"
        );
        assert!(
            stdout.contains("touched=5"),
            "{backend}: 3 persons + 2 companies — edsger and Initech were never \
             asked for and must not be resident: {stdout}"
        );
        assert!(
            stdout.contains("sound=true,true"),
            "{backend}: both partially-loaded heaps must be structurally sound: {stdout}"
        );
        assert!(
            stdout.contains("healthy=\n"),
            "{backend}: a clean traversal reports NO fault, and the two error \
             channels concatenated must therefore be empty: {stdout}"
        );
    }
}

// ── Step 7: the schema check, and the failure that does not announce itself ────

/// The refusal a lookup reports for this database, or `""` when the fetch worked.
fn bind_and_look(db_path: &std::path::Path, program: &std::path::Path) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(program)
        .env("LOFT_SQL_TARGET", format!("sqlite:{}", db_path.display()))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to invoke loft");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_schema_that_cannot_serve_the_lookup_is_refused_and_says_why() {
    let Some(_sqlite) =
        sqlite_guard("a_schema_that_cannot_serve_the_lookup_is_refused_and_says_why")
    else {
        return;
    };
    let program = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/129-lazy-sql-schema.loft");

    // The control: a table with an indexed key and matching types FETCHES.
    let good = scratch("schema_ok").with_file_name("good.db");
    seed(
        &good,
        "CREATE TABLE sqchecked(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO sqchecked VALUES (42,'grace')",
    );
    let out = bind_and_look(&good, &program);
    assert!(out.contains("value=grace"), "control must fetch: {out}");
    assert!(out.contains("why=[]"), "control must report healthy: {out}");

    // A column the type declares and the table does not.
    let missing = scratch("schema_col").with_file_name("missing.db");
    seed(
        &missing,
        "CREATE TABLE sqchecked(id INTEGER PRIMARY KEY, naam TEXT); \
         INSERT INTO sqchecked VALUES (42,'grace')",
    );
    let out = bind_and_look(&missing, &program);
    assert!(
        out.contains("value=null"),
        "a refused bind answers null: {out}"
    );
    assert!(
        out.contains("no column `name`"),
        "the refusal must NAME what is wrong: {out}"
    );

    // A column whose affinity cannot hold the field: `name` is loft `text` and
    // the column is INTEGER, so every fetch would reinterpret someone's data.
    let wrong = scratch("schema_type").with_file_name("wrong.db");
    seed(
        &wrong,
        "CREATE TABLE sqchecked(id INTEGER PRIMARY KEY, name INTEGER); \
         INSERT INTO sqchecked VALUES (42,7)",
    );
    let out = bind_and_look(&wrong, &program);
    assert!(out.contains("value=null"), "{out}");
    assert!(
        out.contains("affinity"),
        "the refusal must name the type: {out}"
    );

    // The one that does NOT announce itself: no index on the key. Every answer
    // stays right and every fault reads the whole table — a working feature and
    // a catastrophic one, which is why this is measured rather than assumed.
    let unindexed = scratch("schema_scan").with_file_name("scan.db");
    seed(
        &unindexed,
        "CREATE TABLE sqchecked(id INTEGER, name TEXT); \
         INSERT INTO sqchecked VALUES (42,'grace')",
    );
    let out = bind_and_look(&unindexed, &program);
    assert!(
        out.contains("value=null"),
        "an unindexed bind must be refused, not served slowly: {out}"
    );
    assert!(
        out.contains("no index") && out.contains("SCAN"),
        "the refusal must quote the plan that proves it: {out}"
    );
}

// ── Step 9 (B2): the explicit query, and one record however it arrived ────────

/// @PLN129 arc B2 — an explicit predicate populates the COLLECTION, so a person
/// found by `LIKE` and the same person found by key are one record.
#[test]
fn an_explicit_query_populates_the_collection_both_backends() {
    let Some(_sqlite) = sqlite_guard("an_explicit_query_populates_the_collection_both_backends")
    else {
        return;
    };
    let path = scratch("b2").with_file_name("liked.db");
    seed(
        &path,
        "CREATE TABLE sqliked(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO sqliked VALUES (1,'ada'),(2,'adam'),(42,'grace'),(7,'alan')",
    );
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/129-lazy-sql-query.loft");

    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&script)
            .env("LOFT_SQL_TARGET", format!("sqlite:{}", path.display()))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(out.status.success(), "{backend}: {stdout}");

        assert!(
            stdout.contains("key=grace resident=1"),
            "{backend}: {stdout}"
        );
        // TWO added, not three: grace was already resident and must be left
        // alone. A count of 3 would mean a second record for one person.
        assert!(
            stdout.contains("added=2 resident=3"),
            "{backend}: a row already resident is skipped, not fetched twice: {stdout}"
        );
        assert!(
            stdout.contains("identity=true"),
            "{backend}: the query POPULATES the collection — a detached result set \
             would pass every value assertion and fail this one: {stdout}"
        );
        assert!(
            stdout.contains("brought_in=ada,adam"),
            "{backend}: and the rows it brought in are readable by key: {stdout}"
        );
        // A predicate matching nothing is an ordinary answer.
        assert!(
            stdout.contains("none=0 still=3 err2=[]"),
            "{backend}: no match is not a failure: {stdout}"
        );
        // A predicate that cannot RUN is not the same answer.
        assert!(
            stdout.contains("bad=0 err3_empty=false"),
            "{backend}: a broken query must be reported, not read as empty: {stdout}"
        );
        assert!(
            stdout.contains("sound=true"),
            "{backend}: the partially-loaded heap must be structurally sound: {stdout}"
        );
    }
}

// ── Step 8: a range is ONE query ─────────────────────────────────────────────

/// @PLN129 arc B step 8 — the batching claim, and the ordered kinds.
///
/// The COUNT is the assertion: five records for one query is what makes lazy
/// reading usable, and the same five values would come back from five lookups.
#[test]
fn a_key_range_is_one_query_both_backends() {
    let Some(_sqlite) = sqlite_guard("a_key_range_is_one_query_both_backends") else {
        return;
    };
    let path = scratch("range").with_file_name("events.db");
    let rows: Vec<String> = (1..=20).map(|i| format!("({i},'e{i}')")).collect();
    seed(
        &path,
        &format!(
            "CREATE TABLE sqevent(at INTEGER PRIMARY KEY, what TEXT); \
             INSERT INTO sqevent VALUES {}",
            rows.join(",")
        ),
    );
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/129-lazy-sql-range.loft");

    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&script)
            .env("LOFT_SQL_TARGET", format!("sqlite:{}", path.display()))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(out.status.success(), "{backend}: {stdout}");

        // A keyed lookup on an ORDERED collection — the hash-only restriction is
        // gone, because materialising goes through the same `record_finish`
        // every insert does.
        assert!(
            stdout.contains("keyed=e12 resident=1"),
            "{backend}: {stdout}"
        );
        assert!(
            stdout.contains("added=5 resident=6 err=[]"),
            "{backend}: one range, five records: {stdout}"
        );
        // Placed by KEY, not appended in arrival order: 12 was fetched first and
        // must sort after 9.
        assert!(
            stdout.contains("order=5 6 7 8 9 12 "),
            "{backend}: an ordered collection places what it materialises: {stdout}"
        );
        assert!(
            stdout.contains("again=2 resident=8"),
            "{backend}: an overlapping range adds only what is new: {stdout}"
        );
        assert!(
            stdout.contains("hash_range=0 why_empty=false"),
            "{backend}: a hash has no order to range over, and says so: {stdout}"
        );
        assert!(
            stdout.contains("sound=true"),
            "{backend}: the partially-loaded heap must be structurally sound: {stdout}"
        );
    }
}

#[test]
fn a_range_of_five_records_costs_one_query() {
    let Some(_sqlite) = sqlite_guard("a_range_of_five_records_costs_one_query") else {
        return;
    };
    let path = scratch("range_count").with_file_name("evcount.db");
    let rows: Vec<String> = (1..=20).map(|i| format!("({i},'e{i}')")).collect();
    seed(
        &path,
        &format!(
            "CREATE TABLE sqcounted(at INTEGER PRIMARY KEY, what TEXT); \
             INSERT INTO sqcounted VALUES {}",
            rows.join(",")
        ),
    );
    let src = format!(
        r#"
struct SqCounted {{ at: integer, what: text }}

fn test() {{
  events: sorted<SqCounted[at]> = [];
  assert(store_bind_lazy(events, "sqlite:{}"), "bind");
  assert(store_lazy_range(events, 5, 9) == 5, "five records");
  assert(events.len() == 5, "five resident");
}}
"#,
        path.to_string_lossy()
    );

    let target = path.to_string_lossy().to_string();
    let before = queries_run(&target);
    run_loft(&src);
    let spent = queries_run(&target) - before;
    // ONE query for five records, plus the two one-off schema probes. Five
    // lookups would have cost five, and that difference is the whole reason the
    // range form exists — N+1 is not a slow path, it is the natural way to
    // write the traversal unless there is a better one.
    assert_eq!(spent, 3, "1 range query + 2 one-off schema probes");
}

/// @C104 — a SLICE on a lazily-bound collection reads the resident set and never fetches;
/// a range is the explicit `store_lazy_range`.  The source holds 20 rows and five are
/// resident, so a slice that consulted the source would walk 19 records over `1..20` and
/// cost a query; the resident-set answer is 5, and the query count stays the range's own.
/// `len` agrees with the walk, which is the decision's reason: a fetching slice would make
/// `xs[lo..hi]`, `for x in xs` and `len(xs)` disagree.
#[test]
fn a_slice_reads_what_is_resident_and_fetches_nothing() {
    let Some(_sqlite) = sqlite_guard("a_slice_reads_what_is_resident_and_fetches_nothing") else {
        return;
    };
    let path = scratch("slice_count").with_file_name("evslice.db");
    let rows: Vec<String> = (1..=20).map(|i| format!("({i},'e{i}')")).collect();
    seed(
        &path,
        &format!(
            "CREATE TABLE sqsliced(at INTEGER PRIMARY KEY, what TEXT); \
             INSERT INTO sqsliced VALUES {}",
            rows.join(",")
        ),
    );
    let src = format!(
        r#"
struct SqSliced {{ at: integer, what: text }}

fn test() {{
  events: sorted<SqSliced[at]> = [];
  assert(store_bind_lazy(events, "sqlite:{}"), "bind");
  assert(store_lazy_range(events, 5, 9) == 5, "five records by the explicit range");
  walked = 0;
  keys = "";
  for e in events[1..20] {{ walked += 1; keys += "{{e.at}} "; }}
  assert(walked == 5, "the slice walks the resident five, not the source's 19: {{walked}}");
  assert(keys == "5 6 7 8 9 ", "in key order: {{keys}}");
  assert(events.len() == 5, "and made nothing resident: {{events.len()}}");
}}
"#,
        path.to_string_lossy()
    );

    let target = path.to_string_lossy().to_string();
    let before = queries_run(&target);
    run_loft(&src);
    let spent = queries_run(&target) - before;
    // The range's one query and the two one-off schema probes — the same count
    // `a_range_of_five_records_costs_one_query` pins without a slice.
    assert_eq!(spent, 3, "the slice issued a query");
}

// ── B4's shape: a collection-valued field as an owner-parameterised query ─────

/// @PLN129 arc B4 — `company.people` is `WHERE company_id = <this company>`,
/// and it needs no new language surface: the explicit query spells it.
///
/// What it DID need is for a collection declared as a struct FIELD to resolve to
/// its own type rather than the wrapper's — the same resolution the paged loader
/// made for #632, now shared rather than duplicated.
#[test]
fn a_collection_field_is_an_owner_parameterised_query_both_backends() {
    let Some(_sqlite) =
        sqlite_guard("a_collection_field_is_an_owner_parameterised_query_both_backends")
    else {
        return;
    };
    let path = scratch("owner").with_file_name("hands.db");
    seed(
        &path,
        "CREATE TABLE sqhand(id INTEGER PRIMARY KEY, name TEXT, company_id INTEGER); \
         INSERT INTO sqhand VALUES (1,'ada',7),(2,'grace',7),(3,'alan',7),(4,'edsger',9); \
         CREATE INDEX ix_hand_company ON sqhand(company_id)",
    );
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/129-lazy-sql-owner.loft");

    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&script)
            .env("LOFT_SQL_TARGET", format!("sqlite:{}", path.display()))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(out.status.success(), "{backend}: {stdout}");

        assert!(stdout.contains("bound=true"), "{backend}: {stdout}");
        assert!(
            stdout.contains("added=3 resident=3 err=[]"),
            "{backend}: Acme's three, and only Acme's: {stdout}"
        );
        assert!(
            stdout.contains("who=ada grace alan "),
            "{backend}: {stdout}"
        );
        // Per COLLECTION, not per store: two fields of one type over one table,
        // each holding its owner's rows.
        assert!(
            stdout.contains("other=1 resident=1 acme_unchanged=3"),
            "{backend}: a second field is a separate binding: {stdout}"
        );
        // A row the query already brought in is a HIT, not a fetch.
        assert!(
            stdout.contains("keyed=alan resident_after=3"),
            "{backend}: a lookup finds what the query populated: {stdout}"
        );
        assert!(
            stdout.contains("ambiguous=0 why_empty=false"),
            "{backend}: a wrapper with two keyed fields refuses rather than \
             guessing which one a reference names: {stdout}"
        );
        assert!(
            stdout.contains("sound=true,true"),
            "{backend}: the filled FIELD and its owner must both verify: {stdout}"
        );
    }
}

/// @PLN133 S8 — the lazy fetch is LOFT CODE, called re-entrantly from inside the
/// lookup that missed.
///
/// Core drives one database in Rust (sqlite) and the loft library drives four
/// behind one interface. Restating the other three in Rust is N drivers now and
/// +1 forever, with the loft versions left to drift — so a collection bound to a
/// scheme core has no driver for CALLS loft instead.
///
/// @PLN129 costed this as a bytecode-level control change and it is not: the
/// retry already existed (the collection stays the only authority on what is
/// resident), and what was missing is only that the fetch was Rust.
///
/// **Both backends, one answer, and they reach it differently.** The interpreter
/// runs the driver through the ordinary call machinery — `fn_call` pushes the
/// frame, the dispatch loop runs until it pops. `--native` cannot do that:
/// `OpGetRecord` is compiled into libloft and cannot see a function the
/// generator wrote, so generated `init()` installs a POINTER to it. Two
/// mechanisms, and the output is compared whole rather than line by line,
/// because that is the only assertion that catches a divergence nobody predicted.
#[test]
fn a_lazy_fetch_can_be_a_loft_function() {
    let _serial = SQLITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/133-lazy-loft-driver.loft");
    let run = |backend: &str| -> String {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg(&script)
            // The leak below is asserted on BOTH backends, and native only
            // counts stores when asked.
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        assert!(
            out.status.success(),
            "{backend} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // stderr too: the store-leak warning is written there, and it is one of
        // the things this test pins.  rustc's own warnings are dropped so the
        // two backends' outputs are comparable.
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        for line in String::from_utf8_lossy(&out.stderr).lines() {
            if line.starts_with("Warning:") {
                text.push_str(line);
                text.push('\n');
            }
        }
        text
    };

    let s = run("--interpret");
    // The fetch, and the retry after it. `same=true` is the claim that matters:
    // the second lookup answers the same RECORD, which it can only do because
    // the driver inserted into the collection and the collection was re-asked.
    assert!(s.contains("hit=row-7"), "the driver's row must arrive: {s}");
    assert!(
        s.contains("again=row-7 same=true"),
        "a repeat lookup is a resident HIT, and the same record: {s}"
    );
    // Exactly one driver line per MISS. The repeat of key 7 must not reach it.
    assert_eq!(
        s.matches("driver source=postgres://localhost/loft key=7")
            .count(),
        1,
        "a resident key must not re-enter the driver: {s}"
    );
    // Absence and unreachability are the same null and must never be the same
    // fact — the pair is what makes this cell non-vacuous.
    assert!(
        s.contains("absent=true err=[]"),
        "a genuine absence leaves the channel EMPTY: {s}"
    );
    assert!(
        s.contains(
            "unreachable=true err=[the postgres://localhost/loft server is not answering] faults=1"
        ),
        "an unreachable source reports the driver's own reason: {s}"
    );
    // A driver that FAULTS is contained. For an ordinary call propagating is
    // right; for a fetch it would turn a lookup into a program halt, which is a
    // regression against what @PLN129 already ships.
    assert!(
        s.contains("contained=true"),
        "a faulting driver must answer null, not halt: {s}"
    );
    assert!(
        s.contains("outer n=111 v=7,9"),
        "the outer frame must survive the contained fault intact: {s}"
    );
    assert!(
        s.contains("after=row-8 faults=2"),
        "a LATER fetch must still work — the machinery has to be usable, not \
         merely running: {s}"
    );

    // The releasing unwind: a contained fault leaves NOTHING behind. It used to
    // leak the aborted driver's allocation, one store per failed fetch — the
    // long-running case arc C's sticky counter exists for. The matrix that
    // decides the free is safe is a separate test below; this asserts the
    // headline on the same program that once demonstrated the leak.
    assert!(
        !s.contains("not freed"),
        "@PLN133 S8: a contained fault must leave nothing behind: {s}"
    );

    // One driver, two mechanisms, one answer — asserted whole. A backend that
    // installed the pointer but never called it, or that contained a fault
    // differently, would differ HERE and nowhere else.
    let n = run("--native");
    assert_eq!(
        s, n,
        "both backends must give the same answers about a loft-driven lazy fetch"
    );
}

/// @PLN133 S9 — a lazy driver serves ONE element type, and the miss finds it.
///
/// S8 let a program declare one `lazy_fetch`, and **nothing checked that the
/// driver a miss reached was declared for THAT collection**. So a program with a
/// second lazily-bound element type ran the first type's driver against it —
/// measured on both backends, inserting a `Person` into a `hash<Order[id]>` and
/// reading its `nm` straight back through `Order.what`'s offset. A plausible
/// wrong value, which is the class @PLN129 arc C exists to keep out.
///
/// One mechanism does both jobs: the driver is looked up by the collection's
/// ELEMENT TYPE, which makes several drivers possible and reaching the wrong one
/// impossible. What each serves is read off its collection parameter — the name
/// after `lazy_fetch_` is free, and exists only because loft refuses a
/// redefinition.
///
/// **The cell that would have been silent is `orphan`**: a bound collection whose
/// type no driver serves. It must now report, and the absence of a `driver=`
/// line before it is the proof that no driver ran — a value assertion alone would
/// pass on a driver that happened to answer nothing.
///
/// The two REFUSALS are separate programs because a refused driver set poisons
/// every lookup, which is the point: the set is one answer, so a program carrying
/// a broken one cannot be half-served.
#[test]
fn a_lazy_driver_serves_one_element_type_and_the_miss_finds_it() {
    let _serial = SQLITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let run = |fixture: &str, backend: &str| -> String {
        let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg(&script)
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        assert!(
            out.status.success(),
            "{fixture} {backend} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        for line in String::from_utf8_lossy(&out.stderr).lines() {
            if line.starts_with("Warning:") {
                text.push_str(line);
                text.push('\n');
            }
        }
        text
    };

    let s = run("133-lazy-driver-dispatch.loft", "--interpret");

    // Each collection reached ITS OWN driver. The values name the driver that
    // made them, so a dispatch that collapsed to one implementation reads as
    // `order person-9` here rather than as a count that happens to match.
    assert!(s.contains("person person-7"), "person driver: {s}");
    assert!(s.contains("order order-9"), "order driver: {s}");
    // …over an `index`, not only a `hash`: the element-type key is not a
    // hash-shaped trick.
    assert!(s.contains("ticket seat-3"), "index-collection driver: {s}");

    // THE cell. A bound collection whose element type no driver serves must
    // report, and must not reach somebody else's driver. The count is the proof:
    // three misses reach a driver and this one reaches none.
    assert!(
        s.contains("orphan <none>"),
        "a collection with no driver answers null: {s}"
    );
    assert!(
        s.contains("orphan-why `postgres://db/orphans` needs a loft driver for LddNoDriver"),
        "the refusal names the TYPE that has no driver: {s}"
    );
    assert_eq!(
        s.matches("driver=").count(),
        6,
        "exactly six driver calls — person 7, order 9, ticket 3, person 8, \
         order 400, order 66. The repeat of person 7 is a resident hit and the \
         orphan reaches NO driver, so neither is in the count: {s}"
    );
    assert!(
        !s.contains("orphan orphan") && !s.contains("orphan person"),
        "the orphan must never be filled by another type's driver: {s}"
    );

    // A resident key does not re-enter the driver, and a neighbour with no driver
    // does not disturb one that has.
    assert_eq!(
        s.matches("driver=person source=postgres://db/people key=7")
            .count(),
        1,
        "a resident key must not re-enter the driver: {s}"
    );
    assert!(
        s.contains("after person-8"),
        "a later fetch still works: {s}"
    );

    // Absence and unreachability are the same null and must never be the same
    // fact. The pair is what makes the channel non-vacuous.
    assert!(
        s.contains("absent <none> why=\n"),
        "a genuine absence leaves the channel EMPTY: {s}"
    );
    assert!(
        s.contains("unreachable <none> why=the duckdb:orders.db order service is not answering"),
        "an unreachable source reports the driver's own reason: {s}"
    );

    // A helper sharing the drivers' name prefix is an ordinary function. Under a
    // name-only membership rule it was read as a malformed driver and poisoned
    // every lookup above, which is the cost that made the rule wrong.
    assert!(s.contains("helper label-4"), "the helper still runs: {s}");
    assert!(
        !s.contains("not freed"),
        "the dispatch must leave nothing behind: {s}"
    );

    let n = run("133-lazy-driver-dispatch.loft", "--native");
    assert_eq!(
        s, n,
        "both backends must reach the same driver for the same collection"
    );

    // --- the refusals ------------------------------------------------------
    //
    // Byte-identical wording is not automatic here and is the thing being
    // gated: the interpreter asks `Data` at every miss, while `--native` cannot
    // ask at all and has the refusal installed as data by generated `init()`.
    // Without that, the same program named a different mistake depending on
    // which backend you ran.
    let dup = run("133-lazy-driver-refused.loft", "--interpret");
    assert!(
        dup.contains("why `lazy_fetch` and `lazy_fetch_also` are both lazy drivers for LdrThing"),
        "two drivers for one type are refused, naming BOTH: {dup}"
    );
    assert!(
        dup.contains("value <none>"),
        "and neither of them runs: {dup}"
    );
    assert_eq!(
        dup,
        run("133-lazy-driver-refused.loft", "--native"),
        "both backends must word the duplicate-driver refusal identically"
    );

    let bad = run("133-lazy-driver-malformed.loft", "--interpret");
    assert!(
        bad.contains("why `lazy_fetch` takes 3 parameter(s)"),
        "THE driver name with a wrong shape says what is wrong with IT, not \
         that there is no driver: {bad}"
    );
    assert_eq!(
        bad,
        run("133-lazy-driver-malformed.loft", "--native"),
        "both backends must word the malformed-driver refusal identically"
    );
}

/// @PLN133 S8 — the releasing unwind, and the matrix that decides it is safe.
///
/// A raise in loft short-circuits the dispatch loop, so the scope-exit frees the
/// compiler emitted never run. For a program about to EXIT that is harmless, and
/// the suite grandfathers it. For a CONTAINED fault it is not: the program
/// continues, so a traversal over an unreachable source leaked once per failed
/// fetch — unbounded.
///
/// The fix frees the stores the driver CREATED during the call it faulted out
/// of, and the whole question is whether anything outside its abandoned frames
/// can still reach one. Every cell in the fixture tries to leave something
/// reachable behind before it faults — an inserted row, a text built during the
/// call, a filled vector, a value from a second frame, two rows either side of
/// an allocation — and then reads it back, late, after fifty more faults.
///
/// **Reading a value back is not proof on its own**: a freed store can read as
/// plausible garbage. So the run also asserts the collection's heap verifies and
/// that nothing is left unfreed.
///
/// Two break-checks were run against this matrix. Removing the free brings the
/// leak straight back (128 stores across 57 faults). Freeing the candidates on
/// the SUCCESS path too changes nothing — which is the stronger half of the
/// safety argument: even where every store the driver produced is definitely
/// still needed, none of them is a candidate, because an insert COPIES into the
/// collection's store and the driver's new stores are only ever its own locals.
#[test]
fn a_contained_driver_fault_releases_what_it_held() {
    let _serial = SQLITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/133-lazy-unwind.loft");
    let run = |backend: &str| -> String {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg(&script)
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .env("LOFT_STORES", "warn")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to invoke loft");
        assert!(
            out.status.success(),
            "{backend} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        for line in String::from_utf8_lossy(&out.stderr).lines() {
            if line.starts_with("Warning:") {
                text.push_str(line);
                text.push('\n');
            }
        }
        text
    };

    let expect = [
        "1 null=true",
        // Each of these faulted having left a row in the collection, and the row
        // survived with its content intact — the text built during the call, the
        // vector filled during it, the value that came from a second frame.
        "2 during=true after=kept-2",
        "3 during=true after=built-3-from-duckdb:unwind",
        "4 during=true after=vec-4/alpha/beta/gamma",
        "5 during=true after=made-5",
        "6 during=true after=six-a sibling=six-b",
        "8 null=true",
        "9 ok-9",
        // …and still intact fifty faults later, which is where a wrong free
        // surfaces if it did not surface at the cell.
        "late 2=kept-2 3=built-3-from-duckdb:unwind 4=vec-4/alpha/beta/gamma 5=made-5",
        "late 6=six-a 7=six-b 9=ok-9",
        "resident=7 faults=57",
        // A value can read right out of a freed store; a heap check cannot.
        "sound=true",
    ];

    let s = run("--interpret");
    for line in expect {
        assert!(s.contains(line), "--interpret: expected `{line}` in:\n{s}");
    }
    // The headline: 57 contained faults, nothing left behind.
    assert!(
        !s.contains("not freed"),
        "57 contained faults must leave NOTHING behind:\n{s}"
    );

    // One answer, two mechanisms — the interpreter stops its dispatch loop, and
    // native unwinds through Rust. Compared whole, because the way they differ
    // is exactly where an unpredicted divergence would hide.
    let n = run("--native");
    assert_eq!(
        s, n,
        "both backends must release identically after a contained fault"
    );
}
