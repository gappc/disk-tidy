//! External commands, behind a trait so tests can record instead of run.
use std::process::{Command, Stdio};

pub trait Runner {
    /// Stdout if the command ran and exited 0; None otherwise, also when it is not installed.
    fn output(&mut self, argv: &[&str]) -> Option<String>;
    /// Success. On failure prints "  Fehler: <first 4 args>...: <stderr>" to stderr.
    fn run(&mut self, argv: &[String]) -> bool;
    /// `sudo` + argv; a password prompt goes to the terminal.
    fn run_sudo(&mut self, argv: &[String]) -> bool;
    /// sudo works without a password, or there is a terminal to type one.
    fn can_sudo(&mut self, interactive: bool) -> bool;
}

pub struct SystemRunner;

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
        eprintln!("  Fehler: {}...: {stderr}", shown.join(" "));
        false
    }

    fn run_sudo(&mut self, argv: &[String]) -> bool {
        Command::new("sudo")
            .args(argv)
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
}

#[cfg(test)]
impl Runner for FakeRunner {
    fn output(&mut self, argv: &[&str]) -> Option<String> {
        self.outputs.get(&argv.join(" ")).cloned()
    }

    fn run(&mut self, argv: &[String]) -> bool {
        self.calls.push(argv.to_vec());
        true
    }

    fn run_sudo(&mut self, argv: &[String]) -> bool {
        let mut call = vec!["sudo".to_string()];
        call.extend_from_slice(argv);
        self.calls.push(call);
        true
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
        assert_eq!(SystemRunner.output(&["echo", "hi"]), Some("hi\n".into()));
    }

    #[test]
    fn output_of_failing_command_is_none() {
        assert_eq!(SystemRunner.output(&["false"]), None);
    }

    #[test]
    fn output_of_missing_binary_is_none() {
        assert_eq!(SystemRunner.output(&["/nonexistent/x"]), None);
    }

    #[test]
    fn run_reports_failure() {
        assert!(!SystemRunner.run(&["false".into()]));
        assert!(SystemRunner.run(&["true".into()]));
    }

    #[test]
    fn run_of_missing_binary_is_failure() {
        assert!(!SystemRunner.run(&["/nonexistent/x".into()]));
    }
}
