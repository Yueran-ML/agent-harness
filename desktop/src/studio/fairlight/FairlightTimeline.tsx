import { keyOf, type Key } from '../../app/keys';
import { editingAllowed } from '../keys';
import { useFailureText, useFairlightText } from './words';
import { useTrackGroups } from './TrackGroups';
import { useCallback, useMemo, useRef, useImperativeHandle, useState, type ComponentProps, type SetStateAction } from 'react';
import { RULER_HEIGHT } from '../timeline/geometry';
import { Timeline, type TimelineHandle, type TimelineViewport, type TimelineScrolling } from '../timeline/Timeline';
import type { TrackClipDrawing } from '../timeline/TrackRow';
import type { Track } from '../engine';
import { Thumbnail, Thumbnails } from '../media/Thumbnails';
import { ChannelLanes } from './ChannelLanes';
import { AutomationLane } from './AutomationLanes';
import { callEngine } from '../engine';
import type { Change } from '../timeline/commands';
import type { FairlightView, FairlightViewUpdate } from './viewState';
import type { MeterSource } from './audioReadings';
import { nextClipEdge, nextFade, smartZoom, JUMP_TARGETS } from './navigation';
import type { JumpTarget } from './viewState';
import { FAIRLIGHT_TRACK_HEIGHT, HeaderDetail, headerMenuItems, useTrackChoice } from './trackHeaders';
type LaneGeometry = { scale: number; from: number; to: number; height: number; snapping: boolean };
export function FairlightTimeline({ view, update, meters, onTracksSelect, ...props }: ComponentProps<typeof Timeline> & {
  view: FairlightView; update: FairlightViewUpdate;
  /** The page's meter readings, which each track header's meter reads for itself. */
  meters?: MeterSource;
  /** The tracks chosen by Ctrl+click, Shift+click, a sweep of the headers or Ctrl+Alt+Up/Down. */
  onTracksSelect?: (ids: string[]) => void;
}) {
  const t = useFairlightText(), groups = useTrackGroups();
  const failure = useFailureText();
  const updateViewport = useCallback((change: SetStateAction<TimelineViewport>) => update(old => ({
    timelineViewport: typeof change === 'function' ? change(old.timelineViewport) : change,
  })), [update]);
  const activeRange=view.automation&&props.timeline?.audio?.record?.active_range;
  const [problem,setProblem]=useState<string|null>(null);
  const commands=useRef(new Map<string,(key:Key)=>boolean>()),active=useRef<string|null>(null);
  const register = useCallback((key: string, command: (key: Key) => boolean) => {
    commands.current.set(key, command);
    return () => { if (commands.current.get(key) === command) commands.current.delete(key); };
  }, []);
  const automationKey=(key:Key)=>{
    if(!view.automation)return false;
    const chosen=active.current&&view.lanePaths[active.current]?active.current:(props.selectedTracks??[]).map(id=>`track:${id}`).find(id=>view.lanePaths[id])??Object.keys(view.lanePaths).find(id=>view.lanePaths[id]);
    return chosen?commands.current.get(chosen)?.(key)??false:false;
  };
  const choice = useTrackChoice({ selected: props.selectedTracks ?? [], onTracks: onTracksSelect, onTrack: props.onTrackSelect });
  const headerDetail = useCallback((track: Track) => <HeaderDetail track={track} meters={meters} />, [meters]);
  const lockedHeights = useMemo(() => Object.keys(view.heightLocks), [view.heightLocks]);
  const section = useRef<HTMLElement>(null);
  const plain = (key: Key) => !key.ctrl && !key.alt && !key.shift && !key.meta;
  const ctrlAlt = (key: Key) => (key.ctrl || key.meta) && key.alt && !key.shift;
  /**
   * The keys of the Fairlight timeline itself (Reference Manual 20, pp. 3671, 3677, 3748): Up and
   * Down to a fade where Jump to is Fade, Ctrl+Alt+Left/Right to a clip's In or Out on the tracks
   * chosen, Ctrl+Alt+Up/Down to move the choice of tracks, and Ctrl+Alt+E for Smart Zoom.
   */
  const viewKey = (key: Key) => {
    const timeline = props.timeline;
    if (!timeline || !section.current || key.composing) return false;
    const chosen = props.selectedTracks ?? [];
    if (view.jumpTo === 'fade' && plain(key) && (key.key === 'ArrowUp' || key.key === 'ArrowDown')) {
      const to = nextFade(timeline, props.frame, key.key === 'ArrowUp' ? -1 : 1);
      if (to !== null) props.onSeek(to, false);
      return true;
    }
    if (!ctrlAlt(key) || key.repeat && !key.key.startsWith('Arrow')) return false;
    if (key.key === 'ArrowLeft' || key.key === 'ArrowRight') {
      const to = nextClipEdge(timeline, chosen, props.frame, key.key === 'ArrowLeft' ? -1 : 1);
      if (to !== null) props.onSeek(to, false);
      return chosen.length > 0;
    }
    if (key.key === 'ArrowUp' || key.key === 'ArrowDown') return choice.move(section.current, key.key === 'ArrowUp' ? -1 : 1);
    if (key.key.toLowerCase() === 'e') {
      if (view.smartZoom) {
        const back = view.smartZoom;
        update(old => ({ smartZoom: null, timelineViewport: { ...old.timelineViewport, ...back } }));
        return true;
      }
      const scroller = section.current.querySelector<HTMLElement>('.timeline-scroll');
      const order = [...section.current.querySelectorAll<HTMLElement>('.timeline-track[data-track]')].map(row => row.dataset.track!);
      const zoomed = smartZoom(view.timelineViewport, order, chosen, lockedHeights, (scroller?.clientHeight ?? 0) - RULER_HEIGHT, FAIRLIGHT_TRACK_HEIGHT);
      if (!zoomed) return true;
      update(old => ({
        smartZoom: { heights: old.timelineViewport.heights, top: old.timelineViewport.top },
        timelineViewport: { ...old.timelineViewport, ...zoomed },
      }));
      return true;
    }
    return false;
  };
  const pageKey = (key: Key) => automationKey(key) || viewKey(key);
  const handle = useRef<TimelineHandle>(null);
  useImperativeHandle(props.ref, () => ({ keyDown: key => pageKey(key) || (handle.current?.keyDown(key) ?? false), perform: action => handle.current?.perform(action) ?? false,
    canPerform: action => handle.current?.canPerform(action) ?? false, clearTransition: () => handle.current?.clearTransition(), marks: () => handle.current?.marks() ?? {in:null,out:null}, clipboardItems: clip => handle.current?.clipboardItems(clip) ?? [] }));
  const edit = (ops:Change[]) => { void callEngine(props.bridge,'transaction.apply',{revision:props.revision,ops},{expectedSession:props.session}).catch(error=>setProblem(failure(error))); };
  const choose = (key:string,path:string) => update({lanePaths:{...view.lanePaths,[key]:path},lastLanePaths:path?{...view.lastLanePaths,[key]:path}:view.lastLanePaths});
  const toggle = () => {
    const keys=props.selectedTracks?.length?props.selectedTracks:props.timeline?.tracks.filter(t=>t.kind==='audio').map(t=>t.id)??[];
    update({lanePaths:{...view.lanePaths,...Object.fromEntries(keys.map(id=>{const key=`track:${id}`;return [key,view.lanePaths[key]?'':view.lastLanePaths[key]??'fader_db'];}))}});
  };
  // The lanes are made again for a change of view, edit or geometry, never for the playhead:
  // what they call back is read through `latest`, and they read the playhead from AudioPlayhead.
  const latest = useRef({ choose, edit, onTrackSelect: props.onTrackSelect });
  latest.current = { choose, edit, onTrackSelect: props.onTrackSelect };
  const marks = useCallback(() => handle.current?.marks() ?? { in: null, out: null }, []);
  const { bridge, session, revision, timeline } = props;
  const { automation, lanePaths, pencil, hidden: hiddenOwners } = view;
  const trackDecoration = useCallback((track: Track, g: LaneGeometry) => automation && track.kind === 'audio' && timeline
  ? <AutomationLane
    register={register}
    onActive={() => { active.current = `track:${track.id}`; latest.current.onTrackSelect?.(track.id); }}
    {...g} bridge={bridge} session={session} revision={revision}
    timeline={timeline} of={{ track: track.id }} lanes={track.lanes}
    locked={track.locked} arm={track.automation_arm} name={track.name} marks={marks}
    path={lanePaths[`track:${track.id}`] ?? ''} pencil={pencil}
    onPath={path => latest.current.choose(`track:${track.id}`, path)}
    onEdit={ops => latest.current.edit(ops)} /> : null,
  [automation, timeline, bridge, session, revision, lanePaths, pencil, register, marks]);
  const afterTracks = useCallback((g: Omit<LaneGeometry, 'height'>) => automation && timeline
    ? timeline.buses?.filter(bus => !hiddenOwners.includes(`bus:${bus.id}`)).map(bus =>
      <div key={bus.id} className="timeline-track automation-bus" style={{ height: 128 }}><span>{bus.name}</span>
        <AutomationLane
        register={register} onActive={() => { active.current = `bus:${bus.id}`; }} {...g}
        height={128} bridge={bridge} session={session} revision={revision}
        timeline={timeline} of={{ bus: bus.id }} lanes={bus.lanes} arm={bus.automation_arm}
        name={bus.name} marks={marks}
        path={lanePaths[`bus:${bus.id}`] ?? ''} pencil={pencil}
        onPath={path => latest.current.choose(`bus:${bus.id}`, path)} onEdit={ops => latest.current.edit(ops)} /></div>)
    : null, [automation, timeline, bridge, session, revision, lanePaths, pencil, hiddenOwners, register, marks]);
  const thumbnails = useMemo(() => new Thumbnails(props.bridge), [props.bridge, props.session]);
  const hidden = useMemo(() => [...view.hidden, ...(props.timeline?.tracks.filter(track => track.kind === 'subtitle' || (!view.video && track.kind === 'video')).map(track => track.id) ?? [])], [props.timeline, view.hidden, view.video]);
  const draw = useCallback((clip: TrackClipDrawing) => clip.track.kind === 'audio'
    ? <ChannelLanes {...clip} gain={view.waveform * (view.trackWaveform[clip.track.id] ?? 1)} />
    : clip.media && <Thumbnail media={clip.media} cache={thumbnails} />, [view.waveform, view.trackWaveform, thumbnails]);
  return <section ref={section} className="fairlight-timeline" data-view={view.automation ? 'automation' : 'clips'}
    data-full-waveforms={view.fullWaveforms || undefined} data-waveform-borders={view.waveformBorders || undefined}
    onPointerDownCapture={choice.pointerDown} onPointerOverCapture={choice.pointerOver} onPointerUpCapture={choice.pointerUp}
    onKeyDownCapture={event=>{
    if(event.target instanceof Element && event.target.closest('.automation-decoration'))return;
    const key=keyOf(event);if(editingAllowed(event.target,key)&&pageKey(key)){event.preventDefault();event.stopPropagation();}
  }} onWheelCapture={event => {
    if (event.ctrlKey && event.altKey) { event.preventDefault(); event.stopPropagation(); update({ waveform: Math.max(1, Math.min(16, view.waveform * (event.deltaY < 0 ? 1.25 : .8))) }); }
  }}>
    <div className="fairlight-timeline-options"><label><input type="checkbox" checked={view.video} onChange={event => update({ video: event.target.checked })} />{t('Video tracks')}</label>
      <label><input type="checkbox" checked={view.gainLine} onChange={event => update({ gainLine: event.target.checked })} />{t('Show Clip Gain Line')}</label>
      <label><input type="checkbox" checked={view.fullWaveforms} onChange={event => update({ fullWaveforms: event.target.checked })} />{t('Full Waveforms')}</label>
      <label><input type="checkbox" checked={view.waveformBorders} onChange={event => update({ waveformBorders: event.target.checked })} />{t('Waveform Borders')}</label>
      <label>{t('Jump to')}<select aria-label={t('Jump to')} value={view.jumpTo} onChange={event => update({ jumpTo: event.target.value as JumpTarget })}>
        {JUMP_TARGETS.map(target => <option key={target} value={target}>{t(target === 'clip' ? 'Clip' : 'Fade')}</option>)}
      </select></label>
      <label>{t('Waveform height')}<input aria-label={t('Waveform height')} type="range" min={1} max={16} step={.25} value={view.waveform} onChange={event => update({ waveform: Number(event.target.value) })} /></label>
      <button type="button" onClick={() => update({ waveform: 1 })}>{t('Reset waveform height')}</button>
      <label>{t('Timeline Scrolling')}<select aria-label={t('Timeline Scrolling')} value={view.scrolling} onChange={event => update({ scrolling: event.target.value as TimelineScrolling })}>
        <option value="fixed">{t('Fixed Playhead')}</option><option value="page">{t('Page Scrolling')}</option><option value="none">{t('No Scrolling')}</option>
      </select></label>
      <button type="button" aria-pressed={view.automation} disabled={!view.automation} onClick={toggle}>{t('Automation view (F4)')}</button>
    </div>
    {problem&&<p role="alert">{problem}</p>}
    <Timeline {...props} ref={handle} onTrackSelect={choice.trackSelect} trackMenuItems={id => [
      ...(props.timeline ? headerMenuItems({ id, timeline: props.timeline, selected: props.selectedTracks ?? [], view, update, onEdit: edit, t }) : []),
      ...(groups && props.timeline?.tracks.some(track => track.id === id && track.kind === 'audio')
        ? [{ id: 'create-group', label: t('Create Group'), run: () => groups.open(undefined, groups.selected.includes(id) ? [...groups.selected] : [id]) }] : []),
    ]} viewport={{ value: view.timelineViewport, update: updateViewport }} scrolling={view.scrolling}
      headerDetail={headerDetail} renameOnClick lockedHeights={lockedHeights} groupResize
      automationRange={activeRange&&activeRange.on&&props.timeline?{from:activeRange.start.ticks/activeRange.start.scale*props.timeline.rate.num/props.timeline.rate.den,to:activeRange.end.ticks/activeRange.end.scale*props.timeline.rate.num/props.timeline.rate.den}:null}
      gainLineDisabled={id => Boolean(view.automation && view.lanePaths[`track:${id}`])}
      trackDecoration={trackDecoration} afterTracks={afterTracks}
      hiddenTracks={hidden} drawClip={draw} gainLine={view.gainLine} trackHeight={FAIRLIGHT_TRACK_HEIGHT}
      audioOnlyDrop={!view.video} label={t('Fairlight timeline')} />
  </section>;
}
