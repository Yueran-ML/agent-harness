import type { Rate, Timeline, Transition } from '../engine';
import { nominalFps } from '../timecode';

export interface Span { start: number; length: number }
export const HEADER_WIDTH = 156;
export const RULER_HEIGHT = 30;
export const pixelAt = (frame: number, scale: number, scroll = 0): number => frame * scale - scroll;
export const frameAt = (pixel: number, scale: number, scroll = 0): number => Math.max(0, Math.round((pixel + scroll) / scale));

/** Engine clips and cues are sorted and do not overlap within their track. */
export function visible<T extends Span>(items: readonly T[], from: number, to: number): T[] {
  if (to <= from) return [];
  let low = 0;
  let high = items.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    if (items[mid].start + items[mid].length <= from) low = mid + 1;
    else high = mid;
  }
  const first = low;
  while (low < items.length && items[low].start < to) low++;
  return items.slice(first, low);
}

export function duration(timeline: Timeline): number {
  return Math.max(0, ...timeline.tracks.map(track => {
    const last = (track.kind === 'subtitle' ? track.cues : track.clips).at(-1);
    return last ? last.start + last.length : 0;
  }));
}

export const fitZoom = (frames: number, width: number): number => Math.min(64, Math.max(1, width - 40) / Math.max(1, frames));
export function zoomAt(scale: number, scroll: number, anchor: number, factor: number): { scale: number; scroll: number } {
  const next = Math.max(0.0001, Math.min(64, scale * factor));
  return { scale: next, scroll: Math.max(0, (scroll + anchor) / scale * next - anchor) };
}

/** NDF labels use nominal seconds, with whole-frame intervals at every zoom. */
export function rulerTicks(from: number, to: number, scale: number, rate: Rate): { frame: number; major: boolean }[] {
  const fps = nominalFps(rate);
  const desired = 90 / scale;
  const options = [...new Set([1, 2, 5, 10, ...[1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600].map(n => n * fps)])].sort((a, b) => a - b);
  const major = options.find(n => n >= desired) ?? Math.ceil(desired / (3600 * fps)) * 3600 * fps;
  const minor = major % 5 === 0 ? major / 5 : major % 2 === 0 ? major / 2 : major;
  const ticks = [];
  for (let frame = Math.max(0, Math.ceil(from / minor) * minor); frame < to; frame += minor) ticks.push({ frame, major: frame % major === 0 });
  return ticks;
}

/** The right clip begins at cut; odd centered lengths leave the extra frame after it. */
export function transitionSpan(transition: Transition, cut: number): Span {
  const before = transition.align === 'start' ? 0 : transition.align === 'end' ? transition.length : Math.floor(transition.length / 2);
  return { start: cut - before, length: transition.length };
}

export function snapPoints(timeline: Timeline): number[] {
  const points = new Set([0]);
  for (const track of timeline.tracks) for (const item of track.kind === 'subtitle' ? track.cues : track.clips) {
    points.add(item.start);
    points.add(item.start + item.length);
  }
  return [...points].sort((a, b) => a - b);
}
