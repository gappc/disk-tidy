"""disk-tidy: see what fills the disk, sorted by size, and remove only the safe parts.

  disk-tidy [preview] [--top N] [--only MOD,...]   List everything per partition (read-only).
  disk-tidy clean [--yes] [--only MOD,...]         Remove the [clean] categories, after asking.

Modules: docker, projects, system, trash, show.  Config: ~/.config/disk-tidy/config.toml
"""
import argparse
import shutil
import sys
from pathlib import Path

from . import docker, projects, show, system, trash
from .core import CONFIG_PATH, can_sudo, fmt_size, load_config, mount_point, run_sudo

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
    for group in groups.values():
        group.sort(key=lambda c: (not c.clean, -c.total))
    return groups


def print_category(c, top):
    mark = ("[clean, sudo]" if c.sudo_cmds else "[clean]") if c.clean else "[manuell]"
    print(f"\n=== {mark} {c.title}: {len(c.items)} Einträge, {fmt_size(c.total).strip()} ===")
    if not c.clean and c.hint:
        print(f"    Befehl: {c.hint}")
    for i in c.items[: top or None]:
        print(f"  {fmt_size(i.size)}  {i.label}")
    if top and len(c.items) > top:
        print(f"  ... und {len(c.items) - top} weitere")


def print_partition(mount):
    u = shutil.disk_usage(mount)
    bar = "#" * 70
    print(f"\n{bar}\n# {mount}: {fmt_size(u.free).strip()} frei von {fmt_size(u.total).strip()}"
          f" ({u.used / (u.used + u.free):.0%} belegt)\n{bar}")


def preview(cats, top):
    for mount, group in by_partition(cats).items():
        print_partition(mount)
        for c in group:
            print_category(c, top)
        cleanable = sum(c.total for c in group if c.clean)
        print(f"\n→ `disk-tidy clean` würde auf {mount} etwa {fmt_size(cleanable).strip()} freigeben.")
    if any(c.title.startswith("Docker") for c in cats):
        print("\nDocker-Images: nur der eigene Anteil (UniqueSize) zählt; geteilte Layer und "
              "geteilter Build-Cache werden erst frei, wenn alle Nutzer weg sind.")


def clean(cats, assume_yes):
    todo = [c for c in cats if c.clean]
    if not todo:
        print("Nichts Sicheres zu löschen.")
        return 0
    for mount, group in by_partition(todo).items():
        print_partition(mount)
        for c in group:
            print_category(c, 10)
    total = sum(c.total for c in todo)
    print(f"\nInsgesamt etwa {fmt_size(total).strip()}.")
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
    p.add_argument("--top", type=int, default=15, help="Einträge pro Kategorie, 0 = alle")
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
        return clean(cats, args.yes)
    preview(cats, args.top)
    return 0
