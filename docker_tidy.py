#!/usr/bin/env python3
"""docker-tidy: see what Docker is using, sorted by size, and remove only the safe parts.

  docker-tidy [preview] [--top N]   List what could be removed (read-only). N=0 lists all.
  docker-tidy clean [--yes]         Remove the categories marked [clean], after asking.

Config: ~/.config/docker-tidy/config.toml
  cache_volumes   = ["*gobuild*"]  named volumes that `clean` may remove when unused
  protect_volumes = ["*db*"]       never removed, even if matched by cache_volumes
"""
import argparse
import fnmatch
import json
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

CONFIG_PATH = Path.home() / ".config" / "docker-tidy" / "config.toml"
DEFAULT_CONFIG = {"cache_volumes": [], "protect_volumes": ["*db*", "*data*"]}

UNITS = {"B": 1, "kB": 1e3, "KB": 1e3, "MB": 1e6, "GB": 1e9, "TB": 1e12}
ANON_VOLUME = re.compile(r"^[0-9a-f]{64}$")


def parse_size(s):
    m = re.match(r"\s*([\d.]+)\s*([kKMGT]?B)", s or "")
    return float(m.group(1)) * UNITS[m.group(2)] if m else 0


def fmt_size(n):
    if n < 1000:
        return f"{n:7.0f} B "
    for unit in ("kB", "MB", "GB", "TB"):
        n /= 1000
        if n < 1000 or unit == "TB":
            return f"{n:7.1f} {unit}"


def load_config(path=CONFIG_PATH):
    cfg = dict(DEFAULT_CONFIG)
    if path.exists():
        with path.open("rb") as f:
            cfg.update(tomllib.load(f))
    return cfg


@dataclass
class Item:
    size: float
    ref: str      # what gets passed to docker rm/rmi/volume rm
    label: str


@dataclass
class Category:
    key: str
    title: str
    hint: str     # manual command, shown for categories that `clean` skips
    clean: bool   # removed by `docker-tidy clean`
    items: list = field(default_factory=list)

    @property
    def total(self):
        return sum(i.size for i in self.items)


def _matches(name, patterns):
    return any(fnmatch.fnmatch(name, p) for p in patterns)


def classify(df, cfg):
    cats = {c.key: c for c in [
        Category("dangling", "Namenlose Images (dangling)", "docker image prune", True),
        Category("build_cache", "Build-Cache, unbenutzt", "docker builder prune -a", True),
        Category("anon_volumes", "Anonyme Volumes ohne Container", "docker volume prune", True),
        Category("cache_volumes", "Cache-Volumes laut Konfiguration", "docker volume rm", True),
        Category("containers", "Gestoppte Container", "docker container prune", False),
        Category("unused_images", "Unbenutzte Images mit Namen", "docker image prune -a", False),
        Category("stopped_images", "Images nur von gestoppten Containern benutzt",
                 "docker rm <container> && docker rmi <image>", False),
        Category("named_volumes", "Benannte Volumes ohne Container  ⚠ evtl. Daten",
                 "docker volume rm <name>", False),
        Category("protected_volumes", "Geschützte Volumes ohne Container (nie gelöscht)",
                 "", False),
    ]}

    containers = df.get("Containers") or []
    running = {c["Image"] for c in containers if c.get("State") == "running"}
    for c in containers:
        if c.get("State") != "running":
            cats["containers"].items.append(Item(
                parse_size(c.get("Size")), c["ID"],
                f"{c['Names']:40}  {c['Image']:35}  {c.get('Status', '')}"))

    for i in df.get("Images") or []:
        repo, tag = i["Repository"], i["Tag"]
        named = repo != "<none>"
        ref = f"{repo}:{tag}" if named and tag != "<none>" else i["ID"]
        item = Item(parse_size(i["UniqueSize"]), ref,
                    f"{ref if named else i['ID'][7:19]:60}  {i['CreatedSince']}")
        if int(i["Containers"]) > 0:
            if not ({repo, ref, i["ID"], i["ID"][7:19]} & running):
                cats["stopped_images"].items.append(item)
        elif named:
            cats["unused_images"].items.append(item)
        else:
            cats["dangling"].items.append(item)

    for b in df.get("BuildCache") or []:
        if b["InUse"] != "true":
            shared = "geteilt " if b["Shared"] == "true" else ""
            cats["build_cache"].items.append(Item(
                parse_size(b["Size"]), b["ID"],
                f"{b['LastUsedSince']:15}  {shared}{b['Description'][:70]}"))

    for v in df.get("Volumes") or []:
        if int(v["Links"]) > 0:
            continue
        name = v["Name"]
        item = Item(parse_size(v["Size"]), name, name)
        if ANON_VOLUME.match(name):
            key = "anon_volumes"
        elif _matches(name, cfg["protect_volumes"]):
            key = "protected_volumes"
        elif _matches(name, cfg["cache_volumes"]):
            key = "cache_volumes"
        else:
            key = "named_volumes"
        cats[key].items.append(item)

    for c in cats.values():
        c.items.sort(key=lambda i: i.size, reverse=True)
    return list(cats.values())


def docker_df():
    out = subprocess.check_output(["docker", "system", "df", "-v", "--format", "json"])
    return json.loads(out)


def print_category(c, top):
    mark = "[clean]" if c.clean else "[manuell]"
    print(f"\n=== {mark} {c.title}: {len(c.items)} Einträge, {fmt_size(c.total).strip()} ===")
    if not c.clean and c.hint:
        print(f"    Befehl: {c.hint}")
    for i in c.items[: top or None]:
        print(f"  {fmt_size(i.size)}  {i.label}")
    if top and len(c.items) > top:
        print(f"  ... und {len(c.items) - top} weitere")


def preview(cats, top):
    for c in cats:
        if c.items:
            print_category(c, top)
    cleanable = sum(c.total for c in cats if c.clean)
    print(f"\n`docker-tidy clean` würde etwa {fmt_size(cleanable).strip()} freigeben.")
    print("Images: nur der eigene Anteil (UniqueSize) zählt; geteilte Layer werden erst frei, "
          "wenn alle Nutzer weg sind. Geteilter Build-Cache ebenso.")


def run(cmd):
    """Run a docker command, return True on success; errors are printed, not raised."""
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"  Fehler: {' '.join(cmd[:4])}...: {r.stderr.strip()}", file=sys.stderr)
    return r.returncode == 0


def clean(cats, assume_yes):
    todo = [c for c in cats if c.clean and c.items]
    if not todo:
        print("Nichts Sicheres zu löschen.")
        return 0
    for c in todo:
        print_category(c, 10)
    total = sum(c.total for c in todo)
    print(f"\nInsgesamt etwa {fmt_size(total).strip()}.")
    if not assume_yes and input("Löschen? [j/N] ").strip().lower() not in ("j", "ja", "y", "yes"):
        print("Abgebrochen, nichts gelöscht.")
        return 1

    by_key = {c.key: c for c in todo}
    ok = True
    if "dangling" in by_key:
        ok &= run(["docker", "rmi", *[i.ref for i in by_key["dangling"].items]])
    if "build_cache" in by_key:
        # Cache entries cannot be removed reliably by ID; this removes exactly the
        # entries that are not in use, which is what the preview listed.
        ok &= run(["docker", "builder", "prune", "-a", "-f"])
    for key in ("anon_volumes", "cache_volumes"):
        if key in by_key:
            ok &= run(["docker", "volume", "rm", *[i.ref for i in by_key[key].items]])

    print("Fertig." if ok else "Fertig, mit Fehlern (siehe oben).")
    return 0 if ok else 2


def main(argv=None):
    p = argparse.ArgumentParser(prog="docker-tidy", description=__doc__.split("\n")[0])
    p.add_argument("command", nargs="?", default="preview", choices=["preview", "clean"])
    p.add_argument("--top", type=int, default=25, help="Einträge pro Kategorie, 0 = alle")
    p.add_argument("--yes", action="store_true", help="clean ohne Rückfrage")
    p.add_argument("--config", type=Path, default=CONFIG_PATH)
    args = p.parse_args(argv)

    cats = classify(docker_df(), load_config(args.config))
    if args.command == "clean":
        return clean(cats, args.yes)
    preview(cats, args.top)
    return 0


if __name__ == "__main__":
    sys.exit(main())
