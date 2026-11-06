import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { callEngine, type AutomationParameter } from '../engine';
import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Dynamics, Equalizer, Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { mixerStrips, operationTargets, type Modifiers } from './mixerModel';
import { defaultDynamics, defaultEq, processorChanges, processorPlayback, processorWritable, targetKey, type ProcessorModule, type ProcessorTarget } from './processorModel';

export interface ProcessorProps {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; target: ProcessorTarget; selected?: readonly string[]; seconds?: number;
}
export function useProcessor<M extends ProcessorModule>(props: ProcessorProps, module: M) {
  type Settings = M extends 'eq' ? Equalizer : Dynamics;
  const { bridge, session, revision, timeline, target, selected = [] } = props;
  const automation=useAutomationTouch(), recording=Boolean(automation?.enabled&&automation.recording&&! ('clip' in target));
  const identity = `${timeline.id}/${targetKey(target)}/${module}`;
  const strips = mixerStrips(timeline), strip = strips.find(s => s.key === targetKey(target));
  const clipOwner = 'clip' in target ? timeline.tracks.find(t => t.kind === 'audio' && t.clips.some(c => c.id === target.clip)) : undefined;
  const clip = 'clip' in target && clipOwner?.kind === 'audio' ? clipOwner.clips.find(c => c.id === target.clip) : undefined;
  const model = (module === 'eq' ? clip?.eq ?? strip?.strip?.eq ?? defaultEq('clip' in target) : strip?.strip?.dynamics ?? defaultDynamics()) as Settings;
  const disabled = 'clip' in target ? !clip || Boolean(clipOwner?.locked) : !strip || strip.locked || (!recording && automation?.enabled!==false && !processorWritable(strip, module));
  const [problem, setProblem] = useState<string | null>(null), [preview, setPreview] = useState<Settings | null>(null);
  const gesture = useAudioGesture({ bridge, session, revision, identity, locked: disabled, onFailure: error => setProblem(error instanceof Error ? error.message : String(error)) });
  const snapshot = useRef(model), touched=useRef<string[]>([]);
  const [parameters,setParameters]=useState<AutomationParameter[]>([]);
  useEffect(()=>{setParameters([]);if('clip' in target||!automation?.enabled)return;let live=true;void callEngine(bridge,'automation.parameters',{timeline:timeline.id,of:target},{expectedSession:session}).then(r=>{if(live&&r.revision===revision)setParameters(r.parameters);}).catch(e=>{if(live)setProblem(String(e));});return()=>{live=false;};},[bridge,session,revision,identity,automation?.enabled]);
  const read=(settings:Settings,path:string):number=>{const parts=path.slice(module.length+1).split('.');if(module==='eq'&&/^\d+$/.test(parts[0]))parts.unshift('bands');let value:any=settings;for(const p of parts)value=value?.[p];return typeof value==='boolean'?Number(value):Number(value);};
  const playback=()=>{
    const value=automation?.enabled===false?model:processorPlayback(model,strip,module,props.seconds??0);
    if(!automation?.recording||'clip' in target)return value;
    const shown=structuredClone(value);
    for(const p of parameters.filter(p=>p.path.startsWith(`${module}.`))){
      const parts=p.path.slice(module.length+1).split('.');if(module==='eq'&&/^\d+$/.test(parts[0]))parts.unshift('bands');
      let owner:any=shown;for(const part of parts.slice(0,-1))owner=owner?.[part];const field=parts.at(-1)!;
      if(owner&&(typeof owner[field]==='number'||typeof owner[field]==='boolean')){const n=automation.reading({of:target,path:p.path},Number(owner[field]),p);owner[field]=typeof owner[field]==='boolean'?n>=.5:n;}
    }
    return shown;
  };
  const values=(settings:Settings,paths:string[])=>paths.map(path=>({target:{of:target as import('../engine').LanesOf,path},value:read(settings,path)}));
  const changed=(value:Settings)=>parameters.filter(p=>p.path.startsWith(`${module}.`)&&read(value,p.path)!==read(playback(),p.path)).map(p=>p.path);
  const ownsTouch=()=>touched.current.some(path=>automation?.active({of:target as import('../engine').LanesOf,path}));
  const changes = (value: Settings, modifiers: Modifiers = {}): Change[] => 'clip' in target
    ? [{ op: 'set_clip', timeline: timeline.id, id: target.clip, eq: value as Equalizer }]
    : processorChanges(timeline.id, operationTargets(strips, targetKey(target), selected, modifiers).map(s=>automation?.enabled===false?{...s,arm:'off' as const}:s), module, value);
  useEffect(() => { setPreview(null); setProblem(null); }, [identity, session]);
  const start = (paths?:string|string[]) => {
    if(disabled)return false;
    if(recording){const wanted=typeof paths==='string'?[paths]:paths??parameters.filter(p=>p.path.startsWith(`${module}.`)).map(p=>p.path);touched.current=wanted.filter(path=>parameters.some(p=>p.path===path));snapshot.current=playback();if(!automation!.startBatch(values(snapshot.current,touched.current))){touched.current=[];return false;}setPreview(snapshot.current);return true;}
    if(!gesture.start(changes(model)))return false;snapshot.current=model;setPreview(model);return true;
  };
  const end=()=>{if(ownsTouch())automation!.end();else if(!recording)gesture.end();setPreview(null);touched.current=[];};
  const commit=(value:Settings,modifiers:Modifiers={})=>{
    if(disabled)return;
    if(recording){const paths=changed(value);if(paths.length)automation!.instantBatch(values(value,paths));return;}
    gesture.commit(changes(value,modifiers));
  };
  return { value: recording&&ownsTouch()?preview??playback():gesture.active()?preview??model:playback(), model, disabled, recording, problem,
    gesture:{...gesture,active:()=>recording?ownsTouch():gesture.active()},
    start,snapshot:()=>snapshot.current,
    change:(value:Settings)=>{if(recording){if(!ownsTouch())return;setPreview(value);automation!.changeBatch(values(value,touched.current));}else if(gesture.active()){setPreview(value);gesture.preview(changes(value));}},
    end,cancel:()=>{if(ownsTouch())automation?.end();gesture.cancel();setPreview(null);touched.current=[];},commit,
    show:(path:string)=>{if(!('clip' in target))automation?.show({of:target,path});},
    enable:(on:boolean,modifiers:Modifiers={})=>{
      if(disabled)return;
      if(recording){automation!.instant({of:target as import('../engine').LanesOf,path:`${module}.on`},Number(on));return;}
      gesture.commit('clip' in target?changes({...model,on}):processorChanges(timeline.id,operationTargets(strips,targetKey(target),selected,modifiers).map(s=>automation?.enabled===false?{...s,arm:'off' as const}:s),module,{on}));
    },
  };
}
