import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Menu } from '../color/ColorMenu';
import { CoveringMenu } from '../color/CoveringMenu';
import { callEngine, viewPointerOf, type Clip, type Media, type Monitor, type OverlayPoint, type Param, type Timeline, type ViewPointer } from '../engine';
import { keyOps, paramOf } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { useCommit } from '../inspector/useCommit';
import type { Change } from '../timeline/commands';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import {
  HIT_PX, POSITION, POSITION_X, POSITION_Y, anchorAtPlayhead, hitKey, interpolated, motionPathOf, movedPoint, overlayOf, pointInterp,
  type MotionPath, type PointInterp, type PositionPath,
} from './motionPath';

/** A point of the path that is being dragged: what was pressed, and the clip as it was then, from which every move is worked out. */
interface Drag {
  clip: Clip;
  timeline: string;
  width: number;
  height: number;
  /** The frame of the keys of the point. */
  frame: number;
  /** Where the pointer pressed, in pixels of the timeline. */
  from: OverlayPoint;
  /** Whether a move has begun the gesture. */
  began: boolean;
  /** Whether Escape took the drag back: the moves until the button is let go do nothing. */
  cancelled: boolean;
}

const setKeys = (timeline: string, id: string, path: PositionPath, param: Param): Change => ({ op: 'set_clip_keys', timeline, id, path, param });

/** How far from a point the pointer may be to press it, in pixels of the timeline, as the picture is shown: `HIT_PX` pixels of the screen. */
function reachOf(pointer: ViewPointer): number {
  const map = pointer.presentation?.map;
  return map && map.dest[2] > 0 ? (HIT_PX * map.picture[0]) / map.dest[2] : HIT_PX;
}

/**
 * The motion path of the one clip of picture that is chosen, in the timeline viewer of the Edit page
 * (DaVinci Resolve Reference Manual 20, pp. 1106–1108), while the on-screen Transform controls are
 * on (`on`) and the position of the clip has keys.
 *
 * The viewer is a window of the engine's, over which the page can draw nothing. So the page works
 * the path out (`motionPath`) and has the engine draw it there (`view.overlay`: lines and dots in
 * pixels of the timeline), again whenever it changes: the clip is edited from anywhere, another is
 * chosen, the playhead moves. With no path to show it has the engine draw nothing. While it shows,
 * the view takes the pointer (`view.pointer`), and tells what the pointer does on the picture; the
 * page finds the point that is pressed among those that it had drawn.
 *
 * - The place at the playhead is marked while the timeline does not play. While it plays the path
 *   and its points stay and the mark is left out: the page hears of the frames some time after the
 *   picture shows them, and a mark drawn from what it hears would trail the picture.
 * - A point dragged with the left button moves its key of `transform.x` and of `transform.y` by as
 *   much as the pointer moves on the picture; Shift keeps it on the axis that it has moved further
 *   along. The drag is one gesture (`useAudioGesture`): each move sets the two settings whole with
 *   `set_clip_keys`, worked out from the clip as it was at the press, and the whole is one step to
 *   undo. Escape takes it back, and the moves until the button is let go do nothing. A setting that
 *   has no key at the frame of the point gets one as it is moved (`movedPoint`).
 * - The right button on a point opens its menu: Linear and Smooth (`interpolated`), one transaction.
 * - A clip on a locked track shows its path; a press on a point of it is refused with the sentence
 *   of the Inspector, and its menu is off.
 * - A press on the picture makes the timeline monitor the one that the keys go to, as a press on
 *   the viewer does while the view does not take the pointer.
 *
 * What it gives: the menu of a point where one is open, what is to be said under the viewer (a
 * refusal of the engine, or that the track is locked), and whether a path is shown.
 */
export function useMotionPath({ bridge, session, revision, native, loaded, on, timeline, media, clips, frame, playing, onFocus, viewer }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number;
  /** Whether the shell lays the engine's view over the viewer: in a browser nothing is drawn. */
  native: boolean;
  /** Whether the timeline monitor has the timeline loaded. */
  loaded: boolean;
  /** Whether the on-screen Transform controls are on. */
  on: boolean;
  timeline: Timeline | null; media: readonly Media[];
  /** The clips chosen on the timeline. */
  clips: readonly string[];
  /** The playhead, in frames of the timeline. */
  frame: number;
  playing: boolean;
  onFocus?: (monitor: Monitor) => void;
  /** What holds the viewers of the page: the menu of a point is opened where the pointer is in the timeline viewer. */
  viewer: RefObject<HTMLElement | null>;
}): { menu: ReactNode; notice: string | null; shown: boolean } {
  const words = useWords();
  // One clip of picture with its linked sound chosen is one clip, as for the Curve Editor.
  const located = useMemo(() => {
    if (!timeline) return null;
    const locations = clipLocations(timeline);
    const chosen = clips.flatMap((id) => {
      const found = locations.get(id);
      return found && found.track.kind === 'video' ? [found] : [];
    });
    return chosen.length === 1 ? chosen[0] : null;
  }, [timeline, clips]);
  const clip = located?.clip ?? null;
  const locked = located?.track.locked ?? false;
  const shape = timeline ? `${timeline.width}x${timeline.height}@${timeline.rate.num}/${timeline.rate.den}` : '';
  // Worked out again only when the clip, the size or the rate of the timeline, or the media changes: not at each frame played.
  const path = useMemo(() => (on && clip && timeline ? motionPathOf(clip, timeline, media) : null), [on, clip, shape, media]);
  const showing = native && loaded && path !== null;

  /** The frame of the keys of the point that is being dragged. */
  const [active, setActive] = useState<number | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; clip: string; frame: number } | null>(null);
  const [lockedTold, setLockedTold] = useState(false);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline?.id ?? ''}/${clip?.id ?? ''}/motion-path`, locked: locked || clip === null,
    onFailure: error => setRefusal(refusalOf(error, words.hint.studio)),
  });
  const drag = useRef<Drag | null>(null);
  /** The native window that the pointer was last told of, and its last event: what is older is passed over. */
  const heard = useRef({ instance: -1, event: -1 });
  // What the listeners read: they are made once, and this render's are these.
  const now = useRef({ session, showing, path, clip, locked, timeline, gesture, onFocus, viewer });
  now.current = { session, showing, path, clip, locked, timeline, gesture, onFocus, viewer };

  const current = showing && !playing && clip && timeline ? anchorAtPlayhead(clip, timeline, media, frame) : null;
  const dragged = path !== null && active !== null ? path.keys.findIndex(key => key.frame === active) : -1;
  const drawn = useMemo(() => (showing && path ? overlayOf(path, current, dragged < 0 ? null : dragged) : null),
    [showing, path, current?.[0], current?.[1], dragged]);

  // What the engine draws, told whenever it changes; and nothing, once no path is shown.
  useEffect(() => {
    if (drawn !== null) void callEngine(bridge, 'view.overlay', { monitor: 'timeline', overlay: drawn }, { expectedSession: session }).catch(() => {});
  }, [bridge, session, drawn]);
  useEffect(() => {
    if (!showing) return undefined;
    return () => { void callEngine(bridge, 'view.overlay', { monitor: 'timeline', overlay: null }, { expectedSession: session }).catch(() => {}); };
  }, [bridge, session, showing]);

  // While a path is shown the view takes the pointer. A view that is attached later is asked again: one that was not there could not take it.
  useEffect(() => {
    if (!showing) return undefined;
    const take = () => { void callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: session }).catch(() => {}); };
    take();
    const unheard = bridge.onEvent((event) => { if (event.kind === 'attached' && event.session === session && event.monitor === 'timeline') take(); });
    return () => {
      unheard();
      void callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: false }, { expectedSession: session }).catch(() => {});
    };
  }, [bridge, session, showing]);

  // Another clip, another timeline, or no path any more: nothing is dragged, and no menu is open. For another clip the gesture has
  // taken itself back already (`useAudioGesture`); a drag whose path is shown no longer (the controls turned off) ends where it is.
  useEffect(() => {
    const held = drag.current;
    drag.current = null;
    if (held?.began && !held.cancelled) now.current.gesture.end();
    setActive(null);
    setMenu(null);
    setLockedTold(false);
  }, [session, timeline?.id, clip?.id, showing]);

  const press = useCallback((pointer: ViewPointer, drawnPath: MotionPath, pressed: Clip, shownOn: Timeline) => {
    if (pointer.x === null || pointer.y === null || (pointer.button !== 'left' && pointer.button !== 'right')) return;
    // A picture of another timeline than the one whose path is drawn is not the one that the point is on.
    if (pointer.snapshot && pointer.snapshot.timeline !== shownOn.id) return;
    const index = hitKey(drawnPath, [pointer.x, pointer.y], reachOf(pointer));
    if (index < 0) return;
    const point = drawnPath.keys[index];
    if (pointer.button === 'right') {
      // The point of the view's window, in pixels of the screen, as a place of the page.
      const box = now.current.viewer.current?.querySelector('[data-monitor="timeline"] [data-test="viewer-box"]')?.getBoundingClientRect();
      const ratio = typeof devicePixelRatio === 'number' && devicePixelRatio > 0 ? devicePixelRatio : 1;
      setMenu({ x: (box?.left ?? 0) + pointer.window.x / ratio, y: (box?.top ?? 0) + pointer.window.y / ratio, clip: pressed.id, frame: point.frame });
      return;
    }
    if (now.current.locked) { setLockedTold(true); return; }
    setLockedTold(false);
    setRefusal(null);
    drag.current = {
      clip: pressed, timeline: shownOn.id, width: shownOn.width, height: shownOn.height, frame: point.frame, from: [pointer.x, pointer.y],
      began: false, cancelled: false,
    };
    setActive(point.frame);
  }, [setRefusal]);

  const move = useCallback((pointer: ViewPointer, held: Drag) => {
    if (held.cancelled || pointer.x === null || pointer.y === null) return;
    const across = pointer.x - held.from[0];
    const down = pointer.y - held.from[1];
    // Shift keeps the point on the axis that the pointer has gone further along.
    const dx = pointer.shift && Math.abs(across) < Math.abs(down) ? 0 : across / held.width;
    const dy = pointer.shift && Math.abs(across) >= Math.abs(down) ? 0 : down / held.height;
    const moved = movedPoint(held.clip, held.frame, dx, dy);
    const { gesture: sending } = now.current;
    if (!held.began) {
      // A press that has not moved the point sends nothing, and leaves nothing to undo.
      if (dx === 0 && dy === 0) return;
      // What the gesture puts back where it is taken back: the two settings as they were at the press.
      if (!sending.start(POSITION.map(path => setKeys(held.timeline, held.clip.id, path, paramOf(held.clip, path))))) {
        drag.current = null;
        setActive(null);
        return;
      }
      held.began = true;
    }
    sending.preview([setKeys(held.timeline, held.clip.id, POSITION_X, moved[POSITION_X]), setKeys(held.timeline, held.clip.id, POSITION_Y, moved[POSITION_Y])]);
  }, []);

  const release = useCallback(() => {
    const held = drag.current;
    if (!held) return;
    drag.current = null;
    setActive(null);
    // Its last move is what stays; one that Escape took back has ended already.
    if (held.began && !held.cancelled) now.current.gesture.end();
  }, []);

  useEffect(() => bridge.onEvent((event) => {
    const pointer = viewPointerOf(event);
    const at = now.current;
    if (!pointer || event.session !== at.session || pointer.monitor !== 'timeline') return;
    const seen = heard.current;
    if (pointer.instance !== undefined) {
      if (pointer.instance < seen.instance) return;
      // Another native window: what was pressed in the one before is let go, keeping what it did.
      if (pointer.instance > seen.instance) { seen.instance = pointer.instance; seen.event = -1; release(); }
    }
    if (pointer.event !== undefined) {
      if (pointer.event <= seen.event) return;
      seen.event = pointer.event;
    }
    if (!at.showing || !at.path || !at.clip || !at.timeline) return;
    if (pointer.kind === 'down') {
      // The page beneath the view does not hear this press: the monitor is made the one that the keys go to here.
      at.onFocus?.('timeline');
      if (drag.current === null) press(pointer, at.path, at.clip, at.timeline);
      return;
    }
    const held = drag.current;
    if (!held) return;
    if (pointer.kind === 'move') move(pointer, held);
    // The window lost the mouse: the drag ends where it is, as when the button is let go. Only Escape takes it back.
    else if ((pointer.kind === 'up' && pointer.button === 'left') || pointer.kind === 'cancel') release();
  }), [bridge, press, move, release]);

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      const held = drag.current;
      if (event.key !== 'Escape' || !held || held.cancelled) return;
      event.preventDefault();
      event.stopPropagation();
      held.cancelled = true;
      setActive(null);
      if (held.began) now.current.gesture.cancel();
    };
    window.addEventListener('keydown', escape, true);
    return () => window.removeEventListener('keydown', escape, true);
  }, []);

  const closeMenu = useCallback(() => setMenu(null), []);
  // The menu of the point that the right button was pressed on: the same menu until the point, the clip or the lock changes.
  const pointMenu = useMemo((): Menu | null => {
    if (!menu || !clip || menu.clip !== clip.id || !showing) return null;
    const { clip: pressed, frame: keyFrame } = menu;
    // One transaction, worked out from the clip as the engine has it when it goes: the keys that differ, of each setting that has a key there.
    const make = (kind: PointInterp) => {
      void commit((timelineNow): Change[] => {
        const there = clipLocations(timelineNow).get(pressed);
        if (!there || there.track.locked) return [];
        return POSITION.flatMap((path) => {
          const made = interpolated(paramOf(there.clip, path), keyFrame, kind);
          return made ? keyOps(timelineNow.id, there.clip, path, made) : [];
        });
      });
    };
    const how = pointInterp(clip, keyFrame);
    const labels = words.label.studio;
    return { x: menu.x, y: menu.y, label: labels.motionPath.point, items: [
      { id: 'path-linear', label: labels.curves.linear, checked: how === 'linear', disabled: locked, run: () => make('linear') },
      { id: 'path-smooth', label: labels.motionPath.smooth, checked: how === 'smooth', disabled: locked, run: () => make('smooth') },
    ] };
  }, [menu, clip, showing, locked, words, commit]);
  const notice = !showing ? null : refusal ? refusal.text : lockedTold ? words.hint.studio.inspector.locked : null;
  return { menu: pointMenu ? <CoveringMenu menu={pointMenu} onClose={closeMenu} /> : null, notice, shown: showing };
}
