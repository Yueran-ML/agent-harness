import type { AutomationParameter, LanesOf } from '../engine';
/** Structural adapter implemented by Fairlight's AutomationTouchControl. Edit leaves it absent. */
export interface AudioAutomationAdapter {
  enabled: boolean; recording: boolean; trim: boolean; preview: boolean;
  start: (target: { of: LanesOf; path: string }, value: number) => boolean;
  change: (value: number) => void; end: () => void; active: (target?: { of: LanesOf; path: string }) => boolean;
  instant: (target: { of: LanesOf; path: string }, value: number) => boolean;
  show: (target: { of: LanesOf; path: string }) => void;
  lives?: readonly { target: { of: LanesOf; path: string }; value: number }[];
  live: { target: { of: LanesOf; path: string }; value: number } | null;
  reading?: (target: {of:LanesOf;path:string}, baseline:number, parameter?:Pick<AutomationParameter,'kind'|'scale'>) => number;
  status?: (target: {of:LanesOf;path:string}) => 'write'|'preview'|null;
}
