# Research (2026-07-30)

Competitive + technical research conducted before writing code. Sources: vendor sites, comparison roundups, crate docs.

## Competitive landscape

| Product | Platform | Model | Notes |
|---|---|---|---|
| **Screen Studio** | macOS only | ~$229 one-time / sub tiers | The quality bar. Auto-zoom on clicks, cursor smoothing, motion blur, backgrounds, iOS device recording, 4K export. No Windows version announced. |
| **FocuSee** (iMobie) | Win + Mac | Sub / version-locked lifetime | Closest Windows analog. Auto-zoom, cursor effects, 3D motion, captions. Reviewers flag reliability issues; free tier watermarks. |
| **Rapidemo** | Win + Mac | $79 lifetime | Auto-zoom + cursor smoothing, 4K60. Small team, fewer templates, slower long exports. |
| **Canvid** | Win | Sub | Auto-zoom on clicks. |
| **Cap** | Win + Mac + web | OSS + paid cloud | Open source, Rust/Tauri. Center of gravity = Loom-style sharing/analytics, not render quality. |
| **CursorClip** | Mac | $59 one-time | LTD Screen Studio alternative; Mac only. |
| **Cursorful** | Chrome ext | Free/sub | Browser-tab only. |
| **Clueso** | Web | Enterprise sub | Different category: AI turns recordings into videos + step-by-step docs, 80+ languages, SOC2. Roadmap inspiration, not a v0 competitor. |
| **OBS / ShareX** | Win | Free OSS | Raw capture, zero polish pipeline. |

## The gap

Open-source + Windows-first + local-only + **render quality as the product**. Cap is OSS but optimizes for sharing; FocuSee/Rapidemo/Canvid are closed and mid; Screen Studio won't come to Windows. An OSS tool that nails the Screen Studio pipeline on Windows has a clear "why you over X" for every X.

## Technical findings

- **Windows.Graphics.Capture** is the modern capture API (Win10 2004+). The `windows-capture` crate (Rust) wraps it: cursor include/exclude toggle, border control, v2.0 adds a hardware-accelerated Media Foundation encoder + DXGI Desktop Duplication support. Known constraint: borderless capture unsupported on older Win10 builds → runtime capability check, graceful fallback.
- **`scap`** is a cross-platform alternative (ScreenCaptureKit / WGC / Pipewire) if macOS/Linux ever matter. Windows-first says start with `windows-capture`.
- **Screen Studio's architecture** (from public descriptions): full-res capture + simultaneous cursor/click/scroll tracking; export renders a "virtual camera" following the cursor with configurable zoom/easing/padding. Confirms the capture-without-cursor + re-render approach.
- **Input logging**: WH_MOUSE_LL / WH_KEYBOARD_LL hooks; QueryPerformanceCounter timestamps; shared epoch with video for sync; DPI mapping required. Privacy: log keypress *timing* only, never identities.

## Positioning axes (v0)

1. Output quality ≥ FocuSee/Rapidemo, targeting Screen Studio.
2. Free + OSS (their pricing is the wedge; "no watermark, no version-locked license" writes its own launch post).
3. Local-only (no forced cloud, no account).
4. Editable-by-data: zooms are segments you can tweak, not baked pixels.
