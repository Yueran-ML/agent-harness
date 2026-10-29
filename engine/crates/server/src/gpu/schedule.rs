//! Submit one composition at a time, giving a waiting preview the next turn. An export
//! holds its turn through readback, so it cannot fill the GPU queue ahead of playback.

use std::sync::{Condvar, Mutex, PoisonError};

#[derive(Default)]
pub(super) struct Schedule {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    busy: bool,
    previews: usize,
}

pub struct Turn<'a>(&'a Schedule);

impl Schedule {
    pub(super) fn turn(&self, preview: bool) -> Turn<'_> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.previews += usize::from(preview);
        while state.busy || !preview && state.previews > 0 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.previews -= usize::from(preview);
        state.busy = true;
        Turn(self)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .busy = false;
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_waiting_preview_runs_before_the_next_export_frame() {
        let schedule = Arc::new(Schedule::default());
        let occupied = schedule.turn(false);
        let (tx, rx) = mpsc::channel();
        let threads: Vec<_> = [false, true]
            .into_iter()
            .map(|preview| {
                let schedule = Arc::clone(&schedule);
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _turn = schedule.turn(preview);
                    tx.send(preview).unwrap();
                })
            })
            .collect();
        let until = Instant::now() + Duration::from_secs(5);
        while schedule.state.lock().unwrap().previews == 0 {
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        drop(occupied);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        assert!(!rx.recv_timeout(Duration::from_secs(5)).unwrap());
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
