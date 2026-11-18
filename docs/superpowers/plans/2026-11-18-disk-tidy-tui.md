# disk-tidy phase 3 (TUI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `disk-tidy` on a terminal opens a Ratatui TUI: overview → categories → entries → review → confirm → progress/report → rescan, with PRÜFEN selectable and a recheck before deleting.

**Architecture:** Model gains PRÜFEN removers, `selectable()`, and item facts. `plan::select` takes the allowed levels, so the TUI's selection becomes a `Plan` that is rechecked like `clean --plan`. `clean`'s removal loop is extracted into `execute` with a progress callback. `src/tui/` holds a pure `App` state machine (`app.rs`), rendering (`view.rs`) and terminal/threads (`mod.rs`, `worker.rs`).

**Tech Stack:** Rust 1.99 edition 2024; new dependency `ratatui = "0.30"` (default features; use its re-export `ratatui::crossterm`, so no separate crossterm version can drift).

**Spec:** `docs/superpowers/specs/2026-11-18-disk-tidy-tui-design.md`

## Global Constraints

- No `unsafe`, no tokio. Only new crate: ratatui (crossterm via its re-export).
- `preview`, `clean`, `plan` output and exit codes byte-identical; all existing tests and golden files pass unchanged (except the one docker assertion Task 1 replaces).
- `clean` and `clean --plan` remove GEFAHRLOS only; PRÜFEN only through the TUI.
- UI texts German; every level shown with symbol + word (✓ gefahrlos, ⚠ prüfen, ✗ vorsicht, ℹ nur info); `NO_COLOR` disables colour.
- Terminal always restored (raw mode off, alternate screen left, cursor shown), also on panic.
- Nothing writes to the terminal while the TUI runs: module notes, runner errors and remover output go through channels.
- Commits: no co-author trailer, no mention of AI.

## Review Focus

1. Hundreds of entries (many `node_modules`): the list scrolls and the cursor row stays visible (Task 6 `long_list_keeps_cursor_visible`).
2. Small terminal (60×16): drawing never panics, columns shrink (Task 6 `small_terminal_does_not_panic`).
3. A ticked GEFAHRLOS build dir whose project became active before `j` is not deleted (Task 3 `ticked_safe_now_check_is_dropped`).
4. Keys while the scan is still running: `d` does nothing until it finished; ␣ on rows already there works (Task 4 `review_waits_for_scan`).
5. A panic inside the TUI leaves a usable terminal: the panic hook and the guard's `Drop` call the same `restore()` (Task 7; checked by review and by the tmux run ending in a sane shell).

---

### Task 1: PRÜFEN removers and `selectable()`

**Files:** Modify `src/model.rs`, `src/modules/docker.rs`, `src/modules/projects.rs`, `src/clean.rs` (test only).

**Interfaces:**
- Produces: `Category::selectable(&self) -> bool` = `Safe`, or `Check` with `remover != Remover::None`.
  `Category::all_or_nothing(&self) -> bool` = remover is `Run { with_ids: false, .. }`.
- Docker: `unused_images` gets `run(&["docker", "rmi"], true)`, `containers` gets `run(&["docker", "rm"], true)`;
  `stopped_images` keeps no remover. Projects: `build_active` gets `Remover::BuildDirs` (hint stays).

- [ ] Step 1: tests:
  - model `selectable_levels`: Safe → true; Check with remover → true; Check without remover → false; Data/Info with a remover → false.
  - model `all_or_nothing_only_for_idless_run`.
  - docker: replace the assertion `c.is_clean() == (c.remover != Remover::None)` with: `is_clean()` ⇔ Safe;
    `unused_images` remover `docker rmi` with ids; `containers` `docker rm` with ids; `stopped_images` remover None (not selectable).
  - projects: `build_active` has `Remover::BuildDirs`, safety Check.
  - clean `check_categories_with_remover_are_not_cleaned`: `Selection::all` of a Check category with
    `run(["docker","rm"], true)` → todo empty, no runner calls.
- [ ] Step 2: `cargo test` → FAIL. Step 3: implement. Step 4: `cargo test` → pass (golden files unchanged).
- [ ] Step 5: commit "Give PRÜFEN categories removers for the TUI; clean still removes GEFAHRLOS only".

### Task 2: Item facts (age and notes)

**Files:** Create `src/time.rs`; modify `src/model.rs`, `src/modules/{projects,docker,trash}.rs`, every `Item { .. }` literal (add `..Default::default()`; derive `Default` on `Item`).

**Interfaces:**
- `Item` gains `#[serde(skip)] pub modified: Option<u64>` (Unix seconds) and `#[serde(skip)] pub facts: Vec<(String, String)>`.
- `time::parse_timestamp(s: &str) -> Option<u64>`: accepts `YYYY-MM-DD HH:MM:SS`, `YYYY-MM-DDTHH:MM:SS`, optionally followed by ` ±HHMM` and anything after (docker: `2025-06-01 12:00:00 +0200 CEST`); without offset UTC. Civil-date conversion by hand (days-from-civil), no new crate.
- `time::fmt_age(now: u64, then: u64) -> String`: `< 1 day` → "heute", `< 60 days` → "vor {d} Tg.", `< 2 years` → "vor {m} Mon." (m = days/30), else "vor {y} J.".
- Filled facts: build dirs `modified` = project last change, facts `[("Projekt", "~/…")]`; docker images `modified` = `CreatedAt`, facts `[("Image", "repo:tag")]`; containers `CreatedAt`, facts `[("Image", image), ("Status", status)]`; build cache `LastUsedAt`; trash `modified` = `DeletionDate` from `info/<name>.trashinfo`.
  (Add `created_at` to `DfImage`/`DfContainer` and `last_used_at` to `DfCache`.) Labels stay unchanged (golden output).

- [ ] Step 1: tests: `time::tests::parses_docker_and_trash_formats` (`"2026-10-08 12:00:00 +0200 CEST"` → 1791453600 (12:00 at +0200 is 10:00 UTC); `"2026-10-08T12:00:00"` → 1791460800; `"x"` → None);
  `fmt_age_steps` (0 s → "heute", 41 d → "vor 41 Tg.", 150 d → "vor 5 Mon.", 800 d → "vor 2 J.");
  projects `stale_item_has_project_fact`; docker `image_and_container_facts` (fixture with `CreatedAt`);
  trash `deletion_date_is_modified`.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: `cargo test` → pass. Commit "Record age and facts per entry".

### Task 3: `select` with allowed levels

**Files:** Modify `src/plan.rs`, `src/main.rs` (call site).

**Interfaces:**
- `pub fn select(plan: &Plan, scan: &Scan, levels: &[Safety]) -> Selection` — a scan category is eligible
  iff `levels.contains(&c.safety) && c.selectable()`; dropped reports plan items whose plan `safety` is in `levels`;
  `NotSafe` = id now in a scan category that is not eligible. `clean --plan` passes `&[Safety::Safe]`.

- [ ] Step 1: existing select tests pass `&[Safety::Safe]`; new:
  - `check_level_allowed_for_tui`: scan `build_active` (Check, BuildDirs) x; plan same, levels [Safe, Check] → todo [x].
  - `ticked_safe_now_check_is_dropped`: plan `build_stale` x (Safe); scan `build_active` x; levels [Safe, Check] → todo empty, dropped [x NotSafe].
  - `unselectable_check_never_selected`: scan `stopped_images` (Check, no remover) y; plan same; levels [Safe, Check] → todo empty, dropped [y NotSafe].
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass. Commit "Let select take the levels it may remove".

### Task 4: Shared removal with progress; quiet runner and per-module scan

**Files:** Modify `src/clean.rs`, `src/runner.rs`, `src/modules/mod.rs`.

**Interfaces:**
- `pub enum Progress<'a> { Started(&'a Category), Done(&'a CatResult) }`.
- `pub fn execute(sel: Selection, interactive: bool, runner: &mut dyn Runner, out: &mut dyn Write, progress: &mut dyn FnMut(Progress)) -> Report`
  — everything `clean` does after confirmation (removal loop, sudo-skipped list, "Fertig." line, outcome incl. partial).
  `clean` = summary/prompt + `execute`; text output unchanged.
- `SystemRunner` becomes `#[derive(Default)] pub struct SystemRunner { pub log: Option<std::sync::mpsc::Sender<String>> }`:
  with `log` set, `run` sends its error line instead of `eprintln!`, and `run_sudo` runs with stdin null, captures
  stdout+stderr and sends them (non-empty, trimmed) instead of inheriting the terminal. Existing call sites use `SystemRunner::default()`.
- `pub fn collect_module(name: &str, cfg: &Config, runner: &mut dyn Runner) -> Result<Vec<Category>, String>` —
  one module, empty categories dropped, `module` set; `collect_all` uses it (stderr note stays there).

- [ ] Step 1: tests: clean `execute_reports_progress_in_order` (two categories → events Started a, Done a, Started b, Done b);
  existing clean tests unchanged; runner `log_receives_error_instead_of_stderr` (`run(["false"])` with log → one message starting "  Fehler: false");
  modules `collect_module_tags_and_drops_empty` (docker fixture runner).
- [ ] Step 2: FAIL. Step 3: implement. Step 4: `cargo test` → pass, golden unchanged. Commit
  "Extract removal with progress events; let the runner report through a channel".

### Task 5: TUI state machine

**Files:** Create `src/tui/mod.rs` (module decl only for now), `src/tui/app.rs`; add `ratatui = "0.30"` to Cargo.toml; `mod tui;` in main.

**Interfaces (all in `app.rs`):**
- `pub enum Key { Up, Down, Left, Right, Enter, Esc, Space, Char(char) }` (mapped from crossterm in Task 7).
- `pub enum ScanEvent { Started(String), Done(String, Vec<Category>), Failed(String, String), Finished }`.
- `pub enum DeleteEvent { Rechecking, Started(String /*key*/), Output(String), Done(CatResult), Finished(Report) }`.
- `pub enum Screen { Overview, Categories, Entries, Review, Deleting, Report }`; `pub enum Popup { None, Details, Help, Confirm, Message(String) }`.
- `pub enum Action { None, Quit, Delete(Plan) }`.
- `pub enum Mark { On, Off, Partial, Whole /*entry of an all-or-nothing category: "·"*/, Fixed /*not selectable*/ }`.
- `pub struct Row { pub mount: PathBuf, pub safety: Safety, pub entries: usize, pub size: u64, pub selected: u64 }`.
- `pub struct App` with `pub fn new(usage: fn(&Path) -> Usage) -> App`, `on_scan(&mut self, ScanEvent)`,
  `on_key(&mut self, Key) -> Action`, `on_delete(&mut self, DeleteEvent)`, and read accessors used by the view:
  `rows()`, `categories()` (of the open row, size-sorted), `entries()` (of the open category), `cursor() -> usize`,
  `mark_cat(&Category) -> Mark`, `mark_item(&Category, &Item) -> Mark`, `selected_total() -> u64`,
  `selected_check() -> (usize, u64)`, `free_after(&Path) -> u64`, `review() -> Vec<(PathBuf, Vec<(Category, Vec<Item>)>)>`,
  plus `pub screen`, `pub popup`, `pub scanning: Option<String>`, `pub failed: Vec<(String, String)>`,
  `pub scan_done: bool`, `pub progress: Vec<(String /*key*/, String /*title*/, Option<CatResult>)>` (None = pending/running), `pub log: Vec<String>`, `pub report: Option<Report>`.
- Rules: on `ScanEvent::Done` every item of GEFAHRLOS categories gets selected; selection key `(category key, item id)`;
  ␣ on a row/category/entry toggles all selectable items below (all on unless all already on → all off);
  ␣ on Fixed does nothing (sets `Popup::Message("Nicht auswählbar – nur von Hand: …hint")`); in all-or-nothing categories ␣ on an entry toggles the whole category;
  `a`/`n` on Entries; `i` → Details; `?` → Help; Esc closes a popup first, else goes one screen back (Review → the screen it came from);
  `d` → Review only if `scan_done` and something selected (else `Message`); ⏎ in Review → `Popup::Confirm`; `j` in Confirm →
  `Action::Delete(plan)` with `plan` = `Plan` of the ticked items (categories with their key/module/safety, modules = their modules),
  screen → Deleting; `q` → `Action::Quit`, except on Deleting → `Message("Löschen läuft – bitte warten")`;
  `DeleteEvent::Finished` → Report; any key on Report → returns `Action::None` and sets a flag `rescan_requested` the loop reads (then `App::reset_scan()`).

- [ ] Step 1: tests (helper `app_with(cats)` feeding Started/Done/Finished; a fake usage):
  `safe_preselected_check_not`; `overview_rows_per_partition_and_level` (entries, size, selected);
  `space_on_row_toggles_selectable_only`; `partial_mark` (one of two items → Partial); `fixed_cannot_be_toggled` (Data and stopped_images);
  `idless_toggles_whole_category`; `navigation_and_back` (Enter/Right/Left/Esc across Overview→Categories→Entries and back, cursor kept);
  `review_waits_for_scan` (d before Finished → Message, screen stays); `review_groups_by_partition`;
  `confirm_builds_plan_of_ticked` (plan has exactly ticked items, Check category included with safety check);
  `quit_refused_while_deleting`; `report_then_rescan`.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass; clippy (dead-code allowed only via `#[allow]` on the module until Task 7, removed there). Commit "Add the TUI state machine".

### Task 6: Rendering

**Files:** Create `src/tui/view.rs`.

**Interfaces:** `pub fn draw(frame: &mut ratatui::Frame, app: &App, colour: bool)`.
Layout per spec screens 1–7: header line (breadcrumb + `Auswahl {fmt_size}`), body, footer with only the keys of the screen;
popups centred; checkbox glyphs ☑ ☐ ◐ · and none for Fixed; PRÜFEN selected rows/lines with ⚠; long paths shortened in the middle with "…".

- [ ] Step 1: tests with `ratatui::backend::TestBackend` (assert on buffer text, not styles):
  `overview_shows_rows_and_total` (80×24: "Auswahl", "✓ gefahrlos", "⚠ prüfen", footer "⏎ öffnen");
  `scanning_line_names_module` ("Scanne docker …"), `failed_module_note`;
  `entries_show_marks_and_age` ("☑", "☐", "vor 41 Tg.");
  `fixed_rows_have_no_checkbox`; `review_highlights_check` ("⚠", "frei danach");
  `confirm_names_check_part` ("Davon ⚠ prüfen: 2 Einträge, 3.6 GB.");
  `progress_and_report_lines` ("✓", "✗", "nicht mehr gültig");
  `long_list_keeps_cursor_visible` (200 entries, cursor 199 → its label in the buffer);
  `small_terminal_does_not_panic` (60×16 and 20×5 for every screen).
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass. Commit "Draw the TUI screens".

### Task 7: Terminal, workers, entry point, docs

**Files:** Modify `src/tui/mod.rs`, create `src/tui/worker.rs`; modify `src/main.rs`, `README.md`.

**Interfaces:**
- `pub fn run(cfg: &Config) -> anyhow::Result<()>` in `tui/mod.rs`: enter raw mode + alternate screen through a guard whose `Drop`
  and a panic hook both call one `restore()`; loop: draw, poll key 100 ms, drain scan/delete channels; on `Action::Delete(plan)`:
  if any selected category `needs_sudo()` and `sudo -n true` fails → `restore()`, run `sudo -v` with the terminal, re-enter;
  then spawn the delete worker.
- `worker::scan(cfg: Config, tx: Sender<ScanEvent>)` thread: for each of `MODULES`: Started, `collect_module` with `SystemRunner::default()`
  (scanning only calls `output`, which is silent), Done/Failed; Finished.
- `worker::delete(cfg: Config, plan: Plan, tx: Sender<DeleteEvent>)` thread: Rechecking; rescan `plan.modules` via `collect_module`;
  `select(&plan, &scan, &[Safe, Check])`; `execute(sel, false, &mut SystemRunner { log: Some(text_tx) }, &mut ChanWriter, progress)`
  where output and log lines become `DeleteEvent::Output`; Finished(report).
- `main.rs`: `command: Option<Command>`; `fn wants_tui(cli: &Cli, stdin_tty: bool, stdout_tty: bool) -> bool` = no command, both ttys,
  none of `--details/--top/--only/--yes/--plan/--json`. Otherwise `None` behaves as `Preview`. TUI errors → `disk-tidy: …`, exit 2; normal exit 0.

- [ ] Step 1: tests: main `wants_tui_rules` (`[]` + ttys → true; `["preview"]` → false; `["--details"]` → false; no tty → false);
  `check_flags` tests still pass with `command: None`.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: `cargo test`, `cargo clippy --all-targets -- -D warnings` → pass.
- [ ] Step 5: manual check in tmux (`tmux new -d -s dt -x 100 -y 30 'HOME=<tmp home with a trash entry> ./target/debug/disk-tidy --config <missing>'`,
  `send-keys`, `capture-pane -p`): overview appears, scan finishes, Enter/Right/Space/d/Enter/j delete the temp trash entry only,
  report shows ✓, rescan shows it gone, `q` exits and the shell is usable (`tput` sane). Never with the real HOME.
- [ ] Step 6: README: section "TUI" (start rule, screens, keys, PRÜFEN selectable only here, recheck before deleting).
  Commit "Add the interactive TUI as default on a terminal".
