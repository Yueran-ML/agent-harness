import { expect, it } from 'vitest';
import { mixerStrips, faderReading, faderPlan, faderChanges, analysisStandardId } from '../src/studio/audio/mixerModel';
import { timeline } from './timeline-fixture';
import type { Timeline } from '../src/studio/engine';

export function song(): Timeline {
  return { ...timeline([]), tracks: [-20, 9, -99, -4].map((db, i) => ({ id: `a${i + 1}`, name: `Audio ${i + 1}`, kind: 'audio', enabled: true, locked: false,
    clips: [], transitions: [], strip: { fader_db: db } as never })), buses: [{ id: 'B1', name: 'Bus 1', color: '#777777ff', format: 'stereo' }] };
}
it('uses old omitted-strip defaults without inventing a bus or output', () => {
  const t = song(); delete (t.tracks[0] as any).strip;
  expect(mixerStrips(t)[0]).toMatchObject({ db: 0, outputs: ['Bus 1'], channels: 2 });
  expect(mixerStrips(t).at(-1)).toMatchObject({ db: 0, outputs: [] });
  t.buses![0].id = 'B9'; expect(mixerStrips(t)[0].outputs).toEqual([]);
});
it('reads actual linear automation, endpoint holding, duplicate points and Off rather than visual taper', () => {
  const t = song(); const track = t.tracks[0] as any;
  track.lanes = { fader_db: { points: [{ at: { ticks: 48000, scale: 48000 }, value: -20 }, { at: { ticks: 96000, scale: 48000 }, value: 0 }] } };
  expect(faderReading(mixerStrips(t)[0], 1.5)).toEqual({ value: -10, automated: true, hasLane: true });
  expect(faderReading(mixerStrips(t)[0], 0).value).toBe(-20);
  expect(faderReading(mixerStrips(t)[0], 3).value).toBe(0);
  track.lanes.fader_db.points.push({ at: { ticks: 96000, scale: 48000 }, value: -3 });
  expect(faderReading(mixerStrips(t)[0], 2).value).toBe(-3);
  track.automation_arm = 'off'; expect(faderReading(mixerStrips(t)[0], 1.5)).toEqual({ value: -20, automated: false, hasLane: true });
});
it('moves different starts by one delta with independent limits and no duplicate group followers', () => {
  const t = song(); t.track_groups = [{ id: 'g', name: 'Group', members: ['a1', 'a2', 'a3'] }];
  const plan = faderPlan(t, 'track:a1', ['track:a1', 'track:a2'], { altKey: true });
  const ops = faderChanges(t.id, plan, 15) as any[];
  expect(ops.map(op => [op.track, op.patch.strip.fader_db])).toEqual([['a1', -5], ['a2', 10], ['a3', -84]]);
  expect(ops.every(op => op.follow_groups === false)).toBe(true);
  expect(plan.restore).toHaveLength(3);
});
it('keeps direct group semantics, clutch and locked members, and Ctrl Alt includes buses', () => {
  const t = song(); t.track_groups = [{ id: 'g', name: 'G', members: ['a1', 'a2'] }, { id: 'h', name: 'H', members: ['a2', 'a3'] }];
  expect(faderPlan(t, 'track:a1', [], {}).members.map(s => s.key)).toEqual(['track:a1', 'track:a2']);
  expect((faderChanges(t.id, faderPlan(t, 'track:a1', [], {}), 2).at(-1) as any).follow_groups).toBe(true);
  expect(faderPlan(t, 'track:a1', [], { ctrlKey: true, shiftKey: true }).members.map(s => s.key)).toEqual(['track:a1']);
  t.tracks[1].locked = true;
  expect(faderPlan(t, 'track:a1', [], { altKey: true, ctrlKey: true }).members.map(s => s.key)).toEqual(['track:a1', 'track:a3', 'track:a4', 'bus:B1']);
});
it('maps model standard names to the actual analysis RPC table, including special spellings', () => {
  expect(['ebu_r128', 'bs1770_1', 'bs1770_4', 'agcom_219', 'disney_2_0', 'disney_5_1', 'apple_podcasts', 'aes_streaming', 'web_video'].map(analysisStandardId))
    .toEqual(['ebu-r128', 'bs1770-1', 'bs1770-4', 'agcom-219', 'disney-2.0', 'disney-5.1', 'apple-podcasts', 'aes-streaming-dialogue', 'web-video']);
});
it('never writes an automated follower through group expansion, direct batching or cancellation', () => {
  const t = song(); t.track_groups = [{ id: 'g', name: 'G', members: ['a1', 'a2', 'a3'] }];
  (t.tracks[1] as any).lanes = { fader_db: { points: [{ at: { ticks: 0, scale: 48000 }, value: -3 }] } };
  for (const modifiers of [{}, { altKey: true }, { altKey: true, ctrlKey: true }, { ctrlKey: true, shiftKey: true }]) {
    const plan = faderPlan(t, 'track:a1', ['track:a1', 'track:a2'], modifiers);
    const ops = [...faderChanges(t.id, plan, 45), ...plan.restore] as any[];
    expect(ops.some(op => op.track === 'a2')).toBe(false);
    expect(ops.every(op => op.follow_groups !== true)).toBe(true);
  }
  (t.tracks[1] as any).automation_arm = 'off';
  const off = faderPlan(t, 'track:a1', [], {});
  expect(off.members.map(member => member.key)).toContain('track:a2'); expect(off.follow).toBe(true);
});
