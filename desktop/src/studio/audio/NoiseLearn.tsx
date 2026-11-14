import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { useAudioAnalysis } from '../fairlight/audioReadings';
import { useFairlightText } from '../fairlight/words';
import type { ProcessorProps } from './useProcessor';
import { targetKey } from './processorModel';

export function NoiseLearn({ context, slot, insert, disabled }: { context: ProcessorProps; slot: number; insert?: string; disabled: boolean }) {
  const t = useFairlightText();
  const identity = `${context.timeline.id}/${targetKey(context.target)}/${insert || slot}`;
  const jobs = useAudioAnalysis(context.bridge as Pick<EngineBridge, 'call' | 'onEvent'>, context.session, context.bridge.onEvent ? context.timeline.id : null, identity);
  const fps = context.timeline.rate.num / context.timeline.rate.den;
  const clipId = 'clip' in context.target ? context.target.clip : null;
  const clip = context.timeline.tracks.flatMap(track => track.kind === 'audio' ? track.clips : []).find(c => c.id === clipId);
  const initial = () => Math.max(clip?.start ?? 0, Math.floor((context.seconds ?? 0) * fps));
  const [from, setFrom] = useState(initial), [to, setTo] = useState(() => Math.min(clip ? clip.start + clip.length : Infinity, initial() + Math.ceil(fps)));
  useEffect(() => { const start = initial(); setFrom(start); setTo(Math.min(clip ? clip.start + clip.length : Infinity, start + Math.ceil(fps))); }, [identity, context.session]);
  const result = jobs.analysis?.state === 'done' && jobs.analysis.method === 'audio.noise_learn' ? jobs.analysis.result : null;
  return <fieldset className="noise-learn"><legend>{t('Learn Noise')}</legend><p>{t('Learn noise from a short noise-only range.')}</p>
    <label>{t('Learn from frame')}<input type="number" step="1" min="0" value={from} disabled={jobs.pending} onChange={e => setFrom(Number(e.target.value))} /></label>
    <label>{t('Learn to frame')}<input type="number" step="1" min="1" value={to} disabled={jobs.pending} onChange={e => setTo(Number(e.target.value))} /></label>
    <button type="button" disabled={disabled || jobs.pending || !context.bridge.onEvent || !Number.isInteger(from) || !Number.isInteger(to) || to <= from || (to-from)/fps > 30} onClick={() => void jobs.start('audio.noise_learn', { target: context.target, slot, range: { from, to }, revision: context.revision })}>{t('Learn Noise')}</button>
    {jobs.pending && <button type="button" onClick={jobs.cancel}>{t('Cancel analysis')}</button>}
    {jobs.analysis?.state === 'running' && <output>{jobs.analysis.done.toFixed(1)} / {jobs.analysis.total.toFixed(1)} s</output>}
    {(jobs.problem || jobs.analysis?.state === 'failed') && <p role="alert">{jobs.problem ?? (jobs.analysis?.state === 'failed' ? jobs.analysis.message : '')}</p>}
    {result && <p role={result.refused ? 'alert' : 'status'}>{result.refused?.message ?? t('Noise learned')}</p>}
    {jobs.analysis?.state === 'cancelled' && <p role="status">{t('Cancelled')}</p>}
  </fieldset>;
}
