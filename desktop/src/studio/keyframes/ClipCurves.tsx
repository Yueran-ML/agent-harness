import { useRef, useState, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import { CLIP_KEYED, type Clip, type ClipKeyed, type Media, type Param, type Timeline } from '../engine';
import { CLIP_BOUNDS, CLIP_SETTINGS, frameOfKey, keyFrameOf, keyOps, mediaOf, paramOf, settingNames } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { useCommit } from '../inspector/useCommit';
import type { Change } from '../timeline/commands';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { CurveEditor, NO_SELECTION, type Curve, type CurveChange, type CurveDisplay, type CurveSelection } from './CurveEditor';

/** The settings of a clip that take keys, in the order of the Video tab of the Inspector: the keyframe track of the timeline lists them so too. */
export const CLIP_CURVES: readonly ClipKeyed[] = CLIP_SETTINGS;
/** The colour of the curve of each setting, as each track of DaVinci Resolve's Keyframe Editor has one (Reference Manual 20, p. 1113). */
const COLORS: Record<ClipKeyed, string> = {
  'transform.scale': '#e9b949', 'transform.x': '#e5534b', 'transform.y': '#57ab5a', 'transform.rotation': '#539bf5', 'transform.opacity': '#c9d1d9',
  'crop.left': '#b083f0', 'crop.right': '#f69d50', 'crop.top': '#39c5cf', 'crop.bottom': '#e275ad', 'crop.softness': '#8ddb8c',
};
const isKeyed = (path: string): path is ClipKeyed => (CLIP_KEYED as readonly string[]).includes(path);

/**
 * The Curve Editor of the Edit page (DaVinci Resolve Reference Manual 20, pp. 1116–1119), beneath
 * the timeline: the settings of the transform and of the crop of the one clip of picture that is
 * chosen, as curves over the frames that their keys count. One clip of picture with its linked
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
  const labels = words.label.studio.inspector;
  const [selection, setSelection] = useState<CurveSelection>(NO_SELECTION);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${clip.id}/curves`, locked,
    onFailure: error => setRefusal(refusalOf(error, words.hint.studio)),
  });
  /** Whether a drag has begun a gesture that is still to be ended or taken back. */
  const dragging = useRef(false);

  const names = settingNames(labels);
  // As the Video tab writes them: the place in pixels of the timeline, the turn in degrees, the rest but the zoom in percent.
  const percent: CurveDisplay = { factor: 100, unit: '%', digits: 1 };
  const displays: Record<ClipKeyed, CurveDisplay> = {
    'transform.scale': { factor: 1, unit: '', digits: 3 },
    'transform.x': { factor: timeline.width, unit: 'px', digits: 1 },
    'transform.y': { factor: timeline.height, unit: 'px', digits: 1 },
    'transform.rotation': { factor: 1, unit: '°', digits: 1 },
    'transform.opacity': percent, 'crop.left': percent, 'crop.right': percent, 'crop.top': percent, 'crop.bottom': percent, 'crop.softness': percent,
  };
  const curves: Curve[] = CLIP_CURVES.map(path => ({
    id: path, label: names[path], color: COLORS[path], param: paramOf(clip, path), bounds: CLIP_BOUNDS[path], display: displays[path],
  }));
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  // The frames of keys that the clip shows: with speed points, from the lowest picture of their path to the highest.
  const range = shownKeyRange(clip, clock);
  const source = clip.source;
  const name = source.kind === 'media' ? mediaOf(clip, media)?.name ?? source.media : source.kind === 'title' ? source.text : words.label.studio.timeline.color;

  const setKeys = (path: ClipKeyed, param: Param): Change => ({ op: 'set_clip_keys', timeline: timeline.id, id: clip.id, path, param });
  const keyed = (changes: CurveChange[]) => changes.flatMap(change => (isKeyed(change.id) ? [{ path: change.id, param: change.param }] : []));
  const preview = (changes: CurveChange[]) => {
    if (locked) return;
    const now = keyed(changes);
    if (!dragging.current) {
      // What the gesture puts back where it is taken back: the settings as they are before its first move.
      if (!gesture.start(now.map(({ path }) => setKeys(path, paramOf(clip, path))))) return;
      dragging.current = true;
    }
    gesture.preview(now.map(({ path, param }) => setKeys(path, param)));
  };
  const commitChanges = (changes: CurveChange[]) => {
    if (locked) return;
    // The end of a drag: its last move is what stays.
    if (dragging.current) { dragging.current = false; gesture.end(); return; }
    const wanted = keyed(changes);
    void commit((now) => {
      const found = clipLocations(now).get(clip.id);
      if (!found || found.track.locked) return [];
      return wanted.flatMap(({ path, param }) => keyOps(now.id, found.clip, path, param));
    });
  };
  const cancel = () => {
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
