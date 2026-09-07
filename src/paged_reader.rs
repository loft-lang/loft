// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//
//! @PLN97 arc G Phase 2 — a read-only, transient PAGED view over a store image.
//!
//! A [`Store`](crate::store::Store) reads through one contiguous buffer
//! (`*(ptr + rec·8 + fld)`). A remote or very large store has no such buffer —
//! bytes arrive a page at a time. This module is the P0 `b-builtin` result: a
//! self-contained Rust reader that resolves any `(offset, len)` by fetching
//! only the pages it touches, so a `find`/range traversal over a 0.5 GB block
//! pulls a handful of pages, not the whole file. The paged reader is **read
//! only and transient** — it resolves + copies out; the *result* of a
//! `store_load_*` is a normal heap [`Store`](crate::store::Store).
//!
//! Two invariants shape it (see `plans/97-layout-contract/REMOTE_STORE_LOADER.md`):
//! - **byte-identical working set** — because a store file IS its serialization
//!   (`byte = rec·8`, native-endian, per `formal/layout.md`), reading a record
//!   is copying its bytes; [`PagedReader::u32_at`] etc. mirror the
//!   [`Store`](crate::store::Store) accessors exactly.
//! - **zero cost to non-users** — the whole module is behind the `remote-store`
//!   feature, so a lean build compiles none of it and drags in no HTTP deps.
//!
//! The page provider is swappable: [`LocalFileProvider`] is the deterministic
//! test harness (it logs every range fetched, making "bytes fetched ≪ file" a
//! countable assertion); #517 HTTP swaps in at Phase 5 behind the same trait.

/// Page granularity (bytes) — the unit of a fetch. 64 KiB matches the design's
/// recommendation for the small-word index traversal.
pub const PAGE_SIZE: usize = 64 * 1024;

/// The source of a store image's bytes. A [`PagedReader`] only ever asks for
/// page-aligned ranges, and tolerates a fetch running past EOF (the tail is
/// zero-padded), so a provider may clamp to its own size.
pub trait PageProvider {
    /// Total byte length of the backing image.
    fn size(&self) -> u64;
    /// Return exactly `len` bytes starting at `off`. Bytes at or past
    /// [`size`](PageProvider::size) read as zero.
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8>;

    /// Fetch several INDEPENDENT ranges, returning them in the order given.
    ///
    /// loft#782 — a keyed load DISCOVERS what it needs by walking, so its reads look
    /// sequential even where they are not: once the entries are located, their pages do not
    /// depend on each other at all. Handing the provider the whole set lets a transport with
    /// concurrency spend one round trip instead of N, and over a link the round trip is the
    /// entire cost — a 1-byte and a 64 kB range measure the same ~45 ms.
    ///
    /// The default loops, so a provider with nothing to gain (a local file, where a round
    /// trip is a `seek`) needs no code and behaves exactly as before.
    fn fetch_many(&mut self, ranges: &[(u64, usize)]) -> Vec<Vec<u8>> {
        ranges.iter().map(|&(o, l)| self.fetch(o, l)).collect()
    }

    /// Can this transport serve several ranges in less time than one at a time?
    ///
    /// loft#787 — the answer decides whether PREFETCHING is worth its own cost.
    /// `warm` computes a page set, sorts it, dedups it, builds runs and splits them,
    /// and every bit of that is spent to turn N round trips into one. A transport
    /// that issues them sequentially anyway collects none of that saving and pays
    /// all of the work, so it is better off demand-paging exactly as it did before
    /// batching existed.
    ///
    /// The wasm HTTP provider is that case, and not by accident: it has no threads
    /// (loft#784), so its `fetch_many` is a loop. Measured on an `--html` kernel,
    /// `warm` and its callees are 15 kB and 8 functions of code that runs during
    /// every load and buys the browser nothing.
    ///
    /// Default `true`: a provider that says nothing is assumed to gain from
    /// batching, which is the safe direction — the cost of a needless prefetch is
    /// work, the cost of a missing one is round trips.
    fn batches(&self) -> bool {
        true
    }
}

/// A local-file page provider — reads ranges from a store file on disk and
/// **logs every `(off, len)` fetched**, so a test can assert the working-set
/// load touched ≪ the whole file. This is the deterministic stand-in for the
/// #517 HTTP `Range` provider (Phase 5); the traversal code is identical.
pub struct LocalFileProvider {
    file: std::fs::File,
    size: u64,
    /// Every `(off, len)` this provider was asked to fetch, in order.
    pub fetches: Vec<(u64, usize)>,
    /// loft#782 — SEQUENTIAL provider invocations: the round-trip DEPTH. One `fetch` is
    /// one; one `fetch_many` is one however many ranges it carries. `fetches.len()` counts
    /// ranges and so cannot tell a batch from a queue — which is the entire question here.
    pub depth: usize,
}

impl LocalFileProvider {
    /// Open `path` read-only.
    ///
    /// # Errors
    /// Returns the underlying `io::Error` if `path` can't be opened or its
    /// metadata (length) can't be read.
    pub fn open(path: &str) -> std::io::Result<Self> {
        use std::io::Read as _;
        let mut file = std::fs::File::open(path)?;
        // Reading the METADATA is not reading the format.  A path that opens and is not
        // a store image — an empty file, a truncated download, a directory, an HTTP 200
        // serving an error page — used to get all the way past here and fail deep inside
        // the load, which has only `false` to answer with; the binding then reported no
        // fault and no error, so an unusable source was indistinguishable from an image
        // that simply lacks the key (loft#994).  Four bytes settle it, and refusing HERE
        // is what gives every `refuse_paged` call site the report it already words
        // correctly.
        let mut head = [0u8; 4];
        let n = file.read(&mut head)?;
        if !crate::store::Store::has_signature(&head[..n]) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a loft store image (the file does not begin with the store signature)",
            ));
        }
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            size,
            fetches: Vec::new(),
            depth: 0,
        })
    }

    /// Total bytes fetched so far — the numerator of the "≪ file" assertion.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        self.fetches.iter().map(|(_, l)| l).sum()
    }
}

impl PageProvider for LocalFileProvider {
    fn size(&self) -> u64 {
        self.size
    }

    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        use std::io::{Read, Seek, SeekFrom};
        self.fetches.push((off, len));
        self.depth += 1;
        let mut buf = vec![0u8; len];
        if off < self.size {
            // Read what actually exists; anything past EOF stays zero.
            let want = len.min((self.size - off) as usize);
            if self.file.seek(SeekFrom::Start(off)).is_ok() {
                let _ = self.file.read_exact(&mut buf[..want]);
            }
        }
        buf
    }
}

/// An HTTP `Range` page provider — fetches byte ranges from a remote store over
/// `Range: bytes=a-b` GETs (the #517 stack), so a working-set load pulls only the
/// pages a lookup touches across the network. The remote counterpart of
/// [`LocalFileProvider`]; the paged reader + traversal + copy above it are
/// identical, so only the byte source differs.
/// The byte source is [`crate::net`], not a client held here, so this provider is
/// target-independent: native builds range-GET over `ureq`, and the browser
/// (`--html`) goes through the asyncify `fetch()` host import (loft#678). Nothing
/// above this struct — the reader, the traversal, the relocating copy — knows
/// which.
pub struct HttpRangeProvider {
    url: String,
    size: u64,
    /// Every `(off, len)` fetched — for the "bytes fetched ≪ file" assertion.
    pub fetches: Vec<(u64, usize)>,
    /// loft#782 — SEQUENTIAL round trips. See [`LocalFileProvider::depth`].
    pub depth: usize,
}

impl HttpRangeProvider {
    /// Open `url`, discovering the store's total size via a `Range: bytes=0-0`
    /// probe (`Content-Range: bytes 0-0/TOTAL`), falling back to `Content-Length`.
    ///
    /// # Errors
    /// Returns a message when the request fails or the size can't be determined.
    pub fn open(url: &str) -> Result<Self, String> {
        let size = crate::net::fetch_size(url)?;
        // A size is not a format.  A host answering `200` with its own error page — a
        // stale CDN path, a bucket's 404 document, a half-finished upload — has a size
        // like any other source, and binding to it used to succeed and answer `null` for
        // every key with the channel reporting healthy.  A genuine 404 and a refused
        // connection were both reported, so this is exactly the "reachable, wrong bytes"
        // case (loft#994).  One four-byte range read, once per bind.
        let head = crate::net::fetch_range(url, 0, 4)?;
        if !crate::store::Store::has_signature(&head) {
            return Err(format!(
                "{url} does not begin with the store signature — it is reachable but is \
                 not a loft store image"
            ));
        }
        Ok(Self {
            url: url.to_string(),
            size,
            fetches: Vec::new(),
            depth: 0,
        })
    }

    /// Total bytes fetched so far.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        self.fetches.iter().map(|(_, l)| l).sum()
    }
}

impl PageProvider for HttpRangeProvider {
    fn size(&self) -> u64 {
        self.size
    }

    /// Only where the ranges actually go out together. On wasm `fetch_many` below is
    /// a sequential loop (loft#784 — a browser has no `std::thread`), so a prefetch
    /// there computes a page set to save round trips it cannot save (loft#787).
    #[cfg(not(target_family = "wasm"))]
    fn batches(&self) -> bool {
        true
    }
    #[cfg(target_family = "wasm")]
    fn batches(&self) -> bool {
        false
    }

    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        self.fetches.push((off, len));
        self.depth += 1;
        let mut buf = vec![0u8; len];
        if off >= self.size {
            return buf; // wholly past EOF — zero
        }
        // Clamp to EOF: asking past the end is legal for the caller (the contract
        // zero-pads), but a server may answer an unsatisfiable range with 416.
        let want = usize::try_from((self.size - off).min(len as u64)).unwrap_or(len);
        if let Ok(got) = crate::net::fetch_range(&self.url, off, want) {
            let n = got.len().min(buf.len());
            buf[..n].copy_from_slice(&got[..n]);
        }
        // A failed fetch reads as zeros, exactly as a past-EOF page does. The
        // caller cannot tell them apart, which is why every loader re-checks the
        // copy with `store_verify` rather than trusting the bytes.
        buf
    }

    /// loft#782 — issue the ranges CONCURRENTLY, one thread each, reassembled in order.
    ///
    /// This is where the win is. Measured on a 7.2 MB image with a scattered 40-key
    /// selection over a 15 ms/request link: 60 serial round trips cost ~1035 ms, and the
    /// same 60 ranges issued together cost about one.
    ///
    /// `fetches` is still recorded in the caller's order, so the "bytes fetched ≪ file"
    /// assertions read exactly as before — going parallel changes when bytes arrive, never
    /// which. A single range takes the direct path: a thread would cost more than it saves.
    ///
    /// **On wasm the ranges go one at a time** (loft#784). A browser has no
    /// `std::thread::spawn`, and the batched path used it unconditionally, so a
    /// multi-range read never completed — `loft --html` stalled with no error and no trap,
    /// which is the worst way for this to fail. There is no browser-side concurrency to
    /// substitute: the bytes arrive through the synchronous `loft_host_http_*` bridge, and
    /// making that concurrent would change the host contract. So the target without threads
    /// gets the sequential loop, which is exactly what it did before the batching landed —
    /// correct, and slower by the round trips the batch would have saved.
    fn fetch_many(&mut self, ranges: &[(u64, usize)]) -> Vec<Vec<u8>> {
        // Each `fetch` records its own range and bumps `depth`, so the sequential path
        // reports the round-trip depth it genuinely spends. Reporting one batch here
        // would make the instrument agree with the native build and describe nothing.
        #[cfg(target_family = "wasm")]
        {
            return ranges.iter().map(|&(o, l)| self.fetch(o, l)).collect();
        }
        #[cfg(not(target_family = "wasm"))]
        {
            if ranges.len() <= 1 {
                return ranges.iter().map(|&(o, l)| self.fetch(o, l)).collect();
            }
            for &r in ranges {
                self.fetches.push(r);
            }
            self.depth += 1; // ONE round trip, however many ranges ride in it
            let (url, size) = (self.url.clone(), self.size);
            let handles: Vec<_> = ranges
                .iter()
                .map(|&(off, len)| {
                    let url = url.clone();
                    std::thread::spawn(move || {
                        let mut buf = vec![0u8; len];
                        if off >= size {
                            return buf;
                        }
                        let want = usize::try_from((size - off).min(len as u64)).unwrap_or(len);
                        if let Ok(got) = crate::net::fetch_range(&url, off, want) {
                            let n = got.len().min(buf.len());
                            buf[..n].copy_from_slice(&got[..n]);
                        }
                        buf
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        }
    }
}

/// The byte source a `store_load*` reads through, chosen by the path scheme: a
/// local file or an HTTP `Range` URL. Lets the generic [`PagedReader`] and the
/// find / relocating-copy above it stay ONE code path regardless of where the
/// bytes live — the whole point of the `PageProvider` seam.
pub enum PageSource {
    Local(LocalFileProvider),
    Http(HttpRangeProvider),
}

impl PageSource {
    /// Open `spec` as an HTTP URL (`http://` / `https://`) or a local file path.
    ///
    /// # Errors
    /// Propagates the underlying provider's open error.
    pub fn open(spec: &str) -> Result<Self, String> {
        if spec.starts_with("http://") || spec.starts_with("https://") {
            Ok(PageSource::Http(HttpRangeProvider::open(spec)?))
        } else {
            LocalFileProvider::open(spec)
                .map(PageSource::Local)
                .map_err(|e| e.to_string())
        }
    }

    /// Total bytes fetched so far, whichever provider backs this source.
    #[must_use]
    pub fn bytes_fetched(&self) -> usize {
        match self {
            PageSource::Local(p) => p.bytes_fetched(),
            PageSource::Http(p) => p.bytes_fetched(),
        }
    }

    /// PROBE (loft#729): bytes over DISTINCT offsets — how much of the image the
    /// traversal actually touched, ignoring re-fetches. `bytes_fetched` minus
    /// this is what the cache failed to keep, and the two answer different
    /// questions: over-READ (touching bytes nobody wanted) versus re-READ
    /// (touching the same bytes again). One is a layout/traversal cost, the
    /// other is a cache cost, and a single total cannot tell them apart.
    /// Number of provider calls — the round-trip count an HTTP source pays,
    /// which bytes alone does not show.
    #[must_use]
    pub fn requests(&self) -> usize {
        match self {
            PageSource::Local(p) => p.fetches.len(),
            PageSource::Http(p) => p.fetches.len(),
        }
    }

    /// loft#782 — sequential round trips, which is the cost over a link.
    ///
    /// Distinct from [`requests`](PageSource::requests): that counts RANGES, so a batch of
    /// 27 issued together and 27 issued one after another look identical to it. They are
    /// the difference between 15 ms and 400 ms.
    #[must_use]
    pub fn round_trips(&self) -> usize {
        match self {
            PageSource::Local(p) => p.depth,
            PageSource::Http(p) => p.depth,
        }
    }

    #[must_use]
    pub fn distinct_bytes(&self) -> usize {
        let log = match self {
            PageSource::Local(p) => &p.fetches,
            PageSource::Http(p) => &p.fetches,
        };
        let mut seen = std::collections::HashSet::new();
        log.iter()
            .filter(|(off, _)| seen.insert(*off))
            .map(|(_, l)| l)
            .sum()
    }
}

impl PageProvider for PageSource {
    fn size(&self) -> u64 {
        match self {
            PageSource::Local(p) => p.size(),
            PageSource::Http(p) => p.size(),
        }
    }
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        match self {
            PageSource::Local(p) => p.fetch(off, len),
            PageSource::Http(p) => p.fetch(off, len),
        }
    }

    /// Forward the BATCH to the concrete provider.
    ///
    /// Without this the trait default applies here — a loop over `PageSource::fetch`, which
    /// dispatches one range at a time and throws away the whole point. An enum that
    /// implements a trait by delegation has to delegate every method that has a default,
    /// because the default is silently correct and silently slow: the batch still returns
    /// the right bytes, so nothing fails, and the win simply does not happen (loft#782).
    fn fetch_many(&mut self, ranges: &[(u64, usize)]) -> Vec<Vec<u8>> {
        match self {
            PageSource::Local(p) => p.fetch_many(ranges),
            PageSource::Http(p) => p.fetch_many(ranges),
        }
    }
}

/// The layout-identity gate for a working-set load (@PLN97 3b.5). Given the
/// store `spec` (a local path or an `http(s)://` URL), read the `.dschema`
/// sidecar beside it and compare its recorded layout with the running program's
/// `current` identity. An ABSENT sidecar (a legacy / pre-3b.5 store) or an
/// unreachable remote yields [`SchemaVerdict::Fresh`] — the caller proceeds and
/// the post-copy `store_verify` stays the backstop; a PRESENT sidecar that is
/// mismatched or unparseable yields a non-raw-safe verdict the caller rejects
/// on, so foreign-layout bytes are never range-read at schema-derived offsets.
#[must_use]
pub fn check_sidecar(
    spec: &str,
    current: &crate::schema_sidecar::LayoutIdentity,
) -> crate::schema_sidecar::SchemaVerdict {
    use crate::schema_sidecar::SchemaVerdict;
    if spec.starts_with("http://") || spec.starts_with("https://") {
        match crate::net::fetch_text(&format!("{spec}.dschema")) {
            Some(text) => crate::schema_sidecar::verdict_for_sidecar_text(&text, current),
            None => SchemaVerdict::Fresh, // 404 / unreachable → proceed (store_verify backstop)
        }
    } else {
        crate::schema_sidecar::check_beside(std::path::Path::new(spec), current)
            .unwrap_or(SchemaVerdict::Fresh) // IO error (not NotFound) → proceed (backstop)
    }
}

/// A read-only, transient paged view: a sparse LRU page table + fetch-on-miss
/// [`resolve`](PagedReader::resolve). Only resident pages cost memory, so a
/// small cache over a huge store is fine — it just fetches more. Correctness is
/// **independent of residency** (the `capacity == 1` test proves it).
pub struct PagedReader<P: PageProvider> {
    provider: P,
    page_size: usize,
    /// `page_idx -> page bytes` (always `page_size` long; the last page is
    /// zero-padded past EOF).
    pages: std::collections::HashMap<u64, Vec<u8>>,
    /// LRU order, front = least-recently used; bounded by `capacity`.
    lru: std::collections::VecDeque<u64>,
    /// Max resident pages.
    capacity: usize,
    /// loft#785 — is `capacity` a HARD cap, or a floor the working set may raise?
    ///
    /// `true` only when the caller named a size (`LOFT_PAGE_CACHE_BYTES`, or
    /// [`with_config`](PagedReader::with_config)): someone who states a memory bound gets
    /// it. Otherwise [`warm`](PagedReader::warm) may grow the cache to hold what it
    /// prefetches, because a prefetched page that is evicted before the walk reads it costs
    /// TWO fetches instead of one — see that method for why the default has to be growth.
    capped: bool,
    /// loft#729 — `resolve` calls and bytes asked for, bucketed by power-of-two
    /// size (bucket `i` is `2^i ..< 2^(i+1)`). What the traversal ASKS for is a
    /// separate fact from what the provider FETCHES, and only the first says
    /// whether a span-shaped optimisation has anything to bite on.
    reads: [(u64, u64); 32],
    /// loft#787 — operations on the page map, which is the other per-READ cost besides
    /// the allocation `read_into` removed. Every one of them hashes a `u64` key and probes
    /// a table, and the hot path used to do TWO for a single four-byte read: `ensure_page`
    /// asked `contains_key`, then the read indexed the map again for the same key. Counted
    /// rather than reasoned about, because "how many times do we hash" is exactly the kind
    /// of claim that reads as obvious and measures otherwise (see `page_ops`).
    page_ops: u64,
}

impl<P: PageProvider> PagedReader<P> {
    /// A reader with the default 64 KiB page and a 64-page (4 MiB) cache.
    pub fn new(provider: P) -> Self {
        // loft#729 — the page size and the cache are separately settable so the
        // two causes of a large `bytes_fetched` can be told apart. They are
        // easily confused: shrinking the page ALSO shrinks a page-counted cache,
        // so a naive page sweep measures thrash and reads it as granularity.
        // The cache is therefore sized in BYTES and held constant across a
        // sweep. Reading a store with the page driven to 8 bytes gives the
        // traversal's true byte footprint, which is what page-rounding waste has
        // to be measured against — assuming that footprint instead is how
        // loft#729 came to attribute a 4× to page granularity that measured
        // 1.36× on the reporter's own data.
        let ps = std::env::var("LOFT_PAGE_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(PAGE_SIZE);
        let asked = std::env::var("LOFT_PAGE_CACHE_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        let cache_bytes = asked.unwrap_or(64 * PAGE_SIZE);
        let mut r = Self::with_config(provider, ps, (cache_bytes / ps).max(1));
        // loft#785 — an unasked-for default is a FLOOR, not a bound. Only a caller who
        // named a number is held to it. **On every target, including wasm.**
        //
        // ⚠ A wasm-only cap was tried here and MEASURED WORSE, so the reasoning that led to
        // it is worth keeping as a warning. It ran: an eviction costs a round trip natively
        // but only a copy in a browser, so the browser should bound its cache and buy back
        // the locality of holding hundreds of 64 KiB pages in linear memory. Every step of
        // that is true and the conclusion still did not hold, because it never priced the
        // RE-FETCH. Against a 5.6 MB working set the 4 MiB default evicted pages the walk
        // had not reached yet: range reads 71 → 115 and bytes 5.7 → 8.5 MB — a smaller copy
        // of the exact amplification loft#785 removed — to save 1 MB of a 17 MB heap.
        // Measured in a real browser, on the app that reported loft#787, arms interleaved.
        //
        // The rule that survives: **residency policy follows the price of an eviction, and
        // that price is a re-fetch on every transport.** What differs per transport is
        // PREFETCH, not retention — see `batches` above.
        r.capped = asked.is_some();
        r
    }

    /// A reader with an explicit page size and cache cap (both clamped to ≥ 1).
    /// The tiny-page form is for tests that want boundary-spanning reads without
    /// a 64 KiB fixture.
    ///
    /// The capacity given here is a HARD cap: naming a size is how a caller states a memory
    /// bound, and [`warm`](PagedReader::warm) will trim its prefetch to fit rather than
    /// exceed it (loft#785).
    pub fn with_config(provider: P, page_size: usize, capacity: usize) -> Self {
        Self {
            provider,
            page_size: page_size.max(1),
            pages: std::collections::HashMap::new(),
            lru: std::collections::VecDeque::new(),
            capacity: capacity.max(1),
            capped: true,
            reads: [(0, 0); 32],
            page_ops: 0,
        }
    }

    /// loft#729 — the `resolve` size histogram as `(log2 bucket, calls, bytes)`,
    /// skipping empty buckets.
    #[must_use]
    pub fn read_histogram(&self) -> Vec<(u32, u64, u64)> {
        self.reads
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| *c > 0)
            .map(|(i, (c, b))| (u32::try_from(i).unwrap_or(0), *c, *b))
            .collect()
    }

    /// The underlying provider (e.g. to read its fetch log).
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Total image size in bytes.
    pub fn size(&self) -> u64 {
        self.provider.size()
    }

    /// Drop least-recently-used pages until the cache is back inside `capacity`.
    ///
    /// loft#785 — a no-op when the capacity was never asked for. A page dropped here is a
    /// page the walk re-fetches, and with a prefetch feeding the cache the pages evicted
    /// are exactly the ones about to be read: on 162 scattered keys over an 87 MB store
    /// that turned a 19.9 MB working set into 88.6 MB fetched. Residency is the reader's
    /// only defence against re-fetching, so it is spent only against a bound someone chose.
    /// Correctness never depended on it (the `capacity == 1` test is the proof).
    fn evict_to_capacity(&mut self) {
        if !self.capped {
            return;
        }
        while self.pages.len() > self.capacity {
            if let Some(old) = self.lru.pop_front() {
                self.page_ops += 1;
                self.pages.remove(&old);
            }
        }
    }

    /// Make page `pidx` resident (fetching on miss) and mark it most-recently
    /// used, evicting the LRU page(s) once over capacity.
    fn ensure_page(&mut self, pidx: u64) {
        self.page_ops += 1;
        if self.pages.contains_key(&pidx) {
            self.touch(pidx);
            return;
        }
        self.page_ops += 1;
        let bytes = self
            .provider
            .fetch(pidx * self.page_size as u64, self.page_size);
        self.pages.insert(pidx, bytes);
        self.note_resident(pidx);
        self.evict_to_capacity();
    }

    /// Make page `pidx` resident and hand it back — hashing the key **once**.
    ///
    /// ⚠ **THE READ PATH USED TO HASH THE SAME KEY TWICE.** `ensure_page` asked
    /// `contains_key(&pidx)`, returned nothing, and the caller then wrote
    /// `&self.pages[&pidx]` — a second hash and a second probe for the page it
    /// had just confirmed. Two map operations to read four bytes, on the path
    /// that runs ~800 000 times per viewport in the consumer of loft#787.
    ///
    /// This is the sibling of the allocation that `read_into` removed, and it is
    /// the same shape of mistake: a cost that is per-READ, invisible in a
    /// profile that attributes by function, and obvious only once counted. The
    /// `entry` API answers "is it there, and if not put it there" with one hash,
    /// which is exactly the question the read path is asking.
    ///
    /// The bounded mode keeps the old two-op route deliberately. Its LRU
    /// bookkeeping needs the membership answer *before* the read in order to
    /// mark recency, and it is opt-in (`LOFT_PAGE_CACHE_BYTES`), so the caller
    /// who asked for a memory bound is the only one who pays for maintaining it.
    fn page(&mut self, pidx: u64) -> &[u8] {
        if self.capped {
            self.ensure_page(pidx);
            self.page_ops += 1;
            return &self.pages[&pidx];
        }
        // Split the borrow so the miss arm can reach the provider from inside
        // `or_insert_with` — without this the closure would hold `&mut self`.
        let Self {
            pages,
            provider,
            page_size,
            page_ops,
            ..
        } = self;
        *page_ops += 1;
        let ps = *page_size;
        pages
            .entry(pidx)
            .or_insert_with(|| provider.fetch(pidx * ps as u64, ps))
    }

    /// Page-map operations so far — every `contains_key` / `entry` / index /
    /// `insert` / `remove`, each of which hashes a key and probes the table.
    ///
    /// Exposed because it is the honest unit for the read path's remaining
    /// per-read cost: a duration measures the machine, this measures the work.
    #[must_use]
    pub fn page_ops(&self) -> u64 {
        self.page_ops
    }

    /// Mark a RESIDENT page most-recently-used.
    ///
    /// A no-op when nothing evicts (loft#787). The recency order exists only so
    /// `evict_to_capacity` can pick a victim; with no cap there is no victim, so
    /// maintaining it buys nothing — and it is not free. The hit path scanned the
    /// deque (`iter().position`) and then shifted it (`VecDeque::remove`), both
    /// O(n), on EVERY page touch. That cost was bounded while the cache was 64
    /// pages; loft#785 removed the cap, so n became the whole working set and the
    /// scan grew with it, on the hottest path in the reader.
    ///
    /// Native barely noticed — a round-trip-bound load hides it, and the same
    /// change removed real round trips. The BROWSER pays it undisguised: it is
    /// sequential by design (loft#784), so there is nothing to hide behind, and a
    /// throttled phone multiplies it. That asymmetry is why this was reported as a
    /// browser regression against a native win.
    fn touch(&mut self, pidx: u64) {
        if !self.capped {
            return;
        }
        if let Some(p) = self.lru.iter().position(|&x| x == pidx) {
            self.lru.remove(p);
        }
        self.lru.push_back(pidx);
    }

    /// Record a NEWLY resident page. Same reasoning as [`touch`](Self::touch): the
    /// list is only a victim queue, so an uncapped reader keeps none.
    fn note_resident(&mut self, pidx: u64) {
        if self.capped {
            self.lru.push_back(pidx);
        }
    }

    /// Fetch the pages a read needs in as few provider calls as possible: each
    /// maximal run of CONSECUTIVE absent pages becomes one request (loft#729).
    ///
    /// Byte-neutral by construction — the same pages arrive, and one request for
    /// a run of them carries exactly what N requests would have. What it removes
    /// is round-trips, which is the cost that does not show up in
    /// `bytes_fetched` and is the one an HTTP source actually pays. It matters
    /// now because record bodies are read whole: a 256 KB body is four
    /// consecutive 64 KiB pages, and it used to ask for them one at a time.
    ///
    /// A run is fetched only where pages are ABSENT, so a read straddling
    /// resident pages still costs nothing for the parts already held.
    /// loft#782 — fetch every page these ranges touch that is not resident, in ONE batched
    /// provider call.
    ///
    /// The caller knows these addresses ahead of the walk, so the reads only LOOK
    /// sequential. Warming them together turns N round trips into one, and every later
    /// `resolve` over the same bytes is a page-table hit.
    ///
    /// Adjacent pages coalesce into runs first, so a clustered selection costs one range
    /// rather than one per page.
    ///
    /// **It must not evict what it just fetched — loft#785.** This once claimed it "cannot
    /// make a load fetch more", and that was false the moment the warmed set did not fit
    /// the cache: the pages land at the LRU back, push out the pages the walk is about to
    /// read, and the walk fetches them a second time. That is strictly worse than no
    /// prefetch at all — plain demand paging at least has locality on its side, which is
    /// why the binary BEFORE batching did not amplify on the same 64-page cache. Measured
    /// on 162 scattered keys over an 87 MB store: 88.6 MB fetched for a 19.9 MB working
    /// set, 3.6× amplification, gone the moment the set fits.
    ///
    /// So the cache is sized to the WORK, two ways:
    ///
    /// - **Uncapped** (the default): grow to hold what is warmed. The bound is the working
    ///   set the caller asked for, which it is already materialising a local copy of — you
    ///   cannot load 162 map cells while holding fewer bytes than those cells occupy.
    /// - **Capped** (a caller named a size): warm only what fits, and let the rest demand
    ///   page. The bound is honoured, and the prefetch stops paying for pages it is about to
    ///   drop — the same 162 keys under an explicit 4 MB cap went 88.6 MB → 58.3 MB. Some
    ///   re-fetching remains, and must: a working set larger than the cache you asked for
    ///   cannot be held, and that is the cost of naming the bound.
    pub fn warm(&mut self, ranges: &[(u64, usize)]) {
        // loft#787 — a transport that cannot batch gains nothing here and pays for
        // the whole computation. Demand paging then fetches exactly the same pages,
        // which is what it did before batching existed.
        if !self.provider.batches() {
            return;
        }
        let ps = self.page_size as u64;
        let mut want: Vec<u64> = Vec::new();
        for &(off, len) in ranges {
            if len == 0 {
                continue;
            }
            let (first, last) = (off / ps, (off + len as u64 - 1) / ps);
            for p in first..=last {
                self.page_ops += 1;
                if !self.pages.contains_key(&p) {
                    want.push(p);
                }
            }
        }
        want.sort_unstable();
        want.dedup();
        if want.is_empty() {
            return;
        }
        if self.capped {
            // Room for the prefetch is what is left after the pages already resident. A
            // page warmed past that is guaranteed to be evicted before it is read, so
            // fetching it is pure waste — demand paging will get it when it is wanted.
            let room = self.capacity.saturating_sub(self.pages.len());
            if room == 0 {
                return;
            }
            want.truncate(room);
        }
        let mut runs: Vec<(u64, u64)> = Vec::new(); // (first page, page count)
        for p in want {
            match runs.last_mut() {
                Some((start, n)) if *start + *n == p => *n += 1,
                _ => runs.push((p, 1)),
            }
        }
        let reqs: Vec<(u64, usize)> = runs
            .iter()
            .map(|&(start, n)| (start * ps, usize::try_from(n).unwrap_or(1) * self.page_size))
            .collect();
        let bodies = self.provider.fetch_many(&reqs);
        for (&(start, n), bytes) in runs.iter().zip(bodies) {
            // Copy ONE page, not the remainder of the run. `bytes[lo..].to_vec()`
            // followed by `truncate` copied every byte after `lo` and threw most of
            // it away, so an n-page run copied n(n+1)/2 pages to keep n — quadratic
            // in run length, and pure CPU. `prefetch_run` (loft#729) already did
            // this correctly with `chunks`; this site is the one that drifted.
            for i in 0..n {
                let lo = usize::try_from(i).unwrap_or(0) * self.page_size;
                let hi = (lo + self.page_size).min(bytes.len());
                let mut page = bytes.get(lo..hi).unwrap_or(&[]).to_vec();
                page.resize(self.page_size, 0); // the last page zero-pads past EOF
                self.page_ops += 1;
                self.pages.insert(start + i, page);
                self.note_resident(start + i);
            }
        }
        self.evict_to_capacity();
    }

    fn prefetch_run(&mut self, off: u64, len: usize) {
        if len <= self.page_size {
            return; // at most two pages, and `ensure_page` handles those
        }
        let ps = self.page_size as u64;
        let (first, last) = (off / ps, (off + len as u64 - 1) / ps);
        let mut p = first;
        while p <= last {
            self.page_ops += 1;
            if self.pages.contains_key(&p) {
                p += 1;
                continue;
            }
            let start = p;
            while p <= last && {
                self.page_ops += 1;
                !self.pages.contains_key(&p)
            } {
                p += 1;
            }
            let count = usize::try_from(p - start).unwrap_or(1);
            if count == 1 {
                continue; // `ensure_page` will take it; no run to coalesce
            }
            let bytes = self.provider.fetch(start * ps, count * self.page_size);
            for (i, chunk) in bytes.chunks(self.page_size).enumerate().take(count) {
                let mut page = chunk.to_vec();
                page.resize(self.page_size, 0); // the last page zero-pads past EOF
                self.page_ops += 1;
                self.pages.insert(start + i as u64, page);
                self.note_resident(start + i as u64);
            }
            self.evict_to_capacity();
        }
    }

    /// Copy `len` bytes at absolute byte offset `off` into a fresh buffer,
    /// fetching any missing page(s). A read within one page copies from that
    /// page; a read spanning a page boundary is coalesced here. The common
    /// index-traversal read (4–8 bytes) stays within a page.
    ///
    /// **Every** read goes through the page table, including a whole record
    /// body — fetching a large span directly instead was measured and is WORSE
    /// (loft#729). It reads as an obvious win and is not: a working set's
    /// records are neighbours, so the pages under them are SHARED, and a span
    /// that bypasses the table pays its own bytes while the pages around it get
    /// fetched anyway for the traversal words. On a real 162 MB store, one
    /// 42-key viewport, bytes fetched: 5 832 704 paged, against 6 384 300 /
    /// 7 367 532 / 7 896 716 as the direct-span threshold came down from 256 KiB
    /// to 4 KiB. Sharing is the whole value of the page, and the amplification
    /// #729 reports is not the reader's to give back.
    pub fn resolve(&mut self, off: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        self.read_into(off, &mut out);
        out
    }

    /// The same read as [`resolve`], into a buffer the caller already has.
    ///
    /// ⚠ **THE ALLOCATION WAS THE READ'S DOMINANT COST, AND ONLY OFF-NATIVE.**
    /// Every typed accessor below asks for 1, 2, 4 or 8 bytes, and each one used
    /// to `vec![0u8; len]` to carry them — one malloc and one free to return a
    /// `u32`. A single viewport does ~800 000 of those reads (loft#783), so it
    /// did ~800 000 allocations of four bytes.
    ///
    /// Natively that is invisible: glibc serves a 4-byte request out of the
    /// thread's tcache in tens of nanoseconds, which is why every probe taken on
    /// the native path — three of them, on this issue — measured nothing. In the
    /// browser the allocator is dlmalloc compiled to wasm, on the linear heap,
    /// with no thread-local fast path, and the cost is per READ rather than per
    /// request: it tracks the read count and not the byte count or the round
    /// trips. That is the shape loft#787 reported (a keyed paged load 1.14x
    /// slower with FEWER requests and fewer bytes, while pure compute was
    /// unchanged at 0.98x) and it is the shape a policy knob cannot explain.
    ///
    /// So the small read now borrows the page instead of buying a buffer for it.
    /// `resolve` keeps the owning signature for the span reads that genuinely
    /// need one (a record body being relocated).
    pub fn read_into(&mut self, off: u64, buf: &mut [u8]) {
        let len = buf.len();
        self.reads[(usize::BITS - len.max(1).leading_zeros() - 1) as usize % 32].0 += 1;
        self.reads[(usize::BITS - len.max(1).leading_zeros() - 1) as usize % 32].1 += len as u64;
        let in_page = (off % self.page_size as u64) as usize;
        // The overwhelmingly common case: the whole read sits inside one page.
        // No `prefetch_run` (it returns immediately for anything this small), no
        // loop, one bounds check, one copy.
        if len <= self.page_size - in_page {
            let pidx = off / self.page_size as u64;
            let page = self.page(pidx);
            buf.copy_from_slice(&page[in_page..in_page + len]);
            return;
        }
        self.prefetch_run(off, len);
        let mut done = 0usize;
        while done < len {
            let abs = off + done as u64;
            let pidx = abs / self.page_size as u64;
            let in_page = (abs % self.page_size as u64) as usize;
            let take = (self.page_size - in_page).min(len - done);
            let page = self.page(pidx);
            buf[done..done + take].copy_from_slice(&page[in_page..in_page + take]);
            done += take;
        }
    }

    // --- typed reads at (rec, fld), byte = rec·8 + fld, native-endian ---
    // These mirror the `Store` accessors so a traversal over the paged reader
    // sees the same values a whole-file `Store` would.

    /// 4-byte unsigned read (bucket slots, record size headers, ptrs).
    pub fn u32_at(&mut self, rec: u32, fld: u32) -> u32 {
        let mut b = [0u8; 4];
        self.read_into(u64::from(rec) * 8 + u64::from(fld), &mut b);
        u32::from_ne_bytes(b)
    }

    /// 4-byte signed read.
    pub fn i32_at(&mut self, rec: u32, fld: u32) -> i32 {
        self.u32_at(rec, fld) as i32
    }

    /// 8-byte signed read (`integer` / `long` key fields, the seed halves).
    pub fn i64_at(&mut self, rec: u32, fld: u32) -> i64 {
        let mut b = [0u8; 8];
        self.read_into(u64::from(rec) * 8 + u64::from(fld), &mut b);
        i64::from_ne_bytes(b)
    }

    /// 2-byte unsigned read (a narrow integer key field).
    pub fn u16_at(&mut self, rec: u32, fld: u32) -> u16 {
        let mut b = [0u8; 2];
        self.read_into(u64::from(rec) * 8 + u64::from(fld), &mut b);
        u16::from_ne_bytes(b)
    }

    /// 1-byte read (a `byte` key field).
    pub fn u8_at(&mut self, rec: u32, fld: u32) -> u8 {
        let mut b = [0u8; 1];
        self.read_into(u64::from(rec) * 8 + u64::from(fld), &mut b);
        b[0]
    }

    /// The record's size header (word 0) = its word count (`room`).
    pub fn record_words(&mut self, rec: u32) -> u32 {
        self.u32_at(rec, 0)
    }
}

/// Where a hash's entry for `key` lives in the paged image, as `(record, byte
/// offset)`, or `(0, 0)` when absent — a read-only port of `hash::find`
/// (`src/hash.rs`) over [`PagedReader`] instead of a `Store`.
///
/// `keys` is the collection's key schema (`Stores::keys(type)`); `key_hash` is reused
/// verbatim (it hashes the `Content` key, not a store). The bucket-record layout
/// mirrors `hash.rs`, and it is the same fact seen twice: seed at `SEED_FLD`, buckets
/// from `BUCKET0`, `elms = (room - RESERVED_WORDS)·2`, a slot holding an arena index
/// decoded through [`crate::arena`] — or, for a table that borrows its records, the
/// record number itself.  A pre-@PLN135-arc-H image cannot reach here: the placement
/// token refuses it at load.
pub fn find_hash_entry<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    key: &[crate::keys::Content],
    keys: &[crate::keys::Key],
) -> (u32, u32) {
    let claim = reader.u32_at(root_rec, root_pos);
    if claim == 0 {
        return (0, 0);
    }
    let room = reader.record_words(claim);
    if room < crate::hash::RESERVED_WORDS {
        return (0, 0);
    }
    let elms = (room - crate::hash::RESERVED_WORDS) * 2;
    let stride = reader.u32_at(claim, crate::hash::STRIDE_FLD);
    let seed = reader.i64_at(claim, crate::hash::SEED_FLD) as u64;
    let hash_val = crate::keys::key_hash(key, seed);
    let mut index = (hash_val % u64::from(elms)) as u32;
    let mut slot = reader.u32_at(
        claim,
        crate::hash::BUCKET0 + index * crate::hash::SLOT_BYTES,
    );
    for _ in 0..elms {
        if slot == 0 {
            return (0, 0);
        }
        let (rec, pos) = entry_at(reader, claim, slot, stride);
        if rec != 0 && key_compare_reader(reader, key, rec, pos, keys) == std::cmp::Ordering::Equal
        {
            return (rec, pos);
        }
        index = (index + 1) % elms;
        slot = reader.u32_at(
            claim,
            crate::hash::BUCKET0 + index * crate::hash::SLOT_BYTES,
        );
    }
    (0, 0)
}

/// Decode one bucket slot of the paged image — the read-only twin of `hash.rs`'s
/// `entry_ref`.  `stride == 0` marks a table that BORROWS its records (a secondary
/// index), where a slot is a record number and the payload starts at byte 8.
fn entry_at<P: PageProvider>(
    reader: &mut PagedReader<P>,
    claim: u32,
    slot: u32,
    stride: u32,
) -> (u32, u32) {
    // Per SLOT, not per table: the high bit marks a record number, and one table can
    // hold both kinds (`hash::SLOT_RECORD`).
    if stride == 0 || slot & crate::hash::SLOT_RECORD != 0 {
        return (slot & !crate::hash::SLOT_RECORD, 8);
    }
    let dir = reader.u32_at(claim, crate::arena::DIR_FLD);
    if dir == 0 {
        return (0, 0);
    }
    let (k, off) = crate::arena::locate(slot, stride);
    let cap = (reader.record_words(dir) - 1) * 2;
    if k >= cap {
        return (0, 0);
    }
    let chunk = reader.u32_at(dir, crate::arena::DIR0 + 4 * k);
    if chunk == 0 { (0, 0) } else { (chunk, off) }
}

/// loft#782 — the byte address of each key's FIRST bucket slot, without a read per key.
///
/// The hash walk looks sequential and mostly is not: `claim`, `elms` and the `seed` are
/// properties of the TABLE, identical for every key, so once they are known each key's slot
/// is `key_hash(key, seed) % elms` — arithmetic. The whole set of first probes is therefore
/// knowable up front and can be fetched in one round trip instead of N.
///
/// Empty when the table is absent or malformed; the caller then simply walks as before. A
/// probe that collides still costs its own read — this warms the first slot, which is the
/// one every lookup takes.
pub fn hash_bucket_addrs<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    key_sets: &[Vec<crate::keys::Content>],
) -> Vec<(u64, usize)> {
    let claim = reader.u32_at(root_rec, root_pos);
    if claim == 0 {
        return Vec::new();
    }
    let room = reader.record_words(claim);
    if room < crate::hash::RESERVED_WORDS {
        return Vec::new();
    }
    let elms = (room - crate::hash::RESERVED_WORDS) * 2;
    if elms == 0 {
        return Vec::new();
    }
    let seed = reader.i64_at(claim, crate::hash::SEED_FLD) as u64;
    key_sets
        .iter()
        .map(|k| {
            let index = (crate::keys::key_hash(k, seed) % u64::from(elms)) as u32;
            (
                u64::from(claim) * 8
                    + u64::from(crate::hash::BUCKET0 + index * crate::hash::SLOT_BYTES),
                crate::hash::SLOT_BYTES as usize,
            )
        })
        .collect()
}

/// Read the string at string-record `rec` over the paged reader (fld 4 = length,
/// fld 8.. = UTF-8 bytes). A `rec == 0` / out-of-range yields the empty string —
/// `resolve` zero-pads, so this never faults on a wild pointer.
fn read_reader_str<P: PageProvider>(reader: &mut PagedReader<P>, rec: u32) -> String {
    if rec == 0 {
        return String::new();
    }
    let len = reader.u32_at(rec, 4) as usize;
    let bytes = reader.resolve(u64::from(rec) * 8 + 8, len);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Compare `key` against the key fields of the entry at `(rec, pos)` in the
/// paged image — the reader port of `keys::compare_key` for the fixed-width
/// numeric key types (1/2 = i64 `integer`/`long`, 8/9/10/11 = narrow ints) and
/// text keys (6).
/// Text/float keys (types 6/3/4) are a follow-up (they need a string-record
/// read / float compare); an unsupported type returns `Greater` so it never
/// falsely matches.
fn key_compare_reader<P: PageProvider>(
    reader: &mut PagedReader<P>,
    key: &[crate::keys::Content],
    rec: u32,
    pos: u32,
    keys: &[crate::keys::Key],
) -> std::cmp::Ordering {
    use crate::keys::Content;
    use std::cmp::Ordering;
    for (k_nr, val) in key.iter().enumerate() {
        let k = &keys[k_nr];
        let p = pos + u32::from(k.position);
        let c = match (val, k.type_nr.abs()) {
            (Content::Long(v), 1 | 2) => v.cmp(&reader.i64_at(rec, p)),
            (Content::Long(v), 8) => v.cmp(&i64::from(reader.i32_at(rec, p))),
            (Content::Long(v), 12) => v.cmp(&i64::from(reader.u32_at(rec, p))),
            (Content::Long(v), 9) => {
                let mut b = [0u8; 2];
                reader.read_into(u64::from(rec) * 8 + u64::from(p), &mut b);
                v.cmp(&i64::from(i16::from_ne_bytes(b)))
            }
            (Content::Long(v), 10) => {
                let mut b = [0u8; 1];
                reader.read_into(u64::from(rec) * 8 + u64::from(p), &mut b);
                v.cmp(&i64::from(b[0] as i8))
            }
            (Content::Long(v), 11) => {
                let mut b = [0u8; 2];
                reader.read_into(u64::from(rec) * 8 + u64::from(p), &mut b);
                v.cmp(&i64::from(u16::from_ne_bytes(b) as i16))
            }
            // text key (3b.6): the field holds a `u32` pointer to a string record
            // (fld 4 = length, fld 8.. = UTF-8 bytes). Read it over the reader and
            // compare as strings. `key_hash` already hashes the `Content::Str`.
            (Content::Str(v), 6) => {
                let str_rec = reader.u32_at(rec, p);
                let s = read_reader_str(reader, str_rec);
                v.str().cmp(s.as_str())
            }
            _ => return Ordering::Greater, // unsupported key type — never match
        };
        if c != Ordering::Equal {
            return if k.type_nr < 0 { c.reverse() } else { c };
        }
    }
    Ordering::Equal
}

/// Locate the byte offsets (within the sorted vector's inner record) of the
/// entries with key in `[lo, hi]`, in a SORTED collection rooted at `(root_rec,
/// root_pos)` of the paged image — a read-only port of `vector::sorted_find`'s
/// binary search + a forward walk over [`PagedReader`]. Returns `(inner_rec,
/// positions)`; `positions` are `8 + i·esize` for each in-range element, in key
/// order. `esize` is the element size in bytes.
#[must_use]
pub fn sorted_range_positions<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    esize: u32,
    lo: &[crate::keys::Content],
    hi: &[crate::keys::Content],
    keys: &[crate::keys::Key],
) -> (u32, Vec<u32>) {
    use std::cmp::Ordering;
    let inner = reader.u32_at(root_rec, root_pos);
    if inner == 0 || esize == 0 {
        return (inner, Vec::new());
    }
    let length = reader.u32_at(inner, 4);
    // Binary search for the FIRST index whose key is >= lo.
    let (mut left, mut right) = (0u32, length);
    while left < right {
        let mid = left + (right - left) / 2;
        let epos = 8 + mid * esize;
        // key_compare_reader(lo, elem) == Greater  ⇔  lo > elem  ⇔  elem < lo.
        if key_compare_reader(reader, lo, inner, epos, keys) == Ordering::Greater {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    // Walk forward while elem key <= hi.
    let mut out = Vec::new();
    let mut i = left;
    while i < length {
        let epos = 8 + i * esize;
        // key_compare_reader(hi, elem) == Less  ⇔  hi < elem  ⇔  past the range.
        if key_compare_reader(reader, hi, inner, epos, keys) == Ordering::Less {
            break;
        }
        out.push(epos);
        i += 1;
    }
    (inner, out)
}

// ---------------------------------------------------------------------------
// @PLN134 — a TRIE read a page at a time
// ---------------------------------------------------------------------------

/// Payload offset of an element record's fields; mirrors `trie_db::PAYLOAD` and
/// `radix_db::PAYLOAD` — the shared keyed-collection convention, so a trie and a
/// spatial index read theirs from the same constant.
const ELEM_PAYLOAD: u32 = 8;

/// `text`'s key type number, as `compare_key` spells it. A trie over any other key
/// type has no bytes to walk, so the paged form declines it rather than reading a
/// number as a string.
const TRIE_TEXT_TYPE: i8 = 6;

/// The tree's node array and key oracle, read out of a PAGED image.
///
/// This is the whole of what a paged trie query adds: the descent, the split point,
/// the in-order step and the seek all come from [`radix_tree`](crate::radix_tree) —
/// the tree that WROTE the image is where its geometry lives, so a query over a file
/// and a query over resident memory cannot drift apart (`r12` pins that they do not).
///
/// **Fuel, because the bytes are not ours.** A resident tree is one this process
/// built; an image is a file, and a file can be truncated, foreign or hostile. Every
/// read is already total (the reader zero-pads past EOF, so no offset faults), but a
/// child pointer that cycles would make a *descent* spin. A legitimate walk visits
/// each node at most once — `bit` strictly increases along a path — so `nodes + 4`
/// hops is a bound no healthy walk reaches, and passing it answers `Empty`: a
/// corrupted image reports the key as ABSENT instead of hanging the program.
///
/// The budget is refilled per WALK (`walk_begin`), not per query, and that
/// distinction is not academic: a seek runs four bounded walks, so one budget for
/// the whole query would have to be a guessed multiple of the depth — and the guess
/// under-provisioned on a SMALL tree, where a 5-node trie answered every exact
/// lookup as absent while the prefix walk still worked.
struct PagedTrie<'a, P: PageProvider> {
    reader: &'a mut PagedReader<P>,
    /// The tree container record, `0` when the collection is empty.
    tree: u32,
    /// Byte offset of the text-key field within an element record.
    key_pos: u32,
    /// Node high-water mark, read once — what each walk's fuel is drawn from.
    nodes: u32,
    /// Node hops left in the CURRENT walk; refilled by `walk_begin`.
    fuel: u32,
    /// The last record's key bytes. `first_diff_words` asks for word after word of
    /// one record's key, and the prefix walk then compares that same key again, so
    /// without this every 8 bytes of comparison costs a string-record read.
    cached: Option<(u32, Vec<u8>)>,
}

impl<'a, P: PageProvider> PagedTrie<'a, P> {
    /// Open the tree rooted in the image at `(root_rec, root_pos)` — the same
    /// 4-byte claim slot a resident trie keeps its tree id in.
    fn open(reader: &'a mut PagedReader<P>, root_rec: u32, root_pos: u32, key_pos: u32) -> Self {
        let tree = reader.u32_at(root_rec, root_pos);
        let nodes = if tree == 0 {
            0
        } else {
            reader.u32_at(tree, crate::radix_tree::NODES)
        };
        Self {
            reader,
            tree,
            key_pos,
            nodes,
            fuel: 0,
            cached: None,
        }
    }

    /// The record's key bytes, read through its string pointer (fld 4 = length,
    /// fld 8.. = the UTF-8 bytes), and kept until another record is asked for.
    fn key_bytes(&mut self, rec: u32) -> &[u8] {
        if self.cached.as_ref().is_none_or(|(r, _)| *r != rec) {
            let ptr = self.reader.u32_at(rec, self.key_pos);
            let len = if ptr == 0 {
                0
            } else {
                self.reader.u32_at(ptr, 4) as usize
            };
            let bytes = if len == 0 {
                Vec::new()
            } else {
                self.reader.resolve(u64::from(ptr) * 8 + 8, len)
            };
            self.cached = Some((rec, bytes));
        }
        self.cached.as_ref().map_or(&[], |(_, b)| b.as_slice())
    }
}

impl<P: PageProvider> crate::radix_tree::TreeNodes for PagedTrie<'_, P> {
    fn walk_begin(&mut self) {
        self.fuel = self.nodes.saturating_add(4);
    }
    fn top(&mut self) -> crate::radix_tree::Child {
        if self.tree == 0 {
            return crate::radix_tree::Child::Empty;
        }
        crate::radix_tree::Child::decode(self.reader.i32_at(self.tree, crate::radix_tree::TOP))
    }
    fn len(&mut self) -> u32 {
        if self.tree == 0 {
            return 0;
        }
        self.reader.u32_at(self.tree, crate::radix_tree::LEN)
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        self.reader
            .u32_at(self.tree, crate::radix_tree::node_off(n))
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        self.reader
            .u32_at(self.tree, crate::radix_tree::node_off(n) + 4)
    }
    fn child(&mut self, n: u32, dir: bool) -> crate::radix_tree::Child {
        if self.fuel == 0 || n == 0 || n > self.nodes {
            return crate::radix_tree::Child::Empty;
        }
        self.fuel -= 1;
        crate::radix_tree::Child::decode(
            self.reader
                .i32_at(self.tree, crate::radix_tree::child_off(n, dir)),
        )
    }
}

impl<P: PageProvider> crate::radix_tree::TreeKeys for PagedTrie<'_, P> {
    fn rec_bits(&mut self, rec: u32) -> u32 {
        self.key_bytes(rec).len() as u32 * 8
    }
    fn rec_word(&mut self, rec: u32, word: u32) -> u64 {
        crate::trie_db::bytes_word(self.key_bytes(rec), word)
    }
}

/// The key field's byte offset within an element record, when the collection is a
/// TEXT-keyed trie. `None` for any other key shape — a numeric trie has no bytes to
/// walk, and answering a miss beats reading an integer as a string.
fn trie_key_pos(keys: &[crate::keys::Key]) -> Option<u32> {
    match keys.first() {
        Some(k) if k.type_nr == TRIE_TEXT_TYPE => Some(ELEM_PAYLOAD + u32::from(k.position)),
        _ => None,
    }
}

/// Locate the record stored under `key` in a TRIE rooted at `(root_rec, root_pos)`
/// of the paged image — the reader port of `trie_db::find`, and the trie's twin of
/// [`find_hash_entry`]. `0` when absent.
///
/// One root→leaf descent plus one key comparison, so the pages it touches are the
/// node pages on that path and the candidate's own — which is the whole reason a
/// trie is worth paging (@PLN134 step 2 measured 2.8 of them, mean, on a 239-page
/// node array laid out by `rtree_relayout`).
pub fn trie_find_rec<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    key: &str,
    keys: &[crate::keys::Key],
) -> u32 {
    let Some(key_pos) = trie_key_pos(keys) else {
        return 0;
    };
    let mut src = PagedTrie::open(reader, root_rec, root_pos, key_pos);
    if src.tree == 0 {
        return 0;
    }
    let q = key.as_bytes();
    let probe = |w: u32| crate::trie_db::bytes_word(q, w);
    let probe_bits = q.len() as u32 * 8;
    let (it, d) = crate::radix_tree::seek_gen(&mut src, &probe, probe_bits);
    let rec = it.rec();
    if rec == 0 {
        return 0;
    }
    // `rtree_get`'s rule, and both halves carry weight: agreement over the probe's
    // whole length does not exclude a LONGER stored key (`"ab"` against `"abc"`),
    // and equal lengths alone say nothing about the bits.
    let matched = d.is_none_or(|d| d >= probe_bits);
    if matched && src.key_bytes(rec).len() as u32 * 8 == probe_bits {
        rec
    } else {
        0
    }
}

/// Every record in the paged TRIE whose key begins with `pre`, in key order, capped
/// at `limit` — the reader port of `trie_db::prefix`, and the paged counterpart of
/// [`sorted_range_positions`].
///
/// **The cap is what makes a search box cheap, so it bounds the WALK and not just
/// the answer.** `t["kerk"..:8]` stops after the eighth record: the ninth is never
/// stepped to, so its pages are never fetched. A prefix walk that materialised the
/// run and then truncated would read all 459 records for `kerk` to return 8 — the
/// one operation where paging could quietly become a whole-image read.
///
/// `rtree_seek` positions at the lowest key `>= pre`, and because a probe carries id
/// `0` that is the first record BEARING the prefix; in-order traversal is increasing
/// key order, so the first key that does not begin with `pre` is a correct stop.
pub fn trie_prefix_recs<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    pre: &str,
    keys: &[crate::keys::Key],
    limit: Option<usize>,
) -> Vec<u32> {
    let mut out = Vec::new();
    let Some(key_pos) = trie_key_pos(keys) else {
        return out;
    };
    let mut src = PagedTrie::open(reader, root_rec, root_pos, key_pos);
    if src.tree == 0 {
        return out;
    }
    let cap = limit.unwrap_or(usize::MAX);
    if cap == 0 {
        return out;
    }
    let q = pre.as_bytes();
    let probe = |w: u32| crate::trie_db::bytes_word(q, w);
    let (mut it, _) = crate::radix_tree::seek_gen(&mut src, &probe, q.len() as u32 * 8);
    let mut rec = it.rec();
    while rec != 0 && out.len() < cap {
        if !src.key_bytes(rec).starts_with(q) {
            break;
        }
        out.push(rec);
        rec = it.step_gen(&mut src, true).unwrap_or(0);
    }
    out
}

// ---------------------------------------------------------------------------
// @PLN136 — a SPATIAL index read a page at a time
// ---------------------------------------------------------------------------

/// The tree's node array and coordinate reader, over a PAGED image — the spatial
/// twin of [`PagedTrie`].
///
/// The kinds share the tree and nothing above it, and the split shows here: this
/// answers a record's AXES, where the trie answers a record's bytes. Everything
/// between — the descent, the split point, the in-order step, the seek — is
/// [`radix_tree`](crate::radix_tree)'s, and the box walk on top of it is
/// [`radix_db`](crate::radix_db)'s, so a query over a file and a query over resident
/// memory are the same query with a different byte source.
///
/// Fuel, for the same reason `PagedTrie` carries it: the bytes are a file, a file
/// can be truncated or hostile, and a child pointer that cycles would make a descent
/// spin. Refilled per WALK, so each of a seek's four bounded walks gets its own
/// budget rather than a guessed share of one.
struct PagedSpatial<'a, P: PageProvider> {
    reader: &'a mut PagedReader<P>,
    /// The tree container record, `0` when the collection is empty.
    tree: u32,
    /// The collection's coordinate key fields, axis 0 first.
    keys: Vec<crate::keys::Key>,
    /// Node high-water mark, read once — what each walk's fuel is drawn from.
    nodes: u32,
    /// Node hops left in the CURRENT walk; refilled by `walk_begin`.
    fuel: u32,
    /// The last record's axis codes. A seek asks for word after word of one
    /// record's key and the box test then asks for its axes, so without this every
    /// comparison costs a re-read of the record's coordinate fields.
    cached: Option<(u32, [u64; crate::radix_db::MAX_AXES])>,
}

impl<'a, P: PageProvider> PagedSpatial<'a, P> {
    fn open(
        reader: &'a mut PagedReader<P>,
        root_rec: u32,
        root_pos: u32,
        keys: &[crate::keys::Key],
    ) -> Self {
        let tree = reader.u32_at(root_rec, root_pos);
        let nodes = if tree == 0 {
            0
        } else {
            reader.u32_at(tree, crate::radix_tree::NODES)
        };
        Self {
            reader,
            tree,
            keys: keys.to_vec(),
            nodes,
            fuel: 0,
            cached: None,
        }
    }

    /// One axis field's signed value — the paged mirror of `radix_db`'s `axis_i64`,
    /// width for width.
    ///
    /// The widths and their null sentinels are the schema's, and this is the SECOND
    /// reader of them. The two disagreeing on one width makes a record's Morton code
    /// differ between the storages, which does not present as an error: it presents
    /// as a box query that misses a point inside it. `a_paged_box_agrees_with_the
    /// _resident_one` drives every width through both readers and compares the
    /// answers, which is what holds them in step.
    fn axis_value(&mut self, rec: u32, key: &crate::keys::Key) -> i64 {
        let p = ELEM_PAYLOAD + u32::from(key.position);
        match key.type_nr.unsigned_abs() {
            // 4-byte signed (`int<…>`), as `Store::get_i32_raw` reads it.
            8 => i64::from(self.reader.i32_at(rec, p)),
            // 4-byte UNSIGNED (`intraw<…>`), as `Store::get_u32_raw` reads it.
            12 => i64::from(self.reader.u32_at(rec, p)),
            // 2-byte, `1`-biased with `0` reserved for null (`Store::get_short`).
            9 => {
                let raw = self.reader.u16_at(rec, p);
                if raw == 0 {
                    i64::from(i32::MIN)
                } else {
                    i64::from(i32::from(raw) - 1)
                }
            }
            // 1-byte, unbiased (`Store::get_byte` with `min` 0).
            10 => i64::from(self.reader.u8_at(rec, p)),
            // 2-byte signed.
            11 => i64::from(self.reader.u16_at(rec, p) as i16),
            // `integer` (1), `long` (2), and any other integer default.
            _ => self.reader.i64_at(rec, p),
        }
    }

    /// The record's axis codes, order-preserving, kept until another record is asked
    /// for.
    fn axes(&mut self, rec: u32) -> [u64; crate::radix_db::MAX_AXES] {
        if self.cached.as_ref().is_none_or(|(r, _)| *r != rec) {
            let mut out = [0u64; crate::radix_db::MAX_AXES];
            // The keys are read out first: `axis_value` takes `&mut self` (a read
            // may fetch a page), so it cannot borrow the key from `self.keys`.
            let axes: Vec<crate::keys::Key> = self
                .keys
                .iter()
                .take(crate::radix_db::MAX_AXES)
                .cloned()
                .collect();
            for (slot, key) in out.iter_mut().zip(&axes) {
                *slot = crate::radix_db::coord_code(self.axis_value(rec, key));
            }
            self.cached = Some((rec, out));
        }
        self.cached
            .map_or([0; crate::radix_db::MAX_AXES], |(_, a)| a)
    }
}

impl<P: PageProvider> crate::radix_tree::TreeNodes for PagedSpatial<'_, P> {
    fn walk_begin(&mut self) {
        self.fuel = self.nodes.saturating_add(4);
    }
    fn top(&mut self) -> crate::radix_tree::Child {
        if self.tree == 0 {
            return crate::radix_tree::Child::Empty;
        }
        crate::radix_tree::Child::decode(self.reader.i32_at(self.tree, crate::radix_tree::TOP))
    }
    fn len(&mut self) -> u32 {
        if self.tree == 0 {
            return 0;
        }
        self.reader.u32_at(self.tree, crate::radix_tree::LEN)
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        self.reader
            .u32_at(self.tree, crate::radix_tree::node_off(n))
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        self.reader
            .u32_at(self.tree, crate::radix_tree::node_off(n) + 4)
    }
    fn child(&mut self, n: u32, dir: bool) -> crate::radix_tree::Child {
        if self.fuel == 0 || n == 0 || n > self.nodes {
            return crate::radix_tree::Child::Empty;
        }
        self.fuel -= 1;
        crate::radix_tree::Child::decode(
            self.reader
                .i32_at(self.tree, crate::radix_tree::child_off(n, dir)),
        )
    }
}

impl<P: PageProvider> crate::radix_tree::TreeKeys for PagedSpatial<'_, P> {
    fn rec_bits(&mut self, _rec: u32) -> u32 {
        crate::radix_db::key_bits(&self.keys)
    }
    fn rec_word(&mut self, rec: u32, word: u32) -> u64 {
        let axes = self.axes(rec);
        crate::radix_db::interleave(word, self.keys.len(), |a| axes[a])
    }
}

impl<P: PageProvider> crate::radix_db::BoxNodes for PagedSpatial<'_, P> {
    fn rec_axes(&mut self, rec: u32, _n: usize) -> [u64; crate::radix_db::MAX_AXES] {
        self.axes(rec)
    }
}

/// One axis field's value, read through the paged decoder — for
/// `radix_db::paged::every_axis_width_decodes_to_what_was_written`, which fixes both
/// readers to the SCHEMA rather than to each other.
#[cfg(test)]
pub fn spatial_axis_value<P: PageProvider>(
    reader: &mut PagedReader<P>,
    rec: u32,
    key: &crate::keys::Key,
) -> i64 {
    // No tree is opened: reading one field needs the reader and the key, nothing the
    // container holds. `open` still probes `(0, 0)` for a tree id and gets whatever
    // the store's signature word says — harmless, because every read here is total
    // (the reader zero-pads past EOF) and `axis_value` never consults `tree`.
    let mut src = PagedSpatial::open(reader, 0, 0, std::slice::from_ref(key));
    src.axis_value(rec, key)
}

/// The coordinate key fields of a SPATIAL collection, when every axis is an integer
/// one. `None` for any other key shape.
///
/// A `spatial` key is 1–3 integer axes by construction (the parser rejects more, and
/// rejects a non-integer axis), so this declines only a collection that is not a
/// spatial one — which is what makes answering a miss right rather than reading a
/// string pointer as a coordinate.
fn spatial_keys(keys: &[crate::keys::Key]) -> Option<Vec<crate::keys::Key>> {
    if keys.is_empty() || keys.len() > crate::radix_db::MAX_AXES {
        return None;
    }
    // The trie's TEXT key is the one shape sharing this tree that has no coordinate.
    if keys.iter().any(|k| k.type_nr == TRIE_TEXT_TYPE) {
        return None;
    }
    Some(keys.to_vec())
}

/// Every record of the paged SPATIAL index inside the closed box with corners
/// `from` and `till`, in Morton order, capped at `limit` — the reader port of
/// `radix_db::box_range`, and the spatial counterpart of [`trie_prefix_recs`].
///
/// **The cap bounds the WALK.** The box's records come out in Morton order and the
/// `limit`-th is the last one anything is fetched for. That is not the same
/// statement as "the answer is capped": a walk that materialised the box and then
/// truncated would fetch every page the box covers to return twenty markers, which
/// is the one way a paged spatial query quietly becomes a whole-image read.
///
/// **And the box bounds it too.** Seeking to one corner and stepping to the other
/// reads the Morton INTERVAL, which is a superset the box threads in and out of —
/// on a map's wide, shallow viewport, 1.46 M records to return 4 k of them
/// (@PLN136). `box_walk` seeks over each gap instead, so the walk reads what it
/// returns and one viewport costs single-digit pages.
pub fn spatial_box_recs<P: PageProvider>(
    reader: &mut PagedReader<P>,
    root_rec: u32,
    root_pos: u32,
    from: &[i64],
    till: &[i64],
    keys: &[crate::keys::Key],
    limit: Option<usize>,
) -> Vec<u32> {
    use crate::radix_db::MAX_AXES;
    let Some(keys) = spatial_keys(keys) else {
        return Vec::new();
    };
    let n = keys.len();
    if from.len() < n || till.len() < n {
        return Vec::new();
    }
    let mut src = PagedSpatial::open(reader, root_rec, root_pos, &keys);
    if src.tree == 0 {
        return Vec::new();
    }
    let (mut qlo, mut qhi) = ([0u64; MAX_AXES], [0u64; MAX_AXES]);
    for a in 0..n {
        // A corner-swapped axis names the same interval, as the resident surface
        // reads it.
        let (lo, hi) = if from[a] <= till[a] {
            (from[a], till[a])
        } else {
            (till[a], from[a])
        };
        qlo[a] = crate::radix_db::coord_code(lo);
        qhi[a] = crate::radix_db::coord_code(hi);
    }
    crate::radix_db::box_walk(&mut src, n, &qlo, &qhi, limit.unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory provider over a byte image — hermetic, and logs fetches like
    /// the file provider so residency/bounded-fetch tests need no disk.
    struct VecProvider {
        bytes: Vec<u8>,
        fetches: Vec<(u64, usize)>,
    }
    impl VecProvider {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                fetches: Vec::new(),
            }
        }
        fn pages_fetched(&self) -> usize {
            self.fetches.len()
        }
    }
    impl PageProvider for VecProvider {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
            self.fetches.push((off, len));
            let mut buf = vec![0u8; len];
            let start = off as usize;
            if start < self.bytes.len() {
                let end = (start + len).min(self.bytes.len());
                buf[..end - start].copy_from_slice(&self.bytes[start..end]);
            }
            buf
        }
    }

    fn ramp(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    /// loft#729 — a read covering a run of absent pages costs ONE provider call,
    /// and carries exactly the same bytes.
    ///
    /// The bytes are asserted alongside the call count on purpose: a coalescer
    /// that mis-slices the returned buffer into the page table would still show
    /// the improved count, so counting alone would report the bug as a success.
    #[test]
    fn a_run_of_absent_pages_is_one_request_carrying_the_same_bytes() {
        let img = ramp(1600);
        let mut coalesced = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
        let got = coalesced.resolve(0, 1600);
        assert_eq!(got, img, "coalescing must not disturb the bytes");
        assert_eq!(
            coalesced.provider().pages_fetched(),
            1,
            "100 consecutive absent pages are one run, so one request"
        );

        // Resident pages are not re-fetched: seed page 3, then read across it.
        let mut partial = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
        let _ = partial.resolve(48, 16); // page 3 alone
        assert_eq!(partial.provider().pages_fetched(), 1);
        let got = partial.resolve(0, 160);
        assert_eq!(got, &img[0..160], "the seeded page must still read right");
        assert_eq!(
            partial.provider().pages_fetched(),
            3,
            "the seeded page splits the run in two, so two more requests"
        );
    }

    #[test]
    fn resolve_returns_exact_bytes_including_page_spans() {
        let img = ramp(1000);
        // Tiny 16-byte pages so most reads straddle a boundary.
        let mut r = PagedReader::with_config(VecProvider::new(img.clone()), 16, 64);
        // every offset/len combo must equal the source slice
        for &off in &[0u64, 1, 15, 16, 17, 31, 100, 500, 984] {
            for &len in &[1usize, 4, 8, 16, 33, 64] {
                if off as usize + len > img.len() {
                    continue;
                }
                let got = r.resolve(off, len);
                assert_eq!(
                    got,
                    &img[off as usize..off as usize + len],
                    "resolve({off},{len}) mismatch"
                );
            }
        }
    }

    #[test]
    fn resolve_past_eof_zero_pads() {
        let img = ramp(20);
        let mut r = PagedReader::with_config(VecProvider::new(img.clone()), 16, 64);
        let got = r.resolve(12, 16); // 12..28, but only 12..20 exists
        assert_eq!(&got[..8], &img[12..20]);
        assert!(
            got[8..].iter().all(|&b| b == 0),
            "tail past EOF must be zero"
        );
    }

    #[test]
    fn only_touched_pages_are_fetched() {
        // 100 pages of 16 bytes = 1600 bytes; read one word near the end.
        let mut r = PagedReader::with_config(VecProvider::new(ramp(1600)), 16, 64);
        let _ = r.u32_at(50, 0); // byte 400 → page 25 only
        // Exactly one page fetched, not the whole file.
        assert_eq!(
            r.provider().pages_fetched(),
            1,
            "a single small read must touch one page, not the file"
        );
    }

    #[test]
    fn correctness_is_independent_of_residency() {
        // capacity == 1: every read past the current page re-fetches, but the
        // bytes must still be correct — proves correctness ⟂ cache size.
        let img = ramp(2000);
        let mut small = PagedReader::with_config(VecProvider::new(img.clone()), 16, 1);
        let mut big = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
        for &off in &[0u64, 100, 500, 1000, 1500, 1984] {
            assert_eq!(
                small.resolve(off, 12),
                big.resolve(off, 12),
                "capacity-1 reader must agree with a fully-resident one at {off}"
            );
        }
        // The tiny cache never holds more than its cap.
        assert!(small.pages.len() <= 1);
    }

    #[test]
    fn typed_reads_are_native_endian_at_rec_times_8() {
        // Lay out a u32 at (rec=3, fld=4) → byte 28, and an i64 at (rec=5,fld=0)
        // → byte 40, then read them back through the paged reader.
        let mut img = vec![0u8; 64];
        img[28..32].copy_from_slice(&0xDEAD_BEEFu32.to_ne_bytes());
        img[40..48].copy_from_slice(&(-1_234_567_890_123i64).to_ne_bytes());
        let mut r = PagedReader::with_config(VecProvider::new(img), 16, 64);
        assert_eq!(r.u32_at(3, 4), 0xDEAD_BEEF);
        assert_eq!(r.i64_at(5, 0), -1_234_567_890_123i64);
    }

    /// loft#787 — a run is split into pages by COPYING one page each.
    ///
    /// `bytes[lo..].to_vec()` followed by `truncate` copied every byte after `lo` and threw
    /// most of it away, so an n-page run copied n(n+1)/2 pages to keep n. The values were
    /// right, which is why it survived: only the volume was wrong. This pins the values a
    /// multi-page run yields, so a future rewrite of the split cannot silently mis-slice.
    #[test]
    fn a_multi_page_run_yields_the_right_page_contents() {
        // 6 pages of 16 bytes, each filled with its own page index.
        let img: Vec<u8> = (0..6u8).flat_map(|p| std::iter::repeat_n(p, 16)).collect();
        let mut r = PagedReader::with_config(VecProvider::new(img), 16, 999);
        r.warm(&[(0, 6 * 16)]);
        for p in 0..6u8 {
            let got = r.resolve(u64::from(p) * 16, 16);
            assert_eq!(
                got,
                vec![p; 16],
                "page {p} must hold its own bytes after a 6-page run was split"
            );
        }
    }

    /// loft#787 — the alloc-free small read must answer exactly what `resolve` answers.
    ///
    /// `read_into` splits on whether the read fits inside one page, and the typed accessors
    /// take the fast branch for 1/2/4/8 bytes. A boundary error there would be invisible on
    /// most offsets and wrong on a few — the worst failure shape available — so this sweeps
    /// EVERY offset across a page seam at every accessor width and compares against the
    /// owning path. Tiny pages (16 B) put a seam every other read; the real 64 KiB page would
    /// hide the bug behind alignment.
    #[test]
    fn the_alloc_free_read_agrees_with_the_owning_one_across_a_page_seam() {
        let img: Vec<u8> = (0..96u8).collect();
        for len in [1usize, 2, 4, 8, 12, 20] {
            for off in 0..(96 - len as u64) {
                let mut fast = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
                let mut slow = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
                let mut buf = vec![0u8; len];
                fast.read_into(off, &mut buf);
                assert_eq!(
                    buf,
                    slow.resolve(off, len),
                    "read_into({off}, {len}) must equal resolve — page size 16, seam at {}",
                    off % 16
                );
            }
        }
        // …and the typed accessors, which is where the fast path is actually taken.
        let mut r = PagedReader::with_config(VecProvider::new(img.clone()), 16, 999);
        let mut plain = PagedReader::with_config(VecProvider::new(img), 16, 999);
        for rec in 0..10u32 {
            for fld in [0u32, 4] {
                let b = plain.resolve(u64::from(rec) * 8 + u64::from(fld), 4);
                assert_eq!(
                    r.u32_at(rec, fld),
                    u32::from_ne_bytes([b[0], b[1], b[2], b[3]]),
                    "u32_at({rec}, {fld})"
                );
            }
            let b = plain.resolve(u64::from(rec) * 8, 8);
            assert_eq!(
                r.i64_at(rec, 0),
                i64::from_ne_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
                "i64_at({rec}, 0)"
            );
        }
    }

    /// loft#787 — an UNCAPPED reader keeps no recency list, because nothing evicts.
    ///
    /// The list is a victim queue and nothing else. With no cap there is no victim, so every
    /// entry is dead weight — and maintaining it cost an O(n) scan plus an O(n) deque
    /// remove on EVERY page hit, on the reader's hottest path. Bounded while the cache was
    /// 64 pages; loft#785 removed the cap and n became the working set.
    ///
    /// Asserting the list stays empty is asserting the work is not done at all — a timing
    /// test would be noise, and the point is the absence of an operation, not its speed.
    #[test]
    fn an_uncapped_reader_keeps_no_recency_list() {
        let img: Vec<u8> = (0..64u8).flat_map(|p| std::iter::repeat_n(p, 16)).collect();
        let mut r = PagedReader::new(VecProvider::new(img));
        assert!(!r.capped, "the default reader is uncapped (loft#785)");
        for p in 0..8u64 {
            r.resolve(p * 16, 16);
            r.resolve(p * 16, 16); // a HIT: the path that used to scan
        }
        assert!(
            r.lru.is_empty(),
            "an uncapped reader must not maintain recency — nothing consumes it"
        );

        // …and a capped one still does, or eviction has nothing to choose by.
        let img2: Vec<u8> = (0..64u8).flat_map(|p| std::iter::repeat_n(p, 16)).collect();
        let mut c = PagedReader::with_config(VecProvider::new(img2), 16, 4);
        for p in 0..8u64 {
            c.resolve(p * 16, 16);
        }
        assert!(
            !c.lru.is_empty() && c.pages.len() <= 4,
            "a capped reader keeps its victim queue and honours the cap"
        );
    }

    /// loft#787 — a transport that cannot batch is not asked to prefetch.
    ///
    /// `warm` exists to turn N round trips into one. A provider whose `fetch_many` is a
    /// sequential loop collects none of that and pays the whole computation — page set,
    /// sort, dedup, run building, split. Asserting NO fetch happens is asserting the work
    /// is skipped entirely; the demand path then fetches the same pages when they are
    /// actually read, which the second half checks.
    #[test]
    fn a_non_batching_provider_is_not_prefetched() {
        struct Sequential(VecProvider);
        impl PageProvider for Sequential {
            fn size(&self) -> u64 {
                self.0.size()
            }
            fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
                self.0.fetch(off, len)
            }
            fn batches(&self) -> bool {
                false
            }
        }
        let img: Vec<u8> = (0..8u8).flat_map(|p| std::iter::repeat_n(p, 16)).collect();
        let mut r = PagedReader::with_config(Sequential(VecProvider::new(img)), 16, 999);
        r.warm(&[(0, 8 * 16)]);
        assert!(
            r.pages.is_empty(),
            "a non-batching provider must not be prefetched — the work buys it nothing"
        );
        // …and the pages still arrive, on demand, with the right bytes.
        for p in 0..8u8 {
            assert_eq!(r.resolve(u64::from(p) * 16, 16), vec![p; 16]);
        }
    }

    /// …while a batching one still is, or the round-trip saving is lost everywhere.
    #[test]
    fn a_batching_provider_is_still_prefetched() {
        let img: Vec<u8> = (0..8u8).flat_map(|p| std::iter::repeat_n(p, 16)).collect();
        let mut r = PagedReader::with_config(VecProvider::new(img), 16, 999);
        r.warm(&[(0, 8 * 16)]);
        assert_eq!(r.pages.len(), 8, "a batching provider keeps its prefetch");
    }

    /// loft#787 — the default is UNCAPPED on every target, wasm included.
    ///
    /// This test asserted the opposite for one commit, and the measurement that reversed it
    /// is the thing worth pinning. The argument for a wasm-only cap was that an eviction
    /// there costs only a copy out of a host buffer, while linear memory is the scarce
    /// resource — true, and still the wrong conclusion, because it priced the eviction and
    /// not the RE-FETCH that follows it. In a real browser, on the app that reported this
    /// issue: 4 MiB of cache against a 5.6 MB working set took range reads from 71 to 115
    /// and bytes from 5.7 MB to 8.5 MB, and saved 1 MB of a 17 MB heap.
    ///
    /// So the cap stays opt-in, and a future "the browser should hold less" proposal has to
    /// answer the re-fetch count before it lands.
    #[test]
    fn the_default_is_uncapped_on_every_target() {
        let img = vec![7u8; 4096];
        let r = PagedReader::new(VecProvider::new(img));
        assert!(
            !r.capped,
            "an unasked-for default is a floor, not a bound (loft#785) — and a wasm-only \
             cap re-fetches more than it saves (loft#787)"
        );
    }
}
