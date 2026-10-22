# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

"""Entry point: `loopback` (GUI), `loopback --list`, `loopback --preset file.json --headless`."""

from __future__ import annotations

import argparse
import signal
import sys
import time
from pathlib import Path


def _list() -> int:
    from . import default_devices, host_name, list_devices

    d_in, d_out = default_devices()
    print(f"Host audio: {host_name()}\n")
    for d in list_devices():
        tags = []
        if d.is_input:
            tags.append(f"IN {d.input_channels}ch")
        if d.is_output:
            tags.append(f"OUT {d.output_channels}ch")
        if d.is_virtual:
            tags.append("virtuale")
        if d.name == d_in:
            tags.append("default in")
        if d.name == d_out:
            tags.append("default out")
        print(f"  {d.name:<45} {d.default_rate:>6} Hz  [{', '.join(tags)}]")
    return 0


def _headless(preset_path: Path) -> int:
    from .controller import RouterController
    from .presets import Preset

    ctl = RouterController(Preset.load(preset_path))
    ctl.start()
    print(f"Routing attivo ({len(ctl.routes)} rotte). Ctrl+C per uscire.")
    stop = False

    def _sig(*_):
        nonlocal stop
        stop = True

    signal.signal(signal.SIGINT, _sig)
    while not stop:
        time.sleep(1.0)
        for e in ctl.take_errors():
            print("errore:", e, file=sys.stderr)
    ctl.stop()
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="loopback", description="Loopback Router")
    from . import __copyright__, __version__

    p.add_argument("--version", action="version",
                   version=f"Loopback Router {__version__} · {__copyright__} · Apache-2.0")
    p.add_argument("--list", action="store_true", help="elenca i dispositivi audio ed esce")
    p.add_argument("--preset", type=Path, help="preset JSON da caricare")
    p.add_argument("--headless", action="store_true", help="avvia il routing senza interfaccia")
    a = p.parse_args(argv)
    if a.list:
        return _list()
    if a.headless:
        if not a.preset:
            p.error("--headless richiede --preset")
        return _headless(a.preset)
    from .gui import run

    run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
