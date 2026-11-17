"""System (needs sudo to clean): disabled snap revisions, journal, apt cache, crash reports."""
import glob
import os
import re
import shutil
import subprocess

from .core import SAFE, Category, Item, parse_size

NAME = "system"
SNAP_DIR = "/var/lib/snapd/snaps"


def parse_disabled_snaps(snap_list):
    """`snap list --all` output -> [(name, rev)] of disabled revisions."""
    out = []
    for line in snap_list.splitlines()[1:]:
        cols = line.split()
        if len(cols) >= 3 and "disabled" in cols[-1].split(","):
            out.append((cols[0], cols[2]))
    return out


def parse_journal_usage(text):
    """'Archived and active journals take up 3.7G in the file system.' -> bytes."""
    m = re.search(r"take up ([\d.]+\s*[KMGT]?B?)", text)
    return parse_size(m.group(1)) if m else 0


def _file_size(path):
    try:
        return os.stat(path).st_blocks * 512
    except OSError:
        return 0


def _output(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True).stdout
    except FileNotFoundError:
        return ""


def snaps():
    cat = Category("snap_revisions", "System: alte Snap-Revisionen (sudo)", SNAP_DIR, SAFE,
                   "ältere Versionen, die Snap nur als Rückfall aufhebt",
                   sudo_cmds=lambda items: [["snap", "remove", n, f"--revision={r}"]
                                            for n, r in (i.ref.split("=") for i in items)])
    if shutil.which("snap"):
        for name, rev in parse_disabled_snaps(_output(["snap", "list", "--all"])):
            size = _file_size(f"{SNAP_DIR}/{name}_{rev}.snap")
            cat.items.append(Item(size, f"{name}={rev}", f"{name} (Revision {rev})"))
    return cat


def journal(max_size):
    cat = Category("journal", f"System: Journal über {max_size} (sudo)", "/var/log/journal", SAFE,
                   f"alte System-Logs; die neuesten {max_size} bleiben",
                   sudo_cmds=lambda items: [["journalctl", f"--vacuum-size={max_size}"]])
    usage = parse_journal_usage(_output(["journalctl", "--disk-usage"]))
    excess = usage - parse_size(max_size)
    if excess > 0:
        cat.items.append(Item(excess, "journal", f"systemd-Journal, belegt insgesamt "
                                                 f"{usage / 1e9:.1f} GB"))
    return cat


def apt_cache():
    cat = Category("apt_cache", "System: apt-Paketcache (sudo)", "/var/cache/apt", SAFE,
                   "bereits installierte .deb-Pakete, werden bei Bedarf neu geladen",
                   sudo_cmds=lambda items: [["apt-get", "clean"]])
    debs = glob.glob("/var/cache/apt/archives/*.deb")
    if debs:
        size = sum(_file_size(d) for d in debs)
        cat.items.append(Item(size, "apt", f"{len(debs)} heruntergeladene .deb-Pakete"))
    return cat


def crashes():
    cat = Category("crash", "System: Absturzberichte (sudo)", "/var/crash", SAFE,
                   "alte Berichte abgestürzter Programme",
                   sudo_cmds=lambda items: [["rm", "-f", "--", *[i.ref for i in items]]])
    for f in glob.glob("/var/crash/*.crash"):
        cat.items.append(Item(_file_size(f), f, os.path.basename(f)))
    return cat


def collect(cfg):
    cats = [snaps(), journal(cfg["system"]["journal_max"]), apt_cache(), crashes()]
    return [c.sort() for c in cats], None
