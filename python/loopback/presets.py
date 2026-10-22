# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

"""Route/preset model and JSON persistence (no audio, no GUI: easy to test)."""

from __future__ import annotations

import json
import os
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

APP_NAME = "Loopback Router"


@dataclass
class RouteConfig:
    source: str
    dest: str
    gain_db: float = 0.0
    mute: bool = False
    src_channels: tuple[int, int] = (0, 1)
    dst_channels: tuple[int, int] = (0, 1)

    @classmethod
    def from_dict(cls, d: dict) -> "RouteConfig":
        return cls(
            source=str(d["source"]),
            dest=str(d["dest"]),
            gain_db=float(d.get("gain_db", 0.0)),
            mute=bool(d.get("mute", False)),
            src_channels=tuple(d.get("src_channels", (0, 1)))[:2],  # type: ignore[arg-type]
            dst_channels=tuple(d.get("dst_channels", (0, 1)))[:2],  # type: ignore[arg-type]
        )


@dataclass
class Preset:
    name: str = "Predefinito"
    latency_ms: float = 20.0
    routes: list[RouteConfig] = field(default_factory=list)

    def to_json(self) -> str:
        d = asdict(self)
        for r in d["routes"]:
            r["src_channels"] = list(r["src_channels"])
            r["dst_channels"] = list(r["dst_channels"])
        return json.dumps(d, indent=2, ensure_ascii=False)

    @classmethod
    def from_json(cls, text: str) -> "Preset":
        d = json.loads(text)
        return cls(
            name=str(d.get("name", "Predefinito")),
            latency_ms=float(d.get("latency_ms", 20.0)),
            routes=[RouteConfig.from_dict(r) for r in d.get("routes", [])],
        )

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_text(self.to_json(), encoding="utf-8")
        os.replace(tmp, path)  # atomic: never leaves a half-written preset

    @classmethod
    def load(cls, path: Path) -> "Preset":
        return cls.from_json(path.read_text(encoding="utf-8"))


def config_dir() -> Path:
    """Per-user settings folder following each OS's convention."""
    if sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    elif sys.platform.startswith("win"):
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming"))
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return base / APP_NAME


def presets_dir() -> Path:
    return config_dir() / "presets"


def last_session_path() -> Path:
    return config_dir() / "last_session.json"


def list_presets() -> list[Path]:
    d = presets_dir()
    return sorted(d.glob("*.json")) if d.exists() else []


def channel_options(n_channels: int) -> list[tuple[str, tuple[int, int]]]:
    """Channel choices for a device: stereo pairs first, then single (mono) channels.

    Labels are 1-based like on audio interfaces; values are 0-based indices.
    """
    n = max(1, int(n_channels))
    opts: list[tuple[str, tuple[int, int]]] = []
    for i in range(0, n - 1, 2):
        opts.append((f"{i + 1}+{i + 2}", (i, i + 1)))
    for i in range(n):
        opts.append((f"{i + 1} mono", (i, i)))
    return opts


def channel_label(pair: tuple[int, int]) -> str:
    a, b = pair
    return f"{a + 1} mono" if a == b else f"{a + 1}+{b + 1}"
