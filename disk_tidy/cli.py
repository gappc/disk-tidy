"""disk-tidy: see what fills the disk, sorted by size, and remove only the safe parts.

  disk-tidy [preview] [--details [--top N]] [--only MOD,...]   Overview per partition (read-only).
  disk-tidy clean [--yes] [--details] [--only MOD,...]         Remove the GEFAHRLOS part, after asking.

Modules: docker, projects, system, trash, show.  Config: ~/.config/disk-tidy/config.toml
"""
import argparse
import os
import shutil
import sys
from pathlib import Path

from . import docker, projects, show, system, trash
from .core import (CHECK, CONFIG_PATH, DATA, INFO, SAFE, SAFETY_ORDER, can_sudo, fmt_size,
                   load_config, mount_point, run_sudo)

MODULES = [docker, projects, system, trash, show]


def gather(cfg, only):
    cats = []
    for mod in MODULES:
        if only and mod.NAME not in only:
            continue
        found, note = mod.collect(cfg)
        if note:
            print(f"Hinweis ({mod.NAME}): {note}", file=sys.stderr)
        cats += [c for c in found if c.items]
    return cats


def by_partition(cats):
    groups = {}
    for c in cats:
        groups.setdefault(mount_point(c.path), []).append(c)
    return groups


# Heading per safety level: (symbol, title, explanation, colour)
LEVELS = {
    SAFE: ("✓", "GEFAHRLOS", "löscht `disk-tidy clean`", "32"),
    CHECK: ("⚠", "PRÜFEN", "wiederherstellbar, kostet aber Zeit oder Downloads; nur von Hand", "33"),
    DATA: ("✗", "VORSICHT", "kann Daten enthalten; nur von Hand", "31"),
    INFO: ("ℹ", "NUR INFO", "wird nie gelöscht", "2"),
}
TITLE_WIDTH = 44


def colour(text, code):
    if not sys.stdout.isatty() or os.environ.get("NO_COLOR"):
        return text
    return f"\033[{code}m{text}\033[0m"


def print_partition(mount):
    u = shutil.disk_usage(mount)
    print(colour(f"\n{mount}  ·  {fmt_size(u.free).strip()} frei von {fmt_size(u.total).strip()}"
                 f" ({u.used / (u.used + u.free):.0%} belegt)", "1"))


def print_level(level, group, details, top):
    cats = sorted((c for c in group if c.safety == level), key=lambda c: -c.total)
    if not cats:
        return
    symbol, title, explanation, code = LEVELS[level]
    total = sum(c.total for c in cats)
    print(colour(f"\n  {symbol} {title} – {explanation}", code + ";1")
          + colour(f"   {fmt_size(total).strip()}", code))
    for c in cats:
        print(f"    {fmt_size(c.total)}  {c.title:{TITLE_WIDTH}}  " + colour(c.why, "2"))
        if not details:
            continue
        if c.hint:
            print(colour(f"{'':15}Befehl: {c.hint}", "2"))
        for i in c.items[: top or None]:
            print(f"{'':15}{fmt_size(i.size)}  {i.label}")
        if top and len(c.items) > top:
            print(f"{'':15}... und {len(c.items) - top} weitere")


def preview(cats, details, top):
    groups = by_partition(cats)
    for mount, group in groups.items():
        print_partition(mount)
        for level in SAFETY_ORDER:
            print_level(level, group, details, top)
    safe = [f"{m} {fmt_size(sum(c.total for c in g if c.clean)).strip()}"
            for m, g in groups.items() if any(c.clean for c in g)]
    print()
    if safe:
        print(colour("Gefahrlos freizugeben: " + ", ".join(safe) + "  →  disk-tidy clean", "32;1"))
    if not details:
        print(colour("Einzelne Einträge und Befehle zum Löschen von Hand: disk-tidy --details", "2"))
    if details and any(c.title.startswith("Docker") for c in cats):
        print(colour("Docker-Images: nur der eigene Anteil (UniqueSize) zählt; geteilte Layer und "
                     "geteilter Build-Cache werden erst frei, wenn alle Nutzer weg sind.", "2"))


def clean(cats, assume_yes, details, top):
    todo = [c for c in cats if c.clean]
    if not todo:
        print("Nichts Sicheres zu löschen.")
        return 0
    for mount, group in by_partition(todo).items():
        print_partition(mount)
        print_level(SAFE, group, details, top)
    total = sum(c.total for c in todo)
    print(f"\nInsgesamt etwa {fmt_size(total).strip()}. Nur die Kategorien oben werden gelöscht"
          + ("." if details else "; einzelne Einträge: disk-tidy clean --details."))
    if any(c.sudo_cmds for c in todo):
        print("Die Kategorien mit (sudo) fragen gegebenenfalls nach deinem Passwort.")
    interactive = sys.stdin.isatty()
    if not assume_yes and input("Löschen? [j/N] ").strip().lower() not in ("j", "ja", "y", "yes"):
        print("Abgebrochen, nichts gelöscht.")
        return 1

    ok = True
    skipped = []
    sudo_ok = None
    for c in todo:
        print(f"→ {c.title}")
        if c.remove:
            ok &= c.remove(c.items)
        if c.sudo_cmds:
            if sudo_ok is None:
                sudo_ok = can_sudo(interactive)
            for cmd in c.sudo_cmds(c.items):
                if sudo_ok:
                    ok &= run_sudo(cmd)
                else:
                    skipped.append(cmd)

    if skipped:
        print("\nOhne sudo übersprungen. Selbst ausführen:")
        for cmd in skipped:
            print("  sudo " + " ".join(cmd))
    print("Fertig." if ok else "Fertig, mit Fehlern (siehe oben).")
    return 0 if ok else 2


def main(argv=None):
    p = argparse.ArgumentParser(prog="disk-tidy", description=__doc__.split("\n")[0])
    p.add_argument("command", nargs="?", default="preview", choices=["preview", "clean"])
    p.add_argument("--details", "-d", action="store_true",
                   help="einzelne Einträge und Befehle zum Löschen von Hand zeigen")
    p.add_argument("--top", type=int, default=15,
                   help="mit --details: Einträge pro Kategorie, 0 = alle")
    p.add_argument("--only", default="",
                   help="nur diese Module, kommagetrennt: " + ",".join(m.NAME for m in MODULES))
    p.add_argument("--yes", action="store_true", help="clean ohne Rückfrage")
    p.add_argument("--config", type=Path, default=CONFIG_PATH)
    args = p.parse_args(argv)

    only = {m.strip() for m in args.only.split(",") if m.strip()}
    unknown = only - {m.NAME for m in MODULES}
    if unknown:
        p.error(f"unbekannte Module: {', '.join(sorted(unknown))}")

    cats = gather(load_config(args.config), only)
    if args.command == "clean":
        return clean(cats, args.yes, args.details, args.top)
    preview(cats, args.details, args.top)
    return 0
