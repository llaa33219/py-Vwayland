# IPC Protocol Specification

The control protocol between the Python client and `vwayland-compositor`.
Consult this when writing your own client or debugging.
(Regular users only need the [Python API](api-python.md).)

## Transport

- **Address**: `<instance directory>/ipc.sock` (Unix domain socket, stream)
- **Connection model**: one connection per command. Request → response → close.
- **Request**: one UTF-8 JSON object + `\n` (a single line)
- **Response**: one UTF-8 JSON object + `\n`. Only `screenshot` appends binary
  data after this line.
- Commands are discriminated by the `"cmd"` field; all keys are snake_case.

### Common response shape

Success: `{"ok": true, ...command-specific fields...}`
Failure: `{"ok": false, "error": "message"}`

Unparseable requests also get a `{"ok": false, ...}` response.

## Commands

### ping

```json
→ {"cmd": "ping"}
← {"ok": true, "version": "0.1.0", "id": "vw-1234abcd", "display": "wayland-1",
   "width": 1280, "height": 720, "headless": true, "app_pid": 12345}
```

- `app_pid`: `null` when no app is running.

### launch

```json
→ {"cmd": "launch", "argv": ["foot", "--title", "x"], "env": {"FOO": "bar"}, "cwd": "/tmp"}
← {"ok": true, "pid": 12345}
```

- `argv` (required): must not be empty.
- `env` (optional, default `{}`): extra environment variables.
- `cwd` (optional, default null): working directory.
- If an app is already running: `{"ok": false, "error": "an app is already running..."}`.
- The app receives `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`; `DISPLAY` is removed.
- `DBUS_SESSION_BUS_ADDRESS` is pointed at the compositor's private session bus
  (a per-instance `dbus-daemon` listening on `<runtime_dir>/bus`), so
  single-instance apps (Firefox, GApplication/KDBusService apps) open their
  windows here instead of activating an instance running on the host session.
  If `dbus-daemon` is unavailable, the variable is removed instead.

### close_app

```json
→ {"cmd": "close_app"}
← {"ok": true, "closed": true}
```

- Sends SIGTERM; the compositor escalates to SIGKILL after 3s. `closed` indicates
  whether SIGTERM was actually sent (`false` when no app is running).

### resize

```json
→ {"cmd": "resize", "width": 1920, "height": 1080}
← {"ok": true, "width": 1920, "height": 1080}
```

- Range: 16..16384. Out of range is an error.
- Any open toplevel is reconfigured to fullscreen at the new size.
- In windowed mode, this also *requests* a host window size change (may be
  ignored).

### screenshot

```json
→ {"cmd": "screenshot"}
← {"ok": true, "width": 1280, "height": 720, "format": "png", "bytes": 15342}\n<15342 bytes of PNG>
```

- Exactly `bytes` bytes follow the header line.
- `format` is currently always `"png"` (RGBA8, 8-bit).
- A full frame is freshly rendered for each request.

### pointer_move

```json
→ {"cmd": "pointer_move", "x": 100.0, "y": 200.0}
← {"ok": true}
```

- Logical coordinates (float). Out-of-bounds values are clamped.

### pointer_button

```json
→ {"cmd": "pointer_button", "button": 272, "pressed": true}
← {"ok": true}
```

- `button`: evdev button code (left `0x110`=272, right `0x111`=273,
  middle `0x112`=274).
- `pressed`: press/release.

### pointer_axis

```json
→ {"cmd": "pointer_axis", "dx": 0.0, "dy": -3.0}
← {"ok": true}
```

- Unit: wheel detents. Clients receive value = detents × 15 and v120 =
  detents × 120.
- `dy > 0` scrolls up.

### key

```json
→ {"cmd": "key", "code": 30, "pressed": true}
← {"ok": true}
```

- `code`: an **evdev key code** (e.g. `KEY_A`=30, `KEY_ENTER`=28).
  The compositor converts it to the xkb convention (+8) internally, so clients
  receive the evdev code unchanged.

### shutdown

```json
→ {"cmd": "shutdown"}
← {"ok": true}
```

- After responding, the compositor's event loop exits. A running app is
  terminated as well.

## Implementation notes

- Requests are read up to the first newline. Do not embed newlines in the JSON.
- The compositor-side read timeout is 5s; the write timeout is 60s.
- Connections are handled sequentially, one at a time (single-threaded event
  loop).
- There is no protocol version negotiation; check the compositor version with
  `ping`'s `version` field.
