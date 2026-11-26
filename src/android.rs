//! Android build target (`--native-android`) — @PLN106 (B1 + B2).
//!
//! @I68 — Native Rust generator (a build target over the shared native core).
//!
//! loft's native backend emits ONE target-agnostic Rust core and compiles it per
//! target; a build target is just a *descriptor* — `(rust triple, linker, runtime
//! entry, packaging)` — over that shared core (B0 confirmed the core cross-compiles
//! to `aarch64-linux-android` unchanged; see `106-android-build-target/README.md`).
//! This module is that descriptor for Android: it holds ALL Android-specific
//! knowledge in one place so adding the target is data, not `if target == android`
//! branches scattered through the build. Public entry: [`AndroidTarget::detect`] +
//! [`AndroidTarget::build`].
//!
//! **Runtime entry (B2).** An Android app has no `fn main`; it is a `NativeActivity`
//! whose `.so` exports `ANativeActivity_onCreate`, which calls a Rust `android_main`.
//! So `--native-android` wraps the emitted program in a tiny generated cargo crate:
//! the program becomes the crate root (its `fn main` untouched) and a fixed
//! [`ANDROID_MAIN_TAIL`] adds the `android_main` entry (via the `android-activity`
//! crate) that runs `main()` and forwards its stdout into logcat. Cargo builds the
//! crate, resolving `android-activity`'s dep tree and cross-building loft (a lean
//! path dep) for the triple; the resulting `.so` is dropped in `lib/<abi>/` of an
//! APK (the `b2-spike/` recipe; loft-side APK packaging is the next slice).
//!
//! There is NO Android codegen path: the program `.rs` is the same
//! `output_native_reachable` emit `--native` / `--native-wasm` produce — only the
//! entry tail and the toolchain differ. Requires the Android NDK (r26+): point
//! `ANDROID_NDK_HOME` (or `ANDROID_NDK_ROOT`) at it. Build the `x86_64-linux-android`
//! triple (`LOFT_ANDROID_TARGET`) for a KVM emulator; `aarch64` is the ship target.

use crate::file_access::PathText;
use crate::platform::process::{Program, Spawn};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The oldest NDK release `--native-android` is built and tested with: its `clang` wrappers and
/// its `native_app_glue` (thirdparty.toml `android-ndk` pins this).
pub(crate) const NDK_MIN: &str = "r26";

/// How an activity library is packaged: where the APK is staged, where the debug keystore
/// lives (it stays, so every later build of the app signs with the same key), how the APK is
/// signed ([`signing`]), whether the app is debuggable (a build that is not `--ship`), the
/// program's files it carries ([`app_assets`]: each path relative to the program, and the file
/// on this machine), the libraries it ships beside its own (the components the C binds,
/// `c_driver::build_android`), and whether it asks for the network ([`needs_internet`]).
pub(crate) struct Package<'a> {
    pub(crate) staging: &'a Path,
    pub(crate) keystore_dir: &'a Path,
    pub(crate) signing: &'a Signing,
    pub(crate) debuggable: bool,
    pub(crate) assets: &'a [(String, std::path::PathBuf)],
    pub(crate) libs: &'a [std::path::PathBuf],
    pub(crate) internet: bool,
}

/// How a packaged APK is signed.  A debug build signs with the per-tree debug key, whose
/// password is the public `android`; a `--ship` build never does: it signs with the key the
/// program's `loft.toml` names, or is written unsigned with the command that signs it.
pub(crate) enum Signing {
    /// A build that is not `--ship`: the debug key in [`Package::keystore_dir`].
    Debug,
    /// A `--ship` build whose `loft.toml` names its key in `[android]`.
    Release(ReleaseKey),
    /// A `--ship` build with no key named: the APK is aligned, not signed, and the
    /// `apksigner` command that signs it is printed.
    Unsigned,
}

/// The release key from a program's `[android]` table.  The passwords are apksigner's own
/// `--ks-pass` / `--key-pass` values (`env:NAME`, `file:PATH`, `stdin`): loft hands them on
/// and never reads the secret.
#[derive(Debug)]
pub(crate) struct ReleaseKey {
    /// The directory of the `loft.toml` that names the key.  apksigner runs there, so a
    /// `file:` password resolves against the same directory the keystore does.
    dir: PathBuf,
    keystore: PathBuf,
    alias: String,
    ks_pass: String,
    key_pass: Option<String>,
}

/// How a `--native-android` build of `program` is signed: [`Signing::Debug`] unless `ship`;
/// with `ship`, the key the nearest `loft.toml`'s `[android]` table names, else
/// [`Signing::Unsigned`].  The table is checked on every build, so a password written into
/// the manifest is refused before a debug build goes on to ship.
pub(crate) fn signing(ship: bool, program: &Path) -> Result<Signing, String> {
    let Some(root) = loft::resolution_scope::project_root(&program.to_string_lossy()) else {
        return Ok(if ship {
            Signing::Unsigned
        } else {
            Signing::Debug
        });
    };
    let manifest = root.join("loft.toml");
    let table = loft::manifest::read_manifest(&manifest.to_string_lossy())
        .map(|m| m.android)
        .unwrap_or_default();
    let key = release_key(&table, &root, &manifest)?;
    Ok(match key {
        _ if !ship => Signing::Debug,
        None => Signing::Unsigned,
        Some(key) => {
            if !crate::file_access::is_file(&key.keystore) {
                return Err(format!(
                    "loft: the keystore {} that `[android] keystore` in {} names is not there",
                    key.keystore.display(),
                    manifest.display()
                ));
            }
            Signing::Release(key)
        }
    })
}

/// The release key `table` names, its keystore resolved against `root` (the directory of
/// `manifest`), or `None` when it names none.  A password must say where apksigner reads it;
/// one written into the manifest (`pass:`) is refused.
fn release_key(
    table: &loft::manifest::AndroidSigning,
    root: &Path,
    manifest: &Path,
) -> Result<Option<ReleaseKey>, String> {
    let ks_pass = table
        .ks_pass
        .as_deref()
        .map(|v| password("ks_pass", v, manifest))
        .transpose()?;
    let key_pass = table
        .key_pass
        .as_deref()
        .map(|v| password("key_pass", v, manifest))
        .transpose()?;
    let Some(keystore) = &table.keystore else {
        if table.key_alias.is_some() || ks_pass.is_some() || key_pass.is_some() {
            return Err(format!(
                "loft: `[android]` in {} names no `keystore`; add `keystore = \"<file>.jks\"`, \
                 relative to that loft.toml",
                manifest.display()
            ));
        }
        return Ok(None);
    };
    let missing = |key: &str, example: &str| {
        format!(
            "loft: `[android]` in {} names a keystore but no `{key}`; add `{key} = \"{example}\"`",
            manifest.display()
        )
    };
    let alias = table
        .key_alias
        .clone()
        .ok_or_else(|| missing("key_alias", "release"))?;
    let ks_pass = ks_pass.ok_or_else(|| missing("ks_pass", "env:ANDROID_KS_PASS"))?;
    Ok(Some(ReleaseKey {
        dir: root.to_path_buf(),
        // In the host's spelling: `keys/release.jks` as written, joined part by part, so on
        // Windows apksigner is handed one separator throughout.
        keystore: {
            use crate::file_access::HostPath as _;
            root.to_path_text().join(keystore).os_spelling()
        },
        alias,
        ks_pass,
        key_pass,
    }))
}

/// A password value from `[android]` in `manifest`, as apksigner reads it: `env:NAME`,
/// `file:PATH` or `stdin`.  `pass:<password>` is refused: written into a manifest the
/// password goes wherever the manifest goes, and with it the power to sign as the app.
fn password(field: &str, value: &str, manifest: &Path) -> Result<String, String> {
    if value.starts_with("pass:") {
        return Err(format!(
            "loft: `[android] {field}` in {} holds the password itself (`pass:`), and a \
             password in a manifest goes wherever the manifest goes; name where apksigner \
             reads it instead: `env:NAME`, `file:PATH` or `stdin`",
            manifest.display()
        ));
    }
    let named = |prefix: &str| value.strip_prefix(prefix).is_some_and(|r| !r.is_empty());
    if value == "stdin" || named("env:") || named("file:") {
        Ok(value.to_string())
    } else {
        Err(format!(
            "loft: `[android] {field}` in {} is `{value}`; apksigner reads a password from \
             `env:NAME`, `file:PATH` or `stdin`",
            manifest.display()
        ))
    }
}

/// apksigner's arguments for the debug key at `keystore` (password `android`).
fn debug_key_args(keystore: &Path) -> Vec<OsString> {
    vec![
        "--ks".into(),
        keystore.into(),
        "--ks-pass".into(),
        "pass:android".into(),
        "--key-pass".into(),
        "pass:android".into(),
    ]
}

/// apksigner's arguments for the release `key`: its passwords as the manifest wrote them.
fn release_key_args(key: &ReleaseKey) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--ks".into(),
        key.keystore.clone().into(),
        "--ks-key-alias".into(),
        key.alias.clone().into(),
        "--ks-pass".into(),
        key.ks_pass.clone().into(),
    ];
    if let Some(pass) = &key.key_pass {
        args.push("--key-pass".into());
        args.push(pass.into());
    }
    args
}

/// What a `--ship` build with no key named prints: the APK at `out_apk` is unsigned, the
/// `apksigner` command that signs it in place, and the `[android]` table that makes `--ship`
/// sign it.
fn unsigned_notice(apksigner: &Path, min_sdk: u32, out_apk: &Path) -> String {
    format!(
        "loft: {out} is aligned but not signed: a --ship build signs only with the key its \
         loft.toml names, and none is named.  Sign it with\n  \
         {signer} sign --ks <release.jks> --ks-key-alias <alias> --ks-pass env:<VAR> \
         --min-sdk-version {min_sdk} {out}\n\
         or name the key beside the program, and --ship signs with it:\n  \
         [android]\n  keystore = \"release.jks\"\n  key_alias = \"release\"\n  \
         ks_pass = \"env:ANDROID_KS_PASS\"",
        out = out_apk.display(),
        signer = apksigner.display(),
    )
}

/// `path` made absolute against the current directory, for a tool run elsewhere.
fn absolute(path: &Path) -> PathBuf {
    if crate::file_access::is_absolute(path) {
        return path.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |d| d.join(path))
}

/// Does the program's C `c` reach the network — a URL fetch of the runtime (`LOFT_NET`), the
/// engine host's (`LOFT_ENGINE`), or a component's natives (the web, server and ssh libraries)?
/// An app opens no socket without the `INTERNET` permission, and asks for none it does not use.
pub(crate) fn needs_internet(c: &str) -> bool {
    c.contains("#define LOFT_NET 1")
        || c.contains("#define LOFT_ENGINE 1")
        || loft::c::component::NAMES
            .iter()
            .any(|n| c.contains(&format!("\"{}\"", loft::components::file_name(n))))
}

/// The list the entry extracts the packed files by (`src/c/android/loft_android_main.c`): it
/// cannot list the APK's `assets/` tree itself (`AAssetDir` names the files of one directory).
const ASSET_LIST: &str = "loft-assets.txt";

/// The files an app built from `program` (its C in `c`) carries, by their path relative to the
/// program's directory: the files an app carries on every target (@C164,
/// `app_files::carried_files`: the ones `listed` names, else the ones the program names).  On
/// the device they are extracted into the app's files directory, which is the program's
/// directory there, so a relative path finds what it found on the PC.  What went in and why is
/// written to `<report_dir>/<app>-report.txt`, as the ESP build writes its report.
///
/// # Errors
/// A listed entry that names nothing, or a path that would leave the files directory (`..`, an
/// absolute path) or that the list cannot hold (a line break in its name).
pub(crate) fn app_assets(
    program: &str,
    c: &str,
    listed: &[String],
    report_dir: &Path,
    out: &str,
) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    use std::fmt::Write as _;
    let dir = Path::new(program)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let carried = crate::app_files::carried_files(dir, c, listed)?;
    let mut report =
        String::from("# The files `loft --native-android` packed into this app, and why.\n\n");
    for (rel, path, why) in &carried {
        let size = crate::file_access::metadata(path)
            .map(|m| m.len())
            .unwrap_or(0);
        let _ = writeln!(report, "{size:9} {rel}  ({why})");
    }
    let name = crate::file_access::file_stem(Path::new(out)).unwrap_or_else(|| "app".into());
    let _ = crate::file_access::write(report_dir.join(format!("{name}-report.txt")), report);
    let files: Vec<(String, std::path::PathBuf)> =
        carried.into_iter().map(|(r, p, _)| (r, p)).collect();
    for (rel, _) in &files {
        if rel == ASSET_LIST {
            return Err(format!(
                "--assets: `{rel}` is the name the app's own file list takes"
            ));
        }
        let leaves = rel.starts_with('/') || rel.split('/').any(|c| c == "..");
        if leaves || rel.contains('\n') {
            return Err(format!(
                "--assets: `{rel}` cannot be packed: an app's file keeps its path below the program"
            ));
        }
    }
    Ok(files)
}

/// Stage `assets` under `staging/assets/`, beside the list the entry reads: a stamp first,
/// which changes when any file's path or content does (the entry extracts again then), and
/// one path a line.
fn stage_assets(staging: &Path, assets: &[(String, std::path::PathBuf)]) -> Result<(), String> {
    use std::hash::{Hash, Hasher};
    let root = staging.join("assets");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut list = String::new();
    for (rel, source) in assets {
        let bytes = crate::file_access::read(source)
            .map_err(|e| format!("loft: read asset {}: {e}", source.display()))?;
        rel.hash(&mut hasher);
        bytes.hash(&mut hasher);
        let to = root.join(rel);
        if let Some(parent) = to.parent() {
            crate::file_access::create_dir_all(parent)
                .map_err(|e| format!("loft: create {}: {e}", parent.display()))?;
        }
        crate::file_access::write(&to, &bytes)
            .map_err(|e| format!("loft: stage asset {rel}: {e}"))?;
        list.push_str(rel);
        list.push('\n');
    }
    let stamp = format!("{:016x}\n", hasher.finish());
    crate::file_access::write(root.join(ASSET_LIST), stamp + &list)
        .map_err(|e| format!("loft: write {ASSET_LIST}: {e}"))
}

/// A resolved Android cross-compile descriptor: the NDK, the API level, and the
/// Rust/NDK target triple. Build one with [`AndroidTarget::detect`]; the linker,
/// the generated crate, and the `.so` all derive from these three.
pub(crate) struct AndroidTarget {
    /// NDK root (from `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT`).
    ndk: PathBuf,
    /// Android API level the NDK `clang` wrapper targets (`LOFT_ANDROID_API`,
    /// default 24 — the lowest with full 64-bit + modern `libc` coverage).
    api: u32,
    /// Rust target triple (`LOFT_ANDROID_TARGET`, default `aarch64-linux-android`).
    triple: String,
}

impl AndroidTarget {
    /// Resolve the Android target from the environment, or return a message
    /// naming exactly what to set. Validates that the NDK directory and the
    /// per-API `clang` linker actually exist, so a misconfigured NDK fails here
    /// with an actionable error rather than an opaque `rustc`/`cc` failure later.
    pub(crate) fn detect() -> Result<Self, String> {
        let ndk = std::env::var_os("ANDROID_NDK_HOME")
            .or_else(|| std::env::var_os("ANDROID_NDK_ROOT"))
            .map(PathBuf::from)
            .ok_or_else(|| {
                format!(
                    "loft: --native-android needs the Android NDK — set ANDROID_NDK_HOME \
                     (or ANDROID_NDK_ROOT) to an NDK {NDK_MIN} or later (the `clang` \
                     linker + sysroot live there)."
                )
            })?;
        if !crate::file_access::is_dir(&ndk) {
            return Err(format!(
                "loft: ANDROID_NDK_HOME points at '{}', which is not a directory.",
                ndk.display()
            ));
        }
        let triple = std::env::var("LOFT_ANDROID_TARGET")
            .unwrap_or_else(|_| "aarch64-linux-android".to_string());
        let api: u32 = std::env::var("LOFT_ANDROID_API")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let target = Self { ndk, api, triple };
        // Fail now, with the resolved path, if the toolchain wrapper is absent
        // (wrong host tag, NDK too old for this API, or a partial install).
        let linker = target.linker();
        if !crate::file_access::exists(&linker) {
            return Err(format!(
                "loft: NDK linker not found at '{}'.  Check the NDK is complete and that \
                 LOFT_ANDROID_API={} / LOFT_ANDROID_TARGET={} match a wrapper in {}.",
                linker.display(),
                target.api,
                target.triple,
                target.toolchain_bin().display()
            ));
        }
        Ok(target)
    }

    /// Prebuilt-toolchain `bin/` for this host inside the NDK
    /// (`toolchains/llvm/prebuilt/<host-tag>/bin`).
    fn toolchain_bin(&self) -> PathBuf {
        self.ndk
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join(host_tag())
            .join("bin")
    }

    /// The NDK `clang` wrapper for `<triple><api>` — serves as BOTH the C compiler
    /// (android-activity's `native_app_glue`, and any future C dep) and the rustc
    /// linker, because it pre-sets the Android sysroot and `--target`.
    fn linker(&self) -> PathBuf {
        self.toolchain_bin()
            .join(format!("{}{}-clang", self.triple, self.api))
    }

    /// The C compiler the C backend's Android build runs: the NDK clang wrapper, which knows
    /// the sysroot and the target (c-backend.md).
    pub(crate) fn c_compiler(&self) -> PathBuf {
        self.linker()
    }

    /// The NDK's `native_app_glue` sources, which the C backend's Android build compiles.
    pub(crate) fn glue_dir(&self) -> PathBuf {
        self.ndk
            .join("sources")
            .join("android")
            .join("native_app_glue")
    }

    /// The NDK's `libc++_shared.so` for this triple — bundled into the APK because
    /// the generated cdylib links it (`-lc++_shared`, for `oboe`'s C++).
    fn libcxx_shared(&self) -> PathBuf {
        self.ndk
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join(host_tag())
            .join("sysroot")
            .join("usr")
            .join("lib")
            .join(&self.triple)
            .join("libc++_shared.so")
    }

    /// The activity library `built` delivered as `out_path`: packaged into an APK, signed as
    /// `package.signing` says, when the output names one, else copied as the bare `.so`.  Shared by the rustc path and the
    /// C path: neither the manifest nor the signing knows which compiler wrote the library.
    pub(crate) fn finish(
        &self,
        built: &Path,
        package: &Package,
        out_path: &Path,
    ) -> Result<(), String> {
        if crate::file_access::extension(out_path).is_some_and(|e| e.eq_ignore_ascii_case("apk")) {
            let sdk = AndroidSdk::detect()?;
            self.package_apk(&sdk, built, package, out_path)
        } else {
            crate::file_access::copy(built, out_path)
                .map(|_| ())
                .map_err(|e| format!("loft: copy to {}: {e}", out_path.display()))
        }
    }

    /// The Android ABI directory name for this triple (`lib/<abi>/` in the APK).
    fn abi(&self) -> &'static str {
        if self.triple.starts_with("aarch64") {
            "arm64-v8a"
        } else if self.triple.starts_with("armv7") || self.triple.starts_with("thumbv7") {
            "armeabi-v7a"
        } else if self.triple.starts_with("x86_64") {
            "x86_64"
        } else if self.triple.starts_with("i686") || self.triple.starts_with("x86") {
            "x86"
        } else {
            "arm64-v8a"
        }
    }

    /// Package the built `so_path` into an APK at `out_apk`, using the discovered `sdk`.
    /// Mirrors the proven `b2-spike/build_apk.sh` recipe: `aapt2 link` the generated
    /// manifest against `android.jar`, add the `.so` under `lib/<abi>/`, `zipalign`, then
    /// sign it as `package.signing` says ([`Self::sign`]). The `.so` is compressed with
    /// `android:extractNativeLibs="true"`, so it is unpacked at install and needs
    /// no page alignment.
    fn package_apk(
        &self,
        sdk: &AndroidSdk,
        so_path: &Path,
        package: &Package,
        out_apk: &Path,
    ) -> Result<(), String> {
        const LIB_NAME: &str = "loft_android_app";
        let app = app_ident(out_apk);
        let work = package.staging.join("apk-build");
        let lib_dir = work.join("staging").join("lib").join(self.abi());
        let _ = crate::file_access::remove_dir_all(&work);
        crate::file_access::create_dir_all(&lib_dir).map_err(|e| format!("loft: create {e}"))?;
        crate::file_access::copy(so_path, lib_dir.join(format!("lib{LIB_NAME}.so")))
            .map_err(|e| format!("loft: stage .so: {e}"))?;
        // Bundle libc++_shared.so when the app .so links it (the rustc path's oboe is C++; the
        // C backend's library names no C++ runtime and gets none).
        let libcxx = self.libcxx_shared();
        let needs_libcxx = crate::file_access::read(so_path).is_ok_and(|b| {
            b.windows(b"libc++_shared.so".len())
                .any(|w| w == b"libc++_shared.so")
        });
        if needs_libcxx && crate::file_access::is_file(&libcxx) {
            crate::file_access::copy(&libcxx, lib_dir.join("libc++_shared.so"))
                .map_err(|e| format!("loft: stage libc++_shared.so: {e}"))?;
        }
        for lib in package.libs {
            let name = crate::file_access::file_name(lib).unwrap_or_default();
            crate::file_access::copy(lib, lib_dir.join(&name))
                .map_err(|e| format!("loft: stage {name}: {e}"))?;
        }
        let manifest = work.join("AndroidManifest.xml");
        crate::file_access::write(
            &manifest,
            android_manifest(&app, LIB_NAME, package.debuggable, package.internet),
        )
        .map_err(|e| format!("loft: write manifest {e}"))?;

        let base = work.join("base.apk");
        run_tool(
            Spawn::new(Program::os(sdk.aapt2()))
                .arg("link")
                .arg("-o")
                .arg(&base)
                .arg("-I")
                .arg(&sdk.android_jar)
                .arg("--manifest")
                .arg(&manifest)
                .arg("--min-sdk-version")
                .arg(self.api.to_string())
                .arg("--target-sdk-version")
                .arg(sdk.platform_api.to_string())
                .arg("--version-code")
                .arg("1")
                .arg("--version-name")
                .arg("1.0"),
            "aapt2 link",
        )?;

        // Add the whole `lib/<abi>/` tree (the app .so + libc++_shared.so) and the program's
        // files under `assets/` to the APK zip via the JDK `jar` (no separate `zip`
        // dependency; `-M` keeps jar from injecting a MANIFEST.MF that would collide with
        // apksigner's signing block).
        let mut add = Spawn::new(Program::os(sdk.jar()))
            .cwd(&PathText::from_os(&work.join("staging")))
            .arg("--update")
            .arg("--no-manifest")
            .arg("--file")
            .arg(&base)
            .arg("lib");
        if !package.assets.is_empty() {
            stage_assets(&work.join("staging"), package.assets)?;
            add = add.arg("assets");
        }
        run_tool(add, "jar (add native libs and assets)")?;

        let aligned = work.join("aligned.apk");
        run_tool(
            Spawn::new(Program::os(sdk.zipalign()))
                .arg("-f")
                .arg("-p")
                .arg("4")
                .arg(&base)
                .arg(&aligned),
            "zipalign",
        )?;

        self.sign(
            &sdk.apksigner(),
            |dir| ensure_debug_keystore(sdk, dir),
            package,
            &aligned,
            out_apk,
        )
    }

    /// Sign the zipaligned `aligned` into `out_apk` as `package.signing` says: the debug key
    /// (`debug_key` finds or makes it in the keystore directory), the release key, or not at
    /// all — the APK copied unsigned and the command that signs it printed.
    fn sign(
        &self,
        apksigner: &Path,
        debug_key: impl FnOnce(&Path) -> Result<PathBuf, String>,
        package: &Package,
        aligned: &Path,
        out_apk: &Path,
    ) -> Result<(), String> {
        let tail = |args: &mut Vec<OsString>, out: &Path, input: &Path| {
            args.extend([
                "--min-sdk-version".into(),
                self.api.to_string().into(),
                "--out".into(),
                out.into(),
                input.into(),
            ]);
        };
        match package.signing {
            Signing::Debug => {
                let mut args = debug_key_args(&debug_key(package.keystore_dir)?);
                tail(&mut args, out_apk, aligned);
                run_tool(
                    Spawn::new(Program::os(apksigner)).arg("sign").args(args),
                    "apksigner sign",
                )
            }
            Signing::Release(key) => {
                let mut args = release_key_args(key);
                tail(&mut args, &absolute(out_apk), &absolute(aligned));
                // On loft's own streams: a `stdin` password is read from the terminal.
                let status = Spawn::new(Program::os(apksigner))
                    .cwd(&PathText::from_os(&key.dir))
                    .arg("sign")
                    .args(args)
                    .status()
                    .map_err(|e| format!("loft: cannot run apksigner sign: {e}"))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "loft: apksigner sign with {} failed ({status})",
                        key.keystore.display()
                    ))
                }
            }
            Signing::Unsigned => {
                crate::file_access::copy(aligned, out_apk)
                    .map_err(|e| format!("loft: copy to {}: {e}", out_apk.display()))?;
                eprintln!("{}", unsigned_notice(apksigner, self.api, out_apk));
                Ok(())
            }
        }
    }
}

/// A reusable debug keystore under the generated crate dir, created once with the
/// JDK `keytool` (password `android`, the Android debug-key convention).
fn ensure_debug_keystore(sdk: &AndroidSdk, dir: &Path) -> Result<PathBuf, String> {
    let keystore = dir.join("loft-debug.keystore");
    if crate::file_access::exists(&keystore) {
        return Ok(keystore);
    }
    // Made under a name of this process's own and linked into place, which fails where another
    // build got there first: two builds beside one program that both find no keystore then sign
    // with the same key, as an update of an installed app requires.
    let fresh = dir.join(format!("loft-debug.keystore.{}", std::process::id()));
    let _ = crate::file_access::remove_file(&fresh);
    run_tool(
        Spawn::new(Program::os(sdk.keytool()))
            .arg("-genkeypair")
            .arg("-keystore")
            .arg(&fresh)
            .arg("-alias")
            .arg("androiddebugkey")
            .arg("-storepass")
            .arg("android")
            .arg("-keypass")
            .arg("android")
            .arg("-keyalg")
            .arg("RSA")
            .arg("-keysize")
            .arg("2048")
            .arg("-validity")
            .arg("10000")
            .arg("-dname")
            .arg("CN=Loft Debug,O=Loft,C=US"),
        "keytool (debug keystore)",
    )?;
    let _ = crate::file_access::hard_link(&fresh, &keystore);
    let _ = crate::file_access::remove_file(&fresh);
    Ok(keystore)
}

/// Discovered Android SDK + JDK tools needed to package a `.so` into a signed APK.
/// Found from `ANDROID_HOME` / `ANDROID_SDK_ROOT` (SDK) and `JAVA_HOME` / `PATH` (JDK).
struct AndroidSdk {
    /// `<sdk>/build-tools/<highest-version>/` — holds `aapt2`, `zipalign`, `apksigner`.
    build_tools: PathBuf,
    /// `<sdk>/platforms/android-<N>/android.jar` — the compile target for `aapt2 link`.
    android_jar: PathBuf,
    /// The `<N>` of the chosen platform — the `--target-sdk-version`.
    platform_api: u32,
    /// JDK root (`bin/jar`, `bin/keytool`); `apksigner` also needs it on `PATH`.
    java_home: PathBuf,
}

impl AndroidSdk {
    /// Resolve the SDK + JDK, or return a message naming exactly what to install/set.
    fn detect() -> Result<Self, String> {
        let root = std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .map(PathBuf::from)
            .ok_or_else(|| {
                "loft: building an .apk needs the Android SDK — set ANDROID_HOME (or \
                 ANDROID_SDK_ROOT), or emit a `.so` output for just the library."
                    .to_string()
            })?;
        let build_tools = highest_versioned_subdir(&root.join("build-tools")).ok_or_else(|| {
            format!(
                "loft: no usable build-tools under {} (install e.g. `build-tools;34.0.0`).",
                root.display()
            )
        })?;
        let (android_jar, platform_api) =
            highest_platform(&root.join("platforms")).ok_or_else(|| {
                format!(
                    "loft: no platform android.jar under {} (install e.g. `platforms;android-34`).",
                    root.display()
                )
            })?;
        let java_home = detect_java_home().ok_or_else(|| {
            "loft: building an .apk needs a JDK (apksigner + keytool) — set JAVA_HOME or \
             put `keytool` on PATH."
                .to_string()
        })?;
        Ok(Self {
            build_tools,
            android_jar,
            platform_api,
            java_home,
        })
    }

    fn aapt2(&self) -> PathBuf {
        self.build_tools.join(exe("aapt2"))
    }
    fn zipalign(&self) -> PathBuf {
        self.build_tools.join(exe("zipalign"))
    }
    fn apksigner(&self) -> PathBuf {
        // apksigner is a launcher script: `.bat` on Windows, extension-less elsewhere.
        self.build_tools
            .join(crate::platform::launcher_script_name("apksigner"))
    }
    fn jar(&self) -> PathBuf {
        self.java_home.join("bin").join(exe("jar"))
    }
    fn keytool(&self) -> PathBuf {
        self.java_home.join("bin").join(exe("keytool"))
    }
}

/// Run an SDK/JDK tool, mapping a non-zero exit to a message with the stderr tail.
fn run_tool(mut cmd: Spawn, name: &str) -> Result<(), String> {
    let out = cmd
        .run(b"")
        .map_err(|e| format!("loft: cannot run {name}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().rev().take(15).collect();
    let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Err(format!("loft: {name} failed:\n{tail}"))
}

/// `<name>.exe` on Windows, `<name>` elsewhere.
fn exe(name: &str) -> String {
    crate::platform::exe_file_name(name)
}

/// A valid Android package/label segment derived from the APK's file stem
/// (`com.loft.<ident>`): lowercase ASCII alnum, must start with a letter.
fn app_ident(out_apk: &Path) -> String {
    let stem = crate::file_access::file_stem(out_apk).unwrap_or_else(|| "app".to_string());
    let s: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    let s = if s.is_empty() { "app".to_string() } else { s };
    if s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        s
    } else {
        format!("a{s}")
    }
}

/// The generated `AndroidManifest.xml` for a code-less `NativeActivity` APK
/// (`hasCode="false"`, no dex). `min`/`target` sdk come from `aapt2` flags, so no
/// `<uses-sdk>` here (aapt2 rejects the duplicate).
fn android_manifest(app: &str, lib_name: &str, debuggable: bool, internet: bool) -> String {
    // A debuggable app lets `adb shell run-as` read its private files: a build that is not
    // `--ship` keeps a copy of its stdout there (c-backend.md, A1).
    let debuggable = if debuggable {
        r#" android:debuggable="true""#
    } else {
        ""
    };
    let internet = if internet {
        "\n    <uses-permission android:name=\"android.permission.INTERNET\"/>"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.loft.{app}">{internet}
    <application android:label="{app}" android:hasCode="false" android:extractNativeLibs="true"{debuggable}>
        <activity android:name="android.app.NativeActivity" android:exported="true" android:label="{app}">
            <meta-data android:name="android.app.lib_name" android:value="{lib_name}"/>
            <intent-filter>
                <action android:name="android.intent.action.MAIN"/>
                <category android:name="android.intent.category.LAUNCHER"/>
            </intent-filter>
        </activity>
    </application>
</manifest>
"#
    )
}

/// The JDK root: `JAVA_HOME` if it holds `bin/keytool`, else derived from a `keytool`
/// on `PATH` (its grandparent).
fn detect_java_home() -> Option<PathBuf> {
    if let Some(jh) = std::env::var_os("JAVA_HOME").map(PathBuf::from) {
        if crate::file_access::is_file(jh.join("bin").join(exe("keytool"))) {
            return Some(jh);
        }
    }
    let kt = find_on_path(&exe("keytool"))?;
    kt.parent()?.parent().map(Path::to_path_buf)
}

/// The first `PATH` directory containing an executable `name`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| crate::file_access::is_file(p))
}

/// The numeric sort key of a dotted version like `34.0.0` → `[34, 0, 0]`.
fn version_key(name: &str) -> Vec<u32> {
    name.split('.').filter_map(|p| p.parse().ok()).collect()
}

/// The highest-versioned `build-tools/<v>/` subdir that actually contains `aapt2`.
fn highest_versioned_subdir(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for entry in crate::file_access::read_dir(dir).ok()? {
        let p = entry.os_spelling();
        let key = version_key(entry.file_name().unwrap_or_default());
        if key.is_empty() || !crate::file_access::exists(p.join(exe("aapt2"))) {
            continue;
        }
        if best.as_ref().is_none_or(|(bk, _)| &key > bk) {
            best = Some((key, p));
        }
    }
    best.map(|(_, p)| p)
}

/// The highest `platforms/android-<N>/android.jar` and its `<N>`.
fn highest_platform(dir: &Path) -> Option<(PathBuf, u32)> {
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in crate::file_access::read_dir(dir).ok()? {
        let name = entry.file_name().unwrap_or_default();
        let Some(n) = name
            .strip_prefix("android-")
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let jar = entry.os_spelling().join("android.jar");
        if crate::file_access::exists(&jar) && best.as_ref().is_none_or(|(bn, _)| n > *bn) {
            best = Some((n, jar));
        }
    }
    best.map(|(n, jar)| (jar, n))
}

/// The NDK prebuilt-toolchain host tag for the machine loft runs on.  Android
/// NDKs since r23 ship only x86_64 host toolchains (there is no aarch64-host or
/// 32-bit build), so every supported host maps to its `*-x86_64` tag.
fn host_tag() -> &'static str {
    crate::platform::ndk_host_tag()
}

#[cfg(test)]
mod tests {
    use super::{AndroidTarget, Package, Signing, app_assets, needs_internet, signing};
    use std::path::{Path, PathBuf};

    /// A fresh directory holding a program, with `toml` as its `loft.toml` when given.
    fn app_dir(tag: &str, toml: Option<&str>) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("loft_android_{tag}_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&dir);
        crate::file_access::create_dir_all(&dir).expect("mkdir");
        let program = dir.join("app.loft");
        crate::file_access::write(&program, "fn main() {}").expect("write program");
        if let Some(toml) = toml {
            crate::file_access::write(dir.join("loft.toml"), toml).expect("write loft.toml");
        }
        (dir, program)
    }

    const RELEASE_TOML: &str = "[package]\nname = \"app\"\n\n[android]\n\
        keystore = \"keys/release.jks\"\nkey_alias = \"release\"\n\
        ks_pass = \"env:APP_KS_PASS\"\nkey_pass = \"file:keys/key.pass\"\n";

    /// A stand-in `apksigner` that writes its working directory and arguments, one a line,
    /// to `<dir>/apksigner.log` (`platform::logging_stub`).
    fn fake_apksigner(dir: &Path) -> (PathBuf, PathBuf) {
        let log = dir.join("apksigner.log");
        (crate::platform::logging_stub(dir, "apksigner", &log), log)
    }

    fn target() -> AndroidTarget {
        AndroidTarget {
            ndk: PathBuf::new(),
            api: 24,
            triple: "aarch64-linux-android".to_string(),
        }
    }

    fn package<'a>(dir: &'a Path, signing: &'a Signing) -> Package<'a> {
        Package {
            staging: dir,
            keystore_dir: dir,
            signing,
            debuggable: matches!(signing, Signing::Debug),
            assets: &[],
            libs: &[],
            internet: false,
        }
    }

    /// A `--ship` build signs with the key `[android]` names: the keystore resolved against the
    /// loft.toml, the alias, and both passwords handed to apksigner exactly as written, with
    /// apksigner run in the loft.toml's directory so a `file:` password resolves as the
    /// keystore does.  The debug key is never asked for.
    /// @falsified-at: 04818af26 — by hand: with `signing` answering `Signing::Debug` for a
    /// `--ship` build (what it did before), the `Release` match fails; with `release_key_args`
    /// dropping `--ks-key-alias`, the argument list differs.
    #[test]
    fn a_ship_build_signs_with_the_key_its_manifest_names() {
        let (dir, program) = app_dir("release", Some(RELEASE_TOML));
        // The keystore is checked for before the build.
        assert!(
            signing(true, &program).is_err_and(|e| e.contains("is not there")),
            "a named keystore that is missing is refused"
        );
        crate::file_access::create_dir_all(dir.join("keys")).expect("mkdir keys");
        crate::file_access::write(dir.join("keys").join("release.jks"), "ks").expect("ks");
        let s = signing(true, &program).expect("a configured key");
        assert!(
            matches!(s, Signing::Release(_)),
            "--ship takes the release key"
        );
        let (apksigner, log) = fake_apksigner(&dir);
        let aligned = dir.join("aligned.apk");
        let out = dir.join("app.apk");
        target()
            .sign(
                &apksigner,
                |_| panic!("a release build asked for the debug key"),
                &package(&dir, &s),
                &aligned,
                &out,
            )
            .expect("signed");
        let root = crate::file_access::plain_canonical(&dir);
        let ran = crate::file_access::read_to_string(&log).expect("apksigner ran");
        let ran: Vec<&str> = ran.lines().collect();
        assert_eq!(
            crate::file_access::plain_canonical(Path::new(ran[0])),
            root,
            "apksigner runs in the loft.toml's directory"
        );
        let ks = root.join("keys").join("release.jks");
        let want = [
            "sign",
            "--ks",
            &ks.to_string_lossy(),
            "--ks-key-alias",
            "release",
            "--ks-pass",
            "env:APP_KS_PASS",
            "--key-pass",
            "file:keys/key.pass",
            "--min-sdk-version",
            "24",
            "--out",
            &out.to_string_lossy(),
            &aligned.to_string_lossy(),
        ];
        assert_eq!(&ran[1..], &want[..]);
        let _ = crate::file_access::remove_dir_all(&dir);
    }

    /// A password written into the manifest (`pass:`) is refused, for either password and on a
    /// debug build too; so is a value apksigner does not read a password from.  `env:`,
    /// `file:` and `stdin` are taken.
    /// @falsified-at: 04818af26 — by hand: with `password` taking `pass:` as apksigner does,
    /// the `pass:` manifests are taken; with only its `pass:` refusal removed, the refusal
    /// that is left echoes the password.
    #[test]
    fn a_password_written_into_the_manifest_is_refused() {
        let toml = |ks_pass: &str, key_pass: &str| {
            format!(
                "[android]\nkeystore = \"r.jks\"\nkey_alias = \"r\"\nks_pass = \"{ks_pass}\"\n{key_pass}"
            )
        };
        for (tag, manifest, refused) in [
            ("ks", toml("pass:hunter2", ""), true),
            ("key", toml("env:KS", "key_pass = \"pass:hunter2\"\n"), true),
            ("bare", toml("hunter2", ""), true),
            ("env", toml("env:KS", "key_pass = \"file:k.pass\"\n"), false),
            ("stdin", toml("stdin", ""), false),
        ] {
            let (dir, program) = app_dir(&format!("pass_{tag}"), Some(&manifest));
            crate::file_access::write(dir.join("r.jks"), "ks").expect("ks");
            for ship in [true, false] {
                let got = signing(ship, &program);
                if refused {
                    let e = got.err().unwrap_or_else(|| panic!("{tag}: taken"));
                    assert!(e.contains("env:NAME"), "{tag}: {e}");
                    assert!(
                        !e.contains("hunter2") || tag == "bare",
                        "{tag}: echoes it: {e}"
                    );
                } else {
                    assert!(got.is_ok(), "{tag}: {:?}", got.err());
                }
            }
            let _ = crate::file_access::remove_dir_all(&dir);
        }
    }

    /// A `--ship` build of a program whose manifest names no key is aligned and left unsigned:
    /// the APK is the aligned one, the debug key is never made or read, and the notice names
    /// the apksigner command that signs it.
    /// @falsified-at: 04818af26 — by hand: with `signing` answering `Signing::Debug` for a
    /// `--ship` build, the `Unsigned` match fails; with the `Unsigned` arm asking for the debug
    /// key, the panic fires.
    #[test]
    fn a_ship_build_without_a_key_is_left_unsigned() {
        for (tag, toml) in [
            ("unsigned_none", None),
            ("unsigned_bare", Some("[package]\nname = \"app\"\n")),
        ] {
            let (dir, program) = app_dir(tag, toml);
            let s = signing(true, &program).expect("no key is not an error");
            assert!(matches!(s, Signing::Unsigned), "{tag}");
            let keys = dir.join("keys");
            crate::file_access::create_dir_all(&keys).expect("mkdir");
            let aligned = dir.join("aligned.apk");
            crate::file_access::write(&aligned, "aligned bytes").expect("aligned");
            let out = dir.join("app.apk");
            let mut p = package(&dir, &s);
            p.keystore_dir = &keys;
            target()
                .sign(
                    Path::new("/no/apksigner"),
                    |_| panic!("an unsigned build asked for the debug key"),
                    &p,
                    &aligned,
                    &out,
                )
                .expect("written unsigned");
            assert_eq!(
                crate::file_access::read_to_string(&out).ok().as_deref(),
                Some("aligned bytes")
            );
            assert_eq!(
                crate::file_access::read_dir(&keys).map(|v| v.len()).ok(),
                Some(0)
            );
            let _ = crate::file_access::remove_dir_all(&dir);
        }
        let notice = super::unsigned_notice(Path::new("/sdk/apksigner"), 24, Path::new("a.apk"));
        assert!(
            notice.contains(
                "/sdk/apksigner sign --ks <release.jks> --ks-key-alias <alias> \
                 --ks-pass env:<VAR> --min-sdk-version 24 a.apk"
            ),
            "{notice}"
        );
        assert!(notice.contains("[android]"), "{notice}");
    }

    /// A debug build signs with the per-tree debug key, also when the manifest names a
    /// release key.
    /// @falsified-at: 04818af26 — by hand: with `signing` answering the release key for a
    /// debug build, the `Debug` match fails.
    #[test]
    fn a_debug_build_signs_with_the_debug_key() {
        let (dir, program) = app_dir("debug", Some(RELEASE_TOML));
        let s = signing(false, &program).expect("a debug build");
        assert!(matches!(s, Signing::Debug));
        let (apksigner, log) = fake_apksigner(&dir);
        let debug_ks = dir.join("loft-debug.keystore");
        let mut asked = false;
        target()
            .sign(
                &apksigner,
                |d| {
                    asked = true;
                    Ok(d.join("loft-debug.keystore"))
                },
                &package(&dir, &s),
                &dir.join("aligned.apk"),
                &dir.join("app.apk"),
            )
            .expect("signed");
        assert!(asked, "the debug key was asked for");
        let ran = crate::file_access::read_to_string(&log).expect("apksigner ran");
        let args: Vec<&str> = ran.lines().skip(1).take(7).collect();
        assert_eq!(
            args,
            [
                "sign",
                "--ks",
                &debug_ks.to_string_lossy(),
                "--ks-pass",
                "pass:android",
                "--key-pass",
                "pass:android"
            ]
        );
        let _ = crate::file_access::remove_dir_all(&dir);
    }

    /// The C the C backend writes for the program `src`.
    fn emitted_c(src: &str) -> String {
        let mut p = loft::parser::Parser::new();
        p.parse_dir("default", true, false).expect("stdlib");
        p.parse_str(src, "<app>", false);
        loft::scopes::check(&mut p.data, &mut p.database);
        loft::c::emit_browser_program(&p.data, &p.database).expect("the program emits as C")
    }

    /// An app asks for the network exactly when its C reaches it: a URL fetch of the runtime, the
    /// engine host's, or a component's natives; a program that does none of these asks for none.
    #[test]
    fn an_app_asks_for_the_network_only_when_its_c_uses_it() {
        let net = loft::components::file_name("net");
        assert!(needs_internet("#define LOFT_NET 1\n"));
        assert!(needs_internet("#define LOFT_ENGINE 1\n"));
        assert!(needs_internet(&format!(
            "loft_ffi_sym(\"/x/{net}\", \"{net}\", \"n_http_do\")"
        )));
        assert!(!needs_internet("int main(void) { return 0; }"));
    }

    /// The files an app carries keep their paths below the program (@C164): with no list, the
    /// ones the program names; with `--assets`, the ones listed.  A path that would leave the
    /// files directory, or that takes the name of the app's own file list, is refused at the
    /// build, and the report says what went in and why.
    #[test]
    fn an_app_carries_its_files_below_the_program() {
        let dir = std::env::temp_dir().join(format!("loft_android_assets_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&dir);
        let app = dir.join("app");
        let program = app.join("prog.loft");
        crate::file_access::create_dir_all(app.join("data")).expect("mkdir");
        crate::file_access::write(&program, "fn main() {}").expect("write");
        crate::file_access::write(app.join("data").join("a.txt"), "a").expect("write");
        crate::file_access::write(app.join("data").join("b.txt"), "b").expect("write");
        crate::file_access::write(dir.join("outside.txt"), "o").expect("write");
        crate::file_access::write(app.join("loft-assets.txt"), "x").expect("write");
        let program = program.to_string_lossy().into_owned();
        let names = |c: &str, listed: &[&str]| {
            let listed: Vec<String> = listed.iter().map(|s| (*s).to_string()).collect();
            app_assets(&program, c, &listed, &dir, "game.apk")
                .map(|f| f.into_iter().map(|(r, _)| r).collect::<Vec<_>>())
        };
        // No list: what the program's C names, and nothing else.
        assert_eq!(names("", &[]), Ok(vec![]));
        // The C the emitter writes for a program that names the file, never a hand-written
        // spelling of it: a text constant's spelling in the C is what the scan reads.
        let c = emitted_c("fn main() { p = \"data/b.txt\"; println(\"{p}\"); }");
        assert_eq!(names(&c, &[]), Ok(vec!["data/b.txt".to_string()]));
        let report =
            crate::file_access::read_to_string(dir.join("game-report.txt")).expect("report");
        assert!(report.contains("data/b.txt"), "{report}");
        // A list: what it names.
        assert_eq!(
            names("", &["data"]),
            Ok(vec!["data/a.txt".to_string(), "data/b.txt".to_string()])
        );
        assert!(names("", &["../outside.txt"]).is_err_and(|e| e.contains("cannot be packed")));
        assert!(names("", &["loft-assets.txt"]).is_err_and(|e| e.contains("file list")));
        assert!(names("", &["missing"]).is_err());
        let _ = crate::file_access::remove_dir_all(&dir);
    }
}
