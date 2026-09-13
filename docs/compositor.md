# Compositor Internals (vwayland-compositor)

`vwayland-compositor` is a minimal virtual Wayland compositor written on top of
[Smithay](https://smithay.github.io/) 0.7, shipped as a binary inside the wheel.
Sources live in `rust/vwayland-compositor/`.

## Source layout

| File | Role |
|---|---|
| `src/main.rs` | Argument parsing, runtime directory setup, calloop event loop, timers (60Hz frame / 200ms app reaper) |
| `src/state.rs` | Compositor state (space, output, seat, smithay globals), app process launch/close/reap |
| `src/handlers.rs` | smithay protocol handlers (compositor, xdg_shell, seat, shm, output, data_device) |
| `src/inject.rs` | IPC input injection (synthesized pointer/keyboard events) + host input handling in windowed mode |
| `src/headless.rs` | Headless backend: pixman software rendering, CPU-buffer screenshots |
| `src/windowed.rs` | Windowed backend: winit + GLES2, offscreen-texture screenshots |
| `src/ipc.rs` | ipc.sock JSON-line protocol server, PNG encoding |

## Run modes

### Headless (`--headless`, default)

- Works without any display server/GPU. CPU rendering via `PixmanRenderer`.
- Creates a single virtual output (`VWAYLAND-1`); no hardware backend
  (winit/udev/...) is used.
- A 60Hz timer checks for damage, redraws, and sends frame callbacks to clients
  (without frame callbacks, clients like GTK/Qt stop drawing after the first
  frame).
- Screenshots read the pixman render-target image (A8R8G8B8) directly and
  convert it to RGBA.

### Windowed (`--windowed`)

- Creates a winit window on the host Wayland/X11 and renders with GLES2.
- Real mouse/keyboard input from the host window is forwarded to the app.
- Host window resizes (user/tiling WM) become output resizes.
- Screenshots re-render to an offscreen GL texture and read back through a PBO.
- Note: at startup, before `XDG_RUNTIME_DIR` is switched to the instance
  directory, the host's `WAYLAND_DISPLAY` is pinned to an absolute path
  (otherwise the compositor would connect to its own socket as a client and
  deadlock).

## Window policy

- A new xdg_toplevel is mapped at `(0,0)` immediately and configured as
  `Fullscreen` with the current output size as the size hint, and receives
  keyboard focus.
- Move/resize requests (move_request/resize_request) are ignored (fullscreen is
  fixed).
- On a `resize` IPC, the output mode changes and every toplevel is reconfigured
  to the new size.
- xdg_popups (menus, dropdowns) are repositioned to stay inside the screen
  (unconstrained). Popup grabs are not implemented, so "click outside to close
  the popup" does not work.

## Supported Wayland protocols

- `wl_compositor` (including subsurfaces), `wl_shm`
- `xdg_wm_base` (toplevel/popup), `xdg_output`
- `wl_seat` (pointer, keyboard), `wl_data_device` (for seat protocol completeness)
- `wl_output`

## Limitations

| Item | Status |
|---|---|
| X11 apps (XWayland) | **Not supported**. Wayland clients only (`DISPLAY` is removed) |
| Cursor rendering | Not supported. The cursor is invisible in screenshots (input works) |
| xdg-decoration | Not supported. Clients draw their own decorations (CSD) |
| dmabuf / linux-dmabuf | Not advertised. GL clients must fall back to the shm path |
| Scale (HiDPI) | Fixed at 1 |
| DnD/clipboard integration with the outside | None (the protocol is bound but nothing is exchanged externally) |
| Frame rate | Fixed 60Hz timer in headless mode |

This list evolves with the implementation. If it diverges from the code, fix
this document to match the code.

## Building

```console
$ cd rust
$ cargo build --release        # binary: target/release/vwayland-compositor
```

Key dependencies: `smithay 0.7` (features: backend_winit, wayland_frontend,
desktop, renderer_gl, renderer_pixman), `pixman`, `serde`/`serde_json`, `png`,
`tracing`.

System build dependencies: pkg-config, libpixman-1-dev, libxkbcommon-dev.
(EGL/GL libraries are dlopen'ed at runtime, so they are not needed at build time.)

## Running the compositor standalone (for debugging)

```console
$ mkdir -p /tmp/vw && RUST_LOG=debug ./vwayland-compositor \
    --id debug1 --runtime-dir /tmp/vw --width 800 --height 600 --headless
$ python3 -c "import socket,json; s=socket.socket(socket.AF_UNIX); s.connect('/tmp/vw/ipc.sock'); s.sendall(b'{\"cmd\":\"ping\"}\n'); print(s.makefile().readline())"
```

## Test client (vwayland-test-client)

`rust/test-client/` is a minimal Wayland client used only by the integration
tests (not shipped).

- Draws a solid-color fullscreen shm buffer (`vwayland-test-client [RRGGBB]`,
  default red).
- Prints `VWTEST ...` lines to stdout for received pointer/keyboard events.
- Prints `VWTEST ready <w>x<h>` after the first draw.

`tests/test_e2e.py` uses this client to verify rendering (screenshot color) and
input delivery.
