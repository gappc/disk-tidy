//! `disk-tidy clean`: show the GEFAHRLOS part, ask, remove it.
use std::io::{BufRead, Write};
use std::path::Path;

use crate::model::{Category, Remover};
use crate::modules::projects::remove_build_dirs;
use crate::modules::trash::remove_entries;
use crate::partition::Usage;
use crate::render::{Style, render_clean_summary};
use crate::runner::Runner;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Aborted,
    DoneWithErrors,
}

impl Outcome {
    pub fn exit_code(&self) -> u8 {
        match self {
            Outcome::Done => 0,
            Outcome::Aborted => 1,
            Outcome::DoneWithErrors => 2,
        }
    }
}

/// How `clean` asks and what its summary shows.
pub struct CleanOptions<'a> {
    pub assume_yes: bool,
    /// Stdin is a terminal: sudo may ask for a password.
    pub interactive: bool,
    pub details: bool,
    pub top: usize,
    pub style: Style,
    pub usage: &'a dyn Fn(&Path) -> Usage,
}

/// The commands a category's remover runs (without "sudo"); empty for in-process removers.
pub fn commands_for(cat: &Category) -> Vec<Vec<String>> {
    match &cat.remover {
        Remover::Run { argv, with_ids, .. } => {
            let mut cmd = argv.clone();
            if *with_ids {
                cmd.extend(cat.items.iter().map(|i| i.id.clone()));
            }
            vec![cmd]
        }
        Remover::SnapRevisions => cat
            .items
            .iter()
            .filter_map(|i| i.id.split_once('='))
            .map(|(name, rev)| {
                vec![
                    "snap".into(),
                    "remove".into(),
                    name.into(),
                    format!("--revision={rev}"),
                ]
            })
            .collect(),
        Remover::None | Remover::BuildDirs | Remover::Trash { .. } => Vec::new(),
    }
}

/// One line; "j", "ja", "y", "yes" (any case) mean yes; anything else and EOF mean no.
pub fn confirm(input: &mut dyn BufRead) -> bool {
    let mut line = String::new();
    if input.read_line(&mut line).is_err() {
        return false;
    }
    matches!(
        line.trim().to_lowercase().as_str(),
        "j" | "ja" | "y" | "yes"
    )
}

pub fn clean(
    cats: Vec<Category>,
    opts: &CleanOptions,
    input: &mut dyn BufRead,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Outcome {
    // Output errors (a closed pipe) must not stop or skip the removal once confirmed.
    let todo: Vec<Category> = cats.into_iter().filter(Category::is_clean).collect();
    if todo.is_empty() {
        let _ = writeln!(out, "Nichts Sicheres zu löschen.");
        return Outcome::Done;
    }
    let summary = render_clean_summary(&todo, opts.details, opts.top, &opts.style, opts.usage);
    let _ = write!(out, "{summary}");
    if !opts.assume_yes {
        let _ = write!(out, "Löschen? [j/N] ");
        let _ = out.flush();
        if !confirm(input) {
            let _ = writeln!(out, "Abgebrochen, nichts gelöscht.");
            return Outcome::Aborted;
        }
    }

    let mut ok = true;
    let mut skipped: Vec<Vec<String>> = Vec::new();
    let mut sudo_ok = None;
    for c in &todo {
        let _ = writeln!(out, "→ {}", c.title);
        let _ = out.flush();
        match &c.remover {
            Remover::BuildDirs => ok &= remove_build_dirs(&c.items, out),
            Remover::Trash { base } => ok &= remove_entries(&c.items, base, out),
            Remover::None => {}
            Remover::Run { .. } | Remover::SnapRevisions => {
                for cmd in commands_for(c) {
                    if !c.needs_sudo() {
                        ok &= runner.run(&cmd);
                    } else if *sudo_ok.get_or_insert_with(|| runner.can_sudo(opts.interactive)) {
                        ok &= runner.run_sudo(&cmd);
                    } else {
                        skipped.push(cmd);
                    }
                }
            }
        }
    }

    if !skipped.is_empty() {
        let _ = writeln!(out, "\nOhne sudo übersprungen. Selbst ausführen:");
        for cmd in &skipped {
            let _ = writeln!(out, "  sudo {}", cmd.join(" "));
        }
    }
    let _ = writeln!(
        out,
        "{}",
        if ok {
            "Fertig."
        } else {
            "Fertig, mit Fehlern (siehe oben)."
        }
    );
    if ok {
        Outcome::Done
    } else {
        Outcome::DoneWithErrors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Safety};
    use crate::runner::FakeRunner;

    fn usage(_: &Path) -> Usage {
        Usage {
            total: 1_000_000_000_000,
            free: 100_000_000_000,
            used: 900_000_000_000,
        }
    }

    fn opts(assume_yes: bool) -> CleanOptions<'static> {
        CleanOptions {
            assume_yes,
            interactive: false,
            details: false,
            top: 15,
            style: Style { colour: false },
            usage: &usage,
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn cat(key: &str, safety: Safety, remover: Remover, ids: &[&str]) -> Category {
        let mut c =
            Category::new(key, &format!("Titel {key}"), "/", safety, "warum").remover(remover);
        c.items = ids
            .iter()
            .map(|id| Item {
                size: 1000,
                id: id.to_string(),
                label: id.to_string(),
                path: None,
            })
            .collect();
        c
    }

    fn run(argv: &[&str], with_ids: bool, sudo: bool) -> Remover {
        Remover::Run {
            argv: strings(argv),
            with_ids,
            sudo,
        }
    }

    fn dangling() -> Category {
        cat(
            "dangling",
            Safety::Safe,
            run(&["docker", "rmi"], true, false),
            &["a", "b"],
        )
    }

    fn journal() -> Category {
        cat(
            "journal",
            Safety::Safe,
            run(&["journalctl", "--vacuum-size=500M"], false, true),
            &["journal"],
        )
    }

    fn do_clean(
        cats: Vec<Category>,
        opts: &CleanOptions,
        input: &str,
        runner: &mut FakeRunner,
    ) -> (Outcome, String) {
        let mut out = Vec::new();
        let outcome = clean(cats, opts, &mut input.as_bytes(), runner, &mut out);
        (outcome, String::from_utf8(out).unwrap())
    }

    #[test]
    fn commands_for_variants() {
        assert_eq!(
            commands_for(&dangling()),
            [strings(&["docker", "rmi", "a", "b"])]
        );
        let cache = cat(
            "build_cache",
            Safety::Safe,
            run(&["docker", "builder", "prune", "-a", "-f"], false, false),
            &["c1", "c2"],
        );
        assert_eq!(
            commands_for(&cache),
            [strings(&["docker", "builder", "prune", "-a", "-f"])]
        );
        let snaps = cat(
            "snap_revisions",
            Safety::Safe,
            Remover::SnapRevisions,
            &["gimp=561"],
        );
        assert_eq!(
            commands_for(&snaps),
            [strings(&["snap", "remove", "gimp", "--revision=561"])]
        );
        assert!(commands_for(&cat("b", Safety::Safe, Remover::BuildDirs, &["/x"])).is_empty());
    }

    #[test]
    fn confirm_answers() {
        for yes in ["j\n", "JA\n", "yes\n", " y \n"] {
            assert!(confirm(&mut yes.as_bytes()), "{yes:?}");
        }
        for no in ["n\n", "\n", "", "jein\n"] {
            assert!(!confirm(&mut no.as_bytes()), "{no:?}");
        }
    }

    #[test]
    fn eof_aborts_and_deletes_nothing() {
        let mut r = FakeRunner::default();
        let (outcome, text) = do_clean(vec![dangling()], &opts(false), "", &mut r);
        assert_eq!(outcome, Outcome::Aborted);
        assert!(r.calls.is_empty());
        assert!(
            text.ends_with("Löschen? [j/N] Abgebrochen, nichts gelöscht.\n"),
            "{text}"
        );
    }

    #[test]
    fn yes_runs_docker_commands_by_id() {
        let mut r = FakeRunner::default();
        let (outcome, text) = do_clean(vec![dangling()], &opts(true), "", &mut r);
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(r.calls, [strings(&["docker", "rmi", "a", "b"])]);
        assert!(!text.contains("Löschen?"));
        assert!(text.ends_with("→ Titel dangling\nFertig.\n"), "{text}");
    }

    #[test]
    fn answer_j_runs() {
        let mut r = FakeRunner::default();
        let (outcome, _) = do_clean(vec![dangling()], &opts(false), "j\n", &mut r);
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(r.calls.len(), 1);
    }

    #[test]
    fn sudo_skipped_without_terminal() {
        let mut r = FakeRunner::default();
        let (outcome, text) = do_clean(vec![journal()], &opts(true), "", &mut r);
        assert!(r.calls.is_empty());
        assert_eq!(outcome, Outcome::Done);
        assert!(text.contains("\nOhne sudo übersprungen. Selbst ausführen:\n  sudo journalctl --vacuum-size=500M\nFertig.\n"), "{text}");
    }

    #[test]
    fn sudo_runs_when_possible() {
        let mut r = FakeRunner {
            sudo_ok: true,
            ..Default::default()
        };
        let snaps = cat(
            "snap_revisions",
            Safety::Safe,
            Remover::SnapRevisions,
            &["gimp=561"],
        );
        do_clean(vec![journal(), snaps], &opts(true), "", &mut r);
        assert_eq!(
            r.calls,
            [
                strings(&["sudo", "journalctl", "--vacuum-size=500M"]),
                strings(&["sudo", "snap", "remove", "gimp", "--revision=561"])
            ]
        );
    }

    #[test]
    fn only_safe_categories_are_touched() {
        let cats = [Safety::Safe, Safety::Check, Safety::Data, Safety::Info]
            .into_iter()
            .map(|s| {
                cat(
                    &format!("{s:?}"),
                    s,
                    run(&["rm", &format!("{s:?}")], false, false),
                    &["x"],
                )
            })
            .collect();
        let mut r = FakeRunner::default();
        do_clean(cats, &opts(true), "", &mut r);
        assert_eq!(r.calls, [strings(&["rm", "Safe"])]);
    }

    #[test]
    fn nothing_safe() {
        let mut r = FakeRunner::default();
        let c = cat("x", Safety::Check, Remover::None, &["x"]);
        let (outcome, text) = do_clean(vec![c], &opts(false), "", &mut r);
        assert_eq!(
            (outcome, text.as_str()),
            (Outcome::Done, "Nichts Sicheres zu löschen.\n")
        );
    }

    #[test]
    fn refused_entry_means_errors() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("keep");
        std::fs::write(&outside, b"x").unwrap();
        let c = cat(
            "trash",
            Safety::Safe,
            Remover::Trash {
                base: dir.path().join("Trash"),
            },
            &[&outside.display().to_string()],
        );
        let (outcome, text) = do_clean(vec![c], &opts(true), "", &mut FakeRunner::default());
        assert_eq!(outcome, Outcome::DoneWithErrors);
        assert!(text.ends_with("Fertig, mit Fehlern (siehe oben).\n"));
        assert!(outside.exists());
    }

    #[test]
    fn summary_before_prompt() {
        let (_, text) = do_clean(
            vec![dangling()],
            &opts(false),
            "n\n",
            &mut FakeRunner::default(),
        );
        assert!(text.contains("Insgesamt etwa 2.0 kB. Nur die Kategorien oben werden gelöscht; einzelne Einträge: disk-tidy clean --details.\nLöschen? [j/N] "), "{text}");
    }
}
