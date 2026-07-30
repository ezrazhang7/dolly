//! Real Windows backend. Implementation order (see lib.rs skeleton note):
//! 1. ✅ Graphics Capture session → mp4 via windows-capture's VideoEncoder,
//!    `CursorCaptureSettings::WithoutCursor`, `DrawBorderSettings::WithoutBorder`.
//! 2. ✅ WH_MOUSE_LL / WH_KEYBOARD_LL hook thread → `Vec<RawEvent>`, QPC clock,
//!    rebased to the first video frame's timestamp (see `timebase` + `input`).
//! 3. ✅ Write the `<video>.events.jsonl` sidecar beside the mp4.
//!
//! Later: `WindowFocus` events via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` —
//! the schema already has the variant, nothing here blocks on it.
//!
//! ## Known limitations (documented, not solved — every competitor shares them)
//! - **Elevated windows:** LL hooks in a non-elevated process don't receive
//!   input aimed at elevated (admin) windows — those stretches record video
//!   with no cursor/click events, so no auto-zoom there. UIPI by design;
//!   running Dolly elevated would lift it, we don't ask users to.
//! - **UAC / secure desktop:** Windows.Graphics.Capture can't see the secure
//!   desktop, so frames blank (or freeze) for the duration of a UAC prompt,
//!   Ctrl+Alt+Del, or the lock screen. Recording resumes on its own.

mod input;
mod video;

use std::path::PathBuf;

use dolly_core::events::{to_jsonl, RecordingMeta};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows_capture::capture::{CaptureControl, GraphicsCaptureApiHandler, GraphicsCaptureApiError};
use windows_capture::graphics_capture_api;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::timebase::ticks_to_ms;
use crate::{Recorder, RecordingArtifacts};
use input::InputHook;
use video::{VideoFlags, VideoHandler};

type HandlerError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug)]
pub enum WinCaptureError {
    AlreadyStarted,
    NotStarted,
    /// Cursor-less capture needs Windows 10 2004+ (build 19041).
    OsTooOld,
    Monitor(String),
    Capture(String),
    Encoder(String),
    /// Low-level input hooks could not be installed.
    Input(String),
    /// Capture stopped before a single frame arrived.
    NoFrames,
    /// The events.jsonl sidecar could not be written.
    Sidecar(String),
}

/// Records one monitor, cursor excluded, straight to an hardware-encoded mp4.
///
/// v0 scope: monitor capture only. Window capture resizes mid-recording and
/// fights the fixed-size encoder; the virtual camera already crops, so
/// "capture the monitor, zoom to the window" produces the same output.
pub struct WinRecorder {
    /// Where the mp4 lands.
    pub output_path: PathBuf,
    /// `None` = primary monitor.
    pub monitor_index: Option<usize>,
    pub fps: u32,
    pub bitrate: u32,
    session: Option<Session>,
}

/// Everything captured at `start()` that `stop()` needs to build the meta.
struct Session {
    control: CaptureControl<VideoHandler, HandlerError>,
    input: InputHook,
    width: u32,
    height: u32,
    scale_factor: f64,
    /// Captured monitor's top-left in virtual-screen physical pixels, so hook
    /// coordinates (also virtual-screen global) can be mapped to surface space.
    origin: (i32, i32),
}

impl WinRecorder {
    pub fn new(output_path: impl Into<PathBuf>) -> Self {
        Self {
            output_path: output_path.into(),
            monitor_index: None,
            fps: 60,
            bitrate: 15_000_000,
            session: None,
        }
    }
}

/// Top-left of the monitor in virtual-screen physical pixels (per-monitor-v2
/// awareness is set in `start`, so this is unvirtualized). `(0, 0)` on failure —
/// correct for the common single-primary-monitor case.
fn monitor_origin(monitor: &Monitor) -> (i32, i32) {
    let hmonitor = HMONITOR(monitor.as_raw_hmonitor());
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(hmonitor, &mut info) }.as_bool() {
        let RECT { left, top, .. } = info.rcMonitor;
        (left, top)
    } else {
        (0, 0)
    }
}

fn monitor_scale_factor(monitor: &Monitor) -> f64 {
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // Meaningful only because we set per-monitor-v2 awareness in start();
    // otherwise the system lies to unaware processes.
    let hmonitor = HMONITOR(monitor.as_raw_hmonitor());
    if unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        return 1.0;
    }
    dpi_x as f64 / 96.0
}

impl Recorder for WinRecorder {
    type Error = WinCaptureError;

    fn start(&mut self) -> Result<(), WinCaptureError> {
        if self.session.is_some() {
            return Err(WinCaptureError::AlreadyStarted);
        }

        // Idempotent per process; fails once awareness is already set (e.g.
        // by a manifest or a previous call) — that's fine, ignore it.
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

        let monitor = match self.monitor_index {
            Some(i) => Monitor::from_index(i),
            None => Monitor::primary(),
        }
        .map_err(|e| WinCaptureError::Monitor(format!("{e}")))?;

        let width = monitor.width().map_err(|e| WinCaptureError::Monitor(format!("{e}")))?;
        let height = monitor.height().map_err(|e| WinCaptureError::Monitor(format!("{e}")))?;
        let scale_factor = monitor_scale_factor(&monitor);
        let origin = monitor_origin(&monitor);

        let try_start = |border| {
            let settings = Settings::new(
                monitor,
                // Hard requirement (Win10 2004+): the OS cursor must stay out
                // of the frames — Dolly re-renders it from the input log.
                CursorCaptureSettings::WithoutCursor,
                border,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                VideoFlags {
                    width,
                    height,
                    fps: self.fps,
                    bitrate: self.bitrate,
                    output_path: self.output_path.display().to_string(),
                },
            );
            VideoHandler::start_free_threaded(settings)
        };

        // Border suppression needs Win11 (build 20348+). On Win10 the system
        // border is drawn on-screen only, never into the captured frames, so
        // falling back to Default is cosmetic — the recording is identical.
        let control = match try_start(DrawBorderSettings::WithoutBorder) {
            Err(GraphicsCaptureApiError::GraphicsCaptureApiError(
                graphics_capture_api::Error::BorderConfigUnsupported,
            )) => try_start(DrawBorderSettings::Default),
            other => other,
        }
        .map_err(|e| match e {
            GraphicsCaptureApiError::GraphicsCaptureApiError(
                graphics_capture_api::Error::CursorConfigUnsupported
                | graphics_capture_api::Error::Unsupported,
            ) => WinCaptureError::OsTooOld,
            other => WinCaptureError::Capture(format!("{other}")),
        })?;

        // Hooks after the encoder is live: pre-first-frame events get dropped by
        // the epoch rebase anyway, and this way a hook-install failure tears the
        // capture back down instead of leaking a running encoder thread.
        let input = match InputHook::start() {
            Ok(input) => input,
            Err(e) => {
                let _ = control.stop();
                return Err(WinCaptureError::Input(format!("{e}")));
            }
        };

        self.session = Some(Session { control, input, width, height, scale_factor, origin });
        Ok(())
    }

    fn stop(&mut self) -> Result<RecordingArtifacts, WinCaptureError> {
        let Session { control, input, width, height, scale_factor, origin } =
            self.session.take().ok_or(WinCaptureError::NotStarted)?;

        // Keep a handle to the handler: stop() joins the capture thread and
        // consumes the control, and the encoder must be finished *after* the
        // last frame has been sent.
        let handler = control.callback();
        control.stop().map_err(|e| WinCaptureError::Capture(format!("{e}")))?;

        let mut handler = handler.lock();
        let encoder = handler.take_encoder().ok_or(WinCaptureError::NoFrames)?;
        encoder.finish().map_err(|e| WinCaptureError::Encoder(format!("{e}")))?;

        let epoch = handler.first_ts_100ns.ok_or(WinCaptureError::NoFrames)?;
        let last = handler.last_ts_100ns.unwrap_or(epoch);
        drop(handler);

        // Stop the hooks and rebase every event onto the first-frame epoch.
        let events = input.finalize(epoch, origin, (width, height));

        // Sidecar convention: `<video stem>.events.jsonl` beside the mp4, so
        // the pair is discoverable from the video path alone and two
        // recordings in one directory never share a sidecar.
        let sidecar = self.output_path.with_extension("events.jsonl");
        std::fs::write(&sidecar, to_jsonl(&events))
            .map_err(|e| WinCaptureError::Sidecar(format!("{}: {e}", sidecar.display())))?;

        Ok(RecordingArtifacts {
            video_path: self.output_path.display().to_string(),
            meta: RecordingMeta {
                width,
                height,
                fps: self.fps as f64,
                scale_factor,
                // Timeline length = span of encoded frames. The mp4's t=0 is
                // the first frame (the encoder rebases), so `epoch` is also
                // what input-event timestamps are rebased against.
                duration_ms: ticks_to_ms(last, epoch),
            },
            events,
        })
    }
}
