import { describe, expect, it } from 'vitest';
import { dropMedia, materialLength } from '../src/studio/timeline/drop';
import { media, timeline } from './timeline-fixture';

describe('media placement lengths', () => {
  it('matches whole source ticks at fractional rates and never adds an end frame beyond the media', () => {
    const item = { ...media, facts: { ...media.facts, duration: { ticks: 9001, scale: 3000 } } };
    expect(materialLength(item, { num: 24000, den: 1001 })).toBe(71);
    expect(materialLength({ ...item, facts: { ...item.facts, duration: { ticks: 1, scale: 1 } } }, { num: 25, den: 1 })).toBe(25);
    expect(materialLength({ ...item, facts: { ...item.facts, duration: { ticks: 2, scale: 1 } }, mark_out: { ticks: 1, scale: 1 } }, { num: 25, den: 1 })).toBe(49);
  });
  it('uses a five-second default for stills and puts the picture on the targeted video track when dropped on audio', () => {
    const still = { ...media, facts: { ...media.facts, video: { ...media.facts.video!, still: true } } };
    expect(materialLength(still, { num: 24000, den: 1001 })).toBe(119);
    const t = timeline(); t.tracks.push({ id: 'a1', kind: 'audio', name: 'A1', clips: [], transitions: [], enabled: true, locked: false });
    expect(dropMedia(t, 5, [media], ['m1'], 'a1', { video_track: 'v1', audio_track: null }, 30, false)).toEqual({ revision: 5, ops: [
      { edit: 'overwrite', timeline: 't1', at: 30, source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: 'a1' },
    ] });
    // With the video target off, only the sound is placed.
    expect(dropMedia(t, 5, [media], ['m1'], 'a1', { video_track: null, audio_track: null }, 30, false)?.ops[0]).toMatchObject({ video_track: null, audio_track: 'a1' });
  });
});
