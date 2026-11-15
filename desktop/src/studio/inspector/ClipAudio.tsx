import { useContext, useEffect, useRef } from 'react';
import { ChevronLeft, ChevronRight, RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { AudioParam, Clip, KeyInterp, Media, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { audioValue, frameAtSource, keyTime, seconds } from '../timeline/audioKeys';
import { clamp } from '../audio/taper';
import { clipLocations } from '../timeline/selection';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { Fades } from './ClipVideo';
import { common, setClips } from './common';
import { FieldsLocked, KeyDiamond, NumberField, type NumberFieldProps } from './fields';
import type { Build, Commit } from './useCommit';
import { ClipEq } from './ClipEq';

type Param = 'gain_db' | 'pan';
type Field = Param | 'semitones' | 'cents';
const keyed = (field: Field): field is Param => field === 'gain_db' || field === 'pan';
const shifted = (param: AudioParam, delta: number): AudioParam => typeof param === 'number'
  ? clamp(param + delta, -1, 1) : { keys: param.keys.map(key => ({ ...key, value: clamp(key.value + delta, -1, 1) })) };

/** Shared source-sample Audio inspector; the timeline supplies its displayed frame. */
export function ClipAudio({ clips, timeline, commit, bridge, session, revision, media, frame, onSeek, onFailure, fineModifiers = false }: {
  clips: readonly Clip[]; timeline: Timeline; commit: Commit; bridge: Pick<EngineBridge, 'call'>;
  session: number; revision: number; media: readonly Media[]; frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void; onFailure: (error: unknown) => void; fineModifiers?: boolean;
}) {
  const labels = useWords().label.studio.inspector;
  const t = useFairlightText();
  const locked = useContext(FieldsLocked);
  const ids = clips.map(clip => clip.id);
  const identity = JSON.stringify([timeline.id, ids]);
  const gesture = useAudioGesture({ bridge, session, revision, identity, locked, onFailure });
  const commitKeys = (build: Build) => { if (!locked && !gesture.active()) void commit(build); };
  const captured = useRef<{ clips: readonly Clip[]; frame: number; restore: Change[] } | null>(null);
  const sampleRate = (clip: Clip) => clip.source.kind === 'media'
    ? media.find(item => item.id === (clip.source as { media: string }).media)?.facts.audio?.rate ?? timeline.sample_rate : timeline.sample_rate;
  const at = (clip: Clip, time = frame, now = timeline) => keyTime(clip, now.rate, time, sampleRate(clip), false);
  const value = (clip: Clip, param: Param) => audioValue(clip[param], seconds(at(clip)));
  const set = (clip: Clip, field: Field, next: number | AudioParam, now = timeline): Change => ({ op: 'set_clip', timeline: now.id, id: clip.id,
    ...(keyed(field) ? { [field]: next } : { pitch: { semitones: 0, cents: 0, ...clip.pitch, [field]: next } }) });
  const absolute = (clip: Clip, field: Field, next: number, time: number, now = timeline): Change => keyed(field) && typeof clip[field] !== 'number'
    ? { edit: 'clip_gain_keys', timeline: now.id, clip: clip.id, param: field, action: 'add', at: at(clip, time, now), value: next }
    : set(clip, field, next, now);
  const available = (now: Timeline) => {
    const locations = clipLocations(now);
    return ids.flatMap(id => { const located = locations.get(id); return located && located.track.kind === 'audio' && !located.track.locked ? [located.clip] : []; });
  };
  const reset = (field: Field) => { if (!locked && !gesture.active()) void commit(setClips(ids, clip => keyed(field) ? { [field]: 0 } : { pitch: { semitones: 0, cents: 0, ...clip.pitch, [field]: 0 } })); };
  const input = (field: Field): Pick<NumberFieldProps, 'gesture' | 'onCommit' | 'onReset' | 'fineModifiers'> => {
    const scale = field === 'pan' ? 100 : 1;
    return {
      fineModifiers, onReset: () => reset(field),
      onCommit: next => { if (!locked) void commit(now => available(now).map(clip => absolute(clip, field, next / scale, frame, now))); },
      gesture: {
        relative: clips.length > 1,
        start: () => {
          const restore = clips.map(clip => set(clip, field, keyed(field) ? clip[field] : clip.pitch?.[field] ?? 0));
          if (!gesture.start(restore)) return false;
          captured.current = { clips, frame, restore }; return true;
        }, active: gesture.active, cancel: gesture.cancel, end: gesture.end,
        preview: (next, delta) => {
          const start = captured.current;
          if (!start || !gesture.active()) return;
          const relative = start.clips.length > 1 && delta !== null;
          let ops: Change[];
          if (relative && field === 'gain_db') {
            // Restore before each total nudge: coalescing and keep-alives cannot accumulate it.
            ops = [...start.restore, ...(delta === 0 ? [] : [{ edit: 'nudge_clip_gain' as const, timeline: timeline.id, clips: ids, delta_db: delta, linked: false }])];
          } else if (relative && field === 'pan') {
            ops = start.clips.map(clip => set(clip, field, shifted(clip.pan, delta / 100)));
          } else if (relative && !keyed(field)) {
            const bound = field === 'semitones' ? 12 : 100;
            const by = Number(delta.toFixed(field === 'semitones' ? 0 : 1));
            ops = start.clips.map(clip => set(clip, field, clamp((clip.pitch?.[field] ?? 0) + by, -bound, bound)));
          } else ops = start.clips.map(clip => absolute(clip, field, next / scale, start.frame));
          gesture.preview(ops);
        },
      },
    };
  };
  const navigation = useRef<{ param: Param; precise: number; displayed: number } | null>(null);
  useEffect(() => { if (navigation.current && navigation.current.displayed !== frame) navigation.current = null; }, [frame]);
  useEffect(() => { navigation.current = null; }, [identity, session]);
  const frames = (param: Param) => clips.flatMap(clip => typeof clip[param] === 'number' ? [] : clip[param].keys
    .map(key => frameAtSource(clip, timeline.rate, seconds(key.at)))
    .filter(time => time >= clip.start && time <= clip.start + clip.length)).sort((a, b) => a - b);
  const navigate = (param: Param, direction: -1 | 1) => {
    const from = navigation.current?.param === param ? navigation.current.precise : frame;
    const list = frames(param);
    const target = direction < 0 ? list.filter(time => time < from - 1e-7).at(-1) : list.find(time => time > from + 1e-7);
    if (target === undefined) return;
    const displayed = Math.round(target); navigation.current = { param, precise: target, displayed }; onSeek(displayed, false);
  };
  const hit = (clip: Clip, param: Param, now = timeline) => {
    if (frame < clip.start || frame > clip.start + clip.length) return undefined;
    const time = at(clip, frame, now), keys = clip[param];
    return typeof keys === 'number' ? undefined : keys.keys.find(key => seconds(key.at) === seconds(time));
  };
  const controls = (param: Param, label: string) => {
    const hits = clips.map(clip => hit(clip, param));
    const all = hits.length > 0 && hits.every(Boolean);
    const active = gesture.active();
    return <div className="audio-param-keys" role="group" aria-label={label} onKeyDown={event => {
      event.stopPropagation();
      if (event.nativeEvent.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;
      if (event.key === '[' || event.key === ']') { event.preventDefault(); navigate(param, event.key === '[' ? -1 : 1); }
    }}>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Previous keyframe')} title={t('Previous keyframe')} disabled={!frames(param).length} onClick={() => navigate(param, -1)}><ChevronLeft size={14} aria-hidden="true" /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t(all ? 'Remove keyframe' : 'Add keyframe')} title={t(all ? 'Remove keyframe' : 'Add keyframe')} aria-pressed={all}
        data-keyed={frames(param).length > 0 || undefined} data-hit={hits.some(Boolean) || undefined}
        disabled={locked || active} onClick={() => commitKeys(now => available(now).map(clip => {
          const found = hit(clip, param, now);
          return { edit: 'clip_gain_keys', timeline: now.id, clip: clip.id, param, action: all && found ? 'remove' : 'add',
            at: found?.at ?? at(clip, frame, now), ...(!all ? { value: audioValue(clip[param], seconds(at(clip, frame, now))) } : {}) };
        }))}><KeyDiamond /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Next keyframe')} title={t('Next keyframe')} disabled={!frames(param).length} onClick={() => navigate(param, 1)}><ChevronRight size={14} aria-hidden="true" /></button>
      <select className="field" aria-label={label + ': ' + t('Keyframe interpolation')} disabled={locked || active || !hits.some(Boolean)}
        value={common(hits, key => key?.interp ?? null) ?? ''} onChange={event => {
          const interp = event.target.value as KeyInterp;
          if (!interp) return;
          commitKeys(now => available(now).flatMap(clip => { const found = hit(clip, param, now); return found ? [{ edit: 'clip_gain_keys', timeline: now.id, clip: clip.id, param, action: 'set_interp', at: found.at, interp }] : []; }));
        }}>
        <option value="">—</option>{(['linear', 'ease_in', 'ease_out'] as const).map(interp => <option key={interp} value={interp}>{t(interp === 'linear' ? 'Linear' : interp === 'ease_in' ? 'Ease In' : 'Ease Out')}</option>)}
      </select>
      <button type="button" className="icon-btn" disabled={locked || active} aria-label={label + ': ' + t('Reset')} title={t('Reset')} onClick={() => reset(param)}><RotateCcw size={12} aria-hidden="true" /></button>
    </div>;
  };
  return <div className="inspector-panel" data-panel="audio">
    <NumberField name="volume" label={labels.volume} value={common(clips, clip => value(clip, 'gain_db'))} fallback={0} unit="dB"
      min={-96} max={24} digits={1} scrub={0.1} slider={{ min: -96, max: 24, step: 0.1 }} {...input('gain_db')} />
    {controls('gain_db', labels.volume)}
    <NumberField name="pan" label={labels.pan} value={common(clips, clip => value(clip, 'pan') * 100)} fallback={0}
      min={-100} max={100} digits={fineModifiers ? 1 : 0} scrub={0.5} slider={{ min: -100, max: 100, step: 1 }} {...input('pan')} />
    {controls('pan', labels.pan)}
    <h3 className="inspector-section">{t('Pitch')}</h3>
    <NumberField name="semitones" label={t('Semitones')} value={common(clips, clip => clip.pitch?.semitones ?? 0)} fallback={0} min={-12} max={12} digits={0} scrub={.1} {...input('semitones')} />
    <NumberField name="cents" label={t('Cents')} value={common(clips, clip => clip.pitch?.cents ?? 0)} fallback={0} min={-100} max={100} digits={1} scrub={1} {...input('cents')} />
    <h3 className="inspector-section">{labels.fades}</h3>
    <Fades clips={clips} timeline={timeline} commit={commit} fineModifiers={fineModifiers} />
    <ClipEq clips={clips} bridge={bridge} session={session} revision={revision} timeline={timeline} />
  </div>;
}
