//! One background export of an immutable project snapshot, using original media and the
//! same compositor and mixer as playback. Subtitles are sidecars, never burned in.

mod output;
#[cfg(test)]
mod pictures;

use std::collections::VecDeque;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_compose::{
    Compositor, LayerColor, Scene, SceneLayer, Source, Target, Text, Transform, YuvCoding,
};
use columbina_media::{
    AudioFormat, AudioSettings, Codec, Hardware, MediaError, NativePicture, Picture, Quality,
    VideoSettings, Writer,
};
use columbina_model::{resolve, Project, TrackKind};
use serde::Deserialize;
use serde_json::json;

use crate::gpu::Graphics;
use crate::grading::{Grading, LutPlaces, MissingLut};
use crate::play::{mix::Mixer, sources::Sources, Notify};
use crate::rpc::{self, Fault};
use crate::srt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    timeline: String,
    path: PathBuf,
    codec: VideoCodec,
    quality: VideoQuality,
    range: Option<FrameRange>,
    /// The tracks of subtitles of the timeline that are written beside the film, each as an SRT,
    /// shown or hidden, as the Deliver page of `DaVinci` has a box to tick for each.
    #[serde(default)]
    subtitle_tracks: Vec<String>,
    #[serde(default)]
    replace: bool,
    /// The revision of the project that the client prepared the export by, as the names of its
    /// SRT files; refused as `stale_revision` where the project is not at it any more.
    #[serde(default)]
    pub revision: Option<u64>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VideoCodec {
    H264,
    Hevc,
    Prores,
}

impl From<VideoCodec> for Codec {
    fn from(value: VideoCodec) -> Self {
        match value {
            VideoCodec::H264 => Self::H264,
            VideoCodec::Hevc => Self::Hevc,
            VideoCodec::Prores => Self::ProRes,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VideoQuality {
    High,
    Medium,
    Small,
}

impl From<VideoQuality> for Quality {
    fn from(value: VideoQuality) -> Self {
        match value {
            VideoQuality::High => Self::High,
            VideoQuality::Medium => Self::Medium,
            VideoQuality::Small => Self::Small,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameRange {
    from: i64,
    to: i64,
}

pub struct Renders {
    graphics: Arc<Graphics>,
    luts: LutPlaces,
    notify: Notify,
    project_dir: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    next: u64,
    active: Option<Active>,
    stopped: bool,
}

struct Active {
    job: String,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// The caller queues the job id before releasing its first notification.
pub struct Launch {
    pub job: String,
    go: mpsc::Sender<()>,
}

impl Launch {
    pub fn start(self) {
        let _ = self.go.send(());
    }
}

impl Renders {
    pub fn new(
        project_dir: &Path,
        graphics: Arc<Graphics>,
        luts: LutPlaces,
        notify: Notify,
    ) -> Self {
        Self {
            graphics,
            luts,
            notify,
            project_dir: project_dir.to_owned(),
            state: Mutex::default(),
        }
    }

    pub fn prepare(&self, project: Project, request: Request) -> Result<Launch, Fault> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.stopped {
            return Err(Fault::new(rpc::REFUSED, "the engine is stopping")
                .with_data(json!({"reason":"stopping"})));
        }
        if state
            .active
            .as_ref()
            .is_some_and(|job| !job.done.load(Ordering::Acquire))
        {
            return Err(Fault::new(rpc::REFUSED, "an export is already running")
                .with_data(json!({"reason":"busy"})));
        }
        if let Some(old) = state.active.take() {
            let _ = old.thread.join();
        }
        let plan = Plan::new(project, request, &self.project_dir)?;
        state.next += 1;
        let job = format!("render-{}", state.next);
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            plan,
            graphics: Arc::clone(&self.graphics),
            luts: self.luts.clone(),
            missing: Vec::new(),
            notify: Arc::clone(&self.notify),
            cancel: Arc::clone(&cancel),
            job: job.clone(),
            frame: 0,
            since: Instant::now(),
            told: None,
            encoder: None,
            message: None,
            restored: 0,
            #[cfg(test)]
            lose_at: None,
        };
        let (go, ready) = mpsc::channel();
        let completed = Arc::clone(&done);
        let thread = std::thread::Builder::new()
            .name("timeline-export".into())
            .spawn(move || {
                if ready.recv().is_ok() {
                    worker.since = Instant::now();
                    let result = worker.run();
                    let status = if result.is_ok() {
                        "done"
                    } else if worker.cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    if let Err(error) = result {
                        worker.message = Some(error);
                    }
                    completed.store(true, Ordering::Release);
                    worker.tell(status);
                }
                completed.store(true, Ordering::Release);
            })
            .map_err(|error| Fault::new(rpc::REFUSED, error.to_string()))?;
        state.active = Some(Active {
            job: job.clone(),
            cancel,
            done,
            thread,
        });
        Ok(Launch { job, go })
    }

    pub fn cancel(&self, job: &str) -> Result<(), Fault> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let active = state
            .active
            .as_ref()
            .filter(|active| active.job == job)
            .ok_or_else(|| Fault::params(format!("there is no render job {job}")))?;
        active.cancel.store(true, Ordering::Release);
        Ok(())
    }

    pub fn stop(&self) {
        let active = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.stopped = true;
            state.active.take()
        };
        if let Some(active) = active {
            active.cancel.store(true, Ordering::Release);
            let _ = active.thread.join();
        }
    }
}

struct Plan {
    project: Arc<Project>,
    timeline: usize,
    path: PathBuf,
    range: Range<i64>,
    video: VideoSettings,
    subtitle_tracks: Vec<String>,
    replace: bool,
    project_dir: PathBuf,
}

impl Plan {
    fn new(project: Project, request: Request, dir: &Path) -> Result<Self, Fault> {
        let index = project
            .timelines
            .iter()
            .position(|timeline| timeline.id == request.timeline)
            .ok_or_else(|| {
                Fault::params(format!(
                    "there is no timeline {} in the project",
                    request.timeline
                ))
                .with_data(json!({"reason": "missing", "timeline": request.timeline}))
            })?;
        let timeline = &project.timelines[index];
        let duration = resolve::duration(timeline);
        let range = request
            .range
            .map_or(0..duration, |range| range.from..range.to);
        if range.start < 0 || range.end > duration || range.start >= range.end {
            return Err(Fault::params(
                "the export range must be nonempty and inside the timeline",
            ));
        }
        let codec = Codec::from(request.codec);
        let extension = request
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if !(extension.eq_ignore_ascii_case("mov")
            || extension.eq_ignore_ascii_case("mp4") && codec != Codec::ProRes)
        {
            return Err(Fault::params("export to .mp4 (h264/hevc) or .mov"));
        }
        for (at, id) in request.subtitle_tracks.iter().enumerate() {
            let subtitles = timeline
                .tracks
                .iter()
                .any(|track| track.id == *id && track.kind == TrackKind::Subtitle);
            let why = if !subtitles {
                "is not a subtitle track of the timeline"
            } else if request.subtitle_tracks[..at].contains(id) {
                "is named twice"
            } else {
                continue;
            };
            return Err(Fault::params(format!("subtitle_tracks: track {id} {why}"))
                .with_data(json!({"reason": "subtitle_track", "track": id})));
        }
        // Every check first: a refused export takes no partial file away.
        let mut parts: Vec<PathBuf> = vec![];
        parts.extend(output::check(&request.path, dir, request.replace)?);
        output::not_a_source(&request.path, &project)?;
        for (path, _) in srt::files(
            timeline,
            &request.subtitle_tracks,
            &request.path,
            range.clone(),
        ) {
            parts.extend(output::check(&path, dir, request.replace)?);
            output::not_a_source(&path, &project)?;
        }
        let bits = if codec == Codec::ProRes
            || codec == Codec::Hevc
                && project.media.iter().any(|media| {
                    media
                        .facts
                        .video
                        .as_ref()
                        .is_some_and(|video| video.bits > 8)
                }) {
            10
        } else {
            8
        };
        let video = VideoSettings {
            codec,
            width: timeline.width,
            height: timeline.height,
            rate: timeline.rate,
            quality: request.quality.into(),
            bits,
        };
        output::remove_parts(&parts)?;
        Ok(Self {
            project: Arc::new(project),
            timeline: index,
            path: request.path,
            range,
            video,
            subtitle_tracks: request.subtitle_tracks,
            replace: request.replace,
            project_dir: dir.to_owned(),
        })
    }
}

struct Worker {
    plan: Plan,
    graphics: Arc<Graphics>,
    /// Where the LUTs of the grades are read from.
    luts: LutPlaces,
    /// The LUTs that the grades of the timeline name and that cannot be read.
    missing: Vec<MissingLut>,
    notify: Notify,
    cancel: Arc<AtomicBool>,
    job: String,
    frame: i64,
    since: Instant,
    told: Option<Instant>,
    encoder: Option<String>,
    message: Option<String>,
    /// How many times a card was made again for the export after one was lost.
    restored: u32,
    /// The frame, counted from the start of the export, at which a test loses the card.
    #[cfg(test)]
    lose_at: Option<i64>,
}

impl Worker {
    fn cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            Err("the export was cancelled".into())
        } else {
            Ok(())
        }
    }

    #[allow(clippy::cast_precision_loss, reason = "display-only export speed")]
    fn tell(&mut self, state: &str) {
        if state == "running"
            && self
                .told
                .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        let mut note = json!({"job":self.job, "state":state, "frame":self.frame,
            "frames":self.plan.range.end - self.plan.range.start,
            "fps":self.frame as f64 / self.since.elapsed().as_secs_f64().max(0.001), "path":self.plan.path});
        if let Some(encoder) = &self.encoder {
            note["encoder"] = json!(encoder);
        }
        if let Some(message) = &self.message {
            note["message"] = json!(message);
        }
        if !self.missing.is_empty() {
            note["missing_luts"] = json!(self.missing);
        }
        (self.notify)(&note);
        self.told = Some(Instant::now());
    }

    fn writer(&mut self, outputs: &mut output::Files, sound: bool) -> Result<Writer, String> {
        self.writer_with(outputs, sound, Writer::create)
    }

    fn writer_with(
        &mut self,
        outputs: &mut output::Files,
        sound: bool,
        mut open: impl FnMut(&Path, VideoSettings, Option<AudioSettings>) -> Result<Writer, MediaError>,
    ) -> Result<Writer, String> {
        let mut movie = outputs
            .add(&self.plan.path)
            .map_err(|error| error.to_string())?;
        let audio = sound.then_some(AudioSettings::default());
        let writer = match open(&movie, self.plan.video, audio) {
            Ok(writer) => writer,
            Err(MediaError::NoEncoder(why)) if self.plan.video.codec != Codec::ProRes => {
                // Recheck the actual destination before creating a fallback. A requested MP4
                // does not authorize overwriting an unrelated existing MOV or subtitle file.
                self.plan.path.set_extension("mov");
                let part =
                    output::check(&self.plan.path, &self.plan.project_dir, self.plan.replace)
                        .map_err(|error| error.message)?;
                output::not_a_source(&self.plan.path, &self.plan.project)
                    .map_err(|error| error.message)?;
                output::remove_parts(part.as_slice()).map_err(|error| error.message)?;
                *outputs = output::Files::new(self.plan.replace);
                movie = outputs
                    .add(&self.plan.path)
                    .map_err(|error| error.to_string())?;
                self.plan.video.codec = Codec::ProRes;
                self.plan.video.bits = 10;
                self.message = Some(format!(
                    "hardware encoding is unavailable ({why}); writing ProRes to {}",
                    self.plan.path.display()
                ));
                open(&movie, self.plan.video, audio).map_err(|error| error.to_string())?
            }
            Err(error) => return Err(error.to_string()),
        };
        self.encoder = Some(writer.encoder().name().into());
        Ok(writer)
    }

    fn run(&mut self) -> Result<(), String> {
        self.cancelled()?;
        let project = Arc::clone(&self.plan.project);
        let timeline = &project.timelines[self.plan.timeline];
        let sound = timeline.tracks.iter().any(|track| {
            track.kind == TrackKind::Audio
                && track.enabled
                && track.clips.iter().any(|clip| clip.enabled)
        });
        let mut outputs = output::Files::new(self.plan.replace);
        let mut writer = self.writer(&mut outputs, sound)?;
        let sidecars = srt::files(
            timeline,
            &self.plan.subtitle_tracks,
            &self.plan.path,
            self.plan.range.clone(),
        );
        for (path, text) in sidecars {
            let stale = output::check(&path, &self.plan.project_dir, self.plan.replace)
                .map_err(|error| error.message)?;
            output::remove_parts(stale.as_slice()).map_err(|error| error.message)?;
            let part = outputs.add(&path).map_err(|error| error.to_string())?;
            std::fs::write(part, text).map_err(|error| error.to_string())?;
        }
        let mut drawing = self.drawing(timeline)?;
        drawing
            .grading
            .update(&drawing.gpu, &project, self.plan.timeline);
        self.missing = drawing.grading.missing().to_vec();
        let sources = Sources::new(Hardware::Auto, Arc::new(|_, _| {}));
        let sample_at = |frame: i64, rate: u32| {
            i64::try_from(
                i128::from(frame) * i128::from(rate) * i128::from(timeline.rate.den)
                    / i128::from(timeline.rate.num),
            )
            .map_err(|_| "export exceeds audio sample time".to_owned())
        };
        let mut mix = Mixer::new(Arc::clone(&project), self.plan.timeline).source(
            AudioFormat {
                rate: 48000,
                channels: 2,
            },
            sample_at(self.plan.range.start, timeline.sample_rate)?,
        );
        let mut pending = VecDeque::new();
        let mut samples = 0;
        let mut picture = Picture::default();
        self.tell("running");
        for frame in self.plan.range.clone() {
            self.cancelled()?;
            let (mut scene, clips) = scene(&project, timeline, frame, &sources)?;
            self.draw(
                &mut drawing,
                timeline,
                (&mut scene, &clips, frame),
                &mut picture,
            )?;
            let encoder = self.encoder.clone().unwrap_or_default();
            writer.video(&picture).map_err(|error| {
                format!(
                    "the encoder {encoder} failed at frame {} of the export: {error}",
                    self.frame
                )
            })?;
            self.frame += 1;
            if sound {
                let end = sample_at(self.frame, 48000)?;
                let needed =
                    usize::try_from((end - samples) * 2).map_err(|_| "invalid audio block")?;
                while pending.len() < needed {
                    if let Some(block) = mix.next()? {
                        pending.extend(block);
                    } else {
                        pending.resize(needed, 0.0);
                        break;
                    }
                }
                writer
                    .audio(&pending.drain(..needed).collect::<Vec<_>>())
                    .map_err(|error| format!("the sound encoder failed: {error}"))?;
                samples = end;
            }
            self.tell("running");
            // Keep at most one export frame submitted; never queue an entire job on the GPU.
            std::thread::yield_now();
        }
        if self.restored > 0 {
            let restored = format!(
                "the graphics device was lost and made again {} time(s); the export went on",
                self.restored
            );
            self.message = Some(match self.message.take() {
                Some(earlier) => format!("{earlier}; {restored}"),
                None => restored,
            });
        }
        self.cancelled()?;
        writer
            .finish()
            .map_err(|error| format!("the movie could not be finished: {error}"))?;
        self.cancelled()?;
        outputs.publish().map_err(|error| error.to_string())
    }

    /// The card, a compositor and a picture of the size of the timeline to draw on with them.
    fn drawing(&self, timeline: &columbina_model::Timeline) -> Result<Drawing, String> {
        let gpu = self.graphics.gpu().map_err(|error| error.to_string())?;
        let compositor = Compositor::new(Arc::clone(&gpu));
        let target = Target::export(&gpu, timeline.width, timeline.height, self.plan.video.bits)
            .map_err(|error| error.to_string())?;
        Ok(Drawing {
            grading: Grading::new(self.luts.clone()),
            gpu,
            compositor,
            target,
        })
    }

    /// Draw a frame, its scene graded as the clips of its layers and the timeline are at that
    /// frame of the timeline, and read it back. Where the card is lost, before or while it
    /// draws, a new card is made and the frame is drawn again with it (its grades made again on
    /// it): nothing of the frame was written yet, so the export goes on from the last frame that
    /// it wrote. The export fails, saying so, where no card can be made again within [`RESTORE`].
    fn draw(
        &mut self,
        drawing: &mut Drawing,
        timeline: &columbina_model::Timeline,
        (scene, clips, frame): (&mut Scene, &[String], i64),
        picture: &mut Picture,
    ) -> Result<(), String> {
        #[cfg(test)]
        if self.lose_at == Some(self.frame) {
            drawing.gpu.mark_lost("lost by a test of the export");
        }
        let mut restored = 0;
        loop {
            if drawing.gpu.is_lost() {
                if restored == 3 {
                    return Err(format!(
                        "the graphics device was lost again and again at frame {} of the export",
                        self.frame
                    ));
                }
                *drawing = self.restore(timeline)?;
                restored += 1;
                self.restored += 1;
            }
            drawing
                .grading
                .update(&drawing.gpu, &self.plan.project, self.plan.timeline);
            drawing.grading.grade(scene, clips, frame);
            let drawn = {
                let _turn = self.graphics.draw_turn(false);
                self.cancelled()?;
                drawing
                    .compositor
                    .render(scene, &drawing.target)
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        drawing
                            .target
                            .read_picture(picture)
                            .map_err(|error| error.to_string())
                    })
            };
            match drawn {
                Ok(()) => return Ok(()),
                Err(_) if drawing.gpu.is_lost() => {}
                Err(error) => {
                    return Err(format!(
                        "frame {} of the export could not be drawn: {error}",
                        self.frame
                    ))
                }
            }
        }
    }

    /// A drawing on a card made in place of one that was lost, waited for until one can be made
    /// or the export is cancelled.
    fn restore(&mut self, timeline: &columbina_model::Timeline) -> Result<Drawing, String> {
        let since = Instant::now();
        let mut why = String::from("every card made was lost at once");
        loop {
            self.cancelled()?;
            match self.drawing(timeline) {
                Ok(drawing) if !drawing.gpu.is_lost() => {
                    crate::log::log(
                        "render_gpu_restored",
                        &json!({"job":self.job, "frame":self.frame, "adapter":drawing.gpu.name()}),
                    );
                    return Ok(drawing);
                }
                Ok(_) => {}
                Err(error) => why = error,
            }
            if since.elapsed() >= RESTORE {
                return Err(format!(
                    "the graphics device was lost at frame {} of the export and could not be \
                     made again: {why}",
                    self.frame
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// How long an export waits for a card to be made again after one was lost.
const RESTORE: Duration = Duration::from_secs(30);

/// What an export draws its frames with.
struct Drawing {
    /// The grades of the timeline, ready on the card.
    grading: Grading,
    gpu: Arc<columbina_compose::Gpu>,
    compositor: Compositor,
    target: Target,
}

/// What an export shows at `frame` of the timeline, not yet graded, and the clip of each of its
/// layers (or the transition, for the colour of a dip), for [`Grading::grade`].
fn scene(
    project: &Project,
    timeline: &columbina_model::Timeline,
    frame: i64,
    sources: &Sources,
) -> Result<(Scene, Vec<String>), String> {
    let mut scene = Scene {
        width: timeline.width,
        height: timeline.height,
        ..Scene::default()
    };
    let mut clips = Vec::new();
    for layer in resolve::video_at(timeline, project, frame) {
        clips.push(layer.clip.clone());
        let (source, color) = match layer.content {
            resolve::Content::Color { rgba } => (Source::color(rgba), LayerColor::default()),
            resolve::Content::Title { text, style } => {
                (Source::Title(Text { text, style }), LayerColor::default())
            }
            resolve::Content::Media { media, tick } => {
                let media = project
                    .media
                    .iter()
                    .find(|item| item.id == media)
                    .ok_or("export media is missing")?;
                let read = sources
                    .picture(media, &layer.clip, tick, false, false)
                    .map_err(|error| error.to_string())?
                    .ok_or("the export decoder pool is busy")?;
                let color = layer_color(&read.picture);
                (Source::Picture(read.picture, read.rotation), color)
            }
        };
        scene.layers.push(SceneLayer {
            source,
            transform: layer.transform.into(),
            opacity: layer.opacity,
            color,
        });
    }
    Ok((scene, clips))
}

/// The colour of a layer of a picture, for the preview and the export alike: how its codes are
/// made R'G'B', and its input transform (none yet). Its grade is that of its clip, which
/// [`Grading::grade`] puts in.
///
/// The matrix and the range are those that the picture says ([`YuvCoding::of_native`]): the
/// readers give pictures that keep the codes of their file, decoded on the card or on the
/// processor alike, whose matrix is the one that the frame says, or that players take where it
/// says none (BT.601 for SD, BT.709 else), and whose range is full where the frame says so; a
/// picture stored as R'G'B' (a still) is made Y'CbCr of BT.709 of limited range as it is read,
/// and says so. A picture of 4:2:2 or 4:4:4 keeps its chroma.
pub(crate) fn layer_color(picture: &NativePicture) -> LayerColor {
    LayerColor {
        coding: YuvCoding::of_native(picture),
        ..LayerColor::default()
    }
}

/// A layer that shows a picture by itself, as the source monitor shows it: fitted to the canvas,
/// unmoved, opaque, decoded as [`layer_color`] says, as a picture of a clip of a timeline is.
pub(crate) fn picture_layer(picture: Arc<NativePicture>, rotation: i16) -> SceneLayer {
    SceneLayer {
        color: layer_color(&picture),
        source: Source::Picture(picture, rotation),
        transform: Transform::NONE,
        opacity: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(dir: &Path) -> Worker {
        let mut project = Project::sample();
        project.timelines[0].width = 64;
        project.timelines[0].height = 36;
        let request = Request {
            timeline: project.timelines[0].id.clone(),
            path: dir.join("film.mp4"),
            codec: VideoCodec::H264,
            quality: VideoQuality::Medium,
            range: None,
            subtitle_tracks: vec![],
            replace: false,
            revision: None,
        };
        Worker {
            plan: Plan::new(project, request, dir).unwrap(),
            graphics: Graphics::new(),
            luts: LutPlaces::with_library(dir, vec![]),
            missing: Vec::new(),
            notify: Arc::new(|_| {}),
            cancel: Arc::new(AtomicBool::new(false)),
            job: "test".into(),
            frame: 0,
            since: Instant::now(),
            told: None,
            encoder: None,
            message: None,
            restored: 0,
            lose_at: None,
        }
    }

    #[test]
    fn an_export_asked_for_while_the_engine_stops_is_refused_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let renders = Renders::new(
            dir.path(),
            Graphics::new(),
            LutPlaces::with_library(dir.path(), vec![]),
            Arc::new(|_| {}),
        );
        renders.stop();
        let project = Project::sample();
        let request = Request {
            timeline: project.timelines[0].id.clone(),
            path: dir.path().join("film.mov"),
            codec: VideoCodec::Prores,
            quality: VideoQuality::Medium,
            range: None,
            subtitle_tracks: vec![],
            replace: false,
            revision: None,
        };
        let Err(error) = renders.prepare(project, request) else {
            panic!("an export started while the engine stops");
        };
        assert_eq!(error.code, rpc::REFUSED);
        assert_eq!(error.data, Some(json!({"reason":"stopping"})));
    }

    #[test]
    fn an_export_goes_on_from_the_frame_it_was_at_with_a_new_card_when_the_card_is_lost() {
        if columbina_compose::gpu_for_tests().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::sample();
        let timeline = &mut project.timelines[0];
        (timeline.width, timeline.height) = (64, 36);
        timeline
            .tracks
            .retain(|track| track.kind == TrackKind::Video);
        let picture = &mut timeline.tracks[0].clips[0];
        picture.source = columbina_model::Source::Color {
            color: columbina_model::Rgba([255, 0, 0, 255]),
        };
        picture.length = 12;
        let request = Request {
            timeline: timeline.id.clone(),
            path: dir.path().join("film.mov"),
            codec: VideoCodec::Prores,
            quality: VideoQuality::Medium,
            range: None,
            subtitle_tracks: vec![],
            replace: false,
            revision: None,
        };
        let mut worker = worker(dir.path());
        worker.plan = Plan::new(project, request, dir.path()).unwrap();
        worker.lose_at = Some(5);
        worker.run().unwrap();
        assert_eq!(worker.restored, 1);
        assert_eq!(worker.frame, 12);
        assert!(worker.message.unwrap().contains("made again 1 time(s)"));
        let mut decoder =
            columbina_media::VideoDecoder::open(&worker.plan.path, Hardware::Off).unwrap();
        for frame in 0..12 {
            let raw = decoder.decoded_at(frame).unwrap();
            let rgba = columbina_media::Converter::new()
                .rgba_scaled(&raw, 64, 36)
                .unwrap();
            let pixel = rgba.data.as_chunks::<4>().0[64 * 18 + 32];
            assert!(pixel[0] > 240 && pixel[1] < 12, "frame {frame}: {pixel:?}");
        }
        assert!(decoder.frame_at(12).is_err());
    }

    /// A grade of one corrector, set by `set`.
    fn corrector(set: impl FnOnce(&mut columbina_model::Corrector)) -> columbina_model::Grade {
        use columbina_model::{Kind, Link, Node, NodeKind};
        let mut node = Node::new("n1", Kind::Corrector);
        let NodeKind::Corrector(settings) = &mut node.kind else {
            unreachable!("a corrector")
        };
        set(settings);
        columbina_model::Grade {
            nodes: vec![node],
            links: vec![Link::rgb("source", "n1"), Link::rgb("n1", "output")],
        }
    }

    #[test]
    fn an_export_is_graded_as_the_monitor_grades_it() {
        use columbina_compose::grade::{evaluate, GradeOptions};
        use columbina_model::{Group, Interp, Key, Param};
        const ORANGE: [u8; 4] = [200, 110, 40, 255];
        if columbina_compose::gpu_for_tests().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::sample();
        let keyed = Param::Keys(vec![
            Key {
                frame: 0,
                value: 1.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 8,
                value: 0.25,
                interp: Interp::Linear,
            },
        ]);
        let own = corrector(|settings| {
            let gain = &mut settings.primaries.gain;
            (gain.r, gain.g, gain.b) = (keyed.clone(), keyed.clone(), keyed);
        });
        let post = corrector(|settings| settings.primaries.saturation = Param::Fixed(0.0));
        let over = corrector(|settings| settings.primaries.gain.y = Param::Fixed(0.8));
        project.groups = vec![Group {
            post: post.clone(),
            ..Group::new("k1", "Look")
        }];
        let timeline = &mut project.timelines[0];
        (timeline.width, timeline.height) = (64, 36);
        timeline.grade = over.clone();
        timeline
            .tracks
            .retain(|track| track.kind == TrackKind::Video);
        let clip = &mut timeline.tracks[0].clips[0];
        clip.source = columbina_model::Source::Color {
            color: columbina_model::Rgba(ORANGE),
        };
        (clip.start, clip.length) = (2, 10);
        clip.grade = own.clone();
        clip.group = Some("k1".into());
        let request = Request {
            timeline: timeline.id.clone(),
            path: dir.path().join("graded.mov"),
            codec: VideoCodec::Prores,
            quality: VideoQuality::High,
            range: Some(FrameRange { from: 2, to: 12 }),
            subtitle_tracks: vec![],
            replace: false,
            revision: None,
        };
        let mut worker = worker(dir.path());
        worker.plan = Plan::new(project, request, dir.path()).unwrap();
        worker.run().unwrap();
        assert_eq!(worker.missing, []);
        let mut decoder =
            columbina_media::VideoDecoder::open(&worker.plan.path, Hardware::Off).unwrap();
        for frame in 0..10_u8 {
            let raw = decoder.decoded_at(i64::from(frame)).unwrap();
            let rgba = columbina_media::Converter::new()
                .rgba_scaled(&raw, 64, 36)
                .unwrap();
            let got = rgba.data.as_chunks::<4>().0[64 * 18 + 32];
            let at = f64::from(frame);
            let mut rgb = [0, 1, 2].map(|i| f32::from(ORANGE[i]) / 255.0);
            for (grade, frame) in [(&own, at), (&post, at), (&over, at + 2.0)] {
                rgb = evaluate(grade, frame, &GradeOptions::default(), rgb);
            }
            for i in 0..3 {
                let wanted = rgb[i] * 255.0;
                assert!(
                    (f32::from(got[i]) - wanted).abs() <= 2.5,
                    "frame {frame}: {got:?}, and {rgb:?} was wanted"
                );
            }
        }
    }

    fn no_hardware(
        path: &Path,
        video: VideoSettings,
        audio: Option<AudioSettings>,
    ) -> Result<Writer, MediaError> {
        if video.codec == Codec::ProRes {
            Writer::create(path, video, audio)
        } else {
            Err(MediaError::NoEncoder("no hardware in this test".into()))
        }
    }

    #[test]
    fn unavailable_hardware_falls_back_to_a_real_prores_mov_with_an_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = worker(dir.path());
        let mut outputs = output::Files::new(false);
        let mut writer = worker
            .writer_with(&mut outputs, false, no_hardware)
            .unwrap();
        assert_eq!(worker.plan.path, dir.path().join("film.mov"));
        assert_eq!(worker.encoder.as_deref(), Some("prores_ks"));
        assert!(worker.message.as_ref().unwrap().contains("ProRes"));
        assert!(!dir.path().join("film.mp4.part").exists());
        writer
            .video(&Picture {
                width: 64,
                height: 36,
                bits: 8,
                luma: vec![16; 64 * 36],
                chroma: vec![128; 64 * 18],
                luma_stride: 64,
                chroma_stride: 64,
                ..Picture::default()
            })
            .unwrap();
        writer.finish().unwrap();
        outputs.publish().unwrap();
        assert_eq!(
            columbina_media::probe(&worker.plan.path)
                .unwrap()
                .video
                .unwrap()
                .codec,
            "prores"
        );
        assert!(!dir.path().join("film.mp4").exists());
    }

    #[test]
    fn fallback_does_not_overwrite_a_mov_that_was_not_the_requested_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("film.mov");
        std::fs::write(&path, b"keep existing movie").unwrap();
        let mut worker = worker(dir.path());
        let mut outputs = output::Files::new(false);
        assert!(worker
            .writer_with(&mut outputs, false, no_hardware)
            .is_err());
        drop(outputs);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing movie");
        assert!(!dir.path().join("film.mp4.part").exists());
        assert!(!dir.path().join("film.mov.part").exists());
    }
}
