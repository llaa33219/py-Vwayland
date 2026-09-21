"""py-Vwayland E2E tests.

How to run:
    python -m unittest discover -s tests -v

Requirements:
- A vwayland-compositor binary (auto-detected: $VWAYLAND_COMPOSITOR_BIN →
  src/vwayland/_native/ → rust/target/debug|release). All tests are skipped if
  none is found.
- App-related tests need the vwayland-test-client binary (rust/target/...).
- No display server is required (only headless mode is tested).
"""

from __future__ import annotations

import os
import shutil
import struct
import sys
import tempfile
import time
import unittest
import zlib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "src"))


def _find_compositor() -> "Path | None":
    env = os.environ.get("VWAYLAND_COMPOSITOR_BIN")
    candidates = [
        Path(env) if env else None,
        REPO_ROOT / "src" / "vwayland" / "_native" / "vwayland-compositor",
        REPO_ROOT / "rust" / "target" / "release" / "vwayland-compositor",
        REPO_ROOT / "rust" / "target" / "debug" / "vwayland-compositor",
    ]
    for c in candidates:
        if c and c.is_file() and os.access(c, os.X_OK):
            return c
    return None


def _find_test_client() -> "Path | None":
    for profile in ("release", "debug"):
        p = REPO_ROOT / "rust" / "target" / profile / "vwayland-test-client"
        if p.is_file() and os.access(p, os.X_OK):
            return p
    return None


COMPOSITOR = _find_compositor()
TEST_CLIENT = _find_test_client()


def read_png_center(path_or_bytes) -> "tuple[int, int, tuple[int, int, int, int]]":
    """(width, height, center RGBA) — minimal PNG decoder for tests."""
    data = (
        path_or_bytes
        if isinstance(path_or_bytes, bytes)
        else Path(path_or_bytes).read_bytes()
    )
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos = 8
    w = h = None
    idat = b""
    while pos < len(data):
        (ln,) = struct.unpack(">I", data[pos : pos + 4])
        typ = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + ln]
        if typ == b"IHDR":
            w, h, _bd, _ct = struct.unpack(">IIBB", chunk[:10])
        elif typ == b"IDAT":
            idat += chunk
        pos += 12 + ln
    raw = zlib.decompress(idat)
    stride = w * 4
    prev = bytearray(stride)
    out = bytearray()
    i = 0
    for _y in range(h):
        f = raw[i]
        i += 1
        line = bytearray(raw[i : i + stride])
        i += stride
        if f == 1:
            for x in range(4, stride):
                line[x] = (line[x] + line[x - 4]) & 0xFF
        elif f == 2:
            for x in range(stride):
                line[x] = (line[x] + prev[x]) & 0xFF
        elif f == 3:
            for x in range(stride):
                line[x] = (line[x] + ((line[x - 4] if x >= 4 else 0) + prev[x]) // 2) & 0xFF
        elif f == 4:
            for x in range(stride):
                a = line[x - 4] if x >= 4 else 0
                b = prev[x]
                c = prev[x - 4] if x >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 0xFF
        out += line
        prev = line
    o = ((h // 2) * w + w // 2) * 4
    return w, h, tuple(out[o : o + 4])


@unittest.skipIf(COMPOSITOR is None, "vwayland-compositor binary not found")
class VwaylandE2E(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="vwayland-test-")
        self._old_env = {
            k: os.environ.get(k)
            for k in ("VWAYLAND_RUNTIME_DIR", "VWAYLAND_COMPOSITOR_BIN")
        }
        os.environ["VWAYLAND_RUNTIME_DIR"] = self._tmp.name
        os.environ["VWAYLAND_COMPOSITOR_BIN"] = str(COMPOSITOR)
        import vwayland

        self.vw = vwayland

    def tearDown(self) -> None:
        for info in self.vw.list():
            try:
                self.vw.connect(info.id).kill()
            except Exception:
                pass
        for k, v in self._old_env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        self._tmp.cleanup()

    def test_spawn_ping_resize_kill(self) -> None:
        comp = self.vw.spawn(width=800, height=600, headless=True)
        info = comp.info()
        self.assertEqual(info["width"], 800)
        self.assertEqual(info["height"], 600)
        self.assertTrue(info["headless"])
        self.assertIsNone(info["app_pid"])

        self.assertEqual(comp.resize(1024, 768), (1024, 768))
        self.assertEqual(comp.info()["width"], 1024)

        listed = {i.id: i for i in self.vw.list()}
        self.assertIn(comp.id, listed)
        self.assertEqual(listed[comp.id].width, 1024)

        comp.kill()
        self.assertNotIn(comp.id, {i.id for i in self.vw.list()})
        self.assertFalse(comp.runtime_dir.exists())

    def test_screenshot_blank(self) -> None:
        with self.vw.spawn(width=320, height=240, headless=True) as comp:
            img = comp.screenshot()
            self.assertEqual((img.width, img.height), (320, 240))
            self.assertTrue(img.png_bytes.startswith(b"\x89PNG"))
            w, h, center = read_png_center(img.png_bytes)
            self.assertEqual((w, h), (320, 240))
            self.assertEqual(center, (0, 0, 0, 255))  # a blank screen is black

    @unittest.skipIf(TEST_CLIENT is None, "vwayland-test-client binary not found")
    def test_launch_and_screenshot_color(self) -> None:
        with self.vw.spawn(width=400, height=300, headless=True) as comp:
            pid = comp.launch([str(TEST_CLIENT), "00ff00"])
            self.assertGreater(pid, 0)
            deadline = time.monotonic() + 10
            center = None
            while time.monotonic() < deadline:
                _w, _h, center = read_png_center(comp.screenshot().png_bytes)
                if center[1] > 200:  # wait until the green app shows up
                    break
                time.sleep(0.3)
            self.assertIsNotNone(center)
            self.assertEqual(center[:3], (0, 255, 0))
            self.assertTrue(comp.close_app())
            self.assertIsNone(comp.app_pid)

    @unittest.skipIf(TEST_CLIENT is None, "vwayland-test-client binary not found")
    def test_input_events_delivered(self) -> None:
        with self.vw.spawn(width=400, height=300, headless=True) as comp:
            comp.launch([str(TEST_CLIENT), "ff0000"])
            log = comp.runtime_dir / "app.log"
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if log.exists() and "VWTEST ready" in log.read_text():
                    break
                time.sleep(0.2)
            self.assertIn("VWTEST ready", log.read_text())

            comp.click(100, 100)
            comp.scroll(dy=-3)
            comp.key("a")
            comp.combo("ctrl", "b")
            time.sleep(0.8)
            text = log.read_text()
            self.assertIn("pointer_enter 100.0 100.0", text)
            self.assertIn("pointer_button 272", text)  # BTN_LEFT
            self.assertIn("pointer_axis", text)
            self.assertIn("key 30 ", text)  # KEY_A (evdev code delivered unchanged)
            self.assertIn("key 48 ", text)  # KEY_B

    @unittest.skipIf(shutil.which("dbus-daemon") is None, "dbus-daemon not found")
    def test_private_dbus_session_bus(self) -> None:
        with self.vw.spawn(width=320, height=240, headless=True) as comp:
            comp.launch(["sh", "-c", 'echo "BUS=$DBUS_SESSION_BUS_ADDRESS"'])
            log = comp.runtime_dir / "app.log"
            addr = None
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and addr is None:
                if log.exists():
                    for line in log.read_text().splitlines():
                        if line.startswith("BUS=") and len(line) > 4:
                            addr = line[4:]
                            break
                time.sleep(0.1)
            self.assertIsNotNone(addr)
            self.assertTrue(addr.startswith(f"unix:path={comp.runtime_dir}/bus"))
            self.assertNotEqual(addr, os.environ.get("DBUS_SESSION_BUS_ADDRESS"))

    def test_double_launch_rejected(self) -> None:
        with self.vw.spawn(width=320, height=240, headless=True) as comp:
            comp.launch(["sleep", "30"])
            with self.assertRaises(self.vw.AppError):
                comp.launch(["sleep", "30"])

    def test_connect_by_id(self) -> None:
        comp = self.vw.spawn(width=320, height=240, headless=True, kill_on_exit=False)
        try:
            same = self.vw.connect(comp.id)
            self.assertEqual(same.info()["id"], comp.id)
            self.assertEqual(same.info()["width"], 320)
        finally:
            comp.kill()


if __name__ == "__main__":
    unittest.main()
