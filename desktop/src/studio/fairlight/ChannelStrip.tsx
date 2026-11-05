import { useState, type PointerEvent } from 'react';
import { createPortal } from 'react-dom';
import type { MeterChannel } from '../engine';
import { NameField } from '../media/NameField';
import { Fader } from '../audio/Fader';
import { Meter } from '../audio/Meter';
import { faderReading, formatDb, type MixerStrip, type Modifiers } from '../audio/mixerModel';
import { useFairlightText } from './words';
import type { ProcessorProps } from '../audio/useProcessor';
import { defaultDynamics, defaultEq, ORDERINGS, processorWritable, type ProcessorModule } from '../audio/processorModel';
import { ProcessorThumbnail } from '../audio/ProcessorThumbnail';
import { PathSettings } from './PathSettings';

export function ChannelStrip({ strip, seconds, value, selected, meters, shadow, onSelect, onRename, onStart, onChange, onEnd, onCancel, onCommit, isActive, onToggle, onSweep, onEnter, processorContext, onProcessor, onStripSetting, compact = false }: {
  strip: MixerStrip; seconds: number; value?: number; selected: boolean; meters?: MeterChannel[]; shadow: boolean;
  onSelect: (modifiers: Modifiers) => void; onRename: (name: string) => Promise<boolean>;
  onStart: (modifiers: Modifiers) => boolean; onChange: (value: number) => void; onEnd: () => void; onCancel: () => void;
  onCommit: (value: number, modifiers?: Modifiers) => void;
  isActive: () => boolean;
  onToggle: (field: 'mute' | 'solo' | 'solo_safe', modifiers: Modifiers) => void;
  onSweep: (field: 'mute' | 'solo' | 'solo_safe', event: PointerEvent<HTMLButtonElement>) => void;
  onEnter: (field: 'mute' | 'solo', event: PointerEvent<HTMLButtonElement>) => void;
  processorContext: ProcessorProps; onProcessor: (module: ProcessorModule) => void;
  onStripSetting: (path: string, value: unknown, modifiers: Modifiers) => void;
  compact?: boolean;
}) {
  const t = useFairlightText(); const [renaming, setRenaming] = useState(false), [pathOpen, setPathOpen] = useState<{ x: number; y: number } | null>(null);
  const reading = faderReading(strip, seconds), model = strip.strip;
  return <article className="mixer-strip" data-mixer-strip={strip.key} data-selected={selected || undefined} data-disabled={!strip.enabled || undefined}
    aria-label={strip.name} onClick={event => { if (event.target === event.currentTarget) onSelect(event); }}>
    <div className="mixer-strip-color" style={{ background: strip.color }}>{strip.kind === 'track' ? 'A' : 'B'}{strip.number}</div>
    <dl className="mixer-summary">
      <div data-core><dt>{t('Input')}</dt><dd><button type="button" aria-label={`${t('Path Settings')} ${strip.name}`} onClick={event => {
        const rect = event.currentTarget.getBoundingClientRect(); setPathOpen(pathOpen ? null : { x: Math.max(8, Math.min(window.innerWidth - 220, rect.left)), y: Math.max(8, Math.min(window.innerHeight - 240, rect.bottom)) });
      }}>{t('Audio')}</button></dd></div>
      <div><dt>{t('Track FX')}</dt><dd>{model?.track_fx ? t('Ducker') : '—'}</dd></div>
      <div data-core><dt>{t('Order')}</dt><dd><select aria-label={`${t('Order')} ${strip.name}`} disabled={strip.locked} value={(model?.order ?? ORDERINGS[0]).join(',')}
        onChange={event => onStripSetting('order', event.target.value.split(','), {})}>{ORDERINGS.map(order => <option key={order.join(',')} value={order.join(',')}>{order.map(s => s === 'fx' ? 'FX' : s === 'eq' ? 'EQ' : 'DY').join(' › ')}</option>)}</select></dd></div>
      <div><dt>{t('Effects')}</dt><dd>{model?.inserts?.length ?? 0} / 6</dd></div>
      <div data-core><dt>{t('Effects In')}</dt><dd><button type="button" className="effects-in" aria-label={`${t('Effects In')} ${strip.name}`} aria-pressed={model?.inserts_on !== false}
        disabled={!processorWritable(strip, 'inserts_on')} onClick={event => onStripSetting('inserts_on', model?.inserts_on === false, event)}>{t(model?.inserts_on === false ? 'Off' : 'On')}</button></dd></div>
      <div data-core><dt>{t('Dynamics')}</dt><dd><ProcessorThumbnail name={strip.name} module="dynamics" openOnClick={compact} on={model?.dynamics?.on ?? false} disabled={!processorWritable(strip, 'dynamics')}
        settings={model?.dynamics ?? defaultDynamics()} onPaste={(settings, modifiers) => onStripSetting('dynamics', settings, modifiers)}
        context={processorContext} showCurve={Boolean(model?.dynamics)} onToggle={modifiers => onStripSetting('dynamics', { on: !model?.dynamics?.on }, modifiers)} onOpen={() => onProcessor('dynamics')} /></dd></div>
      <div data-core><dt>EQ</dt><dd><ProcessorThumbnail name={strip.name} module="eq" openOnClick={compact} on={model?.eq?.on ?? false} disabled={!processorWritable(strip, 'eq')}
        settings={model?.eq ?? defaultEq()} onPaste={(settings, modifiers) => onStripSetting('eq', settings, modifiers)}
        context={processorContext} showCurve={Boolean(model?.eq)} onToggle={modifiers => onStripSetting('eq', { on: !model?.eq?.on }, modifiers)} onOpen={() => onProcessor('eq')} /></dd></div>
      <div><dt>{t('Sends')}</dt><dd>{model?.sends?.length ?? 0}</dd></div>
      <div><dt>{t('Pan')}</dt><dd>{Math.round((model?.pan?.x ?? 0) * 100)}</dd></div>
      <div><dt>{t('Outputs')}</dt><dd title={strip.outputs.join(', ')}>{strip.outputs.join(', ') || '—'}</dd></div>
      <div><dt>VCA</dt><dd>{model?.vcas?.join(', ') || '—'}</dd></div>
    </dl>
    {pathOpen && createPortal(<PathSettings context={processorContext} stripKey={strip.key} anchor={pathOpen} onClose={() => setPathOpen(null)} />, document.body)}
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
