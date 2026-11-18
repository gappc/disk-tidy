//! The modules that find disk usage. Each returns categories; `collect_all` runs the selected ones.
pub mod docker;
pub mod projects;
pub mod show;
pub mod system;
pub mod trash;

use crate::config::Config;
use crate::model::Category;
use crate::runner::Runner;

/// Paths matching a glob, in Python's sorted order; `*` does not match a leading dot.
pub fn glob_sorted(pattern: &str) -> Vec<String> {
    let opts = glob::MatchOptions {
        require_literal_leading_dot: true,
        ..Default::default()
    };
    let mut v: Vec<String> = glob::glob_with(pattern, opts)
        .map(|paths| paths.flatten().map(|p| p.display().to_string()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub const MODULES: [&str; 5] = ["docker", "projects", "system", "trash", "show"];

/// What a scan found, and which modules could not run.
pub struct Scan {
    pub cats: Vec<Category>,
    pub failed: Vec<String>,
}

/// One module's categories, empty ones dropped, each tagged with the module; Err(note) when
/// the module cannot run.
pub fn collect_module(
    name: &str,
    cfg: &Config,
    runner: &mut dyn Runner,
) -> Result<Vec<Category>, String> {
    let found = match name {
        "docker" => docker::collect(&cfg.docker, runner, "docker")?,
        "projects" => projects::collect(&cfg.projects, std::time::SystemTime::now()),
        "system" => system::collect(&cfg.system, runner),
        "trash" => trash::collect(&trash::trash_dir()),
        "show" => show::collect(&cfg.show),
        _ => return Err(format!("unbekanntes Modul {name}")),
    };
    Ok(found
        .into_iter()
        .filter(|c| !c.items.is_empty())
        .map(|mut c| {
            c.module = name.into();
            c
        })
        .collect())
}

/// Selected modules (all when `only` is empty) in MODULES order; drops empty categories and tags
/// each with its module. A module that cannot run prints "Hinweis (<mod>): <note>" to stderr and
/// is listed in `failed`.
pub fn collect_all(cfg: &Config, only: &[String], runner: &mut dyn Runner) -> Scan {
    let mut cats = Vec::new();
    let mut failed = Vec::new();
    for name in MODULES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        match collect_module(name, cfg, runner) {
            Ok(found) => cats.extend(found),
            Err(note) => {
                eprintln!("Hinweis ({name}): {note}");
                failed.push(name.to_string());
            }
        }
    }
    Scan { cats, failed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    fn docker_runner(json: &str) -> FakeRunner {
        let mut r = FakeRunner::default();
        r.outputs
            .insert("docker --version".into(), "Docker version 29".into());
        r.outputs.insert(
            "docker info --format {{.DockerRootDir}}".into(),
            "/var/lib/docker".into(),
        );
        r.outputs
            .insert("docker system df -v --format json".into(), json.into());
        r
    }

    #[test]
    fn drops_empty_categories() {
        let mut r = docker_runner(r#"{"Volumes":[{"Name":"x","Links":"0","Size":"1GB"}]}"#);
        let scan = collect_all(&Config::default(), &["docker".into()], &mut r);
        assert_eq!(
            scan.cats.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            ["named_volumes"]
        );
        assert_eq!(scan.cats[0].module, "docker");
        assert!(scan.failed.is_empty());
    }

    #[test]
    fn collect_module_tags_and_drops_empty() {
        let mut r = docker_runner(r#"{"Volumes":[{"Name":"x","Links":"0","Size":"1GB"}]}"#);
        let cats = collect_module("docker", &Config::default(), &mut r).unwrap();
        assert_eq!(
            cats.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            ["named_volumes"]
        );
        assert_eq!(cats[0].module, "docker");
        let err = collect_module("docker", &Config::default(), &mut FakeRunner::default());
        assert_eq!(err.unwrap_err(), "docker nicht installiert");
    }

    #[test]
    fn failing_module_gives_no_categories() {
        let scan = collect_all(
            &Config::default(),
            &["docker".into()],
            &mut FakeRunner::default(),
        );
        assert!(scan.cats.is_empty());
        assert_eq!(scan.failed, ["docker"]);
    }
}
