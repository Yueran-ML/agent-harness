import type { Clip, Rate, Timeline, Track, Transition } from '../engine';
/** Engine snapshot, not references to live source clips. Automation stays opaque to the UI. */
export interface ClipClipboard {
  rate: Rate;
  at: number;
  clips: {
    track: string;
    index: number;
    kind: 'audio' | 'video';
    clip: Clip;
    lanes: unknown[];
  }[];
  transitions: {
    left: number;
    right: number;
    transition: Transition;
  }[];
}
/** Round timeline frames exactly as the model does, including negative anchor offsets. */
function frames(value: number, source: Rate, destination: Rate): number {
  const n = BigInt(value) * BigInt(source.den) * BigInt(destination.num);
  const d = BigInt(source.num) * BigInt(destination.den);
  const numerator = 2n * n + d, denominator = 2n * d;
  const quotient = numerator / denominator;
  return Number(quotient - (numerator < 0 && numerator % denominator !== 0n ? 1n : 0n));
}
export interface ClipboardPlacement {
  track: Track;
  clip: Clip;
}
/** Keep the track block together; audio moves down, video moves up in display order. */
export function clipboardPlacement(snapshot: ClipClipboard, timeline: Timeline, at: number, shift = 0) {
  const tracks: Record<string, string> = {};
  const clips: ClipboardPlacement[] = [];
  for (const [index, item] of snapshot.clips.entries()) {
    const rows = timeline.tracks.filter(track => track.kind === item.kind);
    const current = rows.findIndex(track => track.id === item.track);
    const destination = (current < 0 ? item.index : current) + shift * (item.kind === 'video' ? -1 : 1);
    const track = rows[destination];
    if (!track)
      return null;
    tracks[item.track] = track.id;
    const start = at + frames(item.clip.start - snapshot.at, snapshot.rate, timeline.rate);
    const end = at + frames(item.clip.start + item.clip.length - snapshot.at, snapshot.rate, timeline.rate);
    const length = Math.max(1, end - start);
    clips.push({
      track, clip: {
        ...item.clip, id: `clipboard-preview-${index}`, start, length,
        fade_in: Math.min(length, frames(item.clip.fade_in, snapshot.rate, timeline.rate)),
        fade_out: Math.min(length, frames(item.clip.fade_out, snapshot.rate, timeline.rate))
      }
    });
  }
  return { tracks, clips };
}
