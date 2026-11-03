// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN204 — `--native-esp`: a loft program as an ESP-IDF app for the Tanmatsu
//! (TANMATSU.md § A loft program as an app).  The project holds the unit
//! `loft::c::emit_esp_program` writes, the `[c]` parts of the libraries it uses, the
//! program's `assets/` packed as a read-only filesystem, the Tanmatsu's partition table and a
//! configuration tuned for speed.  `idf.py build` makes `build/application.bin`, which the
//! launcher installs into `appfs` (badgelink).  No Rust part of any library is built or
//! linked: the C backend loads none.

use crate::app_files::{Reason, asset_files, named_assets};
use loft::file_access as fa;
use loft::platform::process::{Program, Spawn};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The BSP the libraries' Tanmatsu sections are written against, as the Tanmatsu app template
/// pins it.
const BSP: &str = "badgeteam/badge-bsp: \"=1.6.4\"";

/// The ESP-IDF release the template builds with.
const IDF_VERSION: &str = "v6.0.2";

/// The partition table the build checks the app against.  It is never installed: badgelink
/// uploads `application.bin` alone, into the launcher's `appfs` (8 MB on a Tanmatsu, shared by
/// every installed app; TANMATSU.md § The flash layout).  So the one app partition here is as
/// large as `appfs`, where the launcher's own table has two 2 MB slots for itself that would
/// refuse any app over 2 MB.
const PARTITIONS: &str = "\
# Name,   Type, SubType, Offset,   Size,   Flags
nvs,      data, nvs,     0x19000,  16K,
phy_init, data, phy,     0x1F000,  4K,
app,      app,  factory, 0x20000,  8128K,
";

/// The configuration: the Tanmatsu's board (P4 revision 1, 16 MB QIO flash, PSRAM at 200 MHz,
/// the console on the USB serial port, the RTC memory the launcher reads which app to start
/// from, no test of all 32 MB of PSRAM at every start), then speed: `-O2`, a 1 kHz scheduler tick so a
/// `poll(15)` waits 15 ms, a main task stack for loft's recursion, and no task watchdog to stop
/// a long computation.  No radio: an app that uses none starts faster and is smaller.  For size:
/// ESP-IDF's assertions keep their check but drop their message, no table of error names, and
/// only warnings and errors logged (a loft app's own `@boot` lines are not log lines).
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
CONFIG_COMPILER_OPTIMIZATION_ASSERTIONS_SILENT=y
CONFIG_ESP_ERR_TO_NAME_LOOKUP=n
CONFIG_LOG_DEFAULT_LEVEL_WARN=y
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

/// The board's parts `loft_esp.h` can start, the BSP prefix whose use asks for each, and
/// whether it is started whatever the C calls: the clock (a program's wall time reads it, no
/// BSP call names it), the SD card's supply (files reach it through the VFS), power and the
/// LEDs (they hold the board's defaults).  The display is `loft_esp.h`'s own: it blanks it.
const PARTS: &[(&str, &str, bool)] = &[
    ("DISPLAY", "bsp_display_", true),
    ("INPUT", "bsp_input_", false),
    ("POWER", "bsp_power_", true),
    ("RTC", "bsp_rtc_", true),
    ("AUDIO", "bsp_audio_", false),
    ("LED", "bsp_led_", true),
    ("STORAGE", "bsp_storage_", true),
    ("ORIENTATION", "bsp_orientation_", false),
    ("SENSOR", "bsp_sensor_", false),
    ("CATT", "bsp_catt_", false),
    ("SAO", "bsp_sao_", false),
];

/// A C file's text with every file it includes by a local name (`#include "x.c"`) after it,
/// as the compiler reads it: where a library keeps a platform's section.
fn with_includes(path: &Path, seen: &mut Vec<PathBuf>) -> String {
    if seen.iter().any(|p| p == path) {
        return String::new();
    }
    seen.push(path.to_path_buf());
    let Ok(text) = fa::read_to_string(path) else {
        return String::new();
    };
    let mut all = text.clone();
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("#include \"") {
            if let (Some(name), Some(dir)) = (rest.split('"').next(), path.parent()) {
                all.push_str(&with_includes(&dir.join(name), seen));
            }
        }
    }
    all
}

/// Which parts the app starts: each the libraries' C calls (with the first file that calls it),
/// and those started regardless.  Answers `(part, why)` for the started and the left-off ones.
fn board_parts(shims: &[String]) -> (Vec<(&'static str, String)>, Vec<&'static str>) {
    let sources: Vec<(String, String)> = shims
        .iter()
        .map(|s| {
            let p = Path::new(s);
            let name = p
                .file_name()
                .map_or_else(|| s.clone(), |n| n.to_string_lossy().into_owned());
            (name, with_includes(p, &mut Vec::new()))
        })
        .collect();
    let mut on = Vec::new();
    let mut off = Vec::new();
    for (part, prefix, always) in PARTS {
        let caller = sources
            .iter()
            .find(|(_, t)| t.contains(prefix))
            .map(|(n, _)| n.clone());
        match (caller, always) {
            (Some(c), _) => on.push((*part, format!("called by {c}"))),
            (None, true) => on.push((*part, "always".to_string())),
            (None, false) => off.push(*part),
        }
    }
    (on, off)
}

fn assets_c(files: &[(String, PathBuf)]) -> Result<(String, usize), String> {
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
    let (on, off) = board_parts(app.shims);
    let mut unit = String::from(
        "/* The board's parts this app starts: found in the C it holds (loft-esp-report.txt). */\n",
    );
    for (part, _) in &on {
        let _ = writeln!(unit, "#define LOFT_ESP_{part} 1");
    }
    unit.push_str(app.c);
    put(main.join("loft_app.c"), &unit)?;
    let named: Vec<(String, PathBuf, Reason)> = if app.assets.is_empty() {
        named_assets(app.program_dir, app.c)
    } else {
        asset_files(app.program_dir, app.assets)?
            .into_iter()
            .map(|(r, p)| (r, p, "named by --assets".to_string()))
            .collect()
    };
    let files: Vec<(String, PathBuf)> = named
        .iter()
        .map(|(r, p, _)| (r.clone(), p.clone()))
        .collect();
    let (assets, packed) = assets_c(&files)?;
    let mut report = String::from(
        "# What `loft --native-esp` put into this app, and why.\n\n## Board parts started\n",
    );
    for (part, why) in &on {
        let _ = writeln!(report, "{:12} {why}", part.to_lowercase());
    }
    let _ = writeln!(
        report,
        "\n## Board parts left off: nothing in the app's C calls them\n{}",
        off.iter()
            .map(|p| p.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let _ = writeln!(report, "\n## Files packed ({packed} bytes)");
    for (rel, path, why) in &named {
        let size = fa::metadata(path).map(|m| m.len()).unwrap_or(0);
        let _ = writeln!(report, "{size:9} {rel}  ({why})");
    }
    put(out.join("loft-esp-report.txt"), &report)?;
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
    // loft's C (the program and the runtime) is built for size: most of it runs once (loading,
    // set-up, the editor), and with the hot helpers forced inline it was the larger part of the
    // app.  The libraries' C, where a game's pixels are drawn and its pictures decoded, keeps the
    // project's -O2.
    put(
        main.join("CMakeLists.txt"),
        &format!(
            "idf_component_register(\n    SRCS {srcs}\n    INCLUDE_DIRS {dirs}\n    \
             PRIV_REQUIRES badge-bsp vfs nvs_flash esp_timer esp_lcd esp_driver_gpio heap\n)\n\
             # The program's C and its libraries' are loft's, not the app author's: they are built\n\
             # without warnings, as every C target builds them.\n\
             target_compile_options(${{COMPONENT_LIB}} PRIVATE -w{tier})\n\
             set_source_files_properties(loft_app.c PROPERTIES COMPILE_OPTIONS \"-Os;-DLOFT_SIZE\")\n"
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
    let map = fa::read_to_string(out.join("build").join("application.map")).unwrap_or_default();
    let report = out.join("loft-esp-report.txt");
    if let Ok(mut text) = fa::read_to_string(&report) {
        text.push_str(&size_report(&map));
        let _ = fa::write(&report, text);
    }
    Ok(out.join("build").join("application.bin"))
}

/// Where a section of the app came from, as the linker's map names its object: a member of the
/// app's own component (`loft_app.c`: the program, whose functions are `n_`/`t_`, or the
/// runtime; `loft_assets.c`; a library's C file), or an ESP-IDF archive (`libfreertos.a`).
fn origin(object: &str, section: &str) -> String {
    let member = object
        .rsplit_once('(')
        .map_or(object, |(_, m)| m.trim_end_matches(')'));
    let member = member.trim_end_matches(".obj").trim_end_matches(".o");
    if member == "loft_app.c" {
        let name = section.splitn(3, '.').nth(2).unwrap_or("");
        return if name.starts_with("n_") || name.starts_with("t_") {
            "the program (loft_app.c)".to_string()
        } else {
            "loft runtime (loft_app.c)".to_string()
        };
    }
    if object.contains("libmain.a") {
        return member.to_string();
    }
    object
        .rsplit('/')
        .next()
        .unwrap_or(object)
        .split('(')
        .next()
        .unwrap_or(object)
        .to_string()
}

/// What the app's flash and RAM hold, by where each part came from, and its largest functions —
/// read from the linker's map, so it describes the app as linked: what nothing calls is already
/// gone.  The part of the report that says where to look when an app is too large.
fn size_report(map: &str) -> String {
    use std::collections::HashMap;
    let Some(at) = map.find("Linker script and memory map") else {
        return String::new();
    };
    let lines: Vec<&str> = map[at..].lines().collect();
    let mut flash: HashMap<String, u64> = HashMap::new();
    let mut ram: HashMap<String, u64> = HashMap::new();
    let mut functions: Vec<(u64, String, String)> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix(' ') else {
            continue;
        };
        if !rest.starts_with('.') || rest.starts_with(' ') {
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
        let [section, addr, size, object, ..] = words.as_slice() else {
            continue;
        };
        let (Some(addr), Some(size)) = (
            addr.strip_prefix("0x")
                .and_then(|a| u64::from_str_radix(a, 16).ok()),
            size.strip_prefix("0x")
                .and_then(|z| u64::from_str_radix(z, 16).ok()),
        ) else {
            continue;
        };
        if size == 0 || addr == 0 {
            continue;
        }
        let from = origin(object, section);
        // What the image stores (code, constants, initial data) and what RAM is zeroed for; the
        // map's other sections (debug information, attributes) stay on the build machine.
        let stored = [
            ".text", ".literal", ".iram", ".rodata", ".srodata", ".data", ".sdata", ".dram",
            ".tdata",
        ];
        let zeroed = [".bss", ".sbss", ".tbss", ".noinit"];
        if zeroed.iter().any(|z| section.starts_with(z)) {
            *ram.entry(from).or_insert(0) += size;
        } else if stored.iter().any(|z| section.starts_with(z)) {
            if section.starts_with(".text") || section.starts_with(".iram") {
                let name = section.splitn(3, '.').nth(2).unwrap_or(section).to_string();
                functions.push((size, name, from.clone()));
            }
            *flash.entry(from).or_insert(0) += size;
        }
    }
    let mut out = String::from("\n## Image by origin (bytes stored; from the linker's map)\n");
    let mut by: Vec<(u64, String)> = flash.into_iter().map(|(o, s)| (s, o)).collect();
    by.sort_by(|a, b| b.cmp(a));
    for (size, o) in by.iter().take(16) {
        let _ = writeln!(out, "{size:9} {o}");
    }
    let rest: u64 = by.iter().skip(16).map(|(s, _)| s).sum();
    let _ = writeln!(
        out,
        "{rest:9} the other {} origins",
        by.len().saturating_sub(16)
    );
    let total: u64 = by.iter().map(|(s, _)| s).sum();
    let _ = writeln!(out, "{total:9} in all");
    let _ = writeln!(out, "\n## RAM taken at start, zeroed (.bss)");
    let mut by: Vec<(u64, String)> = ram.into_iter().map(|(o, s)| (s, o)).collect();
    by.sort_by(|a, b| b.cmp(a));
    for (size, o) in by.iter().take(8) {
        let _ = writeln!(out, "{size:9} {o}");
    }
    let _ = writeln!(out, "\n## Largest functions");
    functions.sort_by(|a, b| b.cmp(a));
    for (size, name, o) in functions.iter().take(20) {
        let _ = writeln!(out, "{size:9} {name}  ({o})");
    }
    out
}
