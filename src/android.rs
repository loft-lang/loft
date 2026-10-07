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
use std::path::{Path, PathBuf};

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
                "loft: --native-android needs the Android NDK — set ANDROID_NDK_HOME \
                 (or ANDROID_NDK_ROOT) to an NDK r26+ install (the `clang` linker + \
                 sysroot live there)."
                    .to_string()
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

    /// Wrap the emitted program `rs_path` in a `NativeActivity` cdylib and build
    /// it into `out_path` — the whole `--native-android` pipeline. `main.rs` only
    /// emits the program `.rs` and calls this. The output type is the extension:
    /// `.apk` → a signed, installable APK (needs the Android SDK + a JDK); anything
    /// else → the bare `.so` (needs only the NDK).
    ///
    /// Generates a tiny cargo crate under `<tree>/target/loft/android-app/` whose
    /// crate root IS the emitted program (its `fn main` intact) plus the fixed
    /// [`ANDROID_MAIN_TAIL`] entry, with `android-activity` + a lean `loft` path
    /// dep, then `cargo build --release --target <triple>`. Cargo (not hand-rolled
    /// rustc) owns the `android-activity` dep tree and the cross-build of loft, so
    /// there is no transitive-rlib bookkeeping here.
    pub(crate) fn build(
        &self,
        rs_path: &Path,
        out_path: &Path,
        native_packages: &[(String, String)],
    ) -> Result<(), String> {
        let tree = crate::native_utils::loft_source_tree().ok_or_else(|| {
            "loft: --native-android needs loft's source tree to cross-build its Android \
             runtime; run it from a loft checkout (a downloaded release ships no Android \
             runtime)."
                .to_string()
        })?;
        let crate_dir = tree.join("target").join("loft").join("android-app");
        let src_dir = crate_dir.join("src");
        crate::file_access::create_dir_all(&src_dir).map_err(|e| format!("loft: create {e}"))?;

        // Serialise on the SAME lock the package / runtime auto-builds use: the
        // crate dir is shared across runs (so cargo caches android-activity + loft),
        // so concurrent `--native-android` runs must not clobber each other's lib.rs.
        let _build_lock = crate::file_access::open_with(
            std::env::temp_dir().join("loft-native-build.lock"),
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false),
        )
        .ok();
        if let Some(f) = &_build_lock {
            let _ = f.lock();
        }

        // A windowing native package (lib/graphics) needs the `AndroidApp` handed to
        // it before the program runs (the OS entry lives here, not in the package —
        // see android_gl.rs); the entry tail passes it via `loft_gl_android_set_app`.
        let needs_gl_app = native_packages
            .iter()
            .any(|(c, _)| c == "loft-graphics-native");

        // Crate root = the emitted program (its crate-level `#![allow]` attrs and
        // `fn main` stay valid at the root) + `extern crate <pkg>` for each native
        // package (the C-ABI call path names the package's symbols by `#[link_name]`
        // but never `use`s the crate, so force-link its rlib) + the android_main tail.
        let program = crate::file_access::read_to_string(rs_path)
            .map_err(|e| format!("loft: read emitted program {e}"))?;
        let mut externs = String::new();
        for (c, _) in native_packages {
            use std::fmt::Write as _;
            let _ = writeln!(externs, "extern crate {};", c.replace('-', "_"));
        }
        let lib_rs = format!("{program}\n{externs}{}", android_main_tail(needs_gl_app));
        crate::file_access::write(src_dir.join("lib.rs"), lib_rs)
            .map_err(|e| format!("loft: write {e}"))?;
        crate::file_access::write(
            crate_dir.join("Cargo.toml"),
            cargo_toml(&tree, native_packages),
        )
        .map_err(|e| format!("loft: write {e}"))?;

        let linker = self.linker();
        let mut cmd = Spawn::new(Program::search("cargo"))
            .args(["build", "--release", "--target", &self.triple])
            .cwd(&PathText::from_os(&crate_dir))
            .env("ANDROID_NDK_ROOT", &self.ndk)
            .env("ANDROID_NDK_HOME", &self.ndk)
            .env(cargo_linker_var(&self.triple), &linker)
            .env(format!("CC_{}", self.triple), &linker)
            .env(
                format!("CXX_{}", self.triple),
                self.toolchain_bin()
                    .join(format!("{}{}-clang++", self.triple, self.api)),
            )
            .env(
                format!("AR_{}", self.triple),
                self.toolchain_bin().join("llvm-ar"),
            )
            // Host RUSTFLAGS (target-cpu etc.) are host-specific and would break the
            // Android build. Replace them with just the one link-arg the Android
            // cdylib needs: link the NDK's `libc++_shared` so a C++ dependency
            // (rodio's `oboe` audio backend) resolves `__cxa_pure_virtual` etc.
            // `package_apk` bundles `libc++_shared.so` next to the app `.so`.
            // link-args are ignored for the rlib deps, so this only affects the cdylib.
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env("RUSTFLAGS", "-Clink-arg=-lc++_shared")
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let status = cmd
            .status()
            .map_err(|e| format!("loft: cannot run cargo for --native-android: {e}"))?;
        if !status.success() {
            return Err(format!(
                "loft: Android build failed for {} (generated crate kept at {} — inspect \
                 src/lib.rs).",
                self.triple,
                crate_dir.display()
            ));
        }
        let built = crate_dir
            .join("target")
            .join(&self.triple)
            .join("release")
            .join("libloft_android_app.so");

        if crate::file_access::extension(out_path).is_some_and(|e| e.eq_ignore_ascii_case("apk")) {
            let sdk = AndroidSdk::detect()?;
            self.package_apk(&sdk, &built, &crate_dir, out_path)
        } else {
            crate::file_access::copy(&built, out_path)
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

    /// Package the built `so_path` into a signed, installable APK at `out_apk`,
    /// using the discovered `sdk`. Mirrors the proven `b2-spike/build_apk.sh`
    /// recipe: `aapt2 link` the generated manifest against `android.jar`, add the
    /// `.so` under `lib/<abi>/`, `zipalign`, then `apksigner` with a per-tree debug
    /// keystore (generated once). The `.so` is compressed with
    /// `android:extractNativeLibs="true"`, so it is unpacked at install and needs
    /// no page alignment.
    fn package_apk(
        &self,
        sdk: &AndroidSdk,
        so_path: &Path,
        crate_dir: &Path,
        out_apk: &Path,
    ) -> Result<(), String> {
        const LIB_NAME: &str = "loft_android_app";
        let app = app_ident(out_apk);
        let work = crate_dir.join("apk-build");
        let lib_dir = work.join("staging").join("lib").join(self.abi());
        let _ = crate::file_access::remove_dir_all(&work);
        crate::file_access::create_dir_all(&lib_dir).map_err(|e| format!("loft: create {e}"))?;
        crate::file_access::copy(so_path, lib_dir.join(format!("lib{LIB_NAME}.so")))
            .map_err(|e| format!("loft: stage .so: {e}"))?;
        // Bundle libc++_shared.so (the app .so links it for oboe's C++).
        let libcxx = self.libcxx_shared();
        if crate::file_access::is_file(&libcxx) {
            crate::file_access::copy(&libcxx, lib_dir.join("libc++_shared.so"))
                .map_err(|e| format!("loft: stage libc++_shared.so: {e}"))?;
        }
        let manifest = work.join("AndroidManifest.xml");
        crate::file_access::write(&manifest, android_manifest(&app, LIB_NAME))
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

        // Add the whole `lib/<abi>/` tree (the app .so + libc++_shared.so) to the APK
        // zip via the JDK `jar` (no separate `zip` dependency; `-M` keeps jar from
        // injecting a MANIFEST.MF that would collide with apksigner's signing block).
        run_tool(
            Spawn::new(Program::os(sdk.jar()))
                .cwd(&PathText::from_os(&work.join("staging")))
                .arg("--update")
                .arg("--no-manifest")
                .arg("--file")
                .arg(&base)
                .arg("lib"),
            "jar (add native libs)",
        )?;

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

        let keystore = ensure_debug_keystore(sdk, crate_dir)?;
        run_tool(
            Spawn::new(Program::os(sdk.apksigner()))
                .arg("sign")
                .arg("--ks")
                .arg(&keystore)
                .arg("--ks-pass")
                .arg("pass:android")
                .arg("--key-pass")
                .arg("pass:android")
                .arg("--min-sdk-version")
                .arg(self.api.to_string())
                .arg("--out")
                .arg(out_apk)
                .arg(&aligned),
            "apksigner sign",
        )
    }
}

/// A reusable debug keystore under the generated crate dir, created once with the
/// JDK `keytool` (password `android`, the Android debug-key convention).
fn ensure_debug_keystore(sdk: &AndroidSdk, crate_dir: &Path) -> Result<PathBuf, String> {
    let keystore = crate_dir.join("loft-debug.keystore");
    if crate::file_access::exists(&keystore) {
        return Ok(keystore);
    }
    run_tool(
        Spawn::new(Program::os(sdk.keytool()))
            .arg("-genkeypair")
            .arg("-keystore")
            .arg(&keystore)
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
fn android_manifest(app: &str, lib_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.loft.{app}">
    <application android:label="{app}" android:hasCode="false" android:extractNativeLibs="true">
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

/// The generated crate's `Cargo.toml`: a `cdylib` NativeActivity app depending on
/// `android-activity` (+ logging + libc), a lean `loft` path dep (B1 runtime:
/// `random` + `threading`, no `ring`/registry), and each of the program's native
/// packages as a **path (rlib) dep** — the generated code names them with `extern
/// crate` (the non-C-ABI path), and linking them as rlibs (not sealed cdylibs)
/// unifies `android-activity` so the graphics backend and the entry share one
/// `AndroidApp` type. `[patch.crates-io]` points the packages' `loft-ffi` /
/// `loft-ffi-macros` at loft's own copies so there is ONE `loft-ffi` (no colliding
/// `StableCrateId`). The empty `[workspace]` stops cargo walking up into the loft
/// package that contains this directory. Paths are Debug-quoted via `String` (not
/// `Path`, which the lint rejects as lossy) so they land as escaped TOML strings.
fn cargo_toml(tree: &Path, native_packages: &[(String, String)]) -> String {
    let tree_s = tree.display().to_string();
    let mut pkg_deps = String::new();
    for (crate_name, pkg_dir) in native_packages {
        let native_dir = Path::new(pkg_dir).join("native").display().to_string();
        pkg_deps.push_str(&format!("{crate_name} = {{ path = {native_dir:?} }}\n"));
    }
    // The C-ABI call marshalling names `loft_ffi::LoftStr` etc., so the generated
    // crate needs `loft-ffi` in scope (loft's own copy — the `[patch]` unifies it).
    if !native_packages.is_empty() {
        let ffi = tree.join("loft-ffi").display().to_string();
        pkg_deps.push_str(&format!("loft-ffi = {{ path = {ffi:?} }}\n"));
    }
    // Native packages (e.g. lib/graphics) dispatch their registered functions
    // through `loft::native_call`, which is behind the `native-extensions` feature —
    // add it so the generated `extern crate <pkg>` calls resolve.
    let loft_features = if native_packages.is_empty() {
        "\"random\", \"threading\""
    } else {
        "\"random\", \"threading\", \"native-extensions\""
    };
    let patch = if native_packages.is_empty() {
        String::new()
    } else {
        let ffi = tree.join("loft-ffi").display().to_string();
        let ffi_macros = tree.join("loft-ffi-macros").display().to_string();
        format!(
            "\n[patch.crates-io]\n\
             loft-ffi = {{ path = {ffi:?} }}\n\
             loft-ffi-macros = {{ path = {ffi_macros:?} }}\n"
        )
    };
    format!(
        "[package]\n\
         name = \"loft_android_app\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\n\
         [workspace]\n\n\
         [lib]\n\
         crate-type = [\"cdylib\"]\n\n\
         [dependencies]\n\
         android-activity = {{ version = \"0.6\", features = [\"native-activity\"] }}\n\
         log = \"0.4\"\n\
         android_logger = \"0.14\"\n\
         libc = \"0.2\"\n\
         loft = {{ path = {tree_s:?}, default-features = false, features = [{loft_features}] }}\n\
         {pkg_deps}\n\
         [profile.release]\n\
         opt-level = 2\n\
         {patch}"
    )
}

/// The Android `NativeActivity` entry, appended to the emitted program's crate root.
/// `android-activity` (native-activity) exports `ANativeActivity_onCreate` and calls
/// this `android_main` on a dedicated thread; we run the loft program's `main` in it
/// and forward the program's stdout into logcat so `print()` shows up in `adb logcat`.
///
/// When the program uses the graphics backend (`needs_gl_app`), the entry first hands
/// the `AndroidApp` to it via `loft_gl_android_set_app` (the seam in
/// `lib/graphics`'s `android_gl.rs`), so `loft_gl_create_window` can reach the
/// `ANativeWindow`. Sound because loft links the graphics crate as a unified rlib on
/// Android (see `cargo_toml`), so the `AndroidApp` type is shared.
fn android_main_tail(needs_gl_app: bool) -> String {
    let set_app = if needs_gl_app {
        "    unsafe extern \"C\" { fn loft_gl_android_set_app(app_ptr: *const std::ffi::c_void); }\n\
         \x20   unsafe { loft_gl_android_set_app(&app as *const _ as *const std::ffi::c_void); }\n"
    } else {
        ""
    };
    [ANDROID_TAIL_PREFIX, set_app, ANDROID_TAIL_SUFFIX].concat()
}

const ANDROID_TAIL_PREFIX: &str = r#"
// ---- @PLN106 B2/B3: Android NativeActivity entry (emitted by --native-android) ----

// @PLN106 — the android-activity glue spawns `android_main` with `std::thread::spawn` (Rust's
// default ~2MB stack), and loft runs `__run` INLINE on that thread (it owns the ALooper — a
// spawned big-stack thread would panic in the graphics poll). So deep recursion overflows.
// Enlarge that thread's stack to the desktop `NATIVE_MAIN_STACK` (512 MiB) by setting
// `RUST_MIN_STACK` from a `.init_array` constructor that runs at `.so` load — BEFORE the glue's
// spawn reads (and std caches) the default stack size.
#[used]
#[unsafe(link_section = ".init_array")]
static __LOFT_ANDROID_STACK_CTOR: extern "C" fn() = {
    extern "C" fn ctor() {
        unsafe { libc::setenv(c"RUST_MIN_STACK".as_ptr(), c"536870912".as_ptr(), 1); }
    }
    ctor
};

fn __loft_android_redirect_stdio_to_logcat() {
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const libc::c_char, text: *const libc::c_char) -> i32;
    }
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let (rd, wr) = (fds[0], fds[1]);
    unsafe {
        libc::dup2(wr, libc::STDOUT_FILENO);
        libc::dup2(wr, libc::STDERR_FILENO);
        libc::close(wr);
    }
    std::thread::spawn(move || {
        const TAG: &[u8] = b"loft-stdout\0";
        let mut buf = [0u8; 1024];
        let mut line: Vec<u8> = Vec::new();
        loop {
            let n = unsafe { libc::read(rd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if b == b'\n' {
                    line.push(0);
                    unsafe {
                        __android_log_write(
                            4, // ANDROID_LOG_INFO
                            TAG.as_ptr() as *const libc::c_char,
                            line.as_ptr() as *const libc::c_char,
                        );
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
    });
}

// `AndroidApp` is not `#[repr(C)]`, but `android-activity`'s glue is what calls this
// `android_main` (across a Rust ABI it controls), so the C-FFI-safety lint is moot.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
extern "C" fn android_main(app: android_activity::AndroidApp) {
    use android_activity::{MainEvent, PollEvent};
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("loft"),
    );
    log::info!("loft --native-android: android_main reached");
    __loft_android_redirect_stdio_to_logcat();
"#;

/// The rest of the entry: run the program's `main`, then keep the activity alive.
/// `android_main_tail` splices the graphics seam (if any) between the prefix and this.
const ANDROID_TAIL_SUFFIX: &str = r#"    // Run the loft program (its `fn main`); stdout is now piped into logcat.
    main();
    log::info!("loft --native-android: program returned");
    // Stay alive to drain lifecycle events (a real app renders here) until the OS
    // destroys the activity, so the process does not exit out from under the app.
    loop {
        let mut destroy = false;
        app.poll_events(Some(std::time::Duration::from_millis(250)), |event| {
            if let PollEvent::Main(MainEvent::Destroy) = event {
                destroy = true;
            }
        });
        if destroy {
            break;
        }
    }
}
"#;

/// The cargo linker override env var for a triple, e.g. `aarch64-linux-android`
/// → `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`.
fn cargo_linker_var(triple: &str) -> String {
    format!(
        "CARGO_TARGET_{}_LINKER",
        triple.to_uppercase().replace('-', "_")
    )
}

/// The NDK prebuilt-toolchain host tag for the machine loft runs on.  Android
/// NDKs since r23 ship only x86_64 host toolchains (there is no aarch64-host or
/// 32-bit build), so every supported host maps to its `*-x86_64` tag.
fn host_tag() -> &'static str {
    crate::platform::ndk_host_tag()
}
