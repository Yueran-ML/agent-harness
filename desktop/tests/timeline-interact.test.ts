import { describe, expect, it } from 'vitest';
import { selectClips, linkedClips, clipLocations, primaryTrack } from '../src/studio/timeline/selection';
import { snap, snapCandidates } from '../src/studio/timeline/snap';
import { beginDrag, clearing, dragTo, finishDrag, hitClip } from '../src/studio/timeline/interact';
import { clip, media, timeline } from './timeline-fixture';

const scene = () => {
  const t = timeline([clip('c1', 0, 100), clip('c2', 100, 100, { link: 'av', source: { kind: 'media', media: 'm1', in: { ticks: 100, scale: 25 } } }), clip('c3', 200, 100)]);
  t.tracks.push({ id: 'a1', name: 'A1', kind: 'audio', enabled: true, locked: false, transitions: [], clips: [clip('a2', 100, 100, { link: 'av' })] });
  t.tracks.push({ id: 'v2', name: 'V2', kind: 'video', enabled: true, locked: false, transitions: [], clips: [] });
  return t;
};
describe('Track 1 of the Cut page', () => {
  it('is the first video track, with the clips linked to its own', () => {
    expect(primaryTrack(scene())).toEqual({ track: 'v1', clips: new Set(['c1', 'c2', 'c3', 'a2']) });
    expect(primaryTrack({ ...scene(), tracks: [] })).toBeNull();
  });
  it('closes the gaps that a move leaves on it, the latest first, and none where nothing follows', () => {
    const t = scene();
    const magnetic = primaryTrack(t);
    const move = (ids: string[], id: string, by: number, track?: string) => {
      const dragged = dragTo(beginDrag(t, 3, ids, id, 'move', 150, true)!, 150 + by, { media: [media], magnetic, track, scale: 1 });
      return finishDrag(dragged)?.ops;
    };
    // c2 (with its sound a2) 30 frames back over c1: its last 30 frames are left, and closed.
    expect(move(['c2'], 'c2', -30)).toEqual([
      { edit: 'move', timeline: 't1', delta: -30, linked: true, clips: ['c2', 'a2'], overwrite: true },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 170, straddling: 'stay' },
    ]);
    // c3, the last, moved later into empty room: the gap it leaves would close and bring it back, so nothing is done.
    expect(move(['c3'], 'c3', 40)).toBeUndefined();
    // c1 moved later than c3: the gap at 0 closes, c2 and c3 moving back, and c1 with them.
    expect(move(['c1'], 'c1', 300)).toEqual([
      { edit: 'move', timeline: 't1', delta: 300, linked: true, clips: ['c1'], overwrite: true },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 0, straddling: 'stay' },
    ]);
    // c1 and c3 later by 150 (c1 over the end of c2): the gap at 0 and the one c3 leaves after c1 are closed, the later first.
    expect(move(['c1', 'c3'], 'c1', 150)?.slice(1)).toEqual([
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 250, straddling: 'stay' },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 0, straddling: 'stay' },
    ]);
    // Not on Track 1, or with no magnetic track, it is a move as on the Edit page.
    expect(dragTo(beginDrag(t, 3, ['c2'], 'c2', 'move', 150, true)!, 120, { media: [media], scale: 1 }).then).toBeUndefined();
  });
});
describe('clips copied with Alt', () => {
  it('copy a linked pair into a new group, clear what the copies cover from the last span, and refuse a locked track', () => {
    const t = scene();
    t.tracks[1] = { ...t.tracks[1], clips: [clip('a2', 100, 100, { link: 'av' }), clip('long', 250, 400)] } as typeof t.tracks[1];
    const dragged = dragTo(beginDrag(t, 3, ['c2'], 'c2', 'move', 150, true)!, 450, { media: [media], scale: 1, duplicate: true });
    expect(dragged.command).toBeNull();
    expect(finishDrag(dragged)?.ops).toEqual([
      // On V1 nothing lies at 400–500; on A1 the long clip is split at 500, and its first part trimmed to 400.
      { edit: 'blade', timeline: 't1', at: 500, tracks: ['a1'], linked: false },
      { edit: 'trim', timeline: 't1', clip: 'long', edge: 'out', delta: -100, ripple: false, linked: false },
      { op: 'insert_clip', timeline: 't1', track: 'v1', clip: { ...clip('c2', 400, 100, { source: { kind: 'media', media: 'm1', in: { ticks: 100, scale: 25 } } }), id: '', link: '$copy1' } },
      { op: 'insert_clip', timeline: 't1', track: 'a1', clip: { ...clip('a2', 400, 100), id: '', link: '$copy1' } },
    ]);
    // Two spans over one clip: the later is cleared first, so each edit names the clip it finds.
    expect(clearing('t1', [clip('x', 0, 100)], [[10, 20], [50, 60]], 'v1')).toEqual([
      { edit: 'blade', timeline: 't1', at: 60, tracks: ['v1'], linked: false },
      { edit: 'trim', timeline: 't1', clip: 'x', edge: 'out', delta: -10, ripple: false, linked: false },
      { edit: 'blade', timeline: 't1', at: 20, tracks: ['v1'], linked: false },
      { edit: 'trim', timeline: 't1', clip: 'x', edge: 'out', delta: -10, ripple: false, linked: false },
    ]);
    t.tracks[1] = { ...t.tracks[1], locked: true } as typeof t.tracks[1];
    const locked = dragTo(beginDrag(t, 3, ['c2'], 'c2', 'move', 150, true)!, 450, { media: [media], scale: 1, duplicate: true });
    expect(finishDrag(locked)).toBeNull();
    expect(locked.refused).toBe('copy_locked');
    // c1 on V1 and a title on V2 copied a track up: there is no V3 for the copy of the title.
    const u = scene();
    u.tracks[2] = { ...u.tracks[2], clips: [clip('t9', 0, 50)] } as typeof u.tracks[2];
    const above = dragTo(beginDrag(u, 3, ['c1', 't9'], 'c1', 'move', 50, false)!, 50, { media: [media], scale: 1, duplicate: true, track: 'v2' });
    expect(finishDrag(above)).toBeNull();
    expect(above.refused).toBe('copy_no_track');
  });
});
describe('timeline selection and snapping', () => {
  it('extends a range on one track and toggles a whole linked group', () => {
    const t = scene();
    let picked = selectClips(t, { ids: [], anchor: null }, 'c1', { linked: true });
    picked = selectClips(t, picked, 'c3', { linked: true, range: true });
    expect(new Set(picked.ids)).toEqual(new Set(['c1', 'c2', 'c3', 'a2']));
    picked = selectClips(t, picked, 'c2', { linked: true, toggle: true });
    expect(picked.ids).toEqual(['c1', 'c3']);
    expect(linkedClips(t, ['c2'], false)).toEqual(['c2']);
    expect(linkedClips(t, ['c2'], true)).toEqual(['c2', 'a2']);
  });
  it('retains the pointed clip as the first selection item when preparing a cross-track move', () => {
    const t = scene();
    const loc = clipLocations(t);
    expect(loc.get('c2')?.track.id).toBe('v1');
    const gesture = beginDrag(t, 7, ['a2', 'c1', 'c2'], 'c2', 'move', 120, true);
    expect(gesture?.ids[0]).toBe('c2');
  });
  it('snaps either edge by a screen-distance tolerance and chooses the earlier point on a tie', () => {
    expect(snap(97, [0, 20], [100, 118], 4, 8)).toEqual({ delta: 98, point: 118 });
    expect(snap(98, [0], [96, 100], 4, 8)).toEqual({ delta: 96, point: 96 });
    expect(snap(80, [0], [100], 4, 8)).toEqual({ delta: 80, point: null });
    const points = snapCandidates(scene(), ['c2', 'a2'], 150, { in: 20, out: 240 });
    expect(points).toEqual([0, 20, 100, 150, 200, 240, 300]);
  });
});

describe('timeline gestures', () => {
  it('distinguishes the shared cut, each trim edge, and the two halves of trim mode', () => {
    const t = scene();
    expect(hitClip(t, 'c2', 'select', 0.5, 20, 400, 72)).toEqual({ kind: 'roll', left: 'c1', right: 'c2' });
    expect(hitClip(t, 'c2', 'select', 5, 20, 400, 72)).toBe('trim_in');
    expect(hitClip(t, 'c2', 'select', 395, 20, 400, 72)).toBe('trim_out');
    expect(hitClip(t, 'c2', 'trim', 200, 15, 400, 72)).toBe('slip');
    expect(hitClip(t, 'c2', 'trim', 200, 60, 400, 72)).toBe('slide');
    expect(hitClip(t, 'c2', 'blade', 200, 20, 400, 72)).toBe('blade');
  });
  it('emits no transaction for a click and one revision-bound command for a move', () => {
    const t = scene();
    t.tracks.push({ id: 'au2', name: 'A2', kind: 'audio', enabled: true, locked: false, transitions: [], clips: [] });
    const gesture = beginDrag(t, 7, ['c2'], 'c2', 'move', 120, true)!;
    expect(finishDrag(dragTo(gesture, 120, { media: [media] }))).toBeNull();
    const moved = dragTo(gesture, 140, { media: [media], track: 'v2' });
    expect(moved.preview.find(p => p.id === 'c2')).toMatchObject({ start: 120, length: 100, track: 'v2' });
    // Its sound goes as many tracks up, from A1 to A2 (Linked Move Across Tracks).
    expect(moved.preview.find(p => p.id === 'a2')).toMatchObject({ start: 120, length: 100, track: 'au2' });
    expect(finishDrag(moved)).toEqual({ revision: 7, ops: [{ edit: 'move', timeline: 't1', clips: ['c2', 'a2'], delta: 20, to_track: 'v2', overwrite: true, linked: true }] });
    // Nothing stays where it goes: nothing is overwritten.
    expect(moved.covered).toEqual([]);
    expect(t.tracks[0].kind !== 'subtitle' && t.tracks[0].clips[1].start).toBe(100);
  });
  it('refuses a move to another track where a linked clip has no track of its kind to go to', () => {
    // No A2 for the sound of c2: nothing is sent, and the studio says why.
    const gesture = beginDrag(scene(), 7, ['c2'], 'c2', 'move', 120, true)!;
    const moved = dragTo(gesture, 140, { media: [media], track: 'v2' });
    expect(finishDrag(moved)).toBeNull();
    expect(moved.refused).toBe('move_no_track');
    // Unlinked, the picture goes up alone; in time alone, both stay on their tracks.
    expect(finishDrag(dragTo(beginDrag(scene(), 7, ['c2'], 'c2', 'move', 120, false)!, 140, { media: [media], track: 'v2' }))?.ops[0])
      .toMatchObject({ clips: ['c2'], to_track: 'v2' });
    expect(dragTo(gesture, 160, { media: [media] }).refused).toBeUndefined();
  });
  it('overwrites by default, shuffles in with ripple, and with vertical only changes tracks, as DaVinci Resolve drags', () => {
    const g = beginDrag(scene(), 9, ['c2'], 'c2', 'move', 120, false)!;
    const over = dragTo(g, 240, { media: [media] });
    expect(finishDrag(over)?.ops[0]).toMatchObject({ edit: 'move', delta: 120, overwrite: true });
    // The preview shows what of the third clip it would overwrite.
    expect(over.covered).toEqual([{ id: 'c3', track: 'v1', start: 220, length: 80 }]);
    // The insertion point of a shuffle insert belongs to the original timeline, and it overwrites nothing.
    const shuffled = dragTo(g, 240, { media: [media], ripple: true });
    expect(finishDrag(shuffled)?.ops[0]).toEqual({ edit: 'insert_move', timeline: 't1', clips: ['c2'], at: 220, linked: false });
    expect(shuffled.covered).toEqual([]);
    expect(dragTo(g, 240, { media: [media], ripple: true, track: 'v2' }).preview[0].track).toBe('v1');
    // Shift only changes tracks: the clip keeps its place in time, nor snaps.
    const upright = dragTo(g, 240, { media: [media], vertical: true, track: 'v2', candidates: [0, 230], scale: 4 });
    expect(upright.point).toBeNull();
    expect(finishDrag(upright)?.ops[0]).toEqual({ edit: 'move', timeline: 't1', clips: ['c2'], delta: 0, to_track: 'v2', overwrite: true, linked: false });
    expect(finishDrag(dragTo(g, 240, { media: [media], vertical: true }))).toBeNull();
  });
  it('stops an edge that is trimmed without ripple at the clip beside it, also on a linked track, and not with ripple', () => {
    const t = scene();
    // c1 ends where c2 begins, and c3 begins where c2 ends: neither edge of c2 can grow.
    const out = beginDrag(t, 1, ['c2'], 'c2', 'trim_out', 200, false)!;
    expect(dragTo(out, 230, { media: [media] })).toMatchObject({ delta: 0, clamped: true, command: null });
    const into = beginDrag(t, 1, ['c2'], 'c2', 'trim_in', 100, false)!;
    expect(dragTo(into, 80, { media: [media] })).toMatchObject({ delta: 0, clamped: true });
    expect(dragTo(into, 130, { media: [media] }).delta).toBe(30);
    // With a gap of 20 frames after it, the edge stops there.
    const gap = scene();
    if (gap.tracks[0].kind !== 'subtitle') gap.tracks[0].clips[2] = clip('c3', 220, 100);
    expect(dragTo(beginDrag(gap, 1, ['c2'], 'c2', 'trim_out', 200, false)!, 260, { media: [media] }).delta).toBe(20);
    // The sound linked to c2 has a neighbour 10 frames after it: the linked trim stops at the nearer.
    const linked = scene();
    const a1 = linked.tracks[1];
    if (a1.kind !== 'subtitle') a1.clips.push(clip('a3', 210, 50));
    if (linked.tracks[0].kind !== 'subtitle') linked.tracks[0].clips.pop();
    expect(dragTo(beginDrag(linked, 1, ['c2'], 'c2', 'trim_out', 200, true)!, 260, { media: [media] }).delta).toBe(10);
    // A ripple trim moves what follows instead.
    expect(dragTo(beginDrag(t, 1, ['c2'], 'c2', 'trim_out', 200, false, true)!, 230, { media: [media] }).delta).toBe(30);
  });
  it('clamps rough trim previews to source handles and at least one frame, without editing the project', () => {
    const t = scene();
    const short = { ...media, facts: { ...media.facts, duration: { ticks: 210, scale: 25 } } };
    // The clip after c2 is taken away, so that only the source limits its out edge.
    if (t.tracks[0].kind !== 'subtitle') t.tracks[0].clips.pop();
    const g = beginDrag(t, 1, ['c2'], 'c2', 'trim_out', 200, false)!;
    const moved = dragTo(g, 250, { media: [short] });
    expect(moved.delta).toBe(10);
    expect(moved.clamped).toBe(true);
    expect(finishDrag(moved)?.ops[0]).toEqual({ edit: 'trim', timeline: 't1', clip: 'c2', edge: 'out', delta: 10, ripple: false, linked: false });
    expect(dragTo(g, 0, { media: [short] }).preview[0].length).toBe(1);
  });
  it('in trim mode sends ripple trim and preserves the timeline start in its preview', () => {
    const g = beginDrag(scene(), 3, ['c2'], 'c2', 'trim_in', 100, false, true)!;
    const moved = dragTo(g, 120, { media: [media] });
    expect(moved.preview[0]).toMatchObject({ start: 100, length: 80 });
    expect(finishDrag(moved)?.ops[0]).toMatchObject({ edit: 'trim', edge: 'in', ripple: true, delta: 20 });
  });
  it('emits the distinct roll, slip and slide operations, including slide of the last clip', () => {
    const t = scene();
    for (const action of [{ kind: 'roll' as const, left: 'c1', right: 'c2' }, 'slip' as const, 'slide' as const]) {
      const g = beginDrag(t, 5, ['c2'], 'c2', action, 150, false)!;
      expect(finishDrag(dragTo(g, 160, { media: [media] }))?.ops[0]).toMatchObject({ edit: typeof action === 'string' ? action : 'roll', delta: 10 });
    }
    const last = beginDrag(t, 5, ['c3'], 'c3', 'slide', 250, false)!;
    expect(finishDrag(dragTo(last, 260, { media: [media] }))?.ops[0]).toMatchObject({ edit: 'slide', clip: 'c3', delta: 10 });
  });
  it('does not begin a gesture on a locked track and clamps group movement before frame zero', () => {
    const t = scene();
    t.tracks[0].locked = true;
    expect(beginDrag(t, 1, ['c1'], 'c1', 'move', 10, true)).toBeNull();
    t.tracks[0].locked = false;
    const g = beginDrag(t, 1, ['c1', 'c2'], 'c2', 'move', 120, true)!;
    expect(dragTo(g, 0, { media: [media] }).delta).toBe(0);
  });
});
