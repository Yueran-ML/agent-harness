import { useFairlightText } from './words';
import { useImperativeHandle, useState, type Ref } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Timeline } from '../engine';
import { useAudioAnalysis } from './audioReadings';
import { reading } from './Monitoring';

export interface LevelMatcherHandle { capture: (clip: string) => void; apply: (clips: string[]) => void; captured: boolean }
export function LevelMatcher({ bridge, session, timeline, revision, clips, ref }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; timeline: Timeline; revision: number; clips: string[];
  ref?: Ref<LevelMatcherHandle>;
}) {
  const t = useFairlightText();
  const [reference, setReference] = useState<string | null>(null);
  const jobs = useAudioAnalysis(bridge, session, timeline.id);
  const exists = reference && timeline.tracks.some(track => track.kind !== 'subtitle' && track.clips.some(clip => clip.id === reference));
  const targets = clips.filter(id => id !== reference);
  const apply = (ids: string[]) => { if (exists) void jobs.start('audio.level_match', { reference, clips: ids.filter(id => id !== reference), apply: true, revision }); };
  useImperativeHandle(ref, () => ({ capture: setReference, apply, captured: Boolean(exists) }));
  return <section aria-label={t('Level Matcher')} className="fairlight-level-matcher"><strong>{t('Level Matcher')}</strong>
    <button type="button" disabled={!clips.length || jobs.pending} onClick={() => setReference(clips[0])}>{t('Capture Level Profile')}</button>
    <span>{t('Reference')}: {reference ?? '—'}</span>
    <button type="button" disabled={!exists || !targets.length || jobs.pending} onClick={() => apply(clips)}>{t('Apply Level Profile')}</button>
    {jobs.pending && <button type="button" onClick={jobs.cancel}>{t('Cancel Level Matcher')}</button>}
    {jobs.problem && <p role="alert">{jobs.problem}</p>}
    {jobs.analysis?.state === 'running' && <p role="status">{t('Analyzing')} {jobs.analysis.done.toFixed(1)} / {jobs.analysis.total.toFixed(1)} s</p>}
    {jobs.analysis?.state === 'failed' && <p role="alert">{jobs.analysis.message}</p>}
    {jobs.analysis?.state === 'done' && jobs.analysis.method === 'audio.level_match' && <div role="status">
      {jobs.analysis.refused && <p role="alert">{jobs.analysis.refused.message}</p>}
      {jobs.analysis.clips.map(clip => <p key={clip.clip}>{clip.clip}: {reading(clip.gain_db, 'dB')}{clip.clamped ? ` (${t('clamped')})` : ''}</p>)}
    </div>}
  </section>;
}
