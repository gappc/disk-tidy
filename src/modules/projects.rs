//! Build directories in project trees (node_modules, target, .venv).
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::{ProjectsConfig, expand_home, home};
use crate::model::{Category, Item, Remover, Safety};
use crate::size::dir_sizes;

/// Build dir name -> files of which at least one must sit next to it. Without such a file the
/// directory is not treated as build output (a folder that happens to be called "target" stays).
pub const BUILD_DIRS: [(&str, &[&str]); 3] = [
    ("node_modules", &["package.json"]),
    ("target", &["Cargo.toml", "pom.xml"]),
    (
        ".venv",
        &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "uv.lock",
            "Pipfile",
        ],
    ),
];
const SKIP: &str = ".git";
const DAY: u64 = 86400;

/// Build dirs below root. Never descends into build dirs or .git, never follows symlinks.
pub fn find_build_dirs(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_for_build_dirs(root, &mut found);
    found
}

/// Top-down like Python's os.walk: build dirs of a directory first, then its subdirectories.
fn walk_for_build_dirs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut keep = Vec::new();
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name();
        if !ft.is_dir() || name == SKIP {
            continue; // symlinks have their own file type and are never followed
        }
        if let Some(markers) = markers(&name.to_string_lossy()) {
            if markers.iter().any(|m| dir.join(m).exists()) {
                found.push(e.path());
            }
            continue; // never descend into build dirs, marked or not
        }
        keep.push(e.path());
    }
    for d in keep {
        walk_for_build_dirs(&d, found);
    }
}

fn markers(name: &str) -> Option<&'static [&'static str]> {
    BUILD_DIRS.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
}

/// The project a build dir belongs to: the nearest git repo up to root, else its parent.
pub fn project_of(build_dir: &Path, root: &Path) -> PathBuf {
    let parent = build_dir.parent().unwrap_or(build_dir);
    let mut p = parent;
    loop {
        if p.join(".git").exists() {
            return p.to_path_buf();
        }
        match p.parent() {
            Some(up) if p != root => p = up,
            _ => return parent.to_path_buf(),
        }
    }
}

/// Newest mtime of anything in project outside build dirs and .git internals (plus .git/index
/// and .git/HEAD). Stops early once something newer than `newer_than` is found.
pub fn last_change(project: &Path, newer_than: Option<SystemTime>) -> SystemTime {
    let mut newest = UNIX_EPOCH;
    for f in [".git/index", ".git/HEAD"] {
        if let Ok(t) = fs::metadata(project.join(f)).and_then(|m| m.modified()) {
            newest = newest.max(t);
        }
    }
    let mut stack = vec![project.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if ft.is_dir() {
                let name = e.file_name();
                if name != SKIP && markers(&name.to_string_lossy()).is_none() {
                    subdirs.push(path);
                }
                continue;
            }
            if ft.is_symlink() && path.is_dir() {
                continue; // a link to a directory: neither followed nor counted
            }
            if let Ok(t) = fs::symlink_metadata(&path).and_then(|m| m.modified()) {
                newest = newest.max(t);
            }
        }
        if newer_than.is_some_and(|limit| newest > limit) {
            break;
        }
        // Python's os.walk visits subdirectories in listing order.
        stack.extend(subdirs.into_iter().rev());
    }
    newest
}

/// (build dir, project) pairs -> [stale (Safe), active (Check)].
pub fn classify_dirs(
    found: Vec<(PathBuf, PathBuf)>,
    sizes: &dyn Fn(&Path) -> u64,
    stale_days: u64,
    now: SystemTime,
) -> Vec<Category> {
    let path = found.first().map_or_else(home, |(b, _)| b.clone());
    let mut stale = Category::new(
        "build_stale",
        &format!("Build-Ordner, Projekt ≥{stale_days} Tage unberührt"),
        &path,
        Safety::Safe,
        "mit npm install / cargo build / uv sync wiederherstellbar",
    )
    .remover(Remover::BuildDirs);
    let mut active = Category::new(
        "build_active",
        "Build-Ordner in aktiven Projekten",
        &path,
        Safety::Check,
        "du arbeitest daran; danach neu installieren oder bauen",
    )
    .hint("rm -rf <ordner>  (oder cargo clean)")
    .remover(Remover::BuildDirs);

    let cutoff = now - Duration::from_secs(stale_days * DAY);
    let home = home();
    let mut changed: HashMap<PathBuf, SystemTime> = HashMap::new();
    for (b, project) in found {
        let last = *changed
            .entry(project.clone())
            .or_insert_with(|| last_change(&project, Some(cutoff)));
        let days = now.duration_since(last).map_or(0, |d| d.as_secs() / DAY);
        let label = match b.strip_prefix(&home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => b.display().to_string(),
        };
        let item = Item {
            size: sizes(&b),
            id: b.display().to_string(),
            label: format!("{label:72}  zuletzt geändert vor {days} Tagen"),
            path: Some(b),
        };
        if last <= cutoff {
            &mut stale
        } else {
            &mut active
        }
        .items
        .push(item);
    }
    vec![stale.sorted(), active.sorted()]
}

pub fn collect(cfg: &ProjectsConfig, now: SystemTime) -> Vec<Category> {
    let mut found = Vec::new();
    for root in &cfg.roots {
        let root = expand_home(root);
        if root.is_dir() {
            found.extend(find_build_dirs(&root).into_iter().map(|b| {
                let p = project_of(&b, &root);
                (b, p)
            }));
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    // One pass over all build dirs, like one `du -s`: shared hard links count once.
    let dirs: Vec<&Path> = found.iter().map(|(b, _)| b.as_path()).collect();
    let sizes: HashMap<PathBuf, u64> = dirs
        .iter()
        .map(|d| d.to_path_buf())
        .zip(dir_sizes(&dirs))
        .collect();
    classify_dirs(
        found,
        &|b| sizes.get(b).copied().unwrap_or(0),
        cfg.stale_days,
        now,
    )
}

/// Deletes the listed build dirs; refuses symlinks, non-directories and other names.
/// Messages go to `out`; a failing `out` (closed pipe) does not stop the removal.
pub fn remove_build_dirs(items: &[Item], out: &mut dyn Write) -> bool {
    let mut ok = true;
    for i in items {
        let target = i.target();
        let p = target.as_path();
        let is_build_dir = fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
            && p.file_name()
                .is_some_and(|n| markers(&n.to_string_lossy()).is_some());
        if !is_build_dir {
            let _ = writeln!(out, "  Übersprungen (kein Build-Ordner): {}", p.display());
            ok = false;
            continue;
        }
        if let Err(e) = fs::remove_dir_all(p) {
            let _ = writeln!(out, "  Fehler: {}: {e}", p.display());
            ok = false;
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn touch(path: &Path, age_days: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * DAY);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// Same tree as ProjectsTest in the Python tests.
    fn tree() -> (tempfile::TempDir, ProjectsConfig) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        // old: untouched 60 days, git repo, node_modules + nested package
        touch(&r.join("old/package.json"), 60);
        touch(&r.join("old/.git/HEAD"), 60);
        touch(&r.join("old/node_modules/dep/index.js"), 1); // build dir age ignored
        touch(&r.join("old/node_modules/dep/node_modules/x.js"), 1);
        // fresh: edited yesterday, Rust project
        touch(&r.join("fresh/Cargo.toml"), 60);
        touch(&r.join("fresh/src/main.rs"), 1);
        touch(&r.join("fresh/target/debug/bin"), 60);
        // unmarked: a folder called target without Cargo.toml/pom.xml -> ignored
        touch(&r.join("docs/target/important.txt"), 90);
        // monorepo: git root edited recently, package untouched -> still active
        touch(&r.join("mono/.git/HEAD"), 60);
        touch(&r.join("mono/README.md"), 2);
        touch(&r.join("mono/pkgs/a/package.json"), 60);
        touch(&r.join("mono/pkgs/a/node_modules/y.js"), 60);
        let cfg = ProjectsConfig {
            roots: vec![r.display().to_string()],
            stale_days: 28,
        };
        (dir, cfg)
    }

    fn rel(cat: &Category, root: &Path) -> Vec<String> {
        let mut v: Vec<String> = cat
            .items
            .iter()
            .map(|i| {
                Path::new(&i.id)
                    .strip_prefix(root)
                    .unwrap()
                    .display()
                    .to_string()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn classification() {
        let (dir, cfg) = tree();
        let cats = collect(&cfg, SystemTime::now());
        assert_eq!(cats[0].key, "build_stale");
        assert_eq!(cats[0].safety, Safety::Safe);
        assert_eq!(cats[0].remover, Remover::BuildDirs);
        assert_eq!(cats[0].title, "Build-Ordner, Projekt ≥28 Tage unberührt");
        assert_eq!(rel(&cats[0], dir.path()), ["old/node_modules"]);
        assert_eq!(cats[1].key, "build_active");
        assert_eq!(cats[1].safety, Safety::Check);
        assert_eq!(cats[1].hint, "rm -rf <ordner>  (oder cargo clean)");
        assert_eq!(cats[1].remover, Remover::BuildDirs);
        assert_eq!(
            rel(&cats[1], dir.path()),
            ["fresh/target", "mono/pkgs/a/node_modules"]
        );
    }

    #[test]
    fn label_shows_days() {
        let (dir, cfg) = tree(); // in /tmp, so the label is the full path
        let cats = collect(&cfg, SystemTime::now());
        let path = dir.path().join("old/node_modules").display().to_string();
        assert_eq!(
            cats[0].items[0].label,
            format!("{path:72}  zuletzt geändert vor 60 Tagen")
        );
    }

    #[test]
    fn label_relative_to_home() {
        let home = crate::config::home();
        let b = home.join("p/x/node_modules");
        let cats = classify_dirs(vec![(b, home.join("p/x"))], &|_| 1, 28, SystemTime::now());
        let item = &cats.iter().find(|c| !c.items.is_empty()).unwrap().items[0];
        assert!(
            item.label
                .starts_with(&format!("{:72}  zuletzt", "~/p/x/node_modules")),
            "{}",
            item.label
        );
    }

    #[test]
    fn remove_deletes_only_listed_build_dirs() {
        let (dir, cfg) = tree();
        let cats = collect(&cfg, SystemTime::now());
        assert!(remove_build_dirs(&cats[0].items, &mut Vec::new()));
        let r = dir.path();
        assert!(!r.join("old/node_modules").exists());
        assert!(r.join("old/package.json").exists());
        assert!(r.join("fresh/target").exists());
    }

    #[test]
    fn remove_refuses_non_build_dir() {
        let (dir, _) = tree();
        let docs = dir.path().join("docs");
        let item = Item {
            size: 0,
            id: docs.display().to_string(),
            label: "docs".into(),
            path: None,
        };
        let mut out = Vec::new();
        assert!(!remove_build_dirs(&[item], &mut out));
        assert!(docs.exists());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("  Übersprungen (kein Build-Ordner): {}\n", docs.display())
        );
    }

    #[test]
    fn no_roots() {
        let (dir, _) = tree();
        let cfg = ProjectsConfig {
            roots: vec![dir.path().join("missing").display().to_string()],
            stale_days: 28,
        };
        assert!(collect(&cfg, SystemTime::now()).is_empty());
    }

    #[test]
    fn symlinks_are_not_followed() {
        let (dir, _) = tree();
        let r = dir.path();
        touch(&r.join("linked/package.json"), 60);
        symlink(r.join("fresh/target"), r.join("linked/node_modules")).unwrap();
        symlink("..", r.join("old/loop")).unwrap();
        let mut found: Vec<String> = find_build_dirs(r)
            .iter()
            .map(|p| p.strip_prefix(r).unwrap().display().to_string())
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "fresh/target",
                "mono/pkgs/a/node_modules",
                "old/node_modules"
            ]
        );
        // last_change does not run in circles either
        let _ = last_change(&r.join("old"), None);
    }

    #[test]
    fn remove_refuses_symlink() {
        let (dir, _) = tree();
        let r = dir.path();
        touch(&r.join("linked/package.json"), 60);
        let link = r.join("linked/node_modules");
        symlink(r.join("fresh/target"), &link).unwrap();
        let item = Item {
            size: 0,
            id: link.display().to_string(),
            label: String::new(),
            path: None,
        };
        assert!(!remove_build_dirs(&[item], &mut Vec::new()));
        assert!(r.join("fresh/target/debug/bin").exists());
        assert!(link.exists());
    }

    #[test]
    fn build_dirs_with_spaces_and_non_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        touch(&r.join("01 · Einstieg & Startscreen/package.json"), 60);
        touch(&r.join("01 · Einstieg & Startscreen/node_modules/x.js"), 60);
        let cfg = ProjectsConfig {
            roots: vec![r.display().to_string()],
            stale_days: 28,
        };
        let cats = collect(&cfg, SystemTime::now());
        assert_eq!(
            rel(&cats[0], r),
            ["01 · Einstieg & Startscreen/node_modules"]
        );
        assert!(remove_build_dirs(&cats[0].items, &mut Vec::new()));
        assert!(!r.join("01 · Einstieg & Startscreen/node_modules").exists());
        assert!(r.join("01 · Einstieg & Startscreen/package.json").exists());
    }

    #[test]
    fn build_dirs_under_non_utf8_project_are_removed() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
        touch(&project.join("package.json"), 60);
        touch(&project.join("node_modules/x.js"), 60);
        let cfg = ProjectsConfig {
            roots: vec![dir.path().display().to_string()],
            stale_days: 28,
        };
        let cats = collect(&cfg, SystemTime::now());
        assert_eq!(cats[0].items.len(), 1);
        assert!(remove_build_dirs(&cats[0].items, &mut Vec::new()));
        assert!(!project.join("node_modules").exists());
    }

    #[test]
    fn shared_hardlinks_count_once_in_total() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for p in ["p1", "p2"] {
            touch(&r.join(p).join("package.json"), 60);
            fs::create_dir_all(r.join(p).join("node_modules")).unwrap();
        }
        fs::write(r.join("p1/node_modules/f"), vec![1u8; 200_000]).unwrap();
        fs::hard_link(r.join("p1/node_modules/f"), r.join("p2/node_modules/f")).unwrap();
        let cfg = ProjectsConfig {
            roots: vec![r.display().to_string()],
            stale_days: 28,
        };
        let total = collect(&cfg, SystemTime::now())[0].total();
        assert!((200_000..400_000).contains(&total), "{total}");
    }

    #[test]
    fn project_is_git_root_or_parent() {
        let (dir, _) = tree();
        let r = dir.path();
        assert_eq!(
            project_of(&r.join("mono/pkgs/a/node_modules"), r),
            r.join("mono")
        );
        assert_eq!(project_of(&r.join("fresh/target"), r), r.join("fresh"));
    }
}
