import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, mergePatch, type Corrector, type GradeNode, type Param, type Target } from '../engine';
import { fullCorrector, getAt, isKeyed, keyAt, patchOf, shownValue, withValue } from './corrector';
import type { KeyClock } from './keyFrames';
import type { GradeChange } from './nodeEdits';
import { addKeyframe, autoKeyed, autoValue, autoWhole, type AutoKeyed } from './nodeKeys';
import { windowPath, windowValue, type WindowEdit, type EditScope } from './windowEdits';

/**
 * What an edit puts at a path of the node's params: a number, which goes where an edit at the
 * playhead goes (into a value, or into a keyframe, p. 3391), or a value put there whole (the
 * points of a curve, a switch).
 */
export type EditValue = number | { whole: unknown } | WindowEdit;
export type Edits = readonly (readonly [string, EditValue])[];

/** The corrector node that the palettes edit, where the current node is one, and where its grade is. */
export interface EditedNode {
  bridge: Pick<EngineBridge, 'call'>;
  session?: number;
  /** The current clip matters even when two clips edit the same group/node. */
  clip?: string | null;
  /** Stable, serialized inputs of keyClockOf, not the freshly-created clock function. */
  clock?: string;
  /** Timeline sampling and manual seeks; frame below is the grade's source-key clock. */
  timelineFrame?: number;
  /** Continuous playback; false while a manually sought/scrubbed frame is held. */
  playing?: boolean;
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
  /**
   * Auto-Keyframe of the node's track in the Keyframe Editor (Reference Manual 20, p. 3383): a change
   * at a frame where the node has no keyframe first puts a dynamic keyframe of the node there
   * (`addKeyframe`), in the transaction of the change, and goes into it. Not while the timeline
   * plays: the controls do not follow the playhead then (p. 3384). Off where it is left out.
   */
  autoKey?: boolean;
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
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
/**
 * What edits put at their paths, worked out from `base` in their order. With `auto`, `base` is the
 * corrector with the keyframe that Auto-Keyframe puts first, and a value goes into that keyframe
 * (`autoValue`, `autoWhole`).
 */
const resolve = (edits: Edits, base: Corrector, frame: number, auto?: AutoKeyed | null): [string, unknown][] => {
  const values = new Map<string, unknown>();
  let current = base;
  for (const [path, edit] of edits) {
    const value = typeof edit === 'number' ? (auto ? autoValue(path, edit, current, auto, frame) : withValue(getAt(current, path) as Param | undefined, frame, edit))
      : 'window' in edit ? windowValue(current, edit, frame) : auto ? autoWhole(path, edit.whole, current, auto, frame) : edit.whole;
    if (typeof edit === 'object' && 'window' in edit && value === null) continue;
    values.set(path, value);
    current = mergePatch(current, patchOf([[path, value]])) as Corrector;
  }
  return [...values];
};
const scopeOf = (edits: Edits) => edits.reduce<EditScope | null>((scope, [, value]) => typeof value === 'object' && 'window' in value ? value.scope ?? scope : scope, null);
const validScope = (edits: Edits) => scopeOf(edits)?.live !== false;
/** A plain commit resolves by window id against its confirmed predecessor, so it outlives its control. */
const unscoped = (edits: Edits): Edits => edits.map(([path, value]) => typeof value === 'object' && 'window' in value ? [path, { window: value.window }] as const : [path, value] as const);
/** The actual key an edit changes; moving within its interval does not change ownership. */
const keysOf = (params: Corrector, frame: number, paths: readonly string[]) => JSON.stringify(paths.map(path => {
  const param = getAt(params, path) as Param | undefined;
  return [path, isKeyed(param) && Array.isArray(param.keys) && param.keys.length ? param.keys[keyAt(param.keys, frame)].frame : null];
}));
const pathsOf = (edits: Edits, base: Corrector) => edits.map(([path, value]) => typeof value === 'object' && 'window' in value ? windowPath(base, value) : path);
const identityOf = (at: EditedNode | null) => at === null ? '' : JSON.stringify([at.session, at.target, at.node.id, at.clip, at.clock, ...(at.autoKey ? [true] : [])]);
/**
 * What Auto-Keyframe makes of a change to `params` at a frame of the node's keys: the corrector with
 * the keyframe that the change puts first, and as it is shown; null where Auto-Keyframe is off, the
 * timeline plays, or the node has a keyframe at the frame already.
 */
const autoOf = (context: EditedNode | null, params: Corrector, at: number): AutoKeyed | null => {
  if (!context?.autoKey || context.playing || context.node.kind !== 'corrector') return null;
  const keyed = autoKeyed(params, at);
  return keyed === null ? null : { keyed, shown: params };
};
/** The keys that edits at these paths go into (`keysOf`): under Auto-Keyframe, those of the keyframe that the first of them puts. */
const keysAt = (context: EditedNode | null, params: Corrector, at: number, paths: readonly string[]) => keysOf(autoOf(context, params, at)?.keyed ?? params, at, paths);

interface Queued { edits: Edits; frame: number; keys: string; preview?: boolean }
interface Gesture {
  id: string;
  context: EditedNode;
  revision: number | null;
  frame: number;
  initial: Corrector;
  params: Corrector;
  before: Map<string, unknown>;
  keys: string;
  dependencies: string[];
  scope: EditScope | null;
  next: Edits | null;
  last: Map<string, unknown>;
  ending: boolean;
  ended: boolean;
  failed: boolean;
  made: boolean;
  cancelled: boolean;
  /** Under Auto-Keyframe: the corrector with the keyframe that the drag puts first, and as it was shown; null otherwise. */
  auto: AutoKeyed | null;
  /** The operations of that keyframe, which go with the first transaction of the drag; null once they went. */
  keying: unknown[] | null;
}
interface Owner {
  id: number;
  identity: string;
  context: EditedNode | null;
  committed: Corrector;
  /** The furthest revision confirmed by our responses, starting at the copy we began from. */
  confirmed: number;
  sending: boolean;
  retired: boolean;
  queue: Queued[];
  gesture: Gesture | null;
  live: Map<string, unknown>;
  held: Map<string, unknown>;
  local: number;
}
// Reload can reopen the same engine connection. This ID keeps the new renderer's first drag
// out of an unfinished gesture (and undo step) left by the previous renderer.
const renderer = crypto.randomUUID();
let gestures = 0;

/** Edits and replies belong to one owner; a gesture keeps its first revision until its end. */
export function useNodeEdit(edited: EditedNode | null) {
  const [, redraw] = useState(0);
  const alive = useRef(true);
  const serial = useRef(0);
  const active = useRef<Owner | null>(null);
  const retired = useRef<Owner[]>([]);
  const committed = fullCorrector(edited?.node.kind === 'corrector' ? edited.node.params : undefined);
  const makeOwner = (): Owner => ({ id: ++serial.current, identity: identityOf(edited), context: edited, committed,
    confirmed: edited?.revision ?? 0, sending: false, retired: false, queue: [], gesture: null,
    live: new Map(), held: new Map(), local: 0 });
  const retire = (owner: Owner, finish: boolean) => {
    owner.retired = true;
    owner.queue = [];
    if (owner.gesture) {
      owner.gesture.next = null;
      owner.gesture.ending = true;
      owner.gesture.cancelled = false;
      if (!finish) owner.gesture.failed = true;
    }
    retired.current.push(owner);
  };
  let owner = active.current;
  const previous = owner?.context;
  const seek = previous && edited && (previous.timelineFrame ?? previous.frame) !== (edited.timelineFrame ?? edited.frame)
    && !(previous.playing && edited.playing);
  const changedKey = owner?.gesture && edited && owner.gesture.keys !== keysAt(edited, committed, edited.frame, owner.gesture.dependencies);
  const external = owner && edited && !owner.sending && edited.revision > owner.confirmed;
  if (!owner || owner.identity !== identityOf(edited) || seek || (changedKey && !owner.gesture?.scope) || external) {
    if (owner) retire(owner, !external);
    owner = makeOwner();
    active.current = owner;
  } else if (changedKey && owner.gesture?.scope?.live) {
    // A Window key crossing changes the control, not this node's in-flight transaction lane.
    // Finish its accepted value before the new scope can use the confirmed ProjectCopy.
    // A released value is kept: it resolves at the gesture's own frame, so its key is the captured one.
    owner.gesture.scope.live = false;
    if (!owner.gesture.ending) owner.gesture.next = null;
    owner.live.clear();
  }
  owner.context = edited;
  owner.committed = committed;
  // A queued value keeps its original key. A static value survives continuous playback.
  owner.queue = owner.queue.filter(item => {
    const valid = edited && validScope(item.edits) && item.keys === keysAt(edited, committed, edited.frame, pathsOf(item.edits, committed));
    if (!valid) for (const [path] of item.edits) owner.held.delete(path);
    return valid;
  });
  for (const [path, value] of owner.held) if (same(getAt(committed, path), value)) owner.held.delete(path);
  const current = (who: Owner) => alive.current && active.current === who && !who.retired;
  const refresh = (who: Owner) => { if (current(who)) redraw(n => n + 1); };
  const discardExternal = (who: Owner) => {
    if (!current(who) || who.context === null || who.context.revision <= who.confirmed) return false;
    // The response has now identified our progress; anything beyond it belongs to someone else.
    retire(who, false);
    active.current = null;
    redraw(n => n + 1);
    return true;
  };

  const drive = (who: Owner): void => {
    if (!alive.current || who.sending || who.context === null) return;
    let going = who.gesture;
    // A retired control drops its unsent continuation, never a release or Escape already asked for.
    if (going?.scope?.live === false && !going.ended && !going.ending) {
      going.next = null;
      if (going.revision === null || going.failed) { who.gesture = null; going = null; }
      else {
        going.ending = true; going.cancelled = false;
        going.next = [...going.last].map(([path, whole]) => [path, { whole }] as const);
      }
    }
    if (who.retired) {
      if (!going || going.failed || going.ended || going.revision === null || going.last.size === 0) return;
      going.next = [...going.last].map(([path, whole]) => [path, { whole }] as const);
    } else if (discardExternal(who)) return;
    if (going?.failed) return;
    // Ordinary commits wait for their predecessor's acknowledged revision in the project copy.
    // No timer clears this gate: a late copy is not permission to rebase onto a new revision.
    if ((!going || going.revision === null) && who.context.revision < who.confirmed) return;
    let context: EditedNode;
    let revision: number;
    let values: [string, unknown][];
    let ops: unknown[];
    let end = false;
    let gesture: Gesture | null = null;
    const queued = !who.retired && (!going || going.revision === null) ? who.queue.shift() : undefined;
    if (queued && !validScope(queued.edits)) { drive(who); return; }
    if (queued?.preview) {
      who.gesture = makeGesture(who.context, who.committed, queued.edits, queued.frame);
      drive(who); return;
    }
    if (queued) {
      context = who.context;
      revision = context.revision;
      // Auto-Keyframe: the keyframe of the node first, then the change, which goes into it; one step to undo.
      const auto = autoOf(context, who.committed, queued.frame);
      values = resolve(queued.edits, auto?.keyed ?? who.committed, queued.frame, auto);
      ops = [...(context.before ?? []), ...(auto ? addKeyframe(context.target, context.node, queued.frame, 'dynamic') : []),
        { op: 'set_node', grade: context.target, id: context.node.id, params: patchOf(values) }];
    } else {
      if (!going || going.next === null || going.ended) return;
      gesture = going;
      if (going.revision === null) {
        going.context = who.context;
        going.revision = who.context.revision;
        // A preceding ordinary commit may have completed while this drag waited (and its
        // optimistic display may have expired). Escape restores this confirmed starting state.
        going.initial = who.committed;
        going.params = who.committed;
        for (const path of going.before.keys()) going.before.set(path, getAt(going.initial, path));
        going.keys = keysAt(going.context, going.initial, going.frame, going.dependencies);
        // Auto-Keyframe: the drag goes on from the corrector with its keyframe, which its first transaction puts.
        going.auto = autoOf(going.context, going.initial, going.frame);
        going.keying = going.auto ? addKeyframe(going.context.target, going.context.node, going.frame, 'dynamic') : null;
        if (going.auto) going.params = going.auto.keyed;
      }
      context = going.context;
      revision = going.revision;
      end = going.ending;
      values = resolve(going.next, going.params, going.frame, going.auto);
      going.next = null;
      going.ended = end;
      // A drag taken back that put a keyframe puts the settings back whole, so the keyframe goes with it.
      const back = end && going.cancelled && going.auto !== null;
      const set = { op: 'set_node', grade: context.target, id: context.node.id, params: back ? going.initial : patchOf(values) };
      const keying = going.keying ?? [];
      going.keying = null;
      const making = !going.made && (context.before?.length ?? 0) > 0;
      ops = making ? [...context.before!, ...keying, set] : end && going.cancelled && going.made
        ? [{ op: 'remove_node', grade: context.target, id: context.node.id }] : [...keying, set];
      if (making) going.made = true;
    }
    who.sending = true;
    void callEngine(context.bridge, 'transaction.apply', {
      revision, ops, ...(gesture ? { gesture: { id: gesture.id, ...(end ? { end: true } : {}) } } : {}),
    }, context.session === undefined ? undefined : { expectedSession: context.session }).then(applied => {
      if (typeof applied?.revision === 'number') who.confirmed = Math.max(who.confirmed, applied.revision);
      if (gesture) {
        gesture.params = mergePatch(gesture.params, patchOf(values)) as Corrector;
        for (const [path, value] of values) gesture.last.set(path, value);
      }
    }).catch((failure: unknown) => {
      // A drag waiting behind a refused ordinary edit was prepared from that edit too.
      if (who.gesture) who.gesture.failed = true;
      who.queue = [];
      if (!current(who)) return;
      who.live.clear(); who.held.clear();
      context.onFailure(failure);
    }).finally(() => {
      who.sending = false;
      if (!alive.current) return;
      if (gesture?.ended && who.gesture === gesture) who.gesture = null;
      if (!who.retired && discardExternal(who)) return;
      refresh(who);
      drive(who);
    });
  };

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    for (const old of retired.current) drive(old);
    retired.current = retired.current.filter(old => old.sending);
    drive(owner);
  });
  const heldStamp = JSON.stringify([...owner.held]);
  useEffect(() => {
    if (!owner.held.size) return;
    const timer = setTimeout(() => { if (current(owner)) { owner.held.clear(); refresh(owner); } }, HOLD_MS);
    return () => clearTimeout(timer);
  }, [owner, heldStamp]);

  const base = mergePatch(committed, patchOf(owner.held)) as Corrector;
  const shown = mergePatch(base, patchOf(owner.live)) as Corrector;
  const frame = edited?.frame ?? 0;
  // Also reject callbacks retained by callers across a key change, even before React unmounts a control.
  const accepts = (edits: Edits) => validScope(edits) && current(owner) && owner.context?.node.kind === 'corrector'
    && keysAt(edited, committed, frame, pathsOf(edits, committed)) === keysAt(owner.context, owner.committed, owner.context.frame, pathsOf(edits, owner.committed));
  const makeGesture = (context: EditedNode, initial: Corrector, edits: Edits, at: number): Gesture => ({
    id: `palette-${renderer}-${++gestures}`, context, revision: null, frame: at, initial, params: initial,
    before: new Map(edits.map(([path]) => [path, getAt(initial, path)])),
    dependencies: pathsOf(edits, initial), scope: scopeOf(edits), keys: keysAt(context, initial, at, pathsOf(edits, initial)),
    next: edits, last: new Map(), ending: false, ended: false, failed: false, made: false, cancelled: false, auto: null, keying: null,
  });
  const enqueue = (edits: Edits, preview = false) => {
    const scope = scopeOf(edits);
    const waiting = owner.queue.find(item => item.preview && scopeOf(item.edits) === scope);
    // Retiring a control drops its queued previews only; a commit keeps its place and key.
    const queued = preview ? edits : unscoped(edits);
    if (waiting) { waiting.edits = queued; waiting.preview = preview; }
    else owner.queue.push({ edits: queued, frame, keys: keysAt(edited, committed, frame, pathsOf(edits, committed)), preview });
  };
  const preview = (edits: Edits) => {
    if (!accepts(edits) || !edits.length) return;
    owner.local += 1;
    // Under Auto-Keyframe the values are shown as they will be sent: in the keyframe that the change puts.
    const auto = autoOf(owner.context, base, frame);
    const values = resolve(edits, auto?.keyed ?? base, frame, auto);
    let going = owner.gesture;
    if (!going || going.ended) {
      if ((auto ? resolve(edits, base, frame) : values).every(([path, value]) => same(getAt(base, path), value))) return;
      going = makeGesture(owner.context!, base, edits, frame);
      owner.gesture = going;
    }
    if (going.scope !== scopeOf(edits)) {
      enqueue(edits, true); owner.live = new Map(values); refresh(owner); drive(owner); return;
    }
    if (going.failed) return;
    going.dependencies = [...new Set([...going.dependencies, ...pathsOf(edits, going.initial)])];
    for (const [path] of edits) if (!going.before.has(path)) going.before.set(path, getAt(going.initial, path));
    going.keys = keysAt(going.context, going.initial, going.frame, going.dependencies);
    going.next = edits;
    owner.live = new Map(values);
    refresh(owner); drive(owner);
  };
  const commit = (edits: Edits) => {
    if (!accepts(edits)) return;
    owner.local += 1;
    const going = owner.gesture;
    owner.live.clear();
    if (going && !going.ended && going.scope !== scopeOf(edits)) {
      enqueue(edits); for (const [path, value] of resolve(edits, base, frame)) owner.held.set(path, value);
      refresh(owner); drive(owner); return;
    }
    if (going && !going.ended) {
      if (going.failed) { owner.gesture = null; refresh(owner); return; }
      if (going.revision !== null) {
        going.next = edits.length ? edits : [...going.last].map(([path, whole]) => [path, { whole }] as const);
        going.ending = true;
        for (const [path, value] of resolve(going.next, going.params, going.frame, going.auto)) owner.held.set(path, value);
        refresh(owner); drive(owner); return;
      }
      owner.gesture = null;
    }
    if (!edits.length) { refresh(owner); return; }
    const values = resolve(edits, base, frame);
    if (values.every(([path, value]) => same(getAt(base, path), value))) { refresh(owner); return; }
    // Under Auto-Keyframe what is held is what will be sent: the values in the keyframe that the change puts.
    const auto = autoOf(owner.context, base, frame);
    for (const [path, value] of auto ? resolve(edits, auto.keyed, frame, auto) : values) owner.held.set(path, value);
    enqueue(edits);
    refresh(owner); drive(owner);
  };
  const cancel = (scope?: EditScope) => {
    if (!current(owner)) return;
    if (scope && owner.gesture?.scope !== scope) {
      const waiting = owner.queue.some(item => item.preview && scopeOf(item.edits) === scope);
      owner.queue = owner.queue.filter(item => !item.preview || scopeOf(item.edits) !== scope);
      if (waiting) { owner.live.delete('windows'); refresh(owner); }
      return;
    }
    owner.local += 1;
    owner.live.clear(); owner.held.clear();
    const going = owner.gesture;
    if (going && !going.ended) {
      if (going.failed || going.revision === null) owner.gesture = null;
      else {
        going.next = [...going.before].map(([path, whole]) => [path, { whole }] as const);
        going.ending = true; going.cancelled = true;
        drive(owner);
      }
    }
    refresh(owner);
  };
  return {
    enabled: edited?.node.kind === 'corrector', corrector: shown, base, frame,
    interaction: owner.id,
    controlKey: (paths: readonly string[]) => `${owner.id}:${keysAt(edited, committed, frame, paths)}`,
    isCurrent: () => current(owner),
    localVersion: () => owner.local,
    touch: () => { if (current(owner)) owner.local += 1; },
    pending: () => owner.sending || owner.queue.length > 0 || owner.gesture !== null || (owner.context?.revision ?? 0) < owner.confirmed,
    value: (path: string) => shownValue(getAt(shown, path) as Param | undefined, frame),
    keyed: (path: string) => isKeyed(getAt(shown, path) as Param | undefined),
    retireScope: (scope: EditScope) => {
      scope.live = false;
      if (!current(owner)) return;
      owner.queue = owner.queue.filter(item => scopeOf(item.edits) !== scope);
      if (owner.gesture?.scope === scope) owner.live.clear();
      refresh(owner); drive(owner);
    },
    preview, commit, cancel,
  };
}
export type NodeEdit = ReturnType<typeof useNodeEdit>;
