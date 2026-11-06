// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { EditPage } from '../src/studio/pages/EditPage';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import type { MonitorControls } from '../src/studio/useMonitor';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';

beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const node = (id: string) => document.querySelector(`[data-clip="${id}"]`)!;
function select(ids: string[]) {
  ids.forEach((id, index) => {
    const event = { button: 0, ctrlKey: index > 0, clientX: 250, clientY: 30, pointerId: 1 };
    fireEvent.pointerDown(node(id), event); fireEvent.pointerUp(node(id), event);
  });
}
function setup(page: 'Edit' | 'Fairlight' = 'Edit') {
  const selected: Timeline = timeline([clip('video')]);
  selected.tracks.push(...['source', 'one', 'two', 'locked'].map(id => ({ id: `a-${id}`, name: id, kind: 'audio' as const, enabled: true, locked: id === 'locked', clips: [clip(id)], transitions: [] })));
  const monitor = { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
  const call = vi.fn<EngineBridge['call']>(async (method, params) => ({ ok: true, result: method === 'clips.copy'
    ? { revision: 7, clipboard: { rate: selected.rate, at: params?.at, transitions: [], clips: selected.tracks.flatMap(track => track.kind === 'subtitle' ? []
      : track.clips.filter(clip => (params?.clips as string[]).includes(clip.id)).map(clip => ({ track: track.id, index: 0, kind: track.kind, clip, lanes: [] }))) } }
    : method === 'transport.load' ? monitor.state : method === 'audio.meters' ? { timeline: 't1', strips: [], loudness: {}, lost: 0 } : { revision: 8, done: [] } }));
  const bridge = { call, onEvent: () => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const props = { bridge, native: false, session: 1, revision: 7, pool: <div>Pool</div>, media: [media], timelines: [selected], selected, onSelect: vi.fn(), timelineMonitor: monitor, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const draw = (next = page) => <div data-test="studio">{next === 'Edit' ? <EditPage {...props} source={{ ...monitor, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={vi.fn()} /> : <FairlightPage {...props} />}</div>;
  const view = render(draw());
  const menu = (id: string) => fireEvent.contextMenu(node(id), { clientX: 10, clientY: 10 });
  const command = (id: string, label: string) => { menu(id); fireEvent.click(screen.getByRole('menuitem', { name: label })); };
  return { props, view, draw, call, menu, command, writes: () => call.mock.calls.filter(([method]) => method === 'transaction.apply') };
}

it.each(['Edit', 'Fairlight'] as const)('%s shares default-all Paste Attributes and submits selected sound targets once', async page => {
  const app = setup(page); await settle();
  app.menu('one'); expect((screen.getByRole('menuitem', { name: 'Paste Attributes' }) as HTMLButtonElement).disabled).toBe(true); fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  app.command('source', 'Copy Attributes'); select(['one', 'two']); app.command('one', 'Paste Attributes');
  expect(screen.getByRole('dialog', { name: 'Paste Attributes' })).toBeDefined();
  for (const label of ['Volume', 'Plugins', 'Equalizer']) expect((screen.getByRole('checkbox', { name: label }) as HTMLInputElement).checked).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(app.writes()).toEqual([['transaction.apply', { revision: 7, ops: [{ edit: 'paste_attributes', timeline: 't1', from: 'source', to: ['one', 'two'], items: ['volume', 'plugins', 'equalizer'] }] }, { expectedSession: 1 }]]);
  expect(screen.queryByRole('dialog')).toBeNull();
});
it.each(['Edit', 'Fairlight'] as const)('%s removes only Volume via the model command and Cancel writes nothing', async page => {
  const app = setup(page); await settle(); app.command('one', 'Remove Attributes');
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); expect(app.writes()).toHaveLength(0);
  app.command('one', 'Remove Attributes');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Plugins' })); fireEvent.click(screen.getByRole('checkbox', { name: 'Equalizer' }));
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(app.writes()).toEqual([['transaction.apply', { revision: 7, ops: [{ edit: 'remove_attributes', timeline: 't1', clips: ['one'], items: ['volume'] }] }, { expectedSession: 1 }]]);
});
it('keeps a copied source across Edit/Fairlight and does not replace it when VolumeLine handles Ctrl+C', async () => {
  const app = setup(); await settle(); select(['source']); fireEvent.keyDown(document.querySelector('.timeline')!, { key: 'c', ctrlKey: true }); await settle();
  app.view.rerender(app.draw('Fairlight')); await settle(); select(['one']);
  fireEvent.keyDown(screen.getByTestId('volume-line-one'), { key: 'c', ctrlKey: true });
  app.command('two', 'Paste Attributes'); fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect((app.writes()[0][1] as any).ops[0]).toMatchObject({ from: 'source', to: ['two'] });
});
it.each(['session', 'timeline', 'source'] as const)('invalidates a copied source after %s changes, including same-ID restoration', async identity => {
  const app = setup(); await settle(); app.command('source', 'Copy Attributes');
  const original = app.props.selected;
  if (identity === 'session') app.props.session = 2;
  else app.props.selected = identity === 'timeline' ? { ...original, id: 'other' } : { ...original, tracks: original.tracks.filter(track => track.id !== 'a-source') };
  app.view.rerender(app.draw()); await settle();
  app.props.session = 1; app.props.selected = original; app.view.rerender(app.draw()); await settle();
  app.menu('one'); expect((screen.getByRole('menuitem', { name: 'Paste Attributes' }) as HTMLButtonElement).disabled).toBe(true);
  expect(app.writes()).toHaveLength(0);
});
it('prevents empty/stale submissions, disables locked targets and surfaces a real engine refusal', async () => {
  const app = setup(); await settle(); app.menu('locked'); expect((screen.getByRole('menuitem', { name: 'Remove Attributes' }) as HTMLButtonElement).disabled).toBe(true); fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  app.command('one', 'Remove Attributes');
  for (const label of ['Volume', 'Plugins', 'Equalizer']) fireEvent.click(screen.getByRole('checkbox', { name: label }));
  expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Volume' })); app.props.revision = 8; app.view.rerender(app.draw());
  expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true); expect(screen.getByRole('alert').textContent).toContain('timeline changed');
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); app.command('one', 'Remove Attributes');
  app.call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32001, message: 'revision changed', data: null } }));
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle(); expect(screen.getByRole('alert').textContent).toContain('revision changed'); expect(app.writes()).toHaveLength(1);
});
