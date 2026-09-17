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
    crate::registry_index::http_get_bytes(url)
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
#[cfg(all(
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
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_text(url: &str) -> Option<String> {
    agent().get(url).call().ok()?.into_string().ok()
}

/// The shared client for the native range/size/text calls.
#[cfg(all(
    any(feature = "remote-store", feature = "registry"),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
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
    not(any(feature = "remote-store", feature = "registry")),
    not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))
))]
pub(crate) fn fetch_text(_url: &str) -> Option<String> {
    None
}
