//! System (needs sudo to clean): disabled snap revisions, journal, apt cache, crash reports.
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::config::SystemConfig;
use crate::model::{Category, Item, Remover, Safety};
use crate::runner::Runner;
use crate::size::parse_size;

const SNAP_DIR: &str = "/var/lib/snapd/snaps";

/// `snap list --all` output -> [(name, rev)] of disabled revisions.
pub fn parse_disabled_snaps(snap_list: &str) -> Vec<(String, String)> {
    snap_list
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let disabled =
                cols.len() >= 3 && cols[cols.len() - 1].split(',').any(|n| n == "disabled");
            disabled.then(|| (cols[0].to_string(), cols[2].to_string()))
        })
        .collect()
}

/// "Archived and active journals take up 3.7G in the file system." -> bytes.
pub fn parse_journal_usage(text: &str) -> u64 {
    text.find("take up ")
        .map_or(0, |i| parse_size(&text[i + "take up ".len()..]))
}

pub fn collect(cfg: &SystemConfig, runner: &mut dyn Runner) -> Vec<Category> {
    vec![
        snaps(runner),
        journal(&cfg.journal_max, runner),
        apt_cache(),
        crashes(),
    ]
    .into_iter()
    .map(Category::sorted)
    .collect()
}

fn sudo(argv: &[&str], with_ids: bool) -> Remover {
    Remover::Run {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        with_ids,
        sudo: true,
    }
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.blocks() * 512)
}

fn snaps(runner: &mut dyn Runner) -> Category {
    let mut cat = Category::new(
        "snap_revisions",
        "System: alte Snap-Revisionen (sudo)",
        SNAP_DIR,
        Safety::Safe,
        "ältere Versionen, die Snap nur als Rückfall aufhebt",
    )
    .remover(Remover::SnapRevisions);
    let list = runner
        .output(&["snap", "list", "--all"])
        .unwrap_or_default();
    for (name, rev) in parse_disabled_snaps(&list) {
        cat.items.push(Item {
            size: file_size(&Path::new(SNAP_DIR).join(format!("{name}_{rev}.snap"))),
            id: format!("{name}={rev}"),
            label: format!("{name} (Revision {rev})"),
            path: None,
        });
    }
    cat
}

fn journal(max: &str, runner: &mut dyn Runner) -> Category {
    let mut cat = Category::new(
        "journal",
        &format!("System: Journal über {max} (sudo)"),
        "/var/log/journal",
        Safety::Safe,
        &format!("alte System-Logs; die neuesten {max} bleiben"),
    )
    .remover(sudo(
        &["journalctl", &format!("--vacuum-size={max}")],
        false,
    ));
    let usage = parse_journal_usage(
        &runner
            .output(&["journalctl", "--disk-usage"])
            .unwrap_or_default(),
    );
    let excess = usage.saturating_sub(parse_size(max));
    if excess > 0 {
        cat.items.push(Item {
            size: excess,
            id: "journal".into(),
            label: format!(
                "systemd-Journal, belegt insgesamt {:.1} GB",
                usage as f64 / 1e9
            ),
            path: None,
        });
    }
    cat
}

fn apt_cache() -> Category {
    let mut cat = Category::new(
        "apt_cache",
        "System: apt-Paketcache (sudo)",
        "/var/cache/apt",
        Safety::Safe,
        "bereits installierte .deb-Pakete, werden bei Bedarf neu geladen",
    )
    .remover(sudo(&["apt-get", "clean"], false));
    let debs = super::glob_sorted("/var/cache/apt/archives/*.deb");
    if !debs.is_empty() {
        cat.items.push(Item {
            size: debs.iter().map(|d| file_size(Path::new(d))).sum(),
            id: "apt".into(),
            label: format!("{} heruntergeladene .deb-Pakete", debs.len()),
            path: None,
        });
    }
    cat
}

fn crashes() -> Category {
    let mut cat = Category::new(
        "crash",
        "System: Absturzberichte (sudo)",
        "/var/crash",
        Safety::Safe,
        "alte Berichte abgestürzter Programme",
    )
    .remover(sudo(&["rm", "-f", "--"], true));
    for f in super::glob_sorted("/var/crash/*.crash") {
        let name = Path::new(&f)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        cat.items.push(Item {
            size: file_size(Path::new(&f)),
            id: f,
            label: name,
            path: None,
        });
    }
    cat
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Remover, Safety};
    use crate::runner::FakeRunner;

    const SNAPS: &str = "Name    Version  Rev   Tracking       Publisher   Notes\n\
        core22  2026     2987  latest/stable  canonical** base,disabled\n\
        core22  2026     3129  latest/stable  canonical** base\n\
        gimp    3.2      561   latest/stable  gimp**      disabled\n";

    fn cats(runner: &mut FakeRunner, journal_max: &str) -> Vec<Category> {
        collect(
            &SystemConfig {
                journal_max: journal_max.into(),
            },
            runner,
        )
    }

    #[test]
    fn disabled_snaps() {
        assert_eq!(
            parse_disabled_snaps(SNAPS),
            [
                ("core22".to_string(), "2987".to_string()),
                ("gimp".into(), "561".into())
            ]
        );
    }

    #[test]
    fn journal_usage() {
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 3.7G in the file system."),
            3_700_000_000
        );
        assert_eq!(parse_journal_usage("garbage"), 0);
    }

    #[test]
    fn snap_items() {
        let mut r = FakeRunner::default();
        r.outputs.insert("snap list --all".into(), SNAPS.into());
        let cats = cats(&mut r, "500M");
        let snaps = cats.iter().find(|c| c.key == "snap_revisions").unwrap();
        let mut ids: Vec<&str> = snaps.items.iter().map(|i| i.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["core22=2987", "gimp=561"]);
        assert!(snaps.items.iter().any(|i| i.label == "gimp (Revision 561)"));
        assert_eq!(snaps.remover, Remover::SnapRevisions);
        assert_eq!(snaps.title, "System: alte Snap-Revisionen (sudo)");
    }

    #[test]
    fn journal_command_uses_config() {
        let mut r = FakeRunner::default();
        r.outputs.insert(
            "journalctl --disk-usage".into(),
            "Archived and active journals take up 3.7G in the file system.".into(),
        );
        let cats = cats(&mut r, "1G");
        let j = cats.iter().find(|c| c.key == "journal").unwrap();
        assert_eq!(
            j.remover,
            Remover::Run {
                argv: vec!["journalctl".into(), "--vacuum-size=1G".into()],
                with_ids: false,
                sudo: true
            }
        );
        assert_eq!(j.title, "System: Journal über 1G (sudo)");
        assert_eq!(j.why, "alte System-Logs; die neuesten 1G bleiben");
        assert_eq!(j.items[0].size, 2_700_000_000);
        assert_eq!(j.items[0].label, "systemd-Journal, belegt insgesamt 3.7 GB");
    }

    #[test]
    fn journal_below_max_has_no_item() {
        let mut r = FakeRunner::default();
        r.outputs.insert(
            "journalctl --disk-usage".into(),
            "journals take up 100M in".into(),
        );
        let cats = cats(&mut r, "500M");
        assert!(
            cats.iter()
                .find(|c| c.key == "journal")
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn all_system_categories_are_safe_sudo() {
        let cats = cats(&mut FakeRunner::default(), "500M");
        let keys: Vec<&str> = cats.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["snap_revisions", "journal", "apt_cache", "crash"]);
        assert!(
            cats.iter()
                .all(|c| c.safety == Safety::Safe && c.needs_sudo())
        );
        let crash = &cats[3];
        assert_eq!(
            crash.remover,
            Remover::Run {
                argv: vec!["rm".into(), "-f".into(), "--".into()],
                with_ids: true,
                sudo: true
            }
        );
    }
}
