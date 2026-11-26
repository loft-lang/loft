<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 230 — Zero-trust paged stores: pages that are encrypted and authenticated, read as cheaply as a mapped file

## Status

Future.  [loft-lang/plans#230](https://github.com/loft-lang/plans/issues/230) is canonical.
Nothing is built; the first phase is a probe.

## Goal

A paged store — `store_load_key`, `store_load_keys`, `store_load_key_text`,
`store_load_keys_text` and `store_load_range` over HTTP `Range` — that anyone may host.  Each
page is encrypted and authenticated, so neither the server nor the network path is trusted,
and reading a page stays as cheap as a mapped read.

Today a paged read trusts its server the way a mapped file trusts its filesystem;
`store_verify` keeps a bad page from sending a read
outside its own data, and `store_load_url` pins a whole image for a source one does not
control.  Any check short of encrypting the data moves the trust somewhere else.  This plan
removes it.

## Why this shape

- **The check is the decryption.**  A per-page hash table checked on every read makes a paged
  read a fetch-and-verify, which a store read like a mapped file cannot carry.  A confidential
  read decrypts every page anyway, and AES-GCM's tag is the integrity check of that same
  pass: nothing is added per page beyond the decryption.
- **The key schedule exists.**  The zero-trust shared-files project encrypts large files per
  chunk: `chunk_dek[i] = HKDF-SHA256(manifest_dek, info = <label> || u32_be(i))`, AES-256-GCM,
  a fresh `manifest_dek` per version (its `protocol/SPEC.md` §5–6).  A page from another
  version does not decrypt under this version's key, so an old page cannot be served again.
- **The browser can wait on a decryption.**  WebCrypto is asynchronous and the paged reader is
  synchronous, but every range read in the browser already suspends the wasm until its
  `fetch()` resolves (the `loft_host_http_range` bridge, asyncify).  Decrypting is one more
  step in that promise chain, on the browser's own hardware AES.
- **The page size fits a real consumer.**  The routing app reads 64 kB pages by range from
  GitHub Pages, with its own page index and an IndexedDB page cache.

## The format (to be settled by P1)

- A header, signed once per version: image id, version, page size, page count, plaintext size,
  and the wrapped image key (or a key id, by P4).
- Page `i` at `header_len + i × (page + 16)`: AES-256-GCM of plaintext page `i` under
  `page_key[i] = HKDF-SHA256(image_key, "loft-store/v1/page/" || u64_be(i))`, nonce zero (a
  key encrypts one page), AAD = image id ‖ version.  Offsets stay arithmetic, so a range read
  stays one request.
- The `.dschema` layout sidecar is carried inside the signed header, not beside the image.

## Phases

Each phase has a control that must go red.

| Phase | Deliverable | Validated by (what goes red) |
|---|---|---|
| P0 probe | The routing app's 64 kB page path with each page decrypted in the `loft_host_http_range` promise chain (WebCrypto), against the plain read | A cold viewport measured both ways; the plan stops if decryption costs more than the fetch it rides on |
| P1 format + seal | The format above, written by `loft store seal <image>`; the reader in `src/paged_reader.rs` (AES-GCM from `ring`, already linked for TLS) | A page with one flipped bit, a page of another version and a page at another index each refuse the read; a plain image loads as before |
| P2 C runtime | The same reader in `src/c/loft_paged_load.h` | The P1 cells on `--native`; the AES-GCM implementation decided (P2's open question) |
| P3 browser | The bridge decrypts before it resumes | The P1 cells on `--html`; the routing app's paged gate |
| P4 keys | Where the reader's key comes from | Decided before P1's header is frozen |
| P5 readers who hold the key | Each page's GCM tag listed in the signed header, compared on read | A page forged by a key holder is refused; decided by the owner whether one-writer, many-reader stores need it |

## Open questions

- **P2:** the C runtime has no AES.  BearSSL (vendored by the crypto library), a runtime
  component, or a small AES-GCM of loft's own.
- **P4:** a symmetric key passed to the loader, or the shared-files identity (an HPKE-wrapped
  image key).  An optional trailing parameter keeps every existing call unchanged.
- **The page cache:** a browser cache (IndexedDB) that keeps pages sealed is checked on every
  read for free; one that keeps them decrypted is trusted storage.

## Out of scope

- Plain (unsealed) paged stores keep working exactly as now.
- Write-in-place of a sealed image: a new version is a new seal.

## See also

- [@PLN84](84-zero-trust-libs.md) — the zero-trust libraries (crypto, cbor) this builds on.
- [REMOTE_STORES.md](../REMOTE_STORES.md), [LAZY_STORES.md](../LAZY_STORES.md).
- `src/paged_reader.rs`, `src/c/loft_paged_load.h`.
