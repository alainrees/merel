//! Puts the macOS close, minimise and zoom buttons in the middle of a tall title bar.
//!
//! Merel draws its own title bar, as tall as Outlook's (52 points), under a see-through
//! macOS one. AppKit keeps its buttons at the top of a 28-point strip, so they would sit above
//! the search box instead of beside it. [`place`] stretches the strip that holds them to the
//! bar's height and centres them in it. AppKit lays the strip out again when the window
//! resizes, so the app calls this every frame; it only touches the views when they moved.
//! On other platforms, and in full screen (where macOS shows its own bar), it does nothing.
//!
//! The red button closes Merel's window but not Merel, as in Outlook for Mac: the app hides
//! the window and keeps fetching mail. [`watch_dock_clicks`] tells it when the person clicks
//! the Dock icon, so the window can come back; winit itself does not pass that click on.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

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

type Waker = Box<dyn Fn() + Send + Sync>;

static DOCK_CLICKED: AtomicBool = AtomicBool::new(false);

static WAKER: OnceLock<Waker> = OnceLock::new();

/// Starts listening for clicks on the app's Dock icon; `wake` runs on each one, so the app
/// looks at [`take_dock_click`] even while its window is hidden. Call it once the event loop
/// exists. Does nothing on other platforms.
pub fn watch_dock_clicks(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(wake));
    #[cfg(target_os = "macos")]
    macos::watch_dock_clicks();
}

/// Whether the Dock icon was clicked since the last call.
pub fn take_dock_click() -> bool {
    DOCK_CLICKED.swap(false, Ordering::Relaxed)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn dock_clicked() {
    DOCK_CLICKED.store(true, Ordering::Relaxed);
    if let Some(wake) = WAKER.get() {
        wake();
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::encode::Encode;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSView, NSWindowButton, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type Reopen = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool;

    /// `-applicationShouldHandleReopen:hasVisibleWindows:`, which AppKit sends the app's
    /// delegate when the person clicks the Dock icon of a running app.
    unsafe extern "C-unwind" fn reopen(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, _visible: Bool) -> Bool {
        super::dock_clicked();
        // AppKit's own handling after this (bringing back a minimised window) may go ahead.
        Bool::YES
    }

    /// Teaches winit's application delegate to answer clicks on the Dock icon.
    pub(super) fn watch_dock_clicks() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else {
            log::warn!("no application delegate; a click on the Dock icon will not show the window");
            return;
        };
        let object: &AnyObject = delegate.as_ref();
        let class: &AnyClass = object.class();
        let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        if class.responds_to(selector) {
            log::warn!("the application delegate already answers Dock clicks; not replacing that");
            return;
        }
        let object_type = <*mut AnyObject>::ENCODING;
        let types = format!("{}{object_type}{}{object_type}{}", Bool::ENCODING, Sel::ENCODING, Bool::ENCODING);
        let Ok(types) = std::ffi::CString::new(types) else { return };
        // SAFETY: `reopen` takes the receiver, the selector and the two arguments AppKit sends
        // with this selector, and returns a BOOL, as `types` says; the class is winit's own
        // delegate class, which lives for the whole run, and we are on the main thread.
        let added = unsafe {
            let imp: Imp = std::mem::transmute::<Reopen, Imp>(reopen);
            objc2::ffi::class_addMethod(std::ptr::from_ref(class).cast_mut(), selector, imp, types.as_ptr())
        };
        if !added.as_bool() {
            log::warn!("could not listen for clicks on the Dock icon");
        }
    }

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
