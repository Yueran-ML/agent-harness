/**
 * The commands of the Keyframe Editor of the Color page (DaVinci Resolve Reference Manual 20,
 * Ch. 148), which the page keeps so that their keys work whichever panel is shown at the right of
 * the palettes: Add Dynamic Keyframe (Ctrl+[) and Add Static Keyframe (Ctrl+]) at the playhead
 * (pp. 3384–3385), Delete Keyframe (Alt+]) and Delete All Keyframes (p. 3392), Change to Dynamic or
 * Static Keyframe (p. 3386), Change Dynamic Attributes (pp. 3391–3392), and the keyframe before
 * and after the playhead ([ and ]). Each is one transaction, which one undo takes back.
 *
 * It also keeps what the project does not: the mode, All or Color (pp. 3383–3384), which says
 * whether a keyframe goes on every node's track or on the current node's alone; and the tracks
 * whose Auto-Keyframe is on (p. 3383), by grade and node.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import type { Grade, Target } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { keyFrameAt, stepTime, timesOf, type KeySpan } from './keyTracks';
import {
  addKeyframes, changeKeyframes, deleteAllKeyframes, deleteKeyframes, easeKeyframes, keyframeFrames, keyframeKey, nodesFor,
  type KeyKind, type KeyScope, type KeyTracks,
} from './nodeKeys';
import type { GradeEditing } from './useGradeEditing';

export interface KeyframeEditing {
  /** Which tracks the tool bar and the keys put a keyframe on, or take one from. */
  scope: KeyScope;
  setScope: (scope: KeyScope) => void;
  /** The nodes of the grade shown whose tracks have Auto-Keyframe on. */
  auto: ReadonlySet<string>;
  toggleAuto: (node: string) => void;
  /** A keyframe of a kind at the playhead, on the tracks of the mode or on those named. */
  add: (kind: KeyKind, on?: KeyTracks) => void;
  /** The keyframe at a frame of the keys taken away: at the playhead where none is given. */
  remove: (on?: KeyTracks, frame?: number) => void;
  removeAll: (on?: KeyTracks) => void;
  change: (on: KeyTracks, frame: number, kind: KeyKind) => void;
  ease: (on: KeyTracks, frame: number, start: number, end: number) => void;
  /** The playhead to the keyframe before or after it, among those of the tracks of the mode. */
  step: (direction: -1 | 1) => void;
}

export function useKeyframeEditing({ grade, target, gradeKey, current, span, frame, editing, onSeek, reset }: {
  /** The grade shown, with its default node where it has none; null where there is none. */
  grade: Grade | null;
  target: Target | null;
  /** What names the grade shown among those of the page: its level and its clip or timeline. */
  gradeKey: string;
  /** The current node. */
  current: string | null;
  /** Where the grade is shown and the clock of its keys; null where nothing is. */
  span: KeySpan | null;
  /** The playhead, in frames of the timeline; null where no timeline is loaded. */
  frame: number | null;
  editing: Pick<GradeEditing, 'run'>;
  onSeek: (frame: number) => void;
  /** What tells one timeline and one engine from another: with another, no track has Auto-Keyframe on. */
  reset: string;
}): KeyframeEditing {
  const [scope, setScope] = useState<KeyScope>('all');
  const [autoKeys, setAutoKeys] = useState<ReadonlySet<string>>(() => new Set());
  useEffect(() => { setAutoKeys(old => (old.size === 0 ? old : new Set())); }, [reset]);
  const now = useRef({ grade, target, gradeKey, current, span, frame, editing, onSeek, scope });
  now.current = { grade, target, gradeKey, current, span, frame, editing, onSeek, scope };

  /** The frame of the keys at the playhead, or null where there is none. */
  const playhead = useCallback((): number | null => {
    const state = now.current;
    return state.span === null || state.frame === null ? null : keyFrameAt(state.span, state.frame);
  }, []);
  const nodes = useCallback((on: KeyTracks) => {
    const state = now.current;
    return state.grade === null ? [] : nodesFor(state.grade, on, state.scope, state.current);
  }, []);

  const add = useCallback((kind: KeyKind, on: KeyTracks = 'scope') => {
    const { target: grade, editing: send } = now.current;
    const at = playhead();
    if (grade !== null && at !== null) send.run(addKeyframes(grade, nodes(on), at, kind));
  }, [playhead, nodes]);
  const remove = useCallback((on: KeyTracks = 'scope', frame?: number) => {
    const { target: grade, editing: send } = now.current;
    const at = frame ?? playhead();
    if (grade !== null && at !== null) send.run(deleteKeyframes(grade, nodes(on), at));
  }, [playhead, nodes]);
  const removeAll = useCallback((on: KeyTracks = 'scope') => {
    const { target: grade, editing: send } = now.current;
    // Each setting stays what it is at the playhead; without a playhead, what it is at frame 0 of its keys.
    if (grade !== null) send.run(deleteAllKeyframes(grade, nodes(on), playhead() ?? 0));
  }, [playhead, nodes]);
  const change = useCallback((on: KeyTracks, frame: number, kind: KeyKind) => {
    const { target: grade, editing: send } = now.current;
    if (grade !== null) send.run(changeKeyframes(grade, nodes(on), frame, kind));
  }, [nodes]);
  const ease = useCallback((on: KeyTracks, frame: number, start: number, end: number) => {
    const { target: grade, editing: send } = now.current;
    if (grade !== null) send.run(easeKeyframes(grade, nodes(on), frame, start, end));
  }, [nodes]);
  const step = useCallback((direction: -1 | 1) => {
    const state = now.current;
    if (state.span === null || state.frame === null) return;
    const to = stepTime(timesOf(state.span, keyframeFrames(nodes('scope'))), Math.floor(state.frame), direction);
    if (to !== null) state.onSeek(to);
  }, [nodes]);

  const commands = useRef({ add, remove, step });
  commands.current = { add, remove, step };
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const command = keyframeKey(key);
      if (command === null) return;
      event.preventDefault();
      if (command.kind === 'add') commands.current.add(command.key);
      else if (command.kind === 'delete') commands.current.remove();
      else commands.current.step(command.direction);
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  const prefix = `${gradeKey}/`;
  const auto = useMemo<ReadonlySet<string>>(() => new Set([...autoKeys].filter(each => each.startsWith(prefix)).map(each => each.slice(prefix.length))),
    [autoKeys, prefix]);
  const toggleAuto = useCallback((node: string) => setAutoKeys((old) => {
    const name = `${now.current.gradeKey}/${node}`;
    const next = new Set(old);
    if (!next.delete(name)) next.add(name);
    return next;
  }), []);

  return useMemo(() => ({ scope, setScope, auto, toggleAuto, add, remove, removeAll, change, ease, step }),
    [scope, auto, toggleAuto, add, remove, removeAll, change, ease, step]);
}
