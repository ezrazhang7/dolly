//! Real-screen smoke test for the Windows backend.
//!
//! Ignored by default: it records the actual primary monitor, which needs an
//! interactive desktop session (headless CI runners may not have one).
//! Run manually with:
//!
//! ```text
//! cargo test -p dolly-capture --features win-capture -- --ignored
//! ```

#![cfg(all(windows, feature = "win-capture"))]

use dolly_capture::win::WinRecorder;
use dolly_capture::Recorder;

#[test]
#[ignore = "records the real screen; needs an interactive desktop session"]
fn records_two_seconds_of_primary_monitor() {
    let dir = std::env::temp_dir().join("dolly-win-smoke");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("smoke.mp4");

    let mut rec = WinRecorder::new(&out);
    rec.start().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let artifacts = rec.stop().unwrap();

    // Invariants, not exact values: real frame pacing varies.
    assert!(artifacts.meta.width > 0 && artifacts.meta.height > 0);
    assert!(
        artifacts.meta.duration_ms > 1_000.0,
        "expected >1s of frames, got {} ms",
        artifacts.meta.duration_ms
    );
    let size = std::fs::metadata(&out).unwrap().len();
    assert!(size > 10_000, "mp4 suspiciously small: {size} bytes");
}
