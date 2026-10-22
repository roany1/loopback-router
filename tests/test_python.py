# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

import json

import pytest

import loopback
from loopback import _engine
from loopback.controller import RouterController
from loopback.presets import Preset, RouteConfig, channel_label, channel_options


# ------------------------------------------------------------------ presets
def test_preset_roundtrip(tmp_path):
    p = Preset(name="Stream", latency_ms=10, routes=[
        RouteConfig("Mic", "BlackHole 2ch", gain_db=-3, mute=True, src_channels=(2, 3)),
        RouteConfig("BlackHole 2ch", "Speakers", dst_channels=(0, 0)),
    ])
    f = tmp_path / "x.json"
    p.save(f)
    q = Preset.load(f)
    assert q == p
    assert json.loads(f.read_text())["routes"][0]["src_channels"] == [2, 3]


def test_preset_tolerates_missing_fields():
    q = Preset.from_json('{"routes":[{"source":"a","dest":"b"}]}')
    assert q.latency_ms == 20.0 and q.routes[0].gain_db == 0.0
    assert q.routes[0].src_channels == (0, 1)


def test_channel_options():
    labels = [l for l, _ in channel_options(4)]
    assert labels[:2] == ["1+2", "3+4"]
    assert "4 mono" in labels
    assert dict(channel_options(1)) == {"1 mono": (0, 0)}
    assert channel_label((2, 3)) == "3+4" and channel_label((1, 1)) == "2 mono"


# ------------------------------------------------------------------ controller (fake engine)
class FakeEngine:
    def __init__(self, latency_ms):
        self.latency_ms = latency_ms
        self.routes, self.gains, self.mutes = {}, {}, {}
        self.running, self.starts, self._next = False, 0, 1

    def add_route(self, s, d, g, sc, dc):
        rid = self._next
        self._next += 1
        self.routes[rid] = (s, d, sc, dc)
        self.gains[rid] = g
        return rid

    def clear_routes(self):
        self.routes.clear()

    def set_gain(self, rid, g):
        self.gains[rid] = g

    def set_mute(self, rid, m):
        self.mutes[rid] = m

    def start(self):
        self.running, self.starts = True, self.starts + 1

    def stop(self):
        self.running = False

    def status(self):
        return []

    def take_errors(self):
        return []


def test_controller_live_gain_does_not_restart():
    c = RouterController(Preset(routes=[RouteConfig("a", "b")]), engine_factory=FakeEngine)
    c.start()
    starts = c.engine.starts
    rid = c.routes[0].engine_id
    c.update_route(0, gain_db=-12.0)
    c.update_route(0, mute=True)
    assert c.engine.gains[rid] == -12.0 and c.engine.mutes[rid] is True
    assert c.engine.starts == starts  # no glitch: no restart for gain/mute


def test_controller_structural_change_restarts_when_running():
    c = RouterController(Preset(routes=[RouteConfig("a", "b", mute=True)]), engine_factory=FakeEngine)
    c.start()
    c.update_route(0, dest="c")
    assert c.engine.running and c.engine.starts == 2
    (s, d, _, _), = c.engine.routes.values()
    assert (s, d) == ("a", "c")
    # mute survives the rebuild
    assert c.engine.mutes[c.routes[0].engine_id] is True


def test_controller_add_remove():
    c = RouterController(Preset(), engine_factory=FakeEngine)
    c.add_route(RouteConfig("a", "b"))
    c.add_route(RouteConfig("b", "c"))
    c.remove_route(0)
    assert [r.config.source for r in c.routes] == ["b"]
    assert len(c.engine.routes) == 1


# ------------------------------------------------------------------ real Rust engine
def test_engine_binding_api():
    e = _engine.Engine(latency_ms=15)
    assert e.latency_ms == 15
    rid = e.add_route("x", "y", gain_db=-6.0, src_channels=(0, 1), dst_channels=(2, 3))
    e.set_gain(rid, -1.0)
    e.set_mute(rid, True)
    assert e.route_ids() == [rid]
    st = e.status()[0]
    assert st.id == rid and st.peak_db <= -119.0 and st.underruns == 0
    with pytest.raises(KeyError):
        e.set_gain(999, 0.0)
    with pytest.raises(RuntimeError):
        e.start()  # devices "x"/"y" don't exist -> clear error, no crash
    assert not e.running
    e.remove_route(rid)
    with pytest.raises(RuntimeError, match="Nessuna rotta"):
        e.start()


def test_list_devices_does_not_crash():
    try:
        devs = loopback.list_devices()
    except RuntimeError:
        pytest.skip("nessun backend audio nel container")
    for d in devs:
        assert isinstance(d.name, str) and d.default_rate > 0
    assert isinstance(loopback.host_name(), str)
