import { useSyncExternalStore } from 'react';
import type { Effect } from '../engine';
const KEY = 'columbina.fairlight.fx-favorites';
const listeners = new Set<() => void>();
let cached: string | null = null, snapshot: Effect['kind'][] = [];
const kinds = new Set(['gain', 'limiter', 'soft_clipper', 'stereo_width', 'de_esser', 'de_hummer', 'noise_reduction', 'pitch']);
function read() {
  let raw: string | null = null; try { raw = localStorage.getItem(KEY); } catch { /* Session favourites still work without storage. */ }
  if (raw !== cached) {
    cached = raw;
    try { const value: unknown = JSON.parse(raw ?? '[]'); snapshot = Array.isArray(value) ? value.filter((v): v is Effect['kind'] => typeof v === 'string' && kinds.has(v)) : []; } catch { snapshot = []; }
  }
  return snapshot;
}
function subscribe(listener: () => void) {
  listeners.add(listener); window.addEventListener('storage', listener);
  return () => { listeners.delete(listener); window.removeEventListener('storage', listener); };
}
export function useEffectFavorites() {
  const favorites = useSyncExternalStore(subscribe, read, read);
  const toggle = (kind: Effect['kind']) => {
    const current = read(); snapshot = current.includes(kind) ? current.filter(k => k !== kind) : [...current, kind];
    cached = JSON.stringify(snapshot); try { localStorage.setItem(KEY, cached); } catch { cached = null; }
    listeners.forEach(listener => listener());
  };
  return [favorites, toggle] as const;
}
