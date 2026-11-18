//! `disk-tidy clean`: show the GEFAHRLOS part, ask, remove it.
use std::io::{BufRead, Write};
use std::path::Path;

use serde::Serialize;

use crate::model::{Category, Remover};
use crate::modules::projects::remove_build_dirs;
use crate::modules::trash::remove_entries;
use crate::partition::Usage;
use crate::render::{Style, render_clean_summary};
use crate::runner::Runner;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Nothing,
    Aborted,
    DoneWithErrors,
    DoneSkippedSudo,
    DonePartial,
    Error,
}

impl Outcome {
    pub fn exit_code(&self) -> u8 {
        match self {
            Outcome::Done | Outcome::Nothing => 0,
            Outcome::Aborted => 1,
            Outcome::Error => 2,
            Outcome::DoneWithErrors => 3,
            Outcome::DoneSkippedSudo => 4,
            Outcome::DonePartial => 5,
        }
    }
}

/// What happened to one category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Removed,
    Failed,
    SkippedSudo,
    /// A plan held only part of a category whose remover takes no IDs.
    SkippedPartial,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportItem {
    pub id: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CatResult {
    pub key: String,
    pub status: Status,
    pub items: Vec<ReportItem>,
}

/// Why a plan entry was not removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Gone,
    NotSafe,
    ModuleFailed,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Dropped {
    pub key: String,
    pub id: String,
    pub reason: Reason,
}

/// The result of `clean`, printed as JSON with `--json`.
#[derive(Debug, Serialize)]
pub struct Report {
    pub version: u32,
    pub outcome: Outcome,
    pub exit_code: u8,
    /// Sum of the item sizes of removed categories (an estimate, as the preview).
    pub freed: u64,
    pub categories: Vec<CatResult>,
    pub dropped: Vec<Dropped>,
    pub skipped_sudo: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Report {
    fn new(outcome: Outcome) -> Self {
        Report {
            version: 1,
            outcome,
            exit_code: outcome.exit_code(),
            freed: 0,
            categories: Vec::new(),
            dropped: Vec::new(),
            skipped_sudo: Vec::new(),
            error: None,
        }
    }

    /// A plan file, config or other error before anything ran.
    pub fn error(msg: String) -> Self {
        Report {
            error: Some(msg),
            ..Report::new(Outcome::Error)
        }
    }
}

/// What `clean` removes: `todo`; `partial` and `dropped` come from a plan and are only reported.
#[derive(Debug, Default)]
pub struct Selection {
    pub todo: Vec<Category>,
    pub partial: Vec<Category>,
    pub dropped: Vec<Dropped>,
}

impl Selection {
    /// Every GEFAHRLOS category, as interactive `clean` removes.
    pub fn all(cats: Vec<Category>) -> Self {
        Selection {
            todo: cats.into_iter().filter(Category::is_clean).collect(),
            ..Default::default()
        }
    }
}

/// How `clean` asks and what its summary shows.
pub struct CleanOptions<'a> {
    pub assume_yes: bool,
    /// Stdin is a terminal: sudo may ask for a password.
    pub interactive: bool,
    /// `clean` may ask "Löschen?": stdin is a terminal and the output is not JSON.
    pub can_prompt: bool,
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

fn items_of(c: &Category) -> Vec<ReportItem> {
    c.items
        .iter()
        .map(|i| ReportItem {
            id: i.id.clone(),
            size: i.size,
        })
        .collect()
}

/// The base report of a selection: partial categories as skipped, dropped plan entries.
fn report_of_sel(partial: &[Category], dropped: Vec<Dropped>) -> Report {
    let mut report = Report::new(Outcome::Done);
    report.dropped = dropped;
    report.categories = partial
        .iter()
        .map(|c| CatResult {
            key: c.key.clone(),
            status: Status::SkippedPartial,
            items: items_of(c),
        })
        .collect();
    report
}

fn finish(mut report: Report, outcome: Outcome) -> Report {
    report.outcome = outcome;
    report.exit_code = outcome.exit_code();
    report
}

/// Shows the summary, asks (unless `assume_yes`), removes `sel.todo` and reports. Text goes to
/// `out`; only the "cannot ask" message goes to `err`.
pub fn clean(
    sel: Selection,
    opts: &CleanOptions,
    input: &mut dyn BufRead,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Report {
    // Output errors (a closed pipe) must not stop or skip the removal once confirmed.
    if sel.todo.is_empty() {
        let _ = writeln!(out, "Nichts Sicheres zu löschen.");
        plan_notes(out, &sel.partial, &sel.dropped);
        let outcome = if sel.partial.is_empty() && sel.dropped.is_empty() {
            Outcome::Nothing
        } else {
            Outcome::DonePartial
        };
        return finish(report_of_sel(&sel.partial, sel.dropped), outcome);
    }
    let summary = render_clean_summary(&sel.todo, opts.details, opts.top, &opts.style, opts.usage);
    let _ = write!(out, "{summary}");
    plan_notes(out, &sel.partial, &sel.dropped);
    if !opts.assume_yes {
        let aborted =
            |sel: Selection| finish(report_of_sel(&sel.partial, sel.dropped), Outcome::Aborted);
        if !opts.can_prompt {
            let _ = writeln!(err, "Keine Rückfrage möglich: --yes oder --plan angeben.");
            return aborted(sel);
        }
        let _ = write!(out, "Löschen? [j/N] ");
        let _ = out.flush();
        if !confirm(input) {
            let _ = writeln!(out, "Abgebrochen, nichts gelöscht.");
            return aborted(sel);
        }
    }
    execute(sel, opts.interactive, runner, out, &mut |_| {})
}

/// One step of `execute`, for a progress display.
pub enum Progress<'a> {
    Started(&'a Category),
    Done(&'a CatResult),
}

/// Removes `sel.todo` category by category and reports; what `clean` does once confirmed.
/// `interactive`: sudo may ask for a password. Remover messages and the closing lines go to `out`.
pub fn execute(
    sel: Selection,
    interactive: bool,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(Progress),
) -> Report {
    let opts = ExecOptions {
        interactive,
        quiet: false,
    };
    execute_with(sel, opts, runner, out, progress)
}

#[derive(Clone, Copy)]
pub struct ExecOptions {
    /// sudo may ask for a password.
    pub interactive: bool,
    /// Only remover messages go to `out`: no category headers, sudo list or closing line
    /// (the TUI shows those from the progress events and the report).
    pub quiet: bool,
}

pub fn execute_with(
    sel: Selection,
    opts: ExecOptions,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(Progress),
) -> Report {
    let Selection {
        todo,
        partial,
        dropped,
    } = sel;
    let incomplete = !partial.is_empty() || !dropped.is_empty();
    let mut report = report_of_sel(&partial, dropped);
    for c in &todo {
        if !opts.quiet {
            let _ = writeln!(out, "→ {}", c.title);
            let _ = out.flush();
        }
        // Asked once per category: a sudo ticket can expire during a long run.
        let mut sudo_ok = None;
        progress(Progress::Started(c));
        let status = match &c.remover {
            Remover::BuildDirs | Remover::Trash { .. } => {
                let ok = match &c.remover {
                    Remover::Trash { base } => remove_entries(&c.items, base, out),
                    _ => remove_build_dirs(&c.items, out),
                };
                if ok { Status::Removed } else { Status::Failed }
            }
            Remover::None => Status::Removed,
            Remover::Run { .. } | Remover::SnapRevisions => {
                let (mut failed, mut skipped) = (false, false);
                for cmd in commands_for(c) {
                    if !c.needs_sudo() {
                        failed |= !runner.run(&cmd);
                    } else if *sudo_ok.get_or_insert_with(|| runner.can_sudo(opts.interactive)) {
                        failed |= !runner.run_sudo(&cmd);
                    } else {
                        skipped = true;
                        report.skipped_sudo.push(format!("sudo {}", cmd.join(" ")));
                    }
                }
                match (failed, skipped) {
                    (true, _) => Status::Failed,
                    (false, true) => Status::SkippedSudo,
                    (false, false) => Status::Removed,
                }
            }
        };
        if status == Status::Removed {
            report.freed += c.total();
        }
        report.categories.push(CatResult {
            key: c.key.clone(),
            status,
            items: items_of(c),
        });
        progress(Progress::Done(
            report.categories.last().expect("just pushed"),
        ));
    }

    let errors = report.categories.iter().any(|c| c.status == Status::Failed);
    if !opts.quiet {
        closing_lines(out, &report.skipped_sudo, errors);
    }
    let outcome = if errors {
        Outcome::DoneWithErrors
    } else if !report.skipped_sudo.is_empty() {
        Outcome::DoneSkippedSudo
    } else if incomplete {
        Outcome::DonePartial
    } else {
        Outcome::Done
    };
    finish(report, outcome)
}

/// The sudo commands left to run by hand, then "Fertig." (with errors or not).
fn closing_lines(out: &mut dyn Write, skipped_sudo: &[String], errors: bool) {
    if !skipped_sudo.is_empty() {
        let _ = writeln!(out, "\nOhne sudo übersprungen. Selbst ausführen:");
        for cmd in skipped_sudo {
            let _ = writeln!(out, "  {cmd}");
        }
    }
    let _ = writeln!(
        out,
        "{}",
        if errors {
            "Fertig, mit Fehlern (siehe oben)."
        } else {
            "Fertig."
        }
    );
}

/// What of a plan is not carried out, one line each.
fn plan_notes(out: &mut dyn Write, partial: &[Category], dropped: &[Dropped]) {
    if !dropped.is_empty() {
        let _ = writeln!(
            out,
            "Aus dem Plan nicht mehr gültig: {} Einträge (verschwunden, nicht mehr gefahrlos oder Modul ausgefallen).",
            dropped.len()
        );
    }
    for c in partial {
        let _ = writeln!(
            out,
            "Übersprungen, der Plan enthält nicht alle Einträge: {}",
            c.title
        );
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
            can_prompt: true,
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
                ..Default::default()
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
        let (report, out, _) = do_clean_full(Selection::all(cats), opts, input, runner);
        (report.outcome, out)
    }

    /// Report, stdout text, stderr text.
    fn do_clean_full(
        sel: Selection,
        opts: &CleanOptions,
        input: &str,
        runner: &mut FakeRunner,
    ) -> (Report, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let report = clean(sel, opts, &mut input.as_bytes(), runner, &mut out, &mut err);
        (
            report,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn report_of(cats: Vec<Category>, runner: &mut FakeRunner) -> Report {
        do_clean_full(Selection::all(cats), &opts(true), "", runner).0
    }

    #[test]
    fn execute_reports_progress_in_order() {
        let mut events = Vec::new();
        let sel = Selection::all(vec![dangling(), journal()]);
        let mut runner = FakeRunner {
            sudo_ok: true,
            ..Default::default()
        };
        let mut out = Vec::new();
        let report = execute(sel, false, &mut runner, &mut out, &mut |p| {
            events.push(match p {
                Progress::Started(c) => format!("started {}", c.key),
                Progress::Done(r) => format!("done {} {:?}", r.key, r.status),
            })
        });
        assert_eq!(
            events,
            [
                "started dangling",
                "done dangling Removed",
                "started journal",
                "done journal Removed"
            ]
        );
        assert_eq!(report.outcome, Outcome::Done);
        assert_eq!(report.freed, 3000);
        assert!(String::from_utf8(out).unwrap().ends_with("Fertig.\n"));
    }

    #[test]
    fn sudo_is_checked_per_category() {
        let crash = cat(
            "crash",
            Safety::Safe,
            run(&["rm", "-f", "--"], true, true),
            &["/var/crash/a"],
        );
        let mut runner = FakeRunner {
            sudo_checks: vec![true, false],
            ..Default::default()
        };
        let sel = Selection::all(vec![journal(), crash]);
        let report = execute(sel, false, &mut runner, &mut Vec::new(), &mut |_| {});
        let status: Vec<Status> = report.categories.iter().map(|c| c.status).collect();
        assert_eq!(status, [Status::Removed, Status::SkippedSudo]);
        assert_eq!(report.skipped_sudo, ["sudo rm -f -- /var/crash/a"]);
    }

    #[test]
    fn quiet_execute_writes_remover_messages_only() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("node_modules");
        let mut c = cat("build_stale", Safety::Safe, Remover::BuildDirs, &[]);
        c.items = vec![Item {
            size: 1,
            id: missing.display().to_string(),
            ..Default::default()
        }];
        let sel = Selection::all(vec![c, dangling()]);
        let mut out = Vec::new();
        let quiet = ExecOptions {
            interactive: false,
            quiet: true,
        };
        execute_with(
            sel,
            quiet,
            &mut FakeRunner::default(),
            &mut out,
            &mut |_| {},
        );
        let out = String::from_utf8(out).unwrap();
        assert_eq!(
            out,
            format!(
                "  Übersprungen (kein Build-Ordner): {}\n",
                missing.display()
            )
        );
    }

    #[test]
    fn check_categories_with_remover_are_not_cleaned() {
        let c = cat(
            "containers",
            Safety::Check,
            run(&["docker", "rm"], true, false),
            &["x"],
        );
        let sel = Selection::all(vec![c]);
        assert!(sel.todo.is_empty());
        let mut runner = FakeRunner::default();
        report_of(
            vec![cat(
                "containers",
                Safety::Check,
                run(&["docker", "rm"], true, false),
                &["x"],
            )],
            &mut runner,
        );
        assert!(runner.calls.is_empty());
    }

    #[test]
    fn exit_codes() {
        let codes = [
            (Outcome::Done, 0),
            (Outcome::Nothing, 0),
            (Outcome::Aborted, 1),
            (Outcome::Error, 2),
            (Outcome::DoneWithErrors, 3),
            (Outcome::DoneSkippedSudo, 4),
            (Outcome::DonePartial, 5),
        ];
        for (o, code) in codes {
            assert_eq!(o.exit_code(), code, "{o:?}");
        }
    }

    #[test]
    fn no_prompt_without_terminal() {
        let mut r = FakeRunner::default();
        let o = CleanOptions {
            can_prompt: false,
            ..opts(false)
        };
        let (report, out, err) = do_clean_full(Selection::all(vec![dangling()]), &o, "j\n", &mut r);
        assert_eq!(report.outcome, Outcome::Aborted);
        assert!(r.calls.is_empty());
        assert!(!out.contains("Löschen?"), "{out}");
        assert_eq!(err, "Keine Rückfrage möglich: --yes oder --plan angeben.\n");
    }

    #[test]
    fn report_lists_removed_and_freed() {
        let report = report_of(vec![dangling()], &mut FakeRunner::default());
        assert_eq!(report.outcome, Outcome::Done);
        assert_eq!(report.freed, 2000);
        assert_eq!(report.categories.len(), 1);
        let c = &report.categories[0];
        assert_eq!((c.key.as_str(), &c.status), ("dangling", &Status::Removed));
        assert_eq!(
            c.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn failed_command_gives_exit_3() {
        let mut r = FakeRunner::default();
        r.fail.insert("docker rmi a b".into());
        let report = report_of(vec![dangling()], &mut r);
        assert_eq!(report.categories[0].status, Status::Failed);
        assert_eq!(report.outcome, Outcome::DoneWithErrors);
        assert_eq!(report.exit_code, 3);
        assert_eq!(report.freed, 0);
    }

    #[test]
    fn skipped_sudo_gives_exit_4() {
        let (report, out, _) = do_clean_full(
            Selection::all(vec![journal()]),
            &opts(true),
            "",
            &mut FakeRunner::default(),
        );
        assert_eq!(report.categories[0].status, Status::SkippedSudo);
        assert_eq!(report.skipped_sudo, ["sudo journalctl --vacuum-size=500M"]);
        assert_eq!(report.outcome, Outcome::DoneSkippedSudo);
        assert!(out.contains("Ohne sudo übersprungen"), "{out}");
    }

    #[test]
    fn report_json_shape() {
        let report = report_of(vec![dangling()], &mut FakeRunner::default());
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["outcome"], "done");
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["freed"], 2000);
        assert_eq!(v["categories"][0]["status"], "removed");
        assert!(v.get("error").is_none());
        let e = serde_json::to_value(Report::error("x".into())).unwrap();
        assert_eq!(
            (&e["outcome"], &e["exit_code"], &e["error"]),
            (&"error".into(), &2.into(), &"x".into())
        );
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
        assert_eq!(outcome, Outcome::DoneSkippedSudo);
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
            (Outcome::Nothing, "Nichts Sicheres zu löschen.\n")
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

    fn one_dropped() -> Vec<Dropped> {
        vec![Dropped {
            key: "trash".into(),
            id: "/t/x".into(),
            reason: Reason::Gone,
        }]
    }

    #[test]
    fn partial_gives_exit_5() {
        let mut r = FakeRunner::default();
        let sel = Selection {
            todo: vec![dangling()],
            dropped: one_dropped(),
            ..Default::default()
        };
        let (report, out, _) = do_clean_full(sel, &opts(true), "", &mut r);
        assert_eq!(report.outcome, Outcome::DonePartial);
        assert_eq!(report.exit_code, 5);
        assert_eq!(r.calls, [strings(&["docker", "rmi", "a", "b"])]);
        assert_eq!(report.dropped, one_dropped());
        assert!(out.contains("Aus dem Plan nicht mehr gültig: 1 Einträge (verschwunden, nicht mehr gefahrlos oder Modul ausgefallen).\n"), "{out}");
    }

    #[test]
    fn partial_category_is_reported_not_run() {
        let mut r = FakeRunner::default();
        let cache = cat(
            "build_cache",
            Safety::Safe,
            run(&["docker", "builder", "prune", "-a", "-f"], false, false),
            &["c1"],
        );
        let sel = Selection {
            partial: vec![cache],
            ..Default::default()
        };
        let (report, out, _) = do_clean_full(sel, &opts(true), "", &mut r);
        assert!(r.calls.is_empty());
        assert_eq!(report.outcome, Outcome::DonePartial);
        assert_eq!(report.categories[0].status, Status::SkippedPartial);
        assert_eq!(report.freed, 0);
        assert_eq!(
            out,
            "Nichts Sicheres zu löschen.\nÜbersprungen, der Plan enthält nicht alle Einträge: Titel build_cache\n"
        );
    }

    #[test]
    fn all_dropped_gives_exit_5() {
        let mut r = FakeRunner::default();
        let sel = Selection {
            dropped: one_dropped(),
            ..Default::default()
        };
        let (report, _, _) = do_clean_full(sel, &opts(true), "", &mut r);
        assert_eq!(report.outcome, Outcome::DonePartial);
        assert!(r.calls.is_empty());
    }

    #[test]
    fn errors_beat_partial() {
        let mut r = FakeRunner::default();
        r.fail.insert("docker rmi a b".into());
        let sel = Selection {
            todo: vec![dangling()],
            dropped: one_dropped(),
            ..Default::default()
        };
        assert_eq!(
            do_clean_full(sel, &opts(true), "", &mut r).0.outcome,
            Outcome::DoneWithErrors
        );
    }

    #[test]
    fn skipped_sudo_beats_partial() {
        let sel = Selection {
            todo: vec![journal()],
            dropped: one_dropped(),
            ..Default::default()
        };
        let report = do_clean_full(sel, &opts(true), "", &mut FakeRunner::default()).0;
        assert_eq!(report.outcome, Outcome::DoneSkippedSudo);
    }
}
