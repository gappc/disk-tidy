"""Build directories in project trees (node_modules, target, .venv)."""
import os
import shutil
import time
from pathlib import Path

from .core import CHECK, SAFE, Category, Item, du

NAME = "projects"

# Build dir name -> files of which at least one must sit next to it. Without such a
# file the directory is not treated as build output (a folder that happens to be
# called "target" stays untouched).
BUILD_DIRS = {
    "node_modules": ["package.json"],
    "target": ["Cargo.toml", "pom.xml"],
    ".venv": ["pyproject.toml", "requirements.txt", "setup.py", "uv.lock", "Pipfile"],
}
SKIP = {".git"}


def find_build_dirs(root):
    """Yield build dirs below root. Does not descend into them or follow symlinks."""
    for dirpath, dirnames, _ in os.walk(root):
        keep = []
        for d in dirnames:
            full = Path(dirpath) / d
            if d in SKIP or full.is_symlink():
                continue
            if d in BUILD_DIRS:
                if any((Path(dirpath) / m).exists() for m in BUILD_DIRS[d]):
                    yield full
                continue  # never descend into build dirs, marked or not
            keep.append(d)
        dirnames[:] = keep


def project_of(build_dir, root):
    """The project a build dir belongs to: the nearest git repo up to root, else its parent."""
    root = Path(root)
    p = build_dir.parent
    while True:
        if (p / ".git").exists():
            return p
        if p == root or p.parent == p:
            return build_dir.parent
        p = p.parent


def last_change(project, newer_than=None):
    """Newest mtime of anything in project outside build dirs and .git internals.
    Stops early once something newer than `newer_than` is found."""
    newest = 0.0
    for f in (project / ".git" / "index", project / ".git" / "HEAD"):
        if f.exists():
            newest = max(newest, f.stat().st_mtime)
    for dirpath, dirnames, filenames in os.walk(project):
        dirnames[:] = [d for d in dirnames if d not in SKIP and d not in BUILD_DIRS
                       and not os.path.islink(os.path.join(dirpath, d))]
        for name in filenames:
            try:
                newest = max(newest, os.lstat(os.path.join(dirpath, name)).st_mtime)
            except OSError:
                continue
        if newer_than is not None and newest > newer_than:
            break
    return newest


def _remove(items):
    ok = True
    for i in items:
        p = Path(i.ref)
        if p.is_symlink() or not p.is_dir() or p.name not in BUILD_DIRS:
            print(f"  Übersprungen (kein Build-Ordner): {p}")
            ok = False
            continue
        try:
            shutil.rmtree(p)
        except OSError as e:
            print(f"  Fehler: {p}: {e}")
            ok = False
    return ok


def collect(cfg, now=None):
    pc = cfg["projects"]
    now = time.time() if now is None else now
    cutoff = now - pc["stale_days"] * 86400
    stale = Category("build_stale", f"Build-Ordner, Projekt ≥{pc['stale_days']} Tage unberührt",
                     "~", SAFE, "mit npm install / cargo build / uv sync wiederherstellbar",
                     remove=_remove)
    active = Category("build_active", "Build-Ordner in aktiven Projekten", "~", CHECK,
                      "du arbeitest daran; danach neu installieren oder bauen",
                      "rm -rf <ordner>  (oder cargo clean)")

    found = []
    for root in pc["roots"]:
        root = Path(root).expanduser()
        if root.is_dir():
            found += [(b, project_of(b, root)) for b in find_build_dirs(root)]
    if not found:
        return [], None
    stale.path = active.path = str(found[0][0])

    sizes = du([b for b, _ in found])
    changed = {}
    for b, proj in found:
        if proj not in changed:
            changed[proj] = last_change(proj, newer_than=cutoff)
        days = int((now - changed[proj]) / 86400)
        try:
            label = f"~/{b.relative_to(Path.home())}"
        except ValueError:
            label = str(b)
        item = Item(sizes[str(b)], str(b), f"{label:72}  zuletzt geändert vor {days} Tagen")
        (stale if changed[proj] <= cutoff else active).items.append(item)
    return [stale.sort(), active.sort()], None
