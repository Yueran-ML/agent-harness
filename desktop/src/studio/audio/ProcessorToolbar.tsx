import { useState } from 'react';
import type { Dynamics, Equalizer } from '../engine';
import { useFairlightText } from '../fairlight/words';
import type { Modifiers } from './mixerModel';
import type { ProcessorModule } from './processorModel';

// Only this module's settings; no strip identity, routing, inserts, fader or pan.
const clipboard: Partial<Record<ProcessorModule, Equalizer | Dynamics>> = {};
export function copyProcessor(module: ProcessorModule, value: Equalizer | Dynamics) { clipboard[module] = structuredClone(value); }
export function pastedProcessor(module: ProcessorModule, value: Equalizer | Dynamics) {
  const saved = clipboard[module];
  return saved && (module !== 'eq' || (saved as Equalizer).bands.length === (value as Equalizer).bands.length) ? structuredClone(saved) : null;
}
export function ProcessorToolbar<T extends Equalizer | Dynamics>({ module, value, disabled, reset, commit, enable }: {
  module: ProcessorModule; value: T; disabled: boolean; reset: T; commit: (value: T, modifiers?: Modifiers) => void; enable: (on: boolean, modifiers?: Modifiers) => void;
}) {
  const t = useFairlightText(), [, redraw] = useState(0);
  return <div className="processor-toolbar">
    <label><input type="checkbox" checked={value.on} disabled={disabled} aria-label={t('Enable')} onClick={event => { event.preventDefault(); enable(!value.on, event); }} readOnly />{t('Enable')}</label>
    <button type="button" disabled={disabled} onClick={event => commit(reset, event)}>{t('Reset')}</button>
    <button type="button" onClick={() => { copyProcessor(module, value); redraw(n => n + 1); }}>{t('Copy')}</button>
    <button type="button" disabled={disabled} onClick={event => { const saved = pastedProcessor(module, value);
      if (saved) commit(saved as T, event);
    }}>{t('Paste')}</button>
  </div>;
}
