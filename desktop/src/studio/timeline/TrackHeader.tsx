import { memo, useRef, useState, type ReactNode } from 'react';
import { Eye, EyeOff, Lock, Unlock, Volume2, VolumeX, Crosshair } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Op, Track } from '../engine';
import { NameField } from '../media/NameField';

/**
 * Which tracks a drag of a header's lower edge sizes: its own; with Alt, the tracks chosen; with
 * Ctrl+Alt, all of them (Fairlight, Reference Manual 20, pp. 3716-3717). A page that does not take
 * the wider ones sizes the track alone.
 */
export type HeightScope = 'one' | 'selected' | 'all';
const scopeOf = (event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean }): HeightScope =>
  !event.altKey ? 'one' : event.ctrlKey || event.metaKey ? 'all' : 'selected';

export const TrackHeader = memo(function TrackHeader({ timeline, track, height, onHeight, onEdit, targeted = false, onTarget, shownSubtitles = null,
  detail, renameOnClick = false, heightLocked = false }: {
  timeline: string; track: Track; height: number; onHeight: (id: string, height: number, scope?: HeightScope) => void;
  onEdit: (ops: Op[]) => Promise<boolean>;
  targeted?: boolean; onTarget?: (id: string) => void;
  /** The subtitle track of the timeline that is shown, where one is. */
  shownSubtitles?: string | null;
  /** What a page shows of the track beside its name: Fairlight's number, format and meter. */
  detail?: ReactNode;
  /** A single click on the name writes it, as Fairlight's headers do (p. 3669); else a double click or F2. */
  renameOnClick?: boolean;
  /** The height is locked to a preset: dragging the lower edge does not change it. */
  heightLocked?: boolean;
}) {
  const words = useWords().label.studio.timeline;
  const [renaming, setRenaming] = useState(false);
  const resizing = useRef<{ y: number; height: number; scope: HeightScope } | null>(null);
  const lock = track.locked ? words.unlock : words.lock;
  // A track of subtitles that is off is not drawn in the timeline viewer. As in DaVinci Resolve, one
  // is shown at a time (Reference Manual 20, p. 1084): showing one hides the one shown, in the same
  // transaction, which the engine takes in that order. Which are saved as SRT is chosen in the export.
  // The speaker of a track of sound is its mute (p. 3885), not Disable Track: it is on while the
  // track is heard. A click mutes a track that is heard; on one that is not, it clears the mute,
  // and turns on a track that is off (as one of a project of schema 2 may be), so that it is heard.
  const heard = track.kind === 'audio' && track.enabled && !track.mute;
  const enabled = track.kind === 'audio' ? heard ? words.mute : words.unmute
    : track.kind === 'subtitle' ? track.enabled ? words.hideSubtitles : words.showSubtitles : track.enabled ? words.hide : words.show;
  const Visible = track.kind === 'audio' ? heard ? Volume2 : VolumeX : track.enabled ? Eye : EyeOff;
  const sound: Op[] = heard ? [{ op: 'set_track', timeline, id: track.id, mute: true }]
    : [{ op: 'set_track', timeline, id: track.id, mute: false, ...(track.enabled ? {} : { enabled: true }) }];
  const changeHeight = (value: number, scope: HeightScope = 'one') => onHeight(track.id, Math.max(40, Math.min(240, value)), scope);
  return <div className="timeline-track-header" data-height-locked={heightLocked || undefined}>
    {detail}
    {renaming ? <NameField name={track.name} onCommit={name => onEdit([{ op: 'set_track', timeline, id: track.id, name }])} onCancel={() => setRenaming(false)} />
      : <button type="button" className="timeline-track-name" title={track.name}
        onClick={event => { if (renameOnClick && !event.ctrlKey && !event.metaKey && !event.shiftKey) setRenaming(true); }}
        onDoubleClick={() => setRenaming(true)}
        onKeyDown={event => { if (event.key === 'F2') { event.preventDefault(); setRenaming(true); } }}>{track.name}
        {track.kind === 'subtitle' && track.language && <span className="timeline-track-language" data-test="track-language"> · {track.language}</span>}</button>}
    <div className="timeline-track-switches">
      {track.kind !== 'subtitle' && onTarget && <button type="button" className="icon-btn" aria-label={`${words.target}: ${track.name}`} title={words.target} aria-pressed={targeted}
        onClick={() => onTarget(track.id)}><Crosshair size={13} /></button>}
      <button type="button" className="icon-btn" aria-label={`${lock}: ${track.name}`} title={lock} aria-pressed={track.locked}
        onClick={() => void onEdit([{ op: 'set_track', timeline, id: track.id, locked: !track.locked }])}>{track.locked ? <Lock size={13} /> : <Unlock size={13} />}</button>
      <button type="button" className="icon-btn" aria-label={`${enabled}: ${track.name}`} title={enabled} aria-pressed={track.kind === 'audio' ? heard : track.enabled}
        onClick={() => void onEdit(track.kind === 'audio' ? sound : [
          ...(track.kind === 'subtitle' && !track.enabled && shownSubtitles !== null && shownSubtitles !== track.id
            ? [{ op: 'set_track' as const, timeline, id: shownSubtitles, enabled: false }] : []),
          { op: 'set_track', timeline, id: track.id, enabled: !track.enabled },
        ])}><Visible size={14} /></button>
    </div>
    <div className="timeline-track-resize" role="separator" tabIndex={0} aria-label={`${words.height}: ${track.name}`}
      aria-orientation="horizontal" aria-valuemin={40} aria-valuemax={240} aria-valuenow={height} aria-disabled={heightLocked || undefined}
      onPointerDown={event => {
        if (event.button !== 0 || heightLocked) return;
        event.preventDefault(); event.stopPropagation(); event.currentTarget.setPointerCapture?.(event.pointerId);
        resizing.current = { y: event.clientY, height, scope: scopeOf(event) };
      }}
      onPointerMove={event => {
        const drag = resizing.current;
        if (drag) changeHeight(drag.height + event.clientY - drag.y, drag.scope);
      }}
      onPointerUp={event => { resizing.current = null; event.currentTarget.releasePointerCapture?.(event.pointerId); }}
      onPointerCancel={() => { resizing.current = null; }}
      onKeyDown={event => {
        if (heightLocked || (event.key !== 'ArrowDown' && event.key !== 'ArrowUp')) return;
        event.preventDefault(); event.stopPropagation();
        changeHeight(height + (event.key === 'ArrowDown' ? 8 : -8), scopeOf(event));
      }} />
  </div>;
});
