import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { GradeNode, PowerWindow, Target, ViewerTool, WindowKeys, WindowSelection, WindowState } from '../engine';
import { isKeyed, keyAt } from './corrector';
import { NumberControl } from './controls';
import type { NodeEdit } from './useNodeEdit';
import type { EditScope, WindowField } from './windowEdits';
import type { GradeEditing } from './useGradeEditing';
import type { WindowInputStatus } from './useViewerInput';

export const circularWindow = (id: string): PowerWindow => ({ id, label: '', enabled: true, invert: false, combine: 'add', opacity: 100,
  transform: { size: 50, aspect: 50, pan: 50, tilt: 50, rotate: 0 }, shape: { kind: 'circular', softness: 1 } });

const supported = (window: PowerWindow) => window.shape.kind === 'circular' || window.shape.kind === 'gradient';

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
  const create = useCallback((busy: () => boolean, kind: 'circular' | 'gradient' = 'circular') => {
    const at = now.current;
    if (pending.current || !at.enabled || !at.target || !at.node || busy()) return;
    let number = 1; while (at.windows.some(each => each.id === `w${number}`)) number++;
    const asked: Creation = { identity: at.identity, object: at.object, session: at.session, generation: intent.current.generation, id: `w${number}`, revision: null };
    pending.current = asked; redraw(value => value + 1);
    // Preserve an explicitly empty selection too; a late creation must not become the fallback row after intent changes.
    setChoice({ identity: at.identity, id: at.selected?.id ?? null });
    at.editing.run([{ op: 'set_node', grade: at.target, id: at.node.id, params: { windows: [...at.windows, { ...circularWindow(asked.id), shape: { kind, softness: 1 } }] } }], answer => {
      if (pending.current !== asked) return;
      asked.revision = answer.revision; redraw(value => value + 1);
    }, () => { if (pending.current === asked) { pending.current = null; redraw(value => value + 1); } });
  }, []);
  const onState = useCallback((state: WindowState) => {
    setChoice({ identity: now.current.identity, id: state.selected });
    setReported(state.phase === 'arming' || state.phase === 'armed' ? { identity: now.current.identity, keys: state.keys, id: state.selected } : null);
  }, []);
  const selection: WindowSelection | null = enabled && complete && realNode?.id === node?.id && realNode?.kind === 'corrector'
    && selected?.enabled && realNode.params.windows?.some(each => each.id === selected.id)
    ? { monitor: 'timeline', revision, timeline: timeline!, target: target!, ...(clip ? { clip } : {}), node: node!.id,
      selected: selected.id, visible: true, outline: true, tool: 'window' } : null;
  const keyFrame = (name: 'size' | 'aspect' | 'pan' | 'tilt' | 'rotate' | 'softness') => {
    const param = name === 'softness' ? selected && (selected.shape.kind === 'circular' || selected.shape.kind === 'gradient') ? selected.shape.softness : undefined : selected?.transform[name];
    return isKeyed(param) && param.keys.length ? param.keys[keyAt(param.keys, frame)].frame : null;
  };
  const keys: WindowKeys = reported?.identity === identity && reported.id === selected?.id ? reported.keys
    : { size: keyFrame('size'), aspect: keyFrame('aspect'), pan: keyFrame('pan'), tilt: keyFrame('tilt'), rotate: keyFrame('rotate'), softness: keyFrame('softness'), clock, frame };
  return { windows, selected, selection, intention, choose, create, onState, keys, enabled, waitingFor, pending: pending.current !== null };
}

export function WindowPalette({ controls, capturing, busy, status, edit }: {
  edit: NodeEdit; controls: ReturnType<typeof useWindows>; capturing: boolean; busy: () => boolean; status: WindowInputStatus;
}) {
  const words = useWords().label.studio.colorPage.windowControls;
  const { selected, keys } = controls;
  return <div className="color-window-palette" data-test="window-palette">
    <button type="button" className="btn" disabled={!controls.enabled || controls.pending || capturing || edit.pending()}
      onClick={() => controls.create(() => busy() || edit.pending())}>{words.add}</button>
    <button type="button" className="btn" disabled={!controls.enabled || controls.pending || capturing || edit.pending()}
      onClick={() => controls.create(() => busy() || edit.pending(), 'gradient')}>{words.addGradient}</button>
    <div role="listbox" aria-label={words.list} className="color-window-list">
      {controls.windows.map(each => <button type="button" className="btn" role="option" key={each.id} aria-selected={selected?.id === each.id}
        disabled={!supported(each) || controls.waitingFor(each.id)} onClick={() => controls.choose(each.id)}>
        {each.label || each.id} · {words.shapes[each.shape.kind]}</button>)}
    </div>
    {selected && <div data-test="window-keys">
      <p>{words.editing}: {selected.label || selected.id}</p>
      <p>{(['size', 'aspect', 'pan', 'tilt', 'rotate', 'softness'] as const).filter(name => keys[name] !== undefined)
        .map(name => `${words.fields[name]}: ${keys[name] === null ? words.static : `${words.key} ${keys[name]}`}`).join(' · ')}</p>
      <p>{words.clock[keys.clock]} · {words.frame} {keys.frame}</p>
    </div>}
    {selected && <div className="color-settings" data-test="window-numbers">
      {WINDOW_FIELDS.map(({ field, name, neutral, low, high, perPx }) => <WindowNumber key={`${selected.id}:${field}`} edit={edit} id={selected.id} field={field}
        name={words.fields[name]} neutral={neutral} low={low} high={high} perPx={perPx} disabled={capturing || controls.pending || !edit.enabled}
        clock={`${words.clock[keys.clock]}`} />)}
    </div>}
    <p role="status">{controls.pending ? words.creating : words.status[status]}</p>
  </div>;
}

const WINDOW_FIELDS = [
  { field: 'transform.size', name: 'size', neutral: 50, low: 0, high: 1000, perPx: 0.25 },
  { field: 'transform.aspect', name: 'aspect', neutral: 50, low: 0, high: 100, perPx: 0.25 },
  { field: 'transform.pan', name: 'pan', neutral: 50, low: -1000, high: 1000, perPx: 0.25 },
  { field: 'transform.tilt', name: 'tilt', neutral: 50, low: -1000, high: 1000, perPx: 0.25 },
  { field: 'transform.rotate', name: 'rotate', neutral: 0, low: -36000, high: 36000, perPx: 1 },
  { field: 'opacity', name: 'opacity', neutral: 100, low: 0, high: 100, perPx: 0.25 },
  { field: 'shape.softness', name: 'softness', neutral: 1, low: 0, high: 100, perPx: 0.25 },
] as const;

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
  const param = field === 'opacity' ? edit.corrector.windows?.[index]?.opacity
    : field === 'shape.softness' ? (() => { const shape = edit.corrector.windows?.[index]?.shape; return shape?.kind === 'circular' || shape?.kind === 'gradient' ? shape.softness : undefined; })()
    : edit.corrector.windows?.[index]?.transform[field.slice(10) as keyof PowerWindow['transform']];
  const key = isKeyed(param) && param.keys.length ? param.keys[keyAt(param.keys, edit.frame)].frame : null;
  return <NumberControl key={identity} name={name} field={path} value={edit.value(path)} neutral={neutral} low={low} high={high} digits={2} perPx={perPx}
    keyed={edit.keyed(path)} keyedTitle={`${clock} · ${key === null ? words.static : `${words.key} ${key}`}`} disabled={disabled || index < 0}
    onPreview={value => { if (allowed()) edit.preview(edits(value)); }} onCommit={value => { if (allowed()) edit.commit(edits(value)); }}
    onCancel={() => { if (allowed()) edit.cancel(scope); }} />;
}
