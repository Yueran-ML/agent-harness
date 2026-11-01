import type { Corrector, Param, PowerWindow } from '../engine';
import { getAt, withValue } from './corrector';

export type WindowField = 'transform.size' | 'transform.aspect' | 'transform.pan' | 'transform.tilt' | 'transform.rotate' | 'opacity'
  | 'shape.softness' | `shape.softness.${0 | 1 | 2 | 3}` | 'shape.inside' | 'shape.outside';
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
  const shape = window.shape;
  if (field === 'opacity') next = { ...window, opacity: param };
  else if (field === 'shape.softness') {
    if (shape.kind !== 'circular' && shape.kind !== 'gradient') return null;
    next = { ...window, shape: { ...shape, softness: param } };
  } else if (field.startsWith('shape.softness.')) {
    // Linear Soft 1–4: one side, keeping the other three Params as they are.
    if (shape.kind !== 'linear') return null;
    const side = Number(field.slice('shape.softness.'.length));
    next = { ...window, shape: { ...shape, softness: shape.softness.map((each, i) => i === side ? param : each) as typeof shape.softness } };
  } else if (field === 'shape.inside' || field === 'shape.outside') {
    if (shape.kind !== 'polygon' && shape.kind !== 'curve') return null;
    next = { ...window, shape: { ...shape, [field.slice('shape.'.length)]: param } };
  } else next = { ...window, transform: { ...window.transform, [field.slice('transform.'.length)]: param } };
  return windows.map((w, i) => i === index ? next : w);
}
