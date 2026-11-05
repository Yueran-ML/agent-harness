import { expect, it } from 'vitest';
import { defaultEq, defaultDynamics, eqBandKinds, eqBounds, changeBand, frequencyAt, frequencyPosition, processorWritable, processorPlayback, processorChanges, ORDERINGS, sidechainSources } from '../src/studio/audio/processorModel';
import { mixerStrips } from '../src/studio/audio/mixerModel';
import { timeline } from './timeline-fixture';

it('uses model defaults, strip zones and sample-rate cap without changing clip semantics', () => {
  const eq = defaultEq();
  expect(eq.bands.map(b => b.freq)).toEqual([80, 100, 500, 2000, 8000, 16000]);
  expect(eq.bands.map(b => b.on)).toEqual([false, true, true, true, true, false]);
  expect(eqBounds(eq.bands[4], 32000)).toEqual([2000, 14400]);
  expect(changeBand(eq, 1, { range: 'medium_high' }, 48000).bands[1].freq).toBe(500);
  expect(changeBand(eq, 1, { q: 100, gain_db: -50 }, 48000).bands[1]).toMatchObject({ q: 10.3, gain_db: -20 });
  expect(defaultEq(true).bands.map(b => b.freq)).toEqual([80, 300, 3000, 16000]);
  expect(eqBandKinds(0, 6)).not.toContain('notch');
  expect(eqBandKinds(0, 4)).toContain('notch');
  expect(frequencyAt(frequencyPosition(1234))).toBeCloseTo(1234);
});
it('builds only targeted module edits, excludes locks and active automation, preserves six exact orders', () => {
  const t = timeline([]); t.tracks = [{ id: 'a', name: 'a', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] }];
  const strip = mixerStrips(t)[0];
  expect(processorChanges(t.id, [strip], 'eq', { on: true })).toEqual([{ edit: 'set_track_audio', timeline: t.id, track: 'a', patch: { strip: { eq: { on: true } } }, follow_groups: false }]);
  strip.lanes = { 'eq.band.2.freq': { points: [] } };
  expect(processorWritable(strip, 'eq')).toBe(false);
  expect(processorWritable(strip, 'dynamics')).toBe(true);
  strip.arm = 'off'; expect(processorWritable(strip, 'eq')).toBe(true);
  strip.locked = true; expect(processorWritable(strip, 'eq')).toBe(false);
  expect(new Set(ORDERINGS.map(order => order.join(','))).size).toBe(6);
});
it('reads model lane paths for processor playback without changing the static base', () => {
  const eq = defaultEq();
  const lanes = { 'eq.1.gain_db': { points: [{ at: { ticks: 0, scale: 48000 }, value: 0 }, { at: { ticks: 48000, scale: 48000 }, value: 10 }] } };
  const strip = { locked: false, lanes } as any;
  expect(processorPlayback(eq, strip, 'eq', .5).bands[1].gain_db).toBe(5);
  expect(eq.bands[1].gain_db).toBe(0);
  strip.arm = 'off'; expect(processorPlayback(eq, strip, 'eq', .5).bands[1].gain_db).toBe(0);
});
it('uses actual dynamics defaults and excludes self and routing cycles from sidechain choices', () => {
  expect(defaultDynamics()).toMatchObject({ on: false, makeup_db: 0, expander: { mode: 'expander', threshold_db: -35, range_db: 18, ratio: 1.1 }, compressor: { mix: 0, ratio: 2, threshold_db: -15 }, limiter: { threshold_db: -21, attack_ms: .71 } });
  const t = timeline([]); t.tracks = [{ id: 'a', name: 'Audio', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] }];
  t.buses = [{ id: 'B1', name: 'Main', format: 'stereo', color: '#777777ff' }, { id: 'B2', name: 'Other', format: 'stereo', color: '#777777ff' }];
  const choices = sidechainSources(t, { track: 'a' });
  expect(choices.map(c => c.key)).not.toContain('track:a');
  expect(choices.find(c => c.key === 'bus:B1')?.disabled).toBe(true);
  expect(choices.find(c => c.key === 'bus:B2')?.disabled).toBe(false);
});
