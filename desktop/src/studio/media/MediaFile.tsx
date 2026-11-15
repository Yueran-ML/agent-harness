import { useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { ChannelMap, Media, Op } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { common } from '../inspector/common';
import { NumberField } from '../inspector/fields';
import { useFileWaveforms } from '../inspector/FileWaveforms';
import { mappedWaveform } from '../timeline/ChannelWaveform';
import { Waveform, type WaveClip } from '../timeline/Waveform';
import { audioSources, sourceLayout } from './audioLayout';
import { AuditionCursor, FileTransport, pointerFraction, useFileAudition } from '../inspector/FileAudition';

/** Pool channel edits retain each selected file's layout and unrelated channel settings. */
export function MediaFile({ media, bridge, session, onApply }: {
  media: readonly Media[]; bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number;
  onApply: (ops: Op[]) => Promise<boolean>;
}) {
  const t = useFairlightText(), last = media.at(-1)!;
  const { box, width, waves } = useFileWaveforms(bridge, session);
  const layouts = media.map(sourceLayout), shown = layouts.at(-1)!.flatMap(row => row.channels);
  const compatible = layouts.every(layout => layout.flatMap(row => row.channels).length === shown.length);
  const available = media.every(item => audioSources(item).length > 0);
  const canAudition = audioSources(last).length > 0;
  const duration = last.facts.duration.ticks / last.facts.duration.scale;
  const audition = useFileAudition(bridge, session, { kind: 'media', media: last.id }, 0, duration, !canAudition);
  const [selected, setSelected] = useState<number[]>([]);
  const [pending, setPending] = useState(false), [failed, setFailed] = useState(false);
  const sending = useRef(false), live = useRef(true);
  const latest = useRef({ media, onApply }); latest.current = { media, onApply };
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => setSelected(old => old.filter(index => index < shown.length)), [shown.length]);
  // Media time units describe the whole source, independent of project frame rate or marks.
  const rate = useMemo(() => ({ num: last.facts.duration.scale, den: 1 }), [last.facts.duration.scale]);
  const clip = useMemo<WaveClip>(() => ({ start: 0, length: last.facts.duration.ticks,
    source: { kind: 'media', media: last.id, in: { ticks: 0, scale: 1 } },
    speed: { num: 1, den: 1 }, reverse: false,
  }), [last.id, last.facts.duration.ticks]);
  const disabled = pending || !compatible || !available;
  const save = async (patch: (channel: ChannelMap, index: number, item: Media) => ChannelMap) => {
    if (disabled || sending.current || !live.current) return;
    const current = latest.current;
    const ops: Op[] = current.media.flatMap(item => {
      const config = sourceLayout(item);
      let index = 0;
      const edited = config.map(row => ({ ...row, channels: row.channels.map(channel => patch(channel, index++, item)) }));
      return JSON.stringify(config) === JSON.stringify(edited) ? [] : [{ op: 'set_media_audio', id: item.id, config: edited }];
    });
    if (!ops.length) return;
    sending.current = true; setPending(true); setFailed(false);
    try {
      const ok = await current.onApply(ops);
      if (live.current) setFailed(!ok);
    } catch { if (live.current) setFailed(true); }
    finally { sending.current = false; if (live.current) setPending(false); }
  };
  const changeFlag = (index: number, field: 'enabled' | 'mute', value: boolean, isolate: boolean) =>
    void save((channel, at) => isolate ? { ...channel, [field]: field === 'enabled' ? at === index : at !== index }
      : at === index ? { ...channel, [field]: value } : channel);
  const selectedRows = layouts.flatMap(layout => selected.flatMap(index => layout.flatMap(row => row.channels)[index] ?? []));
  const level = common(selectedRows, channel => channel.level_db ?? 0);
  const mono = layouts.at(-1)!.every(row => row.format === 'mono');
  const sources = audioSources(last);
  // RM20 p.3834: the composite shows every original file channel, independently of
  // the configured clip outputs. Reuse their cached envelopes without summing PCM.
  const waveform = (channel: ChannelMap, key?: number) => {
    const drawing = mappedWaveform(channel, last);
    return width > 0 && !drawing.silent ? <Waveform key={key} cache={waves} media={last.id} clip={clip} rate={rate}
      from={0} to={clip.length} scale={width / Math.max(1, clip.length)}
      channel={drawing.channel} stream={drawing.stream} mix={drawing.mix} remix={drawing.remix} gain={drawing.gain} /> : null;
  };
  const compositeLabel = media.length > 1 ? t('Multiple Clips') : t('Composite waveform');
  return <section className="media-file" aria-label={t('File')} onKeyDown={audition.keyDown}>
    <h3>{media.length > 1 ? t('Multiple Clips') : last.name}</h3>
    {!compatible && <p role="status">{t('Select clips with the same channel count to configure them together.')}</p>}
    {!available && <p role="status">{t('Audio stream information is unavailable.')}</p>}
    {failed && <p role="alert">{t('Could not update audio channels. Try again.')}</p>}
    <div ref={box} className="file-waveforms" role="group" aria-label={t('Channels')}>
      <button type="button" className="media-file-composite file-channel-waveform" aria-label={compositeLabel}
        disabled={!canAudition} onClick={event => {
          if (event.ctrlKey || event.metaKey || event.shiftKey) return;
          // Enter or Space has no pointer position: keep the current one.
          if (event.detail === 0) { audition.locate(audition.position, null); return; }
          const fraction = pointerFraction(event);
          if (fraction !== null) audition.locate(fraction * duration, null);
        }} onPointerMove={event => {
          if (event.buttons || event.ctrlKey || event.metaKey || event.shiftKey || event.pointerType === 'touch') return;
          const fraction = pointerFraction(event);
          if (fraction !== null) audition.skim(fraction * duration, null);
        }} onPointerLeave={audition.leave}>
        <span className="file-channel-name">{compositeLabel}</span>
        {sources.map((source, index) => waveform({ source }, index))}
        <AuditionCursor position={audition.position} start={0} end={duration} />
      </button>
      {shown.map((channel, index) => {
        const label = `${t('Channel')} ${index + 1}`;
        const all = layouts.map(layout => layout.flatMap(row => row.channels)[index]);
        const sourceIndex = sources.findIndex(source => !Array.isArray(channel.source)
          && source.stream === channel.source?.stream && source.channel === channel.source?.channel);
        const selectedSource = channel.remix ? 'custom' : channel.source == null ? 'none' : sourceIndex >= 0 ? String(sourceIndex) : 'custom';
        return <div key={index} className="media-file-channel">
          <button type="button" className="file-channel-waveform" aria-label={label} aria-pressed={selected.includes(index)}
            onClick={event => {
              const extend = event.ctrlKey || event.metaKey || event.shiftKey;
              setSelected(old => !extend ? [index] : old.includes(index) ? old.filter(item => item !== index) : [...old, index]);
              if (!extend && canAudition) {
                if (event.detail === 0) { audition.locate(audition.position, index); return; }
                const fraction = pointerFraction(event);
                if (fraction !== null) audition.locate(fraction * duration, index);
              }
            }} onPointerMove={event => {
              if (!canAudition || event.buttons || event.ctrlKey || event.metaKey || event.shiftKey || event.pointerType === 'touch') return;
              const fraction = pointerFraction(event);
              if (fraction !== null) audition.skim(fraction * duration, index);
            }} onPointerLeave={audition.leave}>
            <span className="file-channel-name">{label}</span>
            {waveform(channel)}
            <AuditionCursor position={audition.position} start={0} end={duration} />
          </button>
          <fieldset disabled={disabled} className="media-file-controls">
            <label className="check"><input type="checkbox" aria-label={`${label} ${t('Active')}`} checked={channel.enabled !== false}
              ref={node => { if (node) node.indeterminate = common(all, row => row?.enabled !== false) === null; }}
              onChange={event => changeFlag(index, 'enabled', event.target.checked, event.nativeEvent instanceof MouseEvent && event.nativeEvent.altKey)} />{t('Active')}</label>
            <label className="check"><input type="checkbox" aria-label={`${label} ${t('Mute')}`} checked={channel.mute === true}
              ref={node => { if (node) node.indeterminate = common(all, row => row?.mute === true) === null; }}
              onChange={event => changeFlag(index, 'mute', event.target.checked, event.nativeEvent instanceof MouseEvent && event.nativeEvent.altKey)} />{t('Mute')}</label>
            {mono && <label>{t('Source')}<select className="field" aria-label={`${label} ${t('Source')}`} value={selectedSource}
              onChange={event => {
                const value = event.target.value;
                void save((row, at, item) => at !== index ? row : { ...row, remix: undefined, source: value === 'none' ? null : audioSources(item)[Number(value)] });
              }}>
              <option value="none">{t('None')}</option>
              {selectedSource === 'custom' && <option value="custom" disabled>{t('Custom mapping')}</option>}
              {sources.map((source, at) => <option key={at} value={at} disabled={media.some(item => !audioSources(item)[at])}>
                {t('Stream')} {source.stream} · {t('Channel')} {source.channel + 1}
              </option>)}
            </select></label>}
          </fieldset>
        </div>;
      })}
    </div>
    <FileTransport audition={audition} disabled={!canAudition || duration <= 0} />
    <NumberField label={t('Channel Level')} name="media-channel-level" value={level} fallback={0} min={-100} max={24} digits={1}
      scrub={.1} slider={{ min: -100, max: 24, step: .1 }} unit="dB" disabled={disabled || selected.length === 0}
      onCommit={value => void save((row, index) => selected.includes(index) ? { ...row, level_db: value } : row)} />
    <p className="caption">{t('Ctrl or Shift-click channels to set their level together.')}</p>
  </section>;
}
