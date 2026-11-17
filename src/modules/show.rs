//! Large paths that are only displayed, never deleted (e.g. a mail profile).
use std::path::Path;

use crate::config::{ShowConfig, expand_home, home};
use crate::model::{Category, Item, Safety};
use crate::size::dir_size;

pub fn collect(cfg: &ShowConfig) -> Vec<Category> {
    let paths: Vec<String> = cfg
        .paths
        .iter()
        .flat_map(|p| super::glob_sorted(&expand_home(p).display().to_string()))
        .collect();
    let Some(first) = paths.first() else {
        return Vec::new();
    };
    let mut cat = Category::new(
        "show",
        "Große Ordner zur Info",
        first,
        Safety::Info,
        "z. B. Mail-Profile; wird nie gelöscht",
    );
    cat.items = paths
        .iter()
        .map(|p| Item {
            size: dir_size(Path::new(p)),
            id: p.clone(),
            label: tilde(p),
        })
        .collect();
    vec![cat.sorted()]
}

/// Home directory shown as "~" (first occurrence, as Python's str.replace(home, "~", 1)).
fn tilde(path: &str) -> String {
    path.replacen(&home().display().to_string(), "~", 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Remover, Safety};
    use std::fs;

    #[test]
    fn show_expands_globs() {
        let dir = tempfile::tempdir().unwrap();
        for d in ["y/common", "x/common", ".hidden/common"] {
            fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let cfg = ShowConfig {
            paths: vec![format!("{}/*/common", dir.path().display())],
        };
        let cats = collect(&cfg);
        let c = &cats[0];
        assert_eq!(
            (c.title.as_str(), c.safety, &c.remover),
            ("Große Ordner zur Info", Safety::Info, &Remover::None)
        );
        assert_eq!(c.why, "z. B. Mail-Profile; wird nie gelöscht");
        let mut labels: Vec<String> = c.items.iter().map(|i| i.label.clone()).collect();
        labels.sort();
        let expect = |d: &str| dir.path().join(d).display().to_string();
        assert_eq!(labels, [expect("x/common"), expect("y/common")]);
        assert_eq!(c.path, dir.path().join("x/common"));
    }

    #[test]
    fn home_shown_as_tilde() {
        let home = crate::config::home().display().to_string();
        assert_eq!(tilde(&format!("{home}/snap/x/common")), "~/snap/x/common");
        assert_eq!(tilde("/srv/x"), "/srv/x");
    }

    #[test]
    fn nothing_found_is_no_category() {
        let cfg = ShowConfig {
            paths: vec!["/nonexistent/*/x".into()],
        };
        assert!(collect(&cfg).is_empty());
    }
}
