import type { Corrector, Param, PowerWindow, WindowPoint } from '../engine';
import { getAt, withValue } from './corrector';

/** The five shapes of the Window palette (RM20 p. 3189). */
export type WindowKind = PowerWindow['shape']['kind'];
const rectangle = () => [[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]].map(([x, y]) => ({ x, y }));
/** A new window's shape, as the model makes it (Linear Soft 1–4 at 1; Polygon/Curve inside/outside 0).
 * A Curve starts as an empty open draft: clicks in the viewer draw it (RM20 p. 3193). */
export function windowShape(kind: WindowKind): PowerWindow['shape'] {
  if (kind === 'curve') return { kind, points: [], closed: false, inside: 0, outside: 0 };
  if (kind === 'linear') return { kind, points: rectangle() as [WindowPoint, WindowPoint, WindowPoint, WindowPoint], softness: [1, 1, 1, 1] };
  if (kind === 'polygon') return { kind, points: rectangle(), inside: 0, outside: 0 };
  return { kind, softness: 1 };
}
/** A new enabled, additive (Mask on), uninverted window at the centre, as the model makes one. */
export const newWindow = (id: string, kind: WindowKind = 'circular'): PowerWindow => ({ id, label: '', enabled: true, invert: false,
  combine: 'add', opacity: 100, transform: { size: 50, aspect: 50, pan: 50, tilt: 50, rotate: 0 }, shape: windowShape(kind) });

export type WindowField = 'transform.size' | 'transform.aspect' | 'transform.pan' | 'transform.tilt' | 'transform.rotate' | 'opacity'
  | 'shape.softness' | `shape.softness.${0 | 1 | 2 | 3}` | 'shape.inside' | 'shape.outside';
/** A local control lifetime. Once retired, returning to the same window/key cannot revive it. */
export interface EditScope { live: boolean }
/**
 * One window's field as a control sets it, resolved by window id. A number goes where an edit at the playhead goes
 * (a value or the left key); On/Off (`enabled`), the Layer Name (`label`), Invert and Mask (`combine`) are of the whole
 * window, never keyed (PROTOCOL.md).
 */
export type WindowChange = { id: string; field: WindowField; value: number } | { id: string; field: 'enabled' | 'invert'; value: boolean }
  | { id: string; field: 'label'; value: string } | { id: string; field: 'combine'; value: PowerWindow['combine'] };
export interface WindowEdit { window: WindowChange; scope?: EditScope }

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
  const change = edit.window;
  if (change.field === 'enabled' || change.field === 'invert' || change.field === 'label' || change.field === 'combine') {
    return windows.map((w, i) => i === index ? { ...w, [change.field]: change.value } : w);
  }
  const param = withValue(getAt(window, field) as Param, frame, value as number);
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
