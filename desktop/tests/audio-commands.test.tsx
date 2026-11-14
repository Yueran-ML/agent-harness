// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { EditPage } from '../src/studio/pages/EditPage';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import { editCommand, editKey, type EditContext } from '../src/studio/timeline/editKeys';
import { TransitionPanel } from '../src/studio/inspector/TransitionPanel';
import type { Commit } from '../src/studio/inspector/useCommit';
import type { EngineBridge } from '../src/bridge';
import type { EngineEvent, Timeline } from '../src/studio/engine';
import type { MonitorControls } from '../src/studio/useMonitor';
import { Words } from '../src/app/locale';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { ja } from '../src/messages/ja';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function setup(page: 'Edit' | 'Fairlight' = 'Edit', words = en) {
  const selected: Timeline = timeline([clip('video')]);
  selected.tracks.push(...['sound1', 'sound2', 'locked'].map((id, i) => ({ id: `a${i}`, name: id, kind: 'audio' as const, enabled: true, locked: id === 'locked', clips: [clip(id)], transitions: [] })));
  const monitor = { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: true, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: true, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
  const call = vi.fn<EngineBridge['call']>(async method => ({ ok: true, result: method === 'transport.load' ? monitor.state : method === 'audio.meters' ? { timeline: 't1', strips: [], loudness: {}, lost: 0 } : {} }));
  const bridge = { call, onEvent: (_fn: (event: EngineEvent) => void) => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const props = { bridge, native: false, session: 1, revision: 7, pool: <div>Pool</div>, media: [media], timelines: [selected], selected, onSelect: vi.fn(), timelineMonitor: monitor, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const draw = () => <Words.Provider value={words}><div className="studio" data-test="studio">{page === 'Edit' ? <EditPage {...props} source={{ ...monitor, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={vi.fn()} /> : <FairlightPage {...props} />}</div></Words.Provider>;
  const view = render(draw());
  if (page === 'Edit') fireEvent.click(document.querySelector('[data-test="toggle-inspector"]')!);
  return { props, view, draw, call, monitor };
}
function selectClips(ids: string[]) {
  for (const [index, id] of ids.entries()) {
    const element = document.querySelector('[data-clip="' + id + '"]')!;
    fireEvent.pointerDown(element, { button: 0, ctrlKey: index > 0, clientX: 250, clientY: 30, pointerId: 1 });
    fireEvent.pointerUp(element, { button: 0, ctrlKey: index > 0, clientX: 250, clientY: 30, pointerId: 1 });
  }
}
const transactions = (app: ReturnType<typeof setup>) => app.call.mock.calls.filter(([m]) => m === 'transaction.apply').map(([, p]) => p);
const gainKey = { key: '+', code: 'Equal', ctrlKey: true, altKey: true };
function withCrossfade(app: ReturnType<typeof setup>) {
  const song = context().timeline!;
  app.props.selected = { ...song, tracks: song.tracks.map(track => track.id === 'a1' && track.kind !== 'subtitle' ? { ...track, transitions: [{ id: 'fade', left: 'sound', right: 'next', kind: 'crossfade', length: 25, align: 'center' }] } : track) };
  app.view.rerender(app.draw());
}
it.each(['mixer', 'index', 'header'] as const)('Fairlight returns from crossfade to %s track selection and can reselect the same fade', async entry => {
  const app = setup('Fairlight'); withCrossfade(app); await settle();
  if (entry === 'index') fireEvent.click(screen.getByRole('button', { name: 'Index' }));
  const selectTrack = () => {
    if (entry === 'mixer') fireEvent.click(document.querySelector('.mixer-name')!);
    else if (entry === 'index') fireEvent.click(screen.getByRole('button', { name: 'a1' }));
    else fireEvent.pointerDown(document.querySelector('[data-track="a1"] .timeline-track-name')!, { button: 0 });
  };
  // Selecting a track does not erase the user's existing clip selection.
  selectClips(['sound']); selectTrack(); await settle();
  expect(document.querySelector('[data-clip="sound"]')?.getAttribute('aria-selected')).toBe('true');
  for (let i = 0; i < 2; i++) {
    fireEvent.pointerDown(document.querySelector('[data-transition="fade"]')!, { button: 0 }); await settle();
    expect(screen.getByLabelText('Fade In Level')).toBeDefined();
    selectTrack(); await settle();
    const inspector = screen.getByRole('complementary', { name: 'Fairlight Inspector' });
    expect(within(inspector).queryByLabelText('Fade In Level')).toBeNull();
    expect(within(inspector).getByRole('button', { name: 'Track' }).getAttribute('aria-pressed')).toBe('true');
    expect(within(inspector).getByLabelText('Fader')).toBeDefined();
  }
});
it.each(['Edit', 'Fairlight'] as const)('%s disables an outstanding Use Longest retry after the transition track locks', async page => {
  const app = setup(page); withCrossfade(app); await settle();
  fireEvent.pointerDown(document.querySelector('[data-transition="fade"]')!, { button: 0 }); await settle();
  app.call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'short handles', data: { reason: 'no_handles', clip: 'sound', longest: 9 } } });
  const length = document.querySelector('[data-field="length"] input.inspector-number')!;
  fireEvent.change(length, { target: { value: '40' } }); fireEvent.keyDown(length, { key: 'Enter' }); await settle();
  expect(transactions(app)).toHaveLength(1);
  app.props.selected = { ...app.props.selected, tracks: app.props.selected.tracks.map(track => track.id === 'a1' ? { ...track, locked: true } : track) }; app.view.rerender(app.draw()); await settle();
  const retry = document.querySelector<HTMLButtonElement>('[data-test="use-longest"]')!;
  expect(retry.disabled).toBe(true); fireEvent.click(retry); await settle(); expect(transactions(app)).toHaveLength(1);
  app.props.selected = { ...app.props.selected, tracks: app.props.selected.tracks.map(track => track.id === 'a1' ? { ...track, locked: false } : track) }; app.view.rerender(app.draw()); await settle();
  expect(retry.disabled).toBe(false); fireEvent.click(retry); await settle();
  expect(transactions(app).at(-1)).toEqual({ revision: 7, ops: [{ edit: 'set_transition', timeline: 't1', id: 'fade', length: 9 }] });
});
it('a saved longest callback reads the current timeline when its edit is built', async () => {
  const song = context().timeline!; const track = song.tracks[1]; if (track.kind === 'subtitle') throw Error();
  const fade = { id: 'fade', left: 'sound', right: 'next', kind: 'crossfade' as const, length: 25, align: 'center' as const };
  track.transitions = [fade]; let current = song;
  const sent: unknown[] = []; let retry: ((frames: number) => void) | undefined;
  const commit: Commit = async (build, options) => { const ops = build(current); if (ops?.length) sent.push(ops); retry = options?.onLongest ?? retry; return true; };
  render(<TransitionPanel transition={fade} track={track} timeline={song} commit={commit} />);
  const length = document.querySelector('[data-field="length"] input.inspector-number')!;
  fireEvent.change(length, { target: { value: '40' } }); fireEvent.keyDown(length, { key: 'Enter' }); await settle();
  current = { ...song, tracks: song.tracks.map(each => each.id === track.id ? { ...each, locked: true } : each) };
  retry!(9); await settle(); expect(sent).toHaveLength(1);
  current = song; retry!(9); await settle();
  expect(sent.at(-1)).toEqual([{ edit: 'set_transition', timeline: 't1', id: 'fade', length: 9 }]);
});
it.each(['Edit', 'Fairlight'] as const)('%s sends the same gain edit from keys and Audio menu while playing', async page => {
  const app = setup(page); await settle(); selectClips(['sound1', 'sound2']);
  fireEvent.keyDown(document.querySelector('.timeline')!, gainKey); await settle();
  expect(transactions(app)).toEqual([{ revision: 7, ops: [{ edit: 'nudge_clip_gain', timeline: 't1', clips: ['sound1', 'sound2'], delta_db: 1, linked: false }] }]);
  fireEvent.click(screen.getByRole('button', { name: 'Audio' })); fireEvent.click(screen.getByRole('menuitem', { name: /Increase Audio Level 1 dB/ })); await settle();
  expect(transactions(app)).toHaveLength(2); expect(transactions(app)[1]).toEqual(transactions(app)[0]);
});
it.each(['Edit', 'Fairlight'] as const)('%s routes all four gain menu items and keys identically with keyed linked audio', async page => {
  const app = setup(page); app.props.selected = context().timeline!; app.view.rerender(app.draw()); await settle(); selectClips(['sound']);
  const before = JSON.stringify(app.props.selected);
  for (const [delta, code, ctrlKey, shiftKey, label] of [
    [1, 'Equal', true, false, 'Increase Audio Level 1 dB'], [-1, 'Minus', true, false, 'Decrease Audio Level 1 dB'],
    [3, 'Equal', false, true, 'Increase Audio Level 3 dB'], [-3, 'Minus', false, true, 'Decrease Audio Level 3 dB'],
  ] as const) {
    fireEvent.keyDown(document.querySelector('[data-test="studio"]')!, { key: 'layout-dependent', code, ctrlKey, shiftKey, altKey: true }); await settle();
    const fromKey = transactions(app).at(-1);
    expect(fromKey).toEqual({ revision: 7, ops: [{ edit: 'nudge_clip_gain', timeline: 't1', clips: ['sound'], delta_db: delta, linked: false }] });
    fireEvent.click(screen.getByRole('button', { name: 'Audio' })); fireEvent.click(screen.getByRole('menuitem', { name: label })); await settle();
    expect(transactions(app).at(-1)).toEqual(fromKey);
  }
  expect(transactions(app)).toHaveLength(8); expect(JSON.stringify(app.props.selected)).toBe(before);
});
it.each(['Edit', 'Fairlight'] as const)('%s writes nothing for no audio or a selection that becomes locked and disables its gain menu', async page => {
  const app = setup(page); await settle();
  fireEvent.keyDown(document.querySelector('.timeline')!, gainKey); selectClips(['sound1']);
  app.props.selected = { ...app.props.selected, tracks: app.props.selected.tracks.map(track => track.id === 'a0' ? { ...track, locked: true } : track) }; app.view.rerender(app.draw());
  fireEvent.keyDown(document.querySelector('.timeline')!, gainKey); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Audio' }));
  for (const label of ['Increase Audio Level 1 dB', 'Decrease Audio Level 1 dB', 'Increase Audio Level 3 dB', 'Decrease Audio Level 3 dB']) expect((screen.getByRole('menuitem', { name: label }) as HTMLButtonElement).disabled).toBe(true);
  expect(transactions(app)).toEqual([]);
});
it.each(['Edit', 'Fairlight'] as const)('%s blocks repeat, IME, text and modal gain shortcuts', async page => {
  const app = setup(page); await settle(); selectClips(['sound1']);
  for (const extra of [{ repeat: true }, { isComposing: true }, { keyCode: 229 }, { shiftKey: true }, { metaKey: true }]) fireEvent.keyDown(document.querySelector('.timeline')!, { ...gainKey, ...extra });
  const input = document.createElement('input'); document.querySelector('[data-test="studio"]')!.appendChild(input); fireEvent.keyDown(input, gainKey); input.remove();
  const dialog = document.createElement('dialog'); dialog.open = true; document.body.appendChild(dialog); fireEvent.keyDown(document.querySelector('.timeline')!, gainKey); dialog.remove();
  await settle(); expect(transactions(app)).toEqual([]);
});
it.each(['Edit', 'Fairlight'] as const)('%s opens the selected crossfade and changes each level alone', async page => {
  const app = setup(page); const sound = app.props.selected.tracks[1]; if (sound.kind === 'subtitle') throw Error();
  sound.clips = [clip('sound1', 0, 50), clip('next', 50, 50)]; sound.transitions = [{ id: 'fade', left: 'sound1', right: 'next', kind: 'crossfade', length: 25, align: 'center' }];
  app.props.selected = { ...app.props.selected, tracks: app.props.selected.tracks.map(track => ({ ...track })) }; app.view.rerender(app.draw()); await settle();
  fireEvent.pointerDown(document.querySelector('[data-transition="fade"]')!, { button: 0 }); await settle();
  const incoming = screen.getByLabelText('Fade In Level') as HTMLSelectElement; const outgoing = screen.getByLabelText('Fade Out Level') as HTMLSelectElement;
  expect(incoming.value).toBe('0'); expect(outgoing.value).toBe('0');
  fireEvent.change(incoming, { target: { value: '-3' } }); await settle(); fireEvent.change(outgoing, { target: { value: '3' } }); await settle();
  expect(transactions(app)).toEqual([{ revision: 7, ops: [{ edit: 'set_transition', timeline: 't1', id: 'fade', fade_in_level_db: -3 }] }, { revision: 7, ops: [{ edit: 'set_transition', timeline: 't1', id: 'fade', fade_out_level_db: 3 }] }]);
  app.call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'short handles', data: { reason: 'no_handles', clip: 'sound1', longest: 9 } } });
  const length = document.querySelector('[data-field="length"] input.inspector-number')!;
  fireEvent.change(length, { target: { value: '40' } }); fireEvent.keyDown(length, { key: 'Enter' }); await settle();
  expect(screen.getByRole('alert')).toBeDefined(); fireEvent.click(document.querySelector('[data-test="use-longest"]')!); await settle();
  expect(transactions(app).at(-1)).toEqual({ revision: 7, ops: [{ edit: 'set_transition', timeline: 't1', id: 'fade', length: 9 }] });
});
it('classifies gain by physical code and exact modifiers, preserving zoom and track locks', () => {
  const key = { key: 'unrelated', code: 'Equal', ctrl: true, alt: true, shift: false, meta: false, repeat: false, composing: false };
  expect(editKey(key)).toEqual({ kind: 'gain', delta: 1 });
  expect(editKey({ ...key, code: 'Minus' })).toEqual({ kind: 'gain', delta: -1 });
  expect(editKey({ ...key, ctrl: false, shift: true })).toEqual({ kind: 'gain', delta: 3 });
  expect(editKey({ ...key, ctrl: false, shift: true, code: 'Minus' })).toEqual({ kind: 'gain', delta: -3 });
  expect(editKey({ ...key, code: 'NumpadAdd', key: '+' })).toBeNull();
  expect(editKey({ ...key, alt: false, key: '=' })).toEqual({ kind: 'zoom', direction: 1 });
  expect(editKey({ ...key, ctrl: false, shift: true, code: 'Digit1', key: '!' })).toEqual({ kind: 'lock', tracks: 'video', number: 1 });
  expect(editKey({ ...key, ctrl: false, alt: false, shift: true, key: 'T' })).toEqual({ kind: 'audio-transition' });
});
function context(): EditContext {
  const song = timeline([clip('video', 0, 50, { link: 'linked' }), clip('video2', 50, 50)]);
  song.tracks.push({ id: 'a1', name: 'Audio One', kind: 'audio', enabled: true, locked: false, transitions: [], clips: [
    clip('sound', 0, 50, { link: 'linked', gain_db: { keys: [{ at: { ticks: 0, scale: 48000 }, value: -8, interp: 'linear' }, { at: { ticks: 48000, scale: 48000 }, value: -2, interp: 'linear' }] } }), clip('next', 50, 50, { gain_db: 4 }),
  ] }, { id: 'locked', name: 'Locked', kind: 'audio', enabled: true, locked: true, transitions: [], clips: [clip('locked-sound', 0, 50, { link: 'linked' }), clip('locked-next', 50, 50)] });
  return { timeline: song, revision: 9, selected: ['video', 'sound', 'sound', 'next'], linked: true, targets: { video_track: 'v1', audio_track: 'a1' }, marks: { in: null, out: null }, frame: 50, focus: 'timeline', source: null };
}
it('nudges mixed static and keyed gains through the engine edit, expands linked once and leaves keys untouched', () => {
  const ctx = context(); const before = JSON.stringify(ctx.timeline);
  expect(editCommand({ kind: 'gain', delta: -3 }, ctx)).toEqual({ kind: 'transaction', transaction: { revision: 9, ops: [{ edit: 'nudge_clip_gain', timeline: 't1', clips: ['sound', 'next'], delta_db: -3, linked: false }] } });
  expect(JSON.stringify(ctx.timeline)).toBe(before);
  expect(editCommand({ kind: 'gain', delta: 1 }, { ...ctx, selected: ['video'] })).toMatchObject({ transaction: { ops: [{ clips: ['sound'], linked: false }] } });
  for (const selected of [[], ['missing'], ['locked-next']]) expect(editCommand({ kind: 'gain', delta: 1 }, { ...ctx, selected })).toBeNull();
  expect(editCommand({ kind: 'gain', delta: 1 }, { ...ctx, selected: ['video'], linked: false })).toBeNull();
});
it('audio transition uses selected or target cut, skips locks/existing fades, and keeps Ctrl+T and Cut quick behavior', () => {
  const ctx = context();
  const expected = { revision: 9, ops: [{ edit: 'add_transition', timeline: 't1', left: 'sound', right: 'next', kind: 'crossfade', length: 25, align: 'center', linked: false }] };
  expect(editCommand({ kind: 'audio-transition' }, ctx)).toEqual({ kind: 'transaction', transaction: expected });
  expect(editCommand({ kind: 'audio-transition' }, { ...ctx, selected: [] })).toEqual({ kind: 'transaction', transaction: expected });
  expect(editCommand({ kind: 'audio-transition' }, { ...ctx, selected: ['locked-next'] })?.kind).toBe('refused');
  expect(editCommand({ kind: 'transition' }, ctx)).toMatchObject({ transaction: { ops: [expect.objectContaining({ kind: 'dissolve' }), expect.objectContaining({ kind: 'crossfade' })] } });
  expect(editCommand({ kind: 'transition' }, { ...ctx, cut: true })).toEqual(editCommand({ kind: 'quick', transition: 'dissolve' }, { ...ctx, cut: true }));
  const audio = ctx.timeline!.tracks[1]; if (audio.kind === 'subtitle') throw Error();
  for (const length of [25, 5]) {
    audio.transitions = [{ id: 'fade', left: 'sound', right: 'next', kind: 'crossfade', length, align: 'center' }];
    expect(editCommand({ kind: 'audio-transition' }, ctx)).toEqual({ kind: 'none' });
  }
});
it.each(['Edit', 'Fairlight'] as const)('%s Shift+T and menu share the retry for insufficient audio handles', async page => {
  const app = setup(page); const ctx = context(); app.props.selected = ctx.timeline!; app.view.rerender(app.draw()); await settle(); selectClips(['sound']);
  const normal = app.call.getMockImplementation()!; let refused = false;
  app.call.mockImplementation(async (method, params, options) => {
    if (method === 'transaction.apply' && !refused) { refused = true; return { ok: false, error: { code: -32001, message: 'short handles', data: { reason: 'no_handles', clip: 'sound', longest: 7 } } }; }
    return normal(method, params, options);
  });
  fireEvent.keyDown(document.querySelector('.timeline')!, { key: 'T', code: 'KeyT', shiftKey: true }); await settle();
  expect(transactions(app)).toHaveLength(2);
  expect(transactions(app)[0]).toMatchObject({ ops: [{ kind: 'crossfade', length: 25, linked: false }] });
  expect(transactions(app)[1]).toMatchObject({ ops: [{ kind: 'crossfade', length: 7, linked: false }] });
  fireEvent.click(screen.getByRole('button', { name: 'Audio' })); fireEvent.click(screen.getByRole('menuitem', { name: 'Add Audio Transition' })); await settle();
  expect(transactions(app)[2]).toEqual(transactions(app)[0]);
});
it.each(['Edit', 'Fairlight'] as const)('%s hides crossfade levels for video and locks selected audio transition controls', async page => {
  const app = setup(page); const song = context().timeline!;
  for (const track of song.tracks) if (track.kind !== 'subtitle') track.transitions = [{ id: track.id + '-fade', left: track.clips[0].id, right: track.clips[1].id, kind: track.kind === 'audio' ? 'crossfade' : 'dissolve', length: 25, align: 'center' }];
  app.props.selected = song; app.view.rerender(app.draw()); await settle();
  if (page === 'Fairlight') fireEvent.click(screen.getByRole('checkbox', { name: 'Video tracks' }));
  fireEvent.pointerDown(document.querySelector('[data-transition="v1-fade"]')!, { button: 0 }); await settle();
  expect(screen.queryByLabelText('Fade In Level')).toBeNull(); expect(screen.queryByLabelText('Fade Out Level')).toBeNull();
  fireEvent.pointerDown(document.querySelector('[data-transition="a1-fade"]')!, { button: 0 }); await settle();
  app.props.selected = { ...song, tracks: song.tracks.map(track => track.id === 'a1' ? { ...track, locked: true } : track) }; app.view.rerender(app.draw()); await settle();
  expect((screen.getByLabelText('Fade In Level') as HTMLSelectElement).disabled).toBe(true);
  expect((document.querySelector('[data-test="remove-transition"]') as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText('Fade In Level'), { target: { value: '-3' } }); await settle(); expect(transactions(app)).toEqual([]);
});
it.each([en, zhCN, ja])('uses translated gain, transition and level controls', async words => {
  const app = setup('Edit', words); app.props.selected = context().timeline!; app.view.rerender(app.draw()); await settle(); selectClips(['sound']);
  fireEvent.click(screen.getByRole('button', { name: words.label.studio.fairlightPage.Audio }));
  for (const label of ['Increase Audio Level 1 dB', 'Decrease Audio Level 1 dB', 'Increase Audio Level 3 dB', 'Decrease Audio Level 3 dB', 'Add Audio Transition'] as const) expect(screen.getByRole('menuitem', { name: words.label.studio.fairlightPage[label] })).toBeDefined();
  fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  const song = app.props.selected;
  app.props.selected = { ...song, tracks: song.tracks.map(track => track.id === 'a1' && track.kind !== 'subtitle' ? { ...track, transitions: [{ id: 'fade', left: 'sound', right: 'next', kind: 'crossfade', length: 25, align: 'center' }] } : track) };
  app.view.rerender(app.draw()); await settle(); fireEvent.pointerDown(document.querySelector('[data-transition="fade"]')!, { button: 0 }); await settle();
  for (const label of ['Fade In Level', 'Fade Out Level'] as const) expect(screen.getByLabelText(words.label.studio.fairlightPage[label])).toBeDefined();
});
