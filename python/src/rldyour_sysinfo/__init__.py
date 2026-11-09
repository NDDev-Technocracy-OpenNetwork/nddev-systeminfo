"""Client for the rldyour-sysinfo local protocol."""

from __future__ import annotations

import argparse
import json
import os
import socket
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import Any, TypedDict

__version__ = "0.3.11"
MAX_LINE = 4096


class Cpu(TypedDict):
    usage: float | None
    temp: float | None


class Memory(TypedDict):
    used: float | None
    swap: float | None


class Gpu(TypedDict):
    usage: float | None
    memory: float | None
    temp: float | None


class Disk(TypedDict):
    read: int | None
    write: int | None
    temp: float | None


class Net(TypedDict):
    rx: int | None
    tx: int | None


class Sample(TypedDict):
    v: int
    cpu: Cpu
    memory: Memory
    gpu: Gpu
    disk: Disk
    net: Net


def socket_path() -> Path:
    """Return the daemon socket used by the current platform."""
    if sys.platform == "darwin":
        # Application Support, not Caches: Caches is purgeable under disk
        # pressure, which would strand the socket of a live daemon.
        return (
            Path.home()
            / "Library/Application Support/rldyour-sysinfo/rldyour-sysinfo.sock"
        )
    if os.name == "nt":
        root = os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP")
        if not root:
            raise RuntimeError("LOCALAPPDATA and TEMP are unset")
        return Path(root) / "rldyour-sysinfo/rldyour-sysinfo.sock"
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime:
        runtime = f"/run/user/{os.getuid()}"
    return Path(runtime) / "rldyour-sysinfo.sock"


def _decode(line: bytes) -> Sample:
    value: Any = json.loads(line)
    required = {"v", "cpu", "memory", "gpu", "disk", "net"}
    # The version byte is the protocol contract; extra keys inside v1 are a
    # compatible extension, not a different protocol.
    if (not isinstance(value, dict) or not required.issubset(value)
            or type(value.get("v")) is not int or value["v"] != 1
            or any(not isinstance(value[key], dict) for key in required - {"v"})):
        raise ValueError("unsupported rldyour-sysinfo sample")
    return value


def samples(interval: int = 5, path: str | os.PathLike[str] | None = None) -> Iterator[Sample]:
    """Yield live samples until the connection closes.

    `interval` is the cadence in seconds; 0 selects realtime (500 ms ticks).
    """
    if type(interval) is not int or not 0 <= interval <= 60:
        raise ValueError("interval must be between 0 (realtime) and 60 seconds")
    # CPython on Windows does not build socket.AF_UNIX, even though the OS
    # itself speaks it; fail with the reason rather than an AttributeError.
    if not hasattr(socket, "AF_UNIX"):
        raise RuntimeError("this Python build has no AF_UNIX support")
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(Path(path) if path is not None else socket_path()))
        connection.sendall(json.dumps({"interval": interval}).encode() + b"\n")
        with connection.makefile("rb") as stream:
            while line := stream.readline(MAX_LINE + 1):
                if len(line) > MAX_LINE or not line.endswith(b"\n"):
                    raise ValueError("invalid or oversized rldyour-sysinfo frame")
                yield _decode(line)


def main() -> None:
    parser = argparse.ArgumentParser(description="Read live rldyour-sysinfo metrics")
    parser.add_argument(
        "--interval",
        type=int,
        default=5,
        help="cadence in seconds (0 = realtime, 500 ms ticks)",
    )
    parser.add_argument("--once", action="store_true")
    args = parser.parse_args()
    for sample in samples(args.interval):
        print(json.dumps(sample, separators=(",", ":")), flush=True)
        if args.once:
            break
