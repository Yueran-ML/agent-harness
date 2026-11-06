import { useEffect, useState } from 'react';
import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { callEngine, type AutomationParameter } from '../engine';
import { targetKey } from './processorModel';
import type { ProcessorProps } from './useProcessor';

/** FX use the same model catalogue and held-value reader as EQ, Pan and Sends. */
export function useEffectAutomation(context: ProcessorProps) {
  const provider = useAutomationTouch(), automation = provider ?? context.automation;
  const [catalogue, setCatalogue] = useState<{identity:string; parameters:AutomationParameter[]} | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const identity = `${context.session}/${context.timeline.id}/${context.revision}/${targetKey(context.target)}`;
  const recording = Boolean(automation?.enabled && automation.recording);
  useEffect(() => {
    setCatalogue(null); setProblem(null);
    if (!provider?.enabled || 'clip' in context.target) return;
    let live = true;
    void callEngine(context.bridge, 'automation.parameters', {timeline:context.timeline.id, of:context.target}, {expectedSession:context.session})
      .then(result => { if (live && result.revision === context.revision) setCatalogue({identity,parameters:result.parameters}); })
      .catch(error => { if (live) setProblem(String(error)); });
    return () => { live = false; };
  }, [context.bridge, identity, provider?.enabled]);
  const parameter = (path:string) => catalogue?.identity === identity ? catalogue.parameters.find(value => value.path === path) : undefined;
  // A legacy injected adapter is only a compatibility seam. Production always has the Provider.
  const supported = (path:string) => !('clip' in context.target) && (!provider || Boolean(parameter(path)));
  const reading = (path:string, baseline:number) => 'clip' in context.target || !automation?.enabled ? baseline
    : automation.reading?.({of:context.target,path},baseline,parameter(path)) ?? baseline;
  const status = (path:string) => 'clip' in context.target ? null : automation?.status?.({of:context.target,path}) ?? null;
  return {automation, recording, parameter, supported, reading, status, problem};
}
