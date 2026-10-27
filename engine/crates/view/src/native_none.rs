//! No views on this platform: [`crate::View::attach`] fails at once with
//! [`crate::ViewError::Unsupported`], and nothing of the platform's windows is touched.
//!
//! **macOS** is here too, for phase 0. A view there would be an `NSView` added to the parent's,
//! but neither half of that can work for the engine as it is:
//!
//! - the handle that Electron gives (`getNativeWindowHandle()`) is an `NSView *` in the memory of
//!   Electron's process, and means nothing in the engine's;
//! - `AppKit` is used on the main thread only, through the main queue of dispatch, and the
//!   engine's main thread never serves that queue (it waits in the runtime of tokio), so that a
//!   call sent to it would wait for ever.
//!
//! A view of the engine shown in a window of Electron's needs another way on macOS: a layer of
//! the engine shown in Electron's window with `CALayerHost`, or the pictures handed over as
//! `IOSurface`s to a module of Electron's process. Until then the desktop shows the preview
//! another way there (see "The view" in `PROTOCOL.md`). The pictures are still drawn with Metal
//! offscreen, as the tests of `render` do.
#![allow(
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_ref_mut,
    reason = "the shape of the module of Windows"
)]

use std::sync::Arc;
use std::time::Duration;

use crate::{RawParent, Rect, Shared, ViewError};

/// Why there is no view.
const UNSUPPORTED: &str = if cfg!(target_os = "macos") {
    "on macOS the view is not available yet: the desktop shows the preview another way"
} else {
    "views are made on Windows only"
};

/// The backends that pictures are drawn with on this platform, offscreen.
pub(crate) const BACKENDS: wgpu::Backends = if cfg!(target_os = "macos") {
    wgpu::Backends::METAL
} else {
    wgpu::Backends::empty()
};

pub(crate) fn prepare_process() {}

pub(crate) fn dpi_awareness() -> String {
    "not applicable".into()
}

pub(crate) struct Native;

#[derive(Clone)]
pub(crate) struct Syncer;

impl Syncer {
    pub(crate) fn sync(&self) {}
}

impl Native {
    pub(crate) fn open(
        _parent: RawParent,
        _rect: Rect,
        _shared: &Arc<Shared>,
    ) -> Result<Native, ViewError> {
        Err(ViewError::Unsupported(UNSUPPORTED.into()))
    }

    pub(crate) fn handle(&self) -> usize {
        0
    }

    pub(crate) fn syncer(&self) -> Syncer {
        Syncer
    }

    pub(crate) fn close(&mut self, _within: Duration) {}
}

pub(crate) fn create_surface(
    _instance: &wgpu::Instance,
    _handle: usize,
) -> Result<wgpu::Surface<'static>, String> {
    Err(UNSUPPORTED.into())
}
