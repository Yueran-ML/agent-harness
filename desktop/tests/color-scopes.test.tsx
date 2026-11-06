// @vitest-environment jsdom
/**
 * The video scopes of the Color page (T19): a scope subscribed for each pane while it is shown and
 * let go when it is hidden, closed or no longer in the layout; the controls of a pane and the
 * option menu sent as `scopes.set`; the pictures drawn into the pane, the graticule over them; the
 * pane prepared until its first picture comes; the 16 scopes of a connection; the settings kept on
 * this desktop.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import type { EngineAnswer, EngineEvent, ScopeGraticule } from '../src/studio/engine';
import { ScopePane, useColorScopes, type ScopesContext } from '../src/studio/color/Scopes';
import {
  defaultScopes, engineSettings, fitted, isCircleOrCross, newPane, overlayOf, pictureBytes, pictureSize, readScopes, scopesStorageKey,
  DEFAULT_OPTIONS, MOST_SCOPES,
} from '../src/studio/color/scopeSettings';
import { hubOf, ScopeHub } from '../src/studio/color/useScope';
import { likeABrowser } from './mounted';

const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
let painted: { width: number; height: number; data: Uint8ClampedArray }[];
let visibility: DocumentVisibilityState;
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  painted = [];
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('scope-pane-body')) return rect(0, 0, 600, 300);
    return rect(0, 0, 100, 24);
  });
  const context = {
    createImageData: (width: number, height: number) => ({ width, height, data: new Uint8ClampedArray(width * height * 4) }),
    putImageData: (image: { width: number; height: number; data: Uint8ClampedArray }) => { painted.push(image); },
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => context) as never);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (document as { visibilityState?: unknown }).visibilityState; });

/** The size of the picture of a scope with `settings`, as the engine answers it (PROTOCOL.md, "Scopes"). */
function sizeOf(settings: Record<string, any>): { width: number; height: number } {
  switch (settings.kind) {
    case 'waveform': return { width: settings.width ?? 512, height: settings.levels ?? 256 };
    case 'parade': return { width: 3 * (settings.width ?? 256), height: settings.levels ?? 256 };
    case 'vectorscope': return { width: settings.size ?? 256, height: settings.size ?? 256 };
    case 'histogram': return { width: settings.bins ?? 256, height: settings.height ?? 256 };
    default: return { width: settings.width ?? 512, height: settings.height ?? 576 };
  }
}
const graticuleOf = ({ width, height }: { width: number; height: number }): ScopeGraticule => ({
  width, height,
  lines: [{ from: [0, height / 2], to: [width, height / 2], colour: [168, 150, 104] }, { from: [0, 1], to: [width, 1], colour: [168, 150, 104] }],
  labels: [{ at: [2, height / 2], text: '512', colour: [168, 150, 104] }],
});

/** An engine that answers the scopes as the real one does, and tells their pictures when asked. */
function fakeEngine() {
  const listeners = new Set<(event: EngineEvent) => void>();
  let made = 0;
  /** Answers held back, to be let go by the test. */
  const held: (() => void)[] = [];
  const state = { hold: false, refuse: null as null | { code: number; message: string; data: unknown } };
  const bridge = {
    call: vi.fn(async (method: string, params?: Record<string, any>) => {
      const answer = () => {
        if ((method === 'scopes.subscribe' || method === 'scopes.set') && state.refuse) return { ok: false as const, error: state.refuse };
        if (method === 'scopes.subscribe' || method === 'scopes.set') {
          const id = method === 'scopes.subscribe' ? `scope-${++made}` : params!.scope_id;
          const size = sizeOf(params!.settings);
          return { ok: true as const, result: { scope_id: id, size, graticule: graticuleOf(size), settings: params!.settings } };
        }
        return { ok: true as const, result: {} };
      };
      if (state.hold && method !== 'scopes.unsubscribe') return new Promise(resolve => { held.push(() => resolve(answer())); });
      return answer();
    }),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as ScopesContext['bridge'] & { call: Mock<(method: string, params?: Record<string, any>) => Promise<EngineAnswer>> };
  const calls = (method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params as Record<string, any>);
  return {
    bridge, state, held, calls,
    listeners: () => listeners.size,
    subscribed: () => calls('scopes.subscribe'),
    unsubscribed: () => calls('scopes.unsubscribe').map(params => params.scope_id),
    /** The engine sends a picture of a scope, of its size unless another is given. */
    picture: (scope: string, width: number, height: number, session = 1, bytes = width * height * 4) => act(() => {
      const rgba = Buffer.from(new Uint8Array(bytes).map((_, at) => at % 251)).toString('base64');
      listeners.forEach(listener => listener({ kind: 'notification', session, method: 'scopes.frame', params: { scope_id: scope, view: 'timeline', frame: 7, width, height, rgba } }));
    }),
  };
}

/** The scopes as the Color page has them: the docked one where the Scopes panel is shown, and the floating window. */
function Page({ bridge, panel = true, viewerMode = 'normal', session = 1, loaded = true, native = true }: {
  bridge: ScopesContext['bridge']; panel?: boolean; viewerMode?: string; session?: number | null; loaded?: boolean; native?: boolean;
}) {
  const scopes = useColorScopes({ bridge, session, view: 'timeline', loaded, native, viewerMode });
  return <div className="studio">{panel && scopes.docked}{scopes.floating}</div>;
}
const ctrlShiftW = () => fireEvent.keyDown(document.body, { key: 'W', code: 'KeyW', ctrlKey: true, shiftKey: true });
const status = () => [...document.querySelectorAll<HTMLElement>('[data-test="scope-status"]')].map(each => each.textContent);
const panes = () => [...document.querySelectorAll<HTMLElement>('[data-test="scope-pane"]')];

describe('the scopes of the Color page', () => {
  it('subscribe a scope while the docked panel is shown, and let it go when it is hidden', async () => {
    const engine = fakeEngine();
    const view = render(<Page bridge={engine.bridge} />);
    await settle();
    expect(engine.subscribed()).toHaveLength(1);
    const [first] = engine.subscribed();
    // The docked scope is the parade, as large as its pane, measuring the timeline monitor, its graticule drawn by the page.
    expect(first).toEqual({ view: 'timeline', settings: { kind: 'parade', width: 192, levels: 288, scale: '10-bit', look: { brightness: 0.5, graticule: 0 } } });
    view.rerender(<Page bridge={engine.bridge} panel={false} />);
    await settle();
    expect(engine.unsubscribed()).toEqual(['scope-1']);
    expect(engine.listeners()).toBe(0);
    view.rerender(<Page bridge={engine.bridge} />);
    await settle();
    expect(engine.subscribed()).toHaveLength(2);
    // The viewer over the palettes (Full page), and the window put away, let it go too.
    view.rerender(<Page bridge={engine.bridge} viewerMode="full" />);
    await settle();
    expect(engine.unsubscribed()).toEqual(['scope-1', 'scope-2']);
    view.rerender(<Page bridge={engine.bridge} />);
    await settle();
    visibility = 'hidden';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await settle();
    expect(engine.unsubscribed()).toEqual(['scope-1', 'scope-2', 'scope-3']);
    visibility = 'visible';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await settle();
    expect(engine.subscribed()).toHaveLength(4);
  });

  it('opens the floating window by Expand or Ctrl+Shift+W, a scope for each pane its layout shows', async () => {
    const engine = fakeEngine();
    render(<Page bridge={engine.bridge} />);
    await settle();
    fireEvent.click(screen.getByRole('button', { name: 'Open in a floating window' }));
    await settle();
    // The docked scope gives way to the window, which shows four panes at first.
    expect(engine.unsubscribed()).toEqual(['scope-1']);
    expect(document.querySelector('[data-test="scope-floating-note"]')?.textContent).toBe('The scopes are shown in their floating window.');
    const floating = screen.getByRole('dialog', { name: 'Video Scopes' });
    expect(floating.closest('[data-studio-portal]')).not.toBeNull();
    expect(panes()).toHaveLength(4);
    expect(engine.subscribed().slice(1).map(each => each.settings.kind)).toEqual(['parade', 'waveform', 'vectorscope', 'histogram']);
    // The layout buttons: 1, 2, 4 and 9 (p. 3018).
    expect(screen.getAllByRole('radio').filter(each => each.closest('.scope-layouts')).map(each => each.textContent)).toEqual(['1', '2', '4', '9']);
    fireEvent.click(document.querySelector('[data-test="scope-layout-9"]')!);
    await settle();
    expect(panes()).toHaveLength(9);
    expect(engine.subscribed()).toHaveLength(10);
    fireEvent.click(document.querySelector('[data-test="scope-layout-1"]')!);
    await settle();
    expect(panes()).toHaveLength(1);
    expect(engine.unsubscribed()).toEqual(['scope-1', 'scope-3', 'scope-4', 'scope-5', 'scope-6', 'scope-7', 'scope-8', 'scope-9', 'scope-10']);
    // Closed by the keys: its scope goes, and the docked one comes back.
    ctrlShiftW();
    await settle();
    expect(screen.queryByRole('dialog', { name: 'Video Scopes' })).toBeNull();
    expect(engine.unsubscribed().at(-1)).toBe('scope-2');
    expect(engine.subscribed()).toHaveLength(11);
    ctrlShiftW();
    await settle();
    expect(panes()).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await settle();
    expect(screen.queryByRole('dialog', { name: 'Video Scopes' })).toBeNull();
  });

  it('takes no keys typed into a field, and none with another modifier', async () => {
    const engine = fakeEngine();
    render(<Page bridge={engine.bridge} />);
    await settle();
    const field = document.querySelector<HTMLSelectElement>('[data-test="scope-kind"]')!;
    fireEvent.keyDown(field, { key: 'W', code: 'KeyW', ctrlKey: true, shiftKey: true });
    fireEvent.keyDown(document.body, { key: 'W', code: 'KeyW', ctrlKey: true, shiftKey: true, altKey: true });
    fireEvent.keyDown(document.body, { key: 'w', code: 'KeyW', ctrlKey: true });
    await settle();
    expect(screen.queryByRole('dialog', { name: 'Video Scopes' })).toBeNull();
  });

  it('sends the scope chosen and its controls as scopes.set, the latest once the one before is answered', async () => {
    const engine = fakeEngine();
    render(<Page bridge={engine.bridge} />);
    await settle();
    const set = () => engine.calls('scopes.set');
    fireEvent.change(document.querySelector('[data-test="scope-kind"]')!, { target: { value: 'vectorscope' } });
    await settle();
    expect(set()).toEqual([{ scope_id: 'scope-1', settings: {
      kind: 'vectorscope', size: 288, zoom: 1, tones: 'all', low: 0.3, high: 0.7, targets: '75', skin_tone: false, look: { brightness: 0.5, graticule: 0 },
    } }]);
    // The Custom Controls of the vectorscope (p. 3021).
    fireEvent.click(screen.getByRole('button', { name: 'Scope settings' }));
    const controls = screen.getByRole('dialog', { name: 'Scope settings' });
    expect(controls.textContent).toContain('Show Skin Tone Indicator');
    fireEvent.click(controls.querySelector('[data-test="scope-zoom2x"]')!);
    fireEvent.click(controls.querySelector('[data-test="scope-skin-tone"]')!);
    fireEvent.click(controls.querySelector('[data-test="scope-tones"] [data-value="high"]')!);
    fireEvent.change(controls.querySelector('[data-test="scope-low-range"]')!, { target: { value: '0.25' } });
    await settle();
    expect(set().at(-1)!.settings).toMatchObject({ zoom: 2, skin_tone: true, tones: 'high', low: 0.25, high: 0.7 });
    // Held answers: of three changes, the first is sent and the last after it, the one between never.
    engine.state.hold = true;
    const before = set().length;
    for (const value of ['0.6', '0.7', '0.8']) fireEvent.change(controls.querySelector('[data-test="scope-brightness"]')!, { target: { value } });
    await settle();
    expect(set().slice(before).map(each => each.settings.look.brightness)).toEqual([0.6]);
    engine.held.shift()!();
    await settle();
    expect(set().slice(before).map(each => each.settings.look.brightness)).toEqual([0.6, 0.8]);
    engine.held.shift()!();
    await settle();
    // The Graticule slider, Show Graticule and the reference levels are drawn by the page, and ask nothing of the engine.
    const count = set().length;
    fireEvent.change(controls.querySelector('[data-test="scope-graticule"]')!, { target: { value: '0.2' } });
    fireEvent.click(controls.querySelector('[data-test="scope-show-graticule"]')!);
    await settle();
    expect(set()).toHaveLength(count);
    // Escape puts the controls away.
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Scope settings' })).toBeNull();
  });

  it('sends the option menu to every scope: the scale of the waveform, the parade and the histogram, and the targets', async () => {
    const engine = fakeEngine();
    render(<Page bridge={engine.bridge} />);
    await settle();
    fireEvent.click(screen.getByRole('button', { name: 'Video scope options' }));
    const scale = document.querySelector('[data-test="menu-waveform-scale"]')!;
    fireEvent.click(scale);
    // The submenu lies in the menu layer, not inside its item, so that no scrolling parent cuts it off.
    const submenu = document.getElementById(scale.getAttribute('aria-controls')!)!;
    expect(submenu.getAttribute('role')).toBe('menu');
    expect([...submenu.querySelectorAll('[role="menuitem"]')].map(each => each.getAttribute('aria-label')))
      .toEqual(['10-bit', 'Percentage', 'HDR (ST.2084/HLG)']);
    fireEvent.click(document.querySelector('[data-test="menu-scale-percent"]')!);
    await settle();
    expect(engine.calls('scopes.set').at(-1)).toMatchObject({ scope_id: 'scope-1', settings: { kind: 'parade', scale: 'percent' } });
    // The vectorscope with the scale Off has no targets and no skin tone line (p. 3010).
    const off = engineSettings({ ...newPane('vectorscope'), vectorscope: { ...newPane().vectorscope, skinTone: true } }, { ...DEFAULT_OPTIONS, vectorStyle: 'off' }, { size: 256 });
    expect(off).toMatchObject({ targets: 'off', skin_tone: false });
    expect(engineSettings(newPane('vectorscope'), { ...DEFAULT_OPTIONS, targets: 'both' }, { size: 256 })).toMatchObject({ targets: 'both', skin_tone: false });
  });

  it('draws each picture into its pane, the graticule over it, and passes over those of other scopes, engines and sizes', async () => {
    const engine = fakeEngine();
    render(<Page bridge={engine.bridge} />);
    await settle();
    const canvas = document.querySelector<HTMLCanvasElement>('[data-test="scope-picture"]')!;
    expect(canvas.hidden).toBe(true);
    await engine.picture('scope-2', 576, 288);
    await engine.picture('scope-1', 576, 288, 2);
    await engine.picture('scope-1', 512, 256);
    await engine.picture('scope-1', 576, 288, 1, 100);
    expect(painted).toHaveLength(0);
    await engine.picture('scope-1', 576, 288);
    expect(painted).toHaveLength(1);
    expect([painted[0].width, painted[0].height, painted[0].data[0], painted[0].data[300]]).toEqual([576, 288, 0, 300 % 251]);
    expect([canvas.width, canvas.height, canvas.hidden]).toEqual([576, 288, false]);
    // As large as fits in its pane of 600 × 300, in the middle.
    expect([canvas.style.width, canvas.style.height, canvas.style.left, canvas.style.top]).toEqual(['600px', '300px', '0px', '0px']);
    expect(status()).toEqual([]);
    // The graticule as the page draws it: in pixels of the page, the labels in its type.
    const svg = document.querySelector<SVGElement>('[data-test="scope-graticule"]')!;
    const lines = [...svg.querySelectorAll('line')];
    expect(lines.map(line => [line.getAttribute('y1'), line.getAttribute('x2')])).toEqual([['150', '600'], [String(300 / 288), '600']]);
    expect(svg.querySelector('text')?.textContent).toBe('512');
    expect(svg.querySelector('g')!.getAttribute('opacity')).toBe('0.8');
  });

  it('says that a scope is being prepared until its first picture comes, and that there is nothing to measure where nothing is loaded', async () => {
    const engine = fakeEngine();
    engine.state.hold = true;
    const view = render(<Page bridge={engine.bridge} />);
    await settle();
    // The engine compiling its shaders the first time: no answer, and no error.
    expect(status()).toEqual(['The scope is being prepared. The first time takes a while.']);
    engine.held.shift()!();
    await settle();
    expect(status()).toEqual(['The scope is being prepared. The first time takes a while.']);
    await engine.picture('scope-1', 576, 288);
    expect(status()).toEqual([]);
    view.rerender(<Page bridge={engine.bridge} loaded={false} viewerMode="full" />);
    await settle();
    view.rerender(<Page bridge={engine.bridge} loaded={false} />);
    await settle();
    engine.held.shift()!();
    await settle();
    expect(status()).toEqual(['There is nothing in the viewer to measure.']);
    // Another scope is prepared anew: its shaders may not be compiled yet.
    await engine.picture('scope-2', 576, 288);
    fireEvent.change(document.querySelector('[data-test="scope-kind"]')!, { target: { value: 'histogram' } });
    await settle();
    engine.held.shift()!();
    await settle();
    expect(status()).toEqual(['There is nothing in the viewer to measure.']);
  });

  it('says why where the engine has no scopes, and outside the desktop app asks for none', async () => {
    const engine = fakeEngine();
    engine.state.refuse = { code: -32601, message: 'no such method', data: null };
    const view = render(<Page bridge={engine.bridge} />);
    await settle();
    expect(status()).toEqual(['This engine has no viewer, so it has no scopes.']);
    engine.state.refuse = { code: -32004, message: 'lost', data: null };
    view.rerender(<Page bridge={engine.bridge} panel={false} />);
    view.rerender(<Page bridge={engine.bridge} />);
    await settle();
    expect(status()).toEqual(['The scope could not be shown.']);
    cleanup();
    const browser = fakeEngine();
    render(<Page bridge={browser.bridge} native={false} />);
    await settle();
    expect(browser.subscribed()).toHaveLength(0);
    expect(status()).toEqual(['The scopes appear only in the desktop app.']);
  });

  it('subscribes anew for another engine, letting go nothing of the old one on the new connection', async () => {
    const engine = fakeEngine();
    const view = render(<Page bridge={engine.bridge} session={1} />);
    await settle();
    view.rerender(<Page bridge={engine.bridge} session={2} />);
    await settle();
    expect(engine.unsubscribed()).toEqual([]);
    expect(engine.subscribed()).toHaveLength(2);
    // Its pictures are those of the new engine.
    await engine.picture('scope-2', 576, 288, 1);
    expect(painted).toHaveLength(0);
    await engine.picture('scope-2', 576, 288, 2);
    expect(painted).toHaveLength(1);
    view.rerender(<Page bridge={engine.bridge} session={null} />);
    await settle();
    expect(engine.subscribed()).toHaveLength(2);
  });

  it('holds at most 16 scopes of a connection, the next waiting for a place and taking the first that is given back', async () => {
    const engine = fakeEngine();
    const context: ScopesContext = { bridge: engine.bridge, session: 1, view: 'timeline', loaded: true, native: true };
    const Many = ({ from }: { from: number }) => <>{Array.from({ length: MOST_SCOPES + 1 - from }, (_, at) => <ScopePane key={from + at} index={at}
      context={context} active pane={newPane('waveform')} options={DEFAULT_OPTIONS} onPane={() => {}} />)}</>;
    const tooMany = () => status().filter(each => each === 'No more scopes can be open at once.');
    const view = render(<Many from={0} />);
    await settle();
    expect(engine.subscribed()).toHaveLength(16);
    expect(tooMany()).toHaveLength(1);
    expect(hubOf(engine.bridge).count).toBe(16);
    // The first pane goes and gives its place back, which the one that waited takes.
    view.rerender(<Many from={1} />);
    await settle();
    expect(engine.unsubscribed()).toEqual(['scope-1']);
    expect(engine.subscribed()).toHaveLength(17);
    expect(tooMany()).toHaveLength(0);
    expect(hubOf(engine.bridge).count).toBe(16);
    cleanup();
    expect(hubOf(engine.bridge).count).toBe(0);
    // The engine's refusal waits too, and the pane takes a place again when one is given back.
    const hub = new ScopeHub(engine.bridge, 2);
    const [a, b, c] = [{}, {}, {}];
    const woken = vi.fn();
    expect([hub.take(a), hub.take(b), hub.take(c), hub.take(a)]).toEqual([true, true, false, true]);
    hub.wait(woken);
    hub.give(c);
    expect(woken).not.toHaveBeenCalled();
    hub.give(a);
    expect(woken).toHaveBeenCalledOnce();
    expect(hub.take(c)).toBe(true);
  });

  it('waits where the engine says it has too many', async () => {
    const engine = fakeEngine();
    engine.state.refuse = { code: -32602, message: 'too many scopes', data: { reason: 'too_many', most: 16 } };
    render(<Page bridge={engine.bridge} />);
    await settle();
    expect(status()).toEqual(['No more scopes can be open at once.']);
    expect(hubOf(engine.bridge).count).toBe(0);
  });

  it('keeps the settings of each pane, the layout and the options on this desktop', async () => {
    const engine = fakeEngine();
    const view = render(<Page bridge={engine.bridge} />);
    await settle();
    fireEvent.change(document.querySelector('[data-test="scope-kind"]')!, { target: { value: 'histogram' } });
    fireEvent.click(screen.getByRole('button', { name: 'Scope settings' }));
    fireEvent.click(document.querySelector('[data-test="scope-mode"] [data-value="yrgb"]')!);
    fireEvent.click(document.querySelector('[data-test="scope-reference"]')!);
    fireEvent.change(document.querySelector('[data-test="scope-reference-high"]')!, { target: { value: '940' } });
    ctrlShiftW();
    await settle();
    fireEvent.click(document.querySelector('[data-test="scope-layout-2"]')!);
    fireEvent.change(document.querySelectorAll('[data-test="scope-kind"]')[1]!, { target: { value: 'chromaticity' } });
    await settle();
    const kept = JSON.parse(localStorage.getItem(scopesStorageKey)!);
    expect(kept.docked).toMatchObject({ kind: 'histogram', histogram: { channels: 'yrgb', reference: { shown: true, low: 0, high: 940 } } });
    expect(kept.floating.layout).toBe(2);
    expect(kept.floating.panes.map((each: { kind: string }) => each.kind).slice(0, 2)).toEqual(['parade', 'chromaticity']);
    view.unmount();
    // Opened again, as they were left.
    const again = fakeEngine();
    render(<Page bridge={again.bridge} />);
    await settle();
    expect(again.subscribed()[0].settings).toMatchObject({ kind: 'histogram', channels: 'yrgb' });
    ctrlShiftW();
    await settle();
    expect(again.subscribed().slice(1).map(each => each.settings.kind)).toEqual(['parade', 'chromaticity']);
    expect(again.subscribed()[2].settings.gamuts).toEqual([{ name: 'Rec.709', primaries: 'rec709' }]);
  });

  it('reads kept settings that are missing or wrong as they are at first', () => {
    expect(readScopes(null)).toEqual(defaultScopes());
    expect(readScopes('not json')).toEqual(defaultScopes());
    const read = readScopes(JSON.stringify({
      options: { scale: 'mV', targets: '100', hueLabels: 'yes' },
      docked: { kind: 'oscilloscope', vectorscope: { low: 4, tones: 'mid', zoom2x: true }, waveform: { reference: { shown: true, high: 2000 } } },
      floating: { layout: 3, panes: [{ kind: 'chromaticity', chromaticity: { extra: 'dci-p3' } }, { chromaticity: { extra: 'nope' } }], rect: { x: 1, y: 2, width: 10, height: 20 } },
    }));
    expect(read.options).toEqual({ scale: '10-bit', vectorStyle: 'standard', targets: '100', hueLabels: true });
    expect(read.docked.kind).toBe('parade');
    expect(read.docked.vectorscope).toMatchObject({ low: 1, tones: 'mid', zoom2x: true });
    expect(read.docked.waveform.reference).toEqual({ shown: true, low: 0, high: 1023 });
    expect(read.floating.layout).toBe(4);
    expect(read.floating.panes).toHaveLength(9);
    expect([read.floating.panes[0].kind, read.floating.panes[0].chromaticity.extra, read.floating.panes[1].kind, read.floating.panes[1].chromaticity.extra])
      .toEqual(['chromaticity', 'dci-p3', 'waveform', null]);
    expect(read.floating.rect).toEqual({ x: 1, y: 2, width: 160, height: 120 });
  });
});

describe('the settings of a scope', () => {
  it('ask for a picture as large as the screen shows the pane, in steps, within the engine bounds', () => {
    expect(pictureSize('waveform', 600, 300)).toEqual({ width: 576, levels: 288 });
    expect(pictureSize('waveform', 600, 300, 2)).toEqual({ width: 864, levels: 416 });
    expect(pictureSize('parade', 600, 300)).toEqual({ width: 192, levels: 288 });
    expect(pictureSize('vectorscope', 600, 300)).toEqual({ size: 288 });
    expect(pictureSize('histogram', 20, 10)).toEqual({ bins: 64, height: 64 });
    expect(pictureSize('chromaticity', 600, 300)).toEqual({ width: 256, height: 288 });
    // Never more than 1024 × 384 in all, nor 1024 either way.
    for (const kind of ['waveform', 'parade', 'vectorscope', 'histogram', 'chromaticity'] as const) {
      const size = pictureSize(kind, 4000, 3000, 2);
      const { width, height } = sizeOf({ kind, ...size });
      expect(width * height).toBeLessThanOrEqual(1024 * 384 * 1.05);
      expect(Math.max(width, height)).toBeLessThanOrEqual(1024);
      expect(Math.min(width, height)).toBeGreaterThanOrEqual(16);
    }
    expect(pictureSize('vectorscope', 333, 333).size % 2).toBe(0);
  });

  it('map each scope to the settings of the engine, the graticule left to the page', () => {
    const options = { ...DEFAULT_OPTIONS, scale: 'nits' as const };
    const pane = newPane('waveform');
    expect(engineSettings({ ...pane, waveform: { ...pane.waveform, channels: 'luma', brightness: 0.75 } }, options, { width: 512, levels: 256 }))
      .toEqual({ kind: 'waveform', width: 512, levels: 256, channels: 'luma', scale: 'nits', look: { brightness: 0.75, graticule: 0 } });
    expect(engineSettings({ ...pane, kind: 'histogram', histogram: { ...pane.histogram, brightness: 0.9 } }, options, { bins: 256, height: 128 }))
      .toEqual({ kind: 'histogram', bins: 256, height: 128, channels: 'rgb', scale: 'nits', look: { brightness: 0.9, graticule: 0 } });
    expect(engineSettings({ ...pane, kind: 'chromaticity', chromaticity: { ...pane.chromaticity, extra: 'dci-p3' } }, options, { width: 256, height: 288 }))
      .toEqual({ kind: 'chromaticity', width: 256, height: 288, space: 'rec709',
        gamuts: [{ name: 'Rec.709', primaries: 'rec709' }, { name: 'DCI-P3', primaries: 'dci-p3' }], look: { brightness: 0.5, graticule: 0 } });
    // Low Range is never above High Range.
    expect(engineSettings({ ...pane, kind: 'vectorscope', vectorscope: { ...pane.vectorscope, low: 0.8, high: 0.6 } }, options, { size: 128 }))
      .toMatchObject({ low: 0.6, high: 0.6 });
  });

  it('draw over the picture what the boxes of the pane leave, and the reference levels moved off their rest', () => {
    const size = 256;
    const centre = size / 2;
    const radius = centre - 0.5;
    const circle = { from: [centre + radius, centre] as [number, number], to: [centre, centre - radius] as [number, number], colour: [168, 150, 104] as [number, number, number] };
    const cross = { from: [centre - 10, centre] as [number, number], to: [centre + 10, centre] as [number, number], colour: [168, 150, 104] as [number, number, number] };
    const target = { from: [40, 40] as [number, number], to: [50, 40] as [number, number], colour: [168, 150, 104] as [number, number, number] };
    const skin = { from: [centre, centre] as [number, number], to: [80, 60] as [number, number], colour: [222, 160, 120] as [number, number, number] };
    expect([circle, cross, target, skin].map(line => isCircleOrCross(line, size))).toEqual([true, true, false, false]);
    const graticule: ScopeGraticule = { width: size, height: size, lines: [circle, cross, target, skin],
      labels: [{ at: [30, 30], text: 'R', colour: [168, 150, 104] }, { at: [2, 2], text: '0.5', colour: [72, 72, 72] }] };
    const vector = newPane('vectorscope');
    expect(overlayOf(graticule, vector, DEFAULT_OPTIONS).lines).toHaveLength(4);
    const hidden = overlayOf(graticule, { ...vector, vectorscope: { ...vector.vectorscope, showGraticule: false } }, { ...DEFAULT_OPTIONS, hueLabels: false });
    expect([hidden.lines.length, hidden.labels.map(each => each.text)]).toEqual([2, ['0.5']]);
    const cie = newPane('chromaticity');
    expect(overlayOf({ ...graticule, lines: [{ ...target, colour: [72, 72, 72] }, skin] }, { ...cie, chromaticity: { ...cie.chromaticity, showGraticule: false } }, DEFAULT_OPTIONS))
      .toEqual({ lines: [{ x1: centre, y1: centre, x2: 80, y2: 60, colour: '222,160,120' }], labels: [{ x: 30, y: 30, text: 'R', colour: '168,150,104' }] });
    // Reference levels: only those moved off 0 and 1023, across the waveform and up the histogram.
    const wave = newPane('waveform');
    const levels = { width: 512, height: 256, lines: [], labels: [] };
    const shown = (low: number, high: number, kind: 'waveform' | 'histogram' = 'waveform') => overlayOf(levels, { ...wave, kind, [kind]: { ...wave[kind], reference: { shown: true, low, high } } }, DEFAULT_OPTIONS).lines;
    expect(shown(0, 1023)).toEqual([]);
    expect(shown(64, 1023)).toEqual([{ x1: 0, y1: 255 * (1 - 64 / 1023) + 0.5, x2: 512, y2: 255 * (1 - 64 / 1023) + 0.5, colour: '236,140,48', dashed: true }]);
    expect(shown(0, 940, 'histogram')).toEqual([{ x1: 511 * (940 / 1023) + 0.5, y1: 0, x2: 511 * (940 / 1023) + 0.5, y2: 256, colour: '236,140,48', dashed: true }]);
    expect(overlayOf(levels, { ...wave, waveform: { ...wave.waveform, reference: { shown: false, low: 64, high: 940 } } }, DEFAULT_OPTIONS).lines).toEqual([]);
  });

  it('fit a picture into its pane, and read only the bytes of its size', () => {
    expect(fitted(256, 256, 600, 300)).toEqual({ x: 150, y: 0, scale: 300 / 256 });
    expect(fitted(512, 256, 600, 600)).toEqual({ x: 0, y: 150, scale: 600 / 512 });
    expect(fitted(0, 256, 600, 600)).toEqual({ x: 0, y: 0, scale: 1 });
    expect(pictureBytes(Buffer.from([1, 2, 3, 4]).toString('base64'), 1, 1)).toEqual(new Uint8ClampedArray([1, 2, 3, 4]));
    expect(pictureBytes(Buffer.from([1, 2, 3]).toString('base64'), 1, 1)).toBeNull();
    expect(pictureBytes('***', 1, 1)).toBeNull();
  });
});
