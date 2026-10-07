// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! GL-render gold-image regression for the crystal editor smoke
//! mode (`tools/audience-demo/crystal_editor.loft --smoke`).
//!
//! Drives the real GL pipeline under Xvfb + llvmpipe so the render is
//! CPU-deterministic and matches CI rather than the dev box's GPU.
//! Looser tolerance than the canvas gold-image tests (those live in
//! `lib/graphics/native/tests/gold.rs` so they travel with the
//! library) — GL line/triangle AA can drift a few LSB across Mesa
//! versions, but the test still catches beams / ground / palette
//! gone, mispositioned, or recoloured.
//!
//! Skips (does not fail) when xvfb-run or a working software-GL context is
//! unavailable — unless `LOFT_REQUIRE_GL_GOLD=1`, which CI's Linux leg sets
//! because it installs both, so a skip there is a retired guard and not an
//! absent display.  A program that does not COMPILE is never a skip.  Updating:
//!
//!   UPDATE_GOLD=1 cargo test --test crystal_editor_gold
//!
//! References `tools/audience-demo/crystal_editor.loft` — an
//! audience-demo tool, not a library — so this test stays in the loft
//! repo, not in any extracted library chunk.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Skip this environment, or fail where CI promised the environment exists.
///
/// The guard used to gate on `lib/graphics/native/…/libloft_graphics_native.so`,
/// a path that left this repository with the graphics library.  Every run then
/// skipped at that first line — CI's included, although CI installs Xvfb and
/// Mesa for this test alone — and the demo stopped compiling unnoticed.
///
/// Unset, empty or `0` means NOT required: ci.yml sets the variable on every OS and gives it a
/// value only on Linux (`runner.os == 'Linux' && '1' || ''`), where Xvfb is installed.
fn skip(reason: &str) {
    assert!(
        std::env::var_os("LOFT_REQUIRE_GL_GOLD").is_none_or(|v| v.is_empty() || v == "0"),
        "crystal GL gold did not run, and LOFT_REQUIRE_GL_GOLD says it must: {reason}"
    );
    eprintln!("skipping crystal GL gold: {reason}");
}

fn has_cmd(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn update_gold() -> bool {
    std::env::var_os("UPDATE_GOLD").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Decode a PNG into an (rgba, width, height) tuple.
fn decode_rgba8(path: &std::path::Path) -> (Vec<u8>, u32, u32) {
    let file = fa::open(path).unwrap_or_else(|e| panic!("opening {}: {e}", path.display()));
    let decoder = png::Decoder::new(file);
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("reading info for {}: {e}", path.display()));
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("decoding frame of {}: {e}", path.display()));
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(buf.len() / 3 * 4);
            for chunk in buf.as_chunks::<3>().0 {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            out
        }
        other => panic!(
            "{}: unsupported color type {other:?} (expected RGB or RGBA)",
            path.display()
        ),
    };
    (rgba, w, h)
}

struct DiffReport {
    max_abs: u32,
    /// The largest channel difference once each pixel may match any gold pixel within one
    /// pixel of it: two software rasterizers place a thin line's edge a pixel apart, while
    /// missing or wrong CONTENT has no gold pixel near it that matches.
    max_abs_shifted: u32,
    mean_abs: f64,
    differing_pixels: u64,
    total_pixels: u64,
}

/// The largest channel difference between pixel `i` of `a` and the closest pixel of `b` in its
/// 3x3 neighbourhood (`w` pixels per row).
fn shifted_diff(a: &[u8], b: &[u8], w: usize, i: usize) -> u32 {
    let h = a.len() / 4 / w;
    let (x, y) = ((i % w) as isize, (i / w) as isize);
    let mut best = u32::MAX;
    for dy in -1..=1isize {
        for dx in -1..=1isize {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                continue;
            }
            let j = (ny as usize * w + nx as usize) * 4;
            let d = (0..4)
                .map(|c| u32::from(a[i * 4 + c].abs_diff(b[j + c])))
                .max()
                .unwrap_or(0);
            best = best.min(d);
        }
    }
    best
}

fn compare_rgba(a: &[u8], b: &[u8], w: usize) -> DiffReport {
    assert_eq!(a.len(), b.len(), "rgba buffers have different lengths");
    let mut max_abs = 0u32;
    let mut sum_abs = 0u64;
    let mut differing_pixels = 0u64;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let mut pixel_diff = 0u32;
        for (x, y) in p.iter().zip(q.iter()) {
            let d = x.abs_diff(*y) as u32;
            if d > max_abs {
                max_abs = d;
            }
            sum_abs += d as u64;
            pixel_diff += d;
        }
        if pixel_diff > 0 {
            differing_pixels += 1;
        }
    }
    let total_pixels = (a.len() / 4) as u64;
    let channel_count = a.len() as f64;
    let max_abs_shifted = if max_abs == 0 {
        0
    } else {
        // Both directions: every actual pixel near a matching gold pixel AND every gold pixel
        // near a matching actual one — one way only, a MISSING thin stroke passes, because
        // each blank pixel where it should be has a blank gold neighbour.
        (0..total_pixels as usize)
            .map(|i| shifted_diff(a, b, w, i).max(shifted_diff(b, a, w, i)))
            .max()
            .unwrap_or(0)
    };
    DiffReport {
        max_abs,
        max_abs_shifted,
        mean_abs: sum_abs as f64 / channel_count,
        differing_pixels,
        total_pixels,
    }
}

#[test]
fn crystal_editor_gl_matches_gold() {
    if !has_cmd("xvfb-run") {
        skip("xvfb-run not installed");
        return;
    }
    let root = workspace_root();
    // A program that does not compile produces no screenshot either, and the
    // screenshot check below reads that as "no software GL".  Ask first.
    let check = Command::new(loft_bin())
        .args(["--no-warnings", "--path"])
        .arg(format!("{}/", root.display()))
        .arg("--lib")
        .arg(root.join("lib"))
        .args(["--check", "tools/audience-demo/crystal_editor.loft"])
        .current_dir(&root)
        .output()
        .expect("invoke loft --check");
    assert!(
        check.status.success(),
        "tools/audience-demo/crystal_editor.loft does not compile:\n{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let shot = PathBuf::from("/tmp/crystal_editor_gold.png");
    let _ = fa::remove_file(&shot);
    let out = Command::new("xvfb-run")
        .args([
            "-a",
            "-s",
            "-screen 0 1000x1000x24",
            "env",
            // On a Wayland session `xvfb-run` only sets DISPLAY (X11), but
            // winit/glutin prefer Wayland and would connect to the REAL
            // compositor — popping a visible window on the user's screen and
            // bypassing Xvfb (and breaking truly-headless runs).  Unset
            // WAYLAND_DISPLAY and pin the winit backend to x11 so the window
            // lands on the virtual Xvfb display instead.
            "-u",
            "WAYLAND_DISPLAY",
            "WINIT_UNIX_BACKEND=x11",
            "LIBGL_ALWAYS_SOFTWARE=1",
            "GALLIUM_DRIVER=llvmpipe",
            // @PLN11 N3 Step 3 — default-native is now on, so a `use`d library
            // (here `audience_crystal`) would auto-build a cdylib.  This is a
            // dev/CI gold-image test, not a native-dispatch test, and native↔interp
            // is parity-guaranteed; interpret the library to keep the run fast and
            // avoid writing a `native-auto/` into the source tree.
            "LOFT_NO_NATIVE_LIBS=1",
        ])
        .arg(loft_bin())
        .arg("--no-warnings")
        .arg("--path")
        .arg(format!("{}/", root.display()))
        .arg("--lib")
        .arg(root.join("lib"))
        .arg("tools/audience-demo/crystal_editor.loft")
        .arg("--smoke")
        .arg("--screenshot")
        .arg(&shot)
        .current_dir(&root)
        .output()
        .expect("invoke xvfb-run");

    if !fa::exists(&shot) {
        // No framebuffer captured — almost always a missing software-GL
        // context in this environment, not a rendering regression.  Skip.
        skip(&format!(
            "no screenshot produced (software GL unavailable?)\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
        return;
    }

    let gold = root.join("tests/gold").join("crystal-editor-gl.png");
    if update_gold() {
        fa::copy(&shot, &gold).expect("copying new GL gold");
        eprintln!("UPDATE_GOLD=1: wrote {}", gold.display());
        return;
    }
    assert!(
        fa::exists(&gold),
        "GL gold missing: {}\nrun `UPDATE_GOLD=1 cargo test --test crystal_editor_gold`",
        gold.display()
    );
    let (actual, aw, ah) = decode_rgba8(&shot);
    let (expected, ew, eh) = decode_rgba8(&gold);
    // One accepted render PER RENDERER: `crystal-editor-gl.png` is this repo's reference, and
    // `crystal-editor-gl.<host>.png` beside it is the same scene on another software GL (the
    // CI runner's llvmpipe rasterizes stroke edges and blends differently in 876 of 1e6
    // pixels, the content identical).  Each is held to the same strict limits, so a missing
    // or wrong element fails against every one of them.
    let mut accepted: Vec<(String, Vec<u8>)> = vec![("crystal-editor-gl.png".into(), expected)];
    if let Ok(dir) = std::fs::read_dir(root.join("tests/gold")) {
        let mut others: Vec<PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("crystal-editor-gl.")
                        && n.ends_with(".png")
                        && n != "crystal-editor-gl.png"
                })
            })
            .collect();
        others.sort();
        for p in others {
            let (px, w, h) = decode_rgba8(&p);
            if (w, h) == (ew, eh) {
                accepted.push((p.file_name().unwrap().to_string_lossy().into_owned(), px));
            }
        }
    }
    // @P348 — HiDPI / display-scaled environments can hand the GL window a
    // SCALED framebuffer (observed 1333x1333 = 1000 × 1.333) even under
    // `xvfb-run`.  The controlled `make test-gl-golden` path (fixed Xvfb
    // screen) and CI always produce the exact gold size, so a dimension
    // mismatch here is environmental — skip gracefully.
    if (aw, ah) != (ew, eh) {
        skip(&format!(
            "framebuffer {aw}x{ah} != gold {ew}x{eh} \
             (HiDPI/display-scaled environment — run via `make test-gl-golden` for a controlled size)"
        ));
        return;
    }
    let (max_abs, mean_abs) = (16u32, 2.0f64);
    // The one-pixel measure, not the raw one: a thin line's edge may still land a pixel over
    // between two runs of one renderer.  The mean stays over the raw difference.
    let (best, diff) = accepted
        .iter()
        .map(|(name, px)| (name.clone(), compare_rgba(&actual, px, aw as usize)))
        .min_by(|a, b| {
            (a.1.max_abs_shifted, a.1.mean_abs)
                .partial_cmp(&(b.1.max_abs_shifted, b.1.mean_abs))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("at least the reference gold");
    assert!(
        diff.max_abs_shifted <= max_abs && diff.mean_abs <= mean_abs,
        "crystal GL gold mismatch (closest of {} accepted renders: {best}):\n  max_abs={} (within one pixel: {}, limit {max_abs})\n  mean_abs={:.4} (limit {mean_abs})\n  \
         differing={}/{} pixels\n  to accept: UPDATE_GOLD=1 cargo test --test crystal_editor_gold\n  \
         (a new renderer: look at the screenshot, then add it as tests/gold/crystal-editor-gl.<host>.png)",
        accepted.len(),
        diff.max_abs,
        diff.max_abs_shifted,
        diff.mean_abs,
        diff.differing_pixels,
        diff.total_pixels
    );
}

/// The comparator forgives a thin line drawn one pixel over and nothing else: the same stroke
/// shifted by a pixel passes, and a stroke that is missing or moved further fails.
#[test]
fn a_one_pixel_shift_is_forgiven_and_missing_content_is_not() {
    let (w, h) = (20usize, 20usize);
    let blank = vec![0u8; w * h * 4];
    let stroke = |x0: usize| {
        let mut img = blank.clone();
        for y in 5..15 {
            let i = (y * w + x0) * 4;
            img[i..i + 4].copy_from_slice(&[230, 230, 240, 255]);
        }
        img
    };
    let gold = stroke(10);
    let shifted = compare_rgba(&stroke(11), &gold, w);
    assert!(
        shifted.max_abs > 16 && shifted.max_abs_shifted == 0,
        "a one-pixel shift: raw {} shifted {}",
        shifted.max_abs,
        shifted.max_abs_shifted
    );
    let missing = compare_rgba(&blank, &gold, w);
    assert!(missing.max_abs_shifted > 16, "a missing stroke must fail");
    let moved = compare_rgba(&stroke(14), &gold, w);
    assert!(
        moved.max_abs_shifted > 16,
        "a stroke moved four pixels must fail"
    );
}
