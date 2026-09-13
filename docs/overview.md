# Overview and Motivation

## What this package does

py-Vwayland is a Python package that spawns **virtual Wayland compositors** as
processes on Linux, runs a GUI program inside each compositor, and lets you
**capture its screen and inject input events**.

The name says it all:

- **V** = Virtual
- **wayland** = Wayland, the Linux display server protocol
- **py-** = distributed as a Python package

## Why it exists (motivation)

Automating or testing a GUI program usually requires one of the following:

1. **A real desktop environment** — CI servers, headless machines, and containers
   don't have one.
2. **Xvfb + X11** — legacy X11 only, awkward for Wayland apps, and you must combine
   separate tools (xdotool, import, ...) for input injection and screen capture.
3. **Existing headless compositors (weston headless, cage, sway headless, etc.)** —
   they can host an app, but "click at coordinates / type keys / take a screenshot
   from Python" requires fighting a different interface for each.

py-Vwayland solves this by **embedding the compositor itself into the library**.

- No display server, desktop environment, or GPU required (headless mode).
- A single `pip install` ships the compositor binary and its native libraries.
- Spawning, screen capture, input injection, and teardown are one Python API.

## Concept: 1 compositor = 1 program = fullscreen

Since a desktop environment is not the goal, window management is deliberately
minimal:

- **One program per compositor.**
- The program's toplevel window is always placed **fullscreen at (0,0)**.
- The (virtual) output size can be changed at any time via the API, and the window
  is automatically reconfigured to the new fullscreen size.
- Popups (context menus, combo box dropdowns, etc. via xdg_popup) are displayed
  and receive input normally.

As a result, the "screen = app window" coordinate systems match exactly, so no
coordinate conversion is needed between screenshots and clicks.

## Key features

| # | Feature | Python API |
|---|---|---|
| 1 | Spawn a compositor | `vwayland.spawn(...)` |
| 2 | Open a program in a compositor | `comp.launch(...)` |
| 3 | Close the program | `comp.close_app()` |
| 4 | Resize the screen | `comp.resize(w, h)` |
| 5 | Toggle headless mode | `comp.set_headless(bool)` |
| 6 | Capture the screen | `comp.screenshot()` |
| 7 | Click / drag / scroll | `comp.click/drag/scroll/move_to/mouse_down/mouse_up` |
| 8 | Keyboard input | `comp.key/combo/type_text/key_down/key_up` |
| 9 | Kill a compositor | `comp.kill()` |
| 10 | List running compositors | `vwayland.list()` |

Every feature is also available through the [CLI](api-cli.md).

## Architecture at a glance

```
[your Python code]
      │  vwayland package (pure Python, standard library only)
      │  Unix socket + JSON-line IPC
      ▼
[vwayland-compositor]  ← Rust + Smithay, bundled in the package
      │  Wayland protocol (headless: CPU rendering / windowed: GL)
      ▼
[GUI program]  (fullscreen, isolated environment with its own WAYLAND_DISPLAY)
```

See [architecture.md](architecture.md) for the full structure and
[compositor.md](compositor.md) for compositor internals.

## Use cases

- **GUI app E2E testing**: run a Wayland app on CI (no display), click/type, and
  verify with screenshots.
- **Screenshot pipelines**: capture app screens periodically for image
  processing/OCR/AI agents.
- **Remote/automated control**: coordinate-based macros, smoke tests, demo recording.
- **Isolated execution**: run an app only inside a mini compositor with its own
  WAYLAND_DISPLAY.

## Design principles

1. **Compatibility first** — headless mode works without any display server, GPU,
   or desktop. The compositor and its native dependencies (libpixman,
   libxkbcommon) are bundled into the wheel.
2. **Pythonic convenience** — everything from `spawn()` to `kill()` behind a
   single import. Zero runtime Python dependencies (standard library only).
3. **Simple policy** — 1 compositor = 1 app = fullscreen. Window manager features
   are intentionally excluded.
4. **Docs match code** — docs/ is always kept up to date (see the rule in AGENTS.md).

## Current limitations

- X11-only apps (requiring XWayland) cannot run. Wayland clients only.
- The cursor is not rendered (not visible in screenshots). Input events work fine.
- `set_headless` **restarts** the compositor under the same id, so a running app
  is terminated.
- Scale (HiDPI) is fixed at 1.

See [compositor.md](compositor.md#limitations) for the full list.
