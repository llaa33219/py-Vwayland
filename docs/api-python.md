# Python API Reference

```python
import vwayland
```

Names exported at the `vwayland` top level: `spawn`, `connect`, `list`,
`runtime_root`, `Compositor`, `CompositorInfo`, `Image`, exception classes,
`__version__`.

---

## Top-level functions

### `spawn(width=1280, height=720, headless=True, id=None, compositor_bin=None, kill_on_exit=True, startup_timeout=10.0) -> Compositor`

Spawns a virtual compositor.

| Argument | Description |
|---|---|
| `width`, `height` | Initial screen size in px. 16..16384 |
| `headless` | `True`: offscreen (CPU rendering, no display needed). `False`: shown as a window on the host |
| `id` | Instance identifier. Auto-generated as `vw-xxxxxxxx` when omitted. Error if it already exists |
| `compositor_bin` | Explicit compositor binary path |
| `kill_on_exit` | Automatically `kill()` when the Python process exits (default `True`) |
| `startup_timeout` | Seconds to wait for the compositor to become ready |

Raises `CompositorStartError` / `CompositorTimeoutError` / `VwaylandError` on failure.

### `connect(id, kill_on_exit=False) -> Compositor`

Connects to an already-running compositor by id. Raises
`CompositorNotFoundError` if none exists.

### `list() -> list[CompositorInfo]`

Lists all running compositors. Leftover directories of dead instances are cleaned
up automatically.

`CompositorInfo` fields: `id`, `pid`, `runtime_dir` (Path), `display`
(e.g. `"wayland-1"`), `width`, `height`, `headless`, `app_pid`.

### `runtime_root() -> Path`

Returns (creating if needed) the root directory of instance directories.
See [environment variables](installation.md#environment-variables).

---

## `Compositor`

A handle to one running virtual compositor. Supports the context manager protocol.

```python
with vwayland.spawn() as comp:   # comp.kill() is called automatically on exit
    ...
```

### Properties

| Property | Description |
|---|---|
| `comp.id` | Instance id |
| `comp.runtime_dir` | Instance directory (Path) |
| `comp.binary` | Path of the compositor binary in use |
| `comp.pid` | Compositor process pid (`int | None`) |
| `comp.headless` | Current headless flag (updated after `set_headless`) |
| `comp.app_pid` | pid of the running app (`int | None`, queried via ping) |

### Status

#### `comp.info() -> dict`

Pings the compositor and returns its current state. Keys of the returned dict:
`ok`, `version`, `id`, `display`, `width`, `height`, `headless`, `app_pid`.

### 2. Open a program

#### `comp.launch(command, *args, env=None, cwd=None) -> int`

Opens a program inside the compositor and returns its pid.

- `command`: a string (executable) or an argv sequence (`["app", "--flag"]`).
- `env`: extra environment variables dict.
- `cwd`: working directory.

Only **one app per compositor**. Raises `AppError` if one is already running, or
if the executable does not exist. The app's stdout/stderr are appended to
`<runtime_dir>/app.log`.

The app receives `WAYLAND_DISPLAY` (this compositor's socket) and
`XDG_RUNTIME_DIR` (the instance directory); `DISPLAY` is removed (prevents X11
fallback).

### 3. Close the program

#### `comp.close_app(timeout=5.0) -> bool`

Sends SIGTERM to the app and waits for it to exit. The compositor finishes with
SIGKILL after 3 seconds. Returns `True` once no app remains, `False` on timeout.
Returns `True` immediately if no app is running.

### 4. Resize the screen

#### `comp.resize(width, height) -> tuple[int, int]`

Changes the virtual output size and reconfigures the app window to the new
fullscreen size. Returns the actually applied `(width, height)`.
In windowed mode this *requests* a host window size change (which may be ignored).

### 5. Toggle headless mode

#### `comp.set_headless(headless, startup_timeout=10.0) -> None`

Switches between headless and windowed mode. Because this **restarts the
compositor under the same id**, any running app is terminated. The screen size is
preserved.

### 6. Capture the screen

#### `comp.screenshot(timeout=60.0) -> Image`

Renders a fresh frame and returns it as a PNG (see `Image` below).

### 7. Mouse

Coordinates are logical pixels of the screen (= the fullscreen app window) and are
clamped to the screen bounds automatically.

| Method | Description |
|---|---|
| `comp.move_to(x, y)` | Move the pointer to absolute coordinates |
| `comp.click(x=None, y=None, button="left")` | Move then click (clicks at the current position when coordinates are omitted) |
| `comp.mouse_down(button="left")` / `comp.mouse_up(button="left")` | Press / release a button |
| `comp.drag(x1, y1, x2, y2, button="left", steps=20, step_delay=0.0)` | Press at (x1,y1), move to (x2,y2) in `steps` increments, release |
| `comp.scroll(dx=0.0, dy=0.0)` | Scroll. Unit: wheel detents. `dy > 0` scrolls up |

Buttons are given by name (`"left"`, `"right"`, `"middle"`, `"side"`, `"extra"`,
`"forward"`, `"back"`) or evdev button code (`0x110`..`0x117`).

### 8. Keyboard

| Method | Description |
|---|---|
| `comp.key(name)` | Press and release a key |
| `comp.key_down(name)` / `comp.key_up(name)` | Press / release a key |
| `comp.combo(*names)` | Key combo. `combo("ctrl", "c")` → c while holding ctrl |
| `comp.type_text(text, interval=0.0)` | Type a string (US layout; shift is applied automatically for uppercase/symbols). `\n` = enter, `\t` = tab |

Keys are given by name (table below) or evdev key code as int.

#### Key name table

| Name | Code | Notes |
|---|---|---|
| `"a"`..`"z"` | 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44 | |
| `"1"`..`"0"` | 2..11 | |
| `"enter"` (`"return"`) | 28 | |
| `"esc"` (`"escape"`) | 1 | |
| `"tab"` / `"backspace"` / `"space"` (`" "`) | 15 / 14 / 57 | |
| `"shift"` / `"ctrl"` / `"alt"` / `"super"` (`"meta"`, `"win"`) | 42 / 29 / 56 / 125 | left keys |
| `"rightshift"` / `"rightctrl"` / `"rightalt"` / `"rightmeta"` | 54 / 97 / 100 / 126 | |
| `"capslock"` / `"numlock"` / `"scrolllock"` | 58 / 69 / 70 | |
| `"f1"`..`"f10"` / `"f11"` / `"f12"` | 59..68 / 87 / 88 | |
| `"up"` / `"down"` / `"left"` / `"right"` | 103 / 108 / 105 / 106 | arrow keys |
| `"home"` / `"end"` / `"pageup"` / `"pagedown"` | 102 / 107 / 104 / 109 | |
| `"insert"` / `"delete"` | 110 / 111 | |
| `"printscreen"` (`"sysrq"`) / `"pause"` / `"menu"` | 99 / 119 / 139 | |
| `"minus"`(`"-"`) / `"equal"`(`"="`) / `"leftbrace"`(`"["`) / `"rightbrace"`(`"]"`) | 12 / 13 / 26 / 27 | |
| `"semicolon"`(`";"`) / `"apostrophe"`(`"'"`) / `"grave"`(`` "`" ``) / `"backslash"`(`"\\"`) | 39 / 40 / 41 / 43 | |
| `"comma"`(`","`) / `"dot"`(`"."`) / `"slash"`(`"/"`) | 51 / 52 / 53 | |
| `"kp0"`..`"kp9"` | 82, 79, 80, 81, 75, 76, 77, 71, 72, 73 | keypad |
| `"kpenter"` / `"kpdot"` / `"kpplus"` / `"kpminus"` / `"kpasterisk"` / `"kpslash"` | 96 / 83 / 78 / 74 / 55 / 98 | |

A `key_` prefix (e.g. `"KEY_ENTER"`) is also accepted. Unknown names raise
`VwaylandError`.

### 9. Kill a compositor

#### `comp.kill(timeout=5.0) -> None`

Terminates the compositor (and the app inside) and removes the instance
directory. Tries a graceful shutdown (shutdown command) first, then SIGTERM,
then SIGKILL. Does nothing if already dead.

---

## `Image`

The return value of `screenshot()`: a PNG-encoded RGBA frame.

| Member | Description |
|---|---|
| `img.width`, `img.height` | Size in pixels |
| `img.png_bytes` | PNG bytes (same as `bytes(img)`) |
| `img.save(path)` | Save to a PNG file |
| `img.to_pil()` | Convert to `PIL.Image` (requires Pillow) |

---

## Exceptions

All inherit from `vwayland.VwaylandError`.

| Exception | Raised when |
|---|---|
| `CompositorStartError` | The compositor binary was not found or failed to start |
| `CompositorNotFoundError` | `connect(id)` target does not exist, or the compositor in use is already dead |
| `CompositorTimeoutError` | A readiness/response wait timed out |
| `AppError` | App launch failed, or an app is already running |
| `ProtocolError` | The compositor returned an error response, or the IPC stream is broken |
| `VwaylandError` | Common base; also raised for invalid arguments (key names, ranges, etc.) |

---

## Examples

```python
import time
import vwayland

with vwayland.spawn(width=1024, height=768, headless=True) as comp:
    comp.launch(["my-gui-app", "--fullscreen-flag"])
    time.sleep(2)                      # wait for the app to start

    comp.click(512, 100)               # click a button near the top
    comp.type_text("search terms here")
    comp.key("enter")
    comp.scroll(dy=-5)                 # 5 detents down

    img = comp.screenshot()
    img.save("result.png")

    comp.resize(800, 600)              # resize (the app is reconfigured)
    comp.close_app()
```

Multiple compositors at once:

```python
comps = [vwayland.spawn(width=640, height=480) for _ in range(4)]
for c in comps:
    c.launch("my-gui-app")
...
for c in comps:
    c.kill()
```
