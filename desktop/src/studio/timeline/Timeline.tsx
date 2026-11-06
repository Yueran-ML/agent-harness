import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type Ref, type ReactNode, type Dispatch, type SetStateAction } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import { keyOf, type Key } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import type { Media, Monitor, Timeline as TimelineModel } from '../engine';
import { editingAllowed } from '../keys';
import { LET_GO_HOLD_MS } from '../useMonitor';
import { duration, fitZoom, frameAt, HEADER_WIDTH, RULER_HEIGHT, scrollWindow, zoomAt } from './geometry';
import { Ruler } from './Ruler';
import { Scrub } from './scrub';
import { AddSubtitleTrack, pageLanguage, TimelineMenu, type TrackMenu } from './SubtitleTrack';
import { Toolbar } from './Toolbar';
import { TrackRow, type TrackClipDrawing } from './TrackRow';
import { ClipView } from './ClipView';
import { Waveforms } from './waveforms';
import { useEditing } from './useEditing';
import type { EditAction, EditContext } from './editKeys';
import type { Marks } from './snap';
import type { AudioClipboard } from './VolumeLine';
import { refusalText } from '../inspector/refusal';
import { CoveringMenu } from '../color/CoveringMenu';
import type { ChannelContext } from '../inspector/ChannelSourceMenu';
import { AudioPlayhead, type AudioOverlayContext } from './AudioOverlay';
import './timeline.css';

/** A view of the accepted engine project. Scrolling, zoom and track height do not add undo steps. */
export interface TimelineHandle {
  keyDown: (key: Key) => boolean;
  /** Do an edit that a button of the page asks for, as its key would. */
  perform: (action: EditAction) => boolean;
  canPerform: (action: EditAction) => boolean;
  clearTransition: () => void;
  /** The In and Out points of the timeline, which exist only on the page: an export may be of the part between them. */
  marks: () => Marks;
  /** Clip commands shared with the page-owned context menu. */
  clipboardItems: (clip?: string) => TrackMenu['items'];
}
/** What of a timeline a view shows: the pixels of a frame, how far it is scrolled, and how wide it is. */
export interface TimelineView { scale: number; left: number; width: number }
/** Persistent view coordinates; viewport width is measured from the current layout. */
export interface TimelineViewport { scale: number; left: number; top: number; heights: Record<string, number> }
export type TimelineScrolling = 'fixed' | 'page' | 'none';
export function Timeline({ timeline, media, bridge, session, revision, frame, playing, onSeek, source = null, focus = 'timeline', onSpeed, onSelection, onTransition, onCue, ref,
  cut = false, overview = false, center = false, trackHeight = 72, indicator = null, detail = null, onView, label, hiddenTracks, drawClip, onClipContext, onCopyClips, onTrackSelect, selectedTracks, audioOnlyDrop, gainLine = true, trackDecoration, afterTracks, gainLineDisabled, automationRange, viewport, scrolling, trackMenuItems }: {
  timeline: TimelineModel | null; media: readonly Media[]; bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number;
  revision: number; frame: number; playing: boolean; onSeek: (frame: number, scrubbing: boolean) => void;
  source?: EditContext['source']; focus?: Monitor; onSpeed?: (clips: string[]) => void; onSelection?: (clips: string[]) => void;
  onTransition?: (transition: string | null) => void;
  /** The subtitle chosen, for the inspector; `open` where it was added or double-clicked, to be written in. */
  onCue?: (cue: string | null, open: boolean) => void; ref?: Ref<TimelineHandle>;
  /** A timeline of the Cut page: its Track 1 is magnetic, and the keys are those of the Cut page. */
  cut?: boolean;
  /**
   * The whole timeline at once, as the upper timeline of the Cut page: it is always fitted to its
   * width, has no tool bar, and a press on a track moves the playhead there.
   */
  overview?: boolean;
  /** The playhead kept in the middle, as the lower timeline of the Cut page keeps it. */
  center?: boolean;
  /** How high a track is until it is made higher or lower. */
  trackHeight?: number;
  /** The edit point that Smart Insert would go in at, marked on the timeline. */
  indicator?: number | null;
  /** The frames that another view of this timeline shows, marked on this one. */
  detail?: { from: number; to: number } | null;
  /** Told of the view as it scrolls or zooms. */
  onView?: (view: TimelineView) => void;
  /** Optional page-owned view state; Edit and Cut keep their existing local state. */
  viewport?: { value: TimelineViewport; update: Dispatch<SetStateAction<TimelineViewport>> };
  /** Fairlight's explicit scrolling mode; absent means the existing Follow/center controls. */
  scrolling?: TimelineScrolling;
  /** The name of the region, where a page shows more than one view of a timeline. */
  label?: string;
  /** View filtering only; editing still sees the complete, accepted timeline. */
  hiddenTracks?: readonly string[];
  drawClip?: (props: TrackClipDrawing) => ReactNode;
  onClipContext?: (clip: string, x: number, y: number, channel: ChannelContext) => void;
  onCopyClips?: (clips: readonly string[]) => boolean;
  trackMenuItems?: (track: string) => TrackMenu['items'];
  onTrackSelect?: (track: string) => void;
  /** View-only selection shared with the mixer; it does not change edit destination tracks. */
  selectedTracks?: readonly string[];
  audioOnlyDrop?: boolean;
  gainLine?: boolean;
  gainLineDisabled?: (id: string) => boolean;
  trackDecoration?: (track: import('../engine').Track, geometry: { scale: number; from: number; to: number; height: number; snapping: boolean }) => ReactNode;
  afterTracks?: (geometry: { scale: number; from: number; to: number; snapping: boolean }) => ReactNode;
  automationRange?: {from:number;to:number} | null;
}) {
  const words = useWords();
  const labels = words.label.studio.timeline;
  const root = useRef<HTMLElement>(null);
  const audioClipboard = useRef<AudioClipboard>({ keys: [] });
  useEffect(() => { audioClipboard.current.keys = []; }, [session]);
  const scroll = useRef<HTMLDivElement>(null);
  const draw = useRef<number | null>(null);
  const [localViewport, setLocalViewport] = useState<TimelineViewport>({ scale: 4, left: 0, top: 0, heights: {} });
  const position = viewport?.value ?? localViewport;
  const setPosition = viewport?.update ?? setLocalViewport;
  const controlled = viewport !== undefined;
  const [width, setWidth] = useState(800);
  const measuredWidth = useRef(width); measuredWidth.current = width;
  const view = useMemo(() => ({ scale: position.scale, left: position.left, width }), [position.scale, position.left, width]);
  const setView = useCallback((update: SetStateAction<TimelineView>) => setPosition(old => {
    const next = typeof update === 'function' ? update({ scale: old.scale, left: old.left, width: measuredWidth.current }) : update;
    return old.scale === next.scale && old.left === next.left ? old : { ...old, scale: next.scale, left: next.left };
  }), [setPosition]);
  const heights = position.heights;
  const setHeights = useCallback((update: SetStateAction<Record<string, number>>) => setPosition(old => ({ ...old,
    heights: typeof update === 'function' ? update(old.heights) : update })), [setPosition]);
  const [follow, setFollow] = useState(true);
  // The menu of a right-click, where one is open.
  const [menu, setMenu] = useState<(TrackMenu & { clipboard?: boolean }) | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  useEffect(closeMenu, [session, timeline?.id, closeMenu]);
  const [held, setHeld] = useState<{ frame: number; dragging: boolean } | null>(null);
  const drag = useRef<{ frame: number } | null>(null);
  const now = useRef({ view, onSeek, overview, frame, heights, trackHeight });
  const scrub = useMemo(() => new Scrub((to, scrubbing) => now.current.onSeek(to, scrubbing)), []);
  const waves = useMemo(() => new Waveforms(bridge, session), [bridge, session]);
  useEffect(() => { waves.connect(); return () => waves.disconnect(); }, [waves]);
  useEffect(() => () => scrub.cancel(), [scrub]);
  const byId = useMemo(() => new Map(media.map(item => [item.id, item])), [media]);
  const end = useMemo(() => timeline ? duration(timeline) : 0, [timeline]);
  const last = Math.max(0, end - 1);
  // The frame under the playhead as it is shown: where it is dragged or was let go, until the
  // transport tells it. The edits, the marks and the zoom take it, as DaVinci Resolve takes the frame shown.
  const shown = held?.frame ?? frame;
  now.current = { view, onSeek, overview, frame: shown, heights, trackHeight };
  const tracks = useMemo(() => timeline ? [
    ...timeline.tracks.filter(track => track.kind === 'subtitle').reverse(),
    ...timeline.tracks.filter(track => track.kind === 'video').reverse(),
    ...timeline.tracks.filter(track => track.kind === 'audio'),
  ].filter(track => !hiddenTracks?.includes(track.id)) : [], [timeline, hiddenTracks]);

  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const measure = () => {
      const width = Math.max(1, element.clientWidth - HEADER_WIDTH);
      setWidth(width);
      if (!controlled) setView(old => old.left === element.scrollLeft ? old : { ...old, left: element.scrollLeft });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => { observer.disconnect(); if (draw.current !== null) cancelAnimationFrame(draw.current); draw.current = null; };
  }, [timeline?.id, controlled, setView]);
  useLayoutEffect(() => {
    if (!scroll.current) return;
    if (scroll.current.scrollLeft !== view.left) scroll.current.scrollLeft = view.left;
    if (controlled && scroll.current.scrollTop !== position.top) scroll.current.scrollTop = position.top;
  }, [view.left, view.scale, position.top, controlled]);
  useEffect(() => {
    scrub.cancel(); drag.current = null; setHeld(null);
    if (!controlled) setView(old => ({ ...old, left: 0 }));
  }, [timeline?.id, session, scrub, controlled, setView]);
  useEffect(() => {
    if (held && !held.dragging && (frame === held.frame || playing)) setHeld(null);
  }, [frame, playing, held]);
  useEffect(() => {
    if (!held || held.dragging) return;
    const timer = setTimeout(() => setHeld(null), LET_GO_HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  useEffect(() => {
    if (scrolling !== undefined || !follow || !playing || drag.current) return;
    setView(old => {
      const x = frame * old.scale;
      return x >= old.left && x <= old.left + old.width * 0.9 ? old : { ...old, left: Math.max(0, x - old.width * 0.15) };
    });
  }, [frame, playing, follow, scrolling, setView]);
  useEffect(() => {
    if (scrolling !== 'page' || (!playing && !held?.dragging)) return;
    setView(old => {
      const x = shown * old.scale;
      return x >= old.left && x < old.left + old.width ? old : { ...old, left: Math.max(0, Math.floor(x / old.width) * old.width) };
    });
  }, [shown, playing, held?.dragging, scrolling, setView]);

  /**
   * Zoom by `factor`, centred on the playhead as DaVinci Resolve zooms (Reference Manual 20, p. 711):
   * the playhead stays where it is in the view, and one out of the view is brought to its middle.
   */
  const zoom = useCallback((factor: number) => now.current.overview ? undefined : setView(old => {
    const playhead = now.current.frame * old.scale - old.left;
    if (playhead >= 0 && playhead <= old.width) {
      const next = zoomAt(old.scale, old.left, playhead, factor);
      return { ...old, scale: next.scale, left: next.scroll };
    }
    const scale = zoomAt(old.scale, old.left, 0, factor).scale;
    return { ...old, scale, left: Math.max(0, Math.round(now.current.frame * scale - old.width / 2)) };
  }), [setView]);
  // Fitting reads the end when it is asked for, so that the bar is not drawn again as the timeline changes.
  const ends = useRef(end);
  ends.current = end;
  // The zoom that fitting left, and the one it was at before, which Shift+Z goes back to.
  const unfit = useRef<{ fitted: number; scale: number } | null>(null);
  /**
   * Shift+Z, as DaVinci Resolve has it: the whole timeline fitted to the view; pressed again, the
   * zoom that was before, around the playhead where it is now (Reference Manual 20, p. 711).
   */
  const fit = useCallback(() => setView(old => {
    const was = unfit.current;
    if (was && was.fitted === old.scale) {
      unfit.current = null;
      return { ...old, scale: was.scale, left: Math.max(0, Math.round(now.current.frame * was.scale - old.width / 2)) };
    }
    const scale = fitZoom(ends.current, old.width);
    unfit.current = { fitted: scale, scale: old.scale };
    return { ...old, scale, left: 0 };
  }), [setView]);
  // The whole timeline is always shown in an overview, as it grows and as the view is made wider.
  useEffect(() => {
    if (!overview) return;
    setView(old => { const scale = fitZoom(end, old.width); return old.scale === scale && old.left === 0 ? old : { ...old, scale, left: 0 }; });
  }, [overview, end, view.width]);
  // The playhead in the middle of a view that keeps it there, unless the playhead is being dragged.
  const dragging = held?.dragging === true;
  useEffect(() => {
    if (!(center || scrolling === 'fixed') || dragging) return;
    setView(old => { const left = Math.max(0, Math.round(shown * old.scale - old.width / 2)); return old.left === left ? old : { ...old, left }; });
  }, [center, scrolling, dragging, shown, view.scale, view.width, setView]);
  useEffect(() => { onView?.(view); }, [view, onView]);
  const editing = useEditing({ timeline, media, bridge, session, revision, frame: shown, focus, source, root, scroll, view, onSeek, fit, zoom, onSpeed, onSelection, onTransition, onCue, onCopyClips, cut, audioOnlyDrop });
  const clipboardMenu = useRef(editing.clipboard.items); clipboardMenu.current = editing.clipboard.items;
  const openEditMenu = useCallback((anchor: HTMLElement) => {
    const button = anchor.getBoundingClientRect();
    setMenu({ x: button.left, y: button.bottom, items: clipboardMenu.current(), clipboard: true });
  }, []);
  const audioFailure = useCallback((error: unknown) => editing.setProblem(refusalText(error, words.hint.studio)), [editing.setProblem, words.hint.studio]);
  const audioContext = useMemo<AudioOverlayContext | undefined>(() => timeline ? {
    bridge, timeline: timeline.id, revision, session, sampleRate: timeline.sample_rate,
    snapping: editing.snapping, marks: editing.marks, clipboard: audioClipboard.current, gainLine, onSeek, onFailure: audioFailure,
  } : undefined, [bridge, timeline?.id, timeline?.sample_rate, revision, session, editing.snapping, editing.marks, gainLine, onSeek, audioFailure]);
  useImperativeHandle(ref, () => ({ keyDown: editing.keyDown, perform: editing.perform, canPerform: editing.canPerform, clearTransition: editing.clearTransition, marks: () => editing.marks, clipboardItems: editing.clipboard.items }));
  useEffect(() => {
    const element = scroll.current;
    if (!element) return;
    // The wheel as DaVinci Resolve takes it on the timeline (Reference Manual 20, p. 712): with Alt
    // it zooms, with Ctrl it goes back and forward in time, with Shift it makes the tracks of the
    // part under the pointer (picture and subtitles, or sound) higher or lower; alone it scrolls the
    // tracks. A wheel turned with Shift may be told as a sideways one.
    const wheel = (event: WheelEvent) => {
      const by = event.deltaY !== 0 ? event.deltaY : event.deltaX;
      if (by === 0) return;
      if (event.altKey && !event.ctrlKey && !event.metaKey) {
        event.preventDefault();
        zoom(Math.exp(-by * 0.002));
      } else if ((event.ctrlKey || event.metaKey) && !event.altKey) {
        event.preventDefault();
        element.scrollLeft = Math.max(0, element.scrollLeft + by);
        setView(old => ({ ...old, left: element.scrollLeft }));
      } else if (event.shiftKey) {
        event.preventDefault();
        const pointed = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-track]')?.dataset.kind : undefined;
        const sound = pointed === 'audio';
        const { heights: old, trackHeight: base } = now.current;
        const step = by < 0 ? 8 : -8;
        const changed: Record<string, number> = {};
        for (const each of element.querySelectorAll<HTMLElement>('[data-track]')) {
          if ((each.dataset.kind === 'audio') !== sound) continue;
          const id = each.dataset.track!;
          changed[id] = Math.max(40, Math.min(240, (old[id] ?? base) + step));
        }
        setHeights(was => ({ ...was, ...changed }));
      }
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [timeline?.id, zoom, setHeights, setView]);
  const onScroll = () => {
    if (controlled && scroll.current) {
      const { scrollLeft: left, scrollTop: top } = scroll.current;
      setPosition(old => old.left === left && old.top === top ? old : { ...old, left, top });
      return;
    }
    if (draw.current !== null) return;
    draw.current = requestAnimationFrame(() => {
      draw.current = null;
      if (scroll.current) { const left = scroll.current.scrollLeft; setView(old => old.left === left ? old : { ...old, left }); }
    });
  };
  const onHeight = useCallback((id: string, height: number) => setHeights(old => ({ ...old, [id]: height })), [setHeights]);
  // The frame under the pointer; the playhead is held to the last frame, but a subtitle may be added past it.
  const under = (event: { clientX: number }) => frameAt(event.clientX - (scroll.current?.getBoundingClientRect().left ?? 0) - HEADER_WIDTH, view.scale, scroll.current?.scrollLeft ?? view.left);
  const at = (event: { clientX: number }) => Math.min(last, under(event));
  const down = (event: PointerEvent) => {
    if (!timeline || end === 0 || event.button !== 0) return;
    event.preventDefault(); editing.cancel();
    root.current?.setPointerCapture?.(event.pointerId);
    const frame = at(event); drag.current = { frame }; setHeld({ frame, dragging: true }); scrub.drag(frame);
  };
  const release = (event: PointerEvent, cancelled = false) => {
    if (!drag.current) return;
    const frame = cancelled ? drag.current.frame : at(event);
    drag.current = null; setHeld({ frame, dragging: false }); scrub.finish(frame);
    root.current?.releasePointerCapture?.(event.pointerId);
  };
  // The tracks and the ruler draw a window that moves by half a view, not at every pixel of a scroll.
  const { from, to } = scrollWindow(view.left, view.width, view.scale);
  // An overview is as wide as its view; another goes on half a view past the end.
  const shownSubtitles = timeline?.tracks.find(track => track.kind === 'subtitle' && track.enabled)?.id ?? null;
  const total = HEADER_WIDTH + (overview ? view.width : Math.max(view.width, end * view.scale + view.width / 2));
  let rowTop = RULER_HEIGHT;
  const rows = new Map(tracks.map(track => { const height = heights[track.id] ?? trackHeight; const row = { top: rowTop, height }; rowTop += height; return [track.id, row]; }));
  return <section ref={root} className="timeline" role="region" aria-label={label ?? labels.title} aria-busy={editing.pending} tabIndex={0} data-timeline={timeline?.id} data-scale={view.scale} data-tool={editing.tool}
    data-overview={overview || undefined}
    onKeyDown={event => {
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      if (editing.keyDown(key)) { event.preventDefault(); event.stopPropagation(); }
    }}
    onPointerDown={event => {
      const track = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-track]')?.dataset.track : undefined;
      if (track) onTrackSelect?.(track);
      // A press on a track of the overview, not on what lies on it, moves the playhead there.
      if (overview && event.target instanceof Element && event.target.classList.contains('timeline-lane')) down(event);
      else editing.pointerDown(event);
    }}
    onDragOver={editing.dragOver} onDrop={editing.drop} onDragLeave={editing.dragLeave} onDoubleClick={editing.doubleClick}
    onContextMenu={event => {
      const clip = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-clip]')?.dataset.clip : undefined;
      if (clip && onClipContext && event.target instanceof Element && event.target.closest<HTMLElement>('[data-track]')?.dataset.kind === 'audio') {
        const lane = event.target.closest<HTMLElement>('[data-channel]')?.dataset.channel;
        event.preventDefault(); onClipContext(clip, event.clientX, event.clientY, {
          ...(lane === undefined ? {} : { channel: Number(lane) }), sourcesOnly: event.ctrlKey || event.metaKey,
        }); return;
      }
      // As DaVinci Resolve: a track header offers Add Subtitle Track, a track of subtitles Add Subtitle (Reference Manual 20, pp. 1088–1089).
      // Resolve adds the subtitle at the playhead; here it goes where the track was clicked, as the author chose.
      const track = !overview && timeline && event.target instanceof Element ? event.target.closest<HTMLElement>('[data-track]') : null;
      if (!track || !(event.target instanceof Element)) return;
      const clicked = under(event);
      const items: TrackMenu['items'] = event.target.closest('.timeline-track-header')
        ? [{ id: 'add-subtitle-track', label: labels.addSubtitleTrack, disabled: editing.pending, run: () => editing.addSubtitleTrack(pageLanguage()) }, ...(trackMenuItems?.(track.dataset.track!) ?? [])]
        : track.dataset.kind === 'subtitle' ? [{ id: 'add-subtitle', label: labels.addCue, disabled: track.dataset.locked === 'true' || editing.pending, run: () => editing.addCue(track.dataset.track!, clicked) }] : editing.clipboard.items(clip);
      if (items.length === 0) return;
      event.preventDefault();
      if (track.dataset.kind !== 'subtitle' && !event.target.closest('.timeline-track-header')) {
        setMenu({ x: event.clientX, y: event.clientY, items, clipboard: true });
        return;
      }
      const box = root.current!.getBoundingClientRect();
      setMenu({ x: event.clientX - box.left, y: event.clientY - box.top, items });
    }}
    onPointerMove={event => { if (drag.current) { const frame = at(event); drag.current.frame = frame; setHeld({ frame, dragging: true }); scrub.drag(frame); } else editing.pointerMove(event); }}
    onPointerUp={event => { release(event); editing.pointerUp(event); }} onPointerCancel={event => { release(event, true); editing.cancel(); }} onLostPointerCapture={editing.cancel}>
    {!overview && <Toolbar name={timeline?.name ?? labels.title} tool={editing.tool} snapping={editing.snapping} linked={editing.linked} follow={follow}
      onTool={editing.chooseTool} onSnapping={editing.setSnapping} onLinked={editing.setLinked} onZoom={zoom} onFit={fit} onFollow={setFollow} onEditMenu={openEditMenu} hideFollow={scrolling !== undefined} />}
    {menu && (menu.clipboard ? <CoveringMenu menu={{ ...menu, label: words.label.studio.page.edit }} onClose={closeMenu} /> : <TimelineMenu menu={menu} onClose={closeMenu} />)}
    {editing.problem && <div className="timeline-problem" role="alert"><span>{editing.problem}</span><button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => editing.setProblem(null)}><X size={14} /></button></div>}
    {!timeline ? <p className="caption timeline-empty">{words.hint.studio.timeline.empty}</p> : <div ref={scroll} className="timeline-scroll" onScroll={onScroll}>
      <div className="timeline-content" style={{ width: total, minHeight: RULER_HEIGHT + tracks.reduce((sum, track) => sum + (heights[track.id] ?? trackHeight), 0), '--scale': view.scale } as CSSProperties}>
        <div className="timeline-ruler-row"><Ruler rate={timeline.rate} scale={view.scale} from={from} to={to}
          frame={shown} last={last} onDown={down} onSeek={onSeek} />
          {automationRange&&<span className="automation-active-range" style={{left:HEADER_WIDTH+automationRange.from*view.scale,width:(automationRange.to-automationRange.from)*view.scale}}/>}
          {editing.marks.in !== null && <span className="timeline-mark" data-edge="in" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale }} />}
          {editing.marks.out !== null && <span className="timeline-mark" data-edge="out" style={{ left: HEADER_WIDTH + editing.marks.out * view.scale }} />}
          {editing.marks.in !== null && editing.marks.out !== null && <span className="timeline-mark-range" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale, width: (editing.marks.out - editing.marks.in) * view.scale }} />}
          <div className="timeline-ruler-corner">{!overview && <AddSubtitleTrack onAdd={editing.addSubtitleTrack} disabled={editing.pending} />}</div></div>
        <AudioPlayhead.Provider value={shown}>{tracks.map(track => <TrackRow key={track.id} timeline={timeline.id} track={track} media={byId} rate={timeline.rate} scale={view.scale}
          from={from} to={to} height={heights[track.id] ?? trackHeight} waves={waves} decoration={trackDecoration?.(track, { scale: view.scale, from, to, height: heights[track.id] ?? trackHeight, snapping: editing.snapping })} onEdit={editing.edit} onHeight={onHeight} drawClip={drawClip}
          audioOverlay={track.kind === 'audio' && !cut && !overview && !gainLineDisabled?.(track.id) ? audioContext : undefined}
          selected={editing.selected} mixerSelected={selectedTracks?.includes(track.id)} targeted={editing.targets.video_track === track.id || editing.targets.audio_track === track.id} onTarget={editing.target}
          chosen={editing.transition !== null && track.kind !== 'subtitle' && track.transitions.some(each => each.id === editing.transition) ? editing.transition : null}
          chosenCue={editing.cue !== null && track.kind === 'subtitle' && track.cues.some(each => each.id === editing.cue) ? editing.cue : null}
          shownSubtitles={track.kind === 'subtitle' ? shownSubtitles : null} />)}</AudioPlayhead.Provider>
        {afterTracks?.({ scale: view.scale, from, to, snapping: editing.snapping })}
        {editing.clipboard.ghosts.map(({ track, clip }) => {
          const row = rows.get(track.id);
          const source = clip.source.kind === 'media' ? byId.get(clip.source.media) : undefined;
          return row && track.kind !== 'subtitle' && <div key={clip.id} data-clipboard-ghost={track.id} aria-hidden="true"
            style={{ position: 'absolute', pointerEvents: 'none', opacity: .45, left: HEADER_WIDTH, right: 0, top: row.top, height: row.height }}>
            <ClipView clip={clip} kind={track.kind} media={source} rate={timeline.rate} scale={view.scale} from={from} to={to} waves={waves}>
              {drawClip?.({ clip, track, media: source, rate: timeline.rate, scale: view.scale, from, to, waves })}
            </ClipView>
          </div>;
        })}
        {editing.preview?.preview.map(item => { const row = rows.get(item.track); return row && <div key={item.id} className="timeline-preview" data-preview={item.id} data-clamped={editing.preview!.clamped}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.preview?.covered.map(item => { const row = rows.get(item.track); return row && <div key={`covered-${item.id}`} className="timeline-covered" data-covered={item.id}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.cuePreview && rows.get(editing.cuePreview.track) && (() => { const item = editing.cuePreview!; const row = rows.get(item.track)!; return <div className="timeline-preview" data-preview={item.id} data-clamped={item.clamped}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })()}
        {editing.preview?.point != null && <div className="timeline-snap" style={{ left: HEADER_WIDTH + editing.preview.point * view.scale }} />}
        {editing.effectDrop?.preview.map(item => { const row = rows.get(item.track); return row && <div key="effect-preview" className="timeline-preview" data-preview="effect"
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.effectDrop?.covered.map(item => { const row = rows.get(item.track); return row && <div key={`effect-covered-${item.id}`} className="timeline-covered" data-covered={item.id}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.dropAt !== null && <div className="timeline-snap" style={{ left: HEADER_WIDTH + editing.dropAt * view.scale }} />}
        {detail && <div className="timeline-window" data-test="detail-window" data-from={detail.from} data-to={detail.to} aria-hidden="true"
          style={{ left: HEADER_WIDTH + detail.from * view.scale, width: Math.max(1, (detail.to - detail.from) * view.scale) }} />}
        {indicator !== null && <div className="timeline-indicator" data-test="smart-indicator" data-frame={indicator} aria-hidden="true"
          style={{ left: HEADER_WIDTH + indicator * view.scale }} />}
        <div className="timeline-playhead" style={{ left: HEADER_WIDTH + shown * view.scale }} aria-hidden="true" onPointerDown={down} />
      </div>
    </div>}
  </section>;
}
