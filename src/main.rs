//! disk-tidy: see what fills the disk, sorted by size, and remove only the safe parts.
//!
//!   disk-tidy [preview] [--details [--top N]] [--only MOD,...]   Overview per partition (read-only).
//!   disk-tidy clean [--yes] [--details] [--only MOD,...]         Remove the GEFAHRLOS part, after asking.
mod clean;
mod config;
mod model;
mod modules;
mod partition;
mod render;
mod runner;
mod size;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};

use clean::{CleanOptions, Outcome};
use modules::MODULES;
use render::Style;
use runner::SystemRunner;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Command {
    Preview,
    Clean,
}

/// disk-tidy: see what fills the disk, sorted by size, and remove only the safe parts.
#[derive(Parser)]
#[command(name = "disk-tidy", version)]
struct Cli {
    #[arg(value_enum, default_value = "preview")]
    command: Command,
    /// einzelne Einträge und Befehle zum Löschen von Hand zeigen
    #[arg(short, long)]
    details: bool,
    /// mit --details: Einträge pro Kategorie, 0 = alle
    #[arg(long, default_value_t = 15)]
    top: usize,
    /// nur diese Module, kommagetrennt: docker,projects,system,trash,show
    #[arg(long, default_value = "")]
    only: String,
    /// clean ohne Rückfrage
    #[arg(long)]
    yes: bool,
    #[arg(long, default_value_os_t = config::default_path())]
    config: PathBuf,
}

/// "docker, trash" -> ["docker", "trash"]; unknown names -> "unbekannte Module: a, b" (sorted).
fn parse_only(only: &str) -> Result<Vec<String>, String> {
    let names: Vec<String> = only
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(String::from)
        .collect();
    let mut unknown: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|m| !MODULES.contains(m))
        .collect();
    if unknown.is_empty() {
        return Ok(names);
    }
    unknown.sort();
    unknown.dedup();
    Err(format!("unbekannte Module: {}", unknown.join(", ")))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let only = parse_only(&cli.only)
        .unwrap_or_else(|e| Cli::command().error(ErrorKind::InvalidValue, e).exit());
    let cfg = match config::load(&cli.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("disk-tidy: {e:#}");
            return ExitCode::from(2);
        }
    };
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let style = Style {
        colour: io::stdout().is_terminal() && !no_color,
    };
    let mut runner = SystemRunner;
    let cats = modules::collect_all(&cfg, &only, &mut runner);
    let usage = partition::disk_usage;

    if cli.command == Command::Preview {
        print!(
            "{}",
            render::render_preview(&cats, cli.details, cli.top, &style, &usage)
        );
        return ExitCode::SUCCESS;
    }
    let opts = CleanOptions {
        assume_yes: cli.yes,
        interactive: io::stdin().is_terminal(),
        details: cli.details,
        top: cli.top,
        style,
        usage: &usage,
    };
    let outcome: Outcome = clean::clean(
        cats,
        &opts,
        &mut io::stdin().lock(),
        &mut runner,
        &mut io::stdout(),
    );
    ExitCode::from(outcome.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_is_comma_list() {
        assert_eq!(parse_only("docker, trash,").unwrap(), ["docker", "trash"]);
        assert!(parse_only("").unwrap().is_empty());
    }

    #[test]
    fn unknown_modules_are_named() {
        assert_eq!(
            parse_only("zz,docker,aa").unwrap_err(),
            "unbekannte Module: aa, zz"
        );
    }
}
