import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, Spline } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { covers, currentClip, numberedClips, shownClips, stepClip, type ColorClip } from '../color/clips';
import { pageKey } from '../color/nodeEdits';
import { useGradeEditing } from '../color/useGradeEditing';
import { EngineError, type Clip, type Media, type Timeline as TimelineModel } from '../engine';
import { FusionCurves } from '../fusion/FusionCurves';
import { FusionInspector } from '../fusion/FusionInspector';
import { FusionNodeEditor, NO_SELECTION, type FusionSelection } from '../fusion/FusionNodeEditor';
import { shownComposition, type FusionAt } from '../fusion/edits';
import { namesOf } from '../fusion/graph';
import { useFusionOnscreen } from '../fusion/useFusionOnscreen';
import { useFusionView } from '../fusion/useFusionView';
import { refusalText } from '../inspector/refusal';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { useTimelineLoad } from './useTimelineLoad';
import '../fusion/fusion.css';

/** The parts of the page that take the focus: the keys of the node editor are its own while it has it. */
type Area = 'viewer' | 'nodes' | 'inspector';

const NO_SKIPPED: ReadonlySet<string> = new Set();
const NO_CLIPS: readonly string[] = [];

/** Whether the engine refused a change because it changes nothing: a handle let go where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * The one clip of picture among the clips chosen on the Edit page, or null where there is none or
 * several: a clip of picture chosen with its linked sound is one clip.
 */
export function chosenPicture(shown: readonly ColorClip[], chosen: readonly string[]): string | null {
  const pictures = shown.filter(each => chosen.includes(each.clip.id));
  return pictures.length === 1 ? pictures[0].clip.id : null;
}

/**
 * The clip whose composition the page shows. As the Fusion page of DaVinci Resolve takes the topmost
 * visible clip under the playhead (p. 1265), it is the clip of an enabled video track at the
 * playhead, the one on the highest track where several lie there; but a clip that was pinned (the
 * one clip of picture chosen on the Edit page, or one gone to with Previous or Next) is the one
 * shown while the playhead is on it, and also while the playhead has not moved since it was pinned
 * (`pinnedAt`), so that a clip chosen on the Edit page is shown though the playhead stands elsewhere.
 */
export function fusionClip(shown: readonly ColorClip[], frame: number | null, pin: string | null, pinnedAt: number | null): ColorClip | null {
  const pinned = pin === null ? null : shown.find(each => each.clip.id === pin) ?? null;
  if (pinned !== null && (frame === null || frame === pinnedAt || covers(pinned.clip, frame))) return pinned;
  return currentClip(shown, frame, pin);
}

interface Props {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'>;
  native: boolean;
  session: number;
  revision: number;
  media: readonly Media[];
  timelines: readonly TimelineModel[];
  /** The timeline that the page shows and the timeline monitor plays. */
  selected: TimelineModel | null;
  onSelect: (timeline: string) => void;
  timelineMonitor: MonitorControls;
  noticeOf: (monitor: 'timeline') => ViewerNotice | null;
  onRetry: () => void;
  onProblem: (said: string, failure: unknown) => void;
  /** The clips chosen on the Edit page as it was left: one clip of picture among them is the clip that the page opens on. */
  chosen?: readonly string[];
}

/**
 * The Fusion page: the composition of a clip, a graph of nodes, built in the node editor, and the
 * settings of its nodes set in the Inspector, as on the Fusion page of DaVinci Resolve (Reference
 * Manual 20, Ch. 64, pp. 1216–1263).
 *
 * The viewer is the timeline monitor of the other pages, with its transport and its keys, above the
 * node editor; the Inspector is at the right, and the Spline Editor opens beneath the node editor.
 * While the page is shown the monitor shows the composition of the clip alone, as the viewers of
 * Fusion do (`useFusionView`, the Fusion view of the engine): what comes out of its MediaOut at
 * first, or the node that is viewed (the key `1` in the node editor, and View in the menu of a
 * node), which the bar names and whose card has a dot; another clip begins with the output again,
 * and so does a node that is taken away. The page left, the monitor shows the timeline again. Over
 * the picture the node that is chosen has its on-screen controls, where its kind has any: the
 * outline and the handles of a mask, the centre of a Merge or a Transform, and the patterns of a
 * Tracker with their paths, dragged on the picture (`useFusionOnscreen`).
 *
 * The clip is the one at the playhead on the topmost enabled video track, or the one clip of
 * picture chosen on the Edit page (`fusionClip`); the bar names it, and goes to the clip before and
 * after it. A clip chosen on the Edit page that the playhead does not lie on has the playhead taken
 * to its first frame as the page is entered, as Previous and Next take it, so that the viewer has
 * its picture to show; but not while the timeline plays. Where there is none, the page says so. Another timeline, or another engine, begins
 * anew: nothing chosen of the one before is kept. A clip on a locked track is shown, and nothing of
 * its composition is changed: the node editor, the Inspector and the Spline Editor are told so.
 */
export function FusionPage(props: Props) {
  return <FusionSession key={`${props.session}/${props.selected?.id ?? ''}`} {...props} />;
}

function FusionSession({ bridge, native, session, revision, media, timelines, selected, onSelect, timelineMonitor, noticeOf, onRetry, onProblem, chosen = NO_CLIPS }: Props) {
  const words = useWords();
  const labels = words.label.studio.fusionPage;
  const said = words.hint.studio.fusionPage;
  const colorLabels = words.label.studio.colorPage;
  const [area, setArea] = useState<Area>('nodes');
  const [curvesOpen, setCurvesOpen] = useState(false);
  const shown = useMemo(() => shownClips(numberedClips(selected), NO_SKIPPED), [selected]);
  // The clip pinned: at first the one clip of picture chosen on the Edit page, then the one gone to with Previous or Next.
  const [pin, setPin] = useState<string | null>(() => chosenPicture(shown, chosen));
  // Where the playhead stood when the clip was pinned: until it moves, the clip is shown though the playhead is not on it.
  const pinnedAt = useRef<number | null>(null);
  // The nodes chosen in the composition of a clip: another clip begins with none chosen.
  const [picked, setPicked] = useState<{ clip: string; selection: FusionSelection } | null>(null);
  const now = useRef({ onProblem, words });
  now.current = { onProblem, words };
  const failed = useCallback((failure: unknown) => now.current.onProblem(now.current.words.hint.studio.failed, failure), []);

  const playing = useTimelineLoad(bridge, selected, timelineMonitor, session, failed);
  const timelineState = timelineMonitor.state?.monitor === 'timeline' && selected !== null && playing === selected.id ? timelineMonitor.state : null;
  const frame = timelineState === null ? null : timelineMonitor.frame ?? 0;
  if (pin !== null && frame !== null) {
    // The first frame that is known since the clip was pinned is where the playhead stood; once it has moved, it stands there no more.
    if (pinnedAt.current === null) pinnedAt.current = frame;
    else if (frame !== pinnedAt.current) pinnedAt.current = Number.NaN;
  }
  const current = fusionClip(shown, frame, pin, pinnedAt.current);
  const clip: Clip | null = current?.clip ?? null;
  const composition = clip === null ? null : shownComposition(clip);
  // A clip on a locked track is shown and not changed, as on the Edit page: the node editor, the Inspector and the Spline Editor are told.
  const locked = current?.track.locked ?? false;
  const at = useMemo<FusionAt | null>(() => (selected === null || clip === null ? null : { timeline: selected.id, clip: clip.id }), [selected?.id, clip?.id]);

  const editing = useGradeEditing({ bridge, revision, session });
  const selection = useMemo<FusionSelection>(() => {
    if (clip === null || composition === null || picked?.clip !== clip.id) return NO_SELECTION;
    const there = (id: string) => composition.nodes.some(node => node.id === id);
    const others = picked.selection.others.filter(there);
    const chosenNow = picked.selection.current !== null && there(picked.selection.current) ? picked.selection.current : others[0] ?? null;
    return { current: chosenNow, others: others.filter(id => id !== chosenNow) };
  }, [clip?.id, composition, picked]);
  const clipId = clip?.id ?? null;
  const onSelection = useCallback((next: FusionSelection) => { if (clipId !== null) setPicked({ clip: clipId, selection: next }); }, [clipId]);
  // The node whose settings the Inspector shows: the one chosen, where it is the only one.
  const node = composition !== null && selection.current !== null && selection.others.length === 0
    ? composition.nodes.find(each => each.id === selection.current) ?? null : null;
  const names = useMemo(() => (composition === null ? new Map<string, string>() : namesOf(composition, labels.kinds)), [composition, labels.kinds]);

  const seek = timelineMonitor.seek;
  /** Go to a clip: it is the one shown, and the playhead goes to its first frame. */
  const go = useCallback((to: ColorClip | null) => {
    if (to === null) return;
    pinnedAt.current = to.clip.start;
    setPin(to.clip.id);
    seek(to.clip.start, false);
  }, [seek]);
  // A clip pinned that the playhead does not lie on (the one chosen on the Edit page, as the page is entered): the playhead goes to
  // its first frame, as Previous and Next take it there, so that the viewer has the picture of the clip to show. Once for each
  // pin, when the frame of the playhead is first known, and before the view of the clip is asked for; and not while the timeline
  // plays, which is not stopped for it: the clip under the playhead is the one shown then, as soon as the playhead has moved.
  const sought = useRef<string | null>(null);
  const frameKnown = frame !== null;
  const playingNow = timelineState?.playing ?? false;
  useEffect(() => {
    if (pin === null || frame === null || sought.current === pin) return;
    const pinned = shown.find(each => each.clip.id === pin) ?? null;
    if (pinned === null) return;
    sought.current = pin;
    if (playingNow || covers(pinned.clip, frame)) return;
    go(pinned);
  }, [pin, frameKnown, go]);

  // The node that the viewer shows, of the composition of a clip: another clip begins with what comes out of its composition, and
  // a node that is taken away, or the MediaOut, is the output too.
  const [viewing, setViewing] = useState<{ clip: string; node: string } | null>(null);
  const viewingNode = viewing !== null && viewing.clip === clipId ? viewing.node : null;
  const viewed = viewingNode !== null && composition !== null && composition.nodes.some(each => each.id === viewingNode && each.kind !== 'media_out') ? viewingNode : null;
  const onView = useCallback((id: string | null) => setViewing(id === null || clipId === null ? null : { clip: clipId, node: id }), [clipId]);
  const toOutput = useCallback(() => setViewing(null), []);
  // The Fusion view as the monitor tells it: on while the page is shown and has a clip, and off again as the page is left.
  const told = timelineState?.monitor === 'timeline' && typeof timelineState.fusion === 'object' && timelineState.fusion !== null ? timelineState.fusion : null;
  useFusionView({ bridge, session, on: native && timelineState !== null && timelineMonitor.loaded, clip: clipId, node: viewed, told: told?.clip ?? null, onNodeGone: toOutput });
  // The controls of the node that is chosen, over the picture of the composition of this clip, where the monitor shows it.
  const viewerPressed = useCallback(() => setArea('viewer'), []);
  useFusionOnscreen({
    bridge, session, revision, native, timeline: selected, clip, media, composition, node, frame: frame ?? 0, view: told !== null && told.clip === clipId ? told : null, locked,
    onPress: viewerPressed,
    onFailure: (failure) => { if (!isEmpty(failure)) editing.setProblem(refusalText(failure, now.current.words.hint.studio)); },
  });

  // The keys of the transport go to the timeline monitor; Ctrl+Z and Ctrl+Shift+Z undo and redo in the one history of the engine.
  useStudioKeys(timelineMonitor.loaded, timelineMonitor.context, timelineMonitor.run);
  const step = editing.step;
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const command = pageKey(key);
      if (command?.kind !== 'history') return;
      event.preventDefault();
      step(command.method);
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [step]);

  const before = stepClip(shown, current, frame ?? 0, -1);
  const after = stepClip(shown, current, frame ?? 0, 1);

  const clipName = (each: Clip) => (each.source.kind === 'media' ? media.find(item => item.id === (each.source as { media: string }).media)?.name ?? each.source.media
    : each.source.kind === 'title' ? each.source.text || colorLabels.title : words.label.studio.timeline.color);
  // Why there is nothing to show: no timeline, only sound chosen on the Edit page, or no clip of picture at the playhead.
  const soundChosen = chosen.length > 0 && pin === null && selected !== null
    && chosen.every(id => selected.tracks.some(track => track.kind === 'audio' && track.clips.some(each => each.id === id)));
  const empty = timelines.length === 0 || selected === null ? words.hint.studio.cutPage.noTimeline : soundChosen ? said.soundClip : said.noClip;

  const focusOf = (each: Area) => ({
    'data-focused': area === each || undefined,
    onPointerDownCapture: () => setArea(each),
    onFocusCapture: () => setArea(each),
  });
  const closeCurves = useCallback(() => setCurvesOpen(false), []);

  return <div className="studio-body fusion-page" data-curves={curvesOpen || undefined} data-test="fusion-page">
    <div className="fusion-bar" role="toolbar" aria-label={labels.toolbar}>
      <select className="field edit-timelines" aria-label={words.label.studio.timeline.title} value={selected?.id ?? ''} disabled={timelines.length === 0}
        onChange={(event) => { if (event.target.value) { onSelect(event.target.value); event.currentTarget.blur(); } }} data-test="timeline-picker">
        {timelines.length === 0 && <option value="">{words.label.studio.editPage.noTimeline}</option>}
        {timelines.map(each => <option key={each.id} value={each.id}>{each.name}</option>)}
      </select>
      <div className="fusion-clip" role="group" aria-label={labels.clip}>
        <button type="button" className="icon-btn" aria-label={labels.previousClip} title={labels.previousClip} disabled={before === null} onClick={() => go(before)}
          data-test="fusion-previous-clip"><ChevronLeft size={15} /></button>
        <strong className="fusion-clip-name" title={clip ? clipName(clip) : undefined} data-clip={clip?.id} data-test="fusion-clip-name">
          {current ? `${String(current.number).padStart(2, '0')} · ${clipName(current.clip)}` : labels.noClip}</strong>
        <button type="button" className="icon-btn" aria-label={labels.nextClip} title={labels.nextClip} disabled={after === null} onClick={() => go(after)}
          data-test="fusion-next-clip"><ChevronRight size={15} /></button>
      </div>
      {/* What the viewer shows of the composition: the node that is viewed, or what comes out of it. */}
      {clip !== null && <span className="fusion-viewed" data-node={viewed ?? undefined} data-test="fusion-viewed">
        {labels.viewing.replace('{name}', () => (viewed !== null ? names.get(viewed) ?? viewed : labels.output))}</span>}
      <span className="fusion-gap" />
      <button type="button" className="color-tool" aria-pressed={curvesOpen} disabled={clip === null} onClick={() => setCurvesOpen(open => !open)}
        data-test="toggle-spline"><Spline size={14} aria-hidden="true" />{labels.spline}</button>
    </div>
    <section className="fusion-viewer" aria-label={labels.viewer} data-test="fusion-viewer" {...focusOf('viewer')}>
      <MonitorPane controls={timelineMonitor} bridge={bridge} native={native} active notice={noticeOf('timeline')} onRetry={onRetry}
        hint={words.hint.studio.timeline.empty} />
    </section>
    <section className="fusion-nodes" aria-label={colorLabels.nodeEditor} data-test="fusion-node-editor" {...focusOf('nodes')}>
      {selected !== null && clip !== null && composition !== null && at !== null ? <>
        {/* Made anew for another clip, so that nothing dragged, picked or written of the clip before is carried over. */}
        <FusionNodeEditor key={clip.id} bridge={bridge} session={session} revision={revision} clip={clip} composition={composition} at={at} selection={selection}
          onSelection={onSelection} focused={area === 'nodes'} editing={editing} locked={locked} viewed={viewed} onView={onView} />
        {curvesOpen && <FusionCurves key={`${clip.id}/${node?.id ?? ''}`} bridge={bridge} session={session} revision={revision} timeline={selected} clip={clip}
          media={media} composition={composition} node={node} names={names} frame={frame ?? 0} onSeek={seek} onClose={closeCurves} locked={locked} />}
      </> : <p className="caption fusion-empty" data-test="fusion-empty">{empty}</p>}
    </section>
    <aside className="fusion-inspector" aria-label={words.label.studio.editPage.inspector} data-test="fusion-inspector" {...focusOf('inspector')}>
      <h2 className="overline">{words.label.studio.editPage.inspector}</h2>
      {selected !== null && clip !== null
        ? <FusionInspector key={`${clip.id}/${node?.id ?? ''}`} bridge={bridge} session={session} revision={revision} timeline={selected} clip={clip} media={media}
          node={node} name={node ? names.get(node.id) ?? node.id : ''} frame={frame ?? 0} onSeek={seek} locked={locked} />
        : <p className="caption" data-test="fusion-inspector-empty">{empty}</p>}
    </aside>
  </div>;
}
