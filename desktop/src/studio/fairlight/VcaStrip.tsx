import { useEffect, useRef, useState } from 'react';
import type { Vca, Timeline } from '../engine';
import { Fader } from '../audio/Fader';
import { formatDb, vcaSoloChanges } from '../audio/mixerModel';
import { laneValue } from './automation';
import { useAudioGesture } from '../timeline/useAudioGesture';
import type { Change } from '../timeline/commands';
import type { RoutingContext } from './BusFormat';
import { useAutomationTouch } from './AutomationTouch';
import { VcaAssignment } from './VcaAssignment';
import { NameField } from '../media/NameField';
import { useFailureText, useFairlightText } from './words';
import { clamp, FADER_GAIN } from '../audio/taper';

/** The same deduplicated ancestor traversal as graph/build::chain, including live recorder values. */
export function useVcaGain(timeline: Timeline, ids: readonly string[], seconds: number) {
  const control=useAutomationTouch(), seen=new Set<string>(), next=[...ids]; let gain=0;
  while(next.length){const id=next.pop()!;if(seen.has(id))continue;seen.add(id);const vca=timeline.vcas?.find(v=>v.id===id);if(!vca)continue;
    const base=laneValue(control?.enabled!==false&&vca.automation_arm!=='off'?vca.lanes?.fader_db:undefined,seconds,vca.fader_db??0);
    const target={of:{vca:id},path:'fader_db'};
    const held=control?.status(target), heldTrim=Boolean(control?.trim&&(held==='write'||held==='preview'));
    const value=control?.reading(target,heldTrim?0:base)??base;
    // Recorder Trim values are offsets; VCA's baseline is still audible underneath.
    // graph::automation::Held::apply: Trim cannot lift silence, and stops at +10 dB.
    const term=heldTrim ? base<=-100?base:Math.min(10,base+value) : value;
    if(term<=-100)return -Infinity;
    gain += term;
    next.push(...vca.vcas??[]);
  }
  return gain;
}

export function VcaStrip({context,vca,seconds,selected,onSelect}:{context:RoutingContext;vca:Vca;seconds:number;selected:boolean;onSelect:()=>void}) {
  const t=useFairlightText(), failure=useFailureText(), control=useAutomationTouch(), [renaming,setRenaming]=useState(false), [problem,setProblem]=useState(''), [preview,setPreview]=useState<number|null>(null);
  const target={of:{vca:vca.id},path:'fader_db'}, recording=Boolean(control?.enabled&&control.recording);
  const lane=control?.enabled!==false&&vca.automation_arm!=='off'?vca.lanes?.fader_db:undefined;
  const base=laneValue(lane,seconds,vca.fader_db??0), trim=Boolean(recording&&control?.trim&&vca.automation_arm!=='safe'&&vca.automation_arm!=='off'&&(vca.automation_arm===true||context.timeline.audio?.record?.touch!=='off'));
  const value=control?.reading(target,trim?0:base)??base;
  const offset=useVcaGain(context.timeline,vca.vcas??[],seconds);
  const op=(patch:Record<string,unknown>):Change=>({op:'set_vca',timeline:context.timeline.id,id:vca.id,patch});
  const gesture=useAudioGesture({...context,identity:`${context.timeline.id}/vca/${vca.id}`,locked:false,onFailure:e=>setProblem(failure(e))});
  const end=()=>{if(recording){if(control?.active(target))control.end();}else gesture.end();setPreview(null);};
  const cancel=()=>{if(control?.active(target))control.end();gesture.cancel();setPreview(null);};
  const latest=useRef(cancel);latest.current=cancel;useEffect(()=>()=>latest.current(),[context.session,context.timeline.id,vca.id]);
  const muteTarget={of:{vca:vca.id},path:'mute'};
  const baseMute=laneValue(control?.enabled!==false&&vca.automation_arm!=='off'?vca.lanes?.mute:undefined,seconds,Number(vca.mute??false));
  const muted=(control?.reading(muteTarget,baseMute,{kind:'switch',scale:'linear'})??baseMute)>=.5;
  return <article className="mixer-strip" data-mixer-strip={`vca:${vca.id}`} data-selected={selected||undefined} aria-label={vca.name}>
    <div className="mixer-strip-color">{vca.id}</div><VcaAssignment context={context} of={{vca:vca.id}}/>
    {renaming?<NameField name={vca.name} onCancel={()=>setRenaming(false)} onCommit={async name=>{gesture.commit([op({name})]);setRenaming(false);return true;}}/>:<button className="mixer-name" onClick={onSelect} onDoubleClick={()=>setRenaming(true)}>{vca.name}</button>}
    <div className="mixer-switches"><button type="button" aria-label={`${t('solo')} ${vca.name}`} aria-pressed={vca.solo??false} onClick={()=>gesture.commit(vcaSoloChanges(context.timeline,vca.id,!vca.solo))}>S</button>
      <button type="button" aria-label={`${t('mute')} ${vca.name}`} aria-pressed={muted} disabled={!recording&&Boolean(vca.lanes?.mute)&&vca.automation_arm!=='off'&&control?.enabled!==false} onClick={()=>recording?control?.instant(muteTarget,Number(!muted)):gesture.commit([op({mute:!muted})])}>M</button></div>
    {control?.enabled&&<><button type="button" aria-pressed={vca.automation_arm===true} onClick={()=>gesture.commit([op({automation_arm:vca.automation_arm!==true})])}>{t('Arm')}</button><button type="button" onClick={()=>control.show(target)}>{t('Show automation in timeline')}</button></>}
    <Fader label={`${t('Fader')} ${vca.name}`} value={gesture.active()?preview??value:value} displayOffset={trim?0:offset} hasLane={Boolean(vca.lanes?.fader_db)} readOnly={Boolean(lane)&&!recording}
      onStart={()=>recording?Boolean(control?.start(target,value)):gesture.start([op({fader_db:vca.fader_db??0})])}
      onChange={next=>{const value=clamp(next,...FADER_GAIN);if(recording){if(control?.active(target))control.changeBatch([{target,value}]);}else{setPreview(value);gesture.preview([op({fader_db:value})]);}}} onEnd={end} onCancel={cancel}
      onCommit={next=>{if(recording)control?.instant(target,next);else gesture.commit([op({fader_db:next})]);}} isActive={()=>recording?Boolean(control?.active(target)):gesture.active()}/>
    <small>{t('Base value')}: {formatDb(vca.fader_db??0)} dB</small>{problem&&<p role="alert">{problem}</p>}
  </article>;
}
