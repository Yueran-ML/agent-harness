import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Expand, Film, FolderOpen, GalleryHorizontalEnd, Images, Layers3, Maximize2, Minimize2, Workflow } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type ClipTrack, type Group, type Media, type TimelineTransportState, type Timeline as TimelineModel } from '../engine';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import { Thumbnails } from '../media/Thumbnails';
import { rateOf, timecode } from '../timecode';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { useTimelineLoad } from './useTimelineLoad';
import {
  CLIP_FILTERS, currentClip, gradeOf, keptBy, levelsOf, numberedClips, shownClips, sourceFrameAt, stepClip,
  type ClipFilter, type GradeLevel,
} from '../color/clips';
import { MiniTimeline } from '../color/MiniTimeline';
import { NodeEditor } from '../color/NodeEditor';
import { useColorPageGrades } from '../color/useColorPageGrades';
import { Palettes, type CenterPalette, type LeftPalette, type RightPanel } from '../color/Palettes';
import { useColorScopes } from '../color/Scopes';
import { ThumbnailTimeline } from '../color/ThumbnailTimeline';
import { placeOf, type EditedNode } from '../color/useNodeEdit';
import '../color/color.css';

/** Narrower than this, the left and center palettes are merged into one group (Reference Manual 20, p. 2976). */
export const MERGE_PALETTES_PX = 1280;

/** A LUT that a grade of the timeline names and the engine could not read (PROTOCOL.md, "Grades in the picture"). */
export interface MissingLut { name: string; message: string }
/** The state of the timeline monitor with what T13a added to it, which `engine.ts` does not type yet. */
type TimelineState = TimelineTransportState & { missing_luts?: MissingLut[] };

/**
 * How large the viewer is (pp. 2968–2969, 2993–2995): as laid out; Enhanced (Alt+F), over all that
 * lies beside and beneath it down to the palettes; Full page (Shift+F), over the palettes too; or
 * Cinema (P), over the whole window, its transport shown while the pointer is on it.
 */
export type ViewerMode = 'normal' | 'enhanced' | 'full' | 'cinema';
/** What lies at the top left, in the place of the gallery: one of three, or none (p. 2972). */
export type SidePanel = 'gallery' | 'luts' | 'pool';
/** The parts of the page that take the focus, the one that has it marked at its top edge (p. 2967). */
type Area = 'side' | 'viewer' | 'nodes' | 'thumbnails' | 'mini' | 'palettes';
/** What the timecode in the head of the viewer shows (p. 2993): the source timecode at first. */
type HeadTimecode = 'source' | 'record' | 'sourceFrame' | 'recordFrame';

const NO_GROUPS: readonly Group[] = [];

/** Whether the monitor plays in real time: it does unless a frame came late in the last second of playing. */
function useRealTime(state: TimelineState | null): 'stopped' | 'realtime' | 'dropping' {
  const seen = useRef({ late: 0, at: -Infinity });
  const [, redraw] = useState(0);
  const late = state?.late ?? 0;
  const playing = state?.playing ?? false;
  if (late > seen.current.late) seen.current.at = Date.now();
  seen.current.late = late;
  const dropping = playing && Date.now() - seen.current.at < 1000;
  useEffect(() => {
    if (!dropping) return undefined;
    const timer = setTimeout(() => redraw(count => count + 1), 1000);
    return () => clearTimeout(timer);
  }, [dropping, late]);
  return !playing ? 'stopped' : dropping ? 'dropping' : 'realtime';
}

/**
 * The Color page, laid out as DaVinci Resolve's (Reference Manual 20, pp. 2966–2977, and the
 * interaction reference, §1–2): the Interface toolbar at the top; beneath it the gallery (or the LUT
 * browser, or the media pool) at the left, the viewer of the timeline, and the node editor at the
 * right; then the thumbnail timeline and the mini-timeline; and the palettes at the foot.
 *
 * The viewer is the timeline monitor of the Cut and Edit pages, with its transport and its keys.
 * The current clip is the one at the playhead: its grade is in the node editor and its thumbnail is
 * outlined in orange. Up and Down go to the clip before and after it. Alt+F, Shift+F and P make the
 * viewer larger. The palettes, the gallery and the LUT browser are placeholders until T17–T19, T26;
 * the node editor shows the grade, and edits it from T16.
 */
export function ColorPage({
  bridge, native, session, revision, pool, media, timelines, groups = NO_GROUPS, selected, onSelect, timelineMonitor, noticeOf, onRetry, onProblem,
}: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'>;
  native: boolean;
  session: number;
  revision: number;
  /** The media pool, which takes the place of the gallery when it is opened. */
  pool: ReactNode;
  media: readonly Media[];
  timelines: readonly TimelineModel[];
  groups?: readonly Group[];
  /** The timeline that the page shows and the timeline monitor plays. */
  selected: TimelineModel | null;
  onSelect: (timeline: string) => void;
  timelineMonitor: MonitorControls;
  noticeOf: (monitor: 'timeline') => ViewerNotice | null;
  onRetry: () => void;
  onProblem: (said: string, failure: unknown) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const said = words.hint.studio.colorPage;
  const page = useRef<HTMLDivElement>(null);
  const [merged, setMerged] = useState(false);
  const [side, setSide] = useState<SidePanel | null>('gallery');
  const [clipsOpen, setClipsOpen] = useState(true);
  const [miniOpen, setMiniOpen] = useState(true);
  const [nodesOpen, setNodesOpen] = useState(true);
  const [nodesExpanded, setNodesExpanded] = useState(false);
  const [filter, setFilter] = useState<ClipFilter>('all');
  const [viewerMode, setViewerMode] = useState<ViewerMode>('normal');
  const [area, setArea] = useState<Area>('viewer');
  const [headTimecode, setHeadTimecode] = useState<HeadTimecode>('source');
  // The clip chosen last, which is current where it lies at the playhead with others; the clips chosen together.
  const [chosen, setChosen] = useState<string | null>(null);
  const [chosenClips, setChosenClips] = useState<ReadonlySet<string>>(new Set());
  const anchor = useRef<string | null>(null);
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(new Set());
  const [pictures, setPictures] = useState<Record<string, number>>({});
  const [level, setLevel] = useState<GradeLevel>('clip');
  // The node chosen in each grade shown, by level and clip (p. 3314: a clip keeps the node chosen in it last).
  const [nodes, setNodes] = useState<Record<string, string>>({});
  const [left, setLeft] = useState<LeftPalette>('primaries');
  const [center, setCenter] = useState<CenterPalette>('curves');
  const [last, setLast] = useState<'left' | 'center'>('left');
  const [right, setRight] = useState<RightPanel>('scopes');
  const thumbnails = useMemo(() => new Thumbnails(bridge), [bridge, session]);
  const now = useRef({ onProblem, words });
  now.current = { onProblem, words };
  const failed = useCallback((failure: unknown) => now.current.onProblem(now.current.words.hint.studio.failed, failure), []);

  useLayoutEffect(() => {
    const element = page.current;
    if (!element) return undefined;
    const measure = () => {
      const width = element.getBoundingClientRect().width;
      if (width > 0) setMerged(width < MERGE_PALETTES_PX);
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, []);

  // A timeline chosen anew, or another engine: what was chosen of the one before goes.
  useEffect(() => { setChosen(null); setChosenClips(new Set()); setSkipped(new Set()); setPictures({}); setNodes({}); }, [selected?.id, session]);

  const playing = useTimelineLoad(bridge, selected, timelineMonitor, session, failed);
  const timelineState = timelineMonitor.state?.monitor === 'timeline' && selected !== null && playing === selected.id
    ? timelineMonitor.state as TimelineState : null;
  const frame = timelineState === null ? null : timelineMonitor.frame ?? 0;
  const mediaById = useMemo(() => new Map(media.map(each => [each.id, each])), [media]);
  const numbered = useMemo(() => numberedClips(selected), [selected]);
  const numbers = useMemo(() => new Map(numbered.map(each => [each.clip.id, each.number])), [numbered]);
  const visible = useMemo(() => shownClips(numbered, skipped), [numbered, skipped]);
  const listed = useMemo(() => visible.filter(each => keptBy(filter, each.clip)), [visible, filter]);
  const current = currentClip(visible, frame, chosen);
  const levels = levelsOf(current?.clip ?? null, groups);
  const shownLevel = levels.includes(level) ? level : 'clip';
  const grade = gradeOf(shownLevel, selected, current?.clip ?? null, groups);
  const gradeKey = `${shownLevel}:${shownLevel === 'timeline' ? selected?.id ?? '' : current?.clip.id ?? ''}`;
  const currentNode = grade === null ? null
    : grade.nodes.some(node => node.id === nodes[gradeKey]) ? nodes[gradeKey] : grade.nodes[0]?.id ?? null;
  const onCurrentNode = useCallback((node: string) => setNodes(old => ({ ...old, [gradeKey]: node })), [gradeKey]);
  // The grade shown, as the edits name it; undo and redo, = and -, whole grades copied and pasted, and the menu of a thumbnail.
  const visibleClips = useMemo(() => visible.map(each => each.clip), [visible]);
  const grades = useColorPageGrades({
    bridge, revision, session, area, level: shownLevel, timeline: selected, clips: visibleClips, current: current?.clip ?? null, chosen: chosenClips,
  });
  // The node that the palettes edit: the current node, where it is, with its grade and the playhead as its keys count it (T17).
  const placed = placeOf(shownLevel, selected?.id ?? null, current?.clip ?? null, frame ?? 0);
  const editedNode = grade?.nodes.find(node => node.id === currentNode) ?? null;
  const edited = useMemo<EditedNode | null>(() => (placed === null || editedNode === null ? null
    : { bridge, revision, target: placed.target, node: editedNode, frame: placed.frame, onFailure: failed }),
  [bridge, revision, JSON.stringify(placed), editedNode, failed]);

  // Going to a clip: it is chosen, alone, and the playhead goes to its first frame (p. 3024).
  const go = useCallback((clip: string, start: number) => {
    setChosen(clip); setChosenClips(new Set([clip])); anchor.current = clip;
    timelineMonitor.seek(start, false);
  }, [timelineMonitor.seek]);
  const pick = useCallback((clip: string, how: { ctrl: boolean; shift: boolean }) => {
    const entry = listed.find(each => each.clip.id === clip);
    if (!entry) return;
    if (how.ctrl) {
      // Ctrl+click adds a clip to those chosen, or takes it away, and leaves the playhead (pp. 3049, 3258).
      setChosenClips((old) => { const next = new Set(old); if (next.has(clip)) next.delete(clip); else next.add(clip); return next; });
      anchor.current = clip;
      return;
    }
    if (how.shift && anchor.current !== null) {
      const from = listed.findIndex(each => each.clip.id === anchor.current);
      const to = listed.indexOf(entry);
      if (from >= 0) { setChosenClips(new Set(listed.slice(Math.min(from, to), Math.max(from, to) + 1).map(each => each.clip.id))); return; }
    }
    go(clip, entry.clip.start);
  }, [listed, go]);
  const onPicture = useCallback((clip: string, offset: number) => setPictures(old => ({ ...old, [clip]: offset })), []);
  const clipPressed = useCallback((clip: string) => {
    const entry = numbered.find(each => each.clip.id === clip);
    if (entry) go(clip, entry.clip.start);
  }, [numbered, go]);
  const enable = useCallback((track: ClipTrack) => {
    if (selected === null) return;
    void callEngine(bridge, 'transaction.apply', { revision, ops: [{ op: 'set_track', timeline: selected.id, id: track.id, enabled: !track.enabled }] }).catch(failed);
  }, [bridge, selected, revision, failed]);
  const skip = useCallback((track: ClipTrack) => setSkipped((old) => {
    const next = new Set(old); if (next.has(track.id)) next.delete(track.id); else next.add(track.id); return next;
  }), []);

  // The keys of the transport go to the timeline monitor; Up and Down go from clip to clip, and three keys make the viewer larger.
  useStudioKeys(timelineMonitor.loaded, timelineMonitor.context, timelineMonitor.run);
  const keys = useRef({ listed, current, frame, go, viewerMode });
  keys.current = { listed, current, frame, go, viewerMode };
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const letter = key.key.length === 1 ? key.key.toLowerCase() : key.key;
      const plain = !key.ctrl && !key.alt && !key.shift && !key.meta;
      const state = keys.current;
      if (plain && (letter === 'ArrowUp' || letter === 'ArrowDown')) {
        event.preventDefault();
        const to = stepClip(state.listed, state.current, state.frame ?? 0, letter === 'ArrowUp' ? -1 : 1);
        if (to !== null && state.frame !== null) state.go(to.clip.id, to.clip.start);
        return;
      }
      if (key.repeat) return;
      const toggle = (mode: ViewerMode) => { event.preventDefault(); setViewerMode(old => (old === mode ? 'normal' : mode)); };
      // Alt+F, Shift+F and P (pp. 2968–2969): by the key where it lies, as Alt changes what a letter types on some layouts.
      const isF = key.code === 'KeyF' || letter === 'f';
      if (isF && key.alt && !key.ctrl && !key.shift && !key.meta) toggle('enhanced');
      else if (isF && key.shift && !key.ctrl && !key.alt && !key.meta) toggle('full');
      else if (plain && (key.code === 'KeyP' || letter === 'p')) toggle('cinema');
      else if (plain && letter === 'Escape' && state.viewerMode === 'cinema') { event.preventDefault(); setViewerMode('normal'); }
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  // The video scopes of the timeline monitor (T19): docked in the Scopes panel, or in their floating window.
  const scopes = useColorScopes({ bridge, session, view: 'timeline', loaded: timelineState !== null && timelineMonitor.loaded, native, viewerMode });
  const realTime = useRealTime(timelineState);
  const missing = timelineState?.missing_luts ?? [];
  const currentMedia = current?.clip.source.kind === 'media' ? mediaById.get(current.clip.source.media) : undefined;
  const headValue = (() => {
    if (frame === null || selected === null) return '';
    if (headTimecode === 'record') return timecode(frame, selected.rate);
    if (headTimecode === 'recordFrame') return String(frame);
    const source = current === null ? null : sourceFrameAt(current.clip, frame - current.clip.start, selected.rate, currentMedia);
    if (source === null) return headTimecode === 'source' ? '--:--:--:--' : '--';
    return headTimecode === 'source' ? timecode(source, rateOf(currentMedia)) : String(source);
  })();

  const focusOf = (each: Area) => ({
    'data-focused': area === each || undefined,
    onPointerDownCapture: () => setArea(each),
    onFocusCapture: () => setArea(each),
  });
  const sideButton = (panel: SidePanel, name: string, Icon: typeof Images) => <button type="button" className="color-tool" aria-pressed={side === panel}
    onClick={() => setSide(old => (old === panel ? null : panel))} data-test={`toggle-${panel}`}><Icon size={14} aria-hidden="true" />{name}</button>;
  const viewerHead = <div className="color-viewer-head">
    <select className="field edit-timelines" aria-label={words.label.studio.timeline.title} value={selected?.id ?? ''} disabled={timelines.length === 0}
      onChange={(event) => { if (event.target.value) { onSelect(event.target.value); event.currentTarget.blur(); } }} data-test="timeline-picker">
      {timelines.length === 0 && <option value="">{words.label.studio.editPage.noTimeline}</option>}
      {timelines.map(each => <option key={each.id} value={each.id}>{each.name}</option>)}
    </select>
    <span className="color-realtime" data-state={realTime} role="img" aria-label={labels[realTime]} title={labels[realTime]} data-test="realtime" />
    <span className="color-gap" />
    <output className="color-head-timecode" aria-label={labels.timecode} data-test="head-timecode">{headValue}</output>
    <select className="field color-head-kind" aria-label={labels.timecode} value={headTimecode} onChange={event => setHeadTimecode(event.target.value as HeadTimecode)}
      data-test="head-timecode-kind">
      {(['source', 'record', 'sourceFrame', 'recordFrame'] as const).map(each => <option key={each} value={each}>{labels.timecodes[each]}</option>)}
    </select>
    <button type="button" className="icon-btn" aria-pressed={viewerMode === 'enhanced'} aria-label={labels.enhanced}
      title={`${labels.enhanced} (Alt+F)`}
      onClick={event => setViewerMode(old => (event.altKey ? (old === 'full' ? 'normal' : 'full') : old === 'enhanced' ? 'normal' : 'enhanced'))}
      data-test="enhanced-viewer">{viewerMode === 'normal' ? <Maximize2 size={14} /> : <Minimize2 size={14} />}</button>
  </div>;
  // Where a LUT of the timeline cannot be read, the viewer says so at its lower right (p. 3413): the one by its name, more as more.
  const missingNotice = missing.length > 0 && <p className="color-missing-luts" role="status"
    title={missing.map(each => `${each.name}: ${each.message}`).join('\n')} data-test="missing-luts">
    {missing.length === 1 ? labels.missingLut(missing[0].name) : labels.missingLuts}
  </p>;
  const viewer = <section className="color-viewer" aria-label={labels.viewer} data-test="color-viewer" {...focusOf('viewer')}>
    <MonitorPane controls={timelineMonitor} bridge={bridge} native={native} active hidden={nodesOpen && nodesExpanded && viewerMode === 'normal'}
      notice={noticeOf('timeline')} onRetry={onRetry} hint={words.hint.studio.timeline.empty} switcher={viewerHead} footer={missingNotice || undefined} />
    {viewerMode === 'cinema' && <button type="button" className="btn color-cinema-exit" onClick={() => setViewerMode('normal')} data-test="exit-cinema">
      {labels.exitCinema}</button>}
  </section>;
  const noTimeline = timelines.length === 0 ? words.hint.studio.cutPage.noTimeline : said.noClip;

  return <div ref={page} className="studio-body color-page" data-viewer-mode={viewerMode} data-merged={merged || undefined}
    data-side={side ?? undefined} data-nodes={nodesOpen || undefined} data-nodes-expanded={(nodesOpen && nodesExpanded) || undefined} data-test="color-page">
    <div className="color-bar" role="toolbar" aria-label={labels.toolbar}>
      {sideButton('gallery', labels.gallery, Images)}
      {sideButton('luts', labels.luts, Layers3)}
      {sideButton('pool', words.label.studio.pool, FolderOpen)}
      <span className="color-tool-pair">
        <button type="button" className="color-tool" aria-pressed={clipsOpen} data-filtered={filter !== 'all' || undefined}
          onClick={() => setClipsOpen(open => !open)} data-test="toggle-clips"><GalleryHorizontalEnd size={14} aria-hidden="true" />{labels.clips}</button>
        <select className="field color-filter" aria-label={labels.clipFilter} value={filter} onChange={event => setFilter(event.target.value as ClipFilter)}
          data-test="clip-filter">
          {CLIP_FILTERS.map(each => <option key={each} value={each}>{labels.filter[each]}</option>)}
        </select>
      </span>
      <button type="button" className="color-tool" aria-pressed={miniOpen} onClick={() => setMiniOpen(open => !open)} data-test="toggle-mini">
        <Film size={14} aria-hidden="true" />{labels.timeline}</button>
      <button type="button" className="color-tool" aria-pressed={nodesOpen} onClick={() => setNodesOpen(open => !open)} data-test="toggle-nodes">
        <Workflow size={14} aria-hidden="true" />{labels.nodes}</button>
    </div>
    {side !== null && <section className="color-side" aria-label={side === 'gallery' ? labels.gallery : side === 'luts' ? labels.lutBrowser : undefined}
      data-panel={side} data-test="color-side" {...focusOf('side')}>
      {side === 'pool' ? pool : <>
        <h2 className="overline">{side === 'gallery' ? labels.gallery : labels.lutBrowser}</h2>
        <p className="caption color-empty">{side === 'gallery' ? said.gallery : said.luts}</p>
      </>}
    </section>}
    {viewerMode === 'cinema' ? createPortal(<div className="color-cinema" data-studio-portal="">{viewer}</div>, document.body) : viewer}
    {nodesOpen && <section className="color-nodes" aria-label={labels.nodeEditor} data-test="node-editor" {...focusOf('nodes')}>
      <NodeEditor grade={grade} level={shownLevel} levels={levels} onLevel={setLevel} current={currentNode} onCurrent={onCurrentNode}
        hint={noTimeline} target={grades.target} gradeKey={gradeKey} views={grades.views} focused={area === 'nodes'} editing={grades.editing} />
      <button type="button" className="icon-btn color-expand" aria-pressed={nodesExpanded} aria-label={labels.nodeEditor} title={labels.nodeEditor}
        onClick={() => setNodesExpanded(open => !open)} data-test="expand-nodes"><Expand size={13} /></button>
    </section>}
    {clipsOpen && <div className="color-thumbs-area" {...focusOf('thumbnails')} onContextMenu={grades.onThumbnailMenu}>
      <ThumbnailTimeline clips={listed} media={mediaById} rate={selected?.rate ?? { num: 25, den: 1 }} cache={thumbnails}
        current={current?.clip.id ?? null} selected={chosenClips} onPick={pick} pictures={pictures} onPicture={onPicture}
        hint={selected === null ? noTimeline : said.noClips} />
    </div>}
    {miniOpen && selected !== null && <div className="color-mini-area" {...focusOf('mini')}>
      <MiniTimeline timeline={selected} numbers={numbers} frame={frame ?? 0} current={current?.clip.id ?? null} skipped={skipped}
        onClip={clipPressed} onSeek={timelineMonitor.seek} onEnable={enable} onSkip={skip} />
    </div>}
    <div className="color-palettes-area" {...focusOf('palettes')}>
      <Palettes merged={merged} left={left} center={center} last={last} right={right} edited={edited} scopes={scopes.docked}
        onLeft={(each) => { setLeft(each); setLast('left'); }} onCenter={(each) => { setCenter(each); setLast('center'); }} onRight={setRight} />
    </div>
    {grades.menu}
    {scopes.floating}
  </div>;
}
