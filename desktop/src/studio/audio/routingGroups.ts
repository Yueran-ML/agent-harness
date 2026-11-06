import type { Pan, Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { groupMembers, mixerStrips, stripChange, type MixerStrip } from './mixerModel';
import { processorWritable } from './processorModel';
import { defaultPan } from './routingModel';
export type RoutingValues = Record<string, number>;
export function routingBase(strip: MixerStrip, bus: string | undefined, field: string): number {
  const send = strip.strip?.sends?.find(row => row.bus === bus);
  if (bus === undefined)
    return Number(({ ...defaultPan(), ...strip.strip?.pan })[field as keyof Pan]);
  if (field.startsWith('pan.'))
    return Number(({ ...defaultPan(), ...send?.pan })[field.slice(4) as keyof Pan]);
  return Number(field === 'on' ? send?.on ?? true : field === 'pre' ? send?.pre ?? false : send?.level_db ?? 0);
}
function change(timeline: string, strip: MixerStrip, bus: string | undefined, values: RoutingValues, follow = false): Change {
  const panPatch = (prefix = '') => Object.fromEntries(Object.entries(values).filter(([key]) => key.startsWith(prefix)).map(([key, value]) => {
    const field = key.slice(prefix.length);
    return [field, ['on', 'lfe_on', 'lfe_pre'].includes(field) ? value >= .5 : value];
  }));
  const patch = bus === undefined ? { pan: panPatch() } : { sends: (strip.strip?.sends ?? []).map(row => row.bus !== bus ? row : {
      ...row, ...Object.fromEntries(Object.entries(values).filter(([key]) => !key.startsWith('pan.')).map(([key, value]) => [key, key === 'on' || key === 'pre' ? value >= .5 : value])),
      ...(Object.keys(values).some(key => key.startsWith('pan.')) ? { pan: { ...defaultPan(), ...row.pan, ...panPatch('pan.') } } : {}),
    }) };
  return stripChange(timeline, strip, { strip: patch }, follow);
}
export interface RoutingPlan {
  timeline: string;
  source: MixerStrip;
  bus: string | undefined;
  members: MixerStrip[];
  follow: boolean;
  fields: string[];
  restore: Change[];
}
export function routingPlan(timeline: Timeline, source: MixerStrip, bus: string | undefined, fields: string[], playback: boolean): RoutingPlan {
  const linkedFields = bus === undefined ? fields : fields.filter(field => field === 'level_db' || field === 'on');
  const linked = linkedFields.length ? groupMembers(timeline, mixerStrips(timeline), [source], bus === undefined ? 'panning' : 'sends', false)
    .filter(member => member.key === source.key || bus === undefined || member.strip?.sends?.some(send => send.bus === bus)) : [source];
  const prefix = bus === undefined ? 'pan.' : `send.${bus}.`;
  const members = linked.filter(member => !playback || linkedFields.every(field => processorWritable(member, prefix + field)));
  const memberFields = (member: MixerStrip) => member.key === source.key ? fields : linkedFields;
  return {
    timeline: timeline.id, source, bus, members, fields, follow: source.kind === 'track' && members.length === linked.length,
    restore: members.map(member => change(timeline.id, member, bus, Object.fromEntries(memberFields(member).map(field => [field, routingBase(member, bus, field)]))))
  };
}
export function routingChanges(plan: RoutingPlan, values: RoutingValues, preview = false): Change[] {
  if (plan.follow)
    return [...(preview && plan.members.length > 1 ? plan.restore : []), change(plan.timeline, plan.source, plan.bus, values, true)];
  // A readonly automation lane cannot follow a static edit. Address only writable members;
  // the relative math below mirrors model/edit/audio.rs for this filtered fallback.
  return plan.members.map(member => {
    if (member.key === plan.source.key)
      return change(plan.timeline, member, plan.bus, values);
    const moved: RoutingValues = {};
    for (const [field, value] of Object.entries(values)) {
      if (plan.bus !== undefined && field !== 'on' && field !== 'level_db')
        continue;
      const before = routingBase(plan.source, plan.bus, field), own = routingBase(member, plan.bus, field);
      if (['on', 'lfe_on', 'lfe_pre'].includes(field)) {
        if (value !== before)
          moved[field] = value;
        continue;
      }
      const level = field === 'level_db' || field === 'lfe_db';
      const delta = level ? Math.max(-100, value) - Math.max(-100, before) : value - before;
      const [low, high] = level ? [-100, 10] : field === 'rotate' ? [-180, 180] : field === 'spread' ? [1, 99] : field === 'divergence' ? [0, 100] : [-1, 1];
      moved[field] = level && own <= -100 && delta <= 0 ? own : Math.max(low, Math.min(high, own + delta));
    }
    return change(plan.timeline, member, plan.bus, moved);
  });
}
