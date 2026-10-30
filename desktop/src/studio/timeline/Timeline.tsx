import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type Ref } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import { keyOf, type Key } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import type { Media, Monitor, Timeline as TimelineModel } from '../engine';
import { editingAllowed } from '../keys';
import { LET_GO_HOLD_MS } from '../Transport';
import { duration, fitZoom, frameAt, HEADER_WIDTH, RULER_HEIGHT, scrollWindow, zoomAt } from './geometry';
import { Ruler } from './Ruler';
import { Scrub } from './scrub';
import { AddSubtitleTrack, pageLanguage, TimelineMenu, type TrackMenu } from './SubtitleTrack';
import { Toolbar } from './Toolbar';
import { TrackRow } from './TrackRow';
import { Waveforms } from './waveforms';
import { useEditing } from './useEditing';
import type { EditAction, EditContext } from './editKeys';
import type { Marks } from './snap';
import './timeline.css';

/** A view of the accepted engine project. Scrolling, zoom and track height do not add undo steps. */
export interface TimelineHandle {
  keyDown: (key: Key) => boolean;
  /** Do an edit that a button of the page asks for, as its key would. */
  perform: (action: EditAction) => boolean;
  /** The In and Out points of the timeline, which exist only on the page: an export may be of the part between them. */
  marks: () => Marks;
}
/** What of a timeline a view shows: the pixels of a frame, how far it is scrolled, and how wide it is. */
export interface TimelineView { scale: number; left: number; width: number }
export function Timeline({ timeline, media, bridge, session, revision, frame, playing, onSeek, source = null, focus = 'timeline', onSpeed, onSelection, onTransition, onCue, ref,
  cut = false, overview = false, center = false, trackHeight = 72, indicator = null, detail = null, onView, label }: {
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
  /** The name of the region, where a page shows more than one view of a timeline. */
  label?: string;
}) {
  const words = useWords();
  const labels = words.label.studio.timeline;
  const root = useRef<HTMLElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const draw = useRef<number | null>(null);
  const [view, setView] = useState({ scale: 4, left: 0, width: 800 });
  const [heights, setHeights] = useState<Record<string, number>>({});
  const [follow, setFollow] = useState(true);
  // The menu of a right-click, where one is open.
  const [menu, setMenu] = useState<TrackMenu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  const [held, setHeld] = useState<{ frame: number; dragging: boolean } | null>(null);
  const drag = useRef<{ frame: number } | null>(null);
  const now = useRef({ view, onSeek, overview, frame, heights, trackHeight });
  now.current = { view, onSeek, overview, frame, heights, trackHeight };
  const scrub = useMemo(() => new Scrub((to, scrubbing) => now.current.onSeek(to, scrubbing)), []);
  const waves = useMemo(() => new Waveforms(bridge, session), [bridge, session]);
  useEffect(() => { waves.connect(); return () => waves.disconnect(); }, [waves]);
  useEffect(() => () => scrub.cancel(), [scrub]);
  const byId = useMemo(() => new Map(media.map(item => [item.id, item])), [media]);
  const end = useMemo(() => timeline ? duration(timeline) : 0, [timeline]);
  const last = Math.max(0, end - 1);
  const shown = held?.frame ?? frame;
  const tracks = useMemo(() => timeline ? [
    ...timeline.tracks.filter(track => track.kind === 'subtitle').reverse(),
    ...timeline.tracks.filter(track => track.kind === 'video').reverse(),
    ...timeline.tracks.filter(track => track.kind === 'audio'),
  ] : [], [timeline]);

  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const measure = () => {
      const width = Math.max(1, element.clientWidth - HEADER_WIDTH);
      setView(old => old.width === width && old.left === element.scrollLeft ? old : { ...old, width, left: element.scrollLeft });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => { observer.disconnect(); if (draw.current !== null) cancelAnimationFrame(draw.current); draw.current = null; };
  }, [timeline?.id]);
  useLayoutEffect(() => { if (scroll.current && scroll.current.scrollLeft !== view.left) scroll.current.scrollLeft = view.left; }, [view.left, view.scale]);
  useEffect(() => {
    scrub.cancel(); drag.current = null; setHeld(null);
    setView(old => ({ ...old, left: 0 }));
  }, [timeline?.id, session, scrub]);
  useEffect(() => {
    if (held && !held.dragging && (frame === held.frame || playing)) setHeld(null);
  }, [frame, playing, held]);
  useEffect(() => {
    if (!held || held.dragging) return;
    const timer = setTimeout(() => setHeld(null), LET_GO_HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  useEffect(() => {
    if (!follow || !playing || drag.current) return;
    setView(old => {
      const x = frame * old.scale;
      return x >= old.left && x <= old.left + old.width * 0.9 ? old : { ...old, left: Math.max(0, x - old.width * 0.15) };
    });
  }, [frame, playing, follow]);

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
  }), []);
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
  }), []);
  // The whole timeline is always shown in an overview, as it grows and as the view is made wider.
  useEffect(() => {
    if (!overview) return;
    setView(old => { const scale = fitZoom(end, old.width); return old.scale === scale && old.left === 0 ? old : { ...old, scale, left: 0 }; });
  }, [overview, end, view.width]);
  // The playhead in the middle of a view that keeps it there, unless the playhead is being dragged.
  const dragging = held?.dragging === true;
  useEffect(() => {
    if (!center || dragging) return;
    setView(old => { const left = Math.max(0, Math.round(frame * old.scale - old.width / 2)); return old.left === left ? old : { ...old, left }; });
  }, [center, dragging, frame, view.scale, view.width]);
  useEffect(() => { onView?.(view); }, [view, onView]);
  const editing = useEditing({ timeline, media, bridge, session, revision, frame, focus, source, root, scroll, view, onSeek, fit, zoom, onSpeed, onSelection, onTransition, onCue, cut });
  useImperativeHandle(ref, () => ({ keyDown: editing.keyDown, perform: editing.perform, marks: () => editing.marks }));
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
  }, [timeline?.id, zoom]);
  const onScroll = () => {
    if (draw.current !== null) return;
    draw.current = requestAnimationFrame(() => {
      draw.current = null;
      if (scroll.current) { const left = scroll.current.scrollLeft; setView(old => old.left === left ? old : { ...old, left }); }
    });
  };
  const onHeight = useCallback((id: string, height: number) => setHeights(old => ({ ...old, [id]: height })), []);
  const at = (event: PointerEvent) => Math.min(last, frameAt(event.clientX - (scroll.current?.getBoundingClientRect().left ?? 0) - HEADER_WIDTH, view.scale, scroll.current?.scrollLeft ?? view.left));
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
      // A press on a track of the overview, not on what lies on it, moves the playhead there.
      if (overview && event.target instanceof Element && event.target.classList.contains('timeline-lane')) down(event);
      else editing.pointerDown(event);
    }}
    onDragOver={editing.dragOver} onDrop={editing.drop} onDragLeave={editing.dragLeave} onDoubleClick={editing.doubleClick}
    onContextMenu={event => {
      // As DaVinci Resolve: a track header offers Add Subtitle Track, a track of subtitles Add Subtitle (Reference Manual 20, pp. 1088–1089).
      const track = !overview && timeline && event.target instanceof Element ? event.target.closest<HTMLElement>('[data-track]') : null;
      if (!track || !(event.target instanceof Element)) return;
      const items: TrackMenu['items'] = event.target.closest('.timeline-track-header')
        ? [{ id: 'add-subtitle-track', label: labels.addSubtitleTrack, disabled: editing.pending, run: () => editing.addSubtitleTrack(pageLanguage()) }]
        : track.dataset.kind === 'subtitle' ? [{ id: 'add-subtitle', label: labels.addCue, disabled: track.dataset.locked === 'true' || editing.pending, run: () => editing.addCue(track.dataset.track!) }] : [];
      if (items.length === 0) return;
      event.preventDefault();
      const box = root.current!.getBoundingClientRect();
      setMenu({ x: event.clientX - box.left, y: event.clientY - box.top, items });
    }}
    onPointerMove={event => { if (drag.current) { const frame = at(event); drag.current.frame = frame; setHeld({ frame, dragging: true }); scrub.drag(frame); } else editing.pointerMove(event); }}
    onPointerUp={event => { release(event); editing.pointerUp(event); }} onPointerCancel={event => { release(event, true); editing.cancel(); }} onLostPointerCapture={editing.cancel}>
    {!overview && <Toolbar name={timeline?.name ?? labels.title} tool={editing.tool} snapping={editing.snapping} linked={editing.linked} follow={follow}
      onTool={editing.chooseTool} onSnapping={editing.setSnapping} onLinked={editing.setLinked} onZoom={zoom} onFit={fit} onFollow={setFollow} />}
    {menu && <TimelineMenu menu={menu} onClose={closeMenu} />}
    {editing.problem && <div className="timeline-problem" role="alert"><span>{editing.problem}</span><button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => editing.setProblem(null)}><X size={14} /></button></div>}
    {!timeline ? <p className="caption timeline-empty">{words.hint.studio.timeline.empty}</p> : <div ref={scroll} className="timeline-scroll" onScroll={onScroll}>
      <div className="timeline-content" style={{ width: total, minHeight: RULER_HEIGHT + tracks.reduce((sum, track) => sum + (heights[track.id] ?? trackHeight), 0), '--scale': view.scale } as CSSProperties}>
        <div className="timeline-ruler-row"><Ruler rate={timeline.rate} scale={view.scale} from={from} to={to}
          frame={shown} last={last} onDown={down} onSeek={onSeek} />
          {editing.marks.in !== null && <span className="timeline-mark" data-edge="in" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale }} />}
          {editing.marks.out !== null && <span className="timeline-mark" data-edge="out" style={{ left: HEADER_WIDTH + editing.marks.out * view.scale }} />}
          {editing.marks.in !== null && editing.marks.out !== null && <span className="timeline-mark-range" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale, width: (editing.marks.out - editing.marks.in) * view.scale }} />}
          <div className="timeline-ruler-corner">{!overview && <AddSubtitleTrack onAdd={editing.addSubtitleTrack} disabled={editing.pending} />}</div></div>
        {tracks.map(track => <TrackRow key={track.id} timeline={timeline.id} track={track} media={byId} rate={timeline.rate} scale={view.scale}
          from={from} to={to} height={heights[track.id] ?? trackHeight} waves={waves} onEdit={editing.edit} onHeight={onHeight}
          selected={editing.selected} targeted={editing.targets.video_track === track.id || editing.targets.audio_track === track.id} onTarget={editing.target}
          chosen={editing.transition !== null && track.kind !== 'subtitle' && track.transitions.some(each => each.id === editing.transition) ? editing.transition : null}
          chosenCue={editing.cue !== null && track.kind === 'subtitle' && track.cues.some(each => each.id === editing.cue) ? editing.cue : null}
          shownSubtitles={track.kind === 'subtitle' ? shownSubtitles : null} />)}
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
