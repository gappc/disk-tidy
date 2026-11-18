//! `disk-tidy plan`: everything a scan found, as JSON, and what `clean --plan` may remove of it.
use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::clean::{Dropped, Reason, Selection};
use crate::model::{Remover, SAFETY_ORDER, Safety};
use crate::modules::Scan;
use crate::partition::Usage;
use crate::render::by_partition;

pub const PLAN_VERSION: u32 = 1;

/// Unknown fields are ignored, so a later version may add some.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    /// Unix seconds.
    pub created: u64,
    /// The modules that were scanned; `clean --plan` scans these again.
    pub modules: Vec<String>,
    pub partitions: Vec<PlanPartition>,
    pub categories: Vec<PlanCategory>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanPartition {
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanCategory {
    pub key: String,
    pub module: String,
    pub title: String,
    pub safety: Safety,
    pub why: String,
    pub hint: String,
    pub mount: String,
    pub total: u64,
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub label: String,
    pub size: u64,
}

/// The plan of a scan, in preview order (partition, level, size). Items whose path is not valid
/// UTF-8 cannot be matched exactly through JSON: they are left out and returned by display form.
pub fn build_plan(
    scan: &Scan,
    modules: &[String],
    created: u64,
    usage: &dyn Fn(&Path) -> Usage,
) -> (Plan, Vec<String>) {
    let mut partitions = Vec::new();
    let mut categories = Vec::new();
    let mut left_out = Vec::new();
    for (mount, group) in by_partition(&scan.cats) {
        let mount_str = mount.display().to_string();
        let u = usage(&mount);
        partitions.push(PlanPartition {
            mount: mount_str.clone(),
            total: u.total,
            used: u.used,
            free: u.free,
        });
        for safety in SAFETY_ORDER {
            let mut cats: Vec<_> = group.iter().filter(|c| c.safety == safety).collect();
            cats.sort_by_key(|c| std::cmp::Reverse(c.total()));
            for c in cats {
                let mut items = Vec::new();
                for i in &c.items {
                    if i.path.as_ref().is_some_and(|p| p.to_str().is_none()) {
                        left_out.push(i.id.clone());
                    } else {
                        items.push(PlanItem {
                            id: i.id.clone(),
                            label: i.label.clone(),
                            size: i.size,
                        });
                    }
                }
                categories.push(PlanCategory {
                    key: c.key.clone(),
                    module: c.module.clone(),
                    title: c.title.clone(),
                    safety: c.safety,
                    why: c.why.clone(),
                    hint: c.hint.clone(),
                    mount: mount_str.clone(),
                    total: items.iter().map(|i| i.size).sum(),
                    items,
                });
            }
        }
    }
    let plan = Plan {
        version: PLAN_VERSION,
        created,
        modules: modules.to_vec(),
        partitions,
        categories,
    };
    (plan, left_out)
}

/// What `clean --plan` may remove: the items of GEFAHRLOS categories of the fresh scan that the
/// plan lists. The plan's own `safety` decides nothing; it only selects which unmatched plan items
/// are reported as dropped (those it marked `safe`).
pub fn select(plan: &Plan, scan: &Scan) -> Selection {
    let listed: HashSet<(&str, &str)> = plan
        .categories
        .iter()
        .flat_map(|c| c.items.iter().map(|i| (c.key.as_str(), i.id.as_str())))
        .collect();
    let mut sel = Selection::default();
    for c in scan.cats.iter().filter(|c| c.is_clean()) {
        let mut kept = c.clone();
        // A non-UTF-8 path is never in a plan; its lossy id could match a real name.
        kept.items.retain(|i| {
            i.path.as_ref().is_none_or(|p| p.to_str().is_some())
                && listed.contains(&(c.key.as_str(), i.id.as_str()))
        });
        if kept.items.is_empty() {
            continue;
        }
        let idless = matches!(
            c.remover,
            Remover::Run {
                with_ids: false,
                ..
            }
        );
        if idless && kept.items.len() != c.items.len() {
            sel.partial.push(kept);
        } else {
            sel.todo.push(kept);
        }
    }

    let taken: HashSet<(&str, &str)> = sel
        .todo
        .iter()
        .chain(&sel.partial)
        .flat_map(|c| c.items.iter().map(|i| (c.key.as_str(), i.id.as_str())))
        .collect();
    let unsafe_ids: HashSet<&str> = scan
        .cats
        .iter()
        .filter(|c| !c.is_clean())
        .flat_map(|c| c.items.iter().map(|i| i.id.as_str()))
        .collect();
    let mut dropped = Vec::new();
    for c in plan.categories.iter().filter(|c| c.safety == Safety::Safe) {
        for i in &c.items {
            if taken.contains(&(c.key.as_str(), i.id.as_str())) {
                continue;
            }
            let reason = if scan.failed.contains(&c.module) {
                Reason::ModuleFailed
            } else if unsafe_ids.contains(i.id.as_str()) {
                Reason::NotSafe
            } else {
                Reason::Gone
            };
            dropped.push(Dropped {
                key: c.key.clone(),
                id: i.id.clone(),
                reason,
            });
        }
    }
    sel.dropped = dropped;
    sel
}

pub fn parse_plan(text: &str) -> anyhow::Result<Plan> {
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let v: Version = serde_json::from_str(text).context("Plan ungültig")?;
    if v.version != PLAN_VERSION {
        bail!("Plan-Version {} wird nicht unterstützt", v.version);
    }
    serde_json::from_str(text).context("Plan ungültig")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::{Dropped, Reason};
    use crate::model::{Category, Item, Remover, Safety};
    use crate::modules::Scan;
    use crate::partition::Usage;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    fn usage(_: &Path) -> Usage {
        Usage {
            total: 100,
            free: 40,
            used: 60,
        }
    }

    fn item(id: &str, size: u64) -> Item {
        Item {
            size,
            id: id.into(),
            label: format!("label {id}"),
            path: None,
        }
    }

    fn cat(key: &str, safety: Safety, items: Vec<Item>) -> Category {
        let mut c = Category::new(key, &format!("Titel {key}"), "/", safety, "warum").hint("h");
        c.module = "projects".into();
        c.items = items;
        c
    }

    fn scan(cats: Vec<Category>) -> Scan {
        Scan {
            cats,
            failed: Vec::new(),
        }
    }

    fn modules() -> Vec<String> {
        vec!["projects".into()]
    }

    #[test]
    fn plan_keeps_all_levels_in_preview_order() {
        let s = scan(vec![
            cat("safe1", Safety::Safe, vec![item("a", 1)]),
            cat("check5", Safety::Check, vec![item("b", 5)]),
            cat("safe10", Safety::Safe, vec![item("c", 10)]),
        ]);
        let (plan, left_out) = build_plan(&s, &modules(), 7, &usage);
        assert!(left_out.is_empty());
        assert_eq!(plan.version, 1);
        assert_eq!(plan.created, 7);
        assert_eq!(plan.modules, ["projects"]);
        assert_eq!(
            plan.partitions,
            [PlanPartition {
                mount: "/".into(),
                total: 100,
                used: 60,
                free: 40
            }]
        );
        let keys: Vec<&str> = plan.categories.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["safe10", "safe1", "check5"]);
        let c = &plan.categories[0];
        assert_eq!(
            (
                c.module.as_str(),
                c.mount.as_str(),
                c.total,
                c.hint.as_str()
            ),
            ("projects", "/", 10, "h")
        );
        assert_eq!(
            c.items,
            [PlanItem {
                id: "c".into(),
                label: "label c".into(),
                size: 10
            }]
        );
    }

    #[test]
    fn plan_leaves_out_non_utf8_paths() {
        let mut bad = item("/t/a\u{fffd}", 5);
        bad.path = Some(PathBuf::from(OsString::from_vec(b"/t/a\xff".to_vec())));
        let s = scan(vec![cat("trash", Safety::Safe, vec![bad, item("/t/b", 3)])]);
        let (plan, left_out) = build_plan(&s, &modules(), 0, &usage);
        assert_eq!(left_out, ["/t/a\u{fffd}"]);
        let c = &plan.categories[0];
        assert_eq!(c.items.len(), 1);
        assert_eq!(c.total, 3);
    }

    #[test]
    fn plan_json_roundtrip() {
        let s = scan(vec![cat("safe1", Safety::Safe, vec![item("a", 1)])]);
        let (plan, _) = build_plan(&s, &modules(), 7, &usage);
        let text = serde_json::to_string(&plan).unwrap();
        assert!(text.contains(r#""safety":"safe""#), "{text}");
        assert_eq!(parse_plan(&text).unwrap(), plan);
    }

    #[test]
    fn parse_rejects_version_and_garbage() {
        let v2 = r#"{"version":2,"created":0,"modules":[],"partitions":[],"categories":[]}"#;
        let e = format!("{:#}", parse_plan(v2).unwrap_err());
        assert!(e.contains("Plan-Version 2 wird nicht unterstützt"), "{e}");
        let e = format!("{:#}", parse_plan("{").unwrap_err());
        assert!(e.contains("Plan ungültig"), "{e}");
    }

    #[test]
    fn parse_ignores_unknown_fields() {
        let text =
            r#"{"version":1,"created":0,"modules":[],"partitions":[],"categories":[],"note":"x"}"#;
        assert!(parse_plan(text).unwrap().categories.is_empty());
    }

    fn plan_of(cats: Vec<Category>) -> Plan {
        build_plan(&scan(cats), &modules(), 0, &usage).0
    }

    fn ids(cats: &[Category]) -> Vec<(&str, Vec<&str>)> {
        cats.iter()
            .map(|c| {
                (
                    c.key.as_str(),
                    c.items.iter().map(|i| i.id.as_str()).collect(),
                )
            })
            .collect()
    }

    fn dropped(key: &str, id: &str, reason: Reason) -> Dropped {
        Dropped {
            key: key.into(),
            id: id.into(),
            reason,
        }
    }

    fn idless(items: Vec<Item>) -> Category {
        let mut c = cat("build_cache", Safety::Safe, items).remover(Remover::Run {
            argv: vec!["docker".into(), "builder".into(), "prune".into()],
            with_ids: false,
            sudo: false,
        });
        c.module = "docker".into();
        c
    }

    #[test]
    fn stale_plan_only_shrinks() {
        let plan = plan_of(vec![cat(
            "trash",
            Safety::Safe,
            vec![item("a", 1), item("b", 1)],
        )]);
        let now = scan(vec![cat(
            "trash",
            Safety::Safe,
            vec![item("a", 1), item("c", 1)],
        )]);
        let sel = select(&plan, &now);
        assert_eq!(ids(&sel.todo), [("trash", vec!["a"])]);
        assert!(sel.partial.is_empty());
        assert_eq!(sel.dropped, [dropped("trash", "b", Reason::Gone)]);
    }

    #[test]
    fn edited_safety_has_no_effect() {
        let mut plan = plan_of(vec![cat("build_active", Safety::Check, vec![item("x", 1)])]);
        plan.categories[0].safety = Safety::Safe;
        let now = scan(vec![cat("build_active", Safety::Check, vec![item("x", 1)])]);
        let sel = select(&plan, &now);
        assert!(sel.todo.is_empty());
        assert_eq!(sel.dropped, [dropped("build_active", "x", Reason::NotSafe)]);
    }

    #[test]
    fn moved_to_active_is_not_safe() {
        let plan = plan_of(vec![cat("build_stale", Safety::Safe, vec![item("x", 1)])]);
        let now = scan(vec![cat("build_active", Safety::Check, vec![item("x", 1)])]);
        let sel = select(&plan, &now);
        assert!(sel.todo.is_empty());
        assert_eq!(sel.dropped, [dropped("build_stale", "x", Reason::NotSafe)]);
    }

    #[test]
    fn idless_category_needs_every_item() {
        let now = scan(vec![idless(vec![item("c1", 1), item("c2", 1)])]);
        let sel = select(&plan_of(vec![idless(vec![item("c1", 1)])]), &now);
        assert!(sel.todo.is_empty());
        assert_eq!(ids(&sel.partial), [("build_cache", vec!["c1"])]);
        assert!(sel.dropped.is_empty());
        let both = plan_of(vec![idless(vec![item("c1", 1), item("c2", 1)])]);
        let sel = select(&both, &now);
        assert_eq!(ids(&sel.todo), [("build_cache", vec!["c1", "c2"])]);
        assert!(sel.partial.is_empty());
    }

    #[test]
    fn failed_module_drops_its_items() {
        let plan = plan_of(vec![idless(vec![item("c1", 1)])]);
        let now = Scan {
            cats: Vec::new(),
            failed: vec!["docker".into()],
        };
        let sel = select(&plan, &now);
        assert_eq!(
            sel.dropped,
            [dropped("build_cache", "c1", Reason::ModuleFailed)]
        );
    }

    #[test]
    fn other_levels_never_reported() {
        let plan = plan_of(vec![
            cat("build_active", Safety::Check, vec![item("x", 1)]),
            cat("show", Safety::Info, vec![item("y", 1)]),
        ]);
        let sel = select(&plan, &scan(Vec::new()));
        assert!(sel.todo.is_empty() && sel.partial.is_empty() && sel.dropped.is_empty());
    }

    #[test]
    fn non_utf8_twin_is_not_removed() {
        let plain = item("/t/a\u{fffd}", 1); // a real name with U+FFFD: in the plan
        let mut twin = item("/t/a\u{fffd}", 1); // b"/t/a\xff": same display id, left out
        twin.path = Some(PathBuf::from(OsString::from_vec(b"/t/a\xff".to_vec())));
        let now = scan(vec![cat("trash", Safety::Safe, vec![plain.clone(), twin])]);
        let plan = plan_of(vec![cat("trash", Safety::Safe, vec![plain])]);
        let sel = select(&plan, &now);
        assert_eq!(sel.todo.len(), 1);
        assert_eq!(sel.todo[0].items.len(), 1);
        assert!(sel.todo[0].items[0].path.is_none());
    }
}
