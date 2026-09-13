# py-Vwayland Documentation

py-Vwayland is a library that spawns **virtual Wayland compositors** from Python,
runs GUI programs inside them, captures their screens, and injects mouse/keyboard
events.

## Table of Contents

| Document | Contents |
|---|---|
| [overview.md](overview.md) | Motivation, concept, key features, use cases |
| [installation.md](installation.md) | Installation, system requirements, compatibility, native bundle layout |
| [api-python.md](api-python.md) | Full Python API reference (functions, classes, key name table) |
| [api-cli.md](api-cli.md) | Full `vwayland` CLI command reference |
| [architecture.md](architecture.md) | Internal structure, instance lifecycle, directory layout |
| [protocol.md](protocol.md) | Python ↔ compositor IPC protocol specification |
| [compositor.md](compositor.md) | Rust compositor internals, supported Wayland protocols, limitations |
| [development.md](development.md) | Dev setup, build, test, release (bundling) procedures |

## Quick Start

```python
import vwayland

with vwayland.spawn(width=1280, height=720, headless=True) as comp:
    comp.launch("my-gui-app")          # open a program inside the compositor
    img = comp.screenshot()            # capture the screen (PNG)
    img.save("screen.png")
    comp.click(100, 200)               # click at coordinates
    comp.type_text("hello world")      # keyboard input
    comp.key("enter")
```

```console
$ vwayland spawn --width 1280 --height 720 --id demo
$ vwayland launch demo -- my-gui-app
$ vwayland screenshot demo -o screen.png
$ vwayland kill demo
```

## Documentation Freshness Rule

The documents under `docs/` must **always match the code.** When you change code,
update the related documents in the same commit. If you find a discrepancy between
code and docs, **fix the docs to match the code.**
(The same rule is stated in AGENTS.md.)
