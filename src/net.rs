// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I77 — Registry / manifest / lockfile resolution (the shared HTTP-GET layer:
// native delegates to registry_index::http_get_bytes; browser uses the host
// fetch() import).

//! Target-uniform HTTP GET behind the `store_load_url*` stdlib functions.
//!
//! The loft-visible API is identical on every target —
//! `store_load_url_trusted(r, url) -> boolean` is a plain synchronous call — but
//! *how* the bytes are fetched differs, and that difference is confined here:
//!
//! - **native** (`registry` feature): `ureq` (via [`crate::registry_index`]),
//!   including the `file://` offline-mirror path.
//! - **browser** (`--html`, `wasm32-unknown-unknown`): the JS `fetch()` is async,
//!   so a *synchronous* call would deadlock the single-threaded event loop.
//!   Instead this bridges to the `loft_io.loft_host_http_get` host import, which
//!   is in the wasm-opt `--asyncify` allowlist: calling it unwinds the whole wasm
//!   stack back to the event loop, the shell's `AsyncifyCtrl` runs
//!   `await fetch(url)`, then rewinds with the bytes — the same async→sync bridge
//!   `loft_web.ws_yield` proves. So the call returns synchronously and the page
//!   never freezes.
//! - **any other build** (native without `registry`, or the wasm-bindgen gallery
//!   bundle): a clear "unavailable" error, never a silent wrong answer.
//!
//! That last arm is REAL code rather than an absent module.  It used to be spelled by
//! not compiling this file at all, which reads the same from a caller that only ever
//! fetches — and differently from one that merely NAMES the fetcher.  `paged_reader`
//! is the second kind: its HTTP provider refers to `crate::net` whatever the byte
//! source, so a build with the paged loaders and no transport could not compile
//! instead of simply refusing a URL at run time.  A capability this build lacks is
//! an answer, not a missing symbol.

/// The browser-wasm target (`--html`): `wasm32-unknown-unknown`, not WASI, and not
/// the in-process `wasm` host feature. HTTP there goes through the asyncify host
/// import; every other target uses a native client or errors.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    // `loft_host_http_get` suspends via asyncify while JS fetches; it returns the
    // response byte length, or `usize::MAX` on a network / non-2xx error.
    let n = crate::loft_host_http_get(url.as_ptr(), url.len());
    if n == usize::MAX {
        return Err(format!("browser fetch failed for {url}"));
    }
    let mut buf = vec![0u8; n];
    crate::loft_host_http_get_copy(buf.as_mut_ptr());
    Ok(buf)
}

/// Native path: delegate to the `ureq`-backed client (and `file://`) that the
/// registry install already uses.  Reached only on a non-browser build with the
/// `registry` feature — the sole other case where the module compiles (below).
#[cfg(all(
    feature = "registry",
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    http_get_bytes(url)
}

/// A file fetched BESIDE another (a store image's `.dschema` sidecar): its bytes, `Err(None)`
/// when the host answered that it has no such file (403, 404, 410, a `file://` path that is not
/// there), `Err(Some(why))` when the request itself failed — a refused handshake, no connection,
/// another status.  A loader that reads the first as "there is no sidecar" must not read the
/// second so, or a TLS refusal is reported as a missing file.  The browser cannot tell them
/// apart and answers `Err(None)`.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_beside(url: &str) -> Result<Vec<u8>, Option<String>> {
    fetch_bytes(url).map_err(|_| None)
}

#[cfg(all(
    feature = "registry",
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_beside(url: &str) -> Result<Vec<u8>, Option<String>> {
    http_get_within(url, CONNECT_BOUND, DOWNLOAD_BOUND).map_err(|m| match m {
        Miss::Absent(_) => None,
        Miss::Failed(why) => Some(why),
    })
}

// ---------------------------------------------------------------------------
// loft#678 — the RANGE half, behind the working-set store loaders.
//
// `fetch_bytes` above pulls a whole image; these pull the pages a lookup
// actually touches.  The split is the same one, for the same reason, so the
// paged reader and the relocating copy above it stay ONE code path on every
// target: only the byte source differs.
// ---------------------------------------------------------------------------

/// One byte range of `url`, as a `Range: bytes=off-(off+len-1)` GET.  Returns
/// fewer than `len` bytes when the range runs past EOF; the caller zero-pads
/// (a [`crate::paged_reader::PageProvider`] tolerates a short tail by contract).
///
/// # Errors
/// A transport failure or a non-2xx status.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_range(url: &str, off: u64, len: usize) -> Result<Vec<u8>, String> {
    #[allow(clippy::cast_precision_loss)] // < 2^53: exact (see the import's note)
    let n = crate::loft_host_http_range(url.as_ptr(), url.len(), off as f64, len as f64);
    if n == usize::MAX {
        return Err(format!("browser range fetch failed for {url}"));
    }
    let mut buf = vec![0u8; n];
    crate::loft_host_http_get_copy(buf.as_mut_ptr());
    Ok(buf)
}

/// Total byte length of `url`, discovered by the `bytes=0-0` probe that opens a
/// paged source: the response's `Content-Range: bytes 0-0/TOTAL`.
///
/// # Errors
/// A transport failure, or a server that answered without a total.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_size(url: &str) -> Result<u64, String> {
    // The probe must RUN before the total is meaningful — the host reports what
    // the last range response carried, so a bare `_total()` here would read a
    // stale value from an earlier fetch (or -1 on the first).
    fetch_range(url, 0, 1)?;
    let total = crate::loft_host_http_range_total();
    if total < 0.0 {
        return Err(format!("remote store: could not determine size of {url}"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // checked >= 0 above
    Ok(total as u64)
}

/// A small text resource in full (the `.dschema` sidecar beside a store).
/// `None` on a non-2xx status — notably 404, which means "no sidecar", a normal
/// answer the layout gate treats as `Fresh` rather than an error.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
pub(crate) fn fetch_text(url: &str) -> Option<String> {
    fetch_bytes(url)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

/// Native range GET over the same `ureq` client the whole-image path uses.
/// Available whenever a feature that pulls `ureq` is on — `remote-store` (the
/// paged loaders' own feature) or `registry`.
///
/// `paged_store` gates it too, because `paged_reader` — the only caller of this
/// function, its size and text siblings, and `agent` — is `#[cfg(paged_store)]`,
/// and `build.rs` sets that flag for `remote-store` or a browser build, never for
/// `registry`.  Tracking the transport features alone left all four compiled with
/// nothing to call them under `--no-default-features` (the stubs below) and under
/// `--features registry` (these), which `-D warnings` reports as dead code.  The
/// browser-wasm twins above need no clause: `build.rs` sets `paged_store` for that
/// build, so it is already implied there.
#[cfg(all(
    paged_store,
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_range(url: &str, off: u64, len: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if len == 0 {
        return Ok(Vec::new());
    }
    let last = off + len as u64 - 1;
    let resp = agent()
        .get(url)
        .set("Range", &format!("bytes={off}-{last}"))
        .call()
        .map_err(|e| e.to_string())?;
    let partial = resp.status() == 206;
    let mut r = resp.into_reader();
    if !partial {
        // The server ignored `Range` and sent the whole file (200).  Correct, but
        // it means reading the prefix to reach `off` — kept because a source that
        // cannot serve ranges must still produce RIGHT answers, only slowly.
        let mut skip = vec![0u8; usize::try_from(off).unwrap_or(usize::MAX)];
        r.read_exact(&mut skip).map_err(|e| e.to_string())?;
    }
    let mut buf = vec![0u8; len];
    // A short read is EOF, not a failure: the range may overhang the end.
    let mut got = 0;
    while got < len {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) => return Err(e.to_string()),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// Native size probe — `Range: bytes=0-0`, reading `Content-Range`'s total and
/// falling back to `Content-Length`.
///
/// # Errors
/// A transport failure, or neither header carrying a usable total.
#[cfg(all(
    paged_store,
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_size(url: &str) -> Result<u64, String> {
    let resp = agent()
        .get(url)
        .set("Range", "bytes=0-0")
        .call()
        .map_err(|e| e.to_string())?;
    resp.header("Content-Range")
        .and_then(|h| h.rsplit('/').next().and_then(|t| t.trim().parse().ok()))
        .or_else(|| resp.header("Content-Length").and_then(|c| c.parse().ok()))
        .ok_or_else(|| format!("remote store: could not determine size of {url}"))
}

/// Native small-text GET — see the browser twin for why `None` is a normal answer.
#[cfg(all(
    paged_store,
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_text(url: &str) -> Option<String> {
    agent().get(url).call().ok()?.into_string().ok()
}

/// The shared client for the native range/size/text calls, and so carrying their
/// `paged_store` clause: it exists exactly where they do.
#[cfg(all(
    paged_store,
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
fn agent() -> ureq::Agent {
    crate::tls::agent_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

// ---------------------------------------------------------------------------
// The "no transport" arm — the module header's third case, written out.
//
// It is reached by the wasm-bindgen gallery bundle (`wasm32`, the in-process
// `wasm` host feature, no `ureq`) and by a lean native build without `registry`.
// Both can read a store the page CARRIES; neither can reach one over HTTP.  Each
// function answers that in the shape its callers already handle — an `Err` the
// loader turns into a refusal naming the URL, or `None` for the optional sidecar
// — so a URL is refused with a reason rather than fetched wrongly or silently.
// ---------------------------------------------------------------------------

/// No whole-image HTTP transport in this build.
///
/// # Errors
/// Always — the message names the capability, because a caller that reached here
/// asked for something this binary cannot do.
#[cfg(all(
    not(feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
#[allow(dead_code)] // reachable only where the URL loaders are themselves absent
pub(crate) fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    Err(format!(
        "this build has no HTTP transport, so it cannot fetch {url} — a store it \
         carries reads with `store_load`, which needs no network"
    ))
}

/// No ranged HTTP transport in this build.
///
/// # Errors
/// Always; see [`fetch_bytes`]'s twin for why this is an answer and not a panic.
#[cfg(all(
    paged_store,
    not(any(feature = "remote-store", feature = "registry")),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_range(url: &str, _off: u64, _len: usize) -> Result<Vec<u8>, String> {
    Err(format!(
        "this build has no HTTP transport, so it cannot range-read {url} — a store \
         it carries pages through the host filesystem instead"
    ))
}

/// No ranged HTTP transport, so no remote size probe either.
///
/// # Errors
/// Always — refusing at the size probe is what stops a paged open before it starts.
#[cfg(all(
    paged_store,
    not(any(feature = "remote-store", feature = "registry")),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_size(url: &str) -> Result<u64, String> {
    Err(format!(
        "this build has no HTTP transport, so it cannot size {url}"
    ))
}

/// No transport, so no remote sidecar — `None`, which the layout gate already
/// treats as "no sidecar" rather than as a failure.
#[cfg(all(
    paged_store,
    not(any(feature = "remote-store", feature = "registry")),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_text(_url: &str) -> Option<String> {
    None
}

/// Default ceiling on a single HTTP response — the registry index, or a package
/// tarball.  A ceiling has to exist (an unbounded response is an unbounded
/// allocation), but it must never be reached by SILENCE: the previous 50 MB cap
/// TRUNCATED the body and handed the caller a short file, which surfaced as
/// `JSON parse error: unterminated string` at byte 52,428,800 rather than as
/// "the index is bigger than this loft will read".  That number is compiled into
/// every released binary, so an index growing past it would have taken the
/// registry down for every client already in the wild, with nothing the registry
/// itself could do about it.  Raised, and — the part that matters — turned into a
/// refusal, so the day it is reached the message names the ceiling and the way
/// past it.
#[cfg(feature = "registry")]
pub const DEFAULT_MAX_DOWNLOAD: u64 = 512 << 20;

/// The download ceiling in bytes, or `0` for none — `LOFT_MAX_DOWNLOAD`
/// overriding [`DEFAULT_MAX_DOWNLOAD`], written the way a person writes a size
/// (`1G`, `512M`).  An unparseable value keeps the default and says so: a typo
/// in a limit must not silently remove the limit.
#[cfg(feature = "registry")]
#[must_use]
pub fn max_download_bytes() -> u64 {
    let Ok(v) = std::env::var("LOFT_MAX_DOWNLOAD") else {
        return DEFAULT_MAX_DOWNLOAD;
    };
    if let Some(bytes) = crate::store_budget::parse_limit(&v) {
        bytes
    } else {
        eprintln!(
            "loft: LOFT_MAX_DOWNLOAD='{v}' is not a size (try 1G, 512M or 0) — \
             keeping the default {}",
            crate::store_budget::human(DEFAULT_MAX_DOWNLOAD)
        );
        DEFAULT_MAX_DOWNLOAD
    }
}

/// Read a whole response body, REFUSING one longer than `limit` bytes (`0` = no
/// limit) rather than truncating it.  Reads one byte past the ceiling, so a body
/// exactly at it still passes and the first byte over is what trips the refusal.
/// The message leaves the URL to the caller, which already names it.
#[cfg(feature = "registry")]
pub(crate) fn read_capped(reader: impl std::io::Read, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let ceiling = if limit == 0 {
        u64::MAX
    } else {
        limit.saturating_add(1)
    };
    let mut buf = Vec::new();
    reader
        .take(ceiling)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    if limit != 0 && u64::try_from(buf.len()).unwrap_or(u64::MAX) > limit {
        return Err(format!(
            "response is larger than the {} download ceiling — refusing it rather \
             than handing back a truncated file; raise the ceiling with \
             LOFT_MAX_DOWNLOAD=<size> (e.g. 1G, or 0 for no ceiling)",
            crate::store_budget::human(limit)
        ));
    }
    Ok(buf)
}

/// Fetch `url` in full.  A `file://` URL reads the local path directly and is not
/// subject to the download ceiling — nothing about a local mirror is unbounded in
/// the way a remote response is.
#[cfg(feature = "registry")]
pub(crate) fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    http_get_within(url, CONNECT_BOUND, DOWNLOAD_BOUND).map_err(Miss::message)
}

/// How long a download may take to connect, and to finish.
#[cfg(feature = "registry")]
pub(crate) const CONNECT_BOUND: std::time::Duration = std::time::Duration::from_secs(15);
#[cfg(feature = "registry")]
pub(crate) const DOWNLOAD_BOUND: std::time::Duration = std::time::Duration::from_mins(1);

/// Why a request answered no bytes.  Only one of the two says anything about the file:
/// a host that answers "not there" has spoken for it, a request that got no usable
/// answer has not.
#[cfg(feature = "registry")]
#[derive(Debug)]
pub(crate) enum Miss {
    /// The host answered that it does not serve this address (403, 404, 410), or a
    /// `file://` path could not be read.
    Absent(String),
    /// No connection, a timeout, any other status, or a body over the download ceiling.
    Failed(String),
}

#[cfg(feature = "registry")]
impl Miss {
    pub(crate) fn message(self) -> String {
        match self {
            Miss::Absent(why) | Miss::Failed(why) => why,
        }
    }
}

/// [`http_get_bytes`] with the caller's own bounds on connecting and on the whole
/// exchange, telling a file the host does not have from a request that failed.
#[cfg(feature = "registry")]
pub(crate) fn http_get_within(
    url: &str,
    connect: std::time::Duration,
    whole: std::time::Duration,
) -> Result<Vec<u8>, Miss> {
    // Support `file://` URLs for offline
    // mirrors + bundle-import-served indexes.  Same contract as the
    // HTTP path: return the raw bytes at the URL.
    if let Some(path) = url.strip_prefix("file://") {
        return crate::file_access::read(path)
            .map_err(|e| Miss::Absent(format!("file:// read error for {e}")));
    }
    let agent = crate::tls::agent_builder()
        .timeout_connect(connect)
        .timeout(whole)
        .build();
    let response = agent.get(url).call().map_err(|e| {
        let why = format!("HTTP error: {e}");
        match e {
            ureq::Error::Status(403 | 404 | 410, _) => Miss::Absent(why),
            _ => Miss::Failed(why),
        }
    })?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(Miss::Failed(format!("HTTP {status} for {url}")));
    }
    read_capped(response.into_reader(), max_download_bytes()).map_err(Miss::Failed)
}
