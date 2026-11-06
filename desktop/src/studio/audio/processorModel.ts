import type { Dynamics, EqBand, EqKind, EqRange, Equalizer, SourceRef, Stage, Timeline } from '../engine';
import { laneValue, mixerStrips, stripChange, type MixerStrip } from './mixerModel';
import { clamp } from './taper';

export type ProcessorModule = 'eq' | 'dynamics';
export type ProcessorTarget = SourceRef | { clip: string };
export const ORDERINGS: Stage[][] = [['fx', 'dynamics', 'eq'], ['fx', 'eq', 'dynamics'], ['dynamics', 'fx', 'eq'], ['dynamics', 'eq', 'fx'], ['eq', 'fx', 'dynamics'], ['eq', 'dynamics', 'fx']];
export const EQ_ZONES: Record<EqRange, readonly [number, number]> = { low: [20, 200], medium_low: [100, 1000], medium_high: [500, 5000], high: [2000, 20000] };
export function defaultEq(clip = false): Equalizer {
  const frequencies = clip ? [80, 300, 3000, 16000] : [80, 100, 500, 2000, 8000, 16000];
  return { on: false, gain_db: 0, bands: frequencies.map((freq, index) => ({ on: index > 0 && index < frequencies.length - 1,
    kind: index === 0 ? 'high_pass' : index === frequencies.length - 1 ? 'low_pass' : 'bell', freq, gain_db: 0, q: 1, slope: 12,
    range: !clip && index > 0 && index < 5 ? (['low', 'medium_low', 'medium_high', 'high'] as const)[index - 1] : null })) };
}
export function defaultDynamics(): Dynamics {
  return { on: false, expander: { on: false, mode: 'expander', threshold_db: -35, range_db: 18, ratio: 1.1, attack_ms: 1.4, hold_ms: 0, release_ms: 93 },
    compressor: { on: false, threshold_db: -15, ratio: 2, knee_db: 0, mix: 0, attack_ms: 1.4, hold_ms: 0, release_ms: 93, sidechain: { on: false, listen: false, source: null } },
    limiter: { on: false, threshold_db: -21, attack_ms: .71, hold_ms: 0, release_ms: 90 }, makeup_db: 0 };
}
export function eqBandKinds(index: number, count: number): EqKind[] {
  if (count === 4 && (index === 0 || index === 3)) return ['high_shelf', 'high_pass', 'bell', 'notch', 'low_pass', 'low_shelf'];
  if (index === 0) return ['low_shelf', 'bell', 'high_shelf', 'high_pass'];
  if (index === 5) return ['low_pass', 'low_shelf', 'bell', 'high_shelf'];
  return ['low_shelf', 'bell', 'notch', 'high_shelf'];
}
export const eqHasGain = (kind: EqKind) => ['bell', 'low_shelf', 'high_shelf'].includes(kind);
export const eqHasQ = (kind: EqKind) => kind === 'bell' || kind === 'notch';
export function eqBounds(band: EqBand, sampleRate: number): [number, number] {
  const [low, high] = band.range ? EQ_ZONES[band.range] : [20, 20000];
  return [Math.min(low, .45 * sampleRate), Math.min(high, .45 * sampleRate)];
}
export function changeBand(eq: Equalizer, index: number, patch: Partial<EqBand>, sampleRate: number): Equalizer {
  const band = { ...eq.bands[index], ...patch };
  band.freq = clamp(band.freq, ...eqBounds(band, sampleRate)); band.gain_db = clamp(band.gain_db, -20, 20); band.q = clamp(band.q, .3, 10.3);
  return { ...eq, bands: eq.bands.map((b, i) => i === index ? band : b) };
}
export const frequencyPosition = (hz: number) => Math.log(hz / 20) / Math.log(1000);
export const frequencyAt = (position: number) => 20 * Math.pow(1000, clamp(position, 0, 1));
export const targetOf = (strip: MixerStrip): SourceRef => strip.kind === 'track' ? { track: strip.id } : { bus: strip.id };
export const targetKey = (target: ProcessorTarget) => 'track' in target ? `track:${target.track}` : 'bus' in target ? `bus:${target.bus}` : `clip:${target.clip}`;
/** Module replacement is blocked if it would overwrite an active lane's underlying settings. */
export function processorWritable(strip: MixerStrip, path: string) {
  return !strip.locked && (strip.arm === 'off' || !Object.keys(strip.lanes ?? {}).some(key => key === path || key.startsWith(`${path}.`) || path.startsWith(`${key}.`)));
}
/** Existing lane paths use zero-based EQ indices (`eq.1.freq`) and Dynamics field names. */
export function processorPlayback<T extends Equalizer | Dynamics>(base: T, strip: MixerStrip | undefined, module: ProcessorModule, seconds: number): T {
  if (!strip || strip.arm === 'off' || !strip.lanes) return base;
  const value = structuredClone(base);
  for (const [path, lane] of Object.entries(strip.lanes)) {
    if (!path.startsWith(`${module}.`) || !lane.points.length) continue;
    const parts = path.slice(module.length + 1).split('.');
    if (module === 'eq' && /^\d+$/.test(parts[0])) parts.unshift('bands');
    let owner = value as unknown as Record<string, any>;
    for (const part of parts.slice(0, -1)) { owner = owner?.[part]; if (!owner) break; }
    const field = parts.at(-1)!;
    if (owner && (typeof owner[field] === 'number' || typeof owner[field] === 'boolean')) owner[field] = typeof owner[field] === 'boolean' ? laneValue(lane, seconds) !== 0 : laneValue(lane, seconds);
  }
  return value;
}
export function processorChanges(timeline: string, strips: MixerStrip[], path: string, value: unknown) {
  return strips.filter(s => processorWritable(s, path)).map(s => stripChange(timeline, s, { strip: { [path]: value } }));
}
/** Dependencies point from a source to its consumer, including inactive stored routing, as model validation does. */
export function routingEdges(timeline: Timeline, replacingCompressor?: string) {
  const strips = mixerStrips(timeline), edges = new Map<string, Set<string>>();
  const edge = (a: string, b: string) => { if (!edges.has(a)) edges.set(a, new Set()); edges.get(a)!.add(b); };
  for (const owner of strips) {
    const strip = owner.strip;
    const outputs = strip?.outputs ?? (!strip && owner.kind === 'track' && timeline.buses?.some(b => b.id === 'B1') ? [{ bus: 'B1' }] : []);
    for (const output of [...outputs, ...(strip?.sends ?? [])]) edge(owner.key, `bus:${output.bus}`);
    const sources = [...(strip?.track_fx?.sources ?? []), ...strip?.inserts?.flatMap(i => i.effect.kind === 'limiter' && i.effect.sidechain.source ? [i.effect.sidechain.source] : []) ?? []];
    // Replacing this owner's compressor source removes its old dependency.
    if (owner.key !== replacingCompressor && strip?.dynamics?.compressor.sidechain.source) sources.push(strip.dynamics.compressor.sidechain.source);
    for (const source of sources) edge(targetKey(source), owner.key);
  }
  return edges;
}
export function sidechainSources(timeline: Timeline, target: SourceRef) {
  const strips = mixerStrips(timeline), key = targetKey(target), edges = routingEdges(timeline, key);
  const downstream = new Set<string>();
  const visit = (node: string) => { if (downstream.has(node)) return; downstream.add(node); for (const child of edges.get(node) ?? []) visit(child); };
  visit(key);
  return strips.filter(s => s.key !== key).map(s => ({ key: s.key, name: s.name, source: targetOf(s), disabled: downstream.has(s.key) }));
}
