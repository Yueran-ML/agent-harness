import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, mergePatch, type Corrector, type GradeNode, type Param, type Target } from '../engine';
import { fullCorrector, getAt, isKeyed, patchOf, shownValue, withValue } from './corrector';
import type { KeyClock } from './keyFrames';
import type { GradeChange } from './nodeEdits';

/**
 * What an edit puts at a path of the node's params: a number, which goes where an edit at the
 * playhead goes (into a value, or into a keyframe, p. 3391), or a value put there whole (the
 * points of a curve, a switch).
 */
export type EditValue = number | { whole: unknown };
export type Edits = readonly (readonly [string, EditValue])[];

/** The corrector node that the palettes edit, where the current node is one, and where its grade is. */
export interface EditedNode {
  bridge: Pick<EngineBridge, 'call'>;
  /** The revision of the copy of the project that `node` is of. */
  revision: number;
  target: Target;
  node: GradeNode;
  /**
   * The frame of the playhead as the keys of the grade count it (PROTOCOL.md, "Frames of keys"): the
   * frame of the clip's media whose picture is shown, or of the clip from its first for a title, a
   * colour or a still (`keyClockOf`); of the timeline, for its grade.
   */
  frame: number;
  onFailure: (failure: unknown) => void;
  /**
   * What goes first in the transaction of an edit: where the grade has no node and the palettes edit
   * the node 01 it is shown with (DaVinci's default node), the `add_serial` that makes it (`withDefaultNode`).
   */
  before?: readonly GradeChange[];
}

/**
 * Where a grade of a level is, for the current clip of a timeline, as an operation names it
 * (PROTOCOL.md, "Grades"); and the frame of the playhead as its keys count it (PROTOCOL.md, "Frames
 * of keys"): for the clip's grade and its group's, the frame that `clock` (`keyClockOf` the clip)
 * gives, the frame of its media whose picture is shown, or, without one, the frame of the clip from
 * its first; from 0 of the timeline for the timeline's.
 */
export function placeOf(level: 'pre_clip' | 'clip' | 'post_clip' | 'timeline', timeline: string | null,
  clip: { id: string; start: number; group: string | null } | null, frame: number, clock?: KeyClock): { target: Target; frame: number } | null {
  if (timeline === null) return null;
  if (level === 'timeline') return { target: { level, timeline }, frame };
  if (clip === null) return null;
  const keyed = clock ? clock(frame) : frame - clip.start;
  if (level === 'clip') return { target: { level, timeline, clip: clip.id }, frame: keyed };
  return clip.group === null ? null : { target: { level, group: clip.group }, frame: keyed };
}

/** How long a value that was sent is shown, at most, until the engine tells of it, as the inspector's fields hold theirs. */
export const HOLD_MS = 1500;
/** How long a commit waits for the copy to take in the one before it, as the inspector's do. */
const WAIT_MS = 2000;

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * A drag that is sent as it goes (PROTOCOL.md, "Gestures"): each preview a transaction of one
 * gesture, at most one waiting for its answer, the newest edits taking the place of those not sent
 * yet; the commit (or Escape) the last, which ends it. The engine draws each, and keeps them all as
 * one step to undo.
 */
interface Gesture {
  id: string;
  /** The edits to send next, the newest; null where there are none. */
  next: Edits | null;
  /** Whether a transaction of it waits for its answer. */
  sending: boolean;
  /** Whether one of it was sent, and so the engine has it as a step. */
  started: boolean;
  /** Whether `next` is the last: the gesture ends with it. */
  ending: boolean;
  /** Whether the last was sent. */
  ended: boolean;
  /** Whether the engine refused one: nothing more of it is sent. */
  failed: boolean;
  /** What each setting it changed was before it, to put back on Escape. */
  before: Map<string, unknown>;
  /** Whether its first made the node it edits, the default node 01 of a grade without nodes (`EditedNode.before`). */
  made: boolean;
  /** Whether it was cancelled (Escape): its last takes back all it did. */
  cancelled: boolean;
}
let gestures = 0;

/**
 * The editing of the current node by the palettes, as DaVinci Resolve's controls edit it: what a
 * control is dragged to is shown at once (`preview`) and sent as it goes, as transactions of one
 * gesture, so that the viewer draws it while it is dragged; when it is let go (`commit`) the last
 * is sent, and the engine keeps the whole drag as one step to undo; Escape puts back what was
 * there (`cancel`), and the drag then leaves no step. An edit made at once, with no drag, is one
 * transaction. Until the engine tells of what was sent, the palettes show it, so that a control
 * does not jump back.
 *
 * Commits go in the order they were made, each worked out from the node as the copy has it once
 * the copy has taken in the one before, and bound to that revision, as the inspector's are; so a
 * second control set at once does not undo the first, and a change by someone else in between is
 * refused rather than overwritten.
 */
export function useNodeEdit(edited: EditedNode | null) {
  const node = edited?.node.kind === 'corrector' ? edited.node : null;
  const committed = useMemo(() => fullCorrector(node?.params), [node?.params]);
  const [live, setLive] = useState<ReadonlyMap<string, unknown>>(new Map());
  const [held, setHeld] = useState<ReadonlyMap<string, unknown>>(new Map());
  const now = useRef({ edited, committed });
  now.current = { edited, committed };
  const queue = useRef<Edits[]>([]);
  const busy = useRef(false);
  const awaited = useRef(-Infinity);
  const waiting = useRef<ReturnType<typeof setTimeout> | null>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; if (waiting.current !== null) clearTimeout(waiting.current); }; }, []);

  // What was sent and is now in the node is shown from the node; the rest a while longer.
  useEffect(() => {
    setHeld(old => (old.size === 0 ? old : new Map([...old].filter(([path, value]) => !same(getAt(committed, path), value)))));
  }, [committed]);
  useEffect(() => {
    if (held.size === 0) return undefined;
    const timer = setTimeout(() => setHeld(new Map()), HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  // Another node, or another grade: nothing of the one before is shown over it.
  const identity = node === null || edited === null ? '' : `${JSON.stringify(edited.target)}#${node.id}`;
  useEffect(() => { setLive(new Map()); setHeld(new Map()); }, [identity]);

  /** The values of edits, worked out from `base`: a number into its value or its key at the playhead. */
  const resolve = (edits: Edits, base: Corrector, frame: number): [string, unknown][] => edits.map(([path, value]) => [
    path, typeof value === 'number' ? withValue(getAt(base, path) as Param | undefined, frame, value) : value.whole,
  ]);
  const base = useMemo(() => (held.size === 0 ? committed : mergePatch(committed, patchOf(held)) as Corrector), [committed, held]);
  const shown = useMemo(() => (live.size === 0 ? base : mergePatch(base, patchOf(live)) as Corrector), [base, live]);
  const baseRef = useRef(base);
  baseRef.current = base;

  const gesture = useRef<Gesture | null>(null);
  /** Send the newest edits of the gesture going on, once the one before was answered. */
  const sendGesture = useCallback((): void => {
    const going = gesture.current;
    if (going === null || going.sending || going.failed || going.next === null) return;
    const context = now.current.edited;
    if (context === null || context.node.kind !== 'corrector') { gesture.current = null; return; }
    // The first waits, as a commit does, for the copy to have taken in what was sent before.
    if (!going.started && (busy.current || queue.current.length > 0 || context.revision < awaited.current)) {
      if (!busy.current && waiting.current === null) {
        waiting.current = setTimeout(() => { waiting.current = null; awaited.current = -Infinity; sendGesture(); }, WAIT_MS);
      }
      return;
    }
    const edits = going.next;
    const end = going.ending;
    going.next = null;
    going.sending = true;
    going.started = true;
    going.ended = end;
    const params = patchOf(resolve(edits, now.current.committed, context.frame));
    // The first of a gesture on the default node 01 makes it, in its transaction; the others name
    // it by the id it was made with. Cancelled, the last takes the node away again, so that the
    // drag leaves the grade without nodes as it was.
    const making = !going.made && context.before !== undefined && context.before.length > 0;
    if (making) going.made = true;
    const set = { op: 'set_node', grade: context.target, id: context.node.id, params };
    const ops = making ? [...context.before!, set]
      : end && going.cancelled && going.made ? [{ op: 'remove_node', grade: context.target, id: context.node.id }] : [set];
    // Bound to the revision of the copy, which may lag behind the gesture's own changes: the
    // engine takes any since the gesture began, and refuses one from before another's change.
    callEngine(context.bridge, 'transaction.apply', {
      revision: context.revision, ops,
      gesture: end ? { id: going.id, end: true } : { id: going.id },
    }).then((applied) => {
      if (end && typeof applied?.revision === 'number') awaited.current = applied.revision;
    }).catch((failure: unknown) => {
      going.failed = true;
      if (!alive.current) return;
      setHeld(new Map());
      now.current.edited?.onFailure(failure);
    }).finally(() => {
      going.sending = false;
      // One refused stays, sending nothing more, until the drag ends.
      if (gesture.current !== going || going.failed) return;
      if (going.ended) gesture.current = null;
      else sendGesture();
    });
  }, []);

  const pump = useCallback((): void => {
    if (busy.current) return;
    const context = now.current.edited;
    if (context === null) { queue.current = []; return; }
    if (context.revision < awaited.current) {
      if (queue.current.length > 0 && waiting.current === null) {
        waiting.current = setTimeout(() => { waiting.current = null; awaited.current = -Infinity; pump(); }, WAIT_MS);
      }
      return;
    }
    if (waiting.current !== null) { clearTimeout(waiting.current); waiting.current = null; }
    const edits = queue.current.shift();
    if (!edits) return;
    const params = patchOf(resolve(edits, now.current.committed, context.frame));
    busy.current = true;
    callEngine(context.bridge, 'transaction.apply', {
      revision: context.revision, ops: [...(context.before ?? []), { op: 'set_node', grade: context.target, id: context.node.id, params }],
    }).then((applied) => {
      if (typeof applied?.revision === 'number') awaited.current = applied.revision;
    }).catch((failure: unknown) => {
      if (!alive.current) return;
      setHeld(new Map());
      now.current.edited?.onFailure(failure);
    }).finally(() => { busy.current = false; pump(); sendGesture(); });
  }, [sendGesture]);
  useEffect(() => { pump(); sendGesture(); }, [edited?.revision, pump, sendGesture]);

  const frame = edited?.frame ?? 0;
  /**
   * Show edits at once, over the node as it is, and send them as part of the gesture going on (one
   * is begun): a control being dragged. Each call takes the place of the one before.
   */
  const preview = useCallback((edits: Edits) => {
    const values = resolve(edits, baseRef.current, now.current.edited?.frame ?? 0);
    setLive(new Map(values));
    if (now.current.edited?.node.kind !== 'corrector' || edits.length === 0) return;
    let going = gesture.current;
    if (going === null || going.ended) {
      // A drag that has changed nothing yet sends nothing (up and down on a master wheel).
      if (values.every(([path, value]) => same(getAt(baseRef.current, path), value))) return;
      gestures += 1;
      going = {
        id: `palette-${gestures}`, next: null, sending: false, started: false, ending: false, ended: false, failed: false, before: new Map(),
        made: false, cancelled: false,
      };
      gesture.current = going;
    }
    if (going.failed) return;
    for (const [path] of edits) if (!going.before.has(path)) going.before.set(path, getAt(baseRef.current, path));
    going.next = edits;
    sendGesture();
  }, [sendGesture]);
  /** Send edits, as one step to undo: the end of a gesture, or an edit made at once. */
  const commit = useCallback((edits: Edits) => {
    const going = gesture.current;
    // A drag none of which was sent yet is an edit made at once.
    if (going !== null && !going.ended && !going.started && !going.failed) gesture.current = null;
    else if (going !== null && !going.ended) {
      // The last of a drag: sent even where it changes nothing more, since it ends the gesture.
      setLive(new Map());
      if (going.failed) { gesture.current = null; return; }
      if (now.current.edited !== null && edits.length > 0) {
        setHeld(old => new Map([...old, ...resolve(edits, baseRef.current, now.current.edited!.frame)]));
        going.next = edits;
      } else going.next ??= [...going.before].map(([path, value]) => [path, { whole: value }] as const);
      going.ending = true;
      sendGesture();
      return;
    }
    if (now.current.edited === null || now.current.edited.node.kind !== 'corrector' || edits.length === 0) { setLive(new Map()); return; }
    const values = resolve(edits, baseRef.current, now.current.edited.frame);
    setLive(new Map());
    // Nothing that would change nothing is sent.
    if (values.every(([path, value]) => same(getAt(baseRef.current, path), value))) return;
    setHeld(old => new Map([...old, ...values]));
    queue.current.push(edits);
    pump();
  }, [pump]);
  /** Put back what the controls showed before a gesture (Escape); one that was sent ends where it began, and leaves no step. */
  const cancel = useCallback(() => {
    setLive(new Map());
    const going = gesture.current;
    if (going === null || going.ended) return;
    if (going.failed || !going.started) { gesture.current = null; return; }
    going.next = [...going.before].map(([path, value]) => [path, { whole: value }] as const);
    going.ending = true;
    going.cancelled = true;
    sendGesture();
  }, [sendGesture]);

  return {
    /** Whether there is a corrector to edit: the current node of the grade shown is one. */
    enabled: node !== null,
    /** The node's settings as the palettes show them, a gesture's and those sent laid over it. */
    corrector: shown,
    /** The settings without the gesture going on: where a gesture starts from. */
    base,
    frame,
    /** The value that a control of a number shows at the playhead (p. 3384: the value of the key that an edit goes into). */
    value: (path: string) => shownValue(getAt(shown, path) as Param | undefined, frame),
    /** Whether a setting has keyframes. */
    keyed: (path: string) => isKeyed(getAt(shown, path) as Param | undefined),
    preview, commit, cancel,
  };
}
export type NodeEdit = ReturnType<typeof useNodeEdit>;
