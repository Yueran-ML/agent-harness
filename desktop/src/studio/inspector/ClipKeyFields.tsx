import { useContext, type ReactNode } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import type { Clip, ClipKeyed, Media, Op, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { clipLocations } from '../timeline/selection';
import { addKey, keyAt, keyFrameOf, keyTimes, paramOf, putValue, removeKey, settingAt } from './clipKeys';
import { common } from './common';
import { FieldsLocked, KeyDiamond } from './fields';
import type { Build, Commit } from './useCommit';

/** What the fields of the settings of clips that take keys are made of: see `useClipKeyFields`. */
export interface ClipKeyFields {
  /** What a setting is at the playhead, as `shown` writes what is kept, or null where the clips differ. */
  value: (path: ClipKeyed, shown: (kept: number) => number) => number | null;
  /** Give a setting of every clip a value at the playhead: a key there once it has keys, its one number before. */
  set: (path: ClipKeyed, kept: number) => void;
  /** The three buttons of a setting, to stand beside its field: to the key before, a key put or taken away at the playhead, to the key after. */
  keys: (path: ClipKeyed, label: string) => ReactNode;
}

/**
 * The fields of the settings of clips that take keys (Reference Manual 20, Ch. 53), for every tab
 * of the Inspector that has such settings: the Video tab its transform and crop, the Title tab the
 * size of a title and the place of its anchor. A field shows what its setting is at the playhead;
 * the diamond beside it puts a key there, or takes away the one there, and the arrows go to the key
 * before and after. A setting that has keys takes a value typed or dragged as a key at the
 * playhead; one without is one number, as before.
 *
 * Each change sets every clip chosen that may be set, in one transaction, worked out from the
 * timeline as it is when it is sent.
 */
export function useClipKeyFields({ clips, timeline, commit, media, frame, onSeek }: {
  clips: readonly Clip[]; timeline: Timeline; commit: Commit; media: readonly Media[]; frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void;
}): ClipKeyFields {
  const t = useFairlightText();
  const locked = useContext(FieldsLocked);
  const ids = clips.map(clip => clip.id);
  const at = (clip: Clip, now: Pick<Timeline, 'rate'> = timeline) => keyFrameOf(clip, now, media, frame);
  /** What each chosen clip that may be set gives, worked out from the timeline as it is when it is sent. */
  const each = (ops: (clip: Clip, now: Timeline) => Op[]): Build => now => {
    const locations = clipLocations(now);
    return ids.flatMap(id => {
      const located = locations.get(id);
      return !located || located.track.locked ? [] : ops(located.clip, now);
    });
  };
  const value = (path: ClipKeyed, shown: (kept: number) => number) => common(clips, clip => shown(settingAt(clip, path, at(clip))));
  const set = (path: ClipKeyed, kept: number) => void commit(each((clip, now) => putValue(now.id, clip, path, kept, at(clip, now))));
  const keys = (path: ClipKeyed, label: string) => {
    const hits = clips.map(clip => keyAt(paramOf(clip, path), at(clip)));
    const all = hits.length > 0 && hits.every(Boolean);
    const times = [...new Set(clips.flatMap(clip => keyTimes(clip, timeline, media, path)))].sort((a, b) => a - b);
    const go = (direction: -1 | 1) => {
      const target = direction < 0 ? times.filter(time => time < frame).at(-1) : times.find(time => time > frame);
      if (target !== undefined) onSeek(target, false);
    };
    // The diamond says three things apart: the setting has no keys, it has keys (`data-keyed`), a key is at the playhead (`data-hit`, and pressed where every clip has one).
    return <div className="audio-param-keys" role="group" aria-label={label} data-keys={path}>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Previous keyframe')} title={t('Previous keyframe')} disabled={!times.some(time => time < frame)} onClick={() => go(-1)}><ChevronLeft size={14} aria-hidden="true" /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t(all ? 'Remove keyframe' : 'Add keyframe')} title={t(all ? 'Remove keyframe' : 'Add keyframe')} aria-pressed={all}
        data-keyed={times.length > 0 || undefined} data-hit={hits.some(Boolean) || undefined}
        disabled={locked} onClick={() => void commit(each((clip, now) => all ? removeKey(now.id, clip, path, at(clip, now)) : addKey(now.id, clip, path, at(clip, now))))}><KeyDiamond /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Next keyframe')} title={t('Next keyframe')} disabled={!times.some(time => time > frame)} onClick={() => go(1)}><ChevronRight size={14} aria-hidden="true" /></button>
    </div>;
  };
  return { value, set, keys };
}
