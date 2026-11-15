//! What a project of the studio is, as plain data that the engine reads and writes.
//!
//! The JSON of a project is read by JavaScript too, which holds every number as a double: so
//! every integer in it stays within 2^53.

pub mod audio;
pub mod catalog;
pub mod color;
pub mod color_management;
pub mod edit;
pub mod fusion;
pub mod grade;
pub mod history;
pub mod keyframes;
pub mod media_audio;
pub mod ops;
pub mod project;
pub mod resolve;
pub mod retime;
#[cfg(any(test, feature = "sample"))]
pub mod samples;
pub mod span;
pub mod time;
pub mod upgrade;

pub use audio::{
    AudioFade, AudioKey, AudioParam, AutomationArm, Bus, BusFormat, ChannelMap, ChannelOrder,
    ChannelSource, ChannelSources, ClipPitch, CrossfadeLevel, Effect, Equalizer, GroupControl,
    Insert, KeyInterp, Lane, LaneKind, LanePoint, LaneScale, Lanes, Output, Pan, RemixSource, Send,
    SourceRef, Stage, Strip, TimelineAudio, TrackAudio, TrackFormat, TrackGroup, Vca,
};
pub use color::{
    input_space, Basis, ColorFacts, ColorRange, DisplayPrimaries, Gamma, Gamut, InputSpace,
    LightLevel, Luminance, MasteringDisplay,
};
pub use color_management::{ColorOverride, ColorSettings, Levels, Science};
pub use edit::{
    expand, Attribute, Change, CopyOf, CueItem, Edge, Edit, EditError, Expansion, Given, KeyAction,
    Keyed, Material, Straddling,
};
pub use fusion::{Composition, Fused, FusedKind, FusedNode};
pub use grade::{
    Composite, Corrector, Grade, Group, Interp, Key, Kind, Link, Node, NodeKind, Param, Port, Pos,
    Qualifier, QualifierMode, ShotMatch, Target,
};
pub use history::{Applied, Changed, History, Step, TransactionError, HISTORY_MOST};
pub use media_audio::MediaAudioTrack;
pub use ops::{apply, apply_done, InsertsOf, Invalid, LanesOf, Op, OpError, LARGEST};
pub use project::{
    Alignment, AudioFacts, AudioStream, Bin, Clip, Crop, Cropped, Cue, Facts, Id, Identity, Media,
    Placed, Project, Proxy, ReadError, Rgba, Source, Styled, TextAlign, Timeline, TitleStyle,
    Track, TrackKind, Transform, Transition, TransitionKind, VideoFacts, SCHEMA,
};
pub use time::{Rate, Speed, Time};
