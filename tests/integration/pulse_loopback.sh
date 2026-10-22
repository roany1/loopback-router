#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano
# Real-clock integration test on Linux, using PulseAudio null sinks as stand-ins
# for BlackHole / VB-CABLE:
#   sox 1 kHz sine @44.1k -> sink appA -> [Loopback Router] -> sink appB @48k -> parec
# Then checks the recording sample by sample for clicks/dropouts.
# Needs: pulseaudio pulseaudio-utils libasound2-plugins sox, numpy, loopback installed.
set -euo pipefail
LAT="${1:-20}"
T="$(mktemp -d)"
pulseaudio --check 2>/dev/null || pulseaudio -D --exit-idle-time=-1
pactl load-module module-null-sink sink_name=appA rate=44100 >/dev/null
pactl load-module module-null-sink sink_name=appB rate=48000 >/dev/null
sox -n -r 44100 -c 2 -b 16 "$T/sine.wav" synth 14 sine 1000 vol 0.5
PULSE_SINK=appA paplay "$T/sine.wav" & sleep 0.3
parec -d appB.monitor --rate=48000 --channels=2 --format=s16le --file-format=wav "$T/out.wav" &
REC=$!
PULSE_SOURCE=appA.monitor PULSE_SINK=appB python - "$LAT" <<'PY'
import sys, time
from loopback import _engine, list_devices
dev = next(d.name for d in list_devices() if "PulseAudio sound server" in d.name)
e = _engine.Engine(latency_ms=float(sys.argv[1])); e.add_route(dev, dev); e.start()
time.sleep(12); s = e.status()[0]
print(f"latenza {s.latency_ms:.1f} ms, drift {s.drift_ppm:+d} ppm, underrun {s.underruns}, overrun {s.overruns}")
e.stop()
PY
sleep 0.5; kill $REC; sleep 0.3
python "$(dirname "$0")/analyze_glitches.py" "$T/out.wav" --max-glitches 1
