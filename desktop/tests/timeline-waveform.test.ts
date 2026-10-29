import { describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { EngineEvent } from '../src/studio/engine';
import { PeakData, sourceInterval, Waveforms } from '../src/studio/timeline/waveforms';
import { clip } from './timeline-fixture';

const encode = (values: number[]) => Buffer.from(new Int8Array(values).buffer).toString('base64');
const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
describe('timeline audio waveforms', () => {
  it('decodes signed, interleaved min/max bytes and preserves every peak at coarse zoom', () => {
    const data = new PeakData(100, 2, encode([-10, 20, -30, 40, -127, 100, -50, 60, -20, 30, -90, 127]));
    expect(data.range(0, 3, 0)).toEqual([-127, 100]);
    expect(data.range(0, 3, 1)).toEqual([-90, 127]);
    expect(data.range(1, 2, 1)).toEqual([-50, 60]);
    expect(data.range(5, 6, 1)).toEqual([0, 0]);
  });
  it('matches a brute-force peak envelope for fractional windows and non-power-of-two sizes', () => {
    const values = Array.from({ length: 206 }, (_, i) => i % 2 ? (i * 31) % 128 : -((i * 13) % 128));
    const data = new PeakData(100, 1, encode(values));
    for (let from = 0; from < 103; from += 1.7) {
      const to = Math.min(103, from + 30.3);
      const pairs = Array.from({ length: Math.ceil(to) - Math.floor(from) }, (_, i) => Math.floor(from) + i);
      expect(data.range(from, to, 0)).toEqual([Math.min(...pairs.map(i => values[i * 2])), Math.max(...pairs.map(i => values[i * 2 + 1]))]);
    }
  });
  it('uses source in, fractional speed and reverse for the visible portion of a clip', () => {
    const c = clip('a1', 100, 100, { source: { kind: 'media', media: 'm1', in: { ticks: 50, scale: 25 } }, speed: { num: 3, den: 2 } });
    expect(sourceInterval(c, { num: 25, den: 1 }, 100, 125)).toEqual([2, 3.5]);
    expect(sourceInterval({ ...c, reverse: true }, { num: 25, den: 1 }, 100, 125)).toEqual([6.5, 8]);
    expect(sourceInterval({ ...c, reverse: true }, { num: 25, den: 1 }, 175, 200)).toEqual([2, 3.5]);
  });
  it('deduplicates a pending source and only re-queries on a notification from its session', async () => {
    let notify!: (event: EngineEvent) => void;
    const off = vi.fn();
    const call = vi.fn().mockResolvedValueOnce({ ok: true, result: { pending: true } }).mockResolvedValue({ ok: true, result: { rate: 100, channels: 1, peaks: encode([-2, 3]) } });
    const cache = new Waveforms({ call, onEvent: listener => { notify = listener; return off; } } as Pick<EngineBridge, 'call' | 'onEvent'>, 3);
    cache.connect();
    const changed = vi.fn();
    const stop = cache.subscribe('m1', changed);
    const stop2 = cache.subscribe('m1', vi.fn());
    await settle();
    expect(call).toHaveBeenCalledTimes(1);
    expect(cache.get('m1').state).toBe('pending');
    notify({ kind: 'notification', session: 2, method: 'media.waveform', params: { media: 'm1' } });
    await settle();
    expect(call).toHaveBeenCalledTimes(1);
    notify({ kind: 'notification', session: 3, method: 'media.waveform', params: { media: 'm1' } });
    await settle();
    expect(call).toHaveBeenCalledTimes(2);
    expect(cache.get('m1').state).toBe('ready');
    expect(changed).toHaveBeenCalled();
    stop(); stop2(); cache.disconnect();
    expect(off).toHaveBeenCalledOnce();
  });
  it('does not miss a completion notification arriving before the pending response', async () => {
    let notify!: (event: EngineEvent) => void;
    let finish!: (answer: unknown) => void;
    const call = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
      .mockResolvedValue({ ok: true, result: { rate: 100, channels: 1, peaks: encode([-4, 5]) } });
    const cache = new Waveforms({ call, onEvent: listener => { notify = listener; return () => {}; } } as Pick<EngineBridge, 'call' | 'onEvent'>, 1);
    cache.connect(); cache.subscribe('m1', () => {});
    notify({ kind: 'notification', session: 1, method: 'media.waveform', params: { media: 'm1' } });
    finish({ ok: true, result: { pending: true } });
    await settle();
    expect(call).toHaveBeenCalledTimes(2);
    expect(cache.get('m1').state).toBe('ready');
    cache.disconnect();
  });

  it('evicts completed offscreen analyses after scrolling away while they were in flight', async () => {
    const replies: ((answer: unknown) => void)[] = [];
    const call = vi.fn().mockImplementation(() => new Promise(resolve => replies.push(resolve)));
    const cache = new Waveforms({ call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>, 1);
    cache.connect();
    for (let i = 0; i < 40; i++) cache.subscribe(`m${i}`, () => {})();
    for (const reply of replies) reply({ ok: true, result: { rate: 100, channels: 1, peaks: encode([-2, 3]) } });
    await settle();
    cache.subscribe('m0', () => {});
    expect(call).toHaveBeenCalledTimes(41);
    cache.disconnect();
  });
});
