// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2c — the generic descriptor-driven reader: reconstruct a delivered loft value
// from wasm linear memory using ONLY its layout descriptor + the store base address, with no
// serialization. This MIRRORS `Stores::read_via_descriptor` (src/database/descriptor.rs) — keep
// the two in lockstep; the node harness (tests/deliver_wasm.rs) is the byte-parity gate.
//
// Addressing (Store::checked_offset):  addr(rec, pos) = storeBase + rec*8 + pos
//   * a RECORD field recurses at (rec, pos + field.pos) in the SAME record;
//   * a VECTOR field holds a child record-INDEX vRec (len at vRec*8+4, elem i at vRec*8+8+size*i);
//   * an ARRAY (by-ref vector) holds per-element record-indices (each element data at elmRec*8+8);
//   * TEXT is an interned record: the field holds a string id, len at id*8+4, UTF-8 at id*8+8;
//   * store-internal kinds (Ref / ChildRec / Iterated) are out of this subset — cursor-walked in
//     Phase 3 — exactly as `read_via_descriptor` refuses them.
//
// `desc` is the parsed `LayoutDesc::to_json` blob: {nodes:{<id>:node}, names, sizes}.
// `mem` is the WebAssembly.Memory; its `.buffer` MUST be re-read on each deliver (memory.grow
// detaches the old ArrayBuffer — §5 borrow contract). Read (or copy out) within the deliver call.

// NOTE: a plain function (no inline `export`) so it can be `include_str!`-embedded into the
// non-module `--html` shim; the trailing `export { readLoftValue }` (stripped at embed time) keeps
// the node harness's `import { readLoftValue }` working.
function readLoftValue(mem, storeBase, desc, typeId, rec, pos) {
  const view = new DataView(mem.buffer);
  const nodes = desc.nodes;
  const sizeOf = (id) => (desc.sizes && desc.sizes[id] != null ? +desc.sizes[id] : 0);
  // A threaded page's memory is shared, which a plain TextDecoder refuses (`loftTextDecoder`).
  const dec = typeof loftTextDecoder === 'function' ? loftTextDecoder() : new TextDecoder();

  function read(typeId, rec, pos) {
    const node = nodes[typeId];
    if (!node) throw new Error(`deliver: type ${typeId} not in descriptor`);
    const at = storeBase + rec * 8 + pos;
    switch (node.kind) {
      case "base":
        return readBase(node.base, at);
      case "byte": {
        // Narrow 1-byte int (Parts::Byte): stored as `(value - start)` so the packed byte spans
        // 0..255; a nullable byte reserves 255 for null. Re-add `start` to recover the signed value
        // (u8 start=0 → 0..255; i8 start=-128 → -128..127). NOTE: the Rust twin read_via_descriptor
        // emits the PACKED byte (a byte-parity artefact) — this reader reconstructs the true VALUE.
        const raw = view.getUint8(at);
        if (node.nullable && raw === 255) return null;
        return raw + node.start;
      }
      case "shortraw": {
        // Non-null narrow 2-byte int (Parts::ShortRaw): `(value - start)` as a u16, full range, no
        // reserved sentinel. Read UNSIGNED then re-add start (u16 start=0; i16 start=-32768).
        return view.getUint16(at, true) + node.start;
      }
      case "short": {
        // Nullable narrow 2-byte int (Parts::Short): `(value - start + 1)` as a u16 with 0 reserved
        // for null (so a nullable field holds 65535 distinct values). Undo the +1 shift.
        const raw = view.getUint16(at, true);
        if (node.nullable && raw === 0) return null;
        return raw + node.start - 1;
      }
      case "int":
        // 4-byte int (Parts::Int): stored directly as i32 — `start` is the null-sentinel boundary
        // (i32::MIN+1), NOT a packing offset, so no adjustment. (u32 > 2^31 is a known edge: it
        // reads back negative here, matching the Rust twin's get_i32_raw.)
        return view.getInt32(at, true);
      case "record":
      case "enumvalue": {
        const o = {};
        for (const f of node.fields) {
          // Skip the non-data fields read_data skips: the enum discriminant, absent fields, and any
          // `#`-prefixed SYNTHETIC field (e.g. an index node's #left/#right/#color tree bookkeeping).
          if (f.name === "enum" || f.pos === 65535 || f.name[0] === "#") continue;
          o[f.name] = read(f.content, rec, pos + f.pos);
        }
        return o;
      }
      case "enum": {
        // Plain (value) enum — Parts::Enum/Choices, stored 1-BASED (first variant = 1; 0 unused,
        // 255 = null sentinel), so the variant is `variants[disc - 1]`. The Rust twin emits the raw
        // 1-based byte; this reader resolves it to the variant NAME.
        const disc = view.getUint8(at);
        if (disc === 0 || disc === 255) return null;
        const v = node.variants[disc - 1];
        return v ? v.name : disc;
      }
      case "vector": {
        const vRec = view.getUint32(at, true);
        if (vRec === 0) return [];
        const len = view.getUint32(storeBase + vRec * 8 + 4, true);
        const size = sizeOf(node.elem);
        const fast = scalarFastLane(nodes[node.elem], storeBase + vRec * 8 + 8, len);
        if (fast) return fast; // zero-copy typed-array VIEW over wasm memory
        const a = new Array(len);
        for (let i = 0; i < len; i++) a[i] = read(node.elem, vRec, 8 + size * i);
        return a;
      }
      case "array": {
        const vRec = view.getUint32(at, true);
        if (vRec === 0) return [];
        const len = view.getUint32(storeBase + vRec * 8 + 4, true);
        const a = new Array(len);
        for (let i = 0; i < len; i++) {
          const elmRec = view.getUint32(storeBase + vRec * 8 + 8 + 4 * i, true);
          a[i] = read(node.elem, elmRec, 8);
        }
        return a;
      }
      case "flatarray": {
        // @PLN105 Phase 3 — a keyed collection pre-flattened at deliver time. The materialised data
        // record is NOT in the (type-shared) node; it is looked up in the `flat` redirect map by
        // this collection's `(rec, pos)` — so ONE `flatarray` node serves every instance (e.g. each
        // element of a `vector<Bag>`).
        //
        // `stride` says what an element is, and it is not always the same: 4 means a record
        // number whose payload starts at byte 8 (radix, trie, index), 8 means a
        // `(record, offset)` PAIR — a hash, whose entries are slots inside a chunked arena and
        // have no record of their own. Defaulting to 4 keeps a descriptor written before the
        // field existed readable.
        const dRec = (desc.flat && desc.flat[rec + "_" + pos]) || 0;
        if (dRec === 0) return [];
        const stride = node.stride || 4;
        const len = view.getUint32(storeBase + dRec * 8 + 4, true);
        const a = new Array(len);
        for (let i = 0; i < len; i++) {
          const at = storeBase + dRec * 8 + 8 + stride * i;
          const elmRec = view.getUint32(at, true);
          const elmPos = stride === 8 ? view.getUint32(at + 4, true) : 8;
          a[i] = read(node.elem, elmRec, elmPos);
        }
        return a;
      }
      case "ref":
      case "childrec":
      case "iterated":
        throw new Error(
          `deliver: type ${typeId} is store-internal (${node.kind}) — cursor-walked in Phase 3`,
        );
      default:
        throw new Error(`deliver: unknown node kind ${node.kind}`);
    }
  }

  function readBase(base, at) {
    switch (base) {
      case "integer":
      case "long":
        return view.getBigInt64(at, true); // loft integer/long are i64
      case "single":
        return view.getFloat32(at, true);
      case "float":
        return view.getFloat64(at, true);
      case "boolean":
        return view.getUint8(at) !== 0;
      case "character":
        return view.getUint32(at, true);
      case "text": {
        const strRec = view.getUint32(at, true);
        if (strRec === 0 || strRec > 0x7fffffff) return null; // STRING_NULL
        const len = view.getUint32(storeBase + strRec * 8 + 4, true);
        return dec.decode(new Uint8Array(mem.buffer, storeBase + strRec * 8 + 8, len));
      }
      default:
        throw new Error(`deliver: unknown base ${base}`);
    }
  }

  // The scalar-vector fast lane — the zero-copy win: a scalar element type maps the whole vector
  // to a typed-array VIEW straight over wasm memory (no per-element decode, no intermediate array).
  // Valid only during the borrow; copy out (Array.from / .slice()) to retain past the deliver call.
  function scalarFastLane(elem, byteBase, len) {
    if (!elem) return null;
    if (elem.kind === "int") return new Int32Array(mem.buffer, byteBase, len);
    if (elem.kind === "base") {
      switch (elem.base) {
        case "single":
          return new Float32Array(mem.buffer, byteBase, len);
        case "float":
          return new Float64Array(mem.buffer, byteBase, len);
        case "integer":
        case "long":
          return new BigInt64Array(mem.buffer, byteBase, len);
      }
    }
    return null;
  }

  return read(typeId, rec, pos);
}

export { readLoftValue };
