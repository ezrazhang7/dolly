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
- `win` module (feature `win-capture`, Windows-only): windows-capture crate, CursorCaptureSettings::WithoutCursor, LL input hooks, QPC shared epoch.

## Planned: renderer + app
- **Renderer**: per output frame — decode source frame → crop to CameraFrame rect → composite vector cursor at CursorFrame pos (scale/blur from speed) → inset/corner-radius/shadow/background → encode. GPU path: wgpu offscreen; encode via Media Foundation (already in windows-capture) or bundled ffmpeg.
- **App**: Tauri 2. Rust core owns capture/render; web UI for timeline + segment editing + style presets. Preview = same render_plan applied to <canvas> at interactive rate.

## Testing strategy
- Core: unit tests on math invariants (no overshoot, bounds clamping, cluster behavior, pullback on idle) + integration smoke on realistic synthetic sessions.
- Windows backend: compiled + tested on GitHub Actions `windows-latest` (CI is the Windows dev box).
- Renderer (later): golden-frame tests — render synthetic sessions, compare against approved PNGs.
