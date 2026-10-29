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

use std::sync::{Arc, PoisonError};

use columbina_view::{Event, Gpu, RawParent, Rect, View, ViewError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::Mutex;

use crate::gpu::{Graphics, Subscription};
use crate::log::log;
use crate::rpc::{self, Fault};
use crate::session::Kind;

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
    fn index(self) -> usize {
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

const ATTACH: &str = r#"{"monitor":"source"|"timeline", "parent": "<handle in decimal>", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const PLACE: &str =
    r#"{"monitor":"source"|"timeline", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const SHOW: &str = r#"{"monitor":"source"|"timeline", "shown": true | false}"#;

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
        }
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
            _ => Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            )),
        }
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
        let reporter = reporter(&graphics, &slot, attach.monitor);
        let made = tokio::task::spawn_blocking(move || {
            drop(old);
            let gpu = graphics
                .gpu()
                .map_err(|error| ViewError::Gpu(error.to_string()))?;
            View::attach_reporting(parent, rect, gpu, reporter)
        })
        .await
        .map_err(|error| {
            Fault::new(
                rpc::VIEW_FAILED,
                format!("the view could not be made: {error}"),
            )
            .with_data(json!({ "reason": "failed" }))
        })?;
        let view = made.map_err(|error| fault(&error))?;
        let result = json!({ "adapter": view.adapter(), "backend": view.backend() });
        log(
            "view_attached",
            &json!({"monitor":attach.monitor, "adapter":view.adapter(), "backend":view.backend()}),
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

/// Destroy a view where blocking is allowed: its threads are waited for.
async fn drop_view(view: Option<View>) {
    if let Some(view) = view {
        let _ = tokio::task::spawn_blocking(move || drop(view)).await;
    }
}

/// The error that a request is answered with when the view fails.
fn fault(error: &ViewError) -> Fault {
    let code = match error {
        ViewError::Parent(_) | ViewError::Rect(_) => rpc::INVALID_PARAMS,
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
fn reporter(graphics: &Arc<Graphics>, slot: &ViewSlot, monitor: Monitor) -> columbina_view::Report {
    let graphics = Arc::downgrade(graphics);
    let slot = Arc::downgrade(slot);
    Arc::new(move |event: Event| {
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
        Event::Error { message } => log(
            "view_error",
            &json!({ "monitor":monitor, "message": message }),
        ),
    }
}
