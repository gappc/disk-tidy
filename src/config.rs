//! `~/.config/disk-tidy/config.toml`: same keys and defaults as the Python version;
//! missing keys fall back per key.
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    pub docker: DockerConfig,
    pub projects: ProjectsConfig,
    pub system: SystemConfig,
    pub show: ShowConfig,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DockerConfig {
    /// Named volumes that `clean` may remove when unused (globs).
    pub cache_volumes: Vec<String>,
    /// Never removed, even if matched by cache_volumes.
    pub protect_volumes: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProjectsConfig {
    pub roots: Vec<String>,
    /// A project counts as untouched when no file outside its build dirs changed for this many days.
    pub stale_days: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SystemConfig {
    /// journalctl --vacuum-size target.
    pub journal_max: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ShowConfig {
    /// Large paths that are only displayed, never deleted (globs allowed).
    pub paths: Vec<String>,
}

impl Default for DockerConfig {
    fn default() -> Self {
        DockerConfig {
            cache_volumes: Vec::new(),
            protect_volumes: vec!["*db*".into(), "*data*".into()],
        }
    }
}

impl Default for ProjectsConfig {
    fn default() -> Self {
        ProjectsConfig {
            roots: vec!["~/projects".into()],
            stale_days: 28,
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        SystemConfig {
            journal_max: "500M".into(),
        }
    }
}

impl Default for ShowConfig {
    fn default() -> Self {
        ShowConfig {
            paths: vec!["~/snap/*/common".into()],
        }
    }
}

pub fn default_path() -> PathBuf {
    home().join(".config/disk-tidy/config.toml")
}

/// A missing file gives the defaults.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Konfiguration {} nicht lesbar", path.display()))?;
    toml::from_str(&text).with_context(|| format!("Konfiguration {} ungültig", path.display()))
}

/// A leading "~" becomes $HOME.
pub fn expand_home(s: &str) -> PathBuf {
    match s.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(s),
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let c = load(Path::new("/nonexistent/x.toml")).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.docker.cache_volumes, Vec::<String>::new());
        assert_eq!(c.docker.protect_volumes, ["*db*", "*data*"]);
        assert_eq!(c.projects.roots, ["~/projects"]);
        assert_eq!(c.projects.stale_days, 28);
        assert_eq!(c.system.journal_max, "500M");
        assert_eq!(c.show.paths, ["~/snap/*/common"]);
    }

    #[test]
    fn file_overrides_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(
            &p,
            "[docker]\ncache_volumes=[\"x*\"]\n[projects]\nstale_days=7\n",
        )
        .unwrap();
        let c = load(&p).unwrap();
        assert_eq!(c.docker.cache_volumes, ["x*"]);
        assert_eq!(c.docker.protect_volumes, ["*db*", "*data*"]);
        assert_eq!(c.projects.stale_days, 7);
        assert_eq!(c.projects.roots, ["~/projects"]);
    }

    #[test]
    fn invalid_toml_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "[docker").unwrap();
        let err = format!("{:#}", load(&p).unwrap_err());
        assert!(err.contains(&p.display().to_string()), "{err}");
    }

    #[test]
    fn user_config_parses() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("python/config.example.toml");
        assert_eq!(load(&p).unwrap().docker.cache_volumes.len(), 3);
    }

    #[test]
    fn expands_home() {
        assert_eq!(expand_home("~/x"), home().join("x"));
        assert_eq!(expand_home("~"), home());
        assert_eq!(expand_home("/a"), PathBuf::from("/a"));
    }

    #[test]
    fn default_path_is_in_home() {
        assert_eq!(default_path(), home().join(".config/disk-tidy/config.toml"));
    }
}
