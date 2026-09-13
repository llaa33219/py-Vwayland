"""Key/button mapping unit tests (no compositor needed)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "src"))

from vwayland.errors import VwaylandError
from vwayland.keys import SHIFT_CHARS, KEYS, resolve_button, resolve_key


class TestKeys(unittest.TestCase):
    def test_common_names(self) -> None:
        self.assertEqual(resolve_key("enter"), 28)
        self.assertEqual(resolve_key("a"), 30)
        self.assertEqual(resolve_key("A".lower()), 30)
        self.assertEqual(resolve_key("esc"), 1)
        self.assertEqual(resolve_key("f5"), 63)
        self.assertEqual(resolve_key("ctrl"), 29)
        self.assertEqual(resolve_key("shift"), 42)
        self.assertEqual(resolve_key("up"), 103)

    def test_space_not_stripped(self) -> None:
        self.assertEqual(resolve_key(" "), 57)

    def test_int_passthrough(self) -> None:
        self.assertEqual(resolve_key(30), 30)

    def test_key_prefix(self) -> None:
        self.assertEqual(resolve_key("KEY_ENTER"), 28)

    def test_unknown(self) -> None:
        with self.assertRaises(VwaylandError):
            resolve_key("not-a-key")

    def test_buttons(self) -> None:
        self.assertEqual(resolve_button("left"), 0x110)
        self.assertEqual(resolve_button("right"), 0x111)
        self.assertEqual(resolve_button("middle"), 0x112)
        self.assertEqual(resolve_button(0x110), 0x110)
        with self.assertRaises(VwaylandError):
            resolve_button("nope")

    def test_shift_chars_have_base_keys(self) -> None:
        for ch, base in SHIFT_CHARS.items():
            self.assertIn(base, KEYS, f"shift char {ch!r} -> {base!r} not in KEYS")


if __name__ == "__main__":
    unittest.main()
