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

    // Break the input log down by kind — a quick sanity check that the hooks
    // fired and coordinates landed inside the surface. Move around and type
    // during the recording to see these climb.
    use dolly_core::events::RawEvent;
    let (mut moves, mut clicks, mut wheels, mut keys) = (0, 0, 0, 0);
    for e in &artifacts.events {
        match e {
            RawEvent::MouseMove { .. } => moves += 1,
            RawEvent::MouseDown { .. } | RawEvent::MouseUp { .. } => clicks += 1,
            RawEvent::Wheel { .. } => wheels += 1,
            RawEvent::KeyPress { .. } => keys += 1,
            RawEvent::WindowFocus { .. } => {}
        }
    }
    println!(
        "captured {} events: {moves} moves, {clicks} button, {wheels} wheel, {keys} keypress",
        artifacts.events.len()
    );

    // The sidecar sits beside the mp4; read it back to prove the write + parse
    // round-trips.
    let sidecar = std::path::Path::new(&artifacts.video_path).with_extension("events.jsonl");
    let reloaded =
        dolly_core::events::from_jsonl(&std::fs::read_to_string(&sidecar).expect("sidecar readable"));
    assert_eq!(reloaded.len(), artifacts.events.len(), "sidecar round-trip lost events");
    println!("sidecar {} round-trips ({} events)", sidecar.display(), reloaded.len());
}

#[cfg(not(all(windows, feature = "win-capture")))]
fn main() {
    eprintln!("record_monitor needs Windows and --features win-capture");
}
