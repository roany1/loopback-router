# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

"""Glue between the preset model and the Rust engine.

Gain and mute are applied live. Changing which devices/channels a route uses
needs the streams to be reopened, so the controller restarts the engine
transparently when it is running.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Callable

from . import _engine
from .presets import Preset, RouteConfig


@dataclass
class RouteState:
    config: RouteConfig
    engine_id: int


class RouterController:
    def __init__(self, preset: Preset | None = None, engine_factory: Callable = _engine.Engine):
        self._factory = engine_factory
        self.preset = preset or Preset()
        self.engine = engine_factory(self.preset.latency_ms)
        self.routes: list[RouteState] = []
        self._rebuild()

    # ---------- device info ----------
    @staticmethod
    def devices():
        return _engine.list_devices()

    @staticmethod
    def host_name() -> str:
        return _engine.host_name()

    # ---------- engine lifecycle ----------
    @property
    def running(self) -> bool:
        return bool(self.engine.running)

    def start(self) -> None:
        self.engine.start()

    def stop(self) -> None:
        self.engine.stop()

    def _rebuild(self) -> None:
        """Recreate engine routes from the preset, keeping run state."""
        was_running = self.running
        self.engine.stop()
        self.engine.clear_routes()
        self.engine.latency_ms = self.preset.latency_ms
        self.routes = []
        for rc in self.preset.routes:
            rid = self.engine.add_route(
                rc.source, rc.dest, rc.gain_db, tuple(rc.src_channels), tuple(rc.dst_channels)
            )
            if rc.mute:
                self.engine.set_mute(rid, True)
            self.routes.append(RouteState(rc, rid))
        if was_running and self.routes:
            self.engine.start()

    # ---------- route editing ----------
    def add_route(self, rc: RouteConfig) -> int:
        self.preset.routes.append(rc)
        self._rebuild()
        return len(self.routes) - 1

    def remove_route(self, index: int) -> None:
        del self.preset.routes[index]
        self._rebuild()

    def update_route(self, index: int, **changes) -> None:
        old = self.preset.routes[index]
        new = replace(old, **changes)
        self.preset.routes[index] = new
        st = self.routes[index]
        structural = (
            new.source != old.source
            or new.dest != old.dest
            or tuple(new.src_channels) != tuple(old.src_channels)
            or tuple(new.dst_channels) != tuple(old.dst_channels)
        )
        if structural:
            self._rebuild()
            return
        st.config = new
        if new.gain_db != old.gain_db:
            self.engine.set_gain(st.engine_id, new.gain_db)
        if new.mute != old.mute:
            self.engine.set_mute(st.engine_id, new.mute)

    def set_latency(self, ms: float) -> None:
        self.preset.latency_ms = float(ms)
        self._rebuild()

    def load_preset(self, preset: Preset) -> None:
        self.preset = preset
        self._rebuild()

    # ---------- monitoring ----------
    def status(self) -> list:
        """Per-route status in the same order as `self.routes`."""
        by_id = {s.id: s for s in self.engine.status()}
        return [by_id.get(r.engine_id) for r in self.routes]

    def take_errors(self) -> list[str]:
        return list(self.engine.take_errors())
