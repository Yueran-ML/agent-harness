import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { PanelLeft, Upload } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import { shortcut } from '../../app/system';
import type { EngineBridge } from '../../bridge';
import type { Media, Monitor, Timeline as TimelineModel } from '../engine';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import { EditButtons } from '../cut/EditButtons';
import { SourceTape, tapeOf, tapeStep, useTapeGo } from '../cut/SourceTape';
import { nearestCut, type EditAction } from '../timeline/editKeys';
import { Timeline, type TimelineHandle, type TimelineView } from '../timeline/Timeline';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { useTimelineLoad } from './useTimelineLoad';
import { isExportKey } from '../render/renders';
import type { Marks } from '../timeline/snap';

/** Narrower than this, the Cut page has its buttons without their names, and its pool narrower. */
export const CUT_NARROW_PX = 1100;
/** How high a track is in the upper timeline of the Cut page, which shows the whole of it. */
const OVERVIEW_TRACK_PX = 26;

/** What the one viewer of the Cut page shows: the source clip, the source tape of the bin, or the timeline. */
export type ViewerMode = 'source' | 'tape' | 'timeline';

/**
 * The Cut page, laid out as DaVinci Resolve's: the media pool in its thumbnail view at the top left,
 * one viewer at the top right, which shows the source clip, the source tape (the media of the bin
 * shown in the pool, end to end) or the timeline, the buttons of its edits beneath it, and two
 * timelines at the foot. The upper timeline shows all of it, the lower the part around the playhead;
 * both are views of the same timeline with the same playhead, and clips are moved and trimmed in
 * either. Track 1 (the first video track) is magnetic, as on DaVinci Resolve's Cut page
 * (Reference Manual 20, pp. 509, 584–590): a dragged edge of its clips ripples, a clip moved or
 * deleted leaves no gap, and one dropped on an edit point goes in there; on the tracks above, an
 * edge only resizes its clip and a move only moves it. The seam between two clips rolls.
 *
 * The edits put what the viewer has as its source (its marks, its medium of the tape) on the
 * timeline: Smart Insert at the edit point nearest the playhead, which the lower timeline marks.
 * They leave the viewer as it was, so that the source tape is gone through edit after edit.
 * Quick Export, at the right of the bar, and Ctrl+M open the export of the timeline.
 */
export function CutPage({
  bridge, native, session, revision, pool, media, bin, timelines, selected, onSelect, source, sourceMedia, timelineMonitor,
  focus, onFocus, noticeOf, onRetry, onProblem, onExport,
}: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'>;
  native: boolean;
  session: number;
  revision: number;
  /** The media pool, in its thumbnail view. */
  pool: ReactNode;
  media: readonly Media[];
  /** The bin that the pool shows, whose media are the source tape (null for the master bin). */
  bin: string | null;
  timelines: readonly TimelineModel[];
  selected: TimelineModel | null;
  onSelect: (timeline: string) => void;
  source: MonitorControls;
  sourceMedia: Media | null;
  timelineMonitor: MonitorControls;
  /** The monitor that the viewer shows: the source monitor shows the source clip and the tape. */
  focus: Monitor;
  onFocus: (monitor: Monitor) => void;
  noticeOf: (monitor: Monitor) => ViewerNotice | null;
  onRetry: () => void;
  onProblem: (said: string, failure: unknown) => void;
  /** Open the export of the timeline, with the In and Out points of the lower timeline. */
  onExport?: (marks: Marks | null) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.cutPage;
  const page = useRef<HTMLDivElement>(null);
  const detail = useRef<TimelineHandle>(null);
  const [poolOpen, setPoolOpen] = useState(true);
  const [narrow, setNarrow] = useState(false);
  // Whether the source monitor shows the tape, kept while the viewer shows the timeline, to go back to.
  const [tape, setTape] = useState(false);
  const [view, setView] = useState<TimelineView | null>(null);
  const mode: ViewerMode = focus === 'timeline' ? 'timeline' : tape ? 'tape' : 'source';
  const now = useRef({ onProblem, words, onExport, selected });
  now.current = { onProblem, words, onExport, selected };
  // The marks are those of the lower timeline, which takes the keys.
  const exportNow = useCallback(() => {
    const { onExport: open, selected: shown } = now.current;
    if (open && shown !== null) open(detail.current?.marks() ?? null);
  }, []);
  const failed = useCallback((failure: unknown) => now.current.onProblem(now.current.words.hint.studio.failed, failure), []);

  useLayoutEffect(() => {
    const element = page.current;
    if (!element) return undefined;
    const measure = () => {
      const width = element.getBoundingClientRect().width;
      if (width > 0) setNarrow(width < CUT_NARROW_PX);
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, []);

  const playing = useTimelineLoad(bridge, selected, timelineMonitor, session, failed);
  const timelineState = timelineMonitor.state?.monitor === 'timeline' && selected !== null && playing === selected.id ? timelineMonitor.state : null;
  const frame = timelineState?.frame ?? 0;
  const sourceState = source.state?.monitor === 'source' ? source.state : null;
  const sourceFrame = sourceState !== null && sourceMedia !== null && sourceState.media === sourceMedia.id ? sourceState.frame ?? 0 : null;
  const sourceContext = useMemo(() => (sourceMedia === null || sourceFrame === null ? null : { media: sourceMedia, frame: sourceFrame, rate: source.rate }),
    [sourceMedia, sourceFrame, source.rate]);

  // The source tape: the media of the bin shown in the pool, in its order.
  const binMedia = useMemo(() => media.filter(each => each.bin === bin), [media, bin]);
  const reel = useMemo(() => tapeOf(binMedia, timelines), [binMedia, timelines]);
  const go = useTapeGo(bridge, sourceState?.media ?? null, failed);
  const choose = (next: ViewerMode) => {
    if (next === 'timeline') { onFocus('timeline'); return; }
    setTape(next === 'tape'); onFocus('source');
    // The tape begins at the first medium of the bin where the viewer has none of its media.
    if (next === 'tape' && !reel.segments.some(each => each.media.id === sourceState?.media) && reel.segments.length > 0) go(reel.segments[0].media.id, 0, false);
  };
  const perform = useCallback((action: EditAction) => { detail.current?.perform(action); }, []);

  // The keys of the transport go to what the viewer shows; those of editing to the lower timeline.
  const focused = focus === 'source' ? source : timelineMonitor;
  useStudioKeys(focused.loaded, focused.context, focused.run);
  const keys = useRef({ mode, reel, loaded: sourceState?.media ?? null, go });
  keys.current = { mode, reel, loaded: sourceState?.media ?? null, go };
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      // Ctrl+M exports, wherever in the page the keyboard is, the timelines too.
      if (isExportKey(key)) {
        if (editingAllowed(event.target, key)) { event.preventDefault(); exportNow(); }
        return;
      }
      if (event.target instanceof Element && event.target.closest('.timeline')) return;
      if (!editingAllowed(event.target, key)) return;
      // On the source tape the arrows up and down go to the medium before or after, as they go from edit to edit on a timeline.
      const { mode: shown, reel: strip, loaded, go: to } = keys.current;
      if (shown === 'tape' && !key.ctrl && !key.alt && !key.shift && !key.meta && (key.key === 'ArrowUp' || key.key === 'ArrowDown')) {
        event.preventDefault();
        const next = tapeStep(strip, loaded, key.key === 'ArrowUp' ? -1 : 1);
        if (next !== null && !key.repeat) to(next, 0, false);
        return;
      }
      if (detail.current?.keyDown(key)) event.preventDefault();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [exportNow]);

  const switcher = <div className="monitor-switch" role="group" aria-label={labels.viewerMode}>
    {(['source', 'tape', 'timeline'] as const).map(each => <button key={each} type="button" aria-pressed={mode === each} onClick={() => choose(each)}
      data-test={`viewer-${each}`}>{labels[each]}</button>)}
  </div>;
  const pane = (controls: MonitorControls, name: string | undefined, hint: string) => <MonitorPane controls={controls} bridge={bridge} native={native}
    active notice={noticeOf(controls.monitor)} onRetry={onRetry} hidden={focus !== controls.monitor} focused={focus === controls.monitor}
    onFocus={onFocus} name={name} hint={hint} switcher={switcher} />;
  const shown = view && view.scale > 0 ? { from: Math.max(0, view.left / view.scale), to: (view.left + view.width) / view.scale } : null;
  const indicator = selected !== null ? nearestCut(selected, frame) : null;
  const common = {
    timeline: selected, media, bridge, session, revision, frame, playing: timelineState?.playing ?? false, onSeek: timelineMonitor.seek,
    source: sourceContext, focus, cut: true,
  };

  return <div ref={page} className="studio-body cut-page" data-narrow={narrow || undefined} data-pool={poolOpen || undefined} data-mode={mode} data-test="cut-page">
    <div className="cut-bar">
      <button type="button" className="icon-btn" aria-label={words.label.studio.pool} title={words.label.studio.pool} aria-pressed={poolOpen}
        onClick={() => setPoolOpen(open => !open)} data-test="toggle-pool"><PanelLeft size={15} /></button>
      <select className="field edit-timelines" aria-label={words.label.studio.timeline.title} value={selected?.id ?? ''} disabled={timelines.length === 0}
        onChange={event => { if (event.target.value) { onSelect(event.target.value); onFocus('timeline'); event.currentTarget.blur(); } }} data-test="timeline-picker">
        {timelines.length === 0 && <option value="">{words.label.studio.editPage.noTimeline}</option>}
        {timelines.map(each => <option key={each.id} value={each.id}>{each.name}</option>)}
      </select>
      {timelines.length === 0 && <span className="caption" data-test="cut-no-timeline">{words.hint.studio.cutPage.noTimeline}</span>}
      <span className="edit-bar-gap" />
      {onExport && <button type="button" className="icon-btn" aria-label={words.label.studio.render.quickExport}
        title={`${words.label.studio.render.quickExport} (${shortcut('M')})`} disabled={selected === null} onClick={exportNow} data-test="quick-export">
        <Upload size={15} />
      </button>}
    </div>
    {poolOpen && <div className="cut-left">{pool}</div>}
    <div className="cut-viewer">
      {pane(source, sourceMedia?.name, words.hint.studio.loadSource)}
      {pane(timelineMonitor, selected?.name, words.hint.studio.timeline.empty)}
      {mode === 'tape' && <SourceTape tape={reel} loaded={sourceState?.media ?? null} frame={sourceState?.frame ?? null} onGo={go} />}
      <EditButtons onAction={perform} timeline={selected !== null} source={sourceContext !== null} />
    </div>
    <div className="cut-timelines" onPointerDownCapture={() => onFocus('timeline')} onFocusCapture={() => onFocus('timeline')}>
      <Timeline {...common} overview trackHeight={OVERVIEW_TRACK_PX} detail={shown} label={labels.overview} />
      <Timeline {...common} ref={detail} center indicator={indicator} onView={setView} label={labels.detail} />
    </div>
  </div>;
}
