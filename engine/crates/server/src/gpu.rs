//! The engine's graphics card ([`Gpu`]): made once, when it is first needed, shared by those who
//! draw with it (the view, and from phase 1 the compositor), and made again when its device is
//! lost.
//!
//! Those who draw subscribe ([`Graphics::subscribe`]): each card made in place of another is
//! given to every one of them. When the device of the card is lost, a thread of its own makes
//! another, a second apart until one can be made. A card that was lost is waited for a while
//! ([`HARDWARE_FIRST`]) before the software adapter is taken in its place, since a driver that
//! resets brings it back in a few seconds; and while the engine draws in software, a card is
//! looked for every [`UPGRADE`], and taken as soon as there is one. A headless engine makes no
//! card yet.

mod schedule;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use columbina_compose::{Gpu, GpuError, Preference, Watch};
use serde_json::json;

use crate::log::log;

/// How long a card that cannot be made is waited for before it is tried again.
const REMAKE: Duration = Duration::from_secs(1);

/// How often a card that still cannot be made is logged.
const LOG_REMAKING: Duration = Duration::from_secs(30);

/// How long after a card was lost only a card is taken, not the software adapter.
const HARDWARE_FIRST: Duration = Duration::from_secs(10);

/// How often a card is looked for while the engine draws in software.
const UPGRADE: Duration = Duration::from_secs(30);

/// What is given each card made in place of another.
type Subscriber = Arc<dyn Fn(Arc<Gpu>) + Send + Sync>;

/// The card of the engine, and who is given the next.
pub struct Graphics {
    schedule: schedule::Schedule,
    state: Mutex<State>,
    /// Whether a thread is making the card again.
    remaking: AtomicBool,
    /// Whether a thread looks for a card while the engine draws in software.
    upgrading: AtomicBool,
    me: Weak<Graphics>,
}

struct State {
    /// The card, with what logs its errors and what tells that it was lost.
    current: Option<(Arc<Gpu>, Watch, Watch)>,
    /// When a card (not the software adapter) was last lost.
    hardware_lost_at: Option<Instant>,
    /// Whether the loss of the card there is was noted in `hardware_lost_at`.
    lost_noted: bool,
    subscribers: Vec<(u64, Subscriber)>,
    next: u64,
}

/// A subscriber of [`Graphics`], which is no longer given cards once this is dropped.
#[must_use = "the subscriber is let go when this is dropped"]
pub struct Subscription {
    graphics: Weak<Graphics>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(graphics) = self.graphics.upgrade() {
            graphics
                .lock()
                .subscribers
                .retain(|(each, _)| *each != self.id);
        }
    }
}

/// Which adapter is asked for, a card having been lost at `hardware_lost_at`: a card only for
/// a while after, and then any.
fn preference(hardware_lost_at: Option<Instant>, now: Instant) -> Preference {
    match hardware_lost_at {
        Some(at) if now.saturating_duration_since(at) < HARDWARE_FIRST => Preference::Hardware,
        _ => Preference::HighPerformance,
    }
}

impl Graphics {
    pub fn new() -> Arc<Graphics> {
        Arc::new_cyclic(|me| Graphics {
            schedule: schedule::Schedule::default(),
            state: Mutex::new(State {
                current: None,
                hardware_lost_at: None,
                lost_noted: false,
                subscribers: Vec::new(),
                next: 0,
            }),
            remaking: AtomicBool::new(false),
            upgrading: AtomicBool::new(false),
            me: me.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn draw_turn(&self, preview: bool) -> schedule::Turn<'_> {
        self.schedule.turn(preview)
    }

    /// Give `subscriber` each card made in place of another, from now on. It is called on the
    /// thread that made the card, and must not block for long.
    pub fn subscribe(&self, subscriber: impl Fn(Arc<Gpu>) + Send + Sync + 'static) -> Subscription {
        let mut state = self.lock();
        let id = state.next;
        state.next += 1;
        state.subscribers.push((id, Arc::new(subscriber)));
        Subscription {
            graphics: self.me.clone(),
            id,
        }
    }

    /// The card, made if there is none yet or the one there was is lost; those who subscribed
    /// are given one that is made in place of another. Making it takes some 100 ms: call it
    /// where blocking is allowed.
    pub fn gpu(&self) -> Result<Arc<Gpu>, GpuError> {
        self.made(None)
    }

    /// The card that is there and not lost, or one made as `preference` says (else as the time
    /// since a card was lost says), given to the subscribers.
    fn made(&self, preference: Option<Preference>) -> Result<Arc<Gpu>, GpuError> {
        let mut state = self.lock();
        if let Some((gpu, _, _)) = &state.current {
            if !gpu.is_lost() && preference.is_none() {
                return Ok(Arc::clone(gpu));
            }
            // A card lost: the first time it is found so, a card only is taken for a while.
            if gpu.is_lost() && !gpu.is_software() && !state.lost_noted {
                state.hardware_lost_at = Some(Instant::now());
                state.lost_noted = true;
            }
        }
        let wanted =
            preference.unwrap_or_else(|| self::preference(state.hardware_lost_at, Instant::now()));
        let gpu = Gpu::new(wanted)?;
        log(
            "gpu_ready",
            &json!({
                "adapter": gpu.name(),
                "backend": gpu.backend(),
                "software": gpu.is_software(),
                "sixteen_norm": gpu.has_sixteen_norm(),
            }),
        );
        let errors = gpu.when_error({
            let gpu = Arc::downgrade(&gpu);
            move |message: &str| {
                // The first errors, then one in a thousand: an error of every frame would
                // fill the log.
                let count = gpu.upgrade().map_or(1, |gpu| gpu.uncaught_errors().0);
                if count <= 10 || count % 1000 == 0 {
                    log("gpu_error", &json!({ "message": message, "count": count }));
                }
            }
        });
        let lost = gpu.when_lost({
            let me = self.me.clone();
            move |why: &str| {
                log("gpu_lost", &json!({ "message": why }));
                if let Some(graphics) = me.upgrade() {
                    graphics.remake();
                }
            }
        });
        let replaced = state.current.replace((Arc::clone(&gpu), errors, lost));
        state.lost_noted = false;
        let subscribers: Vec<Subscriber> = state
            .subscribers
            .iter()
            .map(|(_, subscriber)| Arc::clone(subscriber))
            .collect();
        drop(state);
        // The card that was there is let go here, so that those who drew with it can let it
        // go too, as they take the new one: a window has one swap chain at a time.
        let replacing = replaced.is_some();
        drop(replaced);
        if replacing {
            for subscriber in subscribers {
                subscriber(Arc::clone(&gpu));
            }
        }
        if gpu.is_software() {
            self.look_for_a_card();
        }
        Ok(gpu)
    }

    /// The card was lost: make another on a thread of its own, a second apart until one is made,
    /// and give it to the subscribers. Asked again while it is being made, nothing more is done.
    pub fn remake(&self) {
        if self.remaking.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        let spawned = std::thread::Builder::new()
            .name("columbina-gpu-remake".into())
            .spawn(move || {
                let mut failures = 0_u32;
                let mut logged_at: Option<Instant> = None;
                loop {
                    let Some(graphics) = me.upgrade() else {
                        return;
                    };
                    match graphics.made(None) {
                        Ok(_) => {
                            graphics.remaking.store(false, Ordering::SeqCst);
                            return;
                        }
                        Err(error) => {
                            failures += 1;
                            if logged_at.is_none_or(|at| at.elapsed() >= LOG_REMAKING) {
                                logged_at = Some(Instant::now());
                                log(
                                    "gpu_remaking",
                                    &json!({ "tries": failures, "error": error.to_string() }),
                                );
                            }
                        }
                    }
                    drop(graphics);
                    std::thread::sleep(REMAKE);
                }
            });
        if spawned.is_err() {
            self.remaking.store(false, Ordering::SeqCst);
        }
    }

    /// While the card is the software adapter, look for a card every [`UPGRADE`], and take it
    /// in its place, given to the subscribers.
    fn look_for_a_card(&self) {
        if self.upgrading.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        let spawned = std::thread::Builder::new()
            .name("columbina-gpu-upgrade".into())
            .spawn(move || loop {
                std::thread::sleep(UPGRADE);
                let Some(graphics) = me.upgrade() else {
                    return;
                };
                let software = graphics
                    .lock()
                    .current
                    .as_ref()
                    .is_some_and(|(gpu, _, _)| gpu.is_software() && !gpu.is_lost());
                if !software {
                    graphics.upgrading.store(false, Ordering::SeqCst);
                    return;
                }
                if graphics.made(Some(Preference::Hardware)).is_ok() {
                    log("gpu_upgraded", &json!({}));
                    graphics.upgrading.store(false, Ordering::SeqCst);
                    return;
                }
            });
        if spawned.is_err() {
            self.upgrading.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_card_is_taken_alone_for_a_while_after_one_was_lost() {
        let now = Instant::now();
        assert_eq!(preference(None, now), Preference::HighPerformance);
        assert_eq!(preference(Some(now), now), Preference::Hardware);
        let nine = now + Duration::from_secs(9);
        assert_eq!(preference(Some(now), nine), Preference::Hardware);
        let ten = now + HARDWARE_FIRST;
        assert_eq!(preference(Some(now), ten), Preference::HighPerformance);
    }

    #[test]
    fn a_card_that_was_lost_is_made_again_and_given_to_every_subscriber() {
        let graphics = Graphics::new();
        let (given, got) = mpsc::channel();
        let subscribe = |name: &'static str| {
            let given = given.clone();
            graphics.subscribe(move |gpu| given.send((name, gpu)).unwrap())
        };
        let (_first_monitor, _second_monitor) = (subscribe("source"), subscribe("timeline"));
        let dropped = subscribe("gone");
        drop(dropped);
        let first = match graphics.gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(
                    std::env::var("COLUMBINA_REQUIRE_GPU").as_deref() != Ok("1"),
                    "COLUMBINA_REQUIRE_GPU is 1, and there is no graphics card: {error}"
                );
                return;
            }
        };
        // Made once.
        assert!(Arc::ptr_eq(&first, &graphics.gpu().unwrap()));
        assert!(got.try_recv().is_err());
        // Lost as View::lose_device loses it: the device destroyed, and taken as lost.
        first.device().destroy();
        first.mark_lost("lost on purpose, by a test");
        let mut names = Vec::new();
        let mut cards = Vec::new();
        for _ in 0..2 {
            let (name, gpu) = got.recv_timeout(Duration::from_secs(30)).unwrap();
            names.push(name);
            cards.push(gpu);
        }
        names.sort_unstable();
        assert_eq!(names, ["source", "timeline"]);
        assert!(Arc::ptr_eq(&cards[0], &cards[1]));
        assert!(!Arc::ptr_eq(&first, &cards[0]));
        assert!(!cards[0].is_lost());
        assert_eq!(cards[0].is_software(), first.is_software());
        assert!(Arc::ptr_eq(&cards[0], &graphics.gpu().unwrap()));
        assert!(got.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(!graphics.remaking.load(Ordering::SeqCst));
    }
}
