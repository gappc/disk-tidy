//! The TUI's state: what was found, what is ticked, where the cursor is. Pure data, changed only
//! by `on_scan`, `on_key` and `on_delete`, so it is tested without a terminal.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::clean::{CatResult, Report};
use crate::model::{Category, Item, SAFETY_ORDER, Safety};
use crate::partition::Usage;
use crate::plan::{PLAN_VERSION, Plan, PlanCategory, PlanItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Space,
    Char(char),
}

/// From the scan worker.
pub enum ScanEvent {
    Started(String),
    Done(String, Vec<Category>),
    Failed(String, String),
    Finished,
}

/// From the delete worker.
pub enum DeleteEvent {
    Rechecking,
    /// Category key.
    Started(String),
    Output(String),
    Done(CatResult),
    Finished(Report),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Categories,
    Entries,
    Review,
    Deleting,
    Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    None,
    Details,
    Help,
    Confirm,
    Message(String),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Delete(Plan),
}

/// The checkbox of a category or entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    On,
    Off,
    Partial,
    /// An entry of an all-or-nothing category: no box of its own.
    Whole,
    /// Not selectable.
    Fixed,
}

/// One overview line: a partition and a level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub mount: PathBuf,
    pub safety: Safety,
    pub entries: usize,
    pub size: u64,
    pub selected: u64,
}

/// One line of the review. `cat` indexes `App::all_categories`, `item` that category's items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewLine {
    Partition {
        mount: PathBuf,
        size: u64,
        free_after: u64,
    },
    /// The ticked part of a GEFAHRLOS category.
    Category {
        cat: usize,
        items: usize,
        of: usize,
        size: u64,
    },
    /// A ticked PRÜFEN entry, shown on its own.
    Entry { cat: usize, item: usize },
}

pub struct App {
    usage: fn(&Path) -> Usage,
    mount: fn(&Path) -> PathBuf,
    /// Categories in arrival order, with their partition.
    cats: Vec<Category>,
    mounts: Vec<PathBuf>,
    selected: HashSet<(String, String)>,
    pub screen: Screen,
    pub popup: Popup,
    /// Where Esc leaves the review to.
    back: Screen,
    row_cursor: usize,
    cat_cursor: usize,
    entry_cursor: usize,
    review_cursor: usize,
    open_row: Option<(PathBuf, Safety)>,
    open_cat: Option<usize>,
    /// The module being scanned.
    pub scanning: Option<String>,
    /// (module, note) of modules that could not run.
    pub failed: Vec<(String, String)>,
    pub scan_done: bool,
    /// (key, title, result) per category being deleted; None = pending or running.
    pub progress: Vec<(String, String, Option<CatResult>)>,
    /// Key of the category being deleted.
    pub running: Option<String>,
    pub rechecking: bool,
    /// Remover and runner messages.
    pub log: Vec<String>,
    pub report: Option<Report>,
    /// A key was pressed on the report: the loop starts a new scan and calls `reset_scan`.
    pub rescan_requested: bool,
}

fn sel_key(c: &Category, i: &Item) -> (String, String) {
    (c.key.clone(), i.id.clone())
}

impl App {
    pub fn new(usage: fn(&Path) -> Usage, mount: fn(&Path) -> PathBuf) -> App {
        App {
            usage,
            mount,
            cats: Vec::new(),
            mounts: Vec::new(),
            selected: HashSet::new(),
            screen: Screen::Overview,
            popup: Popup::None,
            back: Screen::Overview,
            row_cursor: 0,
            cat_cursor: 0,
            entry_cursor: 0,
            review_cursor: 0,
            open_row: None,
            open_cat: None,
            scanning: None,
            failed: Vec::new(),
            scan_done: false,
            progress: Vec::new(),
            running: None,
            rechecking: false,
            log: Vec::new(),
            report: None,
            rescan_requested: false,
        }
    }

    /// Forget the last scan and selection, for a new scan.
    pub fn reset_scan(&mut self) {
        *self = App::new(self.usage, self.mount);
    }

    pub fn on_scan(&mut self, ev: ScanEvent) {
        match ev {
            ScanEvent::Started(name) => self.scanning = Some(name),
            ScanEvent::Done(_, cats) => {
                for c in cats {
                    if c.safety == Safety::Safe {
                        self.selected.extend(c.items.iter().map(|i| sel_key(&c, i)));
                    }
                    self.mounts.push((self.mount)(&c.path));
                    self.cats.push(c);
                }
            }
            ScanEvent::Failed(name, note) => self.failed.push((name, note)),
            ScanEvent::Finished => {
                self.scanning = None;
                self.scan_done = true;
            }
        }
    }

    pub fn on_delete(&mut self, ev: DeleteEvent) {
        match ev {
            DeleteEvent::Rechecking => self.rechecking = true,
            DeleteEvent::Started(key) => {
                self.rechecking = false;
                self.running = Some(key);
            }
            DeleteEvent::Output(line) => self.log.push(line),
            DeleteEvent::Done(r) => {
                self.running = None;
                match self.progress.iter_mut().find(|(k, _, _)| *k == r.key) {
                    Some(p) => p.2 = Some(r),
                    None => self.progress.push((r.key.clone(), r.key.clone(), Some(r))),
                }
            }
            DeleteEvent::Finished(report) => {
                self.rechecking = false;
                self.running = None;
                self.report = Some(report);
                self.screen = Screen::Report;
                self.popup = Popup::None;
            }
        }
    }

    // ---- read access for the view ----

    pub fn all_categories(&self) -> &[Category] {
        &self.cats
    }

    /// Partitions in order of first appearance.
    fn partitions(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = Vec::new();
        for m in &self.mounts {
            if !v.contains(m) {
                v.push(m.clone());
            }
        }
        v
    }

    /// Indexes of the categories on `mount` with level `safety`, largest first.
    fn indexes(&self, mount: &Path, safety: Safety) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.cats.len())
            .filter(|&i| self.mounts[i] == mount && self.cats[i].safety == safety)
            .collect();
        v.sort_by_key(|&i| std::cmp::Reverse(self.cats[i].total()));
        v
    }

    fn selected_size(&self, c: &Category) -> u64 {
        c.items
            .iter()
            .filter(|i| self.is_selected(c, i))
            .map(|i| i.size)
            .sum()
    }

    fn is_selected(&self, c: &Category, i: &Item) -> bool {
        c.selectable() && self.selected.contains(&sel_key(c, i))
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for mount in self.partitions() {
            for safety in SAFETY_ORDER {
                let idx = self.indexes(&mount, safety);
                if idx.is_empty() {
                    continue;
                }
                let cats = idx.iter().map(|&i| &self.cats[i]);
                rows.push(Row {
                    mount: mount.clone(),
                    safety,
                    entries: cats.clone().map(|c| c.items.len()).sum(),
                    size: cats.clone().map(Category::total).sum(),
                    selected: cats.map(|c| self.selected_size(c)).sum(),
                });
            }
        }
        rows
    }

    /// The open row, if any.
    pub fn open_row(&self) -> Option<(&Path, Safety)> {
        self.open_row.as_ref().map(|(m, s)| (m.as_path(), *s))
    }

    fn open_indexes(&self) -> Vec<usize> {
        match &self.open_row {
            Some((m, s)) => self.indexes(m, *s),
            None => Vec::new(),
        }
    }

    /// Categories of the open row, largest first.
    pub fn categories(&self) -> Vec<&Category> {
        self.open_indexes()
            .into_iter()
            .map(|i| &self.cats[i])
            .collect()
    }

    pub fn open_cat(&self) -> Option<&Category> {
        self.open_cat.map(|i| &self.cats[i])
    }

    /// Entries of the open category, largest first.
    pub fn entries(&self) -> &[Item] {
        self.open_cat().map_or(&[], |c| &c.items)
    }

    pub fn cursor(&self) -> usize {
        match self.screen {
            Screen::Overview => self.row_cursor,
            Screen::Categories => self.cat_cursor,
            Screen::Entries => self.entry_cursor,
            Screen::Review => self.review_cursor,
            Screen::Deleting | Screen::Report => 0,
        }
    }

    pub fn mark_cat(&self, c: &Category) -> Mark {
        if !c.selectable() {
            return Mark::Fixed;
        }
        let on = c.items.iter().filter(|i| self.is_selected(c, i)).count();
        match on {
            0 => Mark::Off,
            n if n == c.items.len() => Mark::On,
            _ => Mark::Partial,
        }
    }

    pub fn mark_item(&self, c: &Category, i: &Item) -> Mark {
        if !c.selectable() {
            Mark::Fixed
        } else if c.all_or_nothing() {
            Mark::Whole
        } else if self.is_selected(c, i) {
            Mark::On
        } else {
            Mark::Off
        }
    }

    pub fn selected_total(&self) -> u64 {
        self.cats.iter().map(|c| self.selected_size(c)).sum()
    }

    /// Ticked PRÜFEN entries: count and size.
    pub fn selected_check(&self) -> (usize, u64) {
        self.cats
            .iter()
            .filter(|c| c.safety == Safety::Check)
            .flat_map(|c| c.items.iter().filter(move |i| self.is_selected(c, i)))
            .fold((0, 0), |(n, s), i| (n + 1, s + i.size))
    }

    /// Categories with something ticked.
    pub fn selected_categories(&self) -> usize {
        self.cats
            .iter()
            .filter(|c| c.items.iter().any(|i| self.is_selected(c, i)))
            .count()
    }

    /// Free space on `mount` once the ticked entries there are gone.
    pub fn free_after(&self, mount: &Path) -> u64 {
        let selected: u64 = (0..self.cats.len())
            .filter(|&i| self.mounts[i] == mount)
            .map(|i| self.selected_size(&self.cats[i]))
            .sum();
        (self.usage)(mount).free + selected
    }

    pub fn usage(&self, mount: &Path) -> Usage {
        (self.usage)(mount)
    }

    pub fn review_lines(&self) -> Vec<ReviewLine> {
        let mut lines = Vec::new();
        for mount in self.partitions() {
            let mut body = Vec::new();
            let mut size = 0;
            for safety in SAFETY_ORDER {
                for i in self.indexes(&mount, safety) {
                    let c = &self.cats[i];
                    let ticked: Vec<usize> = (0..c.items.len())
                        .filter(|&j| self.is_selected(c, &c.items[j]))
                        .collect();
                    if ticked.is_empty() {
                        continue;
                    }
                    let s: u64 = ticked.iter().map(|&j| c.items[j].size).sum();
                    size += s;
                    if safety == Safety::Check {
                        body.extend(
                            ticked
                                .iter()
                                .map(|&j| ReviewLine::Entry { cat: i, item: j }),
                        );
                    } else {
                        body.push(ReviewLine::Category {
                            cat: i,
                            items: ticked.len(),
                            of: c.items.len(),
                            size: s,
                        });
                    }
                }
            }
            if !body.is_empty() {
                lines.push(ReviewLine::Partition {
                    free_after: self.free_after(&mount),
                    mount,
                    size,
                });
                lines.extend(body);
            }
        }
        lines
    }

    /// Review lines the cursor can stand on.
    fn review_targets(&self) -> Vec<ReviewLine> {
        self.review_lines()
            .into_iter()
            .filter(|l| !matches!(l, ReviewLine::Partition { .. }))
            .collect()
    }

    /// The category and entry the details popup shows.
    pub fn current(&self) -> Option<(&Category, Option<&Item>)> {
        match self.screen {
            Screen::Categories => self
                .open_indexes()
                .get(self.cat_cursor)
                .map(|&i| (&self.cats[i], None)),
            Screen::Entries => {
                let c = self.open_cat()?;
                Some((c, c.items.get(self.entry_cursor)))
            }
            Screen::Review => match self.review_targets().get(self.review_cursor)? {
                ReviewLine::Category { cat, .. } => Some((&self.cats[*cat], None)),
                ReviewLine::Entry { cat, item } => {
                    let c = &self.cats[*cat];
                    Some((c, c.items.get(*item)))
                }
                ReviewLine::Partition { .. } => None,
            },
            _ => None,
        }
    }

    // ---- keys ----

    pub fn on_key(&mut self, key: Key) -> Action {
        match &self.popup {
            Popup::None => {}
            Popup::Confirm => {
                return match key {
                    Key::Char('j') => self.start_delete(),
                    Key::Esc | Key::Char('n') | Key::Char('q') => {
                        self.popup = Popup::None;
                        Action::None
                    }
                    _ => Action::None,
                };
            }
            Popup::Details | Popup::Help | Popup::Message(_) => {
                self.popup = Popup::None;
                return Action::None;
            }
        }
        match self.screen {
            Screen::Deleting => {
                if key == Key::Char('q') {
                    self.popup = Popup::Message("Löschen läuft – bitte warten".into());
                }
                return Action::None;
            }
            Screen::Report => {
                self.rescan_requested = true;
                return Action::None;
            }
            _ => {}
        }
        match key {
            Key::Char('q') => return Action::Quit,
            Key::Char('?') => self.popup = Popup::Help,
            Key::Char('d') if self.screen != Screen::Review => self.open_review(),
            Key::Up => self.move_cursor(-1),
            Key::Down => self.move_cursor(1),
            Key::Space => self.toggle_here(),
            Key::Char('i') if self.current().is_some() => self.popup = Popup::Details,
            _ => self.screen_key(key),
        }
        Action::None
    }

    fn screen_key(&mut self, key: Key) {
        match (self.screen, key) {
            (Screen::Overview, Key::Enter | Key::Right) => {
                if let Some(row) = self.rows().get(self.row_cursor) {
                    self.open_row = Some((row.mount.clone(), row.safety));
                    self.cat_cursor = 0;
                    self.screen = Screen::Categories;
                }
            }
            (Screen::Categories, Key::Enter | Key::Right) => {
                if let Some(&i) = self.open_indexes().get(self.cat_cursor) {
                    self.open_cat = Some(i);
                    self.entry_cursor = 0;
                    self.screen = Screen::Entries;
                }
            }
            (Screen::Categories, Key::Esc | Key::Left) => self.screen = Screen::Overview,
            (Screen::Entries, Key::Esc | Key::Left) => self.screen = Screen::Categories,
            (Screen::Entries, Key::Char('a')) => self.set_open_cat(true),
            (Screen::Entries, Key::Char('n')) => self.set_open_cat(false),
            (Screen::Review, Key::Esc | Key::Left) => self.screen = self.back,
            (Screen::Review, Key::Enter) => {
                self.popup = if self.any_selected() {
                    Popup::Confirm
                } else {
                    Popup::Message("Nichts ausgewählt.".into())
                }
            }
            _ => {}
        }
    }

    fn any_selected(&self) -> bool {
        self.cats
            .iter()
            .any(|c| c.items.iter().any(|i| self.is_selected(c, i)))
    }

    fn screen_len(&self) -> usize {
        match self.screen {
            Screen::Overview => self.rows().len(),
            Screen::Categories => self.open_indexes().len(),
            Screen::Entries => self.entries().len(),
            Screen::Review => self.review_targets().len(),
            Screen::Deleting | Screen::Report => 0,
        }
    }

    fn move_cursor(&mut self, by: isize) {
        let len = self.screen_len();
        let cur = match self.screen {
            Screen::Overview => &mut self.row_cursor,
            Screen::Categories => &mut self.cat_cursor,
            Screen::Entries => &mut self.entry_cursor,
            Screen::Review => &mut self.review_cursor,
            Screen::Deleting | Screen::Report => return,
        };
        *cur = cur.saturating_add_signed(by).min(len.saturating_sub(1));
    }

    fn open_review(&mut self) {
        if !self.scan_done {
            self.popup = Popup::Message("Scan läuft noch – bitte warten".into());
        } else if !self.any_selected() {
            self.popup = Popup::Message("Nichts ausgewählt.".into());
        } else {
            self.back = self.screen;
            self.review_cursor = 0;
            self.screen = Screen::Review;
        }
    }

    /// All on, unless all are on already: then all off.
    fn toggle(&mut self, keys: Vec<(String, String)>) {
        if keys.iter().all(|k| self.selected.contains(k)) {
            for k in &keys {
                self.selected.remove(k);
            }
        } else {
            self.selected.extend(keys);
        }
    }

    fn keys_of(c: &Category) -> Vec<(String, String)> {
        c.items.iter().map(|i| sel_key(c, i)).collect()
    }

    fn not_selectable(&mut self, c: &Category) {
        let msg = if c.hint.is_empty() {
            format!("Nicht auswählbar – {}", c.why)
        } else {
            format!("Nicht auswählbar – nur von Hand: {}", c.hint)
        };
        self.popup = Popup::Message(msg);
    }

    fn toggle_here(&mut self) {
        match self.screen {
            Screen::Overview => {
                let Some(row) = self.rows().get(self.row_cursor).cloned() else {
                    return;
                };
                let idx = self.indexes(&row.mount, row.safety);
                let keys: Vec<_> = idx
                    .iter()
                    .map(|&i| &self.cats[i])
                    .filter(|c| c.selectable())
                    .flat_map(Self::keys_of)
                    .collect();
                if keys.is_empty() {
                    let c = self.cats[idx[0]].clone();
                    self.not_selectable(&c);
                } else {
                    self.toggle(keys);
                }
            }
            Screen::Categories => {
                if let Some(&i) = self.open_indexes().get(self.cat_cursor) {
                    self.toggle_cat(i);
                }
            }
            Screen::Entries => {
                let Some(i) = self.open_cat else { return };
                let c = &self.cats[i];
                if !c.selectable() || c.all_or_nothing() {
                    self.toggle_cat(i);
                } else if let Some(item) = c.items.get(self.entry_cursor) {
                    let k = sel_key(c, item);
                    self.toggle(vec![k]);
                }
            }
            Screen::Review => {
                let Some(line) = self.review_targets().get(self.review_cursor).cloned() else {
                    return;
                };
                match line {
                    ReviewLine::Category { cat, .. } => {
                        for k in Self::keys_of(&self.cats[cat]) {
                            self.selected.remove(&k);
                        }
                    }
                    ReviewLine::Entry { cat, item } => {
                        let k = sel_key(&self.cats[cat], &self.cats[cat].items[item]);
                        self.selected.remove(&k);
                    }
                    ReviewLine::Partition { .. } => {}
                }
                let len = self.review_targets().len();
                self.review_cursor = self.review_cursor.min(len.saturating_sub(1));
            }
            Screen::Deleting | Screen::Report => {}
        }
    }

    fn toggle_cat(&mut self, i: usize) {
        let c = &self.cats[i];
        if c.selectable() {
            let keys = Self::keys_of(c);
            self.toggle(keys);
        } else {
            let c = c.clone();
            self.not_selectable(&c);
        }
    }

    fn set_open_cat(&mut self, on: bool) {
        let Some(i) = self.open_cat else { return };
        let c = &self.cats[i];
        if !c.selectable() {
            let c = c.clone();
            return self.not_selectable(&c);
        }
        for k in Self::keys_of(c) {
            if on {
                self.selected.insert(k);
            } else {
                self.selected.remove(&k);
            }
        }
    }

    /// `j` in the confirm popup: the ticked entries as a plan, and the progress list.
    fn start_delete(&mut self) -> Action {
        let mut plan = Plan {
            version: PLAN_VERSION,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            modules: Vec::new(),
            partitions: Vec::new(),
            categories: Vec::new(),
        };
        self.progress.clear();
        for line in self.review_lines() {
            let cat = match line {
                ReviewLine::Category { cat, .. } | ReviewLine::Entry { cat, .. } => cat,
                ReviewLine::Partition { .. } => continue,
            };
            let c = &self.cats[cat];
            if plan.categories.iter().any(|p| p.key == c.key) {
                continue; // a PRÜFEN category has one line per entry
            }
            let items: Vec<PlanItem> = c
                .items
                .iter()
                .filter(|i| self.is_selected(c, i))
                .map(|i| PlanItem {
                    id: i.id.clone(),
                    label: i.label.clone(),
                    size: i.size,
                })
                .collect();
            if !plan.modules.contains(&c.module) {
                plan.modules.push(c.module.clone());
            }
            self.progress.push((c.key.clone(), c.title.clone(), None));
            plan.categories.push(PlanCategory {
                key: c.key.clone(),
                module: c.module.clone(),
                title: c.title.clone(),
                safety: c.safety,
                why: c.why.clone(),
                hint: c.hint.clone(),
                mount: self.mounts[cat].display().to_string(),
                total: items.iter().map(|i| i.size).sum(),
                items,
            });
        }
        self.popup = Popup::None;
        self.screen = Screen::Deleting;
        self.log.clear();
        self.report = None;
        Action::Delete(plan)
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
