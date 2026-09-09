// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — Browser / WASM target (--html / --native-wasm)

//! GL6.1–GL6.3: WebGL2 bridge for WASM builds.
//!
//! Each `wgl_*` function matches a `#native "loft_gl_*"` declaration in
//! `lib/graphics/src/graphics.loft`.  Arguments are read from the interpreter
//! stack and forwarded to JavaScript via `host_call("gl_*", &args)`.
//!
//! The JavaScript side (`gallery.html`) provides the actual WebGL2
//! implementation on `globalThis.loftHost`.

use crate::database::Stores;
use crate::keys::{DbRef, Str};

/// Register all WebGL bridge functions, replacing the panic stubs created by
/// `compile::register_native_stubs`.  Call this after `byte_code()`.
pub fn register_wgl_natives(state: &mut crate::state::State) {
    // Window lifecycle
    state.replace_native("loft_gl_create_window", wgl_create_window);
    state.replace_native("loft_gl_poll_events", wgl_poll_events);
    state.replace_native("loft_gl_swap_buffers", wgl_swap_buffers);
    state.replace_native("loft_gl_clear", wgl_clear);
    state.replace_native("loft_gl_destroy_window", wgl_destroy_window);
    // Shaders
    state.replace_native("loft_gl_create_shader", wgl_create_shader);
    state.replace_native("loft_gl_use_shader", wgl_use_shader);
    // Vertex upload + drawing
    state.replace_native("loft_gl_upload_vertices", wgl_upload_vertices);
    state.replace_native("loft_gl_draw", wgl_draw);
    state.replace_native("loft_gl_draw_mode", wgl_draw_mode);
    state.replace_native("loft_gl_draw_elements", wgl_draw_elements);
    state.replace_native("loft_gl_draw_fullscreen_quad", wgl_draw_fullscreen_quad);
    // Uniforms
    state.replace_native("loft_gl_set_mat4", wgl_set_uniform_mat4);
    state.replace_native("loft_gl_set_uniform_float", wgl_set_uniform_float);
    state.replace_native("loft_gl_set_uniform_int", wgl_set_uniform_int);
    state.replace_native("loft_gl_set_uniform_vec3", wgl_set_uniform_vec3);
    // GL state
    state.replace_native("loft_gl_enable", wgl_enable);
    state.replace_native("loft_gl_disable", wgl_disable);
    state.replace_native("loft_gl_blend_func", wgl_blend_func);
    state.replace_native("loft_gl_cull_face", wgl_cull_face);
    state.replace_native("loft_gl_depth_mask", wgl_depth_mask);
    state.replace_native("loft_gl_viewport", wgl_viewport);
    state.replace_native("loft_gl_line_width", wgl_line_width);
    state.replace_native("loft_gl_point_size", wgl_point_size);
    // Framebuffers
    state.replace_native("loft_gl_create_framebuffer", wgl_create_framebuffer);
    state.replace_native("loft_gl_bind_framebuffer", wgl_bind_framebuffer);
    state.replace_native("loft_gl_framebuffer_texture", wgl_framebuffer_texture);
    state.replace_native("loft_gl_create_depth_texture", wgl_create_depth_texture);
    state.replace_native("loft_gl_create_color_texture", wgl_create_color_texture);
    // Textures
    state.replace_native("loft_gl_load_texture", wgl_load_texture);
    state.replace_native("loft_gl_upload_canvas", wgl_upload_canvas);
    state.replace_native("loft_gl_bind_texture", wgl_bind_texture);
    state.replace_native("loft_gl_delete_texture", wgl_delete_texture);
    // Cleanup
    state.replace_native("loft_gl_delete_shader", wgl_delete_shader);
    state.replace_native("loft_gl_delete_vao", wgl_delete_vao);
    state.replace_native("loft_gl_delete_framebuffer", wgl_delete_framebuffer);
    // Input
    state.replace_native("loft_gl_key_pressed", wgl_key_pressed);
    state.replace_native("loft_gl_mouse_x", wgl_mouse_x);
    state.replace_native("loft_gl_mouse_y", wgl_mouse_y);
    state.replace_native("loft_gl_mouse_button", wgl_mouse_button);
    // GL7.2: PNG save
    state.replace_native("loft_save_png", wgl_save_png);
    // GL7.3: Font/text — delegate to JS host
    state.replace_native("loft_gl_load_font", wgl_load_font);
    state.replace_native("loft_gl_measure_text", wgl_measure_text);
    state.replace_native("loft_text_height", wgl_text_height);
    state.replace_native("loft_rasterize_text_into", wgl_rasterize_text_into);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

#[cfg(feature = "wasm")]
fn gl_call(method: &str, args: &js_sys::Array) -> wasm_bindgen::JsValue {
    crate::wasm::host_call_raw(method, args)
}

#[cfg(not(feature = "wasm"))]
#[allow(dead_code, clippy::trivially_copy_pass_by_ref)]
fn gl_call(_method: &str, _args: &()) -> i32 {
    0
}

/// Extract a `vector<single>` (f32 elements) from a DbRef into a JS Float32Array.
#[cfg(feature = "wasm")]
fn extract_f32_vector(stores: &Stores, vref: &DbRef) -> js_sys::Float32Array {
    let allocs = &stores.allocations;
    let store = &allocs[vref.store_nr as usize];
    let v_rec = store.get_u32_raw(vref.rec, vref.pos);
    if v_rec == 0 {
        return js_sys::Float32Array::new_with_length(0);
    }
    let len = store.get_u32_raw(v_rec, 4);
    // Elements stored at byte offset 8+ within the vector data record.
    let arr = js_sys::Float32Array::new_with_length(len);
    for i in 0..len {
        let val = store.get_single(v_rec, 8 + i * 4);
        arr.set_index(i, val);
    }
    arr
}

/// Extract a `vector<float>` (f64 elements) from a DbRef into a JS Float32Array
/// (converting f64→f32 for WebGL uniforms).
#[cfg(feature = "wasm")]
fn extract_f64_as_f32_vector(stores: &Stores, vref: &DbRef) -> js_sys::Float32Array {
    let allocs = &stores.allocations;
    let store = &allocs[vref.store_nr as usize];
    let v_rec = store.get_u32_raw(vref.rec, vref.pos);
    if v_rec == 0 {
        return js_sys::Float32Array::new_with_length(0);
    }
    let len = store.get_u32_raw(v_rec, 4);
    // Each f64 is 8 bytes, stored at byte offset 8+ within the vector data record.
    let arr = js_sys::Float32Array::new_with_length(len);
    for i in 0..len {
        let val = store.get_float(v_rec, 8 + i * 8);
        arr.set_index(i, val as f32);
    }
    arr
}

// ── Window lifecycle ─────────────────────────────────────────────────────────

fn wgl_create_window(stores: &mut Stores, stack: &mut DbRef) {
    let _title = stores.get::<Str>(stack);
    let _height = stores.get::<i64>(stack);
    let _width = stores.get::<i64>(stack);
    // WebGL context is created by JavaScript before WASM runs.
    // Just return true to indicate success.
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&_width.into(), &_height.into(), &_title.str().into());
        let result = gl_call("gl_create_window", &args);
        stores.put(stack, result.as_bool().unwrap_or(true));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, true);
}

fn wgl_poll_events(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_poll_events", &args);
        stores.put(stack, result.as_bool().unwrap_or(true));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, true);
}

fn wgl_swap_buffers(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_swap_buffers", &args);
        // FY.1: signal the interpreter to yield back to JavaScript.
        _stores.frame_yield = true;
    }
}

fn wgl_clear(_stores: &mut Stores, stack: &mut DbRef) {
    let color = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&color.into());
        gl_call("gl_clear", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = color;
}

fn wgl_destroy_window(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_destroy_window", &args);
    }
}

// ── Shaders ──────────────────────────────────────────────────────────────────

fn wgl_create_shader(stores: &mut Stores, stack: &mut DbRef) {
    let frag = stores.get::<Str>(stack);
    let vert = stores.get::<Str>(stack);
    #[cfg(feature = "wasm")]
    {
        // GL6.5: patch shader version for WebGL2
        let vert_patched = patch_shader(vert.str());
        let frag_patched = patch_shader(frag.str());
        let args = js_sys::Array::of2(&vert_patched.into(), &frag_patched.into());
        let result = gl_call("gl_create_shader", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (vert, frag);
        stores.put(stack, -1i64);
    }
}

/// GL6.5: Convert GLSL 330 core → GLSL 300 es for WebGL2.
#[cfg(feature = "wasm")]
fn patch_shader(src: &str) -> String {
    let mut result = src.to_string();
    // Replace version line
    if result.contains("#version 330 core") {
        result = result.replace("#version 330 core", "#version 300 es");
        // Add precision qualifier after version line
        if let Some(pos) = result.find("#version 300 es") {
            let end = pos + "#version 300 es".len();
            let newline_end = result[end..].find('\n').map_or(end, |p| end + p + 1);
            result.insert_str(
                newline_end,
                "precision highp float;\nprecision highp int;\n",
            );
        }
    } else if result.contains("#version 330") {
        result = result.replace("#version 330", "#version 300 es");
        if let Some(pos) = result.find("#version 300 es") {
            let end = pos + "#version 300 es".len();
            let newline_end = result[end..].find('\n').map_or(end, |p| end + p + 1);
            result.insert_str(
                newline_end,
                "precision highp float;\nprecision highp int;\n",
            );
        }
    }
    // WebGL requires gl_PointSize in vertex shaders for point rendering.
    // If the shader sets gl_Position but not gl_PointSize, inject a default.
    if result.contains("gl_Position") && !result.contains("gl_PointSize") {
        result = result.replace("gl_Position =", "gl_PointSize = 4.0; gl_Position =");
    }
    result
}

fn wgl_use_shader(_stores: &mut Stores, stack: &mut DbRef) {
    let program = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&program.into());
        gl_call("gl_use_shader", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = program;
}

// ── Vertex upload + drawing ──────────────────────────────────────────────────

fn wgl_upload_vertices(stores: &mut Stores, stack: &mut DbRef) {
    let stride = stores.get::<i64>(stack);
    let data_ref = stores.get::<DbRef>(stack);
    #[cfg(feature = "wasm")]
    {
        let data = extract_f32_vector(stores, &data_ref);
        let args = js_sys::Array::of2(&data.into(), &stride.into());
        let result = gl_call("gl_upload_vertices", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (stride, data_ref);
        stores.put(stack, -1i64);
    }
}

fn wgl_draw(_stores: &mut Stores, stack: &mut DbRef) {
    let vertex_count = _stores.get::<i64>(stack);
    let vao = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&vao.into(), &vertex_count.into());
        gl_call("gl_draw", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, vertex_count);
}

fn wgl_draw_mode(_stores: &mut Stores, stack: &mut DbRef) {
    let mode = _stores.get::<i64>(stack);
    let vertex_count = _stores.get::<i64>(stack);
    let vao = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&vao.into(), &vertex_count.into(), &mode.into());
        gl_call("gl_draw_mode", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, vertex_count, mode);
}

fn wgl_draw_elements(_stores: &mut Stores, stack: &mut DbRef) {
    let mode = _stores.get::<i64>(stack);
    let index_count = _stores.get::<i64>(stack);
    let vao = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&vao.into(), &index_count.into(), &mode.into());
        gl_call("gl_draw_elements", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (vao, index_count, mode);
}

fn wgl_draw_fullscreen_quad(_stores: &mut Stores, _stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        gl_call("gl_draw_fullscreen_quad", &args);
    }
}

// ── Uniforms ─────────────────────────────────────────────────────────────────

fn wgl_set_uniform_mat4(stores: &mut Stores, stack: &mut DbRef) {
    let mat_ref = stores.get::<DbRef>(stack);
    let name = stores.get::<Str>(stack);
    let program = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let mat = extract_f64_as_f32_vector(stores, &mat_ref);
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &mat.into());
        gl_call("gl_set_uniform_mat4", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, mat_ref);
}

fn wgl_set_uniform_float(stores: &mut Stores, stack: &mut DbRef) {
    let val = stores.get::<f64>(stack);
    let name = stores.get::<Str>(stack);
    let program = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &val.into());
        gl_call("gl_set_uniform_float", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, val);
}

fn wgl_set_uniform_int(stores: &mut Stores, stack: &mut DbRef) {
    let val = stores.get::<i64>(stack);
    let name = stores.get::<Str>(stack);
    let program = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&program.into(), &name.str().into(), &val.into());
        gl_call("gl_set_uniform_int", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, val);
}

fn wgl_set_uniform_vec3(stores: &mut Stores, stack: &mut DbRef) {
    let z = stores.get::<f64>(stack);
    let y = stores.get::<f64>(stack);
    let x = stores.get::<f64>(stack);
    let name = stores.get::<Str>(stack);
    let program = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        args.push(&program.into());
        args.push(&name.str().into());
        args.push(&x.into());
        args.push(&y.into());
        args.push(&z.into());
        gl_call("gl_set_uniform_vec3", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (program, name, x, y, z);
}

// ── GL state ─────────────────────────────────────────────────────────────────

fn wgl_enable(_stores: &mut Stores, stack: &mut DbRef) {
    let cap = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&cap.into());
        gl_call("gl_enable", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = cap;
}

fn wgl_disable(_stores: &mut Stores, stack: &mut DbRef) {
    let cap = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&cap.into());
        gl_call("gl_disable", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = cap;
}

fn wgl_blend_func(_stores: &mut Stores, stack: &mut DbRef) {
    let dst = _stores.get::<i64>(stack);
    let src = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&src.into(), &dst.into());
        gl_call("gl_blend_func", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (src, dst);
}

fn wgl_cull_face(_stores: &mut Stores, stack: &mut DbRef) {
    let face = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&face.into());
        gl_call("gl_cull_face", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = face;
}

fn wgl_depth_mask(_stores: &mut Stores, stack: &mut DbRef) {
    let write = _stores.get::<bool>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&write.into());
        gl_call("gl_depth_mask", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = write;
}

fn wgl_viewport(_stores: &mut Stores, stack: &mut DbRef) {
    let h = _stores.get::<i64>(stack);
    let w = _stores.get::<i64>(stack);
    let y = _stores.get::<i64>(stack);
    let x = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        args.push(&x.into());
        args.push(&y.into());
        args.push(&w.into());
        args.push(&h.into());
        gl_call("gl_viewport", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (x, y, w, h);
}

fn wgl_line_width(_stores: &mut Stores, stack: &mut DbRef) {
    let width = _stores.get::<f64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&width.into());
        gl_call("gl_line_width", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = width;
}

fn wgl_point_size(_stores: &mut Stores, stack: &mut DbRef) {
    let size = _stores.get::<f64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&size.into());
        gl_call("gl_point_size", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = size;
}

// ── Framebuffers ─────────────────────────────────────────────────────────────

fn wgl_create_framebuffer(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_create_framebuffer", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, -1i64);
}

fn wgl_bind_framebuffer(_stores: &mut Stores, stack: &mut DbRef) {
    let fbo = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&fbo.into());
        gl_call("gl_bind_framebuffer", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = fbo;
}

fn wgl_framebuffer_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let tex = _stores.get::<i64>(stack);
    let attachment = _stores.get::<i64>(stack);
    let fbo = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of3(&fbo.into(), &attachment.into(), &tex.into());
        gl_call("gl_framebuffer_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (fbo, attachment, tex);
}

fn wgl_create_depth_texture(stores: &mut Stores, stack: &mut DbRef) {
    let height = stores.get::<i64>(stack);
    let width = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&width.into(), &height.into());
        let result = gl_call("gl_create_depth_texture", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (width, height);
        stores.put(stack, -1i64);
    }
}

fn wgl_create_color_texture(stores: &mut Stores, stack: &mut DbRef) {
    let height = stores.get::<i64>(stack);
    let width = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&width.into(), &height.into());
        let result = gl_call("gl_create_color_texture", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (width, height);
        stores.put(stack, -1i64);
    }
}

// ── Textures ─────────────────────────────────────────────────────────────────

fn wgl_load_texture(stores: &mut Stores, stack: &mut DbRef) {
    let path = stores.get::<Str>(stack);
    #[cfg(feature = "wasm")]
    {
        // GL7.1: Pass path to JS; the gallery pre-loads assets and decodes
        // the image via the browser's native image decoder.
        let args = js_sys::Array::of1(&path.str().into());
        let result = gl_call("gl_load_texture", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        stores.put(stack, -1i64);
    }
}

fn wgl_upload_canvas(stores: &mut Stores, stack: &mut DbRef) {
    let height = stores.get::<i64>(stack);
    let width = stores.get::<i64>(stack);
    let data_ref = stores.get::<DbRef>(stack);
    #[cfg(feature = "wasm")]
    {
        // Extract vector<integer> as Uint32Array and pass to JS
        let allocs = &stores.allocations;
        let store = &allocs[data_ref.store_nr as usize];
        let v_rec = store.get_u32_raw(data_ref.rec, data_ref.pos);
        let len = if v_rec == 0 {
            0
        } else {
            store.get_u32_raw(v_rec, 4)
        };
        let arr = js_sys::Uint32Array::new_with_length(len);
        for i in 0..len {
            // Post-2c: vector<integer> elements are 8 bytes; read low 32.
            let val = store.get_int(v_rec, 8 + i * 8) as u32;
            arr.set_index(i, val);
        }
        let args = js_sys::Array::of3(&arr.into(), &width.into(), &height.into());
        let result = gl_call("gl_upload_canvas", &args);
        stores.put(stack, result.as_f64().unwrap_or(-1.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (data_ref, width, height);
        stores.put(stack, -1i64);
    }
}

fn wgl_bind_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let unit = _stores.get::<i64>(stack);
    let tex_id = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of2(&tex_id.into(), &unit.into());
        gl_call("gl_bind_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = (tex_id, unit);
}

fn wgl_delete_texture(_stores: &mut Stores, stack: &mut DbRef) {
    let tex_id = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&tex_id.into());
        gl_call("gl_delete_texture", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = tex_id;
}

// ── Cleanup ──────────────────────────────────────────────────────────────────

fn wgl_delete_shader(_stores: &mut Stores, stack: &mut DbRef) {
    let program = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&program.into());
        gl_call("gl_delete_shader", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = program;
}

fn wgl_delete_vao(_stores: &mut Stores, stack: &mut DbRef) {
    let vao = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&vao.into());
        gl_call("gl_delete_vao", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = vao;
}

fn wgl_delete_framebuffer(_stores: &mut Stores, stack: &mut DbRef) {
    let fbo = _stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&fbo.into());
        gl_call("gl_delete_framebuffer", &args);
    }
    #[cfg(not(feature = "wasm"))]
    let _ = fbo;
}

// ── Input ────────────────────────────────────────────────────────────────────

fn wgl_key_pressed(stores: &mut Stores, stack: &mut DbRef) {
    let key_code = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::of1(&key_code.into());
        let result = gl_call("gl_key_pressed", &args);
        stores.put(stack, result.as_bool().unwrap_or(false));
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = key_code;
        stores.put(stack, false);
    }
}

fn wgl_mouse_x(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_x", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0.0f64);
}

fn wgl_mouse_y(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_y", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0));
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0.0f64);
}

fn wgl_mouse_button(stores: &mut Stores, stack: &mut DbRef) {
    #[cfg(feature = "wasm")]
    {
        let args = js_sys::Array::new();
        let result = gl_call("gl_mouse_button", &args);
        stores.put(stack, result.as_f64().unwrap_or(0.0) as i64);
    }
    #[cfg(not(feature = "wasm"))]
    stores.put(stack, 0i64);
}

// ── GL7.2: PNG save ──────────────────────────────────────────────────────────

/// Save_png_raw(path, width, height, data) -> boolean
fn wgl_save_png(stores: &mut Stores, stack: &mut DbRef) {
    let data_ref = stores.get::<DbRef>(stack);
    let height = stores.get::<i64>(stack);
    let width = stores.get::<i64>(stack);
    let path = stores.get::<Str>(stack);
    #[cfg(feature = "wasm")]
    {
        // Extract pixel data and pass to JS for download.
        let allocs = &stores.allocations;
        let store = &allocs[data_ref.store_nr as usize];
        let v_rec = store.get_u32_raw(data_ref.rec, data_ref.pos);
        let len = if v_rec == 0 {
            0
        } else {
            store.get_u32_raw(v_rec, 4)
        };
        let arr = js_sys::Uint32Array::new_with_length(len);
        for i in 0..len {
            // Post-2c: vector<integer> elements are 8 bytes; read low 32.
            let val = store.get_int(v_rec, 8 + i * 8) as u32;
            arr.set_index(i, val);
        }
        let args = js_sys::Array::new();
        args.push(&path.str().into());
        args.push(&width.into());
        args.push(&height.into());
        args.push(&arr.into());
        gl_call("save_png", &args);
        stores.put(stack, true);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (path, width, height, data_ref);
        stores.put(stack, false);
    }
}

// ── GL7.3: Font / text (fontdue in WASM) ─────────────────────────────────────

#[cfg(feature = "wasm")]
use std::cell::RefCell;

#[cfg(feature = "wasm")]
thread_local! {
    static FONTS: RefCell<Vec<fontdue::Font>> = const { RefCell::new(Vec::new()) };
}

/// Gl_load_font(path) -> integer
fn wgl_load_font(stores: &mut Stores, stack: &mut DbRef) {
    let path = stores.get::<Str>(stack);
    #[cfg(feature = "wasm")]
    {
        // Ask JS for binary asset data (Uint8Array — no base64 overhead).
        let args = js_sys::Array::of1(&path.str().into());
        let result = gl_call("load_binary_asset", &args);
        let data = if wasm_bindgen::JsCast::is_instance_of::<js_sys::Uint8Array>(&result) {
            let arr = js_sys::Uint8Array::from(result);
            let mut buf = vec![0u8; arr.length() as usize];
            arr.copy_to(&mut buf);
            Some(buf)
        } else {
            None
        };
        let Some(bytes) = data else {
            stores.put(stack, -1i64);
            return;
        };
        let Ok(font) =
            fontdue::Font::from_bytes(bytes.as_slice(), fontdue::FontSettings::default())
        else {
            stores.put(stack, -1i64);
            return;
        };
        let idx = FONTS.with(|fonts| {
            let mut fonts = fonts.borrow_mut();
            let idx = fonts.len() as i64;
            fonts.push(font);
            idx
        });
        stores.put(stack, idx);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = path;
        stores.put(stack, -1i64);
    }
}

/// Gl_measure_text(font, content, size) -> float
fn wgl_measure_text(stores: &mut Stores, stack: &mut DbRef) {
    let size = stores.get::<f64>(stack);
    let content = stores.get::<Str>(stack);
    let font_idx = stores.get::<i64>(stack);
    #[cfg(feature = "wasm")]
    {
        let width: f64 = FONTS.with(|fonts| {
            let fonts = fonts.borrow();
            let Some(font) = fonts.get(font_idx as usize) else {
                return 0.0;
            };
            content
                .str()
                .chars()
                .map(|c| {
                    let (metrics, _) = font.rasterize(c, size as f32);
                    f64::from(metrics.advance_width)
                })
                .sum()
        });
        stores.put(stack, width);
    }
    #[cfg(not(feature = "wasm"))]
    {
        let _ = (font_idx, content, size);
        stores.put(stack, 0.0f64);
    }
}

/// Gl_text_height(font, size) -> integer
fn wgl_text_height(stores: &mut Stores, stack: &mut DbRef) {
    let size = stores.get::<f64>(stack);
    let _font_idx = stores.get::<i64>(stack);
    stores.put(stack, (size * 1.2) as i64);
}

/// Rasterize_text_into(font, content, size, buf) -> integer (width)
/// Rasterizes text into a pre-allocated loft vector<integer> of alpha values.
fn wgl_rasterize_text_into(stores: &mut Stores, stack: &mut DbRef) {
    let buf_ref = stores.get::<DbRef>(stack);
    let size = stores.get::<f64>(stack);
    let content = stores.get::<Str>(stack);
    let font_idx = stores.get::<i64>(stack);

    #[cfg(not(feature = "wasm"))]
    {
        let _ = (buf_ref, size, content, font_idx);
        stores.put(stack, 0i64);
    }

    #[cfg(feature = "wasm")]
    {
        let size_f32 = size as f32;
        let text = content.str().to_string();
        let (bw, _bh, pixels) = FONTS.with(|fonts| {
            let fonts = fonts.borrow();
            let Some(font) = fonts.get(font_idx as usize) else {
                return (0u32, 0u32, Vec::new());
            };
            // Rasterize each glyph
            let mut glyphs: Vec<(fontdue::Metrics, Vec<u8>)> = Vec::new();
            let mut total_w = 0u32;
            for c in text.chars() {
                let (m, bmp) = font.rasterize(c, size_f32);
                total_w += m.advance_width as u32;
                glyphs.push((m, bmp));
            }
            let line_h = (size_f32 * 1.2) as u32;
            if total_w == 0 || line_h == 0 {
                return (0, 0, Vec::new());
            }
            let mut px = vec![0u8; (total_w * line_h) as usize];
            let mut cx = 0u32;
            for (m, bmp) in &glyphs {
                let gw = m.width as u32;
                let gh = m.height as u32;
                let baseline = (size_f32 * 0.8) as i32;
                let y_off = (baseline - m.height as i32 - m.ymin).max(0) as u32;
                for gy in 0..gh {
                    for gx in 0..gw {
                        let dx = cx + gx;
                        let dy = y_off + gy;
                        if dx < total_w && dy < line_h {
                            let src = bmp[(gy * gw + gx) as usize];
                            let di = (dy * total_w + dx) as usize;
                            if di < px.len() {
                                px[di] = src.max(px[di]);
                            }
                        }
                    }
                }
                cx += m.advance_width as u32;
            }
            (total_w, line_h, px)
        });

        if bw == 0 || pixels.is_empty() {
            stores.put(stack, 0i64);
            return;
        }

        // Write alpha values into the loft vector<integer> buffer.
        let allocs = &mut stores.allocations;
        let store = &mut allocs[buf_ref.store_nr as usize];
        let v_rec = store.get_u32_raw(buf_ref.rec, buf_ref.pos);
        if v_rec != 0 {
            let buf_len = store.get_u32_raw(v_rec, 4);
            let count = pixels.len().min(buf_len as usize);
            for (i, &p) in pixels.iter().enumerate().take(count) {
                store.set_int(v_rec, 8 + i as u32 * 8, i64::from(p));
            }
        }

        stores.put(stack, i64::from(bw));
    } // cfg(feature = "wasm")
}
