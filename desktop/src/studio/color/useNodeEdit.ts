import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, mergePatch, type Corrector, type GradeNode, type Param, type Target } from '../engine';
import { fullCorrector, getAt, isKeyed, patchOf, shownValue, withValue } from './corrector';

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
  /** The frame of the playhead as the keys of the grade count it: from the clip's first frame (from 0 of the timeline, for its grade). */
  frame: number;
  onFailure: (failure: unknown) => void;
}

/**
 * Where a grade of a level is, for the current clip of a timeline, as an operation names it
 * (PROTOCOL.md, "Grades"); and the frame of the playhead as its keys count it: from the clip's first
 * frame for the clip's grade and its group's (PROTOCOL.md, "Frames of keys"), from 0 of the
 * timeline for the timeline's.
 */
export function placeOf(level: 'pre_clip' | 'clip' | 'post_clip' | 'timeline', timeline: string | null,
  clip: { id: string; start: number; group: string | null } | null, frame: number): { target: Target; frame: number } | null {
  if (timeline === null) return null;
  if (level === 'timeline') return { target: { level, timeline }, frame };
  if (clip === null) return null;
  if (level === 'clip') return { target: { level, timeline, clip: clip.id }, frame: frame - clip.start };
  return clip.group === null ? null : { target: { level, group: clip.group }, frame: frame - clip.start };
}

/** How long a value that was sent is shown, at most, until the engine tells of it, as the inspector's fields hold theirs. */
export const HOLD_MS = 1500;
/** How long a commit waits for the copy to take in the one before it, as the inspector's do. */
const WAIT_MS = 2000;

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The editing of the current node by the palettes, as DaVinci Resolve's controls edit it: what a
 * control is dragged to is shown at once (`preview`), and sent once, as one transaction and so one
 * step to undo, when it is let go (`commit`); Escape puts back what was there (`cancel`). Until the
 * engine tells of what was sent, the palettes show it, so that a control does not jump back.
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
      revision: context.revision, ops: [{ op: 'set_node', grade: context.target, id: context.node.id, params }],
    }).then((applied) => {
      if (typeof applied?.revision === 'number') awaited.current = applied.revision;
    }).catch((failure: unknown) => {
      if (!alive.current) return;
      setHeld(new Map());
      now.current.edited?.onFailure(failure);
    }).finally(() => { busy.current = false; pump(); });
  }, []);
  useEffect(() => { pump(); }, [edited?.revision, pump]);

  const frame = edited?.frame ?? 0;
  /** Show edits at once, over the node as it is: a gesture going on. Each call takes the place of the one before. */
  const preview = useCallback((edits: Edits) => {
    setLive(new Map(resolve(edits, baseRef.current, now.current.edited?.frame ?? 0)));
  }, []);
  /** Send edits, as one step to undo: the end of a gesture. */
  const commit = useCallback((edits: Edits) => {
    if (now.current.edited === null || now.current.edited.node.kind !== 'corrector' || edits.length === 0) { setLive(new Map()); return; }
    const values = resolve(edits, baseRef.current, now.current.edited.frame);
    setLive(new Map());
    // Nothing that would change nothing is sent.
    if (values.every(([path, value]) => same(getAt(baseRef.current, path), value))) return;
    setHeld(old => new Map([...old, ...values]));
    queue.current.push(edits);
    pump();
  }, [pump]);
  /** Put back what the controls showed before a gesture (Escape). */
  const cancel = useCallback(() => setLive(new Map()), []);

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
