import { describe, expect, it } from 'vitest';
import type { Key } from '../src/app/keys';
import { editKey, editCommand, sourceTime, type EditContext } from '../src/studio/timeline/editKeys';
import { clip, media, timeline } from './timeline-fixture';

const key = (value: string, fields: Partial<Key> = {}): Key => ({ key: value, ctrl: false, meta: false, shift: false, alt: false, composing: false, repeat: false, ...fields });
const context = (fields: Partial<EditContext> = {}): EditContext => ({
  timeline: timeline(), selected: ['c1'], linked: true, frame: 100, marks: { in: null, out: null },
  targets: { video_track: 'v1', audio_track: null }, focus: 'timeline', source: { media, frame: 30, rate: { num: 25, den: 1 } }, revision: 4, ...fields,
});
const command = (value: string, fields: Partial<Key> = {}, ctx = context()) => editCommand(editKey(key(value, fields)), ctx);
describe('DaVinci editing keys', () => {
  it('maps tool, snapping and linked-selection keys without stealing modified or IME keys', () => {
    expect(editKey(key('a'))).toEqual({ kind: 'tool', tool: 'select' });
    expect(editKey(key('t'))).toEqual({ kind: 'tool', tool: 'trim' });
    expect(editKey(key('b'))).toEqual({ kind: 'tool', tool: 'blade' });
    expect(editKey(key('n'))).toEqual({ kind: 'snap' });
    expect(editKey(key('l', { ctrl: true, shift: true }))).toEqual({ kind: 'linked' });
    for (const fields of [{ composing: true }, { repeat: true }, { alt: true }, { ctrl: true }]) expect(editKey(key('a', fields))).toBeNull();
  });
  it('distinguishes undo, redo and fit so Ctrl+Shift+Z is never fit', () => {
    expect(command('z', { ctrl: true })).toEqual({ kind: 'history', method: 'history.undo' });
    expect(command('z', { meta: true, shift: true })).toEqual({ kind: 'history', method: 'history.redo' });
    expect(editKey(key('z', { shift: true }))).toEqual({ kind: 'fit' });
  });
  it('sends distinct insert/overwrite/replace/top/append commands with explicit track targets', () => {
    for (const [value, edit] of [['F9', 'insert'], ['F10', 'overwrite'], ['F11', 'replace'], ['F12', 'place_on_top']]) {
      const result = command(value);
      expect(result).toMatchObject({ kind: 'transaction', transaction: { revision: 4, ops: [{ edit, timeline: 't1', at: 100 }] } });
    }
    expect(command('F9')).toMatchObject({ transaction: { ops: [{ source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: null }] } });
    expect(command('F11')).toMatchObject({ transaction: { ops: [{ source: { kind: 'media', media: 'm1', from: { ticks: 30, scale: 25 } } }] } });
    expect(command('F12', { shift: true })).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [{ edit: 'append', timeline: 't1', source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: null }] } });
  });
  it('keeps F10 an overwrite on the Cut page, and Source Overwrite, with no timecode to sync by, does nothing', () => {
    // Reference Manual 20, p. 565: F10 overwrites on the Cut page; p. 563: no overlapping timecode, no edit.
    expect(command('F10', {}, context({ cut: true }))).toMatchObject({ transaction: { ops: [{ edit: 'overwrite', at: 100, video_track: 'v1' }] } });
    expect(command('F9', {}, context({ cut: true }))).toMatchObject({ transaction: { ops: [{ edit: 'smart_insert' }] } });
    expect(editCommand({ kind: 'place', edit: 'source_overwrite' }, context({ cut: true }))).toBeNull();
  });
  it('uses the marked timeline range for a three-point edit, with an exclusive out frame', () => {
    const ctx = context({ marks: { in: 50, out: 90 } });
    expect(command('F9', {}, ctx)).toMatchObject({ transaction: { ops: [{ edit: 'insert', at: 50, source: { length: 40 } }] } });
    expect(command('i')).toEqual({ kind: 'marks', marks: { in: 100, out: null } });
    expect(command('o')).toEqual({ kind: 'marks', marks: { in: null, out: 101 } });
    expect(command('x', { alt: true }, ctx)).toEqual({ kind: 'marks', marks: { in: null, out: null } });
  });
  it('follows the rules of three-point editing of DaVinci Resolve for every set of marks', () => {
    // The media lasts 4000 s at a scale of 25 ticks a second; the timeline is at 25 fps, a tick a frame.
    const at = (marks: { in: number | null; out: number | null }, source: Partial<typeof media> = {}, edit = 'F10') => {
      const result = command(edit, {}, context({ marks, source: { media: { ...media, ...source }, frame: 30, rate: { num: 25, den: 1 } } }));
      if (result?.kind !== 'transaction') throw new Error('No edit.');
      const op = result.transaction.ops[0] as { at: number; source: Record<string, unknown> };
      return { at: op.at, source: op.source };
    };
    const tick = (ticks: number) => ({ ticks, scale: 25 });
    // No marks: the source from its first frame (not the frame of the source monitor) at the playhead of the timeline.
    expect(at({ in: null, out: null })).toEqual({ at: 100, source: { kind: 'media', media: 'm1' } });
    // A source In and Out at the playhead: the engine takes the marks of the media.
    expect(at({ in: null, out: null }, { mark_in: tick(200), mark_out: tick(240) })).toEqual({ at: 100, source: { kind: 'media', media: 'm1' } });
    // A timeline In alone is where it goes.
    expect(at({ in: 50, out: null }, { mark_in: tick(200) })).toEqual({ at: 50, source: { kind: 'media', media: 'm1' } });
    // Timeline In and Out decide the length; the source In goes to the timeline In (all four marks: the source Out is passed over).
    expect(at({ in: 50, out: 90 })).toEqual({ at: 50, source: { kind: 'media', media: 'm1', length: 40 } });
    expect(at({ in: 50, out: 90 }, { mark_in: tick(200), mark_out: tick(400) })).toEqual({ at: 50, source: { kind: 'media', media: 'm1', length: 40 } });
    // Timeline In and Out with only a source Out: the source is backtimed from its Out.
    expect(at({ in: 50, out: 90 }, { mark_out: tick(240) })).toEqual({ at: 50, source: { kind: 'media', media: 'm1', length: 40, from: tick(200) } });
    // ... and a source Out nearer its start than the length begins at its first frame.
    expect(at({ in: 50, out: 90 }, { mark_out: tick(10) })).toMatchObject({ source: { from: tick(0), length: 40 } });
    // Only a timeline Out: backtimed, so that the source Out lands on it.
    expect(at({ in: null, out: 500 }, { mark_in: tick(200), mark_out: tick(240) })).toEqual({ at: 460, source: { kind: 'media', media: 'm1' } });
    // ... with a source In alone, its end lands on it; where it is longer than the time before, its end that fits is used from 0.
    expect(at({ in: null, out: 500 }, { mark_in: tick(99_800) })).toEqual({ at: 300, source: { kind: 'media', media: 'm1' } });
    expect(at({ in: null, out: 500 }, { mark_out: tick(240) })).toEqual({ at: 260, source: { kind: 'media', media: 'm1' } });
    expect(at({ in: null, out: 50 }, { mark_out: tick(240) })).toEqual({ at: 0, source: { kind: 'media', media: 'm1', length: 50, from: tick(190) } });
    // A still without marks lasts five seconds, backtimed too.
    const still = { facts: { ...media.facts, video: { ...media.facts.video!, still: true } } };
    expect(at({ in: null, out: 500 }, still)).toEqual({ at: 375, source: { kind: 'media', media: 'm1' } });
    // Insert and place on top follow the same rules; replace lines the source monitor's frame up with the playhead.
    expect(at({ in: null, out: 500 }, { mark_in: tick(200), mark_out: tick(240) }, 'F9')).toEqual({ at: 460, source: { kind: 'media', media: 'm1' } });
    expect(at({ in: null, out: 500 }, { mark_in: tick(200), mark_out: tick(240) }, 'F12')).toEqual({ at: 460, source: { kind: 'media', media: 'm1' } });
    expect(at({ in: null, out: null }, {}, 'F11')).toEqual({ at: 100, source: { kind: 'media', media: 'm1', from: tick(30) } });
  });
  it('backtimes in whole source ticks at fractional rates, ending at or before the source Out', () => {
    const t = { ...timeline(), rate: { num: 30000, den: 1001 } };
    const m = { ...media, facts: { ...media.facts, duration: { ticks: 48000 * 600, scale: 48000 } }, mark_out: { ticks: 48000 * 60, scale: 48000 } };
    const result = command('F10', {}, context({ timeline: t, marks: { in: 0, out: 7 }, source: { media: m, frame: 0, rate: t.rate } }));
    // 7 frames at 29.97 fps are 11211.2 ticks at 48 kHz: the clip begins 11212 ticks before the Out.
    expect(result).toMatchObject({ transaction: { ops: [{ at: 0, source: { length: 7, from: { ticks: 48000 * 60 - 11212, scale: 48000 } } }] } });
  });
  it('sets source marks in the source timebase and clears a now-invalid opposite mark', () => {
    const m = { ...media, mark_out: { ticks: 20, scale: 25 } };
    const ctx = context({ focus: 'source', source: { media: m, frame: 30, rate: { num: 25, den: 1 } } });
    expect(command('i', {}, ctx)).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [{ op: 'set_media', id: 'm1', mark_in: { ticks: 30, scale: 25 }, mark_out: null }] } });
    expect(command('o', {}, ctx)).toMatchObject({ transaction: { ops: [{ op: 'set_media', mark_out: { ticks: 31, scale: 25 } }] } });
    expect(command('x', { alt: true }, ctx)).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [{ op: 'set_media', id: 'm1', mark_out: null }] } });
    // A mark where it already is, or clearing marks that there are not, is no edit: the engine would refuse it.
    const marked = { ...media, mark_in: { ticks: 30, scale: 25 } };
    expect(command('i', {}, context({ focus: 'source', source: { media: marked, frame: 30, rate: { num: 25, den: 1 } } }))).toBeNull();
    expect(command('x', { alt: true }, context({ focus: 'source', source: { media, frame: 30, rate: { num: 25, den: 1 } } }))).toBeNull();
  });
  it('uses shared undo and keeps destructive keys from repeating', () => {
    // Backspace lifts; Delete, and Shift with either, ripple deletes, as DaVinci Resolve has them.
    expect(command('Backspace')).toMatchObject({ transaction: { ops: [{ edit: 'lift', clips: ['c1'], linked: true }] } });
    expect(command('Delete')).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete', clips: ['c1'], linked: true }] } });
    expect(command('Delete', { shift: true })).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete' }] } });
    expect(command('Backspace', { shift: true })).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete' }] } });
    expect(command('Delete', {}, context({ marks: { in: 20, out: 80 } }))).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete_range', from: 20, to: 80 }] } });
    expect(command('Delete', { shift: true }, context({ marks: { in: 20, out: 80 } }))).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete_range', from: 20, to: 80 }] } });
    expect(command('Delete', { repeat: true })).toBeNull();
    expect(command('Delete', {}, context({ selected: [] }))).toBeNull();
  });
  it('blades target tracks, finds previous/next cuts and asks for the speed dialog', () => {
    expect(command('b', { ctrl: true })).toMatchObject({ transaction: { ops: [{ edit: 'blade', at: 100, tracks: ['v1'], linked: true }] } });
    expect(command('\\', { ctrl: true })).toEqual(command('b', { ctrl: true }));
    expect(command('ArrowUp', {}, context({ frame: 110 }))).toEqual({ kind: 'seek', monitor: 'timeline', frame: 100 });
    expect(command('ArrowDown', {}, context({ frame: 10 }))).toEqual({ kind: 'seek', monitor: 'timeline', frame: 100 });
    expect(command('r')).toEqual({ kind: 'speed', clips: ['c1'] });
  });
  it('adds one default transition per selected cut and toggles selected clips and target locks', () => {
    expect(command('t', { ctrl: true })).toMatchObject({ transaction: { ops: [{ edit: 'add_transition', left: 'c1', right: 'c2', length: 25, kind: 'dissolve', align: 'center' }] } });
    expect(command('d')).toMatchObject({ transaction: { ops: [{ op: 'set_clip', id: 'c1', enabled: false }] } });
    const t = timeline([clip('c1', 0, 100, { enabled: false })]);
    expect(command('d', {}, context({ timeline: t }))).toMatchObject({ transaction: { ops: [{ op: 'set_clip', enabled: true }] } });
  });
  it('links or unlinks the clips chosen with Ctrl+Alt+L, and locks tracks by number with Alt+Shift, as DaVinci Resolve', () => {
    const t = timeline([clip('c1', 0, 100, { link: 'g1' }), clip('c2', 100, 100), clip('c3', 200, 100)]);
    t.tracks.push({ id: 'a1', kind: 'audio', name: 'A1', enabled: true, locked: false, clips: [clip('a1c', 0, 100, { link: 'g1' })], transitions: [] });
    t.tracks.push({ id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: true, clips: [], transitions: [] });
    // Reference Manual 20, pp. 771, 1097: Linked Clips (Option-Command-L) unlinks a linked clip, and links clips that are not.
    expect(command('l', { ctrl: true, alt: true }, context({ timeline: t, selected: ['c1'] }))).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [
      { op: 'set_clip', timeline: 't1', id: 'c1', link: null }, { op: 'set_clip', timeline: 't1', id: 'a1c', link: null },
    ] } });
    expect(command('l', { ctrl: true, alt: true }, context({ timeline: t, selected: ['c2', 'c3'] }))).toMatchObject({ transaction: { ops: [
      { op: 'set_clip', id: 'c2', link: '$link' }, { op: 'set_clip', id: 'c3', link: '$link' },
    ] } });
    expect(command('l', { ctrl: true, alt: true }, context({ timeline: t, selected: ['c1', 'c2'] }))).toMatchObject({ transaction: { ops: [
      { id: 'c1', link: '$link' }, { id: 'c2', link: '$link' }, { id: 'a1c', link: '$link' },
    ] } });
    expect(command('l', { ctrl: true, alt: true }, context({ timeline: t, selected: ['c2'] }))).toBeNull();
    // P. 762: Option-Shift-1 to 8 lock V1 to V8, Option-Shift-9 all video tracks, Option-Shift-F1 to F8 A1 to A8.
    // A digit typed with Shift is another sign: it is known by where it lies.
    expect(command('!', { alt: true, shift: true, code: 'Digit1' }, context({ timeline: t }))).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [{ op: 'set_track', timeline: 't1', id: 'v1', locked: true }] } });
    expect(command('2', { alt: true, shift: true }, context({ timeline: t }))).toMatchObject({ transaction: { ops: [{ op: 'set_track', id: 'v2', locked: false }] } });
    expect(command('(', { alt: true, shift: true, code: 'Digit9' }, context({ timeline: t }))).toMatchObject({ transaction: { ops: [
      { id: 'v1', locked: true }, { id: 'v2', locked: true },
    ] } });
    expect(command('F1', { alt: true, shift: true }, context({ timeline: t }))).toMatchObject({ transaction: { ops: [{ op: 'set_track', id: 'a1', locked: true }] } });
    expect(command('#', { alt: true, shift: true, code: 'Digit3' }, context({ timeline: t }))).toBeNull();
    expect(command('F9', { alt: true, shift: true })).toBeNull();
  });
  it('does nothing when a timeline or source needed by the command is absent', () => {
    expect(command('F9', {}, context({ timeline: null }))).toBeNull();
    expect(command('F9', {}, context({ source: null }))).toBeNull();
    expect(command('i', {}, context({ focus: 'source', source: null }))).toBeNull();
  });
  it('converts fractional rates to integer source ticks without a floating-point boundary error', () => {
    expect(sourceTime(12345, { num: 24000, den: 1001 }, 48000)).toEqual({ ticks: 24714690, scale: 48000 });
    expect(sourceTime(7, { num: 25, den: 1 }, 15360)).toEqual({ ticks: 4300, scale: 15360 });
  });
});
