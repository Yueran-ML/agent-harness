import type { AudioPresetPayload, Effect } from '../engine';
import { defaultDynamics, defaultEq } from './processorModel';
import { defaultEffect } from './effectModel';
export function presetKind(payload: AudioPresetPayload) { return payload.kind === 'effect' ? payload.settings.kind : payload.kind; }
export function cleanAudioPreset(payload: AudioPresetPayload): AudioPresetPayload {
  const copy = structuredClone(payload);
  if (copy.kind === 'dynamics') copy.settings.compressor.sidechain = { on: false, listen: false, source: null };
  if (copy.kind === 'effect' && copy.settings.kind === 'limiter') copy.settings.sidechain = { on: false, listen: false, source: null };
  return copy;
}
export function factoryAudioPresets(payload: AudioPresetPayload): { id: string; name: string; payload: AudioPresetPayload }[] {
  let presets: { name: string; payload: AudioPresetPayload }[];
  if (payload.kind === 'eq' || payload.kind === 'clip_eq') {
    const clip = payload.kind === 'clip_eq', warm = defaultEq(clip), clear = defaultEq(clip);
    warm.on = true; warm.bands[0].on = true; warm.bands[clip ? 1 : 2] = { ...warm.bands[clip ? 1 : 2], freq: 300, gain_db: -3 };
    clear.on = true; clear.bands[clip ? 2 : 4] = { ...clear.bands[clip ? 2 : 4], kind: 'high_shelf', freq: 8000, gain_db: 3 };
    presets = [{ name: 'Default', payload: { kind: payload.kind, settings: defaultEq(clip) } }, { name: 'Vocal warmth cut', payload: { kind: payload.kind, settings: warm } }, { name: 'Vocal clarity', payload: { kind: payload.kind, settings: clear } }];
  } else if (payload.kind === 'dynamics') {
    const comp = defaultDynamics(), gate = defaultDynamics(), limiter = defaultDynamics();
    comp.on = true; Object.assign(comp.compressor, { on: true, threshold_db: -20, ratio: 3, attack_ms: 5, release_ms: 120 });
    gate.on = true; Object.assign(gate.expander, { on: true, mode: 'gate', threshold_db: -45, range_db: 18 });
    limiter.on = true; Object.assign(limiter.limiter, { on: true, threshold_db: -1 });
    presets = [{ name: 'Default', payload: { kind: 'dynamics', settings: defaultDynamics() } }, { name: 'Dialogue compression', payload: { kind: 'dynamics', settings: comp } }, { name: 'Dialogue gate', payload: { kind: 'dynamics', settings: gate } }, { name: 'Bus limiting', payload: { kind: 'dynamics', settings: limiter } }];
  } else {
    presets = [{ name: 'Default', payload: { kind: 'effect', settings: defaultEffect(payload.settings.kind) } }];
    if (payload.settings.kind === 'noise_reduction') for (const [name, band] of [['De-Hiss', 'hiss'], ['De-Rumble', 'rumble'], ['De-Rumble and Hiss', 'rumble_and_hiss']] as const) presets.push({ name, payload: { kind: 'effect', settings: { ...defaultEffect('noise_reduction'), band } as Effect } });
  }
  return presets.map((preset, i) => ({ ...preset, id: `factory:${i}` }));
}
