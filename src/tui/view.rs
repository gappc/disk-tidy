//! Drawing the TUI from the app state: header (where you are, selected total), body, footer
//! (the keys of this screen), popups on top.
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
};

use crate::clean::{Reason, Status};
use crate::config::home;
use crate::model::{Category, Item, Safety};
use crate::size::fmt_size;
use crate::time::fmt_age;
use crate::tui::app::{App, Mark, Popup, ReviewLine, Screen};

/// Colours, or plain text when colour is off (`NO_COLOR`).
struct Paint {
    colour: bool,
}

impl Paint {
    fn fg(&self, c: Color) -> Style {
        if self.colour {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }

    fn dim(&self) -> Style {
        if self.colour {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new()
        }
    }

    fn level(&self, s: Safety) -> Style {
        self.fg(level_colour(s))
    }
}

fn level_colour(s: Safety) -> Color {
    match s {
        Safety::Safe => Color::Green,
        Safety::Check => Color::Yellow,
        Safety::Data => Color::Red,
        Safety::Info => Color::Blue,
    }
}

/// Symbol and word: colour is never the only signal.
pub fn level_name(s: Safety) -> &'static str {
    match s {
        Safety::Safe => "✓ gefahrlos",
        Safety::Check => "⚠ prüfen",
        Safety::Data => "✗ vorsicht",
        Safety::Info => "ℹ nur info",
    }
}

fn size(n: u64) -> String {
    fmt_size(n).trim().to_string()
}

fn mark(m: Mark) -> &'static str {
    match m {
        Mark::On => "☑",
        Mark::Off => "☐",
        Mark::Partial => "◐",
        Mark::Whole => "·",
        Mark::Fixed => " ",
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `s` in at most `width` characters, the middle replaced by "…".
pub fn shorten(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let head = (width - 1) / 2;
    let tail = width - 1 - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// What an entry is called on screen: its path (home as "~"), else its label without padding.
fn entry_name(i: &Item) -> String {
    match &i.path {
        Some(p) => match p.strip_prefix(home()) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => p.display().to_string(),
        },
        None => i.label.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

pub fn draw(frame: &mut Frame, app: &App, colour: bool) {
    let p = Paint { colour };
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (title, keys) = match app.screen {
        Screen::Overview => overview(frame, app, &p, body),
        Screen::Categories => categories(frame, app, &p, body),
        Screen::Entries => entries(frame, app, &p, body),
        Screen::Review => review(frame, app, &p, body),
        Screen::Deleting | Screen::Report => progress(frame, app, &p, body),
    };
    let right = match app.screen {
        Screen::Overview => "? Hilfe  q Ende",
        Screen::Deleting => "",
        Screen::Report => "",
        _ => "Esc zurück",
    };
    let pad = (head.width as usize).saturating_sub(title.chars().count() + right.chars().count());
    let header = format!("{title}{}{right}", " ".repeat(pad));
    frame.render_widget(
        Paragraph::new(header).style(Style::new().add_modifier(Modifier::REVERSED)),
        head,
    );
    frame.render_widget(Paragraph::new(keys).style(p.dim()), foot);

    match &app.popup {
        Popup::None => {}
        Popup::Details => details(frame, app, &p),
        Popup::Help => help(frame),
        Popup::Confirm => confirm(frame, app, &p),
        Popup::Message(m) => message(frame, m),
    }
}

fn auswahl(app: &App) -> String {
    format!("Auswahl {}", size(app.selected_total()))
}

fn overview(frame: &mut Frame, app: &App, p: &Paint, area: Rect) -> (String, String) {
    let rows = app.rows();
    let mut last_mount = None;
    let table_rows: Vec<Row> = rows
        .iter()
        .map(|r| {
            let first = last_mount != Some(&r.mount);
            last_mount = Some(&r.mount);
            let selectable = matches!(r.safety, Safety::Safe | Safety::Check);
            let selected = if selectable {
                size(r.selected)
            } else {
                "–".into()
            };
            let free = if first {
                size(app.free_after(&r.mount))
            } else {
                String::new()
            };
            Row::new(vec![
                Cell::from(if first {
                    r.mount.display().to_string()
                } else {
                    String::new()
                }),
                Cell::from(level_name(r.safety)).style(p.level(r.safety)),
                Cell::from(Line::from(r.entries.to_string()).right_aligned()),
                Cell::from(Line::from(size(r.size)).right_aligned()),
                Cell::from(Line::from(selected).right_aligned()),
                Cell::from(Line::from(free).right_aligned()),
            ])
        })
        .collect();
    let mut notes: Vec<Line> = Vec::new();
    if let Some(m) = &app.scanning {
        notes.push(Line::from(format!("Scanne {m} …")).style(p.dim()));
    }
    for (m, note) in &app.failed {
        notes.push(Line::from(format!("Hinweis {m}: {note}")).style(p.fg(Color::Yellow)));
    }
    if app.scan_done && rows.is_empty() {
        notes.push(Line::from("Nichts gefunden."));
    }
    let [table_area, notes_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(notes.len() as u16)]).areas(area);
    let table = Table::new(
        table_rows,
        [
            Constraint::Max(14),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("Partition"),
            Cell::from("Stufe"),
            Cell::from(Line::from("Einträge").right_aligned()),
            Cell::from(Line::from("Größe").right_aligned()),
            Cell::from(Line::from("ausgewählt").right_aligned()),
            Cell::from(Line::from("frei danach").right_aligned()),
        ])
        .style(Style::new().add_modifier(Modifier::BOLD))
        .bottom_margin(1),
    )
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = TableState::default().with_selected((!rows.is_empty()).then(|| app.cursor()));
    frame.render_stateful_widget(table, table_area, &mut state);
    frame.render_widget(Paragraph::new(notes), notes_area);
    (
        format!(" disk-tidy · {}", auswahl(app)),
        " ↑↓ wählen  ⏎ öffnen  ␣ ganze Zeile an/aus  d prüfen & löschen  q Ende".into(),
    )
}

fn breadcrumb(app: &App) -> String {
    match app.open_row() {
        Some((m, s)) => format!(" {} › {}", m.display(), level_name(s)),
        None => " disk-tidy".into(),
    }
}

/// Grey hint of a category: entry count, oldest age, "nur als Ganzes".
fn cat_hint(c: &Category) -> String {
    let mut parts = vec![format!("{} Einträge", c.items.len())];
    if let Some(oldest) = c.items.iter().filter_map(|i| i.modified).min() {
        parts.push(format!("ältestes {}", fmt_age(now(), oldest)));
    }
    if !c.selectable() {
        parts.push("nicht auswählbar".into());
    } else if c.all_or_nothing() {
        parts.push("nur als Ganzes".into());
    }
    parts.join(" · ")
}

fn fixed_row(app: &App) -> bool {
    matches!(app.open_row(), Some((_, Safety::Data | Safety::Info)))
}

fn categories(frame: &mut Frame, app: &App, p: &Paint, area: Rect) -> (String, String) {
    let cats = app.categories();
    let rows: Vec<Row> = cats
        .iter()
        .map(|c| {
            Row::new(vec![
                Cell::from(mark(app.mark_cat(c))),
                Cell::from(Line::from(size(c.total())).right_aligned()),
                Cell::from(c.title.clone()).style(p.level(c.safety)),
                Cell::from(cat_hint(c)).style(p.dim()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Fill(3),
            Constraint::Fill(2),
        ],
    )
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = TableState::default().with_selected((!cats.is_empty()).then(|| app.cursor()));
    frame.render_stateful_widget(table, area, &mut state);
    (
        format!(
            "{} · {} Kategorien · {}",
            breadcrumb(app),
            cats.len(),
            auswahl(app)
        ),
        if fixed_row(app) {
            " nicht auswählbar · → Einträge  i Details  ← zurück  d prüfen & löschen".into()
        } else {
            " ␣ an/aus  → Einträge  i Details  ← zurück  d prüfen & löschen".into()
        },
    )
}

fn entries(frame: &mut Frame, app: &App, p: &Paint, area: Rect) -> (String, String) {
    let Some(c) = app.open_cat() else {
        return (breadcrumb(app), String::new());
    };
    let now = now();
    let name_width = (area.width as usize).saturating_sub(1 + 10 + 13 + 3);
    let rows: Vec<Row> = c
        .items
        .iter()
        .map(|i| {
            let age = i.modified.map(|t| fmt_age(now, t)).unwrap_or_default();
            Row::new(vec![
                Cell::from(mark(app.mark_item(c, i))),
                Cell::from(Line::from(size(i.size)).right_aligned()),
                Cell::from(age).style(p.dim()),
                Cell::from(shorten(&entry_name(i), name_width)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Length(13),
            Constraint::Fill(1),
        ],
    )
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state =
        TableState::default().with_selected((!c.items.is_empty()).then(|| app.cursor()));
    frame.render_stateful_widget(table, area, &mut state);
    let keys = if !c.selectable() {
        " nicht auswählbar · i Details  ← zurück  d prüfen & löschen".into()
    } else if c.all_or_nothing() {
        " nur als Ganzes: ␣ an/aus  i Details  ← zurück  d prüfen & löschen".into()
    } else {
        " ␣ an/aus  a alle  n keine  i Details  ← zurück  d prüfen & löschen".into()
    };
    (
        format!(
            "{} › {} · {} · {}",
            breadcrumb(app),
            c.title,
            c.items.len(),
            auswahl(app)
        ),
        keys,
    )
}

fn review(frame: &mut Frame, app: &App, p: &Paint, area: Rect) -> (String, String) {
    let cats = app.all_categories();
    let lines = app.review_lines();
    let mut cursor_line = None;
    let mut target = 0;
    let items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(n, l)| {
            if !matches!(l, ReviewLine::Partition { .. }) {
                if target == app.cursor() {
                    cursor_line = Some(n);
                }
                target += 1;
            }
            match l {
                ReviewLine::Partition {
                    mount,
                    size: s,
                    free_after,
                } => ListItem::new(Line::from(format!(
                    "{:<34} {:>10}   frei danach {}",
                    mount.display(),
                    size(*s),
                    size(*free_after)
                )))
                .style(Style::new().add_modifier(Modifier::BOLD)),
                ReviewLine::Category {
                    cat,
                    items,
                    of,
                    size: s,
                } => {
                    let c = &cats[*cat];
                    let title = if items == of {
                        c.title.clone()
                    } else {
                        format!("{} ({items} von {of})", c.title)
                    };
                    let sudo = if c.needs_sudo() { " (sudo)" } else { "" };
                    ListItem::new(Line::from(format!(
                        "  ☑ {:<30} {:>10}",
                        shorten(&format!("{title}{sudo}"), 30),
                        size(*s)
                    )))
                }
                ReviewLine::Entry { cat, item } => {
                    let c = &cats[*cat];
                    let i = &c.items[*item];
                    ListItem::new(Line::from(vec![
                        Span::raw("  ☑ "),
                        Span::styled(
                            format!(
                                "⚠ {:<28} {:>10}   prüfen: {}",
                                shorten(&entry_name(i), 28),
                                size(i.size),
                                c.why
                            ),
                            p.fg(Color::Yellow),
                        ),
                    ]))
                }
            }
        })
        .collect();
    let sudo = cats
        .iter()
        .filter(|c| c.needs_sudo() && matches!(app.mark_cat(c), Mark::On | Mark::Partial))
        .count();
    let note_height = u16::from(sudo > 0) * 2;
    let [list_area, note_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(note_height)]).areas(area);
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(cursor_line);
    frame.render_stateful_widget(list, list_area, &mut state);
    if sudo > 0 {
        frame.render_widget(
            Paragraph::new(format!(
                "\nsudo nötig für {sudo} Kategorien – Passwort wird beim Löschen abgefragt."
            ))
            .style(p.dim()),
            note_area,
        );
    }
    (
        format!(" Zu löschen · {}", size(app.selected_total())),
        " ␣ abwählen  ⏎ löschen…  i Details  Esc zurück zur Auswahl".into(),
    )
}

fn reason(r: Reason) -> &'static str {
    match r {
        Reason::Gone => "verschwunden",
        Reason::NotSafe => "nicht mehr gefahrlos",
        Reason::ModuleFailed => "Modul ausgefallen",
    }
}

fn progress(frame: &mut Frame, app: &App, p: &Paint, area: Rect) -> (String, String) {
    let mut lines: Vec<Line> = Vec::new();
    if app.rechecking {
        lines.push(Line::from("Prüfe neu …").style(p.dim()));
        lines.push(Line::from(""));
    }
    let (mut freed, mut total, mut finished) = (0, 0, 0);
    for (key, title, result) in &app.progress {
        let line = match result {
            Some(r) => {
                finished += 1;
                let s: u64 = r.items.iter().map(|i| i.size).sum();
                total += s;
                match r.status {
                    Status::Removed => {
                        freed += s;
                        Line::from(vec![
                            Span::styled("✓", p.fg(Color::Green)),
                            Span::raw(format!(" {title:<40} {:>10}", size(s))),
                        ])
                    }
                    Status::Failed => Line::from(vec![
                        Span::styled("✗", p.fg(Color::Red)),
                        Span::raw(format!(" {title:<40} ")),
                        Span::styled("Fehler (siehe unten)", p.fg(Color::Red)),
                    ]),
                    Status::SkippedSudo => Line::from(format!("– {title:<40} übersprungen (sudo)")),
                    Status::SkippedPartial => {
                        Line::from(format!("– {title:<40} übersprungen (nicht vollständig)"))
                    }
                }
            }
            None if app.running.as_deref() == Some(key) => Line::from(format!("… {title}")),
            None => Line::from(format!("  {title}")).style(p.dim()),
        };
        lines.push(line);
    }
    if let Some(report) = &app.report {
        if !report.dropped.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from("nicht mehr gültig, nicht gelöscht:"));
            for d in &report.dropped {
                lines.push(Line::from(format!("  {} ({})", d.id, reason(d.reason))).style(p.dim()));
            }
        }
        if !report.skipped_sudo.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from("Ohne sudo übersprungen. Selbst ausführen:"));
            for cmd in &report.skipped_sudo {
                lines.push(Line::from(format!("  {cmd}")));
            }
        }
    }
    if !app.log.is_empty() {
        lines.push(Line::from(""));
        lines.extend(app.log.iter().map(|l| Line::from(l.clone()).style(p.dim())));
    }
    // Keep the end in view: the newest lines matter most.
    let scroll = (lines.len() as u16).saturating_sub(area.height.saturating_sub(2));
    let [text_area, bar_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).areas(area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), text_area);
    let planned: u64 = app.selected_total().max(total).max(1);
    let width = 30;
    let filled = ((freed.min(planned) * width) / planned) as usize;
    let bar = format!(
        "\n{}{}  frei gemacht {}",
        "█".repeat(filled),
        "░".repeat(width as usize - filled),
        size(app.report.as_ref().map_or(freed, |r| r.freed.max(freed)))
    );
    frame.render_widget(Paragraph::new(bar), bar_area);
    let n = app.progress.len();
    match app.screen {
        Screen::Report => (
            format!(" Bericht · {finished} von {n} fertig"),
            " Taste drücken: neu scannen".into(),
        ),
        _ => (
            format!(" Löschen · {finished} von {n} fertig"),
            " bitte warten …".into(),
        ),
    }
}

/// A rectangle of at most `w`×`h`, centred in `area`.
fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn popup(frame: &mut Frame, title: &str, bottom: &str, lines: Vec<Line>) {
    let width = lines
        .iter()
        .map(Line::width)
        .chain([title.chars().count() + 4, bottom.chars().count() + 4])
        .max()
        .unwrap_or(0) as u16
        + 4;
    let area = centred(frame.area(), width.max(30), lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .title_bottom(Line::from(format!(" {bottom} ")).right_aligned());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn details(frame: &mut Frame, app: &App, p: &Paint) {
    let Some((c, item)) = app.current() else {
        return;
    };
    let (name, s) = match item {
        Some(i) => (entry_name(i), i.size),
        None => (c.title.clone(), c.total()),
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(level_name(c.safety), p.level(c.safety)),
            Span::raw(format!(" · {}", size(s))),
        ]),
        Line::from(c.why.clone()),
    ];
    if let Some(i) = item {
        for (k, v) in &i.facts {
            lines.push(Line::from(format!("{k}: {v}")));
        }
        if let Some(t) = i.modified {
            lines.push(Line::from(format!("Alter: {}", fmt_age(now(), t))));
        }
    } else {
        lines.push(Line::from(cat_hint(c)));
    }
    if !c.hint.is_empty() {
        lines.push(Line::from(format!("Von Hand: {}", c.hint)).style(p.dim()));
    }
    popup(frame, &name, "Esc schließen", lines);
}

fn confirm(frame: &mut Frame, app: &App, p: &Paint) {
    let mut lines = vec![Line::from(format!(
        "{} in {} Kategorien.",
        size(app.selected_total()),
        app.selected_categories()
    ))];
    let (n, s) = app.selected_check();
    if n > 0 {
        lines.push(
            Line::from(format!("Davon ⚠ prüfen: {n} Einträge, {}.", size(s)))
                .style(p.fg(Color::Yellow)),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from("[ j  Löschen ]     [ Esc  Abbrechen ]"));
    popup(frame, "Löschen?", "", lines);
}

fn help(frame: &mut Frame) {
    let lines = [
        "↑ ↓       Zeile wählen",
        "⏎ →       öffnen",
        "← Esc     zurück",
        "␣         an/aus (Zeile, Kategorie, Eintrag)",
        "a n       alle / keine Einträge",
        "i         Details",
        "d         prüfen & löschen",
        "q         Ende",
        "",
        "✓ gefahrlos  vorausgewählt",
        "⚠ prüfen     wählbar, nie vorausgewählt",
        "✗ vorsicht   nur von Hand",
        "ℹ nur info   nur zur Übersicht",
    ];
    popup(
        frame,
        "Tasten",
        "Esc schließen",
        lines.into_iter().map(Line::from).collect(),
    );
}

fn message(frame: &mut Frame, text: &str) {
    popup(
        frame,
        "Hinweis",
        "Taste drücken",
        vec![Line::from(text.to_string())],
    );
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
