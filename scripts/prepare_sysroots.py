#!/usr/bin/env python3
"""Fetch cross toolchains and build pixman/xkbcommon sysroots for cross builds.

Downloads prebuilt cross compilers (musl.cc for musl targets, Bootlin for
glibc targets) into ``.miniouto/toolchains/`` and builds pixman + libxkbcommon
into ``.miniouto/sysroots/<platform>/``:

- musl targets  -> static libs (.a), linked into the compositor binary
- glibc targets -> shared libs (.so), bundled under _native/<tag>/lib

Then generates ``scripts/cross_env.sh`` with the environment cargo needs.
Only needed for cross compilation; the native build needs none of this.

Usage:
    python3 scripts/prepare_sysroots.py [TARGET ...]
    # TARGET is a Rust triple; default: all supported cross targets.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tarfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORK = REPO_ROOT / ".miniouto"
TC_DIR = WORK / "toolchains"
SR_DIR = WORK / "sysroots"
CROSS_DIR = SR_DIR / "cross"

PIXMAN_VERSION = "0.42.2"
XKBCOMMON_VERSION = "1.6.0"
PIXMAN_URL = f"https://www.cairographics.org/releases/pixman-{PIXMAN_VERSION}.tar.gz"
XKBCOMMON_URL = (
    f"https://xkbcommon.org/download/libxkbcommon-{XKBCOMMON_VERSION}.tar.xz"
)

# Rust triple -> (toolchain url, toolchain dir name, gcc prefix, libc kind)
TOOLCHAINS = {
    "x86_64-unknown-linux-musl": (
        "https://toolchains.bootlin.com/downloads/releases/toolchains/x86-64/"
        "tarballs/x86-64--musl--stable-2024.02-1.tar.bz2",
        "x86-64--musl--stable-2024.02-1",
        "x86_64-linux",
        "musl",
    ),
    "aarch64-unknown-linux-musl": (
        "https://toolchains.bootlin.com/downloads/releases/toolchains/aarch64/"
        "tarballs/aarch64--musl--stable-2024.02-1.tar.bz2",
        "aarch64--musl--stable-2024.02-1",
        "aarch64-linux",
        "musl",
    ),
    "aarch64-unknown-linux-gnu": (
        "https://toolchains.bootlin.com/downloads/releases/toolchains/aarch64/"
        "tarballs/aarch64--glibc--stable-2024.05-1.tar.xz",
        "aarch64--glibc--stable-2024.05-1",
        "aarch64-linux",
        "glibc",
    ),
    "riscv64gc-unknown-linux-gnu": (
        "https://toolchains.bootlin.com/downloads/releases/toolchains/riscv64-lp64d/"
        "tarballs/riscv64-lp64d--glibc--stable-2024.05-1.tar.xz",
        "riscv64-lp64d--glibc--stable-2024.05-1",
        "riscv64-linux",
        "glibc",
    ),
}

CPU_FAMILY = {"x86_64": "x86_64", "aarch64": "aarch64", "riscv64gc": "riscv64"}


def _meson_cmd() -> list[str]:
    """Prefer the `meson` entry point; fall back to the module form."""
    import shutil

    exe = shutil.which("meson")
    if exe:
        return [exe]
    return [sys.executable, "-m", "mesonbuild.mesonmain"]


MESON = _meson_cmd()


def fetch(url: str, dest: Path) -> None:
    if dest.exists():
        return
    print(f"downloading {url}")
    # Use curl: honours proxy env vars (HTTPS_PROXY) that GitHub Actions
    # containers rely on, unlike urllib's direct connection.
    subprocess.run(
        ["curl", "-fsSL", "--retry", "3", "-o", str(dest), url], check=True
    )


def ensure_toolchain(triple: str) -> Path:
    url, dirname, _, _ = TOOLCHAINS[triple]
    tc = TC_DIR / dirname
    if tc.is_dir():
        return tc
    TC_DIR.mkdir(parents=True, exist_ok=True)
    archive = TC_DIR / (dirname + ".tgz")
    fetch(url, archive)
    with tarfile.open(archive) as tf:
        tf.extractall(TC_DIR, filter="data")
    return tc


def ensure_source(name: str, url: str, version: str) -> Path:
    src = SR_DIR / "src"
    src.mkdir(parents=True, exist_ok=True)
    out = src / f"{name}-{version}"
    if out.is_dir():
        return out
    archive = src / Path(url).name
    fetch(url, archive)
    with tarfile.open(archive) as tf:
        tf.extractall(src, filter="data")
    return out


def write_cross_file(triple: str) -> Path:
    _, dirname, gcc, _ = TOOLCHAINS[triple]
    bin_dir = TC_DIR / dirname / "bin"
    arch = triple.split("-")[0]
    CROSS_DIR.mkdir(parents=True, exist_ok=True)
    cf = CROSS_DIR / f"{triple}.ini"
    cf.write_text(
        f"""[binaries]
c = '{bin_dir}/{gcc}-gcc'
cpp = '{bin_dir}/{gcc}-g++'
ar = '{bin_dir}/{gcc}-ar'
strip = '{bin_dir}/{gcc}-strip'
[host_machine]
system = 'linux'
cpu_family = '{CPU_FAMILY[arch]}'
cpu = '{CPU_FAMILY[arch]}'
endian = 'little'
"""
    )
    return cf


def meson_build(src: Path, build: Path, cross: Path, prefix: str, static: bool,
                extra: list[str]) -> None:
    args = (MESON + ["setup", str(build), f"--cross-file={cross}",
                     f"--prefix={prefix}",
                     f"--default-library={'static' if static else 'shared'}"]
            + extra)
    if (build / "build.ninja").exists():
        # --wipe re-reads the cross file (a plain --reconfigure keeps the old
        # toolchain paths and fails when the toolchain changed).
        subprocess.run(args + ["--wipe"], cwd=src, check=True,
                       stdout=subprocess.DEVNULL)
    else:
        subprocess.run(args, cwd=src, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["ninja", "-C", str(build)], check=True,
                   stdout=subprocess.DEVNULL)


def prepare(triple: str) -> Path:
    """Build the sysroot for one target; return its path."""
    _, _, _, libc = TOOLCHAINS[triple]
    libc_tag = "musl" if libc == "musl" else "gnu"
    arch = triple.split("-")[0]
    sysroot = SR_DIR / f"{arch}-{libc_tag}"
    static = libc == "musl"

    tc = ensure_toolchain(triple)
    cross = write_cross_file(triple)
    pixman = ensure_source("pixman", PIXMAN_URL, PIXMAN_VERSION)
    xkb = ensure_source("libxkbcommon", XKBCOMMON_URL, XKBCOMMON_VERSION)

    b = pixman / f"build-{arch}-{libc_tag}"
    prefix = str(sysroot)
    meson_build(pixman, b, cross, prefix, static,
                ["-Dtests=disabled", "-Dgtk=disabled", "-Dlibpng=disabled"])
    subprocess.run(["ninja", "-C", str(b), "install"], check=True,
                   stdout=subprocess.DEVNULL)

    b2 = xkb / f"build-{arch}-{libc_tag}"
    meson_build(xkb, b2, cross, "/usr", static,
                ["-Denable-x11=false", "-Denable-wayland=false",
                 "-Denable-docs=false", "-Denable-tools=false",
                 "-Denable-bash-completion=false", "-Denable-xkbregistry=false"])
    stage = SR_DIR / f"stage-{arch}-{libc_tag}"
    env = dict(os.environ, DESTDIR=str(stage))
    subprocess.run(["ninja", "-C", str(b2), "install"], check=True,
                   stdout=subprocess.DEVNULL, env=env)
    libdir = sysroot / "lib"
    libdir.mkdir(parents=True, exist_ok=True)
    import shutil
    if static:
        shutil.copy2(stage / "usr" / "lib" / "libxkbcommon.a", libdir)
    else:
        for so in sorted((stage / "usr" / "lib").glob("libxkbcommon.so*")):
            shutil.copy2(so, libdir / so.name)
    shutil.rmtree(stage, ignore_errors=True)
    print(f"sysroot ready: {sysroot}")
    return sysroot


def main() -> None:
    targets = sys.argv[1:] or sorted(TOOLCHAINS)
    for t in targets:
        if t not in TOOLCHAINS:
            sys.exit(f"error: unknown target {t!r} (known: {sorted(TOOLCHAINS)})")
        prepare(t)
    print("\nAdd these to your environment before build_compositor.py --target:")
    for t in targets:
        _, _, gcc, libc = TOOLCHAINS[t]
        arch = t.split("-")[0]
        libc_tag = "musl" if libc == "musl" else "gnu"
        env_name = t.upper().replace("-", "_")
        print(f'  export CC_{env_name}={TC_DIR}/{TOOLCHAINS[t][1]}/bin/{gcc}-gcc')
        print(f'  export CARGO_TARGET_{env_name}_LINKER={TC_DIR}/{TOOLCHAINS[t][1]}/bin/{gcc}-gcc')
        print(f'  export RUSTFLAGS="-L {SR_DIR}/{arch}-{libc_tag}/lib"  # set per-target')
        if libc != "musl":
            print(f'  export VWAYLAND_SYSROOT_LIB={SR_DIR}/{arch}-{libc_tag}/lib')


if __name__ == "__main__":
    main()
