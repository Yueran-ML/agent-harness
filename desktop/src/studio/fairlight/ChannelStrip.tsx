import { useState, type PointerEvent } from 'react';
import type { MeterChannel } from '../engine';
import { NameField } from '../media/NameField';
import { Fader } from '../audio/Fader';
import { Meter } from '../audio/Meter';
import { faderReading, formatDb, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { useFairlightText } from './words';

export function ChannelStrip({ strip, seconds, value, selected, meters, shadow, onSelect, onRename, onStart, onChange, onEnd, onCancel, onCommit, isActive, onToggle, onSweep, onEnter }: {
  strip: MixerStrip; seconds: number; value?: number; selected: boolean; meters?: MeterChannel[]; shadow: boolean;
  onSelect: (modifiers: Modifiers) => void; onRename: (name: string) => Promise<boolean>;
  onStart: (modifiers: Modifiers) => boolean; onChange: (value: number) => void; onEnd: () => void; onCancel: () => void;
  onCommit: (value: number, modifiers?: Modifiers) => void;
  isActive: () => boolean;
  onToggle: (field: 'mute' | 'solo' | 'solo_safe', modifiers: Modifiers) => void;
  onSweep: (field: 'mute' | 'solo' | 'solo_safe', event: PointerEvent<HTMLButtonElement>) => void;
  onEnter: (field: 'mute' | 'solo', event: PointerEvent<HTMLButtonElement>) => void;
}) {
  const t = useFairlightText(); const [renaming, setRenaming] = useState(false);
  const reading = faderReading(strip, seconds), model = strip.strip;
  return <article className="mixer-strip" data-mixer-strip={strip.key} data-selected={selected || undefined} data-disabled={!strip.enabled || undefined}
    aria-label={strip.name} onClick={event => { if (event.target === event.currentTarget) onSelect(event); }}>
    <div className="mixer-strip-color" style={{ background: strip.color }}>{strip.kind === 'track' ? 'A' : 'B'}{strip.number}</div>
    <dl className="mixer-summary">
      <div><dt>{t('Input')}</dt><dd>{t('Audio')}</dd></div>
      <div><dt>{t('Track FX')}</dt><dd>{model?.track_fx ? t('Ducker') : '—'}</dd></div>
      <div><dt>{t('Order')}</dt><dd>{(model?.order ?? ['fx', 'dynamics', 'eq']).map(s => s === 'fx' ? 'FX' : s === 'eq' ? 'EQ' : 'DY').join(' › ')}</dd></div>
      <div><dt>{t('Effects')}</dt><dd>{model?.inserts?.length ?? 0} / 6</dd></div>
      <div><dt>{t('Effects In')}</dt><dd>{t(model?.inserts_on === false ? 'Off' : 'On')}</dd></div>
      <div><dt>{t('Dynamics')}</dt><dd>{t(model?.dynamics?.on ? 'On' : 'Off')}</dd></div>
      <div><dt>EQ</dt><dd>{t(model?.eq?.on ? 'On' : 'Off')}</dd></div>
      <div><dt>{t('Sends')}</dt><dd>{model?.sends?.length ?? 0}</dd></div>
      <div><dt>{t('Pan')}</dt><dd>{Math.round((model?.pan?.x ?? 0) * 100)}</dd></div>
      <div><dt>{t('Outputs')}</dt><dd title={strip.outputs.join(', ')}>{strip.outputs.join(', ') || '—'}</dd></div>
      <div><dt>VCA</dt><dd>{model?.vcas?.join(', ') || '—'}</dd></div>
    </dl>
    {renaming ? <NameField name={strip.name} onCancel={() => setRenaming(false)} onCommit={onRename} />
      : <button type="button" className="mixer-name" onClick={event => onSelect(event)} onDoubleClick={() => { if (!strip.locked) setRenaming(true); }}>{strip.name}</button>}
    <div className="mixer-switches">{(['solo', 'mute'] as const).map(field => <button key={field} type="button" disabled={strip.locked}
      aria-label={`${t(field)} ${strip.name}`} aria-pressed={strip[field]} data-safe={field === 'solo' && strip.safe || undefined}
      title={field === 'solo' ? `${t('Solo Safe')}: Ctrl+Shift` : t('mute')}
      onPointerDown={event => onSweep(field === 'solo' && event.ctrlKey && event.shiftKey ? 'solo_safe' : field, event)}
      onPointerEnter={event => onEnter(field, event)}
      onClick={event => { if (event.detail === 0) onToggle(field === 'solo' && event.ctrlKey && event.shiftKey ? 'solo_safe' : field, event); }}>{field === 'solo' ? strip.safe ? 'S·' : 'S' : 'M'}</button>)}</div>
    <div className="mixer-fader-meter">
      <Fader label={`${t('Fader')} ${strip.name}`} valueLabel={`${t('Fader value')} ${strip.name}`} value={reading.automated ? reading.value : value ?? reading.value}
        disabled={strip.locked} readOnly={reading.automated} hasLane={reading.hasLane} onStart={onStart} onChange={onChange} onEnd={onEnd} onCancel={onCancel} onCommit={onCommit} isActive={isActive} />
      <Meter label={strip.name} channels={strip.channels} values={meters} disabled={!strip.enabled} shadow={shadow} />
    </div>
    {reading.automated && <small className="mixer-automation" title={t('Automation controls playback. Static settings do not replace the lane.')}>{t('Automation controls playback')}<br />{t('Base value')}: {formatDb(strip.db)} dB</small>}
  </article>;
}
