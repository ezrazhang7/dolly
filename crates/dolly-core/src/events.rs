//! Raw input event schema.
//!
//! During recording, Dolly captures the screen with the OS cursor *excluded*
//! and logs every input event to a sidecar `events.jsonl`. The cursor you see
//! in the final export is synthetic — re-rendered from this log with spring
//! smoothing. This is the architectural move that separates Screen
//! Studio-quality output from "screen recorder with a zoom filter."
//!
//! Timestamps are milliseconds since recording start (f64 for sub-ms input
//! APIs). Coordinates are physical pixels in the captured surface's space.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RawEvent {
    /// High-frequency pointer position (typically 125–1000 Hz).
    MouseMove { t: f64, x: f64, y: f64 },
    MouseDown { t: f64, x: f64, y: f64, button: MouseButton },
    MouseUp { t: f64, x: f64, y: f64, button: MouseButton },
    /// Scroll. `dy` in native wheel deltas (sign: positive = up).
    Wheel { t: f64, x: f64, y: f64, dx: f64, dy: f64 },
    /// A keypress (key identity deliberately NOT stored by default —
    /// privacy: we only need *that* typing happened, to zoom on the caret
    /// region, never *what* was typed).
    KeyPress { t: f64 },
    /// Foreground window changed; rect of the new window if known.
    WindowFocus { t: f64, x: f64, y: f64, w: f64, h: f64 },
}

impl RawEvent {
    pub fn t(&self) -> f64 {
        match *self {
            RawEvent::MouseMove { t, .. }
            | RawEvent::MouseDown { t, .. }
            | RawEvent::MouseUp { t, .. }
            | RawEvent::Wheel { t, .. }
            | RawEvent::KeyPress { t }
            | RawEvent::WindowFocus { t, .. } => t,
        }
    }
}

/// Metadata written once per recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingMeta {
    /// Captured surface size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Nominal capture frame rate.
    pub fps: f64,
    /// Display scale factor at record time (Windows DPI scaling), so logged
    /// cursor coords can be mapped onto captured pixels correctly.
    pub scale_factor: f64,
    /// Duration in ms (filled at stop).
    pub duration_ms: f64,
}

/// Serialize events to JSONL (one event per line).
pub fn to_jsonl(events: &[RawEvent]) -> String {
    let mut out = String::new();
    for e in events {
        out.push_str(&serde_json::to_string(e).expect("event serializes"));
        out.push('\n');
    }
    out
}

/// Parse JSONL, skipping blank/corrupt lines (a crashed recording should
/// still open — we salvage what we can).
pub fn from_jsonl(s: &str) -> Vec<RawEvent> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_roundtrip() {
        let events = vec![
            RawEvent::MouseMove { t: 0.0, x: 10.0, y: 20.0 },
            RawEvent::MouseDown { t: 5.5, x: 10.0, y: 20.0, button: MouseButton::Left },
            RawEvent::KeyPress { t: 9.0 },
        ];
        let s = to_jsonl(&events);
        assert_eq!(from_jsonl(&s), events);
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let s = "{\"type\":\"key_press\",\"t\":1.0}\ngarbage\n\n{\"type\":\"mouse_move\",\"t\":2.0,\"x\":1.0,\"y\":2.0}\n";
        let events = from_jsonl(s);
        assert_eq!(events.len(), 2);
    }
}
