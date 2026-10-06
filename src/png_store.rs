// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (png-in-store)

//! Opening png images inside a store
use crate::store::Store;
use png::Decoder;
#[cfg(not(feature = "wasm"))]
use std::fs::File;
#[cfg(not(feature = "wasm"))]
use std::io::BufReader;

/// Decode a PNG from any `Read` source into the store.
/// Returns `(img_record, width, height)`.
fn decode_into_store<R: std::io::Read>(
    reader: R,
    store: &mut Store,
) -> std::io::Result<(u32, u32, u32)> {
    let decoder = Decoder::new(reader);
    let mut r = decoder.read_info()?;
    let buf_size = r.output_buffer_size();
    // Two words of header: the record's own `[size:4][length:4]`, which `Store::buffer`
    // steps over, and the 8 bytes this decoder then skips itself — so the pixels start at
    // byte 16 of the record.  That second skip is this file's own convention and nothing
    // reads the record as a loft vector, so loft#970 leaves it alone; what it fixes is the
    // ROUNDING.  `buf_size / 8` truncates, and the slack that hid it was `Store::buffer`
    // returning one word too many.  With `buffer` corrected the last partial word has to
    // be claimed here, or a PNG whose byte count is not a multiple of 8 decodes into a
    // buffer up to seven bytes short.
    let img = store.claim(u32::try_from(buf_size.div_ceil(8)).unwrap_or(u32::MAX) + 2);
    let pixel_count = buf_size / 3; // 3 bytes per Pixel (r, g, b as u8)
    store.set_int(img, 4, pixel_count as i64);
    // Decode PNG directly into offset 8 (after the vector header).
    let buf = store.buffer(img);
    let header_bytes = 8;
    if buf.len() >= header_bytes + buf_size {
        let info = r.next_frame(&mut buf[header_bytes..])?;
        Ok((img, info.width, info.height))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "PNG buffer too small",
        ))
    }
}

/// Read a PNG from the filesystem (native path).
#[cfg(not(feature = "wasm"))]
pub fn read(file_path: &str, store: &mut Store) -> std::io::Result<(u32, u32, u32)> {
    let file = crate::file_access::open(&crate::file_access::at(file_path))?;
    decode_into_store(BufReader::new(file), store)
}

/// Read a PNG via the WASM host bridge (wasm path).
#[cfg(feature = "wasm")]
pub fn read(file_path: &str, store: &mut Store) -> std::io::Result<(u32, u32, u32)> {
    let bytes = crate::wasm::host_fs_read_binary(file_path).unwrap_or_default();
    decode_into_store(std::io::Cursor::new(bytes), store)
}

/// loft#970 — a PNG whose decoded size is not a whole number of words.
///
/// `map.png` is 256x256x3 = 196 608 bytes, which is divisible by 8, so it cannot witness
/// the rounding: `buf_size / 8` truncates only when there is a remainder.  Every byte of
/// slack that used to cover that truncation came from `Store::buffer` returning one word
/// too many, and correcting it (which is what loft#970 is) takes the slack away.
///
/// 3x3 RGB is 27 bytes — 3 short of a word — so this cell fails with the old
/// `(buf_size / 8) + 2` claim and passes with `div_ceil`.  It asserts the CORNER PIXELS,
/// not just that decoding returned: a buffer three bytes short loses the end of the image,
/// which a success-only check would not see.
#[test]
#[cfg(feature = "png")]
fn a_png_whose_size_is_not_a_whole_word_decodes_completely() {
    let mut store = Store::new(64);
    let (img, w, h) = read("tests/example/rgb3x3.png", &mut store).expect("decode 3x3 png");
    assert_eq!((w, h), (3, 3));
    // Pixels start at byte 16 of the record: the record's own header word, then the 8 bytes
    // this decoder skips itself (see `decode_into_store`).
    let at = |i: u32| {
        (
            store.get_byte(img, 16 + i * 3, 0),
            store.get_byte(img, 17 + i * 3, 0),
            store.get_byte(img, 18 + i * 3, 0),
        )
    };
    assert_eq!(at(0), (255, 0, 0), "first pixel");
    // The last pixel is the one a short buffer drops.
    assert_eq!(
        at(8),
        (70, 80, 90),
        "last pixel — a truncated buffer loses this"
    );
}

#[test]
fn show_png() {
    let mut store = Store::new(12 + 256 * 256 * 3 / 8);
    let (img, w, _h) = read("tests/example/map.png", &mut store).unwrap();
    let mut count = 0;
    for y in 0..128 {
        if y % 2 == 0 {
            continue;
        }
        for x in 0..128 {
            // Vector header is 8 bytes = 16 store units; each pixel is 3 bytes = 6 units.
            if store.get_byte(img, 16 + (x + y * w) * 6, 0) > 0 {
                //print!("x");
                count += 10;
            } else if store.get_byte(img, 17 + (x + y * w) * 6, 0) > 0 {
                //print!("b");
                count += 11;
            } else {
                //print!(".");
                count += 1;
            }
        }
        //println!();
    }
    assert_eq!(34972, count);
}
