import { useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { AudioStream, ChannelMap, Clip, Media, Timeline } from '../engine';
import { automaticChannels, channelChoice, channelKey, channelSources as sources } from '../media/audioLayout';
import { useFairlightText } from '../fairlight/words';
import { clipLocations } from '../timeline/selection';
import { common, setClips } from './common';
import { FieldsLocked } from './fields';
import type { Commit } from './useCommit';
import { FileWaveforms } from './FileWaveforms';

const streamOf = (clip: Clip, media: readonly Media[]) => clip.source.kind === 'media' ? media.find(item => item.id === (clip.source as { media: string }).media) : undefined;
const countOf = (timeline: Timeline, id: string) => {
  const track = clipLocations(timeline).get(id)?.track;
  return track?.kind === 'audio' ? track.format === 'mono' ? 1 : track.format === '5.1' ? 6 : track.format === 'adaptive' ? track.channels ?? 1 : 2 : 0;
};
const autoOf = (timeline: Timeline, clip: Clip) => {
  const track = clipLocations(timeline).get(clip.id)?.track;
  return track ? automaticChannels(track) : [];
};
const empty = (): ChannelMap => ({ source: null, enabled: true, mute: false, level_db: 0 });
const mapsOf = (timeline: Timeline, clip: Clip) => (clip.channels ?? autoOf(timeline, clip)).map(channel => ({ ...empty(), ...channel }));

/** Timeline output channels. Custom mapping is explicit; null remains the engine's automatic mix. */
export function AudioConfig({ clips, timeline, media, commit, bridge, session }: {
  clips: readonly Clip[]; timeline: Timeline; media: readonly Media[]; commit: Commit;
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number;
}) {
  const t = useFairlightText(), locked = useContext(FieldsLocked), last = clips.at(-1)!;
  const medium = streamOf(last, media), facts = medium?.facts.audio, count = countOf(timeline, last.id);
  const ids = clips.map(clip => clip.id);
  const [custom, setCustom] = useState(clips.some(clip => clip.channels != null));
  const [draft, setDraft] = useState<Record<string, ChannelMap[]>>(() => Object.fromEntries(clips.map(clip => [clip.id,
    mapsOf(timeline, clip)])));
  const [pending, setPending] = useState(false);
  const [selectedChannels, setSelectedChannels] = useState<number[]>([]);
  useEffect(() => setSelectedChannels(old => old.filter(index => index < count)), [count]);
  const modelKey = JSON.stringify(clips.map(clip => [clip.id, autoOf(timeline, clip), clip.channels]));
  // The model the draft started from; a draft that differs from it holds unapplied changes.
  const baseline = useRef(JSON.stringify(draft));
  const [discarded, setDiscarded] = useState(false);
  useEffect(() => {
    const next = Object.fromEntries(clips.map(clip => [clip.id, mapsOf(timeline, clip)]));
    const was = JSON.stringify(draft), now = JSON.stringify(next);
    // Changed elsewhere (another page, undo) while this panel held unapplied edits: say so.
    setDiscarded(was !== baseline.current && was !== now);
    baseline.current = now;
    setCustom(clips.some(clip => clip.channels != null));
    setDraft(next);
  }, [modelKey]);
  const compatible = clips.every(clip => countOf(timeline, clip.id) === count);
  const streams = facts?.streams ?? [];
  const validSource = (source: ChannelMap['source'], clip: Clip) => sources(source).every(selected => streamOf(clip, media)?.facts.audio?.streams?.some(stream => stream.index === selected.stream && selected.channel < stream.channels));
  const valid = compatible && clips.every(clip => draft[clip.id]?.length === countOf(timeline, clip.id)
    && draft[clip.id].every(channel => Number.isFinite(channel.level_db) && channel.level_db! >= -100 && channel.level_db! <= 24 && validSource(channel.source, clip)));
  const disabled = locked || pending || !compatible;
  const update = (index: number, patch: Partial<ChannelMap> | ((clip: Clip) => Partial<ChannelMap>)) => (setDiscarded(false), setDraft(old => Object.fromEntries(clips.map(clip => [clip.id,
    old[clip.id].map((channel, i) => i === index ? { ...channel, ...(typeof patch === 'function' ? patch(clip) : patch) } : channel)]))));
  const apply = async (automatic = false) => {
    if (locked || pending || (!automatic && !valid)) return;
    setDiscarded(false);
    setPending(true);
    const sent = await commit(setClips(ids, (clip, now) => automatic ? { channels: null }
      : draft[clip.id]?.length === countOf(now, clip.id) && draft[clip.id].every(channel => validSource(channel.source, clip)) ? { channels: draft[clip.id] } : null));
    setPending(false);
    if (sent && automatic) { setCustom(false); setDraft(Object.fromEntries(clips.map(clip => [clip.id, autoOf(timeline, clip)]))); }
  };
  const streamName = (stream: AudioStream) => `${t('Stream')} ${stream.index}`;
  const track = clipLocations(timeline).get(last.id)?.track;
  const preview = useMemo(() => custom ? { ...last, channels: draft[last.id] } : last, [custom, last, draft]);
  const selectedRows = clips.flatMap(clip => selectedChannels.flatMap(index => draft[clip.id]?.[index] ?? []));
  const selectedLevel = common(selectedRows, channel => channel.level_db ?? 0);
  const setSelectedLevel = (value: number) => { setCustom(true); setDraft(old => Object.fromEntries(clips.map(clip => [clip.id,
    old[clip.id].map((channel, i) => selectedChannels.includes(i) ? { ...channel, level_db: value } : channel)]))); };
  return <section className="audio-config" aria-label={t('Audio Configuration')}>
    <h3>{t('Audio Configuration')}</h3>
    <p>{clips.length > 1 ? t('Multiple Clips') : medium?.name ?? last.id}</p>
    {facts && <p className="caption">{facts.codec} · {facts.rate} Hz · {facts.channels} {t('Channels')} · {(medium!.facts.duration.ticks / medium!.facts.duration.scale).toFixed(2)} s</p>}
    <p>{t('Format')}: {count === 1 ? t('Mono') : count === 2 ? t('Stereo') : count === 6 ? '5.1' : t('Adaptive')} ({count})</p>
    {track && <FileWaveforms bridge={bridge} session={session} clip={preview} track={track} media={medium} timeline={timeline}
      unapplied={pending || JSON.stringify(preview.channels) !== JSON.stringify(last.channels)}
      selected={selectedChannels} onSelect={(index, extend) => setSelectedChannels(old => !extend ? [index]
        : old.includes(index) ? old.filter(item => item !== index) : [...old, index])} />}
    <fieldset disabled={disabled || selectedChannels.length === 0}>
      <legend>{t('Channel Level')}</legend>
      <p className="caption">{t('Ctrl or Shift-click channels to set their level together.')}</p>
      <label>{t('Level')}<input type="range" aria-label={t('Selected channel level')} min={-100} max={24} step={.1}
        value={selectedLevel !== null && Number.isFinite(selectedLevel) ? selectedLevel : 0}
        aria-valuetext={selectedLevel === null ? t('Mixed values') : Number.isFinite(selectedLevel) ? `${selectedLevel} dB` : '—'}
        onChange={event => setSelectedLevel(Number(event.target.value))} /></label>
      <label>{t('Level')}<input type="number" aria-label={t('Selected channel level')} min={-100} max={24} step={.1}
        value={selectedLevel !== null && Number.isFinite(selectedLevel) ? selectedLevel : ''} placeholder="—"
        onChange={event => setSelectedLevel(event.target.value === '' ? NaN : Number(event.target.value))} /> dB</label>
    </fieldset>
    {discarded && <p role="status">{t('The channels were changed elsewhere, so the changes you had not applied were reset.')}</p>}
    {!custom ? <><p>{t('Automatic mix')}</p><button type="button" disabled={disabled} onClick={() => setCustom(true)}>{t('Custom mapping')}</button></> : <>
      <p className="caption">{t('Choose a source for each output channel. Unassigned channels are silent.')}</p>
      {!compatible && <p role="status">{t('Select clips with the same channel count to configure them together.')}</p>}
      {!streams.length && <p role="status">{t('Audio stream information is unavailable.')}</p>}

      {Array.from({ length: count }, (_, index) => {
        const channels = clips.map(clip => draft[clip.id]?.[index] ?? empty()), row = draft[last.id]?.[index] ?? empty();
        const name = `${t('Channel')} ${index + 1}`;
        const level = common(channels, channel => channel.level_db ?? 0);
        const selected = common(channels, channel => channelKey(channel, streams)) ?? '';
        const automatic = autoOf(timeline, last)[index];
        const automaticKey = automatic ? channelKey(automatic, streams) : '';
        const known = selected.startsWith('remix:') || selected === 'none' || streams.some(stream => selected === `mix:${stream.index}` || Array.from({ length: stream.channels }, (_, c) => `${stream.index}:${c}`).includes(selected));
        return <fieldset key={index} disabled={disabled}>
          <legend>{name}</legend>
          <label>{t('Source')}<select aria-label={`${name} ${t('Source')}`} value={selected} onChange={event => {
            const value = event.target.value;
            if (value) update(index, value === automaticKey
              ? clip => ({ source: null, remix: autoOf(timeline, clip)[index]?.remix })
              : channelChoice(value, streams));
          }}>
            <option value="">—</option><option value="none">{t('None')}</option>
            {automatic && <option value={automaticKey}>{t('Automatic mix')}</option>}
            {row.remix && selected && selected !== automaticKey && <option value={selected}>{t('Automatic mix')} {row.remix.channels} · {t('Channel')} {row.remix.channel + 1}</option>}
            {!known && selected && <option value={selected}>{t('Custom mapping')}</option>}
            {streams.flatMap(stream => [
              ...Array.from({ length: stream.channels }, (_, channel) => <option key={`${stream.index}:${channel}`} value={`${stream.index}:${channel}`}>{streamName(stream)} · {t('Channel')} {channel + 1}</option>),
              ...(stream.channels > 1 ? [<option key={`mix:${stream.index}`} value={`mix:${stream.index}`}>{streamName(stream)} · {t('Mix to mono')}</option>] : []),
            ])}
          </select></label>
          <label><input type="checkbox" aria-label={`${name} ${t('Active')}`} checked={row.enabled !== false} disabled readOnly />{t('Active')}</label>
          <label><input type="checkbox" aria-label={`${name} ${t('Mute')}`} checked={row.mute === true} ref={node => { if (node) node.indeterminate = common(channels, channel => channel.mute ?? false) === null; }}
            onChange={event => {
              if (event.nativeEvent instanceof MouseEvent && event.nativeEvent.altKey) setDraft(old => Object.fromEntries(clips.map(clip => [clip.id, old[clip.id].map((channel, i) => ({ ...channel, mute: i !== index }))])));
              else update(index, { mute: event.target.checked });
            }} />{t('Mute')}</label>
          <label>{t('Level')}<input type="number" aria-label={`${name} ${t('Level')}`} min={-100} max={24} step={.1}
            value={level !== null && Number.isFinite(level) ? level : ''} placeholder="—"
            onChange={event => update(index, { level_db: event.target.value === '' ? NaN : Number(event.target.value) })} /> dB</label>
        </fieldset>;
      })}
      <button type="button" disabled={disabled || !valid} onClick={() => void apply()}>{t('Apply channel mapping')}</button>
      <button type="button" disabled={locked || pending} onClick={() => void apply(true)}>{t('Restore automatic mix')}</button>
    </>}
  </section>;
}
