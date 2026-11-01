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
//! connection alone, as `view.pointer` notifications ([`pointer`]).

mod pointer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};

use columbina_compose::ShaderCompiler;
use columbina_view::{Cursor, Event, Gpu, Output, RawParent, Rect, View, ViewError, Viewport};
use serde::{Deserialize, Serialize};
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
pub type ViewSlot = Arc<std::sync::Mutex<Option<View>>>;

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
    /// Viewer-only geometry, retained when a native window is replaced or detached.
    viewports: std::sync::Mutex<[Viewport; 2]>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Center {
    x: f64,
    y: f64,
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
}

const ATTACH: &str = r#"{"monitor":"source"|"timeline", "parent": "<handle in decimal>", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const PLACE: &str =
    r#"{"monitor":"source"|"timeline", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const SHOW: &str = r#"{"monitor":"source"|"timeline", "shown": true | false}"#;
const DISPLAY: &str = r#"{"hdr"?: true | false}"#;
const VIEWPORT: &str = r#"{"monitor":"source"|"timeline", "mode"?:"fit"|"scale", "scale":positive number, "center":{"x":number,"y":number}}; scale and center are required only with mode scale"#;
const POINTER: &str = r#"{"monitor":"source"|"timeline", "on": true | false, "cursor"?: "arrow" | "crosshair" | "move" | "resize_ew" | "resize_ns"}"#;

impl Viewer {
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
        Self {
            headless,
            slots,
            graphics,
            _subscription: subscription,
            busy: Mutex::new(()),
            hdr: AtomicBool::new(true),
            pointers: Pointers::new(),
            viewports: std::sync::Mutex::new([Viewport::Fit; 2]),
        }
    }

    /// `view.pointer`, of the connection `connection` of the desktop: have the view of a monitor
    /// take the pointer, and tell what it does there to this connection alone (in place of one
    /// that asked before), or let it go.
    pub async fn pointer(
        &self,
        kind: Kind,
        connection: u64,
        sender: &queue::Sender,
        params: Option<&RawValue>,
    ) -> Result<serde_json::Value, Fault> {
        let method = "view.pointer";
        self.check(kind, method)?;
        let asked: TakePointer = rpc::params(method, POINTER, params)?;
        let cursor = asked.on.then_some(Cursor::from(asked.cursor));
        let taken = self
            .with_view(asked.monitor, |view| view.take_pointer(cursor))
            .await;
        if asked.on {
            // Only a view that is there takes it.
            taken?;
            self.pointers
                .tell(asked.monitor, connection, sender.clone(), cursor);
        } else {
            // Let go, whether or not a view is still there.
            self.pointers.stop(asked.monitor);
        }
        Ok(json!({
            "monitor": asked.monitor,
            "on": asked.on,
            "cursor": asked.on.then_some(asked.cursor),
        }))
    }

    /// Forget a connection that has ended: the views that took the pointer for it let it go.
    pub fn forget(&self, connection: u64) {
        for monitor in self.pointers.forget(connection) {
            if let Some(view) = self.slots[monitor.index()]
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            {
                let _ = view.take_pointer(None);
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

    /// Take the view out of its slot.
    fn take(&self, monitor: Monitor) -> Option<View> {
        self.slots[monitor.index()]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
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
        Ok(match settings[monitor.index()] {
            Viewport::Fit => json!({"monitor":monitor,"mode":"fit"}),
            Viewport::Scale { scale, center } => {
                json!({"monitor":monitor,"mode":"scale","scale":scale,"center":{"x":center[0],"y":center[1]}})
            }
        })
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
        let old = self.take(attach.monitor);
        let graphics = Arc::clone(&self.graphics);
        let slot = self.slot(attach.monitor);
        let reporter = reporter(&graphics, &slot, attach.monitor, &self.pointers);
        let hdr = self.hdr.load(Ordering::SeqCst);
        let made = tokio::task::spawn_blocking(move || {
            drop(old);
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
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(view);
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
async fn drop_view(view: Option<View>) {
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
fn give(slot: &std::sync::Weak<std::sync::Mutex<Option<View>>>, gpu: Arc<Gpu>) {
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
    pointers: &Pointers,
) -> columbina_view::Report {
    let graphics = Arc::downgrade(graphics);
    let slot = Arc::downgrade(slot);
    let pointers = pointers.clone();
    Arc::new(move |event: Event| {
        if let Event::Pointer(pointer) = event {
            pointers.told(monitor, &pointer);
            return;
        }
        let lost = matches!(event, Event::DeviceLost { .. });
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
        Event::Pointer(_) => {}
    }
}
