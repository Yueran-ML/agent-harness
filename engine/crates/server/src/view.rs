//! The methods `view.*`: the picture drawn by the engine in a window of its own inside the
//! desktop's window (see `columbina-view`, and `PROTOCOL.md`).
//!
//! There is one view at a time, which belongs to the engine, not to the connection that made
//! it: it stays when that connection closes, and goes when it is detached, when another is
//! attached, or when the engine ends. A view is made and destroyed on threads where blocking is
//! allowed: its own threads make and serve the window, and the card is made ready on one of them.

use std::sync::Arc;

use columbina_view::{Event, RawParent, Rect, View, ViewError};
use serde::Deserialize;
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::Mutex;

use crate::log::log;
use crate::rpc::{self, Fault};
use crate::session::Kind;

/// The view of the engine, if one is attached.
pub struct Viewer {
    /// Whether the engine is headless, and so draws in no window.
    headless: bool,
    view: Mutex<Option<View>>,
}

/// The params of `view.attach`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attach {
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
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// The params of `view.show`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Show {
    shown: bool,
}

const ATTACH: &str =
    r#"{"parent": "<handle in decimal>", "x": 0, "y": 0, "width": 640, "height": 360}"#;
const PLACE: &str = r#"{"x": 0, "y": 0, "width": 640, "height": 360}"#;
const SHOW: &str = r#"{"shown": true | false}"#;

impl Viewer {
    pub fn new(headless: bool) -> Self {
        Self {
            headless,
            view: Mutex::new(None),
        }
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
                self.with_view(|view| view.place(rect)).await
            }
            "view.show" => {
                let show: Show = rpc::params(method, SHOW, params)?;
                self.with_view(|view| view.show(show.shown)).await
            }
            "view.detach" => {
                rpc::no_params(method, params)?;
                let old = self.view.lock().await.take();
                let detached = old.is_some();
                drop_view(old).await;
                if detached {
                    log("view_detached", &json!({}));
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
        let mut slot = self.view.lock().await;
        let old = slot.take();
        let made = tokio::task::spawn_blocking(move || {
            drop(old);
            View::attach_reporting(parent, rect, Arc::new(report))
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
        log("view_attached", &result);
        *slot = Some(view);
        Ok(result)
    }

    /// Do something with the view that is attached; a view whose window was destroyed, with its
    /// parent, is let go.
    async fn with_view(
        &self,
        work: impl FnOnce(&View) -> Result<(), ViewError>,
    ) -> Result<serde_json::Value, Fault> {
        let mut slot = self.view.lock().await;
        let Some(view) = slot.as_ref() else {
            return Err(Fault::new(rpc::VIEW_FAILED, "no view is attached")
                .with_data(json!({ "reason": "no_view" })));
        };
        match work(view) {
            Ok(()) => Ok(json!({})),
            Err(error) => {
                if error == ViewError::Gone {
                    drop_view(slot.take()).await;
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

/// Log what happens to the view.
fn report(event: Event) {
    match event {
        Event::Ready {
            adapter,
            backend,
            dpi_awareness,
        } => {
            log(
                "view_ready",
                &json!({ "adapter": adapter, "backend": backend, "dpi_awareness": dpi_awareness }),
            );
        }
        Event::Stalled => log(
            "view_stalled",
            &json!({ "seconds": columbina_view::STALL.as_secs() }),
        ),
        Event::Recovered { millis } => log("view_recovered", &json!({ "millis": millis })),
        Event::DeviceLost { message } => log("view_device_lost", &json!({ "message": message })),
        Event::SurfaceLost => log("view_surface_lost", &json!({})),
        Event::Error { message } => log("view_error", &json!({ "message": message })),
    }
}
