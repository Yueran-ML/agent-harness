import { memo, useMemo } from 'react';
import { useWords } from '../../app/locale';
import type { Media, Op, Rate, Track } from '../engine';
import { ClipView } from './ClipView';
import { spanStyle, transitionSpan, visible } from './geometry';
import { SubtitleCues } from './SubtitleTrack';
import { TrackHeader } from './TrackHeader';
import type { Waveforms } from './waveforms';

/**
 * A track: its header and what of it lies in the window `from`–`to` (frames). The window moves by
 * whole steps as the timeline scrolls (see `scrollWindow`), so a row is drawn again only where the
 * scroll crosses a step, or what it shows changes; not as the playhead moves.
 */
export const TrackRow = memo(function TrackRow({ timeline, track, media, rate, scale, from, to, height, waves, onEdit, onHeight, selected, targeted, onTarget, chosen = null, chosenCue = null, shownSubtitles = null }: {
  timeline: string; track: Track; media: ReadonlyMap<string, Media>; rate: Rate; scale: number; from: number; to: number;
  height: number; waves: Waveforms; onEdit: (ops: Op[]) => Promise<boolean>; onHeight: (id: string, height: number) => void;
  selected: ReadonlySet<string>; targeted: boolean; onTarget: (id: string) => void;
  /** The transition chosen, where it is on this track. */
  chosen?: string | null;
  /** The subtitle chosen, where it is on this track. */
  chosenCue?: string | null;
  /** The subtitle track of the timeline that is shown, for a track of subtitles. */
  shownSubtitles?: string | null;
}) {
  const all = useWords();
  const words = all.label.studio.timeline;
  const transitions = useMemo(() => {
    if (track.kind === 'subtitle') return [];
    const starts = new Map(track.clips.map(clip => [clip.id, clip.start]));
    return track.transitions.map(transition => ({ transition, ...transitionSpan(transition, starts.get(transition.right)!) }));
  }, [track]);
  return <div className="timeline-track" data-track={track.id} data-kind={track.kind} data-enabled={track.enabled} data-locked={track.locked} style={{ height }}>
    <TrackHeader timeline={timeline} track={track} height={height} onEdit={onEdit} onHeight={onHeight} targeted={targeted} onTarget={onTarget}
      shownSubtitles={shownSubtitles} />
    <div className="timeline-lane" role="listbox" aria-label={track.name} title={track.kind === 'subtitle' && !track.locked ? all.hint.studio.inspector.addCue : undefined} aria-multiselectable={track.kind === 'subtitle' ? undefined : true}>
      {track.kind === 'subtitle' ? <SubtitleCues track={track} from={from} to={to} chosen={chosenCue} />
        // A clip of video is given its own span, which does not change as the window moves; one of
        // sound is given the window, of which its waveform draws the tiles that it covers.
        : visible(track.clips, from, to).map(clip => <ClipView key={clip.id} clip={clip} kind={track.kind} selected={selected.has(clip.id)}
          media={clip.source.kind === 'media' ? media.get(clip.source.media) : undefined} rate={rate} scale={track.kind === 'audio' ? scale : undefined}
          from={track.kind === 'audio' ? from : clip.start} to={track.kind === 'audio' ? to : clip.start + clip.length} waves={waves} />)}
      {transitions.filter(span => span.start < to && span.start + span.length > from).map(({ transition, start, length }) => <div
        key={transition.id} className="timeline-transition" data-transition={transition.id} data-kind={transition.kind} data-selected={chosen === transition.id}
        style={spanStyle(start, length)} title={words[transition.kind]}><span>{words[transition.kind]}</span></div>)}
    </div>
  </div>;
});
