import { useFailureText } from '../fairlight/words';
import { useEffect, useRef, useState } from 'react';
import { useAutomationParameters } from './useAutomationParameters';
import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { laneValue } from '../fairlight/automation';
import { mixerStrips } from './mixerModel';
import { processorWritable, targetOf } from './processorModel';
import { routingBase, routingChanges, routingPlan, type RoutingPlan } from './routingGroups';
import type { ProcessorProps } from './useProcessor';
import { useAudioGesture } from '../timeline/useAudioGesture';
type Values = Record<string, number>;
/** One routing control owns explicit paths, both for static undo and recorder touch batches. */
export function useRoutingParameter(context: ProcessorProps, stripKey: string, bus?: string) {
  const strip = mixerStrips(context.timeline).find(row => row.key === stripKey)!;
  const of = targetOf(strip), automation = useAutomationTouch();
  const recording = Boolean(automation?.enabled && automation.recording);
  const send = strip.strip?.sends?.find(row => row.bus === bus);
  const identity = `${context.timeline.id}/${stripKey}/${bus === undefined ? 'pan' : `send.${bus}`}`;
  const prefix = bus === undefined ? 'pan.' : `send.${bus}.`;
  const path = (field: string) => prefix + field;
  const [problem, setProblem] = useState('');
  const failure = useFailureText();
  const { parameters, problem: parameterProblem } = useAutomationParameters(context, of, Boolean(automation?.enabled));
  const [preview, setPreview] = useState<Values>({});
  const touched = useRef<string[]>([]), snapshot = useRef<Values>({});
  const owner = useRef<object | undefined>(undefined);
  const gesture = useAudioGesture({
    ...context, identity, locked: strip.locked || bus !== undefined && !send, onFailure: error => setProblem(failure(error))
  });
  const parameter = (field: string) => parameters.find(item => item.path === path(field));
  const base = (field: string) => routingBase(strip, bus, field);
  const captured = useRef<RoutingPlan | null>(null);
  const owns = (fields: string[], source?: object) => (!source || source === owner.current) && fields.some(field => touched.current.includes(field) && (recording ? automation?.active({ of, path: path(field) }) : gesture.active()));
  const reading = (field: string) => {
    const value = laneValue(automation?.enabled !== false && strip.arm !== 'off' ? strip.lanes?.[path(field)] : undefined, context.seconds ?? 0, base(field));
    if (owns([field]) && field in preview)
      return preview[field];
    return automation?.reading({ of, path: path(field) }, value, parameter(field)) ?? value;
  };
  const writable = (field: string) => !strip.locked && (bus === undefined || Boolean(send)) && (recording ? Boolean(parameter(field)) : automation?.enabled === false || processorWritable(strip, path(field)));
  const valuesFor = (values: Values) => Object.entries(values).map(([field, value]) => ({ target: { of, path: path(field) }, value }));
  const start = (fields: string[], source?: object) => {
    if (!fields.length || fields.some(field => !writable(field)))
      return false;
    const initial = Object.fromEntries(fields.map(field => [field, reading(field)]));
    const plan = routingPlan(context.timeline, strip, bus, fields, automation?.enabled !== false);
    if (!(recording ? automation!.startBatch(valuesFor(initial)) : gesture.start(plan.restore)))
      return false;
    captured.current = plan;
    touched.current = fields;
    owner.current = source;
    snapshot.current = initial;
    setPreview(initial);
    return true;
  };
  const end = (fields = touched.current, cancel = false) => {
    if (!owns(fields))
      return;
    if (recording)
      automation!.end();
    else if (cancel)
      gesture.cancel();
    else
      gesture.end();
    touched.current = [];
    setPreview({});
  };
  const latest = useRef({ end });
  latest.current = { end };
  useEffect(() => () => latest.current.end(undefined, true), [identity, context.session]);
  useEffect(() => {
    if (!gesture.active())
      setPreview({});
  }, [context.revision, identity]);
  return {
    strip, send, recording, problem: problem || parameterProblem, parameter, path, reading, writable, active: owns, start,
    snapshot: () => snapshot.current,
    change: (values: Values, source?: object) => {
      const fields = Object.keys(values);
      if (!fields.length || !owns(fields, source) || fields.some(field => !touched.current.includes(field)))
        return;
      setPreview(old => ({ ...old, ...values }));
      if (recording)
        automation!.changeBatch(valuesFor(values));
      else
        gesture.preview(routingChanges(captured.current!, { ...snapshot.current, ...values }, true));
    },
    end: (fields = touched.current, source?: object) => {
      if (owns(fields, source))
        end(fields);
    }, cancel: (fields = touched.current, source?: object) => {
      if (owns(fields, source))
        end(fields, true);
    },
    commit: (values: Values) => {
      if (Object.keys(values).some(field => !writable(field)))
        return;
      if (recording)
        automation!.instantBatch(valuesFor(values));
      else
        gesture.commit(routingChanges(routingPlan(context.timeline, strip, bus, Object.keys(values), automation?.enabled !== false), values));
    },
    show: (field: string) => automation?.show({ of, path: path(field) }),
    /** How the control of `field` shows its automation: written, in Preview or played back. */
    status: (field: string) => automation?.status?.({ of, path: path(field) }) ?? null,
  };
}
export type RoutingParameter = ReturnType<typeof useRoutingParameter>;
