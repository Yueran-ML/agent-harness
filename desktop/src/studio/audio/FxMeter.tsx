import { useEffect, useRef, useState } from 'react';
import type { FxReading } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { targetKey } from './processorModel';
import { useFxMeters } from './useFxMeters';
import type { ProcessorProps } from './useProcessor';
import { clamp } from './taper';
export function FxMeter({ context, kind, insert, slot, voice = false, drawBall = true }: {
  context: ProcessorProps;
  kind: FxReading['kind'];
  insert?: string;
  slot?: number;
  voice?: boolean;
  drawBall?: boolean;
}) {
  const t = useFairlightText(), data = useFxMeters(context);
  const reading = data?.readings.find(r => targetKey(r.owner) === targetKey(context.target) && r.reading.kind === kind && (insert ? r.insert === insert : slot !== undefined ? r.slot === slot : r.insert === null))?.reading;
  const [history, setHistory] = useState<FxReading[]>([]);
  const sequence = useRef(0);
  const [generation, setGeneration] = useState<number | null>(null);
  useEffect(() => {
    setHistory(old => reading ? [...(generation === data?.generation && (data?.sequence ?? 0) > sequence.current ? old : []), reading].slice(-50) : []);
    setGeneration(data?.generation ?? null);
    sequence.current = data?.sequence ?? 0;
  }, [data?.generation, data?.sequence, context.session, targetKey(context.target), insert, slot]);
  const y = (db: number) => 100 - (clamp(db, -80, 0) + 80) / .8;
  const curve = (values: number[]) => values.map((db, i) => `${i * 300 / Math.max(1, values.length - 1)},${y(db)}`).join(' ');
  return <div className={`fx-meter fx-meter-${kind}`} aria-label={t('Live audio meters')}>
    {!reading && <p>{t(voice ? 'Spectrum unavailable in voice mode' : 'No live audio')}</p>}
    {reading && <>
      {kind === 'dynamics' && drawBall && <svg viewBox="0 0 120 120" aria-label={t('Live dynamics ball')}>
        <path d="M0 120L120 0" className="processor-grid" />
        <circle
          cx={(clamp(reading.input_db ?? -80, -60, 0) + 60) * 2}
          cy={-clamp(reading.output_db ?? -80, -60, 0) * 2}
          r="4"
          className="processor-ball"
        />
      </svg>}
      {kind === 'ducker' && <svg viewBox="0 0 300 100" aria-label={t('Ducker history')}>
        <polyline points={curve(history.map(r => r.input_db ?? -200))} className="fx-input" />
        <polyline points={curve(history.map(r => (r.input_db ?? -200) + (r.reduction_db ?? 0)))} className="fx-noise" />
      </svg>}
      {kind === 'noise_reduction' && reading.input_spectrum_db && <>
        <small>{t('Input spectrum')} / {t('Noise estimate')} · 0–{Math.round((data?.sample_rate ?? 0) / 2)} Hz · {t('dB power')}</small>
        <svg viewBox="0 0 300 100" aria-label={t('Noise spectrum')}>
          <polyline className="fx-input" points={curve(reading.input_spectrum_db)} />
          <polyline className="fx-noise" points={curve(reading.noise_spectrum_db ?? [])} />
        </svg>
      </>}
      {kind === 'stereo_width' && reading.scope && <svg viewBox="-1 -1 2 2" aria-label={t('Stereo scope')}>
        <path d="M-1 0H1M0-1V1" className="scope-grid" />
        <polyline className="fx-scope" points={Array.from({ length: reading.scope.length / 2 }, (_, i) => {
          const l = reading.scope![i * 2], r = reading.scope![i * 2 + 1];
          return `${clamp((l - r) / 2, -1, 1)},${clamp(-(l + r) / 2, -1, 1)}`;
        }).join(' ')} />
      </svg>}
      <div className="fx-meter-bars">
        {(['input_db', 'source_db', 'reduction_db', 'expander_db', 'compressor_db', 'limiter_db', 'output_db'] as const)
          .filter(field => reading[field] !== undefined)
          .map(field => <label key={field}>
            {t(({
              input_db: 'Input', source_db: 'Source', reduction_db: 'Gain reduction', expander_db: 'Expander',
              compressor_db: 'Compressor', limiter_db: 'Limiter', output_db: 'Output'
            } as const)[field])}
            <meter min={-80} max={0} value={clamp(reading[field]!, -80, 0)} />
            <output>{reading[field]! <= -200 ? '−∞' : reading[field]!.toFixed(1)} dB</output>
          </label>)}
      </div>
    </>}
  </div>;
}
