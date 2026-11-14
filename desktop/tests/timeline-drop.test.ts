import { describe, expect, it } from 'vitest';
import { dropMedia, dropMode, materialLength } from '../src/studio/timeline/drop';
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
    expect(dropMedia(t, 5, [media], ['m1'], 'a1', { video_track: 'v1', audio_track: null }, 30)).toEqual({ revision: 5, ops: [
      { edit: 'overwrite', timeline: 't1', at: 30, source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: 'a1' },
    ] });
    // With the video target off, only the sound is placed.
    expect(dropMedia(t, 5, [media], ['m1'], 'a1', { video_track: null, audio_track: null }, 30)?.ops[0]).toMatchObject({ video_track: null, audio_track: 'a1' });
  });
  it('places only the sound with Shift, only the picture with Alt, and inserts with Ctrl+Shift, as DaVinci Resolve', () => {
    // Reference Manual 20, p. 748.
    const keys = (fields: Partial<Record<'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey', boolean>>) => dropMode({ ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...fields });
    expect(keys({})).toEqual({ insert: false, picture: true, sound: true });
    expect(keys({ shiftKey: true })).toEqual({ insert: false, picture: false, sound: true });
    expect(keys({ altKey: true })).toEqual({ insert: false, picture: true, sound: false });
    expect(keys({ ctrlKey: true, shiftKey: true })).toEqual({ insert: true, picture: true, sound: true });
    expect(keys({ ctrlKey: true })).toEqual({ insert: false, picture: true, sound: true });
    const t = timeline(); t.tracks.push({ id: 'a1', kind: 'audio', name: 'A1', clips: [], transitions: [], enabled: true, locked: false });
    const targets = { video_track: 'v1', audio_track: 'a1' };
    expect(dropMedia(t, 5, [media], ['m1'], 'v1', targets, 30, keys({ shiftKey: true }))?.ops[0]).toMatchObject({ edit: 'overwrite', video_track: null, audio_track: 'a1' });
    expect(dropMedia(t, 5, [media], ['m1'], 'a1', targets, 30, keys({ altKey: true }))?.ops[0]).toMatchObject({ edit: 'overwrite', video_track: 'v1', audio_track: null });
    expect(dropMedia(t, 5, [media], ['m1'], 'v1', targets, 30, keys({ ctrlKey: true, shiftKey: true }))?.ops[0]).toMatchObject({ edit: 'insert', video_track: 'v1', audio_track: 'a1' });
    // Only the sound, with no audio track to take it, places nothing.
    expect(dropMedia(t, 5, [media], ['m1'], 'v1', { video_track: 'v1', audio_track: null }, 30, keys({ shiftKey: true }))).toBeNull();
  });
});
