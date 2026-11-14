import type { Bus, BusFormat, LanesOf, Output, Pan, Send, SourceRef, Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { laneValue, mixerStrips, stripChange, type MixerStrip } from './mixerModel';
import { routingEdges, targetKey } from './processorModel';

export const defaultPan = (): Pan => ({ on: true, x: 0, y: 0, rotate: 0, spread: 99, divergence: 0, lfe_db: -100, lfe_on: false, lfe_pre: false });
export type Routes = 'outputs' | 'sends';
export const BUS_FORMATS: BusFormat[] = ['mono', 'stereo', '3.0', '4.0', '5.0', '5.1', '7.0', '7.1'];
export const busChannels = (format: BusFormat) => BUS_FORMATS.indexOf(format) + 1;
export function nextBus(timeline: Timeline, buses: Bus[], copy?: Bus): Bus {
  const used = new Set([...timeline.tracks.map(t => t.id), ...buses.map(b => b.id)]);
  let n = 1; while (used.has(`B${n}`)) n++;
  const colors = ['#5aa0e0ff', '#e0ad5aff', '#8bbf75ff', '#c689cfff'];
  return copy ? { ...structuredClone(copy), id: `B${n}`, name: `${copy.name} (${n})` }
    : { id: `B${n}`, name: `Bus ${n}`, color: colors[(n - 1) % colors.length], format: 'stereo' };
}
/** Report the same references that make model::remove_bus return in_use, including disabled routes. */
export function busUsers(timeline: Timeline, id: string): string[] {
  const users = mixerStrips(timeline).filter(owner => {
    const strip = owner.strip;
    const sources = [...strip?.track_fx?.sources ?? [], ...strip?.inserts?.flatMap(i => i.effect.kind === 'limiter' && i.effect.sidechain.source ? [i.effect.sidechain.source] : []) ?? [],
      ...strip?.dynamics?.compressor.sidechain.source ? [strip.dynamics.compressor.sidechain.source] : []];
    return [...outputsOf(timeline, owner), ...strip?.sends ?? []].some(r => r.bus === id) || sources.some(s => 'bus' in s && s.bus === id);
  }).map(owner => owner.name);
  if (timeline.audio?.monitor === id) users.push('Control Room');
  return users;
}
type Edges = Map<string, Set<string>>;
const reaches = (edges: Edges, from: string, to: string, seen = new Set<string>()): boolean => {
  if (from === to) return true;
  if (seen.has(from)) return false;
  seen.add(from);
  return [...edges.get(from) ?? []].some(child => reaches(edges, child, to, seen));
};
function deepest(edges: Edges) {
  const memo = new Map<string, number>();
  const visit = (id: string): number => {
    if (memo.has(id)) return memo.get(id)!;
    const n = 1 + Math.max(0, ...[...edges.get(id) ?? []].map(visit)); memo.set(id, n); return n;
  };
  return Math.max(0, ...[...edges.keys()].map(visit));
}
export function outputsOf(timeline: Timeline, strip: MixerStrip): Output[] {
  return strip.strip?.outputs ?? (!strip.strip && strip.kind === 'track' && timeline.buses?.some(b => b.id === 'B1') ? [{ bus: 'B1', on: true }] : []);
}
export function busChoices(timeline: Timeline, of: SourceRef, field: Routes) {
  const source = targetKey(of), owner = mixerStrips(timeline).find(s => s.key === source);
  const routes = owner ? field === 'outputs' ? outputsOf(timeline, owner) : owner.strip?.sends ?? [] : [];
  const edges = routingEdges(timeline);
  const busEdges: Edges = new Map((timeline.buses ?? []).map(b => [b.id, new Set([...(b.strip?.outputs ?? []), ...(b.strip?.sends ?? [])].map(o => o.bus))]));
  return (timeline.buses ?? []).map(bus => {
    const assigned = routes.some(r => r.bus === bus.id);
    let reason: 'bus_cycle' | 'bus_depth' | 'out_of_range' | null = null;
    if (!assigned) {
      if (reaches(edges, `bus:${bus.id}`, source)) reason = 'bus_cycle';
      else if (routes.length >= 10) reason = 'out_of_range';
      else if ('bus' in of) {
        const trial = new Map(busEdges); trial.set(of.bus, new Set([...(trial.get(of.bus) ?? []), bus.id]));
        if (deepest(trial) > 6) reason = 'bus_depth';
      }
    }
    return { bus, assigned, reason };
  });
}
export function routingChanges(timeline: Timeline, owners: SourceRef[], field: Routes, bus: string, action: 'add' | 'remove' | 'toggle'): Change[] {
  const keys = new Set(owners.map(targetKey)), changes: Change[] = [];
  for (const owner of mixerStrips(timeline).filter(s => keys.has(s.key) && !s.locked)) {
    const of = owner.kind === 'track' ? { track: owner.id } : { bus: owner.id };
    const routes: (Send | Output)[] = field === 'outputs' ? outputsOf(timeline, owner) : owner.strip?.sends ?? [];
    const found = routes.find(r => r.bus === bus);
    if (action === 'add' && (found || !busChoices(timeline, of, field).some(c => c.bus.id === bus && !c.reason))) continue;
    if (action !== 'add' && !found) continue;
    const next = action === 'add' ? [...routes, field === 'outputs' ? { bus, on: true } : { bus, on: true, pre: false, level_db: 0, pan: defaultPan() }]
      : action === 'remove' ? routes.filter(r => r.bus !== bus) : routes.map(r => r.bus === bus ? { ...r, on: !r.on } : r);
    changes.push(stripChange(timeline.id, owner, { strip: { [field]: next } }));
  }
  return changes;
}
/** As graph/build::chain: a shared VCA ancestor contributes only once. */
export function vcaOffset(timeline: Timeline, ids: readonly string[], seconds: number, automation = true) {
  const seen = new Set<string>(), next = [...ids]; let value = 0;
  while (next.length) {
    const id = next.pop()!; if (seen.has(id)) continue; seen.add(id);
    const vca = timeline.vcas?.find(v => v.id === id); if (!vca) continue;
    const lane = automation && vca.automation_arm !== 'off' ? vca.lanes?.fader_db : undefined;
    value += lane?.points.length ? laneValue(lane, seconds) : vca.fader_db ?? 0;
    next.push(...vca.vcas ?? []);
  }
  return value;
}
export function vcaChoices(timeline: Timeline, of: LanesOf) {
  const vcas = timeline.vcas ?? [];
  const edges: Edges = new Map(vcas.map(v => [v.id, new Set(v.vcas ?? [])]));
  const assigned = 'vca' in of ? vcas.find(v => v.id === of.vca)?.vcas ?? []
    : mixerStrips(timeline).find(s => s.key === targetKey(of))?.strip?.vcas ?? [];
  return vcas.map(vca => {
    let reason: 'vca_cycle' | 'vca_depth' | 'out_of_range' | null = null;
    if (!assigned.includes(vca.id)) {
      if ('vca' in of && reaches(edges, vca.id, of.vca)) reason = 'vca_cycle';
      else if (assigned.some(id => reaches(edges, id, vca.id) || reaches(edges, vca.id, id))) reason = 'out_of_range';
      else if ('vca' in of) {
        const trial = new Map(edges); trial.set(of.vca, new Set([...assigned, vca.id]));
        if (deepest(trial) > 6) reason = 'vca_depth';
      }
    }
    return { vca, assigned: assigned.includes(vca.id), reason };
  });
}
