//! The desktop trash (~/.local/share/Trash).
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::home;
use crate::model::{Category, Item, Remover, Safety};
use crate::size::dir_size;

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
        for e in entries.flatten() {
            let path = e.path();
            cat.items.push(Item {
                size: dir_size(&path),
                id: path.display().to_string(),
                label: e.file_name().to_string_lossy().into_owned(),
                path: Some(path),
            });
        }
    }
    vec![cat.sorted()]
}

/// Deletes direct children of base/files and their base/info/<name>.trashinfo; refuses others.
pub fn remove_entries(items: &[Item], base: &Path) -> bool {
    let files = base.join("files");
    let mut ok = true;
    for i in items {
        let target = i.target();
        let p = target.as_path();
        // file_name() is None for a path ending in "..", which parent() would otherwise accept
        let (Some(name), true) = (p.file_name(), p.parent() == Some(files.as_path())) else {
            println!("  Übersprungen (nicht im Papierkorb): {}", p.display());
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
            println!("  Fehler: {}: {e}", p.display());
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
        assert!(remove_entries(&c.items, b));
        assert!(names(&b.join("files")).is_empty());
        assert!(names(&b.join("info")).is_empty());
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
        };
        assert!(!remove_entries(&[item], dir.path()));
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
        };
        assert!(!remove_entries(&[item], dir.path()));
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
        assert!(remove_entries(&cats[0].items, b));
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
        assert!(remove_entries(&cats[0].items, b));
        assert!(names(&b.join("files")).is_empty());
        assert!(names(&b.join("info")).is_empty());
    }

    #[test]
    fn missing_trash_is_empty_category() {
        let cats = collect(Path::new("/nonexistent/trash"));
        assert!(cats[0].items.is_empty());
    }
}
