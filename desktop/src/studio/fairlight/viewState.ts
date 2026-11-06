import { useCallback, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { TimelineViewport, TimelineScrolling } from '../timeline/Timeline';
export interface FairlightView {
  pool: boolean; effects: boolean; index: boolean; groups: boolean; mixer: boolean; meters: boolean; inspector: boolean;
  automationControls: boolean; pencil: boolean; lanePaths: Record<string,string>; lastLanePaths: Record<string,string>;
  video: boolean; automation: boolean; viewer: boolean; waveform: number; hidden: string[]; gainLine: boolean;
  leftWidth: number; rightWidth: number; monitoringHeight: number; mixerHeight: number;
  timelineViewport: TimelineViewport; scrolling: TimelineScrolling;
}
export const defaultFairlightView = (): FairlightView => ({ pool: false, effects: false, index: false, groups: false, mixer: true, meters: true, inspector: true,
  automationControls: false, pencil: false, lanePaths: {}, lastLanePaths: {},
  video: false, automation: false, viewer: true, waveform: 1, hidden: [], gainLine: true, leftWidth: 340, rightWidth: 280, monitoringHeight: 240, mixerHeight: 300,
  timelineViewport: { scale: 4, left: 0, top: 0, heights: {} }, scrolling: 'page' });
export type FairlightViewUpdate = (patch: Partial<FairlightView> | ((old: FairlightView) => Partial<FairlightView>)) => void;
// Per engine/session/timeline: returning from Edit retains this layout; a new project session does not.
const views = new WeakMap<object, Map<string, FairlightView>>();
export function useFairlightView(bridge: Pick<EngineBridge, 'call'>, session: number, timeline: string) {
  let stored = views.get(bridge); if (!stored) { stored = new Map(); views.set(bridge, stored); }
  const key = `${session}/${timeline}`;
  const [view, setView] = useState(() => stored!.get(key) ?? defaultFairlightView());
  const update: FairlightViewUpdate = useCallback(patch => setView(old => { const next = { ...old, ...(typeof patch === 'function' ? patch(old) : patch) }; stored!.set(key, next); return next; }), [stored, key]);
  return [view, update, () => update(defaultFairlightView())] as const;
}
