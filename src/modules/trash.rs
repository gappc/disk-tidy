//! The desktop trash (~/.local/share/Trash).
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::home;
use crate::model::{Category, Item, Remover, Safety};
use crate::size::dir_sizes;
use crate::time::parse_timestamp;

pub fn trash_dir() -> PathBuf {
    home().join(".local/share/Trash")
}

pub fn collect(base: &Path) -> Vec<Category> {
    let mut cat = Category::new(
        "trash",
        "Papierkorb",
        base,
        Safety::Safe,
        "schon gelöscht; danach endgültig weg",
    )
    .remover(Remover::Trash {
        base: base.to_path_buf(),
    });
    if let Ok(entries) = fs::read_dir(base.join("files")) {
        let entries: Vec<fs::DirEntry> = entries.flatten().collect();
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path()).collect();
        let sizes = dir_sizes(&paths.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        for ((e, path), size) in entries.iter().zip(paths).zip(sizes) {
            let mut info = e.file_name();
            info.push(".trashinfo");
            cat.items.push(Item {
                size,
                id: path.display().to_string(),
                label: e.file_name().to_string_lossy().into_owned(),
                path: Some(path),
                modified: deletion_date(&base.join("info").join(info)),
                ..Default::default()
            });
        }
    }
    vec![cat.sorted()]
}

/// `DeletionDate=` of a `.trashinfo` file, as Unix seconds.
fn deletion_date(info: &Path) -> Option<u64> {
    let text = fs::read_to_string(info).ok()?;
    let date = text.lines().find_map(|l| l.strip_prefix("DeletionDate="))?;
    parse_timestamp(date.trim())
}

/// Deletes direct children of base/files and their base/info/<name>.trashinfo; refuses others.
/// Messages go to `out`; a failing `out` (closed pipe) does not stop the removal.
pub fn remove_entries(items: &[Item], base: &Path, out: &mut dyn Write) -> bool {
    let files = base.join("files");
    let mut ok = true;
    for i in items {
        let target = i.target();
        let p = target.as_path();
        // file_name() is None for a path ending in "..", which parent() would otherwise accept
        let (Some(name), true) = (p.file_name(), p.parent() == Some(files.as_path())) else {
            let _ = writeln!(out, "  Übersprungen (nicht im Papierkorb): {}", p.display());
            ok = false;
            continue;
        };
        let is_dir = fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        let removed = if is_dir {
            fs::remove_dir_all(p)
        } else {
            fs::remove_file(p)
        };
        let mut info_name = name.to_os_string();
        info_name.push(".trashinfo");
        let info = base.join("info").join(info_name);
        let removed = removed.and_then(|()| match fs::remove_file(&info) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        });
        if let Err(e) = removed {
            let _ = writeln!(out, "  Fehler: {}: {e}", p.display());
            ok = false;
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Remover, Safety};
    use std::fs;

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn trash() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        touch(&b.join("files/a.txt"), &[b'x'; 5000]);
        touch(&b.join("files/dir/b.txt"), b"x");
        touch(&b.join("info/a.txt.trashinfo"), b"x");
        touch(&b.join("info/dir.trashinfo"), b"x");
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn collect_and_remove() {
        let dir = trash();
        let b = dir.path();
        let cats = collect(b);
        let c = &cats[0];
        assert_eq!(
            (c.key.as_str(), c.title.as_str(), c.safety),
            ("trash", "Papierkorb", Safety::Safe)
        );
        assert_eq!(c.why, "schon gelöscht; danach endgültig weg");
        assert_eq!(
            c.remover,
            Remover::Trash {
                base: b.to_path_buf()
            }
        );
        let mut labels: Vec<&str> = c.items.iter().map(|i| i.label.as_str()).collect();
        labels.sort();
        assert_eq!(labels, ["a.txt", "dir"]);
        assert!(remove_entries(&c.items, b, &mut Vec::new()));
        assert!(names(&b.join("files")).is_empty());
        assert!(names(&b.join("info")).is_empty());
    }

    #[test]
    fn deletion_date_is_modified() {
        let dir = trash();
        let b = dir.path();
        touch(
            &b.join("info/a.txt.trashinfo"),
            b"[Trash Info]\nPath=/home/u/a.txt\nDeletionDate=2026-10-08T12:00:00\n",
        );
        let cats = collect(b);
        let a = cats[0].items.iter().find(|i| i.label == "a.txt").unwrap();
        assert_eq!(a.modified, Some(1791460800));
        let d = cats[0].items.iter().find(|i| i.label == "dir").unwrap();
        assert_eq!(d.modified, None); // info file without a date
    }

    #[test]
    fn refuses_path_outside_trash() {
        let dir = trash();
        let outside = dir.path().join("keep.txt");
        touch(&outside, b"x");
        let item = Item {
            size: 0,
            id: outside.display().to_string(),
            label: String::new(),
            path: None,
            ..Default::default()
        };
        assert!(!remove_entries(&[item], dir.path(), &mut Vec::new()));
        assert!(outside.exists());
    }

    #[test]
    fn refuses_parent_reference() {
        let dir = trash();
        let sneaky = dir.path().join("files/..");
        let item = Item {
            size: 0,
            id: sneaky.display().to_string(),
            label: String::new(),
            path: None,
            ..Default::default()
        };
        assert!(!remove_entries(&[item], dir.path(), &mut Vec::new()));
        assert!(dir.path().join("files/a.txt").exists());
    }

    #[test]
    fn trash_unicode_names() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        touch(&b.join("files/01 · Einstieg & Startscreen.png"), b"png");
        touch(&b.join("files/a b/c"), b"x");
        touch(
            &b.join("info/01 · Einstieg & Startscreen.png.trashinfo"),
            b"x",
        );
        touch(&b.join("info/a b.trashinfo"), b"x");
        let cats = collect(b);
        let mut labels: Vec<&str> = cats[0].items.iter().map(|i| i.label.as_str()).collect();
        labels.sort();
        assert_eq!(labels, ["01 · Einstieg & Startscreen.png", "a b"]);
        assert!(remove_entries(&cats[0].items, b, &mut Vec::new()));
        assert!(names(&b.join("files")).is_empty());
        assert!(names(&b.join("info")).is_empty());
    }

    #[test]
    fn non_utf8_names_are_removed() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        touch(&b.join("files").join(name), b"x");
        touch(
            &b.join("info")
                .join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt.trashinfo")),
            b"x",
        );
        let cats = collect(b);
        assert_eq!(cats[0].items.len(), 1);
        assert!(remove_entries(&cats[0].items, b, &mut Vec::new()));
        assert!(names(&b.join("files")).is_empty());
        assert!(names(&b.join("info")).is_empty());
    }

    /// stdout closed (`disk-tidy clean --yes | head -1`): messages fail, removal goes on.
    struct ClosedPipe;
    impl std::io::Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn messages_go_to_writer_and_closed_pipe_does_not_stop_removal() {
        let dir = trash();
        let b = dir.path();
        let outside = Item {
            size: 0,
            id: b.join("keep.txt").display().to_string(),
            label: String::new(),
            path: None,
            ..Default::default()
        };
        let mut items = vec![outside];
        items.extend(collect(b)[0].items.clone());
        let mut out = Vec::new();
        assert!(!remove_entries(&items[..1], b, &mut out));
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("  Übersprungen (nicht im Papierkorb): ")
        );
        assert!(!remove_entries(&items, b, &mut ClosedPipe));
        assert!(names(&b.join("files")).is_empty());
    }

    #[test]
    fn shared_hardlinks_count_once_in_total() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        touch(&b.join("files/a"), &[1u8; 200_000]);
        fs::hard_link(b.join("files/a"), b.join("files/b")).unwrap();
        let total = collect(b)[0].total();
        assert!((200_000..400_000).contains(&total), "{total}");
    }

    #[test]
    fn missing_trash_is_empty_category() {
        let cats = collect(Path::new("/nonexistent/trash"));
        assert!(cats[0].items.is_empty());
    }
}
