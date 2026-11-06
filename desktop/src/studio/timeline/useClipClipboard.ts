import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { Key } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { clipboardPlacement, type ClipboardPlacement, type ClipClipboard } from './clipboard';
import type { Targets, Transaction } from './commands';
import type { Marks } from './snap';
import type { TrackMenu } from './SubtitleTrack';
type Bridge = Pick<EngineBridge, 'call'>;
type Saved = {
  session: number;
  clipboard: ClipClipboard;
  shift: number;
  visible: boolean;
};
class ClipboardStore {
  value: Saved | null = null;
  listeners = new Set<() => void>();
  read = () => this.value;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  write(value: Saved | null) {
    this.value = value;
    for (const listener of this.listeners)
      listener();
  }
}
const stores = new WeakMap<Bridge, ClipboardStore>();
const storeFor = (bridge: Bridge) => {
  let store = stores.get(bridge);
  if (!store) {
    store = new ClipboardStore();
    stores.set(bridge, store);
  }
  return store;
};
interface Options {
  bridge: Bridge;
  session: number;
  revision: number;
  timeline: Timeline | null;
  frame: number;
  selected: readonly string[];
  linked: boolean;
  marks: Marks;
  targets: Targets;
  pending: boolean;
  apply: (transaction: Transaction) => Promise<boolean>;
  problem: (error: unknown) => void;
  copied?: (ids: readonly string[]) => boolean;
}
type Side = 'head' | 'tail';
const NO_GHOSTS: ClipboardPlacement[] = [];
/** Shared by Edit/Cut/Fairlight. Every mutation still goes through the timeline's edit queue. */
export function useClipClipboard(options: Options) {
  const t = useFairlightText();
  const store = useMemo(() => storeFor(options.bridge), [options.bridge]);
  const saved = useSyncExternalStore(store.subscribe, store.read);
  const [reading, setReading] = useState(false);
  const request = useRef(0), active = useRef(false), live = useRef(true);
  const now = useRef(options);
  now.current = options;
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      request.current++;
    };
  }, []);
  useEffect(() => {
    request.current++;
    active.current = false;
    setReading(false);
    if (store.value && store.value.session !== options.session)
      store.write(null);
  }, [store, options.session, options.timeline?.id]);
  const current = saved?.session === options.session ? saved : null;
  // Where a paste would put the clips, worked out again only when one of its inputs changes, so
  // that the ghosts shown while the clipboard is visible keep their clips (and are not drawn
  // again) through renders of anything else.
  const clipboard = current?.clipboard, shift = current?.shift ?? 0;
  const placement = useMemo(() => clipboard && options.timeline
    ? clipboardPlacement(clipboard, options.timeline, options.frame, shift) : null,
  [clipboard, options.timeline, options.frame, shift]);
  const selection = (context?: string, side?: Side) => {
    const state = now.current;
    const ids = context && !state.selected.includes(context) ? [context] : [...state.selected];
    const all = state.timeline?.tracks.flatMap(track => track.kind === 'subtitle' ? [] : track.clips.map(clip => ({ track, clip }))) ?? [];
    const range = !side && state.marks.in !== null && state.marks.out !== null && state.marks.in < state.marks.out
      ? { from: state.marks.in, to: state.marks.out } : undefined;
    const target = new Set([state.targets.audio_track, state.targets.video_track]);
    let chosen = all.filter(({ track, clip }) => ids.length ? ids.includes(clip.id) : Boolean((side || range) && target.has(track.id) && !track.locked));
    if (side)
      chosen = chosen.filter(({ clip }) => side === 'head'
        ? clip.start < state.frame && state.frame <= clip.start + clip.length
        : clip.start <= state.frame && state.frame < clip.start + clip.length);
    if (range)
      chosen = chosen.filter(({ clip }) => clip.start < range.to && clip.start + clip.length > range.from);
    return {
      clips: chosen.map(({ clip }) => clip.id), range: side && chosen.length
        ? {
          from: side === 'head' ? Math.min(...chosen.map(({ clip }) => clip.start)) : state.frame,
          to: side === 'tail' ? Math.max(...chosen.map(({ clip }) => clip.start + clip.length)) : state.frame
        } : range
    };
  };
  const copy = async (cut: boolean, context?: string, side?: Side) => {
    const state = now.current, chosen = selection(context, side);
    if (!state.timeline || !chosen.clips.length || active.current || state.pending)
      return;
    const ticket = ++request.current;
    const valid = () => live.current && ticket === request.current && state.session === now.current.session && state.timeline?.id === now.current.timeline?.id;
    active.current = true;
    setReading(true);
    try {
      const result = await callEngine(state.bridge, 'clips.copy', {
        timeline: state.timeline.id, revision: state.revision,
        clips: chosen.clips, at: state.frame, ...chosen.range, linked: state.linked, grouped: true
      }, { expectedSession: state.session });
      if (!valid())
        return;
      if (cut && !await state.apply({
        revision: result.revision, ops: [{
          edit: 'cut_clips', timeline: state.timeline.id,
          clips: chosen.clips, ...chosen.range, linked: state.linked, grouped: true
        }]
      }))
        return;
      if (!valid())
        return;
      store.write({
        session: state.session, clipboard: result.clipboard, shift: 0, visible: true
      });
      if (!cut)
        state.copied?.(chosen.clips);
    }
    catch (error) {
      if (valid())
        now.current.problem(error);
    }
    finally {
      if (ticket === request.current) {
        active.current = false;
        if (live.current)
          setReading(false);
      }
    }
  };
  const paste = async () => {
    const state = now.current, content = store.value;
    if (!state.timeline || content?.session !== state.session || active.current || state.pending)
      return;
    const destination = clipboardPlacement(content.clipboard, state.timeline, state.frame, content.shift);
    if (!destination) {
      state.problem(new Error(t('The destination has too few tracks.')));
      return;
    }
    const done = await state.apply({
      revision: state.revision, ops: [{
        edit: 'paste_clips', timeline: state.timeline.id,
        clipboard: content.clipboard, at: state.frame, tracks: destination.tracks
      }]
    });
    if (done && store.value === content && live.current && now.current.session === state.session)
      store.write({ ...content, visible: false });
  };
  const keyDown = (key: Key) => {
    if (key.composing || key.repeat)
      return false;
    if (key.key === 'Escape' && current?.visible) {
      store.write({ ...current, visible: false });
      return true;
    }
    if (!(key.ctrl || key.meta) || key.shift)
      return false;
    if (key.alt && (key.key === 'ArrowUp' || key.key === 'ArrowDown') && current && options.timeline) {
      const shift = current.shift + (key.key === 'ArrowDown' ? 1 : -1);
      if (clipboardPlacement(current.clipboard, options.timeline, options.frame, shift))
        store.write({ ...current, shift, visible: true });
      return true;
    }
    if (key.alt)
      return false;
    switch (key.key.toLowerCase()) {
      case 'c':
      case 'x':
        if (!selection().clips.length)
          return false;
        void copy(key.key.toLowerCase() === 'x');
        return true;
      case 'v':
        if (!current)
          return false;
        void paste();
        return true;
      default: return false;
    }
  };
  const items = (context?: string): TrackMenu['items'] => {
    const blocked = reading || options.pending;
    return [
      {
        id: 'cut-clips', label: t('Cut'), disabled: blocked || !selection(context).clips.length, run: () => void copy(true, context)
      },
      {
        id: 'copy-clips', label: t('Copy'), disabled: blocked || !selection(context).clips.length, run: () => void copy(false, context)
      },
      {
        id: 'paste-clips', label: t('Paste'), disabled: blocked || !current || !placement, run: () => void paste()
      },
      ...(['head', 'tail'] as const).flatMap(side => [
        {
          id: `cut-${side}`, label: t(side === 'head' ? 'Cut Head' : 'Cut Tail'), disabled: blocked || !selection(context, side).clips.length, run: () => void copy(true, context, side)
        },
        {
          id: `copy-${side}`, label: t(side === 'head' ? 'Copy Head' : 'Copy Tail'), disabled: blocked || !selection(context, side).clips.length, run: () => void copy(false, context, side)
        },
      ]),
    ];
  };
  return {
    keyDown, items, reading, ghosts: current?.visible && placement ? placement.clips : NO_GHOSTS
  };
}
