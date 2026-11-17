//! The modules that find disk usage. Each returns categories; `collect_all` runs the selected ones.
pub mod docker;
pub mod projects;

use crate::config::Config;
use crate::model::Category;
use crate::runner::Runner;

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
            _ => Ok(Vec::new()), // registered by later modules
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
