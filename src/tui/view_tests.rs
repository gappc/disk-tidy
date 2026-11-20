use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::clean::{CatResult, Dropped, Reason, Report, ReportItem, Status};
use crate::model::{Category, Remover, Safety};
use crate::tui::app::tests::{
    app, cat, keys, mount, picked, picked_scanning, run, scanning, usage,
};
use crate::tui::app::{App, DeleteEvent, Key, ScanEvent};

fn screen(app: &App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, app, false)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn has(text: &str, want: &str) {
    assert!(text.contains(want), "missing {want:?} in\n{text}");
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn overview_shows_rows_and_total() {
    let s = screen(&picked(), 80, 24);
    has(&s, "Auswahl");
    has(&s, "167 B");
    has(&s, "✓ gefahrlos");
    has(&s, "⚠ prüfen");
    has(&s, "✗ vorsicht");
    has(&s, "frei danach");
    has(&s, "⏎ öffnen");
}

#[test]
fn scanning_line_names_module() {
    has(&screen(&scanning(), 80, 24), "Scanne docker …");
}

#[test]
fn failed_module_note() {
    let mut app = app();
    app.on_scan(ScanEvent::Failed(
        "docker".into(),
        "Docker-Daemon nicht erreichbar".into(),
    ));
    has(
        &screen(&app, 80, 24),
        "docker: Docker-Daemon nicht erreichbar",
    );
}

#[test]
fn entries_show_marks_and_age() {
    let mut app = app();
    keys(&mut app, &[Key::Enter, Key::Right, Key::Space]);
    let s = screen(&app, 80, 24);
    has(&s, "☐");
    has(&s, "☑");
    has(&s, "› Titel trash");

    let mut aged = App::new(usage, mount);
    let mut c = cat(
        "trash",
        "/",
        Safety::Safe,
        Remover::BuildDirs,
        &[("old", 5)],
    );
    c.items[0].modified = Some(now() - 41 * 86_400);
    aged.on_scan(ScanEvent::Done(vec![c]));
    aged.on_scan(ScanEvent::Finished);
    keys(&mut aged, &[Key::Enter, Key::Right]);
    has(&screen(&aged, 80, 24), "vor 41 Tg.");
}

#[test]
fn fixed_rows_have_no_checkbox() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Down, Key::Enter]);
    let s = screen(&app, 80, 24);
    has(&s, "Titel named_volumes");
    assert!(!s.contains('☐') && !s.contains('☑'), "{s}");
    has(&s, "nicht auswählbar");
}

#[test]
fn idless_entries_show_dot() {
    let mut app = app();
    keys(&mut app, &[Key::Enter, Key::Down, Key::Right]);
    let s = screen(&app, 80, 24);
    has(&s, "·");
    has(&s, "nur als Ganzes");
}

#[test]
fn review_highlights_check() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Space, Key::Char('d')]);
    let s = screen(&app, 80, 24);
    has(&s, "Zu löschen");
    has(&s, "⚠ x");
    has(&s, "frei danach");
}

fn check_app() -> App {
    let mut app = App::new(usage, mount);
    app.on_scan(ScanEvent::Done(vec![cat(
        "unused_images",
        "/",
        Safety::Check,
        run(true),
        &[("rust:1", 2_400_000_000), ("node:22", 1_200_000_000)],
    )]));
    app.on_scan(ScanEvent::Finished);
    app
}

#[test]
fn confirm_names_check_part() {
    let mut app = check_app();
    keys(&mut app, &[Key::Space, Key::Char('d'), Key::Enter]);
    let s = screen(&app, 80, 24);
    has(&s, "3.6 GB in 1 Kategorien.");
    has(&s, "Davon ⚠ prüfen: 2 Einträge, 3.6 GB.");
    has(&s, "j  Löschen");
}

#[test]
fn progress_and_report_lines() {
    let mut app = picked();
    keys(&mut app, &[Key::Char('d'), Key::Enter, Key::Char('j')]);
    app.on_delete(DeleteEvent::Rechecking);
    has(&screen(&app, 80, 24), "Prüfe neu …");
    let done = |key: &str, status| CatResult {
        key: key.into(),
        status,
        items: vec![ReportItem {
            id: "a".into(),
            size: 100,
        }],
    };
    app.on_delete(DeleteEvent::Started("trash".into()));
    app.on_delete(DeleteEvent::Done(done("trash", Status::Removed)));
    app.on_delete(DeleteEvent::Done(done("build_cache", Status::Failed)));
    let s = screen(&app, 80, 24);
    has(&s, "✓ Titel trash");
    has(&s, "✗ Titel build_cache");
    let mut report = Report::error("x".into());
    report.dropped = vec![Dropped {
        key: "trash".into(),
        id: "b".into(),
        reason: Reason::Gone,
    }];
    app.on_delete(DeleteEvent::Finished(report));
    let s = screen(&app, 80, 24);
    has(&s, "nicht mehr gültig");
    has(&s, "b (verschwunden)");
}

#[test]
fn long_list_keeps_cursor_visible() {
    let mut app = App::new(usage, mount);
    let ids: Vec<(String, u64)> = (0..200).map(|i| (format!("e{i:03}"), 1000 - i)).collect();
    let refs: Vec<(&str, u64)> = ids.iter().map(|(s, n)| (s.as_str(), *n)).collect();
    app.on_scan(ScanEvent::Done(vec![cat(
        "trash",
        "/",
        Safety::Safe,
        Remover::BuildDirs,
        &refs,
    )]));
    app.on_scan(ScanEvent::Finished);
    keys(&mut app, &[Key::Enter, Key::Right]);
    for _ in 0..199 {
        app.on_key(Key::Down);
    }
    has(&screen(&app, 80, 24), "e199");
}

#[test]
fn small_terminal_does_not_panic() {
    let mut app = app();
    let walk: &[&[Key]] = &[
        &[],
        &[Key::Enter],
        &[Key::Right],
        &[Key::Char('i')],
        &[Key::Esc, Key::Char('?')],
        &[Key::Esc, Key::Char('d')],
        &[Key::Enter],
    ];
    for step in walk {
        keys(&mut app, step);
        for (w, h) in [(60, 16), (20, 5), (1, 1)] {
            screen(&app, w, h);
        }
    }
    keys(&mut app, &[Key::Char('j')]);
    app.on_delete(DeleteEvent::Finished(Report::error("x".into())));
    for (w, h) in [(60, 16), (20, 5), (1, 1)] {
        screen(&app, w, h);
    }
}

#[test]
fn details_popup_shows_facts() {
    let mut app = App::new(usage, mount);
    let mut c = cat(
        "build_stale",
        "/",
        Safety::Safe,
        Remover::BuildDirs,
        &[("/p/x/target", 5)],
    );
    c.items[0].facts = vec![("Projekt".into(), "~/p/x".into())];
    app.on_scan(ScanEvent::Done(vec![c]));
    app.on_scan(ScanEvent::Finished);
    keys(&mut app, &[Key::Enter, Key::Right, Key::Char('i')]);
    let s = screen(&app, 80, 24);
    has(&s, "Projekt: ~/p/x");
    has(&s, "warum");
    has(&s, "Esc schließen");
}

#[test]
fn shorten_keeps_both_ends() {
    assert_eq!(shorten("abcdefghij", 7), "abc…hij");
    assert_eq!(shorten("abc", 7), "abc");
    assert_eq!(shorten("abc", 0), "");
    let _ = Path::new("/");
    let _: Option<&Category> = None;
}

/// Readable on any terminal background: a cell with its own text colour also sets its own
/// background (a badge), and nothing uses fixed grey.
#[test]
fn colours_are_readable_on_any_background() {
    use ratatui::style::Color;
    let check = |app: &App| {
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| draw(f, app, true)).unwrap();
        let buf = t.backend().buffer().clone();
        for cell in buf.content() {
            if cell.symbol().trim().is_empty() {
                continue;
            }
            assert!(
                cell.fg == Color::Reset || cell.bg != Color::Reset,
                "{:?} on default background: {:?}",
                cell.fg,
                cell.symbol()
            );
            assert_ne!(cell.fg, Color::DarkGray, "{:?}", cell.symbol());
        }
    };
    let mut app = app();
    app.on_scan(ScanEvent::Failed("docker".into(), "weg".into()));
    check(&app);
    keys(&mut app, &[Key::Enter]);
    check(&app);
    keys(&mut app, &[Key::Right]);
    check(&app);
    keys(&mut app, &[Key::Char('i')]);
    check(&app);
    keys(
        &mut app,
        &[
            Key::Esc,
            Key::Esc,
            Key::Esc,
            Key::Down,
            Key::Space,
            Key::Char('d'),
        ],
    );
    check(&app);
    keys(&mut app, &[Key::Enter]);
    check(&app);
    keys(&mut app, &[Key::Char('j')]);
    let done = |key: &str, status| CatResult {
        key: key.into(),
        status,
        items: Vec::new(),
    };
    app.on_delete(DeleteEvent::Done(done("trash", Status::Removed)));
    app.on_delete(DeleteEvent::Done(done("build_cache", Status::Failed)));
    check(&app);
}

#[test]
fn spinner_turns_with_tick() {
    let mut app = scanning();
    app.tick = 0;
    has(&screen(&app, 80, 24), "⠋ Scanne docker …");
    app.tick = 1;
    has(&screen(&app, 80, 24), "⠙ Scanne docker …");
    app.tick = 10;
    has(&screen(&app, 80, 24), "⠋ Scanne docker …");
}

#[test]
fn spinner_before_first_module() {
    let app = App::new(usage, mount);
    has(&screen(&app, 80, 24), "⠋ Scanne …");
}

#[test]
fn spinner_while_deleting_not_on_report() {
    let mut app = picked();
    keys(&mut app, &[Key::Char('d'), Key::Enter, Key::Char('j')]);
    app.on_delete(DeleteEvent::Rechecking);
    let s = screen(&app, 80, 24);
    has(&s, "⠋ Prüfe neu …");
    has(&s, "⠋ Löschen ·");
    app.on_delete(DeleteEvent::Started("trash".into()));
    has(&screen(&app, 80, 24), "⠋ Titel trash");
    app.on_delete(DeleteEvent::Finished(Report::error("x".into())));
    assert!(!screen(&app, 80, 24).contains('⠋'));
}

fn head(s: &str) -> &str {
    s.lines().next().unwrap()
}

#[test]
fn spinner_in_header_while_busy() {
    let mut a = picked_scanning();
    assert!(head(&screen(&a, 80, 24)).starts_with(" ⠋ disk-tidy"));
    keys(&mut a, &[Key::Enter]);
    assert!(head(&screen(&a, 80, 24)).starts_with(" ⠋ /"));
    a.on_scan(ScanEvent::Finished);
    assert!(!head(&screen(&a, 80, 24)).contains('⠋'));

    let mut a = picked();
    assert!(!head(&screen(&a, 80, 24)).contains('⠋'));
    keys(&mut a, &[Key::Char('d'), Key::Enter, Key::Char('j')]);
    a.on_delete(DeleteEvent::Rechecking);
    assert!(head(&screen(&a, 80, 24)).starts_with(" ⠋ Löschen ·"));
}

#[test]
fn overview_shows_free_now_and_after() {
    // usage(): 1000 B free; the ticked GEFAHRLOS entries add 167 B.
    let s = screen(&picked(), 80, 24);
    let header = s.lines().find(|l| l.contains("Partition")).unwrap();
    assert!(
        header.contains("frei jetzt") && header.contains("frei danach"),
        "{header}"
    );
    let first = s.lines().find(|l| l.contains("✓ gefahrlos")).unwrap();
    assert!(
        first.contains("1.0 kB") && first.contains("1.2 kB"),
        "{first}"
    );
}

#[test]
fn entries_show_sort() {
    let mut a = app();
    keys(&mut a, &[Key::Enter, Key::Right]);
    let s = screen(&a, 80, 24);
    has(head(&s), "nach Größe");
    has(s.lines().last().unwrap(), "s sortieren");
    keys(&mut a, &[Key::Char('s')]);
    has(head(&screen(&a, 80, 24)), "nach Alter");
    keys(&mut a, &[Key::Char('?')]);
    has(
        &screen(&a, 80, 24),
        "s         sortieren: Größe, Alter, Name",
    );
}
