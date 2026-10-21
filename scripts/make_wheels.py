#!/usr/bin/env python3
"""Repack the pure-python wheel into per-platform wheels.

``python -m build`` produces one ``py3-none-any`` wheel containing the bundled
``_native/<tag>/`` directories. This script splits it into one wheel per
bundled platform, each tagged correctly so pip refuses to install it on the
wrong platform (instead of installing and crashing at runtime).

It is pure-stdlib: wheels are zip files; only the ``.dist-info/WHEEL`` tag and
``RECORD`` change, plus the wheel file name.

Usage:
    python3 -m build --wheel          # produce dist/py_vwayland-*-py3-none-any.whl
    python3 scripts/make_wheels.py    # write per-platform wheels next to it
"""

from __future__ import annotations

import base64
import csv
import hashlib
import io
import re
import sys
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DIST = REPO_ROOT / "dist"

NATIVE_PREFIX = "vwayland/_native/"


def urlsafe_b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def record_hash(data: bytes) -> str:
    return "sha256=" + urlsafe_b64(hashlib.sha256(data).digest())


def platform_tags(native_names: list[str]) -> list[str]:
    """Directory names under _native/ that hold a compositor binary."""
    tags = []
    for name in native_names:
        parts = name.split("/")
        # vwayland/_native/<tag>/vwayland-compositor
        if len(parts) >= 3 and parts[-1] == "vwayland-compositor":
            tags.append(parts[2])
    return sorted(set(tags))


def repack(src: Path) -> list[Path]:
    zin = zipfile.ZipFile(src)
    names = zin.namelist()
    tags = platform_tags(names)
    if not tags:
        sys.exit(f"error: no bundled compositor found under {NATIVE_PREFIX} in {src.name}")
    print(f"bundled platforms in {src.name}: {tags}")

    # Split members into shared (python code, metadata) and per-platform native.
    def is_native(n: str) -> bool:
        return n.startswith(NATIVE_PREFIX)

    shared = [n for n in names if not is_native(n)]

    written: list[Path] = []
    for tag in tags:
        keep_native = [
            n for n in names
            if is_native(n) and n.split("/")[2] == tag
        ]
        out_name = re.sub(r"-py3-none-any\.whl$", f"-py3-none-{tag}.whl", src.name)
        if out_name == src.name:
            sys.exit(f"error: {src.name} is not a py3-none-any wheel")
        out_path = src.with_name(out_name)

        dist_info = next(n for n in shared if n.endswith(".dist-info/WHEEL"))
        record_name = dist_info.rsplit("/", 1)[0] + "/RECORD"

        records: list[tuple[str, str, str]] = []
        with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED) as zout:
            for n in shared + keep_native:
                info = zin.getinfo(n)
                data = zin.read(n)
                if n.endswith(".dist-info/WHEEL"):
                    data = re.sub(
                        r"^Tag:.*$",
                        f"Tag: py3-none-{tag}",
                        data.decode("utf-8"),
                        flags=re.M,
                    ).encode("utf-8")
                    data = re.sub(
                        r"^Root-Is-Purelib:.*$",
                        "Root-Is-Purelib: false",
                        data.decode("utf-8"),
                        flags=re.M,
                    ).encode("utf-8")
                if n == record_name:
                    continue  # written last
                zi = zipfile.ZipInfo(n, date_time=info.date_time)
                zi.compress_type = zipfile.ZIP_DEFLATED
                zi.external_attr = info.external_attr  # keep +x on binaries/libs
                zout.writestr(zi, data)
                records.append((n, record_hash(data), str(len(data))))
            records.append((record_name, "", ""))
            buf = io.StringIO()
            csv.writer(buf, lineterminator="\n").writerows(records)
            zout.writestr(record_name, buf.getvalue())
        written.append(out_path)
        print(f"wrote {out_path.name}")
    return written


def main() -> None:
    wheels = sorted(DIST.glob("*-py3-none-any.whl"))
    if not wheels:
        sys.exit("error: no py3-none-any wheel in dist/ (run `python3 -m build --wheel`)")
    for w in wheels:
        repack(w)


if __name__ == "__main__":
    main()
