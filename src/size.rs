//! Parsing, formatting and measuring sizes. Units are decimal (kB = 1000 B).
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// "1.17GB", "7.834kB", "3.7G", "500M", "2.36GB (virtual 3GB)" -> bytes. Unknown -> 0.
pub fn parse_size(s: &str) -> u64 {
    let s = s.trim_start();
    let num_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let Ok(value) = s[..num_end].parse::<f64>() else {
        return 0;
    };
    let rest = &s[num_end..];
    let unit = rest.trim_start();
    // A unit counts only when a word boundary follows it, as `\b` in the Python regex.
    let ends = |tail: &str| !tail.starts_with(|c: char| c.is_alphanumeric() || c == '_');
    let ends_with_b = |tail: &str| ends(tail) || tail.strip_prefix('B').is_some_and(ends);
    let factor = match unit.chars().next() {
        Some(p @ ('k' | 'K' | 'M' | 'G' | 'T')) if ends_with_b(&unit[1..]) => match p {
            'k' | 'K' => 1e3,
            'M' => 1e6,
            'G' => 1e9,
            _ => 1e12,
        },
        Some('B') if ends(&unit[1..]) => 1.0,
        _ if ends(rest) => 1.0,
        _ => return 0,
    };
    (value * factor).round() as u64
}

/// Seven columns for the number, then the unit, like the Python version.
pub fn fmt_size(n: u64) -> String {
    if n < 1000 {
        return format!("{n:7} B ");
    }
    let mut v = n as f64;
    for unit in ["kB", "MB", "GB", "TB"] {
        v /= 1000.0;
        if v < 1000.0 || unit == "TB" {
            return format!("{v:7.1} {unit}");
        }
    }
    unreachable!()
}

/// Allocated bytes per path like one `du -s a b ...`: st_blocks*512, symlinks not followed,
/// unreadable parts skipped, and a file hard-linked several times (also across the paths)
/// counts only once, for the first path, so the sizes add up to what removing all of them frees.
pub fn dir_sizes(paths: &[&Path]) -> Vec<u64> {
    let mut seen = HashSet::new();
    paths.iter().map(|p| walk(p, &mut seen)).collect()
}

fn walk(path: &Path, seen: &mut HashSet<(u64, u64)>) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.nlink() > 1 && !meta.is_dir() && !seen.insert((meta.dev(), meta.ino())) {
            continue;
        }
        total += meta.blocks() * 512;
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(&p)
        {
            stack.extend(entries.flatten().map(|e| e.path()));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn dir_size(path: &Path) -> u64 {
        dir_sizes(&[path])[0]
    }

    #[test]
    fn parse_docker_units() {
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("7.834kB"), 7834);
        assert_eq!(parse_size("1.17GB"), 1_170_000_000);
        assert_eq!(parse_size("2.36GB (virtual 3GB)"), 2_360_000_000);
        assert_eq!(parse_size(""), 0);
    }

    #[test]
    fn parse_short_units() {
        assert_eq!(parse_size("500M"), 500_000_000);
        assert_eq!(parse_size("3.7G"), 3_700_000_000);
    }

    #[test]
    fn format() {
        assert_eq!(fmt_size(1_600_000_000).trim(), "1.6 GB");
        assert_eq!(fmt_size(512).trim(), "512 B");
        assert_eq!(fmt_size(512), "    512 B ");
        assert_eq!(fmt_size(7834), "    7.8 kB");
        assert_eq!(fmt_size(5_000_000_000_000_000), " 5000.0 TB");
    }

    #[test]
    fn dir_size_counts_hardlink_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), vec![1u8; 100_000]).unwrap();
        fs::hard_link(dir.path().join("a"), dir.path().join("b")).unwrap();
        let n = dir_size(dir.path());
        assert!((100_000..200_000).contains(&n), "{n}");
    }

    #[test]
    fn dir_size_skips_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), vec![1u8; 100_000]).unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("f"), b"x").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let n = dir_size(dir.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(n >= 100_000, "{n}");
    }

    #[test]
    fn dir_sizes_count_shared_hardlink_once_like_du() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("f"), vec![1u8; 100_000]).unwrap();
        fs::hard_link(a.join("f"), b.join("f")).unwrap();
        let sizes = dir_sizes(&[a.as_path(), b.as_path()]);
        assert!(sizes[0] >= 100_000, "{sizes:?}");
        assert!(sizes[1] < 100_000, "{sizes:?}"); // du -s a b: the link counts in a only
        assert_eq!(dir_sizes(&[b.as_path()])[0], dir_size(&b));
    }

    #[test]
    fn dir_size_missing_path_is_zero() {
        assert_eq!(dir_size(Path::new("/nonexistent/disk-tidy")), 0);
    }

    #[test]
    fn dir_size_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let big = tempfile::tempdir().unwrap();
        fs::write(big.path().join("a"), vec![1u8; 1_000_000]).unwrap();
        std::os::unix::fs::symlink(big.path(), dir.path().join("link")).unwrap();
        assert!(dir_size(dir.path()) < 100_000);
    }
}
