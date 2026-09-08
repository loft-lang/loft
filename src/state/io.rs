// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor

use super::{State, new_ref, size_ref};
use crate::database::{Parts, ShowDb};
use crate::keys::{Content, DbRef, Key};
use crate::{hash, tree, vector};
#[cfg(not(host_fs))]
use std::fs::{File, OpenOptions};
#[cfg(not(host_fs))]
use std::io::{Read, Seek, SeekFrom, Write};

impl State {
    /**
    Read data from a file
    # Panics
    When the reading was incorrect.
    */
    pub fn get_file_text(&mut self) {
        let r = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            return;
        }
        #[cfg(host_fs)]
        {
            // FS-B: read entire file via JS host bridge.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
            // @P349: consult VIRT_FS first — `compile_and_run` populates it with
            // every file passed to the playground/gallery (the program's bundled
            // data).  Without this, runtime `file("x").content()` only hit the JS
            // `fs_read_text` host bridge, which the browser playground does not
            // provide → empty string (the lexer already reads source/lib files
            // from VIRT_FS via `virt_fs_get`, but the runtime read did not).  A
            // real host with no VIRT_FS entry for the path still falls through to
            // its `host_fs_read_text` bridge, so live-FS hosts are unaffected.
            if let Some(text) = crate::wasm::virt_fs_get(&file_path) {
                *buf = text;
            } else if let Some(text) = crate::wasm::host_fs_read_text(&file_path) {
                *buf = text;
            }
        }
        // Everything else has a real filesystem: native, and `wasm32-wasip2`
        // through its `--dir` preopens.  `--html` used to need a third arm here
        // — an explicit stub, because `std::fs::File::open` compiles on
        // wasm32-unknown-unknown but fails at runtime in whatever way the
        // embedding's panic hook decides.  It reaches the host bridge above
        // instead now (loft#851), which is the answer that stub was standing in
        // for all along.
        #[cfg(not(host_fs))]
        {
            let store = self.database.store(&file);
            let raw_path = store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned();
            // #255 / @PLN9: re-home a relative path against the program anchor.
            let path_string = self.database.resolve_path(&raw_path);
            let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
            // One home for the read and its warning, shared with native codegen's
            // `OpGetFileText` — the warning used to live here only, so `--native`
            // read a binary file in complete silence (loft#829).
            crate::codegen_runtime::read_file_text_into(&path_string, buf);
        }
    }

    /// Assemble the raw bytes to write for `val` of type `db_tp`.
    /// Shared by the native and WASM write paths.
    fn assemble_write_data(&self, val: DbRef, db_tp: u16, little_endian: bool) -> Vec<u8> {
        let mut data = Vec::new();
        if self.database.is_text_type(db_tp) {
            let store = self.database.store(&val);
            let s: &String = store.addr::<String>(val.rec, val.pos);
            data.extend_from_slice(s.as_bytes());
        } else if matches!(
            &self.database.types[db_tp as usize].parts,
            Parts::Byte(_, _)
                | Parts::Short(_, _)
                | Parts::ShortRaw(_, _)
                | Parts::Int(_, _)
                | Parts::IntRaw(_, _)
        ) {
            // @FR-L-Narrow classifies the width; the ENCODING here is deliberately not the
            // one that rule's field form uses, so this site must not be folded onto
            // `write_narrow_value` however alike the two type lists look.
            // Narrow-integer values sit in an 8B variable slot stored raw as i64
            // (OpPutInt), not via the +1-encoded Parts::Byte/Short encoding that
            // structs use (nor the i32::MIN null sentinel of Parts::Int).
            // Read the raw i64 and serialise directly.
            let v = self.database.store(&val).get_int(val.rec, val.pos);
            match &self.database.types[db_tp as usize].parts {
                Parts::Byte(_, _) => data.push(v as u8),
                Parts::Short(_, _) | Parts::ShortRaw(_, _) => {
                    let b = if little_endian {
                        (v as u16).to_le_bytes()
                    } else {
                        (v as u16).to_be_bytes()
                    };
                    data.extend_from_slice(&b);
                }
                Parts::Int(_, _) => {
                    let b = if little_endian {
                        (v as i32).to_le_bytes()
                    } else {
                        (v as i32).to_be_bytes()
                    };
                    data.extend_from_slice(&b);
                }
                Parts::IntRaw(_, _) => {
                    let b = if little_endian {
                        (v as u32).to_le_bytes()
                    } else {
                        (v as u32).to_be_bytes()
                    };
                    data.extend_from_slice(&b);
                }
                _ => unreachable!(),
            }
        } else if let Parts::Vector(elem_tp) = &self.database.types[db_tp as usize].parts {
            let elem_tp = *elem_tp;
            let vec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
            let (v_ptr, store_nr) = {
                let store = self.database.store(&vec_ref);
                (
                    store.get_u32_raw(vec_ref.rec, vec_ref.pos),
                    vec_ref.store_nr,
                )
            };
            if v_ptr != 0 {
                let length = self.database.allocations[store_nr as usize].get_u32_raw(v_ptr, 4);
                let elem_size = u32::from(self.database.size(elem_tp));
                for i in 0..length {
                    let elem = DbRef {
                        store_nr,
                        rec: v_ptr,
                        pos: 8 + elem_size * i,
                    };
                    self.database
                        .read_data(&elem, elem_tp, little_endian, &mut data);
                }
            }
        } else if matches!(
            &self.database.types[db_tp as usize].parts,
            Parts::Struct(_) | Parts::EnumValue(_, _)
        ) {
            // @PLN85 p9: like the Vector arm, a struct's RECORD DbRef is stored in
            // the slot `val` refs — deref it and serialise the RECORD's fields.
            // Serialising `val` directly read the DbRef bytes AS fields and wrote
            // them (garbage) to the file; the read then filled the reader's record
            // with that garbage.  Native derefs (FileVal for DbRef); this was the
            // interp-only write half of the same defect.
            let rec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
            self.database
                .read_data(&rec_ref, db_tp, little_endian, &mut data);
        } else {
            self.database
                .read_data(&val, db_tp, little_endian, &mut data);
        }
        data
    }

    pub fn write_file(&mut self) {
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let format = self
            .database
            .store(&file)
            .get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists (default to TextFile).
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        // `+=` is append-only: when the user has not explicitly set
        // `f#next = N`, the write position is the current end of file
        // so successive `f += …` calls append (consistent with
        // `vector += [elem]` / `text += "more"`).  An explicit
        // `f#next = N` still overwrites at offset N.  Call
        // `f.set_file_size(0)` before the first write to truncate.
        let next_pos = if raw_next == i64::MIN {
            #[cfg(host_fs)]
            {
                let file_path = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                let sz = crate::wasm::host_fs_file_size(&file_path);
                if sz < 0 { 0 } else { sz }
            }
            #[cfg(not(host_fs))]
            {
                let path = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                // #255 / @PLN9: re-home against the program anchor.
                let path = self.database.resolve_path(&path);
                std::fs::metadata(&path).map_or(0, |m| m.len() as i64)
            }
        } else {
            raw_next
        };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        let data = self.assemble_write_data(val, db_tp, little_endian);
        let written = data.len();
        #[cfg(host_fs)]
        {
            // FS-C: seek to position then write; VirtFS writeBytes handles extension.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, next_pos);
            crate::wasm::host_fs_write_bytes(&file_path, &data);
        }
        #[cfg(not(host_fs))]
        {
            let f_nr = self.database.files.len() as i32;
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            let file_ref = if file_ref == i32::MIN {
                let file_name = {
                    let store = self.database.store(&file);
                    store
                        .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                        .to_owned()
                };
                // #255 / @PLN9: re-home against the program anchor.
                let file_name = self.database.resolve_path(&file_name);
                // Open for read+write without truncating so that earlier
                // bytes are preserved.  Create the file if it does not
                // exist yet.  Explicit truncation happens via
                // `f.set_file_size(0)` (or `f#size = 0`).
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&file_name)
                {
                    Ok(mut f) => {
                        // Seek to the stored write position (end of file
                        // for default appends, explicit offset for
                        // `f#next = N`).
                        let _ = f.seek(SeekFrom::Start(next_pos as u64));
                        self.database
                            .store_mut(&file)
                            .set_i32_raw(file.rec, file.pos + 28, f_nr);
                        if format == 5 {
                            self.database
                                .store_mut(&file)
                                .set_byte(file.rec, file.pos + 32, 0, 1);
                        }
                        self.database.files.push(Some(f));
                        f_nr
                    }
                    Err(e) => {
                        eprintln!("file open error for {file_name:?}: {e}");
                        return;
                    }
                }
            } else {
                // Handle already open: sync OS handle with stored next_pos so
                // the user can `f#next = N` to overwrite bytes in place.
                if let Some(f) = &mut self.database.files[file_ref as usize] {
                    let _ = f.seek(SeekFrom::Start(next_pos as u64));
                }
                file_ref
            };
            if let Some(f) = &mut self.database.files[file_ref as usize]
                && let Err(e) = f.write_all(&data)
            {
                eprintln!("file write error: {e}");
            }
        }
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 16, next_pos + written as i64);
    }

    /// Dispatch read bytes into `val` of type `db_tp`.
    /// Shared by the native and WASM read paths.
    fn dispatch_read_data(
        &mut self,
        val: DbRef,
        db_tp: u16,
        little_endian: bool,
        data: Vec<u8>,
        n: usize,
    ) {
        let actual = data.len();
        let is_text = self.database.is_text_type(db_tp);
        if is_text {
            let s = unsafe { String::from_utf8_unchecked(data) };
            *self
                .database
                .store_mut(&val)
                .addr_mut::<String>(val.rec, val.pos) = s;
        } else if actual == n {
            if let Parts::Vector(_) = &self.database.types[db_tp as usize].parts {
                let vec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
                self.database
                    .write_data(&vec_ref, db_tp, little_endian, &data);
            } else if matches!(
                &self.database.types[db_tp as usize].parts,
                Parts::Byte(_, _)
                    | Parts::Short(_, _)
                    | Parts::ShortRaw(_, _)
                    | Parts::Int(_, _)
                    | Parts::IntRaw(_, _)
            ) {
                // @FR-L-Narrow classifies the width; the raw-slot ENCODING below is not the
                // field form, so this is not a fold candidate — see the write twin above.
                // Narrow-integer reads (byte/short/int) target a u8/u16/i32
                // variable whose stack slot is 8 bytes (Phase 2c integer
                // width) and holds a raw i64 — not the +1-encoded form that
                // Parts::Byte/Short use in struct fields, nor the i32::MIN
                // null sentinel Parts::Int uses.  Decode the bytes into the
                // raw i64.  @PLN47 W2/W3: a SIGNED narrow type (`i8`/`i16`,
                // whose range `from` is negative) must sign-extend; an
                // unsigned one (`u8`/`u16`, `from >= 0`) zero-extends.  The
                // native backend reads these as `i16`/`i8` and lets the slot
                // type wrap; the interpreter holds a raw i64, so it must pick
                // the extension explicitly or a signed value read back as a
                // large positive (`-12345 as i16` → 53191) — a silent
                // corruption that diverged from native.
                let v: i64 = match &self.database.types[db_tp as usize].parts {
                    Parts::Byte(from, _) => {
                        if *from < 0 {
                            i64::from(data[0] as i8)
                        } else {
                            i64::from(data[0])
                        }
                    }
                    Parts::Short(from, _) | Parts::ShortRaw(from, _) => {
                        let d: [u8; 2] = data[0..2].try_into().unwrap();
                        let signed = *from < 0;
                        match (little_endian, signed) {
                            (true, true) => i64::from(i16::from_le_bytes(d)),
                            (true, false) => i64::from(u16::from_le_bytes(d)),
                            (false, true) => i64::from(i16::from_be_bytes(d)),
                            (false, false) => i64::from(u16::from_be_bytes(d)),
                        }
                    }
                    Parts::Int(_, _) => {
                        let d: [u8; 4] = data[0..4].try_into().unwrap();
                        if little_endian {
                            i64::from(i32::from_le_bytes(d))
                        } else {
                            i64::from(i32::from_be_bytes(d))
                        }
                    }
                    // The unsigned 4-byte width ZERO-extends, for the same reason `u8`
                    // and `u16` do one and two widths down: sign-extending it reads every
                    // value past 2147483647 back as a negative number.
                    Parts::IntRaw(_, _) => {
                        let d: [u8; 4] = data[0..4].try_into().unwrap();
                        if little_endian {
                            i64::from(u32::from_le_bytes(d))
                        } else {
                            i64::from(u32::from_be_bytes(d))
                        }
                    }
                    _ => unreachable!(),
                };
                self.database.store_mut(&val).set_int(val.rec, val.pos, v);
            } else if matches!(
                &self.database.types[db_tp as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                // @PLN85 p9: like the Vector arm, a struct's RECORD DbRef is
                // stored in the slot `val` refs — deref it and fill the record.
                // Writing into `val` directly filled the eval-stack slot, so the
                // record was never populated and the delivered value was the slot
                // bytes reinterpreted as a DbRef.  Native derefs (FileVal for
                // DbRef); this was the interp-only read half.
                let rec_ref = *self.database.store(&val).addr::<DbRef>(val.rec, val.pos);
                self.database
                    .write_data(&rec_ref, db_tp, little_endian, &data);
            } else {
                self.database.write_data(&val, db_tp, little_endian, &data);
            }
        }
    }

    pub fn read_file(&mut self) {
        let bytes = *self.get_stack::<i64>() as i32;
        let val = *self.get_stack::<DbRef>();
        let file = *self.get_stack::<DbRef>();
        let db_tp = self.code::<u16>();
        if file.rec == 0 {
            return;
        }
        let format = self
            .database
            .store(&file)
            .get_byte(file.rec, file.pos + 32, 0);
        // format: 1=TextFile, 2=LittleEndian, 3=BigEndian, 5=NotExists.
        if format != 1 && format != 5 && format != 2 && format != 3 {
            return;
        }
        let little_endian = format == 2;
        let raw_next = self.database.store(&file).get_long(file.rec, file.pos + 16);
        let next_pos = if raw_next == i64::MIN { 0 } else { raw_next };
        self.database
            .store_mut(&file)
            .set_long(file.rec, file.pos + 8, next_pos);
        let n = bytes as usize;
        #[cfg(host_fs)]
        {
            // FS-D: seek JS cursor to position then read n bytes.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, next_pos);
            if let Some(data) = crate::wasm::host_fs_read_bytes(&file_path, n) {
                let actual = data.len();
                self.database.store_mut(&file).set_long(
                    file.rec,
                    file.pos + 16,
                    next_pos + actual as i64,
                );
                self.dispatch_read_data(val, db_tp, little_endian, data, n);
            }
        }
        #[cfg(not(host_fs))]
        {
            let f_nr = self.database.files.len() as i32;
            // #255 / @PLN9: resolve against the program anchor before borrowing
            // the store mutably (resolve_path needs a shared borrow).
            let resolved = {
                let s = self.database.store(&file);
                let raw = s.get_str(s.get_u32_raw(file.rec, file.pos + 24)).to_owned();
                self.database.resolve_path(&raw)
            };
            let resolved_name = resolved;
            let store = self.database.store_mut(&file);
            let mut file_ref = store.get_i32_raw(file.rec, file.pos + 28);
            if file_ref == i32::MIN {
                match File::open(&resolved_name) {
                    Ok(mut f) => {
                        // apply stored seek position on first open.
                        if next_pos != 0 {
                            let _ = f.seek(SeekFrom::Start(next_pos as u64));
                        }
                        store.set_i32_raw(file.rec, file.pos + 28, f_nr);
                        self.database.files.push(Some(f));
                        file_ref = f_nr;
                    }
                    Err(e) => {
                        // A failed open must NOT leave a dangling ref — indexing
                        // `files[]` with it panicked.  Warn and skip the read (the
                        // recoverable-fault posture), mirroring both the write
                        // path's create fix and the native runtime
                        // (`file_handle_read` → i32::MIN → return).
                        eprintln!("file open error for {resolved_name:?}: {e}");
                        return;
                    }
                }
            } else if let Some(Some(f)) = self.database.files.get_mut(file_ref as usize) {
                // Handle already open: user may have set #next explicitly to seek.
                // Sync the OS file handle with the stored next_pos.
                let _ = f.seek(SeekFrom::Start(next_pos as u64));
            }
            let is_text = self.database.is_text_type(db_tp);
            let mut data = vec![0u8; n];
            let actual = if let Some(Some(f)) = self.database.files.get_mut(file_ref as usize) {
                if is_text {
                    f.read(&mut data).unwrap_or_else(|e| {
                        eprintln!("file read error: {e}");
                        0
                    })
                } else if f.read_exact(&mut data).is_ok() {
                    n
                } else {
                    0
                }
            } else {
                0
            };
            self.database.store_mut(&file).set_long(
                file.rec,
                file.pos + 16,
                next_pos + actual as i64,
            );
            if is_text {
                data.truncate(actual);
            }
            self.dispatch_read_data(val, db_tp, little_endian, data, n);
        }
    }

    pub fn seek_file(&mut self) {
        let pos = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            return;
        }
        #[cfg(host_fs)]
        {
            // FS-D: seek JS-side cursor and update store #next position.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            crate::wasm::host_fs_seek(&file_path, pos);
            self.database
                .store_mut(&file)
                .set_long(file.rec, file.pos + 16, pos);
        }
        #[cfg(not(host_fs))]
        {
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            if file_ref == i32::MIN {
                // File not yet open — store the seek position in #next so the first
                // read/write applies it after opening the file.
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 16, pos);
            } else if let Some(f) = &mut self.database.files[file_ref as usize]
                && let Err(e) = f.seek(SeekFrom::Start(pos as u64))
            {
                eprintln!("file seek error: {e}");
            }
        }
    }

    pub fn size_file(&mut self) {
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(i64::MIN);
            return;
        }
        #[cfg(host_fs)]
        {
            // FS-D: get file size from JS host bridge.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let size = crate::wasm::host_fs_file_size(&file_path);
            self.put_stack(if size < 0 { i64::MIN } else { size });
        }
        #[cfg(not(host_fs))]
        {
            let store = self.database.store(&file);
            let file_path = store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned();
            // #255 / @PLN9: re-home against the program anchor.
            let file_path = self.database.resolve_path(&file_path);
            let size = std::fs::metadata(&file_path).map_or(i64::MIN, |meta| meta.len() as i64);
            self.put_stack(size);
        }
    }

    pub fn sync_file(&mut self) {
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(false);
            return;
        }
        #[cfg(host_fs)]
        {
            let _ = file;
            self.put_stack(false);
        }
        #[cfg(not(host_fs))]
        {
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            let ok = if file_ref != i32::MIN
                && (file_ref as usize) < self.database.files.len()
                && let Some(f) = &self.database.files[file_ref as usize]
            {
                f.sync_data().is_ok()
            } else {
                true
            };
            self.put_stack(ok);
        }
    }

    pub fn truncate_file(&mut self) {
        let size = *self.get_stack::<i64>();
        let file = *self.get_stack::<DbRef>();
        if file.rec == 0 {
            self.put_stack(false);
            return;
        }
        #[cfg(host_fs)]
        {
            // FS-D: truncate by reading current content, slicing, and rewriting.
            let file_path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            let ok = if let Some(mut bytes) = crate::wasm::host_fs_read_binary(&file_path) {
                let new_len = size.max(0) as usize;
                bytes.truncate(new_len);
                bytes.resize(new_len, 0);
                crate::wasm::host_fs_write_binary(&file_path, &bytes) == 0
            } else {
                false
            };
            self.put_stack(ok);
        }
        #[cfg(not(host_fs))]
        {
            let path = {
                let store = self.database.store(&file);
                store
                    .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                    .to_owned()
            };
            // #255 / @PLN9: re-home against the program anchor.
            let path = self.database.resolve_path(&path);
            // Close any open handle: the handle may be in read or write mode with a stale
            // position, and after resize the position might be beyond the new end of file.
            let file_ref = self
                .database
                .store(&file)
                .get_i32_raw(file.rec, file.pos + 28);
            if file_ref != i32::MIN && (file_ref as usize) < self.database.files.len() {
                self.database.files[file_ref as usize] = None;
                self.database
                    .store_mut(&file)
                    .set_i32_raw(file.rec, file.pos + 28, i32::MIN);
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 8, i64::MIN);
                self.database
                    .store_mut(&file)
                    .set_long(file.rec, file.pos + 16, i64::MIN);
            }
            let ok = OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_len(size as u64))
                .is_ok();
            self.put_stack(ok);
        }
    }

    pub fn free_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        self.free_ref_db(db);
    }

    /// Plan-57 store-identity gate (`OpFreeRefTag`): pop the DbRef, read the
    /// allocation-site `tag` operand, verify it against the store's recorded tag
    /// (a mismatch means a wrong-store / cross-owner free that `free_named` would
    /// otherwise silently no-op), then free.  Verification build only.
    ///
    /// # Panics
    /// Panics (release included) on a store-tag mismatch — a wrong-store /
    /// cross-owner free.  Only reachable when the `LOFT_STORE_TAG` gate emitted
    /// this op (a deliberate testing build); the op is absent otherwise.
    pub fn free_ref_tag(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let tag = u32::from(self.code::<u16>());
        if db.store_nr != u16::MAX && (db.store_nr as usize) < self.database.allocations.len() {
            let st = &self.database.allocations[db.store_nr as usize];
            // Hard assert (not debug_assert): this op only exists when the
            // LOFT_STORE_TAG gate emitted it — a deliberate testing build — so it
            // must fire in release too. Zero cost when the gate is off (op absent).
            // `tag == 0` covers a slot reused by a NON-`OpDatabase` allocator (e.g.
            // `file()`) after a tagged store freed it — `free_named` resets the tag
            // to 0, so the stale tag never false-positives.  What remains a hard
            // error: a free of a REUSED store re-tagged by a different `OpDatabase`
            // owner (the wrong-store / cross-owner free this gate exists to catch).
            assert!(
                st.free || st.tag == 0 || st.tag == tag,
                "store-tag mismatch on free: store #{} has tag {} but OpFreeRefTag expected {}",
                db.store_nr,
                st.tag,
                tag,
            );
        }
        self.free_ref_db(db);
    }

    /// Plan-57 store-identity gate (`OpStoreTag`): pop the DbRef, read the
    /// allocation-site `tag` operand, stamp it on the store so a later
    /// `OpFreeRefTag` can verify the free corresponds.  Verification build only.
    pub fn store_tag(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let tag = u32::from(self.code::<u16>());
        if db.store_nr != u16::MAX && (db.store_nr as usize) < self.database.allocations.len() {
            self.database.allocations[db.store_nr as usize].tag = tag;
        }
    }

    /// The body of [`free_ref`], factored so [`free_ref_tag`] reuses it after the
    /// DbRef is already popped (the tag operand sits between the popped ref and the
    /// free in the tagged op).
    pub fn free_ref_db(&mut self, db: DbRef) {
        // S37: coroutine DbRefs use store_nr == COROUTINE_STORE (u16::MAX).
        // database.free() is a no-op for this sentinel.  Free the coroutine frame
        // explicitly so that text_owned, stack_bytes, and call_frames are released
        // when a `for` loop exits early (before the generator exhausts).
        if db.store_nr == super::COROUTINE_STORE {
            self.free_coroutine(&db);
            return;
        }
        // Plan-57 Phase C: single-ownership (ref-count removed) — close the OS file
        // handle whenever its File store is freed (free_named frees unconditionally).
        #[cfg(not(host_fs))]
        self.database.close_file_handle(&db);
        self.database.free(&db);
    }

    pub fn format_database(&mut self) {
        let pos = self.code::<u16>();
        let s = self.format_db();
        // @PLAN53 cluster 2 / S4 (2d): format_db pops the composite value's 12-byte
        // DbRef, which advances TOS by stack_step(12) = 16 under LOFT_ALIGN (12 off).
        // The destination-buffer offset must back up by the SAME stepped width, else
        // the String slot is read 4 bytes low and the composite renders empty.  The
        // DbRef (size_ref=12) is the only non-8-multiple here, so it is the lone
        // divergent term.  Identity flag-OFF (stack_step(12) == 12).
        let off = pos - self.stack_step(size_ref()) as u16;
        self.string_mut(off).push_str(&s);
    }

    pub fn format_stack_database(&mut self) {
        let pos = self.code::<u16>();
        let s = self.format_db();
        // @PLAN53 cluster 2 / S4 (2d): see format_database — back up by the stepped
        // DbRef width so the destination String slot is read on its 8-byte boundary.
        let off = pos - self.stack_step(size_ref()) as u16;
        self.string_ref_mut(off).push_str(&s);
    }

    pub fn sizeof_ref(&mut self) {
        let db = *self.get_stack::<DbRef>();
        let new_value: i64 = if db.rec == 0 {
            0
        } else {
            let db_tp = self.database.store(&db).get_int(db.rec, 4) as u16;
            i64::from(self.database.size(db_tp))
        };
        self.put_stack(new_value);
    }

    pub(super) fn format_db(&mut self) -> String {
        let db_tp = self.code::<u16>();
        let format = self.code::<u8>();
        let val = *self.get_stack::<DbRef>();
        // validate DbRef and known_type before raw pointer access
        #[cfg(debug_assertions)]
        {
            assert!(
                val.store_nr == u16::MAX
                    || (val.store_nr as usize) < self.database.allocations.len(),
                "format_db: store_nr={} out of range (allocations.len={}), \
                 rec={}, pos={}, db_tp={}",
                val.store_nr,
                self.database.allocations.len(),
                val.rec,
                val.pos,
                db_tp
            );
            assert!(
                (db_tp as usize) < self.database.types.len(),
                "format_db: db_tp={} out of range (types.len={}), \
                 store_nr={}, rec={}, pos={}",
                db_tp,
                self.database.types.len(),
                val.store_nr,
                val.rec,
                val.pos
            );
        }
        let mut s = String::new();
        ShowDb {
            stores: &self.database,
            store: val.store_nr,
            rec: val.rec,
            pos: val.pos,
            known_type: db_tp,
            pretty: format & 1 > 0,
            json: format & 2 > 0,
            loft: false,
            dump: false,
            compact: false,
            max_depth: u16::MAX,
            max_elements: u16::MAX,
        }
        .write(&mut s, 0);
        s
    }

    pub fn database(&mut self) {
        let var = self.code::<u16>();
        let db_tp = self.code::<u16>();
        let code_pos = self.code_pos;
        self.alloc_record_at(var, db_tp, code_pos);
    }

    /// Allocate a fresh `db_tp` record into variable slot `var`, reusing the slot's
    /// current store in place (`clear` + `claim`) or a fresh store when the slot is
    /// null/sentinel. The core of `OpDatabase`; also used by `bind_or_copy` (@PLN85).
    fn alloc_record_at(&mut self, var: u16, db_tp: u16, code_pos: u32) {
        // B2-runtime: for EnumValue types, the record must be large enough
        // for the parent enum's largest variant.  The parent's size covers
        // all variants; the variant's own size may be smaller (unit variants).
        let size = self.database.enum_parent_size(db_tp);
        let mut db = *self.get_var::<DbRef>(var);
        if crate::keys::trace_db() {
            eprintln!(
                "[db] OpDatabase var={var} db_tp={db_tp} db=#{}@{},{} size={size}",
                db.store_nr, db.rec, db.pos,
            );
        }
        // @PLAN51 Cluster II Step 2 — handle u16::MAX sentinel inputs.
        // Mirrors the native runtime (`codegen_runtime.rs::OpDatabase`
        // line 214) so the bytecode VM's `database` opcode accepts the
        // same sentinel a caller-side `OpInitRefSentinel` reset
        // produces.  Without this, `clear()` below would OOB into
        // `allocations[u16::MAX as usize]`.
        if db.store_nr == u16::MAX {
            db = self.database.null();
            *self.mut_var::<DbRef>(var) = db;
        }
        let r = self.alloc_record_into(&db, db_tp, size, code_pos);
        let db = self.mut_var::<DbRef>(var);
        db.store_nr = r.store_nr;
        db.rec = 1;
        db.pos = 8;
    }

    /// Claim a fresh `db_tp` record of `size` payload bytes in `db`'s store and
    /// stamp its header — the store-side half of [`alloc_record_at`], split out
    /// so a caller that has no variable slot to write back to (the entry frame's
    /// hidden return buffer, #618) allocates by exactly the same rules.
    /// The returned `DbRef` addresses the record's payload.
    fn alloc_record_into(&mut self, db: &DbRef, db_tp: u16, size: u16, code_pos: u32) -> DbRef {
        self.database.clear(db);
        // The record layout is: word 0 = size header (4 B) + type tag (4 B),
        // payload from byte 8.  `Stores::claim` takes WORDS, so the payload's
        // `size` BYTES need `1 + ceil(size / 8)` words — passing `size` raw
        // under-claims for 1-byte payloads (a single-boolean struct/closure
        // record got zero payload bytes and wrote into the next word).
        let r = self.database.claim(db, 1 + u32::from(size).div_ceil(8));
        self.database.allocations[r.store_nr as usize].set_created_at(code_pos);
        // P259 commit 3 — record the type allocated into this store so
        // free_named can recognise closure-record stores at free time
        // (cascade-free walks `__closure_*` records' DbRef fields).
        self.database.allocations[r.store_nr as usize].set_known_type(db_tp);
        self.database
            .store_mut(&r)
            .set_u32_raw(r.rec, 4, u32::from(db_tp));
        self.database.set_default_value(db_tp, &r);
        DbRef {
            store_nr: r.store_nr,
            rec: 1,
            pos: 8,
        }
    }

    /// Allocate the hidden heap return buffer an entry function's caller is
    /// contracted to supply, for a `ref_return`-promoted attr of type `t`
    /// (#618).  `None` when the type has no shared-store schema name — the
    /// caller then falls back to the null sentinel, i.e. today's behaviour.
    ///
    /// An ordinary call site emits `OpDatabase` on a local before the call
    /// (`__ref_1 = null; OpDatabase(__ref_1, main_vector<integer>); get(__ref_1)`);
    /// `execute_argv` has no bytecode to do that, so it allocates here instead.
    pub(crate) fn alloc_hidden_return_buffer(
        &mut self,
        data: &crate::data::Data,
        t: &crate::data::Type,
    ) -> Option<DbRef> {
        let tname = crate::native_lib::hidden_dest_type_name(data, t)?;
        let db_tp = self.database.name(&tname);
        if db_tp == u16::MAX {
            return None;
        }
        // B2-runtime: EnumValue records must fit the parent enum's largest variant.
        let size = self.database.enum_parent_size(db_tp);
        let fresh = self.database.null();
        Some(self.alloc_record_into(&fresh, db_tp, size, self.code_pos))
    }

    pub fn new_record(&mut self) {
        let parent_tp = self.code::<u16>();
        let fld = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        let new_value = self.database.record_new(&data, parent_tp, fld);
        self.database.set_default_value(
            if fld == u16::MAX {
                self.database.content(parent_tp)
            } else {
                self.database
                    .content(self.database.field_type(parent_tp, fld))
            },
            &new_value,
        );
        self.put_stack(new_value);
    }

    /// `OpGetRecord` — the keyed lookup `c[k]`.  Its one exit answers the record as a VALUE:
    /// `nullref` for a miss, whichever arm below missed (`DbRef::or_null`, @FR-L-Null).
    /// `codegen_runtime::OpGetRecord` is the native twin, normalised at its exit the same way.
    pub fn get_record(&mut self) {
        let (db_tp, key) = self.read_key(false);
        let data = *self.get_stack::<DbRef>();
        let res = if data.rec == 0 {
            DbRef::NULL
        } else {
            // @PLN129 arc A — the miss path consults a bound source; identical to
            // `find` for an unbound collection, which is every collection today.
            //
            // @PLN133 S8 — split HERE rather than inside `Stores`, so the
            // `&mut Stores` borrow ends between the miss and the fetch. That is
            // the whole structural change: a fetch that is loft code has to run
            // where a loft function CAN be run, and `Stores` is not that place.
            let found = self.database.find(&data, db_tp, &key);
            if found.rec != 0 {
                found // resident: no query, no source, ordinary loft speed
            } else if let Some(source) = self
                .database
                .lazy_loft_source(&data, self.has_lazy_driver(db_tp))
            {
                // @PLN133 S8 — a backend core has no Rust driver for. The fetch
                // is loft code, and the retry after it is arc A's rule
                // unchanged: the collection stays the only authority on what is
                // resident, so this never hands back what the fetch returned.
                self.lazy_fetch_through_loft(&data, db_tp, &key, &source);
                self.database.find(&data, db_tp, &key)
            } else {
                self.database.fetch_missing(&data, db_tp, &key)
            }
        };
        self.put_stack(res.or_null());
    }

    /// @PLN133 S9 — does this program declare a driver for what this collection
    /// holds?
    ///
    /// The question that decides whether a source core CAN drive is nonetheless
    /// served by loft. Answered here rather than in `Stores`, because only
    /// `State` reaches `Data`; `--native` answers the same question off the table
    /// generated `init()` filled, and the two must agree or one backend quietly
    /// takes a different path through the same program.
    ///
    /// A malformed driver set answers `true`: the refusal then travels down the
    /// loft path and is REPORTED, where answering `false` would silently fall
    /// back to Rust and swallow the mistake.
    fn has_lazy_driver(&self, db_tp: u16) -> bool {
        if self.data_ptr.is_null() {
            return false;
        }
        let data: &crate::data::Data = unsafe { &*self.data_ptr };
        let element = self.database.element_type_name(db_tp);
        match data.lazy_fetch_driver_for(element) {
            Ok(found) => found.is_some(),
            Err(_) => true,
        }
    }

    /// @PLN133 S8 — run the program's own lazy driver for one missing key.
    ///
    /// The driver is a loft function, and everything about this is deliberately
    /// the ordinary call it looks like: it receives the COLLECTION, so what it
    /// inserts lands in the very collection the lookup is walking, and the caller
    /// then re-runs the lookup.
    ///
    /// ```loft
    /// fn lazy_fetch(coll: hash<Person[id]>, source: text,
    ///               key_int: integer, key_text: text) -> integer
    /// ```
    ///
    /// **S9 — the driver is chosen by the collection's ELEMENT TYPE.** A program
    /// may declare one per type (`lazy_fetch`, `lazy_fetch_orders`, …), and what
    /// each serves is read off its collection parameter rather than its name.
    /// Before this, a program's single driver was called for every lazily-bound
    /// collection whatever it was declared for, which inserted one element type's
    /// record into another's collection — a wrong VALUE, on both backends.
    ///
    /// Answer `1` when the record was inserted, `0` when the source was reached
    /// and does not hold that key. **The two are not the same fact and must not
    /// read the same** (@PLN129 arc C): "no such person" is stable and true,
    /// "the source is down" is neither — so a driver that cannot reach its
    /// source reports the reason through `store_lazy_fail` and the lookup
    /// answers null.
    ///
    /// A FAULT inside the driver is contained rather than propagated
    /// ([`State::run_until_return`]): a buggy driver must not turn a lookup into
    /// a program halt, because that would make moving the source from Rust to
    /// loft a regression against what @PLN129 already ships.
    fn lazy_fetch_through_loft(
        &mut self,
        coll: &DbRef,
        db_tp: u16,
        key: &[crate::keys::Content],
        source: &str,
    ) {
        if self.data_ptr.is_null() {
            self.database.lazy_fail(
                coll,
                "a lazy fetch needs a running program to call its driver",
            );
            return;
        }
        let data: &crate::data::Data = unsafe { &*self.data_ptr };
        let element = self.database.element_type_name(db_tp).to_string();
        let d_nr = match data.lazy_fetch_driver_for(&element) {
            Ok(Some(d)) => d,
            Ok(None) => {
                // Failure path 1, and it is the whole reason `Loft` is its own
                // source kind: a backend that is not here must read as
                // UNREACHABLE and name itself. Before this, `postgres://…`
                // classified as a `.store` image and reported something about a
                // paged reader.
                //
                // S9 — it names the ELEMENT TYPE, because that is what has no
                // driver. A program with a driver for another type used to reach
                // that one instead and fill this collection with the wrong
                // records; now it reads as a collection nothing serves, and the
                // message says which one to write.
                self.database.lazy_fail(
                    coll,
                    &crate::database::lazy::no_lazy_driver(source, &element),
                );
                return;
            }
            Err(why) => {
                self.database.lazy_fail(coll, &why);
                return;
            }
        };
        // One key, whichever spelling it arrived in — the same limit the paged
        // loader has, and for the same reason: a composite key needs @PLN125's
        // associated types to carry it, and fetching the WRONG row is worse than
        // answering absent.
        let (key_int, key_text) = match key {
            [crate::keys::Content::Long(k)] => (*k, String::new()),
            [crate::keys::Content::Str(s)] => (0, s.str().to_string()),
            _ => {
                self.database.lazy_fail(
                    coll,
                    "a composite key cannot be fetched — a driver is given one key",
                );
                return;
            }
        };
        let args = [
            crate::state::LoftArg::Ref(*coll),
            crate::state::LoftArg::Text(source),
            crate::state::LoftArg::Int(key_int),
            crate::state::LoftArg::Text(&key_text),
        ];
        // The teardown the fault cannot run. Every store the driver creates is
        // remembered for the length of the call; on a fault they are what its
        // abandoned frames held, and freeing them is what stops a traversal over
        // an unreachable source leaking once per failed fetch.
        let outer_watch = self.database.lazy_watch_begin();
        let answer = self.run_until_return(d_nr, &args);
        self.database.lazy_watch_end(outer_watch, answer.is_err());
        match answer {
            // Inserted, or a genuine absence. Neither clears an earlier failure:
            // a success says nothing about what a partial traversal already
            // lost, and reporting "healthy" afterwards is exactly the silent
            // wrong answer arc C exists to prevent.
            Ok(1 | 0) => {}
            // A driver that answered something else did not follow the contract,
            // and guessing which of the two it meant is how an unreachable
            // source starts reading as an empty table.
            Ok(other) => self.database.lazy_fail(
                coll,
                &format!(
                    "`lazy_fetch` answered {other}; it must answer 1 (inserted) or 0 (absent)"
                ),
            ),
            Err(why) => self.database.lazy_fail(coll, &why),
        }
    }

    /**
    Iterate through a data structure from a given key to a given end-key.
    # Panics
    When called on a not implemented data-structure
    */
    #[allow(clippy::too_many_lines)]
    pub fn iterate(&mut self) {
        let on = self.code::<u8>();
        let arg = self.code::<u16>();
        let keys_size = self.code::<u8>();
        let mut keys = Vec::new();
        for _ in 0..keys_size {
            keys.push(Key {
                type_nr: self.code::<i8>(),
                position: self.code::<u16>(),
                start: self.code::<i32>(),
            });
        }
        let from_key = self.code::<u8>();
        let till_key = self.code::<u8>();
        let till = self.stack_key(till_key, &keys);
        let from = self.stack_key(from_key, &keys);
        let data = *self.get_stack::<DbRef>();
        // A holder with no record — a field reached through `nullref` — holds no
        // collection, and the cursor says so before any store is resolved: `finish ==
        // u32::MAX` is what `step` reads as "done", and the same test opens
        // `codegen_runtime::OpIterate`.  Resolving the store first indexed
        // `allocations[u16::MAX]`.
        if data.rec == 0 {
            self.put_stack((u64::from(u32::MAX) << 32) | u64::from(u32::MAX));
            return;
        }
        // Start the loop at the 'till' key and walk to the 'from' key
        let reverse = on & 64 != 0;
        // The 'till' key is exclusive the found key
        let ex = on & 128 == 0;
        let start;
        let finish;
        let all = &self.database.allocations;
        let trace_iter = std::env::var("LOFT_ITERATE_TRACE").is_ok();
        match on & 63 {
            1 => {
                // index points to the record position inside the store.  The cursor
                // derivation is shared with the native runtime (loft#691) — the two
                // had their own copies and drifted apart.
                let (s_cur, f_cur) =
                    tree::range_cursors(&data, arg, all, &keys, &from, &till, ex, reverse);
                start = s_cur;
                finish = f_cur;
                if trace_iter {
                    eprintln!(
                        "[iterate] on=index reverse={reverse} ex={ex} start={start} finish={finish} from_keys={} till_keys={}",
                        from.len(),
                        till.len()
                    );
                }
            }
            2 => {
                // sorted points to the position of the record inside the vector
                // empty from/till arrays signal "no constraint on this side".
                // Phase 2c: sorted collection pointer is a 4-byte u32 — using
                // `get_int` (8 bytes post-2c) overflows into the next field.
                // get_i32_raw returns i32::MIN for unresolved-type fields;
                // guard against negative values (0 = empty, i32::MIN = unresolved).
                let sorted_rec_raw = all[data.store_nr as usize].get_i32_raw(data.rec, data.pos);
                let sorted_rec = if sorted_rec_raw <= 0 {
                    0
                } else {
                    sorted_rec_raw as u32
                };
                let vec_len = if sorted_rec == 0 {
                    0
                } else {
                    all[data.store_nr as usize].get_u32_raw(sorted_rec, 4)
                };
                if reverse {
                    // loft#691 — `start` is the cursor `vector_step_rev` pre-decrements
                    // from, so it is one PAST the first element to visit (`>= vec_len` is
                    // its not-started sentinel), and `finish` is the LOWEST position to
                    // visit.  Both were off: the `+ !ex` made an inclusive upper bound
                    // begin one element too high, and searching `from` with `ex` asked
                    // the wrong question — the lower bound is always INCLUSIVE (`ex`
                    // describes the upper one), which is why the forward branch below
                    // hardcodes `true` for it.  The stepper then never compared against
                    // `finish` at all, so a reverse range ran to the first element.
                    start = if till.is_empty() {
                        // no upper bound → start past the last element
                        vec_len
                    } else {
                        vector::sorted_find(&data, ex, arg, all, &keys, &till).0
                    };
                    finish = if from.is_empty() {
                        // no lower bound → finish at 0 (visit all elements down to first)
                        0
                    } else {
                        vector::sorted_find(&data, true, arg, all, &keys, &from).0
                    };
                } else {
                    let s = if from.is_empty() {
                        0
                    } else {
                        vector::sorted_find(&data, true, arg, all, &keys, &from).0
                    };
                    start = if s == 0 { u32::MAX } else { s - 1 };
                    finish = if till.is_empty() {
                        vec_len
                    } else {
                        let (t, cmp) = vector::sorted_find(&data, ex, arg, all, &keys, &till);
                        if ex || cmp { t } else { t + 1 }
                    };
                }
                if trace_iter {
                    eprintln!(
                        "[iterate] on=sorted reverse={reverse} ex={ex} sorted_rec={sorted_rec} \
                         vec_len={vec_len} start={start} finish={finish}"
                    );
                }
            }
            3 | 4 => {
                // ordered points to the position inside the vector of references.
                // on=4 (fresh-scratch hash/radix, source store in the header) shares the
                // exact same cursor setup — only step()'s yield store differs, and its
                // from/till are always empty, so it takes the unbounded answer.
                //
                // The derivation is shared with the native runtime (loft#904), the way
                // `tree::range_cursors` is for an index: each backend had its own copy of
                // the bounded arms, in SLOT INDICES where the stepper walks BYTE OFFSETS,
                // and the two had already drifted apart — invisibly, because neither was
                // ever consumed.
                let (s_cur, f_cur) =
                    vector::ordered_range_cursors(&data, all, &keys, &from, &till, ex, reverse);
                start = s_cur;
                finish = f_cur;
                if trace_iter {
                    eprintln!(
                        "[iterate] on=ordered reverse={reverse} ex={ex} start={start} \
                         finish={finish} from_keys={} till_keys={}",
                        from.len(),
                        till.len()
                    );
                }
            }
            _ => panic!("Not implemented on {on}"),
        }
        // @PLAN53 cluster 2 / S4 (2b+2c): push the iterator state as ONE
        // contiguous 8-byte value, not two separate u32s.  The consumer stores
        // it into a single I64 var (`{id}#iter_state`, "cur<<32|finish") via an
        // i64 read.  Under LOFT_ALIGN each `put_stack::<u32>` advances a stepped
        // (8-byte) slot, so two pushes leave start@P and finish@P+8 with a 4-byte
        // gap — the i64 read-back then takes [finish | padding] and drops `start`,
        // killing the sorted cursor (2b) and shifting the hash cursor (2c).  A
        // single u64 push writes the 8 bytes contiguously in BOTH modes; the
        // little-endian layout (low=start, high=finish) is byte-identical to the
        // old two-u32 push when the step is identity (flag-OFF).
        self.put_stack((u64::from(finish) << 32) | u64::from(start));
    }

    pub(super) fn stack_key(&mut self, size: u8, keys: &[Key]) -> Vec<Content> {
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= size as usize {
                break;
            }
            match k.type_nr.abs() {
                1 => key.push(Content::Long(*self.get_stack::<i64>())),
                2 => key.push(Content::Long(*self.get_stack::<i64>())),
                3 => key.push(Content::Single(*self.get_stack::<f32>())),
                4 => key.push(Content::Float(*self.get_stack::<f64>())),
                5 => key.push(Content::Long(i64::from(*self.get_stack::<bool>()))),
                6 => key.push(Content::Str(self.string())),
                7 => key.push(Content::Long(i64::from(*self.get_stack::<u8>()))),
                // The five narrow STORAGE widths (`Parts::Int` / `Short` / `Byte` /
                // `ShortRaw` / `IntRaw`).  Narrowing happens at the field boundary, so the bound a
                // caller pushes for a ranged scan is an ordinary 8-byte integer whatever
                // the field's width — the same rule `generation/text.rs::emit_content`
                // states for the native side, where every integer width answers a
                // `Content::Long(i64)` and the WIDTH lives in the stored record.
                //
                // Without these arms the interpreter panicked "Unknown key type" on any
                // iteration of a collection keyed on `u8` / `i16` / `i32` / a limited
                // integer — including a bare `for r in coll`, which pushes bounds too, so
                // the collection could be built and counted but never walked (loft#812).
                8..=12 => key.push(Content::Long(*self.get_stack::<i64>())),
                _ => panic!("Unknown key type"),
            }
        }
        key
    }

    /**
    Step to the next value for the iterator.
    # Panics
    When requested on a not-implemented iterator.
    */
    pub fn step(&mut self) {
        let state_var = self.code::<u16>();
        let on = self.code::<u8>();
        let arg = self.code::<u16>();
        let cur = *self.get_var::<u32>(state_var);
        let finish = *self.get_var::<u32>(state_var - 4);
        let reverse = on & 64 != 0;
        let data = *self.get_stack::<DbRef>();
        // The holder's record is tested BEFORE its store is resolved: a holder reached
        // through `nullref` has no store to resolve (`codegen_runtime::OpStep` asks in the
        // same order).
        let cur = if data.rec == 0 || finish == u32::MAX {
            new_ref(&data, 0, 0)
        } else {
            let store = crate::keys::store(&data, &self.database.allocations);
            match on & 63 {
                1 => {
                    let rec = new_ref(&data, cur, arg);
                    let n = if cur == 0 {
                        if reverse {
                            tree::last(&data, arg, &self.database.allocations).rec
                        } else {
                            tree::first(&data, arg, &self.database.allocations).rec
                        }
                    } else if reverse {
                        tree::previous(store, &rec)
                    } else {
                        tree::next(store, &rec)
                    };
                    self.put_var(state_var - 8, n);
                    if std::env::var("LOFT_ITERATE_TRACE").is_ok() {
                        eprintln!(
                            "[step] on=index reverse={reverse} cur={cur} -> n={n} finish={finish} done={}",
                            n == finish
                        );
                    }
                    if n == finish {
                        self.put_var(state_var - 12, u32::MAX);
                    }
                    new_ref(&data, n, 8)
                }
                2 => {
                    let mut pos = if cur == u32::MAX {
                        i32::MAX
                    } else {
                        cur as i32
                    };
                    if reverse {
                        // `iterate()` sets start > length for the "not started" sentinel
                        // (pos >= length is treated as past-the-end in vector_step_rev).
                        vector::vector_step_rev(&data, &mut pos, &self.database.allocations);
                        self.put_var(state_var - 8, pos as u32);
                        // loft#691 — stop once the cursor drops below the lowest in-range
                        // position, the mirror of the forward `pos >= finish` below.
                        // Without it a reverse range ignored its lower bound entirely and
                        // ran on to the first element.  `finish == 0` (no lower bound)
                        // never trips, so the open forms walk to the end as before.
                        if pos != i32::MAX && (pos as u32) < finish {
                            pos = i32::MAX;
                        }
                        if pos == i32::MAX {
                            self.put_var(state_var - 12, u32::MAX);
                        }
                    } else {
                        vector::vector_step(&data, &mut pos, &self.database.allocations);
                        self.put_var(state_var - 8, pos as u32);
                        if pos as u32 >= finish {
                            pos = i32::MAX;
                            self.put_var(state_var - 12, u32::MAX);
                        }
                    }
                    self.database.element_reference(
                        &data,
                        if pos == i32::MAX {
                            i32::MAX
                        } else {
                            8 + pos * i32::from(arg)
                        },
                    )
                }
                3 | 4 => {
                    // on=4 (fresh-scratch hash/radix) yields in the source store recorded
                    // in the scratch header; on=3 (co-located) yields in data.store_nr.
                    // A read-only source's dedicated scratch store is freed by the loop
                    // epilogue's OpFreeScratch, not here (covers break too).
                    let (elem, new_pos) = vector::step_ordered(
                        &data,
                        cur,
                        &self.database.allocations,
                        on & 63 == 4,
                        reverse,
                    );
                    self.put_var(state_var - 8, new_pos);
                    // `finish` bounds the range in the same BYTE-OFFSET unit the cursor
                    // walks in, and 0 means no bound (it is below every valid position,
                    // so it never trips) — which is what an unbounded walk and every
                    // `on = 4` scratch return.  Nothing consulted it here at all, so a
                    // bounded range ran past its end (loft#904).
                    let past = finish != 0
                        && new_pos != i32::MAX as u32
                        && if reverse {
                            new_pos < finish
                        } else {
                            new_pos >= finish
                        };
                    if past {
                        self.put_var(state_var - 12, u32::MAX);
                        new_ref(&data, 0, 0)
                    } else {
                        elem
                    }
                }
                _ => panic!("Not implemented"),
            }
        };
        self.put_stack(cur);
    }

    /**
    Remove the current value from the iterator. Move the iterator to the previous value.
    # Panics
    When requested on a not-implemented iterator.
    */
    /// `e#remove` — @FR-Col-Remove's in-loop spelling, and the one that also has to say
    /// where the cursor lands: the arms below dispatch on the container's layout, and each
    /// leaves the walk on the element that takes the removed one's place, so a plain
    /// `for e in v { e#remove; }` visits every element exactly once.
    pub fn remove(&mut self) {
        let state_var = self.code::<u16>();
        let on = self.code::<u8>();
        let tp = self.code::<u16>();
        let reverse = on & 64 != 0;
        let cur = *self.get_var::<i32>(state_var);
        let data = *self.get_stack::<DbRef>();
        // Defense-in-depth: coroutine DbRefs (store_nr == u16::MAX) must not reach remove().
        // The compiler already rejects e#remove on generator iterators (CO1.5c / S24), so this
        // guard only fires if that check is somehow bypassed — preventing release-build corruption.
        assert!(
            data.store_nr != u16::MAX,
            "e#remove on coroutine DbRef — compiler check should have rejected this"
        );
        if data.store_nr == u16::MAX {
            return;
        }
        match on & 63 {
            0 => {
                // vector / array — `tp` is the ELEMENT type, and
                // `remove_vector_at` reads the layout off it.
                //
                // Where the cursor lands next is decided by the DIRECTION, because
                // the shift only touches the elements ABOVE the one removed:
                // forward, the successor moves into slot `cur`, so the cursor must
                // step back to `cur - 1` to visit it; backwards, the next element
                // is `cur - 1` and it has not moved, so the cursor must STAY at
                // `cur` — the step subtracts one either way.  Rewinding forwards
                // (`cur + 1`) revisits the element that shifted down.
                let n = if reverse { cur } else { cur - 1 };
                self.database.remove_vector_at(&data, tp, i64::from(cur));
                // iter_var (#index) is i64 on stack post-2c.  Sign-extend n
                // (may be −1 after remove-at-index-0) and write all 8 bytes
                // so the high word doesn't leak a stale value into the next
                // VarInt — otherwise −1 comes back as 0xFFFFFFFF = 2^32−1.
                // put_var adds size_of::<T>(); switching i32→i64 shifts the
                // target address up by 4 bytes, so pos moves from −8 to −4.
                // @PLAN53 cluster 2 / 2f: this is the lone non-self-compensating
                // delta in remove() — an i64 put (step(8)=8 both modes) after the
                // DbRef pop (step(12): 12 V1, 16 V2).  The post-pop 4-byte puts
                // self-compensate (step(12)−step(4) cancels); this i64 one does not,
                // so the raw −4 lands 4 bytes low under V2 and stomps the loop's
                // vector-variable slot → next iteration reads a garbage DbRef
                // (store_nr=65535) → panic in keys.rs/get_vector.  Thread the
                // step(12) term: −(step(12)−8) = −4 off (identity), −8 aligned.
                self.put_var(state_var - (self.stack_step(12) - 8) as u16, i64::from(n));
            }
            1 => {
                // Use the outer `cur` (read as i32 before the data DbRef was popped).
                // Re-reading `state_var` here would give the wrong slot because `get_stack`
                // already consumed the 12-byte DbRef, shifting the effective slot offset.
                let cur = cur as u32;
                if cur == u32::MAX {
                    return;
                }
                // @PLAN53 cluster 2 / 2f: get_var adds NO step, so this finish-read delta
                // must thread step(12) the other way: −(step(12)+4) = −16 off (identity),
                // −20 aligned.  The same disease as the case-0 fix above.
                let finish = *self.get_var::<u32>(state_var - (self.stack_step(12) + 4) as u16);
                // The removal and the cursor it leaves behind are one decision, shared with
                // the native runtime (loft#1272) — `tp` is the Index TYPE, which the shared
                // side turns into the fields offset tree navigation needs.
                match self
                    .database
                    .remove_during_tree_iteration(&data, tp, cur, finish, reverse)
                {
                    // Nothing follows in range; signal end-of-iteration by overwriting the
                    // finish slot (same as step's put_var(state_var-12)).
                    None => self.put_var(state_var - 12, u32::MAX),
                    // Park at the predecessor so the next step() computes next(pred) and
                    // visits it.
                    Some(pred) => self.put_var(state_var - 8, pred),
                }
            }
            2 => {
                // sorted: inline elements, `tp` is the element type; `cur` is an index
                if cur < 0 {
                    return;
                }
                // Same rewind rule as arm 0 — see there.
                let n = if reverse { cur } else { cur - 1 };
                self.database.remove_vector_at(&data, tp, i64::from(cur));
                self.put_var(state_var - 8, n);
            }
            3 => {
                // ordered: 4-byte record-id slots, `tp` is the element type; `cur` is
                // a BYTE offset into the slot vector (8, 12, ...), so the slot index
                // it names is `(cur - 8) / 4`.
                if cur < 8 {
                    return;
                }
                // Same rewind rule as arm 0, one slot wide instead of one index.
                let n = if reverse { cur } else { cur - 4 };
                self.database
                    .remove_vector_at(&data, tp, i64::from((cur - 8) / 4));
                self.put_var(state_var - 8, n);
            }
            _ => panic!("Not implemented on {on}"),
        }
    }

    /**
    Clear the given structure on the field
    */
    pub fn clear(&mut self) {
        let tp = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        self.database.remove_claims(&data, tp);
    }

    pub fn append_copy(&mut self) {
        let tp = self.code::<u16>();
        let count = *self.get_stack::<i64>();
        // A count is a TOTAL, and a negative total is not a shorter vector — it is not a
        // vector at all, so it clamps to zero and yields no elements. A bare `as u32`
        // instead turns `[7; -1]` into 4 294 967 295 `copy_block`s that walk off the store
        // until glibc aborts the process.
        // Twin of `OpAppendCopy` (`src/codegen_runtime.rs`) — keep the two in step, or
        // they disagree on a heap-corrupting input.
        let multiply = count.clamp(0, i64::from(u32::MAX)) as u32;
        let data = *self.get_stack::<DbRef>();
        let ctp = self.database.content(tp);
        let size = u32::from(self.database.size(ctp));
        let length = vector::length_vector(&data, &self.database.allocations);
        // `[x; n]` asks for n elements TOTAL and the template is already appended, so
        // `n - 1` more are needed. Adding `n` instead grows the vector one too far and
        // leaves the last slot never written — `[7; 3]` then reads back as length 4 with
        // garbage in it.
        //
        // `n == 0` is the same statement taken to its end: the template must go too, and
        // the early return keeps `0..(multiply - 1)` from running on a `u32`, where zero
        // wraps to 4 294 967 295 `copy_block`s that walk off the store and abort in
        // glibc's allocator.
        if multiply == 0 {
            let store = crate::keys::mut_store(&data, &mut self.database.allocations);
            let vec_rec = store.get_u32_raw(data.rec, data.pos);
            let len_now = store.get_u32_raw(vec_rec, 4);
            store.set_u32_raw(vec_rec, 4, len_now.saturating_sub(1));
            return;
        }
        let extra = multiply - 1;
        if extra == 0 {
            return; // the template alone already IS the answer
        }
        vector::vector_append(&data, size, &mut self.database.allocations);
        self.database.vector_set_size(&data, extra, size);
        // Re-read the backing record AFTER the resize: growing the vector can move it,
        // and both ends of the copy live inside it. Reading it once up front left the
        // template `from` — and every destination — pointing at the record's old home.
        let v_rec =
            crate::keys::store(&data, &self.database.allocations).get_u32_raw(data.rec, data.pos);
        let from = DbRef {
            store_nr: data.store_nr,
            rec: v_rec,
            pos: 8 + (length * size - size),
        };
        for i in 0..extra {
            let to = DbRef {
                store_nr: data.store_nr,
                rec: v_rec,
                pos: 8 + (length + i) * size,
            };
            self.database.copy_block(&from, &to, size);
            // The claim source is the TEMPLATE element, not the vector handle. `data`
            // points at the vector's 4-byte record id, so a `text` element re-interned
            // whatever those bytes decoded to: `["abc"; 4]` gave "abc" in element 0 and
            // junk in every copy. Same word wrong in the `--native` twin.
            self.database.copy_claims(&from, &to, ctp);
            self.database
                .watch_oob_text(&to, ctp, Some(&data), "append_copy");
        }
    }

    pub fn copy_record(&mut self) {
        // @PLN118 arc B — mark the deep-copy so the gen-detector classifies a stale read here
        // (a copy reading a stale sub-ref) apart from a plain deref (a premature free). The
        // source-record pops below are exactly where a stale `Vec3` sub-field surfaces.
        let mark = crate::keys::uaf_gen_enabled();
        if mark {
            crate::keys::uaf_copy_enter();
        }
        let raw_tp = self.code::<u16>();
        let to = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        self.do_copy_record(data, to, raw_tp);
        if mark {
            crate::keys::uaf_copy_exit();
        }
    }

    /// Core of `OpCopyRecord` / `OpCopyRefOrNull`: deep-copy record `data` into
    /// the already-allocated destination `to`.  `raw_tp`'s high bit (`0x8000`)
    /// frees the source store after the copy (#120).  Factored so the nullable
    /// struct-return ABI-B path (`copy_ref_or_null`) reuses the non-null branch.
    fn do_copy_record(&mut self, data: DbRef, to: DbRef, raw_tp: u16) {
        // Issue #120: high bit of tp signals "free source store after copy".
        let free_source = raw_tp & 0x8000 != 0;
        let tp = raw_tp & 0x7FFF;
        // @PLAN51 Cluster II — true alias copy is a no-op.  When data
        // and to refer to the SAME slot (full DbRef equality), the
        // remove_claims + copy_block + copy_claims sequence would
        // destroy and rebuild the same nested vectors / texts (because
        // remove_claims's prelude frees nested heap records BEFORE
        // copy_block can read them — corrupting same-store callers
        // like extended-S1 inner Sets).  Short-circuit before any
        // destructive op so callers that pass aliased src/dst (e.g.
        // ref_return-promoted callees whose return aliases their
        // hidden buffer arg) get safe semantics.  Mirrors the safety
        // gate the native runtime needs at codegen_runtime.rs:OpCopyRecord.
        if data == to {
            // free_source is suppressed (data.store_nr == to.store_nr
            // is already the existing skip-free condition at line 1225);
            // nothing more to do.
            let _ = free_source;
            return;
        }
        // @PLN25 — a NULL source (the `store_nr == u16::MAX` sentinel) has no
        // store to read; copying from it is meaningless.  `store(&data)` below
        // would index `allocations[65535]` and panic (`allocation.rs:560` OOB).
        // This guards the pre-existing `main` crash on `vec[i] = <runtime-null>`
        // into a non-nullable inline element (where there is no null encoding) —
        // leave the destination unchanged rather than crash.  (For a NULLABLE
        // `__nullable<S>` element the parser emits the discriminant-0 store, not
        // OpCopyRecord, so this path is the can't-represent-null fallback.)
        if data.store_nr == u16::MAX {
            return;
        }
        // @FR-L-Null, the DESTINATION half of the same question (loft#1374's boundary).  A
        // read naming no record answers `nullref`, so an out-of-range element write
        // (`w.es[9] = e` on a one-element vector) hands this a null PLACE.  There is nothing
        // to write into, so the write is dropped — which is what the language already says
        // such a write does.  Without it `store_mut` indexes `allocations[65535]` and the
        // process dies on a program the compiler accepted.  The source half above and this
        // one are one rule read from its two ends.
        if to.store_nr == u16::MAX {
            return;
        }
        // @PLN90 phase 1 — make the copy visible. A real record deep-copy is about to
        // run (the no-op-alias and null cases returned above). LOFT_COPY_DUMP prints one
        // line per executed structure copy so we can inventory every copy + map it to its
        // source line (the runtime ground truth the compile-time decision must cover).
        if crate::keys::copy_dump_enabled() {
            let line = self
                .line_numbers
                .range(..=self.code_pos)
                .next_back()
                .map_or(0, |(_, &v)| v);
            eprintln!("[copy] record       line={line}  tp={tp}");
        }
        // cluster-462 tool-gap #1 — name the premature-free op: when the copy
        // SOURCE is read while its store is still freed (the operand-stack stale
        // ref the LOFT_UAF frame scan misses), report the source, the line that
        // freed it, and the copy site.  Gated on LOFT_UAF; capped to avoid flood.
        if crate::keys::uaf_any_enabled()
            && (data.store_nr as usize) < self.database.allocations.len()
            && self.database.allocations[data.store_nr as usize].free
        {
            thread_local! {
                static REPORTED: std::cell::RefCell<std::collections::HashSet<u16>> =
                    std::cell::RefCell::new(std::collections::HashSet::new());
            }
            let first = REPORTED.with(|s| s.borrow_mut().insert(data.store_nr));
            if first {
                let line_of = |ln: &std::collections::BTreeMap<u32, u32>, pc: u32| -> u32 {
                    ln.range(..=pc).next_back().map_or(0, |(_, &v)| v)
                };
                let copy_line = line_of(&self.line_numbers, self.code_pos);
                let copy_d_nr = self.call_stack.last().map_or(u32::MAX, |f| f.d_nr);
                let (free_pc, free_line, free_d_nr, free_op) =
                    crate::keys::uaf_freed_pc(data.store_nr)
                        .map_or((0, 0, u32::MAX, u16::MAX), |(pc, d, op)| {
                            (pc, line_of(&self.line_numbers, pc), d, op)
                        });
                eprintln!(
                    "[uaf-src] OpCopyRecord reads FREED source store #{} (rec={},pos={},tp={tp}) \
                     at copy-site pc={} (line {copy_line}, fn d_nr={copy_d_nr}); that store was \
                     last freed at pc={free_pc} (line {free_line}, fn d_nr={free_d_nr}, op={free_op})",
                    data.store_nr, data.rec, data.pos, self.code_pos,
                );
            }
        }
        // (b) cluster-462 LOFT_UAF_REUSE — the REUSED case the free-check above misses:
        // the source slot is LIVE (free=false) but `validate_claims` finds it structurally
        // invalid for `tp` (out-of-range rec/pos, broken interior edges), i.e. it was
        // freed-then-reused as a DIFFERENT record since the source ref was minted. That is
        // the stale-reused read behind the post-reuse SIGSEGV (#462 @ sim.loft:3546) —
        // named here, before the faulting copy. `validate_claims` is bounds-checked (it
        // reports, never faults). Deduped by store slot.
        if crate::keys::uaf_reuse_enabled()
            && data.rec != 0
            && (data.store_nr as usize) < self.database.allocations.len()
            && !self.database.allocations[data.store_nr as usize].free
        {
            let mut problems = 0u32;
            self.database
                .validate_claims(&data, tp, "uaf-reuse-src", &mut problems);
            if problems > 0 {
                thread_local! {
                    static REPORTED_REUSE: std::cell::RefCell<std::collections::HashSet<u16>> =
                        std::cell::RefCell::new(std::collections::HashSet::new());
                }
                let first = REPORTED_REUSE.with(|s| s.borrow_mut().insert(data.store_nr));
                if first {
                    let line = self
                        .line_numbers
                        .range(..=self.code_pos)
                        .next_back()
                        .map_or(0, |(_, &v)| v);
                    let cur_kt = self.database.allocations[data.store_nr as usize].known_type;
                    eprintln!(
                        "[uaf-reuse] OpCopyRecord source store #{} (rec={},pos={}) is LIVE but \
                         STRUCTURALLY INVALID for tp={tp} ({problems} broken edge(s); slot now \
                         holds known_type={cur_kt}) — freed-then-reused-as-different, a stale read \
                         at copy-site pc={} (line {line})",
                        data.store_nr, data.rec, data.pos, self.code_pos,
                    );
                }
            }
        }
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            let src_w = if data.rec != 0 {
                self.database.store(&data).get_int(data.rec, data.pos)
            } else {
                -1
            };
            let src_data_ptr = if data.rec != 0 {
                self.database
                    .store(&data)
                    .get_u32_raw(data.rec, data.pos + 8)
            } else {
                0
            };
            let src_vec0 = if src_data_ptr != 0 {
                let len = self.database.store(&data).get_u32_raw(src_data_ptr, 4);
                if len > 0 {
                    self.database.store(&data).get_int(src_data_ptr, 8)
                } else {
                    -3
                }
            } else {
                -4
            };
            let dst_w_before = if to.rec != 0 {
                self.database.store(&to).get_int(to.rec, to.pos)
            } else {
                -1
            };
            eprintln!(
                "[cr] OpCopyRecord src=#{}@{},{} (w={src_w} data_ptr={src_data_ptr} vec0={src_vec0}) dst=#{}@{},{} (w_before={dst_w_before}) tp={tp} free_src={free_source}",
                data.store_nr, data.rec, data.pos, to.store_nr, to.rec, to.pos,
            );
        }
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            // #306 — bounds-checked pre-walk of both records' claim graphs;
            // names the first broken interior edge instead of faulting on it.
            let mut problems = 0u32;
            self.database
                .validate_claims(&data, tp, "src", &mut problems);
            self.database.validate_claims(&to, tp, "dst", &mut problems);
            if problems > 0 {
                eprintln!(
                    "[cr-check] {problems} broken edge(s) found before copy \
                     src=#{}.{},{} dst=#{}.{},{} tp={tp}",
                    data.store_nr, data.rec, data.pos, to.store_nr, to.rec, to.pos,
                );
            }
        }
        let code_pos = self.code_pos;
        let size = u32::from(self.database.size(tp));
        // free any nested vectors/strings already owned by the destination
        // before overwriting it, to prevent double-free and leaks when a struct
        // field containing a nested vector is reassigned.
        self.database.remove_claims(&to, tp);
        self.database.copy_block(&data, &to, size);
        self.database.copy_claims(&data, &to, tp);
        // LOFT_WATCH_STORE (cluster-462 write-watch) — name the copy that leaves a garbage
        // text-ptr in the watched store; src_oob says propagated-vs-introduced-here.
        self.database
            .watch_oob_text(&to, tp, Some(&data), "copy_record");
        if std::env::var("LOFT_TRACE_CR").is_ok() {
            let dst_w = self.database.store(&to).get_int(to.rec, to.pos);
            let dst_data_ptr = self.database.store(&to).get_u32_raw(to.rec, to.pos + 8);
            eprintln!(
                "[cr]   AFTER copy: dst=#{}@{},{} w={dst_w} data_ptr={dst_data_ptr}",
                to.store_nr, to.rec, to.pos,
            );
            if dst_data_ptr != 0 {
                let len = self.database.store(&to).get_u32_raw(dst_data_ptr, 4);
                let first = if len > 0 {
                    self.database.store(&to).get_int(dst_data_ptr, 8)
                } else {
                    -2
                };
                eprintln!("[cr]   AFTER copy: dst.vec_rec={dst_data_ptr} len={len} [0]={first}");
            }
        }
        // @P317 — LOFT_LOG=copy_check: warn if the deep copy changed any nested
        // collection length (before the source-free below, so src is intact).
        // Call-site-gated so production pays only a branch-predicted cached
        // read, not a function call, when the mode is off.
        if self.database.copy_check_enabled() {
            self.database
                .report_copy_mismatches(&data, &to, tp, "copy_record");
        }
        // Record which bytecode position performed this deep copy.
        self.database.allocations[to.store_nr as usize].last_op_at = code_pos;
        // Issue #120: free the source store after deep copy when the caller
        // knows the source is a temporary (callee's return store) that would
        // otherwise leak because is_ret_work_ref suppresses its OpFreeRef.
        if free_source
            && data.store_nr != to.store_nr
            && !self.database.is_stack_store(data.store_nr)
            && !self.database.allocations[data.store_nr as usize].free
            && !self.database.allocations[data.store_nr as usize].read_only
            && !self.database.allocations[data.store_nr as usize].is_free_protected()
        {
            self.database.free(&data);
        }
    }

    /// `OpCopyRefOrNull` — the ABI-B caller path for `v = call()` where the
    /// callee has a heap parameter and returns a struct-OR-null.  Pops the call
    /// result `src`; the destination variable `pos` already holds a freshly
    /// allocated record (emitted `OpDatabase` just before).  When `src` is the
    /// null sentinel (`store_nr == u16::MAX`), a plain `OpCopyRecord` would index
    /// `allocations[u16::MAX]` and panic — and even guarded it would leave the
    /// pre-allocated record bound, so `v == null` (which tests `rec == 0`) would
    /// read false.  So: free the pre-allocated record and bind the sentinel into
    /// the slot, making `v` genuinely null.  Otherwise deep-copy as usual.
    pub fn copy_ref_or_null(&mut self) {
        let pos = self.code::<u16>();
        let raw_tp = self.code::<u16>();
        let src = *self.get_stack::<DbRef>();
        // `rec == 0` is the absence test, not `store_nr == u16::MAX` — the doc above already
        // says so ("`v == null` … tests `rec == 0`") while the guard asked the narrower
        // question.  A read that names no record answers `nullref` (`DbRef::or_null`,
        // @FR-L-Null), whose `rec` is 0 as well, so this test is total over every source a
        // call can hand back; reading the `store_nr` alone left `k = v[oob]` deep-copying
        // from a record that was not there, so `k` came back holding the pre-allocated
        // record's uninitialised bytes instead of null (loft#823).
        if src.rec == 0 {
            // Null return: reclaim the pre-allocated destination record and bind
            // the null sentinel so `v == null` holds.
            //
            // The sentinel is `DbRef::NULL` — the value `OpNullRefSentinel` pushes and
            // `OpRefIsNull` tests for (`store_nr == u16::MAX`).  `Stores::null()` is not
            // it: that ALLOCATES a store and hands back a ref into it, so the slot read as
            // present to `== null` while rendering as null, and one store leaked per null
            // result (loft#1346, the interpreter's nullable-record reassign copy).
            let dst = *self.get_var::<DbRef>(pos);
            if dst.store_nr != u16::MAX {
                self.database.free(&dst);
            }
            *self.mut_var::<DbRef>(pos) = DbRef::NULL;
            return;
        }
        let dst = *self.get_var::<DbRef>(pos);
        self.do_copy_record(src, dst, raw_tp);
    }

    /// @PLN85 `OpBindOrCopy` — bind a `Join` call return into the owned slot `pos`
    /// with the runtime arg-aliasing GUARD the `ownership_of` oracle witnesses.
    /// Stack (top first): `witness` (the argument the return may borrow, = the
    /// oracle's resolved `base`), then `src` (the call result).
    ///
    /// - `src` aliases `witness`'s store → it is the BORROW arm of the `??` join:
    ///   deep-copy into a fresh store at `pos` so the append's source-free hits the
    ///   copy, not the borrowed element;
    /// - else `src` is fresh (the OWNED arm) → adopt it directly, so the source-free
    ///   correctly frees this owned store.
    ///
    /// A static gate cannot choose — the same call returns owned on one branch,
    /// borrowed on the other; the store-identity check resolves it per execution.
    pub fn bind_or_copy(&mut self) {
        let pos = self.code::<u16>();
        let raw_tp = self.code::<u16>();
        let code_pos = self.code_pos;
        let witness = *self.get_stack::<DbRef>();
        let src = *self.get_stack::<DbRef>();
        if src.store_nr != u16::MAX && src.store_nr == witness.store_nr {
            // BORROW arm — materialise: `raw_tp`'s `0x8000` source-free bit applies to
            // the COPY, not the fresh allocation, so mask it for `alloc_record_at`.
            self.alloc_record_at(pos, raw_tp & 0x7fff, code_pos);
            let new_dst = *self.get_var::<DbRef>(pos);
            self.do_copy_record(src, new_dst, raw_tp);
        } else {
            // OWNED arm (or null) — adopt the store directly.
            *self.mut_var::<DbRef>(pos) = src;
        }
    }

    /// @P295 — deep-copy a keyed collection (`sorted`/`hash`/`index`) into a
    /// keyed-collection LOCAL `dest`, replacing its prior contents.  Unlike
    /// `copy_record` there is NO `copy_block`: a keyed local's slot is a
    /// `DbRef` to a dedicated store whose collection header lives at
    /// `(store, 1, 8)`, so `remove_claims(dest)` (free + reset) followed by
    /// `copy_claims(src, dest, tp)` (per-kind rebuild of the bucket/tree
    /// index) is the complete, correct copy.  `tp` is the SPECIFIC keyed
    /// type id; its `0x8000` high bit frees the source store after the copy
    /// (set by the parser for fresh-storage RHS like `s = build()`).
    pub fn replace_keyed(&mut self) {
        let raw_tp = self.code::<u16>();
        let free_source = raw_tp & 0x8000 != 0;
        let tp = raw_tp & 0x7FFF;
        let dest = *self.get_stack::<DbRef>();
        let src = *self.get_stack::<DbRef>();
        // loft#1150 — an ABSENT source copies NOTHING.  A `-> hash<T[k]>?` that answers
        // `null` reaches here as the `u16::MAX` sentinel, and `copy_claims` dereferences the
        // source store: `allocations[65535]` panicked on both backends.  The destination is
        // left exactly as it was, which is what makes this the whole fix — a nullable keyed
        // local's `Set(v, Null)` lowers to the sentinel, so skipping the copy leaves it
        // absent, and a NON-null destination keeps the empty store that same `Set` allocated
        // for it.  The FREE leg below has carried this guard all along, for the same reason
        // and in the same words; only the COPY was missing it.
        if src.store_nr == u16::MAX {
            self.database.remove_claims(&dest, tp);
            self.database.mark_collection_absent(&dest);
            return;
        }
        self.database.remove_claims(&dest, tp);
        self.database.copy_claims(&src, &dest, tp);
        if self.database.copy_check_enabled() {
            self.database
                .report_copy_mismatches(&src, &dest, tp, "replace_keyed");
        }
        if free_source
            && src.store_nr != dest.store_nr
            // Sentinel guard: the `allocations[..]` prechecks below index by store_nr.
            && src.store_nr != u16::MAX
            && !self.database.is_stack_store(src.store_nr)
            && !self.database.allocations[src.store_nr as usize].free
            && !self.database.allocations[src.store_nr as usize].read_only
            && !self.database.allocations[src.store_nr as usize].is_free_protected()
        {
            self.database.free(&src);
        }
    }

    /// @P305 — `coll[key] = value` insert-or-replace into a keyed
    /// collection.  `OpSetKeyed(coll, value, tp)`: `tp`'s `0x8000` bit frees
    /// `value`'s store after the deep copy (caller temp).
    pub fn set_keyed(&mut self) {
        let raw_tp = self.code::<u16>();
        let free_source = raw_tp & 0x8000 != 0;
        let db_tp = raw_tp & 0x7FFF;
        let value = *self.get_stack::<DbRef>();
        let coll = *self.get_stack::<DbRef>();
        self.database.set_keyed(&coll, &value, db_tp, free_source);
    }

    pub fn hash_add(&mut self) {
        let tp = self.code::<u16>();
        let rec = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        hash::add(
            &data,
            &rec,
            &mut self.database.allocations,
            &self.database.types[tp as usize].keys,
        );
    }

    pub fn validate(&mut self) {
        let tp = self.code::<u16>();
        let data = *self.get_stack::<DbRef>();
        self.database.validate(&data, tp);
    }

    pub fn hash_find(&mut self) {
        let data = *self.get_stack::<DbRef>();
        let (db_tp, key) = self.read_key(true);
        let res = hash::find(
            &data,
            &self.database.allocations,
            &self.database.types[db_tp as usize].keys,
            &key,
        );
        self.put_stack(res);
    }

    /// Remove one record from a keyed collection.
    ///
    /// `tp`'s [`CLEAR_KEYED_VIEW`] bit says UNLINK ONLY — the same convention
    /// `OpClearKeyed` and `OpSetKeyed` use on theirs. It marks the members of a
    /// linked collection group that the removal passes through on its way to the
    /// one that frees (loft#900): every member must let the record go, and the
    /// record must be freed once, after the last unlink has read its key out of it.
    pub fn hash_remove(&mut self) {
        let raw = self.code::<u16>();
        let tp = raw & !crate::database::CLEAR_KEYED_VIEW;
        let rec = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        if rec.rec != 0 {
            if raw & crate::database::CLEAR_KEYED_VIEW == 0 {
                self.database.remove_owned(&data, &rec, tp);
            } else {
                self.database.remove(&data, &rec, tp);
            }
        }
    }

    pub(super) fn read_key(&mut self, full: bool) -> (u16, Vec<Content>) {
        let db_tp = self.code::<u16>();
        let keys = self.database.get_keys(db_tp);
        let no_keys = if full {
            keys.len() as u8
        } else {
            self.code::<u8>()
        };
        let mut key = Vec::new();
        for (k_nr, k) in keys.iter().enumerate() {
            if k_nr >= no_keys as usize {
                break;
            }
            match k {
                0 | 1 => key.push(Content::Long(*self.get_stack::<i64>())),
                2 => key.push(Content::Single(*self.get_stack::<f32>())),
                3 => key.push(Content::Float(*self.get_stack::<f64>())),
                4 => key.push(Content::Long(i64::from(*self.get_stack::<bool>()))),
                5 => key.push(Content::Str(self.string())),
                6 => key.push(Content::Long(i64::from(*self.get_stack::<u32>()))),
                _ => {
                    // Narrow integer storage (Parts::Int / Short / ShortRaw /
                    // Byte) — the lookup value is still an i64 on the stack
                    // even when the field's storage is narrower.  Pop 8
                    // bytes so subsequent stack reads stay aligned; the
                    // catch-all 1-byte pop here was broken silently for
                    // every narrow-key hash since Parts::Int was introduced.
                    match &self.database.types[*k as usize].parts {
                        crate::database::Parts::Int(_, _)
                        | crate::database::Parts::IntRaw(_, _)
                        | crate::database::Parts::Short(_, _)
                        | crate::database::Parts::ShortRaw(_, _)
                        | crate::database::Parts::Byte(_, _) => {
                            key.push(Content::Long(*self.get_stack::<i64>()));
                        }
                        _ => key.push(Content::Long(i64::from(*self.get_stack::<u8>()))),
                    }
                }
            }
            // We assume that all none-base types are enumerate types.
        }
        (db_tp, key)
    }

    pub fn finish_record(&mut self) {
        // @PLN118 arc B — same copy-context mark as `copy_record` (finish deep-copies the
        // source record into a parent field; a stale source pop here is a copy-incomplete).
        let mark = crate::keys::uaf_gen_enabled();
        if mark {
            crate::keys::uaf_copy_enter();
        }
        let parent_tp = self.code::<u16>();
        let fld = self.code::<u16>();
        let record = *self.get_stack::<DbRef>();
        let data = *self.get_stack::<DbRef>();
        self.database.record_finish(&data, &record, parent_tp, fld);
        if mark {
            crate::keys::uaf_copy_exit();
        }
    }

    pub fn db_from_text(&mut self, val: &str, db_tp: u16) -> DbRef {
        // @P372: size the target record to the struct, not a fixed 8 words.
        // The struct is written at `pos: 8` (after the 8-byte record header),
        // so it needs `8 + size` bytes = `1 + size.div_ceil(8)` words.  The old
        // fixed `database(8)` (64 bytes) left only 56 bytes for the struct, so
        // any struct larger than 56 bytes (e.g. 8 `integer` fields = 64 bytes)
        // overflowed: the tail field's write — notably a vector field's handle
        // at byte 64 — landed on the ADJACENT block's header, zeroing it.  A
        // later `claim` then walked the free list into that zero-sized block and
        // `claim_scan` spun forever (`pos += abs(claim)` never advances; the
        // `debug_assert_ne!` guard is compiled out in release).  A vector field
        // made it deterministic because appending its elements triggers exactly
        // such a `claim`.  Keep a floor of 8 words to preserve the prior slack
        // for small structs.
        let words = std::cmp::max(8, 1 + u32::from(self.database.size(db_tp)).div_ceil(8));
        let db = self.database.database(words);
        let into = DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: 8,
        };
        self.database.set_final_default_value(db_tp, &into);
        self.database.last_parse_errors.clear();
        if let Some(err) = self.database.parse(val, db_tp, &into) {
            self.database.last_parse_errors.push(err);
        }
        into
    }

    pub fn insert_vector(&mut self) {
        let size = self.code::<u16>();
        let db_tp = self.code::<u16>();
        let index = *self.get_stack::<i64>();
        let r = *self.get_stack::<DbRef>();
        let new_value =
            vector::insert_vector(&r, u32::from(size), index, &mut self.database.allocations);
        self.database.set_default_value(db_tp, &new_value);
        self.put_stack(new_value);
    }
}
