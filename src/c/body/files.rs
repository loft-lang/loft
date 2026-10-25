// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! Files as C: a `File`'s description, its text, a value written to it or read from it in its
//! binary form, its position, size and length, and the filesystem's own operations.  The
//! runtime half is `loft_io.h`; the binary forms of the program's types are `crate::c::files`.

use super::{CType, E, FnWriter, K, as_int, as_ref, as_view};
use crate::data::Value;
use crate::database::Parts;

impl FnWriter<'_> {
    /// The variable an `OpCreateStack(v)` operand names: where the bytecode hands a file operation
    /// the slot of the value it writes or reads, the C hands it the variable.
    fn stack_var(&self, v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Call(d, args) if self.cx.data.def(*d).name() == "OpCreateStack" => {
                match args.as_slice() {
                    [Value::Var(var)] => Some(*var),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The file operators, or `None` for another.
    pub(super) fn file_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let one = |s: &mut Self, f: &str, k: K| -> Result<Option<E>, String> {
            let a = s.expr(&args[0], ind)?;
            Ok(Some(E::new(format!("{f}({})", as_ref(a)?), k)))
        };
        let path = |s: &mut Self, f: &str| -> Result<Option<E>, String> {
            let a = s.expr(&args[0], ind)?;
            Ok(Some(E::new(format!("{f}({})", as_view(a)?), K::Int)))
        };
        match (name, args) {
            ("OpGetFile", [_]) => one(self, "loft_get_file", K::Bool),
            ("OpSizeFile", [_]) => one(self, "loft_size_file", K::Int),
            ("OpSyncFile", [_]) => one(self, "loft_sync_file", K::Bool),
            ("OpDelete", [_]) => path(self, "loft_fs_delete"),
            ("OpRmdir", [_]) => path(self, "loft_fs_rmdir"),
            ("OpMkdir", [p]) => {
                let p = self.expr(p, ind)?;
                Ok(Some(E::new(
                    format!("loft_fs_mkdir({}, 0)", as_view(p)?),
                    K::Int,
                )))
            }
            ("OpMkdirAll", [p]) => {
                let p = self.expr(p, ind)?;
                Ok(Some(E::new(
                    format!("loft_fs_mkdir({}, 1)", as_view(p)?),
                    K::Int,
                )))
            }
            ("OpMoveFile", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                Ok(Some(E::new(
                    format!("loft_fs_move({}, {})", as_view(a)?, as_view(b)?),
                    K::Int,
                )))
            }
            ("OpSeekFile" | "OpTruncateFile", [_, _]) => {
                let (f, n) = self.two(name, args, ind)?;
                let (call, k) = if name == "OpSeekFile" {
                    ("loft_seek_file", K::Void)
                } else {
                    ("loft_truncate_file", K::Bool)
                };
                Ok(Some(E::new(
                    format!("{call}({}, {})", as_ref(f)?, as_int(n)?),
                    k,
                )))
            }
            ("OpGetFileText", [f, t]) => {
                let dest = match self.stack_var(t) {
                    Some(v) => self.text_dest(&Value::Var(v))?,
                    None => self.text_dest(t)?,
                };
                let f = self.expr(f, ind)?;
                Ok(Some(E::new(
                    format!("loft_get_file_text({}, &{dest})", as_ref(f)?),
                    K::Void,
                )))
            }
            ("OpGetDir", [_, _]) => {
                let file = self.cx.stores.name("File");
                let size = self.cx.stores.size(file);
                let (p, v) = self.two(name, args, ind)?;
                Ok(Some(E::new(
                    format!("loft_get_dir({}, {}, {size})", as_view(p)?, as_ref(v)?),
                    K::Bool,
                )))
            }
            ("OpWriteFile", [f, v, tp]) => self.write_file(f, v, tp, ind).map(Some),
            ("OpReadFile", [f, v, n, tp]) => self.read_file(f, v, n, tp, ind).map(Some),
            _ => Ok(None),
        }
    }

    /// The narrow storage kind of type `tp` — `Parts::Byte` and the other sized integers — and
    /// whether it is signed (its range starts below 0), or `None` for any other type.
    fn narrow_kind(&self, tp: u16) -> Option<(u8, bool)> {
        match self.cx.stores.types.get(tp as usize).map(|t| &t.parts)? {
            Parts::Byte(from, _) => Some((1, *from < 0)),
            Parts::Short(from, _) | Parts::ShortRaw(from, _) => Some((2, *from < 0)),
            Parts::Int(..) => Some((4, true)),
            Parts::IntRaw(..) => Some((4, false)),
            _ => None,
        }
    }

    /// Is type `tp` a plain enum, written and read as its one byte?
    fn plain_enum(&self, tp: u16) -> bool {
        matches!(
            self.cx.stores.types.get(tp as usize).map(|t| &t.parts),
            Some(Parts::Enum(_))
        )
    }

    /// `OpWriteFile(f, OpCreateStack(v), tp)` (State::write_file): `v` in its binary form at the
    /// File's next position.
    fn write_file(&mut self, f: &Value, v: &Value, tp: &Value, ind: usize) -> Result<E, String> {
        let var = self
            .stack_var(v)
            .ok_or("a file write of a value that is not a variable")?;
        let tp = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let file = self.expr(f, ind)?;
        let fr = self.temp(CType::Ref);
        self.line(ind, &format!("{fr} = {};", as_ref(file)?));
        let value = self.expr(&Value::Var(var), ind)?;
        let put = |n: u8, x: &str| format!("loft_put(&loft_b, (uint64_t) ({x}), {n}, loft_le);");
        let ser = match (tp, self.narrow_kind(tp)) {
            (0 | 1, _) => put(8, &as_int(value)?),
            (2, _) => put(4, &format!("loft_bits32({})", super::as_single(value)?)),
            (3, _) => put(8, &format!("loft_bits64({})", super::as_float(value)?)),
            (4, _) => format!(
                "loft_push_char(&loft_b, (char) {});",
                super::as_bool(value)?
            ),
            (5, _) => format!("loft_text_append(&loft_b, {});", as_view(value)?),
            (6, _) => put(4, &super::as_char(value)?),
            (_, Some((n, _))) => put(n, &format!("(uint64_t) {}", as_int(value)?)),
            _ if self.plain_enum(tp) => format!(
                "loft_push_char(&loft_b, (char) {});",
                super::as_enum(value)?
            ),
            _ => {
                self.cx.ser_type(tp)?;
                format!("loft_ser{tp}(&loft_b, {}, loft_le);", as_ref(value)?)
            }
        };
        self.line(ind, "{");
        self.line(ind + 1, "loft_text loft_b = { 0, 0, 0 };");
        self.line(ind + 1, &format!("int loft_le = loft_file_le({fr});"));
        self.line(ind + 1, &ser);
        self.line(ind + 1, &format!("loft_write_file({fr}, &loft_b);"));
        self.line(ind + 1, "loft_text_free(&loft_b);");
        self.line(ind, "}");
        Ok(E::new("(void) 0", K::Void))
    }

    /// `OpReadFile(f, OpCreateStack(v), n, tp)` (State::read_file and `dispatch_read_data`): `n`
    /// bytes from the File's next position, decoded into `v` — a text takes what arrived; any
    /// other value only all `n` bytes, and no fewer than its width, or it stays as it was.
    fn read_file(
        &mut self,
        f: &Value,
        v: &Value,
        n: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let var = self
            .stack_var(v)
            .ok_or("a file read into a value that is not a variable")?;
        let tp = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let (file, count) = self.two("OpReadFile", &[f.clone(), n.clone()], ind)?;
        let (fr, nr) = (self.temp(CType::Ref), self.temp(CType::Int));
        self.line(ind, &format!("{fr} = {};", as_ref(file)?));
        self.line(ind, &format!("{nr} = {};", as_int(count)?));
        let text = self.cx.stores.is_text_type(tp);
        let width = self.cx.stores.binary_size(tp);
        let name = self.var(var);
        let take = |n: u8| format!("loft_take(loft_d, {n}, loft_le)");
        let set = match (tp, self.narrow_kind(tp)) {
            _ if text => {
                if !(self.shapes[var as usize] == CType::Text && self.text_held(var)) {
                    return Err("a file read into a text that is not a local".into());
                }
                format!("loft_text_set(&{name}, loft_lit((const char *) loft_d, (size_t) loft_n));")
            }
            (0 | 1, _) => format!("{{ uint64_t u = {}; memcpy(&{name}, &u, 8); }}", take(8)),
            (2, _) => format!("{name} = loft_f32((uint32_t) {});", take(4)),
            (3, _) => format!("{name} = loft_f64({});", take(8)),
            (4, _) => format!("{name} = loft_d[0];"),
            (6, _) => format!("{name} = (uint32_t) {};", take(4)),
            (_, Some((w, signed))) => {
                let bits = u32::from(w) * 8;
                if signed {
                    format!(
                        "{{ uint64_t u = {}; {name} = u >= (UINT64_C(1) << {}) ? (int64_t) u - (int64_t) (UINT64_C(1) << {bits}) : (int64_t) u; }}",
                        take(w),
                        bits - 1
                    )
                } else {
                    format!("{name} = (int64_t) {};", take(w))
                }
            }
            _ if self.plain_enum(tp) => format!("{name} = loft_d[0];"),
            _ => {
                self.cx.ser_type(tp)?;
                format!("loft_de{tp}({name}, loft_le, loft_d, (size_t) loft_n);")
            }
        };
        self.line(ind, "{");
        self.line(ind + 1, "uint8_t *loft_d;");
        self.line(
            ind + 1,
            &format!(
                "int64_t loft_n = loft_read_file({fr}, {nr}, {}, {width}, &loft_d);",
                i32::from(text)
            ),
        );
        self.line(ind + 1, &format!("int loft_le = loft_file_le({fr});"));
        let ok = if text {
            "loft_n >= 0".to_string()
        } else {
            format!("loft_n >= {width} && loft_n == {nr}")
        };
        self.line(ind + 1, &format!("if ({ok}) {set}"));
        self.line(ind + 1, "(void) loft_le;");
        self.line(ind + 1, "free(loft_d);");
        self.line(ind, "}");
        Ok(E::new("(void) 0", K::Void))
    }
}
