# disk-tidy phase 2 (automation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `disk-tidy plan` (JSON), `clean --plan FILE|-`, `clean --json`, no prompt without a terminal, exit codes 0–5.

**Architecture:** `collect_all` returns a `Scan` (categories tagged with their module, plus failed modules).
`plan.rs` holds the plan format, `build_plan` and `select` (rescan ∩ plan). `clean` takes a `Selection`
and returns a serialisable `Report`; `main.rs` decides where text and JSON go.

**Tech Stack:** Rust 1.99, edition 2024; clap 4, serde, serde_json, anyhow (existing). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-18-disk-tidy-automation-design.md`

## Global Constraints

- No new crates; no `unsafe`.
- JSON keys English, user-facing texts German. Both JSON formats carry `"version": 1`.
- Exit codes: 0 done/nothing, 1 aborted, 2 usage/config/plan error, 3 done with errors, 4 sudo
  skipped, 5 plan partly carried out. Priority 3 > 4 > 5.
- A plan can never cause removal of anything outside a GEFAHRLOS category of the fresh scan.
- With `--json`, stdout holds exactly one JSON object; everything else goes to stderr.
- Phase 1 output (preview, interactive clean text) stays byte-identical; golden tests must pass.
- Commits: no co-author trailer, no mention of AI.

## Review Focus

1. Plan edited to mark a PRÜFEN category `"safety": "safe"` → nothing of it removed (Task 4 test
   `edited_safety_has_no_effect`).
2. Id-less category (build cache) grew since the plan → skipped, reported `skipped_partial`, exit 5
   (Task 4 `idless_category_needs_every_item`).
3. Docker down at apply time → plan's docker items `module_failed`, exit 5, nothing run (Task 4
   `failed_module_drops_its_items`).
4. `clean --plan -` with the plan on a pipe: no prompt, no hang (Task 5 e2e `plan_roundtrip_via_stdin`).
5. sudo child output landing on stdout would corrupt `--json`: `run_sudo` sends the child's stdout to
   stderr (Task 5, manual check with a fake `sudo`).

---

### Task 1: Scan with module tags and failed modules

**Files:** Modify `src/model.rs`, `src/modules/mod.rs`, `src/main.rs`.

**Interfaces:**
- Produces: `Category.module: String` (serialised; `Category::new` leaves it empty);
  `pub struct Scan { pub cats: Vec<Category>, pub failed: Vec<String> }`;
  `pub fn collect_all(cfg: &Config, only: &[String], runner: &mut dyn Runner) -> Scan` — sets
  `module` on every category, pushes the module name to `failed` when it returns `Err` (the stderr
  note stays).

- [ ] Step 1: tests in `modules/mod.rs`: adapt `drops_empty_categories` to `.cats` and assert
  `scan.cats[0].module == "docker"`; `failing_module_gives_no_categories` also asserts
  `scan.failed == ["docker"]`.
- [ ] Step 2: `cargo test modules::tests` → FAIL (no `Scan`).
- [ ] Step 3: implement; `main.rs` uses `scan.cats`.
- [ ] Step 4: `cargo test` → all pass. Commit "Tag categories with their module; report modules that failed".

### Task 2: Clean returns a report; exit codes; no prompt without a terminal

**Files:** Modify `src/clean.rs`, `src/runner.rs` (FakeRunner), `src/main.rs`.

**Interfaces:**
- Produces (in `clean.rs`, all `Serialize`, enums `rename_all = "snake_case"`):
  - `Outcome { Done, Nothing, Aborted, DoneWithErrors, DoneSkippedSudo, DonePartial, Error }`,
    `exit_code()` → 0,0,1,3,4,5,2.
  - `Status { Removed, Failed, SkippedSudo, SkippedPartial }`.
  - `ReportItem { id: String, size: u64 }`, `CatResult { key: String, status: Status, items: Vec<ReportItem> }`.
  - `Reason { Gone, NotSafe, ModuleFailed }`, `Dropped { key: String, id: String, reason: Reason }`.
  - `Report { version: u32 (=1), outcome: Outcome, exit_code: u8, freed: u64, categories: Vec<CatResult>, dropped: Vec<Dropped>, skipped_sudo: Vec<String>, #[serde(skip_serializing_if = "Option::is_none")] error: Option<String> }`
    with `Report::error(msg: String) -> Report` (outcome Error, exit 2, empty lists).
  - `Selection { todo: Vec<Category>, partial: Vec<Category>, dropped: Vec<Dropped> }` with
    `Selection::all(cats: Vec<Category>) -> Selection` (GEFAHRLOS categories only, nothing dropped).
  - `CleanOptions` gains `can_prompt: bool` (stdin is a terminal and not `--json`); `interactive`
    keeps its sudo meaning.
  - `pub fn clean(sel: Selection, opts: &CleanOptions, input: &mut dyn BufRead, runner: &mut dyn Runner, out: &mut dyn Write, err: &mut dyn Write) -> Report`
- `FakeRunner` gains `pub fail: HashSet<String>`: `run`/`run_sudo` return false for argv joined by " "
  found in it.

Rules: `freed` = sum of item sizes of `Removed` categories. A `Run` remover's commands decide its
category's status: any failed → `Failed`, any skipped for sudo → `SkippedSudo`, else `Removed`.
BuildDirs/Trash: `Removed` if the remover returned true, else `Failed`. `skipped_sudo` strings are
`"sudo " + argv joined by " "`. Outcome: nothing to do → `Nothing`; aborted → `Aborted`; else errors →
`DoneWithErrors`, else skipped sudo → `DoneSkippedSudo`, else `Done`. (Partial handling: Task 4.)
Without `assume_yes` and without `can_prompt`: write `Keine Rückfrage möglich: --yes oder --plan angeben.`
plus newline to `err`, no prompt, nothing removed, `Aborted`.

- [ ] Step 1: adapt existing tests to the new signature (`do_clean` returns `(Report, String, String)`
  for out and err; existing assertions use `report.outcome`); `opts()` sets `can_prompt: true`. New tests:
  - `exit_codes`: Done 0, Nothing 0, Aborted 1, DoneWithErrors 3, DoneSkippedSudo 4, DonePartial 5, Error 2.
  - `no_prompt_without_terminal`: `can_prompt: false`, `assume_yes: false`, input "j\n" → `Aborted`,
    `runner.calls` empty, out has no "Löschen?", err == "Keine Rückfrage möglich: --yes oder --plan angeben.\n".
  - `report_lists_removed_and_freed`: `--yes` on `dangling()` (2 × 1000) → categories
    `[{key: "dangling", status: Removed, items: [a, b]}]`, `freed == 2000`, outcome Done.
  - `failed_command_gives_exit_3`: fail set contains "docker rmi a b" → status Failed, outcome
    DoneWithErrors, freed 0.
  - `skipped_sudo_gives_exit_4`: `journal()`, `sudo_ok: false`, interactive false → status SkippedSudo,
    `skipped_sudo == ["sudo journalctl --vacuum-size=500M"]`, outcome DoneSkippedSudo; text output still
    contains "Ohne sudo übersprungen".
  - `report_json_shape`: `serde_json::to_value(report)` has keys version=1, outcome="done", exit_code=0,
    freed, categories[0].status="removed", no "error" key; `Report::error("x".into())` serialises
    outcome="error", exit_code=2, error="x".
- [ ] Step 2: `cargo test clean::tests` → FAIL (compile errors).
- [ ] Step 3: implement; `main.rs`: `can_prompt = stdin.is_terminal()`, pass `&mut io::stderr()` as err,
  exit with `report.exit_code`.
- [ ] Step 4: `cargo test` → all pass (golden tests unchanged). Commit
  "Report what clean did; exit 3 for errors, 4 for skipped sudo; never prompt without a terminal".

### Task 3: Plan format

**Files:** Create `src/plan.rs`; modify `src/model.rs` (`Safety` also `Deserialize`), `src/main.rs` (`mod plan;`).

**Interfaces:**
- Produces (all `Serialize + Deserialize`, unknown fields ignored):
  - `Plan { version: u32, created: u64, modules: Vec<String>, partitions: Vec<PlanPartition>, categories: Vec<PlanCategory> }`
  - `PlanPartition { mount: String, total: u64, used: u64, free: u64 }`
  - `PlanCategory { key, module, title: String, safety: Safety, why, hint, mount: String, total: u64, items: Vec<PlanItem> }`
  - `PlanItem { id: String, label: String, size: u64 }`
  - `pub fn build_plan(scan: &Scan, modules: &[String], created: u64, usage: &dyn Fn(&Path) -> Usage) -> (Plan, Vec<String>)`
    — second value: display forms of items left out because `item.path` is not valid UTF-8.
    Partitions and category order as `render_preview` (`by_partition`, then `SAFETY_ORDER`, then
    total descending); `mount` via the partition grouping; `total` = sum of the included items.
  - `pub fn parse_plan(text: &str) -> anyhow::Result<Plan>` — invalid JSON: error "Plan ungültig: …";
    version ≠ 1: "Plan-Version {v} wird nicht unterstützt".
  - `pub const PLAN_VERSION: u32 = 1`.

- [ ] Step 1: tests in `plan.rs`:
  - `plan_keeps_all_levels_in_preview_order`: categories safe(total 1), check(5), safe(10) on "/" →
    keys in order safe10, safe1, check5; each has `module`, `mount == "/"`; `partitions[0].mount == "/"`.
  - `plan_leaves_out_non_utf8_paths`: item with `path: Some(PathBuf::from(OsString::from_vec(b"a\xff".to_vec())))`
    is absent, returned in the second value; the category's `total` excludes it.
  - `plan_json_roundtrip`: `parse_plan(&serde_json::to_string(&plan)?)? == plan` (derive `PartialEq`);
    safety serialises as `"safe"`.
  - `parse_rejects_version_and_garbage`: version 2 → error contains "Plan-Version 2"; "{" → error contains "Plan ungültig".
  - `parse_ignores_unknown_fields`.
- [ ] Step 2: `cargo test plan::` → FAIL.
- [ ] Step 3: implement.
- [ ] Step 4: `cargo test` → pass. Commit "Add the plan format".

### Task 4: Select what a plan may remove

**Files:** Modify `src/plan.rs`, `src/clean.rs`.

**Interfaces:**
- Consumes: `Scan`, `Plan`, `Selection`, `Dropped`, `Reason`, `CatResult`, `Status::SkippedPartial`.
- Produces: `pub fn select(plan: &Plan, scan: &Scan) -> Selection`.

Algorithm: plan set = all `(key, id)` of the plan. For every GEFAHRLOS category of the scan, keep
items in the set; none kept → skip; remover `Run { with_ids: false, .. }` and kept ≠ all items →
`partial` (category with its kept items), else `todo`. Dropped: every item of a plan category with
`safety == Safe` that is in neither `todo` nor `partial` under its key; reason `ModuleFailed` if the
category's module is in `scan.failed`, else `NotSafe` if the id occurs in any non-GEFAHRLOS category
of the scan, else `Gone`. Plan items of other levels are ignored.

`clean`: `partial` categories become `CatResult { status: SkippedPartial, items }` without running;
non-empty `dropped` or `partial` downgrades `Done`/`Nothing` to `DonePartial` (3 and 4 still win).
Text output, after the summary and before the prompt: if dropped non-empty
`Aus dem Plan nicht mehr gültig: {n} Einträge (verschwunden, nicht mehr gefahrlos oder Modul ausgefallen).`;
per partial category `Übersprungen, der Plan enthält nicht alle Einträge: {title}`. When `todo` is
empty these lines follow "Nichts Sicheres zu löschen.".

- [ ] Step 1: tests in `plan.rs` (scan built from `Category` values with `module` set):
  - `stale_plan_only_shrinks`: plan safe items a, b; scan safe a, c → todo items [a]; dropped [b Gone]; c not removed.
  - `edited_safety_has_no_effect`: scan category `build_active` (Check) with x; plan has it as Safe with x
    → todo empty; dropped [x NotSafe].
  - `moved_to_active_is_not_safe`: plan `build_stale` x, scan `build_active` x → dropped [x NotSafe].
  - `idless_category_needs_every_item`: scan `build_cache` (Run, with_ids false) items c1, c2; plan c1
    → partial [build_cache], todo empty, no dropped; plan c1, c2 → todo [build_cache].
  - `failed_module_drops_its_items`: scan.failed ["docker"], plan docker item → dropped ModuleFailed.
  - `other_levels_never_reported`: plan check/info items that are gone → dropped empty.
- [ ] Step 2: tests in `clean.rs`: `partial_gives_exit_5` (Selection with one dropped, todo dangling →
  outcome DonePartial, still runs docker rmi; text contains "Aus dem Plan nicht mehr gültig: 1 Einträge");
  `all_dropped_gives_exit_5` (empty todo, one dropped → DonePartial, no calls); `errors_beat_partial`.
- [ ] Step 3: `cargo test` → FAIL. Step 4: implement. Step 5: `cargo test` → pass. Commit
  "Remove only what a plan lists and the rescan still finds GEFAHRLOS".

### Task 5: CLI, JSON output, docs

**Files:** Modify `src/main.rs`, `src/runner.rs`, `README.md`,
`docs/superpowers/specs/2026-11-17-disk-tidy-rust-design.md`; create `tests/cli.rs`.

**Interfaces:**
- `Command` gains `Plan`. `Cli` gains `--plan <FILE>` (`Option<PathBuf>`, "-" = stdin) and `--json`.
  `--top` becomes `Option<usize>` so its use can be detected (default 15 applied in code).
- `fn check_flags(cli: &Cli) -> Result<(), String>` — usage errors (clap `ArgumentConflict`, exit 2):
  `plan` with `--details/--top/--yes/--json/--plan`; `preview` with `--json/--plan/--yes`;
  `clean --plan` with `--only/--yes/--details/--top`. Message e.g. "--top passt nicht zu plan".
- `plan`: scan, `build_plan(created = now in Unix seconds)`, pretty JSON + newline to stdout (write
  errors ignored), left-out items on stderr as `Hinweis: nicht im Plan (Name kein UTF-8): {id}`; exit 0.
- `clean --plan`: read the whole file/stdin first; unreadable or `parse_plan` error, or unknown module
  in `plan.modules` → message (`disk-tidy: …` on stderr, or `Report::error` on stdout with `--json`), exit 2.
  Then scan `plan.modules`, `select`, `clean` with `assume_yes: true`.
- `--json`: config errors also as `Report::error`; text goes to stderr (`out` = stderr), report pretty
  JSON on stdout.
- `SystemRunner::run_sudo`: child stdout to stderr (`.stdout(std::io::stderr())`).

- [ ] Step 1: unit tests in `main.rs`: `check_flags_rejects_mismatches` (each rule above via
  `Cli::try_parse_from`), `check_flags_accepts_valid` (`clean --plan p --json`, `plan --only trash`,
  `clean --yes -d --top 3`).
- [ ] Step 2: e2e tests in `tests/cli.rs` with `env!("CARGO_BIN_EXE_disk-tidy")`, `HOME` = tempdir,
  `--config` pointing to a missing file, `--only trash`, a trash entry `files/x` + `info/x.trashinfo`:
  - `clean_without_terminal_aborts`: `clean --only trash`, stdin null → exit 1, stderr contains
    "Keine Rückfrage möglich", entry still there.
  - `plan_roundtrip_via_stdin`: `plan --only trash` → JSON with `modules == ["trash"]`, category
    `trash` safety "safe"; pipe it into `clean --plan - --json` → exit 0, stdout parses, outcome "done",
    entry gone.
  - `stale_plan_exit_5`: plan, delete the entry by hand, `clean --plan FILE --json` → exit 5,
    dropped[0].reason "gone".
  - `bad_plan_json_error`: `clean --plan FILE --json` with "{" → exit 2, stdout parses, outcome "error".
  - `plan_flag_conflict`: `plan --top 3` → exit 2.
- [ ] Step 3: `cargo test` → FAIL. Step 4: implement. Step 5: `cargo test` and
  `cargo clippy --all-targets -- -D warnings` → pass.
- [ ] Step 6: manual check: in a scratch dir put a fake `sudo` that only prints `SUDO $*` to stdout
  and runs nothing (`-n true` exits 0), first on PATH; run `disk-tidy clean --only system --json` with
  stdin null → stdout is pure JSON (`python3 -m json.tool` succeeds), the "SUDO …" lines appear on
  stderr. Nothing is removed, because the fake sudo executes nothing.
- [ ] Step 7: README section "Automation" (commands, matching rules, both JSON formats with an
  example each, exit-code table; update the exit codes mentioned elsewhere); phase-1 spec phase 2 line
  points to the phase-2 spec. Commit "Add plan, clean --plan and --json; document automation".
