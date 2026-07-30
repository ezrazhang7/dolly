# dolly

**Open-source Screen Studio for Windows.**

Record your screen. Dolly re-renders your cursor with spring physics, auto-zooms on your clicks, and exports something that looks like a human camera operator was following the action. Local-only. No subscription. No watermark.

> 🎥 A *dolly* is the rig that makes film camera movement smooth. That's the whole product: a virtual dolly operator for your screen.

## Why this exists

Screen Studio made recordings look cinematic and became the default for indie devs and product people — on macOS only, with no Windows version announced. Windows users get closed-source clones: version-locked lifetime licenses, subscriptions, watermarked free tiers, mixed reliability. The one open-source player in the space centers on Loom-style cloud sharing, not the render pipeline.

Nobody has shipped the thing itself: an open, local, Windows-first recorder where the *output quality* is the product.

## The trick (and why most clones get it wrong)

The polished look isn't a filter. It's an architecture:

1. **Capture the screen with the cursor *excluded*** (Windows.Graphics.Capture supports this natively).
2. **Log raw input** — mouse positions at hardware rate, clicks, scrolls, keypress *timing* (never key identities).
3. **Re-render in post**: a synthetic vector cursor glides on a critically damped spring; a virtual camera pushes in on click clusters, holds, and pulls back when you go idle.

Because the cursor is synthetic, you can resize it, restyle it, motion-blur it, or hide it *after* recording. Because the camera is solved from the event log, every zoom is editable data, not baked pixels. Recordings stay raw + non-destructive edits forever.

## Status

🚧 **Early — engine-first development.**

| Piece | Status |
|---|---|
| `dolly-core` — spring physics, cursor solver, auto-zoom clustering, camera solver, project format | ✅ implemented + unit-tested |
| `dolly-capture` — capture/input traits + deterministic mock backend | ✅ implemented |
| Windows capture backend (windows-capture / Graphics Capture API) | 🔜 next |
| GPU renderer + exporter (crop → cursor composite → style frame → encode) | 🔜 |
| Editor app (Tauri): timeline, zoom segment editing, style presets | 🔜 |
| Webcam / mic / system audio | planned |
| AI layer (auto-docs from recordings, à la Clueso) | someday |

The engine is deliberately platform-independent and test-driven: camera *feel* gets tuned in unit tests with synthetic sessions, not record-render-squint loops.

## Layout

```
crates/
├── dolly-core      # the brain: events → render plan (cursor + camera per frame)
└── dolly-capture   # capture behind traits; mock backend now, Windows backend next
docs/
├── RESEARCH.md     # competitive + technical research that shaped the design
└── ARCHITECTURE.md # system design and key decisions
```

## Dev

```bash
cargo test          # runs everywhere — core is platform-independent
```

Windows-specific code is feature-gated (`win-capture`) and built on `windows-latest` CI runners.

## License

MIT
