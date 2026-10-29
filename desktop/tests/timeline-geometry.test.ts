import { describe, expect, it } from 'vitest';
import { frameAt, pixelAt, visible, fitZoom, zoomAt, rulerTicks, transitionSpan, snapPoints, duration } from '../src/studio/timeline/geometry';
import { clip, timeline } from './timeline-fixture';

describe('timeline geometry', () => {
  it('rounds a pointer to the nearest whole frame, with scroll and fractional zoom', () => {
    for (const scale of [0.03, 0.8, 4, 32]) for (const frame of [0, 1, 12345]) {
      expect(frameAt(pixelAt(frame, scale, 83), scale, 83)).toBe(frame);
    }
    expect(frameAt(-100, 2, 10)).toBe(0);
    expect(frameAt(3, 2)).toBe(2);
  });
  it('only returns clips intersecting the half-open visible range, including a long clip starting offscreen', () => {
    const items = [clip('long', 0, 500), clip('edge', 500, 20), clip('later', 600, 10)];
    expect(visible(items, 200, 500).map(c => c.id)).toEqual(['long']);
    expect(visible(items, 500, 600).map(c => c.id)).toEqual(['edge']);
    expect(visible(items, 520, 600)).toEqual([]);
    expect(visible(items, 600, 600)).toEqual([]);
  });
  it('matches a linear visibility oracle across gaps and different zoom windows', () => {
    const items = Array.from({ length: 1000 }, (_, i) => clip(`c${i}`, i * 30, 20));
    for (let from = 0; from < 30000; from += 137) {
      expect(visible(items, from, from + 71)).toEqual(items.filter(c => c.start < from + 71 && c.start + c.length > from));
    }
  });
  it('fits the entire duration and zooms about the pointer without moving its frame', () => {
    expect(fitZoom(1000, 1040)).toBe(1);
    expect(fitZoom(0, 800)).toBeGreaterThan(0);
    const next = zoomAt(2, 400, 150, 2);
    expect(next.scale).toBe(4);
    expect((150 + next.scroll) / next.scale).toBe((150 + 400) / 2);
  });
  it('chooses readable integer-frame ticks at fractional frame rates', () => {
    for (const rate of [{ num: 25, den: 1 }, { num: 30000, den: 1001 }]) {
      for (const scale of [0.02, 0.3, 2, 24]) {
        const ticks = rulerTicks(123, 123 + 1000 / scale, scale, rate);
        const labels = ticks.filter(tick => tick.major);
        expect(ticks.every(tick => Number.isInteger(tick.frame))).toBe(true);
        expect(labels.length).toBeGreaterThan(0);
        expect(labels.length).toBeLessThan(15);
        for (let i = 1; i < labels.length; i++) expect((labels[i].frame - labels[i - 1].frame) * scale).toBeGreaterThanOrEqual(80);
      }
    }
  });
  it('draws transition handles around the cut with the engine odd-frame split', () => {
    const base = { id: 'x1', kind: 'dissolve' as const, left: 'c1', right: 'c2', length: 5 };
    expect(transitionSpan({ ...base, align: 'center' }, 100)).toEqual({ start: 98, length: 5 });
    expect(transitionSpan({ ...base, align: 'start' }, 100)).toEqual({ start: 100, length: 5 });
    expect(transitionSpan({ ...base, align: 'end' }, 100)).toEqual({ start: 95, length: 5 });
  });
  it('counts disabled and locked content and exposes unique sorted clip and cue boundaries', () => {
    const t = timeline([clip('c1', 0, 20), clip('c2', 30, 20)]);
    t.tracks.push({ id: 's1', kind: 'subtitle', name: 'English', language: 'en', enabled: false, locked: true, cues: [{ id: 'q1', start: 20, length: 100, text: 'Hello' }] });
    expect(duration(t)).toBe(120);
    expect(snapPoints(t)).toEqual([0, 20, 30, 50, 120]);
  });
});
