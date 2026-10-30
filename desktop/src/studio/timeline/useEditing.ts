import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type PointerEvent, type RefObject } from 'react';
import type { Key } from '../../app/keys';
import { overlayOpen } from '../../app/overlay';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Applied, type Media, type Monitor, type Op, type SubtitleTrack, type Timeline } from '../engine';
import { editCommand, editKey, type EditAction, type EditContext } from './editKeys';
import type { Targets, Transaction } from './commands';
import { frameAt, HEADER_WIDTH } from './geometry';
import { beginDrag, dragTo, finishDrag, hitClip, type Dragged, type Gesture, type Hit, type Preview, type Tool } from './interact';
import { clipLocations, EMPTY_SELECTION, primaryTrack, selectClips, type Selection } from './selection';
import { snap, snapCandidates, type Marks } from './snap';
import { draggedMedia, MEDIA_DRAG } from '../media/drag';
import { dropMedia, dropMode } from './drop';
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
  /** The subtitle chosen on the timeline, for the inspector; `open` where it was just added or double-clicked, to be written in. */
  onCue?: (cue: string | null, open: boolean) => void;
  /**
   * Whether the timeline is one of the Cut page: there Track 1 is magnetic with the selection tool
   * (an edge of its clips ripples, a move leaves no gap, an edit point takes a clip dropped on it;
   * see `primaryTrack` and `dragTo`), and the keys of editing are those of the Cut page (see
   * `EditContext.cut`).
   */
  cut?: boolean;
}
/** How long a subtitle added at the playhead is, unless the next one starts sooner: 3 seconds, as in DaVinci Resolve. */
export const CUE_SECONDS = 3;
/** A subtitle dragged by its middle or by an edge: where it was, and the room between its neighbours. */
interface CueDrag {
  id: string; track: string; part: 'move' | 'in' | 'out'; x: number; y: number; pointer: number; origin: number;
  start: number; length: number; low: number; high: number; revision: number; moved: boolean; candidates: number[];
  /** Whether N was pressed during the drag, turning snapping the other way until it ends; and where the pointer was last. */
  flip?: boolean; last?: number;
}
/** Where the pointer was, and the keys held, as a drag takes them. */
interface Pointed { clientX: number; clientY: number; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean }
interface PointerDrag {
  gesture: Gesture; x: number; y: number; pointer: number; moved: boolean; clickSelection?: Selection;
  /** The points to snap to, worked out once for the gesture: what it moves does not change them. */
  candidates?: number[];
  /**
   * Whether N was pressed during the drag, an odd number of times: snapping is the other way until
   * the drag ends, and then as it was (Reference Manual 20, p. 711). And where the pointer was last.
   */
  flip?: boolean; last?: Pointed;
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
  const [cue, setCue] = useState<string | null>(null);
  // A subtitle being dragged, and where it would go: clamped at its neighbours, as the engine refuses overlaps.
  const cueDrag = useRef<CueDrag | null>(null);
  const [cuePreview, setCuePreview] = useState<(Preview & { clamped: boolean }) | null>(null);
  const [targets, setTargets] = useState<Targets>(() => defaultTargets(options.timeline));
  const [marks, setMarks] = useState<Marks>({ in: null, out: null });
  const [preview, setPreview] = useState<Dragged | null>(null);
  const [dropAt, setDropAt] = useState<number | null>(null);
  // A title or a solid colour dragged over a track: where it would go, and what of the clips there it would overwrite.
  const [effectDrop, setEffectDrop] = useState<{ preview: Preview[]; covered: Preview[] } | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const busy = useRef(false);
  // Undo and redo asked for while an edit is under way, done after it in turn.
  const queued = useRef<('history.undo' | 'history.redo')[]>([]);
  const active = useRef<PointerDrag | null>(null);
  const live = useRef(true);
  const cueText = all.label.studio.timeline.newCue;
  const now = useRef({ ...options, words, studioWords, titleText, cueText, tool, snapping, linked, selection, transition, cue, targets, marks });
  now.current = { ...options, words, studioWords, titleText, cueText, tool, snapping, linked, selection, transition, cue, targets, marks };
  const selected = useMemo(() => new Set(selection.ids), [selection]);
  const cancel = useCallback(() => {
    const pointer = active.current?.pointer ?? cueDrag.current?.pointer;
    active.current = null; cueDrag.current = null; setPreview(null); setCuePreview(null); setDropAt(null); setEffectDrop(null);
    const root = now.current.root.current;
    if (pointer !== undefined && root?.hasPointerCapture?.(pointer)) root.releasePointerCapture(pointer);
  }, []);
  useEffect(() => {
    live.current = true;
    const away = () => cancel();
    window.addEventListener('blur', away);
    return () => { live.current = false; active.current = null; cueDrag.current = null; window.removeEventListener('blur', away); };
  }, [cancel]);
  useEffect(() => {
    cancel(); setSelection(EMPTY_SELECTION); setTransition(null); setCue(null); setMarks({ in: null, out: null });
    setTargets(defaultTargets(now.current.timeline)); setProblem(null);
  }, [options.timeline?.id, options.session, cancel]);
  useEffect(() => {
    const revision = active.current?.gesture.revision ?? cueDrag.current?.revision;
    if (revision !== undefined && revision !== options.revision) { cancel(); setProblem(now.current.words.stale_revision); }
  }, [options.revision, cancel]);
  useEffect(() => {
    const timeline = options.timeline;
    if (!timeline) return;
    const locations = clipLocations(timeline);
    setSelection(old => old.ids.every(id => locations.has(id)) ? old : { ids: old.ids.filter(id => locations.has(id)), anchor: old.anchor && locations.has(old.anchor) ? old.anchor : null });
    setTransition(old => old !== null && timeline.tracks.some(track => track.kind !== 'subtitle' && track.transitions.some(each => each.id === old)) ? old : null);
    setCue(old => old !== null && timeline.tracks.some(track => track.kind === 'subtitle' && track.cues.some(each => each.id === old)) ? old : null);
    setTargets(old => {
      const next = { ...old };
      for (const key of ['video_track', 'audio_track'] as const) if (old[key] && !timeline.tracks.some(t => t.id === old[key])) next[key] = null;
      return next.video_track === old.video_track && next.audio_track === old.audio_track ? old : next;
    });
  }, [options.timeline]);
  useEffect(() => { options.onSelection?.(selection.ids); }, [selection.ids, options.onSelection]);
  useEffect(() => { options.onTransition?.(transition); }, [transition, options.onTransition]);
  // The subtitle chosen is told as it changes; `opening` says that it was added or double-clicked.
  const opening = useRef(false);
  useEffect(() => { options.onCue?.(cue, opening.current); opening.current = false; }, [cue, options.onCue]);
  // A clip chosen leaves the transition and the subtitle, as only one kind of thing is chosen at a time.
  useEffect(() => { if (selection.ids.length > 0) { setTransition(null); setCue(null); } }, [selection.ids]);

  const run = useCallback(async (method: 'transaction.apply' | 'history.undo' | 'history.redo', transaction?: Transaction, then?: (applied: unknown) => void): Promise<boolean> => {
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
    try {
      const applied = await callEngine(context.bridge, method, transaction ? { ...transaction } : undefined); done = true;
      if (then && live.current && context.session === now.current.session) then(applied);
    }
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
  /** Choose a subtitle (or none), leaving the clips and the transition; `open` asks for it to be written in. */
  const chooseCue = useCallback((id: string | null, open = false) => {
    setSelection(old => (old.ids.length === 0 ? old : EMPTY_SELECTION)); setTransition(null);
    if (open && id !== null && id === now.current.cue) { now.current.onCue?.(id, true); return; }
    opening.current = open; setCue(id);
  }, []);
  /** Add a track of subtitles in `language` above the others, as DaVinci Resolve adds one: one transaction. */
  const addSubtitleTrack = useCallback((language: string) => {
    const { timeline, revision } = now.current;
    if (!timeline) return;
    void apply({ revision, ops: [{ edit: 'add_track', timeline: timeline.id, kind: 'subtitle', ...(language ? { language } : {}) }] });
  }, [apply]);
  /**
   * Add a subtitle on `trackId` at the playhead, for three seconds or until the next subtitle, chosen
   * to be written in. Where a subtitle is shown at the playhead already, it is said, and nothing sent.
   */
  /** Add a subtitle at `at`, or at the playhead when no frame is given. */
  const addCue = (trackId: string, at?: number) => {
    const context = now.current;
    const track = context.timeline?.tracks.find(each => each.id === trackId);
    if (!context.timeline || !track || track.kind !== 'subtitle' || track.locked) return;
    const start = Math.max(0, at ?? context.frame);
    if (track.cues.some(each => each.start <= start && start < each.start + each.length)) { setProblem(context.words.cue_overlap); return; }
    const next = Math.min(Infinity, ...track.cues.filter(each => each.start > start).map(each => each.start));
    const length = Math.min(Math.max(1, Math.round(CUE_SECONDS * context.timeline.rate.num / context.timeline.rate.den)), next - start);
    void run('transaction.apply', { revision: context.revision, ops: [{ edit: 'add_cue', timeline: context.timeline.id, track: track.id, start, length, text: context.cueText }] },
      applied => {
        const added = (applied as Applied | null)?.done?.find(op => op.op === 'add_cue');
        if (added && added.op === 'add_cue') chooseCue(added.cue.id, true);
      });
  };
  const doubleClick = (event: { target: EventTarget | null }) => {
    if (busy.current || active.current || cueDrag.current || overlayOpen() || !(event.target instanceof Element)) return;
    const cueElement = event.target.closest<HTMLElement>('[data-cue]');
    const track = event.target.closest<HTMLElement>('[data-track]');
    if (!track || track.dataset.kind !== 'subtitle') return;
    if (cueElement) { chooseCue(cueElement.dataset.cue!, true); return; }
    if (!event.target.classList.contains('timeline-lane')) return;
    if (track.dataset.locked === 'true') { setProblem(now.current.words.locked); return; }
    addCue(track.dataset.track!);
  };
  const keyDown = (key: Key): boolean => {
    if (overlayOpen() || document.querySelector('dialog[open]')) return false;
    const action = editKey(key);
    if (!action) return false;
    return perform(action);
  };
  /** Do what a key, or a button of the page, asks: the same edit, whichever asked for it. */
  const perform = (action: EditAction): boolean => {
    if ((active.current || cueDrag.current) && action.kind !== 'cancel' && action.kind !== 'snap') return true;
    // N while a clip or a subtitle is dragged turns snapping on or off for that drag alone, as in
    // DaVinci Resolve (Reference Manual 20, p. 711): it is as it was when the drag ends.
    if (action.kind === 'snap' && (active.current || cueDrag.current)) {
      const pointer = active.current;
      const drag = cueDrag.current;
      if (pointer) { pointer.flip = !pointer.flip; if (pointer.moved && pointer.last) setPreview(calculate(pointer.last, pointer)); }
      if (drag) { drag.flip = !drag.flip; if (drag.moved && drag.last !== undefined) setCuePreview(cueAt(drag, drag.last)); }
      return true;
    }
    const context = now.current;
    // Delete with a transition chosen, and no clip, takes the transition away, as in DaVinci Resolve.
    if (action.kind === 'delete' && context.transition !== null && context.selection.ids.length === 0 && context.timeline) {
      void apply({ revision: context.revision, ops: [{ edit: 'remove_transition', timeline: context.timeline.id, id: context.transition }] });
      return true;
    }
    // With a subtitle chosen, and no clip, the keys of deleting take it as they take a clip, as DaVinci
    // Resolve ripples subtitles "just like any other clip" (Reference Manual 20, pp. 765, 1092):
    // Backspace lifts it and leaves its gap; Delete (or Shift with either) takes it out and moves the
    // later subtitles of its track back by its length. The other tracks stay as they are.
    if (action.kind === 'delete' && context.cue !== null && context.selection.ids.length === 0 && context.timeline) {
      const timeline = context.timeline;
      const track = timeline.tracks.find(each => each.kind === 'subtitle' && each.cues.some(one => one.id === context.cue)) as SubtitleTrack | undefined;
      const removed = track?.cues.find(each => each.id === context.cue);
      const later = action.ripple && track && removed ? track.cues.filter(each => each.start >= removed.start + removed.length).map(each => each.id) : [];
      void apply({ revision: context.revision, ops: [
        { edit: 'remove_cue', timeline: timeline.id, id: context.cue },
        ...(later.length > 0 ? [{ op: 'shift_cues' as const, timeline: timeline.id, track: track!.id, cues: later, by: -removed!.length }] : []),
      ] });
      return true;
    }
    const result = editCommand(action, { ...context, selected: context.selection.ids });
    // A quick transition with no two clips meeting on a track of picture: it says so.
    if (!result) { if (action.kind === 'quick' || (action.kind === 'transition' && context.cut)) setProblem(context.words.no_edit_point); return true; }
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
      case 'none': break;
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
    if (!context.timeline || event.button !== 0 || busy.current || active.current || cueDrag.current || overlayOpen()) return;
    const chosen = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-transition]') : null;
    if (chosen && context.tool !== 'blade' && chosen.closest<HTMLElement>('[data-track]')?.dataset.locked !== 'true') {
      event.preventDefault(); context.root.current?.focus();
      setSelection(EMPTY_SELECTION); setCue(null); setTransition(chosen.dataset.transition!);
      return;
    }
    const cueElement = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-cue]') : null;
    if (cueElement) { cueDown(event, cueElement); return; }
    const element = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-clip]') : null;
    if (!element) {
      if (event.target instanceof Element && event.target.classList.contains('timeline-lane') && !event.ctrlKey && !event.metaKey && !event.shiftKey) { setSelection(EMPTY_SELECTION); setTransition(null); setCue(null); }
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
    // On the Cut page an edge of a clip of Track 1 ripples; on the tracks above it only that clip is
    // made longer or shorter (Reference Manual 20, pp. 509, 590).
    const rippleTrim = context.tool === 'trim' || (context.cut === true && context.tool === 'select' && primaryTrack(context.timeline)?.clips.has(id) === true);
    const gesture = beginDrag(context.timeline, context.revision, selection.ids, id, action, at(event.clientX), context.linked, rippleTrim);
    if (!gesture) return;
    active.current = { gesture, x: event.clientX, y: event.clientY, pointer: event.pointerId, moved: false, clickSelection };
    context.root.current?.setPointerCapture?.(event.pointerId);
  };
  const calculate = (event: Pointed, pointer: PointerDrag) => {
    const context = now.current;
    const track = [...context.root.current!.querySelectorAll<HTMLElement>('[data-track]')].find(element => {
      const box = element.getBoundingClientRect(); return event.clientY >= box.top && event.clientY < box.bottom;
    });
    return dragTo(pointer.gesture, at(event.clientX), {
      // As in DaVinci Resolve: a drag overwrites, Ctrl+Shift shuffles it in, Shift alone only changes tracks.
      media: context.media, track: track?.dataset.track, overwrite: true, ripple: (event.ctrlKey || event.metaKey) && event.shiftKey,
      vertical: event.shiftKey && !event.ctrlKey && !event.metaKey,
      candidates: context.snapping !== (pointer.flip === true) ? pointer.candidates ??= snapCandidates(pointer.gesture.timeline, pointer.gesture.ids, context.frame, context.marks) : undefined, scale: context.view.scale,
      // Alt held as it is let go copies the clips there, and they stay (Reference Manual 20, p. 811).
      duplicate: event.altKey && !event.ctrlKey && !event.metaKey,
      // Track 1 of the Cut page is magnetic: a clip moved leaves no gap there (Reference Manual 20, pp. 584–588).
      magnetic: context.cut && context.tool === 'select' ? primaryTrack(pointer.gesture.timeline) : null,
    });
  };
  /** Which part of a subtitle is under the pointer: an edge within six pixels (a third of a short one), else its middle. */
  const cuePart = (element: HTMLElement, clientX: number): CueDrag['part'] => {
    const box = element.getBoundingClientRect();
    const edge = Math.min(6, box.width / 3);
    return clientX - box.left <= edge ? 'in' : box.right - clientX <= edge ? 'out' : 'move';
  };
  const cueDown = (event: PointerEvent, element: HTMLElement) => {
    const context = now.current;
    const trackElement = element.closest<HTMLElement>('[data-track]')!;
    if (trackElement.dataset.locked === 'true' || context.tool === 'blade') return;
    event.preventDefault(); context.root.current?.focus();
    const id = element.dataset.cue!;
    chooseCue(id);
    const track = context.timeline!.tracks.find(each => each.id === trackElement.dataset.track) as SubtitleTrack | undefined;
    const index = track?.cues.findIndex(each => each.id === id) ?? -1;
    if (!track || index < 0) return;
    const here = track.cues[index];
    const before = track.cues[index - 1];
    const after = track.cues[index + 1];
    cueDrag.current = {
      id, track: track.id, part: cuePart(element, event.clientX), x: event.clientX, y: event.clientY, pointer: event.pointerId, origin: at(event.clientX),
      start: here.start, length: here.length, low: before ? before.start + before.length : 0, high: after ? after.start : Infinity,
      revision: context.revision, moved: false, candidates: snapCandidates(context.timeline!, [id], context.frame, context.marks),
    };
    context.root.current?.setPointerCapture?.(event.pointerId);
  };
  /** Where a dragged subtitle goes: snapped, then held between its neighbours, never shorter than a frame. */
  const cueAt = (drag: CueDrag, clientX: number): Preview & { clamped: boolean } => {
    const { view } = now.current;
    const end = drag.start + drag.length;
    const anchors = drag.part === 'move' ? [drag.start, end] : drag.part === 'in' ? [drag.start] : [end];
    const raw = at(clientX) - drag.origin;
    const delta = now.current.snapping !== (drag.flip === true) ? snap(raw, anchors, drag.candidates, view.scale).delta : raw;
    let start = drag.start;
    let length = drag.length;
    if (drag.part === 'move') start = Math.min(Math.max(drag.start + delta, drag.low), drag.high - drag.length);
    else if (drag.part === 'in') { start = Math.min(Math.max(drag.start + delta, drag.low), end - 1); length = end - start; }
    else length = Math.min(Math.max(end + delta, drag.start + 1), drag.high) - drag.start;
    const wanted = drag.part === 'out' ? end + delta : drag.start + delta;
    const got = drag.part === 'out' ? start + length : start;
    return { id: drag.id, track: drag.track, start, length, clamped: got !== wanted };
  };
  const pointerMove = (event: PointerEvent) => {
    const drag = cueDrag.current;
    if (drag) {
      if (drag.pointer !== event.pointerId) return;
      if (!drag.moved && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 3) return;
      drag.moved = true; drag.last = event.clientX;
      const next = cueAt(drag, event.clientX);
      setCuePreview(old => old && old.start === next.start && old.length === next.length && old.clamped === next.clamped ? old : next);
      return;
    }
    const pointer = active.current;
    if (!pointer) {
      const cueElement = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-cue]') : null;
      if (cueElement) {
        cueElement.style.cursor = cueElement.closest<HTMLElement>('[data-track]')?.dataset.locked === 'true' ? 'not-allowed'
          : ({ move: 'grab', in: 'w-resize', out: 'e-resize' } as const)[cuePart(cueElement, event.clientX)];
        return;
      }
      const element = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-clip]') : null;
      if (element && now.current.timeline) element.style.cursor = element.closest<HTMLElement>('[data-track]')?.dataset.locked === 'true' ? 'not-allowed' : cursor(hit(element, event));
      return;
    }
    if (pointer.pointer !== event.pointerId) return;
    if (!pointer.moved && Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) < 3) return;
    pointer.moved = true;
    pointer.last = { clientX: event.clientX, clientY: event.clientY, ctrlKey: event.ctrlKey, metaKey: event.metaKey, shiftKey: event.shiftKey, altKey: event.altKey };
    setPreview(calculate(pointer.last, pointer));
  };
  const pointerUp = (event: PointerEvent) => {
    const drag = cueDrag.current;
    if (drag) {
      if (drag.pointer !== event.pointerId) return;
      const timeline = now.current.timeline;
      const next = drag.moved && drag.revision === now.current.revision ? cueAt(drag, event.clientX) : null;
      cancel();
      if (!next || !timeline) return;
      // One gesture, one step to undo: where it starts and how long it is, as one change.
      const change = { ...(next.start !== drag.start ? { start: next.start } : {}), ...(next.length !== drag.length ? { length: next.length } : {}) };
      if (Object.keys(change).length > 0) void apply({ revision: drag.revision, ops: [{ edit: 'set_cue', timeline: timeline.id, id: drag.id, ...change }] });
      return;
    }
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
    if (!track) { setDropAt(null); setEffectDrop(null); return; }
    if (!event.dataTransfer.types.includes(MEDIA_DRAG)) {
      const found = effectAt(event, track);
      if (!found) { setDropAt(null); setEffectDrop(null); return; }
      event.preventDefault(); event.dataTransfer.dropEffect = 'copy';
      const target = found.target;
      setDropAt(target.kind === 'cut' ? target.cut : target.at);
      if (target.kind === 'cut') { setEffectDrop(null); return; }
      // Drawn again only where it would go somewhere else: a drag tells of itself many times a second.
      setEffectDrop(old => old && old.preview[0].track === target.track && old.preview[0].start === target.at && old.preview[0].length === target.length ? old
        : { preview: [{ id: 'effect', track: target.track, start: target.at, length: target.length }], covered: target.covered });
      return;
    }
    event.preventDefault(); event.dataTransfer.dropEffect = 'copy'; setDropAt(dropFrame(event));
  };
  const drop = (event: DragEvent) => {
    setDropAt(null); setEffectDrop(null);
    const track = dropTarget(event);
    if (!track) return;
    const context = now.current;
    if (!event.dataTransfer.types.includes(MEDIA_DRAG)) {
      const found = effectAt(event, track);
      if (!found) return;
      event.preventDefault();
      if (droppedEffect(event.dataTransfer, context.session) !== found.effect) return;
      const transaction = effectTransaction(context.timeline!, context.revision, found.effect, found.target, context.titleText);
      if (transaction.ops.length > 0) void apply(transaction);
      return;
    }
    event.preventDefault();
    const ids = draggedMedia(event.dataTransfer, context.session);
    const transaction = dropMedia(context.timeline!, context.revision, context.media, ids, track, context.targets, dropFrame(event), dropMode(event));
    if (transaction) void apply(transaction);
  };
  const dragLeave = (event: DragEvent) => {
    if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) { setDropAt(null); setEffectDrop(null); }
  };
  return { tool, chooseTool, snapping, setSnapping, linked, setLinked, selected, transition, targets, target, marks, preview, dropAt, effectDrop, cue, cuePreview, chooseCue, addSubtitleTrack, addCue, doubleClick, problem, setProblem, pending,
    edit, apply, keyDown, perform, pointerDown, pointerMove, pointerUp, cancel, dragOver, drop, dragLeave };
}
