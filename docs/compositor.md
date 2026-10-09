# Compositor Internals (vwayland-compositor)

`vwayland-compositor` is a minimal virtual Wayland compositor written on top of
[Smithay](https://smithay.github.io/) 0.7, shipped as a binary inside the wheel.
Sources live in `rust/vwayland-compositor/`.

## Source layout

| File | Role |
|---|---|
| `src/main.rs` | Argument parsing, runtime directory setup, calloop event loop, timers (60Hz frame / 200ms app reaper) |
| `src/state.rs` | Compositor state (space, output, seat, smithay globals), app process launch/close/reap, private D-Bus session bus |
| `src/handlers.rs` | smithay protocol handlers (compositor, xdg_shell, seat, shm, output, data_device) |
| `src/clipboard.rs` | Clipboard selection: set/get/clear over IPC, including the pending-read state machine |
| `src/typing.rs` | Unicode text typing (`type_text`): IME layer dispatch + temporary-keymap key typing |
| `src/text_input.rs` | `zwp_text_input_manager_v3` global, enter/leave focus tracking, `commit_string` commit |
| `src/inject.rs` | IPC input injection (synthesized pointer/keyboard events) + host input handling in windowed mode |
| `src/headless.rs` | Headless backend: pixman software rendering, CPU-buffer screenshots |
| `src/windowed.rs` | Windowed backend: winit + GLES2, offscreen-texture screenshots |
| `src/ipc.rs` | ipc.sock JSON-line protocol server, PNG encoding |

## App environment

- At startup the compositor forks a private `dbus-daemon --session` listening
  on `<runtime_dir>/bus`, and launched apps get its address as
  `DBUS_SESSION_BUS_ADDRESS`. Without this isolation, single-instance apps
  (Firefox, GApplication/KDBusService-based apps) would talk to the host
  session bus and open their windows in an already-running instance on the
  host compositor. If `dbus-daemon` is not installed, the variable is removed
  from the app's environment instead (a warning is logged).
- The daemon is SIGTERM'd when the compositor exits gracefully; the socket
  file is removed with the instance directory in any case.

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
- `wl_seat` (pointer, keyboard), `wl_data_device` (clipboard selection:
  compositor-side set/get/clear via IPC)
- `zwp_text_input_manager_v3` (IME insertion path of `type_text`)
- `wl_output`

## Clipboard

The clipboard is the one selection this compositor implements. It is exposed
through the IPC commands `clipboard_set` / `clipboard_get` / `clipboard_clear`
(see [protocol.md](protocol.md)) and lives in `src/clipboard.rs`.

- **Selection ownership.** `SelectionHandler::SelectionUserData` is
  `Arc<[u8]>`. `clipboard_set` calls `set_data_device_selection()` with the
  mime types `text/plain;charset=utf-8` and `text/plain`; setting a selection
  cancels a previous app-owned one, and `clipboard_clear` calls
  `clear_data_device_selection()`.
- **Serving a selection (`send_selection`).** Runs on the event loop, so the
  pipe write happens on a spawned thread. The fd the app passes to
  `wl_data_offer.receive` is O_NONBLOCK, and `write_all()` on a nonblocking fd
  stops after a partial write and silently truncates the selection, so
  `O_NONBLOCK` is cleared with `fcntl(F_SETFL)` before writing.
- **Reading an app-owned selection (`clipboard_get`).** The compositor hands
  the app a pipe via `request_data_device_client_selection()` and has to wait
  for the app to answer the resulting `wl_data_source.send`. That wait cannot
  block the event loop — the app would never be dispatched and every read
  would burn the full timeout — so the pipe is polled by a timer that also
  flushes the wayland clients between reads, and the IPC connection is answered
  when the pipe reaches EOF. The wait is bounded to 5s.
  A compositor-owned selection needs no round-trip at all: its bytes are read
  straight from the user data via `current_data_device_selection_userdata()`.
- The primary selection (middle-click) is not implemented, and nothing is
  shared with the host clipboard.

## Text typing (`type_text`)

`type_text` (see [protocol.md](protocol.md)) types arbitrary Unicode into the
focused app. Two layers, tried in this order; the answer reports which one ran
in the `method` field (`"ime"` / `"keys"`).

- **Layer C — IME (`src/text_input.rs`).** The compositor itself acts as the
  input method: when the focused app has a `zwp_text_input_v3` object in the
  enabled state, the text is inserted with the `commit_string` event followed by
  `done`, exactly what a real IME sends. Smithay's own `wayland::text_input`
  module is *not* used: it is written around an external IME client
  (`zwp_input_method_v1`) and drops every `enable`/`commit` while no such client
  is connected, which is never the case here. So the text-input objects are
  tracked locally: `enter`/`leave` follow the keyboard focus (from
  `SeatHandler::focus_changed`), `enable`/`disable` stay double-buffered until
  `commit`, and the `done` serial is the number of `commit` requests received on
  that object, as the protocol mandates. The client's state requests
  (surrounding text, content type, cursor rectangle) are accepted and ignored.
- **Layer B — keymap typing (`src/typing.rs`), the universal fallback.** The
  text is typed as *real key events*: a synthetic xkb keymap is generated in
  which one scratch keycode (from 9) carries the keysym of each character,
  installed with `KeyboardHandle::set_keymap_from_string`, then each character is
  injected as a press/release pair of its scratch keycode. `wl_keyboard.key`
  only carries keycodes, so the client resolves them through the keymap that was
  just sent — this is the technique of `wtype`, moved server-side. Keysyms come
  from `xkeysym` (`Keysym::from_char`, plus `Return`/`Tab`/`Escape`/`BackSpace`/
  `Delete` for the control characters) and are written in keymap syntax: the
  table name without its `XK_` prefix, or `UXXXX` for Unicode keysyms.
- **Keymap swapping.** The whole text is typed in chunks of 32 characters, one
  keymap per chunk (repeated characters within a chunk share a key). The
  keymap installed at startup (`XkbConfig::default()`, i.e. the environment /
  "English (US)" default) is restored at the end, including on the error path,
  so no side effect outlives the command. Synthetic keys need no modifiers, and
  every key is released again, so no modifier is left stuck.
- **Timing.** `interval_ms` sleeps between characters on the event loop
  (synchronous, so the compositor is busy while typing); the total sleeping time
  of one call is capped at 10s. The IME layer ignores `interval_ms` (one atomic
  commit).
- Typing never touches the clipboard selection; `clipboard_*` stays specialized
  for copy/paste.

## Limitations

| Item | Status |
|---|---|
| X11 apps (XWayland) | **Not supported**. Wayland clients only (`DISPLAY` is removed) |
| Cursor rendering | Not supported. The cursor is invisible in screenshots (input works) |
| xdg-decoration | Not supported. Clients draw their own decorations (CSD) |
| dmabuf / linux-dmabuf | Not advertised. GL clients must fall back to the shm path |
| Scale (HiDPI) | Fixed at 1 |
| Clipboard | Clipboard selection is supported inside the compositor (IPC `clipboard_set` / `clipboard_get` / `clipboard_clear`). Nothing is exchanged with the *outside* — there is no host clipboard sync. Text/plain only |
| Primary selection | **Not supported** (middle-click selection) |
| DnD | Not implemented. `wl_data_device` is bound, but drag and drop is never started |
| IME protocol | `zwp_input_method_v1` is not implemented: the compositor is the input method itself (`type_text`), no external IME client can attach |
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

- Draws a solid-color fullscreen shm buffer (`vwayland-test-client [RRGGBB]
  [--no-text-input]`, default red).
- Prints `VWTEST ...` lines to stdout for received pointer/keyboard events.
- Prints `VWTEST ready <w>x<h>` after the first draw.
- Prints `VWTEST clipboard <text>`, `VWTEST clipboard-cleared`,
  `VWTEST clipboard-timeout`, `VWTEST clipboard-failed` for selections.
- Binds `zwp_text_input_v3`, enables itself on `enter`, and prints
  `VWTEST text-input <text>` for text an input method commits — this is the
  path `type_text` takes when the focused app supports IME.
- Resolves every key press through the xkb keymap that is current at event
  time (re-read on every `wl_keyboard.keymap` event) and prints
  `VWTEST typed <char>` for each printable character. With `--no-text-input`
  the client has no text input, so `type_text` falls back to the key event
  path and one line per character is printed.

`tests/test_e2e.py` uses this client to verify rendering (screenshot color),
input delivery, clipboard transfer, and both typing paths.
