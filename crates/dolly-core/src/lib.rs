//! # dolly-core
//!
//! The platform-independent brain of Dolly: turns a raw input-event log into
//! a complete render plan — where the synthetic cursor is and where the
//! virtual camera is looking, for every output frame.
//!
//! Pipeline:
//! ```text
//! events.jsonl ─┬─▶ cursor::solve_cursor_path ──▶ per-frame cursor state
//!               └─▶ camera::generate_segments ──▶ camera::solve_camera ──▶ per-frame crop rect
//! ```
//! The renderer (GPU compositor in the app crate) consumes the plan:
//! crop source frame → draw vector cursor → style frame → encode.
//!
//! Nothing in this crate touches an OS API. It compiles and tests anywhere,
//! which is what lets the camera feel be developed with fast unit-test
//! iteration instead of record-render-squint loops.

pub mod camera;
pub mod cursor;
pub mod events;
pub mod project;
pub mod spring;

use camera::CameraFrame;
use cursor::CursorFrame;

/// The complete per-frame plan the renderer consumes.
#[derive(Debug, Clone)]
pub struct RenderPlan {
    pub frame_times_ms: Vec<f64>,
    pub cursor: Vec<CursorFrame>,
    pub camera: Vec<CameraFrame>,
}

/// Build the render plan for a project's events at the given output fps.
pub fn render_plan(
    events: &[events::RawEvent],
    duration_ms: f64,
    fps: f64,
    screen_w: f64,
    screen_h: f64,
    camera_config: &camera::CameraConfig,
    cursor_config: cursor::CursorConfig,
) -> RenderPlan {
    let n_frames = ((duration_ms / 1000.0) * fps).ceil() as usize + 1;
    let frame_times_ms: Vec<f64> = (0..n_frames).map(|i| i as f64 * 1000.0 / fps).collect();

    let cursor = cursor::solve_cursor_path(events, &frame_times_ms, cursor_config);
    let segments = camera::generate_segments(events, camera_config);
    let camera = camera::solve_camera(&segments, &frame_times_ms, screen_w, screen_h, camera_config);

    RenderPlan { frame_times_ms, cursor, camera }
}

#[cfg(test)]
mod tests {
    use super::*;
    use events::{MouseButton, RawEvent};

    /// End-to-end sanity: a realistic 8-second session produces a coherent plan.
    #[test]
    fn full_pipeline_smoke() {
        let mut events = Vec::new();
        // Glide from top-left to a button at (1200, 700), click it, type a bit.
        for i in 0..100 {
            let f = i as f64 / 99.0;
            events.push(RawEvent::MouseMove {
                t: f * 2000.0,
                x: 100.0 + f * 1100.0,
                y: 100.0 + f * 600.0,
            });
        }
        events.push(RawEvent::MouseDown { t: 2100.0, x: 1200.0, y: 700.0, button: MouseButton::Left });
        events.push(RawEvent::MouseUp { t: 2180.0, x: 1200.0, y: 700.0, button: MouseButton::Left });
        for i in 0..10 {
            events.push(RawEvent::KeyPress { t: 2600.0 + i as f64 * 120.0 });
        }

        let plan = render_plan(
            &events,
            8000.0,
            60.0,
            1920.0,
            1080.0,
            &camera::CameraConfig::default(),
            cursor::CursorConfig::default(),
        );

        assert_eq!(plan.cursor.len(), plan.camera.len());
        assert_eq!(plan.cursor.len(), plan.frame_times_ms.len());

        // Camera pushed in around the click...
        let at = |ms: f64| ((ms / 1000.0) * 60.0) as usize;
        assert!(plan.camera[at(3000.0)].zoom > 1.5, "zoom at click: {}", plan.camera[at(3000.0)].zoom);
        // ...and the zoom center is near the button: the crop rect contains it.
        let f = plan.camera[at(3000.0)];
        assert!(f.x <= 1200.0 && 1200.0 <= f.x + f.w && f.y <= 700.0 && 700.0 <= f.y + f.h);
        // Cursor ended where the mouse ended.
        let last_cursor = plan.cursor.last().unwrap();
        assert!((last_cursor.x - 1200.0).abs() < 2.0);
    }
}
