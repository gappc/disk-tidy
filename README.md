# disk-tidy

Shows what fills the disk, per partition and sorted by size, and removes only the parts that are safe to lose.

`disk-tidy preview` is read-only. Per partition it sorts every category into one of four levels and
says in one line what deleting it costs:

| Level | Meaning | Removed by `clean` |
|---|---|---|
| ✓ GEFAHRLOS | recoverable without loss: caches, leftovers, build output of idle projects | yes |
| ⚠ PRÜFEN | recoverable, but costs time or downloads | no; selectable in the TUI or by hand |
| ✗ VORSICHT | may hold data that cannot be recovered | no, by hand |
| ℹ NUR INFO | displayed only | never |

## Usage

```sh
disk-tidy                          # on a terminal: interactive (see TUI below)
disk-tidy preview                  # overview: one line per category, grouped by level
disk-tidy --details                # plus the entries (top 15) and the command to remove by hand
disk-tidy --details --top 0        # every entry
disk-tidy --only docker            # modules: docker, projects, system, trash, show
disk-tidy clean                    # remove everything marked GEFAHRLOS, after asking
disk-tidy clean --only trash --yes
disk-tidy plan > plan.json         # everything found, as JSON (for scripts and agents)
disk-tidy clean --plan plan.json   # remove what the plan lists and is still GEFAHRLOS
```

## What `clean` removes

| Module | Removed | Not removed (listed only) |
|---|---|---|
| docker | dangling images, unused build cache, anonymous volumes without a container, named volumes matching `cache_volumes` | stopped containers, named images, other named volumes; never volumes matching `protect_volumes` |
| projects | `node_modules`, `target`, `.venv` in projects untouched for `stale_days` | build dirs in active projects |
| system (sudo) | disabled snap revisions, journal above `journal_max`, apt cache, `/var/crash/*.crash` | |
| trash | everything in `~/.local/share/Trash` | |
| show | nothing | large paths such as `~/snap/thunderbird/common` |

Details:

- Docker images and volumes are removed by the IDs shown in the preview, so nothing that
  appeared in between is touched. Build cache is the exception: `docker builder prune -a -f`.
- A build dir only counts next to its project file (`target` next to `Cargo.toml` or `pom.xml`, ...).
  A project is the nearest git repo above the build dir; "untouched" means no file outside build dirs
  and `.git` changed, so a monorepo with recent edits keeps all its `node_modules`.
- System commands run with `sudo`. Without a terminal and without passwordless sudo they are skipped
  and printed at the end instead (exit 4).

## TUI

`disk-tidy` without a command opens an interactive view when stdin and stdout are a terminal and
none of `--details`, `--top`, `--only`, `--yes`, `--plan`, `--json` is given (`--config` works).
Otherwise it prints the text preview as before; `disk-tidy preview` always prints text.

1. **Overview:** one row per partition and level: entries, size, selected, free space afterwards.
   Rows appear as the modules finish; a failed module is named below.
2. **Categories** of a row, then **entries** of a category, with size, age and path.
3. **`d`:** review of only what will be deleted, grouped by partition; ⚠ PRÜFEN entries are
   listed one by one with their cost. ⏎ asks once more, naming the PRÜFEN part; `j` deletes.
4. **Progress and report**, then a fresh scan.

| Key | |
|---|---|
| ↑ ↓ | move |
| ⏎ → / ← Esc | open / back |
| ␣ | tick or untick the row, category or entry |
| `a` `n` | all / no entries of the category |
| `i` | details: level, cost, project, image, age, manual command |
| `d` | review and delete |
| `?` `q` | help, quit (refused while deleting) |

- GEFAHRLOS is ticked at the start. PRÜFEN can be ticked: unused images (`docker rmi`), stopped
  containers (`docker rm`), build dirs of active projects. Images of stopped containers, VORSICHT and
  NUR INFO cannot be ticked. Categories without IDs (Docker build cache, journal, apt cache) go as a
  whole.
- Before deleting, the selected modules are scanned again. An entry is removed only if it is still in
  the same category and that category is still GEFAHRLOS or (for ticked PRÜFEN entries) PRÜFEN;
  everything else is listed as "nicht mehr gültig".
- If sudo is needed and has no cached password, the TUI steps aside for `sudo -v` and returns.
- `NO_COLOR` turns colour off; levels always carry a symbol and a word.

`clean` and `clean --plan` still remove GEFAHRLOS only.

## Automation

For scripts and agents. `clean` never asks without a terminal: without `--yes` or `--plan` it removes
nothing and exits 1.

```sh
disk-tidy plan [--only MOD,...] > plan.json     # read, narrow down (delete items or categories)
disk-tidy clean --plan plan.json [--json]       # or --plan - to read it from stdin
disk-tidy clean --yes --json                    # everything GEFAHRLOS, report as JSON
```

`clean --plan` scans the plan's modules again with the current config and removes an entry only if
the plan lists it **and** the fresh scan still finds it in a GEFAHRLOS category. A plan can only
narrow what `clean` would remove: editing `safety` has no effect, and an added entry is removed
only if the fresh scan finds it GEFAHRLOS anyway. A hand-written plan needs only `version`, `modules`
and per category `key`, `module`, `safety` and `items[].id`. Categories whose
remover takes no IDs (Docker build cache, apt cache, journal) are removed only if the plan lists
every entry the fresh scan finds; otherwise they are skipped. Entries whose path is not valid UTF-8
are left out of the plan (named on stderr) and can only be removed interactively.

Plan (`version` 1; all levels, in preview order):

```json
{
  "version": 1,
  "created": 1763459200,
  "modules": ["trash"],
  "partitions": [{"mount": "/home", "total": 500000000000, "used": 400000000000, "free": 100000000000}],
  "categories": [
    {"key": "trash", "module": "trash", "title": "Papierkorb", "safety": "safe",
     "why": "schon gelöscht; danach endgültig weg", "hint": "", "mount": "/home", "total": 12288,
     "items": [{"id": "/home/u/.local/share/Trash/files/x", "label": "x", "size": 12288}]}
  ]
}
```

Report of `clean --json` (stdout holds only this object; all text goes to stderr):

```json
{
  "version": 1,
  "outcome": "done_partial",
  "exit_code": 5,
  "freed": 12288,
  "categories": [{"key": "trash", "status": "removed", "items": [{"id": "/home/u/.local/share/Trash/files/x", "size": 12288}]}],
  "dropped": [{"key": "build_stale", "id": "/home/u/p/target", "reason": "not_safe"}],
  "skipped_sudo": []
}
```

- `status`: `removed`, `failed`, `skipped_sudo`, `skipped_partial` (plan held only part of a
  category without IDs).
- `dropped`: entries the plan marked `safe` that were not removed: `gone`, `not_safe` (now in another
  level, e.g. the project became active), `module_failed` (e.g. Docker not reachable).
- `outcome`: `done`, `nothing`, `aborted`, `done_with_errors`, `done_skipped_sudo`, `done_partial`,
  `error` (then with `"error": "<message>"`; plan file or config errors).
  Usage errors (unknown module, flags that do not fit the command) stay plain text on stderr
  even with `--json`; stdout is then empty and the exit code is 2.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | done, or nothing to remove |
| 1 | aborted: answered no, or no confirmation possible |
| 2 | usage, config or plan file error |
| 3 | done, but some removals failed |
| 4 | done, but sudo commands were skipped |
| 5 | done, but not everything in the plan was carried out (`clean --plan` only) |

If several apply, the lower of 3, 4, 5 wins.

## Install

```sh
./install.sh               # build, install ~/.local/bin/disk-tidy, create ~/.config/disk-tidy/config.toml
./install.sh --uninstall   # remove the binary, keep the config
```

Requires Rust 1.99 (edition 2024) to build. Docker, snap, journalctl and apt are called as external
commands when present.

## Config

`~/.config/disk-tidy/config.toml`, see `config.example.toml`. Missing keys use the defaults; without a
config no named Docker volume is ever removed.

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

`testdata/` holds expected output that the earlier Python version produced; the rendering tests
compare against it byte for byte.
