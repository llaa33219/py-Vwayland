"""Low-level compositor IPC client (Unix socket + JSON-line protocol)."""

from __future__ import annotations

import json
import socket
from typing import Any

from .errors import ProtocolError


def _read_header(f) -> dict[str, Any]:
    line = f.readline()
    if not line:
        raise ProtocolError("compositor closed the connection without a response")
    try:
        header = json.loads(line)
    except json.JSONDecodeError as e:
        raise ProtocolError(f"invalid response from compositor: {line[:200]!r}") from e
    if not isinstance(header, dict):
        raise ProtocolError(f"invalid response from compositor: {line[:200]!r}")
    return header


def request(sock_path: str, payload: dict[str, Any], timeout: float = 10.0) -> dict[str, Any]:
    """Send one command and return the JSON response. Raises ProtocolError on ok=False."""
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(timeout)
        s.connect(sock_path)
        s.sendall(json.dumps(payload).encode() + b"\n")
        with s.makefile("rb") as f:
            header = _read_header(f)
    except (OSError, TimeoutError) as e:
        raise ProtocolError(f"ipc request failed: {e}") from e
    finally:
        try:
            s.close()
        except Exception:
            pass
    if not header.get("ok"):
        raise ProtocolError(str(header.get("error", "unknown compositor error")))
    return header


def request_bytes(
    sock_path: str, payload: dict[str, Any], timeout: float = 60.0
) -> "tuple[dict[str, Any], bytes]":
    """For screenshot: receive a JSON header followed by an N-byte payload."""
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(timeout)
        s.connect(sock_path)
        s.sendall(json.dumps(payload).encode() + b"\n")
        with s.makefile("rb") as f:
            header = _read_header(f)
            data = b""
            if header.get("ok"):
                n = int(header.get("bytes", 0))
                while len(data) < n:
                    chunk = f.read(n - len(data))
                    if not chunk:
                        raise ProtocolError(
                            f"truncated payload: expected {n} bytes, got {len(data)}"
                        )
                    data += chunk
    except (OSError, TimeoutError) as e:
        raise ProtocolError(f"ipc request failed: {e}") from e
    finally:
        try:
            s.close()
        except Exception:
            pass
    if not header.get("ok"):
        raise ProtocolError(str(header.get("error", "unknown compositor error")))
    return header, data
