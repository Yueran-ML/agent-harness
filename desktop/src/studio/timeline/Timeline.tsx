import { memo, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type PointerEvent, type Ref } from 'react';
import { LocateFixed, Maximize2, Minus, Plus, X, MousePointer2, Scissors, Magnet, Link, UnfoldHorizontal } from 'lucide-react';
import { useWords } from '../../app/locale';
import { keyOf, type Key } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import type { Media, Monitor, Op, Rate, Timeline as TimelineModel, Track } from '../engine';
import { editingAllowed } from '../keys';
import { LET_GO_HOLD_MS } from '../Transport';
import { ClipView } from './ClipView';
import { duration, fitZoom, frameAt, HEADER_WIDTH, RULER_HEIGHT, transitionSpan, visible, zoomAt } from './geometry';
import { Ruler } from './Ruler';
import { Scrub } from './scrub';
import { TrackHeader } from './TrackHeader';
import { Waveforms } from './waveforms';
import { useEditing } from './useEditing';
import type { EditContext } from './editKeys';
import './timeline.css';

const TrackRow = memo(function TrackRow({ timeline, track, media, rate, scale, from, to, height, waves, onEdit, onHeight, selected, targeted, onTarget, chosen = null }: {
  timeline: string; track: Track; media: ReadonlyMap<string, Media>; rate: Rate; scale: number; from: number; to: number;
  height: number; waves: Waveforms; onEdit: (ops: Op[]) => Promise<boolean>; onHeight: (id: string, height: number) => void;
  selected: ReadonlySet<string>; targeted: boolean; onTarget: (id: string) => void;
  /** The transition chosen, where it is on this track. */
  chosen?: string | null;
}) {
  const words = useWords().label.studio.timeline;
  const transitions = useMemo(() => {
    if (track.kind === 'subtitle') return [];
    const starts = new Map(track.clips.map(clip => [clip.id, clip.start]));
    return track.transitions.map(transition => ({ transition, ...transitionSpan(transition, starts.get(transition.right)!) }));
  }, [track]);
  return <div className="timeline-track" data-track={track.id} data-kind={track.kind} data-enabled={track.enabled} data-locked={track.locked} style={{ height }}>
    <TrackHeader timeline={timeline} track={track} height={height} onEdit={onEdit} onHeight={onHeight} targeted={targeted} onTarget={onTarget} />
    <div className="timeline-lane" role={track.kind === 'subtitle' ? undefined : 'listbox'} aria-label={track.name} aria-multiselectable={track.kind === 'subtitle' ? undefined : true}>
      {track.kind === 'subtitle' ? visible(track.cues, from, to).map(cue => <div key={cue.id} className="timeline-cue" data-cue={cue.id}
        style={{ left: cue.start * scale, width: cue.length * scale }} title={cue.text}>{cue.text}</div>)
        : visible(track.clips, from, to).map(clip => <ClipView key={clip.id} clip={clip} kind={track.kind} selected={selected.has(clip.id)}
          media={clip.source.kind === 'media' ? media.get(clip.source.media) : undefined} rate={rate} scale={scale}
          from={track.kind === 'audio' ? from : clip.start} to={track.kind === 'audio' ? to : clip.start + clip.length} waves={waves} />)}
      {transitions.filter(span => span.start < to && span.start + span.length > from).map(({ transition, start, length }) => <div
        key={transition.id} className="timeline-transition" data-transition={transition.id} data-kind={transition.kind} data-selected={chosen === transition.id}
        style={{ left: start * scale, width: length * scale }} title={words[transition.kind]}><span>{words[transition.kind]}</span></div>)}
    </div>
  </div>;
});

/** A view of the accepted engine project. Scrolling, zoom and track height do not add undo steps. */
export interface TimelineHandle { keyDown: (key: Key) => boolean }
export function Timeline({ timeline, media, bridge, session, revision, frame, playing, onSeek, source = null, focus = 'timeline', onSpeed, onSelection, onTransition, ref }: {
  timeline: TimelineModel | null; media: readonly Media[]; bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number;
  revision: number; frame: number; playing: boolean; onSeek: (frame: number, scrubbing: boolean) => void;
  source?: EditContext['source']; focus?: Monitor; onSpeed?: (clips: string[]) => void; onSelection?: (clips: string[]) => void;
  onTransition?: (transition: string | null) => void; ref?: Ref<TimelineHandle>;
}) {
  const words = useWords();
  const labels = words.label.studio.timeline;
  const root = useRef<HTMLElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const draw = useRef<number | null>(null);
  const [view, setView] = useState({ scale: 4, left: 0, width: 800 });
  const [heights, setHeights] = useState<Record<string, number>>({});
  const [follow, setFollow] = useState(true);
  const [held, setHeld] = useState<{ frame: number; dragging: boolean } | null>(null);
  const drag = useRef<{ frame: number } | null>(null);
  const now = useRef({ view, onSeek });
  now.current = { view, onSeek };
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

  const zoom = useCallback((factor: number, anchor?: number) => setView(old => {
    const next = zoomAt(old.scale, old.left, anchor ?? old.width / 2, factor);
    return { ...old, scale: next.scale, left: next.scroll };
  }), []);
  const fit = () => setView(old => ({ ...old, scale: fitZoom(end, old.width), left: 0 }));
  const editing = useEditing({ timeline, media, bridge, session, revision, frame, focus, source, root, scroll, view, onSeek, fit, zoom, onSpeed, onSelection, onTransition });
  useImperativeHandle(ref, () => ({ keyDown: editing.keyDown }));
  useEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        const anchor = Math.max(0, Math.min(now.current.view.width, event.clientX - element.getBoundingClientRect().left - HEADER_WIDTH));
        zoom(Math.exp(-event.deltaY * 0.002), anchor);
      } else if (event.shiftKey && event.deltaX === 0) {
        event.preventDefault();
        element.scrollLeft = Math.max(0, element.scrollLeft + event.deltaY);
        setView(old => ({ ...old, left: element.scrollLeft }));
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
  const from = Math.max(0, Math.floor(view.left / view.scale));
  const to = Math.ceil((view.left + view.width) / view.scale);
  const total = HEADER_WIDTH + Math.max(view.width, end * view.scale + view.width / 2);
  let rowTop = RULER_HEIGHT;
  const rows = new Map(tracks.map(track => { const height = heights[track.id] ?? 72; const row = { top: rowTop, height }; rowTop += height; return [track.id, row]; }));
  return <section ref={root} className="timeline" role="region" aria-label={labels.title} aria-busy={editing.pending} tabIndex={0} data-timeline={timeline?.id} data-scale={view.scale} data-tool={editing.tool}
    onKeyDown={event => {
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      if (editing.keyDown(key)) { event.preventDefault(); event.stopPropagation(); }
    }}
    onPointerDown={editing.pointerDown}
    onDragOver={editing.dragOver} onDrop={editing.drop} onDragLeave={editing.dragLeave}
    onPointerMove={event => { if (drag.current) { const frame = at(event); drag.current.frame = frame; setHeld({ frame, dragging: true }); scrub.drag(frame); } else editing.pointerMove(event); }}
    onPointerUp={event => { release(event); editing.pointerUp(event); }} onPointerCancel={event => { release(event, true); editing.cancel(); }} onLostPointerCapture={editing.cancel}>
    <header className="timeline-toolbar">
      <strong>{timeline?.name ?? labels.title}</strong>
      {([['select', 'A', MousePointer2], ['trim', 'T', UnfoldHorizontal], ['blade', 'B', Scissors]] as const).map(([tool, key, Icon]) => <button key={tool} type="button" className="icon-btn"
        aria-label={labels[tool]} title={`${labels[tool]} (${key})`} aria-pressed={editing.tool === tool} onClick={() => editing.chooseTool(tool)}><Icon size={14} /></button>)}
      <button type="button" className="icon-btn" aria-label={labels.snap} title={`${labels.snap} (N)`} aria-pressed={editing.snapping} onClick={() => editing.setSnapping(value => !value)}><Magnet size={14} /></button>
      <button type="button" className="icon-btn" aria-label={labels.linked} title={`${labels.linked} (Ctrl+Shift+L)`} aria-pressed={editing.linked} onClick={() => editing.setLinked(value => !value)}><Link size={14} /></button>
      <button type="button" className="icon-btn" aria-label={labels.zoomOut} title={`${labels.zoomOut} (Ctrl+−)`} onClick={() => zoom(0.8)}><Minus size={14} /></button>
      <button type="button" className="icon-btn" aria-label={labels.zoomIn} title={`${labels.zoomIn} (Ctrl++)`} onClick={() => zoom(1.25)}><Plus size={14} /></button>
      <button type="button" className="icon-btn" aria-label={labels.fit} title={`${labels.fit} (Shift+Z)`} onClick={fit}><Maximize2 size={14} /></button>
      <button type="button" className="icon-btn" aria-label={labels.follow} title={labels.follow} aria-pressed={follow} onClick={() => setFollow(value => !value)}><LocateFixed size={14} /></button>
    </header>
    {editing.problem && <div className="timeline-problem" role="alert"><span>{editing.problem}</span><button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => editing.setProblem(null)}><X size={14} /></button></div>}
    {!timeline ? <p className="caption timeline-empty">{words.hint.studio.timeline.empty}</p> : <div ref={scroll} className="timeline-scroll" onScroll={onScroll}>
      <div className="timeline-content" style={{ width: total, minHeight: RULER_HEIGHT + tracks.reduce((sum, track) => sum + (heights[track.id] ?? 72), 0) }}>
        <div className="timeline-ruler-row"><Ruler rate={timeline.rate} scale={view.scale} from={view.left / view.scale} to={(view.left + view.width) / view.scale}
          frame={shown} last={last} onDown={down} onSeek={onSeek} />
          {editing.marks.in !== null && <span className="timeline-mark" data-edge="in" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale }} />}
          {editing.marks.out !== null && <span className="timeline-mark" data-edge="out" style={{ left: HEADER_WIDTH + editing.marks.out * view.scale }} />}
          {editing.marks.in !== null && editing.marks.out !== null && <span className="timeline-mark-range" style={{ left: HEADER_WIDTH + editing.marks.in * view.scale, width: (editing.marks.out - editing.marks.in) * view.scale }} />}
          <div className="timeline-ruler-corner" /></div>
        {tracks.map(track => <TrackRow key={track.id} timeline={timeline.id} track={track} media={byId} rate={timeline.rate} scale={view.scale}
          from={from} to={to} height={heights[track.id] ?? 72} waves={waves} onEdit={editing.edit} onHeight={onHeight}
          selected={editing.selected} targeted={editing.targets.video_track === track.id || editing.targets.audio_track === track.id} onTarget={editing.target}
          chosen={editing.transition !== null && track.kind !== 'subtitle' && track.transitions.some(each => each.id === editing.transition) ? editing.transition : null} />)}
        {editing.preview?.preview.map(item => { const row = rows.get(item.track); return row && <div key={item.id} className="timeline-preview" data-preview={item.id} data-clamped={editing.preview!.clamped}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.preview?.covered.map(item => { const row = rows.get(item.track); return row && <div key={`covered-${item.id}`} className="timeline-covered" data-covered={item.id}
          style={{ left: HEADER_WIDTH + item.start * view.scale, width: item.length * view.scale, top: row.top + 4, height: row.height - 8 }} />; })}
        {editing.preview?.point != null && <div className="timeline-snap" style={{ left: HEADER_WIDTH + editing.preview.point * view.scale }} />}
        {editing.dropAt !== null && <div className="timeline-snap" style={{ left: HEADER_WIDTH + editing.dropAt * view.scale }} />}
        <div className="timeline-playhead" style={{ left: HEADER_WIDTH + shown * view.scale }} aria-hidden="true" onPointerDown={down} />
      </div>
    </div>}
  </section>;
}
