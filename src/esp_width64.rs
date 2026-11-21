// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! @PLN217 — the 64-bit gate of a 32-bit device image (`--native-esp --width 32`).
//!
//! The ESP32-P4 has 32-bit registers and a single-precision FPU: a `double` operation and a
//! 64-bit multiply, divide or shift are calls into libgcc.  A 32-bit build is meant to make
//! none outside what the source declares `i64` / `f64`, so the linked image is read for those
//! calls and one that makes them where nothing allows it is refused, naming each function.
//!
//! What is judged is loft's side of the image: the program, loft's runtime and the C of the
//! libraries the program uses.  ESP-IDF's own archives are not loft's to change.
//!
//! An exception is one line in an allow file — a function, or a prefix ending in `*`, and the
//! reason 64 bits are needed there:
//!
//! ```text
//! loft_format_float    a float is rendered through its shortest double
//! n_cell_key           the cell key is a 64-bit hash
//! ```
//!
//! Three files are read: loft's own for its runtime (`src/c/esp_width64.allow`), the
//! program's (`esp-width64.allow` beside its entry file) and one per library that ships a C
//! shim (`esp-width64.allow` in the package).

use std::collections::{BTreeMap, HashMap};

/// A function of the image that calls a 64-bit helper: where it came from and which helpers,
/// with how often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wide {
    pub function: String,
    pub origin: String,
    pub helpers: BTreeMap<String, u32>,
}

/// One exception: a function name, or a prefix when `prefix`, and why it needs 64 bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Allow {
    pub name: String,
    pub prefix: bool,
    pub reason: String,
    pub file: String,
}

impl Allow {
    fn covers(&self, function: &str) -> bool {
        if self.prefix {
            return function.starts_with(&self.name);
        }
        // A loft function is written to C under its name and under its twins' (`n_f__fr`,
        // `n_f__inv`): one exception names them all.
        function == self.name
            || function
                .strip_prefix(self.name.as_str())
                .is_some_and(|rest| rest.starts_with("__"))
    }
}

/// Is `name` a libgcc routine for `double` arithmetic or for a 64-bit integer operation the
/// core has no instruction for?  Answers which: `"double"` or `"64-bit integer"`.
pub(crate) fn helper_kind(name: &str) -> Option<&'static str> {
    let body = name.strip_prefix("__")?;
    let strip_digit = |s: &str| -> Option<String> {
        let last = s.chars().last()?;
        matches!(last, '2' | '3').then(|| s[..s.len() - 1].to_string())
    };
    if body == "powidf2" {
        return Some("double");
    }
    // A conversion to or from a double: __extendsfdf2, __truncdfsf2, __fixdfsi, __fixunsdfdi,
    // __floatsidf, __floatundidf.
    for (pre, suf) in [("extend", "df2"), ("trunc", "sf2")] {
        if let Some(mid) = body.strip_prefix(pre).and_then(|r| r.strip_suffix(suf))
            && matches!(mid, "sf" | "df")
            && (pre, mid) != ("extend", "df")
            && (pre, mid) != ("trunc", "sf")
        {
            return Some("double");
        }
    }
    if let Some(rest) = body.strip_prefix("fix") {
        let rest = rest.strip_prefix("uns").unwrap_or(rest);
        if matches!(rest, "dfsi" | "dfdi") {
            return Some("double");
        }
    }
    if let Some(rest) = body.strip_prefix("float") {
        let rest = rest.strip_prefix("un").unwrap_or(rest);
        if matches!(rest, "sidf" | "didf") {
            return Some("double");
        }
    }
    let stem = strip_digit(body)?;
    if let Some(op) = stem.strip_suffix("df")
        && matches!(
            op,
            "add"
                | "sub"
                | "mul"
                | "div"
                | "neg"
                | "eq"
                | "ne"
                | "lt"
                | "le"
                | "gt"
                | "ge"
                | "unord"
                | "cmp"
        )
    {
        return Some("double");
    }
    if let Some(op) = stem.strip_suffix("di")
        && matches!(
            op,
            "div"
                | "mod"
                | "udiv"
                | "umod"
                | "mul"
                | "ashl"
                | "ashr"
                | "lshr"
                | "neg"
                | "cmp"
                | "ucmp"
                | "clz"
                | "ctz"
                | "popcount"
                | "bswap"
        )
    {
        return Some("64-bit integer");
    }
    None
}

/// The function each code section of the linker's map belongs to, by where its object came
/// from (`origin`, the answer of `esp_app::origin`).
pub(crate) fn function_origins(
    map: &str,
    origin: &dyn Fn(&str, &str) -> String,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let lines: Vec<&str> = map.lines().collect();
    for (k, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix(' ') else {
            continue;
        };
        if !rest.starts_with('.') {
            continue;
        }
        let mut words: Vec<&str> = rest.split_whitespace().collect();
        if words.len() == 1 {
            words.extend(
                lines
                    .get(k + 1)
                    .map_or(Vec::new(), |n| n.split_whitespace().collect()),
            );
        }
        let [section, _addr, _size, object, ..] = words.as_slice() else {
            continue;
        };
        let name = [".text.", ".iram1.", ".flash.text."]
            .iter()
            .find_map(|p| section.strip_prefix(p));
        if let Some(name) = name {
            out.insert(source_name(name).to_string(), origin(object, section));
        }
    }
    out
}

/// A function's name as the source wrote it: gcc names a specialised copy `f.isra.0`,
/// `f.constprop.1`, `f.part.2`, `f.cold`.
pub(crate) fn source_name(symbol: &str) -> &str {
    symbol.split_once('.').map_or(symbol, |(name, _)| name)
}

/// The functions of a disassembly (`objdump -d --no-show-raw-insn`) that call a 64-bit helper,
/// in the order they stand.  The helpers themselves are not listed.
pub(crate) fn wide_functions(disasm: &str, origins: &HashMap<String, String>) -> Vec<Wide> {
    let mut out: Vec<Wide> = Vec::new();
    let mut current: Option<Wide> = None;
    let close = |w: Option<Wide>, out: &mut Vec<Wide>| {
        if let Some(w) = w
            && !w.helpers.is_empty()
            && helper_kind(&w.function).is_none()
        {
            out.push(w);
        }
    };
    for line in disasm.lines() {
        // `4200a1c2 <n_main>:` opens a function.
        if let Some(name) = line
            .strip_suffix(">:")
            .and_then(|l| l.split_once(" <"))
            .filter(|(addr, _)| !addr.is_empty() && addr.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|(_, n)| n)
        {
            close(current.take(), &mut out);
            let name = source_name(name);
            current = Some(Wide {
                function: name.to_string(),
                origin: origins.get(name).cloned().unwrap_or_default(),
                helpers: BTreeMap::new(),
            });
            continue;
        }
        let Some(w) = current.as_mut() else {
            continue;
        };
        // `  4200a1d0:  jal  ra,4201f3a8 <__muldi3>`, or a jump through a register to a ROM
        // routine, which objdump names in a comment: `jalr 244(ra) # 4fc00868 <__adddf3>`.
        let Some((_, insn)) = line.split_once(":\t").or_else(|| line.split_once(": ")) else {
            continue;
        };
        let op = insn.split_whitespace().next().unwrap_or("");
        if !matches!(
            op,
            "jal" | "jalr" | "call" | "tail" | "j" | "c.jal" | "c.j" | "c.jalr"
        ) {
            continue;
        }
        let Some(callee) = insn
            .rsplit_once('<')
            .and_then(|(_, r)| r.strip_suffix('>'))
            .filter(|c| !c.contains('+'))
        else {
            continue;
        };
        if helper_kind(callee).is_some() {
            *w.helpers.entry(callee.to_string()).or_insert(0) += 1;
        }
    }
    close(current, &mut out);
    out
}

/// The exceptions of one allow file (`file` names it in a message).  A line is a function
/// name, or a prefix ending in `*`, then the reason; `#` starts a comment.
///
/// # Errors
/// An entry with no reason: an exception says why it is one.
pub(crate) fn parse_allow(text: &str, file: &str) -> Result<Vec<Allow>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (name, reason) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(a, b)| (a, b.trim()));
        if reason.is_empty() {
            return Err(format!(
                "{file}:{}: `{name}` is allowed 64-bit arithmetic without a reason — write why \
                 it needs 64 bits after the name",
                n + 1
            ));
        }
        let (name, prefix) = name.strip_suffix('*').map_or((name, false), |p| (p, true));
        out.push(Allow {
            name: name.to_string(),
            prefix,
            reason: reason.to_string(),
            file: file.to_string(),
        });
    }
    Ok(out)
}

/// Is a function from `origin` loft's side of the image — the program, loft's runtime, a
/// library's C — and so judged?  An ESP-IDF archive (`libfreertos.a`) is not.
pub(crate) fn judged(origin: &str) -> bool {
    // An archive is named `lib<x>.a` by the linker on every host: the comparison is exact.
    !origin.is_empty() && origin.rsplit_once('.').is_none_or(|(_, ext)| ext != "a")
}

/// What the gate found: the functions it refuses, the ones an exception allows (with the
/// exception), and the exceptions that allow nothing any more.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub refused: Vec<Wide>,
    pub allowed: Vec<(Wide, Allow)>,
    pub unused: Vec<Allow>,
}

/// Each judged function of `wide` against the exceptions.
pub(crate) fn judge(wide: &[Wide], allow: &[Allow]) -> Verdict {
    let mut v = Verdict::default();
    let mut used = vec![false; allow.len()];
    for w in wide.iter().filter(|w| judged(&w.origin)) {
        match allow.iter().position(|a| a.covers(&w.function)) {
            Some(i) => {
                used[i] = true;
                v.allowed.push((w.clone(), allow[i].clone()));
            }
            None => v.refused.push(w.clone()),
        }
    }
    v.unused = allow
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(a, _)| a.clone())
        .collect();
    v
}

fn helpers_text(w: &Wide) -> String {
    w.helpers
        .iter()
        .map(|(h, n)| {
            if *n == 1 {
                h.clone()
            } else {
                format!("{h} x{n}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The refusal a build stops with: each function, where it came from and the helpers it
/// calls, then the two cures.  `own_allow` is the file a program's exception goes in.
pub(crate) fn refusal(refused: &[Wide], own_allow: &str) -> String {
    let mut text = format!(
        "--native-esp: this 32-bit app does 64-bit arithmetic in {} function{} — the \
         ESP32-P4 runs a `double` and a 64-bit multiply, divide or shift as library calls:\n",
        refused.len(),
        if refused.len() == 1 { "" } else { "s" }
    );
    for w in refused {
        text.push_str(&format!(
            "  {} ({}) calls {}\n",
            w.function,
            w.origin,
            helpers_text(w)
        ));
    }
    text.push_str(&format!(
        "Where 32 bits are enough, declare the value `integer` or `single` (not `i64`, `f64` or a \
         literal ending in `l` or `d`).  Where 64 bits are needed, allow the function by a line \
         in {own_allow}: its name, then the reason."
    ));
    text
}

/// The part of the build's report that lists what an exception allowed, and the exceptions
/// that allow nothing any more.
pub(crate) fn report(v: &Verdict) -> String {
    let mut text = String::from("\n64-bit arithmetic in this 32-bit app\n");
    if v.allowed.is_empty() {
        text.push_str("  none\n");
    }
    for (w, a) in &v.allowed {
        text.push_str(&format!(
            "  {} ({}) calls {} — allowed: {}\n",
            w.function,
            w.origin,
            helpers_text(w),
            a.reason
        ));
    }
    for a in &v.unused {
        text.push_str(&format!(
            "  an exception that allows nothing now, to remove from {}: {}{}\n",
            a.file,
            a.name,
            if a.prefix { "*" } else { "" }
        ));
    }
    text
}

/// loft's own exceptions, for its runtime.
const RUNTIME_ALLOW: &str = include_str!("c/esp_width64.allow");

const RUNTIME_ALLOW_NAME: &str = "loft's src/c/esp_width64.allow";

/// The name of the allow file a program or a library keeps.
pub(crate) const ALLOW_FILE: &str = "esp-width64.allow";

/// Judge a 32-bit image: `disasm` is its disassembly, `map` the linker's map, `origin` names
/// where a section's object came from, and `allow_files` are the program's and the
/// libraries' allow files that exist, as (name, text).  Answers the report's part, or the
/// refusal the build stops with.
///
/// # Errors
/// An allow file with an entry that gives no reason, or an image that does 64-bit arithmetic
/// where no exception allows it.
pub(crate) fn gate(
    disasm: &str,
    map: &str,
    origin: &dyn Fn(&str, &str) -> String,
    allow_files: &[(String, String)],
    own_allow: &str,
) -> Result<String, String> {
    let mut allow = parse_allow(RUNTIME_ALLOW, RUNTIME_ALLOW_NAME)?;
    for (name, text) in allow_files {
        allow.extend(parse_allow(text, name)?);
    }
    let wide = wide_functions(disasm, &function_origins(map, origin));
    let mut verdict = judge(&wide, &allow);
    // The runtime's exceptions cover every app: one that does not link a function leaves its
    // line unused, and that is no stale line.
    verdict.unused.retain(|a| a.file != RUNTIME_ALLOW_NAME);
    if verdict.refused.is_empty() {
        Ok(report(&verdict))
    } else {
        Err(refusal(&verdict.refused, own_allow))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISASM: &str = "\
4200a000 <n_cell_key>:
4200a000:\taddi\tsp,sp,-16
4200a004:\tjal\tra,4201f3a8 <__muldi3>
4200a008:\tjal\tra,4201f3a8 <__muldi3>
4200a00c:\tret

4200a010 <n_draw>:
4200a010:\tjal\tra,4200a000 <n_cell_key>
4200a014:\tret

4200a018 <loft_format_float>:
4200a018:\tjalr\t244(ra) # 4fc00868 <__adddf3>
4200a01c:\tjal\tra,4201f000 <__extendsfdf2>
4200a020:\tj\t4200a018 <loft_format_float+0x0>
4200a024:\tret

4200a026 <loft_h_ensure_table.isra.0>:
4200a026:\tjal\tra,4201f100 <__udivdi3>

4200a028 <vTaskDelay>:
4200a028:\tjal\tra,4201f100 <__udivdi3>

4201f3a8 <__muldi3>:
4201f3a8:\tjal\tra,4201f200 <__ashldi3>
";

    fn origins() -> HashMap<String, String> {
        [
            ("n_cell_key", "the program (loft_app.c)"),
            ("n_draw", "the program (loft_app.c)"),
            ("loft_format_float", "loft runtime (loft_app.c)"),
            ("loft_h_ensure_table", "loft runtime (loft_app.c)"),
            ("vTaskDelay", "libfreertos.a"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    /// Three functions call a helper; the one that calls only another function does not, and a
    /// helper calling a helper is not listed.  The register jump to a ROM routine counts.
    #[test]
    fn the_functions_that_call_a_64_bit_helper_are_found() {
        let wide = wide_functions(DISASM, &origins());
        let names: Vec<&str> = wide.iter().map(|w| w.function.as_str()).collect();
        assert_eq!(
            names,
            [
                "n_cell_key",
                "loft_format_float",
                "loft_h_ensure_table",
                "vTaskDelay"
            ]
        );
        assert_eq!(wide[0].helpers.get("__muldi3"), Some(&2));
        assert_eq!(
            wide[1].helpers.keys().collect::<Vec<_>>(),
            ["__adddf3", "__extendsfdf2"]
        );
    }

    /// With no exception both of loft's functions are refused and ESP-IDF's is not judged; an
    /// exact name and a prefix each allow one; an exception that matches nothing is reported.
    #[test]
    fn an_exception_allows_its_function_and_nothing_else() {
        let wide = wide_functions(DISASM, &origins());
        let none = judge(&wide, &[]);
        let refused: Vec<&str> = none.refused.iter().map(|w| w.function.as_str()).collect();
        assert_eq!(
            refused,
            ["n_cell_key", "loft_format_float", "loft_h_ensure_table"]
        );

        let allow = parse_allow(
            "# the program\nn_cell_key  the cell key is a 64-bit hash\nloft_format_*  a float is \
             rendered through a double\nloft_h_ensure_table  sized in 64 bits\nn_gone  it was \
             needed once\n",
            "esp-width64.allow",
        )
        .unwrap();
        let v = judge(&wide, &allow);
        assert_eq!(v.refused, Vec::<Wide>::new());
        assert_eq!(v.allowed.len(), 3);
        assert_eq!(v.unused.len(), 1);
        assert_eq!(v.unused[0].name, "n_gone");

        let one = judge(&wide, &[allow[0].clone(), allow[2].clone()]);
        assert_eq!(one.refused.len(), 1);
        let said = refusal(&one.refused, "esp-width64.allow");
        assert!(said.contains(
            "loft_format_float (loft runtime (loft_app.c)) calls __adddf3, __extendsfdf2"
        ));
        assert!(said.contains("in 1 function "));
    }

    /// A loft function's exception covers the twins it is written to C under.
    #[test]
    fn an_exception_covers_a_functions_twins() {
        let a = &parse_allow("n_words_from  a JSON number is a double\n", "x.allow").unwrap()[0];
        assert!(a.covers("n_words_from"));
        assert!(a.covers("n_words_from__fr"));
        assert!(a.covers("n_words_from__inv__fr"));
        assert!(!a.covers("n_words_from_json"));
    }

    /// An exception says why: a bare name is refused with its line.
    #[test]
    fn an_exception_without_a_reason_is_refused() {
        let err = parse_allow("n_a  a hash\nn_b\n", "x.allow").unwrap_err();
        assert!(err.starts_with("x.allow:2: `n_b`"), "{err}");
    }

    #[test]
    fn the_helpers_are_told_from_other_functions() {
        for h in [
            "__adddf3",
            "__muldf3",
            "__divdf3",
            "__ltdf2",
            "__extendsfdf2",
            "__truncdfsf2",
            "__fixdfsi",
            "__fixunsdfdi",
            "__floatsidf",
            "__floatundidf",
            "__powidf2",
        ] {
            assert_eq!(helper_kind(h), Some("double"), "{h}");
        }
        for h in [
            "__muldi3",
            "__divdi3",
            "__udivdi3",
            "__umoddi3",
            "__ashldi3",
            "__clzdi2",
        ] {
            assert_eq!(helper_kind(h), Some("64-bit integer"), "{h}");
        }
        for h in [
            "__addsf3",
            "__mulsi3",
            "__divsi3",
            "__fixsfsi",
            "n_muldi3",
            "memcpy",
        ] {
            assert_eq!(helper_kind(h), None, "{h}");
        }
    }
}
