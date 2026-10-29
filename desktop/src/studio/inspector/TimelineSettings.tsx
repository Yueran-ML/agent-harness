import { useWords } from '../../app/locale';
import type { Op, Rate, Timeline } from '../engine';
import { rateText } from '../timecode';
import { ChoiceField, NumberField, TextField } from './fields';
import type { Commit } from './useCommit';

/** Sizes of picture to choose from, as DaVinci Resolve lists them; others are typed. */
const SIZES: readonly [number, number][] = [[1280, 720], [1920, 1080], [2560, 1440], [3840, 2160], [4096, 2160], [1080, 1920], [1080, 1080]];
/** Frame rates to choose from. */
const RATES: readonly Rate[] = [
  { num: 24000, den: 1001 }, { num: 24, den: 1 }, { num: 25, den: 1 }, { num: 30000, den: 1001 }, { num: 30, den: 1 },
  { num: 48, den: 1 }, { num: 50, den: 1 }, { num: 60000, den: 1001 }, { num: 60, den: 1 },
];
/** The sample rates that the engine takes. */
const SAMPLE_RATES = [44100, 48000, 96000] as const;
/** A size that the engine takes: even, from 16 to 16384 pixels. */
const even = (pixels: number) => Math.min(16384, Math.max(16, Math.floor(pixels / 2) * 2));
const rateKey = (rate: Rate) => `${rate.num}/${rate.den}`;

/**
 * The settings of the timeline: its name, the size of its picture, its frame rate and the rate of
 * its sound, each sent as one change. Where the timeline has clips, the frame rate says what
 * changing it does, and a change that would take a clip past its media is refused with a sentence
 * of its own.
 */
export function TimelineSettings({ timeline, commit }: { timeline: Timeline; commit: Commit }) {
  const words = useWords();
  const labels = words.label.studio.inspector;
  const hints = words.hint.studio.inspector;
  const set = (change: Omit<Extract<Op, { op: 'set_timeline' }>, 'op' | 'id'>) =>
    void commit(() => [{ op: 'set_timeline', id: timeline.id, ...change }], { overrides: { out_of_source: hints.rateRefused } });
  const hasClips = timeline.tracks.some(track => track.kind === 'subtitle' ? track.cues.length > 0 : track.clips.length > 0);
  const size = SIZES.find(([width, height]) => width === timeline.width && height === timeline.height);
  const rates = RATES.some(rate => rateKey(rate) === rateKey(timeline.rate)) ? RATES : [...RATES, timeline.rate];
  return <div className="inspector-panel" data-panel="timeline">
    <TextField name="name" label={labels.name} value={timeline.name} onCommit={name => set({ name: name.trim() })} />
    <ChoiceField name="resolution" label={labels.resolution} value={size ? `${size[0]}x${size[1]}` : 'custom'}
      options={[...SIZES.map(([width, height]) => ({ value: `${width}x${height}`, label: `${width} × ${height}` })), { value: 'custom', label: labels.custom }]}
      onCommit={value => {
        if (value === 'custom') return;
        const [width, height] = value.split('x').map(Number);
        set({ width, height });
      }} />
    <NumberField name="width" label={labels.width} value={timeline.width} fallback={1920} unit="px" min={16} max={16384} digits={0} scrub={2} fit={even}
      onCommit={width => set({ width })} />
    <NumberField name="height" label={labels.height} value={timeline.height} fallback={1080} unit="px" min={16} max={16384} digits={0} scrub={2} fit={even}
      onCommit={height => set({ height })} />
    <ChoiceField name="rate" label={labels.frameRate} value={rateKey(timeline.rate)}
      options={rates.map(rate => ({ value: rateKey(rate), label: words.label.studio.fps(rateText(rate)) }))}
      onCommit={value => {
        const [num, den] = value.split('/').map(Number);
        set({ rate: { num, den } });
      }} />
    {hasClips && <p className="caption inspector-note">{hints.rate}</p>}
    <ChoiceField name="sample_rate" label={labels.sampleRate} value={String(timeline.sample_rate)}
      options={SAMPLE_RATES.map(rate => ({ value: String(rate), label: `${rate / 1000} kHz` }))}
      onCommit={value => set({ sample_rate: Number(value) })} />
  </div>;
}
