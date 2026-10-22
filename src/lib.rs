// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Roany Romano

//! loopback_engine — Rust audio routing core exposed to Python as `loopback._engine`.

pub mod dsp;
pub mod engine;

#[cfg(feature = "python")]
mod py {
    use crate::engine;
    use pyo3::exceptions::{PyKeyError, PyRuntimeError};
    use pyo3::prelude::*;

    #[pyclass(get_all, frozen, skip_from_py_object, module = "loopback._engine")]
    #[derive(Clone)]
    pub struct DeviceInfo {
        name: String,
        is_input: bool,
        is_output: bool,
        input_channels: u16,
        output_channels: u16,
        default_rate: u32,
        is_virtual: bool,
    }

    #[pymethods]
    impl DeviceInfo {
        fn __repr__(&self) -> String {
            format!(
                "DeviceInfo(name={:?}, in={}, out={}, rate={}, virtual={})",
                self.name, self.input_channels, self.output_channels, self.default_rate, self.is_virtual
            )
        }
    }

    #[pyclass(get_all, frozen, skip_from_py_object, module = "loopback._engine")]
    #[derive(Clone)]
    pub struct RouteStatus {
        id: u32,
        peak_db: f32,
        underruns: u64,
        overruns: u64,
        dropped: u64,
        latency_ms: f32,
        drift_ppm: i32,
    }

    /// Return all audio devices of the system's default host (CoreAudio / WASAPI / ALSA).
    #[pyfunction]
    fn list_devices() -> PyResult<Vec<DeviceInfo>> {
        let v = engine::list_devices().map_err(PyRuntimeError::new_err)?;
        Ok(v
            .into_iter()
            .map(|d| DeviceInfo {
                name: d.name,
                is_input: d.is_input,
                is_output: d.is_output,
                input_channels: d.input_channels,
                output_channels: d.output_channels,
                default_rate: d.default_rate,
                is_virtual: d.is_virtual,
            })
            .collect())
    }

    /// (default_input_name, default_output_name)
    #[pyfunction]
    fn default_devices() -> (Option<String>, Option<String>) {
        engine::default_devices()
    }

    #[pyfunction]
    fn host_name() -> String {
        engine::host_name()
    }

    /// cpal::Stream is not Send on every platform, so the engine stays on the
    /// thread that created it (the GUI thread).
    #[pyclass(unsendable, module = "loopback._engine")]
    pub struct Engine {
        inner: engine::Engine,
    }

    #[pymethods]
    impl Engine {
        #[new]
        #[pyo3(signature = (latency_ms = 20.0))]
        fn new(latency_ms: f32) -> Self {
            let mut inner = engine::Engine::new();
            inner.latency_ms = latency_ms.clamp(3.0, 500.0);
            Self { inner }
        }

        #[getter]
        fn latency_ms(&self) -> f32 {
            self.inner.latency_ms
        }

        /// Takes effect at the next start().
        #[setter]
        fn set_latency_ms(&mut self, v: f32) {
            self.inner.latency_ms = v.clamp(3.0, 500.0);
        }

        #[pyo3(signature = (source, dest, gain_db = 0.0, src_channels = (0, 1), dst_channels = (0, 1)))]
        fn add_route(
            &mut self,
            source: &str,
            dest: &str,
            gain_db: f32,
            src_channels: (u16, u16),
            dst_channels: (u16, u16),
        ) -> u32 {
            self.inner.add_route(
                source,
                dest,
                gain_db,
                [src_channels.0, src_channels.1],
                [dst_channels.0, dst_channels.1],
            )
        }

        fn remove_route(&mut self, id: u32) -> PyResult<()> {
            if self.inner.remove_route(id) {
                Ok(())
            } else {
                Err(PyKeyError::new_err(id))
            }
        }

        fn clear_routes(&mut self) {
            self.inner.clear_routes();
        }

        /// Live: applies immediately, also while running.
        fn set_gain(&self, id: u32, gain_db: f32) -> PyResult<()> {
            if self.inner.set_gain(id, gain_db) {
                Ok(())
            } else {
                Err(PyKeyError::new_err(id))
            }
        }

        /// Live: applies immediately, also while running.
        fn set_mute(&self, id: u32, mute: bool) -> PyResult<()> {
            if self.inner.set_mute(id, mute) {
                Ok(())
            } else {
                Err(PyKeyError::new_err(id))
            }
        }

        fn route_ids(&self) -> Vec<u32> {
            self.inner.routes().iter().map(|r| r.spec.id).collect()
        }

        /// Open the devices and start routing. Release the GIL while the OS opens devices.
        fn start(&mut self, py: Python<'_>) -> PyResult<()> {
            let inner = &mut self.inner;
            // Device opening can take a moment on CoreAudio/WASAPI; the GIL is not needed.
            let res = py.detach(|| inner.start());
            res.map_err(PyRuntimeError::new_err)
        }

        fn stop(&mut self) {
            self.inner.stop();
        }

        #[getter]
        fn running(&self) -> bool {
            self.inner.is_running()
        }

        /// Per-route meters and diagnostics. Peaks are reset on every call.
        fn status(&self) -> Vec<RouteStatus> {
            self.inner
                .status()
                .into_iter()
                .map(|s| RouteStatus {
                    id: s.id,
                    peak_db: s.peak_db,
                    underruns: s.underruns,
                    overruns: s.overruns,
                    dropped: s.dropped,
                    latency_ms: s.latency_ms,
                    drift_ppm: s.drift_ppm,
                })
                .collect()
        }

        /// Stream errors reported by the OS since the last call (e.g. device unplugged).
        fn take_errors(&self) -> Vec<String> {
            self.inner.take_errors()
        }
    }

    #[pymodule]
    fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<DeviceInfo>()?;
        m.add_class::<RouteStatus>()?;
        m.add_class::<Engine>()?;
        m.add_function(wrap_pyfunction!(list_devices, m)?)?;
        m.add_function(wrap_pyfunction!(default_devices, m)?)?;
        m.add_function(wrap_pyfunction!(host_name, m)?)?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
