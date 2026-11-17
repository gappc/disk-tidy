//! Which partition a path lives on, and how full it is.
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub free: u64,
    pub used: u64,
}

/// Nearest existing ancestor, resolved, then up while the parent is on the same device.
pub fn mount_point(path: &Path) -> PathBuf {
    let mut p = path;
    while !p.exists() {
        match p.parent() {
            Some(parent) => p = parent,
            None => break,
        }
    }
    let mut p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let dev = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
    while let Some(parent) = p.parent() {
        if dev(parent) != dev(&p) {
            break;
        }
        p = parent.to_path_buf();
    }
    p
}

/// free = space available to unprivileged users, used = blocks in use (as `df`).
pub fn disk_usage(mount: &Path) -> Usage {
    match rustix::fs::statvfs(mount) {
        Ok(v) => Usage {
            total: v.f_blocks * v.f_frsize,
            free: v.f_bavail * v.f_frsize,
            used: (v.f_blocks - v.f_bfree) * v.f_frsize,
        },
        Err(_) => Usage {
            total: 0,
            free: 0,
            used: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_point_of_missing_path() {
        assert_eq!(
            mount_point(Path::new("/proc/does/not/exist")),
            PathBuf::from("/proc")
        );
        assert_eq!(mount_point(Path::new("/")), PathBuf::from("/"));
    }

    /// Same grouping as the Python version (`os.path.ismount` walking up from the resolved path),
    /// on whatever mounts this machine has. Skipped without python3.
    #[test]
    fn mount_point_matches_python_ismount() {
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let mut paths: Vec<PathBuf> = mounts
            .lines()
            .filter_map(|l| l.split(' ').nth(4))
            .filter(|m| {
                !m.starts_with("/proc/")
                    && !m.starts_with("/sys/")
                    && !m.starts_with("/run/snapd/ns")
            })
            .map(|m| PathBuf::from(m.replace("\\040", " ")))
            .collect();
        paths.extend(paths.clone().into_iter().map(|m| m.join("does/not/exist")));
        paths.push(crate::config::home().join("snap"));
        paths.push(PathBuf::from("/var/lib/docker"));
        let script = "import os,sys\nfrom pathlib import Path\nfor a in sys.argv[1:]:\n    p=Path(a)\n    while not p.exists(): p=p.parent\n    p=p.resolve()\n    while not os.path.ismount(p): p=p.parent\n    print(p)";
        let Ok(out) = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .args(&paths)
            .output()
        else {
            return;
        };
        let expected = String::from_utf8(out.stdout).unwrap();
        let got: String = paths
            .iter()
            .map(|p| format!("{}\n", mount_point(p).display()))
            .collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn disk_usage_root() {
        let u = disk_usage(Path::new("/"));
        assert!(u.total > 0 && u.free <= u.total, "{u:?}");
    }
}
