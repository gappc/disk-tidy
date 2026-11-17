import contextlib
import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from disk_tidy import cli, docker  # noqa: E402
from disk_tidy.core import CHECK, DATA, INFO, SAFE, SAFETY_ORDER, Category, Item  # noqa: E402
from tests.test_docker import CFG, DF  # noqa: E402


def cat(key, safety, size):
    return Category(key, f"Titel {key}", "/", safety, f"warum {key}", f"befehl {key}",
                    items=[Item(size, key, f"eintrag {key}")])


CATS = [cat("info", INFO, 4e9), cat("check", CHECK, 3e9), cat("safe", SAFE, 1e9),
        cat("data", DATA, 2e9)]


def render(*args, **kwargs):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        cli.preview(*args, **kwargs)
    return out.getvalue()


class PreviewTest(unittest.TestCase):
    def test_levels_in_order_each_category_under_its_level(self):
        text = render(CATS, details=False, top=5)
        heads = [text.index(h) for h in ("GEFAHRLOS", "PRÜFEN", "VORSICHT", "NUR INFO")]
        self.assertEqual(heads, sorted(heads))
        for key, level_head, next_head in [("safe", "GEFAHRLOS", "PRÜFEN"),
                                           ("check", "PRÜFEN", "VORSICHT"),
                                           ("data", "VORSICHT", "NUR INFO")]:
            pos = text.index(f"Titel {key}")
            self.assertTrue(text.index(level_head) < pos < text.index(next_head), key)

    def test_summary_counts_only_safe(self):
        text = render(CATS, details=False, top=5)
        self.assertIn("Gefahrlos freizugeben: / 1.0 GB", text)

    def test_entries_only_with_details(self):
        self.assertNotIn("eintrag safe", render(CATS, details=False, top=5))
        detailed = render(CATS, details=True, top=5)
        self.assertIn("eintrag safe", detailed)
        self.assertIn("Befehl: befehl check", detailed)


class SafetyTest(unittest.TestCase):
    def test_every_docker_category_has_level_and_reason(self):
        for c in docker.classify(DF, CFG):
            self.assertIn(c.safety, SAFETY_ORDER, c.key)
            self.assertTrue(c.why, c.key)
            self.assertEqual(c.clean, c.safety == SAFE)
            if c.clean:
                self.assertIsNotNone(c.remove, c.key)


if __name__ == "__main__":
    unittest.main()
