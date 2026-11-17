import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from disk_tidy import projects, system, trash  # noqa: E402
from disk_tidy.core import DEFAULT_CONFIG  # noqa: E402

DAY = 86400


def touch(path, age_days=0, content=b"x"):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    t = time.time() - age_days * DAY
    os.utime(path, (t, t))


class ProjectsTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, self.root)
        self.cfg = {"projects": {"roots": [str(self.root)], "stale_days": 28}}

        # old: untouched 60 days, git repo, node_modules + nested package
        touch(self.root / "old" / "package.json", 60)
        touch(self.root / "old" / ".git" / "HEAD", 60)
        touch(self.root / "old" / "node_modules" / "dep" / "index.js", 1)  # build dir age ignored
        touch(self.root / "old" / "node_modules" / "dep" / "node_modules" / "x.js", 1)
        # fresh: edited yesterday, Rust project
        touch(self.root / "fresh" / "Cargo.toml", 60)
        touch(self.root / "fresh" / "src" / "main.rs", 1)
        touch(self.root / "fresh" / "target" / "debug" / "bin", 60)
        # unmarked: a folder called target without Cargo.toml/pom.xml -> ignored
        touch(self.root / "docs" / "target" / "important.txt", 90)
        # monorepo: git root edited recently, package untouched -> still active
        touch(self.root / "mono" / ".git" / "HEAD", 60)
        touch(self.root / "mono" / "README.md", 2)
        touch(self.root / "mono" / "pkgs" / "a" / "package.json", 60)
        touch(self.root / "mono" / "pkgs" / "a" / "node_modules" / "y.js", 60)

    def collect(self):
        cats, note = projects.collect(self.cfg)
        self.assertIsNone(note)
        return {c.key: sorted(Path(i.ref).relative_to(self.root).as_posix() for i in c.items)
                for c in cats}

    def test_classification(self):
        got = self.collect()
        self.assertEqual(got["build_stale"], ["old/node_modules"])
        self.assertEqual(got["build_active"], ["fresh/target", "mono/pkgs/a/node_modules"])

    def test_remove_deletes_only_listed_build_dirs(self):
        cats, _ = projects.collect(self.cfg)
        stale = next(c for c in cats if c.key == "build_stale")
        self.assertTrue(stale.remove(stale.items))
        self.assertFalse((self.root / "old" / "node_modules").exists())
        self.assertTrue((self.root / "old" / "package.json").exists())
        self.assertTrue((self.root / "fresh" / "target").exists())

    def test_remove_refuses_non_build_dir(self):
        from disk_tidy.core import Item
        target = self.root / "docs"
        self.assertFalse(projects._remove([Item(0, str(target), "docs")]))
        self.assertTrue(target.exists())

    def test_no_roots(self):
        cfg = {"projects": {"roots": [str(self.root / "missing")], "stale_days": 28}}
        self.assertEqual(projects.collect(cfg), ([], None))


class SystemParseTest(unittest.TestCase):
    def test_disabled_snaps(self):
        out = ("Name    Version  Rev   Tracking       Publisher   Notes\n"
               "core22  2026     2987  latest/stable  canonical** base,disabled\n"
               "core22  2026     3129  latest/stable  canonical** base\n"
               "gimp    3.2      561   latest/stable  gimp**      disabled\n")
        self.assertEqual(system.parse_disabled_snaps(out), [("core22", "2987"), ("gimp", "561")])

    def test_journal_usage(self):
        text = "Archived and active journals take up 3.7G in the file system."
        self.assertEqual(system.parse_journal_usage(text), 3.7e9)
        self.assertEqual(system.parse_journal_usage("garbage"), 0)

    def test_snap_commands(self):
        from disk_tidy.core import Item
        cat = system.snaps()
        cmds = cat.sudo_cmds([Item(1, "gimp=561", "")])
        self.assertEqual(cmds, [["snap", "remove", "gimp", "--revision=561"]])

    def test_journal_command_uses_config(self):
        self.assertEqual(system.journal("1G").sudo_cmds([]),
                         [["journalctl", "--vacuum-size=1G"]])


class TrashTest(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, self.base)
        touch(self.base / "files" / "a.txt", content=b"x" * 5000)
        touch(self.base / "files" / "dir" / "b.txt")
        touch(self.base / "info" / "a.txt.trashinfo")
        touch(self.base / "info" / "dir.trashinfo")

    def test_collect_and_remove(self):
        cats, _ = trash.collect({}, base=self.base)
        self.assertEqual({Path(i.ref).name for i in cats[0].items}, {"a.txt", "dir"})
        self.assertTrue(cats[0].remove(cats[0].items))
        self.assertEqual(list((self.base / "files").iterdir()), [])
        self.assertEqual(list((self.base / "info").iterdir()), [])

    def test_refuses_path_outside_trash(self):
        from disk_tidy.core import Item
        outside = self.base / "keep.txt"
        touch(outside)
        self.assertFalse(trash._remove([Item(0, str(outside), "")], self.base))
        self.assertTrue(outside.exists())


class DefaultsTest(unittest.TestCase):
    def test_every_module_section_present(self):
        self.assertEqual(set(DEFAULT_CONFIG), {"docker", "projects", "system", "show"})


if __name__ == "__main__":
    unittest.main()
