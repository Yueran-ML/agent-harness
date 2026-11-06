import { AutomationToolbar } from '../fairlight/AutomationToolbar';
import { AutomationTouchProvider } from '../fairlight/AutomationTouch';
import { ownerKey, type AutomationTarget } from '../fairlight/automation';
import { useFairlightText } from '../fairlight/words';
import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import type { EngineBridge } from '../../bridge';
import { keyOf } from '../../app/keys';
import { Sheet } from '../../app/Sheet';
import { CoveringMenu } from '../color/CoveringMenu';
import { callEngine, type Media, type Op, type Timeline as TimelineModel } from '../engine';
import { editingAllowed, inStudio, useStudioKeys } from '../keys';
import { MonitorPane } from '../MonitorPane';
import type { MonitorControls } from '../useMonitor';
import type { ViewerNotice } from '../Viewer';
import { useTimelineLoad } from './useTimelineLoad';
import type { TimelineHandle } from '../timeline/Timeline';
import { useAudioScrubbing } from '../timeline/scrub';
import { SpeedDialog } from '../inspector/SpeedDialog';
import { useNormalizeAudio } from '../inspector/NormalizeDialog';
import { useClipAttributes } from '../inspector/ClipAttributes';
import { Toolbar } from '../fairlight/Toolbar';
import { Monitoring, STANDARDS, reading, standardLabel } from '../fairlight/Monitoring';
import { FairlightTimeline } from '../fairlight/FairlightTimeline';
import { IndexTracks } from '../fairlight/IndexTracks';
import { Inspector } from '../fairlight/Inspector';
import { EffectsLibrary, AUDIO_EFFECT_DRAG, AUDIO_EFFECTS } from '../fairlight/EffectsLibrary';
import { LevelMatcher, type LevelMatcherHandle } from '../fairlight/LevelMatcher';
import { useAudioReadings } from '../fairlight/audioReadings';
import { useFairlightView } from '../fairlight/viewState';
import { Mixer } from '../fairlight/Mixer';
import { analysisStandardId } from '../audio/mixerModel';
import { mixerOrder, moveMixerItems, useMixerLayout } from '../audio/mixerLayout';
import '../fairlight/fairlight.css';

interface Props {
  bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'viewRect' | 'viewShow'>; native: boolean; session: number; revision: number;
  pool: ReactNode; media: readonly Media[]; timelines: readonly TimelineModel[]; selected: TimelineModel | null;
  onSelect: (id: string) => void; timelineMonitor: MonitorControls; noticeOf: (monitor: 'timeline') => ViewerNotice | null;
  onRetry: () => void; onProblem: (said: string, failure: unknown) => void;
  /** T21 supplies Mixer/ChannelStrip; T22 supplies Groups/routing; T23 supplies automation. */
  mixer?: ReactNode; groups?: ReactNode;
}
/** A new session/timeline owns new async jobs and selection, while its view state survives page switches. */
export function FairlightPage(props: Props) { return <FairlightSession key={`${props.session}/${props.selected?.id ?? ''}`} {...props} />; }
function Separator({ label, vertical = false, direction = 1, value, min, max, onChange }: {
  label: string; vertical?: boolean; direction?: 1 | -1; value: number; min: number; max: number; onChange: (value: number) => void;
}) {
  const drag = useRef<{ at: number; value: number } | null>(null);
  const set = (next: number) => onChange(Math.max(min, Math.min(max, next)));
  return <div className="fairlight-separator" role="separator" tabIndex={0} aria-label={label} aria-orientation={vertical ? 'vertical' : 'horizontal'} aria-valuenow={value} aria-valuemin={min} aria-valuemax={max}
    onPointerDown={event => { if (event.button) return; event.preventDefault(); event.currentTarget.setPointerCapture?.(event.pointerId); drag.current = { at: vertical ? event.clientX : event.clientY, value }; }}
    onPointerMove={event => { if (drag.current) set(drag.current.value + direction * ((vertical ? event.clientX : event.clientY) - drag.current.at)); }}
    onPointerUp={event => { drag.current = null; event.currentTarget.releasePointerCapture?.(event.pointerId); }} onPointerCancel={() => { drag.current = null; }}
    onKeyDown={event => { if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) { event.preventDefault(); set(value + direction * (['ArrowLeft', 'ArrowUp'].includes(event.key) ? -16 : 16)); } }} />;
}
function FairlightSession({ bridge, native, session, revision, pool, media, timelines, selected, onSelect, timelineMonitor, noticeOf, onRetry, onProblem, mixer, groups }: Props) {
  const t = useFairlightText();
  const [view, update, reset] = useFairlightView(bridge, session, selected?.id ?? '');
  const [mixerLayout, setMixerLayout] = useMixerLayout(bridge, session, selected?.id ?? '');
  const timeline = useRef<TimelineHandle>(null);
  const matcher = useRef<LevelMatcherHandle>(null);
  const [clips, setClips] = useState<string[]>([]); const [track, setTrack] = useState<string | null>(null);
  const [transition, setTransition] = useState<string | null>(null);
  const normalize = useNormalizeAudio({ bridge, session, revision, timeline: selected, selection: clips, commands: () => timeline.current });
  const attributes = useClipAttributes({ bridge, session, revision, timeline: selected, selection: clips });
  const [stripSelection, setStripSelection] = useState<string[]>([]);
  const chooseStrips = (keys: string[]) => { timeline.current?.clearTransition(); setStripSelection(keys); setTrack(keys.find(key => key.startsWith('track:'))?.slice(6) ?? null); };
  const chooseTrack = (id: string) => chooseStrips([`track:${id}`]);
  const [speed, setSpeed] = useState<string[] | null>(null);
  const [cinema, setCinema] = useState(false);
  const [context, setContext] = useState<{ clip: string; x: number; y: number } | null>(null);
  const [analysisClips, setAnalysisClips] = useState<string[] | null>(null);
  const [analysisStandard, setAnalysisStandard] = useState('ebu_r128');
  const [problem, setProblem] = useState<string | null>(null);
  const live = useRef(true); useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const now = useRef({ revision, selected, onProblem }); now.current = { revision, selected, onProblem };
  const edit = useCallback(async (ops: Op[] | Record<string, unknown>[]) => {
    if (!ops.length || !live.current) return;
    try { await callEngine(bridge, 'transaction.apply', { revision: now.current.revision, ops }, { expectedSession: session }); }
    catch (error) { if (live.current) setProblem(error instanceof Error ? error.message : String(error)); }
  }, [bridge, session]);
  const playing = useTimelineLoad(bridge, selected, timelineMonitor, session, failure => now.current.onProblem(t('Could not load the Fairlight timeline.'), failure));
  const state = selected && playing === selected.id && timelineMonitor.state?.monitor === 'timeline' ? timelineMonitor.state : null;
  const audio = useAudioReadings(bridge, session, selected?.id ?? null, view.meters || view.mixer);
  useStudioKeys(timelineMonitor.loaded, timelineMonitor.context, timelineMonitor.run);
  useAudioScrubbing({ bridge, session, loaded: Boolean(state && timelineMonitor.loaded), state, onFailure: failure => now.current.onProblem(t('Could not change audio scrubbing.'), failure) });
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event); if (!editingAllowed(event.target, key)) return;
      if (event.key === 'F4' && view.automation && selected) { event.preventDefault();
        const keys=stripSelection.filter(k=>k.startsWith('track:'));const targets=keys.length?keys:selected.tracks.filter(t=>t.kind==='audio').map(t=>`track:${t.id}`);
        update({lanePaths:{...view.lanePaths,...Object.fromEntries(targets.map(key=>[key,view.lanePaths[key]?'':view.lastLanePaths[key]??'fader_db']))}});
      }
      else if (event.key === 'Escape') { setCinema(false); setContext(null); }
      else if ((!key.ctrl && !key.alt && !key.shift && !key.meta && event.key.toLowerCase() === 'p') || (key.ctrl && !key.alt && !key.shift && !key.meta && event.key.toLowerCase() === 'f')) { event.preventDefault(); setCinema(old => !old); }
      else if (!(event.target instanceof Element && event.target.closest('.timeline')) && timeline.current?.keyDown(key)) event.preventDefault();
    };
    window.addEventListener('keydown', down); return () => window.removeEventListener('keydown', down);
  }, [view, update, selected, stripSelection]);
  const playbackQueue = useRef(Promise.resolve());
  useEffect(() => {
    if (!selected || playing !== selected.id) return;
    let live = true;
    playbackQueue.current = playbackQueue.current.then(async()=>{try { await callEngine(bridge,'automation.playback',{timeline:selected.id,on:view.automation},{expectedSession:session}); } catch(e){if(live){setProblem(String(e));update({automation:state?.automation_playback??true});}}});
    return ()=>{live=false;};
  },[bridge,session,selected?.id,playing,view.automation]);
  useEffect(()=>()=>{
    if (!selected) return;
    playbackQueue.current=playbackQueue.current.then(async()=>{
      try { await callEngine(bridge,'automation.record',{timeline:selected.id,on:false},{expectedSession:session}); await callEngine(bridge,'automation.playback',{timeline:selected.id,on:true},{expectedSession:session}); } catch { /* A replaced engine session or unloaded timeline already resets recording and playback. */ }
    });
  },[bridge,session,selected?.id]);
  const showAutomation=(target:AutomationTarget)=>{const key=ownerKey(target.of);update({automation:true,lanePaths:{...view.lanePaths,[key]:target.path},lastLanePaths:{...view.lastLanePaths,[key]:target.path}});};
  const soundClips = clips.filter(id => selected?.tracks.some(track => track.kind === 'audio' && track.clips.some(clip => clip.id === id)));
  const viewer = <MonitorPane bridge={bridge} native={native} active={cinema || (view.viewer && view.meters)} controls={timelineMonitor} notice={noticeOf('timeline')} onRetry={onRetry} />;
  const showLeft = view.pool || view.index || view.effects || view.groups;
  const content = <div className="studio-body fairlight-page" data-test="fairlight-page" style={{ '--fairlight-left': `${showLeft ? view.leftWidth : 0}px`, '--fairlight-right': `${view.inspector ? view.rightWidth : 0}px` } as CSSProperties}>
    <Toolbar view={view} update={update} reset={reset} timelines={timelines} selected={selected?.id ?? null} onSelect={onSelect} />
    {selected && <AutomationToolbar bridge={bridge} session={session} timeline={selected} view={view} update={update} recording={state?.recording??false} selected={stripSelection} frame={state?timelineMonitor.frame??0:0} onEdit={ops=>void edit(ops as Record<string,unknown>[])} onFailure={e=>setProblem(String(e))} onToggle={()=>update({automation:!view.automation})} marks={()=>timeline.current?.marks()??{in:null,out:null}}/>}
    {(problem || audio.problem) && <p role="alert">{problem ?? audio.problem}<button type="button" onClick={() => setProblem(null)}>{t('Dismiss')}</button></p>}
    {view.meters && selected && <div className="fairlight-monitoring-wrap" style={{ height: view.monitoringHeight }}>
      <Monitoring timeline={selected} meters={audio.meters} layout={mixerLayout} onAction={action => void audio.action(action)} onResetPeaks={() => void audio.resetPeaks()}
        onPatch={patch => void edit([{ op: 'set_timeline_audio', timeline: selected.id, patch }])} viewer={view.viewer && !cinema ? viewer : undefined} />
      <Separator label={t('Monitoring height')} value={view.monitoringHeight} min={150} max={500} onChange={monitoringHeight => update({ monitoringHeight })} />
    </div>}
    <main className="fairlight-workspace">
      {showLeft && <aside className="fairlight-left">{view.pool && pool}{view.effects && <EffectsLibrary />}
        {view.index && selected && <IndexTracks automation={view.automation} timeline={selected} hidden={view.hidden} onHidden={hidden => update({ hidden })} onSelect={chooseTrack} layout={mixerLayout} onReorder={(keys, before) => setMixerLayout({ order: moveMixerItems(mixerOrder(selected, mixerLayout), keys, before) })}
          selection={stripSelection.filter(key => key.startsWith('track:')).map(key => key.slice(6))} onSelection={ids => chooseStrips(ids.map(id => `track:${id}`))} onEdit={ops => void edit(ops)} />}
        {view.groups && (groups ?? <section aria-label={t('Groups')}><h2>{t('Groups')}</h2><p>{t('Group controls are not available yet.')}</p></section>)}
      </aside>}
      {showLeft && <Separator label={t('Left panel width')} vertical value={view.leftWidth} min={220} max={700} onChange={leftWidth => update({ leftWidth })} />}
      <div className="fairlight-center" onDragOverCapture={event => { if (event.dataTransfer.types.includes(AUDIO_EFFECT_DRAG)) { event.preventDefault(); event.stopPropagation(); } }}
        onDropCapture={event => {
          const effect = event.dataTransfer.getData(AUDIO_EFFECT_DRAG); if (!AUDIO_EFFECTS.some(([id]) => id === effect) || !selected) return;
          const target = event.target instanceof Element ? event.target : null;
          if (target?.closest('.audio-mixer')) return;
          event.preventDefault(); event.stopPropagation();
          const trackId = target?.closest<HTMLElement>('[data-track]')?.dataset.track; const targetTrack = selected.tracks.find(each => each.id === trackId);
          if (targetTrack?.kind !== 'audio' || targetTrack.locked) return;
          const clip = target?.closest<HTMLElement>('[data-clip]')?.dataset.clip;
          const targets = clip ? (clips.includes(clip) ? clips : [clip]).filter(id => selected.tracks.some(track => track.kind === 'audio' && !track.locked && track.clips.some(c => c.id === id))).map(clip => ({ clip })) : [{ track: trackId }];
          if (!clip && (targetTrack.strip?.inserts.length ?? 0) >= 6) return;
          void edit(targets.map(of => ({ op: 'add_insert', timeline: selected.id, of, insert: { on: true, effect: { kind: effect } } })));
        }}>
        <FairlightTimeline ref={timeline} timeline={selected} bridge={bridge} session={session} revision={revision} media={media}
          frame={state ? timelineMonitor.frame ?? 0 : 0} playing={state?.playing ?? false} onSeek={timelineMonitor.seek}
          onSelection={setClips} onTransition={setTransition} onTrackSelect={chooseTrack} selectedTracks={stripSelection.filter(key => key.startsWith('track:')).map(key => key.slice(6))} onSpeed={setSpeed} view={view} update={update}
          onCopyClips={attributes.copy} onClipContext={(id, x, y) => { if (selected?.tracks.some(track => track.kind === 'audio' && track.clips.some(clip => clip.id === id))) setContext({ clip: id, x, y }); }} />
        {view.mixer && <><Separator label={t('Mixer height')} direction={-1} value={view.mixerHeight} min={180} max={500} onChange={mixerHeight => update({ mixerHeight })} />
          <section className="fairlight-mixer-slot" aria-label={t('Mixer')} style={{ height: view.mixerHeight }}>{mixer ?? (selected && <Mixer bridge={bridge} session={session} revision={revision}
            timeline={selected} frame={state ? timelineMonitor.frame ?? 0 : 0} meters={audio.meters} selected={stripSelection} onSelect={chooseStrips} />)}</section></>}
      </div>
      {view.inspector && selected && <><Separator label={t('Inspector width')} vertical direction={-1} value={view.rightWidth} min={240} max={600} onChange={rightWidth => update({ rightWidth })} />
        <Inspector bridge={bridge} session={session} revision={revision} timeline={selected} media={media} clips={clips} track={track} transition={transition} onEdit={ops => void edit(ops)}
          frame={state ? timelineMonitor.frame ?? 0 : 0} onSeek={timelineMonitor.seek} onSpeed={setSpeed} /></>}
    </main>
    <footer className="fairlight-footer"><button type="button" aria-pressed={view.viewer} onClick={() => update({ viewer: !view.viewer })}>{t('Viewer')}</button>
      {normalize.menu}
      <button type="button" onClick={() => setCinema(old => !old)}>{t('Cinema viewer')}</button>
      {selected && <LevelMatcher ref={matcher} key={`${session}/${selected.id}`} bridge={bridge} session={session} timeline={selected} revision={revision} clips={soundClips} />}
    </footer>
    {context && <CoveringMenu onClose={() => setContext(null)} menu={{ ...context, label: t('Audio clip'), items: [
      ...attributes.items(context.clip),
      normalize.item(context.clip),
      { id: 'analyze-audio', label: t('Analyze Audio Level'), run: () => setAnalysisClips(soundClips.includes(context.clip) ? soundClips : [context.clip]) },
      { id: 'capture-level', label: t('Capture Level Profile'), run: () => matcher.current?.capture(context.clip) },
      { id: 'apply-level', label: t('Apply Level Profile'), disabled: !matcher.current?.captured,
        run: () => matcher.current?.apply(soundClips.includes(context.clip) ? soundClips : [context.clip]) },
      { id: 'close', label: t('Close') },
    ] }} />}
    {normalize.dialog}
    {attributes.dialog}
    {analysisClips && <Sheet title={t('Analyze Audio Level')} onClose={() => { audio.cancel(); setAnalysisClips(null); }}>
      <div className="fairlight-analysis"><select aria-label={t('Analysis standard')} value={analysisStandard} onChange={event => setAnalysisStandard(event.target.value)}>{STANDARDS.map(([id, label]) => <option key={id} value={id}>{standardLabel(label, t)}</option>)}</select>
      <button type="button" disabled={audio.pending} onClick={() => void audio.analyze(analysisClips, analysisStandardId(analysisStandard))}>{t('Analyze')}</button>
      {audio.pending && <button type="button" onClick={audio.cancel}>{t('Cancel analysis')}</button>}
      <button type="button" data-first onClick={() => { audio.cancel(); setAnalysisClips(null); }}>{t('Close analysis')}</button>
      {audio.analysis?.state === 'running' && <p role="status">{audio.analysis.done.toFixed(1)} / {audio.analysis.total.toFixed(1)} s</p>}
      {audio.analysis?.state === 'failed' && <p role="alert">{audio.analysis.message}</p>}
      {audio.analysis?.state === 'cancelled' && <p role="status">{t('Cancelled')}</p>}
      {audio.analysis?.state === 'done' && audio.analysis.method === 'audio.analyze' && <table><caption>{t('Target')}: {reading(audio.analysis.level, 'LUFS')}</caption><tbody>{audio.analysis.results.map((result, index) => <tr key={index} data-verdict={result.verdict ?? undefined}>
        <th>{'clip' in result.target ? result.target.clip : ''}</th><td>I {reading(result.integrated, 'LUFS')}</td><td>LRA {reading(result.range, 'LU')}</td><td>TP {reading(result.true_peak, 'dBTP')}</td>
      </tr>)}</tbody></table>}
      </div>
    </Sheet>}
    {cinema && <div className="fairlight-cinema">{viewer}<button type="button" onClick={() => setCinema(false)}>{t('Exit cinema')}</button></div>}
    {speed && selected && <SpeedDialog bridge={bridge} session={session} revision={revision} timeline={selected} clips={speed} onClose={() => setSpeed(null)} />}
  </div>;
  return selected ? <AutomationTouchProvider bridge={bridge} session={session} revision={revision} timeline={selected} enabled={view.automation} recording={state?.recording??false} playing={state?.playing??false} read={state?.all_read??false} show={showAutomation} onFailure={e=>setProblem(String(e))}>{content}</AutomationTouchProvider> : content;
}
