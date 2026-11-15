import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Clip, Media, Timeline, Track } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { channelNames } from '../fairlight/ChannelLanes';
import { ChannelWaveform } from '../timeline/ChannelWaveform';
import { Waveforms } from '../timeline/waveforms';
import { frameAtSource, sourceAtFrame } from '../timeline/audioKeys';
import { retimeCourse } from '../retime';
import { AuditionCursor, FileTransport, pointerFraction, useFileAudition } from './FileAudition';

export function useFileWaveforms(bridge: Pick<EngineBridge, 'call' | 'onEvent'>, session: number) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const waves = useMemo(() => new Waveforms(bridge, session), [bridge, session]);
  useEffect(() => { waves.connect(); return () => waves.disconnect(); }, [waves]);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const measure = () => setWidth(element.clientWidth);
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, []);
  return { box, width, waves };
}

/** Per-channel source views; timeline instances deliberately have no composite lane. */
export function FileWaveforms({ bridge, session, clip, track, media, timeline, selected, onSelect, unapplied = false }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; clip: Clip; track: Track;
  media?: Media; timeline: Timeline; selected: readonly number[]; onSelect: (index: number, extend: boolean) => void; unapplied?: boolean;
}) {
  const t = useFairlightText(), { box, width, waves } = useFileWaveforms(bridge, session);
  const start = clip.source.kind === 'media' ? clip.source.in.ticks / clip.source.in.scale : 0;
  // A clip with speed points shows its media from where it begins in it, the lowest place of their path, to the highest.
  const course = retimeCourse(clip);
  const end = course ? start + Number(course.highest) / course.unit * timeline.rate.den / timeline.rate.num
    : start + clip.length * timeline.rate.den / timeline.rate.num * clip.speed.num / clip.speed.den;
  // The time of the source under a pointer so far along the lane, which is as long as the clip: along the speed points, where the clip has them.
  const sourceAt = (fraction: number) => (course ? sourceAtFrame(clip, timeline.rate, clip.start + fraction * clip.length)
    : start + (clip.reverse ? 1 - fraction : fraction) * (end - start));
  const available = !unapplied && clip.source.kind === 'media' && media?.facts.audio !== undefined && media?.facts.audio !== null;
  const limit = media ? media.facts.duration.ticks / media.facts.duration.scale : end;
  const audition = useFileAudition(bridge, session, { kind: 'clip', timeline: timeline.id, clip: clip.id }, Math.max(0, start), Math.min(end, limit), !available);
  return <div ref={box} className="file-waveforms" role="group" aria-label={t('Channels')} onKeyDown={audition.keyDown}>
    {channelNames(track).map((name, index) => <button key={index} type="button" className="file-channel-waveform"
      aria-label={`${t('Channel')} ${index + 1}`} aria-pressed={selected.includes(index)}
      onClick={event => {
        const extend = event.ctrlKey || event.metaKey || event.shiftKey;
        onSelect(index, extend);
        if (!extend && available) {
          // Enter or Space on the lane has no pointer position: keep the current one.
          if (event.detail === 0) { audition.locate(audition.position, index); return; }
          const fraction = pointerFraction(event);
          if (fraction !== null) audition.locate(sourceAt(fraction), index);
        }
      }} onPointerMove={event => {
        if (!available || event.buttons || event.ctrlKey || event.metaKey || event.shiftKey || event.pointerType === 'touch') return;
        const fraction = pointerFraction(event);
        if (fraction !== null) audition.skim(sourceAt(fraction), index);
      }} onPointerLeave={audition.leave}>
      <span className="file-channel-name">{name}</span>
      {width > 0 && <ChannelWaveform clip={clip} track={track} media={media} index={index} rate={timeline.rate}
        from={clip.start} to={clip.start + clip.length} scale={width / Math.max(1, clip.length)} waves={waves} />}
      {/* With speed points the cursor is at the first frame of the clip that reads the time auditioned. */}
      {course ? <AuditionCursor position={frameAtSource(clip, timeline.rate, audition.position)} start={clip.start} end={clip.start + clip.length} />
        : <AuditionCursor position={audition.position} start={start} end={end} reverse={clip.reverse} />}
    </button>)}
    <FileTransport audition={audition} disabled={!available || Math.min(end, limit) <= Math.max(0, start)} />
    {unapplied && <p role="status">{t('Apply channel changes before auditioning them.')}</p>}
  </div>;
}
