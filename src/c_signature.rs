// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the `#c` signature every caller derives from (@PLN24 arc A).

//! @PLN24 arc A — the C signature a `#c` binding declares, and the one place it
//! is understood.
//!
//! ```loft
//! pub fn status(conn: integer) -> integer;   #c "PQstatus" "int(void*)"
//! ```
//!
//! **The declaration is the sole authority on the C signature.** That is not a
//! style choice — it is what the architecture probe left standing
//! (`tests/fixtures/c_abi/`). Pointed at deliberately wrong signatures, the
//! runtime caller did not fail: an arity-1 symbol called through an arity-3
//! trampoline returned the right answer (extra register arguments are ignored),
//! and a variadic function through a non-variadic one returned the right answer
//! too, by luck. There is no runtime signal to reconcile a second source of
//! truth against, so the check is here, at compile time, or nowhere.
//!
//! The Rust path (`#native`) is authoritative in the other direction — the
//! `#[loft_native]` macro reads the real Rust signature and generates a marshal
//! bridge, so the loft declaration can be loose. A `#c` library contains no
//! Rust and there is nothing to read; that absence is the whole reason this
//! module exists.
//!
//! Every consumer — the interpreter's trampoline choice, the `--native`
//! `extern "C"` emission, and each target arc E adds — derives its call from
//! [`CSignature::parse`] and never re-derives widths from the loft types. The
//! loft types cannot express them: `integer` is i64 whatever the C function
//! takes.
//!
//! Nothing calls a `#c` function yet (arc A lands inert). This module parses,
//! checks, and reports.

use crate::data::{Data, Type};

/// The data model a target uses for the C integer types, which is the whole of
/// what varies between the platforms loft builds for.
///
/// C type names are **not** fixed-width, and a signature string has to keep
/// meaning the same thing everywhere it is compiled: `long` is 64 bits on Linux
/// and macOS and 32 on Windows, and plain `char` is signed on x86-64 and
/// unsigned on AArch64. So the declaration stays portable and the *target*
/// decides what it means — exactly as a C compiler would read the same header.
/// Resolving instead of rejecting is what lets an author write the signature
/// their system header already shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CTarget {
    /// Width of C `long` in bits: 64 on LP64 (Linux, macOS), 32 on LLP64 (Windows).
    pub long_bits: u8,
    /// Whether plain `char` is signed (x86-64) or unsigned (AArch64).
    pub char_signed: bool,
}

impl CTarget {
    /// The target loft is compiling for. Host-derived: `#c` is native-only, and
    /// the cross-compiled targets (wasm) have no C ABI to bind to at all — that
    /// is arc E's subject, not a width question.
    #[must_use]
    pub fn host() -> CTarget {
        CTarget {
            long_bits: if cfg!(windows) { 32 } else { 64 },
            char_signed: !cfg!(any(target_arch = "aarch64", target_arch = "arm")),
        }
    }
}

/// The highest arity a `#c` binding can have, on **either** backend.
///
/// The interpreter calls through a fixed ladder of per-arity trampolines
/// (`c_call`), and the ceiling is a fact about the CONTRACT rather than about
/// that caller — the declaration is checked against it, so it lives with the
/// signature and is readable on every build, including the ones with no C
/// caller compiled in at all.
///
/// @PLN128 arc C — **32, and it binds both backends.** It was 12, enforced on
/// the interpreter only, which made `#c` two different languages: a 13-slot
/// binding compiled under `--native`, shipped, and failed for whoever
/// interpreted it — including `loft debug`, which IS the interpreter. The
/// author never saw it; the cost landed entirely on a downstream consumer who
/// had not written the declaration.
///
/// Unifying downward would have narrowed what compiles today, so the ceiling
/// was raised to meet `--native` instead: extending the ladder is loosening,
/// which COMPATIBILITY.md permits unconditionally, and it leaves the tightening
/// half theoretical rather than practical.
///
/// **Why 32.** A ladder cannot be unbounded, so the stopping point is chosen
/// rather than accidental, and past it an ANSI-C shim is the honest answer.
///
/// The number was originally sized off "`dgemm_`'s 13 by-reference arguments
/// cost two slots each, so it needs 26" — which arc D corrected. A `vector`
/// carries a count only where the C signature has one ([`plan`]), and a Fortran
/// routine takes none, so `dgemm_` costs **13**. Even LAPACK's 20+-argument
/// drivers now fit. The margin is far wider than the ceiling was chosen for,
/// which is a good position to be in and not a reason to move it.
pub const MAX_C_ARITY: usize = 32;

/// A C type as spelled in a `#c` signature.
///
/// Deliberately not a general C type model: it is exactly the vocabulary the
/// binding can carry, so a type that cannot cross is *unrepresentable* rather
/// than represented-and-rejected-later. `Float` is the one exception — it
/// parses so the diagnostic can name the shim, rather than reading as a
/// spelling mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CType {
    Void,
    /// An integer-class scalar: every C integer, and the class that collapses
    /// to one `u64` slot in the caller.
    Int {
        bits: u8,
        signed: bool,
    },
    /// Any pointer. `pointee` is kept for the diagnostic and for the
    /// `--native` extern emission, which has to write a real Rust type.
    ///
    /// `elem` is the pointee resolved to the value C reads at each stride, and
    /// it is what a `vector` argument is checked against (@PLN128 arc E). It is
    /// `None` for `void*`, for a pointer to a pointer, and for any spelling
    /// this binding does not resolve — all of which mean **opaque**: the author
    /// is asserting the bytes, which is how `write(2)` takes a `vector<u8>`.
    Pointer {
        konst: bool,
        pointee: String,
        elem: Option<CElement>,
    },
    /// Parsed so it can be REFUSED with a useful message. A float argument
    /// travels in an SSE register, not the integer registers the caller uses —
    /// a different register file, so no amount of casting reaches it. The C
    /// side wraps it in a shim that takes the bit pattern as an integer.
    Float {
        bits: u8,
    },
}

/// One element of an array, as the byte image both sides must agree on.
///
/// @PLN128 arc E — a `vector` reaches C as a pointer into loft's OWN element
/// bytes, with no conversion anywhere: `dispatch` hands over the store address
/// and the `--native` emission passes the same one. So the loft element type
/// and the C pointee are two spellings of a single layout, and a declaration
/// where they disagree is a wrong program with nothing to catch it at runtime —
/// measured against real OpenBLAS, `vector<integer>` against LAPACK's `int *`
/// pivot array read back `8589934593, 0` where the answer is `1, 2`, and
/// `vector<single>` against `double *` let C write 24 bytes into a 12-byte loft
/// vector. Both on both backends, exit 0, no diagnostic.
///
/// `bytes` is the STRIDE, which is the whole point: it is what C advances by
/// and what decides whether a write stays inside the vector. Signedness is
/// deliberately absent — it changes how a byte is read, never where the next
/// one is, and `vector<u8>` against `const char *` is the ordinary byte-buffer
/// idiom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CElement {
    /// Storage width in bytes — the stride.
    pub bytes: u8,
    /// Float-class rather than integer-class. Same width, entirely different
    /// value: C reading eight integer bytes as a `double` gets a denormal.
    pub float: bool,
}

impl CType {
    /// Does this type cross the boundary directly, or does it need a shim?
    #[must_use]
    pub fn is_integer_class(&self) -> bool {
        matches!(self, CType::Int { .. } | CType::Pointer { .. })
    }

    /// The Rust type this C type is declared as in a generated `extern "C"`
    /// block (@PLN24 arc C).
    ///
    /// The declared width is what makes a call correct, and the RETURN width
    /// especially: rustc truncates a 32-bit C return at the ABI boundary, so
    /// `as i64` then sign-extends it properly. A caller that read the same
    /// return as a bare `u64` — which is what a signature-blind trampoline does
    /// — turns -1 into 4294967295.
    ///
    /// Every pointer is `*const c_void`, because pointers share one ABI whatever
    /// they point at. The parser keeps the pointee spelling for diagnostics; the
    /// emission does not need it, which is one prediction the build corrected.
    #[must_use]
    pub fn rust_type(&self) -> &'static str {
        match self {
            CType::Void => "()",
            CType::Int {
                bits: 8,
                signed: true,
            } => "i8",
            CType::Int {
                bits: 8,
                signed: false,
            } => "u8",
            CType::Int {
                bits: 16,
                signed: true,
            } => "i16",
            CType::Int {
                bits: 16,
                signed: false,
            } => "u16",
            CType::Int {
                bits: 32,
                signed: true,
            } => "i32",
            CType::Int {
                bits: 32,
                signed: false,
            } => "u32",
            CType::Int { signed: true, .. } => "i64",
            CType::Int { signed: false, .. } => "u64",
            CType::Pointer { .. } => "*const std::ffi::c_void",
            CType::Float { bits: 32 } => "f32",
            CType::Float { .. } => "f64",
        }
    }

    /// How the type is named back to the author in a diagnostic.
    #[must_use]
    pub fn spelling(&self) -> String {
        match self {
            CType::Void => "void".to_string(),
            CType::Int { bits, signed } => {
                format!(
                    "{}{bits}-bit integer",
                    if *signed { "signed " } else { "unsigned " }
                )
            }
            CType::Pointer { konst, pointee, .. } => {
                format!("{}{pointee} *", if *konst { "const " } else { "" })
            }
            CType::Float { bits } => format!("{bits}-bit float"),
        }
    }
}

impl CElement {
    /// The array element this C type is, or `None` when it is not a value C
    /// lays out in an array — `void` and every pointer, which are the opaque
    /// cases a `vector` may cross against unchecked.
    #[must_use]
    fn of_c(t: &CType) -> Option<CElement> {
        match *t {
            CType::Int { bits, .. } => Some(CElement {
                bytes: bits / 8,
                float: false,
            }),
            CType::Float { bits } => Some(CElement {
                bytes: bits / 8,
                float: true,
            }),
            CType::Void | CType::Pointer { .. } => None,
        }
    }

    /// The array element ONE loft value is, or `None` when loft's bytes are not
    /// C's — which is a refusal, not an opaque pass.
    ///
    /// Two loft types look like a C scalar and are not:
    ///
    /// * **`i8` and `i16`.** Narrow storage encodes `val - min`
    ///   (`database/types.rs`, `key_descriptor_for_content`), so a loft `0` in a
    ///   `vector<i8>` is the byte `128`. The 4- and 8-byte forms store the value
    ///   itself, which is why `i32` and `integer` are fine and their narrower
    ///   siblings are not.
    /// * **`text`.** Its elements are loft's own 4-byte heap handles, so C
    ///   dereferencing them as `char *` follows a store offset as an address —
    ///   measured, an immediate SIGSEGV.
    #[must_use]
    fn of_loft(t: &Type) -> Option<CElement> {
        let bytes = u8::try_from(crate::data::element_storage_size(t)).ok()?;
        match t {
            // A nullable element carries loft's sentinel as an ordinary number,
            // which is the crossing `shape_of` already refuses for a scalar.
            Type::Optional(_) => None,
            Type::Integer(spec) => (bytes >= 4 || spec.min == 0).then_some(CElement {
                bytes,
                float: false,
            }),
            Type::Boolean | Type::Character => Some(CElement {
                bytes,
                float: false,
            }),
            Type::Float | Type::Single => Some(CElement { bytes, float: true }),
            _ => None,
        }
    }
}

/// A parsed `#c` signature: what the C function really takes and returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSignature {
    pub symbol: String,
    pub ret: CType,
    pub params: Vec<CType>,
}

impl CSignature {
    /// Parse `"<ret>(<param>, …)"` for `symbol`, resolving widths for `target`.
    ///
    /// # Errors
    ///
    /// Returns the whole message the author should see — a malformed signature,
    /// a C type this binding does not understand, or a variadic parameter list
    /// (which no fixed caller can make). The caller adds only the source
    /// position.
    pub fn parse(symbol: &str, text: &str, target: CTarget) -> Result<CSignature, String> {
        let text = text.trim();
        let open = text.find('(').ok_or_else(|| {
            format!(
                "`{text}` is not a C signature — expected `<return>(<params>)`, e.g. `int(void*)`"
            )
        })?;
        if !text.ends_with(')') {
            return Err(format!(
                "`{text}` is missing its closing `)` — expected `<return>(<params>)`, e.g. `int(void*)`"
            ));
        }
        let ret_src = text[..open].trim();
        let params_src = text[open + 1..text.len() - 1].trim();

        let ret = parse_type(ret_src, target)
            .map_err(|e| format!("in the return type of `{text}`: {e}"))?;

        let mut params = Vec::new();
        if !params_src.is_empty() && params_src != "void" {
            for piece in params_src.split(',') {
                let piece = piece.trim();
                if piece == "..." {
                    return Err(
                        "a variadic C function cannot be bound directly — the call site must set \
                         the ABI's vector-register count, which a fixed caller does not. Wrap it \
                         in an ANSI-C shim with a fixed parameter list and bind that"
                            .to_string(),
                    );
                }
                let t = parse_type(piece, target)
                    .map_err(|e| format!("in parameter `{piece}` of `{text}`: {e}"))?;
                if t == CType::Void {
                    return Err(format!(
                        "`void` is not a parameter type in `{text}` — write `()` for no parameters"
                    ));
                }
                params.push(t);
            }
        }
        Ok(CSignature {
            symbol: symbol.to_string(),
            ret,
            params,
        })
    }

    /// Every type that cannot cross directly, with the reason. Empty when the
    /// signature is bindable as written.
    ///
    /// Separate from `parse` because a float is a perfectly good C type and a
    /// perfectly bad binding: the author needs to be told which one they hit.
    #[must_use]
    pub fn boundary_refusals(&self) -> Vec<String> {
        let mut out = Vec::new();
        // @PLN128 arc B — these used to prescribe "a shim taking the bit
        // pattern as an integer", which loft CANNOT EXPRESS: there is no
        // float→bits conversion, and `x as integer` is a VALUE cast (2.5 → 2,
        // measured). The advice was reachable only for a literal an author had
        // converted by hand offline, so a real program holding a computed
        // double was told to do something impossible.
        //
        // The cure prescribed instead is one that works TODAY on both backends
        // and needs no new builtin: a loft `vector<float>` already crosses as
        // pointer-plus-count, so a 1-element vector carries a scalar double in,
        // and C's writes through a `double*` are visible to loft on return
        // (the write-back property the numeric stack is built on). Verified end
        // to end before this text was written.
        // @PLN128 arc E — a float RETURN is no longer refused. The two halves
        // are not one problem, which is the mistake this plan made when it
        // settled them together: an ARGUMENT would need a trampoline per subset
        // of positions that are float (`2^arity`, impossible), while the return
        // is a single axis and costs one more expansion of the same arity list.
        // Refusing it cost the level-1 BLAS *functions* — `ddot_`, `dnrm2_`,
        // `dasum_` — and every LAPACK auxiliary that answers a number, and the
        // cure it named was an ANSI-C shim per routine: a C toolchain in the
        // build of every numeric package, to work around a boundary that can
        // just be correct. See `c_call::call_at_arity_f64`.
        for (i, p) in self.params.iter().enumerate() {
            if let CType::Float { bits } = p {
                out.push(format!(
                    "parameter {} is a {bits}-bit float, which travels in an SSE register the \
                     caller does not write. Wrap `{}` in an ANSI-C shim that takes it by POINTER \
                     instead, and bind that — a loft `vector<float>` crosses as a pointer plus a \
                     count, so a 1-element vector carries a scalar",
                    i + 1,
                    self.symbol
                ));
            }
        }
        out
    }
}

/// Name the C positions that disagree with the loft declaration, for the case
/// where the parameter COUNT is reachable but no reading type-checks.
///
/// Deliberately a greedy left-to-right walk rather than a search: once no
/// reading fits there is no correct alignment to report against, and the walk
/// that consumes a count wherever C offers one is the alignment the author most
/// likely intended. It reports every position it can, not the first, because an
/// author fixing a long signature wants the whole list.
fn position_mismatches(data: &Data, sig: &CSignature, params: &[Type]) -> Vec<String> {
    let mut errs = Vec::new();
    let mut c = 0usize;
    for (i, p) in params.iter().enumerate() {
        let spelling = |at: usize| {
            sig.params.get(at).map_or_else(
                || "nothing — the signature ends here".to_string(),
                CType::spelling,
            )
        };
        match shape_of(p) {
            LoftCShape::Scalar => {
                if !matches!(
                    sig.params.get(c),
                    Some(CType::Int { .. } | CType::Pointer { .. })
                ) {
                    errs.push(format!(
                        "parameter {} is `{}` in loft but `{}` in C — a scalar needs a C integer \
                         type, or a pointer if it is a handle",
                        i + 1,
                        data.type_name_str(p),
                        spelling(c)
                    ));
                }
                c += 1;
            }
            LoftCShape::Pointer => {
                if !matches!(sig.params.get(c), Some(CType::Pointer { .. })) {
                    errs.push(format!(
                        "parameter {} is `{}` in loft but `{}` in C — a text crosses as a \
                         NUL-terminated `const char*`",
                        i + 1,
                        data.type_name_str(p),
                        spelling(c)
                    ));
                }
                c += 1;
            }
            LoftCShape::Vector(img) => {
                if !matches!(sig.params.get(c), Some(CType::Pointer { .. })) {
                    errs.push(format!(
                        "parameter {} is `{}` in loft, so C parameter {} must be the element \
                         pointer, not `{}`",
                        i + 1,
                        data.type_name_str(p),
                        c + 1,
                        spelling(c)
                    ));
                } else if let Some(CType::Pointer {
                    elem: Some(want), ..
                }) = sig.params.get(c)
                    && *want != img
                {
                    // @PLN128 arc E — the pointer is right and the ELEMENT is
                    // wrong, which is the one mismatch that produces a running
                    // program with wrong numbers in it. The author is reading a
                    // C header and a loft declaration side by side, so name
                    // both sides; the fix is always to change one of them.
                    //
                    // The two failures are NOT the same failure and the message
                    // must not merge them: a width disagreement walks off the
                    // end of the vector, while a same-width class disagreement
                    // stays inside it and only reads every value as a different
                    // number. Claiming an overrun that cannot happen is how a
                    // diagnostic teaches the wrong model.
                    let consequence = if want.bytes == img.bytes {
                        format!(
                            "the same width read as {}, so every element arrives as a different \
                             number — eight integer bytes read as a `double` are a denormal, not \
                             the integer",
                            if want.float { "floats" } else { "integers" }
                        )
                    } else {
                        format!(
                            "striding {} bytes, so C reads every element after the first from the \
                             wrong offset — and where C writes, past the end of the vector",
                            want.bytes
                        )
                    };
                    errs.push(format!(
                        "parameter {} is `{}` — {}-byte {} elements — but C parameter {} is \
                         `{}`, {consequence}. Declare the loft element the way the C header \
                         spells it, or `void*` in C if the bytes really are opaque",
                        i + 1,
                        data.type_name_str(p),
                        img.bytes,
                        if img.float { "float" } else { "integer" },
                        c + 1,
                        spelling(c),
                    ));
                }
                c += if matches!(sig.params.get(c + 1), Some(CType::Int { .. })) {
                    2
                } else {
                    1
                };
            }
            LoftCShape::Refused(_) => {}
        }
    }
    errs
}

/// Resolve one C type name. Unknown spellings are refused rather than guessed:
/// a `#c` declaration nobody can check at runtime must not contain a type
/// nobody checked at compile time either.
#[expect(clippy::too_many_lines, reason = "inherited")]
fn parse_type(src: &str, target: CTarget) -> Result<CType, String> {
    let src = src.trim();
    if let Some(base) = src.strip_suffix('*') {
        let base = base.trim();
        let (konst, pointee) = match base.strip_prefix("const ") {
            Some(rest) => (true, rest.trim()),
            None => (false, base),
        };
        if pointee.is_empty() {
            return Err(
                "a pointer needs a pointee type, e.g. `void*` or `const char*`".to_string(),
            );
        }
        // The pointer ITSELF is one machine word whatever it points at, so the
        // spelling is kept verbatim for the diagnostic and the `--native`
        // extern. What the pointee resolves to matters for exactly one
        // question — whether a loft `vector`'s elements are the same layout
        // (@PLN128 arc E) — and that resolution belongs here, where the target
        // that decides C's widths is still in hand. A spelling this binding
        // does not know (`struct foo`, a function pointer, `const char *const`)
        // stays `None`, which reads as opaque rather than as a refusal: the
        // pointee is diagnostics-only for every other purpose, and refusing on
        // it would break `write(2)`'s `const void*`.
        return Ok(CType::Pointer {
            konst,
            elem: parse_type(pointee, target)
                .ok()
                .as_ref()
                .and_then(CElement::of_c),
            pointee: pointee.to_string(),
        });
    }
    let norm = src.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = match norm.as_str() {
        "void" => CType::Void,
        "char" => CType::Int {
            bits: 8,
            signed: target.char_signed,
        },
        "signed char" => CType::Int {
            bits: 8,
            signed: true,
        },
        "unsigned char" => CType::Int {
            bits: 8,
            signed: false,
        },
        "short" | "short int" | "signed short" => CType::Int {
            bits: 16,
            signed: true,
        },
        "unsigned short" | "unsigned short int" => CType::Int {
            bits: 16,
            signed: false,
        },
        "int" | "signed" | "signed int" => CType::Int {
            bits: 32,
            signed: true,
        },
        "unsigned" | "unsigned int" => CType::Int {
            bits: 32,
            signed: false,
        },
        "long" | "long int" | "signed long" => CType::Int {
            bits: target.long_bits,
            signed: true,
        },
        "unsigned long" | "unsigned long int" => CType::Int {
            bits: target.long_bits,
            signed: false,
        },
        "long long" | "long long int" | "signed long long" => CType::Int {
            bits: 64,
            signed: true,
        },
        "unsigned long long" | "unsigned long long int" => CType::Int {
            bits: 64,
            signed: false,
        },
        "size_t" | "uintptr_t" => CType::Int {
            bits: 64,
            signed: false,
        },
        "ssize_t" | "intptr_t" | "ptrdiff_t" => CType::Int {
            bits: 64,
            signed: true,
        },
        "int8_t" => CType::Int {
            bits: 8,
            signed: true,
        },
        "int16_t" => CType::Int {
            bits: 16,
            signed: true,
        },
        "int32_t" => CType::Int {
            bits: 32,
            signed: true,
        },
        "int64_t" => CType::Int {
            bits: 64,
            signed: true,
        },
        "uint8_t" => CType::Int {
            bits: 8,
            signed: false,
        },
        "uint16_t" => CType::Int {
            bits: 16,
            signed: false,
        },
        "uint32_t" => CType::Int {
            bits: 32,
            signed: false,
        },
        "uint64_t" => CType::Int {
            bits: 64,
            signed: false,
        },
        "_Bool" | "bool" => CType::Int {
            bits: 8,
            signed: false,
        },
        "float" => CType::Float { bits: 32 },
        "double" => CType::Float { bits: 64 },
        other => {
            return Err(format!(
                "`{other}` is not a C type this binding understands. Allowed: the C integer types \
                 (char, short, int, long, long long, size_t, the int*_t / uint*_t family), any \
                 pointer (`void*`, `const char*`), `void`, and `float`/`double` — which then need \
                 a shim. A struct passed by value, or a type from a header, has to go through an \
                 ANSI-C shim that presents it as one of these"
            ));
        }
    };
    Ok(t)
}

/// Which C parameters one loft parameter may occupy, and with which C types.
///
/// This is the mapping table, and it lives here alone so that the arity check,
/// the interpreter caller and the `--native` emission cannot drift into three
/// different opinions about what a `vector` looks like from C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoftCShape {
    /// One integer-class C parameter.
    Scalar,
    /// One pointer.
    Pointer,
    /// A loft vector: an element pointer, optionally followed by a count. Which
    /// of the two it is, per parameter, is the C signature's decision —
    /// [`plan`]. The pointer is valid **for the duration of the call only**;
    /// the elements live in a loft store that may move afterwards.
    ///
    /// It carries the element's byte image because the pointer alone is not the
    /// contract: C strides by the POINTEE's width, so a pointee that disagrees
    /// with the loft element reads the wrong values and, when C writes,
    /// straight past the end of the vector (@PLN128 arc E).
    Vector(CElement),
    /// Cannot cross.
    Refused(&'static str),
}

/// How ONE loft parameter is realised in C parameter slots.
///
/// The three callers — the declaration check, the interpreter's `dispatch` and
/// the `--native` `extern "C"` emission — all read this rather than deciding
/// for themselves, because a `vector` is the one loft type whose slot count is
/// not a property of the loft type alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CArg {
    /// One integer-class slot: an integer, boolean, character or enum — or a
    /// handle whose value is a pointer.
    Scalar,
    /// One pointer: a `text`, as a NUL-terminated `const char *`.
    TextPointer,
    /// A vector as an element pointer **and** a count. C carries no length, so
    /// this is the shape a C API written for loft takes.
    VectorPtrCount,
    /// A vector as a **bare** element pointer, with no count.
    ///
    /// @PLN128 arc D — the Fortran shape, and the reason this enum exists.
    /// Every BLAS/LAPACK argument is a bare pointer: the routine learns the
    /// length from a separate `n` argument, or does not need one because the
    /// argument is a by-reference scalar. Measured before it was built: bound
    /// with a count, `dgemm_`-shaped symbols are unreachable — the honest
    /// signature is refused for arity and the shape loft accepted passed each
    /// count where the callee expected the next pointer, which SIGSEGV'd.
    VectorPtr,
}

impl CArg {
    /// How many C parameters this realisation occupies.
    #[must_use]
    pub fn slots(self) -> usize {
        match self {
            CArg::VectorPtrCount => 2,
            _ => 1,
        }
    }
}

/// Why no reading of the C signature fits the loft declaration.
///
/// Split from the message so the caller can fall back to a per-position walk
/// for `NoMatch`, where naming the first parameter that disagrees is far more
/// use than saying the whole thing failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanFailure {
    /// The C signature has a parameter count no assignment can reach.
    Arity { min: usize, max: usize },
    /// The count is reachable, but no assignment type-checks position by
    /// position.
    NoMatch,
}

/// Decide, per loft parameter, which C slots it occupies — the one place that
/// answer is derived.
///
/// A `vector` may cross as a bare pointer or as pointer-then-count, and **the C
/// signature decides which**, because the signature is the sole authority on
/// what the symbol takes. Both are real APIs: a C library written for loft
/// passes the length alongside the pointer, and every Fortran routine passes
/// each argument as a bare pointer.
///
/// **The assignment is unique when it exists**, which is what makes inferring
/// it safe rather than a guess. Suppose two readings both fit and take the
/// leftmost parameter where they differ: it is a vector, counted in one and
/// bare in the other, so from there one reading runs exactly one slot behind
/// the other. To end together, some later vector must be counted in the reading
/// that is behind — and that vector's *pointer* then lands on the slot the
/// other reading uses for its *count*. A slot cannot be both a pointer and an
/// integer, so the two readings cannot both type-check.
/// `every_reachable_shape_has_at_most_one_reading` searches the small shapes
/// exhaustively rather than resting on that argument alone.
///
/// # Errors
///
/// [`PlanFailure`] when no reading fits; the caller turns it into the message.
pub fn plan(params: &[Type], sig: &CSignature) -> Result<Vec<CArg>, PlanFailure> {
    let p = params.len();
    let a = sig.params.len();
    let shapes: Vec<LoftCShape> = params.iter().map(shape_of).collect();
    let (mut min, mut max) = (0usize, 0usize);
    for s in &shapes {
        match s {
            LoftCShape::Vector(_) => {
                min += 1;
                max += 2;
            }
            LoftCShape::Refused(_) => {}
            _ => {
                min += 1;
                max += 1;
            }
        }
    }
    if a < min || a > max {
        return Err(PlanFailure::Arity { min, max });
    }

    // `ways[i][c]` — how many readings consume loft parameters `i..` against C
    // parameters `c..` exactly, saturating at 2 so an ambiguous declaration is
    // distinguishable from a unique one without counting them all.
    let mut ways = vec![vec![0u8; a + 1]; p + 1];
    ways[p][a] = 1;
    for i in (0..p).rev() {
        for c in (0..=a).rev() {
            let mut w = 0u16;
            for opt in options(&shapes[i], sig, c) {
                w += u16::from(ways[i + 1][c + opt.slots()]);
            }
            ways[i][c] = u8::try_from(w.min(2)).unwrap_or(2);
        }
    }
    if ways[0][0] == 0 {
        return Err(PlanFailure::NoMatch);
    }

    let mut out = Vec::with_capacity(p);
    let mut c = 0usize;
    for (i, shape) in shapes.iter().enumerate() {
        let Some(pick) = options(shape, sig, c)
            .into_iter()
            .find(|o| ways[i + 1][c + o.slots()] > 0)
        else {
            return Err(PlanFailure::NoMatch);
        };
        c += pick.slots();
        out.push(pick);
    }
    Ok(out)
}

/// The realisations one loft parameter admits at C position `c`, in the order
/// the reconstruction prefers them.
///
/// `VectorPtrCount` is offered first so that a declaration which fits both ways
/// — which the uniqueness argument above says cannot happen — would still read
/// as it did before the bare-pointer form existed.
fn options(shape: &LoftCShape, sig: &CSignature, c: usize) -> Vec<CArg> {
    let at = |i: usize| sig.params.get(i);
    let is_ptr = |i: usize| matches!(at(i), Some(CType::Pointer { .. }));
    let is_int = |i: usize| matches!(at(i), Some(CType::Int { .. }));
    match shape {
        // A C POINTER is allowed here, and deliberately: it is the handle
        // convention (`PGconn *` held as a loft `integer`), which has to work
        // in both directions or it is not a convention at all.
        LoftCShape::Scalar if is_int(c) || is_ptr(c) => vec![CArg::Scalar],
        LoftCShape::Pointer if is_ptr(c) => vec![CArg::TextPointer],
        LoftCShape::Vector(img) if pointee_takes(at(c), *img) => {
            if is_int(c + 1) {
                vec![CArg::VectorPtrCount, CArg::VectorPtr]
            } else {
                vec![CArg::VectorPtr]
            }
        }
        _ => Vec::new(),
    }
}

/// May a loft vector whose elements are `img` cross at this C parameter?
///
/// Only at a POINTER, and only one whose pointee is the same layout — or one
/// that resolves to nothing, which is the opaque escape hatch (`void*`,
/// `struct foo*`) an author uses to say "these are bytes, I mean it".
fn pointee_takes(t: Option<&CType>, img: CElement) -> bool {
    match t {
        Some(CType::Pointer { elem, .. }) => elem.is_none_or(|e| e == img),
        _ => false,
    }
}

/// What a loft type looks like from C.
#[must_use]
pub fn shape_of(t: &Type) -> LoftCShape {
    match t {
        Type::Integer(_) | Type::Boolean | Type::Character | Type::Enum(_, _, _) => {
            LoftCShape::Scalar
        }
        Type::Text(_) => LoftCShape::Pointer,
        // @PLN128 arc E — a vector crosses as loft's own element bytes, so the
        // ELEMENT decides whether it crosses at all. The two shapes refused
        // here look like C scalars and are not: see [`CElement::of_loft`].
        Type::Vector(elem, _) => match CElement::of_loft(elem) {
            Some(img) => LoftCShape::Vector(img),
            None if matches!(**elem, Type::Text(_)) => LoftCShape::Refused(
                "a `vector<text>` cannot cross — its elements are loft's own heap handles, not \
                 `char *` pointers, so C dereferences a store offset as an address (measured: \
                 SIGSEGV). Pass one `text` at a time, or have C read the bytes through a `void*`",
            ),
            None if matches!(**elem, Type::Integer(_)) => LoftCShape::Refused(
                "a `vector<i8>` or `vector<i16>` cannot cross — loft stores narrow SIGNED \
                 elements as `val - min`, so a loft `0` is the byte `128`, and C would read every \
                 value shifted. Use `vector<i32>` or `vector<integer>`, `vector<u8>` / \
                 `vector<u16>` if the values are unsigned, or declare the C parameter `void*` if \
                 you really mean raw bytes",
            ),
            None if matches!(**elem, Type::Optional(_)) => LoftCShape::Refused(
                "a vector of nullable elements cannot cross — loft's null is an ordinary number \
                 over there, so C reads a sentinel as data. Declare the element non-null",
            ),
            None => LoftCShape::Refused(
                "this vector's elements have no C representation. A vector crosses as a pointer \
                 into loft's own element bytes, so each element must be one C value: an integer \
                 (`integer`, `i32`, `u32`, `u16`, `u8`), `boolean`, `character`, `float` or \
                 `single`",
            ),
        },
        // @PLN24 — `#c` is the declared edge of loft's no-runtime-errors rule,
        // and this is where that becomes a compile error instead of a promise.
        // A nullable value has no C representation: the sentinels are ordinary
        // numbers over there (`i64::MIN` is a number, `NULL` is a crash), so a
        // null crossing the boundary is either silent corruption or a fault C
        // takes. Discharging it on the loft side (`?? 0`, `x?`, `match`) keeps
        // the guarantee where loft can actually keep it.
        Type::Optional(_) => LoftCShape::Refused(
            "a nullable value cannot cross into C — C has no null model, so the sentinel would \
             arrive as an ordinary number or a fault. Discharge it first (`?? 0`, `x?`, or a \
             `match`) and declare the parameter non-null",
        ),
        // @PLN128 arc B — by POINTER, not by bit pattern. loft has no
        // float→bits conversion, so the bit-pattern advice this used to give
        // named a cure no program could write; the pointer form is the one the
        // numeric stack already uses (`daxpy` writes its result through
        // `double*`) and it works on both backends today.
        Type::Float | Type::Single => LoftCShape::Refused(
            "a float cannot cross directly — it travels in an SSE register the caller does not \
             touch. Pass it by POINTER through an ANSI-C shim: a 1-element `vector<float>` \
             crosses as `(const double*, int64_t)`, and C's writes through a `double*` are \
             visible to loft",
        ),
        // A loft record is a position inside a store that the allocator may
        // RELOCATE (`resize_store` reallocs the arena). Handing C an interior
        // pointer that a later claim invalidates is the store-lifetime bug
        // class, not a marshalling detail — so it is refused rather than
        // documented. Pass the fields, or an opaque handle C itself owns.
        Type::Reference(_, _) => LoftCShape::Refused(
            "a reference to a loft record cannot cross — records live in a store that may move \
             them. Pass the fields it needs as scalars, or have C own the object and pass its \
             handle as an `integer`",
        ),
        _ => LoftCShape::Refused(
            "this type has no C representation. Only integers, boolean, character, text, vectors \
             and C-owned handles (as `integer`) cross a `#c` boundary",
        ),
    }
}

/// Check a parsed signature against the loft declaration it annotates.
///
/// Returns every problem, not the first: an author fixing a signature wants the
/// whole list, and each message names the C position so a long parameter list
/// stays navigable.
#[must_use]
pub fn check(
    data: &Data,
    sig: &CSignature,
    params: &[Type],
    ret: &Type,
    void_return: bool,
) -> Vec<String> {
    let mut errs = sig.boundary_refusals();

    for (i, p) in params.iter().enumerate() {
        if let LoftCShape::Refused(why) = shape_of(p) {
            errs.push(format!("parameter {} cannot be bound: {why}", i + 1));
        }
    }

    // Arity and per-parameter class together, because with a vector free to
    // cross either way they are one question: `plan` succeeds exactly when some
    // reading of the C signature both fits and type-checks.
    if errs.is_empty() {
        match plan(params, sig) {
            Ok(_) => {}
            Err(PlanFailure::Arity { min, max }) => errs.push(if min == max {
                format!(
                    "the C signature takes {} parameter(s), the loft declaration needs {min} \
                     (a `text` is one `const char*`)",
                    sig.params.len()
                )
            } else {
                format!(
                    "the C signature takes {} parameter(s), the loft declaration needs between \
                     {min} and {max} — a `vector` crosses as a bare element pointer, or as a \
                     pointer AND a count where the C signature has an integer for it",
                    sig.params.len()
                )
            }),
            // The arity is reachable but nothing type-checks. Walk the
            // declaration greedily and name the first C position that
            // disagrees: which parameter is wrong is far more use than the fact
            // that the whole reading failed.
            Err(PlanFailure::NoMatch) => errs.extend(position_mismatches(data, sig, params)),
        }
    }

    // The return, which is the half the probe found broken. A 32-bit C return
    // read back as 64 bits turns -1 into 4294967295 — no crash, a plausible
    // large positive. The declared width is what the caller truncates to, so a
    // return with no declared width is the one thing that cannot be defaulted.
    if void_return {
        if sig.ret != CType::Void {
            errs.push(format!(
                "the loft declaration returns nothing but the C signature returns `{}`",
                sig.ret.spelling()
            ));
        }
    } else {
        // `shape_of` answers for the ARGUMENT direction, where a nullable type is
        // genuinely unrepresentable — loft has no value to hand C for it. A
        // RETURN is not symmetric: C's NULL is exactly "no string", so a
        // `char *` coming back is the one place a `τ?` is the HONEST type, and
        // the only one where loft can see the null the crossing already carries.
        // Declared `text`, the same NULL still arrives as loft's content
        // sentinel — spelling it `text?` does not add the null, it makes the
        // null-flow analysis demand a discharge for it.
        let ret_shape = match ret {
            Type::Optional(inner)
                if matches!(**inner, Type::Text(_))
                    && matches!(sig.ret, CType::Pointer { ref pointee, .. }
                        if pointee.ends_with("char")) =>
            {
                LoftCShape::Pointer
            }
            other => shape_of(other),
        };
        match (&sig.ret, ret_shape) {
            (CType::Void, _) => errs.push(format!(
                "the loft declaration returns `{}` but the C signature returns `void`",
                data.type_name_str(ret)
            )),
            // @PLN128 arc E — the float pair, ahead of every shape-based arm,
            // because `shape_of` answers for the ARGUMENT direction (where a
            // float genuinely cannot cross) and the return is not symmetric.
            // Held to the exact pair rather than widened: a C `float` leaves a
            // SINGLE in the return register, and reading those bits as a double
            // is a denormal, not the number.
            (CType::Float { bits: 64 }, _) if matches!(ret.base(), Type::Float) => {}
            (CType::Float { bits: 32 }, _) if matches!(ret.base(), Type::Single) => {}
            (CType::Float { bits }, _) => errs.push(format!(
                "the C signature returns a {bits}-bit float, so the loft declaration must return \
                 `{}`, not `{}`",
                if *bits == 32 { "single" } else { "float" },
                data.type_name_str(ret)
            )),
            (c, _) if matches!(ret.base(), Type::Float | Type::Single) => errs.push(format!(
                "the loft declaration returns `{}` but the C signature returns `{}` — a float \
                 comes back only from a C `float` or `double`",
                data.type_name_str(ret),
                c.spelling()
            )),
            (_, LoftCShape::Refused(why)) => {
                errs.push(format!("the return type cannot be bound: {why}"));
            }
            // A `char *` return bound to loft `text`. The bytes are COPIED up to
            // the first NUL and the pointer is never freed, which is the one
            // ownership answer C's type system cannot give: `strerror` and
            // `PQerrorMessage` hand back storage the caller must NOT free, while
            // `strdup` hands back storage it must. `const` does not separate them
            // (POSIX spells both `char *`), so guessing from the signature would
            // free static memory on a wrong guess — the failure that cannot be
            // recovered from. Borrowed is therefore the only default, and a
            // caller-frees function goes through the plan's shim, which is what
            // the shims are for. Stated at the boundary in PACKAGES_C_BINDING.md.
            (CType::Pointer { pointee, .. }, LoftCShape::Pointer) if pointee.ends_with("char") => {}
            // A text return from a pointer that is not spelled `char *`. The
            // declaration is the sole authority here, so it has to SAY it means a
            // string — `void *` bound to `text` is either a mistake or a handle
            // that wanted `integer`, and nothing at runtime tells the two apart.
            // (This is the one decision the pointee spelling carries; everywhere
            // else it is diagnostics only, because pointers share one ABI.)
            (CType::Pointer { pointee, .. }, LoftCShape::Pointer) => errs.push(format!(
                "`{}` returns `{pointee}*`, which cannot come back as a loft `text` — spell the \
                 return `char*` if it really is a C string, or bind it as an `integer` if it is \
                 an opaque handle",
                sig.symbol
            )),
            (CType::Int { .. }, LoftCShape::Scalar) => {}
            // A C function returning a pointer bound to a loft `integer` is the
            // handle convention (`PGconn *`), and it is deliberate.
            (CType::Pointer { .. }, LoftCShape::Scalar) => {}
            (c, _) => errs.push(format!(
                "the loft declaration returns `{}` but the C signature returns `{}`",
                data.type_name_str(ret),
                c.spelling()
            )),
        }
    }
    errs
}

/// Read the signature a definition declared, if any. The one accessor, so a
/// consumer cannot accidentally use the raw string.
#[must_use]
pub fn of(data: &Data, def_nr: u32, target: CTarget) -> Option<Result<CSignature, String>> {
    let d = data.def(def_nr);
    if d.c_sig.is_empty() {
        return None;
    }
    Some(CSignature::parse(&d.c_symbol, &d.c_sig, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LP64: CTarget = CTarget {
        long_bits: 64,
        char_signed: true,
    };
    const WIN: CTarget = CTarget {
        long_bits: 32,
        char_signed: true,
    };

    fn data() -> Data {
        Data::new()
    }

    fn int() -> Type {
        Type::Integer(crate::data::IntegerSpec::wide())
    }

    fn sig(s: &str) -> CSignature {
        CSignature::parse("sym", s, LP64).expect("parses")
    }

    #[test]
    fn parses_the_shapes_a_c_header_actually_shows() {
        let s = sig("int(void*)");
        assert_eq!(
            s.ret,
            CType::Int {
                bits: 32,
                signed: true
            }
        );
        assert_eq!(s.params.len(), 1);
        assert!(matches!(s.params[0], CType::Pointer { konst: false, .. }));

        let s = sig("const char*(void*, int)");
        assert!(matches!(s.ret, CType::Pointer { konst: true, .. }));
        assert_eq!(s.params.len(), 2);

        assert_eq!(sig("void()").params.len(), 0);
        assert_eq!(
            sig("void(void)").params.len(),
            0,
            "`(void)` is C for no parameters"
        );
        assert_eq!(
            sig("size_t(const char*)").ret,
            CType::Int {
                bits: 64,
                signed: false
            }
        );
    }

    /// The reason widths are resolved against a target rather than fixed: the
    /// SAME signature string has to keep meaning what the system header means.
    #[test]
    fn long_is_not_one_width() {
        assert_eq!(
            CSignature::parse("s", "long(int)", LP64).unwrap().ret,
            CType::Int {
                bits: 64,
                signed: true
            }
        );
        assert_eq!(
            CSignature::parse("s", "long(int)", WIN).unwrap().ret,
            CType::Int {
                bits: 32,
                signed: true
            },
            "LLP64 — a `long` return truncated to 64 bits on Windows would be the \
             loft-libs-net bug with a different sign"
        );
        assert_eq!(
            CSignature::parse("s", "long long(int)", WIN).unwrap().ret,
            CType::Int {
                bits: 64,
                signed: true
            },
            "`long long` is 64 everywhere, which is why it is spelled differently"
        );
    }

    #[test]
    fn char_signedness_follows_the_target() {
        let signed = CTarget {
            long_bits: 64,
            char_signed: true,
        };
        let unsigned = CTarget {
            long_bits: 64,
            char_signed: false,
        };
        assert_eq!(
            CSignature::parse("s", "char()", signed).unwrap().ret,
            CType::Int {
                bits: 8,
                signed: true
            }
        );
        assert_eq!(
            CSignature::parse("s", "char()", unsigned).unwrap().ret,
            CType::Int {
                bits: 8,
                signed: false
            },
            "AArch64 — and the difference decides whether a returned byte is negative"
        );
        assert_eq!(
            CSignature::parse("s", "signed char()", unsigned)
                .unwrap()
                .ret,
            CType::Int {
                bits: 8,
                signed: true
            },
            "an explicit spelling overrides the target, as in C"
        );
    }

    #[test]
    fn refuses_what_it_cannot_check() {
        let e = CSignature::parse("s", "struct timeval(int)", LP64).unwrap_err();
        assert!(e.contains("not a C type"), "{e}");
        assert!(
            e.contains("shim"),
            "the message has to say what to do instead: {e}"
        );

        let e = CSignature::parse("s", "int(int, ...)", LP64).unwrap_err();
        assert!(e.contains("variadic"), "{e}");

        let e = CSignature::parse("s", "int", LP64).unwrap_err();
        assert!(e.contains("not a C signature"), "{e}");
    }

    /// A float ARGUMENT is a good C type and a bad binding, so it parses and is
    /// then refused with the cure. The RETURN is neither (@PLN128 arc E): the
    /// register class is one axis there, so it crosses.
    #[test]
    fn a_float_argument_is_refused_with_the_cure_and_a_float_return_is_not() {
        let refusals = sig("double(double)").boundary_refusals();
        assert_eq!(refusals.len(), 1, "the PARAMETER alone: {refusals:?}");
        assert!(refusals[0].contains("parameter 1"), "{:?}", refusals[0]);
        assert!(refusals[0].contains("SSE register"), "{:?}", refusals[0]);
        assert!(refusals[0].contains("POINTER"), "{:?}", refusals[0]);
        // A double IN and a double OUT are not one decision, and this is where
        // the difference shows: the same type refused as a parameter passes as
        // a return.
        assert!(
            sig("double(const double*, int64_t)")
                .boundary_refusals()
                .is_empty()
        );
        assert!(sig("int(int)").boundary_refusals().is_empty());
    }

    fn vec_of(t: Type) -> Type {
        Type::Vector(Box::new(t), crate::data::Deps::none())
    }

    fn text() -> Type {
        Type::Text(crate::data::Deps::none())
    }

    #[test]
    fn arity_counts_c_parameters_not_loft_ones() {
        let ints = vec![vec_of(int())];
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        // A vector is a pointer AND a count where the signature has one...
        assert!(check(&data(), &sig("long(const long*, long)"), &ints, &ret, false).is_empty());
        assert_eq!(
            plan(&ints, &sig("long(const long*, long)")).unwrap(),
            vec![CArg::VectorPtrCount]
        );
        // ...and a BARE pointer where it does not (@PLN128 arc D). This is what
        // makes a Fortran routine bindable: `dgemm_` takes thirteen bare
        // pointers, so a vector that always cost two slots could not reach it
        // at any arity ceiling.
        assert!(check(&data(), &sig("long(const long*)"), &ints, &ret, false).is_empty());
        assert_eq!(
            plan(&ints, &sig("long(const long*)")).unwrap(),
            vec![CArg::VectorPtr]
        );
        // Neither reading can reach three.
        let errs = check(
            &data(),
            &sig("long(const long*, long, long)"),
            &ints,
            &ret,
            false,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("between 1 and 2"), "{}", errs[0]);
    }

    /// The `dgemm_` argument list, which is the case the plan is sized around:
    /// thirteen by-reference arguments, thirteen C slots.
    #[test]
    fn a_fortran_argument_list_costs_one_slot_per_argument() {
        let params = vec![
            text(),
            text(),
            vec_of(int()),
            vec_of(int()),
            vec_of(int()),
            vec_of(Type::Float),
            vec_of(Type::Float),
            vec_of(int()),
            vec_of(Type::Float),
            vec_of(int()),
            vec_of(Type::Float),
            vec_of(Type::Float),
            vec_of(int()),
        ];
        let s = sig(
            "void(const char*, const char*, const int64_t*, const int64_t*, const int64_t*, \
             const double*, const double*, const int64_t*, const double*, const int64_t*, \
             const double*, double*, const int64_t*)",
        );
        let p = plan(&params, &s).expect("a Fortran argument list is bindable");
        assert_eq!(p.len(), 13);
        assert_eq!(p[0], CArg::TextPointer);
        assert!(
            p[2..].iter().all(|a| *a == CArg::VectorPtr),
            "every by-reference argument is a BARE pointer: {p:?}"
        );
        assert_eq!(
            p.iter().map(|a| a.slots()).sum::<usize>(),
            13,
            "thirteen slots, not the twenty-six a mandatory count would cost"
        );
    }

    /// A greedy left-to-right walk gets this wrong: an integer follows the
    /// first vector's pointer, but it is a real argument and the count that IS
    /// present belongs to the second vector.
    #[test]
    fn the_count_is_assigned_by_looking_ahead_not_greedily() {
        let params = vec![vec_of(Type::Float), int(), vec_of(Type::Float)];
        let s = sig("int64_t(const double*, int64_t, const double*, int64_t)");
        assert_eq!(
            plan(&params, &s).unwrap(),
            vec![CArg::VectorPtr, CArg::Scalar, CArg::VectorPtrCount]
        );
    }

    /// Inferring the count from the signature is only safe if the reading is
    /// unique. The argument is written out on `plan`; this searches the small
    /// shapes exhaustively rather than resting on it.
    #[test]
    fn every_reachable_shape_has_at_most_one_reading() {
        let loft_kinds = [text(), int(), vec_of(Type::Float)];
        let c_kinds = [
            CType::Pointer {
                konst: false,
                pointee: "void".to_string(),
                elem: None,
            },
            CType::Int {
                bits: 64,
                signed: true,
            },
        ];
        let mut checked = 0usize;
        // Every loft parameter list up to length 4 against every C parameter
        // list up to length 6 — 3^4 x 2^6 shapes, which covers each way a
        // vector's count can float past a scalar.
        for lp in 1..=4usize {
            for lmask in 0..3usize.pow(u32::try_from(lp).unwrap()) {
                let params: Vec<Type> = (0..lp)
                    .map(|i| loft_kinds[lmask / 3usize.pow(u32::try_from(i).unwrap()) % 3].clone())
                    .collect();
                for cp in 1..=6usize {
                    for cmask in 0..(1usize << cp) {
                        let sig = CSignature {
                            symbol: "s".to_string(),
                            ret: CType::Void,
                            params: (0..cp).map(|i| c_kinds[(cmask >> i) & 1].clone()).collect(),
                        };
                        // Count the readings directly, rather than trusting the
                        // saturating counter `plan` uses internally.
                        let n = readings(&params, &sig, 0);
                        assert!(
                            n <= 1,
                            "{n} readings for {params:?} against {:?}",
                            sig.params
                        );
                        assert_eq!(
                            n == 1,
                            plan(&params, &sig).is_ok(),
                            "`plan` must succeed exactly when a reading exists"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 10_000, "the search must be real: {checked}");
    }

    /// Count every reading, independently of `plan`'s saturating DP — a search
    /// that shares the code it checks proves nothing.
    fn readings(params: &[Type], sig: &CSignature, c: usize) -> usize {
        let Some(first) = params.first() else {
            return usize::from(c == sig.params.len());
        };
        let shape = shape_of(first);
        let mut n = 0;
        for opt in options(&shape, sig, c) {
            n += readings(&params[1..], sig, c + opt.slots());
        }
        n
    }

    #[test]
    fn a_loft_record_is_refused_because_the_store_can_move_it() {
        let params = vec![Type::Reference(0, crate::data::Deps::none())];
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        let errs = check(&data(), &sig("long(void*)"), &params, &ret, false);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("may move them"), "{}", errs[0]);
    }

    /// The handle convention: C returns `PGconn *`, loft holds it as `integer`.
    #[test]
    fn a_returned_pointer_may_be_held_as_an_integer() {
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        assert!(
            check(
                &data(),
                &sig("void*(const char*)"),
                &[Type::Text(crate::data::Deps::none())],
                &ret,
                false
            )
            .is_empty()
        );
    }

    /// The concession, made concrete: `#c` is the declared edge of loft's
    /// no-runtime-errors rule, and this is the compile error that keeps the
    /// guarantee where loft can actually keep it.
    #[test]
    fn a_nullable_value_cannot_cross() {
        let params = vec![Type::Optional(Box::new(int()))];
        let errs = check(&data(), &sig("long(long)"), &params, &int(), false);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("no null model"), "{}", errs[0]);
        assert!(
            errs[0].contains("??"),
            "the message must name the cure: {}",
            errs[0]
        );
        assert!(
            check(&data(), &sig("long(long)"), &[int()], &int(), false).is_empty(),
            "and the non-null sibling is fine — otherwise this proves nothing"
        );
    }

    /// The handle convention has to work in BOTH directions: `lc_open` returns
    /// `void*` into a loft `integer`, and `lc_read` takes that integer back as
    /// `void*`. Allowing only the return half made the fixture's open/read/close
    /// cycle unbindable — half a convention.
    #[test]
    fn a_handle_crosses_in_both_directions() {
        let ret = int();
        assert!(
            check(&data(), &sig("void*(long)"), &[int()], &ret, false).is_empty(),
            "out: a pointer return held as an integer"
        );
        assert!(
            check(&data(), &sig("long(void*)"), &[int()], &ret, false).is_empty(),
            "and back in: that integer passed as the pointer"
        );
    }

    #[test]
    fn a_void_return_and_a_value_return_are_not_interchangeable() {
        let ret = Type::Integer(crate::data::IntegerSpec::wide());
        let errs = check(
            &data(),
            &sig("void(int)"),
            std::slice::from_ref(&ret),
            &ret,
            false,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("returns `void`"), "{}", errs[0]);

        let errs = check(
            &data(),
            &sig("int(int)"),
            std::slice::from_ref(&ret),
            &Type::Void,
            true,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("returns nothing"), "{}", errs[0]);
    }
}
