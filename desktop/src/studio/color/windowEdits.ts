import type { Corrector, Param, PowerWindow } from '../engine';
import { getAt, withValue } from './corrector';

export type WindowField = 'transform.size' | 'transform.aspect' | 'transform.pan' | 'transform.tilt' | 'transform.rotate' | 'opacity' | 'shape.softness';
/** A local control lifetime. Once retired, returning to the same window/key cannot revive it. */
export interface EditScope { live: boolean }
export interface WindowEdit { window: { id: string; field: WindowField; value: number }; scope?: EditScope }

export function windowPath(base: Corrector, edit: WindowEdit): string {
  return `windows.${(base.windows ?? []).findIndex(w => w.id === edit.window.id)}.${edit.window.field}`;
}

/** Resolve a field intention only when its predecessor is confirmed, preserving all other Params. */
export function windowValue(base: Corrector, edit: WindowEdit, frame: number): PowerWindow[] | null {
  const windows = base.windows ?? [];
  const { id, field, value } = edit.window;
  const index = windows.findIndex(w => w.id === id);
  if (index < 0) return null;
  const window = windows[index];
  const param = withValue(getAt(window, field) as Param, frame, value);
  let next: PowerWindow;
  if (field === 'opacity') next = { ...window, opacity: param };
  else if (field === 'shape.softness') {
    if (window.shape.kind !== 'circular' && window.shape.kind !== 'gradient') return null;
    next = { ...window, shape: { ...window.shape, softness: param } };
  } else next = { ...window, transform: { ...window.transform, [field.slice('transform.'.length)]: param } };
  return windows.map((w, i) => i === index ? next : w);
}
