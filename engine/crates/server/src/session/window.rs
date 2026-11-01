//! Connection-owned native Window capture. All controller mutations use the ordinary State
//! mutex. Native callbacks only record evidence; the task waits with no State/view guard held.
use super::{
    json, rpc, Arc, Connection, Deserialize, Duration, Engine, Fault, Instant, Kind, Monitor,
    Mutex, Ordering, OwnedGesture, RawValue, Reply, State, Transport, Value, WorkedOutError,
};
use crate::presentation::{FrameSnapshot, Input, Pin, Registry};
use crate::window::{refused, Movement, Selection, Tool, Update};
use columbina_view::{Button, PointerKind, Presentation};
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct WindowState {
    pub owner: Option<u64>,
    sequence: u64,
    next_capture: u64,
    selection: Option<Arc<Selection>>,
    /// The successful Window bind that may release the timeline pointer, even after its
    /// logical selection retires. Ordinary timeline pointer ownership consumes this proof.
    pointer_binding: Option<(u64, u64)>,
    active: Option<Capture>,
    ended: Option<(u64, Value)>,
}

struct Capture {
    id: u64,
    owner: u64,
    binding: u64,
    pin: Arc<Pin>,
    registry: Registry,
    movement: Movement,
    gesture: Option<OwnedGesture>,
    last_event: u64,
    arming_through: u64,
}

impl State {
    fn window_check_binding(&self, owner: u64, binding: u64) -> Result<(), Fault> {
        let active = self.window.owner == Some(owner)
            && self
                .window
                .selection
                .as_ref()
                .is_some_and(|s| s.binding == binding);
        let retired = self.window.owner.is_none()
            && self.window.selection.is_none()
            && self.window.pointer_binding == Some((owner, binding));
        if active || retired {
            Ok(())
        } else {
            Err(refused("binding"))
        }
    }

    pub(super) fn window_pointer_changed(&mut self) {
        self.window.pointer_binding = None;
        self.window_end("pointer_owner");
    }

    /// Rechecked after the orders turn is acquired. Cancellation of validated external
    /// intent must make this fail before any `FreezeAt` can be enqueued.
    fn window_freeze_request(&self, id: u64) -> Result<(String, u64, i64), Fault> {
        let capture = self
            .window
            .active
            .as_ref()
            .filter(|c| c.id == id && c.pin.live.load(Ordering::Acquire))
            .ok_or_else(|| refused("capture"))?;
        let selection = self
            .window
            .selection
            .as_ref()
            .ok_or_else(|| refused("binding"))?;
        selection.validate(self.history.project(), self.history.revision())?;
        let frame = capture
            .pin
            .down
            .frame
            .as_ref()
            .ok_or_else(|| refused("frame"))?;
        Ok((
            selection.timeline.clone(),
            selection.revision,
            frame.timeline_frame,
        ))
    }
    fn window_update(&mut self) {
        self.window.sequence += 1;
        if let Some(transport) = &self.transport {
            transport.window(Update {
                sequence: self.window.sequence,
                selection: self.window.selection.clone(),
            });
        }
    }

    fn window_note(&mut self, capture: &Capture, phase: &str, reason: Option<&str>) -> Value {
        self.window.sequence += 1;
        let value = json!({"binding":capture.binding,"capture":capture.id,"sequence":self.window.sequence,
            "phase":phase,"reason":reason,"event":capture.last_event,"revision":self.history.revision(),"selected":capture.movement.selected(),
            "keys":capture.movement.keys()});
        self.window_tell(capture.owner, "window.state", &value);
        value
    }

    fn window_tell(&self, owner: u64, method: &str, value: &Value) {
        if let Some(peer) = self.peers.get(&owner) {
            if peer
                .queue
                .push(rpc::notification_line(method, value))
                .is_err()
            {
                let _ = peer.stop.send(true);
            }
        }
    }

    pub(super) fn window_keeps_gesture(&self) -> bool {
        self.window.active.as_ref().is_some_and(|c| {
            c.pin.live.load(Ordering::Acquire)
                && c.gesture.as_ref().is_some_and(|g| {
                    self.history.gesture() == Some((g.key.as_str(), g.base))
                        && self.history.revision() == g.last_own.unwrap_or(g.base)
                })
        })
    }

    /// External edits are already committed when this runs. Only this capture's own old
    /// history key may be flushed after an unrecorded revision; strict owned finish is intact.
    fn finish_capture(&mut self, capture: Capture, reason: &str) -> Value {
        self.close_capture(capture, "ended", reason)
    }

    fn close_capture(&mut self, capture: Capture, phase: &str, reason: &str) -> Value {
        let result = self.window_note(&capture, phase, Some(reason));
        self.window.ended = Some((capture.owner, result.clone()));
        capture.registry.release(&capture.pin);
        if let Some(gesture) = capture.gesture {
            if self.check_owned_gesture(capture.owner, &gesture).is_ok() {
                let _ = self.finish_owned_gesture(capture.owner, gesture);
            } else if self.history.gesture() == Some((gesture.key.as_str(), gesture.base)) {
                self.finish_gesture();
            }
        }
        result
    }

    pub(super) fn window_end(&mut self, reason: &str) {
        if let Some(capture) = self.window.active.take() {
            self.finish_capture(capture, reason);
        }
    }

    pub(super) fn window_unbind(&mut self, reason: &str) {
        self.window_end(reason);
        self.window.owner = None;
        self.window.selection = None;
        self.window_update();
    }

    pub(super) fn window_project_changed(&mut self) {
        self.window_end("project_changed");
        let mut retired = None;
        if let Some(selection) = &mut self.window.selection {
            let selection = Arc::make_mut(selection);
            selection.revision = self.history.revision();
            if selection
                .validate(self.history.project(), selection.revision)
                .is_err()
            {
                retired = self.window.owner.map(|owner| (owner, selection.binding));
                self.window.selection = None;
                self.window.owner = None;
            }
            self.window_update();
        }
        if let Some((owner, binding)) = retired {
            self.window.sequence += 1;
            self.window_tell(
                owner,
                "window.unbound",
                &json!({
                    "binding": binding, "sequence": self.window.sequence,
                    "revision": self.history.revision(), "reason": "project_changed",
                }),
            );
        }
    }

    fn window_begin(
        &mut self,
        owner: u64,
        binding: u64,
        pin: Arc<Pin>,
        registry: Registry,
    ) -> Result<Value, Fault> {
        let selection = self
            .window
            .selection
            .as_ref()
            .filter(|s| s.binding == binding && self.window.owner == Some(owner))
            .ok_or_else(|| refused("binding"))?;
        selection.validate(self.history.project(), self.history.revision())?;
        let down = &pin.down;
        let shown = down
            .pointer
            .presentation
            .ok_or_else(|| refused("presentation"))?;
        let frame = down.frame.as_ref().ok_or_else(|| refused("presentation"))?;
        if !down.pointer.inside
            || down.pointer.button != Some(Button::Left)
            || !pin.live.load(Ordering::Acquire)
        {
            return Err(refused("down"));
        }
        if frame
            .selection
            .as_ref()
            .is_none_or(|s| s.binding != binding || s.revision != selection.revision)
        {
            return Err(refused("selection"));
        }
        let map = shown.map.ok_or_else(|| refused("map"))?;
        #[allow(
            clippy::cast_precision_loss,
            reason = "native pixel coordinates are converted to the overlay's f32 device space"
        )]
        let point = [
            down.pointer.window.0 as f32 + 0.5,
            down.pointer.window.1 as f32 + 0.5,
        ];
        let hit = frame
            .overlay
            .as_ref()
            .and_then(|o| o.hit_control(map, point))
            .ok_or_else(|| refused("hit"))?;
        let selected = hit.id.to_owned();
        let canvas = map.on_picture(down.pointer.window).0;
        let movement = if hit.control == columbina_view::window_overlay::Control::Move {
            Movement::new(selection, frame, &selected, canvas)
        } else {
            Movement::capture(selection, frame, &selected, hit.control, canvas)
        }
        .ok_or_else(|| refused("target"))?;
        self.window_end("new_down");
        Arc::make_mut(self.window.selection.as_mut().expect("validated selection")).selected =
            Some(selected);
        self.window_update();
        self.window.next_capture += 1;
        let capture = Capture {
            id: self.window.next_capture,
            owner,
            binding,
            last_event: down.sequence,
            arming_through: down.sequence,
            pin,
            registry,
            movement,
            gesture: None,
        };
        let result = self.window_note(&capture, "arming", None);
        self.window.active = Some(capture);
        Ok(result)
    }

    #[allow(
        clippy::float_cmp,
        reason = "capture requires exactly the same native placement, not a tolerance"
    )]
    fn window_current(
        &self,
        capture: &Capture,
        current: &(Presentation, Arc<FrameSnapshot>),
    ) -> bool {
        let Some(original) = &capture.pin.down.frame else {
            return false;
        };
        let (shown, frame) = current;
        let Some(selection) = &self.window.selection else {
            return false;
        };
        let latest = capture
            .gesture
            .as_ref()
            .map_or(selection.revision, |g| g.last_own.unwrap_or(g.base));
        let mut displayed = selection.as_ref().clone();
        displayed.revision = frame.revision;
        shown.map == capture.pin.down.pointer.presentation.and_then(|p| p.map)
            && frame.revision >= original.revision
            && frame.revision <= latest
            && frame.timeline_frame == original.timeline_frame
            && frame
                .selection
                .as_ref()
                .is_some_and(|s| s.binding == capture.binding)
            && displayed.placement(frame).is_some()
            && frame.layers.len() == original.layers.len()
            && frame.layers.iter().zip(&original.layers).all(|(a, b)| {
                a.clip == b.clip
                    && a.origin.clip == b.origin.clip
                    && a.origin.media == b.origin.media
                    && a.origin.key_frame == b.origin.key_frame
                    && a.picture_to_canvas == b.picture_to_canvas
                    && a.shown_size == b.shown_size
            })
    }

    fn window_move(&mut self, capture: &mut Capture, input: &Input) -> Result<(), Fault> {
        let original = capture
            .pin
            .down
            .pointer
            .presentation
            .and_then(|p| p.map)
            .ok_or_else(|| refused("map"))?;
        if input.instance != capture.pin.down.instance
            || input.owner != capture.owner
            || input.pointer.presentation.and_then(|p| p.map) != Some(original)
        {
            return Err(refused("map"));
        }
        // Playback may advance between native down and FreezeAt's successful present.
        // Only positions recorded before arming may carry those intervening frames;
        // window_events has separately validated the actual current frozen presentation.
        if input.sequence > capture.arming_through
            && input
                .pointer
                .presentation
                .zip(input.frame.clone())
                .as_ref()
                .is_none_or(|shown| !self.window_current(capture, shown))
        {
            return Err(refused("presentation"));
        }
        let selection = self
            .window
            .selection
            .clone()
            .ok_or_else(|| refused("binding"))?;
        let rotation =
            if capture.movement.control == columbina_view::window_overlay::Control::Rotate {
                let evidence = input
                    .rotation
                    .as_ref()
                    .filter(|e| e.basis == capture.movement.rotation)
                    .ok_or_else(|| refused("rotation"))?;
                for sample in &evidence.samples {
                    if sample.owner != capture.owner
                        || sample.instance != capture.pin.down.instance
                        || sample.shown.and_then(|p| p.map) != Some(original)
                        || (sample.sequence > capture.arming_through
                            && sample
                                .shown
                                .zip(sample.frame.clone())
                                .as_ref()
                                .is_none_or(|s| !self.window_current(capture, s)))
                    {
                        return Err(refused("presentation"));
                    }
                }
                Some(evidence.degrees)
            } else {
                None
            };
        let canvas = original.on_picture(input.pointer.window).0;
        let windows = if rotation.is_some() {
            capture.movement.at_rotation(canvas, rotation)
        } else {
            capture.movement.at(canvas)
        }
        .ok_or_else(|| refused("point"))?;
        if selection
            .corrector(self.history.project())
            .is_some_and(|c| c.windows == windows)
        {
            return Ok(());
        }
        let gesture = capture.gesture.as_mut().ok_or_else(|| refused("arming"))?;
        // Native input already established a live capture at this event, including a held
        // move immediately preceding up. This is not the wire gesture's idle exception.
        self.gesture_last = Some(Instant::now());
        self.apply_worked_out_locked(
            Kind::Desktop,
            None,
            Some((capture.owner, gesture)),
            Instant::now(),
            |_| Ok(vec![Movement::op(&selection, &windows)]),
        )
        .map_err(|error| match error {
            WorkedOutError::Input(f) | WorkedOutError::Apply(f) => f,
        })?;
        Ok(())
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the independent native evidence snapshot is consumed after this locked operation"
    )]
    fn window_events(
        &mut self,
        id: u64,
        mut events: Vec<Input>,
        current: Option<(Presentation, Arc<FrameSnapshot>)>,
    ) {
        let Some(mut capture) = self.window.active.take() else {
            return;
        };
        if capture.id != id {
            self.window.active = Some(capture);
            return;
        }
        if !capture.pin.live.load(Ordering::Acquire) {
            // A native up retains its preceding move. External retirement removes the
            // queue entirely and must prevent an already-cloned move from writing later.
            events = capture.registry.events(&capture.pin);
            if !events
                .iter()
                .any(|event| matches!(event.pointer.kind, PointerKind::Up | PointerKind::Cancel))
            {
                self.finish_capture(capture, "native_end");
                return;
            }
        }
        if current
            .as_ref()
            .is_none_or(|c| !self.window_current(&capture, c))
        {
            self.finish_capture(capture, "presentation_changed");
            return;
        }
        for input in events {
            if input.sequence <= capture.last_event {
                continue;
            }
            capture.last_event = input.sequence;
            match input.pointer.kind {
                PointerKind::Move if capture.gesture.is_some() => {
                    if self.window_move(&mut capture, &input).is_err() {
                        self.finish_capture(capture, "stale");
                        return;
                    }
                }
                PointerKind::Up | PointerKind::Cancel => {
                    self.finish_capture(capture, "up");
                    return;
                }
                _ => {}
            }
        }
        capture
            .registry
            .acknowledged(&capture.pin, capture.last_event);
        self.window.active = Some(capture);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRequest {
    binding: u64,
    kind: InputKind,
    event: Option<u64>,
    capture: Option<u64>,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum InputKind {
    Down,
    Move,
    Up,
    Cancel,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unbind {
    binding: u64,
}

async fn bind(
    state: &Mutex<State>,
    viewer: &crate::view::Viewer,
    connection: &Connection,
    kind: Kind,
    selection: Selection,
) -> Result<Value, Fault> {
    {
        let state = state.lock().await;
        selection.validate(state.history.project(), state.history.revision())?;
    }
    let on = selection.visible && selection.outline && selection.tool == Tool::Window;
    let pointer =
        serde_json::value::to_raw_value(&json!({"monitor":"timeline","on":on,"cursor":"move"}))
            .expect("parameters");
    let change = viewer
        .prepare_pointer(kind, connection.id, connection.sender(), Some(&pointer))
        .await?;
    {
        let state = state.lock().await;
        selection.validate(state.history.project(), state.history.revision())?;
    }
    change.apply().await?;
    finish_bind(state, &change, connection.id, selection).await
}

async fn finish_bind(
    state: &Mutex<State>,
    change: &crate::view::PointerChange<'_>,
    owner: u64,
    mut selection: Selection,
) -> Result<Value, Fault> {
    let mut state = state.lock().await;
    if let Err(error) = selection.validate(state.history.project(), state.history.revision()) {
        drop(state);
        change.rollback().await;
        return Err(error);
    }
    state.window_unbind("binding");
    selection.binding = state.window.sequence + 1;
    state.window.owner = Some(owner);
    state.window.pointer_binding = Some((owner, selection.binding));
    state.window.selection = Some(Arc::new(selection));
    change.commit();
    state.window_update();
    Ok(
        json!({"binding":state.window.selection.as_ref().expect("just bound").binding,"sequence":state.window.sequence}),
    )
}

async fn unbind(
    state: &Mutex<State>,
    viewer: &crate::view::Viewer,
    connection: &Connection,
    kind: Kind,
    binding: u64,
) -> Result<Value, Fault> {
    let pointer = serde_json::value::to_raw_value(&json!({"monitor":"timeline","on":false}))
        .expect("parameters");
    let change = viewer
        .prepare_pointer(kind, connection.id, connection.sender(), Some(&pointer))
        .await?;
    let release_pointer = {
        let state = state.lock().await;
        state.window_check_binding(connection.id, binding)?;
        state.window.pointer_binding == Some((connection.id, binding))
            && change.owned_by(connection.id)
    };
    if release_pointer {
        change.apply().await?;
    }
    finish_unbind(state, &change, connection.id, binding, release_pointer).await
}

async fn finish_unbind(
    state: &Mutex<State>,
    change: &crate::view::PointerChange<'_>,
    owner: u64,
    binding: u64,
    release_pointer: bool,
) -> Result<Value, Fault> {
    let mut state = state.lock().await;
    if let Err(error) = state.window_check_binding(owner, binding) {
        drop(state);
        if release_pointer {
            change.rollback().await;
        }
        return Err(error);
    }
    state.window_unbind("unbind");
    if state.window.pointer_binding == Some((owner, binding)) {
        state.window.pointer_binding = None;
    }
    if release_pointer {
        change.commit();
    }
    Ok(json!({"phase":"ended","binding":binding}))
}

#[allow(
    clippy::too_many_lines,
    reason = "three narrow RPC routes with explicit State lock boundaries"
)]
pub(super) async fn call(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    if engine.headless || kind != Kind::Desktop {
        return Err(refused("desktop").into());
    }
    let result = match method {
        "window.bind" => {
            let selection: Selection = rpc::params(method, "Window selection", params)?;
            bind(&engine.state, &engine.viewer, connection, kind, selection).await?
        }
        "window.unbind" => {
            let asked: Unbind = rpc::params(method, "{binding}", params)?;
            unbind(
                &engine.state,
                &engine.viewer,
                connection,
                kind,
                asked.binding,
            )
            .await?
        }
        "window.input" => {
            let asked: InputRequest =
                rpc::params(method, "{binding,kind,event?,capture?}", params)?;
            if asked.kind == InputKind::Down {
                let pin = engine
                    .viewer
                    .frames
                    .claim(connection.id, asked.event.ok_or_else(|| refused("event"))?)
                    .ok_or_else(|| refused("event"))?;
                let mut state = engine.state.lock().await;
                let native = engine.viewer.presentation_snapshot(Monitor::Timeline);
                if native.is_none_or(|(instance, shown)| {
                    instance != pin.down.instance
                        || shown.and_then(|p| p.map)
                            != pin.down.pointer.presentation.and_then(|p| p.map)
                }) {
                    engine.viewer.frames.release(&pin);
                    return Err(refused("map").into());
                }
                let result = state.window_begin(
                    connection.id,
                    asked.binding,
                    Arc::clone(&pin),
                    engine.viewer.frames.clone(),
                );
                drop(state);
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        engine.viewer.frames.release(&pin);
                        return Err(error.into());
                    }
                };
                let task = Task {
                    state: Arc::clone(&engine.state),
                    transport: engine.transport.clone(),
                    registry: engine.viewer.frames.clone(),
                    pin,
                    view: engine.viewer.slot(Monitor::Timeline),
                    capture: result["capture"].as_u64().expect("capture id"),
                };
                tokio::spawn(task.run());
                result
            } else {
                // Drain actual native moves before an echoed up/cancel. Both queue copies
                // and the current presentation are acquired before awaiting State.
                let pending = {
                    let state = engine.state.lock().await;
                    state
                        .window
                        .active
                        .as_ref()
                        .filter(|c| {
                            c.owner == connection.id
                                && c.binding == asked.binding
                                && Some(c.id) == asked.capture
                        })
                        .map(|c| (c.id, c.registry.clone(), c.pin.clone(), c.gesture.is_some()))
                };
                let pending = pending.map(|(id, registry, pin, armed)| {
                    let events = if armed {
                        registry.events(&pin)
                    } else {
                        Vec::new()
                    };
                    (id, events, registry, pin, armed)
                });
                let mut state = engine.state.lock().await;
                if let Some((id, events, registry, pin, true)) = pending {
                    let native = engine.viewer.presentation_snapshot(Monitor::Timeline);
                    let current = native
                        .filter(|(instance, _)| *instance == pin.down.instance)
                        .and_then(|(instance, shown)| {
                            registry.observe(Monitor::Timeline, instance, shown);
                            registry.current(Monitor::Timeline, instance)
                        });
                    state.window_events(id, events, current);
                }
                if asked.kind != InputKind::Move {
                    if let Some((owner, result)) = &state.window.ended {
                        if *owner == connection.id
                            && result["binding"] == asked.binding
                            && result["capture"].as_u64() == asked.capture
                        {
                            return Ok(match connection.queue(rpc::result_line(id, result)) {
                                Ok(()) => Reply::Queued,
                                Err(refused) => Reply::Refused(refused),
                            });
                        }
                    }
                }
                let Some(capture) = state.window.active.as_ref().filter(|c| {
                    c.owner == connection.id
                        && c.binding == asked.binding
                        && Some(c.id) == asked.capture
                }) else {
                    return Err(refused("capture").into());
                };
                let phase = if capture.gesture.is_some() {
                    "armed"
                } else {
                    "arming"
                };
                let result = json!({"binding":capture.binding,"capture":capture.id,"phase":phase});
                if asked.kind == InputKind::Move {
                    result
                } else {
                    let mut capture = state.window.active.take().expect("checked capture");
                    if asked.kind == InputKind::Cancel {
                        if let (Some(gesture), Some(selection)) =
                            (&mut capture.gesture, state.window.selection.clone())
                        {
                            if state.check_owned_gesture(connection.id, gesture).is_ok()
                                && gesture.last_own.is_some()
                            {
                                state.gesture_last = Some(Instant::now());
                                let op = Movement::op(&selection, &capture.movement.original);
                                let _ = state.apply_worked_out_locked(
                                    Kind::Desktop,
                                    None,
                                    Some((connection.id, gesture)),
                                    Instant::now(),
                                    |_| Ok(vec![op]),
                                );
                            }
                        }
                    }
                    state.finish_capture(
                        capture,
                        if asked.kind == InputKind::Cancel {
                            "cancel"
                        } else {
                            "up"
                        },
                    )
                }
            }
        }
        _ => unreachable!("window routes"),
    };
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

struct Task {
    state: Arc<Mutex<State>>,
    transport: Transport,
    registry: Registry,
    pin: Arc<Pin>,
    view: crate::view::ViewSlot,
    capture: u64,
}
impl Task {
    fn current(&self) -> Option<(Presentation, Arc<FrameSnapshot>)> {
        let native = {
            let slot = self
                .view
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.as_ref().map(|v| (v.instance, v.presentation()))
        };
        let (instance, shown) = native?;
        if instance != self.pin.down.instance {
            return None;
        }
        self.registry.observe(Monitor::Timeline, instance, shown);
        self.registry.current(Monitor::Timeline, instance)
    }
    async fn end(&self, reason: &str) {
        let mut state = self.state.lock().await;
        if state
            .window
            .active
            .as_ref()
            .is_some_and(|c| c.id == self.capture)
        {
            state.window_end(reason);
        }
    }
    async fn run(self) {
        if let Err(fault) = self.arm().await {
            let mut state = self.state.lock().await;
            if state
                .window
                .active
                .as_ref()
                .is_some_and(|c| c.id == self.capture)
            {
                let capture = state.window.active.take().expect("checked capture");
                let reason = fault
                    .data
                    .as_ref()
                    .and_then(|d| d["reason"].as_str())
                    .unwrap_or("freeze");
                state.close_capture(capture, "refused", reason);
            }
            return;
        }
        loop {
            let notified = self.pin.wake.notified();
            let events = self.registry.events(&self.pin);
            {
                let mut state = self.state.lock().await;
                if state
                    .window
                    .active
                    .as_ref()
                    .is_none_or(|c| c.id != self.capture)
                {
                    return;
                }
                let current = self.current();
                state.window_events(self.capture, events, current);
                if state.tell_at.is_some_and(|due| due <= Instant::now()) {
                    state.flush();
                }
                if state
                    .window
                    .active
                    .as_ref()
                    .is_none_or(|c| c.id != self.capture)
                {
                    return;
                }
            }
            if !self.pin.live.load(Ordering::Acquire) {
                self.end("native_end").await;
                return;
            }
            tokio::select! {()=notified=>{},()=tokio::time::sleep(Duration::from_millis(25))=>{}}
        }
    }
    async fn arm(&self) -> Result<(), Fault> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let turn = tokio::time::timeout_at(deadline, self.transport.window_turn())
            .await
            .map_err(|_| refused("arming_timeout"))?;
        let ack = {
            let state = self.state.lock().await;
            let (timeline, revision, frame) = state.window_freeze_request(self.capture)?;
            let current = self.current().ok_or_else(|| refused("presentation"))?;
            if current.0.map != self.pin.down.pointer.presentation.and_then(|p| p.map) {
                return Err(refused("map"));
            }
            self.transport
                .freeze_at(timeline, revision, frame, self.capture)?
        };
        drop(turn);
        tokio::pin!(ack);
        loop {
            tokio::select! {
                result=&mut ack=>{result.map_err(|_|refused("worker"))??;break;},
                ()=self.pin.wake.notified()=>if !self.pin.live.load(Ordering::Acquire){return Err(refused("native_end"));},
                ()=tokio::time::sleep_until(deadline)=>return Err(refused("arming_timeout")),
            }
        }
        loop {
            let notified = self.pin.wake.notified();
            if !self.pin.live.load(Ordering::Acquire) {
                return Err(refused("native_end"));
            }
            {
                let mut state = self.state.lock().await;
                if state
                    .window
                    .active
                    .as_ref()
                    .is_none_or(|c| c.id != self.capture)
                    || !self.pin.live.load(Ordering::Acquire)
                {
                    return Err(refused("capture"));
                }
                if let Some(current) = self
                    .current()
                    .filter(|(_, frame)| frame.frozen == self.capture && frame.ready)
                {
                    let mut capture = state.window.active.take().expect("checked capture");
                    if state.window_current(&capture, &current) {
                        let revision = state.history.revision();
                        capture.gesture = Some(state.begin_owned_gesture(capture.owner, revision)?);
                        // FreezeAt has actually presented. Take this event boundary once;
                        // all later native moves retain their per-event frame/source checks.
                        capture.arming_through = self
                            .registry
                            .events(&self.pin)
                            .last()
                            .map_or(capture.last_event, |event| event.sequence);
                        state.window_note(&capture, "armed", None);
                        state.window.active = Some(capture);
                        return Ok(());
                    }
                    state.window.active = Some(capture);
                    return Err(refused("presentation_changed"));
                }
            }
            tokio::select! {()=notified=>{},()=tokio::time::sleep_until(deadline)=>return Err(refused("arming_timeout"))}
        }
    }
}
