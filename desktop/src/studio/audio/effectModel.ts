import type { Ducker, Effect, Insert, InsertsOf, Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { mixerStrips, stripChange } from './mixerModel';
import { defaultDynamics, defaultEq, ORDERINGS, targetKey } from './processorModel';

export function defaultEffect(kind: Effect['kind']): Effect {
  switch (kind) {
    case 'gain': return { kind, gain_db: 0 };
    case 'limiter': return { kind, input_db: 0, soft: false, ceiling_db: -1, release_ms: 100, sidechain: { on: false, listen: false, source: null } };
    case 'soft_clipper': return { kind, threshold_db: 0, shape: 100, output_db: 0 };
    case 'stereo_width': return { kind, width: 1, diffusion: 0, sparkle: 0 };
    case 'de_esser': return { kind, frequency_hz: 6000, range: 'wide', amount: 50, reaction: 'medium', listen: false };
    case 'de_hummer': return { kind, frequency_hz: 50, amount: 50, slope: .5, listen: false };
    case 'pitch': return { kind, semitones: 0, cents: 0, dry_wet: 100 };
    case 'noise_reduction': return { kind, mode: 'auto_speech', threshold_db: 10, attack_ms: 1000, sensitivity: 50, ratio: 4, frequency_smoothing: 25, time_smoothing: true, dry_wet: 100, level_db: 0, listen: false, profile: null, band: 'full' };
  }
}
export const defaultDucker = (): Ducker => ({ on: false, sources: [], amount_db: 2.7, lookahead_ms: 15, rise_ms: 10, hold_ms: 150, recovery_ms: 750 });
export interface EffectNumber { field: string; label: string; min: number; max: number; fallback: number; step: number; unit?: string; logarithmic?: boolean; integer?: boolean }
const number = (field: string, label: string, min: number, max: number, fallback: number, step = .1, unit?: string, logarithmic?: boolean): EffectNumber => ({ field, label, min, max, fallback, step, unit, logarithmic });
export const EFFECT_NUMBERS: Record<Effect['kind'], EffectNumber[]> = {
  gain: [number('gain_db', 'Gain', -24, 24, 0, .1, 'dB')],
  limiter: [number('input_db', 'Input Level', -18, 18, 0, .1, 'dB'), number('ceiling_db', 'Ceiling', -24, 0, -1, .1, 'dBTP'), number('release_ms', 'Release', .01, 1000, 100, .01, 'ms', true)],
  soft_clipper: [number('threshold_db', 'Threshold', 0, 24, 0, .1, 'dB'), number('shape', 'Shape', 0, 100, 100, 1, '%'), number('output_db', 'Output Level', -24, 12, 0, .1, 'dB')],
  stereo_width: [number('width', 'Width', 0, 2, 1, .01), number('diffusion', 'Diffusion', 0, 100, 0, 1, '%'), number('sparkle', 'Sparkle', 0, 100, 0, 1, '%')],
  de_esser: [number('frequency_hz', 'Frequency', 2000, 16000, 6000, 10, 'Hz'), number('amount', 'Amount', 0, 100, 50, 1, '%')],
  de_hummer: [number('frequency_hz', 'Frequency', 40, 70, 50, .1, 'Hz'), number('amount', 'Amount', 0, 100, 50, 1, '%'), number('slope', 'Slope', 0, 1, .5, .01)],
  pitch: [{ ...number('semitones', 'Semitones', -12, 12, 0, 1), integer: true }, number('cents', 'Cents', -100, 100, 0, 1), number('dry_wet', 'Dry/Wet', 0, 100, 100, 1, '%')],
  noise_reduction: [number('threshold_db', 'Threshold', 0, 40, 10, .1, 'dB'), number('attack_ms', 'Attack', 50, 5000, 1000, 1, 'ms'), number('sensitivity', 'Sensitivity', 0, 100, 50, 1, '%'), number('ratio', 'Ratio', 1, 20, 4, .1), number('frequency_smoothing', 'Frequency Smoothing', 0, 100, 25, 1, '%'), number('dry_wet', 'Dry/Wet', 0, 100, 100, 1, '%'), number('level_db', 'Output Level', -6, 18, 0, .1, 'dB')],
};
export const DUCKER_NUMBERS = [number('amount_db', 'Duck Amount', 0, 30, 2.7, .1, 'dB'), number('lookahead_ms', 'Lookahead', 0, 100, 15, .1, 'ms'), number('rise_ms', 'Rise Time', 1, 1000, 10, 1, 'ms'), number('hold_ms', 'Hold', 0, 4000, 150, 1, 'ms'), number('recovery_ms', 'Recovery Time', 10, 4000, 750, 1, 'ms')];
export function effectOwner(timeline: Timeline, of: InsertsOf) {
  if ('clip' in of) {
    const track = timeline.tracks.find(t => t.kind === 'audio' && t.clips.some(c => c.id === of.clip));
    const clip = track?.kind === 'audio' ? track.clips.find(c => c.id === of.clip) : undefined;
    return { exists: Boolean(clip), locked: !clip || Boolean(track?.locked), inserts: clip?.effects ?? [], channels: track?.kind === 'audio' ? track.channels ?? (track.format === 'mono' ? 1 : 2) : 2, lanes: {}, arm: 'off' as const };
  }
  const strip = mixerStrips(timeline).find(s => s.key === targetKey(of));
  return { exists: Boolean(strip), locked: !strip || strip.locked, inserts: strip?.strip?.inserts ?? [], channels: strip?.channels ?? 2, lanes: strip?.lanes ?? {}, arm: strip?.arm };
}
/** Parameter edits preserve the insert identity and its lanes. Structure edits use the edit API. */
export function insertParameters(timeline: Timeline, of: InsertsOf, index: number, value: Insert): Change[] {
  const owner = effectOwner(timeline, of);
  if (owner.locked || !owner.inserts[index] || owner.inserts[index].effect.kind !== value.effect.kind) return [];
  const inserts = owner.inserts.map((insert, i) => i === index ? { ...value, id: insert.id } : insert);
  if ('clip' in of) return [{ op: 'set_clip', timeline: timeline.id, id: of.clip, effects: inserts }];
  const strip = mixerStrips(timeline).find(s => s.key === targetKey(of))!;
  return [stripChange(timeline.id, strip, { strip: { inserts } })];
}
export function addEffect(timeline: Timeline, of: InsertsOf, kind: Effect['kind']): Change[] {
  const owner = effectOwner(timeline, of);
  return owner.locked || (!('clip' in of) && owner.inserts.length >= 6) ? [] : [{ op: 'add_insert', timeline: timeline.id, of, insert: { on: true, effect: defaultEffect(kind) } }];
}
/** Whole-strip copy keeps the destination's identity and routing. Insert lanes follow copy_effects. */
export function copyStripSettings(timeline: Timeline, from: Exclude<InsertsOf, { clip: string }>, to: Exclude<InsertsOf, { clip: string }>): Change[] {
  const strips = mixerStrips(timeline), source = strips.find(s => s.key === targetKey(from)), destination = strips.find(s => s.key === targetKey(to));
  if (!source || !destination || destination.locked || destination.arm !== 'off' && Object.keys(destination.lanes ?? {}).some(path => !path.startsWith('insert.'))) return [];
  const strip = source.strip, dynamics = structuredClone(strip?.dynamics ?? defaultDynamics());
  dynamics.compressor.sidechain = { on: false, listen: false, source: null };
  const patch = { trim_db: strip?.trim_db ?? 0, invert: strip?.invert ?? false, track_fx: 'track' in to && strip?.track_fx ? { ...strip.track_fx, sources: [] } : null,
    order: strip?.order ?? ORDERINGS[0], inserts_on: strip?.inserts_on ?? true, dynamics, eq: structuredClone(strip?.eq ?? defaultEq()), fader_db: strip?.fader_db ?? 0,
    pan: strip?.pan ?? { on: true, x: 0, y: 0, rotate: 0, spread: 99, divergence: 0, lfe_db: -100, lfe_on: false, lfe_pre: false } };
  return [{ edit: 'copy_effects', timeline: timeline.id, from, to: [to] }, stripChange(timeline.id, destination, { strip: patch })];
}
/** Drop onto an occupied slot swaps; Alt replaces that slot with a new copy while retaining its source. */
export function insertDrop(timeline: Timeline, from: InsertsOf, sourceIndex: number, to: InsertsOf, targetIndex: number, copy: boolean): Change[] {
  if (('track' in from && 'bus' in to) || ('bus' in from && 'track' in to)) return [];
  const a = effectOwner(timeline, from), b = effectOwner(timeline, to), item = a.inserts[sourceIndex];
  if (a.locked || b.locked || !item || targetIndex < 0 || targetIndex > b.inserts.length) return [];
  const same = targetKey(from) === targetKey(to), displaced = b.inserts[targetIndex];
  if (same && sourceIndex === targetIndex) return [];
  const fresh = (insert: Insert): Insert => ({ on: insert.on, effect: structuredClone(insert.effect) });
  const remove = (of: InsertsOf, slot: number): Change => ({ edit: 'set_insert', timeline: timeline.id, of, slot, insert: null });
  const add = (of: InsertsOf, index: number, insert: Insert): Change => ({ op: 'add_insert', timeline: timeline.id, of, index, insert: fresh(insert) });
  if (copy) {
    if (displaced) return [{ edit: 'set_insert', timeline: timeline.id, of: to, slot: targetIndex, insert: fresh(item) }];
    return !('clip' in to) && b.inserts.length >= 6 ? [] : [add(to, targetIndex, item)];
  }
  if (same) {
    const at = displaced ? targetIndex : b.inserts.length - 1;
    if (sourceIndex === at) return [];
    const ops: Change[] = [{ edit: 'move_insert', timeline: timeline.id, of: from, from: sourceIndex, to: at }];
    if (displaced && Math.abs(sourceIndex - at) > 1) ops.push({ edit: 'move_insert', timeline: timeline.id, of: from, from: sourceIndex < at ? at - 1 : at + 1, to: sourceIndex });
    return ops;
  }
  if (!displaced) return !('clip' in to) && b.inserts.length >= 6 ? [] : [{ edit: 'move_insert', timeline: timeline.id, of: from, from: sourceIndex, to: targetIndex, to_of: to }];
  // Remove both before adding: even two full six-slot chains can exchange atomically.
  return [remove(from, sourceIndex), remove(to, targetIndex), add(from, sourceIndex, displaced), add(to, targetIndex, item)];
}
