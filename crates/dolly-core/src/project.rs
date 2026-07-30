//! The `.dolly` project format.
//!
//! A Dolly project is a folder:
//! ```text
//! my-demo.dolly/
//! ├── project.json     ← this struct
//! ├── capture.mp4      ← raw screen capture, cursor EXCLUDED
//! └── events.jsonl     ← raw input log (events.rs schema)
//! ```
//! Raw capture is immutable after recording; every edit is non-destructive
//! data in `project.json`. Re-export at any time, any style.

use crate::camera::{CameraConfig, ZoomSegment};
use crate::cursor::CursorConfig;
use crate::events::RecordingMeta;
use serde::{Deserialize, Serialize};

pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub meta: RecordingMeta,
    /// Auto-generated segments the user can tweak/delete, plus manual ones.
    /// `None` = regenerate from events with `camera` config on open.
    pub segments: Option<Vec<ZoomSegment>>,
    pub camera: CameraSettings,
    pub cursor: CursorSettings,
    pub style: StyleSettings,
    /// Keep-ranges in ms; empty = keep everything. (Trims/cuts.)
    pub keep_ranges: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraSettings {
    pub enabled: bool,
    #[serde(flatten)]
    pub config: CameraConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorSettings {
    /// Cursor render scale (1.0 = native size).
    pub scale: f64,
    /// Hide the cursor when idle.
    pub auto_hide: bool,
    /// Spring frequency for cursor smoothing.
    pub omega: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleSettings {
    /// Padding around the screen frame, as a fraction of output size.
    pub inset: f64,
    /// Corner radius of the screen frame in output px.
    pub corner_radius: f64,
    /// Background: "gradient:<name>", "color:#RRGGBB", or "image:<path>".
    pub background: String,
    pub shadow: bool,
}

impl Project {
    pub fn new(meta: RecordingMeta) -> Self {
        Self {
            version: PROJECT_VERSION,
            meta,
            segments: None,
            camera: CameraSettings { enabled: true, config: CameraConfig::default() },
            cursor: CursorSettings { scale: 1.4, auto_hide: true, omega: CursorConfig::default().omega },
            style: StyleSettings {
                inset: 0.06,
                corner_radius: 16.0,
                background: "gradient:dusk".to_string(),
                shadow: true,
            },
            keep_ranges: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serializes")
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> RecordingMeta {
        RecordingMeta { width: 1920, height: 1080, fps: 60.0, scale_factor: 1.0, duration_ms: 10_000.0 }
    }

    #[test]
    fn project_json_roundtrip() {
        let p = Project::new(meta());
        let p2 = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(p, p2);
    }
}
