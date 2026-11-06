// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { EditPage } from '../src/studio/pages/EditPage';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import { useChannelSourceMenu } from '../src/studio/inspector/ChannelSourceMenu';
import { ColorMenu, type MenuItem } from '../src/studio/color/ColorMenu';
import type { EngineBridge } from '../src/bridge';
import type { MonitorControls } from '../src/studio/useMonitor';
import type { Timeline } from '../src/studio/engine';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
const source = { ...media, facts: { ...media.facts, audio: { rate: 48000, channels: 2, codec: 'pcm',
  streams: [{ index: 3, channels: 2, layout: 'stereo' }, { index: 5, channels: 1, layout: 'mono' }],
} } };
function model(): Timeline {
  const value = timeline([]);
  value.tracks.push({ id: 'a1', name: 'Audio', kind: 'audio', format: 'stereo', enabled: true, locked: false,
    clips: [clip('sound', 0, 100, { channels: [
      { source: { stream: 3, channel: 0 }, level_db: -6, enabled: true, mute: true },
      { source: [{ stream: 3, channel: 0 }, { stream: 3, channel: 1 }], level_db: -3, enabled: false },
    ] }), clip('other', 120, 100)], transitions: [] });
  return value;
}
function monitor(): MonitorControls {
  return { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
}
function connection() {
  const call = vi.fn<EngineBridge['call']>(async method => ({ ok: true, result: method === 'transport.load' ? monitor().state
    : method === 'audio.meters' ? { timeline: 't1', strips: [], loudness: {}, lost: 0 } : method === 'media.waveform' ? { pending: true } : { revision: 8, done: [] } }));
  return { call, onEvent: () => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
}
function page(which: 'Edit' | 'Fairlight') {
  const selected = model(), bridge = connection(), transport = monitor();
  const props = { bridge, native: false, session: 1, revision: 7, pool: null, media: [source], timelines: [selected], selected,
    onSelect: vi.fn(), timelineMonitor: transport, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  render(<div data-test="studio">{which === 'Edit' ? <EditPage {...props} source={{ ...transport, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={vi.fn()} /> : <FairlightPage {...props} />}</div>);
  return { bridge, selected, writes: () => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply') };
}

it.each(['Edit', 'Fairlight'] as const)('%s keeps normal clip commands and changes just the requested explicit channel', async which => {
  const s = page(which); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound"]')!, { clientX: 10, clientY: 10 });
  expect(screen.getByRole('menuitem', { name: 'Copy Attributes' })).toBeTruthy();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Audio Channel' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Channel 2' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Stream 5 · Channel 1' })); await settle();
  expect(s.writes()).toEqual([['transaction.apply', { revision: 7, ops: [{ op: 'set_clip', timeline: 't1', id: 'sound', channels: [
    { source: { stream: 3, channel: 0 }, level_db: -6, enabled: true, mute: true },
    { source: { stream: 5, channel: 0 }, level_db: -3, enabled: false },
  ] }] }, { expectedSession: 1 }]]);
  expect(screen.queryByRole('menu')).toBeNull();
});

it('Fairlight Ctrl-right-click picks the clicked lane and opens only its sources', async () => {
  const s = page('Fairlight'); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound"] [data-channel="0"]')!, { ctrlKey: true, clientX: 10, clientY: 10 });
  expect(screen.queryByRole('menuitem', { name: 'Copy Attributes' })).toBeNull();
  expect(screen.queryByRole('menuitem', { name: 'Audio Channel' })).toBeNull();
  expect(screen.getByRole('menuitem', { name: 'Stream 3 · Channel 1' }).getAttribute('data-checked')).toBe('true');
  fireEvent.click(screen.getByRole('menuitem', { name: 'None' })); await settle();
  const params = s.writes()[0][1] as { ops: { channels: { source: unknown; level_db: number; mute?: boolean }[] }[] };
  expect(params.ops[0].channels[0]).toEqual({ source: null, level_db: -6, enabled: true, mute: true });
  expect(params.ops[0].channels[1].source).toEqual([{ stream: 3, channel: 0 }, { stream: 3, channel: 1 }]);
});

it('Edit Ctrl-right-click suppresses clipboard commands while retaining the channel chooser', async () => {
  page('Edit'); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound"]')!, { ctrlKey: true, clientX: 10, clientY: 10 });
  expect(screen.getAllByRole('menuitem')).toHaveLength(1);
  expect(screen.getByRole('menuitem', { name: 'Audio Channel' })).toBeTruthy();
});

function hook() {
  const bridge = connection(), selected = model();
  let value!: ReturnType<typeof useChannelSourceMenu>;
  function Host({ revision = 7, session = 1, timeline = selected }: { revision?: number; session?: number; timeline?: Timeline }) {
    value = useChannelSourceMenu({ bridge, session, revision, timeline, media: [source] });
    return <div>{value.problem && <p role="alert">{value.problem}</p>}</div>;
  }
  const view = render(<Host />);
  const choices = () => value.items({ clip: 'sound', x: 0, y: 0, channel: 0 });
  return { bridge, selected, Host, view, choices, value: () => value };
}

it('refuses old revisions, sessions and removed controls without overwriting current clip maps', async () => {
  const s = hook(), old = s.choices()[0];
  s.view.rerender(<s.Host revision={8} />);
  act(() => old.run?.()); await settle();
  expect(s.bridge.call).not.toHaveBeenCalled();
  expect(screen.getByRole('alert').textContent).toContain('Reopen');
  const previousSession = s.choices()[0]; s.view.rerender(<s.Host revision={8} session={2} />);
  act(() => previousSession.run?.()); await settle(); expect(s.bridge.call).not.toHaveBeenCalled();
  const unmounted = s.choices()[0]; s.view.unmount(); unmounted.run?.();
  expect(s.bridge.call).not.toHaveBeenCalled();
});

it('does not rewrite unchanged sources, including automatic outputs, or locked clips', async () => {
  const s = hook();
  act(() => s.choices().find(item => item.checked)?.run?.()); await settle();
  expect(s.bridge.call).not.toHaveBeenCalled();
  const automatic = s.value().items({ clip: 'other', x: 0, y: 0, channel: 0 });
  act(() => automatic.find(item => item.checked)?.run?.()); await settle();
  expect(s.bridge.call).not.toHaveBeenCalled();
  const locked = structuredClone(s.selected); locked.tracks.at(-1)!.locked = true;
  s.view.rerender(<s.Host timeline={locked} />);
  expect(s.choices().every(item => item.disabled)).toBe(true);
  act(() => s.choices()[0].run?.()); await settle(); expect(s.bridge.call).not.toHaveBeenCalled();
});

it('serializes a pending source change and reports a failed transaction', async () => {
  const s = hook(); let reject!: (reason: Error) => void;
  s.bridge.call.mockImplementation(() => new Promise((_resolve, no) => { reject = no; }));
  const item = s.choices()[0]; act(() => { item.run?.(); item.run?.(); });
  expect(s.bridge.call).toHaveBeenCalledTimes(1);
  reject(new Error('refused')); await settle();
  expect(screen.getByRole('alert').textContent).toContain('refused');
  s.bridge.call.mockResolvedValue({ ok: true, result: { revision: 8, done: [] } });
  act(() => s.choices()[0].run?.()); await settle(); expect(s.bridge.call).toHaveBeenCalledTimes(2);
});

it('places nested source menus outside scrolling parents and flips them at viewport edges', async () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    return (this.getAttribute('role') === 'menu'
      ? { x: 900, y: 700, left: 900, top: 700, right: 1100, bottom: 1100, width: 200, height: 400 }
      : { x: 900, y: 700, left: 900, top: 700, right: 1000, bottom: 725, width: 100, height: 25 }) as DOMRect;
  });
  const run = vi.fn(), close = vi.fn();
  const items: MenuItem[] = [{ id: 'channels', label: 'Audio Channel', items: [{ id: 'one', label: 'Channel 1', items: [{ id: 'pick', label: 'Source', run }] }] }];
  render(<ColorMenu menu={{ x: 900, y: 700, label: 'Clip', items }} onClose={close} />);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Audio Channel' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Channel 1' }));
  const submenu = screen.getByRole('menu', { name: 'Channel 1' });
  expect(submenu.parentElement?.className).toBe('color-menu-layer');
  expect(submenu.style.left).toBe('700px');
  expect(submenu.style.top).toBe(`${window.innerHeight - 400}px`);
  fireEvent.keyDown(submenu, { key: 'ArrowLeft' });
  expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Channel 1' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Channel 1' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Source' }));
  expect(run).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledOnce();
});


it('changes one automatic output without replacing the other output with native full gain', async () => {
  const s = hook();
  act(() => s.value().items({ clip: 'other', x: 0, y: 0, channel: 1 }).find(item => item.label === 'Stream 5 · Channel 1')?.run?.()); await settle();
  expect(s.bridge.call).toHaveBeenCalledExactlyOnceWith('transaction.apply', { revision: 7, ops: [{ op: 'set_clip', timeline: 't1', id: 'other', channels: [
    { source: null, remix: { channels: 2, channel: 0 }, enabled: true, mute: false, level_db: 0 },
    { source: { stream: 5, channel: 0 }, enabled: true, mute: false, level_db: 0 },
  ] }] }, { expectedSession: 1 });
});

it('keeps Film output order when choosing one source and restores automatic sources without clearing trim or mute', async () => {
  const s = hook(), film = structuredClone(s.selected), track = film.tracks.at(-1)!;
  if (track.kind !== 'audio') throw new Error('audio track required');
  track.format = '5.1'; track.order = 'film';
  s.view.rerender(<s.Host timeline={film} />);
  act(() => s.value().items({ clip: 'other', x: 0, y: 0, channel: 0 })[0].run?.()); await settle();
  const params = s.bridge.call.mock.calls[0][1] as any;
  expect(params.ops[0].channels.map((map: any) => map.remix?.channel ?? null)).toEqual([null, 2, 1, 4, 5, 3]);
  act(() => s.choices().find(item => item.label === 'Automatic mix')?.run?.()); await settle();
  const restore = s.bridge.call.mock.calls[1][1] as any;
  expect(restore.ops[0].channels[0]).toEqual({ source: null, remix: { channels: 6, channel: 0 }, level_db: -6, enabled: true, mute: true });
});
