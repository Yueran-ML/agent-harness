import { useRef, useState, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import { CLIP_KEYED, type Clip, type ClipKeyed, type Keyframe, type Media, type Param, type Retime, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { CLIP_BOUNDS, CLIP_SETTINGS, frameOfKey, keyFrameOf, keyOps, mediaOf, paramOf, settingNames, settingsOf } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { useCommit } from '../inspector/useCommit';
import {
  FASTEST, SLOWEST, beginsLater, clipCourse, constantRetime, courseOf, frameCurveIn, insideMedia, movedPoint, percentOf, raisedPoint, speedOfPercent,
  type Course, type PathEnd,
} from '../retime';
import type { Change } from '../timeline/commands';
import { percentText } from '../timeline/RetimeBars';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { CurveEditor, NO_SELECTION, type Curve, type CurveChange, type CurveClock, type CurveDisplay, type CurveSelection, type LineDrag } from './CurveEditor';

/**
 * The settings of a clip that take keys, in the order of the Inspector, its Video tab and then the Title tab: the keyframe track of the
 * timeline lists them so too. A clip that is no title has none of those of the Title tab, and a title none of a part of its style that it has
 * not got, as its outline, and no curve of them (`settingsOf`).
 */
export const CLIP_CURVES: readonly ClipKeyed[] = CLIP_SETTINGS;
/** The colour of the curve of each setting, as each track of DaVinci Resolve's Keyframe Editor has one (Reference Manual 20, p. 1113). */
const COLORS: Record<ClipKeyed, string> = {
  'transform.scale': '#e9b949', 'transform.x': '#e5534b', 'transform.y': '#57ab5a', 'transform.rotation': '#539bf5', 'transform.opacity': '#c9d1d9',
  'crop.left': '#b083f0', 'crop.right': '#f69d50', 'crop.top': '#39c5cf', 'crop.bottom': '#e275ad', 'crop.softness': '#8ddb8c',
  'title.size': '#f0883e', 'title.x': '#ff938a', 'title.y': '#6bc46d',
  'title.stroke.width': '#a5d6ff', 'title.shadow.x': '#d2a8ff', 'title.shadow.y': '#7ee787', 'title.shadow.blur': '#ffa198',
  'title.background.padding': '#f2cc60', 'title.background.radius': '#56d4dd',
};
/** The colours of the curves, in the order of the settings: another editor of curves (the Spline Editor of the Fusion page) draws its own with them. */
export const CURVE_COLORS: readonly string[] = Object.values(COLORS);
const isKeyed = (path: string): path is ClipKeyed => (CLIP_KEYED as readonly string[]).includes(path);

/** The Retime Frame curve of a clip, among the curves of the editor. */
export const RETIME_FRAME = 'retime.frame';
/** The Retime Speed curve of a clip, among the curves of the editor. */
export const RETIME_SPEED = 'retime.speed';
/** The colours of the two Retime curves. They are drawn over the frames of the clip, never over the curves of the settings. */
const RETIME_COLORS = { frame: '#dcbdfb', speed: '#96d0ff' };
/** A line whose points are placed by its own rules takes any value: the rules keep it where it may be. */
const ANY = { low: -Infinity, high: Infinity };

/** What the last move of a drag of a control point of the Retime Frame curve asks for, and the speed points that the clip had as the drag began. */
interface FramePlan { retime: Retime; keep: PathEnd; was: Retime }
/** What the last move of a drag of a step of the Retime Speed curve asks for: the speed of the segment that begins at frame `at`, in whole hundredths without a sign, and the one that it had. */
interface SpeedPlan { at: number; percent: number; was: number }

/**
 * The Curve Editor of the Edit page (DaVinci Resolve Reference Manual 20, pp. 1116–1119), beneath
 * the timeline: the settings of the transform and of the crop of the one clip of picture that is
 * chosen, and of the style of a title where it is one, as curves over the frames that their keys count. One clip of picture with its linked
 * sound chosen is one clip; with none or several chosen it says so and shows nothing.
 *
 * A drag of keys or of a handle is one gesture (`useAudioGesture`, as a drag of the keys of a clip's
 * volume): each move sets the setting whole with `set_clip_keys`, which is the same whatever went
 * before it, the last move ends the gesture, and the whole is one step to undo; taken back, the
 * setting is set to what it was. Every other edit is one transaction (`useCommit`) of
 * `splice_clip_keys`, of the keys that differ from those that the timeline has as it goes, or of
 * `set_clip_keys` where the last key of a setting is taken away. A clip on a locked track is shown
 * and cannot be changed.
 *
 * The ruler is in the frames of the keys, so a press on it moves the playhead to the frame of the
 * timeline at which the clip shows that picture (`frameOfKey`). Whatever changes the clip (an edit
 * from elsewhere, Undo) is drawn as it comes: the editor keeps nothing of the keys but the frames of
 * those that are chosen, and a key that is gone is chosen no longer.
 *
 * # The Retime curves
 *
 * A clip of media that lasts (not a title, a colour or a still) has two curves more, as DaVinci
 * Resolve's Retime Curve has (pp. 1078–1081), whether or not it has speed points. They are over
 * the frames of the clip, counted from its first, and not over those of its keys: the graph shows
 * them where one of them is the curve that is edited, with a ruler of the clip's frames. They are
 * listed after the settings, and shown from the start where the clip has speed points.
 *
 * - Retime Frame: the frame of the media that the clip is at (counted at the rate of the timeline
 *   from the start of the media) against the frame of the clip. Its control points are the speed
 *   points, one for one, and the end of the clip, with a straight line from each to the next,
 *   which falls where a segment plays backward and is level in a freeze.
 *   A point dragged up or down makes its frame of the clip show another frame of the source: the
 *   frames of the speed points stay, and the speeds of the segments either side change
 *   (`raisedPoint`). A point dragged sideways moves the speed point in time, the places staying,
 *   as the lower handle of the Retime Controls does (`movedPoint`). The first point and the end of
 *   the clip move up and down only. Every speed that a drag makes is a whole hundredth of the
 *   speed of the source from 1% to 10000%, or a freeze, and a move that would show what the media
 *   has not is not made: the point stays where its last move took it.
 *   The drag is one gesture of the edit `set_speed_points`, worked out at each move from the clip
 *   as it was when the drag began, and one step to undo; Escape takes it back. The engine keeps
 *   the place where the path begins at its tick of the media, so the pictures before the segments
 *   that change stay; for the first point it keeps the place where the path stops, and the clip
 *   begins in its media as much later as the point was raised. What the graph draws meanwhile is
 *   counted here as the engine counts it (`beginsLater`).
 * - Retime Speed: the speed of each segment in percent, a level step with a point where the
 *   segment begins. A point dragged up or down gives its segment another speed, the way that it
 *   goes, showing what it showed: one `set_segment_speed` as it is let go, as the upper handle of
 *   the Retime Controls sends, so the segment and the clip last longer or shorter. A freeze has no
 *   speed to change.
 * On either, Alt-click or a double click on the line puts a speed point there (`add_speed_point`),
 * and Delete clears the speed points chosen (`clear_speed_point`); the first point and the end of
 * the clip are not speed points that could be cleared. The buttons of interpolation, of copying
 * and of pasting are off for them: a segment goes at one speed.
 *
 * The page does not know the tool of the timeline nor whether its linked selection is on: these
 * edits take the clips linked to the clip, as the engine does where it is not told, and move
 * nothing that follows a clip that gets longer or shorter.
 */
export function ClipCurves({ bridge, session, revision, timeline, media, clips, frame, onSeek, onClose }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline | null; media: readonly Media[];
  /** The clips chosen on the timeline. */
  clips: readonly string[];
  /** The playhead, in frames of the timeline. */
  frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void;
  onClose?: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.curves;
  const locations = timeline ? clipLocations(timeline) : null;
  const chosen = clips.flatMap((id) => {
    const found = locations?.get(id);
    return found && found.track.kind === 'video' ? [found] : [];
  });
  const close = onClose && <button type="button" className="icon-btn" aria-label={labels.close} title={labels.close} onClick={onClose} data-test="close-curves"><X size={14} /></button>;
  if (!timeline || chosen.length !== 1) {
    return <section className="curve-panel" aria-label={labels.title} data-test="clip-curves">
      <header className="curve-editor-bar"><strong>{labels.title}</strong><span className="curve-editor-gap" />{close}</header>
      <p className="caption" data-test="curves-empty">{words.hint.studio.curvesEmpty}</p>
    </section>;
  }
  const { clip, track } = chosen[0];
  return <Curves key={`${session}/${timeline.id}/${clip.id}`} bridge={bridge} session={session} revision={revision} timeline={timeline} media={media}
    clip={clip} locked={track.locked} frame={frame} onSeek={onSeek} close={close} />;
}

/** The curves of one clip: see `ClipCurves`. Made anew for another clip, so nothing chosen is carried over. */
function Curves({ bridge, session, revision, timeline, media, clip, locked, frame, onSeek, close }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; media: readonly Media[];
  clip: Clip; locked: boolean; frame: number; onSeek: (frame: number, scrubbing: boolean) => void; close: ReactNode;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.inspector;
  const [selection, setSelection] = useState<CurveSelection>(NO_SELECTION);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${clip.id}/curves`, locked,
    onFailure: error => setRefusal(refusalOf(error, words.hint.studio)),
  });
  /** Whether a drag has begun a gesture that is still to be ended or taken back. */
  const dragging = useRef(false);
  /** What the last move of a drag of the Retime Frame curve, and of the Retime Speed curve, asks for. */
  const framePlan = useRef<FramePlan | null>(null);
  const speedPlan = useRef<SpeedPlan | null>(null);

  const names = settingNames(labels);
  // As the Video tab writes them: the place in pixels of the timeline, the turn in degrees, the rest but the zoom in percent.
  // And as the Title tab writes those of a title: its size and the place of its anchor in pixels of the timeline.
  const percent: CurveDisplay = { factor: 100, unit: '%', digits: 1 };
  const displays: Record<ClipKeyed, CurveDisplay> = {
    'transform.scale': { factor: 1, unit: '', digits: 3 },
    'transform.x': { factor: timeline.width, unit: 'px', digits: 1 },
    'transform.y': { factor: timeline.height, unit: 'px', digits: 1 },
    'transform.rotation': { factor: 1, unit: '°', digits: 1 },
    'transform.opacity': percent, 'crop.left': percent, 'crop.right': percent, 'crop.top': percent, 'crop.bottom': percent, 'crop.softness': percent,
    'title.size': { factor: 1, unit: 'px', digits: 1 },
    'title.x': { factor: timeline.width, unit: 'px', digits: 1 },
    'title.y': { factor: timeline.height, unit: 'px', digits: 1 },
    // The outline, the shadow and the box behind a title are in pixels of the timeline as they are kept.
    'title.stroke.width': { factor: 1, unit: 'px', digits: 1 },
    'title.shadow.x': { factor: 1, unit: 'px', digits: 1 }, 'title.shadow.y': { factor: 1, unit: 'px', digits: 1 }, 'title.shadow.blur': { factor: 1, unit: 'px', digits: 1 },
    'title.background.padding': { factor: 1, unit: 'px', digits: 1 }, 'title.background.radius': { factor: 1, unit: 'px', digits: 1 },
  };
  const settings: Curve[] = settingsOf(clip).map(path => ({
    id: path, label: names[path], color: COLORS[path], param: paramOf(clip, path), bounds: CLIP_BOUNDS[path], display: displays[path],
  }));
  const found = mediaOf(clip, media);
  const clock = keyClockOf(clip, timeline.rate, found);
  // The frames of keys that the clip shows: with speed points, from the lowest picture of their path to the highest.
  const range = shownKeyRange(clip, clock);
  const source = clip.source;
  const name = source.kind === 'media' ? found?.name ?? source.media : source.kind === 'title' ? source.text : words.label.studio.timeline.color;

  // --- The Retime curves: of a clip of media that lasts, over the frames of the clip.
  const begins = source.kind === 'media' ? source.in : null;
  const course = begins !== null && found !== undefined && found.facts.video?.still !== true ? clipCourse(clip) : null;
  const rate = timeline.rate;
  const setPoints = (retime: Retime, keep: PathEnd): Change => ({ edit: 'set_speed_points', timeline: timeline.id, clip: clip.id, retime, keep });
  const retimeCurves = (): Curve[] => {
    if (!course || !begins || !found) return [];
    const count = course.segments.length;
    const length = clip.length;
    const frameKeys = (along: Course, at: bigint): Keyframe[] => frameCurveIn(along, at, rate, begins.scale)
      .map((point): Keyframe => ({ frame: point.frame, value: point.source, interp: 'linear' }));
    const speedKeys: Keyframe[] = course.speedCurve().map((step): Keyframe => ({ frame: step.from, value: step.percent, interp: 'hold' }));
    // The speed points as the clip has them. Every move of a drag is worked out from them, and from the course of the clip, as they are at the press:
    // the editor keeps the curve that it was given then, and so these, until the drag ends.
    const was = clip.retime ?? constantRetime(clip.speed, clip.reverse);

    const dragFrame = (_initial: readonly Keyframe[], index: number, frames: number, value: number): LineDrag | null => {
      let retime = was;
      let along = course;
      // A speed point moves in time strictly between the one before it and the end of its segment; the first point and the end of the clip stay at their frames.
      if (index > 0 && index < count) {
        const at = course.segments[index].from;
        const to = Math.max(course.segments[index - 1].from + 1, Math.min(at + Math.round(frames), course.segments[index].to - 1));
        if (to !== at) {
          const moved = movedPoint(course, index, to);
          const made = moved ? courseOf(moved, length) : null;
          if (!moved || !made) return null;
          retime = moved;
          along = made;
        }
      }
      if (value !== 0) {
        const raised = raisedPoint(along, index, value);
        const made = raised ? courseOf(raised.retime, length) : null;
        if (!raised || !made) return null;
        retime = raised.retime;
        along = made;
      }
      // The first point takes the beginning of the path with it, and what follows its segment stays; any other leaves the path beginning where it did.
      const keep: PathEnd = index === 0 ? 'end' : 'start';
      const first = BigInt(begins.ticks) + beginsLater(course, along, keep, rate, begins.scale);
      // What the media has not is not asked for: the engine would refuse it.
      const duration = found.facts.duration;
      if (duration.scale === begins.scale && !insideMedia(along, first, BigInt(duration.ticks), rate, begins.scale)) return null;
      framePlan.current = { retime, keep, was };
      const beside = [index - 1, index].filter(each => each >= 0 && each < count);
      return { keys: frameKeys(along, first), tip: beside.map(each => percentText(percentOf(retime.points[each].speed))).join(' · ') };
    };

    const dragSpeed = (initial: readonly Keyframe[], index: number, _frames: number, value: number): LineDrag | null => {
      const segment = course.segments[index];
      const had = segment ? percentOf(segment.speed) : 0;
      // A freeze shows nothing that could last longer or shorter: its step does not move.
      if (!segment || had === 0) return null;
      // The segment goes on the way that it went, at a whole hundredth that a segment may play at; not raised or lowered, it keeps the speed that it has.
      const size = value === 0 ? Math.abs(had) : Math.min(FASTEST, Math.max(SLOWEST, Math.round(Math.abs(had) + (had < 0 ? -value : value))));
      speedPlan.current = { at: segment.from, percent: size, was: Math.abs(had) };
      const level = had < 0 ? -size : size;
      return { keys: initial.map((key, at) => (at === index ? { ...key, value: level } : { ...key })), tip: percentText(level) };
    };

    const add = (at: number) => {
      // A speed point is put at a frame of the clip after its first, where none is.
      if (locked || at <= 0 || at >= length || course.segments.some(segment => segment.from === at)) return;
      void commit((now): Change[] => {
        const there = clipLocations(now).get(clip.id);
        return !there || there.track.locked ? [] : [{ edit: 'add_speed_point', timeline: now.id, clip: clip.id, at }];
      });
    };
    const remove = (frames: readonly number[]) => {
      // The first point is at frame 0, where every clip has one, and the last is the end of the clip: neither is a speed point to clear.
      const points = frames.filter(at => at > 0 && course.segments.some(segment => segment.from === at));
      if (locked || points.length === 0) return;
      void commit((now): Change[] => {
        const there = clipLocations(now).get(clip.id);
        return !there || there.track.locked ? [] : points.map((at): Change => ({ edit: 'clear_speed_point', timeline: now.id, clip: clip.id, at }));
      });
    };

    // The frames of the clip, from its first: the playhead is kept within the clip, as it is for the keys.
    const own: CurveClock = {
      id: 'clip', playhead: Math.min(Math.max(Math.floor(frame) - clip.start, 0), Math.max(length, 1) - 1), range: [0, length],
      onSeek: at => onSeek(clip.start + Math.min(Math.max(at, 0), Math.max(length, 1) - 1), false),
    };
    const shown = clip.retime !== undefined;
    return [
      {
        id: RETIME_FRAME, label: t('Retime Frame'), color: RETIME_COLORS.frame, param: { keys: frameKeys(course, BigInt(begins.ticks)) }, bounds: ANY,
        display: { factor: 1, unit: '', digits: 1 }, clock: own, line: { shown, drag: dragFrame, add, remove },
      },
      {
        id: RETIME_SPEED, label: t('Retime Speed'), color: RETIME_COLORS.speed, param: { keys: speedKeys }, bounds: ANY,
        display: { factor: 1, unit: '%', digits: 0 }, clock: own, line: { shown, drag: dragSpeed, add, remove },
      },
    ];
  };
  const curves: Curve[] = [...settings, ...retimeCurves()];

  const setKeys = (path: ClipKeyed, param: Param): Change => ({ op: 'set_clip_keys', timeline: timeline.id, id: clip.id, path, param });
  const keyed = (changes: CurveChange[]) => changes.flatMap(change => (isKeyed(change.id) ? [{ path: change.id, param: change.param }] : []));
  const preview = (changes: CurveChange[]) => {
    if (locked) return;
    const now = keyed(changes);
    // A move of a control point of the Retime Frame curve sets the speed points whole. One of a step of the Retime Speed curve sends nothing until it is let go.
    const plan = changes.some(change => change.id === RETIME_FRAME) ? framePlan.current : null;
    const ops: Change[] = [...now.map(({ path, param }) => setKeys(path, param)), ...(plan ? [setPoints(plan.retime, plan.keep)] : [])];
    if (ops.length === 0) return;
    if (!dragging.current) {
      // What the gesture puts back where it is taken back: the settings as they are before its first move, and the speed points that the clip had,
      // which put it back where it began in its media too.
      if (!gesture.start([...now.map(({ path }) => setKeys(path, paramOf(clip, path))), ...(plan ? [setPoints(plan.was, plan.keep)] : [])])) return;
      dragging.current = true;
    }
    gesture.preview(ops);
  };
  const commitChanges = (changes: CurveChange[]) => {
    if (locked) return;
    // The end of a drag: its last move is what stays.
    if (dragging.current) { dragging.current = false; gesture.end(); return; }
    // A step of the Retime Speed curve let go: its segment is given the speed that the graph said, where that is another than it has.
    const speed = changes.some(change => change.id === RETIME_SPEED) ? speedPlan.current : null;
    speedPlan.current = null;
    if (speed && speed.percent !== speed.was) {
      void commit((now): Change[] => {
        const there = clipLocations(now).get(clip.id);
        return !there || there.track.locked ? [] : [{ edit: 'set_segment_speed', timeline: now.id, clip: clip.id, at: speed.at, speed: speedOfPercent(speed.percent) }];
      });
    }
    const wanted = keyed(changes);
    if (wanted.length === 0) return;
    void commit((now) => {
      const there = clipLocations(now).get(clip.id);
      if (!there || there.track.locked) return [];
      return wanted.flatMap(({ path, param }) => keyOps(now.id, there.clip, path, param));
    });
  };
  const cancel = () => {
    speedPlan.current = null;
    if (!dragging.current) return;
    dragging.current = false;
    gesture.cancel();
  };

  return <section className="curve-panel" aria-label={words.label.studio.curves.title} data-test="clip-curves" data-clip={clip.id}>
    {refusal && <div className="inspector-problem" role="alert">
      <p>{refusal.text}</p>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={14} /></button>
    </div>}
    {locked && <p className="caption" data-test="curves-locked">{words.hint.studio.inspector.locked}</p>}
    <CurveEditor curves={curves} playhead={keyFrameOf(clip, timeline, media, frame)} range={range} selection={selection} onSelect={setSelection}
      disabled={locked} title={`${words.label.studio.curves.title} · ${name}`} tools={close}
      onPreview={preview} onCommit={commitChanges} onCancel={cancel} onSeek={keyFrame => onSeek(frameOfKey(clip, timeline, media, keyFrame), false)} />
  </section>;
}
