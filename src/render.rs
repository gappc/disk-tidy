//! The text overview: per partition, grouped by safety level.
use std::path::{Path, PathBuf};

use crate::model::{Category, SAFETY_ORDER, Safety};
use crate::partition::{Usage, mount_point};
use crate::size::fmt_size;

pub struct Style {
    /// Stdout is a terminal and NO_COLOR is unset.
    pub colour: bool,
}

impl Style {
    fn paint(&self, text: &str, code: &str) -> String {
        if self.colour {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

/// Heading per safety level: (symbol, title, explanation, colour).
fn level(safety: Safety) -> (&'static str, &'static str, &'static str, &'static str) {
    match safety {
        Safety::Safe => ("✓", "GEFAHRLOS", "löscht `disk-tidy clean`", "32"),
        Safety::Check => (
            "⚠",
            "PRÜFEN",
            "wiederherstellbar, kostet aber Zeit oder Downloads; nur von Hand",
            "33",
        ),
        Safety::Data => ("✗", "VORSICHT", "kann Daten enthalten; nur von Hand", "31"),
        Safety::Info => ("ℹ", "NUR INFO", "wird nie gelöscht", "2"),
    }
}

const TITLE_WIDTH: usize = 44;
const INDENT: &str = "               "; // 15 spaces

/// Categories grouped by the partition they live on, in order of first appearance.
pub fn by_partition(cats: &[Category]) -> Vec<(PathBuf, Vec<&Category>)> {
    let mut groups: Vec<(PathBuf, Vec<&Category>)> = Vec::new();
    for c in cats {
        let mount = mount_point(&c.path);
        match groups.iter_mut().find(|(m, _)| *m == mount) {
            Some((_, group)) => group.push(c),
            None => groups.push((mount, vec![c])),
        }
    }
    groups
}

fn partition_line(out: &mut String, mount: &Path, style: &Style, usage: &dyn Fn(&Path) -> Usage) {
    let u = usage(mount);
    let share = if u.used + u.free == 0 {
        0.0
    } else {
        u.used as f64 / (u.used + u.free) as f64
    };
    let line = format!(
        "\n{}  ·  {} frei von {} ({:.0}% belegt)",
        mount.display(),
        fmt_size(u.free).trim(),
        fmt_size(u.total).trim(),
        share * 100.0
    );
    out.push_str(&style.paint(&line, "1"));
    out.push('\n');
}

fn level_block(
    out: &mut String,
    safety: Safety,
    group: &[&Category],
    details: bool,
    top: usize,
    style: &Style,
) {
    let mut cats: Vec<&Category> = group
        .iter()
        .copied()
        .filter(|c| c.safety == safety)
        .collect();
    if cats.is_empty() {
        return;
    }
    cats.sort_by_key(|c| std::cmp::Reverse(c.total()));
    let (symbol, title, explanation, code) = level(safety);
    let total: u64 = cats.iter().map(|c| c.total()).sum();
    out.push_str(&style.paint(
        &format!("\n  {symbol} {title} – {explanation}"),
        &format!("{code};1"),
    ));
    out.push_str(&style.paint(&format!("   {}", fmt_size(total).trim()), code));
    out.push('\n');
    for c in cats {
        out.push_str(&format!(
            "    {}  {:TITLE_WIDTH$}  {}\n",
            fmt_size(c.total()),
            c.title,
            style.paint(&c.why, "2")
        ));
        if !details {
            continue;
        }
        if !c.hint.is_empty() {
            out.push_str(&style.paint(&format!("{INDENT}Befehl: {}", c.hint), "2"));
            out.push('\n');
        }
        let shown = if top == 0 {
            c.items.len()
        } else {
            top.min(c.items.len())
        };
        for i in &c.items[..shown] {
            out.push_str(&format!("{INDENT}{}  {}\n", fmt_size(i.size), i.label));
        }
        if c.items.len() > shown {
            out.push_str(&format!(
                "{INDENT}... und {} weitere\n",
                c.items.len() - shown
            ));
        }
    }
}

pub fn render_preview(
    cats: &[Category],
    details: bool,
    top: usize,
    style: &Style,
    usage: &dyn Fn(&Path) -> Usage,
) -> String {
    let mut out = String::new();
    let groups = by_partition(cats);
    for (mount, group) in &groups {
        partition_line(&mut out, mount, style, usage);
        for safety in SAFETY_ORDER {
            level_block(&mut out, safety, group, details, top, style);
        }
    }
    let safe: Vec<String> = groups
        .iter()
        .filter(|(_, g)| g.iter().any(|c| c.is_clean()))
        .map(|(m, g)| {
            let total: u64 = g.iter().filter(|c| c.is_clean()).map(|c| c.total()).sum();
            format!("{} {}", m.display(), fmt_size(total).trim())
        })
        .collect();
    out.push('\n');
    if !safe.is_empty() {
        let line = format!(
            "Gefahrlos freizugeben: {}  →  disk-tidy clean",
            safe.join(", ")
        );
        out.push_str(&style.paint(&line, "32;1"));
        out.push('\n');
    }
    if !details {
        out.push_str(&style.paint(
            "Einzelne Einträge und Befehle zum Löschen von Hand: disk-tidy --details",
            "2",
        ));
        out.push('\n');
    }
    if details && cats.iter().any(|c| c.title.starts_with("Docker")) {
        let note = "Docker-Images: nur der eigene Anteil (UniqueSize) zählt; geteilte Layer und \
                    geteilter Build-Cache werden erst frei, wenn alle Nutzer weg sind.";
        out.push_str(&style.paint(note, "2"));
        out.push('\n');
    }
    out
}

/// What `clean` shows before asking: the GEFAHRLOS level per partition and the total.
pub fn render_clean_summary(
    cats: &[Category],
    details: bool,
    top: usize,
    style: &Style,
    usage: &dyn Fn(&Path) -> Usage,
) -> String {
    let todo: Vec<Category> = cats.iter().filter(|c| c.is_clean()).cloned().collect();
    let mut out = String::new();
    for (mount, group) in by_partition(&todo) {
        partition_line(&mut out, &mount, style, usage);
        level_block(&mut out, Safety::Safe, &group, details, top, style);
    }
    let total: u64 = todo.iter().map(|c| c.total()).sum();
    out.push_str(&format!(
        "\nInsgesamt etwa {}. Nur die Kategorien oben werden gelöscht{}\n",
        fmt_size(total).trim(),
        if details {
            "."
        } else {
            "; einzelne Einträge: disk-tidy clean --details."
        }
    ));
    if todo.iter().any(|c| c.needs_sudo()) {
        out.push_str("Die Kategorien mit (sudo) fragen gegebenenfalls nach deinem Passwort.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Remover};
    use crate::modules::docker::fixture;

    const PLAIN: Style = Style { colour: false };
    const COLOUR: Style = Style { colour: true };

    fn usage(_: &Path) -> Usage {
        Usage {
            total: 1_000_000_000_000,
            free: 100_000_000_000,
            used: 900_000_000_000,
        }
    }

    fn cat(key: &str, safety: Safety, size: u64) -> Category {
        let mut c = Category::new(
            key,
            &format!("Titel {key}"),
            "/",
            safety,
            &format!("warum {key}"),
        )
        .hint(&format!("befehl {key}"));
        c.items = vec![Item {
            size,
            id: key.into(),
            label: format!("eintrag {key}"),
        }];
        c
    }

    fn cats() -> Vec<Category> {
        vec![
            cat("info", Safety::Info, 4_000_000_000),
            cat("check", Safety::Check, 3_000_000_000),
            cat("safe", Safety::Safe, 1_000_000_000),
            cat("data", Safety::Data, 2_000_000_000),
        ]
    }

    /// The Docker fixture as the Python golden files were made: root "/", empty categories dropped.
    fn docker_cats() -> Vec<Category> {
        let cfg = fixture::cfg();
        crate::modules::docker::classify(&fixture::df(), &cfg, Path::new("/"))
            .into_iter()
            .filter(|c| !c.items.is_empty())
            .collect()
    }

    fn golden(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(format!("{name}.txt"));
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn levels_in_order_each_category_under_its_level() {
        let text = render_preview(&cats(), false, 5, &PLAIN, &usage);
        let pos = |s: &str| {
            text.find(s)
                .unwrap_or_else(|| panic!("{s} missing in {text}"))
        };
        let heads = ["GEFAHRLOS", "PRÜFEN", "VORSICHT", "NUR INFO"].map(pos);
        assert!(heads.is_sorted(), "{heads:?}");
        for (key, level, next) in [("safe", 0, 1), ("check", 1, 2), ("data", 2, 3)] {
            let p = pos(&format!("Titel {key}"));
            assert!(heads[level] < p && p < heads[next], "{key}");
        }
    }

    #[test]
    fn summary_counts_only_safe() {
        assert!(
            render_preview(&cats(), false, 5, &PLAIN, &usage)
                .contains("Gefahrlos freizugeben: / 1.0 GB")
        );
    }

    #[test]
    fn entries_only_with_details() {
        assert!(!render_preview(&cats(), false, 5, &PLAIN, &usage).contains("eintrag safe"));
        let detailed = render_preview(&cats(), true, 5, &PLAIN, &usage);
        assert!(detailed.contains("eintrag safe"));
        assert!(detailed.contains("Befehl: befehl check"));
    }

    #[test]
    fn golden_preview() {
        let c = docker_cats();
        assert_eq!(
            render_preview(&c, false, 1, &PLAIN, &usage),
            golden("preview")
        );
        assert_eq!(
            render_preview(&c, true, 1, &PLAIN, &usage),
            golden("preview_details")
        );
    }

    #[test]
    fn golden_preview_colour() {
        let c = docker_cats();
        assert_eq!(
            render_preview(&c, false, 1, &COLOUR, &usage),
            golden("preview_colour")
        );
        assert_eq!(
            render_preview(&c, true, 1, &COLOUR, &usage),
            golden("preview_details_colour")
        );
    }

    #[test]
    fn golden_clean_summary() {
        let c = docker_cats();
        assert_eq!(
            render_clean_summary(&c, false, 1, &PLAIN, &usage),
            golden("clean_summary")
        );
        assert_eq!(
            render_clean_summary(&c, true, 1, &PLAIN, &usage),
            golden("clean_summary_details")
        );
    }

    #[test]
    fn no_ansi_without_colour() {
        let c = docker_cats();
        assert!(!render_preview(&c, true, 0, &PLAIN, &usage).contains("\x1b["));
        assert!(!render_clean_summary(&c, true, 0, &PLAIN, &usage).contains("\x1b["));
    }

    #[test]
    fn top_zero_shows_all_entries() {
        let text = render_preview(&docker_cats(), true, 0, &PLAIN, &usage);
        assert!(text.contains("gobuild_dbdata") && !text.contains("weitere"));
    }

    #[test]
    fn clean_summary_mentions_sudo() {
        let mut c = cat("journal", Safety::Safe, 1);
        c.remover = Remover::Run {
            argv: vec!["journalctl".into()],
            with_ids: false,
            sudo: true,
        };
        let text = render_clean_summary(&[c], false, 1, &PLAIN, &usage);
        assert!(
            text.ends_with(
                "Die Kategorien mit (sudo) fragen gegebenenfalls nach deinem Passwort.\n"
            ),
            "{text}"
        );
    }
}
