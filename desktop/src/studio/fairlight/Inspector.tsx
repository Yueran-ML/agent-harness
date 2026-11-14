import { useFairlightText } from './words';
import { useEffect, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Media, Timeline } from '../engine';
import { ClipAudio } from '../inspector/ClipAudio';
import { FieldsLocked, NumberField } from '../inspector/fields';
import { useCommit } from '../inspector/useCommit';
import { InsertSlots } from './InsertSlots';
import { DuckerPanel } from '../audio/DuckerPanel';
import type { AudioAutomationAdapter } from '../audio/automationAdapter';
import { useAutomationTouch } from './AutomationTouch';
/** Track/clip selection is shared with the timeline. Detailed strip processors plug in with T21. */
import { Inspector as EditInspector } from '../inspector/Inspector';
import { useWords } from '../../app/locale';
import { refusalOf } from '../inspector/refusal';
import { AudioConfig } from '../inspector/AudioConfig';
import { clipLocations } from '../timeline/selection';
import type { Change } from '../timeline/commands';
import { faderChanges, faderPlan, faderReading, mixerStrips } from '../audio/mixerModel';
/** Track/clip selection is shared with the timeline. Detailed strip processors plug in with T21. */
export function Inspector({ bridge, session, revision, timeline, media, clips, track: trackId, transition = null, onEdit, frame = 0, onSeek = () => {}, onSpeed, automation: adapter }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number; timeline: Timeline; media: readonly Media[];
  clips: string[]; track: string | null; onEdit: (ops: Change[]) => void;
  automation?: AudioAutomationAdapter | null;
  transition?: string | null;
  frame?: number; onSeek?: (frame: number, scrubbing: boolean) => void; onSpeed?: (clips: string[]) => void;
}) {
  const t = useFairlightText();
  const control = useAutomationTouch(), automation = control ?? adapter;
  const words = useWords();
  const { commit, refusal, setRefusal } = useCommit({ bridge, session, revision, timeline });
  const [mode, setMode] = useState<'clip' | 'track'>('clip');
  const [tab, setTab] = useState<'audio' | 'effects' | 'file'>('audio');
  useEffect(() => { if (clips.length) setMode('clip'); }, [clips]);
  useEffect(() => { if (trackId !== null && !clips.length && transition === null) setMode('track'); }, [trackId, clips, transition]);
  const seconds = frame * timeline.rate.den / timeline.rate.num;
  const track = timeline.tracks.find(each => each.id === trackId);
  // The fader here is the mixer strip's: it writes the lane while automation is written, shows
  // (and does not write under) a lane that plays, and moves the track's group with it.
  const strip = track ? mixerStrips(timeline).find(each => each.key === `track:${track.id}`) : undefined;
  const fader = strip && faderReading(strip, seconds, automation?.enabled ?? true);
  const recordingFader = Boolean(automation?.enabled && automation.recording);
  const commitFader = (value: number) => {
    if (!strip || !fader) return;
    if (recordingFader) { automation!.instant({ of: { track: strip.id }, path: 'fader_db' }, value); return; }
    const plan = faderPlan(timeline, strip.key, [], {}, automation?.enabled ?? true);
    if (plan.members.length) onEdit(faderChanges(timeline.id, plan, value - Math.max(-100, strip.db)));
  };
  const byId = clipLocations(timeline);
  const locations = clips.flatMap(id => {
    const found = byId.get(id);
    return found?.track.kind === 'audio' ? [{ clip: found.clip, locked: found.track.locked }] : [];
  });
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
      <NumberField name="track-fader" label={t('Fader')} value={fader?.value ?? track.strip?.fader_db ?? 0} fallback={0} unit="dB"
        min={-100} max={10} digits={1} scrub={.1} fineModifiers
        disabled={track.locked || Boolean(fader?.automated && !recordingFader)} onCommit={commitFader} />
      <span>{track.format === '5.1' ? '5.1' : t(track.format === 'mono' ? 'Mono' : track.format === 'adaptive' ? 'Adaptive' : 'Stereo')}</span>
      <DuckerPanel seconds={seconds} bridge={bridge} session={session} revision={revision} timeline={timeline} target={{ track: track.id }} automation={automation} />
    </>}
    {tab === 'effects' && (mode === 'track' && track?.kind === 'audio' ? <InsertSlots seconds={seconds} bridge={bridge} session={session} revision={revision} timeline={timeline} target={{ track: track.id }} automation={automation} inline /> : chosen.map(clip => <div key={clip.id}><strong>{clip.id}</strong><InsertSlots seconds={seconds} bridge={bridge} session={session} revision={revision} timeline={timeline} target={{ clip: clip.id }} inline /></div>))}
    {tab === 'file' && chosen.some(clip => clip.source.kind === 'media') && <FieldsLocked.Provider value={locked}>
      <AudioConfig key={JSON.stringify([session, timeline.id, chosen.map(clip => clip.id)])} clips={chosen.filter(clip => clip.source.kind === 'media')} timeline={timeline} media={media} commit={commit} bridge={bridge} session={session} />
    </FieldsLocked.Provider>}
  </aside>;
}
