//! The system pointer is hidden while the eraser's circle stands in for it.
//!
//! Decided anew each frame: a page under the pointer asks while painting
//! ([`hide_this_frame`]) and the window applies the answer once everything is painted
//! ([`apply`]). A frame where nothing asks (pointer off the page, other tool, an overlay
//! on top, window inactive, tab closed) shows the pointer again.

use std::cell::Cell;

thread_local! {
    /// A page asked for the pointer to be hidden during this frame's paint.
    static WANTED: Cell<bool> = const { Cell::new(false) };
}

/// Ask for the system pointer to be hidden in the frame being painted.
pub fn hide_this_frame() {
    WANTED.set(true);
}

/// Hide or show the system pointer as asked during this frame; call after all pages paint.
pub fn apply() {
    platform::set_hidden(WANTED.take());
}

#[cfg(windows)]
mod platform {
    use std::cell::Cell;

    use windows::Win32::UI::WindowsAndMessaging::ShowCursor;

    thread_local! {
        static HIDDEN: Cell<bool> = const { Cell::new(false) };
    }

    /// `ShowCursor` keeps a per-thread display counter: at most one hide of ours is ever
    /// outstanding, and it is taken back before any other.
    pub fn set_hidden(hide: bool) {
        if HIDDEN.replace(hide) != hide {
            // SAFETY: a plain user32 call on the UI thread, no pointers involved.
            unsafe { ShowCursor(!hide) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    // shortcut: gpui has no hidden cursor style, so on macOS / Linux the system pointer stays
    // over the eraser's circle; hide it through the platform API when those ports need it.
    pub fn set_hidden(_hide: bool) {}
}
