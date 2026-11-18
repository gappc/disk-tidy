//! External commands, behind a trait so tests can record instead of run.
use std::process::{Command, Stdio};

pub trait Runner {
    /// Stdout if the command ran and exited 0; None otherwise, also when it is not installed.
    fn output(&mut self, argv: &[&str]) -> Option<String>;
    /// Success. On failure prints "  Fehler: <first 4 args>...: <stderr>" to stderr.
    fn run(&mut self, argv: &[String]) -> bool;
    /// `sudo` + argv; a password prompt goes to the terminal, the command's output to stderr.
    fn run_sudo(&mut self, argv: &[String]) -> bool;
    /// sudo works without a password, or there is a terminal to type one.
    fn can_sudo(&mut self, interactive: bool) -> bool;
}

/// Runs commands for real. Without `log`, errors and sudo output go to stderr (text mode);
/// with `log` (the TUI) they are sent there instead and nothing touches the terminal.
#[derive(Default)]
pub struct SystemRunner {
    pub log: Option<std::sync::mpsc::Sender<String>>,
}

impl SystemRunner {
    fn say(&self, line: String) {
        match &self.log {
            Some(tx) => {
                let _ = tx.send(line);
            }
            None => eprintln!("{line}"),
        }
    }
}

impl Runner for SystemRunner {
    fn output(&mut self, argv: &[&str]) -> Option<String> {
        let (cmd, args) = argv.split_first()?;
        let out = Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn run(&mut self, argv: &[String]) -> bool {
        let Some((cmd, args)) = argv.split_first() else {
            return false;
        };
        let stderr = match Command::new(cmd).args(args).stdin(Stdio::null()).output() {
            Ok(out) if out.status.success() => return true,
            Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(e) => e.to_string(),
        };
        let shown: Vec<&str> = argv.iter().take(4).map(String::as_str).collect();
        self.say(format!("  Fehler: {}...: {stderr}", shown.join(" ")));
        false
    }

    fn run_sudo(&mut self, argv: &[String]) -> bool {
        if self.log.is_some() {
            // No terminal to ask on: `-n` fails instead of prompting (credentials were
            // cached before), and the output is captured.
            let out = Command::new("sudo")
                .arg("-n")
                .args(argv)
                .stdin(Stdio::null())
                .output();
            return match out {
                Ok(out) => {
                    for text in [&out.stdout, &out.stderr] {
                        let text = String::from_utf8_lossy(text).trim().to_string();
                        if !text.is_empty() {
                            self.say(text);
                        }
                    }
                    out.status.success()
                }
                Err(e) => {
                    self.say(format!("  Fehler: sudo: {e}"));
                    false
                }
            };
        }
        // Its output goes to stderr, so stdout stays clean for --json.
        Command::new("sudo")
            .args(argv)
            .stdout(std::io::stderr())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn can_sudo(&mut self, interactive: bool) -> bool {
        match Command::new("sudo")
            .args(["-n", "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) => status.success() || interactive,
            Err(_) => false, // sudo not installed
        }
    }
}

/// Records commands instead of running them; `output` answers from `outputs`.
#[cfg(test)]
#[derive(Default)]
pub struct FakeRunner {
    /// Key: argv joined by " ".
    pub outputs: std::collections::HashMap<String, String>,
    /// Every run/run_sudo call; sudo calls start with "sudo".
    pub calls: Vec<Vec<String>>,
    pub sudo_ok: bool,
    /// Commands (argv joined by " ", without "sudo") that fail.
    pub fail: std::collections::HashSet<String>,
}

#[cfg(test)]
impl Runner for FakeRunner {
    fn output(&mut self, argv: &[&str]) -> Option<String> {
        self.outputs.get(&argv.join(" ")).cloned()
    }

    fn run(&mut self, argv: &[String]) -> bool {
        self.calls.push(argv.to_vec());
        !self.fail.contains(&argv.join(" "))
    }

    fn run_sudo(&mut self, argv: &[String]) -> bool {
        let mut call = vec!["sudo".to_string()];
        call.extend_from_slice(argv);
        self.calls.push(call);
        !self.fail.contains(&argv.join(" "))
    }

    fn can_sudo(&mut self, interactive: bool) -> bool {
        self.sudo_ok || interactive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_of_true_command() {
        assert_eq!(
            SystemRunner::default().output(&["echo", "hi"]),
            Some("hi\n".into())
        );
    }

    #[test]
    fn output_of_failing_command_is_none() {
        assert_eq!(SystemRunner::default().output(&["false"]), None);
    }

    #[test]
    fn output_of_missing_binary_is_none() {
        assert_eq!(SystemRunner::default().output(&["/nonexistent/x"]), None);
    }

    #[test]
    fn run_reports_failure() {
        assert!(!SystemRunner::default().run(&["false".into()]));
        assert!(SystemRunner::default().run(&["true".into()]));
    }

    #[test]
    fn log_receives_error_instead_of_stderr() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut r = SystemRunner { log: Some(tx) };
        assert!(!r.run(&["false".into()]));
        drop(r);
        let msgs: Vec<String> = rx.iter().collect();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].starts_with("  Fehler: false"), "{msgs:?}");
    }

    #[test]
    fn run_of_missing_binary_is_failure() {
        assert!(!SystemRunner::default().run(&["/nonexistent/x".into()]));
    }
}
