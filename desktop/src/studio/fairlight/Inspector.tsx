import { useFairlightText } from './words';
import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Media, Op, Timeline } from '../engine';
import { ClipAudio } from '../inspector/ClipAudio';
import { FieldsLocked, NumberField } from '../inspector/fields';
import { useCommit } from '../inspector/useCommit';
import { Inspector as EditInspector } from '../inspector/Inspector';
import { AUDIO_EFFECTS } from './EffectsLibrary';
import { useWords } from '../../app/locale';
import { refusalOf } from '../inspector/refusal';
/** Track/clip selection is shared with the timeline. Detailed strip processors plug in with T21. */
export function Inspector({ bridge, session, revision, timeline, media, clips, track: trackId, transition = null, onEdit, frame = 0, onSeek = () => {}, onSpeed }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; media: readonly Media[];
  clips: string[]; track: string | null; onEdit: (ops: Op[]) => void;
  transition?: string | null;
  frame?: number; onSeek?: (frame: number, scrubbing: boolean) => void; onSpeed?: (clips: string[]) => void;
}) {
  const t = useFairlightText();
  const words = useWords();
  const { commit, refusal, setRefusal } = useCommit({ bridge, session, revision, timeline });
  const [mode, setMode] = useState<'clip' | 'track'>('clip');
  const [tab, setTab] = useState<'audio' | 'effects' | 'file'>('audio');
  useEffect(() => { if (clips.length) setMode('clip'); }, [clips]);
  useEffect(() => { if (trackId !== null && !clips.length && transition === null) setMode('track'); }, [trackId, clips, transition]);
  const track = timeline.tracks.find(each => each.id === trackId);
  const locations = timeline.tracks.flatMap(each => each.kind !== 'audio' ? [] : each.clips.filter(clip => clips.includes(clip.id)).map(clip => ({ clip, locked: each.locked })));
  const unlocked = locations.filter(each => !each.locked);
  const locked = locations.length > 0 && unlocked.length === 0;
  const chosen = (locked ? locations : unlocked).map(each => each.clip);
  const hasTransition = transition !== null && timeline.tracks.some(each => each.kind !== 'subtitle' && each.transitions.some(item => item.id === transition));
  if (hasTransition) return <aside className="fairlight-inspector" aria-label={t('Fairlight Inspector')}><h2>{t('Inspector')}</h2>
    <EditInspector bridge={bridge} session={session} revision={revision} timeline={timeline} media={media} clips={[]} transition={transition} onSpeed={() => {}} />
  </aside>;
  return <aside className="fairlight-inspector" aria-label={t('Fairlight Inspector')}><h2>{t('Inspector')}</h2>
    <div role="group" aria-label={t('Inspector mode')}><button type="button" aria-pressed={mode === 'clip'} onClick={() => setMode('clip')}>{t('Clip')}</button>
      <button type="button" aria-pressed={mode === 'track'} onClick={() => setMode('track')}>{t('Track')}</button></div>
    <div role="tablist" aria-label={t('Inspector tabs')}>{(['audio', 'effects', 'file'] as const).map(each => <button type="button" role="tab" key={each} aria-selected={tab === each} onClick={() => setTab(each)}>{t(each === 'audio' ? 'Audio' : each === 'effects' ? 'Effects' : 'File')}</button>)}</div>
    <strong>{mode === 'track' ? track?.name ?? t('Select a track') : `${chosen.length} ${t('clips')}`}</strong>
    {refusal && <p role="alert">{refusal.text}</p>}
    {tab === 'audio' && mode === 'clip' && (chosen.length ? <FieldsLocked.Provider value={locked}>
      {onSpeed && <button type="button" disabled={locked} onClick={() => onSpeed(chosen.map(clip => clip.id))}>{words.label.studio.inspector.changeSpeed}</button>}
      <ClipAudio key={JSON.stringify([session, timeline.id, chosen.map(clip => clip.id)])} timeline={timeline} clips={chosen} commit={commit}
        bridge={bridge} session={session} revision={revision} media={media} frame={frame} onSeek={onSeek} fineModifiers
        onFailure={error => setRefusal(refusalOf(error, words.hint.studio))} />
    </FieldsLocked.Provider> : <p>{t('Select audio clips')}</p>)}
    {tab === 'audio' && mode === 'track' && track?.kind === 'audio' && <>
      <NumberField name="track-fader" label={t('Fader')} value={track.strip?.fader_db ?? 0} fallback={0} unit="dB" min={-100} max={10} digits={1} scrub={.1} fineModifiers disabled={track.locked}
        onCommit={fader_db => onEdit([{ op: 'set_track_audio', timeline: timeline.id, track: track.id, patch: { strip: { fader_db } } }])} />
      <span>{track.format === '5.1' ? '5.1' : t(track.format === 'mono' ? 'Mono' : track.format === 'adaptive' ? 'Adaptive' : 'Stereo')}</span>
    </>}
    {tab === 'effects' && <ul>{(mode === 'track' && track?.kind === 'audio' ? track.strip?.inserts ?? [] : chosen.flatMap(clip => clip.effects ?? [])).map((insert, index) => <li key={index}>{t(AUDIO_EFFECTS.find(([id]) => id === insert.effect.kind)![1])} · {t(insert.on ? 'On' : 'Off')}</li>)}</ul>}
    {tab === 'file' && <ul>{chosen.map(clip => <li key={clip.id}>{clip.source.kind === 'media' ? media.find(item => item.id === (clip.source as { media: string }).media)?.path ?? clip.source.media : clip.source.kind}</li>)}</ul>}
  </aside>;
}
