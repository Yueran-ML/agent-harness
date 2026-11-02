import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { GradeNode, Param, PowerWindow, Target, ViewerTool, WindowDeleted, WindowKeys, WindowPointsDeleted, WindowSelection,
  WindowState, WindowTopology } from '../engine';
import type { Messages } from '../../messages/en';
import { getAt, isKeyed, keyAt } from './corrector';
import { NumberControl } from './controls';
import type { NodeEdit } from './useNodeEdit';
import { newWindow, windowShape, type EditScope, type WindowField, type WindowKind } from './windowEdits';
import type { GradeEditing } from './useGradeEditing';
import type { WindowInputStatus } from './useViewerInput';
import { WindowPresets, type PresetBridge } from './WindowPresets';

/** A new Circular window (`newWindow`), as the Add Circular Window button makes one. */
export const circularWindow = (id: string): PowerWindow => newWindow(id);
export { windowShape };

// Every shape has on-screen controls.
const supported = (_window: PowerWindow) => true;

/** A refused whole-animation Polygon/Curve point insertion or deletion (`deleteSelected`: a Curve's selected points), explained in the palette. */
export interface WindowNotice { action: 'insert' | 'delete' | 'deleteSelected'; topology: WindowTopology }
type WindowWords = Messages['label']['studio']['colorPage']['windowControls'];
const fill = (template: string, values: Record<string, string | number>) => template.replace(/\{(\w+)\}/g, (all, name: string) => String(values[name] ?? all));
/** Points are 1-based for people; the engine reports 0-based indices and the actual key interval.
 * An edge has two ends, so a refusal names one point or two. */
export function topologyText(messages: Messages, notice: WindowNotice): string {
  const topology = notice.topology; const words = messages.label.studio.colorPage.windowControls;
  const part = words.topology; const text = messages.hint.studio.colorPage.windowControls.topology;
  if (topology.why === 'maximum') return fill(text.maximum, { most: topology.most });
  if (topology.why === 'minimum') {
    const curve = notice.action === 'deleteSelected' ? text.minimumCurvePoints : text.minimumCurve;
    return fill(topology.shape === 'curve' ? curve : text.minimum, { least: topology.least });
  }
  if (topology.why === 'unsupported') return text.unsupported;
  const frames = topology.frames ? fill(part.frames, { clock: words.clock[topology.clock], from: topology.frames[0], to: topology.frames[1] }) : part.allFrames;
  const [first, second] = topology.points.map(point => point + 1);
  const points = second === undefined ? fill(part.one, { point: first }) : fill(part.two, { first, second });
  return fill(text[topology.why], { axis: topology.axis.toUpperCase(), points, frames });
}

interface Creation { identity: string; object: string; session: number; generation: number; id: string; revision: number | null }
/** Keeps creation acknowledgment separate from the real ProjectCopy and from the user's current intent. */
export function useWindows({ bridge = null, session, revision, timeline, target, clip, node, realNode, complete, editing, tool, frame, clock }: {
  /** The engine, for the window presets of the option menu; none hides them. */
  bridge?: PresetBridge | null; session: number; revision: number; timeline: string | null; target: Target | null; clip: string | null;
  node: GradeNode | null; realNode: GradeNode | null; complete: boolean; editing: GradeEditing; tool: ViewerTool;
  frame: number; clock: WindowKeys['clock'];
}) {
  const identity = JSON.stringify([session, timeline, target, node?.id, clip]);
  // Group windows are the same object when shown through another clip or timeline.
  const object = JSON.stringify([session, target, node?.id]);
  const intentKey = JSON.stringify([identity, tool]);
  const intent = useRef({ key: intentKey, generation: 0 });
  if (intent.current.key !== intentKey) intent.current = { key: intentKey, generation: intent.current.generation + 1 };
  // `deselected`: Esc left an unfinished Curve; the binding stays, selecting none, so a click picks a window again.
  const [choice, setChoice] = useState<{ identity: string; id: string | null; deselected?: boolean } | null>(null);
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
  /** Idle Esc on an unfinished Curve keeps its draft and selects no window; choosing the Curve again continues it
   * (author decision 2026-11-02). False, doing nothing, for any other selection. */
  const leaveDraft = useCallback(() => {
    const shape = now.current.selected?.shape;
    if (shape?.kind !== 'curve' || shape.closed) return false;
    intent.current.generation++;
    setChoice({ identity: now.current.identity, id: null, deselected: true }); setIntention(value => value + 1); setReported(null);
    return true;
  }, []);
  const create = useCallback((busy: () => boolean, kind: WindowKind = 'circular') => {
    const at = now.current;
    if (pending.current || !at.enabled || !at.target || !at.node || busy()) return;
    let number = 1; while (at.windows.some(each => each.id === `w${number}`)) number++;
    const asked: Creation = { identity: at.identity, object: at.object, session: at.session, generation: intent.current.generation, id: `w${number}`, revision: null };
    pending.current = asked; redraw(value => value + 1);
    // Preserve an explicitly empty selection too; a late creation must not become the fallback row after intent changes.
    setChoice({ identity: at.identity, id: at.selected?.id ?? null });
    at.editing.run([{ op: 'set_node', grade: at.target, id: at.node.id, params: { windows: [...at.windows, newWindow(asked.id, kind)] } }], answer => {
      if (pending.current !== asked) return;
      asked.revision = answer.revision; redraw(value => value + 1);
    }, () => { if (pending.current === asked) { pending.current = null; redraw(value => value + 1); } });
  }, []);
  const onState = useCallback((state: WindowState) => {
    setChoice({ identity: now.current.identity, id: state.selected });
    setReported(state.phase === 'arming' || state.phase === 'armed' ? { identity: now.current.identity, keys: state.keys, id: state.selected } : null);
  }, []);
  /** A middle-click or Backspace deletion selects the window whose points it removed, like a native down. */
  const onDeleted = useCallback((answer: WindowDeleted | WindowPointsDeleted) => {
    setChoice({ identity: now.current.identity, id: answer.selected }); setReported(null);
  }, []);
  const deselected = choice?.identity === identity && choice.deselected === true && unconfirmed === null && windows.some(each => each.enabled);
  const selection: WindowSelection | null = enabled && complete && realNode?.id === node?.id && realNode?.kind === 'corrector'
    && (deselected || (selected?.enabled && realNode.params.windows?.some(each => each.id === selected.id)))
    ? { monitor: 'timeline', revision, timeline: timeline!, target: target!, ...(clip ? { clip } : {}), node: node!.id,
      selected: selected?.id ?? null, visible: true, outline: true, tool: 'window' } : null;
  const keyFrame = (name: 'size' | 'aspect' | 'pan' | 'tilt' | 'rotate' | 'softness') => {
    // Linear and Polygon softness are per-side or inside/outside numbers, each with its own key title.
    const param = name === 'softness' ? selected && (selected.shape.kind === 'circular' || selected.shape.kind === 'gradient') ? selected.shape.softness : undefined : selected?.transform[name];
    if (param === undefined) return undefined;
    return isKeyed(param) && param.keys.length ? param.keys[keyAt(param.keys, frame)].frame : null;
  };
  const keys: WindowKeys = reported?.identity === identity && reported.id === selected?.id ? reported.keys
    : { size: keyFrame('size'), aspect: keyFrame('aspect'), pan: keyFrame('pan'), tilt: keyFrame('tilt'), rotate: keyFrame('rotate'), softness: keyFrame('softness'), clock, frame };
  // A preset is saved from, and recalled onto, a node of the project (not the default node shown before it is made).
  const presets = { bridge, session, target, node: realNode && realNode.id === node?.id ? realNode.id : null, frame, revision, windows: windows.length };
  return { windows, selected, selection, intention, choose, leaveDraft, create, onState, onDeleted, keys, enabled, waitingFor, pending: pending.current !== null, presets };
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
  for (const side of ['inside', 'outside'] as const) if (keys[side] !== undefined) parts.push(`${words.fields[side]}: ${at(keys[side])}`);
  for (const point of keys.points ?? []) {
    const name = point.point === keys.inserted ? words.inserted : `${words.point} ${point.point + 1}`;
    if ('x' in point || 'y' in point) parts.push(`${name}: X ${at(point.x)} · Y ${at(point.y)}`);
    // A Curve point's Bezier handles, each with its own left keys.
    if ('in_x' in point || 'in_y' in point) parts.push(`${name} ${words.handleIn}: X ${at(point.in_x)} · Y ${at(point.in_y)}`);
    if ('out_x' in point || 'out_y' in point) parts.push(`${name} ${words.handleOut}: X ${at(point.out_x)} · Y ${at(point.out_y)}`);
  }
  return parts;
}

export function WindowPalette({ controls, capturing, busy, status, edit, notice = null, points = 0 }: {
  edit: NodeEdit; controls: ReturnType<typeof useWindows>; capturing: boolean; busy: () => boolean; status: WindowInputStatus;
  notice?: WindowNotice | null;
  /** How many of the selected Curve's points are Shift-selected in the viewer (RM20 p. 3195). */
  points?: number;
}) {
  const messages = useWords(); const words = messages.label.studio.colorPage.windowControls;
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
    <button type="button" className="btn" disabled={blocked}
      onClick={() => controls.create(() => busy() || edit.pending(), 'curve')}>{words.addCurve}</button>
    <WindowPresets place={{ ...controls.presets, enabled: !blocked }} onRecalled={controls.choose} />
    {/* Each row (RM20 p. 3189): the On/Off button showing its shape, the button that chooses the window by its id, its
      * Layer Name, then its Invert and Mask switches. */}
    <div role="listbox" aria-label={words.list} className="color-window-list">
      {controls.windows.map(each => {
        const disabled = !controls.enabled || controls.pending || capturing || !edit.enabled || controls.waitingFor(each.id);
        return <div role="presentation" className="color-window-row" key={each.id}>
          <WindowOnOff edit={edit} window={each} disabled={disabled} onEnabled={() => controls.choose(each.id)} />
          <button type="button" className="btn" role="option" aria-selected={selected?.id === each.id}
            disabled={!supported(each) || controls.waitingFor(each.id)} onClick={() => controls.choose(each.id)}>
            {each.id}</button>
          <WindowLayerName edit={edit} window={each} disabled={disabled} />
          <WindowSwitches edit={edit} window={each} disabled={disabled} />
        </div>;
      })}
    </div>
    {selected && <div data-test="window-keys">
      <p>{words.editing}: {selected.label || selected.id}</p>
      <p>{hints(words, keys).join(' · ')}</p>
      <p>{words.clock[keys.clock]} · {words.frame} {keys.frame}</p>
      {points > 0 && selected.shape.kind === 'curve' && <p data-test="window-points">
        {fill(words.selectedPoints, { count: points })} · {messages.hint.studio.colorPage.windowControls.selectedPoints}</p>}
    </div>}
    {selected && <div className="color-settings" data-test="window-numbers">
      {fields.map(({ field, name, neutral, low, high, perPx }) => <WindowNumber key={`${selected.id}:${field}`} edit={edit} id={selected.id} field={field}
        name={words.fields[name]} neutral={neutral} low={low} high={high} perPx={perPx} disabled={capturing || controls.pending || !edit.enabled}
        clock={`${words.clock[keys.clock]}`} />)}
    </div>}
    <p role="status">{controls.pending ? words.creating : notice ? topologyText(messages, notice)
      : status === 'refused' ? messages.hint.studio.colorPage.windowControls.refused : status === 'idle' ? '' : words.status[status]}</p>
  </div>;
}

/**
 * A row's Invert and Mask (RM20 p. 3189): Mask on (pressed, the default) adds the window to the others, off subtracts it
 * (p. 3196, the project's reading). Each click is a plain commit of the edit queue, resolved by window id against its
 * confirmed predecessor and sent as the whole Windows list: one undo step, shown at once. They are switches of the
 * whole window and are never keyed (PROTOCOL.md, Power Windows), as On/Off is not.
 */
function WindowSwitches({ edit, window, disabled }: { edit: NodeEdit; window: PowerWindow; disabled: boolean }) {
  const words = useWords().label.studio.colorPage.windowControls;
  const shown = edit.corrector.windows?.find(each => each.id === window.id) ?? window;
  const name = window.label || window.id;
  return <>
    <button type="button" className="btn color-window-switch" aria-pressed={shown.invert} aria-label={`${words.invert} ${name}`} title={words.invert}
      disabled={disabled} onClick={() => edit.commit([['windows', { window: { id: window.id, field: 'invert', value: !shown.invert } }]])}>
      {words.invert}</button>
    <button type="button" className="btn color-window-switch" aria-pressed={shown.combine === 'add'} aria-label={`${words.mask} ${name}`} title={words.mask}
      disabled={disabled} onClick={() => edit.commit([['windows', { window: { id: window.id, field: 'combine', value: shown.combine === 'add' ? 'subtract' : 'add' } }]])}>
      {words.mask}</button>
  </>;
}

/**
 * A row's On/Off button, which shows the window's shape (RM20 p. 3189), committed as Invert and Mask are. A window turned
 * on is also chosen, so that its onscreen controls appear (p. 3190); one turned off keeps the selection, without them.
 */
function WindowOnOff({ edit, window, disabled, onEnabled }: { edit: NodeEdit; window: PowerWindow; disabled: boolean; onEnabled: () => void }) {
  const words = useWords().label.studio.colorPage.windowControls;
  const shown = edit.corrector.windows?.find(each => each.id === window.id) ?? window;
  const shape = words.shapes[window.shape.kind];
  return <button type="button" className="btn color-window-switch" aria-pressed={shown.enabled} aria-label={`${words.onOff} ${window.label || window.id}`}
    title={`${shape} · ${words.onOff}`} disabled={disabled} onClick={() => {
      edit.commit([['windows', { window: { id: window.id, field: 'enabled', value: !shown.enabled } }]]);
      if (!shown.enabled) onEnabled();
    }}>{shape}</button>;
}

/**
 * A row's Layer Name, blank until it is given one (RM20 p. 3189): typed freely, sent trimmed when Enter is pressed or the
 * field is left, if it changed, as one commit of the whole list like Invert and Mask; Escape puts the name back.
 */
function WindowLayerName({ edit, window, disabled }: { edit: NodeEdit; window: PowerWindow; disabled: boolean }) {
  const words = useWords().label.studio.colorPage.windowControls;
  const shown = edit.corrector.windows?.find(each => each.id === window.id) ?? window;
  const [draft, setDraft] = useState<string | null>(null);
  const typed = useRef<string | null>(null);
  const write = (value: string | null) => { typed.current = value; setDraft(value); };
  const send = () => {
    const label = typed.current?.trim();
    write(null);
    if (label !== undefined && label !== shown.label) edit.commit([['windows', { window: { id: window.id, field: 'label', value: label } }]]);
  };
  return <input type="text" className="field color-window-name" value={draft ?? shown.label} maxLength={100} autoComplete="off" spellCheck={false}
    aria-label={`${words.layerName} ${window.label || window.id}`} title={words.layerName} disabled={disabled}
    onChange={event => write(event.target.value)} onBlur={send}
    onKeyDown={event => {
      if (event.key === 'Enter') { event.preventDefault(); send(); }
      else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); write(null); }
    }} />;
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
  curve: [soft('shape.inside', 'inside', 0), soft('shape.outside', 'outside', 0)],
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
