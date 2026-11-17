import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import docker_tidy as dt  # noqa: E402

ANON = "a" * 64


def image(id_, repo="<none>", tag="<none>", containers="0", unique="100MB"):
    return {"ID": "sha256:" + id_ * 64, "Repository": repo, "Tag": tag,
            "Containers": containers, "UniqueSize": unique, "CreatedSince": "1 day ago"}


def container(name, image_, state="exited", size="10MB"):
    return {"ID": name * 8, "Names": name, "Image": image_, "State": state,
            "Status": "Exited (0)", "Size": size}


def volume(name, links="0", size="1GB"):
    return {"Name": name, "Links": links, "Size": size}


def cache(id_, in_use="false", size="200MB", shared="false"):
    return {"ID": id_, "InUse": in_use, "Size": size, "Shared": shared,
            "LastUsedSince": "1 week ago", "CacheType": "regular", "Description": "RUN x"}


DF = {
    "Containers": [container("web", "app:latest", state="running"),
                   container("old", "old:1", size="2GB")],
    "Images": [image("1", unique="300MB"),                             # dangling
               image("2", repo="node", tag="22", unique="1.6GB"),       # unused named
               image("3", repo="old", tag="1", containers="1"),         # only stopped
               image("4", repo="app", tag="latest", containers="1")],   # running
    "Volumes": [volume(ANON, size="500MB"),
                volume("b" * 64, links="1"),                            # in use
                volume("audit-gobuild", size="3.4GB"),
                volume("proj_dbdata"),
                volume("gobuild_dbdata"),                               # cache AND protected
                volume("random-named")],
    "BuildCache": [cache("c1"), cache("c2", in_use="true"), cache("c3", size="1.5GB")],
}

CFG = {"cache_volumes": ["*gobuild*"], "protect_volumes": ["*dbdata*"]}


class SizeTest(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(dt.parse_size("0B"), 0)
        self.assertEqual(dt.parse_size("7.834kB"), 7834)
        self.assertEqual(dt.parse_size("1.17GB"), 1.17e9)
        self.assertEqual(dt.parse_size("2.36GB (virtual 3GB)"), 2.36e9)
        self.assertEqual(dt.parse_size(""), 0)

    def test_format(self):
        self.assertEqual(dt.fmt_size(1.6e9).strip(), "1.6 GB")
        self.assertEqual(dt.fmt_size(512).strip(), "512 B")


class ClassifyTest(unittest.TestCase):
    def setUp(self):
        self.cats = {c.key: c for c in dt.classify(DF, CFG)}

    def ids(self, key):
        return sorted(i.ref for i in self.cats[key].items)

    def test_stopped_containers(self):
        self.assertEqual(self.ids("containers"), ["old" * 8])

    def test_images(self):
        self.assertEqual(self.ids("dangling"), ["sha256:" + "1" * 64])
        self.assertEqual(self.ids("unused_images"), ["node:22"])
        self.assertEqual(self.ids("stopped_images"), ["old:1"])

    def test_build_cache_excludes_in_use(self):
        self.assertEqual(self.ids("build_cache"), ["c1", "c3"])

    def test_volumes(self):
        self.assertEqual(self.ids("anon_volumes"), [ANON])
        self.assertEqual(self.ids("cache_volumes"), ["audit-gobuild"])
        self.assertEqual(self.ids("named_volumes"), ["random-named"])
        self.assertEqual(self.ids("protected_volumes"), ["gobuild_dbdata", "proj_dbdata"])

    def test_items_sorted_by_size(self):
        sizes = [i.size for i in self.cats["build_cache"].items]
        self.assertEqual(sizes, sorted(sizes, reverse=True))

    def test_only_safe_categories_are_cleaned(self):
        cleaned = {c.key for c in self.cats.values() if c.clean}
        self.assertEqual(cleaned, {"dangling", "build_cache", "anon_volumes", "cache_volumes"})

    def test_default_config_cleans_no_named_volumes(self):
        cats = {c.key: c for c in dt.classify(DF, dt.DEFAULT_CONFIG)}
        self.assertEqual(cats["cache_volumes"].items, [])
        self.assertIn("audit-gobuild", [i.ref for i in cats["named_volumes"].items])


class ConfigTest(unittest.TestCase):
    def test_missing_file_gives_defaults(self):
        self.assertEqual(dt.load_config(Path("/nonexistent/x.toml")), dt.DEFAULT_CONFIG)

    def test_file_overrides(self):
        p = Path(self.tmp()) / "c.toml"
        p.write_text('cache_volumes = ["x*"]\n')
        cfg = dt.load_config(p)
        self.assertEqual(cfg["cache_volumes"], ["x*"])
        self.assertEqual(cfg["protect_volumes"], dt.DEFAULT_CONFIG["protect_volumes"])

    def tmp(self):
        import tempfile
        d = tempfile.mkdtemp()
        self.addCleanup(__import__("shutil").rmtree, d)
        return d


if __name__ == "__main__":
    unittest.main()
