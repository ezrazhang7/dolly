//! Windows.Graphics.Capture handler: frames go straight into the
//! hardware-accelerated Media Foundation encoder, cursor excluded.

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;

/// Passed into [`VideoHandler::new`] via `Settings` flags (runs on the
/// capture thread, hence the owned path).
pub(crate) struct VideoFlags {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub output_path: String,
}

pub(crate) struct VideoHandler {
    /// `Option` because `finish()` consumes the encoder; `WinRecorder::stop`
    /// takes it out through `CaptureControl::callback()` after the capture
    /// thread has joined.
    encoder: Option<VideoEncoder>,
    /// First frame's `SystemRelativeTime` in 100 ns units. The encoder
    /// rebases video time to this instant, so it is the epoch every input
    /// event must be measured against.
    pub first_ts_100ns: Option<i64>,
    pub last_ts_100ns: Option<i64>,
    pub frames: u64,
}

impl VideoHandler {
    pub fn take_encoder(&mut self) -> Option<VideoEncoder> {
        self.encoder.take()
    }
}

impl GraphicsCaptureApiHandler for VideoHandler {
    type Flags = VideoFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = &ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(f.width, f.height)
                .frame_rate(f.fps)
                .bitrate(f.bitrate),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &f.output_path,
        )?;
        Ok(Self { encoder: Some(encoder), first_ts_100ns: None, last_ts_100ns: None, frames: 0 })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let ts = frame.timestamp()?.Duration;
        if self.first_ts_100ns.is_none() {
            self.first_ts_100ns = Some(ts);
        }
        self.last_ts_100ns = Some(ts);
        self.frames += 1;
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame(frame)?;
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        // Monitor capture: only fires if the display itself goes away.
        // stop() still finalizes whatever was encoded up to this point.
        Ok(())
    }
}
