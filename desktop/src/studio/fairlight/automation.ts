import type { AutomationParameter, Lane, LanePoint, LanesOf, Time, Timeline } from '../engine';
export interface AutomationTarget { of: LanesOf; path: string }
export const ownerKey = (of: LanesOf): string => 'track' in of ? `track:${of.track}` : 'bus' in of ? `bus:${of.bus}` : `vca:${of.vca}`;
export function ownerOf(key: string): LanesOf { const [kind, ...id] = key.split(':'); return { [kind]: id.join(':') } as LanesOf; }
export const sampleTime = (sample: number, rate: number): Time => ({ ticks: Math.max(0, Math.round(sample)), scale: rate });
export const sampleAtPixel = (pixel: number, scale: number, timeline: Timeline): number => Math.max(0, Math.round(pixel / scale * timeline.rate.den / timeline.rate.num * timeline.sample_rate));
export const pixelAtSample = (sample: number, scale: number, timeline: Timeline): number => sample / timeline.sample_rate * timeline.rate.num / timeline.rate.den * scale;
export const tickOf = (at: Time, rate: number): number => Math.round(at.ticks / at.scale * rate);
export function laneValue(lane: Lane | undefined, seconds: number, fallback: number): number {
  if (!lane?.points.length) return fallback;
  const points = lane.points; let left = points[0];
  if (seconds <= left.at.ticks / left.at.scale) return left.value;
  for (const right of points.slice(1)) {
    const a = left.at.ticks / left.at.scale, b = right.at.ticks / right.at.scale;
    if (seconds < b) {
      if (lane.kind === 'switch') return left.value;
      const w = (seconds - a) / (b - a);
      return lane.scale === 'log' ? Math.exp(Math.log(left.value) * (1 - w) + Math.log(right.value) * w) : left.value * (1 - w) + right.value * w;
    }
    left = right;
  }
  return left.value;
}
export function valuePosition(value: number, parameter: AutomationParameter): number {
  const min=parameter.display_min??parameter.min, max=parameter.display_max??parameter.max, { scale }=parameter;
  const v = Math.max(min, Math.min(max, value));
  return scale === 'log' && min > 0 ? Math.log(v / min) / Math.log(max / min) : (v - min) / (max - min);
}
export function positionValue(position: number, parameter: AutomationParameter): number {
  const p = Math.max(0, Math.min(1, position)), min=parameter.display_min??parameter.min,max=parameter.display_max??parameter.max,{ scale, kind } = parameter;
  if (kind === 'switch') return p >= .5 ? 1 : 0;
  return scale === 'log' && min > 0 ? min * (max / min) ** p : min + p * (max - min);
}
/** Samples the pointer at <=100 Hz. RDP is solely the engine's lane_draw operation. */
export class Pencil {
  points: LanePoint[] = []; private clock = -Infinity;
  constructor(private rate: number) {}
  add(sample: number, value: number, now: number, force = false) {
    if (!force && now - this.clock < 10) return;
    this.clock = now;
    const at = sampleTime(sample, this.rate), previous = this.points.at(-1);
    if (previous) {
      // Backtracking replaces the newly painted interval instead of connecting across it.
      const a = Math.min(previous.at.ticks, at.ticks), b = Math.max(previous.at.ticks, at.ticks);
      this.points = this.points.filter(p => p.at.ticks < a || p.at.ticks > b);
      if (previous.at.ticks !== at.ticks) this.points.push(previous);
    }
    this.points.push({ at, value });
  }
  finish() { return [...new Map(this.points.map(p => [p.at.ticks, p])).values()].sort((a,b) => a.at.ticks-b.at.ticks); }
}
export const categories = ['fader','mute','pan','eq','comp','gate','lim','sends','plugins','misc'] as const;
export const defaultRecord = (): NonNullable<Timeline['audio']>['record'] => ({ mode: 'write', touch: 'off', on_stop: 'return', categories: Object.fromEntries(categories.map(c => [c,true])) as NonNullable<Timeline['audio']>['record']['categories'], preview: false, active_range: null });

/** Mirrors model Categories::covers (category routing only; parameters/ranges come from metadata). */
export function categoryOfPath(path:string):typeof categories[number] {
 if(path==='fader_db')return 'fader';if(path==='mute')return 'mute';if(path.startsWith('pan.'))return 'pan';if(path.startsWith('eq.'))return 'eq';
 if(path.startsWith('dynamics.expander.'))return 'gate';if(path.startsWith('dynamics.limiter.'))return 'lim';if(path.startsWith('dynamics.'))return 'comp';if(path.startsWith('send.'))return 'sends';
 if(path.startsWith('insert.')&&path.endsWith('.on'))return 'misc';if(path.startsWith('insert.')||path.startsWith('track_fx.'))return 'plugins';return 'misc';
}
