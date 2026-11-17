# disk-tidy

Shows what fills the disk, per partition and sorted by size, and removes only the parts that are safe to lose.

`disk-tidy preview` is read-only. Every category is marked `[clean]` (removed by `disk-tidy clean`) or
`[manuell]` (only listed, with the command to remove it by hand).

## Usage

```sh
disk-tidy                          # same as: disk-tidy preview
disk-tidy preview --top 0          # every entry, not just the top 15 per category
disk-tidy preview --only docker    # modules: docker, projects, system, trash, show
disk-tidy clean                    # remove all [clean] categories, after asking
disk-tidy clean --only trash --yes
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

- Docker images, containers and volumes are removed by the IDs shown in the preview, so nothing that
  appeared in between is touched. Build cache is the exception: `docker builder prune -a -f`.
- A build dir only counts next to its project file (`target` next to `Cargo.toml` or `pom.xml`, ...).
  A project is the nearest git repo above the build dir; "untouched" means no file outside build dirs
  and `.git` changed, so a monorepo with recent edits keeps all its `node_modules`.
- System commands run with `sudo`. Without a terminal and without passwordless sudo they are skipped
  and printed at the end instead.

## Install

```sh
./install.sh   # symlink ~/.local/bin/disk-tidy, create ~/.config/disk-tidy/config.toml
```

Requires Python 3.11+ (uses `tomllib`), no other dependencies.

## Config

`~/.config/disk-tidy/config.toml`, see `config.example.toml`. Missing keys use the defaults; without a
config no named Docker volume is ever removed.

## Tests

```sh
python3 -m unittest discover -s tests
```
