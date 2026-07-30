//! The virtual camera.
//!
//! Two stages:
//!
//! 1. **Keyframe generation** — scan the event log for moments that deserve
//!    attention (clicks, typing bursts) and emit `ZoomSegment`s: "from t₀ to
//!    t₁, the camera should be pushed in at zoom Z centered near (x, y)."
//!    Nearby clicks cluster into one segment so triple-clicking a word doesn't
//!    strobe the camera. Idle time pulls back to the wide shot.
//!
//! 2. **Solving** — turn segments into a per-frame crop rectangle by chasing
//!    segment targets with critically damped springs, then clamping the rect
//!    inside the screen. The clamp runs *after* the spring so the camera can
//!    lean toward an edge click and settle flush against the boundary instead
//!    of jittering.
//!
//! The renderer consumes `CameraFrame`s: crop the source frame to `rect`,
//! scale to output size. That's the whole "cinematic" effect.

use crate::events::RawEvent;
use crate::spring::{Spring, Spring2};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraConfig {
    /// Zoom applied on click segments. 1.0 = no zoom; 1.8 ≈ Screen Studio default feel.
    pub click_zoom: f64,
    /// Clicks closer than this (ms) merge into one segment.
    pub cluster_gap_ms: f64,
    /// Clicks farther apart than this (px) never merge even if close in time.
    pub cluster_radius_px: f64,
    /// Camera starts moving this early (ms) so the push-in lands as the click happens.
    pub lead_in_ms: f64,
    /// Stay pushed in this long (ms) after the last event in a segment.
    pub hold_ms: f64,
    /// Spring frequency for camera center movement (rad/s).
    pub pan_omega: f64,
    /// Spring frequency for zoom changes (rad/s). Slightly slower than pan
    /// reads as more deliberate.
    pub zoom_omega: f64,
    /// A typing burst (KeyPress events) also triggers a push-in on the last
    /// known cursor position. Set false to zoom only on clicks.
    pub zoom_on_typing: bool,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            click_zoom: 1.8,
            cluster_gap_ms: 1600.0,
            cluster_radius_px: 420.0,
            lead_in_ms: 350.0,
            hold_ms: 1200.0,
            pan_omega: 7.0,
            zoom_omega: 5.5,
            zoom_on_typing: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Keyframes
// ---------------------------------------------------------------------------

/// A span of time during which the camera is pushed in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ZoomSegment {
    pub start_ms: f64,
    pub end_ms: f64,
    /// Center of attention in source pixels.
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
}

/// Points of interest extracted from the log: (t, x, y).
fn interest_points(events: &[RawEvent], config: &CameraConfig) -> Vec<(f64, f64, f64)> {
    let mut points = Vec::new();
    let mut last_cursor = (0.0f64, 0.0f64);
    for e in events {
        match *e {
            RawEvent::MouseMove { x, y, .. } => last_cursor = (x, y),
            RawEvent::MouseDown { t, x, y, .. } => {
                last_cursor = (x, y);
                points.push((t, x, y));
            }
            RawEvent::KeyPress { t } if config.zoom_on_typing => {
                points.push((t, last_cursor.0, last_cursor.1));
            }
            _ => {}
        }
    }
    points
}

/// Cluster interest points into zoom segments.
pub fn generate_segments(events: &[RawEvent], config: &CameraConfig) -> Vec<ZoomSegment> {
    let points = interest_points(events, config);
    if points.is_empty() {
        return Vec::new();
    }

    let mut segments: Vec<ZoomSegment> = Vec::new();
    // Current cluster accumulator: (start_t, last_t, sum_x, sum_y, n, anchor_x, anchor_y)
    let mut cur = {
        let (t, x, y) = points[0];
        (t, t, x, y, 1.0f64, x, y)
    };

    for &(t, x, y) in &points[1..] {
        let (start_t, last_t, sum_x, sum_y, n, ax, ay) = cur;
        let dist = ((x - ax).powi(2) + (y - ay).powi(2)).sqrt();
        if t - last_t <= config.cluster_gap_ms && dist <= config.cluster_radius_px {
            // Extend cluster; anchor drifts to the running centroid.
            let n2 = n + 1.0;
            cur = (start_t, t, sum_x + x, sum_y + y, n2, (sum_x + x) / n2, (sum_y + y) / n2);
        } else {
            segments.push(ZoomSegment {
                start_ms: (start_t - config.lead_in_ms).max(0.0),
                end_ms: last_t + config.hold_ms,
                cx: sum_x / n,
                cy: sum_y / n,
                zoom: config.click_zoom,
            });
            cur = (t, t, x, y, 1.0, x, y);
        }
    }
    let (start_t, last_t, sum_x, sum_y, n, _, _) = cur;
    segments.push(ZoomSegment {
        start_ms: (start_t - config.lead_in_ms).max(0.0),
        end_ms: last_t + config.hold_ms,
        cx: sum_x / n,
        cy: sum_y / n,
        zoom: config.click_zoom,
    });

    // Merge overlaps created by lead-in/hold expansion.
    let mut merged: Vec<ZoomSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        match merged.last_mut() {
            Some(prev) if seg.start_ms <= prev.end_ms => {
                prev.end_ms = prev.end_ms.max(seg.end_ms);
                // Weight the center toward the later segment — that's where
                // attention is heading.
                prev.cx = (prev.cx + seg.cx) / 2.0;
                prev.cy = (prev.cy + seg.cy) / 2.0;
            }
            _ => merged.push(seg),
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

/// Per-frame camera state: the crop rect in source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraFrame {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub zoom: f64,
}

/// Segment target at time t: (cx, cy, zoom). Wide shot when idle.
fn target_at(segments: &[ZoomSegment], t: f64, screen_w: f64, screen_h: f64) -> (f64, f64, f64) {
    for seg in segments {
        if t >= seg.start_ms && t <= seg.end_ms {
            return (seg.cx, seg.cy, seg.zoom);
        }
    }
    (screen_w / 2.0, screen_h / 2.0, 1.0)
}

/// Solve the camera path for the given frame times.
pub fn solve_camera(
    segments: &[ZoomSegment],
    frame_times_ms: &[f64],
    screen_w: f64,
    screen_h: f64,
    config: &CameraConfig,
) -> Vec<CameraFrame> {
    let mut center = Spring2::new(config.pan_omega, screen_w / 2.0, screen_h / 2.0);
    let mut zoom = Spring::new(config.zoom_omega, 1.0);
    let mut out = Vec::with_capacity(frame_times_ms.len());
    let mut prev_t = frame_times_ms.first().copied().unwrap_or(0.0);

    for &t in frame_times_ms {
        let (tcx, tcy, tz) = target_at(segments, t, screen_w, screen_h);
        let dt = ((t - prev_t) / 1000.0).max(0.0);
        let (cx, cy) = center.step(dt, tcx, tcy);
        let z = zoom.step(dt, tz).max(1.0);

        // Crop rect from center + zoom, clamped inside the screen.
        let w = screen_w / z;
        let h = screen_h / z;
        let x = (cx - w / 2.0).clamp(0.0, screen_w - w);
        let y = (cy - h / 2.0).clamp(0.0, screen_h - h);

        out.push(CameraFrame { t, x, y, w, h, zoom: z });
        prev_t = t;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::MouseButton;

    fn frame_times(n: usize, fps: f64) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 1000.0 / fps).collect()
    }

    fn click(t: f64, x: f64, y: f64) -> RawEvent {
        RawEvent::MouseDown { t, x, y, button: MouseButton::Left }
    }

    #[test]
    fn triple_click_is_one_segment() {
        let events = vec![click(1000.0, 400.0, 400.0), click(1100.0, 405.0, 400.0), click(1200.0, 410.0, 400.0)];
        let segs = generate_segments(&events, &CameraConfig::default());
        assert_eq!(segs.len(), 1);
        assert!((segs[0].cx - 405.0).abs() < 5.0);
    }

    #[test]
    fn distant_clicks_are_separate_segments() {
        let mut config = CameraConfig::default();
        config.hold_ms = 300.0;
        config.lead_in_ms = 100.0;
        let events = vec![click(1000.0, 200.0, 200.0), click(5000.0, 1700.0, 900.0)];
        let segs = generate_segments(&events, &config);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn camera_rect_always_inside_screen() {
        // Click in the extreme corner — the naive rect would hang off-screen.
        let events = vec![click(500.0, 5.0, 5.0), click(3000.0, 1915.0, 1075.0)];
        let segs = generate_segments(&events, &CameraConfig::default());
        let frames = solve_camera(&segs, &frame_times(360, 60.0), 1920.0, 1080.0, &CameraConfig::default());
        for f in &frames {
            assert!(f.x >= -1e-9 && f.y >= -1e-9, "rect origin negative: {:?}", f);
            assert!(f.x + f.w <= 1920.0 + 1e-9, "rect exceeds width: {:?}", f);
            assert!(f.y + f.h <= 1080.0 + 1e-9, "rect exceeds height: {:?}", f);
            assert!(f.zoom >= 1.0);
        }
    }

    #[test]
    fn camera_zooms_in_on_click_and_pulls_back_when_idle() {
        let events = vec![click(1000.0, 960.0, 540.0)];
        let config = CameraConfig::default();
        let segs = generate_segments(&events, &config);
        // 12 seconds of frames: idle by the end.
        let frames = solve_camera(&segs, &frame_times(720, 60.0), 1920.0, 1080.0, &config);
        let peak = frames.iter().map(|f| f.zoom).fold(f64::MIN, f64::max);
        assert!(peak > 1.6, "camera never pushed in, peak zoom = {}", peak);
        let last = frames.last().unwrap();
        assert!(last.zoom < 1.05, "camera never pulled back, final zoom = {}", last.zoom);
        assert!((last.w - 1920.0).abs() < 60.0, "final crop not wide: {:?}", last);
    }

    #[test]
    fn no_events_means_static_wide_shot() {
        let segs = generate_segments(&[], &CameraConfig::default());
        assert!(segs.is_empty());
        let frames = solve_camera(&segs, &frame_times(60, 60.0), 1920.0, 1080.0, &CameraConfig::default());
        for f in frames {
            assert!((f.zoom - 1.0).abs() < 1e-9);
            assert!((f.w - 1920.0).abs() < 1e-9);
        }
    }
}
