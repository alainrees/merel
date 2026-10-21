//! Puts the macOS close, minimise and zoom buttons in the middle of a tall title bar.
//!
//! Merel draws its own title bar, as tall as Outlook's (52 points), under a see-through
//! macOS one. AppKit keeps its buttons at the top of a 28-point strip, so they would sit above
//! the search box instead of beside it. [`place`] stretches the strip that holds them to the
//! bar's height and centres them in it. AppKit lays the strip out again when the window
//! resizes, so the app calls this every frame; it only touches the views when they moved.
//! On other platforms, and in full screen (where macOS shows its own bar), it does nothing.

use raw_window_handle::HasWindowHandle;

/// Where the close button starts, from the window's left edge, in points.
pub const LEADING: f64 = 20.0;

/// The width the three buttons and the room after them take at the left of the bar, in points.
pub const WIDTH: f32 = 90.0;

/// Centres the window buttons in a title bar `height` points tall.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn place(window: &impl HasWindowHandle, height: f64) {
    #[cfg(target_os = "macos")]
    macos::place(window, height);
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn place(window: &impl HasWindowHandle, height: f64) {
        // AppKit's views may only be touched on the main thread, where eframe draws.
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        // SAFETY: winit hands out the `NSView` of a live window; the handle borrows the window,
        // so the view outlives this call, and we are on the main thread (checked above).
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let Some(window) = view.window() else { return };
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            return;
        }
        let buttons =
            [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton].map(|b| window.standardWindowButton(b));
        let [Some(close), Some(mini), Some(zoom)] = buttons else { return };
        // SAFETY: the buttons live in AppKit's title bar view inside its title bar container;
        // both are kept by the window, which we hold, for as long as we use them here.
        let Some(bar) = (unsafe { close.superview() }) else { return };
        // SAFETY: as above.
        let Some(strip) = (unsafe { bar.superview() }) else { return };

        // The strip runs along the window's top edge; the bar inside it fills it, so that the
        // buttons' middle is the strip's middle.
        let mut frame = strip.frame();
        let top = window.frame().size.height - height;
        if frame.size.height != height || frame.origin.y != top {
            frame.size.height = height;
            frame.origin.y = top;
            strip.setFrame(frame);
        }
        let inner = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
        if bar.frame() != inner {
            bar.setFrame(inner);
        }
        let gap = mini.frame().origin.x - close.frame().origin.x;
        for (i, button) in [close, mini, zoom].iter().enumerate() {
            let size = button.frame().size;
            // The same distance from the strip's top and bottom, whichever way it counts.
            let origin = NSPoint::new(super::LEADING + i as f64 * gap, ((height - size.height) / 2.0).round());
            if button.frame().origin != origin {
                button.setFrameOrigin(origin);
            }
        }
    }
}
