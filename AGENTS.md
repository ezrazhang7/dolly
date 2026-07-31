# AGENTS.md

Context for AI agents (Claude Code etc.) working in this repo.

## What this is
Open-source Screen Studio for Windows. Rust workspace. The product is **render quality**: synthetic spring-smoothed cursor + auto-zoom virtual camera, solved from a raw input log over a cursor-less screen capture.

## Ground rules
- `dolly-core` stays platform-independent, zero OS deps. If you need an OS API you're in the wrong crate.
- Raw capture artifacts are immutable; all edits are data in project.json. Never design a destructive edit.
- Privacy invariant: keypress *timing* may be logged, key *identities* never. Do not "fix" this.
- Every behavior in core gets a unit test on the invariant (bounds, overshoot, clustering), not on exact float values.
- Windows-only code goes behind `#[cfg(windows)]` + feature `win-capture`. CI's windows job is the Windows test environment — keep ubuntu `cargo test` green at all times.
- Rust edition 2021, keep MSRV ≈ 1.75 (avoid bleeding-edge syntax).

## Current state / next milestones
1. ✅ Engine (springs, cursor solver, zoom clustering, camera solver, project format) — tested.
2. ✅ Windows capture backend: `WinRecorder` records one monitor → cursor-less hardware-encoded mp4 (Win10 border fallback), WH_MOUSE_LL/WH_KEYBOARD_LL hooks on a dedicated pump thread (QPC-stamped, rebased to the first-frame epoch via pure `mapping`/`timebase` modules that unit-test on ubuntu; key identities never read), `stop()` writes the `<video stem>.events.jsonl` sidecar. Real-screen smoke test `#[ignore]`d (run locally: `cargo test -p dolly-capture --features win-capture -- --ignored`). Known limitations, documented not solved (all competitors share them): LL hooks miss input to elevated windows; capture blanks on UAC/secure desktop. Later: `WindowFocus` via `SetWinEventHook`. Note: the win-capture feature pulls in an edition-2024 dep, so building *with the feature* needs rustc ≥1.85; the ≈1.75 MSRV applies to our own code and the default feature set.
3. 🔜 Offline renderer: decode → crop → cursor composite → style → encode. Start CPU (image + minimp4/ffmpeg), then wgpu. Until it lands, the app's Export button is a stub.
4. ✅ Editor shell (`crates/dolly-app`, Tauri 2): static no-npm frontend (`ui/`), preview composite (background/padding/corners/shadow + camera transform + synthetic cursor), zoom-block timeline (drag/resize/add/right-click-delete), style/camera/cursor sidebar, mock session on any OS, real recording on Windows (saves `rec-*.dolly/` with capture.mp4 + events.jsonl + project.json under Videos/Dolly). Deliberately excluded from the root workspace (`exclude` in root Cargo.toml) so ubuntu `cargo test --workspace` needs no webview libs — build it with `cd crates/dolly-app && cargo build`.

## Taste notes (the product bar)
- Camera should feel like a deliberate human operator: push in ~1.8x on click clusters, hold, pull back on idle. No strobing on triple-clicks.
- Cursor glide ω≈24; camera pan ω≈7; zoom ω≈5.5. Tune with tests in camera.rs, not vibes.
- Default style: inset frame, rounded corners, shadow, gradient background — good output with zero editing is the whole point.
