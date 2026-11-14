import type { MeterChannel } from '../engine';
import { clamp } from './taper';
export function Meter({ values = [], channels = 1, label, disabled = false, shadow = false }: {
  values?: readonly MeterChannel[]; channels?: number; label: string; disabled?: boolean; shadow?: boolean;
}) {
  return <div className="audio-meter" data-disabled={disabled || undefined} data-shadow={shadow || undefined}>
    {Array.from({ length: channels }, (_, i) => {
      const value = disabled ? undefined : values[i];
      return <div key={i} className="audio-meter-well" role="meter" aria-label={`${label} ${i + 1}`} aria-valuemin={-60} aria-valuemax={0}
        aria-valuenow={value?.bar ?? -60} aria-valuetext={value?.bar == null ? '−∞ dBFS' : `${value.bar.toFixed(1)} dBFS`}>
        <i className="audio-meter-bar" style={{ height: `${clamp(value?.bar_position ?? 0, 0, 1) * 100}%` }} />
        {value?.peak_position != null && <b className="audio-meter-peak" style={{ bottom: `${clamp(value.peak_position, 0, 1) * 100}%` }} />}
      </div>;
    })}
  </div>;
}
