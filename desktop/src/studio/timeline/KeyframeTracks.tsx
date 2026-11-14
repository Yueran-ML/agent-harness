import { memo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { keyClockOf } from '../color/keyFrames';
import type { Clip, ClipKeyed, Keyframe, Media, Param, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { hasKeys, keyFrameOf, keyOps, mediaOf, settingNames } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { useCommit } from '../inspector/useCommit';
import { timecode } from '../timecode';
import type { Change } from './commands';
import { HEADER_WIDTH } from './geometry';
import {
  KEY_ROW, KEY_TRACK_RISE, chosenKeys, copyChosen, deleteChosen, keyId, keyRows, keyTrackHeight, moveChosen, pasteCopied,
  type ChosenKeys, type KeyTrackClipboard, type SettingChange, type TrackKey,
} from './keyTrack';
import { clipLocations } from './selection';
import { useAudioGesture } from './useAudioGesture';

/** The row that a diamond is on: that of a setting, or the group row, whose diamonds stand for every key at their frame. */
type Row = ClipKeyed | 'group';
const GROUP = 'group';
/** How far a pointer goes before a press is a drag. */
const SLOP_PX = 3;
/** How near the ends of its track a diamond is drawn at most, so that one at the first or last frame is whole. */
const EDGE_PX = 5;
const NOTHING: ReadonlySet<string> = new Set();

/** A drag of the keys chosen: what it began with, and what its last move made of them. */
interface Drag {
  pointer: number;
  x: number;
  /** The key that was pressed, and the frame of the timeline that it was drawn at. */
  key: TrackKey;
  at: number;
  /** The settings that have keys chosen, as they were at the press: every move is worked out from these. */
  settings: ChosenKeys[];
  moved: boolean;
  /** Whether a move has begun the gesture, which is then to be ended or taken back. */
  started: boolean;
  /** How many frames of the keys the last move took them, or null before the first. */
  shift: number | null;
  /** The keys chosen, where the last move has them. */
  chosen: ReadonlySet<string>;
  /** A press on keys that were chosen with others keeps them all for a drag; let go where it was, it chooses these alone. */
  click: string[] | null;
}
/** The keys as a drag has them for now: drawn so whatever the timeline is given meanwhile. */
interface Live { keys: ReadonlyMap<ClipKeyed, readonly Keyframe[]>; chosen: ReadonlySet<string>; tip: string }

/**
 * The keyframe tracks that are open on a timeline, each beneath its clip in the room that its track
 * makes for it (`keyTrackHeights`). Mounted only while one is open, and drawn again only as the
 * timeline, the zoom or the heights of the tracks change, not as the playhead moves: it is asked
 * for the playhead where an edit needs it.
 */
export const KeyframeTracks = memo(function KeyframeTracks({ bridge, session, revision, timeline, media, open, rows, scale, playhead, clipboard }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; media: readonly Media[];
  /** The clips whose keyframe tracks are open. */
  open: readonly string[];
  /** Where each track that is shown lies, and how high its clips are. */
  rows: ReadonlyMap<string, { top: number; height: number }>;
  /** The pixels of a frame. */
  scale: number;
  /** The playhead as it is shown, in frames of the timeline. It must stay the same call. */
  playhead: () => number;
  clipboard: KeyTrackClipboard;
}) {
  const locations = clipLocations(timeline);
  return <>{open.map((id) => {
    const found = locations.get(id);
    const row = found ? rows.get(found.track.id) : undefined;
    if (!found || !row || found.track.kind !== 'video' || !hasKeys(found.clip)) return null;
    return <KeyframeTrack key={`${session}/${timeline.id}/${id}`} bridge={bridge} session={session} revision={revision} timeline={timeline} media={media}
      clip={found.clip} locked={found.track.locked} top={row.top + row.height - KEY_TRACK_RISE} scale={scale} playhead={playhead} clipboard={clipboard} />;
  })}</>;
});

/**
 * The keyframe track of one clip, as the Keyframe Editor that DaVinci Resolve opens beneath a clip
 * in the timeline (Reference Manual 20, pp. 1109–1111): as wide as the clip, a group row whose
 * diamonds stand for every key at their frame, and beneath it a row for each setting of the
 * transform and of the crop that has keys, each key a diamond at the frame of the timeline at which
 * the clip shows its picture.
 *
 * What a pointer and the keyboard do:
 * - a click on a diamond chooses its key, or with the group row every key at that frame; Ctrl or
 *   Cmd adds it or takes it out; Shift adds it, and with it the keys of its row from the diamond
 *   clicked before to it (p. 1110); a click where nothing is lets go of them;
 * - a drag of a key chosen moves all that are chosen in time, by whole frames of the keys, no key
 *   past one beside it that is not moved (`moveChosen`); the track says the frame of the key
 *   pressed, counted as the keys count, and how far it has gone, and with one key the name of its
 *   setting (p. 1110). The key pressed stays within the clip;
 * - Delete or Backspace takes the keys chosen away (p. 1114); Ctrl+C, Ctrl+X and Ctrl+V copy, cut
 *   and paste them, the first pasted at the playhead and the rest as far after it as they were
 *   (p. 1114), where the playhead is on the clip; Escape takes a drag back, or lets go of what is
 *   chosen.
 * Those keys are the track's whether or not it has anything to do with them, with or without the
 * other modifiers of Delete: the timeline would delete or copy the clip with them. Every other key
 * goes on, so Undo and the transport work as everywhere.
 *
 * A drag is one gesture (`useAudioGesture`, as a drag in the Curve Editor of the page): each move
 * sets the settings whole with `set_clip_keys`, which is the same whatever went before it, the last
 * move ends the gesture, and the whole is one step to undo; taken back, the settings are set to
 * what they were. Every other edit is one transaction (`useCommit`) of what `keyOps` gives against
 * the timeline as it is when it goes: `splice_clip_keys` of the keys that differ, or
 * `set_clip_keys` with a number where the last key of a setting is taken away.
 *
 * The track keeps nothing of the keys but which are chosen (`keyId`), so whatever changes the clip
 * (an edit from elsewhere, Undo) is drawn as it comes. A clip on a locked track is shown, and its
 * keys can be chosen and copied, not changed.
 *
 * Left for later (all in DaVinci Resolve's Keyframe Editor): a track for each group of settings
 * with its disclosure button (Transform, Composite, Cropping; p. 1109), where there is one group
 * row here; Alt-click to add a key and Alt-drag to duplicate (p. 1110); a box drawn around keys;
 * the menu of interpolation on a right click (p. 1110); Ctrl+← and Ctrl+→ (p. 1111); the Curve
 * Editor opened inside the track (p. 1108: it would be `CurveEditor` laid over these rows, given
 * the settings of this clip as `ClipCurves` gives them); and the keys of a clip of sound.
 */
function KeyframeTrack({ bridge, session, revision, timeline, media, clip, locked, top, scale, playhead, clipboard }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; media: readonly Media[];
  clip: Clip; locked: boolean; top: number; scale: number; playhead: () => number; clipboard: KeyTrackClipboard;
}) {
  const words = useWords();
  const t = useFairlightText();
  const root = useRef<HTMLDivElement>(null);
  const [chosen, setChosen] = useState<ReadonlySet<string>>(NOTHING);
  const [live, setLive] = useState<Live | null>(null);
  const drag = useRef<Drag | null>(null);
  /** The diamond clicked last without Shift: where a Shift-click chooses from. */
  const anchor = useRef<{ at: number; row: Row } | null>(null);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${clip.id}/keys`, locked,
    onFailure: error => setRefusal(refusalOf(error, words.hint.studio)),
  });

  const names = settingNames(words.label.studio.inspector);
  const rows = keyRows(clip, timeline, media, live?.keys);
  const keys = rows.flatMap(row => row.keys);
  const picked = live?.chosen ?? chosen;
  const isChosen = (key: TrackKey) => picked.has(keyId(key.path, key.frame));
  const frames = [...new Set(keys.map(key => key.at))].sort((a, b) => a - b);
  const width = Math.max(1, clip.length * scale);
  const xOf = (at: number) => Math.max(EDGE_PX, Math.min((at - clip.start) * scale, width - EDGE_PX));
  const setKeys = (path: ClipKeyed, param: Param): Change => ({ op: 'set_clip_keys', timeline: timeline.id, id: clip.id, path, param });

  const down = (event: PointerEvent<HTMLDivElement>) => {
    // Every press on the track is the track's: the timeline would choose or drag a clip with it.
    event.stopPropagation();
    if (event.button !== 0 || drag.current) return;
    event.preventDefault();
    root.current?.focus({ preventScroll: true });
    const target = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-key-at]') : null;
    if (!target) { anchor.current = null; setChosen(NOTHING); return; }
    const row = target.dataset.keyRow as Row;
    const at = Number(target.dataset.keyAt);
    const frame = Number(target.dataset.keyFrame);
    const hit = row === GROUP ? keys.filter(key => key.at === at) : keys.filter(key => key.path === row && key.frame === frame);
    if (hit.length === 0) return;
    const ids = hit.map(key => keyId(key.path, key.frame));
    const all = ids.every(id => chosen.has(id));
    const toggle = event.ctrlKey || event.metaKey;
    let next: ReadonlySet<string>;
    if (event.shiftKey) {
      const from = anchor.current;
      const between = from ? keys.filter(key => (row === GROUP || key.path === row) && key.at >= Math.min(from.at, at) && key.at <= Math.max(from.at, at)) : [];
      next = new Set([...chosen, ...between.map(key => keyId(key.path, key.frame)), ...ids]);
    } else if (toggle) next = new Set(all ? [...chosen].filter(id => !ids.includes(id)) : [...chosen, ...ids]);
    else next = all ? chosen : new Set(ids);
    if (!event.shiftKey) anchor.current = { at, row };
    setChosen(next);
    // Keys taken out of what is chosen are not dragged, nor any of a clip that may not be changed.
    if (locked || !ids.every(id => next.has(id))) return;
    drag.current = {
      pointer: event.pointerId, x: event.clientX, key: hit[0], at, settings: chosenKeys(clip, next), moved: false, started: false, shift: null, chosen: next,
      click: all && !toggle && !event.shiftKey ? ids : null,
    };
    try { event.currentTarget.setPointerCapture?.(event.pointerId); } catch { /* A pointer that is no longer down has nothing to keep. */ }
  };

  const move = (event: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.pointer !== event.pointerId) return;
    event.stopPropagation();
    // The gesture was ended for it (the clip was changed from elsewhere, or the engine refused a move): the drag is over.
    if (d.started && !gesture.active()) { drag.current = null; setLive(null); return; }
    const dx = event.clientX - d.x;
    if (!d.moved && Math.abs(dx) < SLOP_PX) return;
    d.moved = true;
    // The frame of the timeline that the key pressed is dragged to, kept within the clip, and the frames of the keys from where it was to there.
    const to = Math.max(clip.start, Math.min(d.at + Math.round(dx / scale), clip.start + Math.max(clip.length, 1) - 1));
    const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
    const moved = moveChosen(d.settings, clock(to) - clock(d.at));
    // A move that leaves the keys where the move before left them says nothing again; nor does a first one that moves none.
    if (moved.shift === d.shift || (!d.started && moved.shift === 0)) return;
    if (!d.started) {
      // What the gesture puts back where it is taken back: the settings as they were at the press.
      if (!gesture.start(d.settings.map(each => setKeys(each.path, { keys: each.keys })))) { drag.current = null; return; }
      d.started = true;
    }
    d.shift = moved.shift;
    d.chosen = new Set(moved.settings.flatMap((each, place) => d.settings[place].indices.map(index => keyId(each.path, each.keys[index].frame))));
    const one = d.settings.length === 1 && d.settings[0].indices.length === 1;
    setLive({
      keys: new Map(moved.settings.map((each): [ClipKeyed, Keyframe[]] => [each.path, each.keys])),
      chosen: d.chosen,
      tip: `${one ? `${names[d.key.path]} ` : ''}${d.key.frame + moved.shift} (${moved.shift > 0 ? '+' : ''}${moved.shift})`,
    });
    gesture.preview(moved.settings.map(each => setKeys(each.path, { keys: each.keys })));
  };

  const up = (event: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    event.stopPropagation();
    drag.current = null;
    setLive(null);
    if (!d.moved && d.click) setChosen(new Set(d.click));
    // The end of a drag: its last move is what stays, and the keys are chosen where it left them.
    if (d.started) { setChosen(d.chosen); gesture.end(); }
  };

  /** Take a drag back. True where there was one. */
  const cancel = (): boolean => {
    const d = drag.current;
    if (!d) return false;
    drag.current = null;
    setLive(null);
    if (d.started) gesture.cancel();
    return true;
  };

  // --- The edits of the keyboard, each one transaction.
  const able = () => !locked && !drag.current && !gesture.busy();
  const send = (changes: readonly SettingChange[]) => {
    if (changes.length === 0) return;
    void commit((now) => {
      const found = clipLocations(now).get(clip.id);
      if (!found || found.track.locked) return [];
      return changes.flatMap(({ path, param }) => keyOps(now.id, found.clip, path, param));
    });
  };
  const remove = () => {
    if (!able()) return;
    const changes = deleteChosen(clip, chosen, keyFrameOf(clip, timeline, media, playhead()));
    if (changes.length === 0) return;
    anchor.current = null;
    setChosen(NOTHING);
    send(changes);
  };
  const copy = (): boolean => {
    const copied = copyChosen(clip, chosen);
    if (copied.length === 0) return false;
    clipboard.settings = copied;
    return true;
  };
  const paste = () => {
    const frame = Math.floor(playhead());
    // Pasted at the playhead: where it is not on the clip, there is no frame of the clip to paste at.
    if (!able() || clipboard.settings.length === 0 || frame < clip.start || frame >= clip.start + clip.length) return;
    const made = pasteCopied(clip, clipboard.settings, keyFrameOf(clip, timeline, media, frame));
    anchor.current = null;
    setChosen(new Set(made.chosen));
    send(made.changes);
  };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing) return;
    const key = event.key.toLowerCase();
    const command = (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey;
    const owned = key === 'escape' || key === 'delete' || key === 'backspace' || (command && (key === 'c' || key === 'x' || key === 'v'));
    if (!owned) return;
    // These are the track's whether or not it has anything to do with them: the timeline would take them for the clip.
    event.preventDefault();
    event.stopPropagation();
    if (key === 'escape') { if (!cancel()) { anchor.current = null; setChosen(NOTHING); } return; }
    if (key === 'delete' || key === 'backspace') { remove(); return; }
    if (key === 'c') { copy(); return; }
    if (key === 'x') { if (able() && copy()) remove(); return; }
    paste();
  };

  const said = (at: number) => timecode(at, timeline.rate);
  return <div ref={root} className="key-track" role="group" aria-label={t('Keyframe Editor')} tabIndex={0} data-key-track={clip.id} data-locked={locked || undefined}
    style={{ left: HEADER_WIDTH + clip.start * scale, width, top, height: keyTrackHeight(rows.length) }}
    onKeyDown={keyDown} onPointerDown={down} onPointerMove={move} onPointerUp={up}
    onPointerCancel={(event) => { event.stopPropagation(); cancel(); }} onLostPointerCapture={() => { cancel(); }}
    onDoubleClick={event => event.stopPropagation()}>
    <div className="key-track-row" data-key-track-row={GROUP} style={{ height: KEY_ROW }}>
      <span className="key-track-name">{t('Keyframes')}</span>
      {/* A diamond is the n-th of its row, not the one of its frame: it stays the same element while it is dragged. */}
      {frames.map((at, n) => {
        const here = keys.filter(key => key.at === at);
        return <span key={n} className="key-track-key" data-key-row={GROUP} data-key-at={at} data-selected={here.every(isChosen)}
          title={`${[...new Set(here.map(key => names[key.path]))].join(', ')} · ${said(at)}`} style={{ left: xOf(at) }} />;
      })}
    </div>
    {rows.map(row => <div key={row.path} className="key-track-row" data-key-track-row={row.path} style={{ height: KEY_ROW }}>
      <span className="key-track-name">{names[row.path]}</span>
      {row.keys.map(key => <span key={key.index} className="key-track-key" data-key-row={row.path} data-key-at={key.at} data-key-frame={key.frame}
        data-selected={isChosen(key)} title={`${names[row.path]} · ${said(key.at)}`} style={{ left: xOf(key.at) }} />)}
    </div>)}
    {live && <span className="key-track-tip" role="status" data-test="key-track-tip">{live.tip}</span>}
    {refusal && <div className="key-track-problem" role="alert">
      <span>{refusal.text}</span>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={12} /></button>
    </div>}
  </div>;
}
