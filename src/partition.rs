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

    #[test]
    fn disk_usage_root() {
        let u = disk_usage(Path::new("/"));
        assert!(u.total > 0 && u.free <= u.total, "{u:?}");
    }
}
