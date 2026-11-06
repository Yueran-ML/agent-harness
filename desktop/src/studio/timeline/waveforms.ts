import type { EngineBridge } from '../../bridge';
import { callEngine, type ChannelSource, type Clip, type Rate } from '../engine';

/** Min/max pyramid: zoomed-out pixels retain peaks without scanning the whole source on every scroll. */
export class PeakData {
  readonly levels: Int8Array[];
  readonly bins: number;
  get bytes(): number { return this.levels.reduce((sum, level) => sum + level.byteLength, 0); }
  constructor(readonly rate: number, readonly channels: number, encoded: string) {
    const bytes = Int8Array.from(atob(encoded), char => char.charCodeAt(0));
    if (rate <= 0 || channels <= 0 || bytes.length % (channels * 2) !== 0) throw new Error('Invalid waveform dimensions.');
    this.bins = bytes.length / (channels * 2);
    this.levels = [bytes];
    let previous = bytes;
    let count = this.bins;
    while (count > 1) {
      const nextCount = Math.ceil(count / 2);
      const next = new Int8Array(nextCount * channels * 2);
      for (let bin = 0; bin < nextCount; bin++) for (let channel = 0; channel < channels; channel++) {
        const a = (bin * 2 * channels + channel) * 2;
        const b = ((Math.min(bin * 2 + 1, count - 1)) * channels + channel) * 2;
        const into = (bin * channels + channel) * 2;
        next[into] = Math.min(previous[a], previous[b]);
        next[into + 1] = Math.max(previous[a + 1], previous[b + 1]);
      }
      this.levels.push(next);
      previous = next;
      count = nextCount;
    }
  }
  range(from: number, to: number, channel: number): [number, number] {
    const out: [number, number] = [0, 0];
    this.extent(from, to, channel, out);
    return out;
  }
  /** The least and the greatest peak between two bins, written into `out`; [0, 0] where there are none. */
  extent(from: number, to: number, channel: number, out: [number, number] | Int16Array): void {
    let first = Math.max(0, Math.floor(from));
    const end = Math.min(this.bins, Math.ceil(to));
    if (first >= end) { out[0] = 0; out[1] = 0; return; }
    let min = 127;
    let max = -128;
    while (first < end) {
      let level = 31 - Math.clz32(end - first);
      while (level > 0 && (first & ((1 << level) - 1)) !== 0) level--;
      const offset = ((first >> level) * this.channels + channel) * 2;
      const values = this.levels[level];
      if (values[offset] < min) min = values[offset];
      if (values[offset + 1] > max) max = values[offset + 1];
      first += 1 << level;
    }
    out[0] = min;
    out[1] = max;
  }
}

/** A display interval maps to the continuous source interval, in seconds; reverse swaps its ends. */
export function sourceInterval(clip: Clip, rate: Rate, from: number, to: number): [number, number] {
  const start = clip.source.kind === 'media' ? clip.source.in.ticks / clip.source.in.scale : 0;
  const step = rate.den / rate.num * clip.speed.num / clip.speed.den;
  const a = Math.max(0, from - clip.start);
  const b = Math.min(clip.length, to - clip.start);
  return clip.reverse ? [start + (clip.length - b) * step, start + (clip.length - a) * step] : [start + a * step, start + b * step];
}

export type WaveState = { state: 'loading' | 'pending' } | { state: 'ready'; data: PeakData } | { state: 'failed'; message: string };
interface Entry { media: string; stream?: number; mix?: ChannelSource[]; value: WaveState; listeners: Set<() => void>; running: boolean; again: boolean }
const waveformKey = (media: string, stream?: number, mix?: readonly ChannelSource[]) => JSON.stringify([media, stream ?? null, mix?.map(source => [source.stream, source.channel]) ?? null]);

/** One cache per project session. Completion notifications can precede the pending response. */
export class Waveforms {
  private readonly entries = new Map<string, Entry>();
  private off: (() => void) | null = null;
  private epoch = 0;
  private active = true;
  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent'>, private readonly session: number) {}
  connect(): void {
    if (this.off) return;
    this.active = true;
    this.off = this.bridge.onEvent(event => {
      if (event.kind !== 'notification' || event.session !== this.session || event.method !== 'media.waveform') return;
      const { media: id, stream, mix } = event.params as { media?: string; stream?: number; mix?: ChannelSource[] };
      const entry = id ? this.entries.get(waveformKey(id, stream, mix)) : null;
      if (!id || !entry) return;
      if (entry.running) entry.again = true;
      else this.fetch(entry);
    });
    for (const entry of this.entries.values()) if (entry.listeners.size && !entry.running && entry.value.state !== 'ready') this.fetch(entry);
  }
  disconnect(): void {
    this.off?.(); this.off = null; this.active = false; this.epoch++;
    for (const entry of this.entries.values()) entry.running = false;
  }
  private entry(id: string, stream?: number, mix?: readonly ChannelSource[]): Entry {
    const key = waveformKey(id, stream, mix);
    let entry = this.entries.get(key);
    if (!entry) { entry = { media: id, stream, ...(mix ? { mix: mix.map(source => ({ ...source })) } : {}), value: { state: 'loading' }, listeners: new Set(), running: false, again: false }; this.entries.set(key, entry); }
    return entry;
  }
  get(id: string, stream?: number, mix?: readonly ChannelSource[]): WaveState { return this.entry(id, stream, mix).value; }
  subscribe(id: string, listener: () => void, stream?: number, mix?: readonly ChannelSource[]): () => void {
    const entry = this.entry(id, stream, mix);
    entry.listeners.add(listener);
    if (entry.value.state === 'loading' || entry.value.state === 'failed') this.fetch(entry);
    return () => {
      entry.listeners.delete(listener);
      this.trim();
    };
  }
  private trim(): void {
    let bytes = [...this.entries.values()].reduce((sum, entry) => sum + (entry.value.state === 'ready' ? entry.value.data.bytes : 0), 0);
    // Visible waveforms remain resident; old, offscreen analyses do not accumulate after a fast scroll.
    for (const [id, entry] of this.entries) {
      if (this.entries.size <= 32 && bytes <= 32 * 1024 * 1024) break;
      if (entry.listeners.size === 0 && !entry.running) {
        if (entry.value.state === 'ready') bytes -= entry.value.data.bytes;
        this.entries.delete(id);
      }
    }
  }
  private fetch(entry: Entry): void {
    if (!this.active || entry.running) return;
    entry.running = true;
    entry.again = false;
    const epoch = this.epoch;
    void callEngine(this.bridge, 'media.waveform', { media: entry.media, ...(entry.stream === undefined ? {} : { stream: entry.stream }), ...(entry.mix ? { mix: entry.mix } : {}) }, { expectedSession: this.session }).then(answer => {
      if (epoch !== this.epoch) return;
      entry.value = 'pending' in answer ? { state: 'pending' } : { state: 'ready', data: new PeakData(answer.rate, answer.channels, answer.peaks) };
    }).catch(error => {
      if (epoch === this.epoch) entry.value = { state: 'failed', message: error instanceof Error ? error.message : String(error) };
    }).finally(() => {
      if (epoch !== this.epoch) return;
      entry.running = false;
      for (const listener of entry.listeners) listener();
      if (entry.again) this.fetch(entry);
      this.trim();
    });
  }
}
