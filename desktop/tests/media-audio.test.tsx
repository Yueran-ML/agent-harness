// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AudioAttributes } from '../src/studio/media/AudioAttributes';
import { MediaPool } from '../src/studio/MediaPool';
import { mapLayout, sourceLayout, standardLayout } from '../src/studio/media/audioLayout';
import { applyOp, type Media, type Op } from '../src/studio/engine';
import { media } from './timeline-fixture';
import { likeABrowser } from './mounted';
import { blockOf } from './stylesheet';

beforeEach(() => { likeABrowser(); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { await Promise.resolve(); });
function source(id = 'm1', stream = 3, channels = 4): Media {
  return { ...media, id, facts: { ...media.facts, audio: {
    rate: 48000, channels, codec: 'pcm', streams: [{ index: stream, channels, layout: null }],
  } } };
}
function dialog(items = [source()]) {
  const apply = vi.fn<(ops: Op[]) => Promise<boolean>>().mockResolvedValue(true);
  const close = vi.fn();
  const view = render(<AudioAttributes media={items} onApply={apply} onClose={close} />);
  return { apply, close, ...view };
}

it('splits four microphones into four named mono tracks in one transaction', async () => {
  const s = dialog();
  fireEvent.click(screen.getByRole('button', { name: 'Remove track 1' }));
  fireEvent.change(screen.getByRole('spinbutton', { name: 'Tracks' }), { target: { value: '4' } });
  fireEvent.click(screen.getByRole('button', { name: 'Add tracks' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Track name 1' }), { target: { value: 'Boom' } });
  expect(s.apply).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  await settle();
  expect(s.apply).toHaveBeenCalledTimes(1);
  const ops = s.apply.mock.calls[0][0];
  expect(ops).toHaveLength(1);
  expect(ops[0]).toMatchObject({ op: 'set_media_audio', id: 'm1', config: [
    { name: 'Boom', format: 'mono', channels: [{ source: { stream: 3, channel: 0 } }] },
    { format: 'mono', channels: [{ source: { stream: 3, channel: 1 } }] },
    { format: 'mono', channels: [{ source: { stream: 3, channel: 2 } }] },
    { format: 'mono', channels: [{ source: { stream: 3, channel: 3 } }] },
  ] });
  expect(s.close).toHaveBeenCalledTimes(1);
});

it('supports none, stereo average, active, mute and independent levels', async () => {
  const s = dialog([source('m1', 7, 2)]);
  fireEvent.change(screen.getByRole('combobox', { name: 'Track 1 Channel 1 Source' }), { target: { value: 'mix:7' } });
  fireEvent.change(screen.getByRole('combobox', { name: 'Track 1 Channel 2 Source' }), { target: { value: 'none' } });
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 1 Active' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 2 Mute' }));
  fireEvent.change(screen.getByRole('spinbutton', { name: 'Track 1 Channel 2 Level' }), { target: { value: '-8' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(s.apply.mock.calls[0][0][0]).toMatchObject({ config: [{ channels: [
    { source: [{ stream: 7, channel: 0 }, { stream: 7, channel: 1 }], enabled: false },
    { source: null, mute: true, level_db: -8 },
  ] }] });
});

it('uses the last selection for display and maps embedded positions across different stream IDs', async () => {
  const s = dialog([source('first', 1, 2), source('last', 7, 2)]);
  expect(screen.getByText('Multiple Clips')).toBeTruthy();
  expect((screen.getByRole('combobox', { name: 'Track 1 Channel 2 Source' }) as HTMLSelectElement).value).toBe('7:1');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 2 Mute' }));
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(s.apply.mock.calls[0][0]).toMatchObject([
    { id: 'first', config: [{ channels: [{ source: { stream: 1, channel: 0 } }, { source: { stream: 1, channel: 1 }, mute: true }] }] },
    { id: 'last', config: [{ channels: [{ source: { stream: 7, channel: 0 } }, { source: { stream: 7, channel: 1 }, mute: true }] }] },
  ]);
});

it('Alt-click isolates one channel across all configured tracks', async () => {
  const s = dialog();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 2 Active' }), { altKey: true });
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 2 Mute' }), { altKey: true });
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(s.apply.mock.calls[0][0][0]).toMatchObject({ config: [{ channels: [
    { enabled: false, mute: true }, { enabled: true, mute: false },
    { enabled: false, mute: true }, { enabled: false, mute: true },
  ] }] });
});

it('refuses incompatible batch sources and explains how to resolve them', () => {
  const s = dialog([source('short', 1, 1), source('long', 7, 4)]);
  expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole('status').textContent).toContain('choose another source');
  fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  expect(s.apply).not.toHaveBeenCalled();
});

it('does not infer stream zero when old media facts are still loading', () => {
  const item = source(); delete item.facts.audio!.streams;
  const s = dialog([item]);
  expect(screen.getByRole('status').textContent).toContain('unavailable');
  fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  expect(s.apply).not.toHaveBeenCalled();
});

it('refuses stale drafts and resets with an explicit null instead of an empty array', async () => {
  const item = source(), s = dialog([item]);
  s.rerender(<AudioAttributes media={[{ ...item, audio_config: [] }]} onApply={s.apply} onClose={s.close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  expect(s.apply).not.toHaveBeenCalled();
  expect(screen.getByRole('status').textContent).toContain('Reopen');
  s.unmount();
  const reset = dialog();
  fireEvent.click(screen.getByRole('button', { name: 'Restore automatic mix' })); await settle();
  expect(reset.apply).toHaveBeenCalledWith([{ op: 'set_media_audio', id: 'm1', config: null }]);
});

it('guards blank and out-of-range levels and retains a draft after a failed transaction', async () => {
  const s = dialog(); s.apply.mockResolvedValue(false);
  const field = screen.getByRole('spinbutton', { name: 'Track 1 Channel 1 Level' });
  for (const value of ['', '25', '-101']) {
    fireEvent.change(field, { target: { value } });
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
  }
  fireEvent.change(field, { target: { value: '-4' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  expect(s.close).not.toHaveBeenCalled();
  expect((field as HTMLInputElement).value).toBe('-4');
  expect(screen.getByRole('alert').textContent).toContain('draft is kept');
});

it('preserves the original objects and represents unmapped channels explicitly', () => {
  const reference = source(), target = source('m2', 9);
  const config = sourceLayout(reference);
  const before = structuredClone(config);
  const mapped = mapLayout(config, reference, target)!;
  expect(mapped[0].channels[3].source).toEqual({ stream: 9, channel: 3 });
  expect(config).toEqual(before);
  expect(reference.audio_config).toBeUndefined();
});

it('changing a standard format retains the surviving source, mute and trim', () => {
  const item = source();
  item.audio_config = sourceLayout(item);
  item.audio_config[0].channels[0] = { source: { stream: 3, channel: 2 }, mute: true, level_db: -9 };
  expect(standardLayout(item, 'mono')[0]).toMatchObject({ format: 'mono', channels: [
    { source: { stream: 3, channel: 2 }, mute: true, level_db: -9 },
  ] });
  expect(item.audio_config[0].channels).toHaveLength(4);
});

it('opens from the pool, guards session changes, and applies standard formats to the selected media', async () => {
  const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  const props = { media: [source()], bins: [], timelines: [], proxies: {}, loaded: null, bridge: { call, onEvent: () => () => {} }, session: 11,
    onImport: vi.fn(), onLoad: vi.fn(), onMakeAgain: vi.fn() };
  const view = render(<MediaPool {...props} />);
  fireEvent.click(document.querySelector('[data-media="m1"]')!);
  fireEvent.change(screen.getByRole('combobox', { name: 'Format' }), { target: { value: 'mono' } }); await settle();
  expect(call).toHaveBeenLastCalledWith('transaction.apply', { ops: [expect.objectContaining({ op: 'set_media_audio', id: 'm1', config: [expect.objectContaining({ format: 'mono' })] })] }, { expectedSession: 11 });
  fireEvent.click(screen.getByRole('button', { name: 'Clip Attributes' }));
  const dlg = screen.getByRole('dialog', { name: 'Clip Attributes' });
  expect((within(dlg).getAllByRole('option', { name: '7.1' })[0] as HTMLOptionElement).disabled).toBe(true);
  const oldApply = within(dlg).getByRole('button', { name: 'Apply' });
  view.rerender(<MediaPool {...props} session={12} />);
  expect(screen.queryByRole('dialog')).toBeNull();
  fireEvent.click(oldApply); await settle();
  expect(call).toHaveBeenCalledTimes(1);
});

it('mirrors media settings without mutating old state, clip maps or the operation payload', () => {
  const before = { schema: 5 as const, next_id: 1, bins: [], groups: [], media: [source()], timelines: [] };
  const config = sourceLayout(before.media[0]);
  const after = applyOp(before, { op: 'set_media_audio', id: 'm1', config });
  expect(before.media[0].audio_config).toBeUndefined();
  config[0].channels[0].mute = true;
  expect(after.media[0].audio_config![0].channels[0].mute).toBe(false);
  expect(applyOp(after, { op: 'set_media_audio', id: 'm1', config: null })).toEqual(before);
});

it('a reply from a closed dialog cannot close a replacement dialog', async () => {
  let finish!: (value: boolean) => void;
  const apply = vi.fn(() => new Promise<boolean>(resolve => { finish = resolve; }));
  const close = vi.fn();
  const view = render(<AudioAttributes key="old" media={[source()]} onApply={apply} onClose={close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
  view.rerender(<AudioAttributes key="new" media={[source('new')]} onApply={apply} onClose={close} />);
  await act(async () => { finish(true); await Promise.resolve(); });
  expect(close).not.toHaveBeenCalled();
  expect(screen.getByRole('dialog', { name: 'Clip Attributes' })).toBeTruthy();
});


it('retains automatic outputs across attribute edits and clears conversion when a native source is chosen', async () => {
  const item = source('m1', 7, 2);
  item.audio_config = [{ name: 'Converted', format: 'stereo', channels: [
    { source: null, remix: { channels: 6, channel: 4 }, level_db: -3 },
    { source: null, remix: { channels: 6, channel: 5 } },
  ] }];
  const s = dialog([item]);
  expect((screen.getByRole('combobox', { name: 'Track 1 Channel 1 Source' }) as HTMLSelectElement).value).toBe('remix:6:4');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Track 1 Channel 1 Mute' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Track 1 Channel 2 Source' }), { target: { value: '7:0' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply' })); await settle();
  const op = s.apply.mock.calls[0][0][0];
  if (op.op !== 'set_media_audio') throw new Error('audio edit required');
  expect(op.config?.[0].channels).toEqual([
    { source: null, remix: { channels: 6, channel: 4 }, level_db: -3, mute: true },
    { source: { stream: 7, channel: 0 } },
  ]);
});

it('keeps the list of the pool in a row of its own, over one pane that tells of the chosen medium and scrolls by itself', () => {
  const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  render(<MediaPool media={[source()]} bins={[]} timelines={[]} proxies={{}} loaded={null} bridge={{ call, onEvent: () => () => {} }} session={1}
    onImport={vi.fn()} onLoad={vi.fn()} onMakeAgain={vi.fn()} />);
  const body = document.querySelector('.pool-body')!;
  expect(body.querySelector('[data-test="pool-pane"]')).toBeNull();
  fireEvent.click(document.querySelector('[data-media="m1"]')!);
  const pane = body.querySelector<HTMLElement>('[data-test="pool-pane"]')!;
  // The list and the pane are two rows of the one body, so that the pane's height is never taken from the list's least.
  expect(pane.parentElement).toBe(body);
  expect(document.querySelector('.pool-browser')!.parentElement).toBe(body);
  expect(pane.contains(document.querySelector('[data-test="pool-list"]'))).toBe(false);
  // The facts of the medium and its audio are both in the pane, in the fields that the Inspector has.
  expect(pane.querySelector('.pool-details')).not.toBeNull();
  const audio = within(pane).getByRole('region', { name: 'Audio Configuration' });
  expect(within(audio).getByRole('combobox', { name: 'Format' }).className).toContain('field');
  expect(within(audio).getByRole('button', { name: 'Clip Attributes' }).className).toContain('btn');
  expect(within(audio).getByRole('checkbox', { name: 'Channel 1 Active' }).closest('label')!.className).toBe('check');
  expect(within(audio).getByRole('slider', { name: 'Channel Level' }).className).toBe('inspector-slider');
  // The stylesheet gives the list at least 120px and the pane at most half, with a scroll of its own.
  const css = readFileSync('src/studio/Studio.css', 'utf8');
  expect(blockOf(css, '.pool-body')).toContain('grid-template-rows: minmax(120px, 1fr) auto fit-content(50%);');
  expect(css).toMatch(/\.pool-pane \{[^}]*min-height: 0;[^}]*overflow-y: auto;/);
});
