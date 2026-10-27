//! The view on macOS: an `NSView` of its own, added to the parent `NSView`, drawn with Metal
//! through a `CAMetalLayer` that wgpu puts on it. It answers no hit test, so that the clicks of
//! the user go to the page beneath it.
//!
//! **Not tested**: it is only checked to compile, for `aarch64-apple-darwin`. And it can only
//! work where the parent view lives **in this process**: an `NSView *` is an address in the
//! memory of its process, and the handle that Electron gives (`getNativeWindowHandle()`) is one
//! of Electron's. A view of the engine, in a process of its own, inside a window of Electron's
//! needs another way on macOS (a layer of another process shown with `CALayerHost`, or the
//! pictures handed over as `IOSurface`s): see the notes for Task 17 in `PROTOCOL.md`.
//!
//! AppKit is used on the main thread only: the view is made, moved and removed there, through
//! the main queue of dispatch, which the main thread of the process must serve (as the run loop
//! of an `NSApplication` does). Drawing is done by the drawer's thread, as Metal allows.

use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSResponder, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use crate::{RawParent, Rect, Shared, ViewError};

/// The backends that the view draws with on macOS.
pub(crate) const BACKENDS: wgpu::Backends = wgpu::Backends::METAL;

define_class!(
    // SAFETY: NSView may be subclassed, and the methods below override two of its own with the
    // same signatures; the class has no ivars and no Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ColumbinaView"]
    struct PassingView;

    impl PassingView {
        /// No point is in the view, so that the page beneath gets the input.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        /// Its rows go down, as the page's do.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

pub(crate) fn prepare_process() {}

/// A view of a view, and the state it follows.
pub(crate) struct Native {
    /// The `PassingView`, retained, as an address: it is only touched on the main thread.
    view: usize,
    shared: Arc<Shared>,
    closed: bool,
}

/// What asks the main thread to make the view match the view's state.
#[derive(Clone)]
pub(crate) struct Syncer {
    view: usize,
    shared: Arc<Shared>,
}

impl Syncer {
    pub(crate) fn sync(&self) {
        let (view, shared) = (self.view, Arc::clone(&self.shared));
        DispatchQueue::main().exec_async(move || {
            let Some(_main) = MainThreadMarker::new() else {
                return;
            };
            // SAFETY: on the main thread, and the view is retained until `Native::close`,
            // which is run on the main queue after this.
            let view = unsafe { &*(view as *const PassingView) };
            sync(view, &shared);
        });
    }
}

/// Make the view match the state: its frame, in points of the parent, and whether it is hidden.
fn sync(view: &PassingView, shared: &Shared) {
    let (rect, visible) = {
        let state = shared.lock();
        (state.rect(), state.visible())
    };
    if let Some(parent) = view.superview() {
        view.setFrame(frame(&parent, rect));
    }
    view.setHidden(!visible);
    shared.change(|state| state.sized((rect.width, rect.height)));
}

/// The frame in the points of `parent` of a rectangle in its device pixels from its top left.
#[allow(clippy::cast_precision_loss, reason = "rectangles of at most 16384")]
fn frame(parent: &NSView, rect: Rect) -> NSRect {
    let scale = parent
        .window()
        .map_or(1.0, |window| window.backingScaleFactor());
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (x, width, height) = (
        f64::from(rect.x) / scale,
        f64::from(rect.width) / scale,
        f64::from(rect.height) / scale,
    );
    let top = f64::from(rect.y) / scale;
    let y = if parent.isFlipped() {
        top
    } else {
        parent.bounds().size.height - top - height
    };
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

impl Native {
    /// Make the view in `parent`, on the main thread.
    pub(crate) fn open(
        parent: RawParent,
        rect: Rect,
        shared: &Arc<Shared>,
    ) -> Result<Native, ViewError> {
        let RawParent::MacOS(parent) = parent else {
            return Err(ViewError::Unsupported(
                "on macOS the parent of a view is an NSView".into(),
            ));
        };
        if parent == 0 {
            return Err(ViewError::Parent("0 is not a view".into()));
        }
        let shared_main = Arc::clone(shared);
        let view = dispatch2::run_on_main(move |main| {
            // SAFETY: the caller gives an `NSView *` of this process, which its window keeps
            // alive while the view is in it; it is only used here, on the main thread.
            let parent = unsafe { &*(parent as *const NSView) };
            let view: Retained<PassingView> = unsafe {
                msg_send![main.alloc::<PassingView>(), initWithFrame: frame(parent, rect)]
            };
            parent.addSubview(&view);
            sync(&view, &shared_main);
            Retained::into_raw(view) as usize
        });
        Ok(Native {
            view,
            shared: Arc::clone(shared),
            closed: false,
        })
    }

    pub(crate) fn handle(&self) -> usize {
        self.view
    }

    pub(crate) fn syncer(&self) -> Syncer {
        Syncer {
            view: self.view,
            shared: Arc::clone(&self.shared),
        }
    }

    /// Remove the view from its parent, and let it go, on the main thread.
    pub(crate) fn close(&mut self, _within: Duration) {
        if std::mem::replace(&mut self.closed, true) {
            return;
        }
        let view = self.view;
        let shared = Arc::clone(&self.shared);
        dispatch2::run_on_main(move |_main| {
            // SAFETY: the view was retained by `open`, and is let go once, here.
            let view = unsafe { Retained::from_raw(view as *mut PassingView) };
            if let Some(view) = view {
                view.removeFromSuperview();
            }
        });
        shared.window_gone();
    }
}

/// The surface of the view of `handle`, made on the main thread, where wgpu puts its layer on
/// the view.
pub(crate) fn create_surface(
    instance: &wgpu::Instance,
    handle: usize,
) -> Result<wgpu::Surface<'static>, String> {
    let view = NonNull::new(handle as *mut core::ffi::c_void).ok_or("the view has no NSView")?;
    let view = view.as_ptr() as usize;
    dispatch2::run_on_main(move |_main| {
        let view = NonNull::new(view as *mut core::ffi::c_void).expect("not null");
        // SAFETY: the view outlives the surface: the view lets its drawer, which holds the
        // surface, end before the view is removed.
        unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
            })
        }
        .map_err(|error| format!("the view takes no surface: {error}"))
    })
}
