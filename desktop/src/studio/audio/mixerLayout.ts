import { useSyncExternalStore } from 'react';
import type { Timeline } from '../engine';
export const MIXER_DIVIDER = 'divider';
export const MIXER_SECTIONS = ['input', 'trackfx', 'order', 'effects', 'effectsIn', 'dynamics', 'eq', 'sends', 'pan', 'outputs', 'vca'] as const;
export interface MixerLayout { small: boolean; labels: boolean; single: boolean; hidden: string[]; order: string[] }
export const defaultMixerLayout = (): MixerLayout => ({ small: false, labels: true, single: false, hidden: [], order: [] });
type Store = { value: MixerLayout; listeners: Set<() => void> };
const stores = new WeakMap<object, Map<string, Store>>();
export function useMixerLayout(bridge: object, session: number, timeline: string) {
  let group = stores.get(bridge); if (!group) { group = new Map(); stores.set(bridge, group); }
  const key = `${session}/${timeline}`; let store = group.get(key); if (!store) { store = { value: defaultMixerLayout(), listeners: new Set() }; group.set(key, store); }
  const current = store;
  const layout = useSyncExternalStore(listener => { current.listeners.add(listener); return () => { current.listeners.delete(listener); }; }, () => current.value);
  return [layout, (patch: Partial<MixerLayout>) => { current.value = { ...current.value, ...patch }; current.listeners.forEach(listener => listener()); }] as const;
}
export function mixerOrder(timeline: Timeline, layout: MixerLayout) {
  const natural = [...timeline.tracks.filter(t => t.kind === 'audio').map(t => `track:${t.id}`), MIXER_DIVIDER, ...(timeline.buses ?? []).map(b => `bus:${b.id}`), ...(timeline.vcas ?? []).map(v => `vca:${v.id}`)];
  if (!layout.order.length) return natural;
  const known = layout.order.filter(key => natural.includes(key));
  const tracks = natural.filter(key => key.startsWith('track:') && !known.includes(key));
  known.splice(Math.max(0, known.indexOf(MIXER_DIVIDER)), 0, ...tracks);
  const result = [...known, ...natural.filter(key => !known.includes(key))];
  // Project edits and Undo remain authoritative for audio tracks, even after view rearrangement.
  const audio = natural.filter(key => key.startsWith('track:')); let index = 0;
  return result.map(key => key.startsWith('track:') ? audio[index++] : key);
}
export function moveMixerItems(order: string[], selected: readonly string[], before: string): string[] {
  if (selected.includes(before)) return order;
  const moving = order.filter(key => selected.includes(key)), rest = order.filter(key => !selected.includes(key));
  const at = rest.indexOf(before); if (at < 0 || !moving.length) return order;
  rest.splice(at, 0, ...moving); return rest;
}
export function resetBusOrder(timeline: Timeline, layout: MixerLayout) {
  const order = mixerOrder(timeline, layout).filter(key => !key.startsWith('bus:'));
  order.splice(order.indexOf(MIXER_DIVIDER) + 1, 0, ...(timeline.buses ?? []).map(bus => `bus:${bus.id}`)); return order;
}
