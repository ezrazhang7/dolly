//! Record the primary monitor (cursor excluded) for N seconds:
//!
//! ```text
//! cargo run -p dolly-capture --example record_monitor --features win-capture -- [seconds] [out.mp4]
//! ```

#[cfg(all(windows, feature = "win-capture"))]
fn main() {
    use dolly_capture::win::WinRecorder;
    use dolly_capture::Recorder;

    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let out = args.next().unwrap_or_else(|| "capture.mp4".into());

    let mut rec = WinRecorder::new(&out);
    rec.start().expect("capture start failed");
    println!("recording primary monitor for {seconds}s (cursor excluded)…");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let artifacts = rec.stop().expect("capture stop failed");

    let meta = &artifacts.meta;
    println!(
        "wrote {}: {}x{} @ {} fps, scale {}, {:.0} ms",
        artifacts.video_path, meta.width, meta.height, meta.fps, meta.scale_factor, meta.duration_ms
    );
}

#[cfg(not(all(windows, feature = "win-capture")))]
fn main() {
    eprintln!("record_monitor needs Windows and --features win-capture");
}
