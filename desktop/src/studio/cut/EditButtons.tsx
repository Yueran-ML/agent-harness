import { ArrowDownToLine, Blend, Layers, ListEnd, Moon, Replace, SquareStack, type LucideIcon } from 'lucide-react';
import { useWords } from '../../app/locale';
import { shortcut } from '../../app/system';
import type { EditAction } from '../timeline/editKeys';

type Button = { id: string; action: EditAction; Icon: LucideIcon; key?: string; source: boolean;
  /** Whether it can never be pressed here, its hint saying why. */
  off?: boolean };

/**
 * The buttons of the Cut page under its viewer, as DaVinci Resolve has them: the edits that put
 * the source on the timeline, then the transitions put at the edit point nearest the playhead.
 * Each does what its key does. Close Up is not among them: in DaVinci Resolve it frames a close up
 * of a face, which this studio does not do. Source Overwrite is shown, but disabled: it syncs by
 * timecode, which the engine does not read of media, and without overlapping timecode DaVinci
 * Resolve's "does nothing" (Reference Manual 20, p. 563). F10 is Overwrite here, as in DaVinci
 * Resolve's Cut page (p. 565), which has no button for it.
 *
 * The edits take what the viewer has as its source, not what is chosen in the pool: a medium is
 * loaded by a double click on it there (or Enter), or by the source tape. A button that cannot be
 * pressed says why before it says what it does: there is no timeline, or the viewer has no source.
 */
const EDITS: readonly Button[] = [
  { id: 'smartInsert', action: { kind: 'place', edit: 'smart_insert' }, Icon: ArrowDownToLine, key: 'F9', source: true },
  { id: 'append', action: { kind: 'place', edit: 'append' }, Icon: ListEnd, key: 'Shift+F12', source: true },
  { id: 'rippleOverwrite', action: { kind: 'place', edit: 'ripple_overwrite' }, Icon: Replace, key: 'Shift+F10', source: true },
  { id: 'placeOnTop', action: { kind: 'place', edit: 'place_on_top' }, Icon: Layers, key: 'F12', source: true },
  { id: 'sourceOverwrite', action: { kind: 'place', edit: 'source_overwrite' }, Icon: SquareStack, source: true, off: true },
];
const TRANSITIONS: readonly Button[] = [
  { id: 'dissolve', action: { kind: 'quick', transition: 'dissolve' }, Icon: Blend, key: 'Ctrl+T', source: false },
  { id: 'dipBlack', action: { kind: 'quick', transition: 'dip_black' }, Icon: Moon, source: false },
];

export function EditButtons({ onAction, timeline, source }: {
  onAction: (action: EditAction) => void;
  /** Whether there is a timeline to edit, and a source to put on it. */
  timeline: boolean; source: boolean;
}) {
  const words = useWords();
  const labels = words.label.studio.cutPage;
  const hints = words.hint.studio.cutPage;
  const button = ({ id, action, Icon, key, source: needsSource, off = false }: Button) => {
    const label = labels[id as keyof typeof labels];
    const hint = id === 'dissolve' || id === 'dipBlack' ? hints.transition : hints[id as keyof typeof hints];
    const does = key ? `${hint} (${key === 'Ctrl+T' ? shortcut('T') : key})` : hint;
    // What keeps it from being pressed now, where something does: one that is never pressed here says so in its hint.
    const why = off ? null : !timeline ? hints.noTimeline : needsSource && !source ? hints.noSource : null;
    const disabled = off || why !== null;
    const title = why === null ? does : `${why} ${does}`;
    // A disabled button is not hovered: its hint is that of what holds it, and is read with it. Every
    // button is held, so that the one focused is not made anew as a source is loaded and it can be pressed.
    return <span key={id} className="cut-button-hold" title={disabled ? title : undefined} data-test={disabled ? `cut-${id}-why` : undefined}>
      <button type="button" className="btn btn-quiet cut-button" aria-label={label} title={title}
        disabled={disabled} onClick={() => onAction(action)} data-test={`cut-${id}`}>
        <Icon size={15} aria-hidden="true" /><span>{label}</span>
      </button>
    </span>;
  };
  return <div className="cut-buttons">
    <div className="cut-button-group" role="toolbar" aria-label={labels.edits}>{EDITS.map(button)}</div>
    <div className="cut-button-group" role="toolbar" aria-label={labels.transitions}>{TRANSITIONS.map(button)}</div>
  </div>;
}
