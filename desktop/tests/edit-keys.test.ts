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
  it('uses the marked timeline range for a three-point edit, with an exclusive out frame', () => {
    const ctx = context({ marks: { in: 50, out: 90 } });
    expect(command('F9', {}, ctx)).toMatchObject({ transaction: { ops: [{ edit: 'insert', at: 50, source: { length: 40 } }] } });
    expect(command('i')).toEqual({ kind: 'marks', marks: { in: 100, out: null } });
    expect(command('o')).toEqual({ kind: 'marks', marks: { in: null, out: 101 } });
    expect(command('x', { alt: true }, ctx)).toEqual({ kind: 'marks', marks: { in: null, out: null } });
  });
  it('sets source marks in the source timebase and clears a now-invalid opposite mark', () => {
    const m = { ...media, mark_out: { ticks: 20, scale: 25 } };
    const ctx = context({ focus: 'source', source: { media: m, frame: 30, rate: { num: 25, den: 1 } } });
    expect(command('i', {}, ctx)).toEqual({ kind: 'transaction', transaction: { revision: 4, ops: [{ op: 'set_media', id: 'm1', mark_in: { ticks: 30, scale: 25 }, mark_out: null }] } });
    expect(command('o', {}, ctx)).toMatchObject({ transaction: { ops: [{ op: 'set_media', mark_out: { ticks: 31, scale: 25 } }] } });
    expect(command('x', { alt: true }, ctx)).toMatchObject({ transaction: { ops: [{ op: 'set_media', mark_in: null, mark_out: null }] } });
  });
  it('uses shared undo and keeps destructive keys from repeating', () => {
    expect(command('Delete')).toMatchObject({ transaction: { ops: [{ edit: 'lift', clips: ['c1'], linked: true }] } });
    expect(command('Delete', { shift: true })).toMatchObject({ transaction: { ops: [{ edit: 'ripple_delete' }] } });
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
    expect(command('l', { ctrl: true, alt: true })).toMatchObject({ transaction: { ops: [{ op: 'set_track', id: 'v1', locked: true }] } });
    const t = timeline([clip('c1', 0, 100, { enabled: false })]);
    expect(command('d', {}, context({ timeline: t }))).toMatchObject({ transaction: { ops: [{ op: 'set_clip', enabled: true }] } });
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
