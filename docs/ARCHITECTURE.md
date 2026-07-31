# Architecture

## Principle: capture is evidence, rendering is opinion

The recording step produces two immutable artifacts:
- `capture.mp4` — the screen, cursor **excluded**, hardware-encoded at capture time
- `events.jsonl` — every input event, sample-accurate against the video clock

Everything aesthetic — cursor, zooms, styling — is *derived data*, regenerated on demand. Edits are non-destructive JSON in `project.json`. You can re-export the same recording in any style forever.

## Crates

### dolly-core (platform-independent, zero OS deps)
- `spring` — critically damped springs, fixed-substep semi-implicit Euler. Deterministic across frame rates.
- `events` — RawEvent schema + JSONL (corruption-tolerant parse: a crashed recording still opens).
- `cursor` — raw move stream → resample at frame times → spring smooth → per-frame {pos, speed}. Speed drives motion blur + idle auto-hide.
- `camera` — two stages:
  1. `generate_segments`: interest points (clicks, typing bursts) → clustered ZoomSegments (time-gap + radius clustering, lead-in/hold expansion, overlap merge).
  2. `solve_camera`: segments → per-frame crop rect via springs on center+zoom, clamped to screen bounds *after* springing (camera leans into edges instead of jittering).
- `project` — .dolly folder format.
- `render_plan()` — the single entrypoint renderers consume.

Why engine-first: camera *feel* is tunable in unit tests against synthetic sessions (see MockRecorder). Fast iteration on the thing that is the actual product.

### dolly-capture
- `Recorder` trait + `RecordingArtifacts`.
- `MockRecorder` — deterministic synthetic session (eased mouse travel, clicks, typing burst) for developing editor/renderer on any OS.
- `timebase` — pure QPC↔ms math, unit-tested on all platforms. Key fact: the mp4's t=0 is the *first encoded frame's* `SystemRelativeTime` (windows-capture rebases internally), so input events are rebased against that instant, not against `start()`.
- `mapping` — pure rebase + surface-space coordinate mapping (plain-number signatures), unit-tested on all platforms; the Windows hook thread only constructs its input types.
- `win` module (feature `win-capture`, Windows-only): `WinRecorder` records one monitor via windows-capture 2.0 straight to a hardware-encoded mp4, `CursorCaptureSettings::WithoutCursor` (hard requirement, Win10 2004+). Border suppression needs Win11; on Win10 it falls back to `DrawBorderSettings::Default` — the system border is on-screen-only and never appears in captured frames. LL input hooks (`win/input.rs`) pump on a dedicated thread, QPC-stamped; `stop()` writes the `<video stem>.events.jsonl` sidecar beside the mp4. Real-screen smoke test is `#[ignore]`d (needs an interactive desktop): records ~2 s, asserts a non-empty mp4, monotonic events, sidecar round-trip, and that `render_plan` accepts the artifacts.
- Known limitations (shared by every commercial competitor, documented in `win/mod.rs`): LL hooks don't receive input aimed at elevated windows (UIPI), and capture blanks during UAC/secure-desktop prompts.

### dolly-app (Tauri 2 editor shell)
- Standalone workspace (root `Cargo.toml` excludes it) so `cargo test --workspace` never needs webview toolchains. Static no-bundler frontend in `ui/` (plain HTML/JS, `withGlobalTauri`) — no npm anywhere.
- Backend commands: `load_mock`, `start_recording`/`stop_recording` (Windows: `WinRecorder` → `Videos/Dolly/rec-<ts>.dolly/` with capture.mp4 + events.jsonl + project.json), `solve` (re-runs `solve_cursor_path` + `solve_camera` for edited segments/settings), `regenerate_segments`, `set_style`, `save_project`, `export_video` (stub until the renderer exists).
- Preview = live DOM composite: background/padding/corner/shadow from StyleSettings, camera crop as a CSS transform on the screen layer, synthetic SVG cursor + click ripples from the plan. Real recordings play the mp4 in a `<video>`; mock sessions show a synthetic desktop so zooms are visible.
- Timeline: zoom segments as draggable blocks (move/resize/add-on-click/right-click delete) — edits round-trip through `solve` and live in project.json, never in pixels.

## Planned: renderer
- **Renderer**: per output frame — decode source frame → crop to CameraFrame rect → composite vector cursor at CursorFrame pos (scale/blur from speed) → inset/corner-radius/shadow/background → encode. GPU path: wgpu offscreen; encode via Media Foundation (already in windows-capture) or bundled ffmpeg. Until then the app's Export button returns a friendly stub error.

## Testing strategy
- Core: unit tests on math invariants (no overshoot, bounds clamping, cluster behavior, pullback on idle) + integration smoke on realistic synthetic sessions.
- Windows backend: compiled + tested on GitHub Actions `windows-latest` (CI is the Windows dev box).
- Renderer (later): golden-frame tests — render synthetic sessions, compare against approved PNGs.
