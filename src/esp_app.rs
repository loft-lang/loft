// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN204 — `--native-esp`: a loft program as an ESP-IDF app for the Tanmatsu
//! (TANMATSU.md § A loft program as an app).  The project holds the unit
//! `loft::c::emit_esp_program` writes, the `[c]` parts of the libraries it uses, the
//! program's `assets/` packed as a read-only filesystem, the Tanmatsu's partition table and a
//! configuration tuned for speed.  `idf.py build` makes `build/application.bin`, which the
//! launcher installs into `appfs` (badgelink).  No Rust part of any library is built or
//! linked: the C backend loads none.

use loft::file_access as fa;
use loft::platform::process::{Program, Spawn};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The BSP the libraries' Tanmatsu sections are written against, as the Tanmatsu app template
/// pins it.
const BSP: &str = "badgeteam/badge-bsp: \"=1.6.4\"";

/// The ESP-IDF release the template builds with.
const IDF_VERSION: &str = "v6.0.2";

/// The Tanmatsu's flash layout, as the launcher ships it: the app is built against it, and
/// installed into `appfs`.
const PARTITIONS: &str = "\
# Name,   Type, SubType, Offset,   Size,   Flags
nvs,      data, nvs,     0x19000,  16K,
otadata,  data, ota,     0x1D000,  8K,
phy_init, data, phy,     0x1F000,  4K,
ota_0,    0,    ota_0,   0x20000,  2048K,
ota_1,    0,    ota_1,   0x220000, 2048K,
appfs,    0x43, 3,       0x420000, 8192K,
locfd,    data, fat,     0xC20000, 3968K,
";

/// The configuration: the Tanmatsu's board (P4 revision 1, 16 MB QIO flash, PSRAM at 200 MHz,
/// the console on the USB serial port, the RTC memory the launcher reads which app to start
/// from, no test of all 32 MB of PSRAM at every start), then speed: `-O2`, a 1 kHz scheduler tick so a
/// `poll(15)` waits 15 ms, a main task stack for loft's recursion, and no task watchdog to stop
/// a long computation.  No radio: an app that uses none starts faster and is smaller.
const SDKCONFIG: &str = "\
CONFIG_IDF_TARGET=\"esp32p4\"
CONFIG_BSP_TARGET_TANMATSU=y
CONFIG_ESP32P4_SELECTS_REV_LESS_V3=y
CONFIG_ESP32P4_REV_MIN_1=y
CONFIG_ESPTOOLPY_FLASHMODE_QIO=y
CONFIG_ESPTOOLPY_FLASHSIZE_16MB=y
CONFIG_PARTITION_TABLE_CUSTOM=y
CONFIG_PARTITION_TABLE_CUSTOM_FILENAME=\"partitions.csv\"
CONFIG_SPIRAM=y
CONFIG_SPIRAM_SPEED_200M=y
CONFIG_IDF_EXPERIMENTAL_FEATURES=y
CONFIG_SPIRAM_MALLOC_RESERVE_INTERNAL=65536
CONFIG_SPIRAM_MEMTEST=n
CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y
CONFIG_LCD_DSI_ISR_CACHE_SAFE=y
CONFIG_SPI_FLASH_SUPPORT_WINBOND_CHIP=y
CONFIG_BOOTLOADER_REGION_PROTECTION_ENABLE=n
CONFIG_BOOTLOADER_CUSTOM_RESERVE_RTC=y
CONFIG_BOOTLOADER_CUSTOM_RESERVE_RTC_SIZE=0x100
CONFIG_COMPILER_OPTIMIZATION_PERF=y
CONFIG_FREERTOS_HZ=1000
CONFIG_ESP_MAIN_TASK_STACK_SIZE=65536
CONFIG_ESP_TASK_WDT_INIT=n
CONFIG_ESP_SYSTEM_PANIC_PRINT_HALT=y
CONFIG_BT_ENABLED=n
";

/// One build: the unit, the libraries' C sources, the directory the program's relative paths
/// start from, and the tier.
pub(crate) struct EspApp<'a> {
    pub c: &'a str,
    pub shims: &'a [String],
    pub program_dir: &'a Path,
    /// What to pack, relative to `program_dir`: files, and directories taken whole.
    pub assets: &'a [String],
    pub ship: bool,
}

/// Where ESP-IDF is: `IDF_PATH`, else the newest `~/.espressif/v*/esp-idf`.  And the tools
/// directory its `export.sh` reads: `IDF_TOOLS_PATH`, else `~/.espressif/tools` when the
/// installer put them there, else ESP-IDF's own default.
fn idf_paths() -> Option<(PathBuf, Option<PathBuf>)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let idf = match std::env::var_os("IDF_PATH").filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => {
            let base = home.as_ref()?.join(".espressif");
            let mut found: Vec<PathBuf> = fa::read_dir(&base)
                .ok()?
                .into_iter()
                .map(|p| p.os_spelling())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('v'))
                        && fa::is_file(p.join("esp-idf").join("export.sh"))
                })
                .map(|p| p.join("esp-idf"))
                .collect();
            found.sort();
            found.pop()?
        }
    };
    if !fa::is_file(idf.join("export.sh")) {
        return None;
    }
    let tools = std::env::var_os("IDF_TOOLS_PATH")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let t = home?.join(".espressif").join("tools");
            fa::is_dir(t.join("python_env")).then_some(t)
        });
    Some((idf, tools))
}

/// How to install ESP-IDF, for a machine without it.
fn idf_cure() -> String {
    format!(
        "install ESP-IDF {IDF_VERSION} for the esp32p4 (TANMATSU.md § A loft program as an app): \
         git clone --branch {IDF_VERSION} --recursive --depth 1 --shallow-submodules \
         https://github.com/espressif/esp-idf.git ~/.espressif/{IDF_VERSION}/esp-idf && \
         ~/.espressif/{IDF_VERSION}/esp-idf/install.sh esp32p4 — or set IDF_PATH"
    )
}

/// Every file under `dir`, as its path relative to `root` with `/` between names, sorted so
/// the packed table is the same on every run.
fn files_under(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fa::read_dir(dir) else {
        return;
    };
    for e in entries {
        let p = e.os_spelling();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if fa::is_dir(&p) {
            files_under(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push((rel.join("/"), p));
        }
    }
    out.sort();
}

/// `loft_assets.c`: each file `assets` names (a directory: every file under it) as a byte
/// array, and the table `loft_esp.h` serves them from by their path relative to the program.
/// Answers the C and the packed byte count.
///
/// # Errors
/// A named asset that does not exist, or one that cannot be read.
fn assets_c(program_dir: &Path, assets: &[String]) -> Result<(String, usize), String> {
    let mut files = Vec::new();
    for a in assets {
        let p = program_dir.join(a);
        if fa::is_dir(&p) {
            files_under(program_dir, &p, &mut files);
        } else if fa::is_file(&p) {
            files.push((a.trim_start_matches("./").replace('\\', "/"), p));
        } else if a != "assets" {
            return Err(format!(
                "--assets: `{a}` is not a file or directory beside the program"
            ));
        }
    }
    files.sort();
    files.dedup();
    let mut c = String::from(
        "/* The program's assets, packed by `loft --native-esp`: served read-only by loft_esp.h. */\n\
         #include <stddef.h>\n\nstruct loft_asset { const char *path; const unsigned char *data; size_t size; };\n",
    );
    let mut total = 0usize;
    for (i, (_, path)) in files.iter().enumerate() {
        let bytes = fa::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        total += bytes.len();
        let _ = write!(
            c,
            "static const unsigned char loft_asset_{i}[{}] = {{",
            bytes.len().max(1)
        );
        if bytes.is_empty() {
            c.push('0');
        }
        for (k, b) in bytes.iter().enumerate() {
            if k % 24 == 0 {
                c.push('\n');
            }
            let _ = write!(c, "{b},");
        }
        c.push_str("\n};\n");
    }
    c.push_str("const struct loft_asset loft_assets[] = {\n");
    for (i, (rel, _)) in files.iter().enumerate() {
        let quoted = rel.replace('\\', "\\\\").replace('"', "\\\"");
        let size = fa::metadata(&files[i].1).map(|m| m.len()).unwrap_or(0);
        let _ = writeln!(c, "    {{ \"{quoted}\", loft_asset_{i}, {size} }},");
    }
    if files.is_empty() {
        c.push_str("    { 0, 0, 0 },\n");
    }
    let _ = writeln!(c, "}};\nconst size_t loft_asset_count = {};", files.len());
    Ok((c, total))
}

/// A path as CMake reads it inside a quoted argument.
fn cmake_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").replace('"', "\\\"")
}

/// Write the ESP-IDF project for `app` into `out`.  Answers the packed asset byte count.
///
/// # Errors
/// A file that cannot be read or written.
pub(crate) fn write_project(app: &EspApp<'_>, out: &Path) -> Result<usize, String> {
    let main = out.join("main");
    fa::create_dir_all(&main).map_err(|e| format!("cannot create {}: {e}", main.display()))?;
    let put = |path: PathBuf, text: &str| {
        // A file that already says this is left alone, so ESP-IDF rebuilds only what changed.
        if fa::read_to_string(&path).is_ok_and(|old| old == text) {
            return Ok(());
        }
        fa::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    };
    put(
        out.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.16)\n\
         # Only the components the app names (and what they need) are built.\n\
         set(MINIMAL_BUILD ON)\n\
         include($ENV{IDF_PATH}/tools/cmake/project.cmake)\n\
         project(application)\n",
    )?;
    put(out.join("partitions.csv"), PARTITIONS)?;
    put(out.join("sdkconfig.defaults"), SDKCONFIG)?;
    put(
        main.join("idf_component.yml"),
        &format!(
            "dependencies:\n  idf: \">={}\"\n  {BSP}\n",
            &IDF_VERSION[1..]
        ),
    )?;
    put(main.join("loft_app.c"), app.c)?;
    let (assets, packed) = assets_c(app.program_dir, app.assets)?;
    put(main.join("loft_assets.c"), &assets)?;
    let mut srcs = String::from("\"loft_app.c\" \"loft_assets.c\"");
    let mut dirs = String::from("\".\"");
    for s in app.shims {
        let p = Path::new(s);
        let _ = write!(srcs, " \"{}\"", cmake_path(p));
        if let Some(d) = p.parent() {
            let d = format!(" \"{}\"", cmake_path(d));
            if !dirs.contains(&d) {
                dirs.push_str(&d);
            }
        }
    }
    // The ship tier drops the call chain a fault names its frames by, as on every C target.
    let tier = if app.ship { "" } else { " -DLOFT_CALL_CHAIN" };
    put(
        main.join("CMakeLists.txt"),
        &format!(
            "idf_component_register(\n    SRCS {srcs}\n    INCLUDE_DIRS {dirs}\n    \
             PRIV_REQUIRES badge-bsp vfs nvs_flash esp_timer esp_lcd esp_driver_gpio heap\n)\n\
             # The program's C and its libraries' are loft's, not the app author's: they are built\n\
             # without warnings, as every C target builds them.\n\
             target_compile_options(${{COMPONENT_LIB}} PRIVATE -w{tier})\n"
        ),
    )?;
    Ok(packed)
}

/// Build the project in `out` with `idf.py`, answering the app binary.
///
/// # Errors
/// No ESP-IDF, or a build that fails, with its output's tail.
pub(crate) fn build_project(out: &Path) -> Result<PathBuf, String> {
    let Some((idf, tools)) = idf_paths() else {
        return Err(format!("--native-esp needs ESP-IDF; {}", idf_cure()));
    };
    let script = format!(
        "set -e\n. \"{}/export.sh\" >/dev/null\nidf.py -C \"{}\" -B \"{}\" build\n",
        idf.display(),
        out.display(),
        out.join("build").display()
    );
    let mut cmd = Spawn::new(Program::bash());
    cmd.push_arg("-c").push_arg(&script);
    cmd.push_env("IDF_PATH", &idf);
    if let Some(t) = &tools {
        cmd.push_env("IDF_TOOLS_PATH", t);
    }
    let ran = cmd
        .run(b"")
        .map_err(|e| format!("--native-esp: cannot run idf.py ({e})"))?;
    if !ran.status.success() {
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr)
        );
        let tail: Vec<&str> = said.lines().rev().take(40).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "--native-esp: the ESP-IDF build failed (the project is {}):\n{}",
            out.display(),
            tail.join("\n")
        ));
    }
    Ok(out.join("build").join("application.bin"))
}
