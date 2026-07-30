//! Conversions between QPC ticks and the recording timeline.
//!
//! Windows.Graphics.Capture stamps frames with `SystemRelativeTime` (100 ns
//! units on the QueryPerformanceCounter timebase), and our input hooks stamp
//! events with raw QPC counts. The mp4's t=0 is the *first encoded frame's*
//! timestamp (windows-capture rebases internally), so every event timestamp
//! must be rebased against that same first-frame instant — not against the
//! moment `start()` was called, which is ~50–100 ms earlier.
//!
//! Pure math, no OS deps: unit-tested on every platform.

/// Convert a raw QPC count to 100 ns units (the `TimeSpan` unit).
///
/// Widens through i128: on modern Windows QPC frequency is typically 10 MHz,
/// so `qpc * 10_000_000` overflows i64 after ~10 days of uptime.
pub fn qpc_to_100ns(qpc: i64, freq: i64) -> i64 {
    ((qpc as i128 * 10_000_000) / freq as i128) as i64
}

/// Milliseconds elapsed since `epoch_100ns` (may be negative for events that
/// arrived before the first video frame; callers should drop those).
pub fn ticks_to_ms(ticks_100ns: i64, epoch_100ns: i64) -> f64 {
    (ticks_100ns - epoch_100ns) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qpc_conversion_survives_long_uptime() {
        // 30 days of uptime at 10 MHz: qpc * 1e7 would overflow i64.
        let freq = 10_000_000;
        let qpc = 30 * 24 * 3600 * freq;
        assert_eq!(qpc_to_100ns(qpc, freq), 30 * 24 * 3600 * 10_000_000);
    }

    #[test]
    fn odd_frequencies_convert_exactly() {
        // Older machines report e.g. 3.5793 MHz. One full second of ticks
        // must land on exactly one second of 100 ns units.
        let freq = 3_579_545;
        assert_eq!(qpc_to_100ns(freq, freq), 10_000_000);
    }

    #[test]
    fn ticks_to_ms_is_relative_to_epoch() {
        let epoch = 5_000_000; // 500 ms after boot
        assert_eq!(ticks_to_ms(epoch, epoch), 0.0);
        assert_eq!(ticks_to_ms(epoch + 10_000, epoch), 1.0);
        // Sub-ms resolution is preserved.
        assert_eq!(ticks_to_ms(epoch + 5, epoch), 0.0005);
        // Before-epoch events come out negative so callers can filter them.
        assert!(ticks_to_ms(epoch - 10_000, epoch) < 0.0);
    }
}
