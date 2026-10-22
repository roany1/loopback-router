// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Roany Romano

//! Audio engine: opens one input stream per source device and one output stream
//! per destination device, connected by lock-free ring buffers (one per route).
//! Each output callback mixes all routes that target its device.

use crate::dsp::{lin_to_db, soft_clip, DriftResampler, Frame, RouteControl};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub is_input: bool,
    pub is_output: bool,
    pub input_channels: u16,
    pub output_channels: u16,
    pub default_rate: u32,
    pub is_virtual: bool,
}

/// Names of well-known virtual loopback drivers (used to highlight them in the UI).
const VIRTUAL_HINTS: &[&str] = &[
    "blackhole",
    "cable",
    "vb-audio",
    "voicemeeter",
    "loopback",
    "soundflower",
    "monitor of",
];

fn is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    VIRTUAL_HINTS.iter().any(|h| n.contains(h))
}

pub fn host_name() -> String {
    cpal::default_host().id().to_string()
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    let host = cpal::default_host();
    let mut map: BTreeMap<String, DeviceInfo> = BTreeMap::new();
    let ins = host.input_devices().map_err(|e| e.to_string())?;
    for d in ins {
        let name = d.to_string();
        if let Ok(cfg) = d.default_input_config() {
            let e = map.entry(name.clone()).or_insert_with(|| DeviceInfo {
                is_virtual: is_virtual(&name),
                name,
                is_input: false,
                is_output: false,
                input_channels: 0,
                output_channels: 0,
                default_rate: cfg.sample_rate(),
            });
            e.is_input = true;
            e.input_channels = cfg.channels();
        }
    }
    let outs = host.output_devices().map_err(|e| e.to_string())?;
    for d in outs {
        let name = d.to_string();
        if let Ok(cfg) = d.default_output_config() {
            let e = map.entry(name.clone()).or_insert_with(|| DeviceInfo {
                is_virtual: is_virtual(&name),
                name,
                is_input: false,
                is_output: false,
                input_channels: 0,
                output_channels: 0,
                default_rate: cfg.sample_rate(),
            });
            e.is_output = true;
            e.output_channels = cfg.channels();
        }
    }
    Ok(map.into_values().collect())
}

pub fn default_devices() -> (Option<String>, Option<String>) {
    let host = cpal::default_host();
    (
        host.default_input_device().map(|d| d.to_string()),
        host.default_output_device().map(|d| d.to_string()),
    )
}

#[derive(Clone, Debug)]
pub struct RouteSpec {
    pub id: u32,
    pub source: String,
    pub dest: String,
    /// Source channels feeding left / right (0-based).
    pub src_ch: [u16; 2],
    /// Destination channels receiving left / right (0-based).
    pub dst_ch: [u16; 2],
}

pub struct Route {
    pub spec: RouteSpec,
    pub ctrl: Arc<RouteControl>,
}

#[derive(Clone, Debug)]
pub struct RouteStatus {
    pub id: u32,
    pub peak_db: f32,
    pub underruns: u64,
    pub overruns: u64,
    pub dropped: u64,
    pub latency_ms: f32,
    pub drift_ppm: i32,
}

/// Per-route state owned by an input callback.
struct InSink {
    prod: HeapProd<Frame>,
    src_ch: [usize; 2],
    ctrl: Arc<RouteControl>,
}

/// Per-route state owned by an output callback.
struct OutSource {
    cons: HeapCons<Frame>,
    rs: DriftResampler,
    dst_ch: [usize; 2],
    ctrl: Arc<RouteControl>,
}

pub struct Engine {
    routes: Vec<Route>,
    next_id: u32,
    streams: Vec<cpal::Stream>,
    errors: Arc<Mutex<Vec<String>>>,
    /// Per-route source sample rate, used to convert fill to milliseconds.
    src_rates: BTreeMap<u32, u32>,
    pub latency_ms: f32,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            routes: vec![],
            next_id: 1,
            streams: vec![],
            errors: Arc::new(Mutex::new(vec![])),
            src_rates: BTreeMap::new(),
            latency_ms: 20.0,
        }
    }

    pub fn add_route(
        &mut self,
        source: &str,
        dest: &str,
        gain_db: f32,
        src_ch: [u16; 2],
        dst_ch: [u16; 2],
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.routes.push(Route {
            spec: RouteSpec {
                id,
                source: source.to_string(),
                dest: dest.to_string(),
                src_ch,
                dst_ch,
            },
            ctrl: Arc::new(RouteControl::new(gain_db)),
        });
        id
    }

    pub fn remove_route(&mut self, id: u32) -> bool {
        let n = self.routes.len();
        self.routes.retain(|r| r.spec.id != id);
        n != self.routes.len()
    }

    pub fn clear_routes(&mut self) {
        self.routes.clear();
    }

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    fn ctrl(&self, id: u32) -> Option<&Arc<RouteControl>> {
        self.routes.iter().find(|r| r.spec.id == id).map(|r| &r.ctrl)
    }

    pub fn set_gain(&self, id: u32, db: f32) -> bool {
        self.ctrl(id).map(|c| c.set_gain_db(db)).is_some()
    }

    pub fn set_mute(&self, id: u32, m: bool) -> bool {
        self.ctrl(id).map(|c| c.set_mute(m)).is_some()
    }

    pub fn is_running(&self) -> bool {
        !self.streams.is_empty()
    }

    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.errors.lock().unwrap())
    }

    pub fn status(&self) -> Vec<RouteStatus> {
        self.routes
            .iter()
            .map(|r| {
                let sr = *self.src_rates.get(&r.spec.id).unwrap_or(&48000) as f32;
                RouteStatus {
                    id: r.spec.id,
                    peak_db: lin_to_db(r.ctrl.take_peak()),
                    underruns: r.ctrl.underruns.load(Ordering::Relaxed),
                    overruns: r.ctrl.overruns.load(Ordering::Relaxed),
                    dropped: r.ctrl.dropped.load(Ordering::Relaxed),
                    latency_ms: r.ctrl.fill_frames.load(Ordering::Relaxed) as f32 * 1000.0 / sr,
                    drift_ppm: r.ctrl.ppm(),
                }
            })
            .collect()
    }

    pub fn stop(&mut self) {
        // Dropping the streams stops them and releases the devices.
        self.streams.clear();
    }

    pub fn start(&mut self) -> Result<(), String> {
        self.stop();
        if self.routes.is_empty() {
            return Err("Nessuna rotta configurata".into());
        }
        let host = cpal::default_host();

        // Resolve devices and their default configs.
        let find = |name: &str, input: bool| -> Result<cpal::Device, String> {
            let mut it = if input {
                host.input_devices().map_err(|e| e.to_string())?.collect::<Vec<_>>()
            } else {
                host.output_devices().map_err(|e| e.to_string())?.collect::<Vec<_>>()
            }
            .into_iter();
            it.find(|d| d.to_string() == name).ok_or_else(|| {
                format!(
                    "Dispositivo {} non trovato: {name}",
                    if input { "di ingresso" } else { "di uscita" }
                )
            })
        };

        let mut in_sinks: BTreeMap<String, Vec<InSink>> = BTreeMap::new();
        let mut out_sources: BTreeMap<String, Vec<OutSource>> = BTreeMap::new();
        let mut in_cfgs: BTreeMap<String, (cpal::Device, cpal::SupportedStreamConfig)> =
            BTreeMap::new();
        let mut out_cfgs: BTreeMap<String, (cpal::Device, cpal::SupportedStreamConfig)> =
            BTreeMap::new();

        for r in &self.routes {
            if !in_cfgs.contains_key(&r.spec.source) {
                let d = find(&r.spec.source, true)?;
                let c = d.default_input_config().map_err(|e| e.to_string())?;
                in_cfgs.insert(r.spec.source.clone(), (d, c));
            }
            if !out_cfgs.contains_key(&r.spec.dest) {
                let d = find(&r.spec.dest, false)?;
                let c = d.default_output_config().map_err(|e| e.to_string())?;
                out_cfgs.insert(r.spec.dest.clone(), (d, c));
            }
        }

        self.src_rates.clear();
        for r in &self.routes {
            let src_rate = in_cfgs[&r.spec.source].1.sample_rate();
            let dst_rate = out_cfgs[&r.spec.dest].1.sample_rate();
            let target = ((src_rate as f32) * self.latency_ms / 1000.0).max(64.0) as usize;
            // Capacity: 1 s (or 8x target). Some hosts (PulseAudio, Bluetooth, WASAPI
            // shared mode) deliver a large first block; excess is trimmed by the
            // resampler, but it must never overflow.
            let cap = (target * 8).max(src_rate as usize);
            let (prod, cons) = HeapRb::<Frame>::new(cap).split();
            r.ctrl.underruns.store(0, Ordering::Relaxed);
            r.ctrl.overruns.store(0, Ordering::Relaxed);
            r.ctrl.dropped.store(0, Ordering::Relaxed);
            r.ctrl.max_in_block.store(0, Ordering::Relaxed);
            self.src_rates.insert(r.spec.id, src_rate);
            in_sinks.entry(r.spec.source.clone()).or_default().push(InSink {
                prod,
                src_ch: [r.spec.src_ch[0] as usize, r.spec.src_ch[1] as usize],
                ctrl: r.ctrl.clone(),
            });
            out_sources.entry(r.spec.dest.clone()).or_default().push(OutSource {
                cons,
                rs: DriftResampler::new(src_rate, dst_rate, target),
                dst_ch: [r.spec.dst_ch[0] as usize, r.spec.dst_ch[1] as usize],
                ctrl: r.ctrl.clone(),
            });
        }

        let mut streams = vec![];
        // Build outputs first so they're ready to pull when inputs start pushing.
        for (name, sources) in out_sources {
            let (dev, sc) = out_cfgs.remove(&name).unwrap();
            let s = build_output(&dev, sc, sources, self.errors.clone(), &name)?;
            streams.push(s);
        }
        for (name, sinks) in in_sinks {
            let (dev, sc) = in_cfgs.remove(&name).unwrap();
            let s = build_input(&dev, sc, sinks, self.errors.clone(), &name)?;
            streams.push(s);
        }
        for s in &streams {
            s.play().map_err(|e| e.to_string())?;
        }
        self.streams = streams;
        Ok(())
    }
}

fn err_cb(errors: Arc<Mutex<Vec<String>>>, dev: String) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e| {
        if let Ok(mut v) = errors.lock() {
            if v.len() < 100 {
                v.push(format!("{dev}: {e}"));
            }
        }
    }
}

macro_rules! dispatch_format {
    ($fmt:expr, $func:ident, $($arg:expr),*) => {
        match $fmt {
            SampleFormat::F32 => $func::<f32>($($arg),*),
            SampleFormat::F64 => $func::<f64>($($arg),*),
            SampleFormat::I16 => $func::<i16>($($arg),*),
            SampleFormat::I32 => $func::<i32>($($arg),*),
            SampleFormat::I8 => $func::<i8>($($arg),*),
            SampleFormat::U8 => $func::<u8>($($arg),*),
            SampleFormat::U16 => $func::<u16>($($arg),*),
            SampleFormat::U32 => $func::<u32>($($arg),*),
            SampleFormat::I24 => $func::<cpal::I24>($($arg),*),
            other => Err(format!("Formato campioni non supportato: {other:?}")),
        }
    };
}

fn build_input(
    dev: &cpal::Device,
    sc: cpal::SupportedStreamConfig,
    sinks: Vec<InSink>,
    errors: Arc<Mutex<Vec<String>>>,
    name: &str,
) -> Result<cpal::Stream, String> {
    let fmt = sc.sample_format();
    let cfg: StreamConfig = sc.config();
    dispatch_format!(fmt, build_input_t, dev, cfg, sinks, errors, name)
}

fn build_input_t<T>(
    dev: &cpal::Device,
    cfg: StreamConfig,
    mut sinks: Vec<InSink>,
    errors: Arc<Mutex<Vec<String>>>,
    name: &str,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let ch = cfg.channels as usize;
    for s in sinks.iter_mut() {
        s.src_ch = [s.src_ch[0].min(ch - 1), s.src_ch[1].min(ch - 1)];
    }
    dev.build_input_stream::<T, _, _>(
        cfg,
        move |data: &[T], _| {
            for s in sinks.iter_mut() {
                let frames = data.len() / ch;
                s.ctrl.max_in_block.fetch_max(frames as u32, Ordering::Relaxed);
                let free = s.prod.vacant_len();
                let n = frames.min(free);
                if n < frames {
                    s.ctrl
                        .overruns
                        .fetch_add((frames - n) as u64, Ordering::Relaxed);
                }
                for f in 0..n {
                    let base = f * ch;
                    let l: f32 = data[base + s.src_ch[0]].to_sample();
                    let r: f32 = data[base + s.src_ch[1]].to_sample();
                    let _ = s.prod.try_push([l, r]);
                }
            }
        },
        err_cb(errors, name.to_string()),
        None,
    )
    .map_err(|e| format!("{name}: {e}"))
}

fn build_output(
    dev: &cpal::Device,
    sc: cpal::SupportedStreamConfig,
    sources: Vec<OutSource>,
    errors: Arc<Mutex<Vec<String>>>,
    name: &str,
) -> Result<cpal::Stream, String> {
    let fmt = sc.sample_format();
    let cfg: StreamConfig = sc.config();
    dispatch_format!(fmt, build_output_t, dev, cfg, sources, errors, name)
}

fn build_output_t<T>(
    dev: &cpal::Device,
    cfg: StreamConfig,
    mut sources: Vec<OutSource>,
    errors: Arc<Mutex<Vec<String>>>,
    name: &str,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let ch = cfg.channels as usize;
    for s in sources.iter_mut() {
        s.dst_ch = [s.dst_ch[0].min(ch - 1), s.dst_ch[1].min(ch - 1)];
    }
    // Preallocated mix bus: no allocation in the callback unless a host suddenly
    // asks for a much larger buffer than usual.
    let mut mix: Vec<f32> = vec![0.0; 8192 * ch];
    dev.build_output_stream::<T, _, _>(
        cfg,
        move |out: &mut [T], _| {
            if mix.len() < out.len() {
                mix.resize(out.len(), 0.0);
            }
            let mix = &mut mix[..out.len()];
            mix.fill(0.0);
            let frames = out.len() / ch;
            for s in sources.iter_mut() {
                let in_block = s.ctrl.max_in_block.load(Ordering::Relaxed) as usize;
                s.rs.ensure_min_target(in_block, s.rs.src_frames_for(frames));
                let mut fill = s.cons.occupied_len();
                let excess = s.rs.excess_frames(fill);
                if excess > 0 {
                    let n = s.cons.skip(excess);
                    fill -= n;
                    s.ctrl.dropped.fetch_add(n as u64, Ordering::Relaxed);
                }
                s.ctrl.fill_frames.store(fill as u32, Ordering::Relaxed);
                if !s.rs.update_fill(fill) {
                    continue; // still buffering
                }
                s.ctrl.set_ppm(s.rs.correction_ppm() as f32);
                let g = s.ctrl.effective_gain();
                let mut peak = 0f32;
                let cons = &mut s.cons;
                let mut pop = || cons.try_pop();
                for f in 0..frames {
                    match s.rs.next_frame(&mut pop) {
                        Some([l, r]) => {
                            let (l, r) = (l * g, r * g);
                            peak = peak.max(l.abs()).max(r.abs());
                            let base = f * ch;
                            if s.dst_ch[1] != s.dst_ch[0] {
                                mix[base + s.dst_ch[0]] += l;
                                mix[base + s.dst_ch[1]] += r;
                            } else {
                                // Mono destination: average L/R instead of doubling level.
                                mix[base + s.dst_ch[0]] += 0.5 * (l + r);
                            }
                        }
                        None => {
                            s.ctrl.underruns.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                s.ctrl.report_peak(peak);
            }
            for (o, m) in out.iter_mut().zip(mix.iter()) {
                *o = T::from_sample(soft_clip(*m));
            }
        },
        err_cb(errors, name.to_string()),
        None,
    )
    .map_err(|e| format!("{name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_management() {
        let mut e = Engine::new();
        let a = e.add_route("Mic", "BlackHole 2ch", 0.0, [0, 1], [0, 1]);
        let b = e.add_route("BlackHole 2ch", "Speakers", -6.0, [0, 1], [0, 1]);
        assert_ne!(a, b);
        assert!(e.set_gain(a, -3.0));
        assert!(e.set_mute(b, true));
        assert!(!e.set_gain(99, 0.0));
        assert!(e.remove_route(a));
        assert_eq!(e.routes().len(), 1);
        assert!(!e.is_running());
    }

    #[test]
    fn start_without_routes_errors() {
        let mut e = Engine::new();
        assert!(e.start().is_err());
    }

    #[test]
    fn missing_device_reports_clear_error() {
        let mut e = Engine::new();
        e.add_route("Dispositivo-inesistente-xyz", "altro", 0.0, [0, 1], [0, 1]);
        let err = e.start().unwrap_err();
        assert!(err.contains("non trovato") || !err.is_empty(), "{err}");
        assert!(!e.is_running());
    }

    #[test]
    fn virtual_detection() {
        assert!(is_virtual("BlackHole 2ch"));
        assert!(is_virtual("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(!is_virtual("MacBook Pro Speakers"));
    }
}
