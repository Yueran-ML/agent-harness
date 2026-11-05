import { useFairlightText } from './words';
import { useCallback, useMemo, type ComponentProps } from 'react';
import { Timeline } from '../timeline/Timeline';
import type { TrackClipDrawing } from '../timeline/TrackRow';
import { Thumbnail, Thumbnails } from '../media/Thumbnails';
import { ChannelLanes } from './ChannelLanes';
import type { FairlightView } from './viewState';
export function FairlightTimeline({ view, update, ...props }: ComponentProps<typeof Timeline> & {
  view: FairlightView; update: (patch: Partial<FairlightView>) => void;
}) {
  const t = useFairlightText();
  const thumbnails = useMemo(() => new Thumbnails(props.bridge), [props.bridge, props.session]);
  const hidden = useMemo(() => [...view.hidden, ...(props.timeline?.tracks.filter(track => track.kind === 'subtitle' || (!view.video && track.kind === 'video')).map(track => track.id) ?? [])], [props.timeline, view.hidden, view.video]);
  const draw = useCallback((clip: TrackClipDrawing) => clip.track.kind === 'audio' ? <ChannelLanes {...clip} gain={view.waveform} />
    : clip.media && <Thumbnail media={clip.media} cache={thumbnails} />, [view.waveform, thumbnails]);
  return <section className="fairlight-timeline" data-view={view.automation ? 'automation' : 'clips'} onWheelCapture={event => {
    if (event.ctrlKey && event.altKey) { event.preventDefault(); event.stopPropagation(); update({ waveform: Math.max(1, Math.min(16, view.waveform * (event.deltaY < 0 ? 1.25 : .8))) }); }
  }}>
    <div className="fairlight-timeline-options"><label><input type="checkbox" checked={view.video} onChange={event => update({ video: event.target.checked })} />{t('Video tracks')}</label>
      <label>{t('Waveform height')}<input aria-label={t('Waveform height')} type="range" min={1} max={16} step={.25} value={view.waveform} onChange={event => update({ waveform: Number(event.target.value) })} /></label>
      <button type="button" onClick={() => update({ waveform: 1 })}>{t('Reset waveform height')}</button>
      <button type="button" aria-pressed={view.automation} onClick={() => update({ automation: !view.automation })}>{t('Automation view (F4)')}</button>
    </div>
    {view.automation && <p className="caption" role="status">{t('Automation lane editing is not available yet.')}</p>}
    <Timeline {...props} hiddenTracks={hidden} drawClip={draw} trackHeight={128} audioOnlyDrop={!view.video} label={t('Fairlight timeline')} />
  </section>;
}
