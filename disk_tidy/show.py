"""Large paths that are only displayed, never deleted (e.g. the Thunderbird profile)."""
import glob
import os

from .core import INFO, Category, Item, du

NAME = "show"


def collect(cfg):
    paths = []
    for pattern in cfg["show"]["paths"]:
        paths += sorted(glob.glob(os.path.expanduser(pattern)))
    if not paths:
        return [], None
    cat = Category("show", "Große Ordner zur Info", paths[0], INFO,
                   "z. B. Mail-Profile; wird nie gelöscht")
    sizes = du(paths)
    home = os.path.expanduser("~")
    cat.items = [Item(sizes[p], p, p.replace(home, "~", 1)) for p in paths]
    return [cat.sort()], None
