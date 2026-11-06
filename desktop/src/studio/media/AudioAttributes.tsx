import { useEffect, useRef, useState } from 'react';
import { Sheet } from '../../app/Sheet';
import type { Media, MediaAudioTrack, Op, TrackFormat } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { audioSources, formatChannels, mapLayout, sourceChoice, sourceKey, sourceLayout, sourceTrack } from './audioLayout';

export function AudioFormats({ disabled = false }: { disabled?: boolean }) {
  const t = useFairlightText();
  return <><option value="mono" disabled={disabled}>{t('Mono')}</option><option value="stereo" disabled={disabled}>{t('Stereo')}</option>
    <option value="5.1" disabled={disabled}>5.1</option><option value="7.1" disabled>7.1</option><option value="adaptive" disabled={disabled}>{t('Adaptive')}</option></>;
}

/** Clip Attributes' Audio page. Saving is one transaction; the originals stay untouched. */
export function AudioAttributes({ media, onApply, onClose }: {
  media: readonly Media[]; onApply: (ops: Op[]) => Promise<boolean>; onClose: () => void;
}) {
  const t = useFairlightText(), last = media.at(-1)!;
  const [base] = useState(() => JSON.stringify(media.map(item => [item.id, item.audio_config, item.facts.audio])));
  const stale = base !== JSON.stringify(media.map(item => [item.id, item.audio_config, item.facts.audio]));
  const [rows, setRows] = useState(() => sourceLayout(last));
  const [format, setFormat] = useState<TrackFormat>('mono');
  const [count, setCount] = useState(1);
  const [adaptive, setAdaptive] = useState(1);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const sending = useRef(false);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const streams = last.facts.audio?.streams ?? [];
  const available = media.every(item => audioSources(item).length > 0);
  const mapped = media.map(item => mapLayout(rows, last, item));
  const compatible = mapped.every(config => config !== null);
  const valid = rows.every(row => row.name.length <= 100 && row.channels.length >= 1 && row.channels.length <= 24
    && row.channels.every(channel => Number.isFinite(channel.level_db ?? 0) && (channel.level_db ?? 0) >= -100 && (channel.level_db ?? 0) <= 24));
  const update = (index: number, patch: Partial<MediaAudioTrack>) => setRows(old => old.map((row, i) => i === index ? { ...row, ...patch } : row));
  const save = async (reset = false) => {
    if (sending.current || stale || !reset && (!available || !compatible || !valid)) return;
    sending.current = true;
    setPending(true);
    setFailed(false);
    try {
      const applied = await onApply(media.map((item, i) => ({ op: 'set_media_audio', id: item.id, config: reset ? null : mapped[i]! })));
      if (applied && live.current) onClose();
      else if (live.current) setFailed(true);
    } finally { sending.current = false; if (live.current) setPending(false); }
  };
  return <Sheet title={t('Clip Attributes')} onClose={onClose} wide foot={<>
    <button type="button" className="btn" data-first onClick={onClose}>{t('Close')}</button>
    <button type="button" className="btn" disabled={pending || stale} onClick={() => void save(true)}>{t('Restore automatic mix')}</button>
    <button type="button" className="btn btn-main" disabled={pending || stale || !available || !compatible || !valid} onClick={() => void save()}>{t('Apply')}</button>
  </>}>
    <section className="audio-config" aria-label={t('Audio')}>
      <h3>{t('Audio')}</h3><p>{media.length > 1 ? t('Multiple Clips') : last.name}</p>
      <p className="caption">{t('These settings affect future edits. Existing timeline clips keep their settings.')}</p>
      {stale && <p role="status">{t('The media changed. Reopen Clip Attributes.')}</p>}
      {failed && <p role="alert">{t('The audio configuration was not saved. Your draft is kept; try applying again.')}</p>}
      {!available && <p role="status">{t('Audio stream information is unavailable.')}</p>}
      {!compatible && <p role="status">{t('Some clips lack the selected source channels. Select compatible clips or choose another source.')}</p>}
      <fieldset disabled={pending || stale || !available}>
        <legend>{t('Add tracks')}</legend>
        <label>{t('Format')}<select aria-label={t('New track format')} value={format} onChange={event => setFormat(event.target.value as TrackFormat)}><AudioFormats /></select></label>
        {format === 'adaptive' && <label>{t('Channels')}<input type="number" aria-label={t('New track channels')} min={1} max={24} value={Number.isNaN(adaptive) ? '' : adaptive} onChange={event => setAdaptive(event.target.valueAsNumber)} /></label>}
        <label>{t('Tracks')}<input type="number" aria-label={t('Tracks')} min={1} max={24} value={Number.isNaN(count) ? '' : count} onChange={event => setCount(event.target.valueAsNumber)} /></label>
        <button type="button" disabled={!Number.isInteger(count) || count < 1 || count > 24 || !Number.isInteger(adaptive) || adaptive < 1 || adaptive > 24}
          onClick={() => setRows(old => {
            const offset = old.reduce((sum, row) => sum + row.channels.length, 0);
            return [...old, ...Array.from({ length: count }, (_, i) => sourceTrack(last, format, offset + i * formatChannels(format, adaptive), adaptive))];
          })}>{t('Add tracks')}</button>
      </fieldset>
      {rows.map((row, index) => <fieldset key={index} disabled={pending || stale || !available}>
        <legend>{t('Track')} {index + 1}</legend>
        <label>{t('Track name')}<input aria-label={`${t('Track name')} ${index + 1}`} maxLength={100} value={row.name} onChange={event => update(index, { name: event.target.value })} /></label>
        <label>{t('Format')}<select aria-label={`${t('Format')} ${index + 1}`} value={row.format} onChange={event => {
          const next = event.target.value as TrackFormat, fresh = sourceTrack(last, next, rows.slice(0, index).reduce((sum, item) => sum + item.channels.length, 0), row.channels.length);
          update(index, { format: next, order: 'smpte', channels: fresh.channels.map((channel, at) => row.channels[at] ?? channel) });
        }}><AudioFormats /></select></label>
        {row.format === 'adaptive' && <label>{t('Channels')}<input aria-label={`${t('Channels')} ${index + 1}`} type="number" min={1} max={24} value={row.channels.length} onChange={event => {
          const n = event.target.valueAsNumber;
          if (Number.isInteger(n) && n >= 1 && n <= 24) update(index, { channels: Array.from({ length: n }, (_, at) => row.channels[at] ?? { source: null }) });
        }} /></label>}
        <button type="button" aria-label={`${t('Remove track')} ${index + 1}`} onClick={() => setRows(old => old.filter((_, i) => i !== index))}>{t('Remove track')}</button>
        {row.channels.map((channel, at) => {
          const label = `${t('Track')} ${index + 1} ${t('Channel')} ${at + 1}`;
          const selected = sourceKey(channel.source, streams);
          const patch = (change: Partial<typeof channel>) => update(index, { channels: row.channels.map((item, i) => i === at ? { ...item, ...change } : item) });
          const isolate = (field: 'enabled' | 'mute') => setRows(old => old.map((track, trackIndex) => ({ ...track,
            channels: track.channels.map((item, channelIndex) => ({ ...item,
              [field]: field === 'enabled' ? trackIndex === index && channelIndex === at : trackIndex !== index || channelIndex !== at,
            })),
          })));
          return <div key={at}>
            <label>{label}<select aria-label={`${label} ${t('Source')}`} value={selected} onChange={event => patch({ source: sourceChoice(event.target.value, streams) })}>
              <option value="none">{t('None')}</option>
              {selected.startsWith('custom:') && <option value={selected}>{t('Custom mapping')}</option>}
              {streams.flatMap(stream => [
                ...Array.from({ length: stream.channels }, (_, c) => <option key={`${stream.index}:${c}`} value={`${stream.index}:${c}`}>{t('Stream')} {stream.index} · {t('Channel')} {c + 1}</option>),
                ...(stream.channels > 1 ? [<option key={`mix:${stream.index}`} value={`mix:${stream.index}`}>{t('Stream')} {stream.index} · {t('Mix to mono')}</option>] : []),
              ])}
            </select></label>
            <label><input type="checkbox" aria-label={`${label} ${t('Active')}`} checked={channel.enabled !== false} onChange={event => {
              if (event.nativeEvent instanceof MouseEvent && event.nativeEvent.altKey) isolate('enabled');
              else patch({ enabled: event.target.checked });
            }} />{t('Active')}</label>
            <label><input type="checkbox" aria-label={`${label} ${t('Mute')}`} checked={channel.mute === true} onChange={event => {
              if (event.nativeEvent instanceof MouseEvent && event.nativeEvent.altKey) isolate('mute');
              else patch({ mute: event.target.checked });
            }} />{t('Mute')}</label>
            <label>{t('Level')}<input type="number" aria-label={`${label} ${t('Level')}`} min={-100} max={24} step={.1} value={Number.isFinite(channel.level_db ?? 0) ? channel.level_db ?? 0 : ''}
              onChange={event => patch({ level_db: event.target.valueAsNumber })} /> dB</label>
          </div>;
        })}
      </fieldset>)}
    </section>
  </Sheet>;
}
