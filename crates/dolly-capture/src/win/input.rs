//! Low-level input hooks on a dedicated pump thread.
//!
//! `WH_MOUSE_LL` / `WH_KEYBOARD_LL` are *global* hooks: the OS calls the hook
//! proc on the thread that installed it, but only while that thread is pumping
//! messages. A hook thread without a `GetMessage` loop silently never fires —
//! the classic footgun — so [`InputHook::start`] spawns a thread that installs
//! both hooks and then pumps until asked to stop.
//!
//! ## Timestamps
//! `MSLLHOOKSTRUCT::time` is `GetTickCount`-based (~15 ms resolution), which is
//! useless against sub-millisecond video frame timestamps. So each hook proc
//! calls [`QueryPerformanceCounter`] directly and stores the raw QPC count.
//! Conversion to the recording timeline (100 ns units, then ms since the first
//! video frame) happens in [`InputHook::finalize`] once the epoch is known —
//! see [`crate::timebase`].
//!
//! ## Privacy invariant
//! For keyboard events we match `WM_KEYDOWN` / `WM_SYSKEYDOWN` and emit a
//! bare [`RawEvent::KeyPress`]. `KBDLLHOOKSTRUCT::vkCode` is *never read* — the
//! invariant holds at the point of capture, not as a downstream filter, so the
//! key identity never enters the process's data at all.
//!
//! ## Coordinates
//! Hook coordinates are virtual-screen global physical pixels (the process is
//! per-monitor-DPI-aware, set in `mod.rs::start`). [`InputHook::finalize`]
//! subtracts the captured monitor's rect origin to get surface-space pixels and
//! drops events that fall outside the monitor (multi-monitor setups).

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use dolly_core::events::{MouseButton, RawEvent};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    HC_ACTION, HHOOK, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::timebase::{qpc_to_100ns, ticks_to_ms};

#[derive(Debug)]
pub(crate) enum WinInputError {
    HookInstall(String),
    ThreadDied,
}

impl std::fmt::Display for WinInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WinInputError::HookInstall(msg) => write!(f, "hook install failed: {msg}"),
            WinInputError::ThreadDied => write!(f, "hook thread died before install"),
        }
    }
}

/// One captured input event, still on the raw QPC clock and in virtual-screen
/// global pixels. Rebased and mapped to surface-space by [`InputHook::finalize`].
struct Stamped {
    qpc: i64,
    kind: Kind,
}

enum Kind {
    Move { x: i32, y: i32 },
    Down { x: i32, y: i32, button: MouseButton },
    Up { x: i32, y: i32, button: MouseButton },
    Wheel { x: i32, y: i32, dx: f64, dy: f64 },
    /// Key identity deliberately absent — see module privacy note.
    KeyPress,
}

// The hook procs are plain `extern "system"` functions with no state parameter,
// so the channel Sender reaches them through a thread-local. LL hooks fire on
// the installing thread, which is exactly the thread that sets this, so a
// thread-local (not a global) is both sufficient and free of cross-thread races.
thread_local! {
    static EVENT_TX: RefCell<Option<Sender<Stamped>>> = const { RefCell::new(None) };
}

fn qpc_now() -> i64 {
    let mut count = 0i64;
    // Infallible on any hardware Windows actually boots on.
    unsafe { QueryPerformanceCounter(&mut count) }.ok();
    count
}

fn send(kind: Kind) {
    let qpc = qpc_now();
    EVENT_TX.with(|tx| {
        if let Some(tx) = tx.borrow().as_ref() {
            // Receiver hung up (stop already drained) — nothing to do.
            let _ = tx.send(Stamped { qpc, kind });
        }
    });
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let (x, y) = (info.pt.x, info.pt.y);
        // High word of mouseData: wheel delta (signed) or X-button identity.
        let hi = ((info.mouseData >> 16) & 0xFFFF) as u16;
        match wparam.0 as u32 {
            WM_MOUSEMOVE => send(Kind::Move { x, y }),
            WM_LBUTTONDOWN => send(Kind::Down { x, y, button: MouseButton::Left }),
            WM_LBUTTONUP => send(Kind::Up { x, y, button: MouseButton::Left }),
            WM_RBUTTONDOWN => send(Kind::Down { x, y, button: MouseButton::Right }),
            WM_RBUTTONUP => send(Kind::Up { x, y, button: MouseButton::Right }),
            WM_MBUTTONDOWN => send(Kind::Down { x, y, button: MouseButton::Middle }),
            WM_MBUTTONUP => send(Kind::Up { x, y, button: MouseButton::Middle }),
            WM_XBUTTONDOWN => send(Kind::Down { x, y, button: MouseButton::Other }),
            WM_XBUTTONUP => send(Kind::Up { x, y, button: MouseButton::Other }),
            // Positive delta = up / right, matching RawEvent::Wheel's convention.
            WM_MOUSEWHEEL => send(Kind::Wheel { x, y, dx: 0.0, dy: hi as i16 as f64 }),
            WM_MOUSEHWHEEL => send(Kind::Wheel { x, y, dx: hi as i16 as f64, dy: 0.0 }),
            _ => {}
        }
    }
    // Return control to the chain immediately; never block the LL hook.
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        match wparam.0 as u32 {
            // vkCode is intentionally not read — see module privacy note.
            WM_KEYDOWN | WM_SYSKEYDOWN => send(Kind::KeyPress),
            _ => {}
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// A running pair of low-level input hooks, pumping on their own thread.
pub(crate) struct InputHook {
    thread_id: u32,
    handle: Option<JoinHandle<()>>,
    rx: Receiver<Stamped>,
    qpc_freq: i64,
}

impl InputHook {
    /// Install both hooks on a fresh pump thread. Returns once the hooks are
    /// confirmed installed (or reports why they weren't).
    pub(crate) fn start() -> Result<Self, WinInputError> {
        let mut qpc_freq = 0i64;
        unsafe { QueryPerformanceFrequency(&mut qpc_freq) }
            .map_err(|e| WinInputError::HookInstall(format!("QueryPerformanceFrequency: {e}")))?;

        let (event_tx, rx) = mpsc::channel::<Stamped>();
        // Hand the spawned thread its own confirmation back: the thread id to
        // post WM_QUIT to, or the install error.
        let (startup_tx, startup_rx) = mpsc::sync_channel::<Result<u32, WinInputError>>(1);

        let handle = std::thread::Builder::new()
            .name("dolly-input-hooks".into())
            .spawn(move || hook_thread(startup_tx, event_tx))
            .map_err(|e| WinInputError::HookInstall(format!("spawn: {e}")))?;

        match startup_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self { thread_id, handle: Some(handle), rx, qpc_freq }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err(WinInputError::ThreadDied)
            }
        }
    }

    /// Stop pumping, join the thread, and rebase every captured event onto the
    /// recording timeline.
    ///
    /// * `epoch_100ns` — the first video frame's timestamp (QPC 100 ns units).
    /// * `origin` — the captured monitor's top-left in virtual-screen pixels.
    /// * `size` — the captured surface size in physical pixels.
    ///
    /// Events before the first frame (negative `t`) or outside the monitor rect
    /// are dropped. The result is sorted by time (LL events arrive in order, but
    /// the sort makes that a guarantee the renderer can rely on).
    pub(crate) fn finalize(
        mut self,
        epoch_100ns: i64,
        origin: (i32, i32),
        size: (u32, u32),
    ) -> Vec<RawEvent> {
        // Break the GetMessage pump, then wait for the thread to unhook + exit.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        rebase_and_map(self.rx.try_iter(), self.qpc_freq, epoch_100ns, origin, size)
    }
}

/// Pure rebase + surface-space mapping, factored out of [`InputHook::finalize`]
/// so it is unit-testable without a live hook thread. Drops events before the
/// epoch and mouse events outside the monitor rect; returns them time-sorted.
fn rebase_and_map(
    raw: impl IntoIterator<Item = Stamped>,
    qpc_freq: i64,
    epoch_100ns: i64,
    origin: (i32, i32),
    size: (u32, u32),
) -> Vec<RawEvent> {
    let (ox, oy) = origin;
    let (w, h) = (size.0 as i32, size.1 as i32);
    // Map a global point into surface space, or None if it lands off-monitor.
    let map = |x: i32, y: i32| -> Option<(f64, f64)> {
        let (sx, sy) = (x - ox, y - oy);
        (sx >= 0 && sx < w && sy >= 0 && sy < h).then_some((sx as f64, sy as f64))
    };

    let mut out = Vec::new();
    for ev in raw {
        let t = ticks_to_ms(qpc_to_100ns(ev.qpc, qpc_freq), epoch_100ns);
        if t < 0.0 {
            continue; // before the first video frame
        }
        let event = match ev.kind {
            Kind::Move { x, y } => map(x, y).map(|(x, y)| RawEvent::MouseMove { t, x, y }),
            Kind::Down { x, y, button } => {
                map(x, y).map(|(x, y)| RawEvent::MouseDown { t, x, y, button })
            }
            Kind::Up { x, y, button } => {
                map(x, y).map(|(x, y)| RawEvent::MouseUp { t, x, y, button })
            }
            Kind::Wheel { x, y, dx, dy } => {
                map(x, y).map(|(x, y)| RawEvent::Wheel { t, x, y, dx, dy })
            }
            // KeyPress has no coordinate to clip against — always kept.
            Kind::KeyPress => Some(RawEvent::KeyPress { t }),
        };
        if let Some(event) = event {
            out.push(event);
        }
    }
    out.sort_by(|a, b| a.t().total_cmp(&b.t()));
    out
}

impl Drop for InputHook {
    fn drop(&mut self) {
        // Safety net for the drop-without-finalize path (e.g. a WinRecorder
        // dropped without stop()): break the pump and join so the hooks are
        // uninstalled and the thread doesn't outlive us.
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = handle.join();
        }
    }
}

/// Body of the dedicated hook thread: install both hooks, report success, pump
/// until `WM_QUIT`, then unhook. Runs entirely on one thread so the hook procs'
/// thread-local `EVENT_TX` is the same one this function seeds.
fn hook_thread(
    startup_tx: SyncSender<Result<u32, WinInputError>>,
    event_tx: Sender<Stamped>,
) {
    // For LL hooks the module handle must belong to the current process.
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map(Into::into)
        .unwrap_or_default();

    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hinstance), 0) };
    let mouse = match mouse {
        Ok(h) => h,
        Err(e) => {
            let _ = startup_tx.send(Err(WinInputError::HookInstall(format!("WH_MOUSE_LL: {e}"))));
            return;
        }
    };
    let keyboard =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hinstance), 0) };
    let keyboard = match keyboard {
        Ok(h) => h,
        Err(e) => {
            unsafe { UnhookWindowsHookEx(mouse).ok() };
            let _ =
                startup_tx.send(Err(WinInputError::HookInstall(format!("WH_KEYBOARD_LL: {e}"))));
            return;
        }
    };

    EVENT_TX.with(|tx| *tx.borrow_mut() = Some(event_tx));

    // Both hooks live — hand back the thread id so stop() can post WM_QUIT.
    if startup_tx.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
        // The starter gave up before we finished; tear down and leave.
        unhook(mouse, keyboard);
        return;
    }

    // The pump. LL hooks fire from inside GetMessage's wait; the messages it
    // returns are only our own WM_QUIT (posted to this thread, no window).
    let mut msg = MSG::default();
    // GetMessageW returns >0 for a normal message, 0 on WM_QUIT, -1 on error.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}

    unhook(mouse, keyboard);
    // Drop the thread-local Sender so finalize()'s try_iter terminates.
    EVENT_TX.with(|tx| *tx.borrow_mut() = None);
}

fn unhook(mouse: HHOOK, keyboard: HHOOK) {
    unsafe {
        UnhookWindowsHookEx(mouse).ok();
        UnhookWindowsHookEx(keyboard).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 10 MHz QPC (typical modern hardware): 1 tick = 0.1 µs, 10_000 ticks = 1 ms.
    const FREQ: i64 = 10_000_000;
    // First video frame at 500 ms since boot, in 100 ns units.
    const EPOCH: i64 = 5_000_000;
    // A 1920x1080 monitor whose top-left sits at virtual-screen (2560, 0) —
    // i.e. a second monitor to the right of a primary one.
    const ORIGIN: (i32, i32) = (2560, 0);
    const SIZE: (u32, u32) = (1920, 1080);

    // QPC count for `ms` after boot at FREQ (100ns units = ms * 10_000).
    fn qpc_at_ms(ms: f64) -> i64 {
        ((ms * 10_000.0) as i64 * FREQ) / 10_000_000
    }

    #[test]
    fn maps_to_surface_space_and_rebases_time() {
        // A click at global (2660, 100) → surface (100, 100), 100 ms in.
        let raw = vec![Stamped {
            qpc: qpc_at_ms(600.0), // 100 ms after the 500 ms epoch
            kind: Kind::Down { x: 2660, y: 100, button: MouseButton::Left },
        }];
        let out = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE);
        assert_eq!(
            out,
            vec![RawEvent::MouseDown { t: 100.0, x: 100.0, y: 100.0, button: MouseButton::Left }]
        );
    }

    #[test]
    fn drops_events_before_first_frame() {
        // 400 ms after boot is before the 500 ms epoch → negative t → dropped.
        let raw = vec![Stamped { qpc: qpc_at_ms(400.0), kind: Kind::KeyPress }];
        assert!(rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE).is_empty());
    }

    #[test]
    fn drops_mouse_events_outside_the_monitor() {
        // Global x=100 is on the *primary* monitor, left of this one's origin.
        let raw = vec![Stamped {
            qpc: qpc_at_ms(600.0),
            kind: Kind::Move { x: 100, y: 100 },
        }];
        assert!(rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE).is_empty());
    }

    #[test]
    fn keypress_survives_without_coordinates() {
        // KeyPress has no position, so the off-monitor clip never applies to it.
        let raw = vec![Stamped { qpc: qpc_at_ms(600.0), kind: Kind::KeyPress }];
        let out = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE);
        assert_eq!(out, vec![RawEvent::KeyPress { t: 100.0 }]);
    }

    #[test]
    fn output_is_time_sorted() {
        let raw = vec![
            Stamped { qpc: qpc_at_ms(700.0), kind: Kind::KeyPress },
            Stamped { qpc: qpc_at_ms(600.0), kind: Kind::KeyPress },
            Stamped { qpc: qpc_at_ms(650.0), kind: Kind::KeyPress },
        ];
        let ts: Vec<f64> = rebase_and_map(raw, FREQ, EPOCH, ORIGIN, SIZE)
            .iter()
            .map(RawEvent::t)
            .collect();
        assert_eq!(ts, vec![100.0, 150.0, 200.0]);
    }
}
