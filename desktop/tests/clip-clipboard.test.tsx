// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { keyOf } from '../src/app/keys';
import { clipboardPlacement, type ClipClipboard } from '../src/studio/timeline/clipboard';
import { useClipClipboard } from '../src/studio/timeline/useClipClipboard';
import { Timeline } from '../src/studio/timeline/Timeline';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
const settle = () => act(async () => {
  for (let i = 0; i < 8; i++)
    await Promise.resolve();
});
const key = (value: string, extra = {}) => keyOf(new KeyboardEvent('keydown', { key: value, ctrlKey: true, ...extra }));
const snapshot = (): ClipClipboard => ({
  rate: { num: 25, den: 1 }, at: 25,
  clips: [{
    track: 'v1', index: 0, kind: 'video', clip: clip('source', 10, 40), lanes: []
  }], transitions: []
});
function setup() {
  const call = vi.fn<EngineBridge['call']>(async () => ({ ok: true, result: { revision: 17, clipboard: snapshot() } }));
  const props = {
    bridge: { call }, session: 1, revision: 17, timeline: timeline([clip('source', 10, 40)]), frame: 25,
    selected: ['source'], linked: true, marks: { in: null as number | null, out: null as number | null },
    targets: { video_track: 'v1' }, pending: false, apply: vi.fn(async () => true), problem: vi.fn(), copied: vi.fn(() => true)
  };
  const hook = renderHook(() => useClipClipboard(props));
  const press = async (value: string, extra = {}) => {
    act(() => {
      hook.result.current.keyDown(key(value, extra));
    });
    await settle();
  };
  return {
    ...hook, props, call, press
  };
}
it('keeps the copy-time playhead anchor and clipboard settings after the source is deleted', async () => {
  const app = setup();
  await app.press('c');
  expect(app.call).toHaveBeenCalledWith('clips.copy', expect.objectContaining({ at: 25, clips: ['source'], grouped: true }), { expectedSession: 1 });
  app.props.frame = 125;
  app.props.timeline = timeline([]);
  app.rerender();
  expect(app.result.current.ghosts[0].clip.start).toBe(110);
  await app.press('v');
  expect(app.props.apply).toHaveBeenCalledWith({
    revision: 17, ops: [{
      edit: 'paste_clips', timeline: 't1',
      clipboard: snapshot(), at: 125, tracks: { v1: 'v1' }
    }]
  });
  expect(app.result.current.ghosts).toHaveLength(0);
  await app.press('v');
  expect(app.props.apply).toHaveBeenCalledTimes(2);
});
it('cuts only after capture, keeps the old clipboard on refusal and forwards marked bounds', async () => {
  const app = setup();
  await app.press('c');
  app.props.marks = { in: 15, out: 30 };
  app.props.apply.mockResolvedValue(false);
  app.rerender();
  await app.press('x');
  expect(app.props.apply).toHaveBeenLastCalledWith({
    revision: 17, ops: [{
      edit: 'cut_clips', timeline: 't1', clips: ['source'], from: 15, to: 30, linked: true, grouped: true
    }]
  });
  expect(app.result.current.ghosts[0].clip.start).toBe(10);
  expect(app.props.copied).toHaveBeenCalledTimes(1);
});
it('drops an old session response and prevents repeated reads while one is pending', async () => {
  const app = setup();
  let finish!: (value: any) => void;
  app.call.mockImplementationOnce(() => new Promise(resolve => {
    finish = resolve;
  }));
  act(() => {
    app.result.current.keyDown(key('x'));
    app.result.current.keyDown(key('x'));
  });
  expect(app.call).toHaveBeenCalledTimes(1);
  app.props.session = 2;
  app.rerender();
  await act(async () => finish({ ok: true, result: { revision: 17, clipboard: snapshot() } }));
  expect(app.props.apply).not.toHaveBeenCalled();
  expect(app.result.current.ghosts).toHaveLength(0);
  await app.press('v');
  expect(app.props.apply).not.toHaveBeenCalled();
});
it('preserves a completed copy across page remounts and Esc dismisses only the ghost', async () => {
  const app = setup();
  await app.press('c');
  app.unmount();
  const other = renderHook(() => useClipClipboard(app.props));
  expect(other.result.current.ghosts).toHaveLength(1);
  act(() => {
    other.result.current.keyDown(key('Escape', { ctrlKey: false }));
  });
  expect(other.result.current.ghosts).toHaveLength(0);
  act(() => {
    other.result.current.keyDown(key('v'));
  });
  await settle();
  expect(app.props.apply).toHaveBeenCalledTimes(1);
});
it('offers four head/tail commands at the playhead without requiring In/Out marks', async () => {
  const app = setup();
  for (const [id, from, to] of [['copy-head', 10, 25], ['copy-tail', 25, 50], ['cut-head', 10, 25], ['cut-tail', 25, 50]] as const) {
    act(() => app.result.current.items('source').find(item => item.id === id)!.run());
    await settle();
    expect(app.call).toHaveBeenLastCalledWith('clips.copy', expect.objectContaining({ from, to }), { expectedSession: 1 });
  }
  expect(app.props.apply).toHaveBeenCalledTimes(2);
});
it('does not consume modified copy keys, repeated commands or IME composition', () => {
  const app = setup();
  for (const extra of [{ shiftKey: true }, { altKey: true }, { repeat: true }, { isComposing: true }]) {
    expect(app.result.current.keyDown(key('c', extra))).toBe(false);
  }
  expect(app.call).not.toHaveBeenCalled();
});
it('permits Tail at the clip start and Head at its end, and refuses empty halves', () => {
  const app = setup();
  app.props.frame = 10;
  app.rerender();
  expect(app.result.current.items('source').find(item => item.id === 'copy-tail')!.disabled).toBe(false);
  expect(app.result.current.items('source').find(item => item.id === 'copy-head')!.disabled).toBe(true);
  app.props.frame = 50;
  app.rerender();
  expect(app.result.current.items('source').find(item => item.id === 'copy-head')!.disabled).toBe(false);
  expect(app.result.current.items('source').find(item => item.id === 'copy-tail')!.disabled).toBe(true);
});
it('moves the ghost by keyboard without sending an edit and pastes on that track', async () => {
  const app = setup();
  app.props.timeline.tracks.push({ ...app.props.timeline.tracks[0], id: 'v2', kind: 'video', clips: [], transitions: [] });
  app.rerender();
  await app.press('c');
  await app.press('ArrowUp', { altKey: true });
  expect(app.result.current.ghosts[0].track.id).toBe('v2');
  expect(app.props.apply).not.toHaveBeenCalled();
  await app.press('ArrowUp', { altKey: true });
  expect(app.result.current.ghosts[0].track.id).toBe('v2');
  await app.press('v');
  expect(app.props.apply.mock.calls[0]).toEqual([{ revision: 17, ops: [expect.objectContaining({ tracks: { v1: 'v2' } })] }]);
});
it('converts anchor offsets exactly and shifts video and audio in visual track order', () => {
  const t = timeline([]);
  t.rate = { num: 50, den: 1 };
  t.tracks.push({ ...t.tracks[0], id: 'v2' });
  const saved = snapshot();
  saved.clips[0].track = 'v2';
  saved.clips[0].index = 1;
  expect(clipboardPlacement(saved, t, 100, 1)?.clips[0]).toMatchObject({ track: { id: 'v1' }, clip: { start: 70, length: 80 } });
  expect(clipboardPlacement(saved, t, 100, -1)).toBeNull();
  const sound = {
    ...t.tracks[0], kind: 'audio' as const, id: 'a1', clips: [], transitions: []
  };
  t.tracks.push(sound, { ...sound, id: 'a2' });
  saved.clips[0] = {
    ...saved.clips[0], kind: 'audio', track: 'a1', index: 0
  };
  expect(clipboardPlacement(saved, t, 100, 1)?.tracks).toEqual({ a1: 'a2' });
});
it('connects the timeline Edit menu, ghost rendering, playhead updates and guarded paste', async () => {
  const call = vi.fn<EngineBridge['call']>(async (method) => ({ ok: true, result: method === 'clips.copy' ? { revision: 17, clipboard: snapshot() } : {} }));
  const props = {
    timeline: timeline([clip('source', 10, 40)]), media: [media], bridge: {
      call, onEvent: () => () => {
      }
    }, session: 1,
    revision: 17, frame: 25, playing: false, onSeek: vi.fn()
  };
  const view = render(<Timeline {...props} />);
  fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Copy Head' }));
  await settle();
  expect(call).toHaveBeenCalledWith('clips.copy', expect.objectContaining({ clips: ['source'], from: 10, to: 25 }), { expectedSession: 1 });
  const ghost = () => document.querySelector<HTMLElement>('[data-clipboard-ghost] [data-clip]')!;
  expect(ghost().style.getPropertyValue('--start')).toBe('10');
  view.rerender(<Timeline {...props} frame={125} />);
  expect(ghost().style.getPropertyValue('--start')).toBe('110');
  fireEvent.keyDown(document.querySelector('.timeline')!, { key: 'v', ctrlKey: true });
  await settle();
  expect(call).toHaveBeenCalledWith('transaction.apply', expect.objectContaining({ ops: [expect.objectContaining({ edit: 'paste_clips', at: 125 })] }), { expectedSession: 1 });
  expect(document.querySelector('[data-clipboard-ghost]')).toBeNull();
});
