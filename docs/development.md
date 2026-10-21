# Development Guide

## Repository layout

```
py-Vwayland/
├── LICENSE                  # Apache-2.0
├── AGENTS.md                # Project guide for agents/contributors + docs rules
├── README.md
├── pyproject.toml           # Python package metadata (hatchling)
├── docs/                    # All technical documentation (always up to date)
├── src/vwayland/            # Python package
│   ├── __init__.py          # Public API exports
│   ├── core.py              # spawn/connect/list + Compositor/Image
│   ├── client.py            # Low-level IPC client
│   ├── keys.py              # Key/button names → evdev codes
│   ├── cli.py               # vwayland CLI
│   ├── errors.py            # Exception hierarchy
│   └── _native/             # (build artifact) compositor binary + bundled libs
├── rust/                    # Rust workspace
│   ├── vwayland-compositor/ # The compositor itself
│   └── test-client/         # Minimal Wayland client for tests
├── scripts/
│   └── build_compositor.py  # Compositor build + _native bundling script
└── tests/                   # unittest-based tests
```

## Dev environment setup

```console
$ git clone <repo> && cd py-Vwayland
# Rust: https://rustup.rs
$ sudo apt install build-essential pkg-config libpixman-1-dev libxkbcommon-dev patchelf
$ python3 scripts/build_compositor.py     # build the compositor + bundle
```

The Python package has no runtime dependencies, so you can develop directly with
`PYTHONPATH=src`.

```console
$ PYTHONPATH=src python3 -c "import vwayland; print(vwayland.__version__)"
```

## Building

### Compositor only (during development)

```console
$ cd rust && cargo build          # debug (fast)
$ cd rust && cargo build --release
```

### Full bundle (release unit)

```console
$ python3 scripts/build_compositor.py
```

- Builds release and bundles it under `src/vwayland/_native/<platform-tag>/`
  (e.g. `manylinux_2_28_x86_64/`)
- Copies non-standard libraries found via ldd (libpixman, libxkbcommon) into
  `_native/<platform-tag>/lib/` (glibc targets)
- Sets RPATH to `$ORIGIN/lib` with patchelf
- Prints the resulting ldd output and verifies execution with `--version`

### Cross-compilation

To build for the other supported platforms (aarch64, riscv64, musl), first
prepare the cross toolchains and pixman/xkbcommon sysroots (downloaded into
`.miniouto/`, requires curl + meson + ninja):

```console
$ python3 scripts/prepare_sysroots.py            # all supported targets
$ python3 scripts/prepare_sysroots.py aarch64-unknown-linux-musl   # one
```

Then export the environment it prints and build each target:

```console
$ export CC_aarch64_unknown_linux_musl=.../aarch64-linux-musl-gcc \
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=... \
      RUSTFLAGS="-L $PWD/.miniouto/sysroots/aarch64-musl/lib"
$ python3 scripts/build_compositor.py --target aarch64-unknown-linux-musl
```

- musl targets produce a statically-linked binary (no `lib/` bundled)
- glibc cross targets need `VWAYLAND_SYSROOT_LIB=<sysroot>/lib` so the bundler
  can find the target's pixman/xkbcommon `.so` files (foreign binaries cannot
  be inspected with the host `ldd`)

### Platform wheels

`python -m build` produces one `py3-none-any` wheel containing every bundled
platform. Split it into correctly-tagged per-platform wheels before release:

```console
$ python3 scripts/make_wheels.py
```

Each output wheel keeps only its own `_native/<tag>/` directory and is tagged
`py3-none-<platform-tag>` with `Root-Is-Purelib: false`, so pip refuses to
install it on the wrong platform.

### Wheel build

```console
$ python3 -m build --wheel
```

The per-platform bundles in `_native/<tag>/` are included in the wheel (the
directory is gitignored, so it is force-included). For releases, run
`build_compositor.py` on an old-glibc environment (e.g. manylinux_2_28) for the
glibc targets to maximize compatibility, then run `make_wheels.py` to produce
the per-platform wheels.

## Releasing (PyPI, GitHub Actions)

Releases are automated through `.github/workflows/release.yml`:

1. A tag push matching `v*.*.*` triggers the pipeline.
2. The `build` job runs inside a `manylinux_2_28` container (glibc 2.28
   baseline): installs Rust + cross targets, prepares cross sysroots
   (`prepare_sysroots.py`), builds and bundles the compositor for every
   supported platform, verifies the tag matches `pyproject.toml`'s `version`,
   runs the test suite, builds the sdist + base wheel, and splits it into
   per-platform wheels (`make_wheels.py`).
3. The `publish` job uploads to PyPI with **Trusted Publishing (OIDC)** — no
   API token is stored anywhere.

### One-time PyPI setup (required before the first tag)

1. Log in to PyPI and open
   <https://pypi.org/manage/account/publishing/>.
2. Add a **pending publisher** with:
   - PyPI project name: `py-Vwayland`
   - Owner: `llaa33219`
   - Repository: `py-Vwayland`
   - Workflow name: `release.yml`
   - Environment name: `pypi`

(The GitHub `pypi` environment referenced by the workflow is created
automatically on first use; you may add protection rules in the repo settings.)

### Cutting a release

```console
# bump version in pyproject.toml first, then:
git tag v0.1.0
git push origin v0.1.0
```

## Testing

```console
$ python3 -m unittest discover -s tests -v
```

- `test_keys.py`: key mapping unit tests (no compositor needed)
- `test_e2e.py`: E2E tests that spawn real compositors
  - spawn/ping/resize/kill, blank-screen screenshot, app launch + color
    screenshot, input event delivery, duplicate launch rejection, reconnect by id
  - Automatically skipped when no compositor binary is found; app-related tests
    are skipped when no test-client is found.
  - Set `VWAYLAND_COMPOSITOR_BIN` to choose the binary under test.

## Debugging tips

- Compositor log: `<runtime_dir>/<id>/compositor.log` (`RUST_LOG=debug`
  recommended)
- App log: `<runtime_dir>/<id>/app.log`
- Set `VWAYLAND_RUNTIME_DIR` to choose the runtime directory explicitly
- You can also run the compositor directly and send IPC by hand
  ([compositor.md](compositor.md#running-the-compositor-standalone-for-debugging))

## Contribution rules

1. **Update docs together**: when you change a feature or behavior, update the
   relevant `docs/` pages in the same commit. If code and docs disagree, fix the
   docs to match the code.
2. When changing the public Python API, update `docs/api-python.md`, `README.md`,
   and the docstrings together.
3. When changing the IPC protocol, update `docs/protocol.md` and change the
   Python client and the compositor in the same commit.
4. Tests: add E2E coverage in `tests/test_e2e.py` for changes that can be
   exercised end to end.
5. Before committing:
   ```console
   $ cd rust && cargo build && cd ..
   $ python3 -m unittest discover -s tests
   ```
6. License: new files are covered by Apache-2.0 (see LICENSE).
