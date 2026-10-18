/**
 * How hard the model thinks, as the page reads it from the status and from what the program
 * answers when a level is kept.
 */
import { describe, expect, it } from 'vitest';
import { effortOf, efforts, effortStoredOf, keptLevel } from '../src/app/effort';
import type { SessionStatus } from '../src/protocol';

const model = (given: Partial<SessionStatus['model']> = {}): SessionStatus['model'] => ({
  provider: 'deepseek', model: 'deepseek-flash', context_window: null, ...given,
});

describe('the levels that the status tells of', () => {
  it('are the level in use and those on offer, in their order', () => {
    expect(effortOf(model({ effort: 'max', efforts: [...efforts] }))).toEqual({ effort: 'max', efforts: [...efforts] });
    // In the order of the levels, whatever order they came in, and without one that is no level.
    expect(effortOf(model({ effort: 'ultra', efforts: ['ultra', 'loud', 'default', 'ultra'] })))
      .toEqual({ effort: 'ultra', efforts: ['default', 'ultra'] });
  });

  it('are none where the status tells of none, or of a level that is not on offer', () => {
    for (const given of [
      undefined, model(), model({ effort: 'max' }), model({ efforts: [...efforts] }),
      model({ effort: 'loud', efforts: [...efforts] }), model({ effort: 'max', efforts: ['default', 'ultra'] }),
      model({ effort: 'max', efforts: 'max' as unknown as string[] }),
    ]) expect(effortOf(given)).toBeNull();
  });
});

it('keeps every level for the next start but ultra', () => {
  expect(efforts.filter(keptLevel)).toEqual(['default', 'off', 'low', 'high', 'max']);
});

describe('what the program answers when a level is kept', () => {
  it('is that it was kept', () => {
    expect(effortStoredOf({ effort: { name: 'deepseek', effort: 'max' } })).toEqual({ kind: 'stored' });
  });

  it('is what it said instead, or nothing', () => {
    expect(effortStoredOf({ error: 'there is no service' })).toEqual({ kind: 'refused', said: 'there is no service' });
    for (const answer of [
      null, 'stored', {}, { effort: null }, { effort: 'max' }, { effort: { name: 'deepseek' } },
      { effort: { name: 7, effort: 'max' } }, { effort: { name: 'deepseek', effort: 'loud' } }, { error: 3 },
    ]) expect(effortStoredOf(answer)).toEqual({ kind: 'refused', said: '' });
  });
});
