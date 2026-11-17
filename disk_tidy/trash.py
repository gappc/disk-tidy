"""The desktop trash (~/.local/share/Trash)."""
import shutil
from pathlib import Path

from .core import Category, Item, du

NAME = "trash"


def trash_dir():
    return Path.home() / ".local" / "share" / "Trash"


def _remove(items, base=None):
    base = base or trash_dir()
    ok = True
    for i in items:
        p = Path(i.ref)
        if p.parent != base / "files":
            print(f"  Übersprungen (nicht im Papierkorb): {p}")
            ok = False
            continue
        try:
            if p.is_dir() and not p.is_symlink():
                shutil.rmtree(p)
            else:
                p.unlink()
            (base / "info" / f"{p.name}.trashinfo").unlink(missing_ok=True)
        except OSError as e:
            print(f"  Fehler: {p}: {e}")
            ok = False
    return ok


def collect(cfg, base=None):
    base = base or trash_dir()
    files = base / "files"
    cat = Category("trash", "Papierkorb", str(base), clean=True,
                   remove=lambda items: _remove(items, base))
    if files.is_dir():
        entries = list(files.iterdir())
        sizes = du(entries)
        cat.items = [Item(sizes[str(e)], str(e), e.name) for e in entries]
    return [cat.sort()], None
