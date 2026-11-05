import { useFairlightText } from './words';
import { useState, type ReactNode } from 'react';
import type { LoudnessStandard, MeterChannel, Meters, Timeline } from '../engine';
import type { FairlightText } from '../../messages/fairlight';
import { Meter as AudioMeter } from '../audio/Meter';

export const STANDARDS: [LoudnessStandard, string][] = [
  ['bs1770_1', 'BS.1770-1'], ['bs1770_4', 'BS.1770-4'], ['atsc_a85', 'ATSC A/85'], ['ebu_r128', 'EBU R128'],
  ['op_59', 'OP-59'], ['tr_b32', 'TR-B32'], ['agcom_219', 'AGCOM 219 (unofficial preset)'], ['netflix', 'Netflix'],
  ['youtube', 'YouTube (unofficial)'], ['disney_2_0', 'Disney 2.0 (unverified)'], ['disney_5_1', 'Disney 5.1 (unverified)'],
  ['apple_podcasts', 'Apple Podcasts'], ['spotify', 'Spotify'], ['aes_streaming', 'AES Streaming'], ['web_video', 'Web video (unofficial)'],
];
export const reading = (value: number | null | undefined, unit: string) => `${value == null ? '−∞' : value.toFixed(1)} ${unit}`;
export const standardLabel = (label: string, text: (key: FairlightText) => string) =>
  (['unofficial preset', 'unofficial', 'unverified', 'Web video'] as const).reduce((shown, key) => shown.replace(key, text(key)), label);
function Meter({ channel, label }: { channel: MeterChannel; label: string }) {
  // The engine sends dB and the position on the chosen meter scale, not linear amplitudes.
  return <div className="fairlight-meter" title={`${label}: ${reading(channel.bar, 'dBFS')}`}>
    <div className="fairlight-meter-well"><AudioMeter label={label} values={[channel]} /></div><small>{reading(channel.bar, 'dBFS')}</small>
  </div>;
}
export function Monitoring({ timeline, meters, onAction, onResetPeaks, onPatch, viewer }: {
  timeline: Timeline; meters: Meters | null; onAction: (action: 'start' | 'pause' | 'reset') => void;
  onResetPeaks: () => void; onPatch: (patch: Record<string, unknown>) => void; viewer?: ReactNode;
}) {
  const t = useFairlightText();
  const [rows, setRows] = useState(1); const [wide, setWide] = useState(true);
  const settings = timeline.audio?.loudness;
  const loudness = meters?.loudness;
  const relative = settings?.relative ?? true;
  const target = loudness?.target ?? settings?.target ?? -23;
  const value = (number: number | null | undefined) => reading(number == null ? null : relative ? number - target : number, relative ? 'LU' : 'LUFS');
  const change = (patch: Record<string, unknown>) => onPatch({ loudness: patch });
  const strips = meters?.strips ?? [];
  const bar = (number: number | null | undefined) => {
    const top = settings?.scale === 'ebu18' ? 18 : 9;
    return Math.max(0, Math.min(100, ((number ?? -Infinity) - target + top * 2) / (top * 3) * 100));
  };
  return <section className="fairlight-monitoring" aria-label={t('Monitoring')} data-rows={rows} data-wide={wide}>
    <div className="fairlight-meter-options"><button type="button" onClick={() => setRows(old => old === 1 ? 2 : 1)}>{t(rows === 1 ? 'Two rows' : 'One row')}</button>
      <button type="button" onClick={() => setWide(old => !old)}>{t(wide ? 'Narrow meters' : 'Wide meters')}</button></div>
    {(['track', 'bus'] as const).map(kind => <div key={kind} className="fairlight-strip-meters" aria-label={t(kind === 'track' ? 'Track meters' : 'Bus meters')}
      onDoubleClick={() => setRows(old => old === 1 ? 2 : 1)}>
      {strips.filter(strip => kind in strip.owner).map(strip => {
        const id = 'track' in strip.owner ? strip.owner.track : strip.owner.bus;
        const owner = kind === 'track' ? timeline.tracks.find(track => track.id === id) : timeline.buses?.find(bus => bus.id === id);
        return <button type="button" className="fairlight-strip-meter" key={id} onClick={onResetPeaks} title={t('Reset meter peaks')}>
          <strong>{owner?.name ?? id}</strong><div className="fairlight-meter-channels">{(kind === 'track' && timeline.audio?.prefader_metering ? strip.pre : strip.post)
            .map((channel, index) => <Meter key={index} channel={channel} label={`${owner?.name ?? id} ${index + 1}`} />)}</div></button>;
      })}
    </div>)}
    <section className="fairlight-control-room" aria-label={t('Control Room')}><strong>{t('Control Room')}</strong>
      {meters?.control_room && <Meter channel={meters.control_room} label={t('Control Room')} />}
      <span>{reading(loudness?.true_peak, 'dBTP')}</span>
      <label>{t('Monitor bus')}<select aria-label={t('Monitor bus')} value={timeline.audio?.monitor ?? timeline.buses?.[0]?.id ?? ''}
        onChange={event => onPatch({ monitor: event.target.value })}>
        {(timeline.buses ?? []).map(bus => <option key={bus.id} value={bus.id}>{bus.name}</option>)}</select></label><small>{t('Speakers: Auto')}</small>
    </section>
    <section className="fairlight-loudness" aria-label={t('Loudness')}>
      <label>{t('Standard')}<select aria-label={t('Loudness standard')} value={settings?.standard ?? 'ebu_r128'} onChange={event => change({ standard: event.target.value })}>
        {STANDARDS.map(([id, label]) => <option key={id} value={id}>{standardLabel(label, t)}</option>)}</select></label>
      <div className="fairlight-loudness-values">
        {([['M', loudness?.momentary], ['S', loudness?.short_term], ['Short Max', loudness?.short_term_max]] as const).map(([label, number]) =>
          <span key={label}>{label === 'Short Max' ? t('Short Max') : label}<output>{value(number)}</output></span>)}
        <span data-testid="loudness-integrated" data-verdict={loudness?.verdict ?? undefined}>I<output>{value(loudness?.integrated)}</output>
          <i className="fairlight-loudness-bar" style={{ width: `${bar(loudness?.integrated)}%` }} /></span>
        <span>LRA<output>{reading(loudness?.range, 'LU')}</output></span><span>TP<output>{reading(loudness?.true_peak, 'dBTP')}</output></span>
      </div>
      <label><input type="checkbox" checked={!relative} onChange={event => change({ relative: !event.target.checked })} />{t('Absolute scale')}</label>
      <select aria-label={t('Loudness scale')} value={settings?.scale ?? 'ebu9'} onChange={event => change({ scale: event.target.value })}><option value="ebu9">EBU +9</option><option value="ebu18">EBU +18</option></select>
      <label><input type="checkbox" checked={settings?.lock_to_transport ?? true} onChange={event => change({ lock_to_transport: event.target.checked })} />{t('Lock Metering to Transport')}</label>
      <button type="button" onClick={() => onAction(loudness?.running === false ? 'start' : 'pause')}>{t(loudness?.running === false ? loudness.seconds ? 'Resume' : 'Start' : 'Pause')}</button>
      <button type="button" onClick={() => onAction('reset')}>{t('Reset')}</button>
    </section>
    {viewer && <div className="fairlight-small-viewer">{viewer}</div>}
  </section>;
}
