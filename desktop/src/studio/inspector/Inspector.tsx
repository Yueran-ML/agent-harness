import { useEffect, useMemo, useState } from 'react';
import { Gauge, X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Clip, ClipTrack, Media, Timeline } from '../engine';
import { clipLocations } from '../timeline/selection';
import { ClipAudio } from './ClipAudio';
import { ClipColor, ClipTitle, isColor, isTitle } from './ClipTitle';
import { ClipVideo } from './ClipVideo';
import { CuePanel, cueOf } from './CuePanel';
import { FieldsLocked } from './fields';
import { TimelineSettings } from './TimelineSettings';
import { TransitionPanel } from './TransitionPanel';
import { useCommit } from './useCommit';
import { refusalOf } from './refusal';
import { AudioConfig } from './AudioConfig';
import { InsertSlots } from '../fairlight/InsertSlots';
import { useFairlightText } from '../fairlight/words';

type Tab = 'source' | 'video' | 'audio' | 'file' | 'effects';

/**
 * The inspector of the Edit page, as DaVinci Resolve's: what is chosen on the timeline, set here.
 *
 * A subtitle chosen shows its text and its timing; a transition chosen shows its own panel. Clips chosen show tabs: Video for those on video tracks,
 * Audio for those on audio tracks, and Title (or Generator) where every clip on a video track is a
 * title (or a solid colour). Several clips show only what they all have; a field where they differ
 * shows a dash, and a change sets them all, in one transaction. Nothing chosen shows the settings of
 * the timeline. Each change is one transaction, worked out from the timeline as the engine has it,
 * with its revision; what the engine refuses is said in a sentence at the top.
 */
export function Inspector({ bridge, session, revision, timeline, media, clips: ids, transition: transitionId, cue: cueId = null, onSpeed, frame = 0, onSeek = () => {} }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number; timeline: Timeline | null; media: readonly Media[];
  clips: readonly string[]; transition: string | null; cue?: string | null; onSpeed: (clips: string[]) => void;
  frame?: number; onSeek?: (frame: number, scrubbing: boolean) => void;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.inspector;
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  // Clips on locked tracks cannot be changed here (Reference Manual 20, p. 762): where some chosen
  // are on tracks that are not locked, the fields show and set those; where all are locked, the
  // fields show them, off, and say why.
  const chosen = useMemo(() => {
    if (!timeline) return { video: [] as Clip[], audio: [] as Clip[], all: [] as Clip[], locked: false };
    const locations = clipLocations(timeline);
    const found = ids.map(id => locations.get(id)).filter(located => located !== undefined);
    const open = found.filter(({ track }) => !track.locked);
    const all = open.length > 0 ? open : found;
    return {
      video: all.filter(({ track }) => track.kind === 'video').map(({ clip }) => clip),
      audio: all.filter(({ track }) => track.kind === 'audio').map(({ clip }) => clip),
      all: all.map(({ clip }) => clip),
      locked: open.length === 0 && found.length > 0,
    };
  }, [timeline, ids]);
  const found = useMemo(() => {
    if (!timeline || transitionId === null) return null;
    for (const track of timeline.tracks) {
      if (track.kind === 'subtitle') continue;
      const transition = track.transitions.find(item => item.id === transitionId);
      if (transition) return { transition, track: track as ClipTrack };
    }
    return null;
  }, [timeline, transitionId]);
  const cue = useMemo(() => (timeline && cueId !== null ? cueOf(timeline, cueId) : null), [timeline, cueId]);
  const titles = chosen.video.length > 0 && chosen.video.every(isTitle) ? chosen.video.filter(isTitle) : [];
  const colors = chosen.video.length > 0 && chosen.video.every(isColor) ? chosen.video.filter(isColor) : [];
  const tabs: Tab[] = [
    ...(titles.length > 0 || colors.length > 0 ? ['source' as const] : []),
    ...(chosen.video.length > 0 ? ['video' as const] : []),
    ...(chosen.audio.length > 0 ? ['audio' as const, 'effects' as const] : []),
    ...(chosen.audio.some(clip => clip.source.kind === 'media') ? ['file' as const] : []),
  ];
  const [tab, setTab] = useState<Tab>('video');
  const shown = tabs.includes(tab) ? tab : tabs[0];
  // A new choice of clips starts at the first tab that it has.
  const tabKey = tabs.join(' ');
  const firstTab = tabs[0] ?? 'video';
  useEffect(() => { setTab(firstTab); }, [tabKey, firstTab]);
  const mediaClips = chosen.all.filter(clip => clip.source.kind === 'media');
  const name = (clip: Clip) => clip.source.kind === 'media' ? media.find(item => item.id === (clip.source as { media: string }).media)?.name ?? clip.source.media
    : clip.source.kind === 'title' ? clip.source.text : words.label.studio.timeline.color;
  const tabName: Record<Tab, string> = { source: titles.length > 0 ? labels.title : labels.generator, video: labels.video, audio: labels.audio, file: t('File'), effects:t('Effects') };

  let body;
  if (!timeline) body = <p className="caption">{words.hint.studio.timeline.empty}</p>;
  else if (cue) {
    body = <>
      <p className="inspector-what">{labels.subtitle}</p>
      <CuePanel key={cue.id} cue={cue} commit={commit} />
    </>;
  } else if (found) {
    body = <>
      <p className="inspector-what">{labels.transition} · {words.label.studio.timeline[found.transition.kind]}</p>
      <TransitionPanel transition={found.transition} track={found.track} timeline={timeline} commit={commit} />
    </>;
  } else if (chosen.all.length > 0) {
    body = <>
      <div className="inspector-what">
        <span className="inspector-name">{chosen.all.length === 1 ? name(chosen.all[0]) : null}</span>
        <span className="caption">{words.label.studio.media.selected(chosen.all.length)}</span>
        {mediaClips.length > 0 && <button type="button" className="icon-btn" aria-label={labels.changeSpeed} title={`${labels.changeSpeed} (R)`}
          disabled={chosen.locked} onClick={() => onSpeed(chosen.all.map(clip => clip.id))} data-test="open-speed"><Gauge size={14} /></button>}
      </div>
      {chosen.locked && <p className="caption" data-test="inspector-locked">{words.hint.studio.inspector.locked}</p>}
      {tabs.length > 1 && <div className="inspector-tabs" role="tablist" aria-label={words.label.studio.editPage.inspector}>
        {tabs.map(each => <button key={each} type="button" role="tab" aria-selected={each === shown} onClick={() => setTab(each)}>{tabName[each]}</button>)}
      </div>}
      <FieldsLocked.Provider value={chosen.locked}>
        {shown === 'effects' && chosen.audio.map(clip => <section key={clip.id} aria-label={clip.id}><strong>{name(clip)}</strong><InsertSlots bridge={bridge} session={session} revision={revision} timeline={timeline} target={{clip:clip.id}} seconds={frame * timeline.rate.den / timeline.rate.num} inline /></section>)}
        {shown === 'file' && <AudioConfig key={JSON.stringify([session, timeline.id, chosen.audio.map(clip => clip.id)])}
          clips={chosen.audio.filter(clip => clip.source.kind === 'media')} timeline={timeline} media={media} commit={commit} bridge={bridge} session={session} />}
        {shown === 'source' && titles.length > 0 && <ClipTitle clips={titles} timeline={timeline} commit={commit} />}
        {shown === 'source' && colors.length > 0 && <ClipColor clips={colors} commit={commit} />}
        {shown === 'video' && <ClipVideo clips={chosen.video} timeline={timeline} commit={commit} />}
        {shown === 'audio' && <ClipAudio key={JSON.stringify([session, timeline.id, chosen.audio.map(clip => clip.id)])} clips={chosen.audio} timeline={timeline} commit={commit}
          bridge={bridge} session={session} revision={revision} media={media} frame={frame} onSeek={onSeek} onFailure={error => setRefusal(refusalOf(error, words.hint.studio))} />}
      </FieldsLocked.Provider>
    </>;
  } else {
    body = <>
      <p className="caption">{words.hint.studio.inspectorEmpty}</p>
      <h3 className="inspector-section">{labels.settings}</h3>
      <TimelineSettings timeline={timeline} commit={commit} />
    </>;
  }
  return <div className="inspector" data-test="inspector">
    {refusal && <div className="inspector-problem" role="alert">
      <p>{refusal.text}{refusal.longest !== null && <span> {words.hint.studio.inspector.longest(refusal.longest)}</span>}</p>
      {refusal.longest !== null && refusal.onLongest && <button type="button" className="btn btn-quiet" data-test="use-longest" disabled={!found || found.track.locked}
        onClick={() => { const { onLongest, longest } = refusal; setRefusal(null); onLongest?.(longest!); }}>{labels.useLongest(refusal.longest)}</button>}
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={14} /></button>
    </div>}
    {body}
  </div>;
}
