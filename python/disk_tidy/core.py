"""Shared model and helpers: items, categories, sizes, partitions, commands, config."""
import os
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

CONFIG_PATH = Path.home() / ".config" / "disk-tidy" / "config.toml"

DEFAULT_CONFIG = {
    "docker": {
        # Named volumes that `clean` may remove when unused (fnmatch globs).
        "cache_volumes": [],
        # Never removed, even if matched by cache_volumes.
        "protect_volumes": ["*db*", "*data*"],
    },
    "projects": {
        "roots": ["~/projects"],
        # A project counts as untouched when no file outside its build dirs
        # changed for this many days.
        "stale_days": 28,
    },
    "system": {
        # journalctl --vacuum-size target.
        "journal_max": "500M",
    },
    "show": {
        # Large paths that are only displayed, never deleted (globs allowed).
        "paths": ["~/snap/*/common"],
    },
}

UNITS = {"B": 1, "K": 1e3, "KB": 1e3, "M": 1e6, "MB": 1e6,
         "G": 1e9, "GB": 1e9, "T": 1e12, "TB": 1e12}


def parse_size(s):
    """'1.17GB', '7.834kB', '3.7G', '500M' -> bytes. Unknown -> 0."""
    m = re.match(r"\s*([\d.]+)\s*([kKMGT]?B?)\b", s or "")
    if not m:
        return 0
    unit = m.group(2).upper() or "B"
    return float(m.group(1)) * UNITS[unit]


def fmt_size(n):
    if n < 1000:
        return f"{n:7.0f} B "
    for unit in ("kB", "MB", "GB", "TB"):
        n /= 1000
        if n < 1000 or unit == "TB":
            return f"{n:7.1f} {unit}"


def load_config(path=CONFIG_PATH):
    cfg = {k: dict(v) for k, v in DEFAULT_CONFIG.items()}
    if path.exists():
        with path.open("rb") as f:
            for section, values in tomllib.load(f).items():
                cfg.setdefault(section, {}).update(values)
    return cfg


@dataclass
class Item:
    size: float
    ref: str      # what the remover acts on: an ID, a name or a path
    label: str


# Safety levels, from harmless to never touched. Only SAFE is removed by `disk-tidy clean`.
SAFE = "safe"    # recoverable without loss: caches, leftovers, build output of idle projects
CHECK = "check"  # recoverable, but costs time or downloads; you decide
DATA = "data"    # may hold data that cannot be recovered
INFO = "info"    # displayed only, never removed
SAFETY_ORDER = [SAFE, CHECK, DATA, INFO]


@dataclass
class Category:
    key: str
    title: str
    path: str            # any path on the partition this category lives on
    safety: str = INFO
    why: str = ""        # one line: what deleting it costs
    hint: str = ""       # manual command, shown for categories that clean skips
    items: list = field(default_factory=list)
    # Non-sudo removal: gets the items, returns True on success.
    remove: Callable[[list], bool] | None = None
    # Sudo removal: gets the items, returns the commands to run (without "sudo").
    sudo_cmds: Callable[[list], list] | None = None

    @property
    def clean(self):
        """Removed by `disk-tidy clean`."""
        return self.safety == SAFE

    @property
    def total(self):
        return sum(i.size for i in self.items)

    def sort(self):
        self.items.sort(key=lambda i: i.size, reverse=True)
        return self


def mount_point(path):
    p = Path(path).expanduser()
    while not p.exists():
        p = p.parent
    p = p.resolve()
    while not os.path.ismount(p):
        p = p.parent
    return str(p)


def du(paths):
    """Allocated size in bytes for each path, like `du -s`. Unreadable parts are skipped."""
    paths = [str(p) for p in paths]
    if not paths:
        return {}
    r = subprocess.run(["du", "-s", "--block-size=1", "--", *paths],
                       capture_output=True, text=True)
    sizes = {}
    for line in r.stdout.splitlines():
        size, _, p = line.partition("\t")
        sizes[p] = int(size)
    return {p: sizes.get(p, 0) for p in paths}


def run(cmd):
    """Run a command, return True on success; errors are printed, not raised."""
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"  Fehler: {' '.join(cmd[:4])}...: {r.stderr.strip()}", file=sys.stderr)
    return r.returncode == 0


def can_sudo(interactive):
    """sudo works without a password, or there is a terminal to type one."""
    if shutil.which("sudo") is None:
        return False
    if subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode == 0:
        return True
    return interactive


def run_sudo(cmd):
    """Run with sudo; the password prompt, if any, goes to the terminal."""
    r = subprocess.run(["sudo", *cmd])
    return r.returncode == 0
