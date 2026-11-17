# disk-tidy in Rust: design

Decided 2026-11-17. Supersedes the Python implementation, which stays as reference in `python/`
until the Rust port reaches parity.

## Why

disk-tidy gets an interactive TUI. Research (2026-11-17) compared Python/Textual, Rust/Ratatui,
Go/Bubble Tea and TypeScript/Ink. Chosen: **Rust + Ratatui**: team-maintained TUI library
(ratatui org), proven for this exact kind of tool (dua-cli), single binary, and the user writes Rust.
No Rust TUI library offers a tree with checkboxes; like dua-cli we build it on a plain List with our
own selection model.

## Phases (one plan each)

1. **Port** the current Python tool to Rust with identical behaviour and output. Remove `python/`.
2. **Automation**: `plan --json` / `clean --plan FILE`, no prompt without a TTY, documented exit codes.
3. **TUI**: `disk-tidy tui` (or interactive default on a TTY, decided in phase 3).

Phase 2 and 3 get their own plans once phase 1 is done.

## Phase 1 requirements (behaviour of the Python version, commit after "safety levels")

- Commands: `disk-tidy [preview] [-d|--details] [--top N] [--only MOD,...] [--config PATH]` and
  `disk-tidy clean [--yes] [-d|--details] [--top N] [--only ...] [--config PATH]`.
  `--top` default 15, 0 = all. Modules: `docker, projects, system, trash, show`; unknown module is a
  usage error.
- Config `~/.config/disk-tidy/config.toml`, same keys and defaults as `python/disk_tidy/core.py`
  `DEFAULT_CONFIG`; missing keys fall back per key. The existing user config must keep working.
- Model: categories with safety level `safe | check | data | info`, a one-line "why", an optional
  manual hint, items (size, id, label). Only `safe` is removed by `clean`.
- Modules and their categories, titles, "why" texts, hints and rules: exactly as in
  `python/disk_tidy/{docker,projects,system,trash,show}.py`.
- Output text (German), layout, colours and summary lines: exactly as `python/disk_tidy/cli.py`.
  Colour only when stdout is a terminal and `NO_COLOR` is unset.
- Sizes: decimal units (kB, MB, GB). Directory sizes are allocated bytes like `du -s` (st_blocks*512,
  hard links counted once, symlinks not followed, unreadable parts skipped). Computed in-process
  instead of calling `du`.
- `clean`: shows only the GEFAHRLOS level, asks `Löschen? [j/N]` (yes = j, ja, y, yes); `--yes` skips.
  Docker images and volumes are removed by the listed IDs; build cache via
  `docker builder prune -a -f`. Build dirs and trash entries are re-checked before deletion.
  Sudo commands run via `sudo` when passwordless sudo works or stdin is a terminal; otherwise they
  are printed at the end. Exit 0 = done, 1 = aborted, 2 = done with errors.
- Docker, snap, journalctl and apt are called as external commands (no Docker API crate).

## Non-goals for phase 1

JSON output, plan files, TUI, parallel scanning, package caches (`~/.cache`, `~/.npm`, ...).
