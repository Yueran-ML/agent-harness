import { useEffect, useRef, useState } from 'react';
import { callEngine, type AutomationParameter, type Pan } from '../engine';
import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { laneValue } from '../fairlight/automation';
import { mixerStrips, stripChange } from './mixerModel';
import { processorWritable, targetOf } from './processorModel';
import { defaultPan } from './routingModel';
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
  const [problem, setProblem] = useState(''), [parameters, setParameters] = useState<AutomationParameter[]>([]);
  const [preview, setPreview] = useState<Values>({});
  const touched = useRef<string[]>([]), snapshot = useRef<Values>({});
  const owner = useRef<object | undefined>(undefined);
  const gesture = useAudioGesture({ ...context, identity, locked: strip.locked || bus !== undefined && !send, onFailure: error => setProblem(String(error)) });
  useEffect(() => {
    setParameters([]); if (!automation?.enabled) return;
    let live = true;
    void callEngine(context.bridge, 'automation.parameters', { timeline: context.timeline.id, of }, { expectedSession: context.session })
      .then(result => { if (live && result.revision === context.revision) setParameters(result.parameters); })
      .catch(error => { if (live) setProblem(String(error)); });
    return () => { live = false; };
  }, [context.bridge, context.session, context.revision, identity, automation?.enabled]);
  const parameter = (field: string) => parameters.find(item => item.path === path(field));
  const base = (field: string): number => {
    if (bus === undefined) return Number(({ ...defaultPan(), ...strip.strip?.pan })[field as keyof Pan]);
    if (field.startsWith('pan.')) return Number(({ ...defaultPan(), ...send?.pan })[field.slice(4) as keyof Pan]);
    return Number(field === 'on' ? send?.on ?? true : field === 'pre' ? send?.pre ?? false : send?.level_db ?? 0);
  };
  const owns = (fields: string[], source?: object) => (!source || source === owner.current) && fields.some(field => touched.current.includes(field) && (recording ? automation?.active({ of, path: path(field) }) : gesture.active()));
  const reading = (field: string) => {
    const value = laneValue(automation?.enabled !== false && strip.arm !== 'off' ? strip.lanes?.[path(field)] : undefined, context.seconds ?? 0, base(field));
    if (owns([field]) && field in preview) return preview[field];
    return automation?.reading({ of, path: path(field) }, value, parameter(field)) ?? value;
  };
  const writable = (field: string) => !strip.locked && (bus === undefined || Boolean(send)) && (recording ? Boolean(parameter(field)) : automation?.enabled === false || processorWritable(strip, path(field)));
  const changes = (values: Values) => {
    const panPatch = (values: Values, prefix = '') => Object.fromEntries(Object.entries(values).filter(([key]) => key.startsWith(prefix)).map(([key, value]) => {
      const field = key.slice(prefix.length); return [field, ['on', 'lfe_on', 'lfe_pre'].includes(field) ? value >= .5 : value];
    }));
    const patch = bus === undefined ? { pan: panPatch(values) } : { sends: (strip.strip?.sends ?? []).map(row => row.bus !== bus ? row : {
      ...row, ...Object.fromEntries(Object.entries(values).filter(([key]) => !key.startsWith('pan.')).map(([key, value]) => [key, key === 'on' || key === 'pre' ? value >= .5 : value])),
      pan: { ...defaultPan(), ...row.pan, ...panPatch(values, 'pan.') },
    }) };
    return [stripChange(context.timeline.id, strip, { strip: patch })];
  };
  const valuesFor = (values: Values) => Object.entries(values).map(([field, value]) => ({ target: { of, path: path(field) }, value }));
  const start = (fields: string[], source?: object) => {
    if (!fields.length || fields.some(field => !writable(field))) return false;
    const initial = Object.fromEntries(fields.map(field => [field, reading(field)]));
    if (!(recording ? automation!.startBatch(valuesFor(initial)) : gesture.start(changes(Object.fromEntries(fields.map(field => [field, base(field)])))))) return false;
    touched.current = fields; owner.current = source; snapshot.current = initial; setPreview(initial); return true;
  };
  const end = (fields = touched.current, cancel = false) => {
    if (!owns(fields)) return;
    if (recording) automation!.end(); else if (cancel) gesture.cancel(); else gesture.end();
    touched.current = []; setPreview({});
  };
  const latest = useRef({ end }); latest.current = { end };
  useEffect(() => () => latest.current.end(undefined, true), [identity, context.session]);
  useEffect(() => { if (!gesture.active()) setPreview({}); }, [context.revision, identity]);
  return { strip, send, recording, problem, parameter, path, reading, writable, active: owns, start,
    snapshot: () => snapshot.current,
    change: (values: Values, source?: object) => {
      const fields = Object.keys(values); if (!fields.length || !owns(fields, source) || fields.some(field => !touched.current.includes(field))) return;
      setPreview(old => ({ ...old, ...values }));
      if (recording) automation!.changeBatch(valuesFor(values)); else gesture.preview(changes({ ...snapshot.current, ...values }));
    },
    end: (fields = touched.current, source?: object) => { if (owns(fields, source)) end(fields); }, cancel: (fields = touched.current, source?: object) => { if (owns(fields, source)) end(fields, true); },
    commit: (values: Values) => { if (Object.keys(values).some(field => !writable(field))) return; if (recording) automation!.instantBatch(valuesFor(values)); else gesture.commit(changes(values)); },
    show: (field: string) => automation?.show({ of, path: path(field) }),
  };
}
export type RoutingParameter = ReturnType<typeof useRoutingParameter>;
