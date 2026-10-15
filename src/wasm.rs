// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — Browser / WASM target (--html / --native-wasm)

//! WASM entry point and host-bridge stubs for the `wasm` Cargo feature.
//!
//! Compiled only when `--features wasm` is active.  Each host-bridge function
//! corresponds to a JS-side counterpart on `globalThis.loftHost`.
//!
//! Steps: W1.1 (this stub) → W1.2 (output capture) → W1.3–W1.8 (bridges) →
//!        W1.9 (entry point) → W1.16 (file I/O, FS-A … FS-F).
//!
//! FS-A (this file): every stub calls `globalThis.loftHost.*` via `js_sys::Reflect`
//! when compiled under `--features wasm`.  Under the default feature set the stubs
//! continue to return the same harmless defaults as before, so native tests are
//! unaffected.

// ── FS-A  js_sys call helpers (wasm only) ────────────────────────────────────

/// Return the `globalThis.loftHost` object.
#[cfg(feature = "wasm")]
fn loft_host() -> wasm_bindgen::JsValue {
    js_sys::Reflect::get(&js_sys::global(), &"loftHost".into())
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
}

/// Call `globalThis.loftHost[method](args…)`.  Returns `JsValue::UNDEFINED` on error.
#[cfg(feature = "wasm")]
fn host_call(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    let host = loft_host();
    let func: wasm_bindgen::JsValue =
        js_sys::Reflect::get(&host, &method.into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    js_sys::Function::from(func)
        .apply(&host, args)
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
}

/// Public version of `host_call` for use from `parallel.rs`.
#[cfg(feature = "wasm")]
pub fn host_call_raw(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    host_call(method, args)
}

// ── loft#851  Raw-import call helpers (`--html`, no wasm-bindgen) ─────────────
//
// `--html` reaches the same host filesystem as the wasm-bindgen build, but over
// raw `loft_io` imports (`src/lib.rs`) rather than `js_sys` — this target
// refuses a page that imports anything else.  These two helpers are the whole
// difference; every `host_fs_*` below shares one body shape across both
// transports, so the two cannot answer differently.

/// Drain the host's read stash into an owned buffer.  `len` is what the read
/// import answered: `usize::MAX` means the path is absent, which is not the
/// same as an empty file that exists.
#[cfg(all(host_fs, not(feature = "wasm")))]
fn stashed(len: usize) -> Option<Vec<u8>> {
    if len == usize::MAX {
        return None;
    }
    let mut buf = vec![0u8; len];
    if len > 0 {
        crate::loft_host_fs_copy(buf.as_mut_ptr());
    }
    Some(buf)
}

/// Drain the host's read stash as UTF-8 text.  Invalid UTF-8 reads as absent —
/// the same answer `read_file_text_into` gives on native for a file whose bytes
/// are not text, so `content()` cannot start returning mojibake on one target.
#[cfg(all(host_fs, not(feature = "wasm")))]
fn stashed_text(len: usize) -> Option<String> {
    String::from_utf8(stashed(len)?).ok()
}

// ── W1.7 / FS-A  File I/O host bridge ────────────────────────────────────────

/// Check whether a path exists in the virtual filesystem.
#[allow(dead_code)]
pub fn host_fs_exists(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_exists", &args).as_bool().unwrap_or(false)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_exists(path.as_ptr(), path.len()) != 0
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        false
    }
}

/// Read an entire text file.  Returns `None` if absent.
pub fn host_fs_read_text(path: &str) -> Option<String> {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        let v = host_call("fs_read_text", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else {
            v.as_string()
        }
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed_text(crate::loft_host_fs_read_text(path.as_ptr(), path.len()))
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        None
    }
}

/// Write `data` as text to `path`, creating or truncating.  Returns 0 on success.
pub fn host_fs_write_text(path: &str, data: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&path.into(), &data.into());
        host_call("fs_write_text", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_write_text(path.as_ptr(), path.len(), data.as_ptr(), data.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = (path, data);
        0
    }
}

/// Read raw bytes from `path`.  Returns `None` if absent.
pub fn host_fs_read_binary(path: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let args = js_sys::Array::of3(&path.into(), &0.into(), &i32::MAX.into());
        let v = host_call("fs_read_binary", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else if let Ok(arr) = v.dyn_into::<js_sys::Uint8Array>() {
            Some(arr.to_vec())
        } else {
            None
        }
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed(crate::loft_host_fs_read_binary(path.as_ptr(), path.len()))
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        None
    }
}

/// Write raw bytes to `path`, creating or truncating.  Returns 0 on success.
pub fn host_fs_write_binary(path: &str, data: &[u8]) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let arr = js_sys::Uint8Array::from(data);
        let args = js_sys::Array::of2(&path.into(), &arr.into());
        host_call("fs_write_binary", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_write_binary(path.as_ptr(), path.len(), data.as_ptr(), data.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = (path, data);
        0
    }
}

/// Delete `path`.  Returns 0 on success, non-zero on error.
pub fn host_fs_delete(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_delete", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_delete(path.as_ptr(), path.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        1
    }
}

/// Move / rename `from` to `to`.  Returns 0 on success.
pub fn host_fs_move(from: &str, to: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&from.into(), &to.into());
        host_call("fs_move", &args).as_f64().map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_move(from.as_ptr(), from.len(), to.as_ptr(), to.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = (from, to);
        1
    }
}

/// Create a directory.  Returns 0 on success.
pub fn host_fs_mkdir(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_mkdir", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_mkdir(path.as_ptr(), path.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        1
    }
}

/// Create a directory and all parents.  Returns 0 on success.
pub fn host_fs_mkdir_all(path: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_mkdir_all", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_mkdir_all(path.as_ptr(), path.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        1
    }
}

/// Return a list of names inside `path` (directory listing).
pub fn host_fs_list_dir(path: &str) -> Vec<String> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let args = js_sys::Array::of1(&path.into());
        let v = host_call("fs_list_dir", &args);
        if let Ok(arr) = v.dyn_into::<js_sys::Array>() {
            arr.iter().filter_map(|x| x.as_string()).collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        // The host joins names with `\n`.  An EMPTY answer is an empty
        // directory, not one name that happens to be "" — `split` on "" yields
        // one empty field, so it is special-cased rather than filtered, which
        // would also drop a legitimately odd name.
        let joined = stashed_text(crate::loft_host_fs_list_dir(path.as_ptr(), path.len()));
        match joined.as_deref() {
            None | Some("") => Vec::new(),
            Some(s) => s.split('\n').map(str::to_owned).collect(),
        }
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        Vec::new()
    }
}

/// Return `true` if `path` is a directory.
pub fn host_fs_is_dir(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_is_dir", &args).as_bool().unwrap_or(false)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_is_dir(path.as_ptr(), path.len()) != 0
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        false
    }
}

/// Return `true` if `path` is a regular file.
pub fn host_fs_is_file(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_is_file", &args).as_bool().unwrap_or(false)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_is_file(path.as_ptr(), path.len()) != 0
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        false
    }
}

/// Return the byte size of `path`, or -1 if absent.
pub fn host_fs_file_size(path: &str) -> i64 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_file_size", &args)
            .as_f64()
            .map_or(-1, |v| v as i64)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_file_size(path.as_ptr(), path.len()) as i64
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        -1
    }
}

/// Seek the JS-side binary cursor for `path` to `pos`.
pub fn host_fs_seek(path: &str, pos: i64) {
    #[cfg(feature = "wasm")]
    {
        #[allow(clippy::cast_precision_loss)]
        let args = js_sys::Array::of2(&path.into(), &(pos as f64).into());
        host_call("fs_seek", &args);
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        #[allow(clippy::cast_precision_loss)]
        crate::loft_host_fs_seek(path.as_ptr(), path.len(), pos as f64);
    }
    #[cfg(not(host_fs))]
    {
        let _ = (path, pos);
    }
}

/// Read `n` bytes from the JS-side cursor position for `path`.  Advances the cursor.
pub fn host_fs_read_bytes(path: &str, n: usize) -> Option<Vec<u8>> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        #[allow(clippy::cast_precision_loss)]
        let args = js_sys::Array::of2(&path.into(), &(n as f64).into());
        let v = host_call("fs_read_bytes", &args);
        if v.is_null() || v.is_undefined() {
            None
        } else if let Ok(arr) = v.dyn_into::<js_sys::Uint8Array>() {
            Some(arr.to_vec())
        } else {
            None
        }
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed(crate::loft_host_fs_read_bytes(path.as_ptr(), path.len(), n))
    }
    #[cfg(not(host_fs))]
    {
        let _ = (path, n);
        None
    }
}

/// Write `bytes` at the JS-side cursor position for `path`.  Advances the cursor.
pub fn host_fs_write_bytes(path: &str, bytes: &[u8]) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let arr = js_sys::Uint8Array::from(bytes);
        let args = js_sys::Array::of2(&path.into(), &arr.into());
        host_call("fs_write_bytes", &args)
            .as_f64()
            .map_or(5, |v| v as i32)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_write_bytes(path.as_ptr(), path.len(), bytes.as_ptr(), bytes.len())
    }
    #[cfg(not(host_fs))]
    {
        let _ = (path, bytes);
        0
    }
}

/// Return the current JS-side cursor position for `path`.
#[allow(dead_code)]
pub fn host_fs_get_cursor(path: &str) -> i64 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&path.into());
        host_call("fs_get_cursor", &args)
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        crate::loft_host_fs_get_cursor(path.as_ptr(), path.len()) as i64
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        0
    }
}

// ── W1.6  Time and environment host bridges ──────────────────────────────────

/// Return the current time as milliseconds since the Unix epoch.
#[allow(dead_code)]
pub fn host_time_now() -> i64 {
    #[cfg(feature = "wasm")]
    {
        host_call("time_now", &js_sys::Array::new())
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Return the current time as milliseconds since the Unix epoch (monotonic approximation).
#[allow(dead_code)]
pub fn host_time_ticks() -> i64 {
    #[cfg(feature = "wasm")]
    {
        host_call("time_ticks", &js_sys::Array::new())
            .as_f64()
            .map_or(0, |v| v as i64)
    }
    #[cfg(not(feature = "wasm"))]
    host_time_now()
}

/// Return the value of environment variable `name`, or empty string if absent.
pub fn host_env_variable(name: &str) -> String {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&name.into());
        // Text-null, not `""`, when the host has no such variable — the same answer
        // `Stores::os_variable` gives on native, so the two backends cannot disagree about
        // what "not set" is (loft#1302).  A host that deliberately answers `""` still gets
        // `""`, exactly as `var_os` returning `Some("")` does natively.
        host_call("env_variable", &args)
            .as_string()
            .unwrap_or_else(|| crate::store::STRING_NULL.to_string())
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = name;
        String::new()
    }
}

/// Return the command-line arguments (always empty under WASM).
#[allow(dead_code)]
pub fn host_arguments() -> Vec<String> {
    #[cfg(feature = "wasm")]
    {
        use wasm_bindgen::JsCast;
        let v = host_call("arguments", &js_sys::Array::new());
        if let Ok(arr) = v.dyn_into::<js_sys::Array>() {
            arr.iter().filter_map(|x| x.as_string()).collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    }
    #[cfg(not(feature = "wasm"))]
    Vec::new()
}

/// Return the current working directory.
pub fn host_fs_cwd() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_cwd", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed_text(crate::loft_host_fs_cwd()).unwrap_or_default()
    }
    #[cfg(not(host_fs))]
    String::new()
}

/// Return the user home directory.
pub fn host_fs_user_dir() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_user_dir", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed_text(crate::loft_host_fs_user_dir()).unwrap_or_default()
    }
    #[cfg(not(host_fs))]
    String::new()
}

/// Return the program executable directory.
pub fn host_fs_program_dir() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("fs_program_dir", &js_sys::Array::new())
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(all(host_fs, not(feature = "wasm")))]
    {
        stashed_text(crate::loft_host_fs_program_dir()).unwrap_or_default()
    }
    #[cfg(not(host_fs))]
    String::new()
}

// ── W1.5  Random host bridge ─────────────────────────────────────────────────

/// Return a random integer in `[lo, hi]` inclusive.
#[allow(dead_code)]
pub fn host_random_int(lo: i32, hi: i32) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&lo.into(), &hi.into());
        host_call("random_int", &args)
            .as_f64()
            .map_or(lo, |v| v as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        lo.max(hi)
    }
}

/// Reseed the host-side RNG.
#[allow(dead_code)]
pub fn host_random_seed(seed: i64) {
    #[cfg(feature = "wasm")]
    {
        let hi = ((seed >> 32) as i32).into();
        let lo = (seed as i32).into();
        let args = js_sys::Array::of2(&hi, &lo);
        host_call("random_seed", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = seed;
    }
}

// ── W1.4  Logger host bridge ─────────────────────────────────────────────────

/// Write a log line to the host console.
pub fn host_log_write(line: &str) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&line.into());
        host_call("log_write", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = line;
    }
}

// ── TTT v3.5  WebSocket bridge (browser-WASM only) ───────────────────────
//
// Host-side counterparts on `globalThis.loftHost`:
//   ws_connect(url) → integer (slot id, -1 on bad url)
//   ws_ready(id)  → 0 / 1   (1 = the socket is open)
//   ws_closed(id) → 0 / 1   (1 = the socket is closing or closed)
//   ws_send(id, msg, binary) → 0 / 1
//   ws_recv(id) → 0 / 1   (1 = a message landed; read via ws_last_message)
//   ws_last_message() → text
//   ws_last_opcode()  → integer (1 = text, 2 = binary, …)
//   ws_close(id)
//
// Mirrors the surface in `lib/web/native/src/ws_client.rs::wasm_impl`
// but lifts the bridge into the loft interpreter binary itself so
// the `compile_and_run` browser-WASM build can run `use web` programs
// without a separately-loaded cdylib (impossible in the browser).

/// Open a WebSocket to `url`.  Returns the host's slot id (-1 on bad URL).
#[allow(dead_code)]
pub fn host_ws_connect(url: &str) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&url.into());
        host_call("ws_connect", &args)
            .as_f64()
            .map_or(-1, |n| n as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = url;
        -1
    }
}

/// Is slot `id` an OPEN socket?  The browser kernel buffers sends until 1
/// (a native connect blocks until upgraded — same contract, async form).
#[allow(dead_code)]
pub fn host_ws_ready(id: i32) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&id.into());
        host_call("ws_ready", &args)
            .as_f64()
            .is_some_and(|n| n as i32 == 1)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = id;
        false
    }
}

/// Is slot `id` closing or closed?  With [`host_ws_ready`] this tells a socket that is
/// still connecting from one that will never open.  A host without the method answers
/// false, and a refused connect is then not noticed.
#[allow(dead_code)]
pub fn host_ws_closed(id: i32) -> bool {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&id.into());
        host_call("ws_closed", &args)
            .as_f64()
            .is_some_and(|n| n as i32 == 1)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = id;
        false
    }
}

/// The serving origin's hostname — `engine_host::default_host()` on a phone
/// is the cabinet that served the page.
#[allow(dead_code)]
pub fn host_origin_host() -> String {
    #[cfg(feature = "wasm")]
    {
        host_call("origin_host", &js_sys::Array::new())
            .as_string()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    }
    #[cfg(not(feature = "wasm"))]
    {
        "127.0.0.1".to_string()
    }
}

/// Send `msg` on slot `id`.  `binary=true` ships an opcode-2 frame;
/// `binary=false` ships opcode-1 (text).  Returns 1 on success, 0 if
/// the slot is in backoff / disconnected.
#[allow(dead_code)]
pub fn host_ws_send(id: i32, msg: &str, binary: bool) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&id.into(), &msg.into(), &binary.into());
        host_call("ws_send", &args).as_f64().map_or(0, |n| n as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (id, msg, binary);
        0
    }
}

/// Poll slot `id` for an inbound frame.  Returns 1 if a message landed
/// (then call `host_ws_last_message` + `host_ws_last_opcode`), 0 if
/// the queue is empty.
#[allow(dead_code)]
pub fn host_ws_recv(id: i32) -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&id.into());
        host_call("ws_recv", &args).as_f64().map_or(0, |n| n as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = id;
        0
    }
}

/// Read the message bytes the most recent successful `host_ws_recv`
/// surfaced.  Empty string when no recent message.
#[allow(dead_code)]
pub fn host_ws_last_message() -> String {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        host_call("ws_last_message", &args)
            .as_string()
            .unwrap_or_default()
    }
    #[cfg(not(feature = "wasm"))]
    {
        String::new()
    }
}

/// Read the opcode of the most recent successful `host_ws_recv`.
/// 1 = text, 2 = binary, 8 = close, 9 = ping, 10 = pong.
#[allow(dead_code)]
pub fn host_ws_last_opcode() -> i32 {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        host_call("ws_last_opcode", &args)
            .as_f64()
            .map_or(0, |n| n as i32)
    }
    #[cfg(not(feature = "wasm"))]
    {
        0
    }
}

/// Close slot `id` permanently.
#[allow(dead_code)]
pub fn host_ws_close(id: i32) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&id.into());
        host_call("ws_close", &args);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = id;
    }
}

// ── W1.9  Virtual filesystem (VIRT_FS) ───────────────────────────────────────

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Per-thread virtual filesystem used by `compile_and_run()`.
    /// Maps filename → content.  Populated before parsing; cleared after execution.
    pub(crate) static VIRT_FS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Populate the virtual filesystem with the given `(name, content)` pairs.
pub fn virt_fs_populate(files: &[(String, String)]) {
    VIRT_FS.with(|fs| {
        let mut map = fs.borrow_mut();
        for (name, content) in files {
            map.insert(name.clone(), content.clone());
        }
    });
}

/// Return the content of `name` from the virtual filesystem, or `None` if absent.
pub fn virt_fs_get(name: &str) -> Option<String> {
    VIRT_FS.with(|fs| fs.borrow().get(name).cloned())
}

/// Clear all entries from the virtual filesystem.
pub fn virt_fs_clear() {
    VIRT_FS.with(|fs| fs.borrow_mut().clear());
}

// ── @PLN18 08-S6 — the living-page swap bridges ─────────────────────────────
// The page (the persistent host layer) drives the swap: it exports the world
// out of the PARKED instance A, stages it, starts instance B (whose
// `swap_world` consumes the stage), and hands B the living WebSocket via the
// loft-rt adoption hook.  These are the two wasm-side halves.

thread_local! {
    /// The world record registered by the running script's `swap_world(w)`.
    static SWAP_ROOT: std::cell::Cell<Option<(crate::keys::DbRef, u16)>> =
        const { std::cell::Cell::new(None) };
    /// A snapshot staged by the page for the NEXT run's `swap_world`.
    static SWAP_STAGE: RefCell<Option<String>> = const { RefCell::new(None) };
}

// Consumed by the browser kernel's `swap_world`, which exists only in the
// wasm-bindgen bundle.  Both other builds that compile this module — a native
// `--all-features` one and `--html` (loft#851) — see these dead, and that is
// the cfg, not rot.
#[cfg_attr(not(all(target_arch = "wasm32", feature = "wasm")), allow(dead_code))]
pub(crate) fn swap_root_set(root: crate::keys::DbRef, kt: u16) {
    SWAP_ROOT.with(|r| r.set(Some((root, kt))));
}

#[cfg_attr(not(all(target_arch = "wasm32", feature = "wasm")), allow(dead_code))]
pub(crate) fn swap_stage_take() -> Option<String> {
    SWAP_STAGE.with(|s| s.borrow_mut().take())
}

/// The world the parked run registered, if any: its root and its known type.
pub(crate) fn swap_root() -> Option<(crate::keys::DbRef, u16)> {
    SWAP_ROOT.with(std::cell::Cell::get)
}

/// Stage a snapshot for the next run's `swap_world` (the browser analog of
/// the native LOFT_RESUME env).
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn swap_stage(snapshot: &str) {
    SWAP_STAGE.with(|s| *s.borrow_mut() = Some(snapshot.to_string()));
}

// ── W1.2  Output capture ─────────────────────────────────────────────────────

thread_local! {
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Append `text` to the per-thread output buffer.
pub fn output_push(text: &str) {
    OUTPUT.with(|buf| buf.borrow_mut().push_str(text));
}

/// Drain and return the accumulated output since the last call.
pub fn output_take() -> String {
    OUTPUT.with(|buf| std::mem::take(&mut *buf.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_capture() {
        output_push("hello ");
        output_push("world");
        assert_eq!(output_take(), "hello world");
        assert_eq!(output_take(), ""); // cleared after take
    }
}
