//! The engine's graphics card ([`Gpu`]): made once, when it is first needed, shared by the view
//! (and, from phase 1, by the compositor), and made again when its device is lost.
//!
//! A headless engine draws in no window and makes no card yet. When the device is lost, the view
//! reports it; a thread of its own then makes another card, a second apart until one can be
//! made, and gives it to the view, which draws again on it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use columbina_compose::{Gpu, GpuError, Preference, Watch};
use serde_json::json;

use crate::log::log;

/// How long a card that cannot be made is waited for before it is tried again.
const REMAKE: Duration = Duration::from_secs(1);

/// How often a card that still cannot be made is logged.
const LOG_REMAKING: Duration = Duration::from_secs(30);

/// The card of the engine, once made, and what logs its errors.
pub struct Graphics {
    current: Mutex<Option<(Arc<Gpu>, Watch)>>,
    /// Whether a thread is making the card again.
    remaking: AtomicBool,
}

impl Graphics {
    pub fn new() -> Arc<Graphics> {
        Arc::new(Graphics {
            current: Mutex::new(None),
            remaking: AtomicBool::new(false),
        })
    }

    /// The card, made if there is none yet or the one there was is lost. Making it takes some
    /// 100 ms: call it where blocking is allowed.
    pub fn gpu(&self) -> Result<Arc<Gpu>, GpuError> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((gpu, _)) = current.as_ref() {
            if !gpu.is_lost() {
                return Ok(Arc::clone(gpu));
            }
        }
        let gpu = Gpu::new(Preference::HighPerformance)?;
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
        *current = Some((Arc::clone(&gpu), errors));
        Ok(gpu)
    }

    /// The card was lost: make another on a thread of its own, a second apart until one is
    /// made, and give it to `give`, unless `wanted` says that it is no longer wanted (the view
    /// was let go). Asked again while it is being made, nothing more is done.
    pub fn remake(
        self: &Arc<Self>,
        wanted: impl Fn() -> bool + Send + 'static,
        give: impl FnOnce(Arc<Gpu>) + Send + 'static,
    ) {
        if self.remaking.swap(true, Ordering::SeqCst) {
            return;
        }
        let graphics: Weak<Graphics> = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name("columbina-gpu-remake".into())
            .spawn(move || {
                let mut failures = 0_u32;
                let mut logged_at: Option<Instant> = None;
                loop {
                    let Some(graphics) = graphics.upgrade() else {
                        return;
                    };
                    if !wanted() {
                        graphics.remaking.store(false, Ordering::SeqCst);
                        return;
                    }
                    match graphics.gpu() {
                        Ok(gpu) => {
                            graphics.remaking.store(false, Ordering::SeqCst);
                            give(gpu);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_card_that_was_lost_is_made_again_and_given_to_whoever_wants_it() {
        let graphics = Graphics::new();
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
        first.mark_lost("as a test");
        let (given, got) = mpsc::channel();
        graphics.remake(|| true, move |gpu| given.send(gpu).unwrap());
        let second = got.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!second.is_lost());
        assert!(Arc::ptr_eq(&second, &graphics.gpu().unwrap()));
        // No longer wanted: nothing is made, and nothing given.
        second.mark_lost("as a test, again");
        let (given, got) = mpsc::channel::<Arc<Gpu>>();
        graphics.remake(|| false, move |gpu| given.send(gpu).unwrap());
        assert!(got.recv_timeout(Duration::from_secs(2)).is_err());
        assert!(!graphics.remaking.load(Ordering::SeqCst));
    }
}
