# Installation and Compatibility

## Supported environments

| Item | Requirement |
|---|---|
| OS | Linux (any environment that can run Wayland clients) |
| Architecture | x86_64 (other architectures: build from source) |
| Python | 3.9+ |
| glibc | The bundled binary requires the build machine's glibc or newer (releases should target glibc 2.28) |
| Display server | **Headless mode: not required** / windowed mode: a Wayland or X11 session + EGL/GLES2 |
| GPU | **Headless mode: not required** (pixman CPU rendering) / windowed mode: host GL |

## Installation

### Wheel (recommended)

```console
$ pip install py-Vwayland
```

The wheel bundles:

- `vwayland-compositor` — the virtual compositor binary written in Rust
- `lib/` — non-standard native dependencies (`libpixman-1.so.0`, `libxkbcommon.so.0`)
  - The binary's RPATH is set to `$ORIGIN/lib`, so the bundled copies are used even
    if the system lacks these libraries (or has different versions).
  - Only the glibc family (`libc`, `libm`, `ld-linux`, `libgcc_s`) comes from the
    system.

For Pillow integration:

```console
$ pip install py-Vwayland[pillow]   # required for screenshot().to_pil()
```

### From source

Source trees do not contain the compositor binary; build it first.

```console
$ git clone <repo> && cd py-Vwayland
$ python3 scripts/build_compositor.py   # requires cargo + patchelf
$ pip install .
```

Build requirements:

- Rust toolchain (cargo; rustup recommended)
- C build tools, `pkg-config`, `libpixman-1` headers, `libxkbcommon` headers
  (Debian/Ubuntu: `sudo apt install build-essential pkg-config libpixman-1-dev libxkbcommon-dev patchelf`)
- `patchelf` — system package or `pip install patchelf`

The `--no-build` flag skips the cargo build and bundles an existing release binary.

## Verifying the installation

```console
$ python3 -c "import vwayland; c = vwayland.spawn(); print(c.info()); c.kill()"
$ vwayland spawn --id smoke && vwayland screenshot smoke -o smoke.png && vwayland kill smoke
```

## Compatibility details

### Headless mode (default)

- No display server, GPU, or desktop environment required **at all**.
- Rendering is done by pixman (CPU); the app draws via wl_shm buffers.
- Works out of the box in containers, CI, and SSH sessions.

### Windowed mode (`headless=False`)

- Requires a host Wayland or X11 session and a working EGL/GLES2 initialization.
- Shown as a regular window on the host; host mouse/keyboard input is forwarded.
- The host window manager may resize the window, and the output size follows the
  window size (`resize()` *requests* a window size change; tiling WMs may ignore it).

### musl (Alpine, etc.)

The bundled binary targets glibc. On musl systems, build from source and point
`VWAYLAND_COMPOSITOR_BIN` at your binary.

## Environment variables

| Variable | Purpose |
|---|---|
| `VWAYLAND_RUNTIME_DIR` | Root of instance directories (default: `$XDG_RUNTIME_DIR/vwayland` or `$TMPDIR/vwayland-<uid>`) |
| `VWAYLAND_COMPOSITOR_BIN` | Force a specific compositor binary (overrides the bundled one) |
| `RUST_LOG` | Compositor log level (default `info`, e.g. `debug`, `vwayland_compositor=trace`) |

## Binary lookup order

`spawn()` locates the compositor binary in this order:

1. The `spawn(..., compositor_bin=...)` argument
2. The `VWAYLAND_COMPOSITOR_BIN` environment variable
3. The bundled `vwayland/_native/vwayland-compositor`
4. `vwayland-compositor` on `$PATH`

If all fail, `CompositorStartError` is raised.
