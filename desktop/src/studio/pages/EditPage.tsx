import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { PanelLeft, Plus, SlidersHorizontal, Sparkles, Upload } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Media, type Monitor, type Timeline as TimelineModel } from '../engine';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import { Timeline, type TimelineHandle } from '../timeline/Timeline';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { NewTimeline } from './NewTimeline';
import { useTimelineLoad } from './useTimelineLoad';
import { Inspector } from '../inspector/Inspector';
import { SpeedDialog } from '../inspector/SpeedDialog';
import { Library } from '../effects/Library';
import type { Marks } from '../timeline/snap';

/**
 * How wide the Edit page has to be for the two monitors side by side. Narrower (a small window, or
 * the conversation beside the studio), it has one, switched between the source and the timeline,
 * as DaVinci Resolve's single viewer mode.
 */
export const TWO_MONITORS_PX = 1280;

/**
 * The Edit page, laid out as DaVinci Resolve's: the media pool at the top left (it can be put away),
 * the source monitor beside the timeline monitor at the top, the inspector at the top right (it can
 * be put away), and the timeline beneath.
 *
 * The effects library shares the place of the pool (both may be open, one above the other), as the
 * Effects button beside the Media Pool button in DaVinci Resolve. The inspector sets what is chosen
 * on the timeline; R, or its button, opens the dialog of speed.
 *
 * Each monitor has its transport and its view of its own. The keys of the transport, and the marks
 * of I and O, go to the monitor that was pressed last, as they go to the viewer clicked last in
 * DaVinci Resolve; the timeline, pressed, makes the timeline monitor that one. The keys of editing
 * (F9 to F12 and the others) place what the source monitor has, between its marks, at the playhead
 * of the timeline monitor or at the marks of the timeline. Where the page is narrow, one monitor is
 * shown, and the one it is switched to is the one the keys go to; the other keeps its place.
 *
 * Quick Export, at the right of the bar beside the inspector, opens the export of the timeline
 * shown, with its In and Out points. Ctrl+M is left unbound: in DaVinci Resolve it adds a marker and
 * opens its dialog (Reference Manual 20, p. 869), which the studio has not yet.
 */
export function EditPage({
  bridge, native, session, revision, pool, media, timelines, selected, onSelect, source, sourceMedia, timelineMonitor,
  focus, onFocus, noticeOf, onRetry, onProblem, onExport,
}: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'>;
  native: boolean;
  session: number;
  revision: number;
  /** The media pool, as the Media page has it. */
  pool: ReactNode;
  media: readonly Media[];
  timelines: readonly TimelineModel[];
  /** The timeline that the page shows and the timeline monitor plays. */
  selected: TimelineModel | null;
  onSelect: (timeline: string) => void;
  source: MonitorControls;
  /** The media that the source monitor has loaded. */
  sourceMedia: Media | null;
  timelineMonitor: MonitorControls;
  focus: Monitor;
  onFocus: (monitor: Monitor) => void;
  noticeOf: (monitor: Monitor) => ViewerNotice | null;
  onRetry: () => void;
  onProblem: (said: string, failure: unknown) => void;
  /** Open the export of the timeline shown, with its In and Out points. */
  onExport?: (marks: Marks | null) => void;
}) {
  const words = useWords();
  const labels = words.label.studio;
  const page = useRef<HTMLDivElement>(null);
  const timeline = useRef<TimelineHandle>(null);
  const [poolOpen, setPoolOpen] = useState(true);
  const [effectsOpen, setEffectsOpen] = useState(false);
  const [chosenTransition, setChosenTransition] = useState<string | null>(null);
  const [speedClips, setSpeedClips] = useState<string[] | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [narrow, setNarrow] = useState(false);
  const [creating, setCreating] = useState(false);
  const [chosenClips, setChosenClips] = useState<string[]>([]);
  const [chosenCue, setChosenCue] = useState<string | null>(null);
  // A subtitle added or double-clicked is to be written in: the inspector opens on it, as DaVinci Resolve's does.
  const onCue = useCallback((cue: string | null, open: boolean) => { setChosenCue(cue); if (cue !== null && open) setInspectorOpen(true); }, []);
  const now = useRef({ onProblem, words, onExport, selected });
  now.current = { onProblem, words, onExport, selected };
  const exportNow = useCallback(() => {
    const { onExport: open, selected: shown } = now.current;
    if (open && shown !== null) open(timeline.current?.marks() ?? null);
  }, []);

  // How wide the page is, whenever it may have changed. A page that is not laid out (0 wide) keeps how it was.
  useLayoutEffect(() => {
    const element = page.current;
    if (!element) return undefined;
    const measure = () => {
      const width = element.getBoundingClientRect().width;
      if (width > 0) setNarrow(width < TWO_MONITORS_PX);
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, []);

  // The timeline monitor plays the timeline shown: loaded once as it is chosen, not again at each edit of it.
  const playing = useTimelineLoad(bridge, selected, timelineMonitor, session, failure => now.current.onProblem(now.current.words.hint.studio.failed, failure));

  // The keys of the transport go to the monitor that has the focus; those of editing to the timeline.
  const focused = focus === 'source' ? source : timelineMonitor;
  useStudioKeys(focused.loaded, focused.context, focused.run);
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      // The timeline takes its keys itself where it has the keyboard; a list of the pool, those it uses.
      if (event.target instanceof Element && event.target.closest('.timeline')) return;
      if (!editingAllowed(event.target, key)) return;
      if (timeline.current?.keyDown(key)) event.preventDefault();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [exportNow]);

  const sourceState = source.state?.monitor === 'source' ? source.state : null;
  const sourceFrame = sourceState !== null && sourceMedia !== null && sourceState.media === sourceMedia.id ? sourceState.frame ?? 0 : null;
  const sourceContext = useMemo(() => (sourceMedia === null || sourceFrame === null ? null : { media: sourceMedia, frame: sourceFrame, rate: source.rate }),
    [sourceMedia, sourceFrame, source.rate]);
  const timelineState = timelineMonitor.state?.monitor === 'timeline' && selected !== null && playing === selected.id ? timelineMonitor.state : null;

  const create = useCallback(async (made: TimelineModel): Promise<string | null> => {
    try {
      const applied = await callEngine(bridge, 'transaction.apply', { ops: [{ op: 'add_timeline', timeline: made }] });
      const added = applied.names[made.id] ?? applied.done.find(op => op.op === 'add_timeline')?.timeline.id;
      if (added) onSelect(added);
      onFocus('timeline');
      setCreating(false);
      return null;
    } catch (failure) {
      return failure instanceof Error ? failure.message : String(failure);
    }
  }, [bridge, onSelect, onFocus]);

  const monitorWords = labels.monitor;
  const switcher = narrow ? <div className="monitor-switch" role="group" aria-label={monitorWords.choose}>
    {(['source', 'timeline'] as const).map(each => <button key={each} type="button" aria-pressed={focus === each} onClick={() => onFocus(each)}
      data-test={`show-${each}`}>{monitorWords[each]}</button>)}
  </div> : undefined;
  const pane = (controls: MonitorControls, name: string | undefined, hint: string) => <MonitorPane controls={controls} bridge={bridge} native={native}
    active notice={noticeOf(controls.monitor)} onRetry={onRetry} hidden={narrow && focus !== controls.monitor} focused={focus === controls.monitor}
    onFocus={onFocus} name={name} hint={hint} switcher={switcher} />;

  return <div ref={page} className="studio-body edit-page" data-narrow={narrow || undefined} data-pool={poolOpen || undefined} data-inspector={inspectorOpen || undefined}
    data-test="edit-page">
    <div className="edit-bar">
      <button type="button" className="icon-btn" aria-label={labels.pool} title={labels.pool} aria-pressed={poolOpen} onClick={() => setPoolOpen(open => !open)}
        data-test="toggle-pool"><PanelLeft size={15} /></button>
      <button type="button" className="icon-btn" aria-label={words.label.studio.effects.title} title={words.label.studio.effects.title} aria-pressed={effectsOpen}
        onClick={() => setEffectsOpen(open => !open)} data-test="toggle-effects"><Sparkles size={15} /></button>
      <select className="field edit-timelines" aria-label={labels.timeline.title} value={selected?.id ?? ''} disabled={timelines.length === 0}
        onChange={event => {
          if (!event.target.value) return;
          onSelect(event.target.value); onFocus('timeline');
          // The list keeps the keyboard no longer: the keys of the transport and of editing go to the timeline.
          event.currentTarget.blur();
          page.current?.querySelector<HTMLElement>('.timeline')?.focus();
        }} data-test="timeline-picker">
        {timelines.length === 0 && <option value="">{labels.editPage.noTimeline}</option>}
        {timelines.map(each => <option key={each.id} value={each.id}>{each.name}</option>)}
      </select>
      <button type="button" className="btn btn-quiet" onClick={() => setCreating(true)} data-test="new-timeline">
        <Plus size={14} aria-hidden="true" />{labels.editPage.newTimeline}
      </button>
      <span className="edit-bar-gap" />
      {onExport && <button type="button" className="icon-btn" aria-label={labels.render.quickExport} title={labels.render.quickExport}
        disabled={selected === null} onClick={exportNow} data-test="quick-export"><Upload size={15} /></button>}
      <button type="button" className="icon-btn" aria-label={labels.editPage.inspector} title={labels.editPage.inspector} aria-pressed={inspectorOpen}
        onClick={() => setInspectorOpen(open => !open)} data-test="toggle-inspector"><SlidersHorizontal size={15} /></button>
    </div>
    {(poolOpen || effectsOpen) && <div className="edit-left">
      {poolOpen && pool}
      {effectsOpen && <Library session={session} />}
    </div>}
    <div className="edit-monitors">
      {pane(source, sourceMedia?.name, words.hint.studio.loadSource)}
      {pane(timelineMonitor, selected?.name, words.hint.studio.timeline.empty)}
    </div>
    {inspectorOpen && <aside className="edit-inspector" aria-label={labels.editPage.inspector} data-test="inspector-slot">
      <h2 className="overline">{labels.editPage.inspector}</h2>
      <Inspector bridge={bridge} session={session} revision={revision} timeline={selected} media={media} clips={chosenClips} transition={chosenTransition}
        cue={chosenCue} onSpeed={setSpeedClips} />
    </aside>}
    <div className="edit-timeline" onPointerDownCapture={() => onFocus('timeline')} onFocusCapture={() => onFocus('timeline')}>
      <Timeline
        ref={timeline}
        timeline={selected}
        media={media}
        bridge={bridge}
        session={session}
        revision={revision}
        frame={timelineState?.frame ?? 0}
        playing={timelineState?.playing ?? false}
        onSeek={timelineMonitor.seek}
        source={sourceContext}
        focus={focus}
        onSelection={setChosenClips}
        onTransition={setChosenTransition}
        onCue={onCue}
        onSpeed={setSpeedClips}
      />
    </div>
    {speedClips !== null && selected !== null && <SpeedDialog clips={speedClips} timeline={selected} bridge={bridge} session={session} revision={revision}
      onClose={() => setSpeedClips(null)} />}
    {creating && <NewTimeline media={media} names={timelines.map(each => each.name)} onCreate={create} onClose={() => setCreating(false)} />}
  </div>;
}
