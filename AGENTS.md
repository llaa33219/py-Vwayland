# AGENTS.md — py-Vwayland Project Guide

This file is the guide for agents and contributors working in this repository.
**Read this file and `docs/README.md` before starting any work.**

## What this project is (30-second summary)

py-Vwayland is a package that **spawns virtual Wayland compositors from Python
to control GUI programs**.

- The compositor is written in **Rust (Smithay 0.7)** and shipped as a binary
  inside the wheel (`src/vwayland/_native/`).
- The Python package (`src/vwayland/`) launches compositor processes and
  controls them over a **JSON-line IPC** on a Unix socket. It has zero runtime
  Python dependencies.
- Policy: **1 compositor = 1 app = fullscreen.** Window management features are
  intentionally absent.
- 11 features: spawn / open program / close program / screen size / headless
  toggle / screen capture / mouse / keyboard / clipboard / kill / list. The
  Python API and the `vwayland` CLI map 1:1.

## Repository map

| Path | What lives there |
|---|---|
| `src/vwayland/core.py` | Most of the public API (spawn/connect/list, Compositor) |
| `src/vwayland/client.py` | Low-level IPC client |
| `src/vwayland/keys.py` | Key/button names → evdev codes |
| `src/vwayland/cli.py` | `vwayland` CLI |
| `rust/vwayland-compositor/` | The compositor itself (main/state/handlers/inject/headless/windowed/ipc) |
| `rust/test-client/` | Minimal Wayland client for E2E tests |
| `scripts/build_compositor.py` | Compositor build + native bundling (patchelf) |
| `tests/` | unittest-based unit/E2E tests |
| `docs/` | All technical documentation (see the docs rules below) |

Deeper material lives in `docs/`: motivation (`overview.md`), structure
(`architecture.md`), IPC spec (`protocol.md`), compositor internals
(`compositor.md`), development procedures (`development.md`).

## Documentation rules (important)

1. **The documents in `docs/` must always be up to date.** When you change code,
   update the related documents in the same commit.
2. **If code and docs disagree, fix the docs to match the code.** The code is
   always the source of truth for behavior; the docs are considered wrong.
   (Exception: if the code is clearly buggy, ask the user whether to fix the
   code and keep the docs.)
3. When changing the public API (Python functions / CLI / IPC protocol), update
   at least: `docs/api-python.md` or `docs/api-cli.md`, `docs/protocol.md`,
   `README.md`, and the relevant docstrings.

## Build/test commands

```console
# Compositor build (debug)
cd rust && cargo build

# Compositor release build + bundle into src/vwayland/_native (requires patchelf)
python3 scripts/build_compositor.py

# Tests (compositor binary auto-detected: _native → target/release → target/debug)
python3 -m unittest discover -s tests -v

# Using Python during development
PYTHONPATH=src python3 -c "import vwayland; ..."
PYTHONPATH=src python3 -m vwayland.cli list
```

## Implementation landmines (read before touching)

- **Key code +8**: the IPC `key.code` is an evdev code, but smithay's keyboard
  input expects the xkb convention (+8). The conversion happens in `inject.rs`.
  If you touch this, verify with `tests/test_e2e.py::test_input_events_delivered`.
- **WAYLAND_DISPLAY in windowed mode**: at startup the compositor switches
  `XDG_RUNTIME_DIR` to the instance directory. The code in `main.rs` that pins
  `WAYLAND_DISPLAY` to an absolute path exists so winit can still find the host
  compositor; removing it makes the compositor connect to itself and deadlock.
- **frame callbacks**: headless mode sends frame callbacks on a 60Hz timer.
  Removing them makes GTK/Qt apps stop drawing after the first frame.
- **Bundle RPATH**: the `_native/` binary uses the bundled libpixman/libxkbcommon
  under `$ORIGIN/lib`. Replacing the binary by hand drops the rpath — use
  `scripts/build_compositor.py --no-build` instead.
- In Python, `list` is shadowed by the module-level function. If you need the
  builtin `list` in core.py, use `builtins.list`.

## Coding rules

- Python: standard library only (runtime). Python 3.9+ compatible. Use the
  exception hierarchy in `errors.py`.
- Rust: `cargo build` must pass without warnings.
- Comments in English, only where truly needed (non-obvious
  protocol/coordinate/format details).
- `python3 -m unittest discover -s tests` must pass before committing.
