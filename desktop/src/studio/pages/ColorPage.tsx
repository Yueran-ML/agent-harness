import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Expand, Film, FolderOpen, GalleryHorizontalEnd, Images, Layers3, Maximize2, Minimize2, Workflow } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, EngineError, keptColor, type ClipTrack, type ColorSettings, type Group, type Media, type TimelineTransportState, type Timeline as TimelineModel } from '../engine';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import { Thumbnails } from '../media/Thumbnails';
import { rateOf, timecode } from '../timecode';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { useTimelineLoad } from './useTimelineLoad';
import { LutBrowser, type RevealedLut } from '../color/LutBrowser';
import { setNodeLut } from '../color/luts';
import { useFavoriteLuts, useLutLibrary } from '../color/useLuts';
import { ViewerOptions } from '../color/ViewerOptions';
import type { NodeLuts } from '../color/NodeEditor';
import {
  CLIP_FILTERS, currentClip, gradeOf, keptBy, levelsOf, numberedClips, shownClips, sourceFrameAt, stepClip,
  type ClipFilter, type GradeLevel,
} from '../color/clips';
import { MiniTimeline } from '../color/MiniTimeline';
import { NodeEditor } from '../color/NodeEditor';
import { useColorPageGrades } from '../color/useColorPageGrades';
import { useShotMatch } from '../color/useShotMatch';
import { Palettes, type CenterPalette, type LeftPalette, type RightPanel } from '../color/Palettes';
import { useColorScopes } from '../color/Scopes';
import { ThumbnailTimeline } from '../color/ThumbnailTimeline';
import { placeOf, type EditedNode } from '../color/useNodeEdit';
import { keyClockOf } from '../color/keyFrames';
import { makeDefaultNode } from '../color/nodeEdits';
import { refusalText } from '../inspector/refusal';
import { useColorGallery } from '../color/Gallery';
import { appendNodeParams, appendNodeTargets, KEYFRAMES_USING } from '../color/stills';
import { WindowPalette, useWindows } from '../color/WindowPalette';
import { isComplete } from '../color/graph';
import { useViewerInput, VIEWER_ZOOM_CHOICES } from '../color/useViewerInput';
import type { ViewerTool } from '../engine';
import { useComparison, type Comparison } from '../color/useComparison';
import { ComparisonBar, comparisonMenuItems, splitMenu } from '../color/ComparisonBar';
import { CoveringMenu } from '../color/CoveringMenu';
import type { Menu } from '../color/ColorMenu';
import { PickerContext, usePickerHub } from '../color/pickers';
import { QualifierPalette } from '../color/Qualifier';
import { KeyPalette } from '../color/KeyPalette';
import { useViewerModes, type HighlightMode } from '../color/useViewerModes';
import { BypassButton } from '../BypassColor';
import { useGenerateLut } from '../color/generateLut';
import { useGradedThumbnails } from '../color/gradedThumbnails';
import '../color/color.css';

/** Narrower than this, the left and center palettes are merged into one group (Reference Manual 20, p. 2976). */
export const MERGE_PALETTES_PX = 1280;

export type { MissingLut } from '../engine';
/** The state of the timeline monitor. */
type TimelineState = TimelineTransportState;

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
 * viewer larger. The LUT browser (T18) puts LUTs on nodes; the Gallery (T26) keeps stills and
 * memories and applies their grades; the viewer wipes the frame against a still or a clip, or shows
 * a split screen (T27); the node editor shows the grade, and edits it from T16.
 */
export function ColorPage({
  bridge, native, session, revision, pool, media, timelines, groups = NO_GROUPS, selected, onSelect, timelineMonitor, noticeOf, onRetry, onProblem, color,
  nextId, project = '', language = 'en',
}: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'> & Partial<Pick<EngineBridge, 'importStills' | 'exportStills' | 'exportLut'>>;
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
  /** The colour management of the project, whose Color Viewer LUT the viewer's menu sets (T18). */
  color?: ColorSettings;
  /** The project's `next_id`: a grade without nodes is shown with the node 01 that the engine would add to it (T3d). */
  nextId?: number;
  /** The folder of the project, by which the memories of the Gallery are kept (T26). */
  project?: string;
  /** The language that the dialogs of the shell (Import and Export of stills) are titled in. */
  language?: string;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const said = words.hint.studio.colorPage;
  const page = useRef<HTMLDivElement>(null);
  const viewerElement = useRef<HTMLElement>(null);
  const [viewerTool, setViewerTool] = useState<ViewerTool>('none');
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
  const [revealed, setRevealed] = useState<RevealedLut | null>(null);
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
  // The grade shown, as the edits name it; undo and redo, = and -, whole grades copied and pasted, and the menu of a thumbnail.
  // A grade without nodes is shown with DaVinci's default node 01 (T3d), which the first change to it makes.
  const visibleClips = useMemo(() => visible.map(each => each.clip), [visible]);
  const shotMatch = useShotMatch({ bridge, session, revision, timeline: selected, clips: visibleClips, chosen: chosenClips,
    current: current?.clip.id ?? null, nodes, color });
  // Wipe Timeline Clip, in the menu of a thumbnail other than the current one's (p. 3002).
  const comparisonRef = useRef<Comparison | null>(null);
  const currentId = useRef<string | null>(null);
  currentId.current = current?.clip.id ?? null;
  // Generate LUT ▸ (p. 3413) and Update All Thumbnails (p. 3026), and the thumbnails as their clips are graded.
  const generateLut = useGenerateLut({ bridge, language, timeline: selected, media: mediaById, current: current?.clip.id ?? null, frame,
    words: { generateLut: labels.generateLut, sizes: labels.lutSize } });
  const graded = useGradedThumbnails({ bridge, session, timeline: selected, groups, media: mediaById, color, native });
  const thumbnailItems = useCallback((clip: string) => [
    ...(clip === currentId.current ? [] : [{ id: 'wipe-timeline-clip', label: labels.galleryPanel.wipeTimelineClip, run: () => comparisonRef.current?.wipeTimelineClip(clip) }]),
    ...shotMatch.items(clip),
    ...generateLut.items(clip),
    ...(graded.pictures ? [{ id: 'update-thumbnails', label: labels.updateThumbnails, run: graded.updateAll }] : []),
  ], [labels, shotMatch.items, generateLut.items, graded.pictures, graded.updateAll]);
  const grades = useColorPageGrades({
    bridge, revision, session, area, level: shownLevel, timeline: selected, clips: visibleClips, current: current?.clip ?? null, chosen: chosenClips,
    grade, nextId, thumbnailItems,
  });
  const shownGrade = grades.shown;
  const currentNode = shownGrade === null ? null
    : shownGrade.nodes.some(node => node.id === nodes[gradeKey]) ? nodes[gradeKey] : shownGrade.nodes[0]?.id ?? null;
  const onCurrentNode = useCallback((node: string) => setNodes(old => ({ ...old, [gradeKey]: node })), [gradeKey]);
  // The node that the palettes edit: the current node, where it is, with its grade and the playhead as its keys count it (T17):
  // the frame of the clip's media whose picture is shown (T3c, `keyClockOf`).
  const clockMedia = current?.clip.source.kind === 'media' ? mediaById.get(current.clip.source.media) : undefined;
  const clock = current && selected ? keyClockOf(current.clip, selected.rate, clockMedia) : undefined;
  const clockInputs = JSON.stringify([selected?.rate, current?.clip.start, current?.clip.length, current?.clip.source,
    current?.clip.speed, current?.clip.reverse, clockMedia?.facts.video?.rate, clockMedia?.facts.video?.still, clockMedia?.facts.duration]);
  const placed = placeOf(shownLevel, selected?.id ?? null, current?.clip ?? null, frame ?? 0, clock);
  const editedNode = shownGrade?.nodes.find(node => node.id === currentNode) ?? null;
  const defaultNode = grades.defaultNode;
  // A refusal of the palettes' edit (a locked node) is said where the node editor says one, in the page's words.
  const paletteFailed = useCallback((failure: unknown) => {
    if (failure instanceof EngineError) grades.editing.setProblem(refusalText(failure, now.current.words.hint.studio));
    else failed(failure);
  }, [grades.editing.setProblem, failed]);
  const edited = useMemo<EditedNode | null>(() => (placed === null || editedNode === null ? null
    : {
      bridge, session, revision, target: placed.target, node: editedNode, frame: placed.frame, onFailure: paletteFailed,
      clip: current?.clip.id ?? null, clock: clockInputs, timelineFrame: frame ?? 0, playing: !!timelineState?.playing && !timelineMonitor.held,
      before: defaultNode !== null && editedNode.id === defaultNode.id ? [makeDefaultNode(defaultNode.target)] : undefined,
    }),
  [bridge, session, revision, JSON.stringify(placed), editedNode, paletteFailed, defaultNode, current?.clip.id, clockInputs, frame, timelineState?.playing, timelineMonitor.held]);
  // The pickers of colour of the palettes on the viewer (T17, T21), and its Highlight modes and Bypass Color (T21, T13d).
  const pickers = usePickerHub(viewerTool, setViewerTool, edited, paletteFailed);
  const modes = useViewerModes({ bridge, session, loaded: timelineState !== null && timelineMonitor.loaded, state: timelineState,
    grade: grades.target, node: currentNode, onFailure: paletteFailed });
  // The LUTs of nodes (T18): Apply LUT to Current Node and drags from the browser, the node menu's LUT ▸, and Reveal Selected LUT.
  const lutLibrary = useLutLibrary(bridge).library;
  const favoriteLuts = useFavoriteLuts();
  const revealLut = useCallback((name: string) => { setSide('luts'); setRevealed(old => ({ name, asked: (old?.asked ?? 0) + 1 })); }, []);
  const nodeLuts = useMemo<NodeLuts>(() => ({ session, library: lutLibrary, favorites: favoriteLuts, onReveal: revealLut }),
    [session, lutLibrary, favoriteLuts, revealLut]);
  const applyLut = (name: string) => { if (grades.target !== null) grades.editing.run(setNodeLut(grades.target, editedNode ?? undefined, name)); };

  // The Gallery (T26) and the comparisons of the viewer (T27).
  const [wiped, setWiped] = useState<string | null>(null);
  const gallery = useColorGallery({
    bridge, session, project, language, revision, level: shownLevel, timeline: selected, clips: visibleClips, chosenClips, current: current?.clip ?? null,
    groups, wipedStill: wiped, onWipe: still => comparisonRef.current?.wipeStill(still), onRefused: grades.editing.setProblem, onFailed: failed,
  });
  const [viewerMenu, setViewerMenu] = useState<Menu | null>(null);
  const closeViewerMenu = useCallback(() => setViewerMenu(null), []);
  const galleryNow = useRef(gallery);
  galleryNow.current = gallery;

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

  // The viewer's right-click menu (p. 2999 and §1.2): Grab Still, Wipe On/Off, Split Screen ▸, and Apply Grade ▸ of the still wiped.
  const openViewerMenu = useCallback((x: number, y: number) => {
    const comparison = comparisonRef.current;
    const compare = now.current.words.label.studio.colorPage.compare;
    const galleryWords = now.current.words.label.studio.colorPage.galleryPanel;
    if (comparison === null) return;
    const wipedNow = comparison.wipedStill;
    setViewerMenu({ x, y, label: galleryWords.viewerMenu, items: [
      { id: 'grab-still', label: galleryWords.grabStill, keys: 'Ctrl+Alt+G', run: () => galleryNow.current.grab() },
      { id: 'wipe-on-off', label: compare.wipeOnOff, keys: 'Ctrl+W', checked: comparison.wiping, run: comparison.toggleWipe },
      splitMenu(comparison, compare),
      ...(wipedNow === null ? [] : [{ id: 'apply-wiped', label: galleryWords.applyGrade, items: KEYFRAMES_USING.map(each => ({
        id: `apply-wiped-${each}`, label: galleryWords.keyframes[each], run: () => galleryNow.current.apply(wipedNow, 'apply', each),
      })) }]),
    ] });
  }, []);
  // The right button on the picture while the view takes the pointer: the point of the view's window, in pixels of the screen.
  const onPointerMenu = useCallback((at: { x: number; y: number }) => {
    const box = viewerElement.current?.querySelector('[data-test="viewer-box"]')?.getBoundingClientRect();
    const ratio = typeof devicePixelRatio === 'number' && devicePixelRatio > 0 ? devicePixelRatio : 1;
    openViewerMenu((box?.left ?? 0) + at.x / ratio, (box?.top ?? 0) + at.y / ratio);
  }, [openViewerMenu]);
  const comparison = useComparison({
    bridge, session, native, loaded: timelineState !== null && timelineMonitor.loaded, timeline: selected, clips: visibleClips, current: current?.clip ?? null,
    frame: frame ?? 0, playing: timelineState?.playing ?? false, comparedAt: timelineState?.compared, chosen: chosenClips, stills: gallery.chosenStills,
    currentStill: gallery.currentStill,
    // A tile of Selected Clips: the playhead goes to the first frame of its clip, which is current, and the clips chosen stay chosen.
    onGo: (clip) => { const entry = numbered.find(each => each.clip.id === clip); if (entry) { setChosen(clip); timelineMonitor.seek(entry.clip.start, false); } },
    onProblem: (failure) => { if (failure instanceof EngineError) grades.editing.setProblem(refusalText(failure, now.current.words.hint.studio)); else failed(failure); },
  });
  const windows = useWindows({ bridge, session, revision, timeline: selected?.id ?? null, target: grades.target, clip: current?.clip.id ?? null,
    node: editedNode, realNode: grade?.nodes.find(each => each.id === currentNode) ?? null, editing: grades.editing, tool: viewerTool,
    complete: grade !== null && isComplete(grade),
    frame: placed?.frame ?? 0, clock: shownLevel === 'timeline' ? 'timeline' : clockMedia?.facts.video && !clockMedia.facts.video.still ? 'source' : 'clip' });
  const viewerInput = useViewerInput({ bridge, session, native, loaded: timelineState !== null && timelineMonitor.loaded,
    timeline: selected?.id ?? null, tool: viewerTool, selection: windows.selection, intention: windows.intention,
    creating: windows.pending, comparison, viewer: viewerElement, onMenu: onPointerMenu, onWindowState: windows.onState, onDeleted: windows.onDeleted,
    onIdleEscape: windows.leaveDraft, picker: pickers.active });
  comparisonRef.current = comparison;
  useEffect(() => { setWiped(comparison.wipedStill); }, [comparison.wipedStill]);
  // Append Node to Selected Clips (Color menu, p. 3297), from the menu of the node: the current node, after the grades of the other clips chosen.
  const appendTo = appendNodeTargets(selected, visibleClips, chosenClips, current?.clip ?? null);
  // The playhead when it is run (No Keyframes keeps the node as it is there), read then: the item is not made again each frame played.
  const playhead = useRef(frame);
  playhead.current = frame;
  const appendNode = useMemo(() => ({
    label: labels.galleryPanel.appendToSelected, can: appendTo.length > 0 && grades.target !== null,
    run: (node: string) => {
      if (grades.target === null || appendTo.length === 0) return;
      callEngine(bridge, 'grade.append_node', appendNodeParams(grades.target, node, appendTo, gallery.settings, revision, playhead.current ?? 0)).catch(paletteFailed);
    },
  }), [labels, JSON.stringify(appendTo), grades.target, bridge, gallery.settings, revision, paletteFailed]);

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

  // The keys of the Gallery and of comparisons (§9.1–§9.3), by where the key lies, as Ctrl+Alt is AltGr on some layouts:
  // Ctrl+Alt+G grabs a still; Ctrl+W wipes, Alt+W inverts; Ctrl+Alt+B and N go to the still before and after; Ctrl+Alt+Left
  // and Right step the clip of a timeline wipe; Ctrl+Alt+W splits the screen; Alt+1…8 saves a memory and Ctrl+1…8 loads it.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (key.meta || key.shift || !editingAllowed(event.target, key)) return;
      const comparison = comparisonRef.current;
      const shown = galleryNow.current;
      if (comparison === null) return;
      const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
      const digit = /^Digit([1-8])$/.exec(key.code ?? '')?.[1] ?? (/^[1-8]$/.test(key.key) ? key.key : null);
      const take = (run: () => void) => { event.preventDefault(); run(); };
      if (key.ctrl && key.alt) {
        if (key.key === 'ArrowLeft' || key.key === 'ArrowRight') { take(() => comparison.stepReference(key.key === 'ArrowLeft' ? -1 : 1)); return; }
        if (key.repeat) return;
        if (letter === 'g') take(shown.grab);
        else if (letter === 'w') take(comparison.toggleSplit);
        else if (letter === 'b' || letter === 'n') {
          take(() => { const still = shown.stepStill(letter === 'b' ? -1 : 1); if (still !== null) comparison.showStill(still); });
        }
        return;
      }
      if (key.repeat) return;
      if (key.ctrl && letter === 'w') take(comparison.toggleWipe);
      else if (key.alt && letter === 'w') take(comparison.invert);
      else if (key.alt && digit !== null) take(() => shown.storeMemory(Number(digit) - 1));
      else if (key.ctrl && digit !== null) take(() => shown.recallMemory(Number(digit) - 1));
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
    <select className="field color-viewer-zoom" aria-label={labels.viewerZoom} data-test="viewer-zoom"
      value={viewerInput.viewport.mode === 'fit' ? 'fit' : String(viewerInput.viewport.scale)}
      disabled={!native || !timelineMonitor.loaded}
      onChange={event => { viewerInput.setZoom(event.target.value === 'fit' ? 'fit' : Number(event.target.value)); event.currentTarget.blur(); viewerElement.current?.focus({ preventScroll: true }); }}>
      <option value="fit">{labels.viewerFit}</option>
      {viewerInput.viewport.mode === 'scale' && !VIEWER_ZOOM_CHOICES.includes(viewerInput.viewport.scale)
        && <option value={String(viewerInput.viewport.scale)}>{Number((viewerInput.viewport.scale * 100).toPrecision(7))}%</option>}
      {VIEWER_ZOOM_CHOICES.map(scale => <option key={scale} value={String(scale)}>{scale * 100}%</option>)}
    </select>
    <output className="color-head-timecode" aria-label={labels.timecode} data-test="head-timecode">{headValue}</output>
    <select className="field color-head-kind" aria-label={labels.timecode} value={headTimecode} onChange={event => setHeadTimecode(event.target.value as HeadTimecode)}
      data-test="head-timecode-kind">
      {(['source', 'record', 'sourceFrame', 'recordFrame'] as const).map(each => <option key={each} value={each}>{labels.timecodes[each]}</option>)}
    </select>
    <select className="field color-osc" aria-label={labels.windowControls.osc} value={viewerTool}
      onChange={event => setViewerTool(event.target.value as ViewerTool)} data-test="viewer-osc">
      {(['none', 'qualifier', 'window', 'image_wipe'] as const).map(tool => <option key={tool} value={tool}>{labels.windowControls[tool]}</option>)}
    </select>
    {/* Bypass Color and Fusion (p. 2987): this studio has no Fusion, so it is the one switch; Shift+D. */}
    <BypassButton className="color-tool" bypass={modes.bypass} disabled={!timelineMonitor.loaded} label={labels.bypass} onToggle={modes.toggleBypass} />
    <ViewerOptions bridge={bridge} color={keptColor({ color })} run={grades.editing.run} comparisons={[
      { id: 'highlight', label: labels.highlight.menu, items: (['off', 'highlight', 'bw'] as const).map((mode: HighlightMode) => ({
        id: `highlight-${mode}`, label: labels.highlight[mode], keys: mode === 'highlight' ? 'Shift+H' : mode === 'bw' ? 'Alt+Shift+H' : undefined,
        checked: modes.highlight === mode, run: () => modes.setHighlight(mode),
      })) },
      ...comparisonMenuItems(comparison, labels.compare)]} />
    <button type="button" className="icon-btn" aria-pressed={viewerMode === 'enhanced'} aria-label={labels.enhanced}
      title={`${labels.enhanced} (Alt+F)`}
      onClick={event => setViewerMode(old => (event.altKey ? (old === 'full' ? 'normal' : 'full') : old === 'enhanced' ? 'normal' : 'enhanced'))}
      data-test="enhanced-viewer">{viewerMode === 'normal' ? <Maximize2 size={14} /> : <Minimize2 size={14} />}</button>
  </div>;
  // Where a LUT of the timeline cannot be read, the viewer says so at its lower right (p. 3413): the one by its name, more as more.
  // The engine writes the words over the picture of its view (T13d); a page without one says them beneath it.
  const missingNotice = missing.length > 0 && !native && <p className="color-missing-luts" role="status"
    title={missing.map(each => `${each.name}: ${each.message}`).join('\n')} data-test="missing-luts">
    {missing.length === 1 ? labels.missingLut(missing[0].name) : labels.missingLuts}
  </p>;
  const viewer = <section ref={viewerElement} tabIndex={-1} className="color-viewer" aria-label={labels.viewer} data-test="color-viewer" data-split={comparison.splitting || undefined}
    {...focusOf('viewer')}
    onContextMenu={(event) => {
      if (!(event.target instanceof Element) || !event.target.closest('[data-test="viewer-box"]')) return;
      event.preventDefault();
      openViewerMenu(event.clientX, event.clientY);
    }}>
    <MonitorPane controls={timelineMonitor} bridge={bridge} native={native} active hidden={nodesOpen && nodesExpanded && viewerMode === 'normal'}
      notice={noticeOf('timeline')} onRetry={onRetry} hint={words.hint.studio.timeline.empty}
      switcher={<div className="color-viewer-heads">{viewerHead}<ComparisonBar comparison={comparison}>
        {/* Highlight (pp. 2986–2987, 3170): Shift+H, and Alt+Shift+H for Highlight B/W. */}
        <button type="button" className="color-tool" aria-pressed={modes.highlight !== 'off'} title={`${labels.highlight.highlight} (Shift+H)`}
          onClick={() => modes.setHighlight(modes.highlight === 'off' ? 'highlight' : 'off')} data-test="highlight">{labels.highlight.highlight}</button>
        {modes.highlight !== 'off' && <button type="button" className="color-tool" aria-pressed={modes.highlight === 'bw'} title={`${labels.highlight.bw} (Alt+Shift+H)`}
          onClick={() => modes.setHighlight(modes.highlight === 'bw' ? 'highlight' : 'bw')} data-test="highlight-bw">{labels.highlight.bw}</button>}
      </ComparisonBar></div>}
      footer={missingNotice || shotMatch.notice || generateLut.notice ? <>{missingNotice}{shotMatch.notice && <p role="status" aria-busy={shotMatch.busy}
        className="color-missing-luts" data-test="shot-match-notice">{shotMatch.notice}</p>}
        {generateLut.notice && <p role="status" className="color-missing-luts" data-test="lut-notice" onClick={generateLut.dismiss}>
          {generateLut.notice.kind === 'done' ? said.lutGenerated(generateLut.notice.path, generateLut.notice.ignored) : said.lutFailed(generateLut.notice.message)}</p>}</>
        : undefined} />
    {viewerMode === 'cinema' && <button type="button" className="btn color-cinema-exit" onClick={() => setViewerMode('normal')} data-test="exit-cinema">
      {labels.exitCinema}</button>}
  </section>;
  const noTimeline = timelines.length === 0 ? words.hint.studio.cutPage.noTimeline : said.noClip;

  return <div ref={page} className="studio-body color-page" data-viewer-mode={viewerMode} data-merged={merged || undefined}
    data-split={comparison.splitting || undefined}
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
      {side === 'pool' ? pool : side === 'luts' ? <LutBrowser bridge={bridge} session={session} onApply={applyLut}
        canApply={grades.target !== null && editedNode?.kind === 'corrector'} missing={missing} timeline={selected} groups={groups} numbers={numbers}
        revealed={revealed} /> : gallery.panel}
    </section>}
    {viewerMode === 'cinema' ? createPortal(<div className="color-cinema" data-studio-portal="">{viewer}</div>, document.body) : viewer}
    {nodesOpen && <section className="color-nodes" aria-label={labels.nodeEditor} data-test="node-editor" {...focusOf('nodes')}>
      <NodeEditor grade={grade} defaultNode={defaultNode?.id ?? null} level={shownLevel} levels={levels} onLevel={setLevel} current={currentNode} onCurrent={onCurrentNode}
        hint={noTimeline} target={grades.target} gradeKey={gradeKey} views={grades.views} focused={area === 'nodes'} editing={grades.editing}
        luts={nodeLuts} appendNode={appendNode} />
      <button type="button" className="icon-btn color-expand" aria-pressed={nodesExpanded} aria-label={labels.nodeEditor} title={labels.nodeEditor}
        onClick={() => setNodesExpanded(open => !open)} data-test="expand-nodes"><Expand size={13} /></button>
    </section>}
    {clipsOpen && <div className="color-thumbs-area" {...focusOf('thumbnails')} onContextMenu={grades.onThumbnailMenu}>
      <ThumbnailTimeline clips={listed} media={mediaById} rate={selected?.rate ?? { num: 25, den: 1 }} cache={thumbnails}
        current={current?.clip.id ?? null} selected={chosenClips} onPick={pick} pictures={pictures} onPicture={onPicture}
        hint={selected === null ? noTimeline : said.noClips} reference={comparison.referenceClip} graded={graded.pictures} />
    </div>}
    {miniOpen && selected !== null && <div className="color-mini-area" {...focusOf('mini')}>
      <MiniTimeline timeline={selected} numbers={numbers} frame={frame ?? 0} current={current?.clip.id ?? null} skipped={skipped}
        onClip={clipPressed} onSeek={timelineMonitor.seek} onEnable={enable} onSkip={skip} />
    </div>}
    <div className="color-palettes-area" {...focusOf('palettes')}>
      <PickerContext.Provider value={pickers.hub}>
      <Palettes merged={merged} left={left} center={center} last={last} right={right} edited={edited} scopes={scopes.docked}
        windows={edit => <WindowPalette edit={edit} controls={windows} capturing={viewerInput.capturing} busy={viewerInput.busy} status={viewerInput.status} notice={viewerInput.notice} />}
        bodies={{ qualifier: edit => <QualifierPalette key={edit.interaction} edit={edit} />, key: edit => <KeyPalette key={edit.interaction} edit={edit} edited={edited} /> }}
        onLeft={(each) => { setLeft(each); setLast('left'); }} onCenter={(each) => { setCenter(each); setLast('center'); if (each === 'window') setViewerTool('window'); if (each === 'qualifier') setViewerTool('qualifier'); }} onRight={setRight} />
      </PickerContext.Provider>
    </div>
    {grades.menu}
    {viewerMenu && <CoveringMenu menu={viewerMenu} onClose={closeViewerMenu} />}
    {scopes.floating}
  </div>;
}
