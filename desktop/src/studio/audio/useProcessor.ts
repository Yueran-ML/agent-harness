import { useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Dynamics, Equalizer, Timeline } from '../engine';
import type { Change } from '../timeline/commands';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { mixerStrips, operationTargets, type Modifiers } from './mixerModel';
import { defaultDynamics, defaultEq, processorChanges, processorPlayback, processorWritable, targetKey, type ProcessorModule, type ProcessorTarget } from './processorModel';

export interface ProcessorProps {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline; target: ProcessorTarget; selected?: readonly string[]; seconds?: number;
}
export function useProcessor<M extends ProcessorModule>(props: ProcessorProps, module: M) {
  type Settings = M extends 'eq' ? Equalizer : Dynamics;
  const { bridge, session, revision, timeline, target, selected = [] } = props;
  const identity = `${timeline.id}/${targetKey(target)}/${module}`;
  const strips = mixerStrips(timeline), strip = strips.find(s => s.key === targetKey(target));
  const clipOwner = 'clip' in target ? timeline.tracks.find(t => t.kind === 'audio' && t.clips.some(c => c.id === target.clip)) : undefined;
  const clip = 'clip' in target && clipOwner?.kind === 'audio' ? clipOwner.clips.find(c => c.id === target.clip) : undefined;
  const model = (module === 'eq' ? clip?.eq ?? strip?.strip?.eq ?? defaultEq('clip' in target) : strip?.strip?.dynamics ?? defaultDynamics()) as Settings;
  const disabled = 'clip' in target ? !clip || Boolean(clipOwner?.locked) : !strip || !processorWritable(strip, module);
  const [problem, setProblem] = useState<string | null>(null), [preview, setPreview] = useState<Settings | null>(null);
  const gesture = useAudioGesture({ bridge, session, revision, identity, locked: disabled, onFailure: error => setProblem(error instanceof Error ? error.message : String(error)) });
  const snapshot = useRef(model);
  const changes = (value: Settings, modifiers: Modifiers = {}): Change[] => 'clip' in target
    ? [{ op: 'set_clip', timeline: timeline.id, id: target.clip, eq: value as Equalizer }]
    : processorChanges(timeline.id, operationTargets(strips, targetKey(target), selected, modifiers), module, value);
  useEffect(() => { setPreview(null); setProblem(null); }, [identity, session]);
  const start = () => { if (disabled || !gesture.start(changes(model))) return false; snapshot.current = model; setPreview(model); return true; };
  return { value: gesture.active() ? preview ?? model : processorPlayback(model, strip, module, props.seconds ?? 0), model, disabled, problem, gesture,
    start, snapshot: () => snapshot.current,
    change: (value: Settings) => { if (!gesture.active()) return; setPreview(value); gesture.preview(changes(value)); },
    end: () => { gesture.end(); setPreview(null); }, cancel: () => { gesture.cancel(); setPreview(null); },
    commit: (value: Settings, modifiers: Modifiers = {}) => { if (!disabled) gesture.commit(changes(value, modifiers)); },
    enable: (on: boolean, modifiers: Modifiers = {}) => {
      if (disabled) return;
      // Enabling a selection changes only its switches, never copies the source's parameters.
      gesture.commit('clip' in target ? changes({ ...model, on })
        : processorChanges(timeline.id, operationTargets(strips, targetKey(target), selected, modifiers), module, { on }));
    },
  };
}
