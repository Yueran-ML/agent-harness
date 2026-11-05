import { useState } from 'react';
import type { EngineBridge } from '../../bridge';
export interface FairlightView {
  pool: boolean; effects: boolean; index: boolean; groups: boolean; mixer: boolean; meters: boolean; inspector: boolean;
  video: boolean; automation: boolean; viewer: boolean; waveform: number; hidden: string[]; gainLine: boolean;
  leftWidth: number; rightWidth: number; monitoringHeight: number; mixerHeight: number;
}
export const defaultFairlightView = (): FairlightView => ({ pool: false, effects: false, index: false, groups: false, mixer: true, meters: true, inspector: true,
  video: false, automation: false, viewer: true, waveform: 1, hidden: [], gainLine: true, leftWidth: 340, rightWidth: 280, monitoringHeight: 240, mixerHeight: 88 });
// Per engine/session/timeline: returning from Edit retains this layout; a new project session does not.
const views = new WeakMap<object, Map<string, FairlightView>>();
export function useFairlightView(bridge: Pick<EngineBridge, 'call'>, session: number, timeline: string) {
  let stored = views.get(bridge); if (!stored) { stored = new Map(); views.set(bridge, stored); }
  const key = `${session}/${timeline}`;
  const [view, setView] = useState(() => stored!.get(key) ?? defaultFairlightView());
  const update = (patch: Partial<FairlightView>) => setView(old => { const next = { ...old, ...patch }; stored!.set(key, next); return next; });
  return [view, update, () => update(defaultFairlightView())] as const;
}
