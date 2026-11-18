use super::*;
use crate::clean::Report;
use crate::model::{Item, Remover};

pub(crate) fn usage(_: &Path) -> Usage {
    Usage {
        total: 2000,
        free: 1000,
        used: 1000,
    }
}

pub(crate) fn mount(p: &Path) -> PathBuf {
    if p.starts_with("/home") {
        PathBuf::from("/home")
    } else {
        PathBuf::from("/")
    }
}

pub(crate) fn cat(
    key: &str,
    path: &str,
    safety: Safety,
    remover: Remover,
    items: &[(&str, u64)],
) -> Category {
    let mut c = Category::new(key, &format!("Titel {key}"), path, safety, "warum")
        .remover(remover)
        .hint("befehl");
    c.module = if key == "trash" { "trash" } else { "docker" }.into();
    c.items = items
        .iter()
        .map(|(id, size)| Item {
            size: *size,
            id: id.to_string(),
            label: id.to_string(),
            ..Default::default()
        })
        .collect();
    c
}

pub(crate) fn run(with_ids: bool) -> Remover {
    Remover::Run {
        argv: vec!["docker".into(), "x".into()],
        with_ids,
        sudo: false,
    }
}

pub(crate) fn cats() -> Vec<Category> {
    vec![
        cat(
            "trash",
            "/",
            Safety::Safe,
            Remover::BuildDirs,
            &[("a", 100), ("b", 50)],
        ),
        cat("containers", "/", Safety::Check, run(true), &[("x", 30)]),
        cat(
            "stopped_images",
            "/",
            Safety::Check,
            Remover::None,
            &[("y", 20)],
        ),
        cat(
            "named_volumes",
            "/",
            Safety::Data,
            Remover::None,
            &[("v", 10)],
        ),
        cat(
            "build_cache",
            "/",
            Safety::Safe,
            run(false),
            &[("c1", 5), ("c2", 5)],
        ),
        cat(
            "build_stale",
            "/home/u",
            Safety::Safe,
            Remover::BuildDirs,
            &[("h1", 7)],
        ),
    ]
}

pub(crate) fn scanning() -> App {
    let mut app = App::new(usage, mount);
    app.on_scan(ScanEvent::Started("docker".into()));
    app.on_scan(ScanEvent::Done("docker".into(), cats()));
    app
}

pub(crate) fn app() -> App {
    let mut app = scanning();
    app.on_scan(ScanEvent::Finished);
    app
}

pub(crate) fn keys(app: &mut App, keys: &[Key]) -> Action {
    let mut last = Action::None;
    for k in keys {
        last = app.on_key(*k);
    }
    last
}

pub(crate) fn find<'a>(app: &'a App, key: &str) -> &'a Category {
    app.all_categories().iter().find(|c| c.key == key).unwrap()
}

pub(crate) fn item<'a>(app: &'a App, key: &str, id: &str) -> (&'a Category, &'a Item) {
    let c = find(app, key);
    (c, c.items.iter().find(|i| i.id == id).unwrap())
}

#[test]
fn safe_preselected_check_not() {
    let app = app();
    assert_eq!(app.selected_total(), 167);
    assert_eq!(app.mark_cat(find(&app, "trash")), Mark::On);
    assert_eq!(app.mark_cat(find(&app, "containers")), Mark::Off);
    assert_eq!(app.selected_check(), (0, 0));
    assert!(app.scanning.is_none() && app.scan_done);
}

#[test]
fn overview_rows_per_partition_and_level() {
    let app = app();
    let row = |mount: &str, safety, entries, size, selected| Row {
        mount: PathBuf::from(mount),
        safety,
        entries,
        size,
        selected,
    };
    assert_eq!(
        app.rows(),
        [
            row("/", Safety::Safe, 4, 160, 160),
            row("/", Safety::Check, 2, 50, 0),
            row("/", Safety::Data, 1, 10, 0),
            row("/home", Safety::Safe, 1, 7, 7),
        ]
    );
    assert_eq!(app.free_after(Path::new("/")), 1160);
}

#[test]
fn space_on_row_toggles_selectable_only() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Space]);
    assert_eq!(app.selected_check(), (1, 30));
    assert_eq!(app.mark_cat(find(&app, "stopped_images")), Mark::Fixed);
    keys(&mut app, &[Key::Space]);
    assert_eq!(app.selected_check(), (0, 0));
    // a row that is all on goes all off
    keys(&mut app, &[Key::Up, Key::Space]);
    assert_eq!(app.selected_total(), 7);
}

#[test]
fn partial_mark() {
    let mut app = app();
    keys(&mut app, &[Key::Enter]);
    assert_eq!(
        app.categories()
            .iter()
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>(),
        ["trash", "build_cache"]
    );
    keys(&mut app, &[Key::Right, Key::Space]);
    assert_eq!(app.screen, Screen::Entries);
    let (c, a) = item(&app, "trash", "a");
    assert_eq!(app.mark_item(c, a), Mark::Off);
    assert_eq!(app.mark_cat(c), Mark::Partial);
    keys(&mut app, &[Key::Char('a')]);
    assert_eq!(app.mark_cat(find(&app, "trash")), Mark::On);
    keys(&mut app, &[Key::Char('n')]);
    assert_eq!(app.mark_cat(find(&app, "trash")), Mark::Off);
}

#[test]
fn fixed_cannot_be_toggled() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Down, Key::Space]);
    assert!(matches!(&app.popup, Popup::Message(m) if m.starts_with("Nicht auswählbar")));
    assert_eq!(app.selected_total(), 167);
    keys(
        &mut app,
        &[Key::Esc, Key::Up, Key::Enter, Key::Down, Key::Space],
    );
    assert_eq!(app.categories()[1].key, "stopped_images");
    assert!(matches!(&app.popup, Popup::Message(m) if m.contains("befehl")));
    assert_eq!(app.selected_check(), (0, 0));
}

#[test]
fn idless_toggles_whole_category() {
    let mut app = app();
    keys(&mut app, &[Key::Enter, Key::Down, Key::Right]);
    let (c, c1) = item(&app, "build_cache", "c1");
    assert_eq!(app.mark_item(c, c1), Mark::Whole);
    keys(&mut app, &[Key::Space]);
    assert_eq!(app.mark_cat(find(&app, "build_cache")), Mark::Off);
    assert_eq!(app.selected_total(), 157);
}

#[test]
fn navigation_and_back() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Enter]);
    assert_eq!(app.screen, Screen::Categories);
    assert_eq!(app.categories()[0].key, "containers");
    keys(&mut app, &[Key::Down, Key::Right]);
    assert_eq!(app.screen, Screen::Entries);
    assert_eq!(app.entries()[0].id, "y");
    keys(&mut app, &[Key::Esc]);
    assert_eq!((app.screen, app.cursor()), (Screen::Categories, 1));
    keys(&mut app, &[Key::Left]);
    assert_eq!((app.screen, app.cursor()), (Screen::Overview, 1));
    keys(&mut app, &[Key::Up, Key::Up]);
    assert_eq!(app.cursor(), 0);
}

#[test]
fn review_waits_for_scan() {
    let mut app = scanning();
    keys(&mut app, &[Key::Char('d')]);
    assert_eq!(app.screen, Screen::Overview);
    assert!(matches!(&app.popup, Popup::Message(_)));
    // rows already there can be toggled
    keys(&mut app, &[Key::Esc, Key::Space]);
    assert_eq!(app.selected_total(), 7);
}

#[test]
fn review_groups_by_partition() {
    let mut app = app();
    keys(&mut app, &[Key::Down, Key::Space, Key::Char('d')]);
    assert_eq!(app.screen, Screen::Review);
    let lines = app.review_lines();
    let mounts: Vec<_> = lines
        .iter()
        .filter_map(|l| match l {
            ReviewLine::Partition {
                mount,
                size,
                free_after,
            } => Some((mount.clone(), *size, *free_after)),
            _ => None,
        })
        .collect();
    assert_eq!(
        mounts,
        [
            (PathBuf::from("/"), 190, 1190),
            (PathBuf::from("/home"), 7, 1007)
        ]
    );
    // GEFAHRLOS per category, PRÜFEN per entry
    let entries = lines
        .iter()
        .filter(|l| matches!(l, ReviewLine::Entry { .. }))
        .count();
    let cats = lines
        .iter()
        .filter(|l| matches!(l, ReviewLine::Category { .. }))
        .count();
    assert_eq!((cats, entries), (3, 1));
    // ␣ unticks the line under the cursor; Esc goes back where we came from
    keys(&mut app, &[Key::Space]);
    assert_eq!(app.selected_total(), 47);
    keys(&mut app, &[Key::Esc]);
    assert_eq!((app.screen, app.cursor()), (Screen::Overview, 1));
}

#[test]
fn empty_review_has_no_confirm() {
    let mut app = app();
    keys(
        &mut app,
        &[Key::Space, Key::Down, Key::Down, Key::Down, Key::Space],
    );
    assert_eq!(app.selected_total(), 0);
    keys(&mut app, &[Key::Char('d')]);
    assert_eq!(app.screen, Screen::Overview);
    assert!(matches!(&app.popup, Popup::Message(_)));
}

#[test]
fn confirm_builds_plan_of_ticked() {
    let mut app = app();
    // tick containers, untick trash b
    keys(
        &mut app,
        &[
            Key::Down,
            Key::Space,
            Key::Up,
            Key::Enter,
            Key::Right,
            Key::Down,
            Key::Space,
        ],
    );
    keys(&mut app, &[Key::Char('d'), Key::Enter]);
    assert_eq!(app.popup, Popup::Confirm);
    assert_eq!(keys(&mut app, &[Key::Esc]), Action::None);
    assert_eq!(app.popup, Popup::None);
    keys(&mut app, &[Key::Enter]);
    let Action::Delete(plan) = keys(&mut app, &[Key::Char('j')]) else {
        panic!("no delete");
    };
    let mut got: Vec<(String, Safety, Vec<String>)> = plan
        .categories
        .iter()
        .map(|c| {
            (
                c.key.clone(),
                c.safety,
                c.items.iter().map(|i| i.id.clone()).collect(),
            )
        })
        .collect();
    got.sort();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        got,
        [
            ("build_cache".into(), Safety::Safe, s(&["c1", "c2"])),
            ("build_stale".into(), Safety::Safe, s(&["h1"])),
            ("containers".into(), Safety::Check, s(&["x"])),
            ("trash".into(), Safety::Safe, s(&["a"])),
        ]
    );
    let mut modules = plan.modules.clone();
    modules.sort();
    assert_eq!(modules, ["docker", "trash"]);
    assert_eq!(app.screen, Screen::Deleting);
    assert_eq!(app.progress.len(), 4);
}

#[test]
fn quit_refused_while_deleting() {
    let mut app = app();
    assert!(matches!(
        keys(&mut app, &[Key::Char('d'), Key::Enter, Key::Char('j')]),
        Action::Delete(_)
    ));
    assert_eq!(keys(&mut app, &[Key::Char('q')]), Action::None);
    assert_eq!(
        app.popup,
        Popup::Message("Löschen läuft – bitte warten".into())
    );
    assert_eq!(app.screen, Screen::Deleting);
}

#[test]
fn quit_elsewhere() {
    let mut app = app();
    assert_eq!(keys(&mut app, &[Key::Enter, Key::Char('q')]), Action::Quit);
}

#[test]
fn report_then_rescan() {
    let mut app = app();
    keys(&mut app, &[Key::Char('d'), Key::Enter, Key::Char('j')]);
    app.on_delete(DeleteEvent::Rechecking);
    assert!(app.rechecking);
    app.on_delete(DeleteEvent::Started("trash".into()));
    app.on_delete(DeleteEvent::Output("  Fehler: x".into()));
    assert_eq!(app.log, ["  Fehler: x"]);
    app.on_delete(DeleteEvent::Finished(Report::error("x".into())));
    assert_eq!(app.screen, Screen::Report);
    assert!(!app.rescan_requested);
    keys(&mut app, &[Key::Char('x')]);
    assert!(app.rescan_requested);
    app.reset_scan();
    assert!(app.rows().is_empty() && !app.scan_done && app.report.is_none());
    assert_eq!(app.screen, Screen::Overview);
}
