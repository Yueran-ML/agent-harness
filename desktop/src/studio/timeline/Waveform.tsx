import { memo, useCallback, useDeferredValue, useMemo, useSyncExternalStore } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Rate } from '../engine';
import type { PeakData, Waveforms } from './waveforms';

/** How wide a tile of a waveform is, in CSS pixels. */
export const TILE = 512;

/** A number of pixels as a path writes it, to a tenth. */
const tenth = (value: number) => Math.round(value * 10) / 10;

function paths(data: PeakData, clip: Clip, rate: Rate, from: number, to: number, width: number): string[] {
  const columns = Math.max(1, Math.ceil(width / 2));
  const shapes = Array.from({ length: data.channels }, () => '');
  const half = 30 / data.channels;
  const extent = new Int16Array(2);
  // A column of pixels is a span of frames of the clip, and so a span of the source, in bins of peaks.
  const begins = clip.source.kind === 'media' ? clip.source.in.ticks / clip.source.in.scale : 0;
  const step = rate.den / rate.num * clip.speed.num / clip.speed.den;
  const bin = (frame: number) => {
    const into = Math.max(0, Math.min(clip.length, frame - clip.start));
    return (clip.reverse ? begins + (clip.length - into) * step : begins + into * step) * data.rate;
  };
  for (let column = 0; column < columns; column++) {
    let a = bin(from + (to - from) * column / columns);
    let b = bin(from + (to - from) * (column + 1) / columns);
    if (a > b) [a, b] = [b, a];
    const x = tenth((column + 0.5) * width / columns);
    for (let channel = 0; channel < data.channels; channel++) {
      data.extent(a, b, channel, extent);
      const center = (channel + 0.5) * 60 / data.channels + 2;
      shapes[channel] += `M${x},${tenth(center - extent[1] / 127 * half)}V${tenth(center - extent[0] / 127 * half)}`;
    }
  }
  return shapes;
}

/** A number for each object that a tile is drawn from, for the keys of the tiles kept. */
const numbers = new WeakMap<object, number>();
let counted = 0;
const numberOf = (thing: object) => {
  let number = numbers.get(thing);
  if (number === undefined) numbers.set(thing, number = ++counted);
  return number;
};
/** How many tiles are kept, at most: some megabytes of paths. */
const KEPT = 512;
/** The shapes of tiles drawn, the last drawn last: a zoom back, or a scroll back, finds them here. */
const kept = new Map<string, string[]>();
function tileShapes(data: PeakData, clip: Clip, rate: Rate, scale: number, index: number, width: number): string[] {
  const key = `${numberOf(data)}:${numberOf(clip)}:${rate.num}/${rate.den}:${scale}:${index}`;
  let shapes = kept.get(key);
  if (shapes) kept.delete(key);
  else {
    const left = index * TILE;
    shapes = paths(data, clip, rate, clip.start + left / scale, clip.start + (left + width) / scale, width);
    if (kept.size >= KEPT) kept.delete(kept.keys().next().value!);
  }
  kept.set(key, shapes);
  return shapes;
}

/**
 * One tile of a waveform: the `index`th TILE pixels of the clip, at a zoom. Its shape is worked out
 * once for its peaks, clip and zoom, and kept a while: a tile already drawn stays as the window
 * moves past it, and is found again at a zoom that it was drawn at before.
 */
const WaveTile = memo(function WaveTile({ data, media, clip, rate, scale, index }: {
  data: PeakData; media: string; clip: Clip; rate: Rate; scale: number; index: number;
}) {
  const left = index * TILE;
  const width = Math.max(1, Math.min(TILE, clip.length * scale - left));
  const drawn = useMemo(() => tileShapes(data, clip, rate, scale, index, width), [data, clip, rate, scale, index, width]);
  return <svg className="timeline-waveform" viewBox={`0 0 ${width} 64`} preserveAspectRatio="none" aria-hidden="true"
    style={{ left, width }} data-waveform={media} data-tile={index}>
    {drawn.map((path, at) => <path key={at} d={path} />)}
  </svg>;
});

/** What a waveform shows before its first tiles: none. */
const NOTHING = { from: 0, to: 0, scale: 1 };

/**
 * The waveform of a clip of sound, where it lies in the window `from`–`to` (frames): fixed tiles of
 * the clip, those the window covers. Scrolling mounts only the tiles that come into the window.
 *
 * New tiles are worked out after the scroll or the zoom has been drawn (a deferred value): until
 * then the tiles drawn before stay, stretched to the new zoom, so that neither waits for them.
 */
export const Waveform = memo(function Waveform({ cache, media, clip, rate, from, to, scale }: {
  cache: Waveforms; media: string; clip: Clip; rate: Rate; from: number; to: number; scale: number;
}) {
  const words = useWords().label.studio.timeline;
  const subscribe = useCallback((listener: () => void) => cache.subscribe(media, listener), [cache, media]);
  const snapshot = useCallback(() => cache.get(media), [cache, media]);
  const state = useSyncExternalStore(subscribe, snapshot, snapshot);
  const wanted = useMemo(() => ({ from, to, scale }), [from, to, scale]);
  // A clip that comes into the window draws its tiles after it is drawn itself.
  const shown = useDeferredValue(wanted, NOTHING);
  if (state.state !== 'ready') return <span className="timeline-wave-status" title={state.state === 'failed' ? state.message : undefined}>
    {state.state === 'failed' ? words.waveFailed : words.wavePending}
  </span>;
  const total = clip.length * shown.scale;
  // (Nothing is shown before the first tiles are worked out: no tile lies between 0 and 0.)
  const first = Math.max(0, Math.floor((shown.from - clip.start) * shown.scale / TILE));
  const last = Math.min(Math.ceil(total / TILE), Math.ceil((shown.to - clip.start) * shown.scale / TILE));
  const tiles = [];
  for (let index = first; index < last; index++) tiles.push(index);
  const stretch = shown === NOTHING ? 1 : scale / shown.scale;
  return <div className="timeline-wave-tiles" style={stretch === 1 ? undefined : { transform: `scaleX(${stretch})` }}>
    {tiles.map(index => <WaveTile key={index} data={state.data} media={media} clip={clip} rate={rate} scale={shown.scale} index={index} />)}
  </div>;
});
