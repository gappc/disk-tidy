//! Categories of removable or displayed disk usage, and how to remove them.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Safety level, from harmless to never touched. Only `Safe` is removed by `disk-tidy clean`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    /// Recoverable without loss: caches, leftovers, build output of idle projects.
    Safe,
    /// Recoverable, but costs time or downloads; the user decides.
    Check,
    /// May hold data that cannot be recovered.
    Data,
    /// Displayed only, never removed.
    Info,
}

pub const SAFETY_ORDER: [Safety; 4] = [Safety::Safe, Safety::Check, Safety::Data, Safety::Info];

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Item {
    pub size: u64,
    /// What the remover acts on: an ID, a name or a path.
    pub id: String,
    pub label: String,
    /// The exact path for path-based removers; `id` is only its display form, which loses
    /// bytes that are not UTF-8.
    #[serde(skip)]
    pub path: Option<PathBuf>,
    /// Unix seconds of what makes the entry old: last project change, image creation,
    /// last cache use, deletion into the trash. Shown in the TUI only.
    #[serde(skip)]
    pub modified: Option<u64>,
    /// Short facts for the TUI's details popup, e.g. ("Projekt", "~/p/x").
    #[serde(skip)]
    pub facts: Vec<(String, String)>,
}

impl Item {
    /// Where a path-based remover acts: `path`, or `id` read as a path.
    pub fn target(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| PathBuf::from(&self.id))
    }
}

/// How the items of a category are removed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Remover {
    None,
    /// Run `argv`; with `with_ids` every item id is appended.
    Run {
        argv: Vec<String>,
        with_ids: bool,
        sudo: bool,
    },
    /// `sudo snap remove <name> --revision=<rev>` per item; id = "name=rev".
    SnapRevisions,
    /// Item ids are build directories.
    BuildDirs,
    /// Item ids are paths in `base/files`.
    Trash {
        base: PathBuf,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Category {
    pub key: String,
    /// The module that found it; set by `collect_all`.
    pub module: String,
    pub title: String,
    /// Any path on the partition this category lives on.
    pub path: PathBuf,
    pub safety: Safety,
    /// One line: what deleting it costs.
    pub why: String,
    /// Manual command, shown for categories that clean skips.
    pub hint: String,
    pub items: Vec<Item>,
    pub remover: Remover,
}

impl Category {
    pub fn new(
        key: &str,
        title: &str,
        path: impl Into<PathBuf>,
        safety: Safety,
        why: &str,
    ) -> Self {
        Category {
            key: key.into(),
            module: String::new(),
            title: title.into(),
            path: path.into(),
            safety,
            why: why.into(),
            hint: String::new(),
            items: Vec::new(),
            remover: Remover::None,
        }
    }

    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn remover(mut self, remover: Remover) -> Self {
        self.remover = remover;
        self
    }

    pub fn total(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    /// Removed by `disk-tidy clean`.
    pub fn is_clean(&self) -> bool {
        self.safety == Safety::Safe
    }

    /// Can be ticked in the TUI: GEFAHRLOS, or PRÜFEN with a remover.
    pub fn selectable(&self) -> bool {
        match self.safety {
            Safety::Safe => true,
            Safety::Check => self.remover != Remover::None,
            Safety::Data | Safety::Info => false,
        }
    }

    /// The remover takes no IDs, so the category is removed as a whole or not at all.
    pub fn all_or_nothing(&self) -> bool {
        matches!(
            self.remover,
            Remover::Run {
                with_ids: false,
                ..
            }
        )
    }

    /// Items by size, largest first.
    pub fn sorted(mut self) -> Self {
        self.items.sort_by_key(|i| std::cmp::Reverse(i.size));
        self
    }

    pub fn needs_sudo(&self) -> bool {
        matches!(
            self.remover,
            Remover::Run { sudo: true, .. } | Remover::SnapRevisions
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(size: u64) -> Item {
        Item {
            size,
            id: size.to_string(),
            label: String::new(),
            path: None,
            ..Default::default()
        }
    }

    #[test]
    fn sorted_and_total() {
        let mut c = Category::new("k", "T", "/", Safety::Safe, "w");
        c.items = vec![item(1), item(3)];
        let c = c.sorted();
        assert_eq!(c.items.iter().map(|i| i.size).collect::<Vec<_>>(), [3, 1]);
        assert_eq!(c.total(), 4);
    }

    #[test]
    fn only_safe_is_clean() {
        for s in SAFETY_ORDER {
            assert_eq!(
                Category::new("k", "T", "/", s, "w").is_clean(),
                s == Safety::Safe
            );
        }
    }

    #[test]
    fn needs_sudo_for_sudo_runs_and_snaps() {
        let c = |r| Category::new("k", "T", "/", Safety::Safe, "w").remover(r);
        let run = |sudo| Remover::Run {
            argv: vec!["x".into()],
            with_ids: false,
            sudo,
        };
        assert!(c(run(true)).needs_sudo());
        assert!(!c(run(false)).needs_sudo());
        assert!(c(Remover::SnapRevisions).needs_sudo());
        assert!(!c(Remover::BuildDirs).needs_sudo());
    }

    #[test]
    fn selectable_levels() {
        let c = |s, r| Category::new("k", "T", "/", s, "w").remover(r);
        assert!(c(Safety::Safe, Remover::None).selectable());
        assert!(c(Safety::Check, Remover::BuildDirs).selectable());
        assert!(!c(Safety::Check, Remover::None).selectable());
        assert!(!c(Safety::Data, Remover::BuildDirs).selectable());
        assert!(!c(Safety::Info, Remover::BuildDirs).selectable());
    }

    #[test]
    fn all_or_nothing_only_for_idless_run() {
        let c = |r| Category::new("k", "T", "/", Safety::Safe, "w").remover(r);
        let run = |with_ids| Remover::Run {
            argv: vec!["x".into()],
            with_ids,
            sudo: false,
        };
        assert!(c(run(false)).all_or_nothing());
        assert!(!c(run(true)).all_or_nothing());
        assert!(!c(Remover::BuildDirs).all_or_nothing());
        assert!(!c(Remover::None).all_or_nothing());
    }
}
