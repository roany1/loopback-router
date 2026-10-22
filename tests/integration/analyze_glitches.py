# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano

"""Detect clicks/dropouts in a recorded 1 kHz sine, sample by sample.

A clean sine obeys x[n+1] = 2·cos(ω)·x[n] − x[n−1]; any discontinuity (dropped or
repeated samples, buffer underrun) shows up as a large prediction error.
Exit code 1 if more glitches than allowed are found.
"""

import argparse
import sys
import wave

import numpy as np


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("wav")
    p.add_argument("--freq", type=float, default=1000.0)
    p.add_argument("--max-glitches", type=int, default=0)
    a = p.parse_args()

    w = wave.open(a.wav)
    sr = w.getframerate()
    x = np.frombuffer(w.readframes(w.getnframes()), np.int16).reshape(-1, w.getnchannels())[:, 0]
    x = x / 32768.0
    nz = np.flatnonzero(np.abs(x) > 0.01)
    if len(nz) < sr:
        print("FAIL: nessun segnale registrato")
        return 1
    x = x[nz[0] + sr // 4 : nz[-1] - sr // 4]  # skip fade-in/out edges
    wv = 2 * np.pi * a.freq / sr
    err = np.abs(x[2:] - (2 * np.cos(wv) * x[1:-1] - x[:-2]))
    events, last = 0, -999
    for i in np.flatnonzero(err > 0.01):
        if i - last > 50:
            events += 1
        last = i
    amp = np.percentile(np.abs(x), 99.9)
    floor = 20 * np.log10(np.median(err) / amp)
    print(f"{len(x) / sr:.1f} s di audio | glitch: {events} | rumore di fondo {floor:.1f} dB rel. segnale")
    ok = events <= a.max_glitches and floor < -70
    print("OK" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
