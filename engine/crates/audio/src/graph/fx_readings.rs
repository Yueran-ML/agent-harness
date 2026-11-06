//! Typed DSP readers retained at construction, read only off the mix thread.
use super::{Graph, Owner};
use crate::fx::{deesser, ducker, dynamics, limiter};
use crate::rt::SeqReader;

#[derive(Clone, Debug)]
pub enum FxReader {
    Dynamics(SeqReader<dynamics::Ball>),
    Ducker(SeqReader<ducker::Meter>),
    Limiter(SeqReader<limiter::Meter>),
    DeEsser(SeqReader<deesser::Meter>),
    Noise(SeqReader<[f32; 258]>),
    Width(SeqReader<[f32; 130]>),
}
#[derive(Clone, Debug)]
pub enum FxOwner {
    Strip(Owner),
    Clip(String), // rt-ok: identities allocated only while building the graph
}
#[derive(Clone, Debug)]
pub struct FxTap {
    pub owner: FxOwner,
    pub insert: Option<String>, // rt-ok: immutable graph construction metadata
    pub slot: Option<usize>,
    pub reader: FxReader,
}
#[derive(Clone, Debug)]
pub struct FxGraph {
    pub generation: u64,
    pub rate: u32,
    pub state: SeqReader<(bool, u64)>,
    pub taps: Vec<FxTap>,
}
impl Graph {
    /// Readers of this exact graph, with whether it is playing and its count of metered blocks.
    #[must_use]
    pub fn fx_readings(&self) -> FxGraph {
        FxGraph {
            generation: self.id,
            rate: self.rate,
            state: self.fx_state.reader(),
            taps: self.fx_taps.clone(), // rt-ok: control-thread snapshot
        }
    }
}
