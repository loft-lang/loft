# WASM host media — PNG decoding and console logging

Split out of [WASM.md](WASM.md): PNG decoding and console logging under the `wasm` feature.

## PNG Image Support in WASM

### Why it works

The `png` crate (v0.17) is pure Rust with no OS dependencies. It implements the full
PNG decode pipeline (inflate, filtering, interlacing) in safe Rust. This compiles to
WASM without modification.

### Adaptation: buffer-based decoding

The only blocker is that `png_store::read()` opens a file via `std::fs::File::open()`.
Under `#[cfg(feature = "wasm")]`, it reads from the VirtFS instead:

```rust
// src/png_store.rs

#[cfg(not(feature = "wasm"))]
pub fn read(path: &str, store: &mut Store) -> Result<(u32, u32, u32)> {
    let file = std::fs::File::open(path)?;
    let decoder = png::Decoder::new(file);
    decode_into_store(decoder, store)
}

#[cfg(feature = "wasm")]
pub fn read(path: &str, store: &mut Store) -> Result<(u32, u32, u32)> {
    let bytes = crate::wasm::host_read_binary(path)
        .ok_or_else(|| anyhow!("file not found: {path}"))?;
    let cursor = std::io::Cursor::new(bytes);
    let decoder = png::Decoder::new(cursor);
    decode_into_store(decoder, store)
}

// Shared decode logic — works with any std::io::Read source
fn decode_into_store<R: std::io::Read>(
    decoder: png::Decoder<R>,
    store: &mut Store,
) -> Result<(u32, u32, u32)> {
    let img = store.claim((reader.output_buffer_size() / 8) as u32 + 1);
    let info = reader.next_frame(store.buffer(img))?;
    Ok((img, info.width, info.height))
}
```

The key insight: `png::Decoder<R>` is generic over `R: Read`. Swapping
`File` for `Cursor<Vec<u8>>` requires no changes to the decode logic.

### Browser workflow

1. User drags a PNG file onto the IDE, or the PNG is in the VirtFS (base tree or
   delta).
2. The VirtFS stores it as a binary node (`"$type": "binary"`, base64 content).
3. When the loft program calls `file("image.png").png()`, the WASM bridge reads the
   binary bytes from VirtFS and passes them to `png::Decoder` via a `Cursor`.
4. Decoded pixels land in the Store heap as usual — the `Image` struct works
   identically.

### Browser-to-loft PNG import via drag-and-drop

```js
// In the IDE: handle dropped PNG files
dropZone.ondrop = async (e) => {
  for (const file of e.dataTransfer.files) {
    if (file.name.endsWith('.png')) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      fs.writeBinary(`/project/${file.name}`, bytes);
    }
  }
};
```

### Displaying Image output in the browser

The reverse direction — loft `Image` → visible in the IDE — can be handled by a
host bridge that receives pixel data and renders to a `<canvas>`:

```js
globalThis.loftHost = {
  // ...
  display_image(width, height, pixels) {
    // pixels: Uint8Array of RGB triplets
    const canvas = document.getElementById('output-canvas');
    const ctx = canvas.getContext('2d');
    canvas.width = width;
    canvas.height = height;
    const imageData = ctx.createImageData(width, height);
    for (let i = 0, j = 0; i < pixels.length; i += 3, j += 4) {
      imageData.data[j]     = pixels[i];     // R
      imageData.data[j + 1] = pixels[i + 1]; // G
      imageData.data[j + 2] = pixels[i + 2]; // B
      imageData.data[j + 3] = 255;           // A
    }
    ctx.putImageData(imageData, 0, 0);
  }
};
```

This is optional — PNG decoding works without it. Display is an IDE convenience.

---

## Logging in WASM

### Problem

The loft logger (`src/logger.rs`) writes to files: it creates log directories,
rotates log files, and archives old entries. None of this makes sense in a browser.

### Design: console-only logging under `#[cfg(feature = "wasm")]`

All log output goes to the JavaScript console via the host bridge. No file I/O, no
rotation, no directories.

**Rust changes in `src/logger.rs`:**

```rust
#[cfg(feature = "wasm")]
fn write_log_entry(level: Level, message: &str) {
    // Call JS host to write to console
    crate::wasm::host_log_write(level.as_str(), message);
}

#[cfg(not(feature = "wasm"))]
fn write_log_entry(level: Level, message: &str) {
    // Existing file-based logging implementation
    // ...
}
```

**Conditional compilation gates:**

```rust
// Skip all file-based setup in WASM
#[cfg(not(feature = "wasm"))]
fn ensure_log_dir() { /* create directory, rotate files */ }

#[cfg(feature = "wasm")]
fn ensure_log_dir() { /* no-op */ }
```

**JS host implementation:**

```js
// Browser
globalThis.loftHost = {
  log_write(level, message) {
    switch (level) {
      case 'info':  console.info(`[loft] ${message}`);  break;
      case 'warn':  console.warn(`[loft] ${message}`);  break;
      case 'error': console.error(`[loft] ${message}`); break;
      case 'fatal': console.error(`[loft FATAL] ${message}`); break;
    }
  }
};

// Node.js — identical, console methods work the same
```

**Loft-side behaviour:**

- `log_info()`, `log_warn()`, `log_error()`, `log_fatal()` all work.
- `log_config()` is accepted but has no effect (no file to configure).
- Rate limiting still applies — implemented in Rust, not in the file layer.

### IDE integration (optional)

The IDE can capture log output in a dedicated "Log" panel instead of (or alongside)
the browser console:

```js
const logEntries = [];
globalThis.loftHost = {
  log_write(level, message) {
    logEntries.push({ level, message, time: Date.now() });
    renderLogPanel(logEntries);         // update UI
    console[level === 'fatal' ? 'error' : level](`[loft] ${message}`);
  }
};
```
