/**
 * Display HDR On Viewers If Available (Reference Manual 20, p. 244): whether the viewers may show
 * HDR where Windows has HDR on for their monitor. The engine keeps it only while it runs
 * (`view.display`, PROTOCOL.md, "The view"), and allows HDR until told otherwise; the user's choice
 * is a preference of this computer, not of the project, so it is kept here and sent again each time
 * an engine is opened, also when it was started again.
 */
import { useCallback, useSyncExternalStore } from 'react';
import type { EngineBridge } from '../bridge';
import { callEngine, type ViewDisplay } from './engine';

const KEY = 'columbina-studio-viewer-hdr';

/** The choice kept: allowed, unless the user turned it off. */
export function storedViewerHdr(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'false';
  } catch {
    return true;
  }
}

export interface ViewerHdr {
  /** Whether the viewers may show HDR. */
  hdr: boolean;
  /** What the engine said its views show, as it answered last; null before it answered. */
  shows: ViewDisplay | null;
}

/** The choice for one engine: sent at each `opened`, and when it is changed. */
export class ViewerDisplay {
  private state: ViewerHdr = { hdr: storedViewerHdr(), shows: null };
  private readonly listeners = new Set<() => void>();

  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent'>) {
    bridge.onEvent((event) => { if (event.kind === 'opened') this.send(); });
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = () => this.state;

  /** Allow HDR on the viewers or keep them to SDR, now and in the engines opened after. */
  set(hdr: boolean): void {
    try {
      localStorage.setItem(KEY, String(hdr));
    } catch {
      // Without storage the choice holds while the page lives.
    }
    this.change({ ...this.state, hdr });
    this.send();
  }

  private send(): void {
    const { hdr } = this.state;
    // An engine without views (headless, or the replay of a browser) has no such setting: nothing is said of it.
    callEngine(this.bridge, 'view.display', { hdr }).then((shows) => {
      if (this.state.hdr === hdr && shows && typeof shows === 'object') this.change({ ...this.state, shows });
    }, () => {});
  }

  private change(state: ViewerHdr): void {
    this.state = state;
    for (const listener of [...this.listeners]) listener();
  }
}

const made = new WeakMap<object, ViewerDisplay>();
/** The choice for the engine the bridge reaches: made, and listening for it to open, the first time it is asked for. */
export function viewerDisplayOf(bridge: Pick<EngineBridge, 'call' | 'onEvent'>): ViewerDisplay {
  let display = made.get(bridge);
  if (!display) { display = new ViewerDisplay(bridge); made.set(bridge, display); }
  return display;
}

/** The choice, drawn again as it changes, and what changes it. */
export function useViewerHdr(bridge: Pick<EngineBridge, 'call' | 'onEvent'>): ViewerHdr & { set: (hdr: boolean) => void } {
  const display = viewerDisplayOf(bridge);
  const state = useSyncExternalStore(display.subscribe, display.getSnapshot, display.getSnapshot);
  const set = useCallback((hdr: boolean) => display.set(hdr), [display]);
  return { ...state, set };
}

/** Whether the viewers show SDR because Windows has HDR off on the monitors of all their views (as far as the engine said). */
export function hdrOffEverywhere(shows: ViewDisplay | null): boolean {
  const views = [shows?.source, shows?.timeline].filter(view => view != null);
  return views.length > 0 && views.every(view => !view!.hdr_on);
}
