import { keyOf, type Key } from '../../app/keys';
import { editingAllowed } from '../keys';
import { useFairlightText } from './words';
import { useTrackGroups } from './TrackGroups';
import { useCallback, useMemo, useRef, useImperativeHandle, useState, type ComponentProps, type SetStateAction } from 'react';
import { Timeline, type TimelineHandle, type TimelineViewport, type TimelineScrolling } from '../timeline/Timeline';
import type { TrackClipDrawing } from '../timeline/TrackRow';
import { Thumbnail, Thumbnails } from '../media/Thumbnails';
import { ChannelLanes } from './ChannelLanes';
import { AutomationLane } from './AutomationLanes';
import { callEngine } from '../engine';
import type { Change } from '../timeline/commands';
import type { FairlightView, FairlightViewUpdate } from './viewState';
export function FairlightTimeline({ view, update, ...props }: ComponentProps<typeof Timeline> & {
  view: FairlightView; update: FairlightViewUpdate;
}) {
  const t = useFairlightText(), groups = useTrackGroups();
  const updateViewport = useCallback((change: SetStateAction<TimelineViewport>) => update(old => ({
    timelineViewport: typeof change === 'function' ? change(old.timelineViewport) : change,
  })), [update]);
  const activeRange=view.automation&&props.timeline?.audio?.record?.active_range;
  const [problem,setProblem]=useState<string|null>(null);
  const commands=useRef(new Map<string,(key:Key)=>boolean>()),active=useRef<string|null>(null);
  const register=(key:string,command:(key:Key)=>boolean)=>{commands.current.set(key,command);return()=>{if(commands.current.get(key)===command)commands.current.delete(key);};};
  const automationKey=(key:Key)=>{
    if(!view.automation)return false;
    const chosen=active.current&&view.lanePaths[active.current]?active.current:(props.selectedTracks??[]).map(id=>`track:${id}`).find(id=>view.lanePaths[id])??Object.keys(view.lanePaths).find(id=>view.lanePaths[id]);
    return chosen?commands.current.get(chosen)?.(key)??false:false;
  };
  const handle = useRef<TimelineHandle>(null);
  useImperativeHandle(props.ref, () => ({ keyDown: key => automationKey(key) || (handle.current?.keyDown(key) ?? false), perform: action => handle.current?.perform(action) ?? false,
    canPerform: action => handle.current?.canPerform(action) ?? false, clearTransition: () => handle.current?.clearTransition(), marks: () => handle.current?.marks() ?? {in:null,out:null}, clipboardItems: clip => handle.current?.clipboardItems(clip) ?? [] }));
  const edit = (ops:Change[]) => { void callEngine(props.bridge,'transaction.apply',{revision:props.revision,ops},{expectedSession:props.session}).catch(error=>setProblem(String(error))); };
  const choose = (key:string,path:string) => update({lanePaths:{...view.lanePaths,[key]:path},lastLanePaths:path?{...view.lastLanePaths,[key]:path}:view.lastLanePaths});
  const toggle = () => {
    const keys=props.selectedTracks?.length?props.selectedTracks:props.timeline?.tracks.filter(t=>t.kind==='audio').map(t=>t.id)??[];
    update({lanePaths:{...view.lanePaths,...Object.fromEntries(keys.map(id=>{const key=`track:${id}`;return [key,view.lanePaths[key]?'':view.lastLanePaths[key]??'fader_db'];}))}});
  };
  const thumbnails = useMemo(() => new Thumbnails(props.bridge), [props.bridge, props.session]);
  const hidden = useMemo(() => [...view.hidden, ...(props.timeline?.tracks.filter(track => track.kind === 'subtitle' || (!view.video && track.kind === 'video')).map(track => track.id) ?? [])], [props.timeline, view.hidden, view.video]);
  const draw = useCallback((clip: TrackClipDrawing) => clip.track.kind === 'audio' ? <ChannelLanes {...clip} gain={view.waveform} />
    : clip.media && <Thumbnail media={clip.media} cache={thumbnails} />, [view.waveform, thumbnails]);
  return <section className="fairlight-timeline" data-view={view.automation ? 'automation' : 'clips'} onKeyDownCapture={event=>{
    if(event.target instanceof Element && event.target.closest('.automation-decoration'))return;
    const key=keyOf(event);if(editingAllowed(event.target,key)&&automationKey(key)){event.preventDefault();event.stopPropagation();}
  }} onWheelCapture={event => {
    if (event.ctrlKey && event.altKey) { event.preventDefault(); event.stopPropagation(); update({ waveform: Math.max(1, Math.min(16, view.waveform * (event.deltaY < 0 ? 1.25 : .8))) }); }
  }}>
    <div className="fairlight-timeline-options"><label><input type="checkbox" checked={view.video} onChange={event => update({ video: event.target.checked })} />{t('Video tracks')}</label>
      <label><input type="checkbox" checked={view.gainLine} onChange={event => update({ gainLine: event.target.checked })} />{t('Show Clip Gain Line')}</label>
      <label>{t('Waveform height')}<input aria-label={t('Waveform height')} type="range" min={1} max={16} step={.25} value={view.waveform} onChange={event => update({ waveform: Number(event.target.value) })} /></label>
      <button type="button" onClick={() => update({ waveform: 1 })}>{t('Reset waveform height')}</button>
      <label>{t('Timeline Scrolling')}<select aria-label={t('Timeline Scrolling')} value={view.scrolling} onChange={event => update({ scrolling: event.target.value as TimelineScrolling })}>
        <option value="fixed">{t('Fixed Playhead')}</option><option value="page">{t('Page Scrolling')}</option><option value="none">{t('No Scrolling')}</option>
      </select></label>
      <button type="button" aria-pressed={view.automation} disabled={!view.automation} onClick={toggle}>{t('Automation view (F4)')}</button>
    </div>
    {problem&&<p role="alert">{problem}</p>}
    <Timeline {...props} ref={handle} trackMenuItems={id => groups && props.timeline?.tracks.some(track => track.id === id && track.kind === 'audio') ? [{ id: 'create-group', label: t('Create Group'), run: () => groups.open(undefined, groups.selected.includes(id) ? [...groups.selected] : [id]) }] : []} viewport={{ value: view.timelineViewport, update: updateViewport }} scrolling={view.scrolling}
      automationRange={activeRange&&activeRange.on&&props.timeline?{from:activeRange.start.ticks/activeRange.start.scale*props.timeline.rate.num/props.timeline.rate.den,to:activeRange.end.ticks/activeRange.end.scale*props.timeline.rate.num/props.timeline.rate.den}:null}
      gainLineDisabled={id => Boolean(view.automation && view.lanePaths[`track:${id}`])}
      trackDecoration={(track, g) => view.automation && track.kind === 'audio' ? <AutomationLane
        register={register}
        onActive={() => { active.current = `track:${track.id}`; props.onTrackSelect?.(track.id); }}
        {...g} bridge={props.bridge} session={props.session} revision={props.revision}
        timeline={props.timeline!} of={{ track: track.id }} lanes={track.lanes}
        locked={track.locked} arm={track.automation_arm} name={track.name} frame={props.frame}
        marks={() => handle.current?.marks() ?? { in: null, out: null }}
        path={view.lanePaths[`track:${track.id}`] ?? ''} pencil={view.pencil}
        onPath={path => choose(`track:${track.id}`, path)} onEdit={edit} /> : null}
      afterTracks={g => view.automation && props.timeline?.buses?.filter(bus => !view.hidden.includes(`bus:${bus.id}`)).map(bus =>
        <div key={bus.id} className="timeline-track automation-bus" style={{ height: 128 }}><span>{bus.name}</span><AutomationLane
          register={register} onActive={() => { active.current = `bus:${bus.id}`; }} {...g}
          height={128} bridge={props.bridge} session={props.session} revision={props.revision}
          timeline={props.timeline!} of={{ bus: bus.id }} lanes={bus.lanes} arm={bus.automation_arm}
          name={bus.name} frame={props.frame} marks={() => handle.current?.marks() ?? { in: null, out: null }}
          path={view.lanePaths[`bus:${bus.id}`] ?? ''} pencil={view.pencil}
          onPath={path => choose(`bus:${bus.id}`, path)} onEdit={edit} /></div>)}
      hiddenTracks={hidden} drawClip={draw} gainLine={view.gainLine} trackHeight={128}
      audioOnlyDrop={!view.video} label={t('Fairlight timeline')} />
  </section>;
}
