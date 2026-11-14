import { useState } from 'react';
import { Mixer, isolateMixerKeys, type MixerProps } from '../fairlight/Mixer';
import {
  sameChannels, stripChannels, useAudioReadings, useMeterReading, type MeterSource,
} from '../fairlight/audioReadings';
import { useFairlightText } from '../fairlight/words';
import { mixerStrips } from './mixerModel';
import { Meter } from './Meter';
import type { EngineBridge } from '../../bridge';

/** The monitored bus's meter, drawn again at each of its own readings and not with the panel. */
function MonitorMeter({ source, bus, label, channels }: {
  source: MeterSource; bus: string; label: string; channels: number;
}) {
  const values = useMeterReading(source, meters => stripChannels(meters, { bus }, false), sameChannels);
  return <Meter label={label} channels={channels} values={values} />;
}
/** Edit never sends automation.touch: the same strips display lane playback read-only. */
export function EditMixer({ bridge, ...props }: Omit<MixerProps, 'meters' | 'compact'> & { bridge: Pick<EngineBridge, 'call' | 'onEvent'> }) {
  const t = useFairlightText(), [metersOnly, setMetersOnly] = useState(false);
  const audio = useAudioReadings(bridge, props.session, props.timeline.id, true);
  // A null monitor means silence; neither the renderer nor DSP falls back to the first bus.
  const monitor = mixerStrips(props.timeline).find(s => s.kind === 'bus' && s.id === props.timeline.audio?.monitor);
  return <aside className="edit-mixer" aria-label={t('Mixer')} onKeyDown={isolateMixerKeys}>
    <div className="edit-mixer-mode"><button type="button" aria-pressed={metersOnly} onClick={() => setMetersOnly(value => !value)}>{t('Meters')}</button></div>
    {audio.problem && <p role="alert">{audio.problem}</p>}
    {metersOnly ? <div className="edit-meters" aria-label={t('Control Room')}>
      {monitor ? <><p>{monitor.name}</p><MonitorMeter source={audio.readings} bus={monitor.id}
        label={`${t('Control Room')} ${monitor.name}`} channels={monitor.channels} /></> : <p>{t('No monitored bus')}</p>}
    </div> : <Mixer {...props} bridge={bridge} meters={audio.readings} compact />}
  </aside>;
}
