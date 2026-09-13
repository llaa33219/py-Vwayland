#!/usr/bin/env python3
"""vwayland-compositor build + in-package bundling script.

What it does:
1. Builds the compositor binary with cargo build --release
2. Copies it to src/vwayland/_native/
3. Finds non-standard shared libraries (libpixman, libxkbcommon, ...) via ldd
   and copies them to src/vwayland/_native/lib/ (the glibc family is taken from
   the system)
4. Sets the binary's RPATH to $ORIGIN/lib with patchelf so the bundled
   libraries are preferred

After this script, src/vwayland/_native/ alone runs headless mode on any Linux
(with a glibc >= the build machine's; glibc 2.28+ recommended).

Usage:
    python3 scripts/build_compositor.py [--no-build]
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
NATIVE_DIR = REPO_ROOT / "src" / "vwayland" / "_native"
LIB_DIR = NATIVE_DIR / "lib"
BINARY_NAME = "vwayland-compositor"

# System libraries that are not bundled (glibc/compiler runtime)
SYSTEM_LIBS = (
    "linux-vdso",
    "ld-linux",
    "ld-musl",
    "libc.so",
    "libm.so",
    "libdl.so",
    "librt.so",
    "libpthread.so",
    "libutil.so",
    "libresolv.so",
    "libgcc_s.so",
    "libstdc++.so",
)

LDD_LINE = re.compile(r"^\s*(\S+)\s+=>?\s*(/\S+)?")


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    print("+", " ".join(str(c) for c in cmd))
    return subprocess.run(cmd, check=True, **kw)


def cargo_build() -> Path:
    cargo = shutil.which("cargo")
    if not cargo:
        sys.exit("error: cargo not found. Install Rust via https://rustup.rs")
    run([cargo, "build", "--release"], cwd=REPO_ROOT / "rust")
    binary = REPO_ROOT / "rust" / "target" / "release" / BINARY_NAME
    if not binary.is_file():
        sys.exit(f"error: build output missing: {binary}")
    return binary


def ldd_paths(binary: Path) -> list[tuple[str, Path]]:
    out = subprocess.run(
        ["ldd", str(binary)], capture_output=True, text=True, check=True
    ).stdout
    deps: list[tuple[str, Path]] = []
    for line in out.splitlines():
        line = line.strip()
        if "not found" in line:
            sys.exit(f"error: unresolved library: {line}")
        m = LDD_LINE.match(line)
        if not m:
            continue
        name, path = m.group(1), m.group(2)
        if path is None:  # linux-vdso etc.
            continue
        deps.append((name, Path(path)))
    return deps


def is_system_lib(name: str) -> bool:
    base = Path(name).name
    return base.startswith(SYSTEM_LIBS)


def bundle(binary: Path) -> None:
    patchelf = shutil.which("patchelf")
    if not patchelf:
        sys.exit(
            "error: patchelf is required (e.g.: sudo apt install patchelf / dnf install patchelf)"
        )

    NATIVE_DIR.mkdir(parents=True, exist_ok=True)
    LIB_DIR.mkdir(parents=True, exist_ok=True)
    for old in LIB_DIR.glob("*.so*"):
        old.unlink()

    dest = NATIVE_DIR / BINARY_NAME
    shutil.copy2(binary, dest)
    dest.chmod(0o755)
    print(f"copied {binary} -> {dest}")

    bundled: list[Path] = []
    for name, path in ldd_paths(dest):
        if is_system_lib(name):
            continue
        target = LIB_DIR / path.name
        shutil.copy2(path, target)
        target.chmod(0o755)
        bundled.append(target)
        print(f"bundled {path} -> {target}")

    if bundled:
        run([patchelf, "--set-rpath", "$ORIGIN/lib", str(dest)])
        for lib in bundled:
            run([patchelf, "--set-rpath", "$ORIGIN", str(lib)])
    else:
        run([patchelf, "--remove-rpath", str(dest)])

    print("\n== bundle verification ==")
    out = subprocess.run(["ldd", str(dest)], capture_output=True, text=True).stdout
    bad = [
        line.strip()
        for line in out.splitlines()
        if "=>" in line and "/_native/lib/" not in line and not is_system_lib(line.split()[0])
    ]
    if bad:
        sys.exit("error: dependencies not resolved by the bundled lib:\n" + "\n".join(bad))
    print(out)
    version = subprocess.run([str(dest), "--version"], capture_output=True, text=True)
    print(version.stdout.strip() or version.stderr.strip())
    print(f"\ndone: {NATIVE_DIR}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--no-build", action="store_true", help="skip cargo build (use the existing release binary)")
    args = ap.parse_args()
    if args.no_build:
        binary = REPO_ROOT / "rust" / "target" / "release" / BINARY_NAME
        if not binary.is_file():
            sys.exit(f"error: {binary} does not exist (run without --no-build)")
    else:
        binary = cargo_build()
    bundle(binary)


if __name__ == "__main__":
    main()
