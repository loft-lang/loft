// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The emulated Windows host (@PLN184 Track W, `LOFT_POISON_HOST=windows`): Windows' naming
//! rules applied over the real Linux or macOS file system, so a defect that only Windows shows
//! shows in the normal test run.  Every rule is Microsoft's ("Naming Files, Paths, and
//! Namespaces"), as `std` reaches it through the Win32 path functions:
//! - a name holding `< > " | ? *` or a control character is refused (`ERROR_INVALID_NAME`);
//! - a trailing dot or space is stripped from every name;
//! - a device name (`CON PRN AUX NUL COM1–9 LPT1–9`, any case, with or without an extension)
//!   is refused — real Windows opens the device instead, which differs between versions and
//!   is not reproduced; the daily comparison (W2) measures it;
//! - `name:stream` (the last name only) is an alternate data stream: the data goes to the
//!   stream, the file `name` exists and is empty, and a listing does not show the stream;
//! - a name matches an existing entry ignoring case, so creating `B.txt` beside `b.txt` opens
//!   `b.txt`.
//!
//! Not reproduced: `MAX_PATH`.  `std` on Windows hands a long path to the OS in the verbatim
//! form, so a file operation does not meet the limit; a spawned tool does (W1.1).

// @PLN184 A1: this module IS the one way to the file system.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use super::{Flavor, PathText};
use std::io;
use std::path::{Path, PathBuf};

/// The real path an emulated-Windows path names, or Windows' refusal.
pub(super) fn os(path: &PathText) -> io::Result<PathBuf> {
    let real = path
        .from_emulated()
        .map_err(|why| io::Error::new(io::ErrorKind::NotFound, why))?;
    let parts = real.parts();
    let mut out = PathBuf::from(if real.is_absolute() { "/" } else { "" });
    for (i, name) in parts.iter().enumerate() {
        if name == ".." {
            out.push(name);
            continue;
        }
        // A name the OS handed over in its own spelling reaches that entry as it is.
        if let Some(raw) = real.raw_at(i) {
            out.push(raw);
            continue;
        }
        let name = windows_name(name, i + 1 == parts.len())
            .map_err(|why| io::Error::new(io::ErrorKind::InvalidFilename, why))?;
        let resolved = match name.split_once(':') {
            Some((base, stream)) => format!("{}:{stream}", match_case(&out, base)),
            None => match_case(&out, &name),
        };
        out.push(resolved);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Ok(out)
}

/// `name` as Windows stores it — trailing dots and spaces stripped — or why it refuses it.
fn windows_name(name: &str, last: bool) -> Result<String, String> {
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || "<>\"|?*".contains(*c))
    {
        return Err(format!(
            "`{name}` holds {c:?}, which Windows refuses in a name"
        ));
    }
    if let Some((base, stream)) = name.split_once(':') {
        if !last || stream.contains(':') {
            return Err(format!("`{name}`: only a file's last name names a stream"));
        }
        return Ok(format!("{}:{stream}", windows_name(base, false)?));
    }
    let stripped = name.trim_end_matches(['.', ' ']);
    if stripped.is_empty() {
        return Err(format!(
            "`{name}` is empty once Windows strips its trailing dots"
        ));
    }
    let stem = stripped
        .split('.')
        .next()
        .unwrap_or(stripped)
        .trim_end()
        .to_ascii_uppercase();
    let numbered = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered {
        return Err(format!("`{name}` is a device on Windows"));
    }
    Ok(stripped.to_string())
}

/// The spelling of `name` in `dir` on disk: itself when it exists, else the first entry equal
/// to it ignoring case, else itself (a name being created).
fn match_case(dir: &Path, name: &str) -> String {
    let listed = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    if dir.join(name).symlink_metadata().is_ok() {
        return name.to_string();
    }
    let Ok(entries) = std::fs::read_dir(listed) else {
        return name.to_string();
    };
    let want = name.to_uppercase();
    let mut matches: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|e| !e.contains(':') && e.to_uppercase() == want)
        .collect();
    matches.sort();
    matches
        .into_iter()
        .next()
        .unwrap_or_else(|| name.to_string())
}

/// Is a real directory entry an emulated stream (`name:stream`), which a Windows listing
/// does not show?
pub(super) fn is_stream(entry: &Path) -> bool {
    entry
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains(':'))
}

/// After `real` was opened for writing: a stream's file exists, empty when it is new — the
/// silent alternate data stream a Windows program writes with `a:b.txt`.
pub(super) fn stream_base(path: &PathText, real: &Path) -> io::Result<()> {
    if path.flavor() == Flavor::HOST || !is_stream(real) {
        return Ok(());
    }
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let base = real.with_file_name(name.split(':').next().unwrap_or(name));
    if base.symlink_metadata().is_err() {
        std::fs::write(base, "")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{
        create_dir_all, exists, read_dir, read_to_string, remove_dir_all, with_program_host, write,
    };
    use super::*;

    /// A scratch directory in the program's spelling, and the names listed in it.
    fn scratch(tag: &str) -> PathText {
        let dir = PathText::from_os(&std::env::temp_dir())
            .join(&format!("loft_emulated_{tag}_{}", std::process::id()));
        let _ = remove_dir_all(&dir);
        create_dir_all(&dir).expect("scratch dir");
        dir.for_program()
    }

    fn names(dir: &PathText) -> Vec<String> {
        read_dir(dir)
            .unwrap()
            .iter()
            .filter_map(|p| p.file_name().map(str::to_string))
            .collect()
    }

    #[test]
    fn a_reserved_character_is_refused() {
        with_program_host(Flavor::Windows, || {
            let d = scratch("chars");
            for bad in ["q?.txt", "a<b", "p|q", "x*", "say\"hi\""] {
                let e = write(d.join(bad), "x").unwrap_err();
                assert_eq!(e.kind(), io::ErrorKind::InvalidFilename, "{bad}: {e}");
            }
            assert_eq!(names(&d), Vec::<String>::new());
            remove_dir_all(&d).unwrap();
        });
    }

    #[test]
    fn a_trailing_dot_or_space_is_stripped() {
        with_program_host(Flavor::Windows, || {
            let d = scratch("trail");
            write(d.join("trail."), "t").unwrap();
            write(d.join("space "), "s").unwrap();
            assert_eq!(names(&d), ["space", "trail"]);
            assert_eq!(read_to_string(d.join("trail")).unwrap(), "t");
            remove_dir_all(&d).unwrap();
        });
    }

    #[test]
    fn a_device_name_is_refused() {
        with_program_host(Flavor::Windows, || {
            if !Flavor::emulating() {
                return; // real Windows opens the device: measured by W2, not here
            }
            let d = scratch("device");
            for bad in ["aux.txt", "CON", "nul.tar.gz", "com1", "LPT9.log", "prn .x"] {
                assert!(write(d.join(bad), "x").is_err(), "{bad}");
            }
            for good in ["com0", "console.txt", "auxiliary"] {
                write(d.join(good), "x").unwrap();
            }
            assert_eq!(names(&d), ["auxiliary", "com0", "console.txt"]);
            remove_dir_all(&d).unwrap();
        });
    }

    #[test]
    fn a_colon_writes_a_silent_stream() {
        with_program_host(Flavor::Windows, || {
            let d = scratch("stream");
            // Spelled inside the path, as a program writes it: a LEADING `a:` is a drive, to
            // `join` here as to `std` on Windows.
            let inside =
                |rel: &str| PathText::parse(&format!("{}/{rel}", d.native()), Flavor::Windows);
            write(inside("a:b.txt"), "data").unwrap();
            assert_eq!(names(&d), ["a"], "the stream is not listed");
            assert_eq!(
                read_to_string(d.join("a")).unwrap(),
                "",
                "the file is empty"
            );
            assert_eq!(read_to_string(inside("a:b.txt")).unwrap(), "data");
            assert!(
                write(inside("x:y/z"), "x").is_err(),
                "a stream is a last name"
            );
            remove_dir_all(&d).unwrap();
        });
    }

    #[test]
    fn a_name_matches_ignoring_case() {
        with_program_host(Flavor::Windows, || {
            let d = scratch("case");
            write(d.join("b.txt"), "lower").unwrap();
            assert!(exists(d.join("B.TXT")));
            write(d.join("B.txt"), "upper").unwrap();
            assert_eq!(names(&d), ["b.txt"], "one file");
            assert_eq!(read_to_string(d.join("b.txt")).unwrap(), "upper");
            create_dir_all(d.join("Sub")).unwrap();
            write(d.join("SUB/x.txt"), "x").unwrap();
            assert_eq!(names(&d.join("Sub")), ["x.txt"], "a directory matches too");
            remove_dir_all(&d).unwrap();
        });
    }

    /// The control: without the switch a Unix host keeps every one of those names apart.
    #[test]
    fn off_the_switch_a_unix_host_keeps_its_own_rules() {
        if Flavor::HOST == Flavor::Windows {
            return; // the host IS Windows: the rules above are its own
        }
        let d = scratch("off");
        for n in ["q?.txt", "trail.", "aux.txt", "a:b.txt", "b.txt", "B.txt"] {
            write(d.join(n), n).unwrap();
        }
        assert_eq!(names(&d).len(), 6);
        remove_dir_all(&d).unwrap();
    }
}
