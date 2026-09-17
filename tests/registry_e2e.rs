// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end integration test for the PKG.REG file-based registry.
//!
//! Sets up a tiny in-process HTTP fixture server, hosts an
//! `index.json` + a real package tarball produced by
//! `loft::package::package_create`, then exercises the full
//! `loft::install::install_one` pipeline against the fixture URL.
//! Verifies:
//!
//! - the tarball is downloaded byte-for-byte;
//! - sha256 is checked (corruption causes hard failure);
//! - the package extracts to `~/.loft/registry/<pkg>-<v>/`;
//! - `loft.lock` is written with the expected fields;
//! - `extract_tarball` produces a directory tree matching the
//!   original `package_create` input (the missing roundtrip test).
//!
//! Test isolation: each test points `HOME` at a fresh tmpdir so the
//! global cache (`~/.loft/registry`) doesn't leak between tests or
//! collide with the developer's real cache.  The fixture HTTP server
//! binds `127.0.0.1:0` (kernel-picked port) so parallel test runs
//! don't conflict.

#![cfg(feature = "registry")]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[path = "common/mod.rs"]
mod common;

// ── HTTP fixture server ───────────────────────────────────────────

/// Tiny single-threaded HTTP server.  Serves a static map of
/// `path → bytes`.  Returns 404 on unknown paths.  Shuts down via
/// `Drop` on the handle.  ~120 lines, no external HTTP framework
/// (the project's stdlib is enough for a 1xx-200 mock server).
struct FixtureServer {
    base_url: String,
    shutdown: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl FixtureServer {
    fn new(initial: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let local = listener.local_addr().expect("addr");
        let base_url = format!("http://{}", local);
        let shutdown = Arc::new(AtomicBool::new(false));
        let files = Arc::new(Mutex::new(initial));
        let shutdown_clone = Arc::clone(&shutdown);
        let files_clone = Arc::clone(&files);
        let join = thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let f = Arc::clone(&files_clone);
                        thread::spawn(move || {
                            let map = f.lock().unwrap().clone();
                            handle_request(stream, &map);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url,
            shutdown,
            join: Some(join),
            files,
        }
    }

    fn url_for(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn handle_request(mut stream: TcpStream, files: &HashMap<String, Vec<u8>>) {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    // Drain headers.
    let mut hdr = String::new();
    loop {
        hdr.clear();
        if reader.read_line(&mut hdr).is_err() {
            return;
        }
        if hdr == "\r\n" || hdr == "\n" || hdr.is_empty() {
            break;
        }
    }
    let response: Vec<u8> = if let Some(data) = files.get(&path) {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
            data.len()
        )
        .into_bytes();
        r.extend_from_slice(data);
        r
    } else {
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found"
            .to_vec()
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

// ── Test helpers ──────────────────────────────────────────────────

/// Per-test tmpdir; the test sets `HOME=<tmpdir>` so the install
/// cache lives there instead of the developer's real `~/.loft/`.
fn tmpdir(name: &str) -> PathBuf {
    let mut p = env::temp_dir();
    p.push(format!("loft_e2e_{}_{}", std::process::id(), name));
    if p.exists() {
        let _ = fs::remove_dir_all(&p);
    }
    fs::create_dir_all(&p).unwrap();
    p
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Build a sample loft package on disk and return its path.
fn make_sample_package(name: &str, version: &str, tmp: &Path) -> PathBuf {
    let pkg = tmp.join("pkg_src");
    write_file(
        &pkg.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/{name}.loft\"\n"
        ),
    );
    write_file(
        &pkg.join("src").join(format!("{name}.loft")),
        &format!("// {name} source\npub fn id() -> text {{ return \"{name}-{version}\"; }}\n"),
    );
    write_file(&pkg.join("README.md"), &format!("# {name}\n"));
    pkg
}

/// Atomically swap HOME + LOFT_HOME for the duration of the test.  Returns a
/// guard that restores the previous values on drop.  Tests in this
/// file share a process so we serialise via a Mutex — install
/// resolution reads `registry_index::cache_dir()` once per call so a short
/// window of mutated env is enough.
///
/// @P332: we set `LOFT_HOME` (not just `HOME`), because `cache_dir()` reads
/// `LOFT_HOME` first and `dirs::home_dir()` ignores `$HOME` on Windows.
/// Setting both keeps the isolation cross-platform.
struct HomeGuard {
    prev_home: Option<String>,
    prev_loft_home: Option<String>,
}
static HOME_LOCK: Mutex<()> = Mutex::new(());

impl HomeGuard {
    fn set(new_home: &Path) -> (Self, std::sync::MutexGuard<'static, ()>) {
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_home = env::var("HOME").ok();
        let prev_loft_home = env::var("LOFT_HOME").ok();
        // SAFETY: env::set_var is documented unsafe in newer Rust due to
        // process-wide effects; we serialise via HOME_LOCK so only one
        // test mutates the env at a time.
        unsafe {
            env::set_var("HOME", new_home);
            env::set_var("LOFT_HOME", new_home);
        }
        (
            Self {
                prev_home,
                prev_loft_home,
            },
            lock,
        )
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.prev_home {
                Some(p) => env::set_var("HOME", p),
                None => env::remove_var("HOME"),
            }
            match &self.prev_loft_home {
                Some(p) => env::set_var("LOFT_HOME", p),
                None => env::remove_var("LOFT_HOME"),
            }
        }
    }
}

/// Atomically swap LOFT_REGISTRY_URL.
struct RegUrlGuard {
    prev: Option<String>,
}
static REG_URL_LOCK: Mutex<()> = Mutex::new(());

impl RegUrlGuard {
    fn set(new_url: &str) -> (Self, std::sync::MutexGuard<'static, ()>) {
        let lock = REG_URL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = env::var("LOFT_REGISTRY_URL").ok();
        unsafe {
            env::set_var("LOFT_REGISTRY_URL", new_url);
        }
        (Self { prev }, lock)
    }
}

impl Drop for RegUrlGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(p) = &self.prev {
                env::set_var("LOFT_REGISTRY_URL", p);
            } else {
                env::remove_var("LOFT_REGISTRY_URL");
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────

// @P332 FIXED 2026-05-26: previously `report.installed.len()` returned 0 on
// Windows because the test isolated the registry via `HOME`, which
// `dirs::home_dir()` ignores on Windows — installs leaked into the real
// profile and cross-run caching skipped them.  `cache_dir()` now honours
// `LOFT_HOME` (set by `HomeGuard` alongside `HOME`), so isolation is
// cross-platform and the Windows gate is removed.
#[test]
fn end_to_end_install_against_fixture_server() {
    let tmp = tmpdir("end_to_end_install_against_fixture_server");

    // 1. Build a real package tarball.
    let pkg_dir = make_sample_package("e2e", "0.1.0", &tmp);
    let pkg_out = loft::package::package_create(&pkg_dir, Some(&tmp)).expect("package_create");
    let tarball_bytes = fs::read(&pkg_out.tarball).expect("read tarball");

    // 2. Stand up the fixture server with index.json + tarball.
    let mut files = HashMap::new();
    // Fill the index AFTER we know the server URL, so the entries
    // can point at the right host.  Start the server with an empty
    // file map, then update it.
    files.insert("/placeholder".to_string(), b"placeholder".to_vec());
    let server = FixtureServer::new(files);

    let tarball_url = server.url_for("/e2e-0.1.0.tar.gz");
    let index_json = format!(
        r#"{{
            "schema_version": 1,
            "updated": "2026-05-24T00:00:00Z",
            "packages": {{
                "e2e": {{
                    "description": "End-to-end test fixture",
                    "categories": ["test"],
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#,
        tarball_url, pkg_out.sha256, pkg_out.size
    );

    {
        let mut map = server.files.lock().unwrap();
        map.insert("/index.json".to_string(), index_json.clone().into_bytes());
        map.insert("/e2e-0.1.0.tar.gz".to_string(), tarball_bytes.clone());
    }

    // 3. Point loft at the fixture.
    let home_dir = tmpdir("end_to_end_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));

    // 4. Run the install.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true, // skip TTL on first run
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    // `loft.lock` is written into cwd — temporarily move to a fresh
    // dir so we don't dirty the dev's working tree.
    let prev_cwd = env::current_dir().expect("cwd");
    let install_cwd = tmpdir("end_to_end_install_cwd");
    env::set_current_dir(&install_cwd).expect("chdir");

    let report = loft::install::install_one("e2e", None, &opts).expect("install_one");
    assert_eq!(report.installed.len(), 1);
    assert_eq!(
        report.installed[0],
        ("e2e".to_string(), "0.1.0".to_string())
    );

    // 5. Cache dir contains the extracted package.
    let extracted = home_dir.join(".loft").join("registry").join("e2e-0.1.0");
    assert!(extracted.exists(), "extracted dir missing: {extracted:?}");
    assert!(extracted.join("loft.toml").exists(), "loft.toml missing");
    assert!(
        extracted.join("src").join("e2e.loft").exists(),
        "src missing"
    );
    assert!(extracted.join("README.md").exists(), "README missing");

    // 6. Extracted contents match the original source.
    let original_src =
        fs::read_to_string(pkg_dir.join("src").join("e2e.loft")).expect("read original src");
    let extracted_src =
        fs::read_to_string(extracted.join("src").join("e2e.loft")).expect("read extracted src");
    assert_eq!(
        original_src, extracted_src,
        "extracted source bytes differ from original"
    );

    // 7. loft.lock written with the expected fields.
    let lock_path = install_cwd.join("loft.lock");
    let lock = loft::lockfile::read_lockfile(&lock_path)
        .expect("io")
        .expect("Some");
    assert_eq!(lock.packages.len(), 1);
    let p = &lock.packages[0];
    assert_eq!(p.name, "e2e");
    assert_eq!(p.version, "0.1.0");
    assert_eq!(p.url, tarball_url);
    assert_eq!(p.sha256, pkg_out.sha256);
    assert_eq!(p.source, "registry");

    // 8. Re-running install is idempotent — re-uses cache.
    let report2 = loft::install::install_one("e2e", None, &opts).expect("install_one #2");
    assert_eq!(report2.installed.len(), 0);
    assert_eq!(report2.skipped_cached.len(), 1);

    // Restore cwd before tmpdir cleanup.
    env::set_current_dir(prev_cwd).ok();

    // Cleanup.
    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
    let _ = fs::remove_dir_all(&install_cwd);
}

/// A `skip_lockfile` install lands the package but writes NO lockfile — the mode
/// auto-install uses when a transitive dep is discovered while parsing a file
/// already inside the registry cache (there is no consumer project to record, and
/// the walk-up would otherwise write `loft.lock` into the immutable cache: a stray
/// file on Unix, an ENOENT that aborts resolution on Windows). The install itself
/// must still succeed so `use <dep>` resolves.
#[test]
fn skip_lockfile_installs_without_writing_a_lockfile() {
    let tmp = tmpdir("skip_lockfile_installs_without_writing_a_lockfile");

    let pkg_dir = make_sample_package("skiplock", "0.1.0", &tmp);
    let pkg_out = loft::package::package_create(&pkg_dir, Some(&tmp)).expect("package_create");
    let tarball_bytes = fs::read(&pkg_out.tarball).expect("read tarball");

    let mut files = HashMap::new();
    files.insert("/placeholder".to_string(), b"placeholder".to_vec());
    let server = FixtureServer::new(files);
    let tarball_url = server.url_for("/skiplock-0.1.0.tar.gz");
    let index_json = format!(
        r#"{{
            "schema_version": 1,
            "updated": "2026-05-24T00:00:00Z",
            "packages": {{
                "skiplock": {{
                    "description": "skip-lockfile fixture",
                    "categories": ["test"],
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#,
        tarball_url, pkg_out.sha256, pkg_out.size
    );
    {
        let mut map = server.files.lock().unwrap();
        map.insert("/index.json".to_string(), index_json.into_bytes());
        map.insert("/skiplock-0.1.0.tar.gz".to_string(), tarball_bytes);
    }

    let home_dir = tmpdir("skip_lockfile_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));

    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: true,
        lock_path: None,
    };
    let prev_cwd = env::current_dir().expect("cwd");
    let install_cwd = tmpdir("skip_lockfile_install_cwd");
    env::set_current_dir(&install_cwd).expect("chdir");

    let report = loft::install::install_one("skiplock", None, &opts).expect("install_one");

    // Restore cwd before any assertion can unwind.
    env::set_current_dir(&prev_cwd).ok();

    // The package installed…
    assert_eq!(report.installed.len(), 1, "package should install");
    assert!(
        home_dir
            .join(".loft/registry/skiplock-0.1.0/loft.toml")
            .exists(),
        "extracted package missing"
    );
    // …but NO lockfile was written, in cwd or anywhere the default path would land.
    assert!(
        !install_cwd.join("loft.lock").exists(),
        "skip_lockfile must not write a lockfile"
    );

    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
    let _ = fs::remove_dir_all(&install_cwd);
}

#[test]
fn install_rejects_tarball_with_wrong_sha256() {
    let tmp = tmpdir("install_rejects_tarball_with_wrong_sha256");

    // Build a tarball but advertise a WRONG sha256 in the index.
    let pkg_dir = make_sample_package("badsha", "0.1.0", &tmp);
    let pkg_out = loft::package::package_create(&pkg_dir, Some(&tmp)).expect("package_create");
    let tarball_bytes = fs::read(&pkg_out.tarball).expect("read");

    let mut files = HashMap::new();
    files.insert("/badsha-0.1.0.tar.gz".to_string(), tarball_bytes);
    let server = FixtureServer::new(files);
    let tarball_url = server.url_for("/badsha-0.1.0.tar.gz");

    let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";
    let index_json = format!(
        r#"{{
            "schema_version": 1,
            "updated": "2026-05-24T00:00:00Z",
            "packages": {{
                "badsha": {{
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#,
        tarball_url, wrong_sha, pkg_out.size
    );
    server
        .files
        .lock()
        .unwrap()
        .insert("/index.json".to_string(), index_json.into_bytes());

    let home_dir = tmpdir("install_rejects_bad_sha_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));

    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let prev_cwd = env::current_dir().expect("cwd");
    let install_cwd = tmpdir("install_rejects_bad_sha_cwd");
    env::set_current_dir(&install_cwd).expect("chdir");

    let result = loft::install::install_one("badsha", None, &opts);
    assert!(result.is_err(), "install should reject mismatched sha256");
    let err = result.unwrap_err();
    assert!(err.contains("sha256 mismatch"), "msg: {err}");

    env::set_current_dir(prev_cwd).ok();
    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
    let _ = fs::remove_dir_all(&install_cwd);
}

#[test]
fn install_rejects_missing_package_in_index() {
    let tmp = tmpdir("install_rejects_missing_package");

    let mut files = HashMap::new();
    let index_json = r#"{
        "schema_version": 1,
        "updated": "2026-05-24T00:00:00Z",
        "packages": {}
    }"#;
    files.insert("/index.json".to_string(), index_json.as_bytes().to_vec());
    let server = FixtureServer::new(files);

    let home_dir = tmpdir("install_missing_pkg_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));

    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let prev_cwd = env::current_dir().expect("cwd");
    let install_cwd = tmpdir("install_missing_pkg_cwd");
    env::set_current_dir(&install_cwd).expect("chdir");

    let result = loft::install::install_one("does-not-exist", None, &opts);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("not found in registry"), "msg: {err}");

    env::set_current_dir(prev_cwd).ok();
    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
    let _ = fs::remove_dir_all(&install_cwd);
}

/// Several processes resolving one `use <pkg>` at once — a server and its clients, or
/// nextest's one process per test — each install the same package into one cache.  The
/// scratch names they share (the tarball beside the cache, `loft.lock.tmp`) let one
/// install delete or rename away the other's file, and an extraction over a directory
/// another install is still filling could be read half-written.  Every install must
/// succeed, and what lands must be whole.
#[test]
fn concurrent_installs_of_one_package_all_succeed() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 6;
    let tmp = tmpdir("concurrent_installs");
    let pkg_dir = make_sample_package("race", "0.1.0", &tmp);
    let pkg_out = loft::package::package_create(&pkg_dir, Some(&tmp)).expect("package_create");
    let tarball_bytes = fs::read(&pkg_out.tarball).expect("read tarball");
    let original_src = fs::read_to_string(pkg_dir.join("src").join("race.loft")).expect("src");

    let mut files = HashMap::new();
    files.insert("/placeholder".to_string(), b"placeholder".to_vec());
    let server = FixtureServer::new(files);
    let index_json = format!(
        r#"{{
            "schema_version": 1,
            "updated": "2026-05-24T00:00:00Z",
            "packages": {{
                "race": {{
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#,
        server.url_for("/race-0.1.0.tar.gz"),
        pkg_out.sha256,
        pkg_out.size
    );
    {
        let mut map = server.files.lock().unwrap();
        map.insert("/index.json".to_string(), index_json.into_bytes());
        map.insert("/race-0.1.0.tar.gz".to_string(), tarball_bytes);
    }
    for round in 0..ROUNDS {
        let home_dir = tmpdir(&format!("concurrent_installs_home_{round}"));
        // Home first, then the URL: the order every test in this file takes the two locks.
        let (_home, _lh) = HomeGuard::set(&home_dir);
        let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));
        let lock_path = home_dir.join("project").join("loft.lock");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(THREADS));
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let lock_path = lock_path.clone();
                thread::spawn(move || {
                    let opts = loft::install::InstallOptions {
                        allow_unsigned: true,
                        refresh: true,
                        offline: false,
                        allow_prerelease: false,
                        skip_lockfile: false,
                        lock_path: Some(lock_path),
                    };
                    barrier.wait();
                    loft::install::install_one("race", None, &opts)
                })
            })
            .collect();
        let mut placed = 0;
        for w in workers {
            let report = w
                .join()
                .expect("install thread panicked")
                .unwrap_or_else(|e| panic!("round {round}: a concurrent install failed: {e}"));
            assert_eq!(
                report.installed.len() + report.skipped_cached.len(),
                1,
                "round {round}: each install accounts for the one package"
            );
            placed += report.installed.len();
        }
        assert!(placed >= 1, "round {round}: nobody installed the package");
        let extracted = home_dir.join(".loft/registry/race-0.1.0");
        assert_eq!(
            fs::read_to_string(extracted.join("src").join("race.loft")).expect("extracted src"),
            original_src,
            "round {round}: the extracted source is whole"
        );
        let lock = loft::lockfile::read_lockfile(&lock_path)
            .expect("read lockfile")
            .expect("a lockfile was written");
        assert_eq!(lock.packages.len(), 1, "round {round}: one locked package");
        assert_eq!(lock.packages[0].name, "race");
        let strays: Vec<_> = fs::read_dir(home_dir.join(".loft/registry"))
            .unwrap()
            .chain(fs::read_dir(lock_path.parent().unwrap()).unwrap())
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp") || n.ends_with(".tar.gz") || n.starts_with(".staging"))
            .collect();
        assert!(
            strays.is_empty(),
            "round {round}: scratch left behind: {strays:?}"
        );
        drop(_reg);
        drop(_home);
        let _ = fs::remove_dir_all(&home_dir);
    }
    let _ = fs::remove_dir_all(&tmp);
}

// ── Tarball extract roundtrip ─────────────────────────────────────

/// `loft package` → `extract_tarball` → directory tree matches the
/// original.  The missing roundtrip test called out in the audit.
#[test]
fn extract_tarball_roundtrip_matches_original() {
    let tmp = tmpdir("extract_tarball_roundtrip");

    let pkg_dir = make_sample_package("rt", "0.1.0", &tmp);
    let pkg_out = loft::package::package_create(&pkg_dir, Some(&tmp)).expect("package_create");

    let extract_root = tmp.join("extracted");
    loft::registry_index::extract_tarball(&pkg_out.tarball, &extract_root)
        .expect("extract_tarball");

    let inside = extract_root.join("rt-0.1.0");
    assert!(inside.exists(), "extracted root missing");
    assert!(inside.join("loft.toml").exists(), "loft.toml missing");
    assert!(inside.join("src").join("rt.loft").exists(), "src missing");
    assert!(inside.join("README.md").exists(), "README missing");

    let original = fs::read(pkg_dir.join("src").join("rt.loft")).unwrap();
    let extracted = fs::read(inside.join("src").join("rt.loft")).unwrap();
    assert_eq!(original, extracted, "src bytes differ after roundtrip");

    let _ = fs::remove_dir_all(&tmp);
}

/// Transitive install: a depends on b, both in the fixture
/// registry.  Verify the resolver+download pipeline walks deps and
/// extracts both packages.
// @P332 FIXED 2026-05-26 — see `end_to_end_install_against_fixture_server`
// (the `LOFT_HOME` cross-platform isolation fix); Windows gate removed.
#[test]
fn end_to_end_install_with_transitive_dep() {
    let tmp = tmpdir("end_to_end_install_with_transitive_dep");

    // Build two packages — a (depends on b) and b.
    let a_src_root = tmp.join("a_src");
    fs::create_dir_all(&a_src_root).unwrap();
    let a_dir = make_sample_package("ta", "0.1.0", &a_src_root);
    let a_out_dir = tmp.join("a_out");
    fs::create_dir_all(&a_out_dir).unwrap();
    let a_out = loft::package::package_create(&a_dir, Some(&a_out_dir)).expect("a pkg");
    let b_src_root = tmp.join("b_src");
    fs::create_dir_all(&b_src_root).unwrap();
    let b_dir = make_sample_package("tb", "0.1.0", &b_src_root);
    let b_out_dir = tmp.join("b_out");
    fs::create_dir_all(&b_out_dir).unwrap();
    let b_out = loft::package::package_create(&b_dir, Some(&b_out_dir)).expect("b pkg");

    let a_bytes = fs::read(&a_out.tarball).unwrap();
    let b_bytes = fs::read(&b_out.tarball).unwrap();

    let mut files = HashMap::new();
    files.insert("/ta-0.1.0.tar.gz".to_string(), a_bytes);
    files.insert("/tb-0.1.0.tar.gz".to_string(), b_bytes);
    let server = FixtureServer::new(files);

    let index_json = format!(
        r#"{{
            "schema_version": 1,
            "updated": "2026-05-24T00:00:00Z",
            "packages": {{
                "ta": {{
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "deps": {{"tb": "^0.1"}},
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }},
                "tb": {{
                    "versions": {{
                        "0.1.0": {{
                            "url": "{}",
                            "sha256": "{}",
                            "size": {},
                            "loft": ">=0.8",
                            "published": "2026-05-24T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#,
        server.url_for("/ta-0.1.0.tar.gz"),
        a_out.sha256,
        a_out.size,
        server.url_for("/tb-0.1.0.tar.gz"),
        b_out.sha256,
        b_out.size,
    );
    server
        .files
        .lock()
        .unwrap()
        .insert("/index.json".to_string(), index_json.into_bytes());

    let home_dir = tmpdir("end_to_end_transitive_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));

    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let prev_cwd = env::current_dir().expect("cwd");
    let install_cwd = tmpdir("end_to_end_transitive_cwd");
    env::set_current_dir(&install_cwd).expect("chdir");

    let report = loft::install::install_one("ta", None, &opts).expect("install_one");
    // Both ta and tb should be installed.
    let names: Vec<&str> = report.installed.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"ta"), "ta missing from report: {names:?}");
    assert!(names.contains(&"tb"), "tb missing from report: {names:?}");

    let reg = home_dir.join(".loft").join("registry");
    assert!(reg.join("ta-0.1.0").exists());
    assert!(reg.join("tb-0.1.0").exists());

    // Lockfile has both with the dep edge recorded.
    let lock = loft::lockfile::read_lockfile(&install_cwd.join("loft.lock"))
        .expect("io")
        .expect("Some");
    assert_eq!(lock.packages.len(), 2);
    let ta_lock = lock.packages.iter().find(|p| p.name == "ta").expect("ta");
    assert_eq!(ta_lock.deps, vec!["tb"]);

    env::set_current_dir(prev_cwd).ok();
    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
    let _ = fs::remove_dir_all(&install_cwd);
}

// ── loft search S4 — offline cache fallback ───────────────────────

/// PKG.REG R8 (search S4): when the registry is unreachable but a cached index
/// exists, the read-only discovery loader serves the cache and flags it
/// (`stale_fallback`), while the install-path loader still surfaces the error
/// — the contained-blast-radius invariant for the fallback.
#[test]
fn load_index_reporting_falls_back_to_cache_when_registry_unreachable() {
    let tmp = tmpdir("search_offline_fallback");

    let index_json = r#"{
        "schema_version": 1,
        "updated": "2026-06-19T00:00:00Z",
        "packages": {
            "regex": {
                "description": "Regular expressions",
                "categories": ["text"],
                "versions": {
                    "0.2.0": {
                        "url": "https://example.com/regex-0.2.0.tar.gz",
                        "sha256": "abc",
                        "size": 1,
                        "loft": ">=0.8",
                        "triggers": ["matches:text"],
                        "published": "2026-06-19T00:00:00Z"
                    }
                }
            }
        }
    }"#;

    let mut files = HashMap::new();
    files.insert("/index.json".to_string(), index_json.as_bytes().to_vec());
    let server = FixtureServer::new(files);

    let home_dir = tmpdir("search_offline_home");
    let (_home, _lh) = HomeGuard::set(&home_dir);

    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true, // force a fetch attempt each call (bypass the TTL)
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };

    // 1. Reachable registry → fresh fetch primes the cache; not a fallback.
    {
        let (_reg, _lr) = RegUrlGuard::set(&server.url_for("/index.json"));
        let loaded = loft::install::load_index_reporting(&opts).expect("fresh fetch");
        assert!(!loaded.stale_fallback, "a fresh fetch is not a fallback");
        assert!(loaded.index.packages.contains_key("regex"));
    }

    // 2. Unreachable registry (dead port) + refresh → fall back to the primed
    //    cache, flagged stale.
    {
        let (_reg, _lr) = RegUrlGuard::set("http://127.0.0.1:1/index.json");
        let loaded =
            loft::install::load_index_reporting(&opts).expect("cache fallback on fetch failure");
        assert!(
            loaded.stale_fallback,
            "unreachable registry + primed cache must report stale_fallback"
        );
        assert!(loaded.index.packages.contains_key("regex"));

        // The install-path loader must NOT silently use the stale cache.
        assert!(
            loft::install::load_index(&opts).is_err(),
            "load_index (install path) must surface the fetch error, not fall back"
        );
    }

    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);
}

/// @PLN143 — a pinned script's sidecar decides which version is INSTALLED, and the run
/// does not amend it.
///
/// The two halves were one field before: `lock_path` meant both "the lock that governs"
/// and "the lock to write", so arc C2 — which stopped a run from writing — also stopped
/// it from reading. A sidecar pinning 0.1.0 then loaded 0.1.0 when the cache already had
/// it and INSTALLED the newest when it did not, so a fresh box quietly ran a different
/// program than the machine that pinned it.
///
/// Both versions are published here, so "it installed the pin" cannot pass by there
/// being nothing else to resolve to. `file://` serves the index and both tarballs — the
/// full path runs (resolve → download → sha256 → extract) with no socket.
#[test]
fn a_sidecar_pin_decides_the_install_and_is_not_rewritten() {
    let tmp = tmpdir("a_sidecar_pin_decides_the_install");
    let home_dir = tmpdir("a_sidecar_pin_home");

    let mut entries = Vec::new();
    let mut shas = Vec::new();
    for version in ["0.1.0", "0.2.0"] {
        let src = tmp.join(format!("src-{version}"));
        write_file(
            &src.join("loft.toml"),
            &format!(
                "[package]\nname = \"pinpkg\"\nversion = \"{version}\"\nloft = \">=0.8\"\n\n\
                 [library]\nentry = \"src/pinpkg.loft\"\n"
            ),
        );
        write_file(
            &src.join("src/pinpkg.loft"),
            &format!("pub fn id() -> text {{ return \"pinpkg-{version}\"; }}\n"),
        );
        let out = loft::package::package_create(&src, Some(&tmp)).expect("package_create");
        shas.push(out.sha256.clone());
        entries.push(format!(
            r#""{version}":{{"url":"{}","sha256":"{}","size":{},"loft":">=0.8",
               "published":"2026-08-18T00:00:00Z"}}"#,
            common::file_url(&out.tarball),
            out.sha256,
            out.size
        ));
    }
    let index = tmp.join("index.json");
    write_file(
        &index,
        &format!(
            r#"{{"schema_version":1,"packages":{{"pinpkg":{{"description":"pin probe",
               "versions":{{{}}}}}}}}}"#,
            entries.join(",")
        ),
    );

    let (_home, _lh) = HomeGuard::set(&home_dir);
    let (_reg, _lr) = RegUrlGuard::set(&common::file_url(&index));

    // The sidecar `loft pin` would have written: the older version, and the file a run
    // must leave exactly as it found it.
    let sidecar = tmp.join("s.loft.lock");
    // The real sha256 of the 0.1.0 tarball: a lockfile pins the BYTES as well as the
    // version, and reading the sidecar means that check now covers a pinned script too —
    // a wrong hash here is refused, which the cell below pins.
    let sidecar_body = format!(
        "schema_version = 1\n\n[[package]]\nname = \"pinpkg\"\nversion = \"0.1.0\"\n\
         url = \"file:///pinpkg-0.1.0.tar.gz\"\nsha256 = \"{}\"\nsource = \"registry\"\n",
        shas[0]
    );
    write_file(&sidecar, &sidecar_body);

    // Exactly what the parser hands auto-install in `PinnedScript` scope: the sidecar
    // governs (`lock_path`), a run may not declare (`skip_lockfile`), and the constraint
    // is the version that lock pins.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: true,
        lock_path: Some(sidecar.clone()),
        skip_lockfile: true,
        ..Default::default()
    };
    let pin = loft::install::constraint_for(Some("0.1.0"), None);
    let report = loft::install::install_one("pinpkg", pin.as_deref(), &opts).expect("install_one");

    let installed: Vec<String> = report
        .installed
        .iter()
        .chain(report.skipped_cached.iter())
        .map(|(n, v)| format!("{n} {v}"))
        .collect();
    let after = fs::read_to_string(&sidecar).expect("read sidecar");
    let newest_extracted = home_dir.join(".loft/registry/pinpkg-0.2.0").exists();

    drop(_reg);
    drop(_home);
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&home_dir);

    assert_eq!(
        installed,
        vec!["pinpkg 0.1.0".to_string()],
        "the sidecar pins 0.1.0 and 0.2.0 is published — the pin decides (@PLN143)"
    );
    assert!(
        !newest_extracted,
        "and 0.2.0 must not be fetched at all: a pin that installs the newest is not a pin"
    );
    assert_eq!(
        after, sidecar_body,
        "a run reads the declaration and never rewrites it"
    );
}

// ── loft#1048 — the advisories feed goes through the same trust gate as the index ──
//
// Two holes that `install.rs` had closed for the registry index and that were never
// carried across: the feed was written to the cache BEFORE its signature was checked,
// so a refused feed outlived the run that fetched it; and `--offline` read the cache
// without verifying at all, which put the whole gate behind one flag.

/// A minimal well-formed feed, so a test that reaches parsing gets past it and one
/// that must NOT reach parsing is failing for the reason it claims.
fn advisory_feed() -> &'static str {
    r#"{
  "schema_version": 1,
  "updated": "2026-08-20T12:00:00Z",
  "retention_days": 90,
  "advisories": []
}"#
}

/// `--offline` verifies the cached feed like every other branch.
///
/// Before the fix this branch read the cache and returned it unchecked, so a feed
/// planted in `~/.loft/registry` was trusted on the word of the flag alone.  The
/// assertion is on the REFUSAL: with no `allow_unsigned`, an unverifiable signature
/// must stop the load rather than produce advisories.
#[test]
fn offline_does_not_trust_the_cached_advisory_feed_unchecked() {
    let home = tmpdir("1048-offline-verify");
    let (_guard, _lock) = HomeGuard::set(&home);

    let cache = home.join(".loft").join("registry");
    fs::create_dir_all(&cache).expect("cache dir");
    write_file(&cache.join("advisories.json"), advisory_feed());
    // 64 bytes that are the right SHAPE for a signature and verify against nothing.
    write_file(&cache.join("advisories.json.sig"), &"a1".repeat(32));

    let outcome =
        loft::registry_advisories::load_or_fetch(&loft::registry_advisories::LoadOptions {
            allow_unsigned: false,
            offline: true,
            refresh: false,
        });

    assert!(
        outcome.is_err(),
        "offline returned {outcome:?} for a feed whose signature does not verify — \
         the gate is behind the flag again"
    );
}

/// The offline path still serves a cached feed, so the test above is measuring the
/// signature gate and not simply a broken branch.
///
/// The signature is ABSENT here rather than wrong, and that distinction is the whole
/// reason this control works: `allow_unsigned` covers a missing or malformed
/// signature, while one that is well-formed and does not verify is `Invalid` and
/// refused whatever the flag says — the same un-bypassable verdict the index gives,
/// and what the test above relies on.
#[test]
fn offline_still_serves_the_cached_feed_when_unsigned_is_allowed() {
    let home = tmpdir("1048-offline-allowed");
    let (_guard, _lock) = HomeGuard::set(&home);

    let cache = home.join(".loft").join("registry");
    fs::create_dir_all(&cache).expect("cache dir");
    write_file(&cache.join("advisories.json"), advisory_feed());

    let feed = loft::registry_advisories::load_or_fetch(&loft::registry_advisories::LoadOptions {
        allow_unsigned: true,
        offline: true,
        refresh: false,
    })
    .expect("an allowed-unsigned offline load");
    assert!(feed.is_some(), "the cached feed did not come back");
}

/// A fetched feed that fails verification is not left in the cache.
///
/// Writing first and checking after is what turned a correctly refused INDEX into a
/// persistent outage: the bytes outlived the run, and every later command read them
/// against the still-valid old signature and failed the same way, with no retry able
/// to clear it.  The advisories path had the same order.  The assertion is that the
/// cache file does not exist after a refused refresh.
#[test]
fn a_refused_advisory_feed_is_not_left_in_the_cache() {
    let home = tmpdir("1048-refused-not-cached");
    let (_home_guard, _home_lock) = HomeGuard::set(&home);

    // Serve the feed from a local directory: `advisories_url` swaps the trailing
    // `index.json` for `advisories.json`, and no `.sig` beside it means the fetched
    // signature is empty — unverifiable, which is the point.
    let served = tmpdir("1048-served");
    fs::create_dir_all(&served).expect("served dir");
    write_file(&served.join("advisories.json"), advisory_feed());
    let url = common::file_url(&served.join("index.json"));
    let (_url_guard, _url_lock) = RegUrlGuard::set(&url);

    let cache = home.join(".loft").join("registry");
    let cached_feed = cache.join("advisories.json");

    let outcome =
        loft::registry_advisories::load_or_fetch(&loft::registry_advisories::LoadOptions {
            allow_unsigned: false,
            offline: false,
            refresh: true,
        });

    assert!(
        outcome.is_err(),
        "the refresh returned {outcome:?} for an unverifiable feed"
    );
    assert!(
        !cached_feed.exists(),
        "a refused feed was written to {} — it will be read by the next run",
        cached_feed.display()
    );
}
