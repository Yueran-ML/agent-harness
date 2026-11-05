import type { Dynamics, SourceRef } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { Knob } from './Knob';
import { defaultDynamics, sidechainSources, targetKey } from './processorModel';
import { ProcessorToolbar } from './ProcessorToolbar';
import { useProcessor, type ProcessorProps } from './useProcessor';
import { useProcessorCurve } from './useProcessorCurve';
import { clamp } from './taper';
import './processors.css';

type Section = 'expander' | 'compressor' | 'limiter';
export function DynamicsPanel(props: ProcessorProps) {
  const t = useFairlightText(), control = useProcessor(props, 'dynamics'), { curve, problem } = useProcessorCurve(props, 'dynamics');
  const value = control.value, defaults = defaultDynamics(), sc = value.compressor.sidechain;
  const patch = (section: Section, fields: Record<string, unknown>, base = value): Dynamics => ({ ...base, [section]: { ...base[section], ...fields } });
  const sources = 'clip' in props.target ? [] : sidechainSources(props.timeline, props.target);
  const number = (section: Section, field: string, label: string, min: number, max: number, unit = '') => {
    const sectionName = t(section === 'expander' ? 'Expander' : section === 'compressor' ? 'Compressor' : 'Limiter');
    return <Knob key={field} path={`dynamics.${section}.${field}`} label={`${sectionName} ${label}`} value={(value[section] as unknown as Record<string, number>)[field]} min={min} max={max} unit={unit}
      fallback={(defaults[section] as unknown as Record<string, number>)[field]} disabled={control.disabled || section === 'expander' && field === 'ratio' && value.expander.mode === 'gate'} onStart={control.start} isActive={control.gesture.active}
      onChange={n => control.change(patch(section, { [field]: n }, control.snapshot()))} onEnd={control.end} onCancel={control.cancel} onCommit={n => control.commit(patch(section, { [field]: n }))} />;
  };
  return <section className="processor-panel dynamics-panel" aria-label={t('Dynamics')} data-audio-owner={targetKey(props.target)} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); control.cancel(); } }}>
    <ProcessorToolbar module="dynamics" value={value} disabled={control.disabled} reset={defaults} commit={control.commit} enable={control.enable} />
    <p className="processor-note">{t('Static model curve')}</p>
    {(control.problem || problem) && <p role="alert">{control.problem ?? problem}</p>}
    {control.disabled && <p>{t('Locked or automated')}</p>}
    <svg className="processor-curve dynamics-curve" viewBox="-35 -10 345 335" aria-label={t('Dynamics curve')}>
      {[-60, -40, -20, 0].map(db => <g key={db}><line x1={(db + 60) * 5} x2={(db + 60) * 5} y1="0" y2="300" className="processor-grid" /><line x1="0" x2="300" y1={-db * 5} y2={-db * 5} className="processor-grid" /><text x={(db + 60) * 5} y="318" textAnchor="middle">{db}</text><text x="-5" y={4 - db * 5} textAnchor="end">{db}</text></g>)}
      {(['expander', 'compressor', 'limiter'] as const).filter(s => value[s].on).map(s => <line key={s} x1={(value[s].threshold_db + 60) * 5} x2={(value[s].threshold_db + 60) * 5} y1="0" y2="300" className="dynamics-threshold" />)}
      {curve && <polyline data-testid="dynamics-response" fill="none" className="processor-response" points={curve.x.map((db, i) => `${(db + 60) * 5},${-clamp(curve.y[i], -60, 0) * 5}`).join(' ')} />}
    </svg>
    <div className="dynamics-modules">{(['expander', 'compressor', 'limiter'] as const).map(section => <fieldset key={section} disabled={control.disabled}>
      <legend>{t(section === 'expander' ? 'Expander / Gate' : section === 'compressor' ? 'Compressor' : 'Limiter')}</legend>
      {section === 'expander' ? (['expander', 'gate'] as const).map(mode => <label key={mode}><input type="checkbox" aria-label={t(mode === 'gate' ? 'Gate' : 'Expander')} checked={value.expander.on && value.expander.mode === mode}
        onChange={event => control.commit(patch('expander', { on: event.target.checked, mode }))} />{t(mode === 'gate' ? 'Gate' : 'Expander')}</label>)
        : <label><input type="checkbox" checked={value[section].on} onChange={event => control.commit(patch(section, { on: event.target.checked }))} />{t('On')}</label>}
      {number(section, 'threshold_db', t('Threshold'), -50, 0, 'dB')}
      {section === 'expander' && number(section, 'range_db', t('Range'), 0, 60.2, 'dB')}
      {section !== 'limiter' && number(section, 'ratio', t('Ratio'), section === 'expander' ? 1.1 : 1, section === 'expander' ? 3 : 10)}
      {section === 'compressor' && <>{number(section, 'knee_db', t('Knee'), 0, 20, 'dB')}{number(section, 'mix', t('Dry Mix'), 0, 100, '%')}</>}
      {number(section, 'attack_ms', t('Attack'), 0, 100, 'ms')}{number(section, 'hold_ms', t('Hold'), 0, 4000, 'ms')}{number(section, 'release_ms', t('Release'), 50, 4000, 'ms')}
    </fieldset>)}</div>
    <fieldset disabled={control.disabled}><legend>{t('Sidechain')}</legend>
      {(['on', 'listen'] as const).map(field => <label key={field}><input type="checkbox" aria-label={`${t('Sidechain')} ${t(field === 'on' ? 'On' : 'Listen')}`} checked={sc[field]}
        onChange={event => control.commit(patch('compressor', { sidechain: { ...sc, [field]: event.target.checked } }))} />{t(field === 'on' ? 'On' : 'Listen')}</label>)}
      <label>{t('Source')}<select aria-label={t('Sidechain Source')} value={sc.source ? targetKey(sc.source) : ''} onChange={event => {
        const source: SourceRef | null = sources.find(s => s.key === event.target.value)?.source ?? null;
        control.commit(patch('compressor', { sidechain: { ...sc, source } }));
      }}><option value="">{t('None')}</option>{sources.map(s => <option key={s.key} value={s.key} disabled={s.disabled}>{s.name}{s.disabled ? ` (${t('Routing cycle')})` : ''}</option>)}</select></label>
    </fieldset>
    <Knob path="dynamics.makeup_db" label={t('Make Up')} value={value.makeup_db} min={-20} max={20} fallback={0} unit="dB" disabled={control.disabled} onStart={control.start} isActive={control.gesture.active}
      onChange={makeup_db => control.change({ ...control.snapshot(), makeup_db })} onEnd={control.end} onCancel={control.cancel} onCommit={makeup_db => control.commit({ ...value, makeup_db })} />
  </section>;
}
