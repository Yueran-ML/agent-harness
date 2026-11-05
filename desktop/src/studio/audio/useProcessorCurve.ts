import { useEffect, useRef, useState } from 'react';
import { callEngine } from '../engine';
import { targetKey, type ProcessorModule } from './processorModel';
import type { ProcessorProps } from './useProcessor';

export const EQ_FREQUENCIES = Array.from({ length: 129 }, (_, i) => 20 * Math.pow(1000, i / 128));
export const DYNAMICS_INPUTS = Array.from({ length: 121 }, (_, i) => -60 + i / 2);
export interface ProcessorCurve { x: number[]; y: number[]; revision: number }
/** Model static settings, never the instantaneous automation or attack/release state. */
export function useProcessorCurve(props: ProcessorProps, module: ProcessorModule) {
  const { bridge, timeline, target, session, revision } = props;
  const key = targetKey(target), current = useRef({ key, session, revision }); current.current = { key, session, revision };
  const [curve, setCurve] = useState<ProcessorCurve | null>(null), [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    let live = true; setCurve(null); setProblem(null);
    const valid = (reply: number) => live && current.current.key === key && current.current.session === session && reply >= current.current.revision;
    const request = module === 'eq' ? callEngine(bridge, 'audio.eq_response', { timeline: timeline.id, target, frequencies: EQ_FREQUENCIES.filter(f => f <= timeline.sample_rate * .45) }, { expectedSession: session })
      .then(result => { if (valid(result.revision) && Array.isArray(result.gain_db)) setCurve({ x: result.frequencies, y: result.gain_db, revision: result.revision }); })
      : callEngine(bridge, 'audio.dynamics_curve', { timeline: timeline.id, target, inputs_db: DYNAMICS_INPUTS }, { expectedSession: session })
        .then(result => { if (valid(result.revision) && Array.isArray(result.outputs_db)) setCurve({ x: result.inputs_db, y: result.outputs_db, revision: result.revision }); });
    void request.catch(error => { if (live) setProblem(error instanceof Error ? error.message : String(error)); });
    return () => { live = false; };
  }, [bridge, timeline.id, timeline.sample_rate, key, session, revision, module]);
  return { curve, problem };
}
