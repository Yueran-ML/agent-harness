// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TrackGroupsProvider, TrackGroups, GroupLabel } from '../src/studio/fairlight/TrackGroups';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { GROUP_CONTROLS, GROUP_DEFAULTS_KEY, allGroup, groupTracks } from '../src/studio/fairlight/groupModel';
import { faderPlan, mixerStrips } from '../src/studio/audio/mixerModel';
import { routingChanges, routingPlan } from '../src/studio/audio/routingGroups';
import { editingSelection, selectClips } from '../src/studio/timeline/selection';
import { beginDrag, dragTo } from '../src/studio/timeline/interact';
import { editCommand, type EditContext } from '../src/studio/timeline/editKeys';
import { timeline, clip } from './timeline-fixture';
import { likeABrowser } from './mounted';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
function setup() {
  const song: Timeline = { ...timeline([]), tracks: ['A1', 'A2', 'A3'].map(id => ({ id, name: id, kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { pan: { x: id === 'A1' ? 0 : .9 }, sends: [{ bus: 'B1', level_db: id === 'A1' ? 0 : 9 }] } as never })),
    track_groups: [{ id: 'TG1', name: 'Voices', members: ['A1', 'A2'], linked: [...GROUP_CONTROLS], active: true }] };
  const call = vi.fn<EngineBridge['call']>(async () => ({ ok: true, result: { revision: 8, done: [] } }));
  const select = vi.fn();
  const context = { timeline: song, bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 7, selected: ['A1'], select };
  const ui = (patch: Partial<typeof context> = {}) => <div className="studio"><TrackGroupsProvider {...context} {...patch}><TrackGroups /><GroupLabel track="A1" /><button>Outside</button></TrackGroupsProvider></div>;
  return { song, call, select, context, ui };
}
beforeEach(() => { likeABrowser(); localStorage.clear(); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); });
it('creates locally with all controls, edits members and defaults, saves once and restores focus', async () => {
  const { ui, call } = setup(); render(ui());
  const create = screen.getByRole('button', { name: 'Create Group' }); create.focus(); fireEvent.click(create);
  const dialog = within(screen.getByRole('dialog')); expect(document.activeElement).toBe(dialog.getByRole('button', { name: 'Cancel' }));
  for (const check of dialog.getAllByRole('checkbox')) expect((check as HTMLInputElement).checked).toBe(true);
  fireEvent.change(dialog.getByRole('textbox', { name: 'Name' }), { target: { value: 'My Group' } });
  const available = dialog.getByRole('listbox', { name: 'Available tracks' }) as HTMLSelectElement;
  available.options[0].selected = true; fireEvent.change(available); fireEvent.click(dialog.getByRole('button', { name: 'Add members' }));
  fireEvent.click(dialog.getByRole('checkbox', { name: 'Panning' }));
  fireEvent.click(dialog.getByRole('button', { name: 'Set as Default' })); expect(JSON.parse(localStorage.getItem(GROUP_DEFAULTS_KEY)!)).not.toContain('panning');
  expect(call).not.toHaveBeenCalled(); fireEvent.click(dialog.getByRole('button', { name: 'Save' })); await settle();
  expect(call).toHaveBeenCalledOnce(); expect(call.mock.calls[0].slice(0, 2)).toMatchObject(['transaction.apply', { revision: 7, ops: [{ op: 'add_track_group', group: { name: 'My Group', members: ['A1', 'A2'] } }] }]);
  expect(screen.queryByRole('dialog')).toBeNull(); expect(document.activeElement).toBe(create);
  fireEvent.click(create); expect((within(screen.getByRole('dialog')).getByRole('checkbox', { name: 'Panning' }) as HTMLInputElement).checked).toBe(false);
});
it('retains a rejected draft, waits for pending save and prevents writes over a changed group', async () => {
  const { ui, call, song } = setup(); let reject!: (e: Error) => void;
  call.mockImplementationOnce(() => new Promise((_, no) => { reject = no; })); const view = render(ui());
  fireEvent.click(screen.getByRole('button', { name: 'Modify Group Voices' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value: 'Draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save' })); expect((screen.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent(screen.getByRole('dialog'), new Event('cancel', { cancelable: true })); expect(screen.getByRole('dialog')).toBeTruthy();
  await act(async () => reject(new Error('revision refused'))); expect((screen.getByRole('textbox', { name: 'Name' }) as HTMLInputElement).value).toBe('Draft');
  expect(screen.getByRole('alert').textContent).toContain('revision refused');
  // Another edit (a new revision, the group as it was) leaves the window open for saving.
  view.rerender(ui({ revision: 8 })); expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false);
  // The group itself changed meanwhile: the window is stale, and nothing is written over it.
  const renamed = { ...song, track_groups: [{ ...song.track_groups![0], name: 'Dialogue' }] };
  view.rerender(ui({ revision: 9, timeline: renamed })); expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
  expect(call).toHaveBeenCalledOnce();
});
it('saves against the latest revision after edits that leave the group as it was', async () => {
  const { ui, call } = setup(); const view = render(ui());
  fireEvent.click(screen.getByRole('button', { name: 'Modify Group Voices' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value: 'Dialogue' } });
  view.rerender(ui({ revision: 8 }));
  fireEvent.click(screen.getByRole('button', { name: 'Save' })); await settle();
  expect(call.mock.calls[0].slice(0, 2)).toMatchObject(['transaction.apply', { revision: 8, ops: [{ op: 'set_track_group', id: 'TG1', patch: { name: 'Dialogue' } }] }]);
});
it('shows partial selection, Shift unions tracks, and bypass preserves per-group active flags', async () => {
  const { ui, select, call } = setup(); const view = render(ui({ selected: ['A1', 'A3'] }));
  const box = screen.getByRole('checkbox', { name: 'Selected Voices' }) as HTMLInputElement; expect(box.indeterminate).toBe(true);
  fireEvent.click(box, { shiftKey: true }); expect(select).toHaveBeenLastCalledWith(['A1', 'A3', 'A2']); expect(call).not.toHaveBeenCalled();
  view.rerender(ui({ selected: ['A1', 'A2'] })); fireEvent.click(screen.getByRole('checkbox', { name: 'Selected Voices' })); await settle();
  expect(call.mock.calls[0].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ op: 'set_track_group', patch: { active: false } }] }]);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Groups Switch' })); await settle();
  expect(call.mock.calls[1].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ op: 'set_timeline_audio', patch: { groups_bypassed: true } }] }]);
});
it('only numpad shortcuts operate groups, including virtual All without initial project writes', async () => {
  const { ui, call, song } = setup(); render(ui()); expect(call).not.toHaveBeenCalled();
  fireEvent.keyDown(document.body, { code: 'Digit1', ctrlKey: true }); expect(call).not.toHaveBeenCalled();
  fireEvent.keyDown(document.body, { code: 'Numpad1', ctrlKey: true }); await settle(); expect(call).toHaveBeenCalledOnce();
  fireEvent.keyDown(document.body, { code: 'NumpadMultiply', ctrlKey: true }); await settle();
  expect(call.mock.calls[1].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ op: 'add_track_group', group: { all_tracks: true, active: true } }] }]);
  expect(groupTracks(song, allGroup(song))).toEqual(['A1', 'A2', 'A3']);
  expect(screen.queryByRole('button', { name: 'Delete Group All Group' })).toBeNull();
});
it('a real double click renames without two toggle writes, while a single click toggles after the click window', async () => {
  vi.useFakeTimers(); const { ui, call } = setup(); render(ui()); const name = screen.getByRole('button', { name: 'Voices' });
  fireEvent.click(name, { detail: 1 }); fireEvent.click(name, { detail: 2 }); fireEvent.doubleClick(name, { detail: 2 });
  act(() => vi.advanceTimersByTime(600)); expect(call).not.toHaveBeenCalled();
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Speech' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' }); await settle();
  expect(call.mock.calls[0].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ op: 'set_track_group', patch: { name: 'Speech' } }] }]);
  fireEvent.click(screen.getByRole('button', { name: 'Voices' }), { detail: 1 }); act(() => vi.advanceTimersByTime(500)); await settle();
  expect(call.mock.calls[1].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ patch: { active: false } }] }]);
});
it('deletion asks confirmation and deletes only the group, while GroupLabel edits ordinary membership', async () => {
  const { ui, call } = setup(); render(ui()); fireEvent.click(screen.getByRole('button', { name: 'Delete Group Voices' }));
  expect(call).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); expect(call).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Delete Group Voices' })); fireEvent.click(screen.getByRole('button', { name: 'Delete' })); await settle();
  expect((call.mock.calls[0] as any)[1].ops).toEqual([{ op: 'remove_track_group', timeline: 't1', id: 'TG1' }]);
  fireEvent.click(screen.getByRole('button', { name: 'Groups A1' })); const menu = within(screen.getByRole('menu'));
  expect(menu.getByRole('menuitem', { name: 'Voices ●' })).toBeTruthy();
  fireEvent.click(menu.getByRole('menuitem', { name: 'Voices ●' })); await settle();
  expect(call.mock.calls[1].slice(0, 2)).toMatchObject(['transaction.apply', { ops: [{ patch: { members: ['A2'] } }] }]);
});
it.each(['timeline', 'session'] as const)('keeps a group deletion bound to its original %s even when revisions and IDs match', async identity => {
  const { ui, call, song } = setup();
  const view = render(ui());
  fireEvent.click(screen.getByRole('button', { name: 'Delete Group Voices' }));
  view.rerender(ui(identity === 'timeline' ? { timeline: { ...song, id: 'other' } } : { session: 2 }));
  const dialog = within(screen.getByRole('dialog'));
  const remove = dialog.getByRole('button', { name: 'Delete' }) as HTMLButtonElement;
  fireEvent.click(remove);
  await settle();
  expect(call).not.toHaveBeenCalled();
  expect(remove.disabled).toBe(true);
  expect(dialog.getByRole('alert').textContent).toContain('Reopen');
});
it.each(['timeline', 'session'] as const)('cancels a pending group-name click when its %s changes', async identity => {
  vi.useFakeTimers();
  const { ui, call, song } = setup();
  const view = render(ui());
  fireEvent.click(screen.getByRole('button', { name: 'Voices' }), { detail: 1 });
  view.rerender(ui(identity === 'timeline' ? { timeline: { ...song, id: 'other' } } : { session: 2 }));
  act(() => vi.advanceTimersByTime(500));
  await settle();
  expect(call).not.toHaveBeenCalled();
});
it.each(['timeline', 'session'] as const)('does not carry group renaming into a new %s', identity => {
  const { ui, song } = setup();
  const view = render(ui());
  fireEvent.keyDown(screen.getByRole('button', { name: 'Voices' }), { key: 'F2' });
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Old draft' } });
  view.rerender(ui(identity === 'timeline' ? { timeline: { ...song, id: 'other' } } : { session: 2 }));
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(screen.getByRole('button', { name: 'Voices' })).toBeTruthy();
});
it('restores captured Pan/Send group members before each model-relative preview and honors Clutch/bypass', () => {
  const { song } = setup(), source = mixerStrips(song)[0];
  for (const [bus, field, value] of [[undefined, 'x', .5], ['B1', 'level_db', 5]] as const) {
    const plan = routingPlan(song, source, bus, [field], false); expect(plan.members.map(s => s.id)).toEqual(['A1', 'A2']);
    const away = routingChanges(plan, { [field]: value }, true), back = routingChanges(plan, { [field]: 0 }, true);
    expect(away.slice(0, 2)).toEqual(plan.restore); expect(back.slice(0, 2)).toEqual(plan.restore);
    expect(away.at(-1)).toMatchObject({ edit: 'set_track_audio', follow_groups: true });
  }
  expect(faderPlan(song, source.key, [], { ctrlKey: true, shiftKey: true }, false).members.map(s => s.id)).toEqual(['A1']);
  song.audio = { groups_bypassed: true } as never; expect(routingPlan(song, source, undefined, ['x'], false).members.map(s => s.id)).toEqual(['A1']);
});
it('Editing uses seed ranges for delete/blade/move/copy while selection highlights companions and trims remain independent', () => {
  const { song } = setup(); if (song.tracks[0].kind === 'subtitle' || song.tracks[1].kind === 'subtitle') throw Error();
  song.tracks[0].clips = [clip('voice', 50, 50)]; song.tracks[1].clips = [clip('bed', 0, 200)];
  const selected = selectClips(song, { ids: [], anchor: null }, 'voice', { linked: false }); expect(selected.ids).toEqual(['voice']);
  expect(editingSelection(song, selected.ids)).toEqual(['voice', 'bed']);
  const context: EditContext = { timeline: song, selected: selected.ids, linked: false, frame: 75, marks: { in: null, out: null }, targets: {}, focus: 'timeline', source: null, revision: 7 };
  expect(editCommand({ kind: 'delete', ripple: false }, context)).toMatchObject({ transaction: { ops: [{ clips: ['voice'], grouped: true }] } });
  expect(editCommand({ kind: 'blade' }, context)).toMatchObject({ transaction: { ops: [{ tracks: ['A1'], grouped: true }] } });
  const gesture = beginDrag(song, 7, selected.ids, 'voice', 'move', 50, false)!;
  expect(dragTo(gesture, 250, { media: [] }).command).toMatchObject({ edit: 'move', grouped: true, clips: ['voice'] });
  expect(dragTo(gesture, 250, { media: [], duplicate: true }).duplicate).toEqual([{ edit: 'place_copies', timeline: 't1', grouped: true, copies: [{ clip: 'voice', track: 'A1', start: 250 }] }]);
  expect(dragTo(beginDrag(song, 7, selected.ids, 'voice', 'trim_out', 100, false)!, 95, { media: [] }).command).not.toHaveProperty('grouped');
  song.audio = { groups_bypassed: true } as never; expect(editingSelection(song, selected.ids)).toEqual(['voice']);
});
it('All remains dynamic after adding an audio track and cannot be renamed or have members removed', () => {
  const { ui, song } = setup(); const view = render(ui());
  song.tracks.push({ id: 'A4', name: 'A4', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] });
  song.tracks.push({ id: 'V1', name: 'V1', kind: 'video', enabled: true, locked: false, clips: [], transitions: [] });
  view.rerender(ui({ timeline: { ...song } })); expect(groupTracks(song, allGroup(song))).toEqual(['A1', 'A2', 'A3', 'A4']);
  fireEvent.doubleClick(screen.getByRole('button', { name: 'All Group' })); expect(screen.queryByRole('textbox')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Modify Group All Group' }));
  expect((screen.getByRole('textbox', { name: 'Name' }) as HTMLInputElement).disabled).toBe(true); expect(screen.queryByRole('listbox')).toBeNull();
});
it('Modify removes members in one saved patch and GroupLabel adds membership without changing active', async () => {
  const { ui, song, call } = setup(); song.track_groups![0].active = false; render(ui());
  fireEvent.click(screen.getByRole('button', { name: 'Modify Group Voices' }));
  const members = screen.getByRole('listbox', { name: 'Group members' }) as HTMLSelectElement;
  members.options[0].selected = true; fireEvent.change(members); fireEvent.click(screen.getByRole('button', { name: 'Remove members' }));
  fireEvent.click(screen.getByRole('button', { name: 'Save' })); await settle();
  expect((call.mock.calls[0] as any)[1].ops).toMatchObject([{ op: 'set_track_group', patch: { members: ['A2'], active: false } }]);
  cleanup(); song.track_groups![0].members = ['A2']; render(ui()); fireEvent.click(screen.getByRole('button', { name: 'Groups A1' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Voices ○' })); await settle();
  expect((call.mock.calls[1] as any)[1].ops).toEqual([{ op: 'set_track_group', timeline: 't1', id: 'TG1', patch: { members: ['A2', 'A1'] } }]);
});
it('readonly Pan followers are excluded, writable followers get relative deltas and Send pan/pre stay local', () => {
  const { song } = setup(); song.track_groups![0].members.push('A3');
  (song.tracks[1] as any).lanes = { 'pan.x': { points: [{ at: { ticks: 0, scale: 48000 }, value: .9 }] } };
  const source = mixerStrips(song)[0], plan = routingPlan(song, source, undefined, ['x'], true);
  expect(plan.follow).toBe(false); expect(plan.members.map(s => s.id)).toEqual(['A1', 'A3']);
  const ops = routingChanges(plan, { x: .5 }, true) as any[];
  expect(ops.map(op => [op.track, op.patch.strip.pan.x, op.follow_groups])).toEqual([['A1', .5, false], ['A3', 1, false]]);
  const back = routingChanges(plan, { x: 0 }, true) as any[]; expect(back[1].patch.strip.pan.x).toBe(.9);
  expect(routingPlan(song, source, 'B1', ['pre', 'pan.x'], false).members.map(s => s.id)).toEqual(['A1']);
});
it('track header adds Create Group without swallowing the existing menu, and strip controls retain their own menus', () => {
  const { context, song } = setup(), controlMenu = vi.fn();
  const bridge = { ...context.bridge, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
  render(<TrackGroupsProvider {...context}><FairlightTimeline timeline={song} bridge={bridge} session={1} revision={7} frame={0} playing={false} onSeek={() => {}} media={[]} view={defaultFairlightView()} update={() => {}} />
    <div className="mixer-strip" data-mixer-strip="track:A1"><button aria-label="Own fader menu" onContextMenu={controlMenu}>Fader</button><span className="mixer-name">Strip A1</span></div>
  </TrackGroupsProvider>);
  const header = document.querySelector('[data-track="A1"] .timeline-track-header')!;
  expect(header).toBeTruthy(); fireEvent.contextMenu(header);
  expect(screen.getByRole('menuitem', { name: 'Add subtitle track' })).toBeTruthy();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Create Group' })); expect(screen.getByRole('dialog')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  fireEvent.contextMenu(screen.getByRole('button', { name: 'Own fader menu' })); expect(controlMenu).toHaveBeenCalledOnce(); expect(screen.queryByRole('menu')).toBeNull();
  fireEvent.contextMenu(screen.getByText('Strip A1')); expect(screen.getByRole('menuitem', { name: 'Create Group' })).toBeTruthy();
});
