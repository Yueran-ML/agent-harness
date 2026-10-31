/**
 * The library of LUTs as the page keeps it, and the LUTs the user likes.
 *
 * The library is what `luts.list` answered, one for each engine: asked for the first time something
 * shows LUTs, again on Refresh (the LUT browser's) or Update Lists (the Project Settings', p. 153),
 * and again when another project is opened, whose folder has other LUTs. The LUT browser, the LUT ▸
 * menus and the Lookup Tables of the Project Settings read the same one.
 *
 * The favorites (p. 3411) are the user's, not the project's: they are kept in the preferences of
 * this computer, by the names that nodes give the LUTs.
 */
import { useCallback, useEffect, useSyncExternalStore } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type LutLibrary } from '../engine';

export interface LutState {
  library: LutLibrary | null;
  /** What the engine said where it could not list them. */
  error: string | null;
  loading: boolean;
}

const NOTHING: LutState = { library: null, error: null, loading: false };

/** The LUTs of one engine. */
export class LutStore {
  private state: LutState = NOTHING;
  private asked = false;
  private asking = 0;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent'>) {
    // Another project (or the engine started again) has its own folder of LUTs.
    bridge.onEvent((event) => {
      if (event.kind === 'opened' && this.asked) void this.load();
    });
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = () => this.state;

  /** List them, unless they were listed already. */
  ensure(): void { if (!this.asked) void this.load(); }

  /** List them again, from the folders on the disk. */
  async load(): Promise<void> {
    this.asked = true;
    const asking = ++this.asking;
    this.set({ ...this.state, loading: true });
    try {
      const answer: Partial<LutLibrary> = await callEngine(this.bridge, 'luts.list') ?? {};
      const library: LutLibrary = {
        roots: Array.isArray(answer.roots) ? answer.roots : [], luts: Array.isArray(answer.luts) ? answer.luts : [], truncated: answer.truncated === true,
      };
      if (asking === this.asking) this.set({ library, error: null, loading: false });
    } catch (failure) {
      if (asking === this.asking) this.set({ library: this.state.library, error: failure instanceof Error ? failure.message : String(failure), loading: false });
    }
  }

  private set(state: LutState): void {
    this.state = state;
    for (const listener of [...this.listeners]) listener();
  }
}

const stores = new WeakMap<object, LutStore>();
/** The LUTs of the engine that the bridge reaches. */
export function lutStoreOf(bridge: Pick<EngineBridge, 'call' | 'onEvent'>): LutStore {
  let store = stores.get(bridge);
  if (!store) { store = new LutStore(bridge); stores.set(bridge, store); }
  return store;
}

/** The library of LUTs, listed once something shows it; and Refresh. */
export function useLutLibrary(bridge: Pick<EngineBridge, 'call' | 'onEvent'>): LutState & { refresh: () => void } {
  const store = lutStoreOf(bridge);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  useEffect(() => { store.ensure(); }, [store]);
  const refresh = useCallback(() => { void store.load(); }, [store]);
  return { ...state, refresh };
}

// The favorites, in the preferences of this computer.

const FAVORITES_KEY = 'columbina-studio-lut-favorites';
const favoriteListeners = new Set<() => void>();
let favoritesKept: ReadonlySet<string> | null = null;

function readFavorites(): ReadonlySet<string> {
  try {
    const kept: unknown = JSON.parse(localStorage.getItem(FAVORITES_KEY) ?? '[]');
    return new Set(Array.isArray(kept) ? kept.filter((name): name is string => typeof name === 'string') : []);
  } catch {
    // Without storage, or with something else kept there, there are none yet.
    return new Set();
  }
}

/** The names of the LUTs the user likes. */
export function favoriteNames(): ReadonlySet<string> {
  favoritesKept ??= readFavorites();
  return favoritesKept;
}

/** Add a LUT to the favorites, or take it out of them. */
export function toggleFavorite(name: string): void {
  const next = new Set(favoriteNames());
  if (next.has(name)) next.delete(name); else next.add(name);
  favoritesKept = next;
  try {
    localStorage.setItem(FAVORITES_KEY, JSON.stringify([...next].sort()));
  } catch {
    // Without storage the favorites hold for this window.
  }
  for (const listener of [...favoriteListeners]) listener();
}

/** Read the favorites again from the preferences (another window may have changed them; and the tests). */
export function forgetFavorites(): void {
  favoritesKept = null;
  for (const listener of [...favoriteListeners]) listener();
}

const subscribeFavorites = (listener: () => void) => { favoriteListeners.add(listener); return () => { favoriteListeners.delete(listener); }; };

/** The favorites, drawn again as they change. */
export function useFavoriteLuts(): ReadonlySet<string> {
  return useSyncExternalStore(subscribeFavorites, favoriteNames, favoriteNames);
}
