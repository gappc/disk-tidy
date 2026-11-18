# disk-tidy phase 2: automation

Decided 2026-11-18. Builds on phase 1 (`2026-11-17-disk-tidy-rust-design.md`, implemented on main).

## Why

disk-tidy is called by scripts and by Claude/agents, not only by a person at a terminal. They need
to read what is there in a stable format, decide (possibly after asking the user), and remove
exactly that, with a result they can parse and exit codes they can act on. Today `clean` without a
terminal prints a prompt, reads EOF as "no" and exits 1, which a script cannot tell apart from
"aborted on purpose"; and "done with errors" shares exit 2 with usage errors.

## Goals

- `disk-tidy plan` prints a machine-readable plan (JSON) of everything found.
- `disk-tidy clean --plan FILE` removes what the plan lists, without asking, but only what is
  **still GEFAHRLOS when it runs**.
- `clean` never prompts without a terminal.
- `clean --json` reports the result as JSON.
- Documented, distinct exit codes.

## Non-goals

- Removing PRÜFEN, VORSICHT or NUR-INFO entries through a plan. A plan can only narrow what `clean`
  would remove, never widen it (decided: option A). An `--include-check` mode may come later.
- Plan expiry, signatures, or locking against a concurrent run.
- JSON for `preview` (agents use `plan`, which carries the same information).
- The TUI (phase 3).

## Anti-goals (a failure even if it works)

- A stale or edited plan causes the removal of anything the current rules would not remove.
- Safety rules implemented twice (once in the modules, once in plan checking).
- A script hangs waiting for input.

## Approaches

1. **Rescan and intersect (chosen).** `clean --plan` runs the plan's modules again with the current
   config and removes only entries that are in the plan *and* in a GEFAHRLOS category of the fresh
   scan. The rules live only in the modules. Cost: applying takes as long as a scan (seconds).
2. Fingerprints in the plan (mtime, size) checked at apply time without rescanning. Faster, but the
   safety rules would need a second implementation, and some changes (a container starting to use a
   volume) do not show in a fingerprint. Rejected.

## Commands

```
disk-tidy plan [--only MOD,...] [--config PATH]          JSON plan on stdout
disk-tidy clean --plan FILE|- [--json] [--config PATH]   no prompt; FILE "-" = stdin
disk-tidy clean [--yes] [--json] [-d] [--top N] [--only ...]
```

- `plan` with `--details`, `--top`, `--yes` or `--json`, and `clean --plan` with `--only`, `--yes`,
  `--details` or `--top`, are usage errors (exit 2). `--json` with `preview` is a usage error.
- `clean` without `--plan` and without `--yes`, when stdin is not a terminal: no prompt; message on
  stderr ("Keine Rückfrage möglich: --yes oder --plan angeben."), nothing removed, exit 1.
  `clean --json` without `--plan`/`--yes` behaves the same even on a terminal.
- `clean` (any form) still prints "Nichts Sicheres zu löschen." / reports nothing and exits 0 when
  there is nothing to remove.

## Plan format (JSON, version 1)

```json
{
  "version": 1,
  "created": 1763459200,
  "modules": ["docker", "projects", "system", "trash", "show"],
  "partitions": [{"mount": "/home", "total": 0, "used": 0, "free": 0}],
  "categories": [
    {
      "key": "build_stale", "module": "projects", "title": "…", "safety": "safe",
      "why": "…", "hint": "…", "mount": "/home", "total": 123,
      "items": [{"id": "/home/u/p/target", "label": "…", "size": 123}]
    }
  ]
}
```

- `created`: Unix seconds. `modules`: the modules that were scanned (all, or `--only`).
- All categories of all levels are included, so an agent sees the full picture; sorted by level,
  then size, like the preview. Items are complete (no `--top` limit).
- `id` is what the item is matched on. Items whose path is not valid UTF-8 cannot be written to JSON
  without loss: they are left out of the plan and named in a note on stderr. They stay removable
  through interactive `clean`.
- `module` is new on `Category` (or derived where categories are collected). Remover details are not
  in the plan.
- Keys are English, texts stay German.
- To narrow a plan, a script or agent deletes items or whole categories. Any other edit (changing
  `safety`, adding items) has no effect beyond what the matching rules below allow.

## How `clean --plan` decides

1. Read and parse the plan; unsupported `version` or invalid JSON: message, exit 2.
2. Rescan the plan's `modules` with the current config.
3. For every GEFAHRLOS category of the rescan, keep the items whose `(category key, id)` is in the
   plan. Categories that are not GEFAHRLOS now are never touched, whatever the plan says.
4. Removers that take no IDs (`docker builder prune -a -f`, apt cache, journal vacuum) act on the
   whole category: they run only if **every** item of that category in the rescan is in the plan;
   otherwise the category is skipped and reported as such.
5. Only plan items that the plan itself marks `safe` are reported when they do not match (the
   plan's `safety` is used for this report only, never for deciding). Reasons: `not_safe` = the id
   now appears in a category that is not GEFAHRLOS, under any key (e.g. a build dir that moved from
   `build_stale` to `build_active`); `module_failed` = its module could not run in the rescan
   (`collect_all` returns which modules failed instead of only printing the note); `gone` = otherwise.
   PRÜFEN/VORSICHT/NUR-INFO items in the plan are ignored silently. *(builder's call)*
6. Removal then runs exactly as interactive `clean` does today (same removers, same re-checks of
   build dirs and trash entries, same sudo handling), using sizes from the rescan.
7. Without `--json`, the text output is the usual summary plus progress, with a short line for
   dropped entries.

## Result report (`clean --json`)

On stdout, always exactly one JSON object, also on abort and on errors after argument parsing (plan
file, config): then `"outcome": "error"`, `"exit_code": 2`, `"error": "<message>"`. Usage errors
found by clap itself stay plain text with exit 2. *(builder's call)* All human text (progress, notes, sudo output) goes to stderr.

```json
{
  "version": 1,
  "outcome": "done",
  "exit_code": 0,
  "freed": 123,
  "categories": [
    {"key": "trash", "status": "removed", "items": [{"id": "…", "size": 1}]}
  ],
  "dropped": [{"key": "build_stale", "id": "…", "reason": "gone"}],
  "skipped_sudo": ["sudo apt-get clean"]
}
```

- `status` per category: `removed`, `failed`, `skipped_sudo`, `skipped_partial` (rule 4).
  Failure is per category because one command removes all its IDs.
- `freed`: sum of item sizes of categories with status `removed` (an estimate, as the preview).
- `outcome`: `done`, `nothing`, `aborted`, `done_with_errors`, `done_skipped_sudo`, `done_partial`,
  `error`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | done, or nothing to remove |
| 1 | aborted: the user said no, or no confirmation was possible |
| 2 | usage, config or plan file error (clap's convention) |
| 3 | done, but some removals failed |
| 4 | done, but sudo commands were skipped (no failure) |
| 5 | done, but not everything in the plan was carried out (only `clean --plan`) |

Priority: 3 over 4 over 5. Code 5 applies when any plan item marked `safe` was dropped (`gone`,
`not_safe`, `module_failed`) or a category was `skipped_partial`; also when that leaves nothing to
remove (then nothing runs, exit 5, not 0).

Changes phase 1: "done with errors" was 2, is now 3. `preview` and `plan` exit 0, or
2 on usage/config errors.

## Docs

README gets a section "Automation" with the commands, the matching rules, both JSON formats and the
exit-code table. The phase 1 spec's phase list points here.

## Left to the builder

- Module structure (e.g. `plan.rs` for format and matching, report types in `clean.rs`).
- Whether `--plan` input is read fully before the rescan (it should be; stdin may be a pipe).
- Exact German wording of new messages, following the existing tone.
- Tests: matching rules (stale plan, edited safety, id-less removers, non-UTF-8 items), exit codes,
  no prompt without a TTY, JSON shape via golden files or serde round-trip.
