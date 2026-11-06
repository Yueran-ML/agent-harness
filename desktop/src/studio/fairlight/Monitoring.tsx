import { useFairlightText } from './words';
import { Fragment, useState, type ReactNode } from 'react';
import type { LoudnessStandard, MeterChannel, Meters, Timeline } from '../engine';
import type { FairlightText } from '../../messages/fairlight';
import { Meter as AudioMeter } from '../audio/Meter';
import { defaultMixerLayout, mixerOrder, MIXER_DIVIDER, type MixerLayout } from '../audio/mixerLayout';
import { defaultMonitoringView, type MonitoringView } from './viewState';
import { Separator } from './Separator';
import { CoveringMenu } from '../color/CoveringMenu';

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
export function Monitoring({ timeline, meters, onAction, onResetPeaks, onPatch, viewer, layout, view, onView }: {
  timeline: Timeline; meters: Meters | null; onAction: (action: 'start' | 'pause' | 'reset') => void;
  onResetPeaks: () => void; onPatch: (patch: Record<string, unknown>) => void; viewer?: ReactNode;
  layout?: MixerLayout;
  view?: MonitoringView; onView?: (view: MonitoringView) => void;
}) {
  const t = useFairlightText();
  const [local,setLocal]=useState(defaultMonitoringView), shown=view??local;
  const update=(patch:Partial<MonitoringView>)=>{const next={...shown,...patch};setLocal(next);onView?.(next);};
  const {rows,wide}=shown;
  const setRows=()=>update({rows:rows===1?2:1}),setWide=()=>update({wide:!wide});
  const [menu,setMenu]=useState<{x:number;y:number}|null>(null);
  const settings = timeline.audio?.loudness;
  const loudness = meters?.loudness;
  const relative = settings?.relative ?? true;
  const target = loudness?.target ?? settings?.target ?? -23;
  const value = (number: number | null | undefined) => reading(number == null ? null : relative ? number - target : number, relative ? 'LU' : 'LUFS');
  const change = (patch: Record<string, unknown>) => onPatch({ loudness: patch });
  const order = mixerOrder(timeline, layout ?? defaultMixerLayout());
  const strips = [...meters?.strips ?? []].sort((a, b) => order.indexOf('track' in a.owner ? `track:${a.owner.track}` : `bus:${a.owner.bus}`) - order.indexOf('track' in b.owner ? `track:${b.owner.track}` : `bus:${b.owner.bus}`));
  const bar = (number: number | null | undefined) => {
    const top = settings?.scale === 'ebu18' ? 18 : 9;
    return Math.max(0, Math.min(100, ((number ?? -Infinity) - target + top * 2) / (top * 3) * 100));
  };
  const top=settings?.scale==='ebu18'?18:9;
  const ticks=[-2*top,-top,0,top];
  const scaleLabel=(n:number)=>{const scaled=relative?n:n+target;return scaled<0?`−${Math.abs(scaled)}`:scaled>0?`+${scaled}`:'0';};
  return <section className="fairlight-monitoring" aria-label={t('Monitoring')} data-rows={rows} data-wide={wide} onContextMenu={event=>{
    if(event.target instanceof Element&&event.target.closest('input,select,.monitor'))return;
    event.preventDefault();setMenu({x:event.clientX,y:event.clientY});
  }}>
    <div className="fairlight-meter-options"><button type="button" onClick={setRows}>{t(rows === 1 ? 'Two rows' : 'One row')}</button>
      <button type="button" onClick={setWide}>{t(wide ? 'Narrow meters' : 'Wide meters')}</button></div>
    {(['track', 'bus'] as const).map(kind => <Fragment key={kind}><div className="fairlight-strip-meters" aria-label={t(kind === 'track' ? 'Track meters' : 'Bus meters')} style={{width:kind==='track'?shown.tracksWidth:shown.busesWidth,flexShrink:0}}
      onDoubleClick={setRows}>
      {strips.filter(strip => { const key = 'track' in strip.owner ? `track:${strip.owner.track}` : `bus:${strip.owner.bus}`; return layout?.single ? kind === 'track' : (order.indexOf(key) < order.indexOf(MIXER_DIVIDER)) === (kind === 'track'); }).map(strip => {
        const id = 'track' in strip.owner ? strip.owner.track : strip.owner.bus;
        const owner = 'track' in strip.owner ? timeline.tracks.find(track => track.id === id) : timeline.buses?.find(bus => bus.id === id);
        return <button type="button" className="fairlight-strip-meter" key={id} onClick={onResetPeaks} title={t('Reset meter peaks')}>
          <strong title={owner?.name??id} style={{borderBottom:`3px solid ${owner?.color??'#5aa0e0ff'}`}}>{id}</strong><div className="fairlight-meter-channels">{('track' in strip.owner && timeline.audio?.prefader_metering ? strip.pre : strip.post)
            .map((channel, index) => <Meter key={index} channel={channel} label={`${owner?.name ?? id} ${index + 1}`} />)}</div></button>;
      })}
    </div><Separator vertical label={t(kind==='track'?'Track meters width':'Bus meters width')} value={kind==='track'?shown.tracksWidth:shown.busesWidth} min={60} max={1000} onChange={value=>update(kind==='track'?{tracksWidth:value}:{busesWidth:value})}/></Fragment>)}
    <section className="fairlight-control-room" aria-label={t('Control Room')}><strong>{t('Control Room')}</strong>
      {meters?.control_room && <Meter channel={meters.control_room} label={t('Control Room')} />}
      <span>{reading(loudness?.true_peak, 'dBTP')}</span>
      <label>{t('Monitor bus')}<select aria-label={t('Monitor bus')} value={timeline.audio?.monitor === null ? '' : timeline.audio?.monitor ?? (timeline.buses?.some(bus => bus.id === 'B1') ? 'B1' : '')}
        onChange={event => onPatch({ monitor: event.target.value || null })}>
        <option value="">{t('None')}</option>{[true, false].map(main => <optgroup key={String(main)} label={t(main ? 'Main outputs' : 'Other buses')}>
          {(timeline.buses ?? []).filter(bus => !(bus.strip?.outputs?.length) === main).map(bus => <option key={bus.id} value={bus.id}>{bus.name}</option>)}
        </optgroup>)}</select></label><small>{t('Speakers: Auto')}</small>
    </section>
    <section className="fairlight-loudness" aria-label={t('Loudness')} style={{width:shown.loudnessWidth,flexShrink:0}}>
      <label>{t('Standard')}<select aria-label={t('Loudness standard')} value={settings?.standard ?? 'ebu_r128'} onChange={event => change({ standard: event.target.value })}>
        {STANDARDS.map(([id, label]) => <option key={id} value={id}>{standardLabel(label, t)}</option>)}</select></label>
      <div className="fairlight-loudness-values">
        {([['M', loudness?.momentary], ['S', loudness?.short_term], ['Short Max', loudness?.short_term_max]] as const).map(([label, number]) =>
          <span key={label}>{label === 'Short Max' ? t('Short Max') : label}<output>{value(number)}</output></span>)}
        <span data-testid="loudness-integrated" data-verdict={loudness?.verdict ?? undefined}>I<output>{value(loudness?.integrated)}</output>
          <i className="fairlight-loudness-bar" style={{ width: `${bar(loudness?.integrated)}%` }} /></span>
        <span>LRA<output>{reading(loudness?.range, 'LU')}</output></span><span>TP<output>{reading(loudness?.true_peak, 'dBTP')}</output></span>
      </div>
      <div className="fairlight-loudness-sum" role="meter" aria-label={t('Momentary sum')} aria-valuemin={-2*top+(relative?0:target)} aria-valuemax={top+(relative?0:target)} aria-valuenow={loudness?.momentary==null?undefined:Math.max(-2*top,Math.min(top,loudness.momentary-target))+(relative?0:target)} aria-valuetext={value(loudness?.momentary)}>
        <span>M <output aria-label={t('Momentary maximum')}>{value(loudness?.momentary_max)}</output></span><i style={{width:`${bar(loudness?.momentary)}%`}}/>
      </div>
      <div aria-label={t('Loudness ruler')}><div className="fairlight-loudness-ruler">{ticks.map(n=><span key={n}>{scaleLabel(n)}</span>)}</div><small>{relative?'LU':'LUFS'}</small></div>
      <label><input type="checkbox" checked={!relative} onChange={event => change({ relative: !event.target.checked })} />{t('Absolute scale')}</label>
      <select aria-label={t('Loudness scale')} value={settings?.scale ?? 'ebu9'} onChange={event => change({ scale: event.target.value })}><option value="ebu9">EBU +9</option><option value="ebu18">EBU +18</option></select>
      <label><input type="checkbox" checked={settings?.lock_to_transport ?? true} onChange={event => change({ lock_to_transport: event.target.checked })} />{t('Lock Metering to Transport')}</label>
      <button type="button" onClick={() => onAction(loudness?.running === false ? 'start' : 'pause')}>{t(loudness?.running === false ? loudness.seconds ? 'Resume' : 'Start' : 'Pause')}</button>
      <button type="button" onClick={() => onAction('reset')}>{t('Reset')}</button>
    </section>
    <Separator vertical label={t('Loudness width')} value={shown.loudnessWidth} min={240} max={600} onChange={loudnessWidth=>update({loudnessWidth})}/>
    {viewer && <><div className="fairlight-small-viewer" style={{width:shown.viewerWidth}}>{viewer}</div><Separator vertical label={t('Viewer width')} value={shown.viewerWidth} min={190} max={900} onChange={viewerWidth=>update({viewerWidth})}/></>}
    {menu&&<CoveringMenu onClose={()=>setMenu(null)} menu={{...menu,label:t('Monitoring'),items:[
      {id:'rows',label:t('Two rows'),checked:rows===2,run:setRows}, {id:'wide',label:t('Wide meters'),checked:wide,run:setWide},
    ]}}/>}
  </section>;
}
