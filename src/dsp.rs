// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Roany Romano

//! Pure DSP building blocks, independent from audio I/O so they can be unit-tested.
//!
//! Every route connects two devices that run on *independent clocks*: even two
//! devices both nominally at 48 kHz drift apart by tens of ppm. Without
//! compensation the ring buffer between them slowly fills up (crackles from
//! dropped frames) or drains (dropouts). `DriftResampler` does 4-point cubic
//! (Catmull-Rom / Hermite) sample-rate conversion and nudges the conversion ratio with a small
//! proportional controller so the ring-buffer fill level stays around a target.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub type Frame = [f32; 2];

/// Convert decibels to linear gain. Values at or below -90 dB are treated as silence.
pub fn db_to_lin(db: f32) -> f32 {
    if db <= -90.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Convert a linear amplitude to dBFS (floor at -120).
pub fn lin_to_db(lin: f32) -> f32 {
    if lin <= 1e-6 {
        -120.0
    } else {
        20.0 * lin.log10()
    }
}

/// Lock-free controls shared between the UI thread and the audio callbacks.
#[derive(Debug)]
pub struct RouteControl {
    gain_bits: AtomicU32,
    mute: AtomicBool,
    peak_bits: AtomicU32,
    pub underruns: AtomicU64,
    pub overruns: AtomicU64,
    /// Frames discarded to pull an oversized buffer back to the target latency.
    pub dropped: AtomicU64,
    /// Largest block (frames) the source device has delivered in one callback.
    pub max_in_block: AtomicU32,
    /// Current ring-buffer fill in frames (for diagnostics / latency display).
    pub fill_frames: AtomicU32,
    /// Current correction applied by the drift controller, in ppm (signed, stored as i32 bits).
    ppm_bits: AtomicU32,
}

impl RouteControl {
    pub fn new(gain_db: f32) -> Self {
        Self {
            gain_bits: AtomicU32::new(db_to_lin(gain_db).to_bits()),
            mute: AtomicBool::new(false),
            peak_bits: AtomicU32::new(0f32.to_bits()),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            max_in_block: AtomicU32::new(0),
            fill_frames: AtomicU32::new(0),
            ppm_bits: AtomicU32::new(0),
        }
    }
    pub fn set_gain_db(&self, db: f32) {
        self.gain_bits.store(db_to_lin(db).to_bits(), Ordering::Relaxed);
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
    pub fn set_mute(&self, m: bool) {
        self.mute.store(m, Ordering::Relaxed);
    }
    pub fn muted(&self) -> bool {
        self.mute.load(Ordering::Relaxed)
    }
    /// Effective gain used by the audio thread.
    pub fn effective_gain(&self) -> f32 {
        if self.muted() {
            0.0
        } else {
            self.gain()
        }
    }
    /// Raise the held peak (called from the audio thread).
    pub fn report_peak(&self, p: f32) {
        let mut cur = self.peak_bits.load(Ordering::Relaxed);
        while p > f32::from_bits(cur) {
            match self.peak_bits.compare_exchange_weak(
                cur,
                p.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => cur = c,
            }
        }
    }
    /// Read and reset the held peak (called from the UI thread).
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0f32.to_bits(), Ordering::Relaxed))
    }
    pub fn set_ppm(&self, ppm: f32) {
        self.ppm_bits.store((ppm as i32) as u32, Ordering::Relaxed);
    }
    pub fn ppm(&self) -> i32 {
        self.ppm_bits.load(Ordering::Relaxed) as i32
    }
}

/// Linear-interpolating resampler with adaptive ratio for clock-drift compensation.
pub struct DriftResampler {
    nominal_step: f64,
    frac: f64,
    /// Interpolation history: we interpolate between h[1] and h[2].
    h: [Frame; 4],
    target_fill: f64,
    /// Upper bound for the adaptive target (4x the requested latency).
    max_target: f64,
    smoothed_fill: f64,
    primed: bool,
    /// Maximum correction (relative). 0.001 = ±1000 ppm (~1.7 cents), far above real clock drift.
    max_corr: f64,
    /// Proportional gain of the fill-level controller.
    kp: f64,
    correction: f64,
}

impl DriftResampler {
    pub fn new(src_rate: u32, dst_rate: u32, target_fill_frames: usize) -> Self {
        Self {
            nominal_step: src_rate as f64 / dst_rate as f64,
            frac: 0.0,
            h: [[0.0; 2]; 4],
            target_fill: target_fill_frames.max(1) as f64,
            max_target: (target_fill_frames.max(1) * 4) as f64,
            smoothed_fill: target_fill_frames as f64,
            primed: false,
            max_corr: 0.001,
            kp: 0.0001,
            correction: 0.0,
        }
    }

    pub fn is_primed(&self) -> bool {
        self.primed
    }

    /// Make sure the target can absorb the host's block granularity: the buffer
    /// must hold at least one full input block plus what one output callback
    /// consumes, with margin. Called every output callback; cheap.
    pub fn ensure_min_target(&mut self, in_block: usize, out_block_src_frames: usize) {
        let need = (in_block as f64) * 1.5 + out_block_src_frames as f64;
        if need > self.target_fill {
            self.target_fill = need;
            self.max_target = self.max_target.max(need * 2.0);
        }
    }

    /// Current (possibly auto-raised) target buffer in frames.
    pub fn target_frames(&self) -> usize {
        self.target_fill as usize
    }

    /// How many source frames `n` output frames consume.
    pub fn src_frames_for(&self, n: usize) -> usize {
        (n as f64 * self.nominal_step).ceil() as usize
    }

    pub fn correction_ppm(&self) -> f64 {
        self.correction * 1e6
    }

    /// Frames to discard right away when the buffer holds far more than the
    /// target (e.g. a host that delivers a large first block, or a producer burst
    /// after a stall). Dropping once is better than minutes of pitch correction.
    pub fn excess_frames(&self, fill: usize) -> usize {
        let t = self.target_fill as usize;
        if fill > t * 2 {
            fill - t
        } else {
            0
        }
    }

    /// Call once per output callback with the current ring fill (frames).
    /// Returns `true` if the resampler is ready to produce audio.
    pub fn update_fill(&mut self, fill: usize) -> bool {
        if !self.primed {
            // Wait until we have buffered the target latency before starting:
            // starting on an empty buffer would underrun immediately.
            if fill as f64 >= self.target_fill {
                self.primed = true;
                self.smoothed_fill = fill as f64;
                self.frac = 0.0;
            } else {
                return false;
            }
        }
        // Slow exponential smoothing: callback jitter must not modulate the pitch.
        self.smoothed_fill += 0.02 * (fill as f64 - self.smoothed_fill);
        let err = (self.smoothed_fill - self.target_fill) / self.target_fill;
        self.correction = (self.kp * err * 10.0).clamp(-self.max_corr, self.max_corr);
        true
    }

    /// Produce one output frame, pulling input frames from `pop` as needed.
    /// Returns `None` on underrun (the resampler then re-primes).
    #[inline]
    pub fn next_frame<F: FnMut() -> Option<Frame>>(&mut self, pop: &mut F) -> Option<Frame> {
        if !self.primed {
            return None;
        }
        let t = self.frac as f32;
        let h = &self.h;
        let mut out = [0.0f32; 2];
        for c in 0..2 {
            let (y0, y1, y2, y3) = (h[0][c], h[1][c], h[2][c], h[3][c]);
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
            out[c] = ((c3 * t + c2) * t + c1) * t + y1;
        }
        self.frac += self.nominal_step * (1.0 + self.correction);
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            match pop() {
                Some(f) => {
                    self.h = [self.h[1], self.h[2], self.h[3], f];
                }
                None => {
                    // Underrun: the host delivers blocks too coarse for this latency.
                    // Raise the target so it doesn't happen again (adaptive safety).
                    self.primed = false;
                    self.frac = 0.0;
                    self.target_fill = (self.target_fill * 1.25).min(self.max_target);
                    return None;
                }
            }
        }
        Some(out)
    }
}

/// Soft limiter used on the final mix so summed routes never hard-clip harshly.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    // Linear up to 0.9, then a smooth tanh knee approaching 1.0.
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        let y = 0.9 + 0.1 * ((a - 0.9) / 0.1).tanh();
        y.copysign(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn db_roundtrip() {
        assert!((db_to_lin(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_lin(-6.0) - 0.501).abs() < 1e-3);
        assert_eq!(db_to_lin(-100.0), 0.0);
        assert!((lin_to_db(db_to_lin(-12.0)) + 12.0).abs() < 1e-3);
    }

    #[test]
    fn peak_hold_and_reset() {
        let c = RouteControl::new(0.0);
        c.report_peak(0.3);
        c.report_peak(0.7);
        c.report_peak(0.5);
        assert!((c.take_peak() - 0.7).abs() < 1e-6);
        assert_eq!(c.take_peak(), 0.0);
    }

    #[test]
    fn mute_zeroes_gain() {
        let c = RouteControl::new(-6.0);
        c.set_mute(true);
        assert_eq!(c.effective_gain(), 0.0);
        c.set_mute(false);
        assert!(c.effective_gain() > 0.4);
    }

    #[test]
    fn soft_clip_is_bounded_and_transparent() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert!(soft_clip(10.0) <= 1.0);
        assert!(soft_clip(-10.0) >= -1.0);
        assert!(soft_clip(0.95) < 0.95 && soft_clip(0.95) > 0.9);
    }

    /// Same rate: after priming, output must reproduce the input (delayed by one frame).
    #[test]
    fn unity_ratio_passthrough() {
        let mut q: VecDeque<Frame> = (0..200).map(|i| [i as f32, -(i as f32)]).collect();
        let mut r = DriftResampler::new(48000, 48000, 200);
        assert!(r.update_fill(q.len()));
        let mut pop = || q.pop_front();
        let mut out = vec![];
        for _ in 0..50 {
            out.push(r.next_frame(&mut pop).unwrap());
        }
        // Three frames of interpolation history, then the stream follows sample-exactly.
        for (k, f) in out.iter().enumerate().skip(3) {
            assert!((f[0] - (k as f32 - 3.0)).abs() < 1e-4, "k={k} f={f:?}");
        }
    }

    #[test]
    fn waits_for_priming_and_reprimes_on_underrun() {
        let mut r = DriftResampler::new(48000, 48000, 64);
        assert!(!r.update_fill(10));
        let mut q: VecDeque<Frame> = (0..64).map(|_| [0.1, 0.1]).collect();
        assert!(r.update_fill(q.len()));
        let mut pop = || q.pop_front();
        let mut n = 0;
        while r.next_frame(&mut pop).is_some() {
            n += 1;
        }
        assert!(n >= 63 && n <= 65, "produced {n}");
        assert!(!r.is_primed());
        assert_eq!(r.target_frames(), 80); // target raised by 25% after the underrun
        // ...but never beyond 4x the requested latency
        for _ in 0..20 {
            let mut q: VecDeque<Frame> = (0..2000).map(|_| [0.0, 0.0]).collect();
            r.update_fill(q.len());
            let mut pop = || q.pop_front();
            while r.next_frame(&mut pop).is_some() {}
        }
        assert_eq!(r.target_frames(), 256);
    }

    /// 44.1 kHz -> 48 kHz: a sine must come out at the same frequency.
    #[test]
    fn rate_conversion_preserves_frequency() {
        let (sr_in, sr_out, f0) = (44100.0f64, 48000.0f64, 1000.0f64);
        let n_in = 44100;
        let mut q: VecDeque<Frame> = (0..n_in)
            .map(|i| {
                let s = (2.0 * std::f64::consts::PI * f0 * i as f64 / sr_in).sin() as f32;
                [s, s]
            })
            .collect();
        let mut r = DriftResampler::new(sr_in as u32, sr_out as u32, 1000);
        r.update_fill(q.len());
        let mut pop = || q.pop_front();
        let mut out = vec![];
        while let Some(f) = r.next_frame(&mut pop) {
            out.push(f[0]);
        }
        // Expected output length ≈ n_in * 48000/44100 (correction is clamped tiny).
        let expected = n_in as f64 * sr_out / sr_in;
        assert!((out.len() as f64 - expected).abs() / expected < 0.003);
        // Count zero crossings over the middle part to estimate frequency.
        let seg = &out[1000..out.len() - 1000];
        let zc = seg.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64;
        let est = zc * sr_out / seg.len() as f64;
        assert!((est - f0).abs() < 3.0, "estimated {est} Hz");
    }

    /// Quality: 1 kHz at 44.1k -> 48k must have THD+N well below -80 dB.
    #[test]
    fn cubic_interpolation_is_clean() {
        let (sr_in, sr_out, f0) = (44100.0f64, 48000.0f64, 1000.0f64);
        let mut q: VecDeque<Frame> = (0..44100)
            .map(|i| {
                let s = (0.5 * (2.0 * std::f64::consts::PI * f0 * i as f64 / sr_in).sin()) as f32;
                [s, s]
            })
            .collect();
        let mut r = DriftResampler::new(sr_in as u32, sr_out as u32, q.len());
        r.update_fill(q.len()); // fill == target -> zero correction
        let mut pop = || q.pop_front();
        let mut y = vec![];
        while let Some(f) = r.next_frame(&mut pop) {
            y.push(f[0] as f64);
        }
        // Least-squares fit of the ideal sine (amplitude/phase) on a middle segment,
        // residual = THD+N.
        let seg = &y[2000..40000];
        let w = 2.0 * std::f64::consts::PI * f0 / sr_out;
        let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (n, &v) in seg.iter().enumerate() {
            let (s, c) = (w * n as f64).sin_cos();
            ss += s * s; cc += c * c; sc += s * c; ys += v * s; yc += v * c;
        }
        let det = ss * cc - sc * sc;
        let a = (ys * cc - yc * sc) / det;
        let b = (yc * ss - ys * sc) / det;
        let (mut e2, mut s2) = (0.0, 0.0);
        for (n, &v) in seg.iter().enumerate() {
            let (s, c) = (w * n as f64).sin_cos();
            let ideal = a * s + b * c;
            e2 += (v - ideal).powi(2);
            s2 += ideal * ideal;
        }
        let thdn = 10.0 * (e2 / s2).log10();
        assert!(thdn < -80.0, "THD+N {thdn:.1} dB");
    }

    #[test]
    fn target_follows_host_block_size() {
        let mut r = DriftResampler::new(48000, 48000, 240); // 5 ms requested
        r.ensure_min_target(1024, 512); // host delivers 1024-frame blocks
        assert_eq!(r.target_frames(), 2048);
        r.ensure_min_target(256, 256); // smaller blocks never lower it
        assert_eq!(r.target_frames(), 2048);
    }

    #[test]
    fn excess_is_dropped_only_when_far_above_target() {
        let r = DriftResampler::new(48000, 48000, 960);
        assert_eq!(r.excess_frames(1500), 0);
        assert_eq!(r.excess_frames(3000), 3000 - 960);
    }

    /// Simulate a producer clock 300 ppm faster than nominal: the controller must
    /// keep the buffer bounded around the target instead of growing forever.
    #[test]
    fn drift_controller_keeps_fill_bounded() {
        let target = 960usize; // 20 ms at 48k
        let mut r = DriftResampler::new(48000, 48000, target);
        let mut q: VecDeque<Frame> = VecDeque::new();
        let producer_rate = 48000.0 * (1.0 + 300e-6);
        let block = 256usize;
        let mut produced_acc = 0.0f64;
        let mut max_fill = 0usize;
        let mut fills = vec![];
        // ~60 s of simulated audio
        for blk in 0..(48000 * 60 / block) {
            produced_acc += producer_rate * block as f64 / 48000.0;
            while produced_acc >= 1.0 {
                q.push_back([0.0, 0.0]);
                produced_acc -= 1.0;
            }
            let ready = r.update_fill(q.len());
            if ready {
                let mut pop = || q.pop_front();
                for _ in 0..block {
                    if r.next_frame(&mut pop).is_none() {
                        break;
                    }
                }
            }
            if blk > 48000 * 30 / block {
                max_fill = max_fill.max(q.len());
                fills.push(q.len());
            }
        }
        let avg = fills.iter().sum::<usize>() as f64 / fills.len() as f64;
        // Without compensation 300 ppm over 60 s would add ~864 frames.
        assert!(max_fill < target * 2, "max fill {max_fill}");
        assert!((avg - target as f64).abs() < target as f64 * 0.6, "avg {avg}");
        assert!(r.correction_ppm() > 100.0, "ppm {}", r.correction_ppm());
    }
}
