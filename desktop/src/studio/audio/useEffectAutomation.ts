import { useAutomationTouch } from '../fairlight/AutomationTouch';
import { useAutomationParameters } from './useAutomationParameters';
import type { ProcessorProps } from './useProcessor';

/** FX use the same model catalogue and held-value reader as EQ, Pan and Sends. */
export function useEffectAutomation(context: ProcessorProps) {
  const provider = useAutomationTouch(), automation = provider ?? context.automation;
  const { parameters, problem } = useAutomationParameters(context,
    'clip' in context.target ? null : context.target, Boolean(provider?.enabled));
  const recording = Boolean(automation?.enabled && automation.recording);
  const parameter = (path:string) => parameters.find(value => value.path === path);
  // A legacy injected adapter is only a compatibility seam. Production always has the Provider.
  const supported = (path:string) => !('clip' in context.target) && (!provider || Boolean(parameter(path)));
  const reading = (path:string, baseline:number) => 'clip' in context.target || !automation?.enabled ? baseline
    : automation.reading?.({of:context.target,path},baseline,parameter(path)) ?? baseline;
  const status = (path:string) => 'clip' in context.target ? null : automation?.status?.({of:context.target,path}) ?? null;
  return {automation, recording, parameter, supported, reading, status, problem};
}
