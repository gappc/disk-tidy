# disk-tidy phase 3: TUI

Decided 2026-11-18. Builds on phase 1 (`2026-11-17-disk-tidy-rust-design.md`) and phase 2
(`2026-11-18-disk-tidy-automation-design.md`), both on main.

## Why

The user wants easier navigation, a clear overview, and to set what gets deleted, instead of reading
`preview --details` and answering one yes/no for everything GEFAHRLOS. Research (2026-11-18, ncdu,
dua-cli, npkill, BleachBit, lazygit, clig.dev) shaped the layout: summary first, one level per screen,
details on demand, a separate review step listing only what will be deleted, confirmation scaled to
risk, a report afterwards.

## Goals

- `disk-tidy` without a command, on a terminal, opens the TUI.
- Overview → drill in → tick/untick → review → confirm → progress → report → fresh scan.
- GEFAHRLOS entries pre-selected; PRÜFEN entries selectable (never pre-selected); VORSICHT and
  NUR INFO visible but never selectable.
- Scan and deletion run in the background with visible progress; the UI stays responsive.

## Non-goals

- Deleting VORSICHT (data) or NUR INFO entries, in any mode.
- PRÜFEN through `clean` or plans: those stay GEFAHRLOS-only (phase 2 unchanged).
- Mouse support, themes, config of key bindings, saving a selection.
- Trash instead of permanent delete.

## Anti-goals

- A PRÜFEN entry is deleted without the user having ticked it and seen it in the review.
- The UI freezes during scan or deletion, or a sudo password prompt corrupts the screen.
- Text mode changes: `preview`, `clean`, `plan` and all their output stay byte-identical;
  `disk-tidy` without a terminal is the text preview as today.

## Approaches

1. **Threads and messages (chosen).** Ratatui + crossterm. App state is plain data with a pure
   `update(state, event)`; rendering is a function of state. Scan and deletion run in a worker thread
   and send events over `std::sync::mpsc`. Testable without a terminal; rendering checked with
   Ratatui's `TestBackend`.
2. Async with tokio: same result, large dependency, excluded by the phase-1 spec. Rejected.
3. TUI only for selecting, scan before with a spinner, deletion as plain text after leaving the TUI:
   simpler, but loses in-UI progress, report and rescan. Rejected.

## Entry point

- `disk-tidy` with no command, stdin and stdout both terminals, and none of `--details`, `--top`,
  `--only`, `--yes`, `--plan`, `--json`: TUI.
- Otherwise exactly as today (`disk-tidy` = text preview). `disk-tidy preview` always prints text.
- `--config` applies to the TUI as well.

## Screens and keys

Each level replaces the previous one and uses the full width (like ncdu). The header always shows
where you are (breadcrumb) and the selected total; the footer shows only the keys that work on that
screen; `?` shows all keys; `q` quits. While a deletion runs, `q` is refused with a note ("Löschen läuft –
bitte warten"); it works again on the report. *(builder's call)* `d` and `j` wait until the scan has
finished. *(builder's call)*

1. **Overview.** One row per partition × level that has entries: partition, level (symbol + word +
   colour), number of entries, size, selected size, "frei danach" (free space + selected, on the first
   row of each partition). While the scan runs, rows appear as modules finish; a line names the
   module currently scanning and any module that failed ("Docker nicht erreichbar").
   Keys: ↑↓, ⏎/→ open, ␣ toggles all selectable entries of the row, `d` review.
2. **Categories** of one partition × level, sorted by size: checkbox (☑ ☐ ◐), size, title, a grey
   hint column (entry count, oldest age, "nur als Ganzes"). Keys: ␣ toggle category, → entries,
   `i` details, ←/Esc back, `d` review.
3. **Entries** of one category, sorted by size: checkbox, size, age, label/path (long paths shortened
   in the middle). Keys: ␣ toggle, `a` all, `n` none, `i` details, ←/Esc back, `d` review.
4. **Details popup** (`i`, on categories or entries) over the current screen: level, why, size, the
   item's known facts (project and last change for build dirs, image/container info, deletion date
   for trash), the manual command. Esc closes.
5. **Review** (`d` from anywhere): only what will be deleted, grouped by partition with subtotal and
   "frei danach"; PRÜFEN entries highlighted with ⚠ and their cost; a note if sudo is needed.
   ␣ unticks; Esc back to where you were; ⏎ confirm. Empty selection: message, no confirm.
6. **Confirm popup.** "50.3 GB in 8 Kategorien." If PRÜFEN entries are selected, an extra line names
   them ("Davon ⚠ prüfen: 2 Images, 3.6 GB."). `j` deletes, Esc cancels.
7. **Progress and report.** First "Prüfe neu …" while the rescan runs. Then one line per category:
   pending, running, ✓ with size, ✗ with error, skipped (sudo); below, "nicht mehr gültig" entries
   with their reason. A bar with bytes freed so far. When done it stays until a key is pressed, then a
   fresh scan starts and the overview shows the new state.

Not selectable: VORSICHT and NUR INFO rows show no checkbox; ␣ does nothing there (footer says so).
Categories without IDs (Docker build cache, journal, apt cache) are all-or-nothing: entries are listed
with "·" instead of a checkbox and ␣ toggles the whole category.

Colour is never the only signal (symbol + word per level); `NO_COLOR` turns colour off.

## Selection model

- Initial state after each scan: every item of every GEFAHRLOS category selected, nothing else.
- Selection is per item, keyed by (category key, item id); a category is ☑/☐/◐ from its items.
- Deletion builds a `Selection` (phase 2 type) from the ticked items and runs the existing removal
  code. PRÜFEN categories need removers:
  - `unused_images`: `docker rmi <ids>` (per item ID).
  - `containers`: `docker rm <ids>`.
  - `build_active`: the existing build-dir remover (`Remover::BuildDirs`, with its re-checks).
  - `stopped_images`: no remover, not selectable (its container must go first; after deleting the
    container a rescan moves the image to `unused_images`). *(builder's call)*
- `clean` and `plan`/`select` keep acting on GEFAHRLOS only: giving a PRÜFEN category a remover must
  not make `clean` remove it (they filter by `is_clean`, which stays GEFAHRLOS-only; a test pins this).

## Background work

- **Scan:** a worker thread runs the modules in order and sends `ModuleDone(name, categories)` or
  `ModuleFailed(name, note)`; the "Hinweis" notes that today go to stderr are shown in the UI instead
  (stderr is the terminal under the TUI).
- **Deletion:** a worker thread runs the removal and sends `CategoryStarted/CategoryDone(result)`
  per category, then `Finished(Report)`. `clean` gains a per-category callback (or the loop is split
  out) so text mode and TUI share one implementation; remover text output is collected per category
  for the report instead of written to the terminal.
- **Recheck before deleting (decided: yes).** After `j`, the worker rescans the modules the
  selection touches and deletes an item only if the fresh scan still has it (same category key and
  id) in a category that is selectable now: GEFAHRLOS always; PRÜFEN only if the user ticked it as
  PRÜFEN (so a ticked GEFAHRLOS build dir whose project became active, now `build_active`, is not
  deleted). Everything else is dropped with reason `gone`, `not_safe` or `module_failed` and shown as
  "nicht mehr gültig" in the report. Id-less categories follow phase 2's rule (all current entries
  must be ticked). This generalises phase 2's `plan::select` to take the set of allowed levels per
  item; `clean --plan` keeps calling it with GEFAHRLOS only. The progress screen shows "Prüfe neu …"
  during the rescan.
- **sudo:** if the selection needs sudo and `sudo -n true` fails, the TUI leaves the alternate screen
  and raw mode, runs `sudo -v` (password prompt on the plain terminal), and returns. If that fails,
  sudo categories are reported as skipped with their commands, as in `clean`.
- External commands run with stdin null and output captured, so nothing writes into the TUI.

## Item facts for details and the age column

`Item` gains optional facts filled where a module knows them cheaply: a timestamp (`modified`:
project last change for build dirs, `CreatedAt` for images/containers, `LastUsedAt` for build cache,
`DeletionDate` for trash) and short key/value notes (e.g. "Projekt", "Image"). Missing facts show as
blank. These fields are not part of the plan JSON (version stays 1). *(builder's call: exact field
names and shapes)*

## Constraints

- Rust 1.99, edition 2024, no `unsafe`, no tokio. New dependencies: `ratatui` and `crossterm` only.
- German UI texts, matching the existing tone.
- Terminal is always restored (also on panic and on error): raw mode off, alternate screen left,
  cursor shown.
- Works in 80×24; narrower terminals shorten columns rather than break layout.

## Left to the builder

- Module layout (e.g. `src/tui/{app,view,worker}.rs`).
- How `clean`'s loop is shared (callback vs. extracted function), as long as text output stays
  byte-identical.
- Exact column widths, path shortening, age format ("vor 41 Tg.", "vor 5 Mon.").
- Tests: state transitions (navigation, toggling incl. ◐ and all-or-nothing categories, unselectable
  levels, review contents, confirm text with PRÜFEN), rendering snapshots with `TestBackend`,
  `clean` still ignoring PRÜFEN categories, entry-point rules.
