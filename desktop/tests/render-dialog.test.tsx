// @vitest-environment jsdom
/**
 * The export of a timeline: Quick Export on the Edit and Cut pages opens its dialog over
 * the page (the views of both monitors hidden); the dialog asks for the range, the codec, the
 * quality and the subtitle tracks saved as SRT, and the shell asks where in Save As; what the engine
 * refuses is said in the dialog. The strip of its progress lies over nothing, so the user edits on
 * while it runs; it tells how far it is, cancels it, shows the file in its folder, says why it
 * failed, and that ProRes was written where the computer had no encoder for H.264 or HEVC.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import { Studio } from '../src/studio/Studio';
import { ProjectCopy, type EngineAnswer, type EngineEvent, type Project, type RenderOptions, type RenderProgress, type TransportState } from '../src/studio/engine';
import { markedRange, shownSubtitleTracks, subtitleTracksOf } from '../src/studio/render/RenderDialog';
import { fellBack, refusalOf } from '../src/studio/render/renders';
import type { EngineBridge } from '../src/bridge';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

let frames: FrameRequestCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
beforeEach(() => {
  likeABrowser(); frames = [];
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const w = this.classList.contains('edit-page') || this.classList.contains('cut-page') ? 1500 : 450;
    return { left: 330, top: 100, width: w, height: 300, right: 330 + w, bottom: 400, x: 330, y: 100, toJSON() {} };
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; });

/** A timeline of 200 frames with a subtitle track that is shown and one that is hidden, and one of 100 frames without subtitles. */
function project(): Project {
  const main = timeline();
  main.tracks.push({ id: 's1', kind: 'subtitle', name: 'S1', enabled: true, locked: false, language: 'en', cues: [{ id: 'q1', start: 0, length: 50, text: 'Hello' }] });
  main.tracks.push({ id: 's2', kind: 'subtitle', name: 'S2', enabled: false, locked: false, language: 'fr', cues: [{ id: 'q2', start: 0, length: 50, text: 'Bonjour' }] });
  return { schema: 4, groups: [], next_id: 10, bins: [], media: [media], timelines: [main, { ...timeline([clip('c3')]), id: 't2', name: 'Alternate' }] };
}

/** An engine that keeps a project and its two transports, and renders as the shell and the engine would. */
function fakeEngine() {
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (event: EngineEvent) => act(() => { listeners.forEach(listener => listener(event)); });
  const states: Record<'source' | 'timeline', TransportState> = {
    source: { monitor: 'source', media: 'm1', frame: 10, shown: 10, last: 249, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: 'original' },
    timeline: { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 },
  };
  const bridge = {
    open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision: 0 }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>): Promise<EngineAnswer> => {
      if (method === 'project.get') return { ok: true, result: { project: project(), revision: 0, next_id: 10 } };
      const monitor = params?.monitor as 'source' | 'timeline';
      if (method === 'transport.state') return { ok: true, result: states[monitor] };
      if (method === 'transport.load') {
        const state = (monitor === 'timeline'
          ? { ...states.timeline, timeline: params!.timeline as string, frame: 0, shown: 0, last: 199 }
          : { ...states.source, media: params!.media as string, frame: 0, shown: 0 }) as TransportState;
        states[monitor] = state; listeners.forEach(listener => listener({ kind: 'notification', session: 1, method: 'transport.state', params: state }));
        return { ok: true, result: state };
      }
      return { ok: true, result: {} };
    }),
    render: vi.fn(async (_options: RenderOptions & { name: string }): Promise<EngineAnswer> => ({ ok: true, result: { job: 'render-1' } })),
    reveal: vi.fn(async () => true),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  };
  const copy = new ProjectCopy(bridge as unknown as EngineBridge);
  return {
    bridge, copy, tell,
    progress: (fields: Partial<RenderProgress>, session = 1) => tell({ kind: 'notification', session, method: 'render.progress', params: { job: 'render-1', state: 'running', frame: 0, frames: 200, fps: 0, ...fields } }),
    notify: (monitor: 'source' | 'timeline', fields: Partial<TransportState>) => act(() => {
      states[monitor] = { ...states[monitor], ...fields } as TransportState;
      listeners.forEach(listener => listener({ kind: 'notification', session: 1, method: 'transport.state', params: states[monitor] }));
    }),
  };
}

async function mount(pageName: 'Edit' | 'Cut' | 'Media' = 'Edit', words?: typeof zhCN) {
  const engine = fakeEngine();
  const tree = <Studio cwd="F:/film" engine={{ bridge: engine.bridge as unknown as EngineBridge, copy: engine.copy, native: true }} language="zh-CN" onClose={() => {}} />;
  const view = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle();
  const tab = words === zhCN ? { Edit: '剪辑', Cut: '快编', Media: '媒体' }[pageName] : pageName;
  fireEvent.click(screen.getByRole('tab', { name: tab })); await settle(); draw(); await settle();
  return { ...engine, ...view };
}
const one = (name: string) => document.querySelector<HTMLElement>(`[data-test="${name}"]`);
const dialog = () => screen.queryByRole('dialog');
const ctrlM = (target: Element = document.body) => fireEvent.keyDown(target, { key: 'm', ctrlKey: true });
/** Open the export from Quick Export, at the right of the bar of the page. */
const openExport = () => fireEvent.click(one('quick-export')!);
const exportTo = async () => { fireEvent.click(one('export-to')!); await settle(); };

describe('the dialog of the export', () => {
  it('opens from Quick Export at the right of the Edit bar, beside the inspector, with the views of both monitors hidden', async () => {
    const { bridge } = await mount();
    const bar = document.querySelector('.edit-bar')!;
    const buttons = [...bar.querySelectorAll('button')].map(each => each.dataset.test);
    expect(buttons.slice(-2)).toEqual(['quick-export', 'toggle-inspector']);
    bridge.viewShow.mockClear();
    fireEvent.click(one('quick-export')!); await settle();
    expect(dialog()?.getAttribute('aria-label')).toBe('Export timeline');
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'source');
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'timeline');
    // The timeline shown is chosen, the whole of it, H.264 at high quality.
    expect((one('render-timeline') as HTMLSelectElement).value).toBe('t1');
    expect((one('range-whole') as HTMLInputElement).checked).toBe(true);
    expect((one('range-marked') as HTMLInputElement).disabled).toBe(true);
    expect(one('range-hint')!.textContent).toBe('Mark an In and an Out point on the timeline to export only the part between them.');
    expect((one('codec-h264') as HTMLInputElement).checked).toBe(true);
    expect((one('render-quality') as HTMLSelectElement).value).toBe('high');
    // Cancel closes it and asks nothing; the views come back.
    fireEvent.click(within(dialog()!).getByRole('button', { name: 'Cancel' })); await settle();
    expect(dialog()).toBeNull();
    expect(bridge.render).not.toHaveBeenCalled();
    expect(bridge.viewShow).toHaveBeenLastCalledWith(true, expect.anything());
  });

  it('leaves Ctrl+M to the markers of DaVinci Resolve: it opens no export, wherever the keyboard is', async () => {
    // Reference Manual 20, p. 869: Command-M adds a marker and opens its edit dialog; the studio has no markers yet.
    await mount();
    expect(one('quick-export')!.title).toBe('Quick export');
    ctrlM(); await settle();
    ctrlM(document.querySelector('.timeline')!); await settle();
    fireEvent.keyDown(document.body, { key: 'M', ctrlKey: true, shiftKey: true }); await settle();
    expect(dialog()).toBeNull();
  });

  it('opens from Quick Export on the Cut page too, and not on the Media page', async () => {
    await mount('Cut');
    expect(document.querySelector('.cut-bar [data-test="quick-export"]')).not.toBeNull();
    fireEvent.click(one('quick-export')!); await settle();
    expect(dialog()).not.toBeNull();
    fireEvent.click(within(dialog()!).getByRole('button', { name: 'Cancel' })); await settle();
    ctrlM(document.querySelectorAll('.timeline')[1]); await settle();
    expect(dialog()).toBeNull();
    cleanup();
    await mount('Media');
    expect(one('quick-export')).toBeNull();
    ctrlM(); await settle();
    expect(dialog()).toBeNull();
  });

  it('offers the part between the In and Out points of the timeline, and only for the timeline that has them', async () => {
    const { bridge, notify } = await mount();
    fireEvent.pointerDown(screen.getByRole('region', { name: 'Timeline viewer' }));
    notify('timeline', { frame: 50, shown: 50 }); fireEvent.keyDown(document.body, { key: 'i' });
    notify('timeline', { frame: 149, shown: 149 }); fireEvent.keyDown(document.body, { key: 'o' });
    openExport(); await settle();
    expect((one('range-marked') as HTMLInputElement).checked).toBe(true);
    // The Out frame, 149, is in the range and is shown as the Out, as DaVinci Resolve shows it.
    expect(one('range-marks')!.textContent).toBe('00:00:02:00 – 00:00:05:24');
    expect(one('range-hint')).toBeNull();
    // Another timeline has no marks on the page.
    fireEvent.change(one('render-timeline')!, { target: { value: 't2' } });
    expect((one('range-marked') as HTMLInputElement).disabled).toBe(true);
    expect((one('range-whole') as HTMLInputElement).checked).toBe(true);
    fireEvent.change(one('render-timeline')!, { target: { value: 't1' } });
    fireEvent.click(one('codec-hevc')!);
    fireEvent.change(one('render-quality')!, { target: { value: 'small' } });
    await exportTo();
    expect(bridge.render).toHaveBeenCalledWith({
      timeline: 't1', name: 'Main', codec: 'hevc', quality: 'small', subtitle_tracks: ['s1'], range: { from: 50, to: 150 }, language: 'zh-CN',
    });
    // The marks are the page's: In alone goes to the end, Out alone from the start.
    expect(markedRange({ in: 50, out: null }, 200)).toEqual({ from: 50, to: 200 });
    expect(markedRange({ in: null, out: 20 }, 200)).toEqual({ from: 0, to: 20 });
    expect(markedRange({ in: 190, out: 300 }, 200)).toEqual({ from: 190, to: 200 });
    expect(markedRange({ in: 250, out: null }, 200)).toBeNull();
    expect(markedRange({ in: null, out: null }, 200)).toBeNull();
    expect(markedRange(null, 200)).toBeNull();
  });

  it('offers each subtitle track as SRT, shown or hidden, the shown one checked, as the Deliver page of DaVinci Resolve', async () => {
    const { bridge } = await mount();
    openExport(); await settle();
    // Reference Manual 20, p. 1102: a checkbox for each subtitle track to output as a separate file.
    expect(one('render-subtitles')!.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
    expect((one('render-subtitle-s1') as HTMLInputElement).checked).toBe(true);
    expect((one('render-subtitle-s2') as HTMLInputElement).checked).toBe(false);
    expect(one('render-subtitles')!.textContent).toContain('S2 · fr');
    // Both, sent in the order of the tracks of the timeline whatever the order they were checked in.
    fireEvent.click(one('render-subtitle-s1')!);
    fireEvent.click(one('render-subtitle-s2')!);
    fireEvent.click(one('render-subtitle-s1')!);
    fireEvent.click(one('codec-prores')!);
    await exportTo();
    expect(bridge.render).toHaveBeenLastCalledWith(expect.objectContaining({ timeline: 't1', codec: 'prores', subtitle_tracks: ['s1', 's2'] }));
    expect(bridge.render.mock.calls.at(-1)![0]).not.toHaveProperty('range');
    expect(bridge.render.mock.calls.at(-1)![0]).not.toHaveProperty('subtitles');
    expect(subtitleTracksOf(project().timelines[0]).map(track => track.id)).toEqual(['s1', 's2']);
    expect(shownSubtitleTracks(project().timelines[0])).toEqual(['s1']);
    expect(subtitleTracksOf(project().timelines[1])).toEqual([]);
    expect(subtitleTracksOf(null)).toEqual([]);
  });

  it('stays open where Save As was cancelled, and closes as the export starts', async () => {
    const { bridge } = await mount();
    bridge.render.mockResolvedValueOnce({ ok: true, result: { cancelled: true } });
    openExport(); await settle();
    fireEvent.change(one('render-timeline')!, { target: { value: 't2' } });
    expect(one('render-subtitles')).toBeNull();
    await exportTo();
    expect(dialog()).not.toBeNull();
    expect(one('render-refused')).toBeNull();
    expect(bridge.render).toHaveBeenLastCalledWith(expect.objectContaining({ timeline: 't2', name: 'Alternate', subtitle_tracks: [] }));
    await exportTo();
    expect(dialog()).toBeNull();
    expect(one('render-progress')).not.toBeNull();
  });

  it('says in the dialog why the engine would not start, whatever the reason, and stays open', async () => {
    const { bridge } = await mount();
    const refuse = (code: number | null, data: unknown, message = 'refused') => bridge.render.mockResolvedValueOnce({ ok: false, error: { code, message, data } });
    openExport(); await settle();
    const cases: [number | null, unknown, string][] = [
      [-32001, { reason: 'busy' }, 'Another export of this project is running, maybe one that the agent started. Try again when it has finished.'],
      // Save As confirmed the file chosen; what it would write beside it was not asked of, and the shell did not replace it.
      [-32001, { reason: 'exists', path: 'F:/out/film.srt' }, 'Files that this export writes beside the film are there already, such as an SRT or a .mov file. Choose another name, or move them away.'],
      [-32001, { reason: 'part_exists', path: 'F:/out/film.mp4.part' }, 'A partial file of an export that did not finish is there. Delete it, or choose another name.'],
      [-32001, { reason: 'source', path: 'F:/media/interview.mp4', media: 'm1' }, 'That file is media of this project, and is not written over. Choose another name.'],
      [-32602, { reason: 'out_of_range' }, 'The engine did not take these settings.'],
      [null, null, 'The export could not start.'],
    ];
    for (const [code, data, said] of cases) {
      refuse(code, data);
      await exportTo();
      expect(dialog()).not.toBeNull();
      expect(one('render-refused')!.firstChild!.textContent).toBe(said);
    }
    expect(one('render-refused')!.querySelector('.caption')!.textContent).toBe('refused');
    // What the shell itself refused is said too.
    bridge.render.mockRejectedValueOnce(new Error('Choose a whole output path in Save As.'));
    await exportTo();
    expect(one('render-refused')!.textContent).toContain('The export could not start.');
    expect(refusalOf(-32001, { reason: 'exists', path: 'F:/a.srt' }, 'x')).toEqual({ kind: 'refused', reason: 'exists', detail: 'F:/a.srt' });
    expect(refusalOf(-32001, { reason: 'stale_revision' }, 'x')).toEqual({ kind: 'refused', reason: 'other', detail: 'x' });
  });

  it('is said in Chinese and Japanese too', async () => {
    await mount('Edit', zhCN);
    expect(one('quick-export')!.getAttribute('aria-label')).toBe('快速导出');
    openExport(); await settle();
    expect(dialog()!.getAttribute('aria-label')).toBe('导出时间线');
    expect(within(dialog()!).getByRole('button', { name: '导出到…' })).toBeDefined();
    expect(within(dialog()!).getByText('另存为 SRT 的字幕轨')).toBeDefined();
    cleanup();
    const engine = fakeEngine();
    render(<Words.Provider value={ja}><Studio cwd="F:/film" engine={{ bridge: engine.bridge as unknown as EngineBridge, copy: engine.copy, native: true }} language="ja" onClose={() => {}} /></Words.Provider>);
    await settle();
    fireEvent.click(screen.getByRole('tab', { name: ja.label.studio.page.edit })); await settle();
    openExport(); await settle();
    expect(dialog()!.getAttribute('aria-label')).toBe('タイムラインを書き出す');
  });
});

describe('the progress of the export', () => {
  async function started() {
    const engine = await mount();
    openExport(); await settle();
    await exportTo();
    return engine;
  }

  it('goes on while the user edits, with how far it is, how fast, and the encoder', async () => {
    const { progress, bridge } = await started();
    const strip = one('render-progress')!;
    expect(strip.dataset.state).toBe('running');
    expect(strip.textContent).toContain('Exporting');
    expect(strip.textContent).toContain('Main');
    // Before the engine tells how many frames, the bar is of one frame, not of none.
    expect(one('render-bar')!.getAttribute('aria-valuemax')).toBe('1');
    // A screen reader is told that it started, and not of each frame.
    expect(one('render-announced')!.textContent).toBe('Exporting Main');
    expect(strip.getAttribute('role')).toBe('group');
    progress({ frame: 50, frames: 200, fps: 48.25, encoder: 'h264_nvenc', path: 'F:\\film\\Main.mp4' });
    expect(one('render-numbers')!.textContent).toBe('50 of 200 frames · 48.3 fps · h264_nvenc');
    expect(one('render-bar')!.getAttribute('aria-valuenow')).toBe('50');
    expect((one('render-bar')!.firstChild as HTMLElement).style.width).toBe('25%');
    expect(strip.textContent).toContain('Main.mp4');
    expect(one('render-bar')!.getAttribute('aria-valuemax')).toBe('200');
    expect(one('render-announced')!.textContent).toBe('Exporting Main');
    // It lies over nothing: the views stay shown, and the page goes on taking its keys.
    expect(document.body.dataset.overlay).toBeUndefined();
    expect(dialog()).toBeNull();
    bridge.call.mockClear();
    fireEvent.keyDown(document.body, { key: 'l' }); await settle();
    expect(bridge.call.mock.calls.map(([method]) => method)).toContain('transport.play');
    // What the engine tells of another job, such as an agent's, is not this one.
    progress({ job: 'render-7', frame: 190 });
    expect(one('render-bar')!.getAttribute('aria-valuenow')).toBe('50');
    // One export at a time: the dialog says so, and starts none.
    openExport(); await settle();
    expect(one('render-running')!.textContent).toBe('An export is running. Wait for it to finish, or cancel it.');
    expect((one('export-to') as HTMLButtonElement).disabled).toBe(true);
  });

  it('draws the strip again as the engine tells of the export, and not the studio', async () => {
    const { progress, copy } = await started();
    // The studio reads the copy of the project each time it is drawn: it is not drawn again.
    const drawn = vi.spyOn(copy, 'getSnapshot');
    for (const frame of [10, 20, 30]) progress({ frame });
    expect(one('render-bar')!.getAttribute('aria-valuenow')).toBe('30');
    expect(drawn).not.toHaveBeenCalled();
    // Its end is told to the studio: the dialog may start another.
    progress({ state: 'done', frame: 200 });
    expect(drawn).toHaveBeenCalled();
  });

  it('is cancelled, and says so once the engine has cleaned its files away', async () => {
    const { progress, bridge } = await started();
    progress({ frame: 20 });
    fireEvent.click(one('render-cancel')!); await settle();
    expect(bridge.call).toHaveBeenCalledWith('render.cancel', { job: 'render-1' });
    expect(one('render-progress')!.dataset.state).toBe('cancelling');
    expect((one('render-cancel') as HTMLButtonElement).disabled).toBe(true);
    progress({ frame: 21 });
    expect(one('render-progress')!.dataset.state).toBe('cancelling');
    progress({ state: 'cancelled', frame: 21 });
    expect(one('render-progress')!.dataset.state).toBe('cancelled');
    expect(one('render-progress')!.textContent).toContain('The export was cancelled, and nothing was written.');
    expect(one('render-cancel')).toBeNull();
    fireEvent.click(one('render-dismiss')!); await settle();
    expect(one('render-progress')).toBeNull();
  });

  it('shows the file in its folder once it is done', async () => {
    const { progress, bridge } = await started();
    progress({ state: 'done', frame: 200, encoder: 'hevc_nvenc', path: 'F:\\out\\Main.mp4' });
    const strip = one('render-progress')!;
    expect(strip.dataset.state).toBe('done');
    expect(strip.textContent).toContain('The export is finished.');
    expect(one('render-announced')!.textContent).toBe('The export is finished. Main.mp4');
    expect(one('render-fallback')).toBeNull();
    fireEvent.click(one('render-reveal')!); await settle();
    expect(bridge.reveal).toHaveBeenCalledWith('render-1');
    // Where the shell knows of no file any more, its path is said instead.
    bridge.reveal.mockResolvedValueOnce(false);
    fireEvent.click(one('render-reveal')!); await settle();
    expect(strip.textContent).toContain('F:\\out\\Main.mp4');
    // A later telling changes nothing of an export that ended.
    progress({ state: 'failed', message: 'late' });
    expect(one('render-progress')!.dataset.state).toBe('done');
  });

  it('says that ProRes was written where the computer has no encoder for H.264 or HEVC', async () => {
    const { progress } = await started();
    progress({ state: 'done', frame: 200, encoder: 'prores_ks', path: 'F:\\out\\Main.mov', message: 'no hardware H.264 encoder; wrote ProRes' });
    expect(one('render-fallback')!.textContent).toBe('This computer has no hardware encoder for H.264 or HEVC, so the film was written as ProRes into a .mov file.');
    expect(one('render-progress')!.textContent).toContain('Main.mov');
    expect(fellBack({ state: 'done', codec: 'prores', encoder: 'prores_ks' })).toBe(false);
    expect(fellBack({ state: 'running', codec: 'h264', encoder: 'prores_ks' })).toBe(false);
  });

  it('says why it failed, in the words of the engine', async () => {
    const { progress } = await started();
    progress({ state: 'failed', frame: 12, message: 'h264_nvenc failed at frame 12: device lost' });
    expect(one('render-progress')!.textContent).toContain('The export failed.');
    expect(one('render-detail')!.textContent).toBe('H264_nvenc failed at frame 12: device lost.');
    expect(one('render-reveal')).toBeNull();
  });

  it('takes what the engine told before the answer that named the job, and ends where the engine stops', async () => {
    const engine = await mount();
    let answer!: (value: EngineAnswer) => void;
    engine.bridge.render.mockReturnValueOnce(new Promise(resolve => { answer = resolve; }));
    openExport(); await settle();
    await exportTo();
    // While Save As is open and the engine starts, the button waits.
    expect((one('export-to') as HTMLButtonElement).disabled).toBe(true);
    engine.progress({ frame: 30, encoder: 'h264_nvenc' });
    await act(async () => { answer({ ok: true, result: { job: 'render-1' } }); });
    await settle();
    expect(one('render-numbers')!.textContent).toContain('30 of 200 frames');
    engine.tell({ kind: 'exited', session: 1, expected: false, code: 1, signal: null });
    expect(one('render-progress')!.dataset.state).toBe('stopped');
    expect(one('render-progress')!.textContent).toContain('The engine of the studio stopped during the export.');
  });

  it('is kept while the studio is left and shown again', async () => {
    const engine = await mount();
    openExport(); await settle();
    await exportTo();
    engine.progress({ frame: 80 });
    engine.unmount();
    render(<Studio cwd="F:/film" engine={{ bridge: engine.bridge as unknown as EngineBridge, copy: engine.copy, native: true }} language="en" onClose={() => {}} />);
    await settle();
    expect(one('render-numbers')!.textContent).toContain('80 of 200 frames');
  });
});
