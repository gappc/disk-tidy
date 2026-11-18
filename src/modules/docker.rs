//! Docker: containers, images, build cache and volumes from `docker system df -v`.
use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Deserializer};

use crate::config::DockerConfig;
use crate::model::{Category, Item, Remover, Safety};
use crate::runner::Runner;
use crate::size::parse_size;
use crate::time::parse_timestamp;

/// `docker system df -v --format json`; every value is a string.
#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct DockerDf {
    #[serde(deserialize_with = "null_is_empty")]
    pub containers: Vec<DfContainer>,
    #[serde(deserialize_with = "null_is_empty")]
    pub images: Vec<DfImage>,
    #[serde(deserialize_with = "null_is_empty")]
    pub volumes: Vec<DfVolume>,
    #[serde(deserialize_with = "null_is_empty")]
    pub build_cache: Vec<DfCache>,
}

fn null_is_empty<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct DfContainer {
    #[serde(rename = "ID")]
    pub id: String,
    pub names: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub size: String,
    pub created_at: String,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct DfImage {
    #[serde(rename = "ID")]
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub containers: String,
    pub unique_size: String,
    pub created_since: String,
    pub created_at: String,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct DfVolume {
    pub name: String,
    pub links: String,
    pub size: String,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct DfCache {
    #[serde(rename = "ID")]
    pub id: String,
    pub in_use: String,
    pub size: String,
    pub shared: String,
    pub last_used_since: String,
    pub last_used_at: String,
    pub description: String,
}

fn run(argv: &[&str], with_ids: bool) -> Remover {
    Remover::Run {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        with_ids,
        sudo: false,
    }
}

fn matches(name: &str, patterns: &[String]) -> bool {
    let name: Vec<char> = name.chars().collect();
    patterns
        .iter()
        .any(|p| fnmatch(&name, &p.chars().collect::<Vec<_>>()))
}

/// Python's fnmatch on POSIX: `*`, `?`, `[seq]`, `[!seq]`; every string is a valid pattern
/// (a `[` without its `]` is a literal), so a protect pattern can never silently match nothing.
fn fnmatch(name: &[char], pat: &[char]) -> bool {
    match pat.first() {
        None => name.is_empty(),
        Some('*') => (0..=name.len()).any(|i| fnmatch(&name[i..], &pat[1..])),
        Some('?') => !name.is_empty() && fnmatch(&name[1..], &pat[1..]),
        Some('[') => match bracket(pat) {
            Some((set_matches, rest)) => {
                name.first().is_some_and(|&c| set_matches(c)) && fnmatch(&name[1..], &pat[rest..])
            }
            None => name.first() == Some(&'[') && fnmatch(&name[1..], &pat[1..]),
        },
        Some(&c) => name.first() == Some(&c) && fnmatch(&name[1..], &pat[1..]),
    }
}

/// `pat` starts with `[`: a predicate for the set and the index after its `]`, or None
/// when there is no closing `]`.
fn bracket(pat: &[char]) -> Option<(impl Fn(char) -> bool + '_, usize)> {
    let mut i = 1;
    let negate = pat.get(i) == Some(&'!');
    if negate {
        i += 1;
    }
    let start = i;
    if pat.get(i) == Some(&']') {
        i += 1; // a `]` right after `[` or `[!` is part of the set
    }
    while pat.get(i)? != &']' {
        i += 1;
    }
    let set = &pat[start..i];
    let contains = move |c: char| {
        let mut j = 0;
        while j < set.len() {
            if j + 2 < set.len() && set[j + 1] == '-' {
                if set[j] <= c && c <= set[j + 2] {
                    return true;
                }
                j += 3;
            } else {
                if set[j] == c {
                    return true;
                }
                j += 1;
            }
        }
        false
    };
    Some((move |c| contains(c) != negate, i + 1))
}

fn is_anonymous(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Characters `from..to` of `s` (Python slice semantics for short strings).
fn chars(s: &str, from: usize, to: usize) -> String {
    s.chars().skip(from).take(to.saturating_sub(from)).collect()
}

const DANGLING: usize = 0;
const BUILD_CACHE: usize = 1;
const ANON_VOLUMES: usize = 2;
const CACHE_VOLUMES: usize = 3;
const UNUSED_IMAGES: usize = 4;
const STOPPED_IMAGES: usize = 5;
const CONTAINERS: usize = 6;
const NAMED_VOLUMES: usize = 7;
const PROTECTED_VOLUMES: usize = 8;

/// Sort `docker system df -v` output into categories, in display order.
pub fn classify(df: &DockerDf, cfg: &DockerConfig, root: &Path) -> Vec<Category> {
    use Safety::*;
    let cat = |key, title, safety, why| Category::new(key, title, root, safety, why);
    let mut cats = vec![
        cat(
            "dangling",
            "Docker: namenlose Images",
            Safe,
            "Überbleibsel alter Builds, von nichts benutzt",
        )
        .remover(run(&["docker", "rmi"], true)),
        // Cache entries cannot be removed reliably by ID; this removes exactly the
        // entries that are not in use, which is what the preview lists.
        cat(
            "build_cache",
            "Docker: Build-Cache",
            Safe,
            "nächster Build dauert einmal länger",
        )
        .remover(run(&["docker", "builder", "prune", "-a", "-f"], false)),
        cat(
            "anon_volumes",
            "Docker: anonyme Volumes ohne Container",
            Safe,
            "Reste gelöschter Container, keinem Projekt mehr zugeordnet",
        )
        .remover(run(&["docker", "volume", "rm"], true)),
        cat(
            "cache_volumes",
            "Docker: Cache-Volumes laut Konfiguration",
            Safe,
            "Caches, werden beim nächsten Build neu gefüllt",
        )
        .remover(run(&["docker", "volume", "rm"], true)),
        cat(
            "unused_images",
            "Docker: unbenutzte Images",
            Check,
            "werden bei Bedarf neu heruntergeladen oder gebaut",
        )
        .hint("docker rmi <image>")
        .remover(run(&["docker", "rmi"], true)),
        cat(
            "stopped_images",
            "Docker: Images gestoppter Container",
            Check,
            "erst Container löschen; dann neu laden oder bauen",
        )
        .hint("docker rm <container> && docker rmi <image>"),
        cat(
            "containers",
            "Docker: gestoppte Container",
            Check,
            "Änderungen und Logs im Container gehen verloren",
        )
        .hint("docker rm <container>")
        .remover(run(&["docker", "rm"], true)),
        cat(
            "named_volumes",
            "Docker: benannte Volumes ohne Container",
            Data,
            "können Daten enthalten, z. B. Datenbanken",
        )
        .hint("docker volume rm <name>"),
        cat(
            "protected_volumes",
            "Docker: geschützte Volumes (Schutzliste)",
            Info,
            "passen auf protect_volumes, werden nie gelöscht",
        ),
    ];

    let running: HashSet<&str> = df
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .map(|c| c.image.as_str())
        .collect();
    for c in df.containers.iter().filter(|c| c.state != "running") {
        cats[CONTAINERS].items.push(Item {
            size: parse_size(&c.size),
            id: c.id.clone(),
            label: format!("{:40}  {:35}  {}", c.names, c.image, c.status),
            path: None,
            modified: parse_timestamp(&c.created_at),
            facts: vec![
                ("Image".into(), c.image.clone()),
                ("Status".into(), c.status.clone()),
            ],
        });
    }

    for i in &df.images {
        let named = i.repository != "<none>";
        let id = if named && i.tag != "<none>" {
            format!("{}:{}", i.repository, i.tag)
        } else {
            i.id.clone()
        };
        let short = chars(&i.id, 7, 19);
        let shown = if named { id.clone() } else { short.clone() };
        let label = format!("{shown:60}  {}", i.created_since);
        let in_use = [i.repository.as_str(), &id, &i.id, &short]
            .iter()
            .any(|r| running.contains(r));
        // A count Docker did not give as a number counts as in use: never offered for removal.
        let Ok(containers) = i.containers.trim().parse::<i64>() else {
            continue;
        };
        let key = if containers > 0 {
            if in_use {
                continue;
            }
            STOPPED_IMAGES
        } else if named {
            UNUSED_IMAGES
        } else {
            DANGLING
        };
        let facts = if named {
            let name = if i.tag != "<none>" { &id } else { &i.repository };
            vec![("Image".into(), name.clone())]
        } else {
            Vec::new()
        };
        cats[key].items.push(Item {
            size: parse_size(&i.unique_size),
            id,
            label,
            path: None,
            modified: parse_timestamp(&i.created_at),
            facts,
        });
    }

    for b in df.build_cache.iter().filter(|b| b.in_use != "true") {
        let shared = if b.shared == "true" { "geteilt " } else { "" };
        cats[BUILD_CACHE].items.push(Item {
            size: parse_size(&b.size),
            id: b.id.clone(),
            label: format!(
                "{:15}  {shared}{}",
                b.last_used_since,
                chars(&b.description, 0, 70)
            ),
            path: None,
            modified: parse_timestamp(&b.last_used_at),
            ..Default::default()
        });
    }

    for v in &df.volumes {
        if v.links
            .trim()
            .parse::<i64>()
            .map_or(true, |links| links > 0)
        {
            continue; // in use, or a count that is not a number
        }
        let key = if is_anonymous(&v.name) {
            ANON_VOLUMES
        } else if matches(&v.name, &cfg.protect_volumes) {
            PROTECTED_VOLUMES
        } else if matches(&v.name, &cfg.cache_volumes) {
            CACHE_VOLUMES
        } else {
            NAMED_VOLUMES
        };
        cats[key].items.push(Item {
            size: parse_size(&v.size),
            id: v.name.clone(),
            label: v.name.clone(),
            path: None,
            ..Default::default()
        });
    }

    cats.into_iter().map(Category::sorted).collect()
}

/// Err("docker nicht installiert") or Err("Docker-Daemon nicht erreichbar").
pub fn collect(
    cfg: &DockerConfig,
    runner: &mut dyn Runner,
    docker_bin: &str,
) -> Result<Vec<Category>, String> {
    if runner.output(&[docker_bin, "--version"]).is_none() {
        return Err("docker nicht installiert".into());
    }
    let unreachable = || "Docker-Daemon nicht erreichbar".to_string();
    let root = runner
        .output(&[docker_bin, "info", "--format", "{{.DockerRootDir}}"])
        .ok_or_else(unreachable)?;
    let root = match root.trim() {
        "" => "/var/lib/docker",
        r => r,
    };
    let json = runner
        .output(&[docker_bin, "system", "df", "-v", "--format", "json"])
        .ok_or_else(unreachable)?;
    let df: DockerDf = serde_json::from_str(&json).map_err(|_| unreachable())?;
    Ok(classify(&df, cfg, Path::new(root)))
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    pub const ANON: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn image(id: &str, repo: &str, tag: &str, containers: &str, unique: &str) -> DfImage {
        DfImage {
            id: format!("sha256:{}", id.repeat(64)),
            repository: repo.into(),
            tag: tag.into(),
            containers: containers.into(),
            unique_size: unique.into(),
            created_since: "1 day ago".into(),
            ..Default::default()
        }
    }

    fn container(name: &str, image: &str, state: &str, size: &str) -> DfContainer {
        DfContainer {
            id: name.repeat(8),
            names: name.into(),
            image: image.into(),
            state: state.into(),
            status: "Exited (0)".into(),
            size: size.into(),
            ..Default::default()
        }
    }

    fn volume(name: &str, links: &str, size: &str) -> DfVolume {
        DfVolume {
            name: name.into(),
            links: links.into(),
            size: size.into(),
        }
    }

    fn cache(id: &str, in_use: &str, size: &str) -> DfCache {
        DfCache {
            id: id.into(),
            in_use: in_use.into(),
            size: size.into(),
            shared: "false".into(),
            last_used_since: "1 week ago".into(),
            description: "RUN x".into(),
            ..Default::default()
        }
    }

    /// Same data as `DF` in the Python tests.
    pub fn df() -> DockerDf {
        DockerDf {
            containers: vec![
                container("web", "app:latest", "running", "10MB"),
                container("old", "old:1", "exited", "2GB"),
            ],
            images: vec![
                image("1", "<none>", "<none>", "0", "300MB"), // dangling
                image("2", "node", "22", "0", "1.6GB"),       // unused named
                image("3", "old", "1", "1", "100MB"),         // only stopped
                image("4", "app", "latest", "1", "100MB"),    // running
            ],
            volumes: vec![
                volume(ANON, "0", "500MB"),
                volume(&"b".repeat(64), "1", "1GB"), // in use
                volume("audit-gobuild", "0", "3.4GB"),
                volume("proj_dbdata", "0", "1GB"),
                volume("gobuild_dbdata", "0", "1GB"), // cache AND protected
                volume("random-named", "0", "1GB"),
            ],
            build_cache: vec![
                cache("c1", "false", "200MB"),
                cache("c2", "true", "200MB"),
                cache("c3", "false", "1.5GB"),
            ],
        }
    }

    pub fn cfg() -> DockerConfig {
        DockerConfig {
            cache_volumes: vec!["*gobuild*".into()],
            protect_volumes: vec!["*dbdata*".into()],
        }
    }

    pub fn categories() -> Vec<Category> {
        classify(&df(), &cfg(), Path::new("/var/lib/docker"))
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::model::SAFETY_ORDER;
    use crate::runner::{FakeRunner, SystemRunner};
    use std::collections::HashMap;

    fn by_key(cats: &[Category]) -> HashMap<&str, &Category> {
        cats.iter().map(|c| (c.key.as_str(), c)).collect()
    }

    fn ids(cats: &[Category], key: &str) -> Vec<String> {
        let mut ids: Vec<String> = by_key(cats)[key]
            .items
            .iter()
            .map(|i| i.id.clone())
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn stopped_containers() {
        assert_eq!(ids(&categories(), "containers"), ["old".repeat(8)]);
    }

    #[test]
    fn images() {
        let cats = categories();
        assert_eq!(
            ids(&cats, "dangling"),
            [format!("sha256:{}", "1".repeat(64))]
        );
        assert_eq!(ids(&cats, "unused_images"), ["node:22"]);
        assert_eq!(ids(&cats, "stopped_images"), ["old:1"]);
    }

    #[test]
    fn build_cache_excludes_in_use() {
        assert_eq!(ids(&categories(), "build_cache"), ["c1", "c3"]);
    }

    #[test]
    fn volumes() {
        let cats = categories();
        assert_eq!(ids(&cats, "anon_volumes"), [ANON]);
        assert_eq!(ids(&cats, "cache_volumes"), ["audit-gobuild"]);
        assert_eq!(ids(&cats, "named_volumes"), ["random-named"]);
        assert_eq!(
            ids(&cats, "protected_volumes"),
            ["gobuild_dbdata", "proj_dbdata"]
        );
    }

    #[test]
    fn items_sorted_by_size() {
        let cats = categories();
        let sizes: Vec<u64> = by_key(&cats)["build_cache"]
            .items
            .iter()
            .map(|i| i.size)
            .collect();
        assert_eq!(sizes, [1_500_000_000, 200_000_000]);
    }

    #[test]
    fn only_safe_categories_are_cleaned() {
        let cats = categories();
        let mut cleaned: Vec<&str> = cats
            .iter()
            .filter(|c| c.is_clean())
            .map(|c| c.key.as_str())
            .collect();
        cleaned.sort();
        assert_eq!(
            cleaned,
            ["anon_volumes", "build_cache", "cache_volumes", "dangling"]
        );
    }

    #[test]
    fn default_config_cleans_no_named_volumes() {
        let cats = classify(&df(), &DockerConfig::default(), Path::new("/"));
        assert!(by_key(&cats)["cache_volumes"].items.is_empty());
        assert!(ids(&cats, "named_volumes").contains(&"audit-gobuild".to_string()));
    }

    #[test]
    fn every_category_has_level_and_reason() {
        let cats = categories();
        let keys: Vec<&str> = cats.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "dangling",
                "build_cache",
                "anon_volumes",
                "cache_volumes",
                "unused_images",
                "stopped_images",
                "containers",
                "named_volumes",
                "protected_volumes"
            ]
        );
        for c in &cats {
            assert!(SAFETY_ORDER.contains(&c.safety), "{}", c.key);
            assert!(!c.why.is_empty(), "{}", c.key);
            assert_eq!(c.is_clean(), c.safety == Safety::Safe, "{}", c.key);
        }
    }

    #[test]
    fn removers_use_ids() {
        let cats = categories();
        let k = by_key(&cats);
        let run = |argv: &[&str], with_ids| Remover::Run {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            with_ids,
            sudo: false,
        };
        assert_eq!(k["dangling"].remover, run(&["docker", "rmi"], true));
        assert_eq!(
            k["build_cache"].remover,
            run(&["docker", "builder", "prune", "-a", "-f"], false)
        );
        assert_eq!(
            k["anon_volumes"].remover,
            run(&["docker", "volume", "rm"], true)
        );
        assert_eq!(
            k["cache_volumes"].remover,
            run(&["docker", "volume", "rm"], true)
        );
        assert_eq!(k["containers"].hint, "docker rm <container>");
        assert_eq!(k["unused_images"].remover, run(&["docker", "rmi"], true));
        assert_eq!(k["containers"].remover, run(&["docker", "rm"], true));
        assert_eq!(k["stopped_images"].remover, Remover::None);
        assert!(!k["stopped_images"].selectable());
    }

    #[test]
    fn labels_as_python() {
        let cats = categories();
        let k = by_key(&cats);
        assert_eq!(
            k["containers"].items[0].label,
            format!("{:40}  {:35}  Exited (0)", "old", "old:1")
        );
        assert_eq!(
            k["dangling"].items[0].label,
            format!("{:60}  1 day ago", "1".repeat(12))
        );
        assert_eq!(
            k["build_cache"].items[0].label,
            format!("{:15}  RUN x", "1 week ago")
        );
    }

    #[test]
    fn parses_real_df_json() {
        let json = r#"{"Images":[{"Containers":"0","CreatedSince":"2 days ago","ID":"sha256:e4","Repository":"<none>","Tag":"<none>","UniqueSize":"648.6MB","SharedSize":"1MB"}],
          "Containers":[{"ID":"f8","Names":"x","Image":"img","State":"exited","Status":"Exited (1)","Size":"1kB","Labels":""}],
          "Volumes":[{"Links":"0","Name":"gobuildcache-audit","Size":"1.058GB","Driver":"local"}],
          "BuildCache":[{"CacheType":"regular","Description":"pulled","ID":"llh","InUse":"false","LastUsedSince":"5 months ago","Shared":"true","Size":"117MB"}]}"#;
        let df: DockerDf = serde_json::from_str(json).unwrap();
        let cats = classify(&df, &DockerConfig::default(), Path::new("/"));
        let k = by_key(&cats);
        assert_eq!(k["dangling"].total(), 648_600_000);
        assert_eq!(k["containers"].items[0].id, "f8");
        assert_eq!(ids(&cats, "named_volumes"), ["gobuildcache-audit"]);
        assert_eq!(
            k["build_cache"].items[0].label,
            format!("{:15}  geteilt pulled", "5 months ago")
        );
    }

    #[test]
    fn image_and_container_facts() {
        let json = r#"{"Images":[{"Containers":"0","CreatedAt":"2026-10-08 12:00:00 +0200 CEST","ID":"sha256:e4","Repository":"node","Tag":"22","UniqueSize":"1GB"}],
          "Containers":[{"ID":"f8","Names":"x","Image":"img:1","State":"exited","Status":"Exited (1)","Size":"1kB","CreatedAt":"2026-10-08 12:00:00 +0200 CEST"}],
          "BuildCache":[{"ID":"llh","InUse":"false","LastUsedAt":"2026-10-08 10:00:00.5 +0000 UTC","Size":"117MB"}]}"#;
        let df: DockerDf = serde_json::from_str(json).unwrap();
        let cats = classify(&df, &DockerConfig::default(), Path::new("/"));
        let k = by_key(&cats);
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        let img = &k["unused_images"].items[0];
        assert_eq!(img.modified, Some(1791453600));
        assert_eq!(img.facts, [s("Image", "node:22")]);
        let c = &k["containers"].items[0];
        assert_eq!(c.modified, Some(1791453600));
        assert_eq!(c.facts, [s("Image", "img:1"), s("Status", "Exited (1)")]);
        assert_eq!(k["build_cache"].items[0].modified, Some(1791453600));
    }

    #[test]
    fn patterns_follow_fnmatch() {
        let m = |name: &str, p: &str| matches(name, &[p.to_string()]);
        assert!(m("my_db", "**db**"));
        assert!(m("[db", "[db"));
        assert!(m("db[", "db["));
        assert!(m("v1", "v[0-9]"));
        assert!(!m("va", "v[0-9]"));
        assert!(m("bx", "[!a]x") && !m("ax", "[!a]x"));
        assert!(m("ab", "a?") && !m("abc", "a?"));
        assert!(m("a/b", "a*"));
        assert!(m("x.y", "x.y") && !m("xzy", "x.y"));
        assert!(!m("Proj_DB", "*db*"));
    }

    #[test]
    fn protect_pattern_glob_crate_rejects_still_protects() {
        let cfg = DockerConfig {
            cache_volumes: vec!["*cache*".into()],
            protect_volumes: vec!["**db**".into()],
        };
        let df = DockerDf {
            volumes: vec![DfVolume {
                name: "pg_db_cache".into(),
                links: "0".into(),
                size: "1GB".into(),
            }],
            ..Default::default()
        };
        let cats = classify(&df, &cfg, Path::new("/"));
        assert_eq!(ids(&cats, "protected_volumes"), ["pg_db_cache"]);
    }

    #[test]
    fn unparseable_counts_are_in_use() {
        let df = DockerDf {
            volumes: vec![DfVolume {
                name: ANON.into(),
                links: "N/A".into(),
                size: "1GB".into(),
            }],
            images: vec![DfImage {
                id: "sha256:1".into(),
                repository: "<none>".into(),
                tag: "<none>".into(),
                containers: "N/A".into(),
                unique_size: "1GB".into(),
                created_since: String::new(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let cats = classify(&df, &DockerConfig::default(), Path::new("/"));
        assert!(
            cats.iter()
                .filter(|c| c.is_clean())
                .all(|c| c.items.is_empty())
        );
    }

    #[test]
    fn missing_docker_is_note() {
        let r = collect(
            &DockerConfig::default(),
            &mut SystemRunner,
            "/nonexistent/docker",
        );
        assert_eq!(r.unwrap_err(), "docker nicht installiert");
    }

    #[test]
    fn daemon_down_is_note() {
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .insert("docker --version".into(), "Docker version 29".into());
        let r = collect(&DockerConfig::default(), &mut runner, "docker");
        assert_eq!(r.unwrap_err(), "Docker-Daemon nicht erreichbar");
    }

    #[test]
    fn collect_uses_docker_root() {
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .insert("docker --version".into(), "Docker version 29".into());
        runner.outputs.insert(
            "docker info --format {{.DockerRootDir}}".into(),
            "/srv/docker\n".into(),
        );
        runner.outputs.insert(
            "docker system df -v --format json".into(),
            r#"{"Images":[],"Containers":[],"Volumes":[],"BuildCache":null}"#.into(),
        );
        let cats = collect(&DockerConfig::default(), &mut runner, "docker").unwrap();
        assert!(cats.iter().all(|c| c.path == Path::new("/srv/docker")));
    }
}
