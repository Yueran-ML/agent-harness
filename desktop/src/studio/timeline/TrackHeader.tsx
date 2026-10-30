import { memo, useRef, useState } from 'react';
import { Eye, EyeOff, Lock, Unlock, Volume2, VolumeX, Crosshair } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Op, Track } from '../engine';
import { NameField } from '../media/NameField';

export const TrackHeader = memo(function TrackHeader({ timeline, track, height, onHeight, onEdit, targeted = false, onTarget, shownSubtitles = null }: {
  timeline: string; track: Track; height: number; onHeight: (id: string, height: number) => void; onEdit: (ops: Op[]) => Promise<boolean>;
  targeted?: boolean; onTarget?: (id: string) => void;
  /** The subtitle track of the timeline that is shown, where one is. */
  shownSubtitles?: string | null;
}) {
  const words = useWords().label.studio.timeline;
  const [renaming, setRenaming] = useState(false);
  const resizing = useRef<{ y: number; height: number } | null>(null);
  const lock = track.locked ? words.unlock : words.lock;
  // A track of subtitles that is off is not drawn in the timeline viewer. As in DaVinci Resolve, one
  // is shown at a time (Reference Manual 20, p. 1084): showing one hides the one shown, in the same
  // transaction, which the engine takes in that order. Which are saved as SRT is chosen in the export.
  const enabled = track.kind === 'audio' ? track.enabled ? words.mute : words.unmute
    : track.kind === 'subtitle' ? track.enabled ? words.hideSubtitles : words.showSubtitles : track.enabled ? words.hide : words.show;
  const Visible = track.kind === 'audio' ? track.enabled ? Volume2 : VolumeX : track.enabled ? Eye : EyeOff;
  const changeHeight = (value: number) => onHeight(track.id, Math.max(40, Math.min(240, value)));
  return <div className="timeline-track-header">
    {renaming ? <NameField name={track.name} onCommit={name => onEdit([{ op: 'set_track', timeline, id: track.id, name }])} onCancel={() => setRenaming(false)} />
      : <button type="button" className="timeline-track-name" title={track.name} onDoubleClick={() => setRenaming(true)}
        onKeyDown={event => { if (event.key === 'F2') { event.preventDefault(); setRenaming(true); } }}>{track.name}
        {track.kind === 'subtitle' && track.language && <span className="timeline-track-language" data-test="track-language"> · {track.language}</span>}</button>}
    <div className="timeline-track-switches">
      {track.kind !== 'subtitle' && onTarget && <button type="button" className="icon-btn" aria-label={`${words.target}: ${track.name}`} title={words.target} aria-pressed={targeted}
        onClick={() => onTarget(track.id)}><Crosshair size={13} /></button>}
      <button type="button" className="icon-btn" aria-label={`${lock}: ${track.name}`} title={lock} aria-pressed={track.locked}
        onClick={() => void onEdit([{ op: 'set_track', timeline, id: track.id, locked: !track.locked }])}>{track.locked ? <Lock size={13} /> : <Unlock size={13} />}</button>
      <button type="button" className="icon-btn" aria-label={`${enabled}: ${track.name}`} title={enabled} aria-pressed={track.enabled}
        onClick={() => void onEdit([
          ...(track.kind === 'subtitle' && !track.enabled && shownSubtitles !== null && shownSubtitles !== track.id
            ? [{ op: 'set_track' as const, timeline, id: shownSubtitles, enabled: false }] : []),
          { op: 'set_track', timeline, id: track.id, enabled: !track.enabled },
        ])}><Visible size={14} /></button>
    </div>
    <div className="timeline-track-resize" role="separator" tabIndex={0} aria-label={`${words.height}: ${track.name}`}
      aria-orientation="horizontal" aria-valuemin={40} aria-valuemax={240} aria-valuenow={height}
      onPointerDown={event => { if (event.button !== 0) return; event.preventDefault(); event.stopPropagation(); event.currentTarget.setPointerCapture?.(event.pointerId); resizing.current = { y: event.clientY, height }; }}
      onPointerMove={event => { if (resizing.current) changeHeight(resizing.current.height + event.clientY - resizing.current.y); }}
      onPointerUp={event => { resizing.current = null; event.currentTarget.releasePointerCapture?.(event.pointerId); }}
      onPointerCancel={() => { resizing.current = null; }}
      onKeyDown={event => { if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); event.stopPropagation(); changeHeight(height + (event.key === 'ArrowDown' ? 8 : -8)); } }} />
  </div>;
});
