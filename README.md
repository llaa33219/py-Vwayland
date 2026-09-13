# py-Vwayland

<p align="center">
  <img src="logo.svg" alt="py-Vwayland logo" width="1000">
</p>

Spawn **virtual Wayland compositors** from Python, run GUI programs inside them,
capture their screens, and inject mouse/keyboard events.

No desktop environment required: one program per compositor, fullscreen.
The compositor is written in Rust (Smithay) and bundled into the package, so no
system compositor or display server is needed (in headless mode).

```python
import vwayland

with vwayland.spawn(width=1280, height=720, headless=True) as comp:
    comp.launch("my-gui-app")
    img = comp.screenshot()
    img.save("screen.png")
    comp.click(100, 200)
    comp.type_text("hello")
    comp.key("enter")
```

## Installation

```console
$ pip install py-vwayland
```

The wheel bundles the compositor binary and its native libraries, so headless
mode works out of the box on any x86_64 Linux with glibc ≥ 2.28 — no display
server, GPU, or extra packages required. For `screenshot().to_pil()`:

```console
$ pip install py-vwayland[pillow]
```

Quick sanity check:

```console
$ python3 -c "import vwayland; c = vwayland.spawn(); print(c.info()); c.kill()"
```

Building from source and platform details (windowed mode, musl, other
architectures): see [docs/installation.md](docs/installation.md).

## Key features

- Create/list/terminate compositors (`spawn`, `list`, `kill`)
- Open/close a program inside a compositor (`launch`, `close_app`)
- Resize the screen (`resize`)
- Switch between headless and windowed mode (`set_headless`)
- Capture the screen (`screenshot` → PNG)
- Mouse move/click/drag/scroll (`move_to`, `click`, `drag`, `scroll`)
- Keyboard input (`key`, `combo`, `type_text`)

## Documentation

See [docs/](docs/README.md) for full documentation.

- [Overview and motivation](docs/overview.md)
- [Installation and compatibility](docs/installation.md)
- [Python API reference](docs/api-python.md)
- [CLI reference](docs/api-cli.md)
- [Architecture and IPC protocol](docs/architecture.md)

## License

Apache License 2.0 — see [LICENSE](LICENSE).
