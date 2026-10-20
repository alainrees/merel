//! Keeps AppKit's Touch Bar bookkeeping from ending the app on macOS.
//!
//! AppKit watches the responder chain of the window's view to find what the Touch Bar should
//! show. AccessKit gives that view a class of its own at runtime, and winit closes windows
//! while AppKit still watches them; either way `-[_NSTouchBarFinderObservation invalidate]`
//! can later fail to stop watching and throws an Objective-C exception. Nothing catches it,
//! so the app aborts, sometimes minutes into normal use. Here `invalidate` gets a wrapper that
//! catches that exception: the observation is gone either way, and the app keeps running.
//! On other platforms [`guard`] does nothing.

#[cfg(target_os = "macos")]
mod macos {
    use std::panic::AssertUnwindSafe;
    use std::sync::OnceLock;

    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;

    type Invalidate = unsafe extern "C-unwind" fn(*mut AnyObject, Sel);

    static ORIGINAL: OnceLock<Invalidate> = OnceLock::new();

    unsafe extern "C-unwind" fn invalidate(this: *mut AnyObject, cmd: Sel) {
        let Some(original) = ORIGINAL.get().copied() else { return };
        // SAFETY: `original` is AppKit's own implementation of this very method, called with the
        // receiver and selector AppKit passed in.
        if let Err(e) = objc2::exception::catch(AssertUnwindSafe(|| unsafe { original(this, cmd) })) {
            log::warn!("Touch Bar observation failed to stop: {e:?}");
        }
    }

    /// Wraps `invalidate`; does nothing when AppKit has no such class.
    pub fn guard() {
        let Some(class) = AnyClass::get(c"_NSTouchBarFinderObservation") else { return };
        let Some(method) = class.instance_method(sel!(invalidate)) else { return };
        // SAFETY: `invalidate` takes no arguments and returns nothing, as the wrapper does; the
        // original is stored before the wrapper can run, as this runs once on the main thread
        // before the event loop starts.
        unsafe {
            let wrapper: Imp = std::mem::transmute::<Invalidate, Imp>(invalidate);
            let original = method.implementation();
            let _ = ORIGINAL.set(std::mem::transmute::<Imp, Invalidate>(original));
            method.set_implementation(wrapper);
        }
    }
}

/// Wraps AppKit's `invalidate` on macOS; does nothing elsewhere.
pub fn guard() {
    #[cfg(target_os = "macos")]
    macos::guard();
}
