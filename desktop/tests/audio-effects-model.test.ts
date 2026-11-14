import { expect, it } from 'vitest';
import { addEffect, defaultDucker, defaultEffect, EFFECT_NUMBERS, insertDrop, insertParameters } from '../src/studio/audio/effectModel';
import type { Insert, Strip, Timeline } from '../src/studio/engine';
import { timeline } from './timeline-fixture';
const inserts = (count: number): Insert[] => Array.from({ length: count }, (_, i) => ({ id: `fx${i + 1}`, on: true, effect: { kind: 'gain', gain_db: i } }));
function fixture(count = 6): Timeline {
  return { ...timeline([]), tracks: ['a', 'b'].map(id => ({ id, name: id, kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { inserts: inserts(count) } as Strip })), buses: [{ id: 'B1', name: 'Main', color: '#ffffff', format: 'stereo', strip: { inserts: inserts(count) } as Strip }] };
}
it('matches model defaults and exposes every continuous FX setting with legal defaults', () => {
  expect(defaultDucker()).toEqual({ on: false, sources: [], amount_db: 2.7, lookahead_ms: 15, rise_ms: 10, hold_ms: 150, recovery_ms: 750 });
  for (const kind of Object.keys(EFFECT_NUMBERS) as (keyof typeof EFFECT_NUMBERS)[]) {
    const effect = defaultEffect(kind);
    for (const p of EFFECT_NUMBERS[kind]) { expect((effect as any)[p.field]).toBe(p.fallback); expect(p.fallback).toBeGreaterThanOrEqual(p.min); expect(p.fallback).toBeLessThanOrEqual(p.max); }
  }
  expect(defaultEffect('noise_reduction')).toMatchObject({ mode: 'auto_speech', profile: null, band: 'full' });
});
it('keeps the existing ID for parameter writes and enforces six inserts', () => {
  const t = fixture();
  expect(addEffect(t, { track: 'a' }, 'gain')).toEqual([]);
  const changes = insertParameters(t, { track: 'a' }, 0, { id: 'wrong', on: false, effect: { kind: 'gain', gain_db: -3 } });
  expect(changes).toMatchObject([{ edit: 'set_track_audio', patch: { strip: { inserts: [{ id: 'fx1', on: false, effect: { gain_db: -3 } }, ...inserts(6).slice(1)] } } }]);
});
it('swaps same-chain occupied slots with stable identities instead of rotating the middle', () => {
  const t = fixture();
  for (const [from, to] of [[0, 5], [5, 0], [1, 2], [4, 1]]) {
    const before = inserts(6).map(i => i.id), actual = [...before];
    const expected = [...before]; [expected[from], expected[to]] = [expected[to], expected[from]];
    for (const op of insertDrop(t, { track: 'a' }, from, { track: 'a' }, to, false) as any[]) actual.splice(op.to, 0, actual.splice(op.from, 1)[0]);
    expect(actual).toEqual(expected);
  }
});
it('swaps two full chains by removal first; Alt keeps the source and replaces with a new ID', () => {
  const t = fixture();
  const swap = insertDrop(t, { track: 'a' }, 0, { track: 'b' }, 3, false);
  expect(swap.map(op => 'edit' in op ? op.edit : op.op)).toEqual(['set_insert', 'set_insert', 'add_insert', 'add_insert']);
  expect(swap[2]).toMatchObject({ of: { track: 'a' }, index: 0, insert: { effect: { gain_db: 3 } } });
  expect((swap[2] as any).insert.id).toBeUndefined();
  expect(insertDrop(t, { track: 'a' }, 0, { track: 'b' }, 3, true)).toMatchObject([{ edit: 'set_insert', of: { track: 'b' }, slot: 3, insert: { effect: { gain_db: 0 } } }]);
  expect(insertDrop(t, { track: 'a' }, 0, { bus: 'B1' }, 1, false)).toEqual([]);
  expect(insertDrop(t, { track: 'a' }, 0, { track: 'b' }, 6, true)).toEqual([]);
});
