import { memo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent } from 'react';
import { X } from 'lucide-react';
import type { EngineBridge } from '../../bridge';
import { CoveringMenu } from '../color/CoveringMenu';
import type { MenuItem } from '../color/ColorMenu';
import type { Clip, Retime, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { clipCourse, courseOf, movedPoint, percentOf, percentToLast, speedOfPercent, type Course } from '../retime';
import type { Change, Edit, Transaction } from './commands';
import { HEADER_WIDTH } from './geometry';
import { clipLocations, movableClips } from './selection';
import { useAudioGesture } from './useAudioGesture';

/**
 * The speeds that Change Speed of a segment's pop-up menu offers, in hundredths of the speed of
 * the source, as DaVinci Resolve lists them (Reference Manual 20, p. 1076).
 */
export const CHANGE_SPEEDS: readonly number[] = [10, 25, 50, 75, 100, 110, 150, 200, 400, 800];

/** How far a pointer goes before a press on a handle is a drag. */
const SLOP_PX = 3;

/** A speed in hundredths of the speed of the source as the bar writes it: to a tenth, with a minus where the segment plays backward. */
export const percentText = (percent: number): string => `${percent < 0 ? '−' : ''}${Number(Math.abs(percent).toFixed(1))}%`;

/**
 * How far apart the arrows of a segment are drawn, in pixels: nearer each other the faster it
 * plays, as DaVinci Resolve draws them (p. 1074), 16 pixels at the speed of the source and never
 * nearer than 6 nor further than 40. The bars of a freeze, which has no speed, are 16 pixels apart.
 */
export const arrowGap = (percent: number): number => (percent === 0 ? 16 : Math.max(6, Math.min(40, Math.round(1600 / Math.abs(percent)))));

/** A drag of a handle of a speed point: what it began with, and where its last move took it. */
interface Drag {
  pointer: number;
  x: number;
  /** The pixels of a frame as the drag began. */
  scale: number;
  handle: 'upper' | 'lower';
  /** The segment that begins at the handle; the number of segments for the handle at the end of the clip. */
  index: number;
  /** The frame of the clip that the handle was at. */
  at: number;
  /** The course of the clip at the press: every move is worked out from it. */
  course: Course;
  /** The clips linked to it that play along the same speed points, which a move of a speed point takes along. */
  partners: string[];
  moved: boolean;
  /** Whether a move has begun the gesture, which is then to be ended or taken back. */
  started: boolean;
  /** The frame that the last move took the handle to. */
  to: number;
  /** The speed that the last move of an upper handle asks for, in hundredths of the speed of the source and without a sign; null before the first, and for a freeze. */
  percent: number | null;
}

/** A drag as it is drawn for now: where the handle is, what the bar says of it, and the speed points that a move of a speed point has made. */
interface Live { handle: 'upper' | 'lower'; index: number; to: number; tip: string; retime: Retime | null }

type Apply = (transaction: Transaction) => Promise<boolean>;

interface Shared {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline;
  /** The playhead as it is shown, in frames of the timeline. It must stay the same call. */
  playhead: () => number;
  /** Whether what follows a clip moves with its end where a change makes it longer or shorter: the Trim mode (p. 1075). */
  ripple: boolean;
  /** Whether the clips linked to a clip take its changes with it. */
  linked: boolean;
  /** Send one transaction as an edit of the timeline is sent: a refusal is said where the timeline says them. */
  apply: Apply;
  /** Put the controls of clips away. */
  onToggle: (clips: readonly string[]) => void;
  /** Say a refusal of the engine that came to a drag. */
  onFailure: (error: unknown) => void;
}

/**
 * The Retime Controls that are open on a timeline, each laid over its clip. Mounted only while
 * some are open, and drawn again only as the timeline, the heights of the tracks or the tool
 * change: not as the playhead moves (it is asked for where a command needs it), nor at a zoom (the
 * style sheet places everything from frames and the `--scale` of the timeline).
 */
export const RetimeBars = memo(function RetimeBars({ open, rows, ...shared }: Shared & {
  /** The clips whose Retime Controls are open. */
  open: readonly string[];
  /** Where each track that is shown lies, and how high its clips are. */
  rows: ReadonlyMap<string, { top: number; height: number }>;
}) {
  const locations = clipLocations(shared.timeline);
  return <>{open.map((id) => {
    const found = locations.get(id);
    const row = found ? rows.get(found.track.id) : undefined;
    if (!found || !row || found.clip.source.kind !== 'media') return null;
    return <RetimeBar key={`${shared.session}/${shared.timeline.id}/${id}`} {...shared} clip={found.clip} locked={found.track.locked} top={row.top + 4} height={row.height - 8} />;
  })}</>;
});

/**
 * The Retime Controls of one clip, as DaVinci Resolve shows them on a clip of the timeline
 * (Reference Manual 20, pp. 1070–1077; Clip › Retime Controls, Ctrl+R): over the clip, a strip of
 * arrows for each segment, which point the way that it plays and stand nearer each other the
 * faster it plays (none in a freeze, which is barred); under each strip the speed of the segment in
 * percent, a button that opens its pop-up menu; and at each speed point, and at the end of the
 * clip, a handle.
 *
 * The pop-up menu of a segment (p. 1076):
 * - Change Speed ▸ gives the segment one of the speeds of the list (`set_segment_speed`): it shows
 *   what it showed, so it and the clip last longer or shorter;
 * - Add Speed Point puts a speed point at the playhead, and Freeze Frame holds the picture at the
 *   playhead for a second (`add_speed_point`, `freeze_frame`), where the playhead is on the clip;
 * - Clear Speed Point clears the speed point at the left of the segment (`clear_speed_point`);
 * - Reverse Segment turns the segment round (`reverse_segment`);
 * - Reset Speed puts the segment back to the speed of the source, the way that it goes, and Reset
 *   Clip the whole clip, forward and with no speed points (`reset_speed`).
 * Each is one edit, sent as the edits of the timeline are, with `at` a frame of the clip counted
 * from its first; a refusal of the engine is said where the timeline says them.
 *
 * The handles (p. 1075):
 * - the upper handle of a speed point, dragged, changes the speed of the segment at its left so
 *   that the segment ends where the handle is let go, and what follows comes sooner or later: one
 *   `set_segment_speed` as it is let go, the bar saying the speed meanwhile. The handle at the end
 *   of the clip does so for the last segment. A freeze shows nothing that could last longer, and
 *   its handle does not move;
 * - the lower handle moves the speed point, the clip lasting as long as it did, so the speeds
 *   either side change (`movedPoint`): one gesture (`useAudioGesture`) of `set_retime`, which sets
 *   the speed points whole and is the same whatever went before, so the picture follows the drag
 *   and the whole is one step to undo. Escape takes it back.
 * A speed that a drag makes is a whole hundredth of the speed of the source, which the engine
 * takes beside any other speeds of the clip. With `ripple` (the Trim tool) what follows the clip
 * moves with its end; with `linked` the clips linked to it take the change.
 *
 * A clip on a locked track shows its controls, and none of them changes it.
 *
 * Left for later (all in DaVinci Resolve's Retime Controls): the Retime Speed and Retime Frame
 * curves (pp. 1078–1081), Rewind and Speed Ramp of the pop-up menu (p. 1077), the Retime Process
 * and Motion Estimation of the Inspector (p. 1082), and typing a speed into the bar.
 */
function RetimeBar({ bridge, session, revision, timeline, clip, locked, top, height, playhead, ripple, linked, apply, onToggle, onFailure }: Shared & {
  clip: Clip; locked: boolean; top: number; height: number;
}) {
  const t = useFairlightText();
  const root = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [live, setLive] = useState<Live | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; segment: number } | null>(null);
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${timeline.id}/${clip.id}/retime`, locked, onFailure });
  const own = clipCourse(clip);
  if (!own) return null;
  // The speed points as a drag of a speed point has them for now, drawn so whatever the timeline is given meanwhile.
  const course = (live?.retime ? courseOf(live.retime, clip.length) : null) ?? own;
  const segments = course.segments;

  const send = (edit: Edit) => { void apply({ revision, ops: [edit] }); };
  const setRetime = (ids: readonly string[], retime: Retime | null): Change[] => ids.map((id): Change => ({ op: 'set_retime', timeline: timeline.id, id, retime }));

  const down = (event: PointerEvent<HTMLElement>, handle: 'upper' | 'lower', index: number) => {
    // A press on a handle is the handle's: the timeline would choose or drag a clip with it.
    event.stopPropagation();
    // A freeze shows nothing that could last longer or shorter: the handle at its end does not move.
    if (event.button !== 0 || drag.current || locked || (handle === 'upper' && own.segments[index - 1].step === 0n)) return;
    event.preventDefault();
    root.current?.focus({ preventScroll: true });
    const at = index < own.segments.length ? own.segments[index].from : clip.length;
    // The clips linked to this one that play along the very same speed points go with a speed point that is moved.
    const same = JSON.stringify(clip.retime ?? null);
    const locations = clipLocations(timeline);
    const partners = handle === 'lower' && linked ? movableClips(timeline, [clip.id], true).movable.filter((id) => {
      const other = locations.get(id)?.clip;
      return other !== undefined && id !== clip.id && other.start === clip.start && other.length === clip.length && JSON.stringify(other.retime ?? null) === same;
    }) : [];
    const scale = Number(event.currentTarget.closest('.timeline')?.getAttribute('data-scale')) || 1;
    drag.current = { pointer: event.pointerId, x: event.clientX, scale, handle, index, at, course: own, partners, moved: false, started: false, to: at, percent: null };
    try { event.currentTarget.setPointerCapture?.(event.pointerId); } catch { /* A pointer that is no longer down has nothing to keep. */ }
  };

  const move = (event: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || d.pointer !== event.pointerId) return;
    event.stopPropagation();
    // The gesture was ended for it (the clip was changed from elsewhere, or the engine refused a move): the drag is over.
    if (d.started && !gesture.active()) { drag.current = null; setLive(null); return; }
    const dx = event.clientX - d.x;
    if (!d.moved && Math.abs(dx) < SLOP_PX) return;
    d.moved = true;
    const wanted = d.at + Math.round(dx / d.scale);
    const before = d.course.segments[d.index - 1];
    if (d.handle === 'upper') {
      // The segment at the left ends where the handle is: it lasts one frame at the least.
      const to = Math.max(before.from + 1, wanted);
      const percent = percentToLast(before, to - before.from, d.course.unit);
      d.to = to;
      d.percent = percent;
      setLive({ handle: 'upper', index: d.index, to, retime: null, tip: percent === null ? percentText(0) : percentText(before.speed.num < 0 ? -percent : percent) });
      return;
    }
    // The speed point stays strictly between the one before it and the end of its segment.
    const to = Math.max(before.from + 1, Math.min(wanted, d.course.segments[d.index].to - 1));
    if (to === d.to) return;
    // Back where it was, the speed points are those that the clip had, to the fraction.
    const retime = to === d.at ? clip.retime ?? null : movedPoint(d.course, d.index, to);
    if (!retime) return;
    if (!d.started) {
      // What the gesture puts back where it is taken back: the speed points as they were at the press.
      if (!gesture.start(setRetime([clip.id, ...d.partners], clip.retime ?? null))) { drag.current = null; return; }
      d.started = true;
    }
    d.to = to;
    const speeds = [retime.points[d.index - 1].speed, retime.points[d.index].speed];
    setLive({ handle: 'lower', index: d.index, to, retime, tip: speeds.map(speed => percentText(percentOf(speed))).join(' · ') });
    gesture.preview(setRetime([clip.id, ...d.partners], retime));
  };

  const up = (event: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d) return;
    event.stopPropagation();
    drag.current = null;
    setLive(null);
    // A speed point let go where it was is no change: the gesture is taken back, and nothing is left to undo.
    if (d.handle === 'lower') { if (d.started && d.to === d.at) gesture.cancel(); else if (d.started) gesture.end(); return; }
    // The end of a drag of an upper handle: the segment at its left is given the speed that the bar said, where that is another than it has.
    const before = d.course.segments[d.index - 1];
    if (!d.moved || d.percent === null || d.to === d.at || d.percent === Math.abs(percentOf(before.speed))) return;
    send({ edit: 'set_segment_speed', timeline: timeline.id, clip: clip.id, at: before.from, speed: speedOfPercent(d.percent), ripple, linked });
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

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape' || !cancel()) return;
    event.preventDefault();
    event.stopPropagation();
  };

  const openMenu = (event: MouseEvent<HTMLElement>, segment: number) => {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ x: event.clientX, y: event.clientY, segment });
  };

  const items = (index: number): MenuItem[] => {
    const segment = own.segments[index];
    const percent = percentOf(segment.speed);
    // The frame of the clip under the playhead, where it is on the clip: a playhead between two frames is in the earlier.
    const at = Math.floor(playhead()) - clip.start;
    const on = at >= 0 && at < clip.length;
    const busy = locked || gesture.busy();
    const speed = (to: number) => send({ edit: 'set_segment_speed', timeline: timeline.id, clip: clip.id, at: segment.from, speed: speedOfPercent(to), ripple, linked });
    const plain = !clip.retime && !clip.reverse && clip.speed.num === clip.speed.den;
    return [
      { id: 'change-speed', label: t('Change Speed'), disabled: busy, items: CHANGE_SPEEDS.map(each => ({
        id: `speed-${each}`, label: `${each}%`, checked: Math.abs(percent) === each, disabled: busy, run: () => speed(each),
      })) },
      { id: 'add-speed-point', label: t('Add Speed Point'), disabled: busy || !on || own.segments.some(each => each.from === at),
        run: () => send({ edit: 'add_speed_point', timeline: timeline.id, clip: clip.id, at, linked }) },
      { id: 'clear-speed-point', label: t('Clear Speed Point'), disabled: busy || index === 0,
        run: () => send({ edit: 'clear_speed_point', timeline: timeline.id, clip: clip.id, at: segment.from, linked }) },
      { id: 'freeze-frame', label: t('Freeze Frame'), disabled: busy || !on,
        run: () => send({ edit: 'freeze_frame', timeline: timeline.id, clip: clip.id, at, hold: Math.max(1, Math.round(timeline.rate.num / timeline.rate.den)), ripple, linked }) },
      { id: 'reverse-segment', label: t('Reverse Segment'), disabled: busy || percent === 0,
        run: () => send({ edit: 'reverse_segment', timeline: timeline.id, clip: clip.id, at: segment.from, linked }) },
      { id: 'reset-speed', label: t('Reset Speed'), disabled: busy || Math.abs(percent) === 100, run: () => speed(100) },
      { id: 'reset-clip', label: t('Reset Clip'), disabled: busy || plain,
        run: () => send({ edit: 'reset_speed', timeline: timeline.id, clip: clip.id, ripple, linked }) },
    ];
  };

  const label = t('Retime Controls');
  const handleAt = (index: number, frame: number) => (live && live.index === index && live.handle === 'upper' ? live.to : frame);
  // Every press on a button of the bar stays with it: the timeline would take it for a press on the lane.
  const quiet = (event: PointerEvent<HTMLElement>) => event.stopPropagation();
  return <div ref={root} className="retime-bar" role="group" aria-label={label} tabIndex={-1} data-retime={clip.id} data-locked={locked || undefined}
    style={{ '--header': HEADER_WIDTH, '--start': clip.start, '--length': clip.length, top, height } as CSSProperties} onKeyDown={keyDown}>
    {segments.map((segment, index) => {
      const percent = percentOf(segment.speed);
      const said = percentText(percent);
      return <div key={index} className="retime-segment" data-retime-segment={index} data-percent={percent}
        data-way={percent === 0 ? 'freeze' : percent < 0 ? 'backward' : 'forward'} data-changed={Math.abs(percent) !== 100 || undefined}
        style={{ '--from': segment.from, '--frames': segment.to - segment.from, '--gap': arrowGap(percent) } as CSSProperties}>
        <span className="retime-arrows" aria-hidden="true" />
        <button type="button" className="retime-speed" data-test="retime-speed" aria-haspopup="menu" aria-label={`${t('Change Speed')} ${said}`} title={t('Change Speed')}
          onPointerDown={quiet} onDoubleClick={event => event.stopPropagation()} onClick={event => openMenu(event, index)} onContextMenu={event => openMenu(event, index)}>
          {said}<span aria-hidden="true"> ▾</span>
        </button>
      </div>;
    })}
    {/* A handle at each speed point, and one at the end of the clip for its last segment. */}
    {[...segments.slice(1).map(segment => segment.from), clip.length].map((frame, place) => {
      const index = place + 1;
      const point = index < segments.length;
      const frozen = segments[index - 1].speed.num === 0;
      return <span key={index} className="retime-point" data-retime-point={frame} data-end={!point || undefined} style={{ '--at': handleAt(index, frame) } as CSSProperties}>
        <button type="button" tabIndex={-1} className="retime-handle" data-retime-handle="upper" data-retime-at={frame} disabled={locked || frozen}
          aria-label={t('Change the speed of the segment before')} title={t('Change the speed of the segment before')}
          onPointerDown={event => down(event, 'upper', index)} onPointerMove={move} onPointerUp={up}
          onPointerCancel={(event) => { event.stopPropagation(); cancel(); }} onLostPointerCapture={() => { cancel(); }} onDoubleClick={event => event.stopPropagation()} />
        {point && <button type="button" tabIndex={-1} className="retime-handle" data-retime-handle="lower" data-retime-at={frame} disabled={locked}
          aria-label={t('Move the speed point')} title={t('Move the speed point')}
          onPointerDown={event => down(event, 'lower', index)} onPointerMove={move} onPointerUp={up}
          onPointerCancel={(event) => { event.stopPropagation(); cancel(); }} onLostPointerCapture={() => { cancel(); }} onDoubleClick={event => event.stopPropagation()} />}
      </span>;
    })}
    <button type="button" className="retime-close" data-test="retime-close" aria-label={t('Close Retime Controls')} title={t('Close Retime Controls')}
      onPointerDown={quiet} onDoubleClick={event => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onToggle([clip.id]); }}>
      <X size={10} aria-hidden="true" />
    </button>
    {live && <span className="retime-tip" role="status" data-test="retime-tip">{live.tip}</span>}
    {menu && menu.segment < own.segments.length && <CoveringMenu menu={{ x: menu.x, y: menu.y, label, items: items(menu.segment) }} onClose={() => setMenu(null)} />}
  </div>;
}
