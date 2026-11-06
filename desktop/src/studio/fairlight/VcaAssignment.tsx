import { useEffect, useRef, useState } from 'react';
import { PortalMenu } from '../audio/PortalMenu';
import { callEngine, type LanesOf, type Timeline, type Vca } from '../engine';
import { mixerStrips, operationTargets, stripChange, type Modifiers } from '../audio/mixerModel';
import { targetOf } from '../audio/processorModel';
import { vcaChoices } from '../audio/routingModel';
import type { Change } from '../timeline/commands';
import type { RoutingContext } from './BusFormat';
import { useFairlightText } from './words';
import './routing.css';

export function vcaAssignments(timeline: Timeline, owners: LanesOf[], vca: Vca | null, action: 'add' | 'remove' = 'add'): Change[] {
  const existing = timeline.vcas ?? [], trial = vca && !existing.some(row => row.id === vca.id) ? { ...timeline, vcas: [...existing, vca] } : timeline;
  if ((trial.vcas?.length ?? 0) > 64) return [];
  const ops: Change[] = [];
  for (const of of owners) {
    const strip = 'vca' in of ? undefined : mixerStrips(trial).find(s => s.key === ('track' in of ? `track:${of.track}` : `bus:${of.bus}`));
    if (!('vca' in of) && (!strip || strip.locked)) continue;
    const owner = 'vca' in of ? trial.vcas?.find(row => row.id === of.vca) : strip?.strip;
    if ('vca' in of && !owner) continue;
    const choices = vcaChoices(trial, of), choice = choices.find(row => row.vca.id === vca?.id);
    if (vca && (!choice || action==='add' && choice.reason)) continue;
    const old = owner?.vcas ?? [];
    if(vca && (action==='add' ? old.includes(vca.id) : !old.includes(vca.id)))continue;
    const ids = !vca ? [] : action==='remove' ? old.filter(id => id !== vca.id) : [...old, vca.id];
    if (!vca && !old.length) continue;
    ops.push('vca' in of ? { op: 'set_vca', timeline: timeline.id, id: of.vca, patch: { vcas: ids } } : stripChange(timeline.id, strip!, { strip: { vcas: ids } }));
  }
  return ops.length && vca && trial !== timeline ? [{ op:'add_vca', timeline: timeline.id, vca }, ...ops] : ops;
}

export function VcaAssignment({ context, of, selected = [] }: { context: RoutingContext; of: LanesOf; selected?: readonly string[] }) {
  const t = useFairlightText(), [open,setOpen] = useState(false), [problem,setProblem] = useState(''), [pending,setPending] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => setOpen(false), [context.session, context.timeline.id]);
  const { timeline, bridge, session, revision } = context;
  const strip = 'vca' in of ? undefined : mixerStrips(timeline).find(s => s.key === ('track' in of ? `track:${of.track}` : `bus:${of.bus}`));
  const owner = 'vca' in of ? timeline.vcas?.find(v => v.id === of.vca) : strip?.strip;
  const name = 'vca' in of ? timeline.vcas?.find(v=>v.id===of.vca)?.name : strip?.name;
  const existing = timeline.vcas ?? [], count = Math.min(64,Math.max(10,existing.length+1));
  const choices = [...existing];
  for(let n=1;choices.length<count;n++) if(!choices.some(v=>v.id===`VCA${n}`))choices.push({id:`VCA${n}`,name:`VCA ${n}`});
  const apply = async (vca: Vca|null, modifiers: Modifiers) => {
    if(pending) return;
    const targets = 'vca' in of ? [of] : operationTargets(mixerStrips(timeline),strip!.key,selected,modifiers).map(targetOf);
    const action = vca && owner?.vcas?.includes(vca.id) ? 'remove' : 'add';
    const ops=vcaAssignments(timeline,targets,vca,action); if(!ops.length)return;
    setPending(true);setProblem('');
    try { await callEngine(bridge,'transaction.apply',{revision,ops},{expectedSession:session});setOpen(false); }
    catch(error) { setProblem(String(error)); } finally { setPending(false); }
  };
  return <span className="vca-assignment"><button ref={trigger} type="button" disabled={strip?.locked || pending} aria-haspopup="menu" aria-expanded={open} aria-label={`VCA ${name ?? ''}`} onClick={()=>setOpen(!open)} onContextMenu={event=>{event.preventDefault();event.currentTarget.focus();setOpen(true);}}
    onKeyDown={event => { if (!event.nativeEvent.isComposing && ['ArrowDown', 'Enter', ' '].includes(event.key)) { event.preventDefault(); event.stopPropagation(); setOpen(true); } }}>
    {owner?.vcas?.map(id=>existing.find(v=>v.id===id)?.name??id).join(', ') || t('No VCA')}</button>
    {open && <PortalMenu anchor={trigger} label={t('Assign VCA')} onClose={() => setOpen(false)}>
      <button type="button" role="menuitem" disabled={pending} onClick={event=>void apply(null,event)}>{t('No VCA')}</button>
      {choices.map(vca=>{const trial=existing.includes(vca)?timeline:{...timeline,vcas:[...existing,vca]}, choice=vcaChoices(trial,of).find(c=>c.vca.id===vca.id)!;return <button type="button" role="menuitemcheckbox" key={vca.id} aria-checked={choice.assigned} disabled={pending||Boolean(choice.reason)} title={choice.reason??undefined} onClick={event=>void apply(vca,event)}>{vca.name}</button>;})}
      <button type="button" onClick={()=>setOpen(false)}>{t('Cancel')}</button></PortalMenu>}{problem&&<span role="alert">{problem}</span>}
  </span>;
}
