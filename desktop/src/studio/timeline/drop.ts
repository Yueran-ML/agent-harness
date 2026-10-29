import type { Media, Rate, Timeline } from '../engine';
import type { Edit, Targets, Transaction } from './commands';

/** Same integer tick bounds as model::span::frames_for_span and edit::place::longest, at speed 1. */
export function materialLength(media: Media, rate: Rate): number {
  if (media.facts.video?.still && !media.mark_out) return Math.floor(5 * rate.num / rate.den);
  const { ticks, scale } = media.facts.duration;
  const from = media.mark_in?.ticks ?? 0;
  const to = media.mark_out?.ticks ?? ticks;
  const frames = (span: number) => span < 0 ? 0n : ((BigInt(span) + 1n) * BigInt(rate.num) - 1n) / (BigInt(rate.den) * BigInt(scale));
  const marked = frames(to - from);
  const bounded = frames(ticks - from - 1) + 1n;
  const source = frames(ticks - from);
  return Number([marked, bounded, source, BigInt(Number.MAX_SAFE_INTEGER)].reduce((a, b) => a < b ? a : b));
}

/**
 * Batch the pool's ordered selection so the whole drop is one shared undo step. Dropped on a video
 * track, the picture goes there and the sound to the targeted audio track; dropped on an audio
 * track, the sound goes there and the picture to the targeted video track, so that media with both
 * keeps both (none where the video target is off).
 */
export function dropMedia(timeline: Timeline, revision: number, media: readonly Media[], ids: readonly string[], track: string, targets: Targets, at: number, ripple: boolean): Transaction | null {
  const destination = timeline.tracks.find(item => item.id === track);
  if (!destination || destination.locked || destination.kind === 'subtitle' || !ids.length) return null;
  const routing = destination.kind === 'video' ? { video_track: track, audio_track: targets.audio_track ?? null } : { video_track: targets.video_track ?? null, audio_track: track };
  const ops: Edit[] = [];
  for (const id of ids) {
    const item = media.find(item => item.id === id);
    if (!item) return null;
    ops.push({ edit: ripple ? 'insert' : 'overwrite', timeline: timeline.id, source: { kind: 'media', media: id }, at, ...routing });
    at += materialLength(item, timeline.rate);
  }
  return { revision, ops };
}
