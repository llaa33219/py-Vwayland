# Installation and Compatibility

## Supported environments

| Item | Requirement |
|---|---|
| OS | Linux (any environment that can run Wayland clients) |
| Architecture | x86_64, aarch64, riscv64 — prebuilt wheels; others: build from source |
| libc | glibc ≥ 2.28 **or** musl ≥ 1.2 — both have prebuilt wheels |
| Python | 3.9+ |
| Display server | **Headless mode: not required** / windowed mode: a Wayland or X11 session + EGL/GLES2 |
| GPU | **Headless mode: not required** (pixman CPU rendering) / windowed mode: host GL |
| dbus-daemon | Optional. If present, each compositor runs a private session bus so single-instance apps (Firefox, GApplication apps) open inside the compositor; if absent, apps run without a session bus |

## Installation

### Wheel (recommended)

```console
$ pip install py-vwayland
```

Wheels are published **per platform** (`py3-none-<platform-tag>`), so pip only
installs a build that matches your machine — installing on an unsupported
platform fails at resolve time instead of crashing at runtime.

| Wheel tag | Platform | Bundled native code |
|---|---|---|
| `manylinux_2_28_x86_64` | x86_64, glibc ≥ 2.28 | binary + `lib/` (pixman, xkbcommon) |
| `manylinux_2_28_aarch64` | aarch64, glibc ≥ 2.28 | binary + `lib/` |
| `manylinux_2_28_riscv64` | riscv64, glibc ≥ 2.28 | binary + `lib/` |
| `musllinux_1_2_x86_64` | x86_64, musl ≥ 1.2 | statically-linked binary (no `lib/`) |
| `musllinux_1_2_aarch64` | aarch64, musl ≥ 1.2 | statically-linked binary (no `lib/`) |

For glibc builds the binary's RPATH is `$ORIGIN/lib`, so the bundled
pixman/xkbcommon are used even if the system lacks them; only the glibc family
(`libc`, `libm`, `ld-linux`, `libgcc_s`) comes from the system. musl builds are
fully static and need nothing but the kernel.

For Pillow integration:

```console
$ pip install py-vwayland[pillow]   # required for screenshot().to_pil()
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

musl is supported via prebuilt `musllinux_1_2_*` wheels (statically linked, no
system libraries required). If you build from source on musl, the build
produces a static binary; point `VWAYLAND_COMPOSITOR_BIN` at it or use
`scripts/build_compositor.py --target <rust-musl-triple>`.

### Other architectures / libc

Build from source and cross-compile. `scripts/prepare_sysroots.py` fetches
prebuilt cross toolchains and builds the pixman/xkbcommon sysroots, then
`scripts/build_compositor.py --target <triple>` bundles the result. See
[development.md](development.md#cross-compilation).

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
3. The bundled `vwayland/_native/<platform-tag>/vwayland-compositor` matching
   the current platform (e.g. `manylinux_2_28_aarch64`, `musllinux_1_2_x86_64`)
4. The legacy flat `vwayland/_native/vwayland-compositor`
5. `vwayland-compositor` on `$PATH`

If all fail, `CompositorStartError` is raised.
