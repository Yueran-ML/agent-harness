import { useFailureText } from '../fairlight/words';
import { useEffectAutomation } from './useEffectAutomation';
import { useEffect, useRef, useState } from 'react';
import type { Insert } from '../engine';
import { useAudioGesture } from '../timeline/useAudioGesture';
import type { ProcessorProps } from './useProcessor';
import { effectOwner, insertParameters } from './effectModel';
import { targetKey } from './processorModel';
import { laneValue, mixerStrips, operationTargets, type Modifiers } from './mixerModel';
import { useAutomationTouch } from '../fairlight/AutomationTouch';

/** The numbers and switches of `next` that differ from `base`, by their dotted fields (`kind` aside). */
export function changedFields(base: object, next: object, prefix = ''): [string, number][] {
  const changed: [string, number][] = [];
  for (const [key, value] of Object.entries(next)) {
    const field = prefix + key, before = (base as Record<string, unknown>)[key];
    if (key === 'kind' && !prefix) continue;
    if ((typeof value === 'number' || typeof value === 'boolean') && value !== before)
      changed.push([field, Number(value)]);
    else if (value && typeof value === 'object' && !Array.isArray(value) && before && typeof before === 'object')
      changed.push(...changedFields(before, value, `${field}.`));
  }
  return changed;
}

export function useEffectEditor(props: ProcessorProps, index: number) {
  const { timeline, target, session, revision, bridge } = props;
  const failure = useFailureText();
  const owner = effectOwner(timeline, target), model = owner.inserts[index];
  const identity = `${timeline.id}/${targetKey(target)}/${model?.id || index}/${model?.effect.kind}`;
  const [problem, setProblem] = useState<string | null>(null), [preview, setPreview] = useState<Insert | null>(null);
  const gesture = useAudioGesture({ bridge, session, revision, identity, locked: owner.locked, onFailure: error => setProblem(failure(error)) });
  const captured = useRef(model);
  const control = useEffectAutomation(props), {automation, recording} = control, touch = useAutomationTouch();
  const touched = useRef<string | null>(null);
  const activeField = useRef<string | null>(null);
  const touchEnd=useRef<(()=>void)|null>(null);
  const finishTouch = () => { touchEnd.current?.(); touchEnd.current=null; touched.current=null; };
  useEffect(() => () => { finishTouch(); activeField.current = null; }, [identity, session]);
  const path = (field: string) => `insert.${model?.id}.${field}`;
  const writable = (field?: string) => !owner.locked && (recording ? Boolean(field && control.supported(path(field))) : automation?.enabled === false || owner.arm === 'off' || !Object.keys(owner.lanes).some(p => field ? p === path(field) : p.startsWith(`insert.${model?.id}.`)));
  const ops = (value: Insert) => insertParameters(timeline, target, index, value);
  const playback = model ? structuredClone(model) : undefined;
  if (playback && automation?.enabled !== false && owner.arm !== 'off') for (const [key, lane] of Object.entries(owner.lanes)) {
    if (!key.startsWith(`insert.${model?.id}.`) || !lane.points.length) continue;
    const parts = key.slice(`insert.${model?.id}.`.length).split('.');
    let object = (parts[0] === 'on' ? playback : playback.effect) as unknown as Record<string, any>;
    for (const part of parts.slice(0, -1)) object = object?.[part];
    const field = parts.at(-1)!;
    if (object && (typeof object[field] === 'number' || typeof object[field] === 'boolean')) {
      const reading = laneValue(lane, props.seconds ?? 0); object[field] = typeof object[field] === 'boolean' ? reading !== 0 : reading;
    }
  }
  const withField = (base: Insert, field: string, value: unknown): Insert => {
    const next = structuredClone(base);
    const parts = field.split('.'); let object = (field === 'on' ? next : next.effect) as unknown as Record<string, any>;
    for (const part of parts.slice(0, -1)) object = object[part];
    object[parts.at(-1)!] = value; return next;
  };
  const batchOps = (field: string | undefined, value: unknown, modifiers: Modifiers = {}) => {
    if (!modifiers.altKey || 'clip' in target) return model ? ops(field ? withField(model, field, value) : {...model,effect:value as Insert['effect']}) : [];
    return operationTargets(mixerStrips(timeline),targetKey(target),props.selected ?? [],modifiers).flatMap(strip=>{
      const of=strip.kind==='track'?{track:strip.id}:{bus:strip.id};const destination=effectOwner(timeline,of),item=destination.inserts[index];
      if(!item || item.effect.kind!==model?.effect.kind || automation?.enabled!==false && destination.arm!=='off' && Object.keys(destination.lanes).some(p=>field?p==='insert.'+item.id+'.'+field:p.startsWith('insert.'+item.id+'.')))return [];
      return insertParameters(timeline,of,index,field?withField(item,field,value):{...item,effect:structuredClone(value as Insert['effect'])});
    });
  };
  const touchActive = () => Boolean(touched.current && !('clip' in target) && automation?.active({ of: target, path: touched.current }));
  let current = gesture.active() || touchActive() ? preview ?? model : playback;
  for (const live of automation?.enabled === false ? [] : automation?.lives ?? (automation?.live ? [automation.live] : [])) {
    if (!current || 'clip' in target || JSON.stringify(live.target.of)!==JSON.stringify(target) || !live.target.path.startsWith('insert.'+model?.id+'.')) continue;
    const field=live.target.path.slice(('insert.'+model?.id+'.').length);const parts=field.split('.');
    let existing:any=field==='on'?current:current.effect;for(const part of parts)existing=existing?.[part];
    if(typeof existing==='number'||typeof existing==='boolean')current=withField(current,field,typeof existing==='boolean'?live.value>=.5:live.value);
  }
  if (current && automation?.enabled) {
    const held = structuredClone(current);
    const visit = (object:Record<string,any>, prefix='') => {
      for (const [field,value] of Object.entries(object)) {
        const name = prefix + field;
        if (typeof value === 'number' || typeof value === 'boolean') { const read=control.reading(path(name),Number(value)); object[field]=typeof value==='boolean'?read>=.5:read; }
        else if (value && typeof value==='object' && !Array.isArray(value)) visit(value,name+'.');
      }
    };
    held.on=control.reading(path('on'),Number(held.on))>=.5; visit(held.effect); current=held;
  }
  const numeric = (field: string) => { const parts = field.split('.'); let value: any = field === 'on' ? current : current?.effect; for (const p of parts) value = value?.[p]; return typeof value === 'boolean' ? Number(value) : value; };
  const active = (field?: string) => (field === undefined || activeField.current === field) && (touched.current ? touchActive() : gesture.active());
  // A preset or a paste may be written as automation while recording (Write on Preset or Clipboard Paste).
  const pastesWritten = Boolean(recording && touch?.writeOnPaste && !('clip' in target));
  return { value: current, model, problem: problem ?? control.problem, path, writable, owner, gesture, active, automation, recording, pastesWritten,
    status: (field:string) => control.status(path(field)), supported:(field:string)=>control.supported(path(field)),
    show: (field: string) => { if (!('clip' in target)) automation?.show({ of: target, path: path(field) }); },
    start: (field: string) => {
      if (!model || !writable(field)) return false;
      if (recording) { finishTouch(); activeField.current = null; if ('clip' in target || !automation?.start({ of: target, path: path(field) }, numeric(field))) return false; touched.current = path(field); const control=automation; const owned={of:target,path:path(field)}; touchEnd.current=()=>{if(control?.active(owned))control.end();}; }
      else if (!gesture.start(ops(model))) return false;
      activeField.current = field; captured.current = current ?? model; setPreview(current ?? model); return true;
    },
    change: (field: string, value: unknown) => {
      if (!active(field) || !captured.current) return;
      const next = withField(captured.current, field, value); setPreview(next);
      if (touched.current) { automation?.change(Number(value)); }
      else gesture.preview(ops(next));
    },
    end: (field: string) => { if (activeField.current !== field) return; if (touched.current) finishTouch(); else gesture.end(); activeField.current = null; setPreview(null); },
    cancel: (field: string) => { if (activeField.current !== field) return; if (touched.current) finishTouch(); else gesture.cancel(); activeField.current = null; setPreview(null); },
    field: (field: string, value: unknown, modifiers: Modifiers = {}) => {
      if (!model || !writable(field)) return;
      if (recording) { if (!('clip' in target) && (typeof value === 'number' || typeof value === 'boolean')) automation?.instant({ of: target, path: path(field) }, Number(value)); return; }
      gesture.commit(batchOps(field, value, modifiers));
    },
    replace: (effect: Insert['effect'], modifiers: Modifiers = {}) => {
      if (!model || model.effect.kind !== effect.kind) return;
      // Write on Preset or Clipboard Paste (p. 121): while recording, the settings that a preset
      // or a paste changes are written as automation, of the categories that are on.
      if (recording && touch?.writeOnPaste && !('clip' in target)) {
        const values = changedFields(model.effect, effect).filter(([field]) => control.supported(path(field)))
          .map(([field, value]) => ({ target: { of: target, path: path(field) }, value }));
        if (values.length) touch.instantBatch(values);
        return;
      }
      if (writable()) gesture.commit(batchOps(undefined, effect, modifiers));
    },
  };
}
