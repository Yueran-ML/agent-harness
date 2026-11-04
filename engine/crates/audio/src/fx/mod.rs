//! The nodes of the effects of a strip and a clip (plan of phase 3, T7–T13). What runs on the
//! thread of the mix here keeps convention 3 (`tests/rt_audit.rs` reads these files too).

pub mod biquad;
pub mod denoise;
pub mod dynamics;
pub mod eq;
pub mod rnnoise;
