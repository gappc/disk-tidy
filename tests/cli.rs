//! End to end: the built binary with HOME in a temp dir and only the trash module.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    /// A home with one trash entry "x".
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join(".local/share/Trash");
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::write(trash.join("files/x"), vec![1u8; 10_000]).unwrap();
        std::fs::write(trash.join("info/x.trashinfo"), b"[Trash Info]\n").unwrap();
        Home { dir }
    }

    fn entry(&self) -> PathBuf {
        self.dir.path().join(".local/share/Trash/files/x")
    }

    fn run(&self, args: &[&str], stdin: &[u8]) -> Output {
        let config = self.dir.path().join("missing.toml");
        let mut child = Command::new(env!("CARGO_BIN_EXE_disk-tidy"))
            .args(args)
            .arg("--config")
            .arg(&config)
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    }

    fn plan(&self) -> Vec<u8> {
        let out = self.run(&["plan", "--only", "trash"], b"");
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        out.stdout
    }

    fn write(&self, name: &str, content: &[u8]) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, content).unwrap();
        p
    }
}

fn json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(bytes)))
}

fn path_arg(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn clean_without_terminal_aborts() {
    let home = Home::new();
    let out = home.run(&["clean", "--only", "trash"], b"j\n");
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Keine Rückfrage möglich"), "{err}");
    assert!(home.entry().exists());
}

#[test]
fn plan_roundtrip_via_stdin() {
    let home = Home::new();
    let plan = home.plan();
    let p = json(&plan);
    assert_eq!(p["version"], 1);
    assert_eq!(p["modules"], serde_json::json!(["trash"]));
    assert_eq!(p["categories"][0]["key"], "trash");
    assert_eq!(p["categories"][0]["safety"], "safe");
    assert!(home.entry().exists());

    let out = home.run(&["clean", "--plan", "-", "--json"], &plan);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let r = json(&out.stdout);
    assert_eq!(r["outcome"], "done");
    assert_eq!(r["categories"][0]["status"], "removed");
    assert!(!home.entry().exists());
}

#[test]
fn stale_plan_exit_5() {
    let home = Home::new();
    let plan = home.write("plan.json", &home.plan());
    std::fs::remove_file(home.entry()).unwrap();
    let out = home.run(&["clean", "--plan", path_arg(&plan), "--json"], b"");
    assert_eq!(out.status.code(), Some(5), "{out:?}");
    let r = json(&out.stdout);
    assert_eq!(r["outcome"], "done_partial");
    assert_eq!(r["dropped"][0]["reason"], "gone");
}

#[test]
fn bad_plan_json_error() {
    let home = Home::new();
    let plan = home.write("plan.json", b"{");
    let out = home.run(&["clean", "--plan", path_arg(&plan), "--json"], b"");
    assert_eq!(out.status.code(), Some(2));
    let r = json(&out.stdout);
    assert_eq!(r["outcome"], "error");
    assert!(
        r["error"].as_str().unwrap().contains("Plan ungültig"),
        "{r}"
    );
    assert!(home.entry().exists());
}

#[test]
fn bad_plan_text_error() {
    let home = Home::new();
    let out = home.run(&["clean", "--plan", "/nonexistent/plan.json"], b"");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("disk-tidy: "));
}

#[test]
fn plan_with_unknown_module_is_rejected() {
    let home = Home::new();
    let text = String::from_utf8(home.plan())
        .unwrap()
        .replace("\"trash\"\n  ]", "\"bogus\"\n  ]");
    assert!(text.contains("bogus"), "{text}");
    let plan = home.write("plan.json", text.as_bytes());
    let out = home.run(&["clean", "--plan", path_arg(&plan)], b"");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(home.entry().exists());
}

#[test]
fn plan_flag_conflict() {
    let home = Home::new();
    assert_eq!(
        home.run(&["plan", "--top", "3"], b"").status.code(),
        Some(2)
    );
    assert_eq!(
        home.run(&["clean", "--plan", "-", "--only", "trash"], b"")
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn json_abort_without_terminal() {
    let home = Home::new();
    let out = home.run(&["clean", "--only", "trash", "--json"], b"");
    assert_eq!(out.status.code(), Some(1));
    let r = json(&out.stdout);
    assert_eq!(
        (&r["outcome"], &r["exit_code"]),
        (&"aborted".into(), &1.into())
    );
    assert!(home.entry().exists());
}

#[test]
fn json_config_error() {
    let home = Home::new();
    let config = home.write("bad.toml", b"[[[");
    let out = Command::new(env!("CARGO_BIN_EXE_disk-tidy"))
        .args(["clean", "--yes", "--json", "--config", path_arg(&config)])
        .env("HOME", home.dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let r = json(&out.stdout);
    assert_eq!(r["outcome"], "error");
    assert!(r["error"].as_str().unwrap().contains("ungültig"), "{r}");
}
