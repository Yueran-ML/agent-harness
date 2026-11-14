import { callEngine, type Effect } from '../engine';
import type { ProcessorProps } from './useProcessor';
import { frequencyPosition } from './processorModel';
import { effectOwner } from './effectModel';
import { useAudioQuery } from './useAudioQuery';
import { useFairlightText } from '../fairlight/words';
export function EffectDiagram({ context, slot, effect }: { context: ProcessorProps; slot: number; effect: Effect }) {
  const t = useFairlightText(), insert = effectOwner(context.timeline, context.target).inserts[slot];
  const { data: curve } = useAudioQuery(context.bridge,
    { kind: 'audio.soft_clip_curve', session: context.session, timeline: context.timeline.id, target: context.target, slot, insert },
    context.revision, effect.kind === 'soft_clipper', () => callEngine(context.bridge, 'audio.soft_clip_curve', {
      timeline: context.timeline.id, target: context.target, slot, inputs: Array.from({ length: 129 }, (_, i) => i / 64 - 1),
    }, { expectedSession: context.session }));
  if (effect.kind === 'soft_clipper') return <svg viewBox="-110 -110 220 220" className="effect-diagram" aria-label={t('Soft Clipper curve')}><path d="M-100 0H100M0-100V100" className="processor-grid" />{curve && <polyline className="fx-input" points={curve.inputs.map((x,i)=>`${x*100},${-curve.outputs[i]*100}`).join(' ')} />}</svg>;
  if (effect.kind !== 'de_hummer' && effect.kind !== 'de_esser') return null;
  const frequencies = effect.kind === 'de_hummer' ? Array.from({length:4}, (_,i)=>effect.frequency_hz*(i+1)) : [effect.frequency_hz];
  return <svg viewBox="0 0 300 70" className="effect-diagram" aria-label={t('Target frequencies')}>{frequencies.filter(hz=>hz<context.timeline.sample_rate*.45).map(hz=><g key={hz}><line x1={frequencyPosition(hz)*300} x2={frequencyPosition(hz)*300} y1="5" y2="45" className="fx-input" /><text x={frequencyPosition(hz)*300} y="63" fontSize="8">{Math.round(hz)}</text></g>)}</svg>;
}
