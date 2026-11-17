# disk-tidy Rust Port (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Python disk-tidy with a Rust binary that behaves and prints exactly the same.

**Architecture:** One binary crate at the repo root. Modules collect `Category` values (pure
classification functions + thin collectors that call external commands). Removal is described as
data (`Remover` enum), executed by one executor, so phase 2 can serialize plans. Rendering is a pure
function from categories to text.

**Tech Stack:** Rust 1.99, edition 2024; clap 4 (derive), serde 1, serde_json 1, toml 1, anyhow 1,
glob 0.3, rustix (feature `fs`, for statvfs); dev: tempfile 3.

**Spec:** `docs/superpowers/specs/2026-11-17-disk-tidy-rust-design.md`. The Python sources in
`python/disk_tidy/` are the reference for every title, "why" text, hint, label format and output line.

## Global Constraints

- Edition 2024, no `unsafe`, no tokio, no Docker API crate; external commands via `std::process::Command`.
- Config path `~/.config/disk-tidy/config.toml`; keys and defaults identical to `python/disk_tidy/core.py` `DEFAULT_CONFIG`.
- All user-facing text identical to the Python version (German), including spacing and column widths.
- Nothing is deleted without the `[j/N]` confirmation or `--yes`.
- Docker images and volumes are removed only by the IDs collected in the same run.
- Commit messages: no AI attribution, no `Co-Authored-By` trailer.
- Tests: `cargo test` must pass at the end of every task; `cargo clippy -- -D warnings` clean.

## Review Focus

1. Trash and build-dir names with spaces and non-ASCII (`01 · Einstieg & Startscreen.png`) must list and delete correctly → test in Task 6.
2. Symlinks inside project roots (to a build dir, or a loop) must never be followed or deleted through → test in Task 5.
3. Unreadable directories while sizing must be skipped, not abort the run → test in Task 1.
4. Docker not installed or daemon down: preview of the other modules continues and a note goes to stderr → test in Task 4.
5. `clean` with stdin at EOF (no terminal, piped empty input) must abort as "no", deleting nothing → test in Task 8.

---

## File Structure

```
Cargo.toml
src/main.rs            clap CLI, module selection, dispatch, exit codes
src/model.rs           Safety, Item, Category, Remover
src/size.rs            parse_size, fmt_size, dir_size
src/config.rs          Config structs with serde defaults, load()
src/partition.rs       mount_point, disk_usage
src/render.rs          colour(), render_preview(), render_clean_summary()
src/clean.rs           confirm(), execute(): runs Removers, sudo handling
src/runner.rs          Runner trait + SystemRunner (std::process::Command)
src/modules/mod.rs     Module list, collect_all()
src/modules/docker.rs  DockerDf structs, classify(), collect()
src/modules/projects.rs
src/modules/system.rs
src/modules/trash.rs
src/modules/show.rs
install.sh             cargo build --release, install binary to ~/.local/bin
python/                reference implementation, deleted in Task 9
```

Unit tests live in each file (`#[cfg(test)] mod tests`).

---

### Task 0: Baseline

**Files:** move `disk-tidy`, `disk_tidy/`, `tests/`, `config.example.toml`, `install.sh` → `python/`

- [ ] **Step 1:** Commit the pending Python changes (safety levels) as they are:
  `git add -A && git commit -m "Group preview by safety level and explain each category"`
- [ ] **Step 2:** `git mv` the files above into `python/`; keep `README.md`, `docs/`, `.gitignore` at the root.
- [ ] **Step 3:** Run `cd python && python3 -m unittest discover -s tests` → `OK` (31 tests); re-run `python/install.sh` so `~/.local/bin/disk-tidy` points to `python/disk-tidy`; `disk-tidy --only trash` prints the overview.
- [ ] **Step 4:** Commit: `Move Python implementation to python/ as reference for the Rust port`

---

### Task 1: Crate, model and sizes

**Files:** Create `Cargo.toml`, `src/main.rs` (stub `fn main() {}` + `mod` lines), `src/model.rs`, `src/size.rs`. Add `target/` to `.gitignore`.

**Interfaces — Produces:**
```rust
// model.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Safety { Safe, Check, Data, Info }          // order = display order
pub const SAFETY_ORDER: [Safety; 4];
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item { pub size: u64, pub id: String, pub label: String }
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Remover {
    None,
    Run { argv: Vec<String>, with_ids: bool, sudo: bool }, // with_ids: append every item id
    SnapRevisions,                                         // sudo; id = "name=rev"
    BuildDirs,                                             // ids are paths
    Trash { base: PathBuf },                               // ids are paths in base/files
}
#[derive(Clone, Debug, Serialize)]
pub struct Category { pub key: String, pub title: String, pub path: PathBuf, pub safety: Safety,
    pub why: String, pub hint: String, pub items: Vec<Item>, pub remover: Remover }
impl Category {
    pub fn new(key: &str, title: &str, path: impl Into<PathBuf>, safety: Safety, why: &str) -> Self; // hint "", items [], Remover::None
    pub fn hint(self, hint: &str) -> Self;
    pub fn remover(self, r: Remover) -> Self;
    pub fn total(&self) -> u64;
    pub fn is_clean(&self) -> bool;          // safety == Safe
    pub fn sorted(self) -> Self;             // items by size desc
    pub fn needs_sudo(&self) -> bool;        // Run{sudo:true} | SnapRevisions
}
// size.rs
pub fn parse_size(s: &str) -> u64;           // "1.17GB", "7.834kB", "3.7G", "500M", "2.36GB (virtual 3GB)", "" -> 0
pub fn fmt_size(n: u64) -> String;          // width 7 + unit, as Python fmt_size
pub fn dir_size(path: &Path) -> u64;        // du -s semantics, see spec
```

- [ ] **Step 1: Write failing tests** in `size.rs`:
  `parse_docker_units`: `parse_size("0B")==0`, `"7.834kB"==7834`, `"1.17GB"==1_170_000_000`, `"2.36GB (virtual 3GB)"==2_360_000_000`, `""==0`.
  `parse_short_units`: `"500M"==500_000_000`, `"3.7G"==3_700_000_000`.
  `format`: `fmt_size(1_600_000_000).trim()=="1.6 GB"`, `fmt_size(512).trim()=="512 B"`, `fmt_size(512)=="    512 B "`.
  `dir_size_counts_hardlink_once`: tempdir with 100 kB file `a` and hard link `b` → `dir_size >= 100_000 && < 200_000`.
  `dir_size_skips_unreadable`: tempdir with readable 100 kB file and subdir chmod 000 containing a file → returns `>= 100_000`, no panic (restore permissions in the test before tempdir drop).
  `dir_size_missing_path_is_zero`.
  In `model.rs`: `sorted_and_total` (items 1 and 3 → order [3,1], total 4), `only_safe_is_clean`.
- [ ] **Step 2:** `cargo test` → fails (functions missing).
- [ ] **Step 3:** Implement. `parse_size` without regex: take leading digits/dot, then optional `[kKMGT]` and optional `B`; units decimal. `dir_size`: iterative walk with `symlink_metadata`, sum `blocks()*512` of every entry including dirs, `HashSet<(dev, ino)>` for `nlink() > 1`, ignore errors.
- [ ] **Step 4:** `cargo test` → pass; `cargo clippy -- -D warnings` clean.
- [ ] **Step 5:** Commit `Add Rust crate with category model and size helpers`.

---

### Task 2: Config and partitions

**Files:** Create `src/config.rs`, `src/partition.rs`.

**Interfaces — Produces:**
```rust
// config.rs  (all structs #[derive(Deserialize, Clone, Debug, PartialEq)] #[serde(default)])
pub struct Config { pub docker: DockerConfig, pub projects: ProjectsConfig, pub system: SystemConfig, pub show: ShowConfig }
pub struct DockerConfig { pub cache_volumes: Vec<String>, pub protect_volumes: Vec<String> }   // [], ["*db*","*data*"]
pub struct ProjectsConfig { pub roots: Vec<String>, pub stale_days: u64 }                       // ["~/projects"], 28
pub struct SystemConfig { pub journal_max: String }                                            // "500M"
pub struct ShowConfig { pub paths: Vec<String> }                                               // ["~/snap/*/common"]
impl Default for each (values above);
pub fn default_path() -> PathBuf;                    // $HOME/.config/disk-tidy/config.toml
pub fn load(path: &Path) -> anyhow::Result<Config>;  // missing file -> Config::default()
pub fn expand_home(s: &str) -> PathBuf;              // leading "~" -> $HOME
// partition.rs
pub fn mount_point(path: &Path) -> PathBuf;          // nearest existing ancestor, then walk up while parent has same st_dev
pub struct Usage { pub total: u64, pub free: u64, pub used: u64 }   // free = bavail*frsize, used = (blocks-bfree)*frsize
pub fn disk_usage(mount: &Path) -> Usage;            // rustix::fs::statvfs
```

- [ ] **Step 1: Write failing tests:**
  `missing_file_gives_defaults`: `load("/nonexistent/x.toml") == Config::default()`.
  `file_overrides_per_key`: toml `[docker]\ncache_volumes=["x*"]\n[projects]\nstale_days=7\n` → `cache_volumes==["x*"]`, `protect_volumes==["*db*","*data*"]`, `stale_days==7`, `roots==["~/projects"]`.
  `invalid_toml_is_error`: `load` of `"[docker"` → `Err` whose message contains the file path.
  `user_config_parses`: `load` of `python/config.example.toml` → `Ok`, `cache_volumes` has 3 entries.
  `expand_home`: `expand_home("~/x") == $HOME/x`, `expand_home("/a") == "/a"`.
  `mount_point_of_missing_path`: `mount_point("/proc/does/not/exist") == "/proc"`; `mount_point("/") == "/"`.
  `disk_usage_root`: `total > 0 && free <= total`.
- [ ] **Step 2:** `cargo test` → fails.
- [ ] **Step 3:** Implement; `load` uses `anyhow::Context` with the path.
- [ ] **Step 4:** `cargo test` → pass; clippy clean.
- [ ] **Step 5:** Commit `Add config loading and partition helpers`.

---

### Task 3: Runner

**Files:** Create `src/runner.rs`.

**Interfaces — Produces:**
```rust
pub trait Runner {
    fn output(&mut self, argv: &[&str]) -> Option<String>;   // stdout if the command ran with exit 0, else None (also when not found)
    fn run(&mut self, argv: &[String]) -> bool;               // success; on failure prints "  Fehler: <first 4 args joined>...: <stderr>" to stderr
    fn run_sudo(&mut self, argv: &[String]) -> bool;          // "sudo" + argv, inherits the terminal
    fn can_sudo(&mut self, interactive: bool) -> bool;        // `sudo -n true` succeeds, or interactive
}
pub struct SystemRunner;
#[cfg(test)] pub struct FakeRunner { pub outputs: HashMap<String, String>, pub calls: Vec<Vec<String>>, pub sudo_ok: bool }
// FakeRunner: key = argv joined by " "; run/run_sudo record calls (run_sudo records with leading "sudo") and return true.
```

- [ ] **Step 1: Write failing tests:** `output_of_true_command` (`SystemRunner.output(&["echo","hi"]) == Some("hi\n")`), `output_of_missing_binary_is_none` (`&["/nonexistent/x"]`), `run_reports_failure` (`SystemRunner.run(&["false".into()]) == false`).
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** Commit `Add command runner`.

---

### Task 4: Docker module

**Files:** Create `src/modules/mod.rs`, `src/modules/docker.rs`.

**Interfaces:**
- Consumes: `Category`, `Item`, `Remover`, `Safety` (Task 1), `parse_size` (Task 1), `DockerConfig` (Task 2), `Runner` (Task 3).
- Produces:
```rust
// docker.rs — serde structs mirror `docker system df -v --format json` (all values are strings)
#[derive(Deserialize, Default)] #[serde(rename_all = "PascalCase", default)]
pub struct DockerDf { pub containers: Vec<DfContainer>, pub images: Vec<DfImage>, pub volumes: Vec<DfVolume>, pub build_cache: Vec<DfCache> }
// DfContainer{id,names,image,state,status,size}, DfImage{id,repository,tag,containers,unique_size,created_since},
// DfVolume{name,links,size}, DfCache{id,in_use,size,shared,last_used_since,description}  (field names PascalCase in JSON; "ID" needs #[serde(rename="ID")])
pub fn classify(df: &DockerDf, cfg: &DockerConfig, root: &Path) -> Vec<Category>;
pub fn collect(cfg: &DockerConfig, runner: &mut dyn Runner, docker_bin: &str) -> Result<Vec<Category>, String>;
// Err("docker nicht installiert") when `<docker_bin> --version` fails; Err("Docker-Daemon nicht erreichbar") when df fails or JSON is invalid.
// mod.rs
pub const MODULES: [&str; 5] = ["docker", "projects", "system", "trash", "show"];
pub fn collect_all(cfg: &Config, only: &[String], runner: &mut dyn Runner) -> Vec<Category>;  // in MODULES order, drops empty categories, prints "Hinweis (<mod>): <note>" to stderr on Err
```
Category keys, order, titles, safety, why, hints and removers exactly as `python/disk_tidy/docker.py`:
`dangling`→`Run{["docker","rmi"],with_ids,!sudo}`, `build_cache`→`Run{["docker","builder","prune","-a","-f"],!with_ids}`, `anon_volumes` and `cache_volumes`→`Run{["docker","volume","rm"],with_ids}`. Anonymous volume = 64 lowercase hex chars. Globs via `glob::Pattern`.

- [ ] **Step 1: Write failing tests**, porting `python/tests/test_docker.py` (same fixture DF and CFG, built as `DockerDf` values): `stopped_containers`, `images`, `build_cache_excludes_in_use`, `volumes`, `items_sorted_by_size`, `only_safe_categories_are_cleaned`, `default_config_cleans_no_named_volumes`, plus `every_category_has_level_and_reason` (from `python/tests/test_cli.py::SafetyTest`), plus:
  `parses_real_df_json`: a JSON string with one entry per list using real field names (`"ID"`, `"Repository"`, `"UniqueSize"`, `"InUse"`, `"Links"`, ...) deserializes and classifies.
  `missing_docker_is_note` (Review Focus 4): `collect(&cfg, &mut SystemRunner, "/nonexistent/docker") == Err("docker nicht installiert")`.
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** Commit `Port Docker module`.

---

### Task 5: Projects module

**Files:** Create `src/modules/projects.rs`; register in `mod.rs`.

**Interfaces — Produces:**
```rust
pub const BUILD_DIRS: [(&str, &[&str]); 3];   // node_modules: package.json; target: Cargo.toml, pom.xml; .venv: pyproject.toml, requirements.txt, setup.py, uv.lock, Pipfile
pub fn find_build_dirs(root: &Path) -> Vec<PathBuf>;            // never descends into build dirs or .git, never follows symlinks
pub fn project_of(build_dir: &Path, root: &Path) -> PathBuf;    // nearest ancestor with .git up to root, else parent
pub fn last_change(project: &Path, newer_than: Option<SystemTime>) -> SystemTime;  // as Python, incl. .git/index and .git/HEAD, early stop
pub fn classify_dirs(found: Vec<(PathBuf, PathBuf)>, sizes: &dyn Fn(&Path) -> u64, stale_days: u64, now: SystemTime) -> Vec<Category>;
pub fn collect(cfg: &ProjectsConfig, now: SystemTime) -> Vec<Category>;
pub fn remove_build_dirs(items: &[Item]) -> bool;               // refuses symlinks, non-dirs and names not in BUILD_DIRS (prints "  Übersprungen (kein Build-Ordner): <path>")
```
Categories `build_stale` (Safe, `Remover::BuildDirs`) and `build_active` (Check, hint `rm -rf <ordner>  (oder cargo clean)`), titles/why/labels as `python/disk_tidy/projects.py`.

- [ ] **Step 1: Write failing tests**, porting `python/tests/test_modules.py::ProjectsTest` (same tree: `old`, `fresh`, `docs/target`, `mono`): `classification`, `remove_deletes_only_listed_build_dirs`, `remove_refuses_non_build_dir`, `no_roots`. Set mtimes with `std::fs::File::set_modified`. Plus (Review Focus 2):
  `symlinks_are_not_followed`: in `old/`, `link_nm -> ../fresh/target` named `node_modules` inside a second project `linked/` (with `package.json`), and a loop `old/loop -> ..`; `find_build_dirs` returns neither the symlink nor anything below the loop, terminates.
  `remove_refuses_symlink`: `remove_build_dirs` on the symlinked `node_modules` returns `false` and the target dir still exists.
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** Commit `Port projects module`.

---

### Task 6: System, trash and show modules

**Files:** Create `src/modules/system.rs`, `src/modules/trash.rs`, `src/modules/show.rs`; register in `mod.rs`.

**Interfaces — Produces:**
```rust
// system.rs
pub fn parse_disabled_snaps(snap_list: &str) -> Vec<(String, String)>;
pub fn parse_journal_usage(text: &str) -> u64;
pub fn collect(cfg: &SystemConfig, runner: &mut dyn Runner) -> Vec<Category>;
// snap_revisions: Remover::SnapRevisions; journal: Run{["journalctl","--vacuum-size=<max>"],!with_ids,sudo};
// apt_cache: Run{["apt-get","clean"],sudo}; crash: Run{["rm","-f","--"],with_ids,sudo}
// trash.rs
pub fn collect(base: &Path) -> Vec<Category>;     // base = ~/.local/share/Trash in production
pub fn remove_entries(items: &[Item], base: &Path) -> bool;   // only direct children of base/files; also deletes base/info/<name>.trashinfo
// show.rs
pub fn collect(cfg: &ShowConfig) -> Vec<Category>;
```
Titles, why texts, labels as the Python modules. File sizes: `blocks()*512`.

- [ ] **Step 1: Write failing tests**, porting `SystemParseTest` (`disabled_snaps`, `journal_usage`, `snap_commands` → assert the argv the executor will build, see Task 8 `commands_for`; here assert ids are `"gimp=561"`), `journal_command_uses_config` (Remover equals `Run{argv:["journalctl","--vacuum-size=1G"],..}`), `TrashTest` (`collect_and_remove`, `refuses_path_outside_trash`), plus (Review Focus 1):
  `trash_unicode_names`: entries `01 · Einstieg & Startscreen.png` and `a b/` with matching `.trashinfo` → both listed with exact names as labels, both removed, info files gone.
  `show_expands_globs`: tempdir with `x/common`, `y/common` and pattern `<tmp>/*/common` → 2 items, labels as paths.
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** Commit `Port system, trash and show modules`.

---

### Task 7: Rendering

**Files:** Create `src/render.rs`.

**Interfaces:**
- Consumes: `Category`, `Safety`, `SAFETY_ORDER`, `fmt_size`, `mount_point`, `disk_usage`.
- Produces:
```rust
pub struct Style { pub colour: bool }          // colour = stdout is terminal && NO_COLOR unset
pub fn by_partition(cats: &[Category]) -> Vec<(PathBuf, Vec<&Category>)>;   // first-appearance order
pub fn render_preview(cats: &[Category], details: bool, top: usize, style: &Style, usage: &dyn Fn(&Path) -> Usage) -> String;
pub fn render_clean_summary(cats: &[Category], details: bool, top: usize, style: &Style, usage: &dyn Fn(&Path) -> Usage) -> String;
```
Text, symbols (✓ ⚠ ✗ ℹ), headings, `TITLE_WIDTH` 44, ANSI codes and summary lines exactly as `python/disk_tidy/cli.py` (`preview`, `print_level`, `print_partition`, and the part of `clean` before the prompt).

- [ ] **Step 1: Write failing tests**, porting `python/tests/test_cli.py::PreviewTest` (`levels_in_order_each_category_under_its_level`, `summary_counts_only_safe` → contains `"Gefahrlos freizugeben: / 1.0 GB"`, `entries_only_with_details`) with `Style{colour:false}` and a fixed `usage` closure, plus:
  `golden_preview`: categories from the Task 4 fixture rendered without colour equal a golden string; produce the golden by running the Python `preview()` on the same fixture (`python3 -c` with `python/` on `sys.path`, `shutil.disk_usage` patched to the same Usage) and paste it into the test.
  `no_ansi_without_colour`: output contains no `\x1b[`.
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** Commit `Port preview rendering`.

---

### Task 8: Clean executor and CLI

**Files:** Create `src/clean.rs`; fill `src/main.rs`.

**Interfaces:**
- Consumes: everything above.
- Produces:
```rust
// clean.rs
pub fn commands_for(cat: &Category) -> Vec<Vec<String>>;   // Run -> [argv (+ids)], SnapRevisions -> per item ["snap","remove",name,"--revision=<rev>"], else []
pub fn confirm(input: &mut dyn BufRead) -> bool;           // reads one line; "j","ja","y","yes" (trimmed, lowercase) -> true; EOF -> false
pub enum Outcome { Done, Aborted, DoneWithErrors }         // exit codes 0, 1, 2
pub fn clean(cats: Vec<Category>, assume_yes: bool, interactive: bool, input: &mut dyn BufRead,
             runner: &mut dyn Runner, out: &mut dyn Write) -> Outcome;
```
`clean` writes the summary (Task 7), the prompt `Löschen? [j/N] `, `Abgebrochen, nichts gelöscht.`, per category `→ <title>`, the skipped-sudo block and `Fertig.` / `Fertig, mit Fehlern (siehe oben).` exactly as Python. `BuildDirs` → `projects::remove_build_dirs`, `Trash` → `trash::remove_entries`, `Run`/`SnapRevisions` → `runner.run` or, if sudo, `runner.run_sudo` when `runner.can_sudo(interactive)` (checked once) else collected and printed as `  sudo <argv joined>`.
`main.rs`: clap derive per spec (`command` positional `preview|clean`, default preview; `--details/-d`, `--top` 15, `--only` comma list validated against `MODULES` → clap usage error, `--yes`, `--config`). Config load error → message on stderr, exit 2. `interactive` = stdin is terminal.

- [ ] **Step 1: Write failing tests** with `FakeRunner` and in-memory IO:
  `commands_for_variants`: dangling with ids `a`,`b` → `[["docker","rmi","a","b"]]`; build cache → `[["docker","builder","prune","-a","-f"]]`; snap item `gimp=561` → `[["snap","remove","gimp","--revision=561"]]`.
  `confirm_answers`: `"j\n"`, `"JA\n"`, `"yes\n"` → true; `"n\n"`, `"\n"` → false.
  `eof_aborts_and_deletes_nothing` (Review Focus 5): `clean` with empty input, `assume_yes=false` → `Outcome::Aborted`, `runner.calls` empty, output ends with `Abgebrochen, nichts gelöscht.\n`.
  `yes_runs_docker_commands_by_id`: dangling category → `runner.calls == [["docker","rmi","a","b"]]`, `Outcome::Done`.
  `sudo_skipped_without_terminal`: journal category, `sudo_ok=false`, `interactive=false` → no calls, output contains `Ohne sudo übersprungen. Selbst ausführen:` and `  sudo journalctl --vacuum-size=500M`.
  `only_safe_categories_are_touched`: categories of all four levels with `Run` removers → only the Safe one is called.
- [ ] **Step 2–4:** fail → implement → pass; clippy clean.
- [ ] **Step 5:** `cargo run -- --help` lists the options; `cargo run -- --only bogus` exits 2 with a usage error.
- [ ] **Step 6:** Commit `Port clean executor and command line`.

---

### Task 9: Parity check, install, remove Python

**Files:** Create `install.sh`, `config.example.toml` (moved back from `python/`); modify `README.md`; delete `python/`.

- [ ] **Step 1: Parity (read-only):** for `M` in `docker system trash show`:
  `diff <(NO_COLOR=1 python/disk-tidy --only $M --details --top 0) <(NO_COLOR=1 cargo run -q --release -- --only $M --details --top 0)` → no differences.
  For `projects`: same command; differences allowed only in size figures of individual lines (du vs in-process); entry set and categories identical: compare after `sed -E 's/[0-9.]+ (B |kB|MB|GB)//g'`.
- [ ] **Step 2: Clean parity without deleting:** `echo n | cargo run -q -- clean` prints the same summary as `echo n | python/disk-tidy clean` and ends with `Abgebrochen, nichts gelöscht.`; exit code 1.
- [ ] **Step 3:** `install.sh`: `cargo build --release`, `install -m 755 target/release/disk-tidy ~/.local/bin/disk-tidy` (replaces the symlink), create `~/.config/disk-tidy/config.toml` from `config.example.toml` if missing; `--uninstall` removes the binary. Run it; `disk-tidy --only trash` works; `readlink ~/.local/bin/disk-tidy` is empty (a real file).
- [ ] **Step 4:** `git mv python/config.example.toml config.example.toml` and change the path in the `user_config_parses` test (Task 2) to `config.example.toml`. README: install via `./install.sh`, tests via `cargo test`, requirements Rust 1.99; drop Python lines. `git rm -r python/`.
- [ ] **Step 5:** `cargo test && cargo clippy -- -D warnings` → pass. Commit `Replace Python implementation with the Rust port`.
