"""Docker: containers, images, build cache and volumes from `docker system df -v`."""
import fnmatch
import json
import re
import shutil
import subprocess

from .core import Category, Item, parse_size, run

NAME = "docker"
ANON_VOLUME = re.compile(r"^[0-9a-f]{64}$")


def _matches(name, patterns):
    return any(fnmatch.fnmatch(name, p) for p in patterns)


def _rm(*prefix):
    return lambda items: run([*prefix, *[i.ref for i in items]])


def classify(df, cfg, root="/var/lib/docker"):
    """Sort `docker system df -v` output into categories. cfg is the [docker] section."""
    def cat(key, title, clean=False, hint="", remove=None):
        return Category(key, title, root, clean, hint, remove=remove)

    cats = {c.key: c for c in [
        cat("dangling", "Docker: namenlose Images (dangling)", True,
            remove=_rm("docker", "rmi")),
        # Cache entries cannot be removed reliably by ID; this removes exactly the
        # entries that are not in use, which is what the preview lists.
        cat("build_cache", "Docker: Build-Cache, unbenutzt", True,
            remove=lambda items: run(["docker", "builder", "prune", "-a", "-f"])),
        cat("anon_volumes", "Docker: anonyme Volumes ohne Container", True,
            remove=_rm("docker", "volume", "rm")),
        cat("cache_volumes", "Docker: Cache-Volumes laut Konfiguration", True,
            remove=_rm("docker", "volume", "rm")),
        cat("containers", "Docker: gestoppte Container", hint="docker container prune"),
        cat("unused_images", "Docker: unbenutzte Images mit Namen", hint="docker image prune -a"),
        cat("stopped_images", "Docker: Images nur von gestoppten Containern benutzt",
            hint="docker rm <container> && docker rmi <image>"),
        cat("named_volumes", "Docker: benannte Volumes ohne Container  ⚠ evtl. Daten",
            hint="docker volume rm <name>"),
        cat("protected_volumes", "Docker: geschützte Volumes ohne Container (nie gelöscht)"),
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

    return [c.sort() for c in cats.values()]


def collect(cfg):
    if shutil.which("docker") is None:
        return [], "docker nicht installiert"
    try:
        root = subprocess.check_output(
            ["docker", "info", "--format", "{{.DockerRootDir}}"], text=True,
            stderr=subprocess.DEVNULL).strip() or "/var/lib/docker"
        df = json.loads(subprocess.check_output(
            ["docker", "system", "df", "-v", "--format", "json"], stderr=subprocess.DEVNULL))
    except (subprocess.CalledProcessError, json.JSONDecodeError):
        return [], "Docker-Daemon nicht erreichbar"
    return classify(df, cfg["docker"], root), None
