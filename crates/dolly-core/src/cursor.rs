//! Synthetic cursor path.
//!
//! Takes the raw MouseMove stream (jittery, variable-rate) and produces a
//! per-frame cursor state: smoothed position + speed. The renderer draws a
//! vector cursor at these positions, so it can be resized, restyled, motion-
//! blurred, or hidden after recording — none of which is possible when the
//! cursor is baked into the capture.

use crate::events::RawEvent;
use crate::spring::Spring2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorFrame {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    /// Smoothed speed in px/s — drives motion-blur strength and the
    /// "auto-hide when idle" opacity ramp.
    pub speed: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct CursorConfig {
    /// Spring frequency for cursor smoothing (rad/s). Screen Studio-feel is
    /// roughly 18–30; lower = dreamier glide, higher = truer to raw input.
    pub omega: f64,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self { omega: 24.0 }
    }
}

/// Linear interpolation of the raw path at time `t` (ms).
fn raw_pos_at(moves: &[(f64, f64, f64)], t: f64) -> (f64, f64) {
    if moves.is_empty() {
        return (0.0, 0.0);
    }
    if t <= moves[0].0 {
        return (moves[0].1, moves[0].2);
    }
    let last = moves[moves.len() - 1];
    if t >= last.0 {
        return (last.1, last.2);
    }
    // Binary search for the segment containing t.
    let idx = moves.partition_point(|m| m.0 <= t);
    let (t0, x0, y0) = moves[idx - 1];
    let (t1, x1, y1) = moves[idx];
    let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
    (x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)
}

/// Build the smoothed per-frame cursor path.
///
/// `frame_times_ms` must be ascending (typically 0, 1000/fps, 2000/fps, …).
pub fn solve_cursor_path(
    events: &[RawEvent],
    frame_times_ms: &[f64],
    config: CursorConfig,
) -> Vec<CursorFrame> {
    // Collect (t, x, y) from anything that carries a pointer position, so the
    // path stays continuous through clicks and scrolls.
    let mut moves: Vec<(f64, f64, f64)> = events
        .iter()
        .filter_map(|e| match *e {
            RawEvent::MouseMove { t, x, y }
            | RawEvent::MouseDown { t, x, y, .. }
            | RawEvent::MouseUp { t, x, y, .. }
            | RawEvent::Wheel { t, x, y, .. } => Some((t, x, y)),
            _ => None,
        })
        .collect();
    moves.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let (ix, iy) = if moves.is_empty() { (0.0, 0.0) } else { (moves[0].1, moves[0].2) };
    let mut spring = Spring2::new(config.omega, ix, iy);
    let mut out = Vec::with_capacity(frame_times_ms.len());
    let mut prev_t = frame_times_ms.first().copied().unwrap_or(0.0);

    for &t in frame_times_ms {
        let (tx, ty) = raw_pos_at(&moves, t);
        let dt = ((t - prev_t) / 1000.0).max(0.0);
        let (x, y) = spring.step(dt, tx, ty);
        out.push(CursorFrame { t, x, y, speed: spring.speed() });
        prev_t = t;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_times(n: usize, fps: f64) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 1000.0 / fps).collect()
    }

    #[test]
    fn cursor_settles_on_final_position() {
        let events = vec![
            RawEvent::MouseMove { t: 0.0, x: 0.0, y: 0.0 },
            RawEvent::MouseMove { t: 100.0, x: 500.0, y: 300.0 },
        ];
        let path = solve_cursor_path(&events, &frame_times(180, 60.0), CursorConfig::default());
        let last = path.last().unwrap();
        assert!((last.x - 500.0).abs() < 1.0 && (last.y - 300.0).abs() < 1.0);
        assert!(last.speed < 5.0, "cursor should be at rest, speed={}", last.speed);
    }

    #[test]
    fn smoothed_path_lags_raw_jitter() {
        // A 1-frame 200px spike should be heavily attenuated by the spring.
        let events = vec![
            RawEvent::MouseMove { t: 0.0, x: 100.0, y: 100.0 },
            RawEvent::MouseMove { t: 500.0, x: 100.0, y: 100.0 },
            RawEvent::MouseMove { t: 508.0, x: 300.0, y: 100.0 }, // spike
            RawEvent::MouseMove { t: 516.0, x: 100.0, y: 100.0 },
            RawEvent::MouseMove { t: 1000.0, x: 100.0, y: 100.0 },
        ];
        let path = solve_cursor_path(&events, &frame_times(70, 60.0), CursorConfig::default());
        let max_x = path.iter().map(|f| f.x).fold(f64::MIN, f64::max);
        assert!(max_x < 200.0, "spike should be attenuated, max_x={}", max_x);
    }

    #[test]
    fn empty_events_do_not_panic() {
        let path = solve_cursor_path(&[], &frame_times(10, 30.0), CursorConfig::default());
        assert_eq!(path.len(), 10);
    }
}
