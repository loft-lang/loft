//! Rust → loft host-call API.
//!
//! Load a loft program once, then call any `pub fn` by name with typed Rust
//! arguments and get a typed return — errors as a `Result`. See
//! `doc/claude/plans/rust-host-call-api.md` for the design.
//!
//! ```ignore
//! let mut prog = loft::host::Program::from_source("fn add(a: integer, b: integer) -> integer { a + b }")?;
//! let out = prog.call("add", &[Value::Int(2), Value::Int(3)])?;   // Value::Int(5)
//! ```
//!
//! The invariant: a host call routes through the SAME stack ABI the interpreter
//! uses internally (`State::execute_host`, the general analogue of the par-worker
//! `execute_at_*` family). It never re-implements how a value is pushed or read,
//! so `call(f, args)` is equivalent to an in-language call of `f`.
//!
//! Parameters and returns may be **text, integer-family, single, boolean, and
//! void**, plus — @PLN119 arc B — a **struct or vector**, which does not fit in a
//! `Value` and crosses as [`Value::Ref`]: a record in a store the caller and the
//! callee both have mapped. Anything else in a signature returns a clean
//! [`LoftError::Unsupported`] rather than being mis-marshalled.

// @I85 — Engine-host kernel natives (the controlled loft ↔ host boundary; this is
// the Rust-host side of it: a host program calling into a loaded loft program).
use crate::compile;
use crate::data::{Data, Type};
use crate::parser;
use crate::scopes;
use crate::state::{HostRetKind, HostReturn, State, WorkerArg};

/// A typed value passed to, or returned from, a loft function.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Void,
    Bool(bool),
    /// integer family (`integer`, `u8`, `i32`, …) — width taken from the parameter type.
    Int(i64),
    /// `single`.
    Float(f64),
    Text(String),
    /// @PLN119 arc B — a struct / vector value, named by the record it lives in.
    ///
    /// Unlike every variant above, this does not CARRY the value: a compound is a
    /// graph of records, and the caller and the callee address it through a store
    /// they both have mapped (@PLN119's call arena). The two are responsible for
    /// agreeing on its layout — `Stores::layout_algo_hash` is that agreement —
    /// because the bytes are read as the receiving program's own type.
    ///
    /// [`DbRef::NULL`](crate::keys::DbRef::NULL) is the absent value.
    Ref(crate::keys::DbRef),
}

impl Value {
    /// The return value as an owned `String`, or a type error.
    ///
    /// # Errors
    /// [`LoftError::ReturnType`] if the value is not a `text`.
    pub fn into_text(self) -> Result<String, LoftError> {
        match self {
            Value::Text(s) => Ok(s),
            other => Err(LoftError::ReturnType {
                expected: "text".into(),
                got: other.kind_name().into(),
            }),
        }
    }
    /// The return value as an `i64`, or `None` if it is not an integer.
    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }
    /// The return value as a `&str`, or `None` if it is not text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
    fn kind_name(&self) -> &'static str {
        match self {
            Value::Void => "void",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "single",
            Value::Text(_) => "text",
            Value::Ref(_) => "struct/vector",
        }
    }
}

/// A failure from loading or calling a loft program.
#[derive(Debug, Clone)]
pub enum LoftError {
    /// The source did not parse / compile.
    Parse(String),
    /// No `pub fn` (or `fn`) of this name in the program.
    UnknownFn(String),
    /// The call passed the wrong number of arguments.
    ArgCount {
        func: String,
        expected: usize,
        got: usize,
    },
    /// An argument's `Value` variant does not match the declared parameter type.
    ArgType {
        func: String,
        index: usize,
        expected: String,
        got: String,
    },
    /// A parameter or the return type is outside the Phase-1 supported set
    /// (a struct / vector / enum). Not silently mis-marshalled.
    Unsupported { func: String, what: String },
    /// The requested return conversion did not match the value produced.
    ReturnType { expected: String, got: String },
    /// The loft function raised / faulted at runtime.
    ///
    /// The WHOLE fault, not its message: a placed library relays this across the
    /// crossing and rebuilds it on the caller's side, and a fault that arrived as
    /// prose has already lost the position, the caret and the frames that make it
    /// readable.
    Runtime(Box<crate::runtime_error::RuntimeError>),
}

impl std::fmt::Display for LoftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoftError::Parse(m) => write!(f, "parse error: {m}"),
            LoftError::UnknownFn(n) => write!(f, "unknown function `{n}`"),
            LoftError::ArgCount {
                func,
                expected,
                got,
            } => write!(f, "`{func}` expects {expected} argument(s), got {got}"),
            LoftError::ArgType {
                func,
                index,
                expected,
                got,
            } => write!(
                f,
                "`{func}` argument {index}: expected {expected}, got {got}"
            ),
            LoftError::Unsupported { func, what } => {
                write!(f, "`{func}`: unsupported type in signature ({what})")
            }
            LoftError::ReturnType { expected, got } => {
                write!(f, "return is {got}, not {expected}")
            }
            LoftError::Runtime(e) => write!(f, "runtime error: {}", e.message),
        }
    }
}

impl std::error::Error for LoftError {}

/// A loaded, compiled loft program ready to be called from Rust.
pub struct Program {
    data: Data,
    state: State,
}

impl Program {
    /// Load `source` (plus the loft standard library) and compile it. The stdlib
    /// directory is resolved next to the running executable, then via
    /// `LOFT_DEFAULT_DIR`, then `./default`.
    ///
    /// # Errors
    /// [`LoftError::Parse`] if the stdlib directory cannot be read or the source
    /// fails to parse / compile.
    pub fn from_source(source: &str) -> Result<Program, LoftError> {
        Self::from_source_with_stdlib(source, &default_stdlib_dir())
    }

    /// Like [`Program::from_source`] but with an explicit stdlib directory
    /// (the `default/` folder shipped beside the `loft` binary).
    ///
    /// # Errors
    /// [`LoftError::Parse`] if `stdlib_dir` cannot be read or the source fails to
    /// parse / compile.
    pub fn from_source_with_stdlib(source: &str, stdlib_dir: &str) -> Result<Program, LoftError> {
        let mut p = parser::Parser::new();
        p.parse_dir(stdlib_dir, true, false)
            .map_err(|e| LoftError::Parse(format!("cannot read stdlib at {stdlib_dir}: {e}")))?;
        p.parse_str(source, "<host>", false);
        scopes::check(&mut p.data, &mut p.database);
        let mut data = p.data;
        let mut state = State::new(p.database);
        compile::byte_code(&mut state, &mut data);
        Ok(Program { data, state })
    }

    /// Load every `.loft` file of the library package rooted at `pkg_dir`
    /// (its `src/` directory), plus the stdlib.
    ///
    /// The whole-directory form exists for @PLN119's out-of-process worker,
    /// which holds a library rather than a snippet: a library is several files
    /// and its `use` declarations resolve against its own package root, neither
    /// of which survives being flattened into one source string.
    ///
    /// # Errors
    /// [`LoftError::Parse`] if either directory cannot be read, or the library
    /// fails to parse / compile.
    pub fn from_library_dir(
        pkg_dir: &std::path::Path,
        stdlib_dir: &std::path::Path,
    ) -> Result<Program, LoftError> {
        let mut p = parser::Parser::new();
        p.parse_dir(&stdlib_dir.to_string_lossy(), true, false)
            .map_err(|e| {
                LoftError::Parse(format!(
                    "cannot read stdlib at {}: {e}",
                    stdlib_dir.display()
                ))
            })?;
        let src = pkg_dir.join("src");
        let dir = if src.is_dir() {
            src
        } else {
            pkg_dir.to_path_buf()
        };
        p.parse_dir(&dir.to_string_lossy(), false, false)
            .map_err(|e| {
                LoftError::Parse(format!("cannot read library at {}: {e}", dir.display()))
            })?;
        scopes::check(&mut p.data, &mut p.database);
        let mut data = p.data;
        let mut state = State::new(p.database);
        compile::byte_code(&mut state, &mut data);
        Ok(Program { data, state })
    }

    /// How many stores this program holds — the base a caller translates a
    /// `DbRef` against when references start crossing a placement boundary
    /// (@PLN119 arc B).
    #[must_use]
    pub fn store_count(&self) -> u32 {
        self.state.database.allocations.len() as u32
    }

    /// The mutation counter of this program's first store (CO1.9/S28), which a
    /// cross-process reader compares to detect a mapping made stale by a
    /// structural change.
    #[must_use]
    pub fn store_epoch(&self) -> u32 {
        self.state
            .database
            .allocations
            .first()
            .map_or(0, |s| s.generation)
    }

    /// Does this program define a `pub fn` of this name that a placed library
    /// can serve? Used to refuse a bad call at load rather than at first use.
    #[must_use]
    pub fn has_fn(&self, func: &str) -> bool {
        self.data.def_nr(&format!("n_{func}")) != u32::MAX
    }

    // The five methods below serve @PLN119's placement layer, whose transport is
    // built on `futex` and so compiles only on Linux (`src/lib_placement.rs`).
    // Every other target — macOS, Windows, and the wasm builds — sees them dead,
    // and that is the platform gate, not rot.

    /// The program's stores — where @PLN119's worker binds the call arena so a
    /// compound argument is addressable from inside the call.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn stores(&mut self) -> &mut crate::database::Stores {
        &mut self.state.database
    }

    /// Anchor this program's relative file access at `dir`.
    ///
    /// A loaded library resolves `file("x")` against its OWN directory, which is
    /// right for a program and wrong for @PLN119's worker: the calls it serves
    /// were written by a consumer somewhere else, and in-process that consumer's
    /// directory is what `file("x")` means. Without this a placed library reads a
    /// different file from the same library in-process — silently, because both
    /// paths are perfectly valid, just not the same one.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn anchor_paths_at(&mut self, dir: &std::path::Path) {
        self.state.database.source_dir = dir.to_string_lossy().into_owned();
        self.state.database.program_relative = true;
    }

    /// The store type id this program uses for `ty`, and the layout it reads
    /// those bytes under (@PLN97's `layout_algo_hash`).
    ///
    /// Two programs that place a compound value in a shared store must agree on
    /// both, and they are different programs — so the agreement is CHECKED
    /// rather than assumed. A worker built from other sources, or from the same
    /// sources compiled differently, would otherwise read the caller's bytes as
    /// whatever its own type happens to lay out at those offsets.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn layout_of(&mut self, ty: &Type) -> (u16, u64) {
        let tp = self.state.database.db_type(ty, &self.data);
        (tp, self.state.database.layout_algo_hash(&[tp]))
    }

    /// Who frees the storage `func`'s heap return names — @PLN103's delivery
    /// lens, asked of the worker's own copy of the library.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn return_delivery(&self, func: &str) -> crate::use_analysis::HeapDelivery {
        let d_nr = self.data.def_nr(&format!("n_{func}"));
        if d_nr == u32::MAX {
            return crate::use_analysis::HeapDelivery::NotHeap;
        }
        crate::use_analysis::heap_return_delivery(&self.data, d_nr)
    }

    /// The declared type of `func`'s parameters (hidden ones excluded) and of its
    /// return — the signature the placement layer hashes for the layout gate.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn signature(&self, func: &str) -> Option<(Vec<Type>, Type)> {
        let d_nr = self.data.def_nr(&format!("n_{func}"));
        if d_nr == u32::MAX {
            return None;
        }
        let def = self.data.def(d_nr);
        Some((
            def.attributes()
                .iter()
                .filter(|a| !a.hidden)
                .map(|a| a.typedef.clone())
                .collect(),
            def.returned.clone(),
        ))
    }

    /// Call `func` with `args`, returning its value. Reusable — call as many
    /// times as needed; each call runs on a fresh stack frame.
    ///
    /// # Errors
    /// [`LoftError::UnknownFn`] if no `fn` of that name exists;
    /// [`LoftError::ArgType`] / [`LoftError::ArgCount`] on an argument mismatch;
    /// [`LoftError::Runtime`] if the call raises at runtime;
    /// [`LoftError::ReturnType`] if the return value cannot be marshalled.
    pub fn call(&mut self, func: &str, args: &[Value]) -> Result<Value, LoftError> {
        self.call_into(func, args, crate::keys::DbRef::NULL)
    }

    /// [`call`](Program::call), with somewhere for a compound return to be built.
    ///
    /// A loft function that returns a struct or a vector carries a hidden
    /// `__retbuf` parameter: the destination the caller offers for the answer.
    /// The caller may offer nothing (`DbRef::NULL`) — an ordinary in-language
    /// call often does, and the callee then mints its own store and hands
    /// ownership back — but @PLN119's worker offers a record in the shared arena,
    /// which is what lets the answer be BUILT where the other process will read
    /// it instead of copied there afterwards.
    ///
    /// # Errors
    /// As [`call`](Program::call).
    pub(crate) fn call_into(
        &mut self,
        func: &str,
        args: &[Value],
        retbuf: crate::keys::DbRef,
    ) -> Result<Value, LoftError> {
        let d_nr = self.data.def_nr(&format!("n_{func}"));
        if d_nr == u32::MAX {
            return Err(LoftError::UnknownFn(func.to_string()));
        }
        let def = self.data.def(d_nr);
        let fn_pos = def.code_position;

        // User parameters = the non-hidden attributes, in order.
        let params: Vec<Type> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden)
            .map(|a| a.typedef.clone())
            .collect();
        if params.len() != args.len() {
            return Err(LoftError::ArgCount {
                func: func.to_string(),
                expected: params.len(),
                got: args.len(),
            });
        }
        // Hidden text work-buffers the callee carries (a `-> text` return needs them).
        let n_hidden_text = def
            .attributes()
            .iter()
            .filter(|a| crate::native_lib::is_text_work_buffer(&a.typedef))
            .count();
        // The hidden compound destination (`__retbuf`), if this function has one.
        // It is pushed like any other argument, so a call that skipped it would
        // not fail here — it would shift every later frame by one cell.
        let n_retbuf = def
            .attributes()
            .iter()
            .filter(|a| a.hidden && !a.work_buffer && is_compound(&a.typedef))
            .count();
        if n_retbuf > 1 {
            return Err(LoftError::Unsupported {
                func: func.to_string(),
                what: "more than one hidden destination parameter".to_string(),
            });
        }
        let ret = ret_kind(func, &def.returned)?;

        // Marshal arguments by declared type. `WorkerArg::Text` borrows from `args`,
        // which outlives the call.
        let mut wargs: Vec<WorkerArg> = Vec::with_capacity(args.len());
        for (i, (val, ty)) in args.iter().zip(params.iter()).enumerate() {
            wargs.push(marshal_arg(func, i, val, ty)?);
        }
        // The hidden compound parameters, in declaration order: the return buffer takes
        // the caller's offer; a work buffer (`@FR-R-WorkBuffer`) takes the null sentinel,
        // on which the callee mints a store of its own and releases it at exit.
        for a in def.attributes() {
            if a.hidden && is_compound(&a.typedef) {
                wargs.push(WorkerArg::Ref(if a.work_buffer {
                    crate::keys::DbRef::NULL
                } else {
                    retbuf
                }));
            }
        }

        let out = self
            .state
            .execute_host(&self.data, fn_pos, &wargs, n_hidden_text, ret);

        // A raise / fault populates `runtime_error`; surface it as an error, drained.
        if let Some(err) = self.state.database.runtime_error.take() {
            return Err(LoftError::Runtime(err));
        }

        Ok(marshal_return(&def_return_type(&self.data, d_nr), out))
    }
}

/// Owned copy of the return type (avoids borrowing `self.data` across `execute_host`).
fn def_return_type(data: &Data, d_nr: u32) -> Type {
    data.def(d_nr).returned.clone()
}

/// How many bytes this type occupies in a stack cell — the SAME answer the
/// compiled callee uses, because it is the same function.
///
/// A host call writes an argument into a cell the callee then reads, so a second
/// opinion about its width is a wrong value rather than a crash. That is not
/// hypothetical: an integer's STORAGE width (`IntegerSpec::byte_width`, what a
/// `u8` struct field occupies) is not its STACK width, which is 8 for every
/// integer alias — `variables::size` narrows only in `Context::Constant`. And
/// because the stack steps in 8-byte units (`aligned_stack_step`), writing a
/// `u8` argument as one byte does not shorten the frame; it leaves the cell's
/// other seven bytes holding whatever the PREVIOUS call left there, and the
/// callee reads them as part of the number (@PLN119: `f(1)` answered
/// `0x0F0F0F0F0F0F0F01` after a call that had put `0x0F..0F` in that cell).
fn cell_width(ty: &Type) -> u32 {
    u32::from(crate::variables::size(ty, &crate::data::Context::Argument))
}

/// A struct or a vector: a value that does not fit in a stack cell and travels
/// as the `DbRef` naming the record it lives in.
///
/// The single home for "does this type cross as a reference" — the host
/// marshaller, the placement dispatcher, and the layout gate must all answer it
/// the same way, and a disagreement between them is a frame read off by one
/// cell rather than a compile error.
///
/// Deliberately NOT every reference-shaped type: a keyed collection
/// (`hash`/`index`/`sorted`) and a tuple are reference-shaped too, but their
/// crossing has questions of its own (a tuple is stack-allocated; a keyed
/// collection carries an index whose ordering is the caller's). They are
/// refused, which runs them in-process — the same fallback every unsupported
/// signature takes.
#[must_use]
pub(crate) fn is_compound(ty: &Type) -> bool {
    matches!(ty, Type::Reference(_, _) | Type::Vector(_, _))
}

/// Classify a parameter/return `Type` for the stack ABI, or reject it.
fn ret_kind(func: &str, ty: &Type) -> Result<HostRetKind, LoftError> {
    match ty {
        Type::Void | Type::Null => Ok(HostRetKind::Void),
        Type::Boolean | Type::Single | Type::Integer(_) => Ok(HostRetKind::Prim(cell_width(ty))),
        Type::Text(_) => Ok(HostRetKind::Text),
        _ if is_compound(ty) => Ok(HostRetKind::Ref),
        other => Err(LoftError::Unsupported {
            func: func.to_string(),
            what: format!("return type {}", type_label(other)),
        }),
    }
}

/// Marshal one Rust `Value` into the stack ABI per its declared loft `Type`.
fn marshal_arg(func: &str, i: usize, val: &Value, ty: &Type) -> Result<WorkerArg, LoftError> {
    let mismatch = |expected: &str| LoftError::ArgType {
        func: func.to_string(),
        index: i,
        expected: expected.to_string(),
        got: val_label(val).to_string(),
    };
    match ty {
        Type::Text(_) => match val {
            Value::Text(s) => Ok(WorkerArg::Text(crate::keys::Str::new(s))),
            _ => Err(mismatch("text")),
        },
        Type::Boolean => match val {
            Value::Bool(b) => Ok(WorkerArg::Primitive {
                value: u64::from(*b),
                size: cell_width(ty),
            }),
            _ => Err(mismatch("boolean")),
        },
        Type::Single => match val {
            Value::Float(x) => Ok(WorkerArg::Primitive {
                value: u64::from((*x as f32).to_bits()),
                size: cell_width(ty),
            }),
            _ => Err(mismatch("single")),
        },
        // The whole cell, sign included: a `u8` and an `integer` parameter are
        // the same eight bytes here, and it is the callee's declared type that
        // decides what the number means. See [`cell_width`].
        Type::Integer(_) => match val {
            Value::Int(v) => Ok(WorkerArg::Primitive {
                value: *v as u64,
                size: cell_width(ty),
            }),
            _ => Err(mismatch("integer")),
        },
        // A struct / vector argument is the record it lives in, not its bytes.
        // The caller has already put it somewhere the callee can reach — for
        // @PLN119 that is the shared arena — so what crosses the ABI is the
        // 12-byte `DbRef`, exactly as an in-language call passes one.
        _ if is_compound(ty) => match val {
            Value::Ref(r) => Ok(WorkerArg::Ref(*r)),
            _ => Err(mismatch("struct/vector")),
        },
        other => Err(LoftError::Unsupported {
            func: func.to_string(),
            what: format!("parameter {i} type {}", type_label(other)),
        }),
    }
}

/// Marshal the ABI return back into a Rust `Value` per the declared return type.
fn marshal_return(ty: &Type, out: HostReturn) -> Value {
    match (ty, out) {
        (_, HostReturn::Void) => Value::Void,
        (Type::Boolean, HostReturn::Prim(v)) => Value::Bool(v != 0),
        (Type::Single, HostReturn::Prim(v)) => Value::Float(f64::from(f32::from_bits(v as u32))),
        (Type::Integer(_), HostReturn::Prim(v)) => Value::Int(v as i64),
        (_, HostReturn::Prim(v)) => Value::Int(v as i64),
        (_, HostReturn::Text(s)) => Value::Text(s),
        (_, HostReturn::Ref(r)) => Value::Ref(r),
    }
}

fn val_label(v: &Value) -> &'static str {
    match v {
        Value::Void => "void",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "single",
        Value::Text(_) => "text",
        Value::Ref(_) => "struct/vector",
    }
}

fn type_label(t: &Type) -> &'static str {
    match t {
        Type::Void => "void",
        Type::Null => "null",
        Type::Boolean => "boolean",
        Type::Single => "single",
        Type::Integer(_) => "integer",
        Type::Text(_) => "text",
        Type::Vector(_, _) => "vector",
        Type::Reference(_, _) => "struct",
        Type::Enum(_, _, _) => "enum",
        _ => "unsupported",
    }
}

/// Resolve the stdlib `default/` directory: next to the executable, then
/// `LOFT_DEFAULT_DIR`, then `./default`.
fn default_stdlib_dir() -> String {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let cand = dir.join("../default");
        if cand.exists() {
            return cand.to_string_lossy().into_owned();
        }
    }
    if let Ok(dir) = std::env::var("LOFT_DEFAULT_DIR") {
        return dir;
    }
    "default".to_string()
}
