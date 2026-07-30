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
2. 🔜 Windows capture backend: windows-capture crate, WithoutCursor, LL hooks, QPC epoch. Land behind win-capture feature; prove on windows-latest CI.
3. 🔜 Offline renderer: decode → crop → cursor composite → style → encode. Start CPU (image + minimp4/ffmpeg), then wgpu.
4. 🔜 Tauri editor shell with mock-backend preview so UI work never requires Windows.

## Taste notes (the product bar)
- Camera should feel like a deliberate human operator: push in ~1.8x on click clusters, hold, pull back on idle. No strobing on triple-clicks.
- Cursor glide ω≈24; camera pan ω≈7; zoom ω≈5.5. Tune with tests in camera.rs, not vibes.
- Default style: inset frame, rounded corners, shadow, gradient background — good output with zero editing is the whole point.
