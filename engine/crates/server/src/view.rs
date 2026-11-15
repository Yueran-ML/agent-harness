//! The methods `view.*`: the picture drawn by the engine in a window of its own inside the
//! desktop's window (see `columbina-view`, and `PROTOCOL.md`).
//!
//! Each monitor has at most one view, belonging to the engine, not the connection that made
//! it: it stays when that connection closes, and goes when detached or replaced on that monitor,
//! or when the engine ends. A view is made and destroyed on threads where blocking is
//! allowed: its own threads make and serve the window, and the card is made ready on one of them.
//!
//! The transport hands the view its pictures ([`ViewSlot`]): the slot is locked only for a
//! moment, to hand a picture over or to put a view in or take it out, never while a view is made
//! or destroyed.
//!
//! The view draws with the engine's card ([`Graphics`]), made when the first view is attached.
//! The viewer subscribes to it: each card made in place of one that was lost is given to the view
//! there is then; and a view that reports its card lost is given the card that is there, made
//! anew if need be.
//!
//! Whether the views may show HDR, where Windows has it on for their monitor ("Display HDR On
//! Viewers If Available", RM20 p244), is a setting of the viewer, for both monitors, allowed
//! unless `view.display` says otherwise; each view attached since is given it.
//!
//! A view takes the pointer while a connection of the desktop asks it to (`view.pointer`), for a
//! tool of the page that is used on the picture; what the pointer does there is told to that
//! connection alone, as `view.pointer` notifications ([`pointer`](mod@pointer)).
//!
//! A page of the desktop may have the view of the timeline monitor draw lines and dots over its
//! picture (`view.overlay`): the motion path of a clip on the Edit page. They are that
//! connection's, drawn until it says otherwise or closes, and given to a view made in place of
//! the one that drew them.

mod pointer;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};

use columbina_compose::ShaderCompiler;
use columbina_view::marks::{DotKind, Dots, Marks};
use columbina_view::{
    Cursor, Event, Gpu, Navigation, Output, RawParent, Rect, View, ViewError, Viewport,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::Mutex;

use crate::gpu::{Graphics, Subscription};
use crate::log::log;
use crate::queue;
use crate::rpc::{self, Fault};
use crate::session::Kind;
use pointer::Pointers;

/// Where the view of the engine is, if one is attached: shared with the transport, which hands
/// it pictures.
pub type ViewSlot = Arc<std::sync::Mutex<Option<AttachedView>>>;

/// One successful attachment, independent of its monitor or native handle.
pub struct AttachedView {
    pub instance: u64,
    view: View,
}
impl std::ops::Deref for AttachedView {
    type Target = View;
    fn deref(&self) -> &View {
        &self.view
    }
}

/// The source and the edited timeline have separate pictures, windows and playheads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Monitor {
    Source,
    Timeline,
}

impl Monitor {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Source => 0,
            Self::Timeline => 1,
        }
    }
}

/// The view of the engine, if one is attached.
pub struct Viewer {
    /// Whether the engine is headless, and so draws in no window.
    headless: bool,
    slots: [ViewSlot; 2],
    /// The card that the view draws with.
    graphics: Arc<Graphics>,
    /// Gives the view each card made in place of one that was lost.
    _subscription: Subscription,
    /// Held while a method changes the view, so that one is done before the next begins.
    busy: Mutex<()>,
    /// Whether the views may show HDR.
    hdr: AtomicBool,
    /// Who is told what the pointer does on each view, while one takes it.
    pointers: Pointers,
    pub frames: crate::presentation::Registry,
    next_instance: AtomicU64,
    /// Viewer-only geometry, retained when a native window is replaced or detached.
    viewports: std::sync::Mutex<[Viewport; 2]>,
    /// What a page has the view of each monitor draw over its picture, and the connection
    /// that asked: retained when a native window is replaced, until that connection closes.
    overlays: std::sync::Mutex<[Option<Asked>; 2]>,
}

/// The marks that a page has a view draw over its picture, with the connection that asked.
type Asked = (u64, Arc<Marks>);

/// One pointer ownership change in the existing view-operation order. Native preparation
/// happens without State; the caller publishes the sink while committing its State change.
pub struct PointerChange<'a> {
    viewer: &'a Viewer,
    _busy: tokio::sync::MutexGuard<'a, ()>,
    asked: TakePointer,
    connection: u64,
    sender: queue::Sender,
    before: Option<Cursor>,
    previous_owner: Option<u64>,
}

impl PointerChange<'_> {
    pub fn owned_by(&self, connection: u64) -> bool {
        self.previous_owner == Some(connection)
    }

    pub async fn apply(&self) -> Result<(), Fault> {
        let cursor = self.asked.on.then_some(Cursor::from(self.asked.cursor));
        let taken = self
            .viewer
            .with_view_ordered(self.asked.monitor, |view| view.take_pointer(cursor))
            .await;
        if self.asked.on {
            taken?;
        }
        Ok(())
    }

    /// Restore the native cursor captured under this turn, with State already released.
    pub async fn rollback(&self) {
        let _ = self
            .viewer
            .with_view_ordered(self.asked.monitor, |view| view.take_pointer(self.before))
            .await;
    }

    pub fn commit(&self) -> serde_json::Value {
        let monitor = self.asked.monitor;
        if self.asked.on {
            if let Some((instance, shown)) = self.viewer.presentation_snapshot(monitor) {
                self.viewer.frames.observe(monitor, instance, shown);
            }
            self.viewer.pointers.tell(
                monitor,
                self.connection,
                self.sender.clone(),
                Some(Cursor::from(self.asked.cursor)),
            );
        } else {
            self.viewer.pointers.stop(monitor);
        }
        json!({"monitor":monitor,"on":self.asked.on,"cursor":self.asked.on.then_some(self.asked.cursor)})
    }
}

/// The params of `view.attach`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attach {
    monitor: Monitor,
    parent: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// The params of `view.place`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Place {
    monitor: Monitor,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// The params of `view.show`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Show {
    monitor: Monitor,
    shown: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Detach {
    monitor: Monitor,
}

/// The params of `view.pointer`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakePointer {
    monitor: Monitor,
    on: bool,
    #[serde(default)]
    cursor: CursorName,
}

/// The cursor shown over a view that takes the pointer, as it is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CursorName {
    #[default]
    Arrow,
    Crosshair,
    Move,
    ResizeEw,
    ResizeNs,
}

impl From<CursorName> for Cursor {
    fn from(name: CursorName) -> Cursor {
        match name {
            CursorName::Arrow => Cursor::Arrow,
            CursorName::Crosshair => Cursor::Crosshair,
            CursorName::Move => Cursor::Move,
            CursorName::ResizeEw => Cursor::ResizeEw,
            CursorName::ResizeNs => Cursor::ResizeNs,
        }
    }
}

/// The params of `view.display`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayParams {
    #[serde(default)]
    hdr: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewportQuery {
    monitor: Monitor,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Center {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
enum ViewportBase {
    Fit,
    Scale { scale: f64, center: Center },
}
impl From<ViewportBase> for Viewport {
    fn from(base: ViewportBase) -> Self {
        match base {
            ViewportBase::Fit => Self::Fit,
            ViewportBase::Scale { scale, center } => Self::Scale {
                scale,
                center: [center.x, center.y],
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigationGeometry {
    surface: (u32, u32),
    picture: (u32, u32),
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
enum ViewportNavigation {
    Prepare {
        monitor: Monitor,
        instance: Option<u64>,
    },
    Zoom {
        monitor: Monitor,
        instance: u64,
        factor: f64,
        at: Option<Center>,
    },
    Pan {
        monitor: Monitor,
        instance: u64,
        base: ViewportBase,
        geometry: NavigationGeometry,
        offset: Center,
    },
    Scale {
        monitor: Monitor,
        instance: u64,
        scale: f64,
    },
    Fit {
        monitor: Monitor,
        instance: u64,
    },
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
enum ViewportSetting {
    Fit {
        monitor: Monitor,
    },
    Scale {
        monitor: Monitor,
        scale: f64,
        center: Center,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ViewportParams {
    Query(ViewportQuery),
    Set(ViewportSetting),
    Navigate(ViewportNavigation),
}

const ATTACH: &str = r#"{"monitor":"source"|"timeline", "parent": "<handle in decimal>", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const PLACE: &str =
    r#"{"monitor":"source"|"timeline", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const SHOW: &str = r#"{"monitor":"source"|"timeline", "shown": true | false}"#;
const DISPLAY: &str = r#"{"hdr"?: true | false}"#;
const VIEWPORT: &str = "query {monitor}; absolute {monitor,mode:fit|scale,...}; navigation {monitor,operation:prepare|zoom|pan|scale|fit,instance,...}; see PROTOCOL.md";
const POINTER: &str = r#"{"monitor":"source"|"timeline", "on": true | false, "cursor"?: "arrow" | "crosshair" | "move" | "resize_ew" | "resize_ns"}"#;
const OVERLAY: &str = r#"{"monitor":"timeline", "overlay"?: {"canvas": [1920, 1080], "lines"?: [[[x, y], ...], ...], "dots"?: [{"kind": "tick" | "key" | "active" | "current", "at": [[x, y], ...]}, ...]} | null}"#;

/// The most points that the lines and the dots of an overlay have together.
pub const OVERLAY_POINTS: usize = 32_768;
/// How far from the corner of the picture a point of an overlay may lie, in its pixels: far
/// beyond any picture, and well within what the card counts exactly.
const OVERLAY_REACH: f64 = 1e7;

/// The params of `view.overlay`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "an overlay left out, null and given are told apart"
)]
struct OverlayParams {
    monitor: Monitor,
    /// Left out, the overlay that is drawn is asked for; null, nothing is to be drawn.
    #[serde(default, deserialize_with = "written")]
    overlay: Option<Option<Drawn>>,
}

/// An overlay that is written, even as `null`, as some: so that one to be cleared is told from
/// a question.
#[allow(
    clippy::option_option,
    reason = "an overlay left out and null are told apart"
)]
fn written<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<Drawn>>, D::Error> {
    Option::<Drawn>::deserialize(deserializer).map(Some)
}

/// What a page has a view draw, as it is written: lines and dots in pixels of a picture of
/// `canvas` pixels.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Drawn {
    canvas: (u32, u32),
    #[serde(default)]
    lines: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    dots: Vec<DrawnDots>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DrawnDots {
    kind: DotName,
    at: Vec<[f64; 2]>,
}

/// What a dot marks, as it is written.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DotName {
    Tick,
    Key,
    Active,
    Current,
}

impl From<DotName> for DotKind {
    fn from(name: DotName) -> DotKind {
        match name {
            DotName::Tick => DotKind::Tick,
            DotName::Key => DotKind::Key,
            DotName::Active => DotKind::Active,
            DotName::Current => DotKind::Current,
        }
    }
}

impl Drawn {
    /// The marks that the view draws, once they are checked: a picture that a view may show,
    /// no more points than [`OVERLAY_POINTS`], and each within reach of the picture.
    fn marks(self) -> Result<Marks, Fault> {
        let refused =
            |message: String| Fault::params(message).with_data(json!({"reason":"overlay"}));
        let (width, height) = self.canvas;
        if width == 0 || height == 0 || width > Rect::MOST || height > Rect::MOST {
            return Err(refused(format!(
                "the canvas of view.overlay is the size of a picture, from 1 to {} on a side, and {width}×{height} is not one",
                Rect::MOST
            )));
        }
        let points = self.lines.iter().map(Vec::len).sum::<usize>()
            + self.dots.iter().map(|dots| dots.at.len()).sum::<usize>();
        if points > OVERLAY_POINTS {
            return Err(refused(format!(
                "an overlay has at most {OVERLAY_POINTS} points in its lines and dots, and this one has {points}"
            )));
        }
        let near = |point: &[f64; 2]| point.iter().all(|v| v.abs() <= OVERLAY_REACH);
        if !self.lines.iter().flatten().all(near)
            || !self.dots.iter().flat_map(|dots| &dots.at).all(near)
        {
            return Err(refused(format!(
                "a point of an overlay lies within {OVERLAY_REACH} pixels of the corner of its picture"
            )));
        }
        Ok(Marks {
            picture: self.canvas,
            lines: self.lines,
            dots: self
                .dots
                .into_iter()
                .map(|dots| Dots {
                    kind: dots.kind.into(),
                    at: dots.at,
                })
                .collect(),
        })
    }
}

impl Viewer {
    #[cfg(test)]
    pub async fn test_busy(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.busy.lock().await
    }

    #[cfg(test)]
    pub fn test_pointer_owner(&self, monitor: Monitor) -> Option<u64> {
        self.pointers.owner(monitor)
    }

    pub fn new(headless: bool, graphics: Arc<Graphics>) -> Self {
        let slots: [ViewSlot; 2] = std::array::from_fn(|_| Arc::new(std::sync::Mutex::new(None)));
        let subscription = graphics.subscribe({
            let slots = slots.each_ref().map(Arc::downgrade);
            move |gpu| {
                for slot in &slots {
                    give(slot, Arc::clone(&gpu));
                }
            }
        });
        let frames = crate::presentation::Registry::default();
        Self {
            headless,
            slots,
            graphics,
            _subscription: subscription,
            busy: Mutex::new(()),
            hdr: AtomicBool::new(true),
            pointers: Pointers::new(frames.clone()),
            frames,
            next_instance: AtomicU64::new(0),
            viewports: std::sync::Mutex::new([Viewport::Fit; 2]),
            overlays: std::sync::Mutex::new([None, None]),
        }
    }

    /// `view.overlay`, of the connection `connection` of the desktop: have the view of the
    /// timeline monitor draw lines and dots over its picture, in place of those drawn before
    /// (whoever asked for them), or none; or, with no `overlay`, say what it draws. What is
    /// to be drawn is kept where no view is attached yet, and given to the next.
    pub async fn overlay(
        &self,
        kind: Kind,
        connection: u64,
        params: Option<&RawValue>,
    ) -> Result<serde_json::Value, Fault> {
        let method = "view.overlay";
        self.check(kind, method)?;
        let asked: OverlayParams = rpc::params(method, OVERLAY, params)?;
        let monitor = asked.monitor;
        if monitor != Monitor::Timeline {
            // The picture of the source monitor is media, turned as it says: the page has
            // nothing to draw over it, and no pixels of a canvas to draw it in.
            return Err(
                Fault::params("view.overlay draws over the timeline monitor alone")
                    .with_data(json!({"reason":"monitor"})),
            );
        }
        let _busy = self.busy.lock().await;
        let mut overlays = self.overlays.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(drawn) = asked.overlay else {
            let kept = overlays[monitor.index()].as_ref();
            return Ok(overlay_written(monitor, kept.map(|(_, marks)| &**marks)));
        };
        let marks = drawn.map(Drawn::marks).transpose()?.map(Arc::new);
        if let Some(view) = self.slots[monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            // A view whose window is gone is let go by the next method that places it.
            let _ = view.set_marks(marks.clone());
        }
        let answer = overlay_written(monitor, marks.as_deref());
        overlays[monitor.index()] = marks.map(|marks| (connection, marks));
        Ok(answer)
    }

    /// `view.pointer`, of the connection `connection` of the desktop: have the view of a monitor
    /// take the pointer, and tell what it does there to this connection alone (in place of one
    /// that asked before), or let it go.
    pub async fn prepare_pointer(
        &self,
        kind: Kind,
        connection: u64,
        sender: &queue::Sender,
        params: Option<&RawValue>,
    ) -> Result<PointerChange<'_>, Fault> {
        let method = "view.pointer";
        self.check(kind, method)?;
        let asked: TakePointer = rpc::params(method, POINTER, params)?;
        let busy = self.busy.lock().await;
        let before = self.slots[asked.monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|v| v.pointer());
        let previous_owner = self.pointers.owner(asked.monitor);
        Ok(PointerChange {
            viewer: self,
            _busy: busy,
            asked,
            connection,
            sender: sender.clone(),
            before,
            previous_owner,
        })
    }

    /// Forget a connection that has ended: the views that took the pointer for it let it go,
    /// and those that drew an overlay for it draw it no more.
    pub async fn forget(&self, connection: u64) {
        let _busy = self.busy.lock().await;
        for monitor in self.pointers.forget(connection) {
            if let Some(view) = self.slots[monitor.index()]
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            {
                let _ = view.take_pointer(None);
            }
        }
        let mut overlays = self.overlays.lock().unwrap_or_else(PoisonError::into_inner);
        for (kept, slot) in overlays.iter_mut().zip(&self.slots) {
            if kept.as_ref().is_some_and(|(owner, _)| *owner == connection) {
                *kept = None;
                if let Some(view) = slot.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    let _ = view.set_marks(None);
                }
            }
        }
    }

    /// Refuse a method of the view where there is none, or the client is not the desktop.
    fn check(&self, kind: Kind, method: &str) -> Result<(), Fault> {
        if self.headless {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method} in a headless engine: it draws in no window"),
            ));
        }
        if kind != Kind::Desktop {
            return Err(Fault::new(
                rpc::INVALID_REQUEST,
                format!("{method} is the desktop's: only it has the window that the view is in"),
            ));
        }
        Ok(())
    }

    /// Where the view is, for the transport.
    pub fn slot(&self, monitor: Monitor) -> ViewSlot {
        Arc::clone(&self.slots[monitor.index()])
    }

    /// Pure synchronous copy, with State -> `ViewSlot` -> View.shared. Registry guards must
    /// already be released; consult the registry only after these view guards are released.
    pub fn presentation_snapshot(
        &self,
        monitor: Monitor,
    ) -> Option<(u64, Option<columbina_view::Presentation>)> {
        let slot = self.slots[monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.as_ref()
            .map(|view| (view.instance, view.presentation()))
    }

    fn take(&self, monitor: Monitor) -> Option<AttachedView> {
        let old = self.slots[monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.frames.activate(monitor, None);
        self.pointers.activate(monitor, None);
        old
    }

    /// Call a method `view.*` for a client of `kind`, and give its result.
    pub async fn call(
        &self,
        kind: Kind,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<serde_json::Value, Fault> {
        self.check(kind, method)?;
        match method {
            "view.attach" => self.attach(rpc::params(method, ATTACH, params)?).await,
            "view.place" => {
                let place: Place = rpc::params(method, PLACE, params)?;
                let rect = Rect {
                    x: place.x,
                    y: place.y,
                    width: place.width,
                    height: place.height,
                };
                self.with_view(place.monitor, |view| view.place(rect)).await
            }
            "view.show" => {
                let show: Show = rpc::params(method, SHOW, params)?;
                self.with_view(show.monitor, |view| view.show(show.shown))
                    .await
            }
            "view.detach" => {
                let detach: Detach =
                    rpc::params(method, r#"{"monitor":"source"|"timeline"}"#, params)?;
                let _busy = self.busy.lock().await;
                let old = self.take(detach.monitor);
                let detached = old.is_some();
                drop_view(old).await;
                if detached {
                    log("view_detached", &json!({"monitor":detach.monitor}));
                }
                Ok(json!({ "detached": detached }))
            }
            "view.display" => {
                let params: DisplayParams = rpc::params(method, DISPLAY, params)?;
                Ok(self.display(params.hdr).await)
            }
            "view.viewport" => self.viewport(rpc::params(method, VIEWPORT, params)?).await,
            _ => Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            )),
        }
    }

    async fn viewport(&self, params: ViewportParams) -> Result<serde_json::Value, Fault> {
        let (monitor, requested) = match params {
            ViewportParams::Navigate(operation) => return self.navigate(operation).await,
            ViewportParams::Query(query) => (query.monitor, None),
            ViewportParams::Set(ViewportSetting::Fit { monitor }) => (monitor, Some(Viewport::Fit)),
            ViewportParams::Set(ViewportSetting::Scale {
                monitor,
                scale,
                center,
            }) => (
                monitor,
                Some(Viewport::Scale {
                    scale,
                    center: [center.x, center.y],
                }),
            ),
        };
        let requested = requested
            .map(Viewport::checked)
            .transpose()
            .map_err(Fault::params)?;
        let _busy = self.busy.lock().await;
        let mut settings = self
            .viewports
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(viewport) = requested {
            let slot = self.slots[monitor.index()]
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(view) = slot.as_ref() {
                match view.set_viewport(viewport) {
                    Ok(()) | Err(ViewError::Gone) => {}
                    Err(error) => return Err(fault(&error)),
                }
            }
            settings[monitor.index()] = viewport;
        }
        Ok(viewport_written(monitor, settings[monitor.index()]))
    }

    async fn navigate(&self, operation: ViewportNavigation) -> Result<serde_json::Value, Fault> {
        let (monitor, instance, intention) = match operation {
            ViewportNavigation::Prepare { monitor, instance } => (monitor, instance, None),
            ViewportNavigation::Zoom {
                monitor,
                instance,
                factor,
                at,
            } => (
                monitor,
                Some(instance),
                Some(Navigation::Zoom {
                    factor,
                    at: at.map(|p| (p.x, p.y)),
                }),
            ),
            ViewportNavigation::Pan {
                monitor,
                instance,
                base,
                geometry,
                offset,
            } => (
                monitor,
                Some(instance),
                Some(Navigation::Pan {
                    base: base.into(),
                    surface: geometry.surface,
                    picture: geometry.picture,
                    offset: (offset.x, offset.y),
                }),
            ),
            ViewportNavigation::Scale {
                monitor,
                instance,
                scale,
            } => (monitor, Some(instance), Some(Navigation::Scale(scale))),
            ViewportNavigation::Fit { monitor, instance } => {
                (monitor, Some(instance), Some(Navigation::Fit))
            }
        };
        let _busy = self.busy.lock().await;
        let slot = self.slots[monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let unavailable = || {
            Fault::new(
                rpc::VIEW_FAILED,
                "navigation native instance is unavailable",
            )
            .with_data(json!({"reason":"instance"}))
        };
        let view = slot.as_ref().ok_or_else(unavailable)?;
        if instance.is_some_and(|instance| instance != view.instance) {
            return Err(unavailable());
        }
        let (requested, shown) = view.navigate(intention).map_err(|error| fault(&error))?;
        if intention.is_some() {
            self.viewports
                .lock()
                .unwrap_or_else(PoisonError::into_inner)[monitor.index()] = requested;
        }
        let mut answer = viewport_written(monitor, requested);
        answer["instance"] = json!(view.instance);
        answer["geometry"] = json!({"surface":shown.surface,"picture":shown.picture});
        if intention.is_none() {
            if let Some(buttons) = columbina_view::pointer::held_buttons() {
                answer["buttons"] = json!([(1, "left"), (2, "right"), (4, "middle")]
                    .into_iter()
                    .filter(|(bit, _)| buttons & bit != 0)
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>());
            }
        }
        Ok(answer)
    }

    /// `view.display`: allow HDR or not, where `hdr` is given, and say what each view shows.
    async fn display(&self, hdr: Option<bool>) -> serde_json::Value {
        let _busy = self.busy.lock().await;
        if let Some(hdr) = hdr {
            if self.hdr.swap(hdr, Ordering::SeqCst) != hdr {
                log("view_hdr_allowed", &json!({ "hdr": hdr }));
            }
        }
        let allowed = self.hdr.load(Ordering::SeqCst);
        let mut result = json!({ "hdr": allowed });
        for (monitor, name) in [(Monitor::Source, "source"), (Monitor::Timeline, "timeline")] {
            let slot = self.slots[monitor.index()]
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            result[name] = slot.as_ref().map_or(serde_json::Value::Null, |view| {
                view.allow_hdr(allowed);
                display_json(view.display())
            });
        }
        result
    }

    /// `view.attach`: make the view, in place of the one there was.
    async fn attach(&self, attach: Attach) -> Result<serde_json::Value, Fault> {
        let parent = RawParent::from_decimal(&attach.parent).ok_or_else(|| {
            Fault::params(format!(
                "the parent of view.attach is the native handle of a window, written in decimal, \
                 and {:?} is not one",
                attach.parent
            ))
            .with_data(json!({ "reason": "parent" }))
        })?;
        let rect = Rect {
            x: attach.x,
            y: attach.y,
            width: attach.width,
            height: attach.height,
        };
        let _busy = self.busy.lock().await;
        let instance = self.next_instance.fetch_add(1, Ordering::Relaxed) + 1;
        let graphics = Arc::clone(&self.graphics);
        let slot = self.slot(attach.monitor);
        let reporter = reporter(
            &graphics,
            &slot,
            attach.monitor,
            instance,
            &self.pointers,
            &self.frames,
        );
        let hdr = self.hdr.load(Ordering::SeqCst);
        let made = tokio::task::spawn_blocking(move || {
            let gpu = graphics
                .gpu()
                .map_err(|error| ViewError::Gpu(error.to_string()))?;
            let compiler = gpu.shader_compiler().map(ShaderCompiler::name);
            View::attach_with_hdr(parent, rect, gpu, reporter, hdr).map(|view| (view, compiler))
        })
        .await
        .map_err(|error| {
            Fault::new(
                rpc::VIEW_FAILED,
                format!("the view could not be made: {error}"),
            )
            .with_data(json!({ "reason": "failed" }))
        })?;
        let (view, compiler) = made.map_err(|error| fault(&error))?;
        let viewport = self
            .viewports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)[attach.monitor.index()];
        view.set_viewport(viewport).map_err(|error| fault(&error))?;
        // A view made in place of one that drew an overlay draws it too.
        let marks = self.overlays.lock().unwrap_or_else(PoisonError::into_inner)
            [attach.monitor.index()]
        .as_ref()
        .map(|(_, marks)| Arc::clone(marks));
        let _ = view.set_marks(marks);
        // A view made in place of one that took the pointer takes it too.
        if let Some(cursor) = self.pointers.cursor(attach.monitor) {
            let _ = view.take_pointer(Some(cursor));
        }
        let result =
            json!({ "adapter": view.adapter(), "backend": view.backend(), "compiler": compiler });
        log(
            "view_attached",
            &json!({"monitor":attach.monitor, "adapter":view.adapter(), "backend":view.backend(), "compiler":compiler}),
        );
        // Preparation above leaves the old view, settings and owner intact on failure.
        // A candidate reporter cannot route pointer events before its instance is activated.
        let old = slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(AttachedView { instance, view });
        self.frames.activate(attach.monitor, Some(instance));
        self.pointers.activate(attach.monitor, Some(instance));
        // Gone may have arrived during construction, before activation. Recheck after the
        // switch as well so that such a callback cannot leave a live registry for a dead view.
        let gone = {
            let slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.as_ref().is_some_and(|view| view.is_gone())
        };
        if gone {
            self.frames.gone(attach.monitor, instance);
            self.pointers.gone(attach.monitor, instance);
        }
        drop_view(old).await;
        Ok(result)
    }

    /// Do something with the view that is attached; a view whose window was destroyed, with its
    /// parent, is let go.
    async fn with_view(
        &self,
        monitor: Monitor,
        work: impl FnOnce(&View) -> Result<(), ViewError>,
    ) -> Result<serde_json::Value, Fault> {
        let _busy = self.busy.lock().await;
        self.with_view_ordered(monitor, work).await
    }

    /// The caller already holds busy, including any later pointer/State publication.
    async fn with_view_ordered(
        &self,
        monitor: Monitor,
        work: impl FnOnce(&View) -> Result<(), ViewError>,
    ) -> Result<serde_json::Value, Fault> {
        let done = {
            let slot = self.slots[monitor.index()]
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let Some(view) = slot.as_ref() else {
                return Err(Fault::new(rpc::VIEW_FAILED, "no view is attached")
                    .with_data(json!({ "reason": "no_view", "monitor":monitor })));
            };
            work(view)
        };
        match done {
            Ok(()) => Ok(json!({})),
            Err(error) => {
                if error == ViewError::Gone {
                    drop_view(self.take(monitor)).await;
                }
                Err(fault(&error))
            }
        }
    }
}

fn viewport_written(monitor: Monitor, setting: Viewport) -> serde_json::Value {
    match setting {
        Viewport::Fit => json!({"monitor":monitor,"mode":"fit"}),
        Viewport::Scale { scale, center } => {
            json!({"monitor":monitor,"mode":"scale","scale":scale,"center":{"x":center[0],"y":center[1]}})
        }
    }
}

/// What a view draws over its picture for a page, as `view.overlay` says it: how many lines
/// and how many dots, none of either where it draws nothing.
fn overlay_written(monitor: Monitor, marks: Option<&Marks>) -> serde_json::Value {
    let lines = marks.map_or(0, |marks| marks.lines.len());
    let dots = marks.map_or(0, |marks| {
        marks.dots.iter().map(|dots| dots.at.len()).sum::<usize>()
    });
    json!({"monitor":monitor,"lines":lines,"dots":dots})
}

/// What a view shows, as `view.display` says it: null before its first frame.
fn display_json(display: Option<columbina_view::Display>) -> serde_json::Value {
    let Some(display) = display else {
        return serde_json::Value::Null;
    };
    let monitor = display.monitor;
    json!({
        "hdr_on": monitor.is_some_and(|monitor| monitor.hdr),
        "hdr_shown": matches!(display.output, Output::Hdr { .. }),
        "peak_nits": monitor.and_then(|monitor| monitor.peak_nits),
        "sdr_white_nits": monitor.and_then(|monitor| monitor.sdr_white_nits),
    })
}

/// Destroy a view where blocking is allowed: its threads are waited for.
async fn drop_view(view: Option<AttachedView>) {
    if let Some(view) = view {
        let _ = tokio::task::spawn_blocking(move || drop(view)).await;
    }
}

/// The error that a request is answered with when the view fails.
fn fault(error: &ViewError) -> Fault {
    let code = match error {
        ViewError::Parent(_) | ViewError::Rect(_) | ViewError::Viewport(_) => rpc::INVALID_PARAMS,
        _ => rpc::VIEW_FAILED,
    };
    Fault::new(code, error.to_string()).with_data(json!({ "reason": error.reason() }))
}

/// Give the view in the slot, if any, the card `gpu`.
fn give(slot: &std::sync::Weak<std::sync::Mutex<Option<AttachedView>>>, gpu: Arc<Gpu>) {
    if let Some(slot) = slot.upgrade() {
        if let Some(view) = slot.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            view.use_gpu(gpu);
        }
    }
}

/// What a view reports to: the log; and, once the card that it draws with is lost, the card of
/// the engine, given to it on a thread of its own: the one made in its place already, if the
/// view was given one that was lost, or else one made now (and given to every subscriber too).
fn reporter(
    graphics: &Arc<Graphics>,
    slot: &ViewSlot,
    monitor: Monitor,
    instance: u64,
    pointers: &Pointers,
    frames: &crate::presentation::Registry,
) -> columbina_view::Report {
    let graphics = Arc::downgrade(graphics);
    let slot = Arc::downgrade(slot);
    let pointers = pointers.clone();
    let frames = frames.clone();
    Arc::new(move |event: Event| {
        if let Event::Presented(shown) = event {
            frames.observe(monitor, instance, Some(shown));
            return;
        }
        if let Event::Pointer(pointer) = event {
            pointers.told(monitor, instance, &pointer);
            return;
        }
        if matches!(event, Event::Gone) {
            frames.gone(monitor, instance);
            pointers.gone(monitor, instance);
        }
        let lost = matches!(event, Event::DeviceLost { .. });
        if lost || matches!(event, Event::SurfaceLost) {
            frames.invalidate(monitor, instance);
        }
        // Device loss belongs to the shared GPU even if this reporter's window was replaced.
        // Keep Graphics recovery and its subscription path for both monitors.
        report(event, monitor);
        let (Some(graphics), true) = (graphics.upgrade(), lost) else {
            return;
        };
        let slot = slot.clone();
        let _ = std::thread::Builder::new()
            .name("columbina-view-gpu".into())
            .spawn(move || match graphics.gpu() {
                Ok(gpu) => give(&slot, gpu),
                Err(_) => graphics.remake(),
            });
    })
}

/// Log what happens to the view.
fn report(event: Event, monitor: Monitor) {
    match event {
        Event::Ready {
            adapter,
            backend,
            dpi_awareness,
        } => {
            log(
                "view_ready",
                &json!({ "monitor":monitor, "adapter": adapter, "backend": backend, "dpi_awareness": dpi_awareness }),
            );
        }
        Event::Stalled => log(
            "view_stalled",
            &json!({ "monitor":monitor, "seconds": columbina_view::STALL.as_secs() }),
        ),
        Event::Recovered { millis } => log(
            "view_recovered",
            &json!({ "monitor":monitor, "millis": millis }),
        ),
        Event::DeviceLost { message } => log(
            "view_device_lost",
            &json!({ "monitor":monitor, "message": message }),
        ),
        Event::SurfaceLost => log("view_surface_lost", &json!({ "monitor":monitor })),
        Event::Display {
            hdr_on,
            hdr_shown,
            peak_nits,
            sdr_white_nits,
        } => log(
            "view_display",
            &json!({ "monitor":monitor, "hdr_on": hdr_on, "hdr_shown": hdr_shown, "peak_nits": peak_nits, "sdr_white_nits": sdr_white_nits }),
        ),
        Event::Error { message } => log(
            "view_error",
            &json!({ "monitor":monitor, "message": message }),
        ),
        Event::Gone => log("view_gone", &json!({"monitor": monitor})),
        Event::Pointer(_) | Event::Presented(_) => {}
    }
}
