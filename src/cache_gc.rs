// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib: the collector for the on-disk caches
// `cache.rs` / `startup_cache.rs` populate — the half that gives space back.
// loft#861 — what the on-disk build caches hold, and what can be dropped.

//! The auto-native caches were append-only.
//!
//! Every entry is keyed on a value that moves forward with each installed loft build,
//! so a reinstall orphans the previous generation — and nothing collected them. The
//! remedy in loft's own documentation was a hand-typed `rm -rf`, in seven places,
//! which is a fair sign it was routine rather than exceptional. It also removes the
//! LIVE generation along with the dead ones, so the next build pays a full cold
//! rebuild: measured at 545 s of rustc CPU on one project's gate.
//!
//! This is the other side of it — [`survey`] says what is there and what a prune
//! would take, [`prune`] takes it.
//!
//! # What counts as dead, and how sure we are
//!
//! The two areas answer that question with different certainty, and the report says
//! which is which rather than presenting both as the same fact.
//!
//! * **`~/.loft/build-cache/<pkg>-<ver>/`** — *exact*. Each tree carries a
//!   `release/.loft-build-fp` sidecar stamped with
//!   [`native_artifact_cache_key`](crate::cache::native_artifact_cache_key), and the
//!   lookup that would reuse the tree requires that stamp to equal the running loft's
//!   key. A tree stamped with anything else can never be selected again; it will be
//!   rebuilt over on next use. No inversion, no heuristic — the same comparison the
//!   cache itself makes.
//!
//! * **`~/.loft/registry/<pkg>-<ver>/native-auto/`** — *conservative*, and usually
//!   already empty of anything to take. The artifact's name carries
//!   `mix_fp(loft_build_fingerprint(), layout_fp)`, and `layout_fp` is a property of
//!   the CONSUMER's type table, of which there is an open set — so "can this name be
//!   produced again?" is not decidable from the name, and no amount of care makes it
//!   so. What is applied is the retention already in force ([`KEEP_ARTIFACTS`] newest
//!   per package), extended to the directories nothing is building into any more: that
//!   sweep runs only after a successful build in the same directory, so a package that
//!   stopped being rebuilt keeps whatever tail it had.
//!
//! An age bound was tried here and removed. "Older than the running loft binary" reads
//! as a sound way to spot an earlier generation, and it is how the issue measured the
//! problem — but the reference point is the binary you happen to have invoked, so a
//! freshly built one dates *everything* and the tool reports 100 % reclaimable. That is
//! the `rm -rf` this exists to replace, wearing a measurement's clothes.
//!
//! So the honest summary of what the caches do: `native-auto` is bounded per package
//! already (this is what the post-build sweep was for), and grows with the number of
//! installed package VERSIONS rather than without limit; `build-cache` is one cargo
//! tree per package version, and its dead generations are exactly identifiable. A
//! pruned artifact that is wanted again is rebuilt — that is the whole risk, and it is
//! the same trade the post-build sweep already makes.

use std::path::{Path, PathBuf};

/// How many auto-native artifacts a package keeps, matching the post-build sweep in
/// `native_lib::prune_artifacts` so a manual prune and an automatic one cannot leave
/// the directory in two different states.
pub const KEEP_ARTIFACTS: usize = 8;

/// How certain an area's "dead" figure is — reported, because a heuristic presented
/// as a measurement is the thing that makes a cleanup tool untrustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The entry's own recorded key differs from the running loft's. It cannot be
    /// selected again.
    Exact,
    /// Bounded by retention only — reachability is not decidable here. Removing one
    /// costs a rebuild at worst.
    Conservative,
}

impl Basis {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Basis::Exact => "cannot be selected by this loft",
            Basis::Conservative => "beyond the newest 8 kept per package",
        }
    }
}

/// One cache area: what it holds, and what a prune would drop.
#[derive(Debug)]
pub struct Area {
    pub name: &'static str,
    pub root: PathBuf,
    pub items: u64,
    pub bytes: u64,
    pub dead_items: u64,
    pub dead_bytes: u64,
    pub basis: Basis,
    /// What `prune` removes. A directory for `build-cache`, an artifact stem's file
    /// set for `native-auto` — never a path outside [`root`](Self::root).
    dead: Vec<Removal>,
}

/// One removable thing, resolved at survey time so `prune` re-walks nothing.
#[derive(Debug)]
enum Removal {
    /// A whole directory tree (a `build-cache/<pkg>-<ver>/`).
    Tree(PathBuf),
    /// An artifact and the generated `.rs` / `.args` it was built from. Kept together
    /// because a `.so` without its source is a directory entry nothing can explain.
    Artifact { so: PathBuf, siblings: Vec<PathBuf> },
}

impl Removal {
    fn remove(&self) -> bool {
        match self {
            Removal::Tree(d) => crate::file_access::remove_dir_all(d).is_ok(),
            Removal::Artifact { so, siblings } => {
                let ok = crate::file_access::remove_file(so).is_ok();
                for s in siblings {
                    let _ = crate::file_access::remove_file(s);
                }
                ok
            }
        }
    }

    fn path(&self) -> &Path {
        match self {
            Removal::Tree(d) => d,
            Removal::Artifact { so, .. } => so,
        }
    }
}

impl Area {
    /// The live remainder — what stays after a prune, and what the next build reuses
    /// instead of paying rustc for.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.bytes.saturating_sub(self.dead_bytes)
    }
}

/// Total bytes under `dir`, following no symlinks. Best-effort: an unreadable entry
/// contributes nothing rather than aborting the walk, because a survey that refuses
/// to report because one file was busy is a survey nobody runs.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = crate::file_access::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for e in &entries {
        // Not following a link, as `DirEntry::file_type` / `metadata` did not.
        let Ok(m) = crate::file_access::symlink_metadata(e) else {
            continue;
        };
        let ft = m.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            total += dir_bytes(&e.os_spelling());
        } else {
            total += m.len();
        }
    }
    total
}

/// The platform's cdylib extension, matching what the build actually emits.
fn cdylib_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// Whether the running binary is the `loft` on `PATH`.
///
/// It decides whose question is being answered. The dead-generation test is "this
/// loft's [`native_artifact_cache_key`](crate::cache::native_artifact_cache_key) is not
/// the one stamped on the entry", and a development build has a different key from the
/// installed one — its `BUILD_ID` is the git HEAD. So `./target/release/loft cache
/// prune` would correctly report the INSTALLED loft's live generation as unusable *by
/// itself*, and deleting it costs every project on the machine a full cold rebuild.
///
/// `None` when neither path can be resolved, which is treated as "cannot confirm" and
/// gets the same guard as a mismatch.
#[must_use]
pub fn running_is_the_installed_loft() -> Option<bool> {
    let running = crate::file_access::try_plain_canonical(&std::env::current_exe().ok()?)?;
    let on_path = std::env::var_os("PATH").map(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(if cfg!(windows) { "loft.exe" } else { "loft" }))
            .find(|c| crate::file_access::is_file(c))
    })??;
    Some(crate::file_access::try_plain_canonical(&on_path)? == running)
}

/// `~/.loft/build-cache` — one cargo target tree per installed package version.
///
/// A tree whose `release/.loft-build-fp` is not the running loft's
/// [`native_artifact_cache_key`](crate::cache::native_artifact_cache_key) is a dead
/// generation: the lookup that would reuse it demands that exact stamp, so it will be
/// rebuilt over rather than read. Trees with no stamp at all are LEFT ALONE — an
/// absent sidecar is what an interrupted or foreign build looks like, and guessing
/// about it is how a cleanup tool deletes something that mattered.
#[must_use]
pub fn survey_build_cache(root: &Path, current_key: u64) -> Area {
    let mut area = Area {
        name: "build-cache",
        root: root.to_path_buf(),
        items: 0,
        bytes: 0,
        dead_items: 0,
        dead_bytes: 0,
        basis: Basis::Exact,
        dead: Vec::new(),
    };
    let Ok(entries) = crate::file_access::read_dir(root) else {
        return area;
    };
    for e in &entries {
        let dir = e.os_spelling();
        if !crate::file_access::is_dir(&dir) {
            continue;
        }
        let bytes = dir_bytes(&dir);
        area.items += 1;
        area.bytes += bytes;
        let stamped = crate::cache::native_artifact_stamped_fp(&dir.join("release"));
        if stamped.is_some_and(|fp| fp != current_key) {
            area.dead_items += 1;
            area.dead_bytes += bytes;
            area.dead.push(Removal::Tree(dir));
        }
    }
    area
}

/// `~/.loft/registry/*/native-auto` — the generated cdylibs, one per consumer type
/// layout per loft build.
///
/// One bound, for the reason the module doc gives: the keep window, which the
/// post-build sweep applies only to directories it is building INTO. Everything within
/// the window stays, so on a machine whose packages are all actively built this
/// reclaims nothing — which is the true answer, not a disappointing one.
///
/// The package SOURCE beside `native-auto/` is never touched, and neither is anything
/// that is not this family's artifact — a `[c] shim` cdylib shares the directory, is
/// content-keyed, is built exactly once, and is therefore always the oldest file there.
/// An unscoped sweep eats it first, and the next run dies with *"`#c` symbol 'x' not
/// found"*, naming neither the library nor the sweep.
#[must_use]
pub fn survey_native_auto(registry_root: &Path, keep: usize) -> Area {
    let mut area = Area {
        name: "registry native-auto",
        root: registry_root.to_path_buf(),
        items: 0,
        bytes: 0,
        dead_items: 0,
        dead_bytes: 0,
        basis: Basis::Conservative,
        dead: Vec::new(),
    };
    let Ok(pkgs) = crate::file_access::read_dir(registry_root) else {
        return area;
    };
    let ext = cdylib_ext();
    for pkg in &pkgs {
        let auto = pkg.os_spelling().join("native-auto");
        if !crate::file_access::is_dir(&auto) {
            continue;
        }
        // Group by artifact family, exactly as the post-build sweep does: the keep
        // window belongs to a package's own auto-native artifacts, not to whatever
        // else shares the directory.
        let mut families: std::collections::BTreeMap<
            String,
            Vec<(std::time::SystemTime, PathBuf, u64)>,
        > = std::collections::BTreeMap::new();
        let Ok(files) = crate::file_access::read_dir(&auto) else {
            continue;
        };
        for f in &files {
            if f.last_is_unspellable() || f.extension() != Some(ext) {
                continue;
            }
            let Some(stem) = f.file_stem() else {
                continue;
            };
            // `libloft_auto_<pkg>_<ver>_<fp>` (the leading `lib` is the platform's,
            // and is absent on Windows). Anything else in here belongs to someone
            // else — leave it alone.
            let unprefixed = stem.strip_prefix("lib").unwrap_or(stem);
            if !unprefixed.starts_with("loft_auto_") {
                continue;
            }
            let Some((family, _fp)) = unprefixed.rsplit_once('_') else {
                continue;
            };
            // Not following a link, as `DirEntry::metadata` did not.
            let Ok(meta) = crate::file_access::symlink_metadata(f) else {
                continue;
            };
            let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            families.entry(family.to_string()).or_default().push((
                mtime,
                f.os_spelling(),
                meta.len(),
            ));
        }
        for (_family, mut built) in families {
            // Newest first, so the keep window is the most recently BUILT — the same
            // ordering the post-build sweep uses.
            built.sort_by_key(|(t, p, _)| (std::cmp::Reverse(*t), p.clone()));
            for (i, (_mtime, so, size)) in built.iter().enumerate() {
                let siblings = sibling_sources(so);
                let sib_bytes: u64 = siblings
                    .iter()
                    .filter_map(|p| crate::file_access::metadata(p).ok().map(|m| m.len()))
                    .sum();
                let total = size + sib_bytes;
                area.items += 1;
                area.bytes += total;
                if i >= keep {
                    area.dead_items += 1;
                    area.dead_bytes += total;
                    area.dead.push(Removal::Artifact {
                        so: so.clone(),
                        siblings,
                    });
                }
            }
        }
    }
    area
}

/// The generated `.rs` and rustc `.args` an artifact was built from. They carry the
/// artifact's own hash in their names, so they are dead exactly when it is.
fn sibling_sources(so: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (so.parent(), crate::file_access::file_stem(so)) else {
        return Vec::new();
    };
    let unprefixed = stem.strip_prefix("lib").unwrap_or(&stem);
    vec![
        dir.join(format!("{unprefixed}.rs")),
        dir.join(format!("{unprefixed}.args")),
    ]
}

/// Take the same advisory lock a build takes, so a prune cannot race one.
///
/// Deleting an artifact a process already `dlopen`ed is safe on both unixes, and the
/// adoption path already refuses an artifact it cannot open (the loser rebuilds). The
/// lock is for the other order: a build that is mid-publish into this directory.
fn with_build_lock<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let Ok(lock) = crate::file_access::create(dir.join(".build.lock")) else {
        return f();
    };
    let held = lock.lock().is_ok();
    let out = f();
    if held {
        let _ = lock.unlock();
    }
    out
}

/// Remove everything an [`Area`] marked dead. Returns `(items, bytes)` actually
/// freed, which is what the caller reports — an intended figure and a delivered one
/// differ when something is busy, and saying the first is how a tool claims space it
/// did not give back.
#[must_use]
pub fn prune(area: &Area) -> (u64, u64) {
    let mut items = 0;
    let mut bytes = 0;
    for r in &area.dead {
        // Size it before removing it: afterwards there is nothing to measure.
        let size = match r {
            Removal::Tree(d) => dir_bytes(d),
            Removal::Artifact { so, siblings } => {
                crate::file_access::metadata(so).map_or(0, |m| m.len())
                    + siblings
                        .iter()
                        .filter_map(|p| crate::file_access::metadata(p).ok().map(|m| m.len()))
                        .sum::<u64>()
            }
        };
        let removed = match r {
            Removal::Artifact { .. } => r
                .path()
                .parent()
                .map_or_else(|| r.remove(), |dir| with_build_lock(dir, || r.remove())),
            Removal::Tree(_) => r.remove(),
        };
        if removed {
            items += 1;
            bytes += size;
        }
    }
    (items, bytes)
}

/// Everything in the area, dead or not — `loft cache prune --all`, the implemented
/// form of the `rm -rf` the docs used to prescribe. Scoped to what this module
/// surveyed, so it cannot reach a package's sources.
#[must_use]
pub fn prune_all(area: &Area) -> (u64, u64) {
    if area.name != "build-cache" {
        // native-auto: a full sweep is the keep-window sweep with no window.
        return prune(&survey_native_auto(&area.root, 0));
    }
    let mut items = 0;
    let mut bytes = 0;
    let Ok(entries) = crate::file_access::read_dir(&area.root) else {
        return (0, 0);
    };
    for e in &entries {
        let d = e.os_spelling();
        if crate::file_access::is_dir(&d) {
            let size = dir_bytes(&d);
            if crate::file_access::remove_dir_all(&d).is_ok() {
                items += 1;
                bytes += size;
            }
        }
    }
    (items, bytes)
}

/// `1.78 GB`, `412 MB`, `9.1 kB` — a size at the scale it happens to be.
#[must_use]
pub fn human(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // display only
    let b = bytes as f64;
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", b / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.0} MB", b / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} kB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("loft_cache_gc_{name}"));
        let _ = crate::file_access::remove_dir_all(&d);
        crate::file_access::create_dir_all(&d).expect("mkdir");
        d
    }

    /// A tree stamped with the running key stays; one stamped with anything else goes.
    /// That is the whole of the exact half, and both directions matter — a sweep that
    /// keeps everything is useless and one that takes the live generation costs the
    /// full cold rebuild the workaround was criticised for.
    #[test]
    fn build_cache_drops_only_the_generations_this_loft_cannot_select() {
        let root = tmp("bc");
        for (name, fp) in [("live-1.0.0", 42u64), ("dead-1.0.0", 7), ("dead-2.0.0", 9)] {
            let rel = root.join(name).join("release");
            crate::file_access::create_dir_all(&rel).unwrap();
            crate::file_access::write(rel.join("libx.rlib"), vec![0u8; 4096]).unwrap();
            crate::file_access::write(rel.join(".loft-build-fp"), fp.to_string()).unwrap();
        }
        let area = survey_build_cache(&root, 42);
        assert_eq!(area.items, 3, "every tree is counted");
        assert_eq!(
            area.dead_items, 2,
            "the two stamped with another key cannot be selected again"
        );
        let (items, _) = prune(&area);
        assert_eq!(items, 2);
        assert!(
            crate::file_access::exists(root.join("live-1.0.0")),
            "the live generation stays"
        );
        assert!(!crate::file_access::exists(root.join("dead-1.0.0")));
        assert!(!crate::file_access::exists(root.join("dead-2.0.0")));
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// An UNSTAMPED tree is what an interrupted build looks like. Deleting it on a
    /// guess is how a cleanup tool eats something that mattered, so it is left.
    #[test]
    fn an_unstamped_build_tree_is_left_alone() {
        let root = tmp("bc_unstamped");
        let rel = root.join("mystery-1.0.0").join("release");
        crate::file_access::create_dir_all(&rel).unwrap();
        crate::file_access::write(rel.join("libx.rlib"), vec![0u8; 128]).unwrap();
        let area = survey_build_cache(&root, 42);
        assert_eq!(area.items, 1);
        assert_eq!(
            area.dead_items, 0,
            "no stamp is not the same fact as a stale stamp"
        );
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// The keep window, and the thing it must not eat: a `[c] shim` cdylib shares the
    /// directory, is built once, and is therefore always the oldest file in it.
    #[test]
    fn native_auto_keeps_the_window_and_spares_a_foreign_cdylib() {
        let root = tmp("na");
        let auto = root.join("pkg-1.0.0").join("native-auto");
        crate::file_access::create_dir_all(&auto).unwrap();
        let ext = cdylib_ext();
        // Oldest first so mtimes are ordered by creation.
        let shim = auto.join(format!("libpkg_shim_abc.{ext}"));
        crate::file_access::write(&shim, vec![0u8; 1024]).unwrap();
        for i in 0..12 {
            let p = auto.join(format!("libloft_auto_pkg_1_0_0_{i:016x}.{ext}"));
            crate::file_access::write(&p, vec![0u8; 2048]).unwrap();
            crate::file_access::write(auto.join(format!("loft_auto_pkg_1_0_0_{i:016x}.rs")), "//")
                .unwrap();
            // Distinct mtimes, newest last.
            let t = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + i * 60);
            crate::file_access::open_with(&p, std::fs::File::options().write(true))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        let area = survey_native_auto(&root, KEEP_ARTIFACTS);
        assert_eq!(
            area.items, 12,
            "the shim is not this family's artifact and is not counted"
        );
        assert_eq!(area.dead_items, 4, "12 built, 8 kept");
        let (freed, _) = prune(&area);
        assert_eq!(freed, 4, "the survey's count is what prune delivers");
        assert!(
            crate::file_access::exists(&shim),
            "the `[c] shim` cdylib is the oldest file here and must survive — an \
             unscoped sweep eats it and the next run cannot find its `#c` symbols"
        );
        let left = crate::file_access::read_dir(&auto)
            .unwrap()
            .iter()
            .filter(|e| e.extension() == Some(ext))
            .filter(|e| {
                e.file_name()
                    .is_some_and(|n| n.starts_with("libloft_auto_"))
            })
            .count();
        assert_eq!(left, KEEP_ARTIFACTS, "the newest 8 stay");
        // The generated source goes with its artifact, never orphaned.
        assert!(!crate::file_access::exists(
            auto.join("loft_auto_pkg_1_0_0_0000000000000000.rs")
        ));
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// An unreadable root is a report of nothing, not a panic — `loft cache status` on
    /// a machine that never installed a package has to work.
    #[test]
    fn a_missing_root_surveys_as_empty() {
        let missing = std::env::temp_dir().join("loft_cache_gc_definitely_absent");
        let _ = crate::file_access::remove_dir_all(&missing);
        let a = survey_build_cache(&missing, 1);
        let b = survey_native_auto(&missing, 8);
        assert_eq!((a.items, a.bytes, b.items, b.bytes), (0, 0, 0, 0));
    }

    #[test]
    fn sizes_read_at_the_scale_they_are() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(2048), "2 kB");
        assert_eq!(human(5 * 1_048_576), "5 MB");
        assert_eq!(human(2 * 1_073_741_824), "2.00 GB");
    }
}
