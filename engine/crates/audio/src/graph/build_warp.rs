//! A graph played at a speed (T18): [`Graph::set_warp`] lays its clips on the samples out
//! through a [`Warp`], before it starts. Here memory may be taken (the window of the media grows
//! with the speed), so this file is one of those where the graph is made (`tests/rt_audit.rs`
//! reads none whose name starts with `build`).

use crate::constants::MAX_CHANNELS;

use super::clip::Window;
use super::{Graph, Warp};

impl Graph {
    /// Plays the timeline through `warp` from now on: sample `u` out is sample
    /// `warp.timeline(u)` of the timeline (T18). Each clip reads its media at its speed times the
    /// speed of the warp, forward or backward, filtered for that speed (its pitch follows the
    /// speed, as a clip's speed does; Pitch Correction and a clip's own pitch are not applied at
    /// another warp than the identity); its fades and keyframes, and every lane, are read where
    /// the timeline is. The nodes run on the samples out as they come, so that what plays
    /// backward is the sound of the timeline backward through the effects (as a clip played in
    /// reverse is). The identity gives the graph back as it was built.
    ///
    /// Called before [`Graph::start`], off the thread of the mix: the window of the media is made
    /// larger where a speed reads more of it in a block.
    pub fn set_warp(&mut self, warp: Warp) {
        self.warp = warp;
        let mut window = 0;
        for clip in &mut self.clips {
            window = window.max(clip.warp(warp, self.max_block));
        }
        if window > self.scratch.window.capacity {
            self.scratch.window = Window {
                data: vec![0.0; MAX_CHANNELS * window].into_boxed_slice(),
                capacity: window,
            };
        }
    }

    /// The first sample out at which a timeline that ends at sample `end` has nothing more to
    /// play: past its end forward, before its start backward ([`Warp::end_of`]).
    #[must_use]
    pub fn end_out(&self, end: i64) -> i64 {
        self.warp.end_of(end)
    }
}
