import { useFairlightText } from './words';
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import type { EngineBridge } from '../../bridge';
import type { CapturedLevel, Media, Timeline } from '../engine';
import { useAudioAnalysis } from './audioReadings';
import { reading } from './Monitoring';

/** A captured level, with the names of where it was taken, as they were then: the clip may go. */
export type CapturedProfile = CapturedLevel & { label: string };
/** A clip by the name it shows on the timeline: its media's, a title's text; the timeline's name before it. */
function profileLabel(timeline: Timeline, media: readonly Media[], clip: string) {
  const found = timeline.tracks.flatMap(track => track.kind === 'subtitle' ? [] : track.clips).find(each => each.id === clip);
  const source = found?.source;
  const name = source?.kind === 'media' ? media.find(item => item.id === source.media)?.name
    : source?.kind === 'title' ? source.text : undefined;
  return name ? `${timeline.name} / ${name}` : timeline.name;
}
export interface LevelMatcherHandle { capture: (clip: string) => void; apply: (clips: string[]) => void; captured: boolean }
export function LevelMatcher({ bridge, session, timeline, media = [], revision, clips, profile, onCapture, onPending, ref }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; timeline: Timeline; media?: readonly Media[]; revision: number;
  clips: string[]; profile: CapturedProfile | null; onCapture: (profile: CapturedProfile) => void; onPending?: (pending: boolean) => void;
  ref?: Ref<LevelMatcherHandle>;
}) {
  const t = useFairlightText();
  const jobs = useAudioAnalysis(bridge, session, timeline.id);
  const cancelled = useRef(false);
  const consumed = useRef<string | null>(null);
  const [noLevel, setNoLevel] = useState(false);
  useEffect(() => { onPending?.(jobs.pending); }, [jobs.pending, onPending]);
  useEffect(() => {
    const result = jobs.analysis;
    if (result?.state !== 'done' || result.method !== 'audio.level_profile' || consumed.current === result.job) return;
    consumed.current = result.job;
    if (cancelled.current) return;
    const usable = result.profile.level_db !== null && Number.isFinite(result.profile.level_db) && result.profile.basis !== null;
    setNoLevel(!usable);
    if (usable) onCapture({ source: result.source, profile: result.profile, seconds: result.seconds,
      label: profileLabel(timeline, media, result.source.clip) });
  }, [jobs.analysis, onCapture]);
  const capture = (clip: string) => {
    if (jobs.pending) return;
    cancelled.current = false; setNoLevel(false);
    void jobs.start('audio.level_profile', { clip, revision });
  };
  const apply = (ids: string[]) => {
    if (!profile || !ids.length || jobs.pending) return;
    setNoLevel(false);
    void jobs.start('audio.level_match', { profile: profile.profile, clips: ids, apply: true, revision });
  };
  const cancel = () => { cancelled.current = true; jobs.cancel(); };
  useImperativeHandle(ref, () => ({ capture, apply, captured: Boolean(profile) && !jobs.pending }));
  return <section aria-label={t('Level Matcher')} className="fairlight-level-matcher"><strong>{t('Level Matcher')}</strong>
    <button type="button" disabled={!clips.length || jobs.pending} onClick={() => capture(clips[0])}>{t('Capture Level Profile')}</button>
    <span>{t('Reference')}: {profile ? `${profile.label} · ${reading(profile.profile.level_db, 'LUFS')} (${t(profile.profile.basis === 'integrated' ? 'Integrated profile' : 'Mean level profile')})` : '—'}</span>
    <button type="button" disabled={!profile || !clips.length || jobs.pending} onClick={() => apply(clips)}>{t('Apply Level Profile')}</button>
    {jobs.pending && <button type="button" onClick={cancel}>{t('Cancel Level Matcher')}</button>}
    {noLevel && <p role="alert">{t('No measurable level; the previous profile is unchanged')}</p>}
    {jobs.problem && <p role="alert">{jobs.problem}</p>}
    {jobs.analysis?.state === 'running' && <p role="status">{t('Analyzing')} {jobs.analysis.done.toFixed(1)} / {jobs.analysis.total.toFixed(1)} s</p>}
    {jobs.analysis?.state === 'failed' && <p role="alert">{jobs.analysis.message}</p>}
    {jobs.analysis?.state === 'cancelled' && <p role="status">{t('Cancelled')}</p>}
    {jobs.analysis?.state === 'done' && jobs.analysis.method === 'audio.level_match' && <div role="status">
      {jobs.analysis.refused && <p role="alert">{jobs.analysis.refused.message}</p>}
      {jobs.analysis.clips.map(clip => <p key={clip.clip}>{clip.clip}: {reading(clip.gain_db, 'dB')}{clip.clamped ? ` (${t('clamped')})` : ''}</p>)}
    </div>}
  </section>;
}
