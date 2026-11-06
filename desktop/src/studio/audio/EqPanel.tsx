import { useEffect, useRef } from 'react';
import type { EqBand, EqKind, EqRange, Equalizer } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { Knob } from './Knob';
import { changeBand, defaultEq, eqBandKinds, eqBounds, eqHasGain, eqHasQ, EQ_ZONES, frequencyAt, frequencyPosition, targetKey } from './processorModel';
import { ProcessorToolbar } from './ProcessorToolbar';
import { useProcessor, type ProcessorProps } from './useProcessor';
import { useProcessorCurve } from './useProcessorCurve';
import { clamp } from './taper';
import './processors.css';

export function EqPanel(props: ProcessorProps) {
  const t = useFairlightText(), control = useProcessor(props, 'eq'), { curve, problem } = useProcessorCurve(props, 'eq');
  const eq = control.value, sampleRate = props.timeline.sample_rate, clip = 'clip' in props.target;
  const defaults = defaultEq(clip);
  const change = (index: number, patch: Partial<EqBand>, base = control.snapshot()) => changeBand(base, index, patch, sampleRate);
  return <section className="processor-panel eq-panel" aria-label="EQ" data-audio-owner={targetKey(props.target)} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); control.cancel(); } }}>
    <ProcessorToolbar module="eq" value={eq} disabled={control.disabled} reset={defaults} commit={control.commit} enable={control.enable} />
    <p className="processor-note">{t('Static model curve')}</p>
    {(control.problem || problem) && <p role="alert">{control.problem ?? problem}</p>}
    {control.disabled && <p>{t('Locked or automated')}</p>}
    <svg className="processor-curve" viewBox="-32 -20 540 245" preserveAspectRatio="xMidYMid meet" aria-label={t('EQ response')}>
      {[-20, 0, 20].map(db => <g key={db}><line x1="0" x2="500" y1={100 - db * 5} y2={100 - db * 5} className="processor-grid" /><text x="-5" y={104 - db * 5} textAnchor="end">{db}</text></g>)}
      {[20, 100, 1000, 10000, 20000].map(hz => <g key={hz}><line x1={frequencyPosition(hz) * 500} x2={frequencyPosition(hz) * 500} y1="0" y2="200" className="processor-grid" /><text x={frequencyPosition(hz) * 500} y="220" textAnchor="middle">{hz >= 1000 ? `${hz / 1000}k` : hz}</text></g>)}
      {curve && <polyline data-testid="eq-response" fill="none" className="processor-response" points={curve.x.map((hz, i) => `${frequencyPosition(hz) * 500},${100 - clamp(curve.y[i], -20, 20) * 5}`).join(' ')} />}
      {eq.bands.map((band, index) => band.on && <EqHandle key={index} band={band} index={index} disabled={control.disabled} recording={control.recording} start={paths=>control.start(paths??[`eq.${index}.freq`,...(eqHasGain(band.kind)?[`eq.${index}.gain_db`]:[])])} active={control.gesture.active}
        snapshot={control.snapshot} change={value => control.change(value)} update={(patch, base) => change(index, patch, base)} end={control.end} cancel={control.cancel} commit={value => control.commit(value)} current={eq} />)}
    </svg>
    <div className="eq-bands">{eq.bands.map((band, index) => {
      const number = index + 1, name = `${t('Band')} ${number}`, [min, max] = eqBounds(band, sampleRate), pass = band.kind === 'high_pass' || band.kind === 'low_pass';
      const knob = (field: 'freq' | 'gain_db' | 'q', label: string, low: number, high: number, step: number, disabled = false, unit = '') =>
        <Knob key={field} path={`eq.${index}.${field}`} label={`${name} ${label}`} value={band[field]} min={low} max={high} step={step} unit={unit} logarithmic={field === 'freq'} fallback={defaults.bands[index][field]}
          disabled={control.disabled || disabled} onStart={()=>control.start(`eq.${index}.${field}`)} isActive={control.gesture.active} onChange={value => control.change(change(index, { [field]: value }))}
          onEnd={control.end} onCancel={control.cancel} onCommit={value => control.commit(change(index, { [field]: value }, eq))} />;
      return <fieldset key={index} disabled={control.disabled}><legend>{name}</legend>
        <label><input type="checkbox" checked={band.on} onChange={event => control.commit(change(index, { on: event.target.checked }, eq))} />{t('On')}</label>
        <select disabled={control.recording} aria-label={`${name} ${t('Type')}`} value={band.kind} onChange={event => control.commit(change(index, { kind: event.target.value as EqKind }, eq))}>
          {eqBandKinds(index, eq.bands.length).map(kind => <option key={kind} value={kind}>{t(kind === 'bell' ? 'Bell' : kind === 'notch' ? 'Notch' : kind === 'high_pass' ? 'High Pass' : kind === 'low_pass' ? 'Low Pass' : kind === 'low_shelf' ? 'Low Shelf' : 'High Shelf')}</option>)}
        </select>
        {!clip && index > 0 && index < 5 && <select disabled={control.recording} aria-label={`${name} ${t('Range')}`} value={band.range ?? 'low'} onChange={event => control.commit(change(index, { range: event.target.value as EqRange }, eq))}>
          {Object.keys(EQ_ZONES).map((zone, i) => <option key={zone} value={zone}>{['L', 'ML', 'MH', 'H'][i]}</option>)}
        </select>}
        {knob('freq', t('Frequency'), min, max, 1, false, 'Hz')}
        {knob('gain_db', t('Gain'), -20, 20, .1, !eqHasGain(band.kind), 'dB')}
        {knob('q', 'Q', .3, 10.3, .1, !eqHasQ(band.kind))}
        {pass && !clip && <label>{t('Slope')}<select disabled={control.recording} aria-label={`${name} ${t('Slope')}`} value={band.slope} onChange={event => control.commit(change(index, { slope: Number(event.target.value) as EqBand['slope'] }, eq))}>
          {[6, 12, 18, 24].map(slope => <option key={slope} value={slope}>{slope} dB/oct</option>)}
        </select></label>}
        {pass && clip && <small>12 dB/oct</small>}
      </fieldset>;
    })}</div>
    <Knob path="eq.gain_db" label={t('Output Gain')} value={eq.gain_db} min={-20} max={20} fallback={0} unit="dB" disabled={control.disabled}
      onStart={()=>control.start('eq.gain_db')} isActive={control.gesture.active} onChange={gain_db => control.change({ ...control.snapshot(), gain_db })} onEnd={control.end} onCancel={control.cancel}
      onCommit={gain_db => control.commit({ ...eq, gain_db })} />
  </section>;
}
function EqHandle(props: {
  band: EqBand; index: number; disabled: boolean; recording: boolean; current: Equalizer; start: (paths?:string[]) => boolean; active: () => boolean;
  snapshot: () => Equalizer; update: (patch: Partial<EqBand>, base: Equalizer) => Equalizer; change: (value: Equalizer) => void;
  end: () => void; cancel: () => void; commit: (value: Equalizer) => void;
}) {
  const t = useFairlightText(), { band, index, disabled } = props, ref = useRef<SVGCircleElement>(null), now = useRef(props); now.current = props;
  const drag = useRef<{ x: number; y: number; scale: number; base: Equalizer } | null>(null);
  const burst = useRef<{ q: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const cancel = () => { drag.current = null; if (burst.current) clearTimeout(burst.current.timer); burst.current = null; now.current.cancel(); };
  useEffect(() => {
    const node = ref.current; if (!node) return;
    const wheel = (event: WheelEvent) => { const p = now.current; if (p.disabled || drag.current || !eqHasQ(p.band.kind) || !event.deltaY) return;
      event.preventDefault(); event.stopPropagation();
      if (burst.current && !p.active()) { clearTimeout(burst.current.timer); burst.current = null; }
      if (!burst.current && !p.start([`eq.${p.index}.q`])) return;
      node.focus({ preventScroll: true });
      const q = clamp((burst.current?.q ?? p.band.q) * (event.deltaY < 0 ? 1.1 : 1 / 1.1), .3, 10.3);
      if (burst.current) clearTimeout(burst.current.timer);
      p.change(p.update({ q }, p.snapshot()));
      burst.current = { q, timer: setTimeout(() => { burst.current = null; now.current.end(); }, 500) };
    };
    node.addEventListener('wheel', wheel, { passive: false });
    window.addEventListener('blur', cancel);
    return () => { node.removeEventListener('wheel', wheel); window.removeEventListener('blur', cancel); if (burst.current || drag.current) cancel(); };
  }, []);
  const x = frequencyPosition(band.freq) * 500, y = 100 - (eqHasGain(band.kind) ? band.gain_db : 0) * 5;
  return <g data-on={band.on}><text x={x} y={y - 12} textAnchor="middle" className="eq-handle-number">{index + 1}</text>
    <circle ref={ref} role="slider" aria-label={`${t('EQ band')} ${index + 1}`} aria-valuemin={20} aria-valuemax={20000} aria-valuenow={band.freq}
      aria-disabled={disabled || undefined} tabIndex={disabled ? -1 : 0} cx={x} cy={y} r="8" className="eq-handle"
      onPointerDown={event => {
        if (disabled || event.button !== 0) return;
        const wheel = burst.current, continuing = wheel !== null && props.active() && !props.recording;
        if (wheel) clearTimeout(wheel.timer); burst.current = null;
        if(wheel&&props.recording)props.end();
        if (!continuing && !props.start()) return; event.preventDefault(); event.stopPropagation();
        event.currentTarget.focus({ preventScroll: true });
        const box = event.currentTarget.ownerSVGElement!.getBoundingClientRect();
        // xMidYMid meet uses one scale for both axes. At max-height the SVG's outer width
        // includes horizontal letterboxing; translation cancels for these pointer deltas.
        const scale = box.width && box.height ? Math.min(box.width / 540, box.height / 245) : 1;
        drag.current = { x: event.clientX, y: event.clientY, scale,
          base: continuing ? props.update({ q: wheel.q }, props.snapshot()) : props.snapshot() };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }} onPointerMove={event => {
        const d = drag.current; if (!d || !props.active()) return; const base = d.base, original = base.bands[index];
        props.change(props.update({ freq: frequencyAt(frequencyPosition(original.freq) + (event.clientX - d.x) / (500 * d.scale)),
          ...(eqHasGain(original.kind) ? { gain_db: original.gain_db + (d.y - event.clientY) / (5 * d.scale) } : {}) }, base));
      }} onPointerUp={event => { if (drag.current) { drag.current = null; props.end(); } event.currentTarget.releasePointerCapture?.(event.pointerId); }}
      onPointerCancel={cancel} onLostPointerCapture={() => { if (drag.current) cancel(); }}
      onKeyDown={event => { if (event.key === 'Escape') cancel(); }} />
  </g>;
}
