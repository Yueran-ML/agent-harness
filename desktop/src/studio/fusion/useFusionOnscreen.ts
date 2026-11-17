import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import {
  callEngine, viewPointerOf, type Clip, type Composition, type FusionNode, type FusionPicture, type FusionView, type Media, type Op, type OverlayPoint, type Timeline,
  type ViewPointer,
} from '../engine';
import { keyFrameOf } from '../inspector/clipKeys';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { followOf, withFirst, type FusionAt } from './edits';
import { controlsOf, draggedPattern, draggedValues, handleName, hitHandle, overlayOf, reachOf, type Controls, type Handle } from './onscreen';
import { paramOf, wholeParam, wholeValue } from './settings';
import { followedPlace, placedPattern, restoredPatterns } from './tracker';

/** A handle that is being dragged: what was pressed, and the node as it was then, from which every move is worked out. */
interface Drag {
  at: FusionAt;
  /** Whether the clip had a composition at the press: where it had none, the first one goes before the first change. */
  has: boolean;
  node: FusionNode;
  handle: Handle;
  /** The frame of the keys of the clip that the press was at. */
  keyFrame: number;
  picture: FusionPicture;
  /** Where the pointer pressed, in pixels of the frame drawn. */
  from: OverlayPoint;
  /** Whether a move has begun the gesture. */
  began: boolean;
  /** Whether Escape took the drag back: the moves until the button is let go do nothing. */
  cancelled: boolean;
}

/**
 * The on-screen controls of the node that is chosen, in the viewer of the Fusion page: the outline
 * of a mask and the handles of its centre, its width and its height or its points, the handle of
 * the centre of a Merge or a Transform, and the boxes, the paths and the handles of the patterns of
 * a Tracker (`onscreen.ts`). A centre that follows a Tracker is drawn where it follows it to, where
 * the page works that out (`followedPlace`: the place of a pattern, not the Steady or the Unsteady
 * Position), and its handle is not taken hold of.
 *
 * The viewer is a window of the engine's, over which the page can draw nothing. So the page works
 * the controls out, in the pixels of the frame drawn, and has the engine draw them
 * (`view.overlay`), again whenever they change: the node is edited from anywhere, another is
 * chosen, the playhead comes to another value of a setting that has keys. They are drawn while the
 * Fusion view tells that the picture of the composition is drawn (`shown`) and where it lies
 * (`picture`), whichever node is viewed: a shape is in the coordinates of the composition either
 * way. With nothing to show the engine draws nothing. While they show, the view takes the pointer
 * (`view.pointer`) and tells what the pointer does on the picture; the page finds the handle that
 * is pressed among those that it had drawn, within a few pixels of the screen.
 *
 * - A handle dragged with the left button sets its settings by as much as the pointer moves
 *   (`draggedValues`), each kept within its range. The drag is one gesture (`useAudioGesture`): each
 *   move sets the settings whole, worked out from the node as it was at the press, as a number
 *   dragged in the Inspector does (`wholeValue`: a setting without keys by a patch of the node, a
 *   number of a point of a Polygon by `set_fusion_keys`, a setting with keys as its keys with the
 *   one at the playhead changed or put there); the release ends it, and the whole is one step to
 *   undo. Escape takes it back, and the moves until the button is let go do nothing. Shift keeps the
 *   centre, a point or a pattern on the axis of the frame that the pointer has gone further along.
 * - The handle of a pattern of a Tracker sets where the pattern is put (`draggedPattern`), from
 *   where it was drawn at the press, by the list of patterns set whole at each move (`placedPattern`),
 *   in one gesture as a centre is; while it is dragged the pattern is drawn where the drag puts it. A
 *   pattern that has a path keeps its path: let go, it is drawn on its path again.
 * - A clip on a locked track shows its controls, and a press on a handle does nothing: the node
 *   editor and the Inspector say why.
 * - A press on the picture is told to `onPress`: the page beneath the view does not hear it.
 * - What the engine refuses of a drag is told to `onFailure`; but not a change that changes nothing
 *   (`empty`), which is no failure.
 */
export function useFusionOnscreen({ bridge, session, revision, native, timeline, clip, media, composition = null, node, frame, view, locked, onPress, onFailure }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number;
  /** Whether the shell lays the engine's view over the viewer: in a browser nothing is drawn. */
  native: boolean;
  timeline: Timeline | null;
  /** The clip whose composition the page shows. */
  clip: Clip | null;
  media: readonly Media[];
  /** The composition as the page shows it: where a centre that follows a Tracker is, is read from it. */
  composition?: Composition | null;
  /** The one node that is chosen, or null where none or several are. */
  node: FusionNode | null;
  /** The playhead, in frames of the timeline. */
  frame: number;
  /** The Fusion view of this clip as the state of the monitor tells it, or null while it tells none. */
  view: FusionView | null;
  /** Whether the clip lies on a locked track: its controls are shown, and none is dragged. */
  locked: boolean;
  onPress?: () => void;
  onFailure?: (error: unknown) => void;
}): { shown: boolean } {
  const lies = view !== null && view.shown && view.picture ? view.picture : null;
  const [left, top, width, height] = lies ? [lies.x, lies.y, lies.width, lies.height] : [0, 0, 0, 0];
  const [canvasWidth, canvasHeight] = view !== null && Array.isArray(view.canvas) ? view.canvas : [0, 0];
  const placed = native && lies !== null && width > 0 && height > 0 && canvasWidth > 0 && canvasHeight > 0;
  const keyFrame = clip !== null && timeline !== null ? keyFrameOf(clip, timeline, media, frame) : 0;
  const picture = useMemo<FusionPicture | null>(() => (placed ? { x: left, y: top, width, height } : null), [placed, left, top, width, height]);
  /** The pattern of a Tracker that a drag has taken, and where it has put it: it is drawn there until the drag ends. */
  const [placing, setPlacing] = useState<{ index: number; place: [number, number] } | null>(null);
  // Worked out again only when the node, the frame of its keys, the place of the picture, the composition (which a centre that follows
  // a Tracker reads) or a pattern dragged changes: not at each frame told.
  const controls = useMemo<Controls | null>(() => {
    if (picture === null || node === null || clip === null || timeline === null) return null;
    const follow = followOf(node);
    const follows = follow === null ? null : { place: composition === null ? null : followedPlace(composition, follow, keyFrame) };
    return controlsOf(node, keyFrame, picture, { follows, placing });
  }, [picture, node, clip?.id, timeline?.id, keyFrame, composition, placing]);
  const showing = controls !== null;

  /** The handle that is being dragged, by its name among those of its node. */
  const [active, setActive] = useState<string | null>(null);
  const has = clip?.fusion !== undefined;
  const failed = useRef(onFailure);
  failed.current = onFailure;
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline?.id ?? ''}/${clip?.id ?? ''}/fusion-onscreen/${node?.id ?? ''}`,
    locked: locked || clip === null || node === null,
    onFailure: (error) => { failed.current?.(error); },
  });
  const drag = useRef<Drag | null>(null);
  /** The native window that the pointer was last told of, and its last event: what is older is passed over. */
  const heard = useRef({ instance: -1, event: -1 });
  // What the listeners read: they are made once, and this render's are these.
  const now = useRef({ session, controls, picture, node, clip, timeline, keyFrame, has, locked, gesture, onPress });
  now.current = { session, controls, picture, node, clip, timeline, keyFrame, has, locked, gesture, onPress };

  const dragged = controls !== null && active !== null ? controls.handles.findIndex(handle => handleName(handle) === active) : -1;
  const drawn = useMemo(() => (controls !== null ? overlayOf(controls, [canvasWidth, canvasHeight], dragged < 0 ? null : dragged) : null),
    [controls, canvasWidth, canvasHeight, dragged]);

  // What the engine draws, told whenever it changes: the same again is not sent again. And nothing, once there are no controls to show.
  const told = useRef<string | null>(null);
  useEffect(() => {
    if (drawn === null) return;
    const written = JSON.stringify(drawn);
    if (written === told.current) return;
    told.current = written;
    void callEngine(bridge, 'view.overlay', { monitor: 'timeline', overlay: drawn }, { expectedSession: session }).catch(() => {});
  }, [bridge, session, drawn]);
  useEffect(() => {
    if (!showing) return undefined;
    return () => {
      told.current = null;
      void callEngine(bridge, 'view.overlay', { monitor: 'timeline', overlay: null }, { expectedSession: session }).catch(() => {});
    };
  }, [bridge, session, showing]);

  // While controls are shown the view takes the pointer. A view that is attached later is asked again: one that was not there could not take it.
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

  // Another node, another clip, another timeline, or no controls any more: nothing is dragged. For another node the gesture has taken
  // itself back already (`useAudioGesture`); a drag whose controls are shown no longer ends where it is.
  useEffect(() => {
    const held = drag.current;
    drag.current = null;
    if (held?.began && !held.cancelled) now.current.gesture.end();
    setActive(null);
    setPlacing(null);
  }, [session, timeline?.id, clip?.id, node?.id, showing]);

  const press = useCallback((pointer: ViewPointer) => {
    const at = now.current;
    if (!at.controls || !at.picture || !at.node || !at.clip || !at.timeline) return;
    if (pointer.x === null || pointer.y === null || pointer.button !== 'left') return;
    // A picture of another timeline than the one whose controls are drawn is not the one that the handle is on.
    if (pointer.snapshot && pointer.snapshot.timeline !== at.timeline.id) return;
    const index = hitHandle(at.controls, [pointer.x, pointer.y], reachOf(pointer));
    // Nothing of a clip on a locked track is changed: its handles are drawn, and none is taken hold of.
    if (index < 0 || at.locked) return;
    const handle = at.controls.handles[index];
    drag.current = {
      at: { timeline: at.timeline.id, clip: at.clip.id }, has: at.has, node: at.node, handle, keyFrame: at.keyFrame, picture: at.picture,
      from: [pointer.x, pointer.y], began: false, cancelled: false,
    };
    setActive(handleName(handle));
  }, []);

  const move = useCallback((pointer: ViewPointer, held: Drag) => {
    if (held.cancelled || pointer.x === null || pointer.y === null) return;
    let dx = pointer.x - held.from[0];
    let dy = pointer.y - held.from[1];
    // Shift keeps the centre, a point or a pattern on the axis that the pointer has gone further along.
    if (pointer.shift && (held.handle.kind === 'center' || held.handle.kind === 'point' || held.handle.kind === 'pattern')) {
      if (Math.abs(dx) < Math.abs(dy)) dx = 0; else dy = 0;
    }
    if (held.handle.kind === 'pattern') {
      // The place where a pattern of a Tracker is put: the list of patterns set whole at each move, the others as they are.
      const index = held.handle.index ?? -1;
      const place = draggedPattern(held.node, index, held.keyFrame, held.picture, dx, dy);
      const placed = place === null ? null : placedPattern(held.at, held.node, index, place[0], place[1]);
      if (place === null || placed === null) return;
      const { gesture: sending } = now.current;
      if (!held.began) {
        if (dx === 0 && dy === 0) return;
        const restore = restoredPatterns(held.at, held.node);
        if (restore === null || !sending.start([restore])) {
          drag.current = null;
          setActive(null);
          return;
        }
        held.began = true;
      }
      setPlacing({ index, place });
      sending.preview([placed]);
      return;
    }
    const moved = draggedValues(held.node, held.handle, held.keyFrame, held.picture, dx, dy);
    if (moved.length === 0) return;
    const { gesture: sending } = now.current;
    if (!held.began) {
      // A press that has not moved the handle sends nothing, and leaves nothing to undo.
      if (dx === 0 && dy === 0) return;
      // What the gesture puts back where it is taken back: the settings as they were at the press, or no composition where the clip had none.
      const restore: Op[] = held.has ? moved.map(({ setting }) => wholeParam(held.at, held.node, setting, paramOf(held.node, setting)))
        : [{ op: 'set_fusion', ...held.at, value: null }];
      if (!sending.start(restore)) {
        drag.current = null;
        setActive(null);
        return;
      }
      held.began = true;
    }
    sending.preview(withFirst(held.at, held.has, moved.map(({ setting, value }) => wholeValue(held.at, held.node, setting, paramOf(held.node, setting), value, held.keyFrame))));
  }, []);

  const release = useCallback(() => {
    const held = drag.current;
    if (!held) return;
    drag.current = null;
    setActive(null);
    setPlacing(null);
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
    if (!at.controls) return;
    if (pointer.kind === 'down') {
      // The page beneath the view does not hear this press: the page is told of it here.
      at.onPress?.();
      if (drag.current === null) press(pointer);
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
      setPlacing(null);
      if (held.began) now.current.gesture.cancel();
    };
    window.addEventListener('keydown', escape, true);
    return () => window.removeEventListener('keydown', escape, true);
  }, []);

  return { shown: showing };
}
