// @vitest-environment jsdom
/**
 * The floating window of the video scopes as a window of its own (Reference Manual 20, pp.
 * 3007–3008, 3018–3019; T19): the Color page asks the shell to open it, tells it what it measures,
 * closes it with Ctrl+Shift+W, its own button or the page, and docks its scope again as the shell
 * says it was closed; the window's page reaches the engine for its scopes through its own bridge,
 * draws their pictures, keeps the settings of its panes with the page's, and closes itself.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineEvent } from '../src/studio/engine';
import { keptWindowBounds, useColorScopes, type ScopesContext } from '../src/studio/color/Scopes';
import { loadScopes, storeScopes } from '../src/studio/color/scopeSettings';
import { ScopesApp } from '../src/scopes/ScopesApp';
import type { ScopesBridge, ScopesWindowContext } from '../src/scopes/bridge';
import { likeABrowser } from './mounted';

const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('scope-pane-body')) return rect(0, 0, 600, 300);
    return rect(0, 0, 100, 24);
  });
  const context = { createImageData: (width: number, height: number) => ({ width, height, data: new Uint8ClampedArray(width * height * 4) }), putImageData: () => {} };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => context) as never);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbinaScopes?: unknown }).columbinaScopes; localStorage.clear(); });

const ctrlShiftW = () => fireEvent.keyDown(document.body, { key: 'W', code: 'KeyW', ctrlKey: true, shiftKey: true });

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
/** What the engine answers a subscription or a change of scope `id` with `settings`. */
const answerOf = (id: string, settings: Record<string, any>) => {
  const size = sizeOf(settings);
  return { ok: true, result: { scope_id: id, size, graticule: { ...size, lines: [], labels: [] }, settings } };
};

/** The desktop app's bridge of the main page: the engine, and the shell's window of the scopes. */
function mainBridge() {
  const listeners = new Set<(event: EngineEvent) => void>();
  const bridge = {
    call: vi.fn(async (method: string, params?: Record<string, any>) => (method === 'scopes.subscribe' || method === 'scopes.set'
      ? answerOf(method === 'scopes.subscribe' ? 'scope-1' : params!.scope_id, params!.settings)
      : { ok: true, result: {} })),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    scopesWindow: vi.fn(async (asked: { open: boolean }) => ({ open: asked.open })),
  };
  const tell = (event: EngineEvent) => act(() => { listeners.forEach(listener => listener(event)); });
  return { bridge: bridge as unknown as ScopesContext['bridge'] & { scopesWindow: typeof bridge.scopesWindow; call: typeof bridge.call }, tell };
}

function Page({ bridge, loaded = true, native = true, language = 'ja' }: { bridge: ScopesContext['bridge']; loaded?: boolean; native?: boolean; language?: string }) {
  const scopes = useColorScopes({ bridge, session: 1, view: 'timeline', loaded, native, viewerMode: 'normal', language });
  return <div className="studio">{scopes.docked}{scopes.floating}</div>;
}

describe('the floating scopes of the Color page in the desktop app', () => {
  it('are a window of the shell: opened with what it measures, told as that changes, docked again as it closes by itself', async () => {
    const { bridge, tell } = mainBridge();
    localStorage.setItem('columbina-scopes-window', JSON.stringify({ x: 40, y: 50, width: 700, height: 420 }));
    const view = render(<Page bridge={bridge} />);
    await settle();
    fireEvent.click(screen.getByRole('button', { name: 'Open in a floating window' }));
    await settle();
    expect(bridge.scopesWindow.mock.calls).toEqual([[{ open: true, loaded: true, language: 'ja', bounds: { x: 40, y: 50, width: 700, height: 420 } }]]);
    // No window within the page; the docked panel says where they are, and measures nothing.
    expect(document.querySelector('[data-test="scope-window"]')).toBeNull();
    expect(document.querySelector('[data-test="scope-floating-note"]')).not.toBeNull();
    expect(bridge.call.mock.calls.filter(([method]) => method === 'scopes.unsubscribe')).toEqual([['scopes.unsubscribe', { scope_id: 'scope-1' }]]);
    // The viewer emptied: the window is told.
    view.rerender(<Page bridge={bridge} loaded={false} />);
    await settle();
    expect(bridge.scopesWindow.mock.calls.at(-1)).toEqual([{ open: true, loaded: false, language: 'ja', bounds: { x: 40, y: 50, width: 700, height: 420 } }]);
    // Closed by itself: docked again, and nothing more is asked of the shell.
    const asked = bridge.scopesWindow.mock.calls.length;
    tell({ kind: 'scopes-window', open: false, session: 1 });
    await settle();
    expect(document.querySelector('[data-test="scope-floating-note"]')).toBeNull();
    expect(bridge.scopesWindow.mock.calls.slice(asked)).toEqual([[{ open: false }]]);
    // Ctrl+Shift+W opens it and closes it.
    ctrlShiftW();
    await settle();
    expect(bridge.scopesWindow.mock.calls.at(-1)![0]).toMatchObject({ open: true });
    ctrlShiftW();
    await settle();
    expect(bridge.scopesWindow.mock.calls.at(-1)).toEqual([{ open: false }]);
    // Open as the page goes: closed with it.
    ctrlShiftW();
    await settle();
    view.unmount();
    await settle();
    expect(bridge.scopesWindow.mock.calls.at(-1)).toEqual([{ open: false }]);
  });

  it('float within the page where there is no shell to open a window, or no engine', async () => {
    const { bridge } = mainBridge();
    render(<Page bridge={bridge} native={false} />);
    await settle();
    ctrlShiftW();
    await settle();
    expect(bridge.scopesWindow).not.toHaveBeenCalled();
    expect(document.querySelector('[data-test="scope-window"]')).not.toBeNull();
  });

  it('read again the settings that the window of the scopes changed', async () => {
    const { bridge } = mainBridge();
    render(<Page bridge={bridge} />);
    await settle();
    const changed = loadScopes();
    changed.docked = { ...changed.docked, kind: 'vectorscope' };
    storeScopes(changed);
    act(() => { window.dispatchEvent(new StorageEvent('storage', { key: 'columbina-scopes' })); });
    await settle();
    expect((screen.getAllByRole('combobox')[0] as HTMLSelectElement).value).toBe('vectorscope');
  });
});

/** The bridge of the window of the scopes, as its preload gives it. */
function windowBridge(context: ScopesWindowContext) {
  const events = new Set<(event: EngineEvent) => void>();
  const contexts = new Set<(context: ScopesWindowContext) => void>();
  let made = 0;
  const shell = {
    context: vi.fn(async () => context),
    onContext: (listener: (context: ScopesWindowContext) => void) => { contexts.add(listener); return () => { contexts.delete(listener); }; },
    call: vi.fn(async (method: string, params?: Record<string, any>) => (method === 'scopes.subscribe' || method === 'scopes.set'
      ? answerOf(method === 'scopes.subscribe' ? `scope-${++made}` : params!.scope_id, params!.settings)
      : { ok: true, result: {} })),
    onEvent: (listener: (event: EngineEvent) => void) => { events.add(listener); return () => { events.delete(listener); }; },
    close: vi.fn(async () => {}),
  };
  window.columbinaScopes = shell as unknown as ScopesBridge;
  return {
    shell,
    told: (next: ScopesWindowContext) => act(() => { contexts.forEach(listener => listener(next)); }),
    subscribed: () => shell.call.mock.calls.filter(([method]) => method === 'scopes.subscribe').map(([, params]) => params),
  };
}

describe('the window of the scopes', () => {
  it('shows the panes of the floating scopes, in the language and theme of the app, measuring the timeline viewer through its own bridge', async () => {
    const { shell, told, subscribed } = windowBridge({ session: 3, loaded: true, language: 'zh-CN', theme: 'moonveil' });
    const stored = loadScopes();
    stored.floating = { ...stored.floating, layout: 2 };
    storeScopes(stored);
    render(<ScopesApp />);
    await settle();
    expect(document.documentElement.lang).toBe('zh-CN');
    expect(screen.getByRole('region', { name: '视频示波器' })).toBeTruthy();
    expect(subscribed()).toHaveLength(2);
    expect(subscribed()[0]).toMatchObject({ view: 'timeline', settings: { kind: 'parade' } });
    // 4 panes: two more of its own.
    fireEvent.click(document.querySelector('[data-test="scope-layout-4"]')!);
    await settle();
    expect(subscribed()).toHaveLength(4);
    expect(loadScopes().floating.layout).toBe(4);
    // Another engine: subscribed again on it.
    told({ session: 4, loaded: true, language: 'zh-CN', theme: 'moonveil' });
    await settle();
    expect(subscribed()).toHaveLength(8);
    // The close button and Ctrl+Shift+W close it.
    fireEvent.click(document.querySelector('[data-test="scope-window-close"]')!);
    ctrlShiftW();
    expect(shell.close).toHaveBeenCalledTimes(2);
  });

  it('keeps where it is and how large as it is resized', async () => {
    vi.useFakeTimers();
    try {
      windowBridge({ session: 1, loaded: true, language: 'en', theme: 'moonveil' });
      render(<ScopesApp />);
      Object.assign(window, { screenX: 100, screenY: 80, outerWidth: 800, outerHeight: 500 });
      act(() => { window.dispatchEvent(new Event('resize')); });
      act(() => { vi.advanceTimersByTime(400); });
      expect(keptWindowBounds()).toEqual({ x: 100, y: 80, width: 800, height: 500 });
    } finally {
      vi.useRealTimers();
    }
  });
});
