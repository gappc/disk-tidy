import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from disk_tidy import core  # noqa: E402
from disk_tidy.core import Category, Item  # noqa: E402


class SizeTest(unittest.TestCase):
    def test_parse_docker_units(self):
        self.assertEqual(core.parse_size("0B"), 0)
        self.assertEqual(core.parse_size("7.834kB"), 7834)
        self.assertEqual(core.parse_size("1.17GB"), 1.17e9)
        self.assertEqual(core.parse_size("2.36GB (virtual 3GB)"), 2.36e9)
        self.assertEqual(core.parse_size(""), 0)

    def test_parse_short_units(self):
        self.assertEqual(core.parse_size("500M"), 500e6)
        self.assertEqual(core.parse_size("3.7G"), 3.7e9)

    def test_format(self):
        self.assertEqual(core.fmt_size(1.6e9).strip(), "1.6 GB")
        self.assertEqual(core.fmt_size(512).strip(), "512 B")


class ConfigTest(unittest.TestCase):
    def test_missing_file_gives_defaults(self):
        self.assertEqual(core.load_config(Path("/nonexistent/x.toml")), core.DEFAULT_CONFIG)

    def test_file_overrides_per_key(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "c.toml"
            p.write_text('[docker]\ncache_volumes = ["x*"]\n[projects]\nstale_days = 7\n')
            cfg = core.load_config(p)
        self.assertEqual(cfg["docker"]["cache_volumes"], ["x*"])
        self.assertEqual(cfg["docker"]["protect_volumes"],
                         core.DEFAULT_CONFIG["docker"]["protect_volumes"])
        self.assertEqual(cfg["projects"]["stale_days"], 7)
        self.assertEqual(cfg["projects"]["roots"], core.DEFAULT_CONFIG["projects"]["roots"])

    def test_defaults_not_mutated(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "c.toml"
            p.write_text('[projects]\nstale_days = 1\n')
            core.load_config(p)
        self.assertEqual(core.DEFAULT_CONFIG["projects"]["stale_days"], 28)


class HelpersTest(unittest.TestCase):
    def test_mount_point_of_missing_path(self):
        self.assertEqual(core.mount_point("/proc/does/not/exist"), "/proc")

    def test_du(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "f"
            f.write_bytes(b"x" * 100_000)
            sizes = core.du([d, Path(d) / "missing"])
        self.assertGreaterEqual(sizes[d], 100_000)
        self.assertEqual(sizes[str(Path(d) / "missing")], 0)

    def test_category_sort_and_total(self):
        c = Category("k", "t", "/", items=[Item(1, "a", "a"), Item(3, "b", "b")]).sort()
        self.assertEqual([i.ref for i in c.items], ["b", "a"])
        self.assertEqual(c.total, 4)


if __name__ == "__main__":
    unittest.main()
