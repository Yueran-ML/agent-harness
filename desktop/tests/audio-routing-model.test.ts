import { expect, it } from 'vitest';
import type { Bus, Timeline } from '../src/studio/engine';
import { busChoices, routingChanges, vcaOffset, vcaChoices, busUsers, outputsOf, nextBus } from '../src/studio/audio/routingModel';
import { mixerStrips } from '../src/studio/audio/mixerModel';
import { timeline } from './timeline-fixture';

const bus = (id: string, outputs: string[] = []): Bus => ({ id, name: id, color: '#777777ff', format: 'stereo', strip: { outputs: outputs.map(bus => ({ bus, on: true })), sends: [] } as any });
function mix(): Timeline {
  const t = timeline([]);
  t.tracks = [{ id: 'A1', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] }];
  t.buses = [bus('B1'), bus('B2')];
  return t;
}
it('rejects output and send cycles including stored disabled routes and compressor dependencies', () => {
  const t = mix(); t.buses![0].strip!.outputs = [{ bus: 'B2', on: false }];
  expect(busChoices(t, { bus: 'B2' }, 'outputs').find(c => c.bus.id === 'B1')?.reason).toBe('bus_cycle');
  expect(busChoices(t, { bus: 'B2' }, 'sends').find(c => c.bus.id === 'B2')?.reason).toBe('bus_cycle');
  t.buses![0].strip!.outputs = [];
  t.tracks[0] = { ...t.tracks[0], strip: { outputs: [], dynamics: { compressor: { sidechain: { source: { bus: 'B2' } } } } } } as any;
  expect(busChoices(t, { track: 'A1' }, 'outputs').find(c => c.bus.id === 'B2')?.reason).toBe('bus_cycle');
});
it('counts bus depth only through outputs and sends and preserves the six-bus boundary', () => {
  const t = mix(); t.buses = Array.from({ length: 7 }, (_, i) => bus(`B${i + 1}`, i < 5 ? [`B${i + 2}`] : []));
  expect(busChoices(t, { bus: 'B6' }, 'outputs').find(c => c.bus.id === 'B7')?.reason).toBe('bus_depth');
  expect(busChoices(t, { bus: 'B5' }, 'outputs').find(c => c.bus.id === 'B7')?.reason).toBeNull();
});
it('makes one edit per unlocked owner, keeps other routes, and gives new sends their real defaults', () => {
  const t = mix();
  const edits = routingChanges(t, [{ track: 'A1' }], 'sends', 'B2', 'add');
  expect(edits).toEqual([{ edit: 'set_track_audio', timeline: 't1', track: 'A1', follow_groups: false, patch: { strip: { sends: [{ bus: 'B2', on: true, pre: false, level_db: 0, pan: { on: true, x: 0, y: 0, rotate: 0, spread: 99, divergence: 0, lfe_db: -100, lfe_on: false, lfe_pre: false } }] } } }]);
  t.tracks[0].locked = true;
  expect(routingChanges(t, [{ track: 'A1' }], 'outputs', 'B2', 'add')).toEqual([]);
});
it('shows each VCA ancestor once, evaluates its lane and respects automation Off', () => {
  const t = mix();
  t.vcas = [{ id: 'V1', name: 'One', fader_db: -2, vcas: ['V3'] }, { id: 'V2', name: 'Two', fader_db: -3, vcas: ['V3'] }, { id: 'V3', name: 'Master', fader_db: -6, lanes: { fader_db: { points: [{ at: { ticks: 0, scale: 10 }, value: -8 }, { at: { ticks: 10, scale: 10 }, value: -4 }] } } }];
  expect(vcaOffset(t, ['V1', 'V2'], .5)).toBe(-11);
  t.vcas[2].automation_arm = 'off';
  expect(vcaOffset(t, ['V1', 'V2'], 1)).toBe(-11);
  expect(vcaChoices(t, { vca: 'V3' }).find(c => c.vca.id === 'V1')?.reason).toBe('vca_cycle');
});
it('uses RawTrack defaults only when strip is absent, and includes monitoring and sidechains in bus references', () => {
  const t = mix(); expect(busUsers(t, 'B1')).toEqual(['Voice']);
  t.tracks[0] = { ...t.tracks[0], strip: {} } as any;
  expect(outputsOf(t, mixerStrips(t)[0])).toEqual([]); expect(mixerStrips(t)[0].outputs).toEqual([]);
  expect(busUsers(t, 'B1')).toEqual([]);
  (t.tracks[0] as any).strip.track_fx = { sources: [{ bus: 'B2' }], on: false };
  t.audio = { monitor: 'B2' } as any;
  expect(busUsers(t, 'B2')).toEqual(['Voice', 'Control Room']);
});
it('duplicates bus configuration without changing incoming routes or sharing mutable draft values', () => {
  const t = mix(); t.buses![1].strip!.outputs = [{ bus: 'B1', on: true }];
  const clone = nextBus(t, t.buses!, t.buses![1]);
  expect(clone.id).toBe('B3'); expect(clone.strip?.outputs).toEqual([{ bus: 'B1', on: true }]);
  clone.strip!.outputs[0].on = false;
  expect(t.buses![1].strip!.outputs[0].on).toBe(true); expect(t.tracks[0]).not.toHaveProperty('strip');
});
