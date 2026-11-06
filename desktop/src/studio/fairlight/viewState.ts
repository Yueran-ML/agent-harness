import { useCallback, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { TimelineViewport, TimelineScrolling } from '../timeline/Timeline';
export interface MonitoringView { rows: 1 | 2; wide: boolean; tracksWidth: number; busesWidth: number; loudnessWidth: number; viewerWidth: number }
export const defaultMonitoringView = (): MonitoringView => ({ rows: 1, wide: true, tracksWidth: 220, busesWidth: 140, loudnessWidth: 300, viewerWidth: 300 });
/**
 * Lock Track Height To (Reference Manual 20, pp. 3670, 3716). The manual gives no pixel heights; the
 * reference assumes 24 / 40 / 64 / 96 / 144, and Custom keeps the height the track has.
 */
export type HeightPreset = 'micro' | 'mini' | 'medium' | 'large' | 'extraLarge' | 'custom';
export const HEIGHT_PRESETS: readonly (readonly [Exclude<HeightPreset, 'custom'>, number])[] = [
  ['micro', 24], ['mini', 40], ['medium', 64], ['large', 96], ['extraLarge', 144],
];
/** Where Up and Down go: the next clip's edges, or the next fade's ends (View Options › Jump to). */
export type JumpTarget = 'clip' | 'fade';
export interface FairlightView {
  pool: boolean; effects: boolean; index: boolean; groups: boolean; mixer: boolean; meters: boolean; inspector: boolean;
  automationControls: boolean; pencil: boolean; lanePaths: Record<string,string>; lastLanePaths: Record<string,string>;
  video: boolean; automation: boolean; viewer: boolean; waveform: number; hidden: string[]; gainLine: boolean;
  leftWidth: number; rightWidth: number; monitoringHeight: number; mixerHeight: number;
  timelineViewport: TimelineViewport; scrolling: TimelineScrolling;
  monitoring: MonitoringView;
  /** Tracks whose height is locked, and to which preset. */
  heightLocks: Record<string, HeightPreset>;
  /** Waveforms over the whole clip, its name bar hidden (View Options › Full Waveforms). */
  fullWaveforms: boolean;
  /** An outline drawn round each waveform (View Options › Waveform Borders). */
  waveformBorders: boolean;
  /** The waveform zoom of single tracks, times the zoom of all of them (`waveform`). */
  trackWaveform: Record<string, number>;
  jumpTo: JumpTarget;
  /** The heights and scroll that Smart Zoom replaced, to go back to when it is pressed again. */
  smartZoom: Pick<TimelineViewport, 'heights' | 'top'> | null;
}
export const defaultFairlightView = (): FairlightView => ({ pool: false, effects: false, index: false, groups: false, mixer: true, meters: true, inspector: true,
  automationControls: false, pencil: false, lanePaths: {}, lastLanePaths: {},
  video: false, automation: false, viewer: true, waveform: 1, hidden: [], gainLine: true, leftWidth: 340, rightWidth: 280, monitoringHeight: 240, mixerHeight: 300,
  timelineViewport: { scale: 4, left: 0, top: 0, heights: {} }, scrolling: 'page', monitoring: defaultMonitoringView(),
  heightLocks: {}, fullWaveforms: false, waveformBorders: false, trackWaveform: {}, jumpTo: 'clip', smartZoom: null });
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
