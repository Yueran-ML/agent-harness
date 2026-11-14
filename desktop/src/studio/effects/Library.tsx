import { Blend, Square, Type, AudioLines } from 'lucide-react';
import { useWords } from '../../app/locale';
import { startEffectDrag, type Effect } from './drag';

/**
 * The effects library of the Edit page, as DaVinci Resolve's toolbox: the transitions of picture and
 * of sound, a title and a solid colour, each dragged onto the timeline. A transition goes to the cut
 * it is dropped on; a title or a colour to the empty part of the video track it is dropped on.
 */
export function Library({ session }: { session: number }) {
  const words = useWords();
  const labels = words.label.studio.effects;
  const names = words.label.studio.timeline;
  const groups: { name: string; items: { effect: Effect; label: string; icon: typeof Blend }[] }[] = [
    { name: labels.videoTransitions, items: (['dissolve', 'dip_black', 'dip_white'] as const).map(effect => ({ effect, label: names[effect], icon: Blend })) },
    { name: labels.audioTransitions, items: [{ effect: 'crossfade', label: names.crossfade, icon: AudioLines }] },
    { name: labels.titles, items: [{ effect: 'title', label: labels.text, icon: Type }] },
    { name: labels.generators, items: [{ effect: 'color', label: names.color, icon: Square }] },
  ];
  return <section className="effects" aria-label={labels.title} data-test="effects">
    <header className="pool-head"><h2 className="overline">{labels.title}</h2></header>
    <div className="effects-list">
      {groups.map(group => <div key={group.name} className="effects-group">
        <h3 className="effects-name">{group.name}</h3>
        <ul role="list">
          {group.items.map(({ effect, label, icon: Icon }) => <li key={effect} className="effects-item" draggable data-effect={effect} title={words.hint.studio.inspector.library}
            onDragStart={event => startEffectDrag(event.dataTransfer, session, effect)}>
            <Icon size={14} aria-hidden="true" /><span>{label}</span>
          </li>)}
        </ul>
      </div>)}
      <p className="caption effects-hint">{words.hint.studio.inspector.library}</p>
    </div>
  </section>;
}
