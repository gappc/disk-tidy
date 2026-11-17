# disk-tidy

Shows what fills the disk, per partition and sorted by size, and removes only the parts that are safe to lose.

`disk-tidy preview` is read-only. Per partition it sorts every category into one of four levels and
says in one line what deleting it costs:

| Level | Meaning | Removed by `clean` |
|---|---|---|
| ✓ GEFAHRLOS | recoverable without loss: caches, leftovers, build output of idle projects | yes |
| ⚠ PRÜFEN | recoverable, but costs time or downloads | no, by hand |
| ✗ VORSICHT | may hold data that cannot be recovered | no, by hand |
| ℹ NUR INFO | displayed only | never |

## Usage

```sh
disk-tidy                          # overview: one line per category, grouped by level
disk-tidy --details                # plus the entries (top 15) and the command to remove by hand
disk-tidy --details --top 0        # every entry
disk-tidy --only docker            # modules: docker, projects, system, trash, show
disk-tidy clean                    # remove everything marked GEFAHRLOS, after asking
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

- Docker images and volumes are removed by the IDs shown in the preview, so nothing that
  appeared in between is touched. Build cache is the exception: `docker builder prune -a -f`.
- A build dir only counts next to its project file (`target` next to `Cargo.toml` or `pom.xml`, ...).
  A project is the nearest git repo above the build dir; "untouched" means no file outside build dirs
  and `.git` changed, so a monorepo with recent edits keeps all its `node_modules`.
- System commands run with `sudo`. Without a terminal and without passwordless sudo they are skipped
  and printed at the end instead.

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
