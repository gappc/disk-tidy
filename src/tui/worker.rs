//! Background threads: scanning and deleting, reporting through channels so the UI stays
//! responsive and nothing writes to the terminal.
use std::io::{self, Write};
use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::clean::{ExecOptions, Progress, Report, execute_with};
use crate::config::Config;
use crate::model::Safety;
use crate::modules::{MODULES, Scan, collect_module};
use crate::plan::{Plan, select};
use crate::runner::SystemRunner;
use crate::tui::app::{DeleteEvent, ScanEvent};

/// Runs `body`; if it panics, runs `fallback`, so the UI still gets a final event.
pub fn guarded(body: impl FnOnce(), fallback: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        fallback();
    }
}

/// Runs every module in order; `Finished` at the end.
pub fn scan(cfg: Config, tx: Sender<ScanEvent>) {
    let fallback_tx = tx.clone();
    thread::spawn(move || {
        guarded(
            move || {
                // Scanning only calls `output`, which never writes to the terminal.
                let mut runner = SystemRunner::default();
                for name in MODULES {
                    let _ = tx.send(ScanEvent::Started(name.into()));
                    let ev = match collect_module(name, &cfg, &mut runner) {
                        Ok(cats) => ScanEvent::Done(cats),
                        Err(note) => ScanEvent::Failed(name.into(), note),
                    };
                    let _ = tx.send(ev);
                }
                let _ = tx.send(ScanEvent::Finished);
            },
            move || {
                let note = "abgebrochen (interner Fehler)".to_string();
                let _ = fallback_tx.send(ScanEvent::Failed("Scan".into(), note));
                let _ = fallback_tx.send(ScanEvent::Finished);
            },
        )
    });
}

/// Rescans the plan's modules, removes what is still there and selectable (GEFAHRLOS, or PRÜFEN
/// as ticked), and reports.
pub fn delete(cfg: Config, plan: Plan, tx: Sender<DeleteEvent>) {
    let fallback_tx = tx.clone();
    thread::spawn(move || {
        guarded(
            move || {
                let _ = tx.send(DeleteEvent::Rechecking);
                let mut quiet = SystemRunner::default();
                let mut scan = Scan {
                    cats: Vec::new(),
                    failed: Vec::new(),
                };
                for name in &plan.modules {
                    match collect_module(name, &cfg, &mut quiet) {
                        Ok(cats) => scan.cats.extend(cats),
                        Err(_) => scan.failed.push(name.clone()),
                    }
                }
                let sel = select(&plan, &scan, &[Safety::Safe, Safety::Check]);

                let (log_tx, log_rx) = mpsc::channel::<String>();
                let forward_tx = tx.clone();
                let forward = thread::spawn(move || {
                    for line in log_rx {
                        let _ = forward_tx.send(DeleteEvent::Output(line));
                    }
                });
                let mut runner = SystemRunner { log: Some(log_tx) };
                let mut out = ChanWriter {
                    tx: tx.clone(),
                    buf: Vec::new(),
                };
                let quiet = ExecOptions {
                    interactive: false,
                    quiet: true,
                };
                let report = execute_with(sel, quiet, &mut runner, &mut out, &mut |p| {
                    let _ = tx.send(match p {
                        Progress::Started(c) => DeleteEvent::Started(c.key.clone()),
                        Progress::Done(r) => DeleteEvent::Done(r.clone()),
                    });
                });
                let _ = out.flush();
                drop(runner); // closes the log channel, so `forward` ends
                let _ = forward.join();
                let _ = tx.send(DeleteEvent::Finished(report));
            },
            move || {
                let msg = "Löschen abgebrochen (interner Fehler)".to_string();
                let _ = fallback_tx.send(DeleteEvent::Output(msg.clone()));
                let _ = fallback_tx.send(DeleteEvent::Finished(Report::error(msg)));
            },
        )
    });
}

/// Remover messages as `DeleteEvent::Output`, one per line; blank lines are left out.
pub struct ChanWriter {
    pub tx: Sender<DeleteEvent>,
    pub buf: Vec<u8>,
}

impl ChanWriter {
    fn send_line(&self, line: &[u8]) {
        let line = String::from_utf8_lossy(line).trim_end().to_string();
        if !line.is_empty() {
            let _ = self.tx.send(DeleteEvent::Output(line));
        }
    }
}

impl Write for ChanWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        while let Some(n) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=n).collect();
            self.send_line(&line);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.send_line(&line);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panicking_body_runs_fallback() {
        let (tx, rx) = mpsc::channel();
        guarded(|| panic!("boom"), move || tx.send("fallback").unwrap());
        assert_eq!(rx.recv().unwrap(), "fallback");
        guarded(|| (), || panic!("no fallback without a panic"));
    }

    #[test]
    fn chan_writer_sends_lines() {
        let (tx, rx) = mpsc::channel();
        let mut w = ChanWriter {
            tx,
            buf: Vec::new(),
        };
        write!(w, "  Fehler: /x: kap").unwrap();
        write!(w, "utt\n\n  sudo: a password is required\nRest").unwrap();
        w.flush().unwrap();
        drop(w);
        let lines: Vec<String> = rx
            .iter()
            .map(|e| match e {
                DeleteEvent::Output(l) => l,
                _ => panic!("only output"),
            })
            .collect();
        assert_eq!(
            lines,
            [
                "  Fehler: /x: kaputt",
                "  sudo: a password is required",
                "Rest"
            ]
        );
    }
}
