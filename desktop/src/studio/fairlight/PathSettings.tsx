import { useState } from 'react';
import { useOverlay } from '../../app/overlay';
import { Knob } from '../audio/Knob';
import type { ProcessorProps } from '../audio/useProcessor';
import { processorChanges, processorWritable } from '../audio/processorModel';
import { mixerStrips } from '../audio/mixerModel';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { useFairlightText } from './words';

export function PathSettings({ context, stripKey, anchor, onClose }: { context: ProcessorProps; stripKey: string; anchor: { x: number; y: number }; onClose: () => void }) {
  useOverlay();
  const t = useFairlightText(), strip = mixerStrips(context.timeline).find(s => s.key === stripKey)!;
  const [problem, setProblem] = useState(''), [preview, setPreview] = useState<number | null>(null);
  const trim = strip.strip?.trim_db ?? 0;
  const gesture = useAudioGesture({ ...context, identity: `${context.timeline.id}/${stripKey}/path`, locked: strip.locked,
    onFailure: error => setProblem(error instanceof Error ? error.message : String(error)) });
  const changes = (path: string, value: unknown) => processorChanges(context.timeline.id, [strip], path, value);
  return <section className="path-settings" style={{ left: anchor.x, top: anchor.y }} aria-label={t('Path Settings')} onKeyDown={event => { if (event.key === 'Escape') { gesture.cancel(); setPreview(null); event.stopPropagation(); } }}>
    <header>{strip.name} · {t('Path Settings')}<button type="button" aria-label={t('Close processor')} onClick={onClose}>×</button></header>
    {problem && <p role="alert">{problem}</p>}
    <Knob path="trim_db" label={t('Trim')} value={gesture.active() ? preview ?? trim : trim} min={-20} max={20} fallback={0} unit="dB" disabled={!processorWritable(strip, 'trim_db')}
      onStart={() => gesture.start(changes('trim_db', trim))} isActive={gesture.active}
      onChange={value => { setPreview(value); gesture.preview(changes('trim_db', value)); }} onEnd={() => { gesture.end(); setPreview(null); }} onCancel={() => { gesture.cancel(); setPreview(null); }}
      onCommit={value => gesture.commit(changes('trim_db', value))} />
    <label><input type="checkbox" checked={strip.strip?.invert ?? false} disabled={!processorWritable(strip, 'invert')} onChange={event => gesture.commit(changes('invert', event.target.checked))} />{t('Polarity')}</label>
    <label><input type="checkbox" checked={strip.strip?.inserts_on !== false} disabled={!processorWritable(strip, 'inserts_on')} onChange={event => gesture.commit(changes('inserts_on', event.target.checked))} />{t('Insert')}</label>
  </section>;
}
