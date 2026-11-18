//! Interactive mode: `disk-tidy` on a terminal.
pub mod app;
pub mod view;
pub mod worker;

use std::io::{self, Stdout};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};

use crate::config::Config;
use crate::partition::{disk_usage, mount_point};
use crate::plan::Plan;
use app::{Action, App, DeleteEvent, Key, ScanEvent};

/// Raw mode off, alternate screen left, cursor shown. Safe to call more than once.
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
}

fn enter() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)
}

/// Owns the terminal; dropping it restores the terminal, also on early return by `?`.
struct Guard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Guard {
    fn new() -> io::Result<Guard> {
        enter().inspect_err(|_| restore())?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Guard { terminal })
    }

    /// Hand the plain terminal to `f` (a sudo password prompt), then take it back.
    fn suspended<T>(&mut self, f: impl FnOnce() -> T) -> io::Result<T> {
        restore();
        let result = f();
        enter()?;
        self.terminal.clear()?;
        Ok(result)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

fn key_of(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    Some(match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => Key::Char('q'),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left | KeyCode::Backspace => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}

fn start_scan(cfg: &Config) -> Receiver<ScanEvent> {
    let (tx, rx) = mpsc::channel();
    worker::scan(cfg.clone(), tx);
    rx
}

/// The selection touches a category whose remover needs sudo.
fn plan_needs_sudo(app: &App, plan: &Plan) -> bool {
    app.all_categories()
        .iter()
        .any(|c| c.needs_sudo() && plan.categories.iter().any(|p| p.key == c.key))
}

fn sudo_cached() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Asks for the sudo password on the plain terminal; a refusal is reported later as skipped.
fn ask_sudo() {
    println!("Für das Löschen wird sudo gebraucht.");
    let _ = Command::new("sudo").arg("-v").status();
}

pub fn run(cfg: &Config, colour: bool) -> anyhow::Result<()> {
    let previous = std::panic::take_hook();
    // Only a panic of the UI thread ends the TUI. A worker panic is caught by `worker::guarded`
    // and reported in the UI; printing it would write into the screen.
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            restore();
            previous(info);
        }
    }));
    let mut guard = Guard::new()?;
    let mut app = App::new(disk_usage, mount_point);
    let mut scan_rx = start_scan(cfg);
    let mut delete_rx: Option<Receiver<DeleteEvent>> = None;
    loop {
        guard
            .terminal
            .draw(|frame| view::draw(frame, &app, colour))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
            && let Some(key) = key_of(k.code, k.modifiers)
        {
            match app.on_key(key) {
                Action::None => {}
                Action::Quit => return Ok(()),
                Action::Delete(plan) => {
                    if plan_needs_sudo(&app, &plan) && !sudo_cached() {
                        guard.suspended(ask_sudo)?;
                    }
                    let (tx, rx) = mpsc::channel();
                    worker::delete(cfg.clone(), plan, tx);
                    delete_rx = Some(rx);
                }
            }
        }
        while let Ok(ev) = scan_rx.try_recv() {
            app.on_scan(ev);
        }
        if let Some(rx) = &delete_rx {
            while let Ok(ev) = rx.try_recv() {
                app.on_delete(ev);
            }
        }
        if app.rescan_requested {
            app.reset_scan();
            delete_rx = None;
            scan_rx = start_scan(cfg);
        }
    }
}
