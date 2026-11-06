// loft-gl.js — WebGL2 bridge for loft WASM games
// Provides the loftHost.gl_* methods that the loft interpreter calls via host_call.
// Usage:
//   import { initLoftGL } from './loft-gl.js';
//   const loftGL = initLoftGL(document.getElementById('my-canvas'));
//   // loftGL.shouldClose, loftGL.assets, loftGL.gl

/**
 * Initialize the WebGL2 bridge on a canvas element.
 * Sets up window.loftHost with all gl_* methods.
 * Returns a control object for the game loop.
 *
 * @param {HTMLCanvasElement} canvas
 * @returns {{ gl: WebGL2RenderingContext, shouldClose: boolean, assets: Object }}
 */
export function initLoftGL(canvas) {
  let gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  if (!gl) throw new Error('WebGL2 not available');

  let programs = [];
  let vaos = [];
  let textures = [];
  let fbos = [];

  // Defensive Number coercion — the loft side now passes integers as
  // i64 (post phase-2c migration); wasm-bindgen converts i64 → BigInt
  // by default, and BigInt silently mis-behaves under `>>>` / `&` /
  // arithmetic the JS bridge uses to unpack RGBA / build GL args.
  // `n()` accepts both Number and BigInt and returns Number; safe for
  // any value that fits in f64 mantissa (every gl_* arg does — widths,
  // colors, handle ids all <= 32 bits).
  const n = (x) => (typeof x === 'bigint' ? Number(x) : x);

  // On-page diagnostics — opt-in via `?debug=1` URL param.  Surfaces
  // the first few calls into each gl_* method as an overlay so we can
  // debug rendering issues on phones / TVs / anywhere we can't open
  // devtools.  Pass-through when debug is off.
  const DEBUG = new URLSearchParams(location.search).get('debug') === '1';
  let debugLog = null, debugSeen = {};
  if (DEBUG) {
    debugLog = document.createElement('pre');
    debugLog.style.cssText = 'position:fixed;top:0;left:0;background:rgba(0,0,0,0.85);' +
      'color:#0f0;font:11px monospace;padding:6px;margin:0;z-index:9999;max-width:90vw;' +
      'max-height:50vh;overflow:auto;white-space:pre-wrap;border-bottom:1px solid #0f0;';
    document.addEventListener('DOMContentLoaded', () => document.body.appendChild(debugLog));
    if (document.body) document.body.appendChild(debugLog);
  }
  function dbg(name, info) {
    if (!DEBUG) return;
    // Show only the first 2 calls per method to avoid the overlay
    // overflowing for per-frame calls (gl_clear runs 60+/s).
    const k = name + ':' + (debugSeen[name] = (debugSeen[name] || 0) + 1);
    if (debugSeen[name] > 2) return;
    if (debugLog) debugLog.textContent += k + ' ' + info + '\n';
  }

  // lib/graphics shaders target desktop GLSL (#version 330 core).  WebGL2
  // needs GLSL ES 3.00 — same grammar for the subset we use, but a
  // different version directive and explicit precision in fragment shaders.
  // Rewrite the header transparently so loft source stays portable.
  function translateShader(src, isFragment) {
    const re = /^\s*#version\s+\d+(\s+\w+)?\s*\n?/;
    const head = isFragment
      ? '#version 300 es\nprecision highp float;\nprecision highp int;\n'
      : '#version 300 es\n';
    if (re.test(src)) return src.replace(re, head);
    return head + src;
  }

  // ── GL capability / blend / draw mode maps ──────────────────────────────

  function glCap(cap) {
    switch (cap) {
      case 1: return gl.DEPTH_TEST;
      case 2: return gl.BLEND;
      case 3: return gl.CULL_FACE;
      default: return cap;
    }
  }

  function glBlendFactor(f) {
    switch (f) {
      case 0: return gl.ZERO;
      case 1: return gl.ONE;
      case 2: return gl.SRC_ALPHA;
      case 3: return gl.ONE_MINUS_SRC_ALPHA;
      case 4: return gl.DST_ALPHA;
      case 5: return gl.ONE_MINUS_DST_ALPHA;
      default: return f;
    }
  }

  function glDrawMode(mode) {
    switch (mode) {
      case 0: return gl.TRIANGLES;
      case 1: return gl.LINES;
      case 2: return gl.POINTS;
      default: return gl.TRIANGLES;
    }
  }

  // ── Input state ─────────────────────────────────────────────────────────

  const keys = new Set();
  let mouseX = 0, mouseY = 0, mouseBtn = 0;

  function mapKeyCode(code) {
    // Loft key constants use lowercase ASCII: KEY_W=119 ('w'), KEY_A=97 ('a')
    if (code.startsWith('Key')) return code.charCodeAt(3) + 32;
    if (code.startsWith('Digit')) return code.charCodeAt(5);
    const special = {
      ArrowUp: 128, ArrowDown: 129, ArrowLeft: 130, ArrowRight: 131,
      ShiftLeft: 132, ShiftRight: 132, ControlLeft: 133, ControlRight: 133,
      Space: 32, Enter: 13, Escape: 27, Tab: 9,
    };
    return special[code] || 0;
  }

  canvas.tabIndex = 0;
  canvas.addEventListener('keydown', e => { keys.add(mapKeyCode(e.code)); e.preventDefault(); });
  canvas.addEventListener('keyup', e => keys.delete(mapKeyCode(e.code)));
  canvas.addEventListener('mousemove', e => {
    const r = canvas.getBoundingClientRect();
    mouseX = e.clientX - r.left;
    mouseY = e.clientY - r.top;
  });
  canvas.addEventListener('mousedown', e => { mouseBtn |= (1 << e.button); });
  canvas.addEventListener('mouseup', e => { mouseBtn &= ~(1 << e.button); });
  // @PLN18 phase 07 — phones: touches map onto the SAME mouse state, so
  // loft scripts read one input surface on every host (a tap = button 0).
  const touchPos = (t) => {
    const r = canvas.getBoundingClientRect();
    mouseX = t.clientX - r.left;
    mouseY = t.clientY - r.top;
  };
  canvas.addEventListener('touchstart', e => {
    e.preventDefault();
    touchPos(e.changedTouches[0]);
    mouseBtn |= 1;
  }, { passive: false });
  canvas.addEventListener('touchmove', e => {
    e.preventDefault();
    touchPos(e.changedTouches[0]);
  }, { passive: false });
  canvas.addEventListener('touchend', e => {
    e.preventDefault();
    mouseBtn &= ~1;
  }, { passive: false });

  // ── Text ────────────────────────────────────────────────────────────────
  // The browser already HAS a rasteriser, so text is a 2D canvas: `measureText` for
  // the metrics and `fillText` white-on-transparent for the coverage bitmap (its
  // alpha channel IS the coverage).  The bitmap's height is the line height and its
  // baseline sits `ascent` from the top, as on the desktop backend.  A font PATH
  // resolves to a CSS family under its base name; a page brings the real face as a
  // `FontFace` (see `registerFontAsset`) or its own CSS, and anything else draws in
  // a generic guessed from the name.  This is the same bridge `loft --html` uses
  // (`loft-gl-wasm.js`, § text) — keep the two answering alike.
  let fonts = [], textCv = null, textCx = null;
  function text2d() {
    if (!textCx) {
      textCv = document.createElement('canvas');
      textCx = textCv.getContext('2d', { willReadFrequently: true });
    }
    return textCx;
  }
  function baseNameOf(path) {
    return String(path).split(/[\\/]/).pop().replace(/\.[^.]*$/, '');
  }
  function genericFor(base) {
    const b = base.toLowerCase();
    if (/mono|courier|consol|code/.test(b)) return 'monospace';
    if (/serif/.test(b) && !/sans/.test(b)) return 'serif';
    return 'sans-serif';
  }
  function familyFor(base) {
    return '"' + base.replace(/"/g, '') + '", ' + genericFor(base);
  }
  // Metrics for font `fi` at `sz`: `asc` from the font's own box, `line` the
  // height a bitmap gets.
  function fontMetrics(fi, sz) {
    const f = fonts[fi];
    const cx = text2d();
    cx.font = sz + 'px ' + (f ? f.family : 'sans-serif');
    const m = cx.measureText('Mg');
    const asc = m.fontBoundingBoxAscent || m.actualBoundingBoxAscent || sz * 0.8;
    const desc = m.fontBoundingBoxDescent || m.actualBoundingBoxDescent || sz * 0.2;
    return { cx: cx, asc: asc, line: Math.max(1, Math.ceil(asc + desc)) };
  }

  /** Register a served font file's bytes as a page font under the base name of each
   *  path in `paths` — the family `gl_load_font(path)` resolves to.  Await it before
   *  the program starts: a face still loading draws in the fallback.  The browser
   *  parses the file; a file it refuses leaves text in the generic fallback. */
  async function registerFontAsset(bytes, paths) {
    const families = [...new Set(paths.map(baseNameOf))].filter(b => b);
    await Promise.all(families.map(async (family) => {
      try {
        document.fonts.add(await new FontFace(family, bytes).load());
      } catch (e) {
        console.warn('[loft] font "' + family + '" could not be registered — text draws in '
          + genericFor(family) + '.');
      }
    }));
  }

  // ── Control object ──────────────────────────────────────────────────────

  const ctrl = {
    gl,
    shouldClose: false,
    /** Pre-loaded binary assets keyed by filename. Set .image to an HTMLImageElement. */
    assets: {},
    registerFontAsset,
  };

  // ── loftHost interface ──────────────────────────────────────────────────

  // Wrap every gl_*/time_* method so every argument is BigInt-coerced
  // to Number at the boundary.  Keeps the per-method bodies clean.
  function coerceArgs(fns) {
    const wrapped = {};
    for (const [name, f] of Object.entries(fns)) {
      wrapped[name] = function(...args) {
        for (let i = 0; i < args.length; i++) args[i] = n(args[i]);
        if (DEBUG && name.startsWith('gl_')) {
          dbg(name, args.map(a => typeof a === 'string' ? `"${a.slice(0,16)}"` : `${a}(${typeof a})`).join(', '));
        }
        return f.apply(this, args);
      };
    }
    return wrapped;
  }

  window.loftHost = window.loftHost || {};
  Object.assign(window.loftHost, coerceArgs({

    // Time — milliseconds since epoch (used by loft ticks())
    time_now() { return Date.now(); },
    time_ticks() { return Math.round(performance.now() * 1000); }, // µs — ticks()'s unit (was ms: a 1000× clock bug)

    // Window lifecycle
    gl_create_window(w, h, _title) {
      canvas.width = w;
      canvas.height = h;
      programs = [];
      vaos = [];
      textures = [];
      fbos = [];
      gl.viewport(0, 0, w, h);
      // Match native: depth test enabled by default
      gl.enable(gl.DEPTH_TEST);
      return true;
    },

    gl_poll_events() {
      return !ctrl.shouldClose;
    },

    gl_swap_buffers() {
      gl.flush();
    },

    gl_clear(color) {
      // Loft rgba() packs as 0xAARRGGBB
      const a = ((color >>> 24) & 0xff) / 255;
      const r = ((color >>> 16) & 0xff) / 255;
      const g = ((color >>> 8)  & 0xff) / 255;
      const b = (color & 0xff) / 255;
      gl.clearColor(r, g, b, a);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    },

    gl_destroy_window() {
      for (const p of programs) if (p) gl.deleteProgram(p);
      for (const v of vaos) if (v) gl.deleteVertexArray(v.vao);
      for (const t of textures) if (t) gl.deleteTexture(t);
      for (const f of fbos) if (f) gl.deleteFramebuffer(f);
      programs = [];
      vaos = [];
      textures = [];
      fbos = [];
    },

    // Shaders
    gl_create_shader(vertSrc, fragSrc) {
      vertSrc = translateShader(vertSrc, false);
      fragSrc = translateShader(fragSrc, true);
      const vs = gl.createShader(gl.VERTEX_SHADER);
      gl.shaderSource(vs, vertSrc);
      gl.compileShader(vs);
      if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
        console.error('Vertex shader:', gl.getShaderInfoLog(vs));
        gl.deleteShader(vs);
        return -1;
      }
      const fs = gl.createShader(gl.FRAGMENT_SHADER);
      gl.shaderSource(fs, fragSrc);
      gl.compileShader(fs);
      if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
        console.error('Fragment shader:', gl.getShaderInfoLog(fs));
        gl.deleteShader(vs);
        gl.deleteShader(fs);
        return -1;
      }
      const prog = gl.createProgram();
      gl.attachShader(prog, vs);
      gl.attachShader(prog, fs);
      gl.linkProgram(prog);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        console.error('Link:', gl.getProgramInfoLog(prog));
        gl.deleteProgram(prog);
        return -1;
      }
      const idx = programs.length;
      programs.push(prog);
      return idx;
    },

    gl_use_shader(program) {
      if (program >= 0 && program < programs.length) {
        gl.useProgram(programs[program]);
      }
    },

    // Vertex upload + drawing
    gl_upload_vertices(data, stride) {
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const vbo = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const bpv = stride * 4;
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, bpv, 0);
      if (stride >= 6) {
        gl.enableVertexAttribArray(1);
        gl.vertexAttribPointer(1, 3, gl.FLOAT, false, bpv, 12);
      }
      if (stride >= 8) {
        gl.enableVertexAttribArray(2);
        gl.vertexAttribPointer(2, 2, gl.FLOAT, false, bpv, 24);
      }
      if (stride >= 10) {
        gl.enableVertexAttribArray(3);
        gl.vertexAttribPointer(3, 2, gl.FLOAT, false, bpv, 32);
      }
      gl.bindVertexArray(null);
      const idx = vaos.length;
      vaos.push({ vao, vbo, vertexCount: data.length / stride });
      return idx;
    },

    gl_draw(vaoIdx, vertexCount) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawArrays(gl.TRIANGLES, 0, vertexCount);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_mode(vaoIdx, vertexCount, mode) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawArrays(glDrawMode(mode), 0, vertexCount);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_elements(vaoIdx, indexCount, mode) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawElements(glDrawMode(mode), indexCount, gl.UNSIGNED_INT, 0);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_fullscreen_quad() {
      // Lazy-init a fullscreen quad VAO: 2 triangles, pos(xy) + uv(st)
      if (!ctrl._fsQuad) {
        const vao = gl.createVertexArray();
        gl.bindVertexArray(vao);
        const vbo = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
          -1, -1, 0, 0,
           1, -1, 1, 0,
          -1,  1, 0, 1,
           1, -1, 1, 0,
           1,  1, 1, 1,
          -1,  1, 0, 1,
        ]), gl.STATIC_DRAW);
        // location 0 = position (vec2), location 1 = UV (vec2)
        gl.enableVertexAttribArray(0);
        gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
        gl.enableVertexAttribArray(1);
        gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);
        gl.bindVertexArray(null);
        ctrl._fsQuad = vao;
      }
      gl.bindVertexArray(ctrl._fsQuad);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
      gl.bindVertexArray(null);
    },

    // Uniforms
    gl_set_uniform_mat4(program, name, mat) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniformMatrix4fv(loc, false, mat);
      }
    },

    gl_set_uniform_float(program, name, val) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform1f(loc, val);
      }
    },

    gl_set_uniform_int(program, name, val) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform1i(loc, val);
      }
    },

    gl_set_uniform_vec3(program, name, x, y, z) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform3f(loc, x, y, z);
      }
    },

    // GL state
    gl_enable(cap) { gl.enable(glCap(cap)); },
    gl_disable(cap) { gl.disable(glCap(cap)); },

    gl_blend_func(src, dst) {
      gl.blendFunc(glBlendFactor(src), glBlendFactor(dst));
    },

    gl_cull_face(face) {
      gl.cullFace(face === 1 ? gl.FRONT : gl.BACK);
    },

    gl_depth_mask(write) { gl.depthMask(write); },
    gl_viewport(x, y, w, h) { gl.viewport(x, y, w, h); },
    gl_line_width(width) { gl.lineWidth(width); },
    gl_point_size(_size) { /* WebGL: use gl_PointSize in vertex shader */ },

    // Framebuffers
    gl_create_framebuffer() {
      const fbo = gl.createFramebuffer();
      // Offset by 1: index 0 = default screen framebuffer (matches native GL)
      const idx = fbos.length + 1;
      fbos.push(fbo);
      return idx;
    },

    gl_bind_framebuffer(fboIdx) {
      if (fboIdx <= 0) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      } else if (fboIdx - 1 < fbos.length) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[fboIdx - 1]);
      }
    },

    gl_framebuffer_texture(fboIdx, attachment, texIdx) {
      const fi = fboIdx - 1;
      if (fi >= 0 && fi < fbos.length && texIdx >= 0 && texIdx < textures.length) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[fi]);
        const attach = attachment === 1 ? gl.DEPTH_ATTACHMENT : gl.COLOR_ATTACHMENT0;
        gl.framebufferTexture2D(gl.FRAMEBUFFER, attach, gl.TEXTURE_2D, textures[texIdx], 0);
      }
    },

    gl_create_depth_texture(w, h) {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, w, h, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_create_color_texture(w, h) {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    // Textures
    gl_load_texture(path) {
      const asset = ctrl.assets[path];
      if (!asset || !asset.image) return -1;
      const img = asset.image;
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
      gl.generateMipmap(gl.TEXTURE_2D);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_upload_canvas(data, w, h) {
      // C58: canonical `(0, 0) = canvas top-left` — no upload-side Y flip.
      // glTexImage2D stores the first buffer row at GL bottom, putting
      // canvas-top at TC.y=0; the 2D ortho's `-2/H` maps that to screen-top.
      const pixels = new Uint8Array(w * h * 4);
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const si = y * w + x;
          const di = (y * w + x) * 4;
          const c = data[si];
          pixels[di + 0] = (c >>> 16) & 0xff;
          pixels[di + 1] = (c >>> 8) & 0xff;
          pixels[di + 2] = c & 0xff;
          pixels[di + 3] = (c >>> 24) & 0xff;
        }
      }
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_bind_texture(texIdx, unit) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      if (texIdx >= 0 && texIdx < textures.length) {
        gl.bindTexture(gl.TEXTURE_2D, textures[texIdx]);
      }
    },

    gl_delete_texture(texIdx) {
      if (texIdx >= 0 && texIdx < textures.length && textures[texIdx]) {
        gl.deleteTexture(textures[texIdx]);
        textures[texIdx] = null;
      }
    },

    // Cleanup
    gl_delete_shader(progIdx) {
      if (progIdx >= 0 && progIdx < programs.length && programs[progIdx]) {
        gl.deleteProgram(programs[progIdx]);
        programs[progIdx] = null;
      }
    },

    gl_delete_vao(vaoIdx) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length && vaos[vaoIdx]) {
        gl.deleteVertexArray(vaos[vaoIdx].vao);
        gl.deleteBuffer(vaos[vaoIdx].vbo);
        vaos[vaoIdx] = null;
      }
    },

    gl_delete_framebuffer(fboIdx) {
      const fi = fboIdx - 1;
      if (fi >= 0 && fi < fbos.length && fbos[fi]) {
        gl.deleteFramebuffer(fbos[fi]);
        fbos[fi] = null;
      }
    },

    // File I/O
    save_png(path, w, h, data) {
      const c2 = document.createElement('canvas');
      c2.width = w; c2.height = h;
      const ctx = c2.getContext('2d');
      const imgData = ctx.createImageData(w, h);
      for (let i = 0; i < data.length; i++) {
        const px = data[i];
        imgData.data[i * 4 + 0] = (px >>> 16) & 0xff;
        imgData.data[i * 4 + 1] = (px >>> 8) & 0xff;
        imgData.data[i * 4 + 2] = px & 0xff;
        imgData.data[i * 4 + 3] = (px >>> 24) & 0xff;
      }
      ctx.putImageData(imgData, 0, 0);
      c2.toBlob(blob => {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = path;
        a.click();
        URL.revokeObjectURL(a.href);
      });
    },

    // Font/text (see § Text above).  The handle is 0-based, as on the desktop
    // backend; a non-number or negative answer is the null handle on the loft side.
    gl_load_font(path) {
      const base = baseNameOf(path);
      if (!base) return -1;
      fonts.push({ family: familyFor(base), base: base });
      return fonts.length - 1;
    },
    gl_measure_text(fi, s, sz) {
      if (!fonts[fi]) return 0.0;
      return fontMetrics(fi, sz).cx.measureText(String(s)).width;
    },
    gl_text_height(fi, sz) {
      if (!fonts[fi]) return Math.ceil(sz * 1.2);
      return fontMetrics(fi, sz).line;
    },
    gl_font_ascent(fi, sz) {
      if (!fonts[fi]) return sz * 0.8;
      return fontMetrics(fi, sz).asc;
    },
    // `[width, Uint8Array alpha]` with rows `width` wide, or null when there is
    // nothing to draw.  The width is `ceil(measure_text)`, which the loft caller
    // sized its buffer from.
    gl_rasterize_text(fi, s, sz) {
      s = String(s);
      if (!fonts[fi] || !s) return null;
      const mt = fontMetrics(fi, sz);
      const w = Math.ceil(mt.cx.measureText(s).width);
      const h = mt.line;
      if (w <= 0 || h <= 0) return null;
      textCv.width = w; textCv.height = h;
      // Resizing the canvas resets its context state, so re-apply the font.
      mt.cx.font = sz + 'px ' + fonts[fi].family;
      mt.cx.clearRect(0, 0, w, h);
      mt.cx.fillStyle = '#fff';
      mt.cx.textBaseline = 'alphabetic';
      mt.cx.fillText(s, 0, mt.asc);
      const rgba = mt.cx.getImageData(0, 0, w, h).data;
      const alpha = new Uint8Array(w * h);
      for (let i = 0; i < alpha.length; i++) alpha[i] = rgba[i * 4 + 3];
      return [w, alpha];
    },

    // Input
    gl_key_pressed(keyCode) { return keys.has(keyCode); },
    gl_mouse_x() { return mouseX; },
    gl_mouse_y() { return mouseY; },
    gl_mouse_button() { return mouseBtn; },
  }));   // ← close coerceArgs()'s argument + Object.assign

  return ctrl;
}
