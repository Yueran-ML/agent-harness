import { callEngine } from '../engine';
import { defaultDynamics, defaultEq, targetKey, type ProcessorModule } from './processorModel';
import { mixerStrips } from './mixerModel';
import { useAudioQuery } from './useAudioQuery';
import type { ProcessorProps } from './useProcessor';

export const EQ_FREQUENCIES = Array.from({ length: 129 }, (_, i) => 20 * Math.pow(1000, i / 128));
export const DYNAMICS_INPUTS = Array.from({ length: 121 }, (_, i) => -60 + i / 2);
export interface ProcessorCurve { x: number[]; y: number[]; revision: number }
/** Model static settings, never the instantaneous automation or attack/release state. */
export function useProcessorCurve(props: ProcessorProps, module: ProcessorModule) {
  const { bridge, timeline, target, session, revision } = props;
  const strip = mixerStrips(timeline).find(row => row.key === targetKey(target));
  const clip = 'clip' in target ? timeline.tracks.flatMap(row => row.kind === 'audio' ? row.clips : []).find(row => row.id === target.clip) : undefined;
  const settings = module === 'eq' ? clip?.eq ?? strip?.strip?.eq ?? defaultEq('clip' in target)
    : strip?.strip?.dynamics ?? defaultDynamics();
  const { data, problem } = useAudioQuery<ProcessorCurve>(bridge,
    { kind: module, session, timeline: timeline.id, target, sampleRate: timeline.sample_rate, settings }, revision, true,
    async () => {
      if (module === 'eq') {
        const result = await callEngine(bridge, 'audio.eq_response', {
          timeline: timeline.id, target, frequencies: EQ_FREQUENCIES.filter(f => f <= timeline.sample_rate * .45),
        }, { expectedSession: session });
        return { x: result.frequencies, y: result.gain_db, revision: result.revision };
      }
      const result = await callEngine(bridge, 'audio.dynamics_curve', {
        timeline: timeline.id, target, inputs_db: DYNAMICS_INPUTS,
      }, { expectedSession: session });
      return { x: result.inputs_db, y: result.outputs_db, revision: result.revision };
    });
  return { curve: data ?? null, problem };
}
