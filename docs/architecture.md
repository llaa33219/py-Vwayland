# Architecture

## Overall structure

```
┌─────────────────────────────────────────────────────────────┐
│ User Python process                                          │
│  import vwayland                                             │
│    ├─ Compositor object (lifecycle / input / capture API)    │
│    └─ Low-level IPC client (Unix socket, JSON lines)         │
└──────────────┬──────────────────────────────────────────────┘
               │ $RUNTIME/vwayland/<id>/ipc.sock
               ▼
┌─────────────────────────────────────────────────────────────┐
│ vwayland-compositor (Rust + Smithay, bundled in the package) │
│  ├─ Wayland server (wayland-N socket)                        │
│  ├─ One output (VWAYLAND-1), one seat (pointer + keyboard)   │
│  ├─ Renderer: headless=pixman(CPU) / windowed=winit+GLES2    │
│  ├─ IPC server (ipc.sock)                                    │
│  └─ App process watcher (SIGTERM→SIGKILL, zombie reaping)    │
└──────────────┬──────────────────────────────────────────────┘
               │ Wayland protocol
               ▼
┌─────────────────────────────────────────────────────────────┐
│ GUI app (one per compositor, fullscreen)                     │
│  WAYLAND_DISPLAY=wayland-N, XDG_RUNTIME_DIR=<instance dir>   │
└─────────────────────────────────────────────────────────────┘
```

## Instance directory

Each compositor corresponds 1:1 to a directory.

```
$VWAYLAND_RUNTIME_DIR (or $XDG_RUNTIME_DIR/vwayland, $TMPDIR/vwayland-<uid>)
└── <id>/                     (mode 0700)
    ├── wayland-1             Wayland server socket (+ wayland-1.lock)
    ├── ipc.sock              Control socket (JSON-line protocol)
    ├── compositor.pid        Compositor pid
    ├── compositor.log        Compositor log (RUST_LOG)
    └── app.log               App stdout/stderr
```

- The instance directory becomes the compositor's own `XDG_RUNTIME_DIR`, so the
  Wayland sockets of multiple compositors never collide.
- `vwayland.list()` scans this root and pings each `ipc.sock`, returning only live
  instances. If a socket does not respond and its pid is dead, the directory is
  removed.

## Lifecycle

### spawn

1. Create the id and instance directory (0700).
2. Exec `vwayland-compositor --id <id> --runtime-dir <dir> --width W --height H
   --headless|--windowed` as a session leader (`start_new_session`);
   stdout/stderr are redirected to `compositor.log`.
3. Wait until a ping on `ipc.sock` succeeds (default 10s). On failure the process
   is cleaned up and a `CompositorStartError` with the tail of `compositor.log`
   is raised.

### kill

1. Send the IPC `shutdown` request and wait for a graceful exit.
2. On timeout: SIGTERM → 2s → SIGKILL.
3. Remove the instance directory.

With `kill_on_exit=True` (default), the same procedure runs from the Python
process's `atexit` handler. The CLI spawns with `kill_on_exit=False`, so an
explicit `kill` is required there.

### set_headless

The headless ↔ windowed switch is implemented as a **process restart**:

1. Read the current size via ping.
2. Shut down the compositor.
3. Restart with the same id, same directory, same size, opposite mode.

Because the Wayland server lives inside the compositor process, swapping only
the renderer while keeping client (app) connections alive is not possible.
Therefore **the app is terminated on switch**; relaunch it with `launch` if
needed.

## Rendering paths

| Mode | Renderer | Behavior |
|---|---|---|
| Headless | pixman (CPU) | A 60Hz timer redraws on damage and sends frame callbacks. Screenshots read the CPU buffer directly |
| Windowed | GLES2 (winit) | Renders with a damage tracker on each winit redraw event and swaps. Screenshots re-render to an offscreen texture and read back |

In both modes, a screenshot always **renders a fresh full frame**, converts it to
RGBA8, and encodes it as PNG.

## Input paths

- **Injected input (IPC)**: coordinates/buttons/key codes from Python are
  synthesized directly into the compositor's seat pointer/keyboard objects. Key
  codes on the protocol are evdev codes; the compositor converts them to
  smithay's xkb convention (+8).
- **Host input (windowed mode)**: real mouse/keyboard events from the winit
  window go through the same path.

Focus policy: a new toplevel receives keyboard focus as soon as it is mapped,
and clicking moves focus to the window under the pointer (with a single app,
that is always the app).

## Security/isolation notes

- Instance directories are mode 0700, so only the same uid can access them.
- The IPC socket has no authentication. **Any local process with the same uid
  can control the compositors**, so do not open the runtime directory to other
  users.
- The compositor and the app are separate processes but not a sandbox (same uid,
  same privileges).

## IPC protocol

The full specification lives in [protocol.md](protocol.md). Requests are one
JSON line, responses are one JSON line; only `screenshot` appends binary (PNG)
payload after its header line.
