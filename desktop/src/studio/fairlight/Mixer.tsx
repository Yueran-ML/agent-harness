import { useAutomationTouch } from './AutomationTouch';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import type { MeterSource } from './audioReadings';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { faderChanges, faderPlan, mixerStrips, operationTargets, stripChange, switchChanges, type FaderPlan, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { mixerTrackMoves } from './IndexTracks';
import { ChannelStrip } from './ChannelStrip';
import { useFailureText, useFairlightText } from './words';
import './mixer.css';
import { keyOf } from '../../app/keys';
import { writing } from '../keys';
import { editKey } from '../timeline/editKeys';
import { processorChanges, targetOf, type ProcessorModule } from '../audio/processorModel';
import { ProcessorWindow } from './ProcessorWindow';
import '../audio/processors.css';
import { BusFormat } from './BusFormat';
import { BusAssign } from './BusAssign';
import { VcaStrip } from './VcaStrip';
import './routing.css';
import type { AudioAutomationAdapter } from '../audio/automationAdapter';
import { MIXER_DIVIDER, MIXER_SECTIONS, mixerOrder, moveMixerItems, resetBusOrder, useMixerLayout } from '../audio/mixerLayout';
import type { FairlightText } from '../../messages/fairlight';

/** Keep text/local edits here; ordinary transport and global history keep their existing route. */
export function isolateMixerKeys(event: KeyboardEvent<HTMLElement>) {
  const key = keyOf(event), action = editKey(key);
  if (key.composing || writing(event.target) || (action && action.kind !== 'history') || ['[', ']'].includes(key.key)
    || ((key.ctrl || key.meta) && ['c', 'x', 'v'].includes(key.key.toLowerCase()))) event.stopPropagation();
}

export interface MixerProps {
  automation?: AudioAutomationAdapter | null;
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; frame: number; meters: MeterSource;
  selected: readonly string[]; onSelect: (keys: string[]) => void; compact?: boolean;
}
export function Mixer({ bridge, session, revision, timeline, frame, meters, selected, onSelect, compact = false }: MixerProps) {
  const automation=useAutomationTouch();
  const [routing, setRouting] = useState<'format' | 'assign' | null>(null);
  const t = useFairlightText(); const [problem, setProblem] = useState<string | null>(null);
  const failure = useFailureText();
  const [processors, setProcessors] = useState<{ key: string; module: ProcessorModule }[]>([]);
  const box = useRef<{ x:number; y:number; selected:readonly string[] } | null>(null);
  const [boxRect, setBoxRect] = useState<{left:number;top:number;width:number;height:number}|null>(null);
  const [layout, setLayout] = useMixerLayout(bridge, session, timeline.id);
  const order = mixerOrder(timeline, layout), unsorted = mixerStrips(timeline);
  const strips = order.flatMap(key => unsorted.filter(strip => strip.key === key)), plan = useRef<FaderPlan | null>(null);
  const [layoutOpen, setLayoutOpen] = useState(false);
  const sectionLabels = ['Input', 'Track FX', 'Order', 'Effects', 'Effects In', 'Dynamics', 'EQ', 'Sends', 'Pan', 'Outputs', 'VCA'];
  const reorder = (keys: string[], before: string) => {
    const ops = mixerTrackMoves(timeline, order, keys, before);
    if (ops.length) gesture.commit(ops);
    setLayout({ order: moveMixerItems(order, keys, before) });
  };
  const [preview, setPreview] = useState<Record<string, number>>({});
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${timeline.id}/mixer`, locked: false, onFailure: error => setProblem(failure(error)) });
  const latest = useRef({ timeline, strips, gesture, selected, revision, session }); latest.current = { timeline, strips, gesture, selected, revision, session };
  const anchor = useRef<string | null>(selected[0] ?? null);
  const localSelection = useRef<readonly string[] | null>(null);
  useEffect(() => {
    const local = localSelection.current;
    if (!local || local.length !== selected.length || local.some((key, i) => key !== selected[i])) anchor.current = selected.at(-1) ?? null;
    localSelection.current = null;
  }, [selected]);
  const sweep = useRef<{ field: 'mute' | 'solo' | 'solo_safe'; value: boolean; keys: Set<string>; modifiers: Modifiers } | null>(null);
  const cancel = () => { sweep.current = null; plan.current = null; gesture.cancel(); setPreview({}); };
  const switchOps = (keys: string[], field: 'mute' | 'solo' | 'solo_safe', value: boolean, modifiers: Modifiers): Change[] => {
    const state = latest.current;
    const seeds = keys.flatMap(key => state.strips.filter(s => s.key === key));
    // Preserve the actual sweep order, so the last requested solo/group wins under Exclusive Solo.
    return switchChanges(state.timeline, state.strips, seeds, field, value, Boolean(modifiers.ctrlKey && modifiers.shiftKey));
  };
  useEffect(() => {
    const abandon = () => { sweep.current = null; };
    const finish = () => { const s = sweep.current; abandon(); if (s) latest.current.gesture.commit(switchOps([...s.keys], s.field, s.value, s.modifiers)); };
    window.addEventListener('pointerup', finish); window.addEventListener('pointercancel', abandon); window.addEventListener('blur', abandon);
    return () => { abandon(); window.removeEventListener('pointerup', finish); window.removeEventListener('pointercancel', abandon); window.removeEventListener('blur', abandon); };
  }, [session, timeline.id]);
  useEffect(() => { sweep.current = null; plan.current = null; setPreview({}); setProcessors([]); setRouting(null); }, [session, timeline.id]);
  const start = (strip: MixerStrip, modifiers: Modifiers) => {
    const captured = faderPlan(timeline, strip.key, selected, modifiers, automation?.enabled??true);
    if (!captured.members.length || !gesture.start(captured.restore)) return false;
    plan.current = captured; return true;
  };
  const change = (value: number) => {
    const active = plan.current; if (!active || !gesture.active()) return;
    const delta = value - Math.max(-100, active.source.db);
    const changes = faderChanges(timeline.id, active, delta);
    setPreview(Object.fromEntries(active.members.map(s => [s.key, Math.max(-100, Math.min(10, Math.max(-100, s.db) + delta))])));
    gesture.preview(changes);
  };
  const select = (strip: MixerStrip, modifiers: Modifiers) => {
    let keys: string[];
    if (modifiers.shiftKey && anchor.current) {
      const from = strips.findIndex(s => s.key === anchor.current), to = strips.indexOf(strip);
      keys = from >= 0 ? strips.slice(Math.min(from, to), Math.max(from, to) + 1).map(s => s.key) : [strip.key];
    } else {
      keys = modifiers.ctrlKey ? selected.includes(strip.key) ? selected.filter(k => k !== strip.key) : [...selected, strip.key] : [strip.key];
      if (!modifiers.ctrlKey) anchor.current = strip.key;
    }
    localSelection.current = keys;
    onSelect(keys);
  };
  const single = (strip: MixerStrip, field: 'mute' | 'solo' | 'solo_safe', modifiers: Modifiers) => {
    const keys = operationTargets(strips, strip.key, selected, modifiers).map(s => s.key);
    const value = !(field === 'solo_safe' ? strip.safe : strip[field]);
    if (!modifiers.altKey && field !== 'solo_safe') gesture.commit([stripChange(timeline.id, strip, { [field]: value }, !(modifiers.ctrlKey && modifiers.shiftKey))]);
    else gesture.commit(switchOps(keys, field, value, modifiers));
  };
  const stripSetting=async(strip:MixerStrip,path:string,value:unknown,modifiers:Modifiers)=>{
    const targets=operationTargets(strips,strip.key,selected,modifiers);
    if(automation?.enabled&&automation.recording&&['eq','dynamics','inserts_on'].includes(path)){
      try {
        const groups=await Promise.all(targets.filter(s=>!s.locked).map(async s=>{
          const of=targetOf(s), result=await callEngine(bridge,'automation.parameters',{timeline:timeline.id,of},{expectedSession:session});
          if(result.revision!==latest.current.revision||session!==latest.current.session)return [];
          return result.parameters.filter(p=>p.path===path||p.path.startsWith(`${path}.`)).flatMap(p=>{
            const parts=p.path===path?[]:p.path.slice(path.length+1).split('.');
            if(path==='eq'&&/^\d+$/.test(parts[0]??''))parts.unshift('bands');
            let field:any=value;for(const part of parts)field=field?.[part];
            return typeof field==='number'||typeof field==='boolean'?[{target:{of,path:p.path},value:Number(field)}]:[];
          });
        }));
        automation.instantBatch(groups.flat());
      }catch(error){setProblem(failure(error));}
      return;
    }
    gesture.commit(processorChanges(timeline.id,targets.map(s=>automation?.enabled===false?{...s,arm:'off' as const}:s),path,value));
  };
  const draw = (strip: MixerStrip) => {
    return <ChannelStrip onArm={(arm,modifiers)=>{const targets=operationTargets(strips,strip.key,selected,modifiers);gesture.commit(targets.map(s=>stripChange(timeline.id,s,{automation_arm:arm})));}} key={strip.key} strip={strip} compact={compact} seconds={frame * timeline.rate.den / timeline.rate.num} selected={selected.includes(strip.key)}
      onReorder={reorder} selectedKeys={selected}
      processorContext={{ bridge, session, revision, timeline, target: targetOf(strip), selected, seconds: frame * timeline.rate.den / timeline.rate.num, automation }}
      onProcessor={module => setProcessors(old => [...(compact ? [] : old.filter(p => p.key !== strip.key || p.module !== module)), { key: strip.key, module }])}
      onStripSetting={(path, value, modifiers) => void stripSetting(strip,path,value,modifiers)}
      value={gesture.active() ? preview[strip.key] : undefined} meters={meters} prefader={Boolean(timeline.audio?.prefader_metering)}
      shadow={Boolean(timeline.audio?.prefader_metering && strips.some(s => s.solo) && !strip.solo && !strip.safe)}
      isActive={gesture.active} onSelect={modifiers => select(strip, modifiers)} onRename={async name => {
        try { await callEngine(bridge, 'transaction.apply', { revision, ops: [strip.kind === 'track' ? { op: 'set_track', timeline: timeline.id, id: strip.id, name }
          : { edit: 'set_bus', timeline: timeline.id, id: strip.id, patch: { name } }] }, { expectedSession: session }); return true; }
        catch (error) { setProblem(failure(error)); return false; }
      }} onStart={modifiers => start(strip, modifiers)} onChange={change} onEnd={() => { plan.current = null; gesture.end(); setPreview({}); }} onCancel={cancel}
      onCommit={(value, modifiers = {}) => { const p = faderPlan(timeline, strip.key, selected, modifiers, automation?.enabled??true); gesture.commit(faderChanges(timeline.id, p, value - Math.max(-100, strip.db))); }}
      onToggle={(field, modifiers) => single(strip, field, modifiers)}
      onSweep={(field, event) => {
        if (event.button !== 0 || strip.locked) return;
        event.preventDefault(); event.stopPropagation(); event.currentTarget.focus();
        sweep.current = { field, value: !(field === 'solo_safe' ? strip.safe : strip[field]), keys: new Set(operationTargets(strips, strip.key, selected, event).map(s => s.key)),
          modifiers: { altKey: event.altKey, ctrlKey: event.ctrlKey, shiftKey: event.shiftKey } };
      }} onEnter={(field, event) => { if (event.buttons === 1 && sweep.current?.field === field && !strip.locked) sweep.current.keys.add(strip.key); }} />;
  };
  const drawItem = (key: string) => {
    const strip = strips.find(s => s.key === key);
    if (strip) return draw(strip);
    const vca = timeline.vcas?.find(v => `vca:${v.id}` === key);
    if (!vca || !(strips.some(s => s.strip?.vcas?.includes(vca.id)) || timeline.vcas?.some(v => v.vcas?.includes(vca.id)))) return null;
    return <VcaStrip key={key} context={{bridge,session,revision,timeline}} vca={vca} seconds={frame*timeline.rate.den/timeline.rate.num} selected={selected.includes(key)} onSelect={()=>onSelect([key])}/>;
  };
  return <section className="audio-mixer" data-compact={compact || undefined} data-small={layout.small || undefined} data-labels={layout.labels} data-hidden={layout.hidden.join(' ')} aria-label={t('Mixer channels')} onKeyDown={event => {
    isolateMixerKeys(event); if (!keyOf(event).composing && event.key === 'Escape') { event.preventDefault(); cancel(); }
  }}>
    <header className="mixer-toolbar"><button type="button" onClick={() => setRouting('format')}>{t('Bus Format')}</button><button type="button" onClick={() => setRouting('assign')}>{t('Bus Assign')}</button><label><input type="checkbox" checked={timeline.audio?.exclusive_solo ?? false}
      onChange={event => gesture.commit([{ op: 'set_timeline_audio', timeline: timeline.id, patch: { exclusive_solo: event.target.checked } }])} />{t('Exclusive Solo')}</label>
      <button type="button" aria-label={t('Mixer options')} aria-expanded={layoutOpen} onClick={() => setLayoutOpen(old => !old)}>…</button>
      {layoutOpen && <div className="mixer-layout-menu" role="menu" aria-label={t('Mixer options')}>
        <button type="button" role="menuitemradio" aria-checked={!layout.small} onClick={() => setLayout({ small: false })}>{t('Full Track Mixer')}</button><button type="button" role="menuitemradio" aria-checked={layout.small} onClick={() => setLayout({ small: true })}>{t('Small Track Mixer')}</button>
        <label><input type="checkbox" checked={layout.labels} onChange={event => setLayout({ labels: event.target.checked })} />{t('Show Labels')}</label>
        <label><input type="checkbox" checked={layout.single} onChange={event => setLayout({ single: event.target.checked })} />{t('Single Mixer')}</label>
        <button type="button" onClick={() => setLayout({ order: resetBusOrder(timeline, layout) })}>{t('Reset Bus Order')}</button>
        <fieldset><legend>{t('Channel Strip Control Visibility')}</legend>{MIXER_SECTIONS.map((section, i) => <label key={section}><input type="checkbox" checked={!layout.hidden.includes(section)} onChange={event => setLayout({ hidden: event.target.checked ? layout.hidden.filter(s => s !== section) : [...layout.hidden, section] })} />{sectionLabels[i] === 'EQ' || sectionLabels[i] === 'VCA' ? sectionLabels[i] : t(sectionLabels[i] as FairlightText)}</label>)}</fieldset>
        <label><input type="checkbox" checked={!layout.hidden.includes('trackfx')} onChange={event => setLayout({ hidden: event.target.checked ? layout.hidden.filter(s => s !== 'trackfx') : [...layout.hidden, 'trackfx'] })} />{t('Visible Track FX')}: {t('Ducker')}</label>
      </div>}
    </header>
    {problem && <p role="alert">{problem}<button type="button" onClick={() => setProblem(null)}>{t('Dismiss')}</button></p>}
    <div className="mixer-regions" onPointerDown={event => {
      if (event.button !== 0 || !(event.target instanceof HTMLElement) || !event.target.matches('.mixer-region, .mixer-regions')) return;
      event.preventDefault(); box.current = {x:event.clientX,y:event.clientY,selected:event.ctrlKey ? selected : []}; event.currentTarget.setPointerCapture?.(event.pointerId);
    }} onPointerMove={event => {
      const start=box.current; if (!start) return;
      const rect={left:Math.min(start.x,event.clientX),top:Math.min(start.y,event.clientY),width:Math.abs(event.clientX-start.x),height:Math.abs(event.clientY-start.y)}; setBoxRect(rect);
      const keys=[...event.currentTarget.querySelectorAll<HTMLElement>('[data-mixer-strip]')].filter(el=>{const r=el.getBoundingClientRect();return r.right>=rect.left && r.left<=rect.left+rect.width && r.bottom>=rect.top && r.top<=rect.top+rect.height;}).map(el=>el.dataset.mixerStrip!);
      onSelect([...new Set([...start.selected,...keys])]);
    }} onPointerUp={() => {box.current=null;setBoxRect(null);}} onPointerCancel={() => {box.current=null;setBoxRect(null);}}>{layout.single ? <div className="mixer-region">{order.map(drawItem)}</div> : <><div className="mixer-region mixer-tracks" aria-label={t('Tracks')}>{order.slice(0,order.indexOf(MIXER_DIVIDER)).map(drawItem)}</div>
      <div role="separator" aria-label={t('Mixer divider')} className="mixer-divider" draggable onDragStart={event => event.dataTransfer.setData('application/x-columbina-mixer-order', JSON.stringify([MIXER_DIVIDER]))} />
      <div className="mixer-region mixer-buses" aria-label={t('Buses')}>{order.slice(order.indexOf(MIXER_DIVIDER)+1).map(drawItem)}</div></>}</div>
    {boxRect && <div className="mixer-selection-box" style={boxRect} />}
    {routing === 'format' && <BusFormat bridge={bridge} session={session} revision={revision} timeline={timeline} onClose={() => setRouting(null)} />}
    {routing === 'assign' && <BusAssign bridge={bridge} session={session} revision={revision} timeline={timeline} onClose={() => setRouting(null)} />}
    {processors.map((p, order) => {
      const owner = strips.find(s => s.key === p.key); if (!owner) return null;
      return <ProcessorWindow key={`${p.key}/${p.module}`} context={{ bridge, session, revision, timeline, target: targetOf(owner), selected, seconds: frame * timeline.rate.den / timeline.rate.num, automation }} module={p.module} name={owner.name} compact={compact} order={order}
        onClose={() => setProcessors(old => old.filter(each => each.key !== p.key || each.module !== p.module))}
        onFront={() => setProcessors(old => old.at(-1) === p ? old : [...old.filter(each => each !== p), p])} />;
    })}
  </section>;
}
