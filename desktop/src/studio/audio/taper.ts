/** Reference C-F2, in canonical travel. Above +10 the final slope continues to +24 at 1.28.
 * Controls normalize their own interval: a clip's 0 dB is 58.46%, a fader's is 75%.
 * This visual difference is recorded for the author's T26 hardware comparison. */
const anchors = [[0, -100], [.04, -60], [.12, -40], [.2, -30], [.32, -20], [.5, -10], [.62, -5], [.75, 0], [.9, 5], [1, 10], [1.28, 24]] as const;
export const FADER_GAIN = [-100, 10] as const;
export const CLIP_GAIN = [-96, 24] as const;
type Range = readonly [number, number];
export const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
function map(value: number, input: 0 | 1): number {
  const output = input === 0 ? 1 : 0;
  for (let i = 1; i < anchors.length; i++) {
    const a = anchors[i - 1], b = anchors[i];
    if (value <= b[input]) return a[output] + (b[output] - a[output]) * (value - a[input]) / (b[input] - a[input]);
  }
  return anchors.at(-1)![output];
}
export function dbToPosition(db: number, range: Range = FADER_GAIN): number {
  const low = map(range[0], 1), high = map(range[1], 1);
  return (map(clamp(db, ...range), 1) - low) / (high - low);
}
export function positionToDb(position: number, range: Range = FADER_GAIN): number {
  const low = map(range[0], 1), high = map(range[1], 1);
  return clamp(map(low + clamp(position, 0, 1) * (high - low), 0), ...range);
}
