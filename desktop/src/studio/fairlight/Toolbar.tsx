import { useFairlightText } from './words';
import type { Timeline } from '../engine';
import type { FairlightView } from './viewState';
export function Toolbar({ view, update, reset, timelines, selected, onSelect }: {
  view: FairlightView; update: (patch: Partial<FairlightView>) => void; reset: () => void;
  timelines: readonly Timeline[]; selected: string | null; onSelect: (id: string) => void;
}) {
  const t = useFairlightText();
  return <header className="fairlight-toolbar">
    {([['pool', 'Media Pool'], ['effects', 'Effects'], ['index', 'Index'], ['groups', 'Groups'], ['mixer', 'Mixer'], ['meters', 'Meters'], ['inspector', 'Inspector']] as const)
      .map(([key, name]) => <button type="button" key={key} aria-pressed={view[key]} onClick={() => update({ [key]: !view[key] })}>{t(name)}</button>)}
    <select aria-label={t('Fairlight timeline')} value={selected ?? ''} onChange={event => onSelect(event.target.value)} disabled={!timelines.length}>
      {!timelines.length && <option value="">{t('No timeline')}</option>}{timelines.map(timeline => <option key={timeline.id} value={timeline.id}>{timeline.name}</option>)}
    </select><button type="button" onClick={reset}>{t('Reset Fairlight Layout')}</button>
  </header>;
}
