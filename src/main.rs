//! disk-tidy: see what fills the disk, sorted by size, and remove only the safe parts.
//!
//!   disk-tidy [preview] [--details [--top N]] [--only MOD,...]   Overview per partition (read-only).
//!   disk-tidy clean [--yes] [--details] [--only MOD,...]         Remove the GEFAHRLOS part, after asking.
//!   disk-tidy plan [--only MOD,...]                              Everything found, as JSON.
//!   disk-tidy clean --plan FILE|- [--json]                       Remove what the plan lists and is still safe.
mod clean;
mod config;
mod model;
mod modules;
mod partition;
mod plan;
mod render;
mod runner;
mod size;

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};

use clean::{CleanOptions, Report, Selection};
use modules::MODULES;
use render::Style;
use runner::SystemRunner;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Command {
    Preview,
    Clean,
    Plan,
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
    /// mit --details: Einträge pro Kategorie, 0 = alle [Standard: 15]
    #[arg(long)]
    top: Option<usize>,
    /// nur diese Module, kommagetrennt: docker,projects,system,trash,show
    #[arg(long, default_value = "")]
    only: String,
    /// clean ohne Rückfrage
    #[arg(long)]
    yes: bool,
    /// clean: nur löschen, was dieser Plan (von disk-tidy plan) nennt und noch gefahrlos ist; - = stdin
    #[arg(long, value_name = "FILE")]
    plan: Option<PathBuf>,
    /// clean: Ergebnis als JSON auf stdout, Text auf stderr
    #[arg(long)]
    json: bool,
    #[arg(long, default_value_os_t = config::default_path())]
    config: PathBuf,
}

const DEFAULT_TOP: usize = 15;

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

/// Flags that do not fit the command: "--top passt nicht zu plan".
fn check_flags(cli: &Cli) -> Result<(), String> {
    let given = [
        ("--details", cli.details),
        ("--top", cli.top.is_some()),
        ("--yes", cli.yes),
        ("--json", cli.json),
        ("--plan", cli.plan.is_some()),
        ("--only", !cli.only.is_empty()),
    ];
    let (context, refused): (&str, &[&str]) = match cli.command {
        Command::Plan => ("plan", &["--details", "--top", "--yes", "--json", "--plan"]),
        Command::Preview => ("preview", &["--json", "--plan", "--yes"]),
        Command::Clean if cli.plan.is_some() => {
            ("clean --plan", &["--only", "--yes", "--details", "--top"])
        }
        Command::Clean => return Ok(()),
    };
    match given
        .iter()
        .find(|(flag, on)| *on && refused.contains(flag))
    {
        Some((flag, _)) => Err(format!("{flag} passt nicht zu {context}")),
        None => Ok(()),
    }
}

/// Text goes to `out`; with --json that is stderr and the report is the only thing on stdout.
fn fail(json: bool, msg: String) -> ExitCode {
    if json {
        print_json(&Report::error(msg));
    } else {
        eprintln!("disk-tidy: {msg}");
    }
    ExitCode::from(2)
}

/// Pretty JSON and a newline on stdout; a closed pipe is fine.
fn print_json<T: serde::Serialize>(value: &T) {
    let text = serde_json::to_string_pretty(value).expect("plain data serialises");
    let _ = writeln!(io::stdout(), "{text}");
}

fn read_plan(path: &Path) -> anyhow::Result<plan::Plan> {
    let mut text = String::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_string(&mut text)
            .context("Plan von stdin nicht lesbar")?;
    } else {
        text = std::fs::read_to_string(path)
            .with_context(|| format!("Plan {} nicht lesbar", path.display()))?;
    }
    let plan = plan::parse_plan(&text)?;
    parse_only(&plan.modules.join(",")).map_err(|e| anyhow::anyhow!("Plan ungültig: {e}"))?;
    Ok(plan)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let usage_error = |kind, msg: String| Cli::command().error(kind, msg).exit();
    if let Err(e) = check_flags(&cli) {
        usage_error(ErrorKind::ArgumentConflict, e);
    }
    let only = parse_only(&cli.only).unwrap_or_else(|e| usage_error(ErrorKind::InvalidValue, e));
    // Read a plan before anything else: stdin may be a pipe that must not be left half read.
    let plan = match &cli.plan {
        Some(path) => match read_plan(path) {
            Ok(plan) => Some(plan),
            Err(e) => return fail(cli.json, format!("{e:#}")),
        },
        None => None,
    };
    let cfg = match config::load(&cli.config) {
        Ok(cfg) => cfg,
        Err(e) => return fail(cli.json, format!("{e:#}")),
    };
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let text_to_stdout = !cli.json;
    let style = Style {
        colour: text_to_stdout && io::stdout().is_terminal() && !no_color,
    };
    let top = cli.top.unwrap_or(DEFAULT_TOP);
    let mut runner = SystemRunner;
    let usage = partition::disk_usage;
    let modules: Vec<String> = match &plan {
        Some(p) => p.modules.clone(),
        None => only,
    };
    let scan = modules::collect_all(&cfg, &modules, &mut runner);

    match cli.command {
        Command::Preview => {
            let text = render::render_preview(&scan.cats, cli.details, top, &style, &usage);
            let _ = io::stdout().write_all(text.as_bytes()); // a closed pipe (| head) is fine
            return ExitCode::SUCCESS;
        }
        Command::Plan => {
            let scanned: Vec<String> = if modules.is_empty() {
                MODULES.iter().map(|m| m.to_string()).collect()
            } else {
                modules
            };
            let created = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let (plan, left_out) = plan::build_plan(&scan, &scanned, created, &usage);
            for id in left_out {
                eprintln!("Hinweis: nicht im Plan (Name kein UTF-8): {id}");
            }
            print_json(&plan);
            return ExitCode::SUCCESS;
        }
        Command::Clean => {}
    }
    let selection = match &plan {
        Some(p) => plan::select(p, &scan),
        None => Selection::all(scan.cats),
    };
    let opts = CleanOptions {
        assume_yes: cli.yes || plan.is_some(),
        interactive: io::stdin().is_terminal(),
        can_prompt: io::stdin().is_terminal() && !cli.json,
        details: cli.details,
        top,
        style,
        usage: &usage,
    };
    let mut stdin = io::stdin().lock();
    let report = if text_to_stdout {
        clean::clean(
            selection,
            &opts,
            &mut stdin,
            &mut runner,
            &mut io::stdout(),
            &mut io::stderr(),
        )
    } else {
        clean::clean(
            selection,
            &opts,
            &mut stdin,
            &mut runner,
            &mut io::stderr(),
            &mut io::stderr(),
        )
    };
    if cli.json {
        print_json(&report);
    }
    ExitCode::from(report.exit_code)
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

    fn check(args: &[&str]) -> Result<(), String> {
        let mut argv = vec!["disk-tidy"];
        argv.extend_from_slice(args);
        check_flags(&Cli::try_parse_from(argv).unwrap())
    }

    #[test]
    fn check_flags_rejects_mismatches() {
        for bad in [
            &["plan", "--details"][..],
            &["plan", "--top", "3"],
            &["plan", "--yes"],
            &["plan", "--json"],
            &["plan", "--plan", "p"],
            &["preview", "--json"],
            &["preview", "--plan", "p"],
            &["preview", "--yes"],
            &["clean", "--plan", "p", "--only", "trash"],
            &["clean", "--plan", "p", "--yes"],
            &["clean", "--plan", "p", "-d"],
            &["clean", "--plan", "p", "--top", "3"],
        ] {
            assert!(check(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            check(&["plan", "--top", "3"]).unwrap_err(),
            "--top passt nicht zu plan"
        );
    }

    #[test]
    fn check_flags_accepts_valid() {
        for good in [
            &["clean", "--plan", "p", "--json"][..],
            &["plan", "--only", "trash"],
            &["clean", "--yes", "-d", "--top", "3"],
            &["clean", "--json", "--yes"],
            &["-d", "--top", "0"],
        ] {
            assert!(check(good).is_ok(), "{good:?}");
        }
    }
}
