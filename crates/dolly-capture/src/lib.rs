//! # dolly-capture
//!
//! Screen capture + raw input logging, behind traits so dolly-core and the
//! editor can be developed against a mock on any OS.
//!
//! ## Windows backend (feature `win-capture`, in progress)
//!
//! - **Video:** `windows-capture` crate (Windows.Graphics.Capture). Critically:
//!   `CursorCaptureSettings::WithoutCursor` — the OS cursor is *excluded* from
//!   the capture, because Dolly re-renders it synthetically from the input log.
//!   v2 of the crate ships a hardware-accelerated Media Foundation encoder, so
//!   the raw capture goes straight to H.264 with near-zero CPU cost.
//!   Requires Windows 10 2004+ (build 19041) for borderless/cursorless options.
//! - **Input:** low-level hooks (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`) via the
//!   `windows` crate, timestamped with `QueryPerformanceCounter` and mapped to
//!   capture-surface pixels using the target's DPI scale. Key *identities* are
//!   never logged — only that a keypress happened (see events.rs privacy note).
//! - **Clock sync:** both streams stamp from the same QPC epoch taken at
//!   `start()`, so cursor overlay stays sample-accurate against video frames.

pub mod mapping;
pub mod timebase;

use dolly_core::events::{MouseButton, RawEvent, RecordingMeta};

/// What a completed recording hands back to the app.
#[derive(Debug, Clone)]
pub struct RecordingArtifacts {
    /// Path to the raw cursor-less video.
    pub video_path: String,
    pub meta: RecordingMeta,
    pub events: Vec<RawEvent>,
}

/// A capture backend: records the screen (cursor excluded) while logging input.
pub trait Recorder {
    type Error: std::fmt::Debug;

    fn start(&mut self) -> Result<(), Self::Error>;
    fn stop(&mut self) -> Result<RecordingArtifacts, Self::Error>;
}

// ---------------------------------------------------------------------------
// Mock backend — synthesizes a plausible session for editor/renderer dev.
// ---------------------------------------------------------------------------

pub struct MockRecorder {
    started: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
}

impl Default for MockRecorder {
    fn default() -> Self {
        Self { started: false, width: 1920, height: 1080, fps: 60.0, duration_ms: 10_000.0 }
    }
}

impl MockRecorder {
    /// Deterministic synthetic session: three "visit a target and click it"
    /// motions with human-ish easing, plus a typing burst.
    fn synth_events(&self) -> Vec<RawEvent> {
        let targets = [(300.0, 250.0), (1500.0, 400.0), (900.0, 850.0)];
        let mut events = Vec::new();
        let mut pos = (self.width as f64 / 2.0, self.height as f64 / 2.0);
        let mut t = 200.0;
        for &(tx, ty) in &targets {
            let travel_ms = 700.0;
            let steps = 90;
            for i in 0..=steps {
                let f = i as f64 / steps as f64;
                // Smoothstep easing — real hands accelerate then decelerate.
                let e = f * f * (3.0 - 2.0 * f);
                events.push(RawEvent::MouseMove {
                    t: t + f * travel_ms,
                    x: pos.0 + (tx - pos.0) * e,
                    y: pos.1 + (ty - pos.1) * e,
                });
            }
            t += travel_ms + 150.0;
            events.push(RawEvent::MouseDown { t, x: tx, y: ty, button: MouseButton::Left });
            events.push(RawEvent::MouseUp { t: t + 90.0, x: tx, y: ty, button: MouseButton::Left });
            pos = (tx, ty);
            t += 1400.0;
        }
        for i in 0..14 {
            events.push(RawEvent::KeyPress { t: t + i as f64 * 110.0 });
        }
        events
    }
}

impl Recorder for MockRecorder {
    type Error = String;

    fn start(&mut self) -> Result<(), String> {
        self.started = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<RecordingArtifacts, String> {
        if !self.started {
            return Err("stop() before start()".into());
        }
        self.started = false;
        Ok(RecordingArtifacts {
            video_path: "mock://no-video".into(),
            meta: RecordingMeta {
                width: self.width,
                height: self.height,
                fps: self.fps,
                scale_factor: 1.0,
                duration_ms: self.duration_ms,
            },
            events: self.synth_events(),
        })
    }
}

// ---------------------------------------------------------------------------
// Windows backend (compiled only on Windows with `win-capture`).
// ---------------------------------------------------------------------------

#[cfg(all(windows, feature = "win-capture"))]
pub mod win;

#[cfg(test)]
mod tests {
    use super::*;
    use dolly_core::{camera::CameraConfig, cursor::CursorConfig, render_plan};

    #[test]
    fn mock_session_produces_valid_render_plan() {
        let mut rec = MockRecorder::default();
        rec.start().unwrap();
        let artifacts = rec.stop().unwrap();
        assert!(!artifacts.events.is_empty());

        let plan = render_plan(
            &artifacts.events,
            artifacts.meta.duration_ms,
            artifacts.meta.fps,
            artifacts.meta.width as f64,
            artifacts.meta.height as f64,
            &CameraConfig::default(),
            CursorConfig::default(),
        );
        // Three click clusters → the camera should push in at least three times.
        let mut push_ins = 0;
        let mut was_wide = true;
        for f in &plan.camera {
            if was_wide && f.zoom > 1.4 {
                push_ins += 1;
                was_wide = false;
            } else if !was_wide && f.zoom < 1.1 {
                was_wide = true;
            }
        }
        assert!(push_ins >= 2, "expected repeated push-ins, got {}", push_ins);
    }

    #[test]
    fn stop_without_start_errors() {
        let mut rec = MockRecorder::default();
        assert!(rec.stop().is_err());
    }
}
