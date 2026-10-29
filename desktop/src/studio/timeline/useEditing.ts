import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type PointerEvent, type RefObject } from 'react';
import type { Key } from '../../app/keys';
import { overlayOpen } from '../../app/overlay';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Media, type Monitor, type Op, type Timeline } from '../engine';
import { editCommand, editKey, type EditContext } from './editKeys';
import type { Targets, Transaction } from './commands';
import { frameAt, HEADER_WIDTH } from './geometry';
import { beginDrag, dragTo, finishDrag, hitClip, type Dragged, type Gesture, type Hit, type Tool } from './interact';
import { clipLocations, EMPTY_SELECTION, selectClips, type Selection } from './selection';
import { snap, snapCandidates, type Marks } from './snap';
import { draggedMedia, MEDIA_DRAG } from '../media/drag';
import { dropMedia } from './drop';
import { droppedEffect, effectOfTypes } from '../effects/drag';
import { effectTarget, effectTransaction } from '../effects/drop';
import { refusalText } from '../inspector/refusal';

interface Options {
  timeline: Timeline | null; revision: number; session: number; media: readonly Media[];
  bridge: Pick<EngineBridge, 'call'>; frame: number; focus: Monitor; source: EditContext['source'];
  root: RefObject<HTMLElement | null>; scroll: RefObject<HTMLDivElement | null>; view: { scale: number; left: number };
  onSeek: (frame: number, scrubbing: boolean) => void; fit: () => void; zoom: (factor: number) => void;
  onSpeed?: (clips: string[]) => void; onSelection?: (clips: string[]) => void;
  /** The transition chosen on the timeline, for the inspector; null where none is. */
  onTransition?: (transition: string | null) => void;
}
interface PointerDrag {
  gesture: Gesture; x: number; y: number; pointer: number; moved: boolean; clickSelection?: Selection;
  /** The points to snap to, worked out once for the gesture: what it moves does not change them. */
  candidates?: number[];
}
const cursor = (hit: Hit) => typeof hit === 'object' ? 'col-resize' : ({ move: 'grab', trim_in: 'w-resize', trim_out: 'e-resize', slip: 'ew-resize', slide: 'move', blade: 'crosshair' })[hit];
const defaultTargets = (timeline: Timeline | null): Targets => ({
  video_track: timeline?.tracks.find(track => track.kind === 'video')?.id ?? null,
  audio_track: timeline?.tracks.find(track => track.kind === 'audio')?.id ?? null,
});

/** Owns transient gestures. Only an accepted engine notification changes the project on screen. */
export function useEditing(options: Options) {
  const all = useWords();
  const words = all.hint.studio.edit;
  const studioWords = all.hint.studio;
  const titleText = all.label.studio.inspector.titleText;
  const [tool, setTool] = useState<Tool>('select');
  const [snapping, setSnapping] = useState(true);
  const [linked, setLinked] = useState(true);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [transition, setTransition] = useState<string | null>(null);
  const [targets, setTargets] = useState<Targets>(() => defaultTargets(options.timeline));
  const [marks, setMarks] = useState<Marks>({ in: null, out: null });
  const [preview, setPreview] = useState<Dragged | null>(null);
  const [dropAt, setDropAt] = useState<number | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const busy = useRef(false);
  // Undo and redo asked for while an edit is under way, done after it in turn.
  const queued = useRef<('history.undo' | 'history.redo')[]>([]);
  const active = useRef<PointerDrag | null>(null);
  const live = useRef(true);
  const now = useRef({ ...options, words, studioWords, titleText, tool, snapping, linked, selection, transition, targets, marks });
  now.current = { ...options, words, studioWords, titleText, tool, snapping, linked, selection, transition, targets, marks };
  const selected = useMemo(() => new Set(selection.ids), [selection]);
  const cancel = useCallback(() => {
    const pointer = active.current?.pointer;
    active.current = null; setPreview(null); setDropAt(null);
    const root = now.current.root.current;
    if (pointer !== undefined && root?.hasPointerCapture?.(pointer)) root.releasePointerCapture(pointer);
  }, []);
  useEffect(() => {
    live.current = true;
    const away = () => cancel();
    window.addEventListener('blur', away);
    return () => { live.current = false; active.current = null; window.removeEventListener('blur', away); };
  }, [cancel]);
  useEffect(() => {
    cancel(); setSelection(EMPTY_SELECTION); setTransition(null); setMarks({ in: null, out: null });
    setTargets(defaultTargets(now.current.timeline)); setProblem(null);
  }, [options.timeline?.id, options.session, cancel]);
  useEffect(() => {
    if (active.current && active.current.gesture.revision !== options.revision) { cancel(); setProblem(now.current.words.stale_revision); }
  }, [options.revision, cancel]);
  useEffect(() => {
    const timeline = options.timeline;
    if (!timeline) return;
    const locations = clipLocations(timeline);
    setSelection(old => old.ids.every(id => locations.has(id)) ? old : { ids: old.ids.filter(id => locations.has(id)), anchor: old.anchor && locations.has(old.anchor) ? old.anchor : null });
    setTransition(old => old !== null && timeline.tracks.some(track => track.kind !== 'subtitle' && track.transitions.some(each => each.id === old)) ? old : null);
    setTargets(old => {
      const next = { ...old };
      for (const key of ['video_track', 'audio_track'] as const) if (old[key] && !timeline.tracks.some(t => t.id === old[key])) next[key] = null;
      return next.video_track === old.video_track && next.audio_track === old.audio_track ? old : next;
    });
  }, [options.timeline]);
  useEffect(() => { options.onSelection?.(selection.ids); }, [selection.ids, options.onSelection]);
  useEffect(() => { options.onTransition?.(transition); }, [transition, options.onTransition]);
  // A clip chosen leaves the transition, as only one kind of thing is chosen at a time.
  useEffect(() => { if (selection.ids.length > 0) setTransition(null); }, [selection.ids]);

  const run = useCallback(async (method: 'transaction.apply' | 'history.undo' | 'history.redo', transaction?: Transaction): Promise<boolean> => {
    if (busy.current) {
      // Ctrl+Z pressed again while an edit is applied: it is not lost, but done after it.
      if (method !== 'transaction.apply') queued.current.push(method);
      // Another edit was worked out from the project before the one under way: it says so, and is not sent.
      else if (live.current) setProblem(now.current.words.pending);
      return false;
    }
    const context = now.current;
    busy.current = true; setPending(true); setProblem(null);
    let done = false;
    try { await callEngine(context.bridge, method, transaction ? { ...transaction } : undefined); done = true; }
    catch (error) {
      queued.current = [];
      if (live.current && context.session === now.current.session && context.timeline?.id === now.current.timeline?.id) {
        setProblem(refusalText(error, now.current.studioWords));
      }
    } finally { busy.current = false; if (live.current) setPending(false); }
    const next = queued.current.shift();
    if (next && live.current && context.session === now.current.session) void run(next);
    return done;
  }, []);
  const apply = useCallback((transaction: Transaction) => run('transaction.apply', transaction), [run]);
  const edit = useCallback((ops: Op[]) => apply({ revision: now.current.revision, ops }), [apply]);
  const target = useCallback((id: string) => {
    const track = now.current.timeline?.tracks.find(track => track.id === id);
    if (!track || track.kind === 'subtitle') return;
    const key = track.kind === 'video' ? 'video_track' : 'audio_track';
    setTargets(old => ({ ...old, [key]: old[key] === id ? null : id }));
  }, []);
  const chooseTool = useCallback((tool: Tool) => { cancel(); setTool(tool); }, [cancel]);
  const keyDown = (key: Key): boolean => {
    if (overlayOpen() || document.querySelector('dialog[open]')) return false;
    const action = editKey(key);
    if (!action) return false;
    if (active.current && action.kind !== 'cancel' && action.kind !== 'snap') return true;
    const context = now.current;
    // Delete with a transition chosen, and no clip, takes the transition away, as in DaVinci Resolve.
    if (action.kind === 'delete' && context.transition !== null && context.selection.ids.length === 0 && context.timeline) {
      void apply({ revision: context.revision, ops: [{ edit: 'remove_transition', timeline: context.timeline.id, id: context.transition }] });
      return true;
    }
    const result = editCommand(action, { ...context, selected: context.selection.ids });
    if (!result) return true;
    switch (result.kind) {
      case 'transaction': void apply(result.transaction); break;
      case 'history': void run(result.method); break;
      case 'marks': setMarks(result.marks); break;
      case 'seek': context.onSeek(result.frame, false); break;
      case 'speed': context.onSpeed?.(result.clips); break;
      case 'tool': chooseTool(result.tool); break;
      case 'snap': setSnapping(value => !value); break;
      case 'linked': setLinked(value => !value); break;
      case 'fit': context.fit(); break;
      case 'zoom': context.zoom(result.direction > 0 ? 1.25 : 0.8); break;
      case 'cancel': cancel(); setProblem(null); break;
    }
    return true;
  };
  const at = (clientX: number) => {
    const { scroll, view } = now.current;
    return frameAt(clientX - (scroll.current?.getBoundingClientRect().left ?? 0) - HEADER_WIDTH, view.scale, scroll.current?.scrollLeft ?? view.left);
  };
  const hit = (element: HTMLElement, event: PointerEvent): Hit => {
    const box = element.getBoundingClientRect();
    return hitClip(now.current.timeline!, element.dataset.clip!, now.current.tool, event.clientX - box.left, event.clientY - box.top, box.width, box.height);
  };
  const pointerDown = (event: PointerEvent) => {
    const context = now.current;
    if (!context.timeline || event.button !== 0 || busy.current || active.current || overlayOpen()) return;
    const chosen = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-transition]') : null;
    if (chosen && context.tool !== 'blade' && chosen.closest<HTMLElement>('[data-track]')?.dataset.locked !== 'true') {
      event.preventDefault(); context.root.current?.focus();
      setSelection(EMPTY_SELECTION); setTransition(chosen.dataset.transition!);
      return;
    }
    const element = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-clip]') : null;
    if (!element) {
      if (event.target instanceof Element && event.target.classList.contains('timeline-lane') && !event.ctrlKey && !event.metaKey && !event.shiftKey) { setSelection(EMPTY_SELECTION); setTransition(null); }
      return;
    }
    const id = element.dataset.clip!;
    const track = element.closest<HTMLElement>('[data-track]')!;
    if (track.dataset.locked === 'true') return;
    event.preventDefault(); context.root.current?.focus();
    const action = hit(element, event);
    if (action === 'blade') {
      // With snapping on, the blade cuts at the playhead where it is near it, as in DaVinci Resolve.
      const pointed = at(event.clientX);
      const cut = context.snapping ? snap(pointed, [0], [context.frame], context.view.scale).delta : pointed;
      void apply({ revision: context.revision, ops: [{ edit: 'blade', timeline: context.timeline.id, at: cut, ...(event.shiftKey ? {} : { tracks: [track.dataset.track!] }), linked: context.linked }] });
      return;
    }
    let selection = !event.ctrlKey && !event.metaKey && !event.shiftKey && context.selection.ids.includes(id) ? context.selection
      : selectClips(context.timeline, context.selection, id, { linked: context.linked, toggle: event.ctrlKey || event.metaKey, range: event.shiftKey });
    // Ctrl-click toggles; Ctrl+Shift-drag shuffles in. Decide whether to deselect only on release.
    const clickSelection = !selection.ids.includes(id) ? selection : undefined;
    if (clickSelection) selection = context.selection;
    setSelection(selection);
    if (!selection.ids.includes(id)) return;
    const gesture = beginDrag(context.timeline, context.revision, selection.ids, id, action, at(event.clientX), context.linked, context.tool === 'trim');
    if (!gesture) return;
    active.current = { gesture, x: event.clientX, y: event.clientY, pointer: event.pointerId, moved: false, clickSelection };
    context.root.current?.setPointerCapture?.(event.pointerId);
  };
  const calculate = (event: PointerEvent, pointer: PointerDrag) => {
    const context = now.current;
    const track = [...context.root.current!.querySelectorAll<HTMLElement>('[data-track]')].find(element => {
      const box = element.getBoundingClientRect(); return event.clientY >= box.top && event.clientY < box.bottom;
    });
    return dragTo(pointer.gesture, at(event.clientX), {
      // As in DaVinci Resolve: a drag overwrites, Ctrl+Shift shuffles it in, Shift alone only changes tracks.
      media: context.media, track: track?.dataset.track, overwrite: true, ripple: (event.ctrlKey || event.metaKey) && event.shiftKey,
      vertical: event.shiftKey && !event.ctrlKey && !event.metaKey,
      candidates: context.snapping ? pointer.candidates ??= snapCandidates(pointer.gesture.timeline, pointer.gesture.ids, context.frame, context.marks) : undefined, scale: context.view.scale,
    });
  };
  const pointerMove = (event: PointerEvent) => {
    const pointer = active.current;
    if (!pointer) {
      const element = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-clip]') : null;
      if (element && now.current.timeline) element.style.cursor = element.closest<HTMLElement>('[data-track]')?.dataset.locked === 'true' ? 'not-allowed' : cursor(hit(element, event));
      return;
    }
    if (pointer.pointer !== event.pointerId) return;
    if (!pointer.moved && Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) < 3) return;
    pointer.moved = true; setPreview(calculate(event, pointer));
  };
  const pointerUp = (event: PointerEvent) => {
    const pointer = active.current;
    if (!pointer || pointer.pointer !== event.pointerId) return;
    const transaction = pointer.moved && pointer.gesture.revision === now.current.revision ? finishDrag(calculate(event, pointer)) : null;
    if (!pointer.moved && pointer.clickSelection) setSelection(pointer.clickSelection);
    cancel(); if (transaction) void apply(transaction);
  };
  const dropTarget = (event: DragEvent) => {
    if (busy.current || !now.current.timeline || !(event.dataTransfer.types.includes(MEDIA_DRAG) || effectOfTypes([...event.dataTransfer.types]))) return null;
    const lane = event.target instanceof Element ? event.target.closest('.timeline-lane') : null;
    const track = lane?.closest<HTMLElement>('[data-track]');
    return track && track.dataset.kind !== 'subtitle' && track.dataset.locked !== 'true' ? track.dataset.track! : null;
  };
  // The points that a drop snaps to, kept while the timeline, the playhead and the marks stay as they are.
  const dropPoints = useRef<{ timeline: Timeline; frame: number; marks: Marks; points: number[] } | null>(null);
  const dropFrame = (event: DragEvent) => {
    const context = now.current;
    const frame = at(event.clientX);
    if (!context.snapping) return frame;
    const kept = dropPoints.current;
    const points = kept && kept.timeline === context.timeline && kept.frame === context.frame && kept.marks === context.marks ? kept.points
      : snapCandidates(context.timeline!, [], context.frame, context.marks);
    dropPoints.current = { timeline: context.timeline!, frame: context.frame, marks: context.marks, points };
    return snap(frame, [0], points, context.view.scale).delta;
  };
  /** Where an effect from the library goes on the track under the pointer: at a cut, or in an empty place. */
  const effectAt = (event: DragEvent, track: string) => {
    const effect = effectOfTypes([...event.dataTransfer.types]);
    if (!effect) return null;
    const { scroll, view, timeline } = now.current;
    const position = (event.clientX - (scroll.current?.getBoundingClientRect().left ?? 0) - HEADER_WIDTH + (scroll.current?.scrollLeft ?? view.left)) / view.scale;
    const target = effectTarget(timeline!, effect, track, position, dropFrame(event), view.scale);
    return target && { effect, target };
  };
  const dragOver = (event: DragEvent) => {
    const track = dropTarget(event);
    if (!track) { setDropAt(null); return; }
    if (!event.dataTransfer.types.includes(MEDIA_DRAG)) {
      const found = effectAt(event, track);
      if (!found) { setDropAt(null); return; }
      event.preventDefault(); event.dataTransfer.dropEffect = 'copy';
      setDropAt(found.target.kind === 'cut' ? found.target.cut : found.target.at);
      return;
    }
    event.preventDefault(); event.dataTransfer.dropEffect = 'copy'; setDropAt(dropFrame(event));
  };
  const drop = (event: DragEvent) => {
    setDropAt(null);
    const track = dropTarget(event);
    if (!track) return;
    const context = now.current;
    if (!event.dataTransfer.types.includes(MEDIA_DRAG)) {
      const found = effectAt(event, track);
      if (!found) return;
      event.preventDefault();
      if (droppedEffect(event.dataTransfer, context.session) !== found.effect) return;
      void apply(effectTransaction(context.timeline!, context.revision, found.effect, found.target, context.titleText));
      return;
    }
    event.preventDefault();
    const ids = draggedMedia(event.dataTransfer, context.session);
    const transaction = dropMedia(context.timeline!, context.revision, context.media, ids, track, context.targets, dropFrame(event), event.shiftKey);
    if (transaction) void apply(transaction);
  };
  const dragLeave = (event: DragEvent) => {
    if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) setDropAt(null);
  };
  return { tool, chooseTool, snapping, setSnapping, linked, setLinked, selected, transition, targets, target, marks, preview, dropAt, problem, setProblem, pending,
    edit, apply, keyDown, pointerDown, pointerMove, pointerUp, cancel, dragOver, drop, dragLeave };
}
