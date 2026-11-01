import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { GradeNode, Param, PowerWindow, Target, ViewerTool, WindowDeleted, WindowKeys, WindowPoint, WindowSelection, WindowState,
  WindowTopology } from '../engine';
import type { Messages } from '../../messages/en';
import { getAt, isKeyed, keyAt } from './corrector';
import { NumberControl } from './controls';
import type { NodeEdit } from './useNodeEdit';
import type { EditScope, WindowField } from './windowEdits';
import type { GradeEditing } from './useGradeEditing';
import type { WindowInputStatus } from './useViewerInput';

export const circularWindow = (id: string): PowerWindow => ({ id, label: '', enabled: true, invert: false, combine: 'add', opacity: 100,
  transform: { size: 50, aspect: 50, pan: 50, tilt: 50, rotate: 0 }, shape: { kind: 'circular', softness: 1 } });

// Curve editing is a later step; its rows stay visible but cannot be chosen yet.
const supported = (window: PowerWindow) => window.shape.kind !== 'curve';

const rectangle = () => [[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]].map(([x, y]) => ({ x, y }));
/** A new window's shape, as the model makes it (Linear Soft 1–4 at 1; Polygon inside/outside 0). */
export function windowShape(kind: 'circular' | 'gradient' | 'linear' | 'polygon'): PowerWindow['shape'] {
  if (kind === 'linear') return { kind, points: rectangle() as [WindowPoint, WindowPoint, WindowPoint, WindowPoint], softness: [1, 1, 1, 1] };
  if (kind === 'polygon') return { kind, points: rectangle(), inside: 0, outside: 0 };
  return { kind, softness: 1 };
}
/** A refused whole-animation Polygon point insertion or deletion, explained in the palette. */
export interface WindowNotice { action: 'insert' | 'delete'; topology: WindowTopology }
type WindowWords = Messages['label']['studio']['colorPage']['windowControls'];
const fill = (template: string, values: Record<string, string | number>) => template.replace(/\{(\w+)\}/g, (all, name: string) => String(values[name] ?? all));
/** Points are 1-based for people; the engine reports 0-based indices and the actual key interval. */
export function topologyText(words: WindowWords, notice: WindowNotice): string {
  const topology = notice.topology; const text = words.topology;
  if (topology.why === 'maximum') return fill(text.maximum, { most: topology.most });
  if (topology.why === 'minimum') return fill(text.minimum, { least: topology.least });
  if (topology.why === 'unsupported') return text.unsupported;
  const frames = topology.frames ? fill(text.frames, { clock: words.clock[topology.clock], from: topology.frames[0], to: topology.frames[1] }) : text.allFrames;
  return fill(text[topology.why], { axis: topology.axis.toUpperCase(), points: topology.points.map(point => point + 1).join(text.joiner), frames });
}

interface Creation { identity: string; object: string; session: number; generation: number; id: string; revision: number | null }
/** Keeps creation acknowledgment separate from the real ProjectCopy and from the user's current intent. */
export function useWindows({ session, revision, timeline, target, clip, node, realNode, complete, editing, tool, frame, clock }: {
  session: number; revision: number; timeline: string | null; target: Target | null; clip: string | null;
  node: GradeNode | null; realNode: GradeNode | null; complete: boolean; editing: GradeEditing; tool: ViewerTool;
  frame: number; clock: WindowKeys['clock'];
}) {
  const identity = JSON.stringify([session, timeline, target, node?.id, clip]);
  // Group windows are the same object when shown through another clip or timeline.
  const object = JSON.stringify([session, target, node?.id]);
  const intentKey = JSON.stringify([identity, tool]);
  const intent = useRef({ key: intentKey, generation: 0 });
  if (intent.current.key !== intentKey) intent.current = { key: intentKey, generation: intent.current.generation + 1 };
  const [choice, setChoice] = useState<{ identity: string; id: string | null } | null>(null);
  const [intention, setIntention] = useState(0);
  const [reported, setReported] = useState<{ identity: string; keys: WindowKeys; id: string } | null>(null);
  const pending = useRef<Creation | null>(null);
  const [, redraw] = useState(0);
  const windows = node?.kind === 'corrector' ? node.params.windows ?? [] : [];
  const unconfirmed = pending.current?.object === object ? pending.current.id : null;
  const selected = (choice?.identity === identity
    ? windows.find(each => each.id === choice.id && supported(each) && each.id !== unconfirmed)
    : windows.find(each => each.shape.kind === 'circular' && each.id !== unconfirmed)
      ?? windows.find(each => supported(each) && each.id !== unconfirmed)) ?? null;
  const enabled = !!target && !!timeline && node?.kind === 'corrector' && node.enabled && !node.locked;
  const now = useRef({ identity, object, revision, session, timeline, target, clip, node, realNode, editing, enabled, windows, selected });
  now.current = { identity, object, revision, session, timeline, target, clip, node, realNode, editing, enabled, windows, selected };

  useEffect(() => {
    // Remember the initial row so deleting it releases its binding instead of silently targeting another row.
    // Returning to a pending first creation also remembers its empty selection, before a late ack clears the gate.
    if (choice?.identity !== identity && (selected || unconfirmed !== null)) setChoice({ identity, id: selected?.id ?? null });
  }, [identity, selected?.id, choice?.identity, unconfirmed]);

  useEffect(() => {
    const asked = pending.current;
    if (!asked) return;
    if (asked.session !== session) { pending.current = null; redraw(x => x + 1); return; }
    if (asked.revision === null || revision < asked.revision) return;
    pending.current = null;
    // The copy may already contain a later undo/delete. A successful reply alone never selects it.
    // Intent changes cancel only auto-selection, not the acknowledgment/copy gate for this object.
    if (asked.identity === identity && asked.generation === intent.current.generation && realNode?.id === node?.id
      && realNode?.kind === 'corrector' && realNode.params.windows?.some(each => each.id === asked.id)) {
      setChoice({ identity, id: asked.id }); setIntention(value => value + 1);
    }
    redraw(x => x + 1);
  });

  const waitingFor = useCallback((id: string) => pending.current?.object === now.current.object && pending.current.id === id, []);
  const choose = useCallback((id: string) => {
    if (waitingFor(id)) return;
    intent.current.generation++;
    setChoice({ identity: now.current.identity, id }); setIntention(value => value + 1); setReported(null);
  }, [waitingFor]);
  const create = useCallback((busy: () => boolean, kind: 'circular' | 'gradient' | 'linear' | 'polygon' = 'circular') => {
    const at = now.current;
    if (pending.current || !at.enabled || !at.target || !at.node || busy()) return;
    let number = 1; while (at.windows.some(each => each.id === `w${number}`)) number++;
    const asked: Creation = { identity: at.identity, object: at.object, session: at.session, generation: intent.current.generation, id: `w${number}`, revision: null };
    pending.current = asked; redraw(value => value + 1);
    // Preserve an explicitly empty selection too; a late creation must not become the fallback row after intent changes.
    setChoice({ identity: at.identity, id: at.selected?.id ?? null });
    at.editing.run([{ op: 'set_node', grade: at.target, id: at.node.id, params: { windows: [...at.windows, { ...circularWindow(asked.id), shape: windowShape(kind) }] } }], answer => {
      if (pending.current !== asked) return;
      asked.revision = answer.revision; redraw(value => value + 1);
    }, () => { if (pending.current === asked) { pending.current = null; redraw(value => value + 1); } });
  }, []);
  const onState = useCallback((state: WindowState) => {
    setChoice({ identity: now.current.identity, id: state.selected });
    setReported(state.phase === 'arming' || state.phase === 'armed' ? { identity: now.current.identity, keys: state.keys, id: state.selected } : null);
  }, []);
  /** A middle-click deletion selects the window whose point it removed, like a native down. */
  const onDeleted = useCallback((answer: WindowDeleted) => {
    setChoice({ identity: now.current.identity, id: answer.selected }); setReported(null);
  }, []);
  const selection: WindowSelection | null = enabled && complete && realNode?.id === node?.id && realNode?.kind === 'corrector'
    && selected?.enabled && realNode.params.windows?.some(each => each.id === selected.id)
    ? { monitor: 'timeline', revision, timeline: timeline!, target: target!, ...(clip ? { clip } : {}), node: node!.id,
      selected: selected.id, visible: true, outline: true, tool: 'window' } : null;
  const keyFrame = (name: 'size' | 'aspect' | 'pan' | 'tilt' | 'rotate' | 'softness') => {
    // Linear and Polygon softness are per-side or inside/outside numbers, each with its own key title.
    const param = name === 'softness' ? selected && (selected.shape.kind === 'circular' || selected.shape.kind === 'gradient') ? selected.shape.softness : undefined : selected?.transform[name];
    if (param === undefined) return undefined;
    return isKeyed(param) && param.keys.length ? param.keys[keyAt(param.keys, frame)].frame : null;
  };
  const keys: WindowKeys = reported?.identity === identity && reported.id === selected?.id ? reported.keys
    : { size: keyFrame('size'), aspect: keyFrame('aspect'), pan: keyFrame('pan'), tilt: keyFrame('tilt'), rotate: keyFrame('rotate'), softness: keyFrame('softness'), clock, frame };
  return { windows, selected, selection, intention, choose, create, onState, onDeleted, keys, enabled, waitingFor, pending: pending.current !== null };
}

/** The participating fields of a captured control, or every keyed field of the selected window. */
function hints(words: WindowWords, keys: WindowKeys): string[] {
  const at = (frame: number | null | undefined) => frame === null || frame === undefined ? words.static : `${words.key} ${frame}`;
  const parts = (['size', 'aspect', 'pan', 'tilt', 'rotate'] as const).filter(name => keys[name] !== undefined)
    .map(name => `${words.fields[name]}: ${at(keys[name])}`);
  if (keys.softness !== undefined) {
    const name = keys.edge === undefined ? 'softness' : (`soft${keys.edge + 1}` as 'soft1');
    parts.push(`${words.fields[name]}: ${at(keys.softness)}`);
  }
  for (const point of keys.points ?? []) {
    parts.push(`${point.point === keys.inserted ? words.inserted : `${words.point} ${point.point + 1}`}: X ${at(point.x)} · Y ${at(point.y)}`);
  }
  return parts;
}

export function WindowPalette({ controls, capturing, busy, status, edit, notice = null }: {
  edit: NodeEdit; controls: ReturnType<typeof useWindows>; capturing: boolean; busy: () => boolean; status: WindowInputStatus;
  notice?: WindowNotice | null;
}) {
  const words = useWords().label.studio.colorPage.windowControls;
  const { selected, keys } = controls;
  const blocked = !controls.enabled || controls.pending || capturing || edit.pending();
  const fields = selected ? [...TRANSFORM_FIELDS, ...SOFTNESS_FIELDS[selected.shape.kind]] : [];
  return <div className="color-window-palette" data-test="window-palette">
    <button type="button" className="btn" disabled={!controls.enabled || controls.pending || capturing || edit.pending()}
      onClick={() => controls.create(() => busy() || edit.pending())}>{words.add}</button>
    <button type="button" className="btn" disabled={blocked}
      onClick={() => controls.create(() => busy() || edit.pending(), 'gradient')}>{words.addGradient}</button>
    <button type="button" className="btn" disabled={blocked}
      onClick={() => controls.create(() => busy() || edit.pending(), 'linear')}>{words.addLinear}</button>
    <button type="button" className="btn" disabled={blocked}
      onClick={() => controls.create(() => busy() || edit.pending(), 'polygon')}>{words.addPolygon}</button>
    <div role="listbox" aria-label={words.list} className="color-window-list">
      {controls.windows.map(each => <button type="button" className="btn" role="option" key={each.id} aria-selected={selected?.id === each.id}
        disabled={!supported(each) || controls.waitingFor(each.id)} onClick={() => controls.choose(each.id)}>
        {each.label || each.id} · {words.shapes[each.shape.kind]}</button>)}
    </div>
    {selected && <div data-test="window-keys">
      <p>{words.editing}: {selected.label || selected.id}</p>
      <p>{hints(words, keys).join(' · ')}</p>
      <p>{words.clock[keys.clock]} · {words.frame} {keys.frame}</p>
    </div>}
    {selected && <div className="color-settings" data-test="window-numbers">
      {fields.map(({ field, name, neutral, low, high, perPx }) => <WindowNumber key={`${selected.id}:${field}`} edit={edit} id={selected.id} field={field}
        name={words.fields[name]} neutral={neutral} low={low} high={high} perPx={perPx} disabled={capturing || controls.pending || !edit.enabled}
        clock={`${words.clock[keys.clock]}`} />)}
    </div>}
    <p role="status">{controls.pending ? words.creating : notice ? topologyText(words, notice) : words.status[status]}</p>
  </div>;
}

interface FieldSpec { field: WindowField; name: keyof WindowWords['fields']; neutral: number; low: number; high: number; perPx: number }
const TRANSFORM_FIELDS: FieldSpec[] = [
  { field: 'transform.size', name: 'size', neutral: 50, low: 0, high: 1000, perPx: 0.25 },
  { field: 'transform.aspect', name: 'aspect', neutral: 50, low: 0, high: 100, perPx: 0.25 },
  { field: 'transform.pan', name: 'pan', neutral: 50, low: -1000, high: 1000, perPx: 0.25 },
  { field: 'transform.tilt', name: 'tilt', neutral: 50, low: -1000, high: 1000, perPx: 0.25 },
  { field: 'transform.rotate', name: 'rotate', neutral: 0, low: -36000, high: 36000, perPx: 1 },
  { field: 'opacity', name: 'opacity', neutral: 100, low: 0, high: 100, perPx: 0.25 },
];
/** The Softness group of the Window palette per shape (RM20 p. 3193): Soft 1, Soft 1–4, or Inside/Outside. */
const soft = (field: WindowField, name: FieldSpec['name'], neutral: number): FieldSpec => ({ field, name, neutral, low: 0, high: 100, perPx: 0.25 });
const SOFTNESS_FIELDS: Record<PowerWindow['shape']['kind'], FieldSpec[]> = {
  circular: [soft('shape.softness', 'softness', 1)],
  gradient: [soft('shape.softness', 'softness', 1)],
  linear: ([0, 1, 2, 3] as const).map(side => soft(`shape.softness.${side}`, `soft${side + 1}` as FieldSpec['name'], 1)),
  polygon: [soft('shape.inside', 'inside', 0), soft('shape.outside', 'outside', 0)],
  curve: [],
};

function WindowNumber({ edit, id, field, name, neutral, low, high, perPx, disabled, clock }: {
  edit: NodeEdit; id: string; field: WindowField; name: string; neutral: number; low: number; high: number; perPx: number; disabled: boolean; clock: string;
}) {
  const words = useWords().label.studio.colorPage.windowControls;
  const index = (edit.corrector.windows ?? []).findIndex(w => w.id === id);
  const path = `windows.${index}.${field}`;
  const identity = `${id}:${edit.controlKey([path])}`;
  const scope = useMemo<EditScope>(() => ({ live: true }), [identity]);
  // A committed selection/key change retires this control even if playback/selection returns.
  // Retiring drops only its previews and unsent drag continuation; a value it already committed
  // (typed, released or Escape) stays queued by window id and lands on the confirmed predecessor.
  useLayoutEffect(() => () => edit.retireScope(scope), [scope]);
  const edits = (value: number) => [['windows', { window: { id, field, value }, scope }]] as const;
  const allowed = () => scope.live && edit.isCurrent();
  const param = getAt(edit.corrector.windows?.[index], field) as Param | undefined;
  const key = isKeyed(param) && param.keys.length ? param.keys[keyAt(param.keys, edit.frame)].frame : null;
  return <NumberControl key={identity} name={name} field={path} value={edit.value(path)} neutral={neutral} low={low} high={high} digits={2} perPx={perPx}
    keyed={edit.keyed(path)} keyedTitle={`${clock} · ${key === null ? words.static : `${words.key} ${key}`}`} disabled={disabled || index < 0}
    onPreview={value => { if (allowed()) edit.preview(edits(value)); }} onCommit={value => { if (allowed()) edit.commit(edits(value)); }}
    onCancel={() => { if (allowed()) edit.cancel(scope); }} />;
}
