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

/// Selected modules (all when `only` is empty) in MODULES order; drops empty categories.
/// A module that cannot run prints "Hinweis (<mod>): <note>" to stderr.
pub fn collect_all(cfg: &Config, only: &[String], runner: &mut dyn Runner) -> Vec<Category> {
    let mut cats = Vec::new();
    for name in MODULES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        let found = match name {
            "docker" => docker::collect(&cfg.docker, runner, "docker"),
            "projects" => Ok(projects::collect(
                &cfg.projects,
                std::time::SystemTime::now(),
            )),
            "system" => Ok(system::collect(&cfg.system, runner)),
            "trash" => Ok(trash::collect(&trash::trash_dir())),
            "show" => Ok(show::collect(&cfg.show)),
            _ => unreachable!("MODULES lists every module"),
        };
        match found {
            Ok(found) => cats.extend(found.into_iter().filter(|c| !c.items.is_empty())),
            Err(note) => eprintln!("Hinweis ({name}): {note}"),
        }
    }
    cats
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
        let cats = collect_all(&Config::default(), &["docker".into()], &mut r);
        assert_eq!(
            cats.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            ["named_volumes"]
        );
    }

    #[test]
    fn failing_module_gives_no_categories() {
        let cats = collect_all(
            &Config::default(),
            &["docker".into()],
            &mut FakeRunner::default(),
        );
        assert!(cats.is_empty());
    }
}
