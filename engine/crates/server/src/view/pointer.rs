//! What the pointer does on a view that takes it, told to the connection that asked
//! (`view.pointer`), as `view.pointer` notifications.
//!
//! Presses and releases are each told. Moves are told as fast as the client reads them: one is
//! queued once the one before it was written to the connection, and those that come meanwhile
//! only take the place of the one held, which is told before the next press or release, or a
//! moment later by a thread of its own (`columbina-pointer`), so that the last place the pointer
//! stopped at is always told. A client that reads slowly is told fewer moves, and is never closed
//! for them.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use columbina_view::{Cursor, Pointer, PointerKind};
use serde_json::json;

use super::Monitor;
use crate::queue;
use crate::rpc;

/// How often a move that is held is looked at, to be told once the one before was written.
const POLL: Duration = Duration::from_millis(4);

/// Who is told what the pointer does on each view.
#[derive(Clone)]
pub struct Pointers(Arc<Inner>);

struct Inner {
    sinks: Mutex<Sinks>,
    frames: crate::presentation::Registry,
    /// Wakes the thread that tells the moves held.
    wake: Condvar,
}

#[derive(Default)]
struct Sinks {
    instances: [Option<u64>; 2],
    sinks: [Option<Sink>; 2],
}

/// The connection told of the pointer on a view, and the cursor shown there.
struct Sink {
    connection: u64,
    sender: queue::Sender,
    cursor: Cursor,
    /// The number of the last move queued: once the writer has written as many, it is written.
    queued: Option<u64>,
    /// A move not told yet.
    held: Option<rpc::Line>,
    /// Whether the thread that tells the moves held was started.
    started: bool,
}

impl Pointers {
    pub fn new(frames: crate::presentation::Registry) -> Pointers {
        Pointers(Arc::new(Inner {
            sinks: Mutex::default(),
            frames,
            wake: Condvar::new(),
        }))
    }

    fn sinks(&self) -> MutexGuard<'_, Sinks> {
        self.0.sinks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace the view but keep its owner/cursor. Already queued RPC lines cannot be
    /// recalled; the old instance remains on those lines for consumers to check.
    pub fn activate(&self, monitor: Monitor, instance: Option<u64>) {
        let mut sinks = self.sinks();
        sinks.instances[monitor.index()] = instance;
        if let Some(sink) = sinks.sinks[monitor.index()].as_mut() {
            sink.held = None;
            sink.queued = None;
        }
    }

    pub fn gone(&self, monitor: Monitor, instance: u64) {
        let mut sinks = self.sinks();
        if sinks.instances[monitor.index()] == Some(instance) {
            sinks.instances[monitor.index()] = None;
            if let Some(sink) = sinks.sinks[monitor.index()].as_mut() {
                sink.held = None;
                sink.queued = None;
            }
        }
    }

    /// Tell `connection` what the pointer does on the view of `monitor` from now on.
    pub fn tell(
        &self,
        monitor: Monitor,
        connection: u64,
        sender: queue::Sender,
        cursor: Option<Cursor>,
    ) {
        let mut sinks = self.sinks();
        let started = sinks.sinks.iter().flatten().any(|sink| sink.started);
        sinks.sinks[monitor.index()] = Some(Sink {
            connection,
            sender,
            cursor: cursor.unwrap_or_default(),
            queued: None,
            held: None,
            started,
        });
    }

    /// Tell no one what the pointer does on the view of `monitor`.
    pub fn stop(&self, monitor: Monitor) {
        self.sinks().sinks[monitor.index()] = None;
    }

    /// The cursor of the view of `monitor`, where it takes the pointer.
    pub fn cursor(&self, monitor: Monitor) -> Option<Cursor> {
        self.sinks().sinks[monitor.index()]
            .as_ref()
            .map(|sink| sink.cursor)
    }

    /// Tell no more to `connection`, which ended: the monitors whose views took the pointer for
    /// it.
    pub fn forget(&self, connection: u64) -> Vec<Monitor> {
        let mut sinks = self.sinks();
        let mut forgotten = Vec::new();
        for monitor in [Monitor::Source, Monitor::Timeline] {
            let slot = &mut sinks.sinks[monitor.index()];
            if slot
                .as_ref()
                .is_some_and(|sink| sink.connection == connection)
            {
                *slot = None;
                forgotten.push(monitor);
            }
        }
        forgotten
    }

    /// What the pointer did on the view of `monitor`: told to whoever asked, if anyone did. It is
    /// called on the view's thread of its window, and never waits.
    pub fn told(&self, monitor: Monitor, instance: u64, pointer: &Pointer) {
        let snapshot = self
            .0
            .frames
            .observe(monitor, instance, pointer.presentation);
        let line = rpc::notification_line(
            "view.pointer",
            &written(monitor, instance, pointer, snapshot.as_deref()),
        );
        let mut sinks = self.sinks();
        if sinks.instances[monitor.index()] != Some(instance) {
            return;
        }
        let Some(sink) = sinks.sinks[monitor.index()].as_mut() else {
            return;
        };
        if pointer.kind == PointerKind::Move {
            if sink
                .queued
                .is_none_or(|queued| sink.sender.written_count() >= queued)
            {
                sink.held = None;
                sink.queued = sink.sender.push_counted(line).ok();
            } else {
                sink.held = Some(line);
                if !sink.started {
                    sink.started = true;
                    drop(sinks);
                    self.start();
                    return;
                }
                drop(sinks);
                self.0.wake.notify_all();
            }
            return;
        }
        if let Some(held) = sink.held.take() {
            let _ = sink.sender.push(held);
        }
        sink.queued = None;
        let _ = sink.sender.push(line);
    }

    /// Start the thread that tells the moves held, once their turn comes.
    fn start(&self) {
        let inner = Arc::downgrade(&self.0);
        let _ = std::thread::Builder::new()
            .name("columbina-pointer".into())
            .spawn(move || loop {
                let Some(inner) = inner.upgrade() else { return };
                let mut sinks = inner.sinks.lock().unwrap_or_else(PoisonError::into_inner);
                let mut waiting = false;
                for sink in sinks.sinks.iter_mut().flatten() {
                    let Some(held) = sink.held.take() else {
                        continue;
                    };
                    if sink
                        .queued
                        .is_none_or(|queued| sink.sender.written_count() >= queued)
                    {
                        sink.queued = sink.sender.push_counted(held).ok();
                    } else {
                        sink.held = Some(held);
                        waiting = true;
                    }
                }
                let wait = if waiting {
                    POLL
                } else {
                    Duration::from_secs(1)
                };
                drop(
                    inner
                        .wake
                        .wait_timeout(sinks, wait)
                        .unwrap_or_else(PoisonError::into_inner),
                );
            });
    }
}

/// A pointer as `view.pointer` tells it.
fn written(
    monitor: Monitor,
    instance: u64,
    pointer: &Pointer,
    snapshot: Option<&crate::presentation::FrameSnapshot>,
) -> serde_json::Value {
    let buttons: Vec<&str> = [(1, "left"), (2, "right"), (4, "middle")]
        .into_iter()
        .filter(|(bit, _)| pointer.buttons & bit != 0)
        .map(|(_, name)| name)
        .collect();
    json!({
        "monitor": monitor,
        "instance": instance,
        "presentation": crate::presentation::written(pointer.presentation),
        "snapshot": snapshot.map(crate::presentation::FrameSnapshot::written),
        "kind": pointer.kind.name(),
        "button": pointer.button.map(columbina_view::Button::name),
        "buttons": buttons,
        "x": pointer.picture.map(|(x, _)| x),
        "y": pointer.picture.map(|(_, y)| y),
        "inside": pointer.inside,
        "window": { "x": pointer.window.0, "y": pointer.window.1 },
        "shift": pointer.shift,
        "ctrl": pointer.ctrl,
        "alt": pointer.alt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_view::Button;

    fn pointer(kind: PointerKind, x: f64) -> Pointer {
        Pointer {
            presentation: None,
            kind,
            button: (kind != PointerKind::Move).then_some(Button::Left),
            buttons: u8::from(kind != PointerKind::Up),
            picture: Some((x, 2.5)),
            inside: true,
            window: (1, 2),
            shift: true,
            ctrl: false,
            alt: false,
        }
    }

    fn read(receiver: &mut queue::Receiver) -> Vec<serde_json::Value> {
        let mut read = Vec::new();
        while let Some(line) = receiver.try_recv() {
            let value: serde_json::Value = serde_json::from_slice(line.as_ref()).unwrap();
            assert_eq!(value["method"], "view.pointer");
            read.push(value["params"].clone());
        }
        read
    }

    #[test]
    fn pointer_keeps_the_successful_presentation_identity_and_map() {
        let mut event = pointer(PointerKind::Down, 2.0);
        event.presentation = Some(columbina_view::Presentation {
            serial: 19,
            tag: Some(71),
            map: columbina_view::Viewport::Fit.map((320, 180), (640, 360), 0),
        });
        let value = written(Monitor::Timeline, 3, &event, None);
        assert_eq!(value["presentation"]["serial"], 19);
        assert_eq!(value["presentation"]["tag"], 71);
        assert_eq!(value["presentation"]["map"]["picture"], json!([640, 360]));
        assert_eq!(value["x"], 2.0);
        assert_eq!(value["instance"], 3);
        assert_eq!(value["snapshot"], serde_json::Value::Null);
    }

    #[test]
    fn old_reporters_and_held_moves_cannot_enter_the_new_instance() {
        let pointers = Pointers::new(crate::presentation::Registry::default());
        let (sender, mut receiver) = queue::queue(queue::Limits {
            messages: 100,
            bytes: 1 << 20,
        });
        pointers.activate(Monitor::Timeline, Some(1));
        pointers.tell(Monitor::Timeline, 7, sender, None);
        pointers.told(Monitor::Timeline, 1, &pointer(PointerKind::Move, 1.0));
        pointers.told(Monitor::Timeline, 1, &pointer(PointerKind::Move, 2.0));
        // The already queued first move remains honestly attributed to the old instance.
        pointers.activate(Monitor::Timeline, Some(2));
        pointers.gone(Monitor::Timeline, 1);
        pointers.told(Monitor::Timeline, 1, &pointer(PointerKind::Down, 3.0));
        pointers.told(Monitor::Timeline, 2, &pointer(PointerKind::Down, 4.0));
        let told = read(&mut receiver);
        assert_eq!(told.len(), 2);
        assert_eq!(told[0]["instance"], 1);
        assert_eq!(told[1]["instance"], 2);
        assert_eq!(told[1]["x"], 4.0);
    }

    #[test]
    fn moves_wait_for_the_one_before_to_be_written_and_the_last_is_always_told() {
        let (sender, mut receiver) = queue::queue(queue::Limits {
            messages: 100,
            bytes: 1 << 20,
        });
        let pointers = Pointers::new(crate::presentation::Registry::default());
        pointers.activate(Monitor::Timeline, Some(3));
        pointers.activate(Monitor::Source, Some(4));
        // No one asked: nothing is told.
        pointers.told(Monitor::Timeline, 3, &pointer(PointerKind::Down, 1.0));
        pointers.tell(Monitor::Timeline, 7, sender, Some(Cursor::Crosshair));
        assert_eq!(pointers.cursor(Monitor::Timeline), Some(Cursor::Crosshair));
        pointers.told(Monitor::Timeline, 3, &pointer(PointerKind::Down, 1.0));
        for x in 2..10 {
            pointers.told(
                Monitor::Timeline,
                3,
                &pointer(PointerKind::Move, f64::from(x)),
            );
        }
        // The press, and the first move: the others wait for it to be written.
        let told = read(&mut receiver);
        assert_eq!(told.len(), 2);
        assert_eq!(told[0]["kind"], "down");
        assert_eq!(told[0]["button"], "left");
        assert_eq!(told[0]["buttons"], json!(["left"]));
        assert_eq!(told[0]["shift"], true);
        assert_eq!(told[1]["kind"], "move");
        assert_eq!(told[1]["x"], 2.0);
        assert_eq!(told[1]["button"], serde_json::Value::Null);
        // The release tells the move held, the last one, first.
        pointers.told(Monitor::Timeline, 3, &pointer(PointerKind::Up, 9.5));
        let told = read(&mut receiver);
        assert_eq!(told.len(), 2);
        assert_eq!(
            (told[0]["kind"].clone(), told[0]["x"].clone()),
            (json!("move"), json!(9.0))
        );
        assert_eq!(told[1]["kind"], "up");
        assert_eq!(told[1]["buttons"], json!([]));
        assert_eq!(told[1]["monitor"], "timeline");
        // Another monitor, and a connection forgotten.
        pointers.told(Monitor::Source, 4, &pointer(PointerKind::Down, 1.0));
        assert_eq!(read(&mut receiver), Vec::<serde_json::Value>::new());
        assert_eq!(pointers.forget(8), Vec::<Monitor>::new());
        assert_eq!(pointers.forget(7), vec![Monitor::Timeline]);
        pointers.told(Monitor::Timeline, 3, &pointer(PointerKind::Down, 1.0));
        assert_eq!(read(&mut receiver), Vec::<serde_json::Value>::new());
    }
}
