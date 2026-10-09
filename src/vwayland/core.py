"""py-Vwayland core API: compositor lifecycle, screen capture, input injection."""

from __future__ import annotations

import atexit
import builtins
import os
import re
import secrets
import shutil
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence

from . import client
from .errors import (
    AppError,
    CompositorNotFoundError,
    CompositorStartError,
    CompositorTimeoutError,
    ProtocolError,
    VwaylandError,
)
from .keys import SHIFT_CHARS, resolve_button, resolve_key

__all__ = [
    "spawn",
    "connect",
    "list",
    "Compositor",
    "CompositorInfo",
    "Image",
    "runtime_root",
]

_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")

_spawned: dict[str, "Compositor"] = {}


def _kill_all_spawned() -> None:
    for comp in tuple(_spawned.values()):
        try:
            comp.kill()
        except Exception:
            pass


atexit.register(_kill_all_spawned)


def runtime_root() -> Path:
    """Root directory where instance directories are created.

    Precedence: $VWAYLAND_RUNTIME_DIR → $XDG_RUNTIME_DIR/vwayland → $TMPDIR/vwayland-<uid>
    """
    env = os.environ.get("VWAYLAND_RUNTIME_DIR")
    if env:
        root = Path(env)
    else:
        xdg = os.environ.get("XDG_RUNTIME_DIR")
        if xdg:
            root = Path(xdg) / "vwayland"
        else:
            root = Path(tempfile.gettempdir()) / f"vwayland-{os.getuid()}"
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        os.chmod(root, 0o700)
    except OSError:
        pass
    return root


def _find_binary(compositor_bin: "str | os.PathLike[str] | None" = None) -> Path:
    candidates: list[Path] = []
    env = os.environ.get("VWAYLAND_COMPOSITOR_BIN")
    if compositor_bin:
        candidates.append(Path(compositor_bin))
    if env:
        candidates.append(Path(env))
    candidates.append(Path(__file__).resolve().parent / "_native" / "vwayland-compositor")
    which = shutil.which("vwayland-compositor")
    if which:
        candidates.append(Path(which))
    for path in candidates:
        if path.is_file() and os.access(path, os.X_OK):
            return path
    raise CompositorStartError(
        "vwayland-compositor binary not found. "
        "Build it with scripts/build_compositor.py or set VWAYLAND_COMPOSITOR_BIN. "
        f"(tried: {[str(c) for c in candidates]})"
    )


def _instance_dir(comp_id: str, root: "Path | None" = None) -> Path:
    return (root or runtime_root()) / comp_id


def _validate_id(comp_id: str) -> str:
    if not _ID_RE.match(comp_id):
        raise VwaylandError(f"invalid compositor id: {comp_id!r}")
    return comp_id


def _is_us_typeable(ch: str) -> bool:
    """Can this character be typed by type_text()'s US-layout key loop?

    Mirrors the per-character branches of the key loop exactly: `\\n`, `\\t`,
    uppercase letters (shift + lowercase), SHIFT_CHARS symbols (shift + base)
    and plain characters that resolve to a key name.
    """
    if ch == "\n" or ch == "\t":
        return True
    if ch.isupper():
        ch = ch.lower()
    elif ch in SHIFT_CHARS:
        ch = SHIFT_CHARS[ch]
    try:
        resolve_key(ch)
    except VwaylandError:
        return False
    return True


def _wait_pid_exit(pid: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            return True
        time.sleep(0.05)
    return False


@dataclass
class CompositorInfo:
    """Information about a running compositor instance (result of list())."""

    id: str
    pid: "int | None"
    runtime_dir: Path
    display: str
    width: int
    height: int
    headless: bool
    app_pid: "int | None"


class Image:
    """Result of screenshot() (PNG-encoded RGBA frame)."""

    def __init__(self, width: int, height: int, png_bytes: bytes):
        self.width = width
        self.height = height
        self.png_bytes = png_bytes

    def save(self, path: "str | os.PathLike[str]") -> None:
        """Save to a PNG file."""
        Path(path).write_bytes(self.png_bytes)

    def to_pil(self):
        """Convert to PIL.Image (requires Pillow: pip install py-Vwayland[pillow])."""
        try:
            import io

            from PIL import Image as PILImage
        except ImportError as e:
            raise VwaylandError(
                "to_pil() requires Pillow: pip install Pillow"
            ) from e
        return PILImage.open(io.BytesIO(self.png_bytes))

    def __bytes__(self) -> bytes:
        return self.png_bytes

    def __repr__(self) -> str:
        return f"<vwayland.Image {self.width}x{self.height} png={len(self.png_bytes)}B>"


class Compositor:
    """A handle to one running virtual Wayland compositor."""

    def __init__(
        self,
        comp_id: str,
        runtime_dir: Path,
        binary: Path,
        proc: "subprocess.Popen | None",
        headless: bool,
        kill_on_exit: bool,
    ):
        self.id = comp_id
        self.runtime_dir = runtime_dir
        self.binary = binary
        self._proc = proc
        self.headless = headless
        self._kill_on_exit = kill_on_exit
        self._dead = False

    # ---- low-level ----

    @property
    def _sock(self) -> str:
        return str(self.runtime_dir / "ipc.sock")

    @property
    def pid(self) -> "int | None":
        """Compositor process pid (Popen if we spawned it, else read from the pid file)."""
        if self._proc is not None:
            return self._proc.pid
        try:
            return int((self.runtime_dir / "compositor.pid").read_text().strip())
        except (OSError, ValueError):
            return None

    def _rpc(self, payload: dict[str, Any], timeout: float = 10.0) -> dict[str, Any]:
        try:
            return client.request(self._sock, payload, timeout=timeout)
        except ProtocolError:
            if self._dead:
                raise CompositorNotFoundError(
                    f"compositor {self.id!r} is not running"
                ) from None
            raise

    # ---- status ----

    def info(self) -> dict[str, Any]:
        """ping: current state (id/size/headless flag/app pid, ...)."""
        return self._rpc({"cmd": "ping"})

    @property
    def app_pid(self) -> "int | None":
        return self.info().get("app_pid")

    # ---- app execution ----

    def launch(
        self,
        command: "str | Sequence[str]",
        *args: str,
        env: "dict[str, str] | None" = None,
        cwd: "str | None" = None,
    ) -> int:
        """Open a program inside the compositor. Returns the app's pid.

        One app per compositor (fullscreen policy). Raises AppError if one is
        already running.
        """
        if isinstance(command, (builtins.list, tuple)):
            argv = [str(c) for c in command] + [str(a) for a in args]
        else:
            argv = [command] + [str(a) for a in args]
        if not argv:
            raise AppError("empty command")
        payload: dict[str, Any] = {"cmd": "launch", "argv": argv}
        if env:
            payload["env"] = {str(k): str(v) for k, v in env.items()}
        if cwd:
            payload["cwd"] = str(cwd)
        try:
            resp = self._rpc(payload)
        except ProtocolError as e:
            raise AppError(str(e)) from e
        return int(resp["pid"])

    def close_app(self, timeout: float = 5.0) -> bool:
        """SIGTERM the app and wait for exit. True once the app is gone."""
        resp = self._rpc({"cmd": "close_app"})
        if not resp.get("closed"):
            return True
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.info().get("app_pid") is None:
                return True
            time.sleep(0.1)
        return False

    # ---- output ----

    def resize(self, width: int, height: int) -> "tuple[int, int]":
        """Change the screen size. Returns the actually applied size."""
        resp = self._rpc({"cmd": "resize", "width": int(width), "height": int(height)})
        return int(resp["width"]), int(resp["height"])

    def set_headless(self, headless: bool, startup_timeout: float = 10.0) -> None:
        """Switch between headless and windowed mode.

        This restarts the compositor under the same id, so a running app is
        terminated.
        """
        headless = bool(headless)
        if headless == self.headless:
            return
        info = self.info()
        width, height = int(info["width"]), int(info["height"])
        self._shutdown_process(timeout=5.0)
        self._spawn_process(
            width=width,
            height=height,
            headless=headless,
            startup_timeout=startup_timeout,
        )

    def screenshot(self, timeout: float = 60.0) -> Image:
        """Capture the current screen as a PNG."""
        header, data = client.request_bytes(
            self._sock, {"cmd": "screenshot"}, timeout=timeout
        )
        return Image(int(header["width"]), int(header["height"]), data)

    # ---- mouse ----

    def move_to(self, x: float, y: float) -> None:
        self._rpc({"cmd": "pointer_move", "x": float(x), "y": float(y)})

    def mouse_down(self, button: "str | int" = "left") -> None:
        self._rpc(
            {"cmd": "pointer_button", "button": resolve_button(button), "pressed": True}
        )

    def mouse_up(self, button: "str | int" = "left") -> None:
        self._rpc(
            {"cmd": "pointer_button", "button": resolve_button(button), "pressed": False}
        )

    def click(
        self,
        x: "float | None" = None,
        y: "float | None" = None,
        button: "str | int" = "left",
    ) -> None:
        """Move to the coordinates and click. Clicks at the current position if omitted."""
        if x is not None and y is not None:
            self.move_to(x, y)
        self.mouse_down(button)
        self.mouse_up(button)

    def drag(
        self,
        x1: float,
        y1: float,
        x2: float,
        y2: float,
        button: "str | int" = "left",
        steps: int = 20,
        step_delay: float = 0.0,
    ) -> None:
        """Press at (x1,y1), drag to (x2,y2), release."""
        steps = max(1, int(steps))
        self.move_to(x1, y1)
        self.mouse_down(button)
        try:
            for i in range(1, steps + 1):
                t = i / steps
                self.move_to(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)
                if step_delay:
                    time.sleep(step_delay)
        finally:
            self.mouse_up(button)

    def scroll(self, dx: float = 0.0, dy: float = 0.0) -> None:
        """Scroll (unit: wheel detents). dy > 0 scrolls up."""
        self._rpc({"cmd": "pointer_axis", "dx": float(dx), "dy": float(dy)})

    # ---- keyboard ----

    def key_down(self, key: "str | int") -> None:
        self._rpc({"cmd": "key", "code": resolve_key(key), "pressed": True})

    def key_up(self, key: "str | int") -> None:
        self._rpc({"cmd": "key", "code": resolve_key(key), "pressed": False})

    def key(self, key: "str | int") -> None:
        """Press and release a key ("enter", "a", "f5", "tab", ...)."""
        self.key_down(key)
        self.key_up(key)

    def combo(self, *keys: "str | int") -> None:
        """Key combo: combo("ctrl", "c") → c while holding ctrl."""
        for k in keys:
            self.key_down(k)
        for k in reversed(keys):
            self.key_up(k)

    def type_text(self, text: str, interval: float = 0.0) -> None:
        """Type a string. Any language is supported (Korean, emoji, ...).

        Two tiers, chosen per string:

        - Every character is typeable on the US layout (letters, digits,
          symbols, space, `\\n`, `\\t`): the text is typed with direct key
          events, one key sequence per character, exactly as before. This is
          the only case where `interval` is honored as a sleep between
          characters.
        - Otherwise (any character outside the US layout): the compositor's
          typing engine handles the whole string. It commits the text through
          `zwp_text_input_v3` when the focused field supports it (a real IME
          commit), and otherwise types real key events through a temporary
          keymap. `interval` is passed on as a per-character delay
          (`interval_ms`) and only takes effect on that key-event fallback; the
          IME commit is atomic.

        This differs from `paste_text()`, which routes the text through the
        clipboard instead of the keyboard.
        """
        if not all(_is_us_typeable(ch) for ch in text):
            self._rpc(
                {"cmd": "type_text", "text": text, "interval_ms": int(interval * 1000)}
            )
            return
        shift = resolve_key("shift")
        for ch in text:
            if ch == "\n":
                self.key("enter")
            elif ch == "\t":
                self.key("tab")
            elif ch.isupper():
                self._type_with_shift(ch.lower(), shift)
            elif ch in SHIFT_CHARS:
                self._type_with_shift(SHIFT_CHARS[ch], shift)
            else:
                self.key(ch)
            if interval:
                time.sleep(interval)

    def _type_with_shift(self, base: str, shift: int) -> None:
        self._rpc({"cmd": "key", "code": shift, "pressed": True})
        try:
            self.key(base)
        finally:
            self._rpc({"cmd": "key", "code": shift, "pressed": False})

    # ---- clipboard ----

    def clipboard_get(self) -> "str | None":
        """Return the clipboard text, or None when there is no selection.

        Raises ProtocolError if the compositor reports an error response.
        """
        resp = self._rpc({"cmd": "clipboard_get"})
        text = resp.get("text")
        return None if text is None else str(text)

    def clipboard_set(self, text: str) -> None:
        """Put text in the clipboard (replacing the current selection).

        Any Unicode is accepted, including non-ASCII text such as Korean.
        """
        self._rpc({"cmd": "clipboard_set", "text": str(text)})

    def clipboard_clear(self) -> None:
        """Clear the clipboard (drop the current selection)."""
        self._rpc({"cmd": "clipboard_clear"})

    def paste_text(
        self, text: str, restore: bool = True, restore_delay: float = 0.15
    ) -> None:
        """Type text by pasting it from the clipboard (works for any language).

        Sequence: back up the clipboard, set it to `text`, press Ctrl+V, wait
        `restore_delay` seconds for the app to request the paste data, then
        put the previous clipboard content back.

        Unlike type_text(), which types the text with the keyboard (IME commit
        or key events), this works even when the app blocks pasting: the text
        travels through the clipboard instead.

        - `restore=True` (default): the backed-up content is put back
          (content-identical restore; the clipboard is cleared again if it was
          empty).
        - `restore=False`: `text` stays in the clipboard.
        - `restore_delay`: seconds to wait between Ctrl+V and the restore.
          Use `0.0` to skip the wait.
        """
        backup = self.clipboard_get()
        self.clipboard_set(text)
        self.combo("ctrl", "v")
        if restore_delay > 0:
            time.sleep(restore_delay)
        if restore:
            if backup:
                self.clipboard_set(backup)
            else:
                self.clipboard_clear()

    # ---- lifecycle ----

    def kill(self, timeout: float = 5.0) -> None:
        """Terminate the compositor (and the app inside), then remove the instance directory."""
        if self._dead:
            return
        self._shutdown_process(timeout=timeout)
        shutil.rmtree(self.runtime_dir, ignore_errors=True)
        self._dead = True
        _spawned.pop(self.id, None)

    def _shutdown_process(self, timeout: float) -> None:
        pid = self.pid
        try:
            self._rpc({"cmd": "shutdown"}, timeout=3.0)
        except Exception:
            pass
        if self._proc is not None:
            try:
                self._proc.wait(timeout=timeout)
                return
            except subprocess.TimeoutExpired:
                self._proc.terminate()
                try:
                    self._proc.wait(timeout=2.0)
                    return
                except subprocess.TimeoutExpired:
                    self._proc.kill()
                    self._proc.wait(timeout=2.0)
                    return
        elif pid is not None:
            if _wait_pid_exit(pid, timeout):
                return
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                return
            if _wait_pid_exit(pid, 2.0):
                return
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                return
            _wait_pid_exit(pid, 2.0)

    def _spawn_process(
        self,
        width: int,
        height: int,
        headless: bool,
        startup_timeout: float,
    ) -> None:
        self.runtime_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        os.chmod(self.runtime_dir, 0o700)
        log_path = self.runtime_dir / "compositor.log"
        log = open(log_path, "ab", buffering=0)
        env = dict(os.environ)
        env["XDG_RUNTIME_DIR"] = str(self.runtime_dir)
        env.setdefault("RUST_LOG", "info")
        mode_flag = "--headless" if headless else "--windowed"
        self._proc = subprocess.Popen(
            [
                str(self.binary),
                "--id", self.id,
                "--runtime-dir", str(self.runtime_dir),
                "--width", str(width),
                "--height", str(height),
                mode_flag,
            ],
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            env=env,
            start_new_session=True,
        )
        log.close()  # the child inherited the fd, so close the parent-side handle
        self.headless = headless
        try:
            self._wait_ready(startup_timeout)
        except Exception:
            self.kill(timeout=3.0)
            raise

    def _wait_ready(self, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        last_err: "Exception | None" = None
        while time.monotonic() < deadline:
            if self._proc is not None and self._proc.poll() is not None:
                tail = ""
                try:
                    tail = (self.runtime_dir / "compositor.log").read_text()[-2000:]
                except OSError:
                    pass
                raise CompositorStartError(
                    f"compositor exited during startup (code {self._proc.returncode}).\n{tail}"
                )
            try:
                self._rpc({"cmd": "ping"}, timeout=1.0)
                return
            except (ProtocolError, CompositorNotFoundError, OSError) as e:
                last_err = e
                time.sleep(0.05)
        raise CompositorTimeoutError(
            f"compositor {self.id!r} did not become ready in {timeout}s: {last_err}"
        )

    def __enter__(self) -> "Compositor":
        return self

    def __exit__(self, *exc) -> None:
        self.kill()

    def __repr__(self) -> str:
        state = "dead" if self._dead else ("headless" if self.headless else "windowed")
        return f"<vwayland.Compositor id={self.id!r} {state}>"


def spawn(
    width: int = 1280,
    height: int = 720,
    headless: bool = True,
    id: "str | None" = None,
    compositor_bin: "str | os.PathLike[str] | None" = None,
    kill_on_exit: bool = True,
    startup_timeout: float = 10.0,
) -> Compositor:
    """Spawn a virtual compositor.

    - width/height: initial screen size (default 1280x720)
    - headless: True for offscreen (CPU rendering, no display server needed),
      False to show a window on the host (needs a display server + GL)
    - id: instance identifier (auto-generated when omitted)
    - kill_on_exit: automatically kill when the Python process exits (default True)
    """
    if not (16 <= int(width) <= 16384 and 16 <= int(height) <= 16384):
        raise VwaylandError("width/height must be in 16..=16384")
    binary = _find_binary(compositor_bin)
    root = runtime_root()
    comp_id = _validate_id(id) if id else f"vw-{secrets.token_hex(4)}"
    instance = _instance_dir(comp_id, root)
    if (instance / "ipc.sock").exists():
        raise VwaylandError(
            f"compositor id {comp_id!r} already exists at {instance}; kill it first or pick another id"
        )
    comp = Compositor(
        comp_id=comp_id,
        runtime_dir=instance,
        binary=binary,
        proc=None,
        headless=headless,
        kill_on_exit=kill_on_exit,
    )
    comp._spawn_process(
        width=int(width),
        height=int(height),
        headless=headless,
        startup_timeout=startup_timeout,
    )
    if kill_on_exit:
        _spawned[comp_id] = comp
    return comp


def connect(id: str, kill_on_exit: bool = False) -> Compositor:
    """Connect to an already-running compositor by id."""
    comp_id = _validate_id(id)
    instance = _instance_dir(comp_id)
    if not (instance / "ipc.sock").exists():
        raise CompositorNotFoundError(
            f"no compositor with id {comp_id!r} under {instance.parent}"
        )
    binary = _find_binary()
    comp = Compositor(
        comp_id=comp_id,
        runtime_dir=instance,
        binary=binary,
        proc=None,
        headless=True,
        kill_on_exit=kill_on_exit,
    )
    info = comp.info()
    comp.headless = bool(info.get("headless", True))
    if kill_on_exit:
        _spawned[comp_id] = comp
    return comp


def list() -> list[CompositorInfo]:
    """List all currently running compositors.

    Leftover directories of dead instances are cleaned up automatically.
    """
    root = runtime_root()
    infos: list[CompositorInfo] = []
    for entry in sorted(root.iterdir()):
        if not entry.is_dir():
            continue
        sock = entry / "ipc.sock"
        if not sock.exists():
            continue
        try:
            resp = client.request(str(sock), {"cmd": "ping"}, timeout=0.5)
        except Exception:
            _prune_if_dead(entry)
            continue
        pid: "int | None" = None
        try:
            pid = int((entry / "compositor.pid").read_text().strip())
        except (OSError, ValueError):
            pass
        infos.append(
            CompositorInfo(
                id=entry.name,
                pid=pid,
                runtime_dir=entry,
                display=str(resp.get("display", "")),
                width=int(resp.get("width", 0)),
                height=int(resp.get("height", 0)),
                headless=bool(resp.get("headless", True)),
                app_pid=resp.get("app_pid"),
            )
        )
    return infos


def _prune_if_dead(instance: Path) -> None:
    try:
        pid = int((instance / "compositor.pid").read_text().strip())
    except (OSError, ValueError):
        pid = None
    if pid is not None:
        try:
            os.kill(pid, 0)
            return  # process is alive (keep it; the ping may just be slow)
        except OSError:
            pass
    shutil.rmtree(instance, ignore_errors=True)
