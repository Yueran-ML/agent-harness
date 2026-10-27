//! No views on this platform: [`crate::View::attach`] fails with
//! [`crate::ViewError::Unsupported`].
#![allow(
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_ref_mut,
    reason = "the shape of the other platforms' modules"
)]

use std::sync::Arc;
use std::time::Duration;

use crate::{RawParent, Rect, Shared, ViewError};

pub(crate) fn prepare_process() {}

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
        Err(ViewError::Unsupported(
            "views are made on Windows and macOS only".into(),
        ))
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
    Err("views are made on Windows and macOS only".into())
}

/// The backends that the view draws with on this platform: none.
pub(crate) const BACKENDS: wgpu::Backends = wgpu::Backends::empty();
