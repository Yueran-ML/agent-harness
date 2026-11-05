import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { Sheet } from '../../app/Sheet';
import { CoveringMenu } from '../color/CoveringMenu';
import type { MenuItem } from '../color/ColorMenu';
import { callEngine, type AudioStandard, type Timeline } from '../engine';
import { useAudioAnalysis } from '../fairlight/audioReadings';
import { standardLabel } from '../fairlight/Monitoring';
import { useFairlightText } from '../fairlight/words';
import type { TimelineHandle } from '../timeline/Timeline';

type Bridge = Pick<EngineBridge, 'call' | 'onEvent'>;
/** A selection may include linked video and locked tracks; only editable sound is normalized. */
export function normalizeTargets(timeline: Timeline | null, selection: readonly string[], context?: string): string[] {
  const requested = context && !selection.includes(context) ? [context] : selection;
  const editable = new Set(timeline?.tracks.flatMap(track => track.kind === 'audio' && !track.locked ? track.clips.map(clip => clip.id) : []) ?? []);
  return [...new Set(requested)].filter(id => editable.has(id));
}

/** Both pages use the same menu, selection rules and dialog. Opening binds the targets to this project. */
export function useNormalizeAudio({ bridge, session, revision, timeline, selection, commands }: {
  bridge: Bridge; session: number; revision: number; timeline: Timeline | null; selection: string[];
  commands?: () => Pick<TimelineHandle, 'perform' | 'canPerform'> | null;
}) {
  const t = useFairlightText();
  const [opened, setOpened] = useState<{ session: number; timeline: string; clips: string[] } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  useEffect(() => { setOpened(null); setMenu(null); }, [session, timeline?.id]);
  const item = (context?: string): MenuItem => {
    const clips = normalizeTargets(timeline, selection, context);
    return { id: 'normalize-audio', label: t('Normalize Audio Levels'), disabled: !clips.length || clips.length > 256,
      run: () => { if (timeline) setOpened({ session, timeline: timeline.id, clips }); } };
  };
  return {
    item,
    menu: <><button type="button" className="btn btn-quiet" aria-haspopup="menu" aria-expanded={menu !== null} onClick={event => {
      const rect = event.currentTarget.getBoundingClientRect(); setMenu({ x: rect.left, y: rect.bottom });
    }}>{t('Audio')}</button>{menu && <CoveringMenu menu={{ ...menu, label: t('Audio'), items: [item(), ...(commands ? [
      ...([-3, -1, 1, 3] as const).map(delta => ({ id: `gain-${delta}`, label: t(delta === -3 ? 'Decrease Audio Level 3 dB' : delta === -1 ? 'Decrease Audio Level 1 dB' : delta === 1 ? 'Increase Audio Level 1 dB' : 'Increase Audio Level 3 dB'),
        disabled: !commands()?.canPerform({ kind: 'gain', delta }), run: () => { commands()?.perform({ kind: 'gain', delta }); } })),
      { id: 'audio-transition', label: t('Add Audio Transition'), disabled: !commands()?.canPerform({ kind: 'audio-transition' }), run: () => { commands()?.perform({ kind: 'audio-transition' }); } },
    ] : [])] }} onClose={() => setMenu(null)} />}</>,
    dialog: opened && opened.session === session && opened.timeline === timeline?.id
      ? <NormalizeDialog bridge={bridge} session={session} timeline={timeline} revision={revision} clips={opened.clips} onClose={() => setOpened(null)} /> : null,
  };
}

export function NormalizeDialog({ bridge, session, timeline, revision, clips, onClose }: {
  bridge: Bridge; session: number; timeline: Timeline; revision: number; clips: string[]; onClose: () => void;
}) {
  const t = useFairlightText();
  const jobs = useAudioAnalysis(bridge, session, timeline.id);
  const [standards, setStandards] = useState<AudioStandard[]>([]);
  const [standardProblem, setStandardProblem] = useState<string | null>(null);
  const [level, setLevel] = useState('sample_peak');
  const [target, setTarget] = useState('-9');
  const [mode, setMode] = useState<'relative' | 'independent'>('relative');
  useEffect(() => {
    let live = true;
    void callEngine(bridge, 'audio.standards', {}, { expectedSession: session }).then(result => {
      if (live) setStandards(result.standards);
    }).catch(error => { if (live) setStandardProblem(error instanceof Error ? error.message : String(error)); });
    return () => { live = false; };
  }, [bridge, session]);
  const targets = normalizeTargets(timeline, clips);
  const peak = level === 'sample_peak' || level === 'true_peak';
  const unit = peak ? 'dBFS' : 'LUFS';
  const valid = targets.length > 0 && targets.length <= 256 && target.trim() !== '' && Number.isFinite(Number(target)) && Number(target) >= -60 && Number(target) <= 0;
  const result = jobs.analysis?.state === 'done' && jobs.analysis.method === 'audio.normalize' ? jobs.analysis : null;
  const resultUnit = result && (result.level === 'sample_peak' || result.level === 'true_peak') ? 'dBFS' : 'LUFS';
  const measured = (value: number | null, units: string) => value === null ? t('No measurable level (silence or too short)') : `${value.toFixed(1)} ${units}`;
  return <Sheet title={t('Normalize Audio Levels')} onClose={onClose} foot={<>
    <button type="button" className="btn" data-first onClick={onClose}>{t('Close')}</button>
    {jobs.pending && <button type="button" className="btn" onClick={jobs.cancel}>{t('Cancel analysis')}</button>}
    <button type="button" className="btn btn-main" disabled={!valid || jobs.pending} onClick={() => void jobs.start('audio.normalize', {
      clips: targets, level, target: Number(target), mode: targets.length > 1 ? mode : 'relative', apply: true, revision,
    })}>{t('Normalize')}</button>
  </>}>
    <div className="normalize-dialog">
      <label>{t('Normalization mode')}<select className="field" aria-label={t('Normalization mode')} value={level} disabled={jobs.pending} onChange={event => {
        const next = event.target.value; setLevel(next);
        setTarget(next === 'sample_peak' || next === 'true_peak' ? '-9' : String(standards.find(row => row.id === next)?.target ?? timeline.audio?.loudness?.target ?? ''));
      }}>
        <option value="sample_peak">{t('Sample Peak')}</option><option value="true_peak">{t('True Peak')}</option>
        {standards.map(row => <option key={row.id} value={row.id}>{standardLabel(row.name, t)}{!row.official ? ` (${t('unofficial')})` : ''}{!row.verified ? ` (${t('unverified')})` : ''}</option>)}
      </select></label>
      <label>{t('Target')} ({unit})<input className="field" type="number" min={-60} max={0} step="any" aria-label={`${t('Target')} (${unit})`} value={target} disabled={jobs.pending} onChange={event => setTarget(event.target.value)} /></label>
      {targets.length > 1 && <label>{t('Set Level')}<select className="field" aria-label={t('Set Level')} value={mode} disabled={jobs.pending} onChange={event => setMode(event.target.value as 'relative' | 'independent')}>
        <option value="relative">{t('Relative')}</option><option value="independent">{t('Independent')}</option>
      </select></label>}
      <p>{targets.length} {t('clips')}</p>
      {targets.length > 256 && <p role="alert">{t('Select at most 256 audio clips.')}</p>}
      {!targets.length && <p role="status">{t('Select audio clips')}</p>}
      {standardProblem && <p role="alert">{standardProblem}</p>}
      {jobs.problem && <p role="alert">{jobs.problem}</p>}
      {jobs.analysis?.state === 'running' && <p role="status">{t('Analyzing')} {jobs.analysis.done.toFixed(1)} / {jobs.analysis.total.toFixed(1)} s</p>}
      {jobs.analysis?.state === 'failed' && <p role="alert">{jobs.analysis.message}</p>}
      {jobs.analysis?.state === 'cancelled' && <p role="status">{t('Cancelled')}</p>}
      {result && <section aria-label={t('Normalization results')}>
        {result.refused ? <p role="alert">{result.refused.message}</p> : <p role="status">{t(result.applied ? 'Applied' : 'Nothing applied')}</p>}
        {result.mode === 'relative' && <p>{t('Measured level')}: <span>{measured(result.measured, resultUnit)}</span></p>}
        <table><thead><tr><th>{t('Clip')}</th><th>{t('Measured level')}</th><th>{t('Gain added')}</th><th>{t('clamped')}</th></tr></thead>
          <tbody>{result.clips.map(clip => <tr key={clip.clip}><th>{clip.clip}</th><td>{measured(clip.measured, resultUnit)}</td>
            <td>{clip.gain_db === null ? '—' : `${clip.gain_db.toFixed(1)} dB`}</td><td>{clip.clamped ? t('clamped') : '—'}</td></tr>)}</tbody>
        </table>
      </section>}
    </div>
  </Sheet>;
}
