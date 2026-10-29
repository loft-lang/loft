// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the prebuilt parts loft ships beside its binary

//! Building the components (@PLN205): the shared libraries loft ships beside its binary
//! (`crate::components` finds them).  `libloft_net` is the web library's natives,
//! `src/c/components/net.c`, over the C runtime's own HTTP client, WebSocket code and TLS
//! client — the same text a C unit pastes, so the two cannot drift apart.
//!
//! Called by the `loft-components` tool (`make components`, `make install`) and by the tests
//! that need a component; `loft` itself never builds one.  A component is rebuilt only when its
//! sources, the compiler or the loft-ffi ABI changed: the key of what it was built from sits
//! beside it (`.<file>.key`), and the directory's `.loft-build-fp` is the ABI fingerprint
//! `components::path` checks.

use crate::platform::process::{Program, Spawn};

/// The components loft knows how to build.
pub const NAMES: &[&str] = &["net"];

/// The sources of `libloft_net`, by the flat name each is written under.
const NET: &[(&str, &str)] = &[
    ("net.c", include_str!("components/net.c")),
    ("loft_cc.h", include_str!("loft_cc.h")),
    ("loft_text.h", include_str!("loft_text.h")),
    ("loft_https.h", include_str!("loft_https.h")),
    ("loft_web.h", include_str!("loft_web.h")),
    ("loft_server.h", include_str!("loft_server.h")),
    (
        "loft_ffi.h",
        include_str!("../../loft-ffi/include/loft_ffi.h"),
    ),
];

/// Build component `name` into `out_dir` (or keep the one there when it is current), returning
/// the library's path.
///
/// # Errors
/// A message naming what failed: an unknown component, a host the C runtime has no host layer
/// for, the TLS client's archive, or the compile itself (carrying `cc`'s diagnostics).
pub fn build(name: &str, out_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let sources = match name {
        "net" => NET,
        _ => {
            return Err(format!(
                "no component `{name}` (known: {})",
                NAMES.join(", ")
            ));
        }
    };
    if !crate::platform::c_runtime_posix_host() {
        return Err(format!(
            "component `{name}` needs the C runtime's POSIX host layer, which this host lacks"
        ));
    }
    let tls = super::tls::archive()
        .map_err(|e| format!("component `{name}`: the TLS client did not build: {e}"))?;
    let file = crate::components::file_name(name);
    let lib = out_dir.join(&file);
    let key_file = out_dir.join(format!(".{file}.key"));
    let cc = crate::c_shim::cc_identity();
    let parts: Vec<&[u8]> = sources
        .iter()
        .flat_map(|(n, text)| [n.as_bytes(), text.as_bytes()])
        .chain([cc.as_bytes(), tls.as_os_str().as_encoded_bytes()])
        .collect();
    let key = format!("{:016x}", crate::cache::stable_hash(&parts));
    let fp = crate::cache::loft_ffi_fingerprint();
    if crate::file_access::exists(&lib)
        && crate::file_access::read_to_string(&key_file).is_ok_and(|k| k.trim() == key)
        && crate::cache::native_artifact_fingerprint_matches(out_dir, fp)
    {
        return Ok(lib);
    }
    crate::file_access::create_dir_all(out_dir).map_err(|e| format!("cannot create {e}"))?;
    // Sources and the library are staged in a directory of this process's own and renamed
    // into place, so a concurrent build or load sees a whole library or none.
    let stage = out_dir.join(format!(".stage-{name}.{}", std::process::id()));
    let _ = crate::file_access::remove_dir_all(&stage);
    crate::file_access::create_dir_all(&stage).map_err(|e| format!("cannot create {e}"))?;
    let built = compile(name, sources, &stage, &tls, &file);
    let published = built.and_then(|tmp| {
        crate::file_access::rename(&tmp, &lib)
            .map_err(|e| format!("cannot publish `{}`: {e}", lib.display()))
    });
    let _ = crate::file_access::remove_dir_all(&stage);
    published?;
    crate::file_access::write(&key_file, &key)
        .map_err(|e| format!("cannot write {}: {e}", key_file.display()))?;
    crate::cache::write_native_artifact_fingerprint(out_dir, fp);
    Ok(lib)
}

/// Write `sources` into `stage` and compile the component there, every symbol hidden but the
/// ones its C marks for export (`NET_EXPORT`): the TLS archive's are kept inside it.
fn compile(
    name: &str,
    sources: &[(&str, &str)],
    stage: &std::path::Path,
    tls: &std::path::Path,
    file: &str,
) -> Result<std::path::PathBuf, String> {
    for (n, text) in sources {
        let path = stage.join(n);
        crate::file_access::write(&path, text)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let cc = crate::c_shim::cc_program();
    let tmp = stage.join(file);
    let mut cmd = Spawn::new(Program::os(&cc));
    cmd.push_arg("-O2")
        .push_arg("-fPIC")
        .push_arg("-shared")
        .push_arg("-fvisibility=hidden")
        .push_arg("-pthread")
        .push_arg("-I")
        .push_arg(stage)
        .push_arg("-o")
        .push_arg(&tmp);
    for arg in crate::platform::install_name_args(file, crate::platform::host_lib_os()) {
        cmd.push_arg(arg);
    }
    cmd.push_arg(stage.join(sources[0].0)).push_arg(tls);
    // The archive's own symbols stay inside the library: GNU ld drops them from the export
    // table, and on macOS only the loft-ffi names are exported.
    if crate::platform::host_lib_os() == crate::platform::LibOs::Macos {
        cmd.push_arg("-Wl,-exported_symbol,_n_*")
            .push_arg("-Wl,-exported_symbol,_loft_register_*");
    } else {
        cmd.push_arg("-Wl,--exclude-libs,ALL");
    }
    let out = cmd.run(b"").map_err(|e| {
        format!("component `{name}` needs a C compiler and `{cc}` could not be run ({e})")
    })?;
    if !out.status.success() {
        return Err(format!(
            "component `{name}` did not compile.\n{}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    Ok(tmp)
}
