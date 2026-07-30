//! Rebase raw hook events onto the recording timeline and map their
//! coordinates into surface space.
//!
//! This is the pure half of the Windows input pipeline: plain-number inputs
//! (QPC counts, virtual-screen pixels), no OS deps, so it unit-tests on every
//! platform — the ubuntu CI job covers it. The impure half (the actual
//! WH_MOUSE_LL / WH_KEYBOARD_LL hooks) lives in `win::input` and only
//! constructs these types.

use dolly_core::events::{MouseButton, RawEvent};

use crate::timebase::{qpc_to_100ns, ticks_to_ms};

/// One captured input event, still on the raw QPC clock and in virtual-screen
/// global pixels. Rebased and mapped to surface-space by [`rebase_and_map`].
pub struct Stamped {
    pub qpc: i64,
    pub kind: Kind,
}

pub enum Kind {
    Move { x: i32, y: i32 },
    Down { x: i32, y: i32, button: MouseButton },
    Up { x: i32, y: i32, button: MouseButton },
    Wheel { x: i32, y: i32, dx: f64, dy: f64 },
    /// Key identity deliberately absent — see the privacy note in `win::input`.
    KeyPress,
}

/// Rebase each event's QPC stamp onto the first-frame epoch and map its
/// coordinates from virtual-screen global pixels into surface space.
///
/// * `qpc_freq` — `QueryPerformanceFrequency` (ticks per second).
/// * `epoch_100ns` — the first video frame's timestamp (QPC 100 ns units).
/// * `origin` — the captured monitor's top-left in virtual-screen pixels.
/// * `size` — the captured surface size in physical pixels.
///
/// Drops events before the epoch (negative `t`) and mouse events outside the
/// monitor rect (multi-monitor setups); returns them time-sorted. LL events
/// arrive in order already, but the sort makes that a guarantee the renderer
/// can rely on.
pub fn rebase_and_map(
    raw: impl IntoIterator<Item = Stamped>,
    qpc_freq: i64,
    epoch_100ns: i64,
    origin: (i32, i32),
    size: (u32, u32),
) -> Vec<RawEvent> {
    let (ox, oy) = origin;
    let (w, h) = (size.0 as i32, size.1 as i32);
    // Map a global point into surface space, or None if it lands off-monitor.
    let map = |x: i32, y: i32| -> Option<(f64, f64)> {
        let (sx, sy) = (x - ox, y - oy);
        (sx >= 0 && sx < w && sy >= 0 && sy < h).then_some((sx as f64, sy as f64))
    };

    let mut out = Vec::new();
    for ev in raw {
        let t = ticks_to_ms(qpc_to_100ns(ev.qpc, qpc_freq), epoch_100ns);
        if t < 0.0 {
            continue; // before the first video frame
        }
        let event = match ev.kind {
            Kind::Move { x, y } => map(x, y).map(|(x, y)| RawEvent::MouseMove { t, x, y }),
            Kind::Down { x, y, button } => {
                map(x, y).map(|(x, y)| RawEvent::MouseDown { t, x, y, button })
            }
            Kind::Up { x, y, button } => {
                map(x, y).map(|(x, y)| RawEvent::MouseUp { t, x, y, button })
            }
            Kind::Wheel { x, y, dx, dy } => {
                map(x, y).map(|(x, y)| RawEvent::Wheel { t, x, y, dx, dy })
            }
            // KeyPress has no coordinate to clip against — always kept.
            Kind::KeyPress => Some(RawEvent::KeyPress { t }),
        };
        if let Some(event) = event {
            out.push(event);
        }
    }
    out.sort_by(|a, b| a.t().total_cmp(&b.t()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // 10 MHz QPC (typical modern hardware): 1 tick = 0.1 µs, 10_000 ticks = 1 ms.
    const FREQ: i64 = 10_000_000;
    // First video frame at 500 ms since boot, in 100 ns units.
    const EPOCH: i64 = 5_000_000;
    // A 1920x1080 monitor whose top-left sits at virtual-screen (2560, 0) —
    // i.e. a second monitor to the right of a primary one.
    const ORIGIN: (i32, i32) = (2560, 0);
    const SIZE: (u32, u32) = (1920, 1080);

    // QPC count for `ms` after boot at FREQ (100ns units = ms * 10_000).
    fn qpc_at_ms(ms: f64) -> i64 {
        ((ms * 10_000.0) as i64 * FREQ) / 10_000_000
    }

    #[test]
    fn maps_to_surface_space_and_rebases_time() {
        // A click at global (2660, 100) → surface (100, 100), 100 ms in.
        let raw = vec![Stamped {
            qpc: qpc_at_ms(600.0), // 100 ms after the 500 ms epoch
            kind: Kind::Down { x: 2660, y: 100, button: MouseButton::Left },
        }];
        let out = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE);
        assert_eq!(
            out,
            vec![RawEvent::MouseDown { t: 100.0, x: 100.0, y: 100.0, button: MouseButton::Left }]
        );
    }

    #[test]
    fn drops_events_before_first_frame() {
        // 400 ms after boot is before the 500 ms epoch → negative t → dropped.
        let raw = vec![Stamped { qpc: qpc_at_ms(400.0), kind: Kind::KeyPress }];
        assert!(rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE).is_empty());
    }

    #[test]
    fn drops_mouse_events_outside_the_monitor() {
        // Global x=100 is on the *primary* monitor, left of this one's origin.
        let raw = vec![Stamped {
            qpc: qpc_at_ms(600.0),
            kind: Kind::Move { x: 100, y: 100 },
        }];
        assert!(rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE).is_empty());
    }

    #[test]
    fn keypress_survives_without_coordinates() {
        // KeyPress has no position, so the off-monitor clip never applies to it.
        let raw = vec![Stamped { qpc: qpc_at_ms(600.0), kind: Kind::KeyPress }];
        let out = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE);
        assert_eq!(out, vec![RawEvent::KeyPress { t: 100.0 }]);
    }

    #[test]
    fn output_is_time_sorted() {
        let raw = vec![
            Stamped { qpc: qpc_at_ms(700.0), kind: Kind::KeyPress },
            Stamped { qpc: qpc_at_ms(600.0), kind: Kind::KeyPress },
            Stamped { qpc: qpc_at_ms(650.0), kind: Kind::KeyPress },
        ];
        let ts: Vec<f64> = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE)
            .iter()
            .map(RawEvent::t)
            .collect();
        assert_eq!(ts, vec![100.0, 150.0, 200.0]);
    }
}
