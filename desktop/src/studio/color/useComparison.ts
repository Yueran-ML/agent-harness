/**
 * Wipes and split screens of the Color page's viewer (Reference Manual 20, pp. 2999–3006,
 * 3260–3261; the interaction reference, §9.3), as the page asks the engine for them: each change
 * sends the whole wipe again (`viewer.wipe`), or the whole split screen (`viewer.split`), and what
 * the engine answers is what the viewer compares now.
 *
 * - Image Wipe and Ctrl+W turn the wipe on and off: against the still chosen in the Gallery
 *   (Reference Wipe Mode Gallery, the default) or the clip of a Wipe Timeline Clip (Timeline).
 *   Alt+W inverts it. A double-click of a still wipes it.
 * - The border is dragged on the picture: while a wipe that has one is on, the view takes the
 *   pointer (`view.pointer`), and each move puts the border where the pointer is; Alt turns a
 *   diagonal one (p. 3000).
 * - A timeline wipe with Gang on keeps its distance from the current clip as another becomes
 *   current (p. 3002); Ctrl+Alt+Left and Right step its clip a frame.
 * - Ctrl+Alt+W turns the split screen on and off in the mode chosen last. While it is on, a click
 *   on a tile chooses it, and the controls drawn on the picture are hidden (p. 3003).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import {
  callEngine, viewPointerOf, type Clip, type Compared, type PointerCursor, type Timeline, type ViewerReference, type ViewPointer,
  type WipeSettings, type WipeStyle,
} from '../engine';
import {
  DEFAULT_SPLIT, DEFAULT_WIPE, referenceClip, splitItems, tileAt, timelineReference, wipeAngleAt, wipeCursor, wipeParams, wipePositionAt,
  type ReferenceMode, type SplitMode, type TimelineWipe,
} from './comparison';

export interface Comparison {
  /** What the viewer compares now, as the engine answered. */
  compared: Compared;
  wipe: WipeSettings;
  referenceMode: ReferenceMode;
  gang: boolean;
  splitMode: SplitMode;
  outline: boolean;
  /** Whether a wipe or a split screen is on. */
  wiping: boolean;
  splitting: boolean;
  /** The still that the wipe shows, where it shows one. */
  wipedStill: string | null;
  /** The clip of the thumbnail timeline that a timeline reference shows now, outlined in blue (p. 3261). */
  referenceClip: string | null;
  /** Wipe On/Off (Ctrl+W, Image Wipe). */
  toggleWipe: () => void;
  /** Wipe this still, from the Gallery (a double-click, p. 3260): again, and the wipe goes off. */
  wipeStill: (still: string) => void;
  /** Show this still in the wipe that is on, where it shows one (Previous / Next Still, p. 3260). */
  showStill: (still: string) => void;
  /** Wipe Timeline Clip (p. 3002): a clip other than the current one. */
  wipeTimelineClip: (clip: string) => void;
  /** Ctrl+Alt+Left and Right: the clip of a timeline wipe a frame back or on (p. 3002). */
  stepReference: (by: -1 | 1) => void;
  setStyle: (style: WipeStyle) => void;
  invert: () => void;
  setReferenceMode: (mode: ReferenceMode) => void;
  setGang: (gang: boolean) => void;
  /** Split Screen (Ctrl+Alt+W): on in the mode chosen last, or off. */
  toggleSplit: () => void;
  setSplitMode: (mode: SplitMode) => void;
  setOutline: (outline: boolean) => void;
}

const OFF: Compared = { mode: 'off' };

export function useComparison({ bridge, session, native, loaded, timeline, clips, current, frame, playing, chosen, stills, currentStill, onGo,
  onProblem, onPointerMenu }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  /** Whether the shell lays the engine's view over the viewer: only then is there a picture to drag on. */
  native: boolean;
  /** Whether the timeline monitor has the timeline loaded. */
  loaded: boolean;
  timeline: Timeline | null;
  /** The clips of the thumbnail timeline, in its order. */
  clips: readonly Clip[];
  current: Clip | null;
  frame: number;
  playing: boolean;
  /** The clips chosen in the thumbnail timeline. */
  chosen: ReadonlySet<string>;
  /** The stills chosen in the Gallery, in its order, and the one chosen last. */
  stills: readonly string[];
  currentStill: string | null;
  /** Go to a clip, from a tile of Selected Clips (p. 3005): it is chosen, and the playhead goes to its first frame. */
  onGo: (clip: string) => void;
  onProblem: (failure: unknown) => void;
  /** The right button pressed on the picture while the view takes the pointer: the viewer's menu, at that point of the view's window. */
  onPointerMenu?: (at: { x: number; y: number }) => void;
}): Comparison {
  const [compared, setCompared] = useState<Compared>(OFF);
  const [wipe, setWipe] = useState<WipeSettings>(DEFAULT_WIPE);
  const [referenceMode, setReferenceModeState] = useState<ReferenceMode>('gallery');
  const [gang, setGangState] = useState(false);
  const [splitMode, setSplitModeState] = useState<SplitMode>(DEFAULT_SPLIT);
  const [outline, setOutlineState] = useState(true);
  const [timelineWipe, setTimelineWipe] = useState<TimelineWipe | null>(null);
  // Of a ganged timeline wipe: how many clips from the current one its clip is, and how many frames from the playhead's offset.
  const [ganged, setGanged] = useState<{ distance: number; shift: number } | null>(null);
  // The reference sent last, of a wipe, with the clip that it names, for the blue outline.
  const sentReference = useRef<{ reference: ViewerReference; clip: string | null } | null>(null);
  const [splitShown, setSplitShown] = useState<{ selected: number; clips: (string | null)[] } | null>(null);

  const now = useRef({ bridge, compared, wipe, referenceMode, gang, splitMode, outline, timelineWipe, ganged, clips, current, frame, timeline, chosen, stills,
    currentStill, onGo, onProblem, onPointerMenu, splitShown });
  now.current = { bridge, compared, wipe, referenceMode, gang, splitMode, outline, timelineWipe, ganged, clips, current, frame, timeline, chosen, stills,
    currentStill, onGo, onProblem, onPointerMenu, splitShown };

  // One call at a time; of those asked meanwhile only the last is sent, so that a drag sends as fast as the engine answers.
  const sending = useRef(false);
  const pending = useRef<{ method: 'viewer.wipe' | 'viewer.split'; params: Record<string, unknown> } | null>(null);
  const live = useRef(session);
  live.current = session;
  const send = useCallback((method: 'viewer.wipe' | 'viewer.split', params: Record<string, unknown>) => {
    pending.current = { method, params };
    if (sending.current) return;
    const next = async () => {
      const asked = pending.current;
      pending.current = null;
      if (asked === null) { sending.current = false; return; }
      sending.current = true;
      const from = live.current;
      try {
        const answer = await callEngine(now.current.bridge, asked.method, asked.params);
        if (from === live.current && answer && typeof answer === 'object' && 'mode' in answer) setCompared(answer);
      } catch (failure) {
        if (from === live.current) now.current.onProblem(failure);
      }
      await next();
    };
    void next();
  }, []);

  // Another engine, or another timeline: nothing is compared there.
  useEffect(() => {
    setCompared(OFF); setTimelineWipe(null); setGanged(null); setSplitShown(null); sentReference.current = null; pending.current = null;
  }, [session, timeline?.id]);
  useEffect(() => { if (!loaded) setCompared(OFF); }, [loaded]);

  const wiping = compared.mode === 'wipe';
  const splitting = compared.mode === 'split';
  const wipedStill = compared.mode === 'wipe' && compared.wipe.reference.kind === 'still' ? compared.wipe.reference.still : null;

  /** Send the wipe whole, with this reference, or that of the mode chosen. */
  const sendWipe = useCallback((settings: WipeSettings, reference: ViewerReference, clip: string | null = null) => {
    sentReference.current = { reference, clip };
    send('viewer.wipe', wipeParams(settings, reference));
  }, [send]);

  /** The reference of a timeline wipe now: with Gang on, from the current clip. */
  const timelineReferenceNow = useCallback((): { reference: ViewerReference; clip: string | null } | null => {
    const state = now.current;
    if (state.timelineWipe === null) return null;
    const reference = timelineReference(state.timelineWipe, state.gang ? state.ganged : null, state.clips, state.current, state.frame);
    if (reference === null) return null;
    const at = state.current === null ? -1 : state.clips.findIndex(clip => clip.id === state.current!.id);
    const clip = reference.kind === 'clip' ? reference.clip
      : reference.kind === 'neighbour' ? state.clips[at + (reference.side === 'previous' ? -1 : 1)]?.id ?? null : null;
    return { reference, clip };
  }, []);

  /** The reference that a wipe turned on shows: the still chosen in the Gallery, or the clip of the timeline wipe. */
  const referenceNow = useCallback((): { reference: ViewerReference; clip: string | null } | null => {
    const state = now.current;
    if (state.referenceMode === 'timeline') return timelineReferenceNow();
    return state.currentStill === null ? null : { reference: { kind: 'still', still: state.currentStill }, clip: null };
  }, [timelineReferenceNow]);

  const off = useCallback(() => {
    const { compared: shown } = now.current;
    if (shown.mode === 'wipe') send('viewer.wipe', { on: false });
    else if (shown.mode === 'split') send('viewer.split', { on: false });
  }, [send]);

  const toggleWipe = useCallback(() => {
    if (now.current.compared.mode === 'wipe') { off(); return; }
    const found = referenceNow();
    if (found !== null) sendWipe(now.current.wipe, found.reference, found.clip);
  }, [off, referenceNow, sendWipe]);

  const wipeStill = useCallback((still: string) => {
    const state = now.current;
    if (state.compared.mode === 'wipe' && state.compared.wipe.reference.kind === 'still' && state.compared.wipe.reference.still === still) { off(); return; }
    setReferenceModeState('gallery');
    sendWipe(state.wipe, { kind: 'still', still });
  }, [off, sendWipe]);

  const showStill = useCallback((still: string) => {
    const state = now.current;
    if (state.compared.mode === 'wipe' && state.compared.wipe.reference.kind === 'still') sendWipe(state.wipe, { kind: 'still', still });
  }, [sendWipe]);

  const wipeTimelineClip = useCallback((clip: string) => {
    const state = now.current;
    const target = state.clips.find(each => each.id === clip);
    if (!target || state.current === null || clip === state.current.id) return;
    const offset = state.frame - state.current.start;
    const chosenWipe = { clip, frame: Math.max(0, Math.min(target.length - 1, offset)) };
    const distance = state.clips.findIndex(each => each.id === clip) - state.clips.findIndex(each => each.id === state.current!.id);
    const gangedNow = { distance, shift: chosenWipe.frame - offset };
    setTimelineWipe(chosenWipe); setGanged(gangedNow); setReferenceModeState('timeline');
    const reference = timelineReference(chosenWipe, state.gang ? gangedNow : null, state.clips, state.current, state.frame);
    if (reference !== null) sendWipe(state.wipe, reference.kind === 'neighbour' ? reference : { kind: 'clip', clip, frame: chosenWipe.frame }, clip);
  }, [sendWipe]);

  const stepReference = useCallback((by: -1 | 1) => {
    const state = now.current;
    if (state.compared.mode !== 'wipe' || state.referenceMode !== 'timeline' || state.timelineWipe === null) return;
    if (state.gang && state.ganged !== null) {
      const next = { ...state.ganged, shift: state.ganged.shift + by };
      setGanged(next);
      now.current.ganged = next;
    } else {
      const clip = state.clips.find(each => each.id === state.timelineWipe!.clip);
      const next = { ...state.timelineWipe, frame: Math.max(0, Math.min((clip?.length ?? 1) - 1, state.timelineWipe.frame + by)) };
      setTimelineWipe(next);
      now.current.timelineWipe = next;
    }
    const found = timelineReferenceNow();
    if (found !== null) sendWipe(state.wipe, found.reference, found.clip);
  }, [sendWipe, timelineReferenceNow]);

  /** A setting of the wipe changed: kept, and sent with the reference shown where it is on. */
  const changeWipe = useCallback((settings: WipeSettings) => {
    setWipe(settings);
    now.current.wipe = settings;
    const state = now.current;
    if (state.compared.mode === 'wipe' && sentReference.current !== null) sendWipe(settings, sentReference.current.reference, sentReference.current.clip);
  }, [sendWipe]);
  const setStyle = useCallback((style: WipeStyle) => changeWipe({ ...now.current.wipe, style }), [changeWipe]);
  const invert = useCallback(() => changeWipe({ ...now.current.wipe, invert: !now.current.wipe.invert }), [changeWipe]);

  const setReferenceMode = useCallback((mode: ReferenceMode) => {
    setReferenceModeState(mode);
    now.current.referenceMode = mode;
    if (now.current.compared.mode !== 'wipe') return;
    const found = referenceNow();
    if (found !== null) sendWipe(now.current.wipe, found.reference, found.clip);
  }, [referenceNow, sendWipe]);

  const setGang = useCallback((on: boolean) => {
    const state = now.current;
    setGangState(on);
    now.current.gang = on;
    // Ganged from where the timeline wipe is now.
    if (on && state.timelineWipe !== null && state.current !== null) {
      const distance = state.clips.findIndex(each => each.id === state.timelineWipe!.clip) - state.clips.findIndex(each => each.id === state.current!.id);
      const next = { distance, shift: state.timelineWipe.frame - (state.frame - state.current.start) };
      setGanged(next);
      now.current.ganged = next;
    }
    if (!on && state.ganged !== null && state.current !== null) {
      // The clip it shows now stays where it is.
      const reference = timelineReference(state.timelineWipe ?? { clip: '', frame: 0 }, state.ganged, state.clips, state.current, state.frame);
      if (reference !== null && reference.kind === 'clip') { setTimelineWipe({ clip: reference.clip, frame: reference.frame ?? 0 }); now.current.timelineWipe = { clip: reference.clip, frame: reference.frame ?? 0 }; }
      if (reference !== null && reference.kind === 'neighbour') {
        const at = state.clips.findIndex(each => each.id === state.current!.id);
        const clip = state.clips[at + (reference.side === 'previous' ? -1 : 1)];
        if (clip) {
          const kept = { clip: clip.id, frame: Math.max(0, Math.min(clip.length - 1, state.frame - state.current.start)) };
          setTimelineWipe(kept); now.current.timelineWipe = kept;
        }
      }
    }
    if (state.compared.mode === 'wipe' && state.referenceMode === 'timeline') {
      const found = timelineReferenceNow();
      if (found !== null) sendWipe(state.wipe, found.reference, found.clip);
    }
  }, [sendWipe, timelineReferenceNow]);

  // With Gang on, another current clip takes the reference along (p. 3002); a clip reference is moved with a playhead that is moved by hand.
  const gangKey = gang && wiping && referenceMode === 'timeline' ? `${current?.id ?? ''}:${playing ? '' : frame}` : null;
  useEffect(() => {
    if (gangKey === null) return;
    const found = timelineReferenceNow();
    if (found === null) return;
    const shown = sentReference.current?.reference;
    if (shown && JSON.stringify(shown) === JSON.stringify(found.reference)) return;
    sendWipe(now.current.wipe, found.reference, found.clip);
  }, [gangKey]);

  // A split screen: its items, made again from what is chosen as that changes.
  const splitNow = useMemo(() => splitItems(splitMode, { clips, chosen, current, stills, frame }), [splitMode, clips, chosen, current, stills,
    splitMode === 'neighbor_clips' && !playing ? frame : 0]);
  const sendSplit = useCallback((shown: ReturnType<typeof splitItems>, selected = shown.selected) => {
    setSplitShown({ selected, clips: shown.clips });
    send('viewer.split', { on: true, items: shown.items, selected, outline: now.current.outline });
  }, [send]);
  const splitKey = splitting ? JSON.stringify(splitNow.items) : null;
  useEffect(() => { if (splitKey !== null) sendSplit(splitNow); }, [splitKey]);

  const toggleSplit = useCallback(() => {
    if (now.current.compared.mode === 'split') { off(); return; }
    sendSplit(splitItems(now.current.splitMode, now.current));
  }, [off, sendSplit]);
  const setSplitMode = useCallback((mode: SplitMode) => {
    setSplitModeState(mode);
    sendSplit(splitItems(mode, now.current));
  }, [sendSplit]);
  const setOutline = useCallback((on: boolean) => {
    setOutlineState(on);
    now.current.outline = on;
    if (now.current.compared.mode === 'split') sendSplit(splitItems(now.current.splitMode, now.current), now.current.splitShown?.selected);
  }, [sendSplit]);

  /** A tile clicked: it is chosen; of Selected Clips, its clip is gone to (p. 3005). */
  const pressTile = useCallback((tile: number) => {
    const state = now.current;
    if (state.compared.mode !== 'split') return;
    const shown = splitItems(state.splitMode, state);
    if (tile >= shown.items.length) return;
    sendSplit(shown, tile);
    const clip = shown.clips[tile];
    if (state.splitMode === 'selected_clips' && clip !== null && clip !== state.current?.id) state.onGo(clip);
  }, [sendSplit]);

  // The pointer on the picture: the view takes it while a wipe with a border to drag is on, or a split screen.
  const cursor: PointerCursor | null = !native || !loaded ? null : wiping ? wipeCursor(wipe.style) : splitting ? 'arrow' : null;
  const took = useRef(false);
  useEffect(() => {
    if (cursor === null) {
      if (took.current) { took.current = false; void callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: false }).catch(() => {}); }
      return;
    }
    took.current = true;
    void callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: true, cursor }).catch(() => {});
  }, [bridge, cursor, session]);
  useEffect(() => () => {
    if (took.current) { took.current = false; void callEngine(bridge, 'view.pointer', { monitor: 'timeline', on: false }).catch(() => {}); }
  }, [bridge]);

  const dragging = useRef(false);
  useEffect(() => bridge.onEvent((event) => {
    const pointer: ViewPointer | null = viewPointerOf(event);
    if (pointer === null || pointer.monitor !== 'timeline') return;
    const state = now.current;
    if (pointer.kind === 'up' || pointer.kind === 'cancel') { dragging.current = false; return; }
    if (pointer.kind === 'down' && pointer.button === 'right') { state.onPointerMenu?.(pointer.window); return; }
    if (pointer.kind === 'down' && pointer.button !== 'left') return;
    if (pointer.kind === 'move' && (!dragging.current || !pointer.buttons.includes('left'))) return;
    if (pointer.x === null || pointer.y === null || state.timeline === null) return;
    const { width, height } = state.timeline;
    if (state.compared.mode === 'split') {
      if (pointer.kind !== 'down') return;
      const tile = tileAt(state.compared.split.items.length, pointer.x, pointer.y, width, height);
      if (tile !== null) pressTile(tile);
      return;
    }
    if (state.compared.mode !== 'wipe') return;
    if (pointer.kind === 'down') dragging.current = true;
    // Alt turns the border of a diagonal wipe about the centre (p. 3000); otherwise it is moved to the pointer.
    if (pointer.alt && state.wipe.style === 'diagonal') {
      const angle = wipeAngleAt(pointer.x, pointer.y, width, height);
      if (angle !== null && angle !== state.wipe.angle) changeWipe({ ...state.wipe, angle });
      return;
    }
    const position = wipePositionAt(state.wipe, pointer.x, pointer.y, width, height);
    if (position !== null && position !== state.wipe.position) changeWipe({ ...state.wipe, position });
  }), [bridge, changeWipe, pressTile]);

  const shownClip = compared.mode === 'wipe' && compared.wipe.reference.kind !== 'still'
    ? referenceClip(clips, compared.wipe.reference.at, sentReference.current?.clip ?? undefined) : null;

  return {
    compared, wipe, referenceMode, gang, splitMode, outline, wiping, splitting, wipedStill, referenceClip: shownClip,
    toggleWipe, wipeStill, showStill, wipeTimelineClip, stepReference, setStyle, invert, setReferenceMode, setGang, toggleSplit, setSplitMode, setOutline,
  };
}
